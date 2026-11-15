//! Damage declarations, registered at 14138c810 (damage/arkDeclDamage),
//! constructor 140780490, parameter construction 140780260.
//! Payload and inherited fields live behind the Catalog declaration registry.
use crate::{archive::Catalog, entities::{self, Value}, gameplay::damage::{atom, number, DamageType}};
use std::sync::Arc;

#[derive(Debug)]
pub struct Declaration {
    pub minimum: f32,
    pub maximum: f32,
    pub damage_type: DamageType,
}
impl Declaration {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        Ok(Self { minimum: number(fields, "m_minDamage", 0.0)?, maximum: number(fields, "m_maxDamage", 0.0)?, damage_type: DamageType::read(atom(fields, "m_damageType").unwrap_or("0"))? })
    }
    pub fn acquire(catalog: &Catalog, name: &str) -> Result<Arc<Self>, String> {
        let path = path(name);
        catalog.damage_declarations.get_or_init(Default::default).lock().map_err(|_| "damage declaration registry is poisoned")?
            .acquire_with_sharing(&path, 0, !catalog.has_overlays(), |_| {
                let _tag = crate::platform::allocation_tags::tag("Damage declarations");
                let fields = entities::player::resolved_decl("damage", name, |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))?;
                Self::read(&fields).map(Arc::new)
            })
    }
}
fn path(name: &str) -> String {
    let name = name.replace('\\', "/").to_ascii_lowercase();
    if name.starts_with("generated/decls/damage/") { name }
    else { format!("generated/decls/damage/{}.decl", name.trim_end_matches(".decl")) }
}
pub(crate) fn register_preload(catalog: &Catalog, name: &str, marker: u8, persistent: bool) -> Result<(), String> {
    catalog.damage_declarations.get_or_init(Default::default).lock().map_err(|_| "damage declaration registry is poisoned")?.register(&path(name), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.damage_declarations.get() { owner.lock().map_err(|_| "damage declaration registry is poisoned")?.promote_bootstrap(); }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.damage_declarations.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "damage", marker);
        owner.report(catalog.identity(), "damage", "scope-released", |_| 0);
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.damage_declarations.get() { owner.lock().unwrap_or_else(|error| error.into_inner()).report(catalog.identity(), "damage", "catalog-retiring", |_| 0); }
}
