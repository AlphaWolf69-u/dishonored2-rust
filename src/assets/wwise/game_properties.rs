//! Game-side sound descriptors: constructor7e5480, reflection1415a56a0.
//! References share the ordinary catalog declaration lifecycle; no media lives here.
use crate::{
    archive::Catalog,
    assets::entities::{Value, player::resolved_decl, scene_motion::atom},
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct GameSoundProperties {
    pub context: i32,
    pub propagation_distance: f32,
    pub attention_meter: i32,
    pub call_for_help: bool,
    pub vertical_attenuation: bool,
}
impl GameSoundProperties {
    fn from_fields(fields: &[(String, Value)]) -> Result<Self, String> {
        // Enum registration1419eaf20, including explicit NONE at0.
        const CONTEXTS: &[&str] = &[
            "NONE",
            "ALARM",
            "AMBIENT",
            "BARK_THREATENED",
            "BARK_WOL_WARNING",
            "BARK_WOL_WITNESSDEATH",
            "BREAKAGE",
            "BREAKAGE_BIG",
            "BURNING",
            "COMBAT_RALLY",
            "DANGER",
            "DECOY",
            "EXPLOSION",
            "FLEE_RALLY",
            "FOOTSTEP_FALL",
            "FOOTSTEP_RUN",
            "FOOTSTEP_SPRINT",
            "FRY",
            "GUNSHOT",
            "HURT",
            "IMPACT",
            "MESMERIZE",
            "NEST_KEEPER",
            "SCREAM",
            "SEARCH_RALLY",
            "WATCHTOWER_ALARM",
            "LOW_FREQUENCY",
            "ANCIENT_HOWL",
        ];
        let context = match atom(fields, "m_aiContext") {
            None => 0,
            Some(value) => CONTEXTS
                .iter()
                .position(|name| value.strip_prefix("ARK_AI_SND_") == Some(*name))
                .map(|index| index as i32)
                .or_else(|| value.parse().ok())
                .ok_or_else(|| format!("unknown sound context {value}"))?,
        };
        let propagation_distance = atom(fields, "m_aiPropagationDistance")
            .map(str::parse::<f32>)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or(-1.);
        if !propagation_distance.is_finite() {
            return Err("nonfinite sound propagation distance".into());
        }
        // Attention enum1419eea50.
        let attention_meter = match atom(fields, "m_aiAttentionMeterOverride") {
            None | Some("ATM_NONE") => -1,
            Some("ATM_PLAYER") => 0,
            Some("ATM_NPCS") => 1,
            Some("ATM_MAX") => 2,
            Some(value) => value
                .parse()
                .map_err(|_| format!("invalid sound attention meter {value}"))?,
        };
        let boolean = |key, default| match atom(fields, key) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(_) => Err(format!("invalid sound property {key}")),
        };
        Ok(Self {
            context,
            propagation_distance,
            attention_meter,
            call_for_help: boolean("m_callForHelp", false)?,
            vertical_attenuation: boolean("m_enableVerticalAttenuationForAI", true)?,
        })
    }
}

pub fn acquire_with(
    catalog: &Catalog,
    name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Arc<GameSoundProperties>, String> {
    let path = format!(
        "generated/decls/gamesoundproperties/{}.decl",
        name.replace('\\', "/").to_ascii_lowercase()
    );
    let shared = !catalog.has_overlays();
    catalog
        .game_sound_properties
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "game sound property registry poisoned")?
        .acquire_with_sharing(&path, 0, shared, |_| {
            let _tag = crate::platform::allocation_tags::tag("Game sound properties");
            GameSoundProperties::from_fields(&resolved_decl(
                "gamesoundproperties",
                name,
                &mut read,
            )?)
            .map(Arc::new)
        })
}
pub(crate) fn register_preload(
    catalog: &Catalog,
    path: &str,
    marker: u8,
    persistent: bool,
) -> Result<(), String> {
    catalog
        .game_sound_properties
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "game sound property registry poisoned")?
        .register(
            &path.replace('\\', "/").to_ascii_lowercase(),
            marker,
            persistent,
        );
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.game_sound_properties.get() {
        owner
            .lock()
            .map_err(|_| "game sound property registry poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.game_sound_properties.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "gamesoundproperties", marker);
        owner.report(
            catalog.identity(),
            "gamesoundproperties",
            "scope-released",
            |_| 0,
        );
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.game_sound_properties.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report(
                catalog.identity(),
                "gamesoundproperties",
                "catalog-retiring",
                |_| 0,
            );
    }
}
