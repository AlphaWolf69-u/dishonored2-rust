//! Observable5fef60 and AttentionSource5d90f0 declaration projections.
//! These immutable fields serve the real component startup; NPC perception
//! consumers request their own typed declaration fields when implemented.
use crate::gameplay::options::decl::{atom, boolean, float};

#[derive(Debug)]
pub struct ObservableDefinition {
    pub category: u8,
    pub visibility: u8,
    pub visible_distance: f32,
}
#[derive(Debug)]
pub struct AttentionDefinition {
    pub default_meter: i32,
    pub passively_handled: bool,
    pub handling_time: f32,
    pub disable_observable_on_handling: bool,
}
fn enumeration(
    fields: &[(String, crate::entities::Value)],
    field: &str,
    names: &[&str],
    default: i32,
) -> Result<i32, String> {
    let Some(value) = atom(fields, field) else {
        return Ok(default);
    };
    if let Some(index) = names.iter().position(|name| *name == value) {
        return Ok(index as i32);
    }
    value
        .parse()
        .map_err(|_| format!("unknown {field} value {value}"))
}
impl ObservableDefinition {
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        //Installed reflection142396de8..ea8 and5ff520's actual dispatch.
        let category = enumeration(
            fields,
            "m_obsType",
            &[
                "OBS_TYPE_PLAYER",
                "OBS_TYPE_NPC",
                "OBS_TYPE_CUE",
                "OBS_TYPE_OBJECT",
                "OBS_TYPE_MESMERIZE",
                "OBS_TYPE_BLOODFLYSWARM",
                "OBS_TYPE_RATSWARM",
                "OBS_TYPE_LIMB",
            ],
            3,
        )?;
        let visibility = enumeration(
            fields,
            "m_obsVis",
            &[
                "OBS_VIS_ORIGIN",
                "OBS_VIS_BOUNDS_CENTER",
                "OBS_VIS_PLAYER",
                "OBS_VIS_NPC",
                "OBS_VIS_MESMERIZE",
                "OBS_VIS_POINTS_LIST",
                "OBS_VIS_SWARM",
            ],
            0,
        )?;
        if !(0..8).contains(&category) || !(0..7).contains(&visibility) {
            return Err("Observable enum outside actual range".into());
        }
        Ok(Self {
            category: category as u8,
            visibility: visibility as u8,
            visible_distance: float(fields, "m_visibleDistance", -1.),
        })
    }
}
impl AttentionDefinition {
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        //Installed reflection142390b00 and Init5d9430's source fallback.
        let default_meter = match atom(fields, "m_defaultMeterToRaise") {
            None | Some("ATM_NONE") => -1,
            Some("ATM_PLAYER") => 0,
            Some("ATM_NPCS") => 1,
            Some(value) => value
                .parse()
                .map_err(|_| format!("unknown attention meter {value}"))?,
        };
        if !(-1..=1).contains(&default_meter) {
            return Err("Attention meter outside actual range".into());
        }
        Ok(Self {
            default_meter,
            passively_handled: boolean(fields, "m_sourceCanbePassivelyHandled", false),
            handling_time: float(fields, "m_timeTobeHandled", 0.),
            disable_observable_on_handling: boolean(fields, "m_disableObservableOnHandling", false),
        })
    }
}
