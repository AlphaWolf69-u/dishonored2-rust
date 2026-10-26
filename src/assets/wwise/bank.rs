//! Sequential version-113 HIRC node layout. Field order is corroborated by
//! the installed banks and wwiser's CAkParameterNode/CAkRanSeqCntr readers:
//! https://github.com/bnnm/wwiser/blob/master/wwiser/parser/wparser.py
use super::{invalid, invalid_owned, SelectionMode};
use std::io;

#[derive(Clone, Debug)]
pub struct CurvePoint {
    pub input: f32,
    pub output: f32,
    pub interpolation: u32,
}

#[derive(Clone, Debug)]
pub struct RtpcCurve {
    pub parameter: u32,
    pub kind: u8,
    pub accumulation: u8,
    pub property: u8,
    pub scaling: u8,
    pub points: Vec<CurvePoint>,
}

#[derive(Clone, Debug)]
pub struct PropertyRange {
    pub property: u8,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, Default)]
pub struct NodeProperties {
    pub id: u32,
    pub parent: u32,
    pub bus: u32,
    pub values: Vec<(u8, u32)>,
    pub ranges: Vec<PropertyRange>,
    pub curves: Vec<RtpcCurve>,
}

impl NodeProperties {
    pub fn float(&self, property: u8) -> f32 {
        self.values.iter().find(|(id, _)| *id == property).map_or(0.0, |(_, bits)| f32::from_bits(*bits))
    }
}

pub(super) struct Playlist {
    pub items: Vec<(u32, u32)>,
    pub mode: SelectionMode,
    pub reset_each_play: bool,
    pub global: bool,
    pub avoid_repeat: u16,
    pub playback: ContainerPlayback,
}

/// Random/sequence playback fields loaded by `FUN_14111aed0`.
#[derive(Clone, Debug)]
pub struct ContainerPlayback {
    /// Zero loops forever. A zero count also discards the loop modifiers.
    pub loop_count: u16,
    pub loop_modifiers: [i16; 2],
    /// Authored milliseconds and additive random modifiers.
    pub transition_ms: f32,
    pub transition_modifiers_ms: [f32; 2],
    pub transition: TransitionMode,
    pub continuous: bool,
    pub restart_backward: bool,
}

impl Default for ContainerPlayback {
    fn default() -> Self {
        // FUN_141118c30 starts a container with one traversal.
        Self { loop_count: 1, loop_modifiers: [0; 2], transition_ms: 0.0,
            transition_modifiers_ms: [0.0; 2], transition: TransitionMode::Disabled,
            continuous: false, restart_backward: false }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransitionMode {
    #[default]
    Disabled,
    CrossfadeAmplitude,
    CrossfadePower,
    Delay,
    SampleAccurate,
    TriggerRate,
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, size: usize) -> io::Result<&'a [u8]> {
        let end = self.offset.checked_add(size).ok_or_else(|| invalid("Wwise node offset overflow"))?;
        let bytes = self.bytes.get(self.offset..end).ok_or_else(|| invalid_owned(format!(
            "truncated Wwise node at {:#x}: need {size} bytes, node has {}", self.offset, self.bytes.len()
        )))?;
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> io::Result<u8> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> io::Result<u16> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> io::Result<u32> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn f32(&mut self) -> io::Result<f32> { Ok(f32::from_bits(self.u32()?)) }

    fn records(&mut self, count: usize, size: usize) -> io::Result<()> {
        self.take(count.checked_mul(size).ok_or_else(|| invalid("Wwise node record count overflow"))?)?;
        Ok(())
    }

    fn points(&mut self, count: usize) -> io::Result<Vec<CurvePoint>> {
        let mut points = Vec::new();
        for _ in 0..count {
            points.push(CurvePoint { input: self.f32()?, output: self.f32()?, interpolation: self.u32()? });
        }
        Ok(points)
    }

    fn rtpcs(&mut self) -> io::Result<Vec<RtpcCurve>> {
        let count = self.u16()?;
        let mut curves = Vec::new();
        for _ in 0..count {
            let parameter = self.u32()?;
            let kind = self.u8()?;
            let accumulation = self.u8()?;
            let property = self.u8()?;
            self.u32()?; // Curve ID.
            let scaling = self.u8()?;
            let points = self.u16()? as usize;
            curves.push(RtpcCurve { parameter, kind, accumulation, property, scaling, points: self.points(points)? });
        }
        Ok(curves)
    }

    fn properties(&mut self) -> io::Result<Vec<(u8, u32)>> {
        let count = self.u8()? as usize;
        let ids = self.take(count)?.to_vec();
        ids.into_iter().map(|id| Ok((id, self.u32()?))).collect()
    }

    fn states(&mut self) -> io::Result<()> {
        let groups = self.u32()?;
        for _ in 0..groups {
            self.take(5)?;
            let states = self.u16()? as usize;
            self.records(states, 8)?;
        }
        Ok(())
    }

    fn node_base(&mut self, feedback: bool) -> io::Result<NodeProperties> {
        let mut node = NodeProperties::default();
        self.u8()?; // Override parent effects.
        let effects = self.u8()? as usize;
        if effects != 0 { self.u8()?; self.records(effects, 7)?; }
        self.u8()?; // Attachment override.
        node.bus = self.u32()?;
        node.parent = self.u32()?;
        self.u8()?; // Priority flags.
        node.values = self.properties()?;
        let count = self.u8()? as usize;
        let ids = self.take(count)?.to_vec();
        for property in ids {
            let (min, max) = (self.f32()?, self.f32()?);
            // AkPropValue also carries integer properties such as Loop.
            // Validate as floats only for the floating gain/pitch properties.
            if matches!(property, 0 | 2 | 5 | 0x17 | 0x21 | 0x36)
                && (!min.is_finite() || !max.is_finite() || min > max)
            { return Err(invalid("invalid Wwise property randomization range")); }
            node.ranges.push(PropertyRange { property, min, max });
        }

        let positioning = self.u8()?;
        if positioning & 1 != 0 && positioning & 8 != 0 {
            let flags = self.u8()?;
            self.u32()?; // Attenuation ID.
            if flags & 3 != 1 {
                self.take(5)?; // Automation path mode and transition time.
                let vertices = self.u32()? as usize;
                self.records(vertices, 16)?;
                let paths = self.u32()? as usize;
                self.records(paths, 20)?; // Vertex spans and XYZ ranges.
            }
        }
        let auxiliary = self.u8()?;
        if auxiliary & 8 != 0 { self.take(16)?; }
        self.take(6)?; // Voice limit, virtualization and analysis settings.
        self.states()?;
        node.curves = self.rtpcs()?;
        if feedback { self.u32()?; }
        Ok(node)
    }

    fn children(&mut self) -> io::Result<Vec<u32>> {
        let count = self.u32()? as usize;
        let bytes = self.take(count.checked_mul(4).ok_or_else(|| invalid("Wwise child count overflow"))?)?;
        Ok(bytes.chunks_exact(4).map(|id| u32::from_le_bytes(id.try_into().unwrap())).collect())
    }
}

pub(super) fn node_properties(object: &[u8], kind: u8, feedback: bool) -> io::Result<NodeProperties> {
    let mut reader = Reader { bytes: object, offset: 0 };
    let id = reader.u32()?;
    if kind == 2 {
        let plugin = reader.u32()?;
        reader.take(10)?; // Stream type, media ID, memory size and source flags.
        if matches!(plugin & 0xf, 2 | 5) {
            let size = reader.u32()? as usize;
            reader.take(size)?;
        }
    }
    let mut node = reader.node_base(feedback)?;
    node.id = id;
    Ok(node)
}

pub(super) fn bus_properties(object: &[u8], feedback: bool) -> io::Result<NodeProperties> {
    let mut reader = Reader { bytes: object, offset: 0 };
    let mut node = NodeProperties { id: reader.u32()?, parent: reader.u32()?, ..Default::default() };
    node.values = reader.properties()?;
    reader.take(17)?; // Bus flags/limits/channel configuration, HDR and duck limits.
    let ducks = reader.u32()? as usize;
    reader.records(ducks, 18)?;
    let effects = reader.u8()? as usize;
    if effects != 0 { reader.u8()?; reader.records(effects, 7)?; }
    reader.take(6)?; // Mixer share set and attachment override.
    node.curves = reader.rtpcs()?;
    reader.states()?;
    if feedback { reader.u32()?; }
    if reader.offset != object.len() { return Err(invalid("Wwise bus fields did not consume its object")); }
    Ok(node)
}

pub(super) fn global_parameters(bytes: &[u8]) -> io::Result<(f32, std::collections::HashMap<u32, f32>)> {
    let mut reader = Reader { bytes, offset: 0 };
    let threshold = reader.f32()?;
    reader.u16()?; // Voice limit.
    let states = reader.u32()?;
    for _ in 0..states {
        reader.take(8)?; // State group and default transition time.
        let transitions = reader.u32()? as usize;
        reader.records(transitions, 12)?;
    }
    let switches = reader.u32()?;
    for _ in 0..switches {
        reader.take(9)?; // Switch ID, RTPC ID and RTPC type.
        let points = reader.u32()? as usize;
        reader.records(points, 12)?;
    }
    let count = reader.u32()?;
    let mut parameters = std::collections::HashMap::new();
    for _ in 0..count {
        let id = reader.u32()?;
        parameters.insert(id, reader.f32()?);
        reader.take(13)?; // Ramp type, up/down times and built-in binding.
    }
    if reader.offset != bytes.len() { return Err(invalid("Wwise global settings did not consume their chunk")); }
    Ok((threshold, parameters))
}

pub(super) fn node_children(object: &[u8], kind: u8, feedback: bool) -> io::Result<(Vec<u32>, Playlist)> {
    let mut reader = Reader { bytes: object, offset: 4 };
    reader.node_base(feedback)?;
    let mut playlist = Playlist {
        items: Vec::new(), mode: SelectionMode::Single, reset_each_play: true, global: true, avoid_repeat: 0,
        playback: ContainerPlayback::default(),
    };
    if kind == 5 {
        playlist.playback.loop_count = reader.u16()?;
        let modifiers = [reader.u16()? as i16, reader.u16()? as i16];
        if playlist.playback.loop_count != 0 {
            playlist.playback.loop_modifiers = modifiers;
        }
        playlist.playback.transition_ms = reader.f32()?;
        playlist.playback.transition_modifiers_ms = [reader.f32()?, reader.f32()?];
        playlist.avoid_repeat = reader.u16()?;
        playlist.playback.transition = match reader.u8()? {
            0 => TransitionMode::Disabled,
            1 => TransitionMode::CrossfadeAmplitude,
            2 => TransitionMode::CrossfadePower,
            3 => TransitionMode::Delay,
            4 => TransitionMode::SampleAccurate,
            5 => TransitionMode::TriggerRate,
            _ => return Err(invalid("invalid Wwise transition mode")),
        };
        let random_mode = reader.u8()?;
        playlist.mode = match reader.u8()? {
            0 => match random_mode {
                0 => SelectionMode::Random,
                1 => SelectionMode::Shuffle,
                _ => return Err(invalid("invalid Wwise random mode")),
            },
            1 => SelectionMode::Sequence,
            _ => return Err(invalid("invalid Wwise random/sequence mode")),
        };
        let flags = reader.u8()?;
        playlist.reset_each_play = flags & 2 != 0;
        playlist.global = flags & 16 != 0;
        playlist.playback.continuous = flags & 8 != 0;
        playlist.playback.restart_backward = flags & 4 != 0;
    }
    let children = reader.children()?;
    if kind == 5 {
        let count = reader.u16()?;
        for _ in 0..count {
            let child = reader.u32()?;
            let weight = reader.u32()?;
            if !children.contains(&child) { return Err(invalid("Wwise playlist references a non-child node")); }
            playlist.items.push((child, weight));
        }
    } else if kind == 9 {
        let layers = reader.u32()?;
        for _ in 0..layers {
            reader.u32()?; // Layer ID.
            reader.rtpcs()?;
            reader.take(5)?; // Crossfade RTPC ID and type.
            let associations = reader.u32()?;
            for _ in 0..associations {
                reader.u32()?;
                let points = reader.u32()? as usize;
                reader.records(points, 12)?;
            }
        }
    }
    if reader.offset != object.len() { return Err(invalid("Wwise container fields did not consume its object")); }
    Ok((children, playlist))
}
