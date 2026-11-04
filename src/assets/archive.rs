//! Read-only access to the installed game's resource indices and archives.
pub mod index;
mod containers;
use flate2::read::ZlibDecoder;
use std::error::Error;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy)]
struct Entry {
    group: u8,
    offset: u64,
    size: u32,
    stored_size: u32,
    slot: u16,
    /// Index into `Catalog::kinds`.
    kind: u16,
    flags: u16,
}

/// Sorted strings packed into one buffer. The installed index lists about a
/// million resource paths; as separate `String`s their headers and
/// allocations cost more than their text.
#[derive(Default)]
struct Names {
    text: String,
    ends: Vec<u32>,
}

impl Names {
    fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut packed = Self::default();
        for name in names {
            packed.text.push_str(name);
            packed.ends.push(packed.text.len() as u32);
        }
        packed.text.shrink_to_fit();
        packed.ends.shrink_to_fit();
        packed
    }

    fn get(&self, index: usize) -> &str {
        let start = index.checked_sub(1).map_or(0, |i| self.ends[i] as usize);
        &self.text[start..self.ends[index] as usize]
    }

    fn len(&self) -> usize {
        self.ends.len()
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(|i| self.get(i))
    }

    fn search(&self, key: &str) -> std::result::Result<usize, usize> {
        let index = self.partition_point(|name| name < key);
        if index < self.len() && self.get(index) == key { Ok(index) } else { Err(index) }
    }

    fn partition_point(&self, before: impl Fn(&str) -> bool) -> usize {
        let (mut low, mut high) = (0, self.len());
        while low < high {
            let middle = (low + high) / 2;
            if before(self.get(middle)) { low = middle + 1 } else { high = middle }
        }
        low
    }
}

pub struct Catalog {
    /// The index kinds (`image`, `material`, ...) of the entries.
    kinds: Vec<String>,
    base: PathBuf,
    /// Sorted resource paths, and the entry of each.
    paths: Names,
    entries: Vec<Entry>,
    // Loose resource files of the rewrite's enabled mods by normalized
    // resource path, sorted; they win over the archives.
    overlay: Vec<(String, PathBuf)>,
    // Image source names are authoritative index aliases, including imageprog
    // expressions and option suffixes. Values index the final override table.
    image_sources: (Names, Vec<u32>),
    image_programs: (Names, Vec<u32>),
    containers: containers::Containers,
}

// Native image programs are tokenized before their source alias is emitted.
// Ignore spacing around punctuation, preserving quoted paths and word breaks.
fn image_program_key(source: &str) -> String {
    let source = source.trim().to_ascii_lowercase();
    let source = source.strip_suffix(".imageprog").unwrap_or(&source);
    let mut key = String::new();
    let mut quoted = false;
    let mut space = false;
    for c in source.chars() {
        let c = if c == '\\' { '/' } else { c };
        if !quoted && c.is_whitespace() {
            space = true;
            continue;
        }
        if space
            && !key.is_empty()
            && !"(),".contains(c)
            && !"(),".contains(key.chars().last().unwrap())
        {
            key.push(' ');
        }
        space = false;
        key.push(c);
        if c == '"' {
            quoted = !quoted;
        }
    }
    key
}

impl Catalog {
    /// 140066ac0..140066b8c classifies these as streamed rather than map
    /// preload requests. The load-list reader already normalizes this path.
    pub(crate) fn preloads(&self, path: &str) -> bool {
        let streamed = self.paths.search(path).ok().is_some_and(|index| self.entries[index].flags & 0x20 != 0);
        !streamed && ![".sbcm", ".tbcm", ".bwm"].iter().any(|suffix| path.contains(suffix))
    }

    fn entry(&self, path: &str) -> Option<&Entry> {
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        self.paths.search(&normalized).ok().map(|index| &self.entries[index])
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

use index::archive_path;
/// One catalog per installation for the whole process: every owner reads the
/// same index instead of opening its own copy.
static SHARED: std::sync::Mutex<Option<(PathBuf, std::sync::Arc<Catalog>)>> = std::sync::Mutex::new(None);

impl Catalog {
    /// The process's catalog of `base`, opened on first use.
    pub fn shared(base: &Path) -> Result<std::sync::Arc<Self>> {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((path, catalog)) = shared.as_ref().filter(|(path, _)| path == base) {
            let _ = path;
            return Ok(catalog.clone());
        }
        let catalog = std::sync::Arc::new(Self::open(base)?);
        *shared = Some((base.to_path_buf(), catalog.clone()));
        Ok(catalog)
    }

    pub fn open(base: &Path) -> Result<Self> {
        let mut entries = Vec::new();
        let mut kinds: Vec<String> = Vec::new();
        let mut image_sources = Vec::new();
        for group in 1..=4 {
            index::visit_group(base, group, |record| {
                let index::IndexEntry {
                    kind,
                    mut source,
                    mut path,
                    offset,
                    size,
                    stored_size,
                    archive_slot,
                    flags,
                    ..
                } = record;
                path.make_ascii_lowercase();
                if path.contains('\\') {
                    path = path.replace('\\', "/");
                }
                if kind.eq_ignore_ascii_case("image") && path.ends_with(".bimage7") {
                    source.make_ascii_lowercase();
                    if source.contains('\\') {
                        source = source.replace('\\', "/");
                    }
                    image_sources.push((source, path.clone()));
                }
                let kind = match kinds.iter().position(|k| *k == kind) {
                    Some(i) => i as u16,
                    None => {
                        kinds.push(kind);
                        (kinds.len() - 1) as u16
                    }
                };
                entries.push((
                    path,
                    Entry {
                        group,
                        offset,
                        size,
                        stored_size,
                        slot: archive_slot,
                        kind,
                        flags,
                    },
                ));
            })?;
        }
        // The original map overwrote duplicate paths as later index groups
        // were read. Stable sorting preserves file order among duplicate
        // keys, and this fold keeps the final entry from that order.
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        entries.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = later.1;
                true
            } else {
                false
            }
        });
        let mut image_sources: Vec<_> = image_sources
            .into_iter()
            .map(|(source, path)| {
                let index = entries.binary_search_by(|(key, _)| key.cmp(&path)).unwrap();
                (source, index)
            })
            .collect();
        image_sources.sort_by(|left, right| left.0.cmp(&right.0));
        image_sources.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = later.1;
                true
            } else {
                false
            }
        });
        let mut image_programs = image_sources
            .iter()
            .filter(|(s, _)| s.ends_with(".imageprog"))
            .map(|(s, i)| (image_program_key(s), *i))
            .collect::<Vec<_>>();
        image_programs.sort();
        image_programs.dedup();
        let packed = |pairs: Vec<(String, usize)>| {
            (Names::new(pairs.iter().map(|(name, _)| name.as_str())), pairs.iter().map(|&(_, i)| i as u32).collect())
        };
        Ok(Self {
            containers: Default::default(),
            kinds,
            base: base.to_path_buf(),
            paths: Names::new(entries.iter().map(|(path, _)| path.as_str())),
            entries: entries.into_iter().map(|(_, entry)| entry).collect(),
            image_sources: packed(image_sources),
            image_programs: packed(image_programs),
            overlay: crate::platform::home::layers().files,
        })
    }

    /// The overlay file replacing `path`, if any.
    fn overlay_file(&self, path: &str) -> Option<&Path> {
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        self.overlay
            .binary_search_by(|(key, _)| key.as_str().cmp(&normalized))
            .ok()
            .map(|i| self.overlay[i].1.as_path())
    }

    /// Resource paths of the archives and the overlay, sorted and unique.
    fn all_paths<'a>(&'a self, keep: impl Fn(&str) -> bool + 'a) -> Vec<String> {
        let mut paths: Vec<String> = self
            .paths
            .iter()
            .chain(self.overlay.iter().map(|(path, _)| path.as_str()))
            .filter(|path| keep(path))
            .map(str::to_string)
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.overlay_file(path).is_some() || self.entry(path).is_some()
    }

    /// Decoded size recorded by the installed resource index (an overlay
    /// file's own size).
    pub fn resource_size(&self, path: &str) -> Option<usize> {
        if let Some(file) = self.overlay_file(path) {
            return std::fs::metadata(file).ok().map(|m| m.len() as usize);
        }
        self.entry(path).map(|entry| entry.size as usize)
    }

    /// Registered container lookup (140064590). This uses the same selected
    /// entry as payload acquisition, without reopening archive indices.
    pub(super) fn resource_container(&self, path: &str) -> Option<(u8, u16)> {
        self.entry(path).map(|entry| (entry.group, entry.slot))
    }

    /// Resolve the source->compiled resource mapping stored by the game itself.
    pub fn image_resource(&self, source: &str) -> Option<&str> {
        // Native idStr slash normalization140140da0/140140de0 treats both
        // Windows separators alike before resource/image-source lookup.
        let source = source.replace('\\', "/").to_ascii_lowercase();
        let index = self.image_sources.0.search(&source).ok()?;
        Some(self.paths.get(self.image_sources.1[index] as usize))
    }

    /// Resolve any compiled image program through authoritative index aliases;
    /// no guessed compiled filename or replacement procedural image is used.
    pub fn image_program_resource(&self, source: &str) -> Option<&str> {
        let key = image_program_key(source);
        let (names, indices) = &self.image_programs;
        let i = names.partition_point(|s| s < key.as_str());
        if i >= names.len() || names.get(i) != key || (i + 1 < names.len() && names.get(i + 1) == key) {
            return None;
        }
        Some(self.paths.get(indices[i] as usize))
    }

    pub fn paths_with_suffix(&self, suffix: &str) -> Vec<String> {
        let suffix = suffix.to_ascii_lowercase();
        self.all_paths(move |path| path.ends_with(&suffix))
    }

    pub fn paths_containing(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_ascii_lowercase();
        self.all_paths(move |path| path.contains(&needle))
    }

    /// The installed `base` folder the catalog reads.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// The index kind of `path`.
    pub fn kind(&self, path: &str) -> Option<&str> {
        self.entry(path).map(|e| self.kinds[e.kind as usize].as_str())
    }

    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let entry = self.entry(path);
        if let Some(entry) = entry {
            crate::loading::read(&self.kinds[entry.kind as usize], path);
        }
        if let Some(file) = self.overlay_file(path) {
            return Ok(Some(std::fs::read(file)?));
        }
        entry
            .map(|entry| {
                let mut reader = self.containers.acquire(&self.base, entry.group, entry.slot)?;
                let length = reader.length;
                read_file(&mut reader.file, length, path, entry)
            })
            .transpose()
    }

    /// Read only the start of an installed resource. This is useful for large
    /// indexed files whose header tables precede a much larger payload.
    pub fn read_prefix(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        if let Some(file) = self.overlay_file(path) {
            let mut output = Vec::new();
            File::open(file)?.take(limit as u64).read_to_end(&mut output)?;
            return Ok(Some(output));
        }
        let Some(entry) = self.entry(path) else {
            return Ok(None);
        };
        let mut reader = self.containers.acquire(&self.base, entry.group, entry.slot)?;
        validate_extent(path, entry, reader.length)?;
        let file = &mut reader.file;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut output = Vec::with_capacity((entry.size as usize).min(limit));
        if entry.size == entry.stored_size {
            file.take(entry.stored_size as u64)
                .take(limit as u64)
                .read_to_end(&mut output)?;
        } else {
            let decoder = ZlibDecoder::new(file.take(entry.stored_size as u64));
            decoder.take(limit as u64).read_to_end(&mut output)?;
        }
        Ok(Some(output))
    }
}

fn read_location(base: &Path, path: &str, entry: &Entry) -> Result<Vec<u8>> {
    let archive = archive_path(base, entry.group, entry.slot)?;
    let mut file = File::open(&archive)?;
    let archive_size = file.metadata()?.len();
    read_file(&mut file, archive_size, path, entry)
}

fn validate_extent(path: &str, entry: &Entry, archive_size: u64) -> Result<()> {
    if entry
        .offset
        .checked_add(u64::from(entry.stored_size))
        .is_none_or(|end| end > archive_size)
    {
        return Err(invalid(format!("{path}: indexed resource exceeds archive extent")).into());
    }
    Ok(())
}

fn read_file(file: &mut File, archive_size: u64, path: &str, entry: &Entry) -> Result<Vec<u8>> {
    validate_extent(path, entry, archive_size)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    // Native worlds exceed 128 MiB. Bound reads by the indexed archive
    // extent and decoded size, rather than an unrelated texture-size cap.
    // Stream decompression so loading a world does not retain a second
    // allocation for its entire compressed representation.
    let input = file.take(u64::from(entry.stored_size));
    let mut output = Vec::new();
    // The decoded size plus the one probe byte that detects an oversized
    // stream, so read_to_end never regrows and copies the buffer.
    output.try_reserve_exact(entry.size as usize + 1)?;
    if entry.size == entry.stored_size {
        input
            .take(u64::from(entry.size) + 1)
            .read_to_end(&mut output)?;
    } else {
        ZlibDecoder::new(input)
            .take(entry.size as u64 + 1)
            .read_to_end(&mut output)?;
    }
    if output.len() != entry.size as usize {
        return Err(invalid(format!(
            "{}: expected {} bytes, decoded {}",
            path,
            entry.size,
            output.len()
        ))
        .into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn be16_bytes(value: u16) -> [u8; 2] {
        value.to_be_bytes()
    }

    fn be32_bytes(value: u32) -> [u8; 4] {
        value.to_be_bytes()
    }

    fn be64_bytes(value: u64) -> [u8; 8] {
        value.to_be_bytes()
    }

    fn index_entry(path: &str, offset: u64, size: u32) -> Vec<u8> {
        typed_index_entry("kind", "source", path, offset, size)
    }

    fn typed_index_entry(kind: &str, source: &str, path: &str, offset: u64, size: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(be32_bytes(0));
        for value in [kind, source, path] {
            bytes.extend((value.len() as u32).to_le_bytes());
            bytes.extend(value.as_bytes());
        }
        bytes.extend(be64_bytes(offset));
        bytes.extend(be32_bytes(size));
        bytes.extend(be32_bytes(size));
        bytes.extend([0; 6]);
        bytes.extend(be16_bytes(0));
        bytes.extend(be16_bytes(0));
        bytes
    }

    fn write_index(path: &Path, entries: &[Vec<u8>]) {
        let payload_size: usize = 4 + entries.iter().map(Vec::len).sum::<usize>();
        let mut file = File::create(path).unwrap();
        file.write_all(b"\x05SER").unwrap();
        file.write_all(&be32_bytes(payload_size as u32)).unwrap();
        file.write_all(&[0; 24]).unwrap();
        file.write_all(&be32_bytes(entries.len() as u32)).unwrap();
        for entry in entries {
            file.write_all(entry).unwrap();
        }
    }

    #[test]
    fn resource_slots_are_restricted() {
        assert!(archive_path(Path::new("base"), 2, 8192)
            .unwrap()
            .ends_with("shared_2_3.sharedrsc"));
        assert!(archive_path(Path::new("base"), 1, 8192).is_err());
    }

    #[test]
    fn level_resolution_rejects_ambiguity_and_uses_indexed_physics_names() {
        use crate::assets::level::LevelResources;
        let root = std::env::temp_dir().join(format!("dh2-level-resources-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let paths = [
            "maps/a/sample_lowchaos/sample_lowchaos.bwm",
            "maps/b/sample_lowchaos/sample_lowchaos.bwm",
            "maps/a/sample_lowchaos.entities",
            "maps/a/sample_lowchaos/sample_lowchaos.tome",
            "generated/physworld/maps/a/sample_lowchaos/sample.bphysworld",
        ];
        let entries: Vec<_> = paths.iter().map(|path| index_entry(path, 0, 0)).collect();
        write_index(&root.join("game1_patch.index"), &entries);
        for group in 2..=4 {
            write_index(&root.join(format!("game{group}_patch.index")), &[]);
        }
        let catalog = Catalog::open(&root).unwrap();
        assert!(LevelResources::resolve(&catalog, "sample_lowchaos").is_err());
        assert!(LevelResources::resolve(&catalog, "missing").is_err());
        let level = LevelResources::resolve(&catalog, paths[0]).unwrap();
        assert_eq!(level.entities, paths[2]);
        assert_eq!(level.tome, paths[3]);
        assert_eq!(level.physics.as_deref(), Some(paths[4]));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sorted_catalog_preserves_case_insensitive_lookup_and_later_group_override() {
        let root = std::env::temp_dir().join(format!(
            "dh2-catalog-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();

        fs::write(root.join("game1.resources"), b"first").unwrap();
        fs::write(root.join("game4.resources"), b"later").unwrap();
        write_index(
            &root.join("game1_patch.index"),
            &[
                index_entry("Thing/Asset.bin", 0, 5),
                typed_index_entry(
                    "image",
                    "tree$nomips.tga",
                    "generated/image/tree$nomips.tga.bimage7",
                    0,
                    5,
                ),
                typed_index_entry(
                    "image",
                    "ipr_constantcolor( 0, 0, 0, 1).imageprog",
                    "generated/image/ipr_constantcolor__0__0__0__1_.bimage7",
                    0,
                    5,
                ),
            ],
        );
        write_index(&root.join("game2_patch.index"), &[]);
        write_index(&root.join("game3_patch.index"), &[]);
        write_index(
            &root.join("game4_patch.index"),
            &[index_entry("thing/asset.bin", 0, 5)],
        );

        let catalog = Catalog::open(&root).unwrap();
        assert_eq!(catalog.len(), 3);
        assert!(catalog.contains("THING/ASSET.BIN"));
        assert!(catalog.contains("THING\\ASSET.BIN"));
        assert_eq!(
            catalog.image_resource("TREE$nomips.TGA"),
            Some("generated/image/tree$nomips.tga.bimage7")
        );
        assert_eq!(
            catalog.image_resource("ipr_constantColor( 0, 0, 0, 1).imageprog"),
            Some("generated/image/ipr_constantcolor__0__0__0__1_.bimage7")
        );
        assert_eq!(catalog.image_resource("tree.tga"), None);
        assert_eq!(
            catalog.image_program_resource("ipr_constantColor(0,0,0,1)"),
            Some("generated/image/ipr_constantcolor__0__0__0__1_.bimage7")
        );
        assert_ne!(
            image_program_key("ipr_test(\"a b\",1)"),
            image_program_key("ipr_test(\"ab\",1)")
        );
        assert_eq!(
            image_program_key("ipr_test(\"folder\\image.tga\",1)"),
            image_program_key("ipr_test(\"folder/image.tga\",1)")
        );
        assert_eq!(
            catalog.read("Thing/Asset.bin").unwrap(),
            Some(b"later".to_vec())
        );

        fs::write(root.join("game4.resources"), b"no").unwrap();
        assert!(catalog
            .read("thing/asset.bin")
            .unwrap_err()
            .to_string()
            .contains("archive extent"));

        drop(catalog);
        fs::remove_dir_all(root).unwrap();
    }
}
