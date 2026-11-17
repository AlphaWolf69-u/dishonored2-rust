//! HUDGrenade45ba30's authored model/component and tag/scale declarations.
//! These immutable leases are acquired once by the existing level HUD owner.
use crate::{
    archive::Catalog,
    assets::{entities, runtime_entity},
    gameplay::options::decl::{atom, float, scope},
};
use std::sync::Arc;

pub(crate) struct Definition {
    pub entity: Arc<runtime_entity::Definition>,
    pub scale: [f32; 3],
    pub tag: String,
    pub sticky_tag: String,
}
impl Definition {
    pub fn acquire(
        catalog: &Catalog,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Option<Self>, String> {
        let read =
            |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
        let state = entities::player::resolved_decl(
            "gamelogicstateingame",
            "ui/gamelogic/state_ingame.gamelogicstateingame",
            read,
        )?;
        let name = scope(&state, "m_hudElements")
            .and_then(|fields| atom(fields, "enumItem[ARK_HUD_ELEMENT_GRENADE]"))
            .ok_or("in-game state has no authored grenade HUD")?;
        let fields = entities::player::resolved_decl("ui_hudelementgrenade", name, read)?;
        let Some(component) = atom(&fields, "m_grenadeCookingDecl")
            .filter(|value| !value.is_empty() && *value != "NULL")
        else {
            return Ok(None);
        };
        let path = atom(&fields, "m_entityDef")
            .filter(|value| !value.is_empty() && *value != "NULL")
            .ok_or("GrenadeCooking HUD has no entity definition")?;
        let base = runtime_entity::Definition::acquire(catalog, path, cancelled)?;
        //45ba30→875d50 appends the actual authored component to the otherwise
        //component-less entity. The variant shares every existing typed lease.
        let extra = runtime_entity::Component::acquire_decl(
            catalog,
            &base.types,
            path,
            "HUDGrenade.m_grenadeCookingDecl",
            component,
            cancelled,
        )?;
        let mut entity = (*base).clone();
        entity.components.push(extra.decoded.component);
        entity.source_paths.extend(extra.source_paths);
        let scale = scope(&fields, "m_cookingScale").unwrap_or(&[]);
        Ok(Some(Self {
            entity: Arc::new(entity),
            scale: [
                float(scale, "x", 1.),
                float(scale, "y", 1.),
                float(scale, "z", 1.),
            ],
            tag: atom(&fields, "m_cookingTag").unwrap_or("").to_owned(),
            sticky_tag: atom(&fields, "m_cookingStickyTag").unwrap_or("").to_owned(),
        }))
    }
}
