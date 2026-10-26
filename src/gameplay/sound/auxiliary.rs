//! Acoustic-zone game sends (1400c3860/1400c4f70).
use crate::assets::sound_propagation::Propagation;
use super::paths::Path;
use std::collections::HashMap;

/// Declaration references retained by acoustic zones (1400b9eb0).
pub struct Buses(HashMap<String, Option<u32>>);
impl Buses {
    pub fn load(graph: &Propagation, mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Self, String> {
        let mut buses = HashMap::new();
        for zone in &graph.zones {
            let name = &zone.auxiliary_bus;
            if buses.contains_key(name) { continue; }
            let bus = if name.is_empty() || name.eq_ignore_ascii_case("NULL") { None } else {
                let path = format!("generated/decls/soundauxbus/{name}.decl");
                let source = read(&path)?.ok_or_else(|| format!("missing auxiliary bus declaration {path}"))?;
                Some(crate::assets::wwise::declared_auxiliary_id(&source).map_err(|error| format!("{path}: {error}"))?)
            };
            buses.insert(name.clone(), bus);
        }
        Ok(Self(buses))
    }

    pub fn resolve(&self, name: &str) -> Result<Option<u32>, String> {
        self.0.get(name).copied().ok_or_else(|| format!("unresolved auxiliary bus {name}"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Send {
    pub bus: u32,
    pub gain: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendKind { Game, User }

#[derive(Clone, Copy, Debug)]
pub struct ControlSend {
    pub bus: u32,
    pub gain: f32,
    pub kind: SendKind,
}

/// The four authored user-send gains are in dB; game-object sends are linear.
#[derive(Clone, Copy, Debug)]
pub struct UserSend {
    pub bus: u32,
    pub decibels: f32,
}

/// 141103810: game sends precede user sends, with independent admission
/// thresholds. The input lists are the four slots of each native owner.
pub fn controls(game: &[Send], game_enabled: bool, game_decibels: f32,
    user: &[UserSend]) -> Result<Vec<ControlSend>, String>
{
    if game.len() > 4 || user.len() > 4 {
        return Err("auxiliary send owner exceeds four slots".into());
    }
    let mut result = Vec::with_capacity(8);
    if game_enabled {
        let gain = super::mix::power10(game_decibels * 0.05);
        for send in game {
            if send.bus == 0 { break; }
            let gain = gain * send.gain;
            // DAT_142427a50; ordered comparison also excludes NaN.
            if gain > f32::from_bits(0x37800000) {
                result.push(ControlSend { bus: send.bus, gain, kind: SendKind::Game });
            }
        }
    }
    for send in user {
        // DAT_142427a54. A zero user bus skips its slot, rather than
        // terminating the list as it does for game-object sends.
        if send.bus != 0 && send.decibels > f32::from_bits(0xc2c0999a) {
            result.push(ControlSend { bus: send.bus,
                gain: super::mix::power10(send.decibels * 0.05), kind: SendKind::User });
        }
    }
    Ok(result)
}

/// 1410fc0f0 adds all matching control entries in their stored order.
/// 1410ec0e0 creates one connection to each bus/device, retaining the first
/// connection's kind. Repeated zones therefore add gains before bus DSP.
pub fn bus_sends(controls: &[ControlSend]) -> Vec<ControlSend> {
    let mut buses: Vec<ControlSend> = Vec::new();
    for send in controls.iter().take(8).take_while(|send| send.bus != 0) {
        if let Some(bus) = buses.iter_mut().find(|bus| bus.bus == send.bus) {
            bus.gain += send.gain;
        } else {
            buses.push(ControlSend { gain: 0.0 + send.gain, ..*send });
        }
    }
    buses
}

/// Rebuild the object's send list in reverse zone-ring order. Repeated bus
/// IDs are retained: c4f70 appends and leaves reconciliation to Wwise.
pub fn project(graph: &Propagation, path: Option<&Path>, enabled: bool, detail: i32,
    mut resolve: impl FnMut(&str) -> Result<Option<u32>, String>) -> Result<Vec<Send>, String>
{
    let mut sends = Vec::new();
    let Some(path) = path.filter(|_| enabled) else { return Ok(sends) };
    let transmission = 1.0 - path.occlusion;
    // DAT_141ae8a78 is 0x34000000; COMISS/JBE rejects unordered values.
    if !(transmission > f32::EPSILON) { return Ok(sends); }
    let distance = f32::EPSILON + path.distance;
    let count = (detail.max(0) as usize).min(path.zone_count);
    let mut slot = path.last_zone_slot;
    for _ in 0..count {
        let segment = path.zones.get(slot).and_then(|segment| *segment)
            .ok_or("auxiliary send references an absent path segment")?;
        let zone = graph.zones.get(segment.zone).ok_or("auxiliary send references a missing zone")?;
        if let Some(bus) = resolve(&zone.auxiliary_bus)? {
            let ratio = segment.distance / distance;
            let mut weight = if ratio <= 1.0 { ratio * ratio } else { 1.0 };
            let minimum = f32::from_bits(zone.properties[0]); // 1400b9c70
            if !(minimum <= weight) { weight = minimum; }
            sends.push(Send { bus, gain: weight * transmission });
        }
        slot = slot.wrapping_sub(1) & 3;
    }
    Ok(sends)
}
