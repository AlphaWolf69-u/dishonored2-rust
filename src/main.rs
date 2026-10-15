mod entities;
mod material_surface;
mod world_mesh;

use flate2::read::ZlibDecoder;
use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct Entry {
    group: u8,
    id: u32,
    kind: String,
    source: String,
    path: String,
    offset: u64,
    size: u32,
    stored_size: u32,
    archive_slot: u16,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn number<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn be_u16(input: &mut impl Read) -> io::Result<u16> {
    Ok(u16::from_be_bytes(number(input)?))
}

fn be_u32(input: &mut impl Read) -> io::Result<u32> {
    Ok(u32::from_be_bytes(number(input)?))
}

fn le_u32(input: &mut impl Read) -> io::Result<u32> {
    Ok(u32::from_le_bytes(number(input)?))
}

fn be_u64(input: &mut impl Read) -> io::Result<u64> {
    Ok(u64::from_be_bytes(number(input)?))
}

fn name(input: &mut impl Read) -> io::Result<String> {
    let length = le_u32(input)? as usize;
    if length > 65_536 {
        return Err(invalid(format!("unreasonable string length: {length}")));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|error| invalid(error.to_string()))
}

fn parse_index(input: &mut impl Read, group: u8) -> io::Result<(u32, Vec<Entry>)> {
    if number::<4>(input)? != *b"\x05SER" {
        return Err(invalid("wrong index signature"));
    }
    let payload_size = be_u32(input)?;
    let _reserved = number::<24>(input)?;
    let count = be_u32(input)?;
    if count > 1_000_000 {
        return Err(invalid(format!("unreasonable record count: {count}")));
    }
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = be_u32(input)?;
        let kind = name(input)?;
        let source = name(input)?;
        let path = name(input)?;
        let offset = be_u64(input)?;
        let size = be_u32(input)?;
        let stored_size = be_u32(input)?;
        let _unknown = number::<6>(input)?;
        let _flags = be_u16(input)?;
        let archive_slot = be_u16(input)? >> 2;
        entries.push(Entry {
            group,
            id,
            kind,
            source,
            path,
            offset,
            size,
            stored_size,
            archive_slot,
        });
    }
    Ok((payload_size, entries))
}

fn archive_path(base: &Path, group: u8, slot: u16) -> Result<PathBuf> {
    let filename = match slot {
        0 => format!("game{group}.resources"),
        1..=5 => format!("game{group}_{slot:03}.resources"),
        6 => format!("game{group}_patch.resources"),
        8192 if group == 2 || group == 3 => "shared_2_3.sharedrsc".to_string(),
        _ => return Err(invalid(format!("unknown resource slot {slot} for game{group}")).into()),
    };
    Ok(base.join(filename))
}

fn load_group(base: &Path, group: u8) -> Result<Vec<Entry>> {
    let path = base.join(format!("game{group}_patch.index"));
    let mut input = BufReader::new(File::open(&path)?);
    let (payload_size, entries) = parse_index(&mut input, group)?;
    let consumed = input.stream_position()?;
    let actual = fs::metadata(&path)?.len();
    if consumed != actual || actual.checked_sub(32) != Some(payload_size as u64) {
        return Err(invalid(format!(
            "{}: header says {} payload bytes; file has {actual} and parser read {consumed}",
            path.display(),
            payload_size
        ))
        .into());
    }
    Ok(entries)
}

fn load_catalog(base: &Path) -> Result<Vec<Entry>> {
    let mut all = Vec::new();
    for group in 1..=4 {
        all.extend(load_group(base, group)?);
    }
    Ok(all)
}

fn check_bounds(base: &Path, entries: &[Entry]) -> Result<()> {
    let mut lengths = BTreeMap::new();
    for entry in entries {
        if entry.stored_size == 0 {
            continue;
        }
        let path = archive_path(base, entry.group, entry.archive_slot)?;
        let length = match lengths.get(&path) {
            Some(length) => *length,
            None => {
                let length = fs::metadata(&path)?.len();
                lengths.insert(path.clone(), length);
                length
            }
        };
        let end = entry
            .offset
            .checked_add(entry.stored_size as u64)
            .ok_or_else(|| invalid("resource offset overflow"))?;
        if end > length {
            return Err(invalid(format!(
                "out-of-bounds resource {} in {}",
                entry.path,
                path.display()
            ))
            .into());
        }
    }
    println!(
        "Validated {} records against {} archive files",
        entries.len(),
        lengths.len()
    );
    Ok(())
}

fn read_entry(base: &Path, entry: &Entry) -> Result<Vec<u8>> {
    // Full campaign worlds exceed 200 MiB; retain a bounded read for archives.
    const MAX_EXTRACT_BYTES: u32 = 512 * 1024 * 1024;
    if entry.size > MAX_EXTRACT_BYTES || entry.stored_size > MAX_EXTRACT_BYTES {
        return Err(invalid("resource exceeds the 512 MiB extraction limit").into());
    }
    let path = archive_path(base, entry.group, entry.archive_slot)?;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(entry.offset))?;
    let mut stored = vec![0; entry.stored_size as usize];
    file.read_exact(&mut stored)?;
    let data = if entry.size == entry.stored_size {
        stored
    } else {
        let decoder = ZlibDecoder::new(stored.as_slice());
        let mut output = Vec::new();
        decoder
            .take(entry.size as u64 + 1)
            .read_to_end(&mut output)?;
        output
    };
    if data.len() != entry.size as usize {
        return Err(invalid(format!(
            "{}: expected {} bytes, decoded {}",
            entry.path,
            entry.size,
            data.len()
        ))
        .into());
    }
    Ok(data)
}

fn decode_game_text(bytes: &[u8]) -> String {
    const WINDOWS_1252: [char; 32] = [
        '€', '\u{0081}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{008D}', 'Ž',
        '\u{008F}', '\u{0090}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{009D}', 'ž', 'Ÿ',
    ];
    let mut output = String::with_capacity(bytes.len());
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match std::str::from_utf8(remaining) {
            Ok(valid) => {
                output.push_str(valid);
                break;
            }
            Err(error) => {
                let valid_len = error.valid_up_to();
                output.push_str(std::str::from_utf8(&remaining[..valid_len]).unwrap());
                remaining = &remaining[valid_len..];
                let bad_len = error.error_len().unwrap_or(remaining.len());
                for &byte in &remaining[..bad_len] {
                    let character = if (0x80..=0x9f).contains(&byte) {
                        WINDOWS_1252[(byte - 0x80) as usize]
                    } else {
                        char::from(byte)
                    };
                    output.push(character);
                }
                remaining = &remaining[bad_len..];
            }
        }
    }
    output
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        return Err("Usage: dishonored2-tools <summary|verify|verify-levels|find|extract|inspect-level|inspect-world|export-world-obj> <Dishonored2/base> [query] [output]".into());
    }
    let base = Path::new(&args[2]);
    let entries = load_catalog(base)?;
    match args[1].as_str() {
        "summary" => {
            let mut by_group = [0usize; 4];
            let mut by_kind = BTreeMap::<&str, usize>::new();
            for entry in &entries {
                by_group[entry.group as usize - 1] += 1;
                *by_kind.entry(&entry.kind).or_default() += 1;
            }
            for (group, count) in by_group.iter().enumerate() {
                println!("game{}: {count} records", group + 1);
            }
            println!("{} resource types", by_kind.len());
            let mut ranked: Vec<_> = by_kind.into_iter().collect();
            ranked.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            for (kind, count) in ranked.into_iter().take(15) {
                println!("  {kind}: {count}");
            }
        }
        "verify" => check_bounds(base, &entries)?,
        "verify-levels" => {
            let mut checked = 0;
            for entry in entries
                .iter()
                .filter(|entry| entry.path.ends_with(".entities") && entry.size > 0)
            {
                let text = decode_game_text(&read_entry(base, entry)?);
                let document = entities::parse(&text)
                    .map_err(|error| invalid(format!("{}: {error}", entry.path)))?;
                if document.version != 6 {
                    return Err(invalid(format!(
                        "{}: unsupported version {}",
                        entry.path, document.version
                    ))
                    .into());
                }
                checked += 1;
            }
            println!("Parsed {checked} level definitions");
        }
        "find" => {
            let query = args
                .get(3)
                .ok_or("find requires a substring")?
                .to_lowercase();
            for entry in entries
                .iter()
                .filter(|entry| entry.path.to_lowercase().contains(&query))
                .take(100)
            {
                println!(
                    "game{} id={} {} {} [{} bytes; source={}]",
                    entry.group, entry.id, entry.kind, entry.path, entry.size, entry.source
                );
            }
        }
        "extract" => {
            let query = args
                .get(3)
                .ok_or("extract requires an exact resource path")?;
            let output = Path::new(args.get(4).ok_or("extract requires an output file")?);
            let matching: Vec<_> = entries
                .iter()
                .filter(|entry| entry.path.eq_ignore_ascii_case(query))
                .collect();
            let first = matching.first().ok_or("resource not indexed")?;
            let bytes = read_entry(base, first)?;
            for duplicate in &matching[1..] {
                if read_entry(base, duplicate)? != bytes {
                    return Err(format!(
                        "{} has {} different indexed copies; extraction is ambiguous",
                        query,
                        matching.len()
                    )
                    .into());
                }
            }
            let base = fs::canonicalize(base)?;
            let parent = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let destination = fs::canonicalize(parent)?
                .join(output.file_name().ok_or("output must be a file path")?);
            if destination.exists() || destination.starts_with(&base) {
                return Err("output must be a new file outside the game directory".into());
            }
            let mut file = File::create_new(&destination)?;
            file.write_all(&bytes)?;
            println!(
                "Extracted {} bytes to {}",
                bytes.len(),
                destination.display()
            );
        }
        "inspect-level" => {
            let query = args
                .get(3)
                .ok_or("inspect-level requires an exact .entities path")?;
            let matching: Vec<_> = entries
                .iter()
                .filter(|entry| entry.path.eq_ignore_ascii_case(query))
                .collect();
            if matching.len() != 1 {
                return Err(
                    format!("expected one matching level, found {}", matching.len()).into(),
                );
            }
            let bytes = read_entry(base, matching[0])?;
            let text = decode_game_text(&bytes);
            let document = entities::parse(&text).map_err(invalid)?;
            let mut kinds = BTreeMap::<&str, usize>::new();
            let mut positioned = 0;
            for definition in &document.definitions {
                *kinds.entry(&definition.kind).or_default() += 1;
                if entities::has_key(&definition.fields, "spawnPosition") {
                    positioned += 1;
                }
            }
            println!(
                "Version {}: {} definitions, {} with spawnPosition",
                document.version,
                document.definitions.len(),
                positioned
            );
            for (kind, count) in kinds {
                println!("  {kind}: {count}");
            }
        }
        "inspect-world" | "export-world-obj" => {
            let query = args
                .get(3)
                .ok_or("world command requires an exact .bwm path")?;
            let matching: Vec<_> = entries
                .iter()
                .filter(|entry| entry.path.eq_ignore_ascii_case(query))
                .collect();
            if matching.len() != 1 {
                return Err(
                    format!("expected one matching world, found {}", matching.len()).into(),
                );
            }
            let bytes = read_entry(base, matching[0])?;
            let world = world_mesh::parse(&bytes).map_err(invalid)?;
            let vertex_count: usize = world.meshes.iter().map(|mesh| mesh.positions.len()).sum();
            let triangle_count: usize =
                world.meshes.iter().map(|mesh| mesh.indices.len() / 3).sum();
            println!(
                "{} meshes, {} objects, {vertex_count} vertices, {triangle_count} triangles",
                world.meshes.len(),
                world.objects.len()
            );
            if args[1] == "inspect-world" {
                for object in world.objects.iter().take(8) {
                    println!(
                        "  mesh {} matrices {}..{} material {}",
                        object.mesh_index,
                        object.matrix_start,
                        object.matrix_end,
                        object.material_path
                    );
                }
            }
            if args[1] == "export-world-obj" {
                let output = Path::new(
                    args.get(4)
                        .ok_or("export-world-obj requires an output file")?,
                );
                let base = fs::canonicalize(base)?;
                let parent = output
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                let destination = fs::canonicalize(parent)?
                    .join(output.file_name().ok_or("output must be a file path")?);
                if destination.exists() || destination.starts_with(&base) {
                    return Err("output must be a new file outside the game directory".into());
                }
                let mut file = BufWriter::new(File::create_new(&destination)?);
                writeln!(
                    file,
                    "# Geometry extracted from user's Dishonored 2 installation"
                )?;
                let mut base_index = 1usize;
                for (number, mesh) in world.meshes.iter().enumerate() {
                    writeln!(file, "g world_mesh_{number}")?;
                    for [x, y, z] in &mesh.positions {
                        writeln!(file, "v {x} {y} {z}")?;
                    }
                    for face in mesh.indices.chunks_exact(3) {
                        writeln!(
                            file,
                            "f {} {} {}",
                            base_index + face[0] as usize,
                            base_index + face[1] as usize,
                            base_index + face[2] as usize
                        )?;
                    }
                    base_index += mesh.positions.len();
                }
                file.flush()?;
                println!("Wrote geometry to {}", destination.display());
            }
        }
        _ => return Err("unknown command".into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_wrong_signature() {
        let mut bytes = io::Cursor::new(b"BAD!".to_vec());
        assert!(parse_index(&mut bytes, 1).is_err());
    }

    #[test]
    fn archive_slots_are_restricted() {
        let base = Path::new("C:/game/base");
        assert!(archive_path(base, 2, 8192)
            .unwrap()
            .ends_with("shared_2_3.sharedrsc"));
        assert!(archive_path(base, 1, 8192).is_err());
        assert!(archive_path(base, 2, 7).is_err());
    }

    #[test]
    fn preserves_utf8_and_decodes_legacy_text() {
        assert_eq!(
            decode_game_text(b"caf\xc3\xa9 / \x93quote\x94"),
            "café / “quote”"
        );
    }
}
