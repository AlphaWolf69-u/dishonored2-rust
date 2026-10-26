//! Authored environment FX descriptors. Serialized fields are partial edits of
//! 21a000's defaults, including individual entries in existing curve arrays.
use super::entities::Value;
use crate::gameplay::effects::{curve::*, environment::EnvironmentEffectDefinition};
pub mod particle_system;
pub mod particles;
pub mod screen_quad;

pub struct EnvironmentEffectAsset {
    pub name: String,
    pub enabled: bool,
    pub environment: Option<String>,
    pub blend: EnvironmentEffectDefinition,
}

/// The location enum's literal table is initialized by 1419fbd70. Eligibility
/// for the current view is decided by the parent FX owner, not by this loader.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EffectGroupLocation {
    #[default]
    Everywhere,
    OutOfWater,
    Underwater,
}

pub struct EnvironmentGroupAsset {
    pub name: String,
    pub enabled: bool,
    pub location: EffectGroupLocation,
    pub timings: crate::gameplay::effects::timing::EffectTimingRanges,
    pub environments: Vec<EnvironmentEffectAsset>,
}
impl Default for EnvironmentGroupAsset {
    fn default() -> Self {
        // 2174a0 resets group ranges independently of StandardTimings: group
        // duration is one, while delay, fade-in and fade-out are zero.
        Self {
            name: String::new(),
            enabled: true,
            location: EffectGroupLocation::Everywhere,
            timings: crate::gameplay::effects::timing::EffectTimingRanges {
                duration: [1.; 2],
                ..Default::default()
            },
            environments: Vec::new(),
        }
    }
}

type Fields = [(String, Value)];
fn scope(value: &Value) -> Result<&Fields, String> {
    match value {
        Value::Scope(fields) => Ok(fields),
        _ => Err("FX field requires a scope".into()),
    }
}
fn atom(value: &Value) -> Result<&str, String> {
    match value {
        Value::Atom(value) => Ok(value.trim_matches('"')),
        _ => Err("FX field requires an atom".into()),
    }
}
fn number(value: &Value) -> Result<f32, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid FX float: {e}"))
}
fn boolean(value: &Value) -> Result<bool, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid FX boolean: {e}"))
}
fn bounds(pair: &mut [f32; 2], fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_max" => pair[0] = number(value)?,
            "m_min" => pair[1] = number(value)?,
            _ => return Err(format!("unknown FX range field {name}")),
        }
    }
    Ok(())
}
fn float_list(values: &mut Vec<f32>, fields: &Fields) -> Result<(), String> {
    edit_list(values, fields, 0., |slot, value| {
        *slot = number(value)?;
        Ok(())
    })
}
fn edit_list<T: Clone>(
    values: &mut Vec<T>,
    fields: &Fields,
    initial: T,
    mut edit: impl FnMut(&mut T, &Value) -> Result<(), String>,
) -> Result<(), String> {
    let mut edited = values.clone();
    if let Some((_, count)) = fields.first().filter(|(name, _)| name == "num") {
        let count = atom(count)?
            .parse::<usize>()
            .map_err(|e| format!("FX array count: {e}"))?;
        // 98c270 -> 14d230 classifies float as a zero-initialized element;
        // 9979c0 preserves old entries and clears newly exposed storage.
        edited.resize(count, initial);
    }
    for (position, (name, value)) in fields.iter().enumerate() {
        if name == "num" {
            if position == 0 {
                continue;
            }
            return Err("FX array count must precede its elements".into());
        }
        let index = name
            .strip_prefix("item[")
            .and_then(|n| n.strip_suffix(']'))
            .ok_or_else(|| format!("unknown FX array field {name}"))?
            .parse::<usize>()
            .map_err(|e| format!("FX array index: {e}"))?;
        let slot = edited
            .get_mut(index)
            .ok_or_else(|| format!("FX array index {index} exceeds declared count"))?;
        edit(slot, value)?;
    }
    *values = edited;
    Ok(())
}
fn curve(curve: &mut EffectFloatCurve, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_times" => float_list(&mut curve.times, scope(value)?)?,
            "m_values" => float_list(&mut curve.values, scope(value)?)?,
            // 1cbd50 samples the packed arrays directly; these generic curve
            // settings/tangent caches are not read by the scalar FX sampler.
            "m_loopMode" | "m_interpolationMode" | "m_tangentsIn" | "m_tangentsOut" => {}
            _ => return Err(format!("unknown FX curve field {name}")),
        }
    }
    Ok(())
}
fn curves(pair: &mut EffectFloatCurvePair, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_maxCurve" => curve(&mut pair.maximum, scope(value)?)?,
            "m_minCurve" => curve(&mut pair.minimum, scope(value)?)?,
            _ => return Err(format!("unknown FX curve-pair field {name}")),
        }
    }
    Ok(())
}

/// Load one `m_envOverrides/item[N]`. The parent group retains ownership of its
/// own timing, activation and visibility mask; this loader does not start it.
pub fn environment_effect(fields: &Fields) -> Result<EnvironmentEffectAsset, String> {
    let mut effect = EnvironmentEffectAsset {
        name: String::new(),
        enabled: true,
        environment: None,
        blend: EnvironmentEffectDefinition::default(),
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => effect.name = atom(value)?.to_owned(),
            "m_enabled" => effect.enabled = boolean(value)?,
            "m_decl" => {
                let name = atom(value)?;
                effect.environment = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                    .then(|| name.to_owned());
            }
            "m_timings" => {
                for (name, value) in scope(value)? {
                    let timing = &mut effect.blend.timing;
                    match name.as_str() {
                        "m_looping" => effect.blend.looping = boolean(value)?,
                        "m_finishLoopOnStop" => {
                            effect.blend.keep_duration_on_stop = boolean(value)?
                        }
                        "m_delay" => bounds(&mut timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX timing field {name}")),
                    }
                }
            }
            "m_blend" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_max" => effect.blend.scale[0] = number(value)?,
                        "m_min" => effect.blend.scale[1] = number(value)?,
                        "m_durationCurve" => curves(&mut effect.blend.duration, scope(value)?)?,
                        "m_fadeinCurve" => curves(&mut effect.blend.fade_in, scope(value)?)?,
                        "m_fadeoutCurve" => curves(&mut effect.blend.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX blend field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown environment FX field {name}")),
        }
    }
    Ok(effect)
}

/// Load the environment portion of one `m_groups/item[N]`. Other FX families
/// remain owned by their loaders/schedulers and may consume the shared RNG
/// before environment effects are emitted. This does not start a whole group.
pub fn environment_group(fields: &Fields) -> Result<EnvironmentGroupAsset, String> {
    let mut group = EnvironmentGroupAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => group.name = atom(value)?.to_owned(),
            "m_enabled" => group.enabled = boolean(value)?,
            "m_conditions" => {
                for (name, value) in scope(value)? {
                    if name != "m_location" {
                        return Err(format!("unknown FX group condition {name}"));
                    }
                    group.location = match atom(value)? {
                        "EVERYWHERE" | "0" => EffectGroupLocation::Everywhere,
                        "OUT_OF_WATER" | "1" => EffectGroupLocation::OutOfWater,
                        "UNDERWATER" | "2" => EffectGroupLocation::Underwater,
                        other => return Err(format!("unknown FX group location {other}")),
                    };
                }
            }
            "m_timings" => {
                for (name, value) in scope(value)? {
                    let range = match name.as_str() {
                        "m_delay" => &mut group.timings.delay,
                        "m_fadein" => &mut group.timings.fade_in,
                        "m_duration" => &mut group.timings.duration,
                        "m_fadeout" => &mut group.timings.fade_out,
                        _ => return Err(format!("unknown FX group timing {name}")),
                    };
                    bounds(range, scope(value)?)?;
                }
            }
            "m_envOverrides" => {
                let entries = scope(value)?;
                for (position, (name, value)) in entries.iter().enumerate() {
                    if name == "num" {
                        if position != 0 {
                            return Err("FX descriptor count must precede its elements".into());
                        }
                        let count = atom(value)?
                            .parse::<usize>()
                            .map_err(|e| format!("FX descriptor count: {e}"))?;
                        group.environments.truncate(count);
                        while group.environments.len() < count {
                            group.environments.push(environment_effect(&[])?);
                        }
                    } else {
                        let index = name
                            .strip_prefix("item[")
                            .and_then(|n| n.strip_suffix(']'))
                            .ok_or_else(|| format!("unknown FX descriptor list field {name}"))?
                            .parse::<usize>()
                            .map_err(|e| format!("FX descriptor index: {e}"))?;
                        let entry = group.environments.get_mut(index).ok_or_else(|| {
                            format!("FX descriptor {index} exceeds declared count")
                        })?;
                        *entry = environment_effect(scope(value)?)?;
                    }
                }
            }
            _ => {} // Other effect families are not environment descriptors.
        }
    }
    Ok(group)
}

#[cfg(test)]
#[path = "fx/tests.rs"]
mod tests;
