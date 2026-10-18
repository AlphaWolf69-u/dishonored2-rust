use dishonored2_rust::entities;
use dishonored2_rust::world_mesh;

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

use dishonored2_rust::archive::index::{
    archive_path, load_catalog, read_entry, IndexEntry as Entry,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
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
