//! Read-only access to the installed game's resource indices and archives.
pub mod index;
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
}

pub struct Catalog {
    base: PathBuf,
    entries: Vec<(String, Entry)>,
    // Image source names are authoritative index aliases, including imageprog
    // expressions and option suffixes. Values index the final override table.
    image_sources: Vec<(String, usize)>,
    image_programs: Vec<(String, usize)>,
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
    fn entry(&self, path: &str) -> Option<&Entry> {
        let normalized = path.replace('\\', "/").to_ascii_lowercase();
        self.entries
            .binary_search_by(|(key, _)| key.as_str().cmp(&normalized))
            .ok()
            .map(|index| &self.entries[index].1)
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

use index::archive_path;
impl Catalog {
    pub fn open(base: &Path) -> Result<Self> {
        let mut entries = Vec::new();
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
                entries.push((
                    path,
                    Entry {
                        group,
                        offset,
                        size,
                        stored_size,
                        slot: archive_slot,
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
        Ok(Self {
            base: base.to_path_buf(),
            entries,
            image_sources,
            image_programs,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entry(path).is_some()
    }

    /// Decoded size recorded by the installed resource index.
    pub fn resource_size(&self, path: &str) -> Option<usize> {
        self.entry(path).map(|entry| entry.size as usize)
    }

    /// Resolve the source->compiled resource mapping stored by the game itself.
    pub fn image_resource(&self, source: &str) -> Option<&str> {
        // Native idStr slash normalization140140da0/140140de0 treats both
        // Windows separators alike before resource/image-source lookup.
        let source = source.replace('\\', "/").to_ascii_lowercase();
        let index = self
            .image_sources
            .binary_search_by(|(key, _)| key.cmp(&source))
            .ok()?;
        Some(&self.entries[self.image_sources[index].1].0)
    }

    /// Resolve any compiled image program through authoritative index aliases;
    /// no guessed compiled filename or replacement procedural image is used.
    pub fn image_program_resource(&self, source: &str) -> Option<&str> {
        let key = image_program_key(source);
        let i = self.image_programs.partition_point(|(s, _)| s < &key);
        let (found, index) = self.image_programs.get(i)?;
        if found != &key
            || self
                .image_programs
                .get(i + 1)
                .is_some_and(|(s, _)| s == &key)
        {
            return None;
        }
        Some(&self.entries[*index].0)
    }

    pub fn paths_with_suffix(&self, suffix: &str) -> Vec<String> {
        let suffix = suffix.to_ascii_lowercase();
        let mut paths: Vec<_> = self
            .entries
            .iter()
            .map(|(path, _)| path)
            .filter(|path| path.ends_with(&suffix))
            .cloned()
            .collect();
        paths.sort();
        paths
    }

    pub fn paths_containing(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_ascii_lowercase();
        let mut paths: Vec<_> = self
            .entries
            .iter()
            .map(|(path, _)| path)
            .filter(|path| path.contains(&needle))
            .cloned()
            .collect();
        paths.sort();
        paths
    }

    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.entry(path)
            .map(|entry| read_location(&self.base, path, entry))
            .transpose()
    }

    /// Read only the start of an installed resource. This is useful for large
    /// indexed files whose header tables precede a much larger payload.
    pub fn read_prefix(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.entry(path) else {
            return Ok(None);
        };
        let archive = archive_path(&self.base, entry.group, entry.slot)?;
        let mut file = File::open(archive)?;
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
    if entry
        .offset
        .checked_add(u64::from(entry.stored_size))
        .is_none_or(|end| end > archive_size)
    {
        return Err(invalid(format!("{path}: indexed resource exceeds archive extent")).into());
    }
    file.seek(SeekFrom::Start(entry.offset))?;
    // Native worlds exceed 128 MiB. Bound reads by the indexed archive
    // extent and decoded size, rather than an unrelated texture-size cap.
    // Stream decompression so loading a world does not retain a second
    // allocation for its entire compressed representation.
    let input = file.take(u64::from(entry.stored_size));
    let mut output = Vec::new();
    output.try_reserve((entry.size as usize).min(64 * 1024))?;
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
