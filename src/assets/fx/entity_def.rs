//! arkFXEntityDefDesc219230/219270 and reflection14155c480.
use super::*;
pub struct EntityDefAsset {
    pub name: String,
    pub enabled: bool,
    pub timings: crate::gameplay::effects::standard_events::StandardEventTimings,
    pub use_owner_orientation: bool,
    pub worldspace: bool,
    pub offset: [[f32; 3]; 2],
    pub declarations: Vec<Option<String>>,
}
pub(crate) fn entity_def(fields: &Fields) -> Result<EntityDefAsset, String> {
    let mut result = EntityDefAsset {
        name: String::new(),
        enabled: true,
        timings: Default::default(),
        use_owner_orientation: true,
        worldspace: false,
        offset: [[0.; 3]; 2],
        declarations: Vec::new(),
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_timings" => result.timings = super::explosion::standard_timings(scope(value)?)?,
            "m_position" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_useOwnerOrientation" => result.use_owner_orientation = boolean(value)?,
                        "m_offset" => {
                            for (name, value) in scope(value)? {
                                match name.as_str() {
                                    "m_worldspace" => result.worldspace = boolean(value)?,
                                    "m_max" | "m_min" => {
                                        let lane = usize::from(name == "m_min");
                                        for (axis, value) in scope(value)? {
                                            let index = match axis.as_str() {
                                                "x" => 0,
                                                "y" => 1,
                                                "z" => 2,
                                                _ => {
                                                    return Err(format!(
                                                        "unknown FX entity offset axis {axis}"
                                                    ));
                                                }
                                            };
                                            result.offset[lane][index] = number(value)?;
                                        }
                                    }
                                    _ => {
                                        return Err(format!(
                                            "unknown FX entity offset field {name}"
                                        ));
                                    }
                                }
                            }
                        }
                        _ => return Err(format!("unknown FX entity position field {name}")),
                    }
                }
            }
            "m_decls" => {
                for (name, value) in scope(value)? {
                    if name == "num" {
                        let count = atom(value)?
                            .parse::<usize>()
                            .map_err(|_| "invalid FX entity declaration count")?;
                        result.declarations.resize(count, None);
                    } else {
                        let index = name
                            .strip_prefix("item[")
                            .and_then(|name| name.strip_suffix(']'))
                            .ok_or("invalid FX entity declaration index")?
                            .parse::<usize>()
                            .map_err(|_| "invalid FX entity declaration index")?;
                        let slot = result
                            .declarations
                            .get_mut(index)
                            .ok_or("FX entity declaration exceeds count")?;
                        let path = atom(value)?;
                        *slot = (!path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
                            .then(|| path.to_owned());
                    }
                }
            }
            _ => return Err(format!("unknown FX entity definition field {name}")),
        }
    }
    Ok(result)
}
