//! Typed usable-state declarations. The temporary generic declaration tree
//! is dropped after the level's component bindings have been resolved.
use super::{components, field, scope, Definition, Document, Value};
use std::{collections::{HashMap, HashSet}, sync::Arc};

#[derive(Clone, Debug)]
pub struct Step {
    pub animation: Option<u8>,
    pub transition_animation: Option<u8>,
    pub transition_min_duration: f32,
    pub effect: Option<String>,
    pub effect_origin: [f32; 3],
    pub effect_axis: [[f32; 3]; 3],
    pub effect_attached: bool,
    pub wait_transition_end_to_play_effect: bool,
    pub overrides_interaction: bool,
    pub interaction_name: Option<String>,
    pub interaction_text: Option<String>,
    pub interaction_icon: Option<String>,
    pub player_state: Option<String>,
    pub maximum_distance: f32,
    pub maximum_height: f32,
    pub occlusion_shape: usize,
    pub locked: bool,
    pub allowed_swimming: bool,
}

impl Default for Step {
    fn default() -> Self {
        // 14077a640: sparse list entries retain their constructed values.
        Self { animation: None, transition_animation: None, transition_min_duration: 0.5,
            effect: None, effect_origin: [0.0; 3],
            effect_axis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            effect_attached: true, wait_transition_end_to_play_effect: false,
            overrides_interaction: false, interaction_name: None, interaction_text: None,
            interaction_icon: None, player_state: None, maximum_distance: 3.0,
            maximum_height: 1000.0, occlusion_shape: 0, locked: false, allowed_swimming: true }
    }
}

#[derive(Clone, Debug)]
pub struct DefinitionData {
    pub starts_on_spawn: bool,
    pub initial_step: usize,
    pub fake_collision: bool,
    pub fake_collision_size: [f32; 3],
    pub fake_collision_offset: [f32; 3],
    pub interaction_name: Option<String>,
    pub interaction_text: Option<String>,
    pub interaction_icon: Option<String>,
    pub player_state: Option<String>,
    pub maximum_distance: f32,
    pub maximum_height: f32,
    pub occlusion_shape: usize,
    pub locked: bool,
    pub allowed_swimming: bool,
    pub steps: Box<[Step]>,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub entity: String,
    pub definition: Arc<DefinitionData>,
}

/// Corners of the fake interaction box in game coordinates. 14077af40
/// applies the entity basis to the offset and uses half of the authored size.
pub fn fake_box_corners(frame: &crate::gameplay::reference_frame::ReferenceFrame, definition: &DefinitionData) -> [[f32; 3]; 8] {
    std::array::from_fn(|corner| {
        let local: [f32; 3] = std::array::from_fn(|axis| {
            let sign = if (corner >> axis) & 1 == 0 { -0.5 } else { 0.5 };
            definition.fake_collision_offset[axis] + definition.fake_collision_size[axis] * sign
        });
        std::array::from_fn(|axis| frame.position[axis] + (0..3).map(|i| frame.basis[i][axis] * local[i]).sum::<f32>())
    })
}

#[derive(Clone, Debug)]
pub struct ReadableBinding {
    pub entity: String,
    pub item: String,
    pub interaction_name: Option<String>,
    pub interaction_text: Option<String>,
    pub interaction_icon: Option<String>,
    pub maximum_distance: f32,
    pub maximum_height: f32,
    pub occlusion_shape: usize,
    pub locked: bool,
    pub allowed_swimming: bool,
}

/// 14061e1a0 lets the interaction sensor pass through a material with
/// CAN_BLINK_THROUGH only when its owning entity has arkCpntBreakable.
pub fn breakable_entities(document: &Document) -> Result<HashSet<String>, String> {
    let declarations: HashMap<&str, &Definition> = document.definitions.iter()
        .filter(|d| d.kind == "component").map(|d| (d.id.as_str(), d)).collect();
    let mut result = HashSet::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        for (key, value) in child(child(&entity.fields, "edit"), "m_componentDecls") {
            if !key.starts_with("item_add[") { continue; }
            let Some(entry) = scope(Some(value)) else { continue };
            let class = match components::entry_class(entry, &declarations) {
                Ok(class) => class,
                Err(error) => { eprintln!("Component {} {key}: {error}", entity.id); continue; }
            };
            if class.as_deref() == Some("cpntbreakable") {
                result.insert(entity.id.clone());
                break;
            }
        }
    }
    Ok(result)
}

/// Readable pickables declare the read state on their pickup interaction;
/// other pickables must not be converted into notes merely by model name.
pub fn readable_bindings(document: &Document, mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<ReadableBinding>, String> {
    let declarations: HashMap<&str, &Definition> = document.definitions.iter()
        .filter(|d| d.kind == "component").map(|d| (d.id.as_str(), d)).collect();
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        for (key, value) in child(child(&entity.fields, "edit"), "m_componentDecls") {
            if !key.starts_with("item_add[") { continue; }
            let Some(entry) = scope(Some(value)) else { continue };
            let fields = match components::entry_fields(entry, "cpntpickable_simple", &declarations, &mut cache, &mut read) {
                Ok(Some(fields)) => fields,
                Ok(None) => continue,
                Err(error) => { eprintln!("Readable component {} {key}: {error}", entity.id); continue; }
            };
            let interaction = child(&fields, "m_pickupInteraction");
            if atom(interaction, "m_playerState") != Some("PLAYER_STATE_READ_NOTE") { continue; }
            let Some(item) = atom(&fields, "m_inventoryItem") else {
                eprintln!("Readable component {} {key} has no inventory item", entity.id);
                continue;
            };
            result.push(ReadableBinding {
                entity: entity.id.clone(), item: item.to_owned(),
                interaction_name: optional_string(interaction, "m_name"),
                interaction_text: optional_string(interaction, "m_usageText"),
                interaction_icon: optional_string(interaction, "m_interactionIcon"),
                maximum_distance: number(interaction, "m_maximumDistance", 3.0)?,
                maximum_height: number(interaction, "m_maxHeight", 1000.0)?,
                occlusion_shape: index(interaction, "m_occlusionShapeIndex", 0)?,
                locked: boolean(interaction, "m_lockedInteraction", false)?,
                allowed_swimming: boolean(interaction, "m_allowedWhileSwimming", true)?,
            });
        }
    }
    Ok(result)
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
fn index(fields: &[(String, Value)], name: &str, default: usize) -> Result<usize, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some(value) => value.parse().map_err(|_| format!("invalid {name}: {value}")),
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
        let state_index = index.parse::<usize>().map_err(|_| "invalid usable state index")?;
        let slot = slots.get_mut(state_index).ok_or("usable state index exceeds count")?;
        let data = scope(Some(value)).ok_or("usable state is not a scope")?;
        let override_interaction = boolean(data, "m_useOverrideInteractionDef", false)?;
        let interaction = if override_interaction { child(data, "m_overrideInteractionDef") } else { &[] };
        let fx = child(data, "m_FX");
        let matrix = child(child(fx, "m_localAxis"), "mat");
        let mut effect_axis = [[0.0; 3]; 3];
        for (row, axis) in effect_axis.iter_mut().enumerate() {
            let mut default = [0.0; 3];
            default[row] = 1.0;
            *axis = vector(matrix, &format!("mat[{row}]"), default)?;
        }
        *slot = Some(Step {
            animation: animation_index(data, "m_animIdx")?,
            transition_animation: animation_index(data, "m_transitionAnimIdx")?,
            // 14077a640 initializes the per-state duration to 0.5 seconds
            // and its effect attachment flag to true.
            transition_min_duration: number(data, "m_transitionMinDuration", 0.5)?,
            effect: optional_string(fx, "m_declFX").filter(|path| !path.eq_ignore_ascii_case("NULL")),
            effect_origin: vector(fx, "m_localOrigin", [0.0; 3])?,
            effect_axis,
            effect_attached: boolean(fx, "m_attached", true)?,
            wait_transition_end_to_play_effect: boolean(data, "m_waitTransitionEndToPlayFx", false)?,
            overrides_interaction: override_interaction,
            interaction_name: optional_string(interaction, "m_name"),
            interaction_text: optional_string(interaction, "m_usageText"),
            interaction_icon: optional_string(interaction, "m_interactionIcon"),
            player_state: optional_string(interaction, "m_playerState"),
            maximum_distance: number(interaction, "m_maximumDistance", 3.0)?,
            maximum_height: number(interaction, "m_maxHeight", 1000.0)?,
            occlusion_shape: index(interaction, "m_occlusionShapeIndex", 0)?,
            locked: boolean(interaction, "m_lockedInteraction", false)?,
            allowed_swimming: boolean(interaction, "m_allowedWhileSwimming", true)?,
        });
    }
    let steps = slots.into_iter().map(Option::unwrap_or_default).collect::<Vec<_>>().into_boxed_slice();
    Ok(DefinitionData {
        starts_on_spawn: components::scheduling(fields,
            crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?.starts_on_spawn(),
        initial_step,
        // arkDeclCpntUsableWithStateList constructor 14077a4a0.
        fake_collision: boolean(fields, "m_useFakeCollision", true)?,
        fake_collision_size: vector(fields, "m_fakeCollisionSize", [0.1, 0.1, 0.3])?,
        fake_collision_offset: vector(fields, "m_fakeCollisionOffset", [0.0, 0.0, 1.4])?,
        interaction_name: optional_string(interaction, "m_name"),
        interaction_text: optional_string(interaction, "m_usageText"),
        interaction_icon: optional_string(interaction, "m_interactionIcon"),
        player_state: optional_string(interaction, "m_playerState"),
        maximum_distance: number(interaction, "m_maximumDistance", 3.0)?,
        maximum_height: number(interaction, "m_maxHeight", 1000.0)?,
        occlusion_shape: index(interaction, "m_occlusionShapeIndex", 0)?,
        locked: boolean(interaction, "m_lockedInteraction", false)?,
        allowed_swimming: boolean(interaction, "m_allowedWhileSwimming", true)?,
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
                let fields = match components::entry_fields(entry, "cpntusablewithstatelist", &declarations, &mut source_cache, &mut read) {
                    Ok(Some(fields)) => fields,
                    Ok(None) => continue,
                    Err(error) => { eprintln!("Usable component {} {key}: {error}", entity.id); continue; }
                };
                let definition = match parse(&fields) {
                    Ok(definition) => Arc::new(definition),
                    Err(error) => { eprintln!("Usable component {} {key}: {error}", entity.id); continue; }
                };
                if let Some(key) = cache_key { typed_cache.insert(key, definition.clone()); }
                definition
            };
            result.push(Binding { entity: entity.id.clone(), definition });
        }
    }
    Ok(result)
}
