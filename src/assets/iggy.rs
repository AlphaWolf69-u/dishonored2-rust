//! RAD Iggy movie and library files (`.iggy`), as loaded by the installed
//! executable's `iggy_file_reader.c` (`FUN_141093380`).
//!
//! A file is a serialized image of Iggy's own runtime structures. A 32-byte
//! header precedes a section table. Data sections hold the structures; each
//! is followed by a relocation program (a type-zero section) that marks which
//! fields are self-relative 64-bit pointers. The reader keeps the original
//! bytes and resolves only the fields that program names as pointers.
use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;

mod movie;
pub mod shape;
pub mod timeline;
pub use movie::{Import, Movie};

/// `DAT_142422f14` / `DAT_142427354`.
pub const MAGIC: u32 = 0xed0a_6749;
/// `DAT_142427358`.
pub const VERSION: u32 = 0x900;
/// `DAT_142427350`: the pointer width this executable accepts.
const POINTER_BITS: u8 = 64;
/// The section kinds Iggy loads in this game: `IggyPlayerCreateFromMemory`
/// passes mask `0x102` to the reader (`FUN_141028ea0`).
const LOADED_KINDS: u64 = 0x102;
/// The first data section's kind; its first record is the movie root.
pub const ROOT_KIND: u8 = 1;
/// A serialized pointer with this value relocates to null.
const NULL_POINTER: u64 = 1;

/// Errors match the reader's return codes and the messages
/// `FUN_141028ea0` reports for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Code 1.
    Header,
    /// Codes 2 and 4.
    Version(u32),
    /// Code 3.
    PointerSize(u8),
    /// Codes 5 and 6.
    Corrupt(&'static str),
    /// Code 7.
    Endianness,
    /// A record access outside the decoded file. This is a rewrite check; the
    /// original reads unchecked memory.
    Range { section: usize, offset: usize },
    /// A field read as a pointer was not relocated by the file's program.
    NotPointer { section: usize, offset: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Header => {
                write!(f, "creating from .iggy: incorrect header (corrupt or wrong type of file)")
            }
            Error::Version(v) => write!(f, "creating from .iggy: unsupported version {v:#x}"),
            Error::PointerSize(bits) => write!(
                f,
                "creating from .iggy: pointer size in file ({bits}) does not match platform"
            ),
            Error::Corrupt(why) => write!(f, "creating from .iggy: corrupt file ({why})"),
            Error::Endianness => write!(
                f,
                "creating from .iggy: wrong endianness or bitfield packing and no fixup data in file"
            ),
            Error::Range { section, offset } => {
                write!(f, "iggy record access out of range: section {section} offset {offset:#x}")
            }
            Error::NotPointer { section, offset } => {
                write!(f, "iggy field is not a relocated pointer: section {section} offset {offset:#x}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// One loaded data section and the fields its relocation program marks.
#[derive(Clone, Debug)]
pub struct DataSection {
    pub kind: u8,
    range: Range<usize>,
    pointers: BTreeSet<usize>,
}

impl DataSection {
    pub fn len(&self) -> usize {
        self.range.len()
    }

    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }

    /// Section-relative offsets of relocated pointer fields.
    pub fn pointers(&self) -> impl Iterator<Item = usize> + '_ {
        self.pointers.iter().copied()
    }
}

/// A decoded `.iggy` file.
#[derive(Clone)]
pub struct IggyFile {
    bytes: Vec<u8>,
    sections: Vec<DataSection>,
}

impl fmt::Debug for IggyFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IggyFile")
            .field("len", &self.bytes.len())
            .field("sections", &self.sections)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct Header {
    little_endian: bool,
    native_bitfields: bool,
    sections: usize,
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn header(bytes: &[u8]) -> Result<Header> {
    if bytes.len() < 0x20 || read_u32(bytes, 0) != Some(MAGIC) {
        return Err(Error::Header);
    }
    let version = read_u32(bytes, 4).unwrap();
    if version != VERSION {
        return Err(Error::Version(version));
    }
    if bytes[9] != POINTER_BITS {
        return Err(Error::PointerSize(bytes[9]));
    }
    let little_endian = bytes[8] == 1;
    let native_bitfields = bytes[10] == 1;
    // Byte 11 says whether the relocation programs carry byte-swap and
    // bitfield fixups for a non-native file.
    if !(little_endian || bytes[11] & 1 != 0) || !(native_bitfields || bytes[11] & 2 != 0) {
        return Err(Error::Endianness);
    }
    Ok(Header {
        little_endian,
        native_bitfields,
        sections: read_u32(bytes, 0x1c).unwrap() as usize,
    })
}

/// One `(offset, kind)` entry of a structure descriptor after the reader's
/// platform filter (`FUN_141093380` before its main loop).
#[derive(Clone, Copy)]
struct Field {
    offset: u8,
    kind: u8,
}

struct Descriptor {
    size: u8,
    fields: Vec<Field>,
}

/// Field kinds within descriptors and inline records (`FUN_141092f60`,
/// `FUN_141093150`).
mod field {
    /// Pointer in file byte order; the reader rewrites it to `NATIVE_POINTER`
    /// or `SWAPPED_POINTER` according to the header.
    pub const POINTER: u8 = 2;
    pub const SWAPPED_POINTER: u8 = 0;
    pub const NATIVE_POINTER: u8 = 1;
    pub const BITFIELD: u8 = 3;
    pub const SWAP16: u8 = 4;
    pub const SWAP64: u8 = 6;
    /// Kinds 8-15 store runtime values supplied by the creating player.
    pub const INJECTED: std::ops::RangeInclusive<u8> = 8..=15;
}

impl Header {
    /// The descriptor filter keeps relocations this platform needs.
    fn filter(&self, kind: u8) -> Result<Option<u8>> {
        Ok(match kind {
            field::POINTER => Some(if self.little_endian {
                field::NATIVE_POINTER
            } else {
                field::SWAPPED_POINTER
            }),
            field::BITFIELD => (!self.native_bitfields).then_some(kind),
            field::SWAP16..=field::SWAP64 => (!self.little_endian).then_some(kind),
            k if field::INJECTED.contains(&k) => Some(k),
            _ => return Err(Error::Corrupt("descriptor field kind")),
        })
    }
}

/// Collects relocated pointer fields while walking one relocation program.
struct Relocation<'a> {
    header: Header,
    section: &'a [u8],
    pointers: BTreeSet<usize>,
}

impl Relocation<'_> {
    fn mark_pointer(&mut self, at: usize) -> Result<()> {
        let raw = read_u64(self.section, at).ok_or(Error::Corrupt("pointer outside section"))?;
        let raw = if self.header.little_endian { raw } else { raw.swap_bytes() };
        // Self-relative offsets must land inside the same allocation: the
        // reader copies each data section to its own buffer.
        if raw != NULL_POINTER && (at as u64).wrapping_add(raw) > self.section.len() as u64 {
            return Err(Error::Corrupt("pointer target outside section"));
        }
        self.pointers.insert(at);
        Ok(())
    }

    /// `FUN_141092f60` / `FUN_141093150`: apply one record layout at `cursor`.
    fn apply(&mut self, cursor: usize, size: u8, fields: &[Field], inline: bool) -> Result<usize> {
        for f in fields {
            let at = cursor + f.offset as usize;
            match f.kind {
                // Inline records still carry the unfiltered pointer kind.
                field::POINTER if inline => self.mark_pointer(at)?,
                field::NATIVE_POINTER | field::SWAPPED_POINTER if !inline => {
                    self.mark_pointer(at)?
                }
                field::BITFIELD..=field::SWAP64 => {}
                k if field::INJECTED.contains(&k) => {}
                _ => return Err(Error::Corrupt("record field kind")),
            }
        }
        Ok(cursor + size as usize)
    }

    /// Walk the program after its descriptor table (`FUN_141093380` main loop).
    fn run(&mut self, program: &[u8]) -> Result<()> {
        let count = *program.first().ok_or(Error::Corrupt("empty relocation program"))? as usize;
        let mut at = 1;
        let mut descriptors = Vec::with_capacity(count);
        for _ in 0..count {
            let size = *program.get(at).ok_or(Error::Corrupt("descriptor"))?;
            let n = *program.get(at + 1).ok_or(Error::Corrupt("descriptor"))? as usize;
            let pairs = program
                .get(at + 2..at + 2 + 2 * n)
                .ok_or(Error::Corrupt("descriptor fields"))?;
            let mut fields = Vec::with_capacity(n);
            for p in pairs.chunks_exact(2) {
                if let Some(kind) = self.header.filter(p[1])? {
                    fields.push(Field { offset: p[0], kind });
                }
            }
            descriptors.push(Descriptor { size, fields });
            at += 2 + 2 * n;
        }
        let byte = |i: usize| program.get(i).copied().ok_or(Error::Corrupt("truncated program"));
        let mut cursor = 0usize;
        while at < program.len() {
            let op = program[at];
            match op {
                0x00..=0x7f => {
                    let d = descriptors.get(op as usize).ok_or(Error::Corrupt("descriptor index"))?;
                    let (size, fields) = (d.size, d.fields.clone());
                    cursor = self.apply(cursor, size, &fields, false)?;
                    at += 1;
                }
                0x80..=0xbf => {
                    let d = descriptors
                        .get(byte(at + 1)? as usize)
                        .ok_or(Error::Corrupt("descriptor index"))?;
                    let (size, fields) = (d.size, d.fields.clone());
                    for _ in 0..op - 0x7f {
                        cursor = self.apply(cursor, size, &fields, false)?;
                    }
                    at += 2;
                }
                0xc0..=0xcf => {
                    cursor += (op as usize - 0xbf) * 2;
                    at += 1;
                }
                0xd0..=0xdf => {
                    let n = byte(at + 1)? as usize + 1;
                    match op & 0xf {
                        2 => {
                            for i in 0..n {
                                self.mark_pointer(cursor + 8 * i)?;
                            }
                            cursor += 8 * n;
                        }
                        4 => cursor += 2 * n,
                        5 => cursor += 4 * n,
                        6 => cursor += 8 * n,
                        _ => return Err(Error::Corrupt("run kind")),
                    }
                    at += 2;
                }
                0xfc => at += 2,
                0xfd => {
                    let size = byte(at + 1)?;
                    let n = byte(at + 2)? as usize;
                    let pairs = program
                        .get(at + 3..at + 3 + 2 * n)
                        .ok_or(Error::Corrupt("inline record"))?;
                    let fields: Vec<_> = pairs
                        .chunks_exact(2)
                        .map(|p| Field { offset: p[0], kind: p[1] })
                        .collect();
                    if fields.iter().any(|f| {
                        !matches!(f.kind, field::POINTER..=field::SWAP64)
                            && !field::INJECTED.contains(&f.kind)
                    }) {
                        return Err(Error::Corrupt("inline record field kind"));
                    }
                    cursor = self.apply(cursor, size, &fields, true)?;
                    at += 3 + 2 * n;
                }
                0xfe => {
                    cursor += byte(at + 1)? as usize + 1;
                    at += 2;
                }
                0xff => {
                    let skip = read_u32(program, at + 1).ok_or(Error::Corrupt("truncated skip"))?;
                    cursor += skip as usize;
                    at += 5;
                }
                _ => return Err(Error::Corrupt("relocation opcode")),
            }
        }
        Ok(())
    }
}

impl IggyFile {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let header = header(&bytes)?;
        let table_end = 0x20 + 16 * header.sections;
        if bytes.len() < table_end {
            return Err(Error::Corrupt("section table"));
        }
        let mut sections: Vec<DataSection> = Vec::new();
        for i in 0..header.sections {
            let entry = 0x20 + 16 * i;
            let kind = bytes[entry];
            let target = bytes[entry + 2] as usize;
            let size = read_u32(&bytes, entry + 4).unwrap() as usize;
            let offset = read_u32(&bytes, entry + 12).unwrap() as usize;
            let range = offset..offset.checked_add(size).ok_or(Error::Corrupt("section size"))?;
            if range.end > bytes.len() {
                return Err(Error::Corrupt("section outside file"));
            }
            if kind != 0 && LOADED_KINDS >> (kind & 0x3f) & 1 == 0 {
                continue;
            }
            if kind != 0 {
                sections.push(DataSection { kind, range, pointers: BTreeSet::new() });
                continue;
            }
            let data = sections.get(target).ok_or(Error::Corrupt("relocation target"))?;
            let mut relocation = Relocation {
                header,
                section: &bytes[data.range.clone()],
                pointers: BTreeSet::new(),
            };
            relocation.run(&bytes[range])?;
            let pointers = relocation.pointers;
            sections[target].pointers.extend(pointers);
        }
        if sections.first().map(|s| s.kind) != Some(ROOT_KIND) {
            return Err(Error::Corrupt("first section is not the movie root"));
        }
        Ok(Self { bytes, sections })
    }

    pub fn sections(&self) -> &[DataSection] {
        &self.sections
    }

    /// The movie root record at the start of the first section.
    pub fn root(&self) -> Node<'_> {
        Node { file: self, section: 0, offset: 0 }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The first loaded data section of `kind`.
    pub fn section_of_kind(&self, kind: u8) -> Option<Node<'_>> {
        let section = self.sections.iter().position(|s| s.kind == kind)?;
        Some(Node { file: self, section, offset: 0 })
    }
}

/// A position inside one data section.
#[derive(Clone, Copy)]
pub struct Node<'a> {
    file: &'a IggyFile,
    section: usize,
    offset: usize,
}

impl fmt::Debug for Node<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({}:{:#x})", self.section, self.offset)
    }
}

impl<'a> Node<'a> {
    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn file(&self) -> &'a IggyFile {
        self.file
    }

    pub fn at(&self, delta: usize) -> Node<'a> {
        Node { offset: self.offset + delta, ..*self }
    }

    pub fn bytes(&self, delta: usize, len: usize) -> Result<&'a [u8]> {
        let section = &self.file.sections[self.section];
        let start = self.offset + delta;
        let error = Error::Range { section: self.section, offset: start };
        if start.checked_add(len).is_none_or(|end| end > section.len()) {
            return Err(error);
        }
        let start = section.range.start + start;
        Ok(&self.file.bytes[start..start + len])
    }

    pub fn u8(&self, delta: usize) -> Result<u8> {
        Ok(self.bytes(delta, 1)?[0])
    }

    pub fn u16(&self, delta: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(delta, 2)?.try_into().unwrap()))
    }

    pub fn i16(&self, delta: usize) -> Result<i16> {
        Ok(self.u16(delta)? as i16)
    }

    pub fn u32(&self, delta: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(delta, 4)?.try_into().unwrap()))
    }

    pub fn i32(&self, delta: usize) -> Result<i32> {
        Ok(self.u32(delta)? as i32)
    }

    pub fn u64(&self, delta: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(delta, 8)?.try_into().unwrap()))
    }

    pub fn f32(&self, delta: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32(delta)?))
    }

    /// Follow a relocated pointer field; `None` is the serialized null.
    pub fn ptr(&self, delta: usize) -> Result<Option<Node<'a>>> {
        let at = self.offset + delta;
        if !self.file.sections[self.section].pointers.contains(&at) {
            return Err(Error::NotPointer { section: self.section, offset: at });
        }
        let raw = self.u64(delta)?;
        Ok((raw != NULL_POINTER).then(|| Node {
            offset: at.wrapping_add(raw as usize),
            ..*self
        }))
    }

    /// Whether the relocation program marks this field as a pointer.
    pub fn is_ptr(&self, delta: usize) -> bool {
        self.file.sections[self.section].pointers.contains(&(self.offset + delta))
    }

    /// A NUL-terminated UTF-16LE string, as Iggy stores library names.
    pub fn wide_str(&self) -> Result<String> {
        let mut units = Vec::new();
        loop {
            let unit = self.u16(2 * units.len())?;
            if unit == 0 {
                return Ok(String::from_utf16_lossy(&units));
            }
            units.push(unit);
        }
    }

    /// A NUL-terminated byte string.
    pub fn c_str(&self) -> Result<String> {
        let mut out = Vec::new();
        loop {
            let b = self.u8(out.len())?;
            if b == 0 {
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            out.push(b);
        }
    }
}

#[cfg(test)]
mod tests;
