//! The player character's component declarations, resolved from the
//! installed entity def (`models/characters/player/<character>/body.def`)
//! the way the entity spawns them: `m_componentDecls` entries merged by key
//! (`item_add` then `item_override`), `inheritedDecl` with its inheritance
//! chain, then `inlineDecl`.
use super::components::{atom, component_fields, overlay, Fields};
use super::{field, scope, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct PlayerDeclarations {
    pub physics: Vec<(String, Value)>,
    pub camera: Vec<(String, Value)>,
    pub controller: Vec<(String, Value)>,
    pub attributes: Vec<(String, Value)>,
    pub autocrouch_sensor: Vec<(String, Value)>,
    pub lean_sensor: Vec<(String, Value)>,
    pub mantle_sensor: Vec<(String, Value)>,
    pub health: Vec<(String, Value)>,
    pub body: Vec<(String, Value)>,
    /// The state machine's startup mode.
    pub mode: Vec<(String, Value)>,
    /// The mode's `m_states`: (decl kind, resolved fields).
    pub states: Vec<(String, Vec<(String, Value)>)>,
    /// The body model's `locomotionDecl`.
    pub locomotion: Vec<(String, Value)>,
    /// The body model's `animBasicDecl` (the additives node's list).
    pub additives: Vec<(String, Value)>,
    pub body_model: String,
}

/// The components the player locomotion reads.
const PLAYER_COMPONENTS: [&str; 10] = [
    "cpntplayerphysics",
    "cpntplayercamera",
    "cpntplayercontroller",
    "cpntattributesplayer",
    "cpntplayerstatemachine",
    "cpntplayersensorautocrouch",
    "cpntplayersensorlean",
    "cpntplayerbody",
    "cpntplayersensormantle",
    "cpnthealthevolved",
];

fn kind_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(_, kind)| kind)
}

fn component_key(key: &str) -> Option<&str> {
    key.strip_prefix("item_add[").or_else(|| key.strip_prefix("item_override[")).and_then(|rest| rest.strip_suffix(']'))
}

/// The entity def chain from the root ancestor to `name`.
fn entity_chain(name: &str, read: &mut impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<Fields>, String> {
    let mut chain = Vec::new();
    let mut next = Some(name.to_owned());
    while let Some(name) = next.take() {
        if chain.len() > 16 {
            return Err(format!("entity def inheritance exceeds 16 at {name}"));
        }
        let path = format!("generated/decls/entitydef/{name}.decl");
        let source = read(&path)?.ok_or_else(|| format!("entity def missing: {path}"))?;
        let fields = super::parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
        next = atom(&fields, "inherit").map(str::to_owned);
        chain.push(fields);
    }
    chain.reverse();
    Ok(chain)
}

fn list_items(fields: &[(String, Value)], name: &str) -> Vec<String> {
    let Some(list) = scope(field(fields, name)) else { return Vec::new() };
    let mut items: Vec<(usize, String)> = list
        .iter()
        .filter_map(|(key, value)| {
            let index = key.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?;
            match value {
                Value::Atom(text) => Some((index, text.trim_matches('"').to_owned())),
                _ => None,
            }
        })
        .collect();
    items.sort_by_key(|(index, _)| *index);
    items.into_iter().map(|(_, text)| text).collect()
}

/// A `kind` decl resolved through its `inherit` chain with the `edit`
/// overlays (as the player components are).
pub fn resolved_decl(kind: &str, name: &str, mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<(String, Value)>, String> {
    component_fields(kind, name, 0, &mut HashMap::new(), &mut read)
}

pub fn player_declarations(
    character: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<PlayerDeclarations, String> {
    let entity = format!("models/characters/player/{character}/body.def");
    let chain = entity_chain(&entity, &mut read)?;
    let mut entries: Vec<(String, Fields)> = Vec::new();
    let mut body_model = String::new();
    for fields in &chain {
        let Some(edit) = scope(field(fields, "edit")) else { continue };
        if let Some(model) = scope(field(edit, "renderModelInfo")).and_then(|info| atom(info, "model")) {
            body_model = model.to_owned();
        }
        let Some(components) = scope(field(edit, "m_componentDecls")) else { continue };
        for (key, value) in components {
            let (Some(key), Some(entry)) = (component_key(key), scope(Some(value))) else { continue };
            match entries.iter_mut().find(|(k, _)| k == key) {
                Some((_, existing)) => overlay(existing, entry),
                None => entries.push((key.to_owned(), entry.to_vec())),
            }
        }
    }
    let mut cache = HashMap::new();
    let mut resolved: HashMap<String, Fields> = HashMap::new();
    for (_, entry) in &entries {
        let Some(name) = atom(entry, "inheritedDecl") else { continue };
        let Some(kind) = kind_of(name).filter(|kind| PLAYER_COMPONENTS.contains(kind)) else { continue };
        let mut fields = component_fields(kind, name, 0, &mut cache, &mut read)?;
        if let Some(inline) = scope(field(entry, "inlineDecl")) {
            overlay(&mut fields, inline);
        }
        resolved.insert(kind.to_owned(), fields);
    }
    let mut take = |kind: &str| resolved.remove(kind).ok_or_else(|| format!("{entity} has no {kind} component"));
    let physics = take("cpntplayerphysics")?;
    let camera = take("cpntplayercamera")?;
    let controller = take("cpntplayercontroller")?;
    let attributes = take("cpntattributesplayer")?;
    let autocrouch_sensor = take("cpntplayersensorautocrouch")?;
    let lean_sensor = take("cpntplayersensorlean")?;
    let mantle_sensor = take("cpntplayersensormantle")?;
    let health = take("cpnthealthevolved")?;
    let body = take("cpntplayerbody")?;
    let machine = take("cpntplayerstatemachine")?;
    let startup = atom(&machine, "m_startupModeId").unwrap_or("PLAYER_MODE_DEFAULT").to_owned();
    let mut mode = None;
    for name in list_items(&machine, "m_modes") {
        let fields = component_fields("playermode", &name, 0, &mut cache, &mut read)?;
        if atom(&fields, "m_id") == Some(startup.as_str()) {
            mode = Some(fields);
            break;
        }
    }
    let mode = mode.ok_or_else(|| format!("{entity}: startup mode {startup} not found"))?;
    let mut states = Vec::new();
    for name in list_items(&mode, "m_states") {
        let Some(kind) = kind_of(&name) else { continue };
        states.push((kind.to_owned(), component_fields(kind, &name, 0, &mut cache, &mut read)?));
    }
    let locomotion = match md6_decl(&body_model, "locomotionDecl", &mut read)? {
        Some(name) => component_fields("playerlocomotion", &name, 0, &mut cache, &mut read)?,
        None => Vec::new(),
    };
    let additives = match md6_decl(&body_model, "animBasicDecl", &mut read)? {
        Some(name) => component_fields("animbasic", &name, 0, &mut cache, &mut read)?,
        None => Vec::new(),
    };
    Ok(PlayerDeclarations { physics, camera, controller, attributes, autocrouch_sensor, lean_sensor, mantle_sensor, health, body, mode,
 states, locomotion, additives, body_model })

}

/// A decl named by the md6 declaration (`locomotionDecl`, `animBasicDecl`),
/// inherited through `inherit`.
fn md6_decl(model: &str, key: &str, read: &mut impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Option<String>, String> {
    let Some(stem) = model.strip_suffix(".md6") else { return Ok(None) };
    let mut path = format!("generated/decls/md6def/{stem}.md6.decl");
    for _ in 0..16 {
        let Some(source) = read(&path)? else { return Ok(None) };
        let tokens = super::declaration_lexemes(&source)?;
        if let Some(i) = tokens.iter().position(|t| *t == key) {
            return Ok(tokens.get(i + 1).map(|t| t.trim_matches('"').to_owned()));
        }
        let Some(i) = tokens.iter().position(|t| *t == "inherit") else { return Ok(None) };
        let Some(parent) = tokens.get(i + 1) else { return Ok(None) };
        path = format!("generated/decls/md6def/{}.md6.decl", parent.trim_matches('"').trim_end_matches(".md6"));
    }
    Err(format!("{model}: md6 inheritance exceeds 16"))
}
