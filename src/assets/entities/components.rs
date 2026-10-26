//! Shared component declaration inheritance and inline field overlays.
use super::{field, parse_declaration, scope, Definition, Value};
use std::collections::HashMap;
pub(super) type Fields = Vec<(String, Value)>;

/// Common scheduling for a component class, using its resolved declaration and
/// inline overlays. Payload readers and lifecycle composition share this path.
pub fn scheduling_bindings(
    document: &super::Document,
    kind: &str,
    defaults: crate::gameplay::component_state::declaration::DeclarationScheduling,
    mut read: impl FnMut(&str)->Result<Option<String>,String>,
) -> Result<Vec<(String,String,crate::gameplay::component_state::declaration::DeclarationScheduling)>,String> {
    let declarations=document.definitions.iter().filter(|d|d.kind=="component")
        .map(|d|(d.id.as_str(),d)).collect::<HashMap<_,_>>();
    let mut cache=HashMap::new();
    let mut result=Vec::new();
    for entity in document.definitions.iter().filter(|d|d.kind=="entity") {
        let Some(entries)=scope(field(&entity.fields,"edit")).and_then(|e|scope(field(e,"m_componentDecls"))) else {continue};
        for (slot,value) in entries.iter().filter(|(k,_)|k.starts_with("item_add[")) {
            let Some(entry)=scope(Some(value)) else {continue};
            if let Some(fields)=entry_fields(entry,kind,&declarations,&mut cache,&mut read)? {
                result.push((entity.id.clone(),slot.clone(),scheduling(&fields,defaults.clone())?));
            }
        }
    }
    Ok(result)
}

/// Common declaration fields from1414fc060, with enum values from1419fa950
/// and141a017b0. Apply after the concrete declaration constructor's defaults.
pub(super) fn scheduling(
    fields: &[(String, Value)],
    mut value: crate::gameplay::component_state::declaration::DeclarationScheduling,
) -> Result<crate::gameplay::component_state::declaration::DeclarationScheduling, String> {
    if let Some(text) = atom(fields, "m_flags") {
        value.update_mask = text.parse::<i32>().map_err(|_| "invalid component m_flags")? as u32;
    }
    if let Some(text) = atom(fields, "m_startOnEntitySpawn") {
        value.set_start_on_spawn(match text {
            "true" => true,
            "false" => false,
            _ => return Err("invalid component m_startOnEntitySpawn".into()),
        });
    }
    if let Some(text) = atom(fields, "m_entityContext") {
        value.start_phases = 0;
        for flag in text.split_whitespace() {
            value.start_phases |= match flag {
                "ENT_CONTEXT_FLAG_ALIVE" => 1,
                "ENT_CONTEXT_FLAG_ASLEEP" => 2,
                "ENT_CONTEXT_FLAG_DEAD" => 4,
                "ENT_CONTEXT_FLAG_HACKED" => 8,
                "ENT_CONTEXT_FLAG_POSSESSED" => 16,
                "ENT_CONTEXT_FLAG_TAMPERING_GHOST" => 32,
                "ENT_CONTEXT_FLAG_ANY" => 63,
                "ENT_CONTEXT_FLAG_ANY_EXCEPT_GHOST" => 31,
                _ => flag.parse::<u32>().map_err(|_| format!("unknown entity context {flag}"))?,
            };
        }
    }
    if let Some(text) = atom(fields, "m_autoPauseLODLevel") {
        value.priority = match text {
            "LOD_LEVEL_0" => 0,
            "LOD_LEVEL_1" => 1,
            "LOD_LEVEL_2" => 2,
            "LOD_LEVEL_3" => 3,
            "LOD_LEVEL_4" => 4,
            "LOD_LEVEL_4_FAR" | "LOD_LEVEL_MODEL_MAX" => 5,
            "LOD_LEVEL_HIDDEN" => 6,
            "LOD_LEVEL_HIDDEN_FAR" => 7,
            "LOD_LEVEL_MAX" => 8,
            _ => text.parse().map_err(|_| format!("unknown component LOD level {text}"))?,
        };
    }
    Ok(value)
}

pub(super) fn entry_class(
    entry: &[(String, Value)],
    declarations: &HashMap<&str, &Definition>,
) -> Result<Option<String>, String> {
    if let Some(name) = atom(entry, "mapComponentDecl") {
        return declarations
            .get(name)
            .map(|declaration| Some(declaration.class.to_ascii_lowercase()))
            .ok_or_else(|| format!("map component declaration missing: {name}"));
    }
    Ok(atom(entry, "inheritedDecl")
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, kind)| kind.to_ascii_lowercase()))
}

/// Resolve one binding using the same inheritance and overlays as the other
/// component readers. The caller keeps a cache for this component class.
pub(super) fn entry_fields(
    entry: &[(String, Value)],
    kind: &str,
    declarations: &HashMap<&str, &Definition>,
    cache: &mut HashMap<String, Fields>,
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Option<Fields>, String> {
    if entry_class(entry, declarations)?.as_deref() != Some(kind) {
        return Ok(None);
    }
    let mut resolved = if let Some(name) = atom(entry, "mapComponentDecl") {
        let declaration = declarations[name];
        let mut fields = match atom(&declaration.fields, "inherit") {
            Some(parent) => component_fields(kind, parent, 0, cache, read)?,
            None => Vec::new(),
        };
        overlay(
            &mut fields,
            scope(field(&declaration.fields, "edit")).unwrap_or(&[]),
        );
        fields
    } else {
        component_fields(kind, atom(entry, "inheritedDecl").unwrap(), 0, cache, read)?
    };
    if let Some(inline) = scope(field(entry, "inlineDecl")) {
        overlay(&mut resolved, inline);
    }
    Ok(Some(resolved))
}

pub(super) fn atom<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    match field(fields, key)? {
        Value::Atom(value) => Some(value.trim_matches('"')),
        _ => None,
    }
}

pub(super) fn overlay(base: &mut Fields, local: &[(String, Value)]) {
    for (key, value) in local {
        if let Some((_, previous)) = base.iter_mut().find(|(name, _)| name == key) {
            if let (Value::Scope(previous), Value::Scope(local)) = (&mut *previous, value) {
                overlay(previous, local);
            } else {
                *previous = value.clone();
            }
        } else {
            base.push((key.clone(), value.clone()));
        }
    }
}

pub(super) fn component_fields(
    kind: &str,
    name: &str,
    depth: usize,
    cache: &mut HashMap<String, Fields>,
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Fields, String> {
    if depth > 16 {
        return Err(format!("component inheritance exceeds 16 at {name}"));
    }
    if let Some(fields) = cache.get(name) {
        return Ok(fields.clone());
    }
    let path = format!("generated/decls/{kind}/{name}.decl");
    let source = read(&path)?.ok_or_else(|| format!("component declaration missing: {path}"))?;
    let fields = parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
    let mut resolved = match atom(&fields, "inherit") {
        Some(parent) => component_fields(kind, parent, depth + 1, cache, read)?,
        None => Vec::new(),
    };
    overlay(
        &mut resolved,
        scope(field(&fields, "edit")).unwrap_or(&fields),
    );
    cache.insert(name.to_owned(), resolved.clone());
    Ok(resolved)
}
