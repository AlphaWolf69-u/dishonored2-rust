//! arkFXExplosionDesc21a6e0 / reflected m_explosionDecl(+28), group+110.
use super::{Fields, atom, boolean, bounds, scope};
use crate::{assets::entities::Value, gameplay::effects::standard_events::StandardEventTimings};

#[derive(Debug)]
pub struct ExplosionAsset {
    pub name: String,
    pub enabled: bool,
    pub timings: StandardEventTimings,
    pub explosion: Option<String>,
}

pub(crate) fn standard_timings(fields: &Fields) -> Result<StandardEventTimings, String> {
    let mut timings = StandardEventTimings::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_delay" => bounds(&mut timings.delay, scope(value)?)?,
            "m_period" => bounds(&mut timings.period, scope(value)?)?,
            "m_cycles" => {
                for (name, value) in scope(value)? {
                    let index = match name.as_str() {
                        "m_max" => 0,
                        "m_min" => 1,
                        _ => return Err(format!("unknown standard-event cycle field {name}")),
                    };
                    timings.cycles[index] = atom(value)?
                        .parse::<i32>()
                        .map_err(|error| format!("standard-event cycles:{error}"))?;
                }
            }
            _ => return Err(format!("unknown standard-event timing field {name}")),
        }
    }
    Ok(timings)
}

pub(crate) fn explosion(fields: &[(String, Value)]) -> Result<ExplosionAsset, String> {
    let mut asset = ExplosionAsset {
        name: String::new(),
        enabled: true,
        timings: Default::default(),
        explosion: None,
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => asset.name = atom(value)?.to_owned(),
            "m_enabled" => asset.enabled = boolean(value)?,
            "m_timings" => asset.timings = standard_timings(scope(value)?)?,
            "m_explosionDecl" => {
                asset.explosion = Some(atom(value)?)
                    .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                    .map(str::to_owned)
            }
            _ => return Err(format!("unknown FX explosion field {name}")),
        }
    }
    Ok(asset)
}
