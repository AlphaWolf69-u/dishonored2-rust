//! Shape (type 1) and bitmap (type 4) records and the `.raw.iggytex` index.
//!
//! Shapes are SWF-style edge paths that the player triangulates at runtime
//! (`FUN_14103d990`, swf_draw_triangulate.c); fills follow `FUN_141089760`.
use super::timeline::kind;
use super::{Error, IggyFile, Node, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    MoveTo,
    LineTo,
    CurveTo,
}

/// A path point (0x18 bytes, `FUN_1410aaff0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub x: f32,
    pub y: f32,
    /// Quadratic control point (`CurveTo` only).
    pub cx: f32,
    pub cy: f32,
    pub segment: Segment,
    /// Fill on each side of the edge: 0 none, k = fill element k-1.
    pub fill0: u16,
    pub fill1: u16,
    /// 0 none, k = line element k-1.
    pub line: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spread {
    Pad,
    Reflect,
    Repeat,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub spread: Spread,
    /// Interpolate through the linear-space table (`DAT_143eb6980`).
    pub linear_space: bool,
    pub focal: f32,
    /// RGBA colour and ratio (0..255).
    pub stops: Vec<([u8; 4], u8)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid([u8; 4]),
    /// Fill-space to shape-space matrix `m0..m5`
    /// (x' = m0·x + m1·y + m4, y' = m2·x + m3·y + m5).
    Linear { matrix: [f32; 6], gradient: Gradient },
    Radial { matrix: [f32; 6], gradient: Gradient },
    Focal { matrix: [f32; 6], gradient: Gradient },
    Bitmap { matrix: [f32; 6], bitmap: Option<u16>, repeat: bool, smooth: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub fill: Fill,
    pub width: f32,
    /// +0x48 multiplier applied to the half width (`FUN_1410a7610`).
    pub multiplier: f32,
    /// +0x57 scale-mode selector bits.
    pub scale_mode: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub bounds: [f32; 4],
    pub points: Vec<PathPoint>,
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    pub fill_bounds: Vec<[f32; 4]>,
    /// Entry +0x38: strokes are drawn only when this is false.
    pub no_strokes: bool,
}

/// A shape or glyph: layers drawn in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub bounds: [f32; 4],
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    /// +0x24 == 4: drawn with blending (`FUN_141093df0`).
    pub blended: bool,
    /// u scale, v scale, u offset, v offset.
    pub uv: [f32; 4],
}

/// One `IggyFileTextureRaw` entry of the type-8 section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureEntry {
    pub offset: u32,
    /// GDraw raw format (13 = BC7_UNORM in every installed file).
    pub format: u8,
    pub mips: u8,
    pub width: u16,
    pub height: u16,
    pub character: u16,
}

impl TextureEntry {
    /// Bytes per block and block edge for a raw format (`FUN_1402e2c00`).
    pub fn block(format: u8) -> Option<(usize, usize)> {
        Some(match format {
            0 => (4, 1),
            3 | 4 => (2, 1),
            5..=7 => (1, 1),
            9 | 12 => (8, 4),
            10 | 11 | 13 => (16, 4),
            _ => return None,
        })
    }

    /// Byte size of all mips, laid out contiguously.
    pub fn byte_len(&self) -> Option<usize> {
        let (bytes, edge) = Self::block(self.format)?;
        Some(
            (0..self.mips.max(1) as u32)
                .map(|m| {
                    let w = (self.width as usize >> m).max(1);
                    let h = (self.height as usize >> m).max(1);
                    w.div_ceil(edge) * h.div_ceil(edge) * bytes
                })
                .sum(),
        )
    }
}

fn f32x4(n: &Node, at: usize) -> Result<[f32; 4]> {
    Ok([n.f32(at)?, n.f32(at + 4)?, n.f32(at + 8)?, n.f32(at + 12)?])
}

fn matrix(n: &Node, at: usize) -> Result<[f32; 6]> {
    let mut m = [0f32; 6];
    for (i, v) in m.iter_mut().enumerate() {
        *v = n.f32(at + 4 * i)?;
    }
    Ok(m)
}

fn gradient(n: Node) -> Result<Gradient> {
    let count = n.u8(2)? as usize;
    let stops = (0..count)
        .map(|i| {
            let s = n.at(0xc + 10 * i);
            Ok((s.bytes(0, 4)?.try_into().unwrap(), s.u8(8)?))
        })
        .collect::<Result<_>>()?;
    Ok(Gradient {
        spread: match n.u8(0)? {
            1 => Spread::Reflect,
            2 => Spread::Repeat,
            _ => Spread::Pad,
        },
        linear_space: n.u8(1)? != 0,
        focal: n.f32(4)?,
        stops,
    })
}

fn fill(n: Node) -> Result<Fill> {
    let t = n.u8(0)?;
    Ok(match t {
        0x00 => Fill::Solid(n.bytes(0x10, 4)?.try_into().unwrap()),
        0x10 | 0x12 | 0x13 => {
            let g = gradient(n.ptr(0x40)?.ok_or(Error::Corrupt("gradient fill without record"))?)?;
            let m = matrix(&n, 0x10)?;
            match t {
                0x10 => Fill::Linear { matrix: m, gradient: g },
                0x12 => Fill::Radial { matrix: m, gradient: g },
                _ => Fill::Focal { matrix: m, gradient: g },
            }
        }
        0x40..=0x43 => Fill::Bitmap {
            matrix: matrix(&n, 0x10)?,
            bitmap: n.ptr(0x40)?.map(|b| b.u16(2)).transpose()?,
            repeat: matches!(t, 0x40 | 0x42),
            // 0x42/0x43 request point sampling.
            smooth: matches!(t, 0x40 | 0x41),
        },
        _ => return Err(Error::Corrupt("fill type")),
    })
}

fn list<T>(n: Option<Node>, stride: usize, read: impl Fn(Node) -> Result<T>) -> Result<Vec<T>> {
    let Some(n) = n else { return Ok(Vec::new()) };
    let count = n.i32(0)?.max(0) as usize;
    (0..count).map(|i| read(n.at(8 + stride * i))).collect()
}

fn layer(n: Node) -> Result<Layer> {
    let count = n.i32(0x18)?.max(0) as usize;
    let mut points = Vec::with_capacity(count);
    if let Some(p) = n.ptr(0x10)? {
        for i in 0..count {
            let q = p.at(0x18 * i);
            points.push(PathPoint {
                x: q.f32(0)?,
                y: q.f32(4)?,
                cx: q.f32(8)?,
                cy: q.f32(0xc)?,
                segment: match q.u8(0x10)? {
                    1 => Segment::MoveTo,
                    2 => Segment::LineTo,
                    3 => Segment::CurveTo,
                    _ => return Err(Error::Corrupt("path segment")),
                },
                fill0: q.u16(0x12)?,
                fill1: q.u16(0x14)?,
                line: q.u16(0x16)?,
            });
        }
    }
    let fills = list(n.ptr(0x20)?, 0x48, fill)?;
    let lines = list(n.ptr(0x28)?, 0x58, |l| {
        Ok(Line { fill: fill(l)?, multiplier: l.f32(0x48)?, width: l.f32(0x4c)?, scale_mode: l.u8(0x57)? & 3 })
    })?;
    let fill_bounds = match n.ptr(0x30)? {
        Some(b) => (0..fills.len()).map(|i| f32x4(&b, 0x10 * i)).collect::<Result<_>>()?,
        None => Vec::new(),
    };
    Ok(Layer { bounds: f32x4(&n, 0)?, points, fills, lines, fill_bounds, no_strokes: n.u8(0x38)? != 0 })
}

/// A shape body: +0 bounds via the caller, +0x08 i16 entry count, +0x18 entries.
pub fn shape_body(bounds: [f32; 4], body: Node) -> Result<Shape> {
    let count = body.i16(0x08)?.max(0) as usize;
    let mut layers = Vec::with_capacity(count);
    if let Some(entries) = body.ptr(0x18)? {
        for i in 0..count {
            layers.push(layer(entries.at(0x40 * i))?);
        }
    }
    Ok(Shape { bounds, layers })
}

impl IggyFile {
    pub fn shape(&self, id: usize) -> Result<Option<Shape>> {
        let Some(rec) = self.character(id)? else { return Ok(None) };
        if rec.u8(0)? != kind::SHAPE {
            return Ok(None);
        }
        shape_body(f32x4(&rec, 0x20)?, rec.at(0x30)).map(Some)
    }

    pub fn bitmap(&self, id: usize) -> Result<Option<Bitmap>> {
        let Some(rec) = self.character(id)? else { return Ok(None) };
        if rec.u8(0)? != kind::BITMAP {
            return Ok(None);
        }
        Ok(Some(Bitmap {
            width: rec.u16(0x20)?,
            height: rec.u16(0x22)?,
            blended: rec.u8(0x24)? == 4,
            uv: f32x4(&rec, 0x30)?,
        }))
    }

    /// Bounds stored in a displayable record (shapes and edit text +0x20).
    pub fn character_bounds(&self, id: usize) -> Result<Option<[f32; 4]>> {
        let Some(rec) = self.character(id)? else { return Ok(None) };
        Ok(match rec.u8(0)? {
            kind::SHAPE | kind::EDIT_TEXT => Some(f32x4(&rec, 0x20)?),
            kind::BITMAP => Some([0.0, 0.0, rec.u16(0x20)? as f32, rec.u16(0x22)? as f32]),
            _ => None,
        })
    }

    /// The `.raw.iggytex` index (the loaded type-8 section).
    pub fn textures(&self) -> Result<Vec<TextureEntry>> {
        let Some(section) = self.section_of_kind(8) else { return Ok(Vec::new()) };
        let count = section.u16(0)? as usize;
        (0..count)
            .map(|i| {
                let e = section.at(0x10 + 12 * i);
                Ok(TextureEntry {
                    offset: e.u32(0)?,
                    format: e.u8(4)?,
                    mips: e.u8(5)?,
                    width: e.u16(6)?,
                    height: e.u16(8)?,
                    character: e.u16(10)?,
                })
            })
            .collect()
    }

    /// Total `.raw.iggytex` size recorded in the index.
    pub fn texture_bytes(&self) -> Result<Option<u32>> {
        self.section_of_kind(8).map(|s| s.u32(4)).transpose()
    }
}
