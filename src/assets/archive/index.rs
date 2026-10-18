//! Original resource-index records shared by loading and diagnostic tools.
use std::{
    error::Error,
    fs::{self, File},
    io::{self, BufReader, Read, Seek},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Debug)]
pub struct IndexEntry {
    pub group: u8,
    pub id: u32,
    pub kind: String,
    pub source: String,
    pub path: String,
    pub offset: u64,
    pub size: u32,
    pub stored_size: u32,
    pub archive_slot: u16,
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

fn parse_index(
    input: &mut impl Read,
    group: u8,
    mut visit: impl FnMut(IndexEntry),
) -> io::Result<u32> {
    if number::<4>(input)? != *b"\x05SER" {
        return Err(invalid("wrong index signature"));
    }
    let payload_size = be_u32(input)?;
    let _reserved = number::<24>(input)?;
    let count = be_u32(input)?;
    if count > 1_000_000 {
        return Err(invalid(format!("unreasonable record count: {count}")));
    }
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
        visit(IndexEntry {
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
    Ok(payload_size)
}

pub fn archive_path(base: &Path, group: u8, slot: u16) -> Result<PathBuf> {
    let filename = match slot {
        0 => format!("game{group}.resources"),
        1..=5 => format!("game{group}_{slot:03}.resources"),
        6 => format!("game{group}_patch.resources"),
        8192 if group == 2 || group == 3 => "shared_2_3.sharedrsc".to_string(),
        _ => return Err(invalid(format!("unknown resource slot {slot} for game{group}")).into()),
    };
    Ok(base.join(filename))
}

pub fn visit_group(base: &Path, group: u8, visit: impl FnMut(IndexEntry)) -> Result<()> {
    let path = base.join(format!("game{group}_patch.index"));
    let mut input = BufReader::new(File::open(&path)?);
    let payload_size = parse_index(&mut input, group, visit)?;
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
    Ok(())
}

pub fn load_catalog(base: &Path) -> Result<Vec<IndexEntry>> {
    let mut all = Vec::new();
    for group in 1..=4 {
        visit_group(base, group, |entry| all.push(entry))?;
    }
    Ok(all)
}

/// Read one exact index record (diagnostics may inspect overridden entries).
pub fn read_entry(base: &Path, entry: &IndexEntry) -> Result<Vec<u8>> {
    super::read_location(
        base,
        &entry.path,
        &super::Entry {
            group: entry.group,
            offset: entry.offset,
            size: entry.size,
            stored_size: entry.stored_size,
            slot: entry.archive_slot,
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_wrong_signature() {
        let mut bytes = std::io::Cursor::new(b"BAD!".to_vec());
        assert!(super::parse_index(&mut bytes, 1, |_| {}).is_err());
    }
}
