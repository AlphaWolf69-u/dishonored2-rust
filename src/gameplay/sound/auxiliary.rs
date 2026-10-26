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
