//! Sprite timelines, placement records and the SymbolClass table.
//!
//! Layouts follow Iggy's SWF loader (`FUN_141073760` sprites,
//! `FUN_14107a5b0` timeline commands, `FUN_1410757a0` placements) which
//! builds the same records a `.iggy` file stores serialized.
use super::{Error, IggyFile, Node, Result};

/// Character record kinds (`FUN_1410786a0` tag handlers).
pub mod kind {
    pub const SHAPE: u8 = 1;
    pub const MORPH_SHAPE: u8 = 2;
    pub const SPRITE: u8 = 3;
    pub const BITMAP: u8 = 4;
    pub const STATIC_TEXT: u8 = 5;
    pub const EDIT_TEXT: u8 = 6;
    pub const BUTTON: u8 = 7;
    pub const FONT: u8 = 0x16;
}

/// Iggy blend-mode codes (`FUN_141070390`); 0x0f means unchanged.
pub const BLEND_UNCHANGED: u8 = 0x0f;
/// Filter count meaning unchanged.
pub const FILTERS_UNCHANGED: u8 = 0x7f;
/// Placement end frame meaning the placement is never replaced.
pub const NEVER: i32 = 0x10000;

/// A colour transform as stored by `FUN_141071160`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourTransform {
    /// Multipliers already scaled by 1/256 (default 1).
    pub mul: [f32; 4],
    /// Raw additive terms (-255..255).
    pub add: [i16; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    DropShadow { colour: [u8; 4], blur: [f32; 2], angle: f32, distance: f32, strength: f32, passes: u8, inner: bool, knockout: bool },
    Blur { blur: [f32; 2], passes: u8 },
    Glow { blur: [f32; 2], strength: f32, passes: u8, inner: bool, knockout: bool },
    Bevel { blur: [f32; 2], angle: f32, distance: f32, strength: f32, passes: u8 },
    GradientGlow { blur: [f32; 2], angle: f32, distance: f32, strength: f32, passes: u8, colours: Vec<[u8; 4]>, ratios: Vec<u8> },
    ColorMatrix,
    Convolution,
    GradientBevel { blur: [f32; 2], angle: f32, distance: f32, strength: f32, passes: u8 },
}

/// PLACE node flags (+0x09).
pub mod place {
    pub const MATRIX: u8 = 0x01;
    pub const COLOUR: u8 = 0x02;
    pub const MOVE: u8 = 0x04;
    pub const BITMAP: u8 = 0x08;
    pub const CACHE_AS_BITMAP: u8 = 0x10;
    pub const CLASS_NAME: u8 = 0x20;
    pub const NAME: u8 = 0x40;
    pub const CLIP_DEPTH: u8 = 0x80;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub flags: u8,
    pub filter_count: u8,
    pub blend: u8,
    /// Frame at which this placement is replaced or removed (`NEVER`).
    pub end_frame: i32,
    /// x' = m0·x + m1·y + m4, y' = m2·x + m3·y + m5 (pixels).
    pub matrix: [f32; 6],
    pub depth: u16,
    /// Character id of a local character.
    pub character: Option<u16>,
    pub ratio: u16,
    pub clip_depth: u16,
    pub name: Option<String>,
    pub colour: Option<ColourTransform>,
    pub class_name: Option<String>,
    pub filters: Vec<Filter>,
}

impl Place {
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// Creates an instance (character or class) rather than modifying one.
    pub fn creates(&self) -> bool {
        self.character.is_some() || self.has(place::CLASS_NAME)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    ShowFrame,
    Place(Box<Place>),
    Remove { depth: u16 },
    TabIndex { depth: u16, index: u16, end_frame: i32 },
    Label { name: String, anchor: bool },
    StartSound { character: u16, stop: bool, no_multiple: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    pub frame_count: u16,
    /// Max placed depth + 1.
    pub depth_size: u16,
    pub commands: Vec<Command>,
    /// DefineScalingGrid rectangle.
    pub scale9: Option<[f32; 4]>,
}

impl Timeline {
    /// Commands of each frame (index 0 = frame 1).
    pub fn frames(&self) -> Vec<&[Command]> {
        let mut out = Vec::new();
        let mut start = 0;
        for (i, c) in self.commands.iter().enumerate() {
            if *c == Command::ShowFrame {
                out.push(&self.commands[start..i]);
                start = i + 1;
            }
        }
        out
    }

    /// Zero-based frame of a label (`FUN_14107db40`).
    pub fn label_frame(&self, label: &str) -> Option<u32> {
        let mut frame = 0;
        for c in &self.commands {
            match c {
                Command::ShowFrame => frame += 1,
                Command::Label { name, .. } if name == label => return Some(frame),
                _ => {}
            }
        }
        None
    }

    pub fn labels(&self) -> Vec<(String, u32)> {
        let mut frame = 0;
        let mut out = Vec::new();
        for c in &self.commands {
            match c {
                Command::ShowFrame => frame += 1,
                Command::Label { name, .. } => out.push((name.clone(), frame)),
                _ => {}
            }
        }
        out
    }
}

fn opt_wide(node: &Node, at: usize) -> Result<Option<String>> {
    node.ptr(at)?.map(|n| n.wide_str()).transpose()
}

fn colour(node: Node) -> Result<ColourTransform> {
    Ok(ColourTransform {
        mul: [node.f32(4)?, node.f32(8)?, node.f32(0xc)?, node.f32(0x10)?],
        add: [node.i16(0x14)?, node.i16(0x16)?, node.i16(0x18)?, node.i16(0x1a)?],
    })
}

/// Filter records (0x38 bytes, `FUN_141079960`).
fn filter(node: Node) -> Result<Filter> {
    let rgba = |at: usize| -> Result<[u8; 4]> { Ok(node.bytes(at, 4)?.try_into().unwrap()) };
    Ok(match node.u8(0)? {
        0 => Filter::DropShadow {
            colour: rgba(4)?,
            blur: [node.f32(8)?, node.f32(0xc)?],
            angle: node.f32(0x10)?,
            distance: node.f32(0x14)?,
            strength: node.f32(0x18)?,
            passes: node.u8(1)?,
            inner: node.u8(2)? & 1 != 0,
            knockout: node.u8(2)? & 2 != 0,
        },
        1 => Filter::Blur { blur: [node.f32(4)?, node.f32(8)?], passes: node.u8(1)? },
        2 => Filter::Glow {
            blur: [node.f32(8)?, node.f32(0xc)?],
            strength: node.f32(0x10)?,
            passes: node.u8(1)?,
            inner: node.u8(2)? & 1 != 0,
            knockout: node.u8(2)? & 2 != 0,
        },
        3 => Filter::Bevel {
            blur: [node.f32(0x10)?, node.f32(0x14)?],
            angle: node.f32(0x18)?,
            distance: node.f32(0x1c)?,
            strength: node.f32(0x20)?,
            passes: node.u8(1)?,
        },
        4 => {
            let count = node.u8(3)? as usize;
            let mut colours = Vec::new();
            let mut ratios = Vec::new();
            if let (Some(c), Some(r)) = (node.ptr(0x20)?, node.ptr(0x28)?) {
                for i in 0..count {
                    colours.push(c.bytes(4 * i, 4)?.try_into().unwrap());
                    ratios.push(r.u8(i)?);
                }
            }
            Filter::GradientGlow {
                blur: [node.f32(8)?, node.f32(0xc)?],
                angle: node.f32(0x14)?,
                distance: node.f32(0x18)?,
                strength: node.f32(0x10)?,
                passes: node.u8(1)?,
                colours,
                ratios,
            }
        }
        5 => Filter::Convolution,
        6 => Filter::ColorMatrix,
        7 => Filter::GradientBevel {
            blur: [node.f32(0x10)?, node.f32(0x14)?],
            angle: node.f32(0x18)?,
            distance: node.f32(0x1c)?,
            strength: node.f32(0x20)?,
            passes: node.u8(1)?,
        },
        _ => return Err(Error::Corrupt("filter id")),
    })
}

fn place_node(node: Node) -> Result<Place> {
    let flags = node.u8(9)?;
    let filter_count = node.u8(0xa)?;
    let mut matrix = [0f32; 6];
    for (i, m) in matrix.iter_mut().enumerate() {
        *m = node.f32(0x10 + 4 * i)?;
    }
    let character = node.ptr(0x30)?.map(|c| c.u16(2)).transpose()?;
    let mut filters = Vec::new();
    if filter_count != FILTERS_UNCHANGED && filter_count > 0 {
        if let Some(list) = node.ptr(0x58)? {
            for i in 0..filter_count as usize {
                filters.push(filter(list.at(0x38 * i))?);
            }
        }
    }
    Ok(Place {
        flags,
        filter_count,
        blend: node.u8(0xb)?,
        end_frame: node.i32(0xc)?,
        matrix,
        depth: node.u16(0x28)?,
        character,
        ratio: node.u16(0x38)?,
        clip_depth: node.u16(0x3a)?,
        name: if flags & place::NAME != 0 { opt_wide(&node, 0x40)? } else { None },
        colour: if flags & place::COLOUR != 0 { node.ptr(0x48)?.map(colour).transpose()? } else { None },
        class_name: if flags & place::CLASS_NAME != 0 { opt_wide(&node, 0x50)? } else { None },
        filters,
    })
}

/// Walk a command list from its head pointer field.
fn commands(head: Option<Node>) -> Result<Vec<Command>> {
    let mut out = Vec::new();
    let mut cur = head;
    while let Some(node) = cur {
        out.push(match node.u8(8)? {
            1 => Command::ShowFrame,
            2 => Command::Place(Box::new(place_node(node)?)),
            3 => Command::Remove { depth: node.u16(0xa)? },
            4 => Command::TabIndex { depth: node.u16(0xc)?, index: node.u16(0xe)?, end_frame: node.i32(0x10)? },
            5 => Command::Label { name: node.ptr(0x10)?.map(|n| n.wide_str()).transpose()?.unwrap_or_default(), anchor: node.u8(9)? != 0 },
            6 => Command::StartSound { character: node.u16(0xc)?, stop: node.u8(0xe)? != 0, no_multiple: node.u8(0xf)? != 0 },
            _ => return Err(Error::Corrupt("timeline command")),
        });
        cur = node.ptr(0)?;
    }
    Ok(out)
}

impl IggyFile {
    /// The main timeline (root +0x10 head, +0x44 frames, +0x48 depth size).
    pub fn main_timeline(&self) -> Result<Timeline> {
        let r = self.root();
        Ok(Timeline {
            frame_count: r.u32(0x44)? as u16,
            depth_size: r.u32(0x48)? as u16,
            commands: commands(r.ptr(0x10)?)?,
            scale9: None,
        })
    }

    /// The timeline of sprite character `id`.
    pub fn sprite(&self, id: usize) -> Result<Option<Timeline>> {
        let Some(rec) = self.character(id)? else { return Ok(None) };
        if rec.u8(0)? != kind::SPRITE {
            return Ok(None);
        }
        let scale9 = match rec.ptr(0x20)? {
            Some(r) => Some([r.f32(0)?, r.f32(4)?, r.f32(8)?, r.f32(0xc)?]),
            None => None,
        };
        Ok(Some(Timeline {
            frame_count: rec.u16(0x28)?,
            depth_size: rec.u16(0x2a)?,
            commands: commands(rec.ptr(0x38)?)?,
            scale9,
        }))
    }

    /// SymbolClass entries: class name and character index (root +0xa0 / +0xb0).
    /// Character index 0 is the main timeline: its class is the document
    /// class (`FUN_14102ad10`).
    pub fn symbol_classes(&self) -> Result<Vec<(String, u32)>> {
        let r = self.root();
        let count = r.u32(0xb0)? as usize;
        let Some(table) = r.ptr(0xa0)? else { return Ok(Vec::new()) };
        (0..count)
            .map(|i| {
                let e = table.at(0x18 * i);
                Ok((opt_wide(&e, 8)?.unwrap_or_default(), e.u32(0x10)?))
            })
            .collect()
    }

    /// The document class bound to the main timeline, if any.
    pub fn document_class(&self) -> Result<Option<String>> {
        Ok(self.symbol_classes()?.into_iter().find(|(_, index)| *index == 0).map(|(name, _)| name))
    }

    /// The library name this file registers when created as a library (root +0x90).
    pub fn library_name(&self) -> Result<Option<String>> {
        opt_wide(&self.root(), 0x90)
    }

    /// Main timeline frame labels (root +0x20, +0x4c count).
    pub fn frame_labels(&self) -> Result<Vec<(String, u32)>> {
        let r = self.root();
        let count = r.u32(0x4c)? as usize;
        let Some(table) = r.ptr(0x20)? else { return Ok(Vec::new()) };
        (0..count)
            .map(|i| {
                let e = table.at(0x10 * i);
                Ok((opt_wide(&e, 0)?.unwrap_or_default(), e.u32(8)?))
            })
            .collect()
    }

    /// The character kind byte, if the slot holds a record.
    pub fn character_kind(&self, id: usize) -> Result<Option<u8>> {
        self.character(id)?.map(|c| c.u8(0)).transpose()
    }
}
