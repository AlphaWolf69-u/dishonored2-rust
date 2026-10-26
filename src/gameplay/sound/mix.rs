//! Wwise version-113 property curves and the authored output-bus hierarchy.
use crate::assets::wwise::{BankMix, MediaProperties, NodeProperties, RtpcCurve};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The logarithm approximation in conversion-table scaling (1410cb000).
pub(super) fn ln(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x7fffff) + 0x3f800000);
    let t = (mantissa - 1.0) / (mantissa + 1.0);
    (t * t * 0.33333334 + 1.0) * t * 2.0 + (((bits >> 23) & 0xff) as f32 - 127.0) * 0.6931472
}

/// 1410cb000 / 1410ca700: base-10 exponent approximation, including the
/// underflow branch. Constants were read from the installed executable.
pub fn power10(value: f32) -> f32 {
    if value < -37.0 { return 0.0; }
    let bits = (value * 27_866_352.0 + 1_065_353_216.0) as i64 as u32;
    power_from_bits(bits)
}

pub(crate) fn power_from_bits(bits: u32) -> f32 {
    let mantissa = f32::from_bits((bits & 0x7fffff) + 0x3f800000);
    ((mantissa * 0.32518977 + 0.020805772) * mantissa + 0.65304345) * f32::from_bits(bits & 0xff800000)
}

/// CAkConversionTable::ApplyCurveScaling, 1410cb000. Positive dB scaling
/// uses -log(1-v), while negative scaling uses log(1+v).
fn scale(value: f32, scaling: u8) -> f32 {
    match scaling {
        2 if value >= 1.0 => 4096.0,
        2 if value >= 0.0 => ln(1.0 - value) * -0.4342945 * 20.0,
        2 if value <= -1.0 => -4096.0,
        2 => ln(value + 1.0) * 0.4342945 * 20.0,
        3 => power10(value),
        4 => power10(value * 0.05),
        _ => value,
    }
}

/// AkInterpolation::InterpolateNoCheck, 1410cd3b0. Constant segments are
/// handled by the conversion table without calling the interpolation helper.
pub(crate) fn interpolate(mut t: f32, a: f32, b: f32, mode: u32) -> f32 {
    match mode {
        0 => { t = 1.0 - t; t * t * t * (a - b) + b }
        1 => {
            t *= std::f32::consts::FRAC_PI_2;
            let x = t * t;
            (((0.008306325 - x * 0.00018363654) * x - 0.16664828) * x + 0.9999966) * t * (b - a) + a
        }
        2 => (t - 3.0) * t * 0.5 * (a - b) + a,
        3 => {
            let upper = t > 0.5;
            let angle = if upper { std::f32::consts::PI - t * std::f32::consts::PI } else { t * std::f32::consts::PI };
            let x = angle * angle;
            let v = (((0.0041531627 - x * 0.00009181827) * x - 0.08332414) * x + 0.4999983) * angle;
            (if upper { 1.0 - v } else { v }) * (b - a) + a
        }
        4 => (b - a) * t + a,
        5 => {
            let x = t * std::f32::consts::PI * t * std::f32::consts::PI;
            (((x * 0.000484834 - 0.01961384) * x + 0.2476748) * x + 0.00069670216) * (b - a) + a
        }
        6 => (t + 1.0) * t * 0.5 * (b - a) + a,
        7 => {
            let x = t * std::f32::consts::FRAC_PI_2 * t * std::f32::consts::FRAC_PI_2;
            (((0.04148775 - x * 0.0012712094) * x - 0.49991244) * x + 0.9999933) * (a - b) + b
        }
        8 => t * t * t * (b - a) + a,
        9 => a,
        _ => 0.0,
    }
}

pub fn curve_value(curve: &RtpcCurve, input: f32) -> Option<f32> {
    conversion_value(&curve.points, curve.scaling, input)
}

pub fn conversion_value(points: &[crate::assets::wwise::CurvePoint], scaling: u8, input: f32) -> Option<f32> {
    let first = points.first()?;
    let last = points.last()?;
    let value = if input <= first.input {
        first.output
    } else if input >= last.input {
        last.output
    } else {
        let upper = points.partition_point(|point| point.input <= input);
        let (a, b) = (&points[upper - 1], &points[upper]);
        interpolate((input - a.input) / (b.input - a.input), a.output, b.output, a.interpolation)
    };
    Some(scale(value, scaling))
}

pub struct Mixer {
    pub buses: HashMap<u32, NodeProperties>,
    pub effects: HashMap<(bool, u32), crate::assets::wwise::EffectDefinition>,
    pub parameters: HashMap<u32, f32>,
    pub volume_threshold_db: f32,
    pub environment: crate::assets::wwise::EnvironmentSettings,
    pub primary_bus: Option<u32>,
    pub bus_order: Vec<u32>,
    pub activity: Option<Arc<super::activity::Activity>>,
}

/// Voice properties before a mixing bus (141104c30/1410dbb30). Makeup
/// gain is kept separate because 1410fcd00 applies it to the source stage.
#[derive(Clone, Copy)]
pub struct VoiceRouting {
    pub volume_db: f32,
    pub makeup_db: f32,
    pub dry_db: f32,
    pub output_filters: [f32; 2],
    pub bus: u32,
}

impl From<BankMix> for Mixer {
    fn from(bank: BankMix) -> Self {
        let bus_order = bank.buses.iter().map(|bus| bus.id).collect();
        Self { buses: bank.buses.into_iter().map(|bus| (bus.id, bus)).collect(), parameters: bank.parameters,
            effects: bank.effects.into_iter().map(|effect| ((effect.shared, effect.id), effect)).collect(),
            volume_threshold_db: bank.volume_threshold_db, environment: bank.environment, primary_bus: bank.primary_bus, bus_order, activity: None }
    }
}

impl Mixer {
    /// 1410df700 selects the nearest positioning override or root.
    /// 141112160 uses static X/Y only if neither axis has an RTPC; once
    /// either is subscribed, an unsubscribed axis is zero. These controls
    /// replace the base property, unlike the additive voice parameters.
    pub fn panner_2d(&self, source: &MediaProperties, parameters: &[(u32, f32)]) -> Result<super::panning::Panner2d, String> {
        let Some(node) = source.nodes.iter().find(|node| node.positioning_flags & 1 != 0 || node.parent == 0) else {
            return Ok(super::panning::Panner2d { left_right: 0.5, front_rear: 0.5, center: 0.0, enabled: false });
        };
        let subscribed = |property| node.curves.iter().any(|curve| curve.property == property);
        let axes = if subscribed(0x14) || subscribed(0x15) {
            [self.rtpc_contribution(node, 0x14, parameters)?, self.rtpc_contribution(node, 0x15, parameters)?]
        } else { [node.float(0xb), node.float(0xc)] };
        let center = if subscribed(0xb) { self.rtpc_contribution(node, 0xb, parameters)? } else { node.float(0xd) };
        let normalize = |value: f32| {
            let value = (value + 100.0) * 0.005;
            if 0.0 <= value { if 1.0 < value { 1.0 } else { value } } else { 0.0 }
        };
        Ok(super::panning::Panner2d { left_right: normalize(axes[0]), front_rear: normalize(axes[1]),
            center: center * 0.01, enabled: node.positioning_flags & 7 == 7 }) // 1410deec0
    }

    fn bus_chain(&self, mut id: u32) -> Result<Vec<&NodeProperties>, String> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        while id != 0 {
            if !visited.insert(id) { return Err("cyclic Wwise bus hierarchy".into()); }
            let bus = self.buses.get(&id).ok_or_else(|| format!("missing Wwise output bus {id:#010x}"))?;
            chain.push(bus);
            id = bus.parent;
        }
        Ok(chain)
    }

    /// 14111e660, param2=0: stop before a parent that mixes. The machine
    /// code at 14111e82d passes the parent to 14111f200 for this decision.
    pub fn bus_output_db(&self, id: u32) -> Result<f32, String> {
        self.bus_output_db_inner(id, true)
    }

    pub fn bus_output_base_db(&self, id: u32) -> Result<f32, String> { self.bus_output_db_inner(id, false) }

    fn bus_output_db_inner(&self, id: u32, ducking: bool) -> Result<f32, String> {
        let mut result = 0.0;
        for (index, bus) in self.bus_chain(id)?.into_iter().enumerate() {
            if index != 0 && self.bus_mixes(bus.id)? { break; }
            result += bus.float(5);
            result += self.rtpc_contribution(bus, 5, &[])?;
            if ducking {
                if let Some(activity) = &self.activity { result += activity.own_ducking(bus.id, 5)?; }
            }
        }
        Ok(result)
    }

    /// Resolve a logical bus to its next mixing ancestor. Its volume is
    /// folded into the incoming connection rather than processed twice.
    pub fn mixing_bus(&self, id: u32) -> Result<u32, String> {
        for bus in self.bus_chain(id)? { if self.bus_mixes(bus.id)? { return Ok(bus.id); } }
        Ok(0)
    }

    pub fn voice_routing(&self, source: &MediaProperties, parameters: &[(u32, f32)]) -> Result<VoiceRouting, String> {
        self.voice_routing_inner(source, parameters, true)
    }

    pub fn voice_routing_base(&self, source: &MediaProperties, parameters: &[(u32, f32)]) -> Result<VoiceRouting, String> {
        self.voice_routing_inner(source, parameters, false)
    }

    pub fn source_ducks(&self, source: &MediaProperties) -> Result<bool, String> {
        Ok(self.bus_chain(source.bus)?.iter().any(|bus| bus.bus_settings.as_ref().is_some_and(|settings| !settings.ducks.is_empty())))
    }

    fn voice_routing_inner(&self, source: &MediaProperties, parameters: &[(u32, f32)], ducking: bool) -> Result<VoiceRouting, String> {
        let mut volume_db = 0.0;
        let mut makeup_db = 0.0;
        for node in &source.nodes {
            volume_db += node.float(0);
            volume_db = self.rtpc_contribution(node, 0, parameters)? + volume_db;
            makeup_db += node.float(0x21);
            makeup_db = self.rtpc_contribution(node, 0x24, parameters)? + makeup_db;
        }
        // 14111e0d0 uses a global context for output-bus voice parameters;
        // a game-object override belongs only to the actor/container chain.
        for bus in self.bus_chain(source.bus)? {
            volume_db += bus.float(0);
            volume_db = self.rtpc_contribution(bus, 0, &[])? + volume_db;
            if ducking {
                if let Some(activity) = &self.activity { volume_db = activity.own_ducking(bus.id, 0)? + volume_db; }
            }
        }
        let mut dry_db = 0.0;
        let mut output_filters = [0.0; 2];
        // param8 in 1410dbb30 selects only the first explicit output bus.
        if let Some(node) = source.nodes.iter().find(|node| node.bus != 0) {
            dry_db = node.float(0x17) + self.rtpc_contribution(node, 0x3c, parameters)?;
            output_filters = [node.float(0x19) + self.rtpc_contribution(node, 0x3e, parameters)?,
                node.float(0x18) + self.rtpc_contribution(node, 0x3d, parameters)?];
        }
        if source.bus != 0 && !self.bus_mixes(source.bus)? {
            dry_db = self.bus_output_db_inner(source.bus, ducking)? + dry_db; // 141104e38
        }
        Ok(VoiceRouting { volume_db, makeup_db, dry_db, output_filters, bus: self.mixing_bus(source.bus)? })
    }

    /// 14111f200. Auxiliary buses have native type 0x0c (141131fe0,
    /// 140075bb0) and always mix. Ordinary buses may only group routing.
    pub fn bus_mixes(&self, id: u32) -> Result<bool, String> {
        let bus = self.buses.get(&id).ok_or_else(|| format!("missing Wwise bus {id:#010x}"))?;
        let settings = bus.bus_settings.as_ref().ok_or("Wwise bus has no bus settings")?;
        Ok(settings.auxiliary || bus.effects.slots.iter().any(|effect| effect.id != 0)
            || settings.channel_config & 0xff != 0 || settings.flags & 1 != 0
            || settings.hdr_flags & 1 != 0 || bus.parent == 0 || settings.mixer_id != 0)
    }

    /// Bank mask plus live bypass subscriptions (1410dcc40/1410dcd70).
    /// Slot bypass is RTPC 0x18..0x1b; bypass-all is 0x1c. A subscribed
    /// value replaces its corresponding static bit, using float != 0.
    pub fn bus_bypass(&self, bus: &NodeProperties, parameters: &[(u32, f32)]) -> Result<u8, String> {
        let mut mask = bus.effects.bypass;
        for slot in 0..=4u8 {
            if slot < 4 && !bus.effects.slots.iter().any(|effect| effect.slot == slot && effect.id != 0) { continue; }
            let property = slot + 0x18;
            if bus.curves.iter().any(|curve| curve.property == property) {
                let value = self.rtpc_contribution(bus, property, parameters)?;
                mask = (mask & !(1 << slot)) | (u8::from(value != 0.0) << slot);
            }
        }
        Ok(mask)
    }

    /// The nearest game/user override owns that category's complete settings
    /// (1410dbb30). The root supplies either category when no child overrides.
    pub fn auxiliary_controls(&self, source: &MediaProperties,
        game: &[super::auxiliary::Send], parameters: &[(u32, f32)])
        -> Result<Vec<super::auxiliary::ControlSend>, String>
    {
        let mut game_enabled = false;
        let mut game_decibels = 0.0;
        if let Some(node) = source.nodes.iter().find(|node| node.auxiliary_flags & 1 != 0 || node.parent == 0) {
            game_enabled = node.auxiliary_flags & 2 != 0;
            game_decibels = node.float(0x16) + self.rtpc_contribution(node, 0x13, parameters)?;
        }
        let mut user = Vec::new();
        if let Some(node) = source.nodes.iter().find(|node| node.auxiliary_flags & 4 != 0 || node.parent == 0) {
            // The native array is allocated only when at least one bus is
            // nonzero. An empty override still prevents parent inheritance.
            if node.auxiliary_buses.iter().any(|bus| *bus != 0) {
                for (slot, &bus) in node.auxiliary_buses.iter().enumerate() {
                    user.push(super::auxiliary::UserSend { bus,
                        decibels: node.float(0x12 + slot as u8) + self.rtpc_contribution(node, 0x38 + slot as u8, parameters)? });
                }
            }
        }
        super::auxiliary::controls(game, game_enabled, game_decibels, &user)
    }

    /// Subscription evaluation (1410d1480/1410cf060/1410cf230). Authored
    /// properties are added by their callers, after the curve fold.
    fn rtpc_contribution(&self, node: &NodeProperties, rtpc: u8,
        parameters: &[(u32, f32)]) -> Result<f32, String>
    {
        let mut contribution = None;
        let mut accumulation = None;
        for curve in node.curves.iter().filter(|curve| curve.property == rtpc) {
            if curve.kind != 0 { return Err(format!("node {:#010x} has a non-game-parameter curve", node.id)); }
            let input = parameters.iter().find(|(id, _)| *id == curve.parameter).map(|(_, value)| *value)
                .or_else(|| self.parameters.get(&curve.parameter).copied())
                .ok_or_else(|| format!("missing Wwise parameter default {:#010x}", curve.parameter))?;
            let next = curve_value(curve, input).ok_or("empty Wwise parameter curve")?;
            if accumulation.is_some_and(|mode| mode != curve.accumulation) {
                return Err("Wwise curves disagree on subscription accumulation".into());
            }
            accumulation = Some(curve.accumulation);
            // 1410d1480 selects 1410cf230 for multiplicative subscriptions,
            // otherwise 1410cf060 sums them. 1410dbb30 then adds that result
            // to the authored property; the base is not part of the fold.
            contribution = Some(if curve.accumulation == 2 {
                contribution.unwrap_or(1.0) * next
            } else { contribution.unwrap_or(0.0) + next });
        }
        Ok(contribution.unwrap_or(0.0))
    }

    /// Source pitch (2), LPF (3), and HPF (4), accumulated through the
    /// source hierarchy by 1410dbb30. Output-bus filters are a separate stage.
    pub fn source_property(&self, source: &MediaProperties, property: u8,
        parameter: Option<(u32, f32)>) -> Result<f32, String>
    {
        self.source_property_with_parameters(source, property, parameter.as_slice())
    }

    /// 14111b290 queries only this container's transition subscription (7).
    /// The result is seconds; authored/random milliseconds are added later.
    pub fn transition_seconds(&self, node: &NodeProperties, parameters: &[(u32, f32)]) -> Result<f32, String> {
        self.rtpc_contribution(node, 7, parameters)
    }

    pub fn source_property_with_parameters(&self, source: &MediaProperties, property: u8,
        parameters: &[(u32, f32)]) -> Result<f32, String>
    {
        if !matches!(property, 2 | 3 | 4) { return Err("unsupported Wwise source property".into()); }
        let mut result = 0.0;
        for node in &source.nodes {
            result += node.float(property);
            result = self.rtpc_contribution(node, property, parameters)? + result;
        }
        for bus in self.bus_chain(source.bus)? {
            result += bus.float(property);
            result = self.rtpc_contribution(bus, property, &[])? + result;
        }
        Ok(result)
    }

    pub fn volume_db(&self, source: &MediaProperties) -> Result<f32, String> {
        self.volume_db_with_parameter(source, None)
    }

    pub fn volume_db_with_parameter(&self, source: &MediaProperties, parameter: Option<(u32, f32)>) -> Result<f32, String> {
        self.volume_db_with_parameters(source, parameter.as_slice())
    }

    /// Game-object values override global/default values by parameter ID
    /// (1400c5c60 / 1410ca580). Distinct object RTPCs coexist on one voice.
    pub fn volume_db_with_parameters(&self, source: &MediaProperties, parameters: &[(u32, f32)]) -> Result<f32, String> {
        let routing = self.voice_routing(source, parameters)?;
        let mut volume = routing.volume_db + routing.makeup_db + routing.dry_db;
        for bus in self.bus_chain(routing.bus)? {
            if self.bus_mixes(bus.id)? { volume += self.bus_output_db(bus.id)?; }
        }
        Ok(volume)
    }

    pub fn gain(&self, source: &MediaProperties, random_db: f32) -> Result<f32, String> {
        self.gain_with_parameter(source, random_db, None)
    }

    pub fn gain_with_parameter(&self, source: &MediaProperties, random_db: f32, parameter: Option<(u32, f32)>) -> Result<f32, String> {
        self.gain_with_parameters(source, random_db, parameter.as_slice())
    }

    pub fn gain_with_parameters(&self, source: &MediaProperties, random_db: f32, parameters: &[(u32, f32)]) -> Result<f32, String> {
        let volume = self.volume_db_with_parameters(source, parameters)? + random_db;
        Ok(if volume <= self.volume_threshold_db { 0.0 } else { power10(volume * 0.05) })
    }
}
