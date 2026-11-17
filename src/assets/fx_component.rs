//! arkCpntFX parameters (4b16c0/4b1c00, reflection14170bdd0).
//! Runtime target/culprit IDs remain with the actual component generation.
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct Definition {
    pub effect: Option<Arc<str>>,
    pub target: Option<crate::gameplay::event_graph::EntityReference>,
    pub target_tag: Arc<str>,
    pub update_position: bool,
    pub dont_spawn_if_culled: bool,
    pub stop_on_target_removal: bool,
    pub stop_on_parent_removal: bool,
    pub remove_entity_on_end: bool,
}
impl Default for Definition {
    fn default() -> Self {
        //4b16c0 stores0x01010000 at params+18, then clears+1c.
        Self {
            effect: None,
            target: None,
            target_tag: Arc::from(""),
            update_position: false,
            dont_spawn_if_culled: false,
            stop_on_target_removal: true,
            stop_on_parent_removal: true,
            remove_entity_on_end: false,
        }
    }
}
impl Definition {
    ///1416f3df0 embeds the reflected14170bdd0 parameters at declaration+68.
    pub(crate) fn read(
        fields: &[(String, crate::assets::entities::Value)],
    ) -> Result<Self, String> {
        use crate::assets::entities::scene_motion::{atom, child};
        let fields = child(fields, "m_parameters");
        let flag = |key: &str, default| match atom(fields, key) {
            None => Ok(default),
            Some("true" | "1") => Ok(true),
            Some("false" | "0") => Ok(false),
            Some(value) => Err(format!("CpntFX {key}: invalid boolean {value}")),
        };
        let defaults = Self::default();
        Ok(Self {
            effect: atom(fields, "m_fx")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(Arc::from),
            target: atom(fields, "m_targetEntity")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(|value| {
                    crate::gameplay::event_graph::EntityReference::Named(value.to_owned())
                }),
            target_tag: Arc::from(atom(fields, "m_targetTagName").unwrap_or("")),
            update_position: flag("m_updatePosition", defaults.update_position)?,
            dont_spawn_if_culled: flag("m_dontSpawnIfCulled", defaults.dont_spawn_if_culled)?,
            stop_on_target_removal: flag("m_stopOnTargetRemoval", defaults.stop_on_target_removal)?,
            stop_on_parent_removal: flag("m_stopOnParentRemoval", defaults.stop_on_parent_removal)?,
            remove_entity_on_end: flag("m_removeEntityOnFxEnd", defaults.remove_entity_on_end)?,
        })
    }
}
