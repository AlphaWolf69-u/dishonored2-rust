//! Original resource-index records shared by loading and diagnostic tools.
use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File},
    io::{self, BufReader, Read, Seek},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
#[derive(Debug, Default)]
pub struct IndexEntry {
    pub group: u8,
    pub id: u32,
    pub kind: String,
    pub source: String,
    pub path: String,
    pub offset: u64,
    pub size: u32,
    pub stored_size: u32,
    pub flags: u16,
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
    let mut value = String::new();
    read_name(input, &mut value)?;
    Ok(value)
}

fn read_name(input: &mut impl Read, value: &mut String) -> io::Result<()> {
    let length = le_u32(input)? as usize;
    if length > 65_536 {
        return Err(invalid(format!("unreasonable string length: {length}")));
    }
    let mut bytes = if value.capacity() == 0 {
        vec![0; length]
    } else {
        let mut bytes = std::mem::take(value).into_bytes();
        bytes.resize(length, 0);
        bytes
    };
    input.read_exact(&mut bytes)?;
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    *value = String::from_utf8(bytes).map_err(|error| invalid(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
fn parse_index(
    input: &mut (impl Read + Seek),
    group: u8,
    mut visit: impl FnMut(IndexEntry),
) -> io::Result<u32> {
    parse_selected_index(input, group, None, |record| visit(std::mem::take(record)))
}

fn skip_name(input: &mut (impl Read + Seek)) -> io::Result<()> {
    let length = le_u32(input)?;
    if length > 65_536 { return Err(invalid(format!("unreasonable string length: {length}"))); }
    input.seek_relative(i64::from(length))
}

fn parse_selected_index(
    input: &mut (impl Read + Seek), group: u8, ids: Option<&HashSet<u32>>,
    mut visit: impl FnMut(&mut IndexEntry),
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
    let mut record = IndexEntry::default();
    for _ in 0..count {
        let id = be_u32(input)?;
        if ids.is_some_and(|ids| !ids.contains(&id)) {
            skip_name(input)?;
            skip_name(input)?;
            skip_name(input)?;
            input.seek_relative(26)?;
            continue;
        }
        record.group = group;
        record.id = id;
        read_name(input, &mut record.kind)?;
        read_name(input, &mut record.source)?;
        read_name(input, &mut record.path)?;
        record.offset = be_u64(input)?;
        record.size = be_u32(input)?;
        record.stored_size = be_u32(input)?;
        let _unknown = number::<6>(input)?;
        record.flags = be_u16(input)?;
        record.archive_slot = be_u16(input)? >> 2;
        visit(&mut record);
    }
    Ok(payload_size)
}

/// Primary header order from master.index (140068c30, 1400658b0).
/// The existing host catalog reads the four installed gameN_patch indexes;
/// additional data archives do not define another primary header namespace.
pub(super) fn primary_header_groups(base: &Path) -> Result<Box<[u8]>> {
    let mut input = BufReader::new(File::open(base.join("master.index"))?);
    if &number::<4>(&mut input)?[1..] != b"SER" {
        return Err(invalid("wrong master resource index signature").into());
    }
    let _version = be_u16(&mut input)?;
    let count = be_u16(&mut input)?;
    let mut groups = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let index = name(&mut input)?.replace('\\', "/").to_ascii_lowercase();
        let _data = name(&mut input)?;
        let group = index.strip_prefix("game").and_then(|name| name.strip_suffix("_patch.index"))
            .and_then(|number| number.parse::<u8>().ok()).filter(|group| (1..=4).contains(group))
            .ok_or_else(|| invalid(format!("primary header index is not hosted by this catalog: {index}")))?;
        if groups.contains(&group) {
            return Err(invalid(format!("duplicate primary header group: {group}")).into());
        }
        groups.push(group);
    }
    Ok(groups.into_boxed_slice())
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
    visit_file(&base.join(format!("game{group}_patch.index")), group, visit)
}

/// The catalog takes only names that it retains. Remaining buffers stay
/// available to the next row in this same installed index.
pub(super) fn visit_group_records(
    base: &Path, group: u8, visit: impl FnMut(&mut IndexEntry),
) -> Result<()> {
    visit_file_records(&base.join(format!("game{group}_patch.index")), group, None, visit)
}

/// The index file whose records describe archive `slot` of `group`
/// (`archive_path`'s naming): its ids number that archive's resources.
pub fn slot_index_path(base: &Path, group: u8, slot: u16) -> Result<PathBuf> {
    let filename = match slot {
        0 => format!("game{group}.index"),
        1..=5 => format!("game{group}_{slot:03}.index"),
        6 => format!("game{group}_patch.index"),
        _ => return Err(invalid(format!("no index file for resource slot {slot} of game{group}")).into()),
    };
    Ok(base.join(filename))
}

pub fn visit_file(path: &Path, group: u8, visit: impl FnMut(IndexEntry)) -> Result<()> {
    visit_selected_file(path, group, None, visit)
}

/// Read only the requested id namespace without retaining or decoding names
/// for unrelated resources. Duplicate ids retain all localization variants.
pub fn visit_file_ids(path: &Path, group: u8, ids: &HashSet<u32>, visit: impl FnMut(IndexEntry)) -> Result<()> {
    visit_selected_file(path, group, Some(ids), visit)
}

fn visit_selected_file(path: &Path, group: u8, ids: Option<&HashSet<u32>>, mut visit: impl FnMut(IndexEntry)) -> Result<()> {
    visit_file_records(path, group, ids, |record| visit(std::mem::take(record)))
}

fn visit_file_records(path: &Path, group: u8, ids: Option<&HashSet<u32>>, visit: impl FnMut(&mut IndexEntry)) -> Result<()> {
    let mut input = BufReader::new(File::open(path)?);
    let payload_size = parse_selected_index(&mut input, group, ids, visit)?;
    let consumed = input.stream_position()?;
    let actual = fs::metadata(path)?.len();
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
            header_groups: 1 << (entry.group - 1),
            offset: entry.offset,
            size: entry.size,
            stored_size: entry.stored_size,
            slot: entry.archive_slot,
            kind: 0,
            flags: entry.flags,
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
