//! Read-only access to the installed game's resource indices and archives.
use flate2::read::ZlibDecoder;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
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
}

impl Catalog {
    fn entry(&self, path: &str) -> Option<&Entry> {
        let normalized = path.to_ascii_lowercase();
        self.entries
            .binary_search_by(|(key, _)| key.as_str().cmp(&normalized))
            .ok()
            .map(|index| &self.entries[index].1)
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn bytes<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut data = [0; N];
    input.read_exact(&mut data)?;
    Ok(data)
}

fn be32(input: &mut impl Read) -> io::Result<u32> {
    Ok(u32::from_be_bytes(bytes(input)?))
}
fn be64(input: &mut impl Read) -> io::Result<u64> {
    Ok(u64::from_be_bytes(bytes(input)?))
}
fn be16(input: &mut impl Read) -> io::Result<u16> {
    Ok(u16::from_be_bytes(bytes(input)?))
}

fn name(input: &mut impl Read) -> io::Result<String> {
    let length = u32::from_le_bytes(bytes(input)?) as usize;
    if length > 65_536 {
        return Err(invalid("unreasonable resource name length"));
    }
    let mut data = vec![0; length];
    input.read_exact(&mut data)?;
    if data.last() == Some(&0) {
        data.pop();
    }
    String::from_utf8(data).map_err(|error| invalid(error.to_string()))
}

fn archive_path(base: &Path, group: u8, slot: u16) -> Result<PathBuf> {
    let filename = match slot {
        0 => format!("game{group}.resources"),
        1..=5 => format!("game{group}_{slot:03}.resources"),
        6 => format!("game{group}_patch.resources"),
        8192 if group == 2 || group == 3 => "shared_2_3.sharedrsc".to_string(),
        _ => return Err(invalid(format!("unsupported resource slot {slot}")).into()),
    };
    Ok(base.join(filename))
}

impl Catalog {
    pub fn open(base: &Path) -> Result<Self> {
        let mut entries = Vec::new();
        let mut image_sources = Vec::new();
        for group in 1..=4 {
            let index_path = base.join(format!("game{group}_patch.index"));
            let mut input = BufReader::new(File::open(&index_path)?);
            if bytes::<4>(&mut input)? != *b"\x05SER" {
                return Err(invalid("wrong resource index signature").into());
            }
            let payload_size = be32(&mut input)?;
            let _reserved = bytes::<24>(&mut input)?;
            let count = be32(&mut input)?;
            if count > 1_000_000 {
                return Err(invalid("unreasonable resource count").into());
            }
            // Each group header gives an upper bound on the number of entries.
            // Reserve before parsing names to avoid repeated vector growth.
            entries.reserve(count as usize);
            for _ in 0..count {
                let _id = be32(&mut input)?;
                let kind = name(&mut input)?;
                let mut source = name(&mut input)?;
                let mut path = name(&mut input)?;
                // `name` already owns this buffer. Normalize it in place
                // instead of allocating a second lowercase String per entry.
                path.make_ascii_lowercase();
                if kind.eq_ignore_ascii_case("image") && path.ends_with(".bimage7") {
                    source.make_ascii_lowercase();
                    image_sources.push((source, path.clone()));
                }
                let offset = be64(&mut input)?;
                let size = be32(&mut input)?;
                let stored_size = be32(&mut input)?;
                let _unknown = bytes::<6>(&mut input)?;
                let _flags = be16(&mut input)?;
                let slot = be16(&mut input)? >> 2;
                entries.push((
                    path,
                    Entry {
                        group,
                        offset,
                        size,
                        stored_size,
                        slot,
                    },
                ));
            }
            let length = fs::metadata(&index_path)?.len();
            if input.stream_position()? != length
                || length.checked_sub(32) != Some(payload_size as u64)
            {
                return Err(
                    invalid(format!("invalid index extent: {}", index_path.display())).into(),
                );
            }
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
        let mut image_sources: Vec<_> = image_sources.into_iter().map(|(source, path)| {
            let index = entries.binary_search_by(|(key, _)| key.cmp(&path)).unwrap();
            (source, index)
        }).collect();
        image_sources.sort_by(|left, right| left.0.cmp(&right.0));
        image_sources.dedup_by(|later, earlier| {
            if later.0 == earlier.0 { earlier.1 = later.1; true } else { false }
        });
        Ok(Self {
            base: base.to_path_buf(),
            entries,
            image_sources,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entry(path).is_some()
    }

    /// Resolve the source->compiled resource mapping stored by the game itself.
    pub fn image_resource(&self, source: &str) -> Option<&str> {
        let source = source.to_ascii_lowercase();
        let index = self.image_sources.binary_search_by(|(key, _)| key.cmp(&source)).ok()?;
        Some(&self.entries[self.image_sources[index].1].0)
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
        let Some(entry) = self.entry(path) else {
            return Ok(None);
        };
        const MAX_BYTES: u32 = 128 * 1024 * 1024;
        if entry.size > MAX_BYTES || entry.stored_size > MAX_BYTES {
            return Err(invalid("resource exceeds 128 MiB limit").into());
        }
        let archive = archive_path(&self.base, entry.group, entry.slot)?;
        let mut file = File::open(&archive)?;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut compressed = vec![0; entry.stored_size as usize];
        file.read_exact(&mut compressed)?;
        let output = if entry.size == entry.stored_size {
            compressed
        } else {
            let mut output = Vec::with_capacity(entry.size as usize);
            ZlibDecoder::new(compressed.as_slice())
                .take(entry.size as u64 + 1)
                .read_to_end(&mut output)?;
            output
        };
        if output.len() != entry.size as usize {
            return Err(invalid(format!(
                "{}: expected {} bytes, decoded {}",
                path,
                entry.size,
                output.len()
            ))
            .into());
        }
        Ok(Some(output))
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

#[cfg(test)]
mod tests {
    use super::*;
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
                typed_index_entry("image", "tree$nomips.tga", "generated/image/tree$nomips.tga.bimage7", 0, 5),
                typed_index_entry("image", "ipr_constantcolor( 0, 0, 0, 1).imageprog", "generated/image/ipr_constantcolor__0__0__0__1_.bimage7", 0, 5),
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
        assert_eq!(catalog.image_resource("TREE$nomips.TGA"), Some("generated/image/tree$nomips.tga.bimage7"));
        assert_eq!(catalog.image_resource("ipr_constantColor( 0, 0, 0, 1).imageprog"), Some("generated/image/ipr_constantcolor__0__0__0__1_.bimage7"));
        assert_eq!(catalog.image_resource("tree.tga"), None);
        assert_eq!(
            catalog.read("Thing/Asset.bin").unwrap(),
            Some(b"later".to_vec())
        );

        drop(catalog);
        fs::remove_dir_all(root).unwrap();
    }
}
