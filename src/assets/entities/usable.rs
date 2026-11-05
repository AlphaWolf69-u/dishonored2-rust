//! Typed usable-state declarations. The temporary generic declaration tree
//! is dropped after the level's component bindings have been resolved.
use super::{components, field, scope, Definition, Document, Value};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct Step {
    pub animation: Option<u8>,
    pub transition_animation: Option<u8>,
    pub transition_min_duration: f32,
    pub effect: Option<String>,
    pub wait_transition_end_to_play_effect: bool,
    pub interaction_name: Option<String>,
    pub interaction_text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DefinitionData {
    pub initial_step: usize,
    pub fake_collision: bool,
    pub fake_collision_size: [f32; 3],
    pub fake_collision_offset: [f32; 3],
    pub interaction_name: Option<String>,
    pub interaction_text: Option<String>,
    pub steps: Box<[Step]>,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub entity: String,
    pub definition: Arc<DefinitionData>,
}

#[derive(Clone, Copy, Debug)]
pub struct Sensor {
    pub max_distance: f32,
    pub shape_radii: [f32; 5],
}

/// Resolve the selected character's inherited interaction sensor. Constructor
/// 1403b7a40 supplies the defaults; 1403b8d60 uses all five cast shapes.
pub fn player_sensor(catalog: &crate::archive::Catalog, character: &str) -> Result<Sensor, String> {
    let entity = format!("models/characters/player/{character}/body.def");
    let fields = super::player::entity_component(&entity, "cpntplayersensorinteraction", |path| {
        Ok(catalog.read(path).map_err(|e| e.to_string())?
            .map(|bytes| super::declaration_text(&bytes).into_owned()))
    })?.ok_or_else(|| format!("{entity}: missing interaction sensor"))?;
    let mut sensor = Sensor { max_distance: number(&fields, "m_maxDistance", 3.0)?, shape_radii: [0.005, 0.05, 0.1, 0.5, 0.7] };
    let radii = child(&fields, "m_shapeRadius");
    for (index, radius) in sensor.shape_radii.iter_mut().enumerate() {
        *radius = number(radii, &format!("m_shapeRadius[{index}]"), *radius)?;
    }
    Ok(sensor)
}

fn child<'a>(fields: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    scope(field(fields, name)).unwrap_or(&[])
}
fn atom<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    components::atom(fields, name)
}
fn optional_string(fields: &[(String, Value)], name: &str) -> Option<String> {
    atom(fields, name).filter(|s| !s.is_empty()).map(str::to_owned)
}
fn boolean(fields: &[(String, Value)], name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(format!("invalid {name}: {other}")),
    }
}
fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some(value) => value.trim_end_matches('m').parse::<f32>().ok()
            .filter(|v| v.is_finite()).ok_or_else(|| format!("invalid {name}: {value}")),
    }
}
fn vector(fields: &[(String, Value)], name: &str, default: [f32; 3]) -> Result<[f32; 3], String> {
    let fields = child(fields, name);
    Ok([number(fields, "x", default[0])?, number(fields, "y", default[1])?, number(fields, "z", default[2])?])
}
fn animation_index(fields: &[(String, Value)], name: &str) -> Result<Option<u8>, String> {
    let Some(value) = atom(fields, name) else { return Ok(None) };
    let index = value.parse::<i16>().map_err(|_| format!("invalid {name}: {value}"))?;
    match index {
        -1 | 255 => Ok(None),
        0..=127 => Ok(Some(index as u8)),
        _ => Err(format!("invalid {name}: {value}")),
    }
}

fn parse(fields: &[(String, Value)]) -> Result<DefinitionData, String> {
    let initial_step = atom(fields, "m_initialStep").unwrap_or("0").parse()
        .map_err(|_| "invalid usable initial step")?;
    let interaction = child(fields, "m_usableInteraction");
    let states = child(fields, "m_stateList");
    let count = atom(states, "num").unwrap_or("0").parse::<usize>()
        .map_err(|_| "invalid usable state count")?;
    let mut slots: Vec<Option<Step>> = vec![None; count];
    for (name, value) in states {
        let Some(index) = name.strip_prefix("item[").and_then(|s| s.strip_suffix(']')) else { continue };
        let index = index.parse::<usize>().map_err(|_| "invalid usable state index")?;
        let slot = slots.get_mut(index).ok_or("usable state index exceeds count")?;
        let data = scope(Some(value)).ok_or("usable state is not a scope")?;
        let override_interaction = boolean(data, "m_useOverrideInteractionDef", false)?;
        let interaction = if override_interaction { child(data, "m_overrideInteractionDef") } else { &[] };
        *slot = Some(Step {
            animation: animation_index(data, "m_animIdx")?,
            transition_animation: animation_index(data, "m_transitionAnimIdx")?,
            transition_min_duration: number(data, "m_transitionMinDuration", 0.0)?,
            effect: optional_string(child(data, "m_FX"), "m_declFX"),
            wait_transition_end_to_play_effect: boolean(data, "m_waitTransitionEndToPlayFx", false)?,
            interaction_name: optional_string(interaction, "m_name"),
            interaction_text: optional_string(interaction, "m_usageText"),
        });
    }
    let steps = slots.into_iter().map(|slot| slot.ok_or_else(|| "sparse usable state list".to_owned()))
        .collect::<Result<Vec<_>, _>>()?.into_boxed_slice();
    Ok(DefinitionData {
        initial_step,
        // arkDeclCpntUsableWithStateList constructor 14077a4a0.
        fake_collision: boolean(fields, "m_useFakeCollision", true)?,
        fake_collision_size: vector(fields, "m_fakeCollisionSize", [0.1, 0.1, 0.3])?,
        fake_collision_offset: vector(fields, "m_fakeCollisionOffset", [0.0, 0.0, 1.4])?,
        interaction_name: optional_string(interaction, "m_name"),
        interaction_text: optional_string(interaction, "m_usageText"),
        steps,
    })
}

/// Resolve map-local and inherited component declarations. Bindings that share
/// a map declaration share the typed data; inline overrides remain independent.
pub fn bindings(document: &Document, mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<Binding>, String> {
    let declarations: HashMap<&str, &Definition> = document.definitions.iter()
        .filter(|d| d.kind == "component").map(|d| (d.id.as_str(), d)).collect();
    let mut source_cache = HashMap::new();
    let mut typed_cache: HashMap<String, Arc<DefinitionData>> = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let entries = child(child(&entity.fields, "edit"), "m_componentDecls");
        for (key, value) in entries.iter().filter(|(key, _)| key.starts_with("item_add[")) {
            let Some(entry) = scope(Some(value)) else { continue };
            let cache_key = if field(entry, "inlineDecl").is_none() {
                atom(entry, "mapComponentDecl").or_else(|| atom(entry, "inheritedDecl")).map(str::to_owned)
            } else { None };
            let definition = if let Some(shared) = cache_key.as_ref().and_then(|k| typed_cache.get(k)) {
                shared.clone()
            } else {
                let Some(fields) = components::entry_fields(entry, "cpntusablewithstatelist", &declarations, &mut source_cache, &mut read)? else { continue };
                let definition = Arc::new(parse(&fields).map_err(|e| format!("{} {key}: {e}", entity.id))?);
                if let Some(key) = cache_key { typed_cache.insert(key, definition.clone()); }
                definition
            };
            result.push(Binding { entity: entity.id.clone(), definition });
        }
    }
    Ok(result)
}
