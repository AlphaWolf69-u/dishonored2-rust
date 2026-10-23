//! The movie root record that `FUN_141028ea0` copies into a new player.
use super::{Error, IggyFile, Node, Result};

/// An imported library and the character slots it fills.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    /// The authored SWF name, e.g. `lib_main.swf`.
    pub library: String,
    /// Root import record +0x10. `FUN_141028ea0` compares it with a running
    /// entry index shared by all imports, so it is an end index, not a count.
    pub entries_end: i32,
}

/// Movie-level properties.
#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    /// Stage rectangle in pixels: x0, y0, x1, y1 (root +0x30..+0x3c; the
    /// player stores x1-x0 and y1-y0 as the stage size).
    pub stage: [i32; 4],
    /// Root +0x58; the player stores its reciprocal as the frame period.
    pub frame_rate: f32,
    /// Root +0x40.
    pub character_count: u32,
    pub imports: Vec<Import>,
    /// Root +0xa8; when set the player creates its AS3 VM.
    pub has_as3: bool,
}

/// Root field offsets, from the copies at the start of `FUN_141028ea0`.
mod root {
    pub const CHARACTERS: usize = 0x00;
    pub const STAGE: usize = 0x30;
    pub const CHARACTER_COUNT: usize = 0x40;
    pub const FRAME_RATE: usize = 0x58;
    pub const IMPORTS: usize = 0x70;
    pub const IMPORT_COUNT: usize = 0x80;
    pub const ABC: usize = 0x98;
    pub const HAS_AS3: usize = 0xa8;
    /// Import records are 0x18 bytes: name, entry list, entry count.
    pub const IMPORT_SIZE: usize = 0x18;
}

impl IggyFile {
    pub fn movie(&self) -> Result<Movie> {
        let r = self.root();
        let mut imports = Vec::new();
        let count = r.u64(root::IMPORT_COUNT)? as usize;
        if count > 0 {
            let table = r.ptr(root::IMPORTS)?.ok_or(Error::Corrupt("null import table"))?;
            for i in 0..count {
                let entry = table.at(i * root::IMPORT_SIZE);
                let name = entry.ptr(0)?.ok_or(Error::Corrupt("unnamed import"))?;
                imports.push(Import { library: name.wide_str()?, entries_end: entry.i32(0x10)? });
            }
        }
        Ok(Movie {
            stage: [
                r.i32(root::STAGE)?,
                r.i32(root::STAGE + 4)?,
                r.i32(root::STAGE + 8)?,
                r.i32(root::STAGE + 12)?,
            ],
            frame_rate: r.f32(root::FRAME_RATE)?,
            character_count: r.u32(root::CHARACTER_COUNT)?,
            imports,
            has_as3: r.u8(root::HAS_AS3)? != 0,
        })
    }

    /// Character record `index`, or `None` for a slot filled by an import.
    pub fn character(&self, index: usize) -> Result<Option<Node<'_>>> {
        let r = self.root();
        if index >= r.u32(root::CHARACTER_COUNT)? as usize {
            return Ok(None);
        }
        let table = r.ptr(root::CHARACTERS)?.ok_or(Error::Corrupt("null character table"))?;
        table.ptr(8 * index)
    }

    /// The movie's standard AVM2 ABC block (root +0x98 -> {u64, u32 size, bytes}).
    pub fn abc(&self) -> Result<Option<&[u8]>> {
        let r = self.root();
        if !r.is_ptr(root::ABC) {
            return Ok(None);
        }
        let Some(block) = r.ptr(root::ABC)? else {
            return Ok(None);
        };
        let size = block.u32(8)? as usize;
        block.bytes(12, size).map(Some)
    }
}
