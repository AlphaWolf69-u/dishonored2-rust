//! Authored component slot identity shared by declaration processing and admission.
/// Component slot identity shared by authored resources and the resolved batch.
/// Returns None for non-slot edit-list entries such as item_begin.
pub fn component_slot_identity(key: &str) -> Result<Option<String>, String> {
    let (numeric, suffix) = if let Some(value) = key.strip_prefix("item_add[") {
        (false, value)
    } else if let Some(value) = key.strip_prefix("item_override[") {
        (false, value)
    } else if let Some(value) = key.strip_prefix("item[") {
        (true, value)
    } else { return Ok(None) };
    let slot = suffix.strip_suffix(']').ok_or_else(|| format!("invalid component slot {key}"))?;
    if numeric {
        if slot.starts_with('"') { return Err(format!("quoted numeric component slot requires native handling: {key}")); }
        let index = slot.parse::<i32>().map_err(|_| format!("invalid numeric component slot {slot}"))?;
        Ok(Some(super::processed_decl::encode_edit_id(index.wrapping_add(1) as u32)))
    } else if slot.starts_with('"') {
        let bytes = super::processed_decl::decode_quoted(slot)?;
        Ok(Some(String::from_utf8(bytes).map_err(|_| format!("non-UTF-8 component slot {key}"))?))
    } else { Ok(Some(slot.to_owned())) }
}

use super::{Definition, Value, event_graph::atom};
use std::sync::Arc;

/// A live declaration owns its source and its complete inherited source chain.
/// The admission owner shares each installed ancestor before creating this lease.
pub(crate) struct ComponentDeclarationLease {
    _source: Arc<Definition>,
    _parent: Option<Arc<ComponentDeclarationLease>>,
    // Allocate token text and lazy projection only for faction declarations.
    faction_source: Option<Box<FactionSource>>,
}
struct FactionSource {
    text: Arc<str>,
    fields: std::sync::OnceLock<Result<Arc<Vec<(String, Value)>>, String>>,
}
impl ComponentDeclarationLease {
    /// The source acquisition owner supplies the canonical declaration and
    /// its already-acquired parent. No I/O or secondary cache is created here.
    pub(crate) fn new(source: Arc<Definition>, parent: Option<Arc<Self>>, faction_text: Option<Arc<str>>) -> Self {
        Self { _source: source, _parent: parent,
            faction_source: faction_text.map(|text| Box::new(FactionSource { text, fields: Default::default() })) }
    }

    /// Project this already-retained declaration chain and the selected
    /// component entry's inline edit, using the same overlay as entry_fields.
    /// The returned fields are temporary; the source chain remains the owner.
    pub(crate) fn resolved_fields_for_entry(&self, entry: &[(String, Value)]) -> Result<Vec<(String, Value)>, String> {
        fn apply(source: &ComponentDeclarationLease, fields: &mut Vec<(String, Value)>, map: bool) {
            if let Some(parent) = source._parent.as_deref() { apply(parent, fields, false); }
            let own = &source._source.fields;
            let edit = super::scope(super::field(own, "edit"));
            super::components::overlay(fields, edit.unwrap_or(if map { &[] } else { own }));
        }
        let mut fields = Vec::new();
        apply(self, &mut fields, atom(entry, "mapComponentDecl").is_some());
        if let Some(inline) = super::field(entry, "inlineDecl") {
            let Value::Scope(inline) = inline else { return Err("component inline declaration is not a scope".into()); };
            super::components::overlay(&mut fields, inline);
        }
        Ok(fields)
    }

    /// Project the retained faction declaration through the existing native
    /// edit-list resolver. Parent source comes from the same retained chain.
    pub(crate) fn faction_fields(&self) -> Result<Arc<Vec<(String, Value)>>, String> {
        use super::processed_decl::{parse, project_scope, Resolver, Source};
        let source = self.faction_source.as_ref().ok_or("faction token source is unavailable")?;
        source.fields.get_or_init(|| {
            let nodes = parse(Source::new(source.text.clone()))?;
            let mut resolver = Resolver::default();
            let resolved = resolver.resolve(&nodes, |name| {
                let mut parent = self._parent.as_deref();
                while let Some(current) = parent {
                    if current._source.id.eq_ignore_ascii_case(name) {
                        return current.faction_source.as_ref().map(|source| source.text.clone())
                            .ok_or_else(|| format!("faction source unavailable: {name}"));
                    }
                    parent = current._parent.as_deref();
                }
                Err(format!("faction parent not retained: {name}"))
            })?;
            project_scope(&resolved).map(Arc::new)
        }).clone()
    }
}
