//! Shared component declaration inheritance and inline field overlays.
use super::{field, parse_declaration, scope, Value};
use std::collections::HashMap;
pub(super) type Fields = Vec<(String, Value)>;

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
