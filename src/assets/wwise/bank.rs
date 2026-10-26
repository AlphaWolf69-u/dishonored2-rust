//! Sequential version-113 HIRC node layout. Field order is corroborated by
//! the installed banks and wwiser's CAkParameterNode/CAkRanSeqCntr readers:
//! https://github.com/bnnm/wwiser/blob/master/wwiser/parser/wparser.py
use super::{invalid, invalid_owned, SelectionMode};
use std::io;

#[derive(Clone, Copy, Debug)]
pub struct ActionTransition {
    pub milliseconds: i32,
    pub modifiers: [i32; 2],
    pub curve: u8,
}
impl ActionTransition {
    /// FUN_141115f20: integer milliseconds, with an additive rounded rand().
    pub fn sample_ms(self, mut random15: impl FnMut() -> u16) -> i32 {
        sample_integer(self.milliseconds, self.modifiers, &mut random15)
    }
}

fn sample_integer(base: i32, [min, max]: [i32; 2], mut random15: impl FnMut() -> u16) -> i32 {
    let difference = max.wrapping_sub(min);
    let offset = if difference == 0 { 0 } else {
        (f64::from(random15()) * f64::from_bits(0x3f00002000400080) * f64::from(difference) + 0.5) as i32
    };
    base.wrapping_add(min).wrapping_add(offset)
}

#[derive(Clone, Copy, Debug)]
pub struct ActionDelay {
    pub frames: i32,
    pub modifiers: [i32; 2],
}
impl ActionDelay {
    /// 141116320 converts milliseconds before 141115e20 randomizes the
    /// integer frame interval. The action scheduler interprets it unsigned.
    pub fn sample_frames(self, random15: impl FnMut() -> u16) -> u32 {
        sample_integer(self.frames, self.modifiers, random15) as u32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlaySettings {
    pub transition: ActionTransition,
    pub delay: ActionDelay,
    pub probability: Option<f32>,
    pub bank: u32,
}
impl PlaySettings {
    pub fn requires_scheduling(self) -> bool {
        self.transition.milliseconds != 0 || self.transition.modifiers != [0; 2]
            || self.delay.frames != 0 || self.delay.modifiers != [0; 2]
    }

    /// 14113b5c0 uses two MSVC rand() values as a 30-bit draw. An absent
    /// probability property plays unconditionally; explicit zero consumes no RNG.
    pub fn admits(self, mut random15: impl FnMut() -> u16) -> bool {
        let Some(probability) = self.probability else { return true; };
        if probability == 0.0 { return false; }
        let draw = u32::from(random15()) * 0x8000 + u32::from(random15());
        f64::from(draw) * f64::from_bits(0x3e10000000400000) * 100.0 <= f64::from(probability)
    }
}

/// 141116320's common action properties, then 14113b7a0's fade curve and
/// bank ID. Keep the frame conversion ahead of delay randomization.
pub(super) fn play_settings(bytes: &[u8]) -> io::Result<PlaySettings> {
    let mut reader = Reader { bytes, offset: 4 };
    reader.take(6)?; // Scope/type and target ID, retained by the Play command.
    if reader.u8()? != 0 { return Err(invalid("Wwise bus Play actions require bus dispatch")); }
    let properties = reader.properties()?;
    let property = |id| properties.iter().find(|(property, _)| *property == id).map(|(_, value)| *value);
    let mut transition = ActionTransition { milliseconds: property(0xf).unwrap_or(0) as i32, modifiers: [0; 2], curve: 4 };
    let mut delay = ActionDelay { frames: (property(0xe).unwrap_or(0) as i32).wrapping_mul(48), modifiers: [0; 2] };
    let probability = property(0x10).map(f32::from_bits);
    let count = usize::from(reader.u8()?);
    let ids = reader.take(count)?.to_vec();
    for id in ids {
        let values = [reader.u32()? as i32, reader.u32()? as i32];
        match id {
            0xf => transition.modifiers = values,
            0xe => delay.modifiers = values.map(|value| value.wrapping_mul(48)),
            _ => {},
        }
    }
    transition.curve = reader.u8()? & 0x1f;
    if transition.curve > 8 { return Err(invalid("unsupported Wwise Play fade curve")); }
    let bank = reader.u32()?;
    if reader.offset != bytes.len() { return Err(invalid("Wwise Play fields did not consume its action")); }
    Ok(PlaySettings { transition, delay, probability, bank })
}

#[derive(Clone, Debug)]
pub struct StopAction {
    pub target: u32,
    pub game_object: bool,
    pub all: bool,
    pub transition: ActionTransition,
}

#[derive(Clone, Copy, Debug)]
pub struct StateAction {
    pub group: u32,
    pub value: u32,
    pub game_object: bool,
    pub delay: ActionDelay,
}

/// 14113c730 reads the group and value after CAkAction's common bundles.
/// Execute (14113c610) changes the global group, independently of its target.
pub(super) fn state_action(bytes: &[u8]) -> io::Result<StateAction> {
    let mut reader = Reader { bytes, offset: 4 };
    let game_object = reader.u8()? & 1 != 0;
    reader.take(6)?; // Action type, unused target and bus flag.
    let properties = reader.properties()?;
    let mut delay = ActionDelay { frames: properties.iter().find(|(id, _)| *id == 0xe)
        .map_or(0, |(_, value)| (*value as i32).wrapping_mul(48)), modifiers: [0; 2] };
    let count = reader.u8()? as usize;
    let ids = reader.take(count)?.to_vec();
    for id in ids {
        let values = [reader.u32()? as i32, reader.u32()? as i32];
        if id == 0xe { delay.modifiers = values.map(|value| value.wrapping_mul(48)); }
    }
    let group = reader.u32()?;
    let value = reader.u32()?;
    if reader.offset != bytes.len() { return Err(invalid("Wwise Set State fields did not consume its action")); }
    Ok(StateAction { group, value, game_object, delay })
}

#[derive(Clone, Debug)]
pub struct StateTransition { pub from: u32, pub to: u32, pub milliseconds: u32 }

#[derive(Clone, Debug)]
pub struct StateGroup {
    pub id: u32,
    pub milliseconds: u32,
    pub transitions: Vec<StateTransition>,
}

/// CAkAction's property bundles (141116320), active fade curve (1411492c0),
/// and Stop scopes (141127000). Target/All execution ignores the serialized
/// exception list (141142380 / 141142190); All Except needs its own filtering.
pub(super) fn stop_action(bytes: &[u8]) -> io::Result<StopAction> {
    let mut reader = Reader { bytes, offset: 4 };
    let scope = reader.u8()?;
    let (game_object, all) = match scope {
        2 => (false, false), 3 => (true, false),
        4 => (false, true), 5 => (true, true),
        8 | 9 => return Err(invalid("Wwise All Except Stop actions require exclusion filtering")),
        _ => return Err(invalid("unsupported Wwise Stop scope")),
    };
    reader.u8()?;
    let target = reader.u32()?;
    let bus = reader.u8()? != 0;
    if bus && !all { return Err(invalid("Wwise bus-targeted Stop actions require bus dispatch")); }
    let properties = reader.properties()?;
    let milliseconds = properties.iter().find(|(id, _)| *id == 0xf).map_or(0, |(_, value)| *value as i32);
    if properties.iter().any(|(id, value)| *id == 0xe && *value != 0) {
        return Err(invalid("delayed Wwise Stop actions require action scheduling"));
    }
    let count = reader.u8()? as usize;
    let ids = reader.take(count)?.to_vec();
    let mut modifiers = [0; 2];
    for id in ids {
        let values = [reader.u32()? as i32, reader.u32()? as i32];
        if id == 0xf { modifiers = values; }
        if id == 0xe && values != [0; 2] { return Err(invalid("randomized Wwise action delays require scheduling")); }
    }
    let curve = reader.u8()? & 0x1f;
    if curve > 8 { return Err(invalid("unsupported Wwise action fade curve")); }
    let exceptions = reader.u32()? as usize;
    reader.records(exceptions, 5)?; // 141141fc0: actor/bus ID and bus flag.
    if reader.offset != bytes.len() { return Err(invalid("Wwise Stop fields did not consume its action")); }
    Ok(StopAction { target, game_object, all, transition: ActionTransition { milliseconds, modifiers, curve } })
}

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
    pub positioning_flags: u8,
    pub positioning_3d: Option<Positioning3d>,
    pub effects: EffectChain,
    /// 1410deb00: game override, game enable, user override, serialized IDs.
    pub auxiliary_flags: u8,
    pub auxiliary_buses: [u32; 4],
    pub bus_settings: Option<BusSettings>,
}

#[derive(Clone, Debug)]
pub struct BusSettings {
    pub auxiliary: bool,
    pub flags: u8,
    pub limit_flags: u8,
    pub max_instances: u16,
    pub channel_config: u32,
    pub hdr_flags: u8,
    pub recovery_ms: u32,
    pub max_duck_volume: f32,
    pub ducks: Vec<BusDuck>,
    pub mixer_id: u32,
    pub mixer_shared: bool,
    pub attachment_override: bool,
}

#[derive(Clone, Debug)]
pub struct BusDuck {
    pub bus: u32,
    pub volume: f32,
    pub fade_out_ms: u32,
    pub fade_in_ms: u32,
    pub curve: u8,
    pub property: u8,
}

#[derive(Clone, Debug, Default)]
pub struct EffectChain {
    pub override_parent: bool,
    pub bypass: u8,
    pub slots: Vec<EffectReference>,
}

#[derive(Clone, Debug)]
pub struct EffectReference {
    pub slot: u8,
    pub id: u32,
    pub shared: bool,
    pub flags: u8,
}

#[derive(Clone, Debug)]
pub struct EffectDefinition {
    pub id: u32,
    pub shared: bool,
    pub plugin: u32,
    pub parameters: Vec<u8>,
    pub media: Vec<(u8, u32)>,
    pub curves: Vec<RtpcCurve>,
    pub values: Vec<(u8, u32)>,
}

#[derive(Clone, Debug)]
pub struct Positioning3d {
    pub flags: u8,
    pub attenuation: u32,
}

#[derive(Clone, Debug)]
pub struct AttenuationCurve {
    pub scaling: u8,
    pub points: Vec<CurvePoint>,
}

#[derive(Clone, Debug, Default)]
pub struct EnvironmentCurve {
    pub enabled: bool,
    pub scaling: u8,
    pub points: Vec<CurvePoint>,
}

/// ENVS: obstruction then occlusion, each with volume, LPF and HPF.
#[derive(Clone, Debug, Default)]
pub struct EnvironmentSettings {
    pub curves: [[EnvironmentCurve; 3]; 2],
}

/// 1410f3450 dispatches ENVS to 1410f5470; 1410f8610/1410f85b0
/// normalize the two volume tables after loading their conversion points.
pub(super) fn environment_settings(bytes: &[u8]) -> io::Result<EnvironmentSettings> {
    let mut reader = Reader { bytes, offset: 0 };
    let mut settings = EnvironmentSettings::default();
    for group in &mut settings.curves {
        for (index, curve) in group.iter_mut().enumerate() {
            curve.enabled = reader.u8()? != 0;
            curve.scaling = reader.u8()?;
            let count = usize::from(reader.u16()?);
            curve.points = reader.points(count)?;
            if index == 0 {
                match curve.scaling {
                    0 => curve.scaling = 4,
                    2 => {
                        for point in &mut curve.points { point.output += 1.0; }
                        curve.scaling = 0;
                    }
                    _ => {}
                }
            }
        }
    }
    if reader.offset != bytes.len() { return Err(invalid("Wwise environment settings did not consume their chunk")); }
    Ok(settings)
}

#[derive(Clone, Debug)]
pub struct AttenuationCone {
    pub inside_half_angle: f32,
    pub outside_half_angle: f32,
    pub outside_volume_db: f32,
    pub low_pass: f32,
    pub high_pass: f32,
}

#[derive(Clone, Debug)]
pub struct Attenuation {
    pub id: u32,
    pub cone: Option<AttenuationCone>,
    pub curve_indices: [i8; 7],
    pub curves: Vec<AttenuationCurve>,
    pub rtpcs: Vec<RtpcCurve>,
}

// FUN_1410e26c0, including its integer subtraction, as confirmed in the
// instructions. Retain the operation order of the installed executable.
fn cone_half_angle(degrees: f32) -> f32 {
    (degrees - (degrees * f32::from_bits(0x3b360b61)) as i32 as f32) * f32::from_bits(0x3c8efa35) * 0.5
}

/// CAkAttenuation::SetInitialValues (141131380), version 113.
pub(super) fn attenuation(bytes: &[u8]) -> io::Result<Attenuation> {
    let mut reader = Reader { bytes, offset: 0 };
    let id = reader.u32()?;
    let cone = if reader.u8()? != 0 {
        Some(AttenuationCone { inside_half_angle: cone_half_angle(reader.f32()?),
            outside_half_angle: cone_half_angle(reader.f32()?), outside_volume_db: reader.f32()?,
            low_pass: reader.f32()?, high_pass: reader.f32()? })
    } else { None };
    let mut curve_indices = [0; 7];
    for index in &mut curve_indices { *index = reader.u8()? as i8; }
    let count = reader.u8()? as usize;
    if count == 0 || count > 7 { return Err(invalid("invalid Wwise attenuation curve count")); }
    let mut curves = Vec::new();
    for index in 0..count {
        let mut scaling = reader.u8()?;
        let size = reader.u16()? as usize;
        let mut points = reader.points(size)?;
        // The first three mappings are volume curves. The loader converts
        // their authored domain into the linear gain domain at load time.
        if curve_indices[..3].contains(&(index as i8)) {
            match scaling {
                0 => scaling = 4,
                2 => { for point in &mut points { point.output += 1.0; } scaling = 0; },
                _ => {},
            }
        }
        curves.push(AttenuationCurve { scaling, points });
    }
    if curve_indices.iter().any(|index| *index < -1 || *index >= count as i8) {
        return Err(invalid("Wwise attenuation mapping references a missing curve"));
    }
    let rtpcs = reader.rtpcs()?;
    if reader.offset != bytes.len() { return Err(invalid("Wwise attenuation fields did not consume its object")); }
    Ok(Attenuation { id, cone, curve_indices, curves, rtpcs })
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
        let override_parent = self.u8()? != 0;
        node.effects = self.effects(override_parent)?;
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
        node.positioning_flags = positioning;
        if positioning & 1 != 0 && positioning & 8 != 0 {
            let flags = self.u8()?;
            node.positioning_3d = Some(Positioning3d { flags, attenuation: self.u32()? });
            if flags & 3 != 1 {
                self.take(5)?; // Automation path mode and transition time.
                let vertices = self.u32()? as usize;
                self.records(vertices, 16)?;
                let paths = self.u32()? as usize;
                self.records(paths, 20)?; // Vertex spans and XYZ ranges.
            }
        }
        node.auxiliary_flags = self.u8()?;
        if node.auxiliary_flags & 8 != 0 {
            for bus in &mut node.auxiliary_buses { *bus = self.u32()?; }
        }
        self.take(6)?; // Voice limit, virtualization and analysis settings.
        self.states()?;
        node.curves = self.rtpcs()?;
        if feedback { self.u32()?; }
        Ok(node)
    }

    /// 1411204d0: the bypass mask follows a nonempty slot list's count.
    fn effects(&mut self, override_parent: bool) -> io::Result<EffectChain> {
        let count = self.u8()?;
        let bypass = if count == 0 { 0 } else { self.u8()? };
        let mut slots = Vec::new();
        for _ in 0..count {
            slots.push(EffectReference { slot: self.u8()?, id: self.u32()?,
                shared: self.u8()? != 0, flags: self.u8()? });
        }
        Ok(EffectChain { override_parent, bypass, slots })
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

pub(super) fn bus_properties(object: &[u8], feedback: bool, auxiliary: bool) -> io::Result<NodeProperties> {
    let mut reader = Reader { bytes: object, offset: 0 };
    let mut node = NodeProperties { id: reader.u32()?, parent: reader.u32()?, ..Default::default() };
    node.values = reader.properties()?;
    // 1411205f0 follows the property bundle with flags, limits, channel
    // configuration and HDR; 141120750 then reads recovery/duck settings.
    let flags = reader.u8()?;
    let limit_flags = reader.u8()?;
    let max_instances = reader.u16()? & 0x3ff;
    let mut channel_config = reader.u32()?;
    if channel_config & 0xf00 == 0x100 {
        let mask = (channel_config >> 12) & 0x3ff3f;
        channel_config = (mask << 12) | 0x100 | mask.count_ones();
    }
    let hdr_flags = reader.u8()?;
    let recovery_ms = reader.u32()?;
    let max_duck_volume = reader.f32()?;
    let ducks = reader.u32()? as usize;
    let mut duck_settings = Vec::<BusDuck>::new();
    for _ in 0..ducks {
        let duck = BusDuck { bus: reader.u32()?, volume: reader.f32()?,
            fade_out_ms: reader.u32()?, fade_in_ms: reader.u32()?, curve: reader.u8()?, property: reader.u8()? };
        // 14111ff70 replaces a target's definition, retaining its list slot.
        if let Some(previous) = duck_settings.iter_mut().find(|previous| previous.bus == duck.bus) { *previous = duck; }
        else { duck_settings.push(duck); }
    }
    node.effects = reader.effects(true)?;
    node.bus_settings = Some(BusSettings { auxiliary, flags, limit_flags, max_instances,
        channel_config, hdr_flags, recovery_ms, max_duck_volume, ducks: duck_settings,
        mixer_id: reader.u32()?, mixer_shared: reader.u8()? != 0, attachment_override: reader.u8()? & 1 != 0 });
    node.curves = reader.rtpcs()?;
    reader.states()?;
    if feedback { reader.u32()?; }
    if reader.offset != object.len() { return Err(invalid("Wwise bus fields did not consume its object")); }
    Ok(node)
}

/// Both FX ShareSet (18) and custom FX (19) call 1411278b0. Parameter bytes
/// belong to the registered plug-in; retain them intact until its reader runs.
pub(super) fn effect_definition(object: &[u8], shared: bool) -> io::Result<EffectDefinition> {
    let mut reader = Reader { bytes: object, offset: 0 };
    let id = reader.u32()?;
    let plugin = reader.u32()?;
    let length = reader.u32()? as usize;
    let parameters = reader.take(length)?.to_vec();
    let mut media = Vec::<(u8, u32)>::new();
    let count = reader.u8()?;
    for _ in 0..count {
        let key = reader.u8()?;
        let value = reader.u32()?;
        if let Some(entry) = media.iter_mut().find(|entry| entry.0 == key) { entry.1 = value; }
        else { media.push((key, value)); }
    }
    let curves = reader.rtpcs()?;
    let mut values = Vec::<(u8, u32)>::new();
    let count = reader.u16()?;
    for _ in 0..count {
        let key = reader.u8()?;
        let value = reader.u32()?;
        if let Some(entry) = values.iter_mut().find(|entry| entry.0 == key) { entry.1 = value; }
        else { values.push((key, value)); }
    }
    if reader.offset != object.len() { return Err(invalid("Wwise effect fields did not consume its object")); }
    Ok(EffectDefinition { id, shared, plugin, parameters, media, curves, values })
}

pub(super) fn global_parameters(bytes: &[u8]) -> io::Result<(f32, std::collections::HashMap<u32, f32>, Vec<StateGroup>)> {
    let mut reader = Reader { bytes, offset: 0 };
    let threshold = reader.f32()?;
    reader.u16()?; // Voice limit.
    let states = reader.u32()?;
    let mut state_groups = Vec::new();
    for _ in 0..states {
        let id = reader.u32()?;
        let milliseconds = reader.u32()?;
        let count = reader.u32()?;
        let mut transitions = Vec::new();
        for _ in 0..count {
            transitions.push(StateTransition { from: reader.u32()?, to: reader.u32()?, milliseconds: reader.u32()? });
        }
        state_groups.push(StateGroup { id, milliseconds, transitions });
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
    Ok((threshold, parameters, state_groups))
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
            // 14111aed0 / 141118eb0 append playlist IDs independently of
            // the child list. 141119b70 resolves them in the global node
            // registry, so membership in this serialized list is not required.
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
