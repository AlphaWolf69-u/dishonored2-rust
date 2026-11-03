//! Font records (type 0x16, `FUN_141072340`; metrics `FUN_14102ba20`).
use super::shape::{shape_body, Shape};
use super::timeline::kind;
use super::{IggyFile, Result};
use std::{cell::OnceCell, rc::Rc};

/// Name/style lookups read only the font header, never its outline table.
#[derive(Clone, Debug, PartialEq)]
pub struct FontMetadata {
    pub name: String,
    pub bold: bool,
    pub italic: bool,
}

/// Host decoding cache over the installed glyph table. The original provider
/// (14102bd60/14102ba20) reads that table in place; decoding every outline when
/// registering a library unnecessarily duplicates outlines never displayed.
#[derive(Clone, Debug)]
pub struct GlyphOutlines(Rc<OutlineStorage>);

#[derive(Debug)]
struct OutlineStorage {
    source: Option<(Rc<IggyFile>, usize)>,
    glyphs: Vec<OnceCell<Result<Shape>>>,
}

impl GlyphOutlines {
    pub fn len(&self) -> usize {
        self.0.glyphs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.glyphs.is_empty()
    }
    /// 14102ba20/LAB_14102bb10 read the first path's bounds directly. Text
    /// measurement must not decode all the points of an undrawn glyph.
    pub fn first_bounds(&self, index: usize) -> Option<[f32; 4]> {
        self.0.glyphs.get(index)?;
        if let Some((file, id)) = &self.0.source {
            let font = file.character(*id).ok()??;
            let glyph = font.ptr(0x30).ok()??.at(index * 0x20);
            if glyph.i16(8).ok()? <= 0 {
                return None;
            }
            let path = glyph.ptr(0x18).ok()??;
            Some([
                path.f32(0).ok()?,
                path.f32(4).ok()?,
                path.f32(8).ok()?,
                path.f32(12).ok()?,
            ])
        } else {
            self.get(index)?.layers.first().map(|layer| layer.bounds)
        }
    }
    pub fn get(&self, index: usize) -> Option<&Shape> {
        self.0
            .glyphs
            .get(index)?
            .get_or_init(|| {
                let _tag = crate::platform::allocation_tags::tag("font outlines");
                let (file, id) = self.0.source.as_ref().expect("eager glyph initialized");
                let record = file
                    .character(*id)?
                    .ok_or(super::Error::Corrupt("font record absent"))?;
                let glyphs = record
                    .ptr(0x30)?
                    .ok_or(super::Error::Corrupt("font outlines absent"))?;
                shape_body([0.; 4], glyphs.at(index * 0x20))
            })
            .as_ref()
            .ok()
    }
    pub fn decoded_count(&self) -> usize {
        self.0
            .glyphs
            .iter()
            .filter(|glyph| glyph.get().is_some())
            .count()
    }
}

impl std::ops::Index<usize> for GlyphOutlines {
    type Output = Shape;
    fn index(&self, index: usize) -> &Shape {
        self.get(index).expect("font glyph outline unavailable")
    }
}

impl PartialEq for GlyphOutlines {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || self.len() == other.len()
                && (0..self.len()).all(|index| self.get(index) == other.get(index))
    }
}

/// DefineFont3 metrics are stored in 1/20480 em units.
const EM_UNITS_V3: f32 = 1.0 / 20480.0;
const EM_UNITS: f32 = 1.0 / 1024.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub name: String,
    pub bold: bool,
    pub italic: bool,
    /// Em-relative ascent, descent and leading (+0x22/+0x24/+0x26).
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    /// +0x2a: the record has layout metrics; without them `FUN_14102ba20`
    /// uses the glyph union bounds.
    pub has_layout: bool,
    /// Union of the glyph bounds, em units (+0x50 ymin, +0x58 ymax).
    pub union_ymin: f32,
    pub union_ymax: f32,
    /// Sorted character codes; glyph `i` draws `codes[i]`.
    pub codes: Vec<u16>,
    /// Advance of each glyph in em units.
    pub advances: Vec<f32>,
    /// Glyph outlines in em units, y down.
    pub glyphs: GlyphOutlines,
    /// Kerning (code1, code2) -> em adjustment.
    pub kerning: Vec<(u16, u16, f32)>,
}

impl Font {
    /// Glyph index for a character (`FUN_14102b970`; U+00A0 maps to space
    /// in `FUN_14102c060`). A miss returns `None`; the player then uses the
    /// last glyph unless a fallback font is set.
    pub fn glyph(&self, c: u16) -> Option<usize> {
        let c = if c == 0xa0 { 0x20 } else { c };
        self.codes.binary_search(&c).ok()
    }

    pub fn kerning(&self, a: u16, b: u16) -> f32 {
        self.kerning
            .iter()
            .find(|k| k.0 == a && k.1 == b)
            .map_or(0.0, |k| k.2)
    }
}

impl IggyFile {
    pub fn font_metadata(&self, id: usize) -> Result<Option<FontMetadata>> {
        let Some(rec) = self.character(id)? else {
            return Ok(None);
        };
        if rec.u8(0)? != kind::FONT {
            return Ok(None);
        }
        Ok(Some(FontMetadata {
            name: rec
                .ptr(0x80)?
                .map(|n| n.wide_str())
                .transpose()?
                .unwrap_or_default(),
            bold: rec.u8(0x98)? != 0,
            italic: rec.u8(0x99)? != 0,
        }))
    }

    /// Share the library's source bytes and decode only requested glyphs.
    pub fn shared_font(self: &Rc<Self>, id: usize) -> Result<Option<Font>> {
        // Acquisition A/B uses the same player and immutable source bytes.
        if std::env::var("DH2_DEBUG_EAGER_FONT_OUTLINES").as_deref() == Ok("1") {
            self.read_font(id, None)
        } else {
            self.read_font(id, Some(self.clone()))
        }
    }

    pub fn font(&self, id: usize) -> Result<Option<Font>> {
        self.read_font(id, None)
    }

    fn read_font(&self, id: usize, source: Option<Rc<Self>>) -> Result<Option<Font>> {
        let _tag = crate::platform::allocation_tags::tag("font records");
        let Some(rec) = self.character(id)? else {
            return Ok(None);
        };
        if rec.u8(0)? != kind::FONT {
            return Ok(None);
        }
        let count = rec.u16(0x20)? as usize;
        let units = if rec.u8(0x28)? >= 3 {
            EM_UNITS_V3
        } else {
            EM_UNITS
        };
        let mut codes = Vec::with_capacity(count);
        let mut advances = Vec::with_capacity(count);
        let mut glyphs = Vec::with_capacity(count);
        if let Some(c) = rec.ptr(0x38)? {
            for i in 0..count {
                codes.push(c.u16(2 * i)?);
            }
        }
        if let Some(a) = rec.ptr(0x40)? {
            for i in 0..count {
                advances.push(a.f32(4 * i)?);
            }
        }
        if let Some(g) = rec.ptr(0x30)? {
            for i in 0..count {
                let cell = OnceCell::new();
                if source.is_none() {
                    let _tag = crate::platform::allocation_tags::tag("font outlines");
                    let _ = cell.set(Ok(shape_body([0.0; 4], g.at(0x20 * i))?));
                }
                glyphs.push(cell);
            }
        }
        let mut kerning = Vec::new();
        if let Some(k) = rec.ptr(0x60)? {
            for i in 0..rec.u32(0x48)? as usize {
                let e = k.at(6 * i);
                kerning.push((e.u16(0)?, e.u16(2)?, e.i16(4)? as f32 * units));
            }
        }
        Ok(Some(Font {
            name: rec
                .ptr(0x80)?
                .map(|n| n.wide_str())
                .transpose()?
                .unwrap_or_default(),
            bold: rec.u8(0x98)? != 0,
            italic: rec.u8(0x99)? != 0,
            ascent: rec.i16(0x22)? as f32 * units,
            descent: rec.i16(0x24)? as f32 * units,
            leading: rec.i16(0x26)? as f32 * units,
            has_layout: rec.u8(0x2a)? != 0,
            union_ymin: rec.f32(0x50)?,
            union_ymax: rec.f32(0x58)?,
            codes,
            advances,
            glyphs: GlyphOutlines(Rc::new(OutlineStorage {
                source: source.map(|file| (file, id)),
                glyphs,
            })),
            kerning,
        }))
    }
}

/// Edit-text flags (+0x30, `FUN_141071fb0`).
pub mod edit {
    pub const WORD_WRAP: u16 = 0x1;
    pub const MULTILINE: u16 = 0x2;
    pub const PASSWORD: u16 = 0x4;
    pub const READ_ONLY: u16 = 0x8;
    pub const AUTO_SIZE: u16 = 0x10;
    pub const NO_SELECT: u16 = 0x20;
    pub const BORDER: u16 = 0x40;
    pub const WAS_STATIC: u16 = 0x80;
    pub const HTML: u16 = 0x100;
    pub const USE_OUTLINES: u16 = 0x200;
}

/// An edit-text record (type 6, read by `FUN_141023c20`).
#[derive(Clone, Debug, PartialEq)]
pub struct EditText {
    pub bounds: [f32; 4],
    pub flags: u16,
    pub font_class: Option<String>,
    pub colour: [u8; 4],
    /// Font size in pixels (twips × 0.05).
    pub size: f32,
    pub max_chars: u16,
    pub left_margin: f32,
    pub right_margin: f32,
    pub leading: f32,
    /// 0 left, 1 right, 2 center, 3 justify.
    pub align: u8,
    pub text: Option<String>,
}

impl EditText {
    pub fn has(&self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

impl IggyFile {
    pub fn edit_text(&self, id: usize) -> Result<Option<EditText>> {
        let Some(rec) = self.character(id)? else {
            return Ok(None);
        };
        if rec.u8(0)? != kind::EDIT_TEXT {
            return Ok(None);
        }
        Ok(Some(EditText {
            bounds: [
                rec.f32(0x20)?,
                rec.f32(0x24)?,
                rec.f32(0x28)?,
                rec.f32(0x2c)?,
            ],
            flags: rec.u16(0x30)?,
            font_class: rec.ptr(0x38)?.map(|n| n.wide_str()).transpose()?,
            colour: rec.bytes(0x40, 4)?.try_into().unwrap(),
            size: rec.u16(0x44)? as f32 * 0.05,
            max_chars: rec.u16(0x46)?,
            left_margin: rec.u16(0x48)? as f32 * 0.05,
            right_margin: rec.u16(0x4a)? as f32 * 0.05,
            leading: rec.i16(0x4e)? as f32 * 0.05,
            align: rec.u8(0x50)?,
            text: rec.ptr(0x60)?.map(|n| n.wide_str()).transpose()?,
        }))
    }
}
