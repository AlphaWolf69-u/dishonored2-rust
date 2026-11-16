//! Explosion declarations7e3560 and141705... reflection. Catalog owns leases.
use crate::{
    archive::Catalog,
    entities::{self, Value},
    gameplay::damage::{atom, number},
};
use std::sync::Arc;
#[derive(Debug)]
pub struct Declaration {
    pub damage: Option<Arc<super::damage::Declaration>>,
    pub safe_radius: f32,
    pub radius: f32,
    pub full_radius: f32,
    pub player_radius_scale: f32,
    pub player_full_radius_scale: f32,
    pub player_damage_scale: f32,
    pub angle: f32,
    pub collision_layers: u32,
    pub occlusion_layers: u32,
    pub filter_inflictor: bool,
    pub filter_attacker: bool,
    pub filter_neutral: bool,
    pub filter_witch: bool,
    pub filter_carried: bool,
    pub filter_doppelganger: bool,
    pub attacker_always_player: bool,
    pub ignore_gender: String,
    pub impulse: f32,
    pub impulse_z_offset: f32,
    pub contact_material: Option<String>,
    pub ground_material: Option<String>,
    pub ground_distance: f32,
    pub water_fx: Option<String>,
    pub water_distance: f32,
    pub source: u8,
}
impl Declaration {
    pub fn acquire(catalog: &Catalog, name: &str) -> Result<Arc<Self>, String> {
        catalog
            .explosion_declarations
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "explosion declaration registry poisoned")?
            .acquire_with_sharing(&path(name), 0, !catalog.has_overlays(), |_| {
                let fields = entities::player::resolved_decl("explosion", name, |path| {
                    super::declaration_texts::read_common_or_catalog(catalog, path)
                })?;
                Self::read(catalog, &fields).map(Arc::new)
            })
    }
    pub fn read(catalog: &Catalog, fields: &[(String, Value)]) -> Result<Self, String> {
        let text = |key| {
            atom(fields, key)
                .filter(|v| !v.is_empty() && *v != "NULL")
                .map(str::to_owned)
        };
        let flag = |key| match atom(fields, key) {
            None | Some("false" | "0") => Ok(false),
            Some("true" | "1") => Ok(true),
            Some(v) => Err(format!("invalid {key}:{v}")),
        };
        let source = match atom(fields, "m_explosionSource").unwrap_or("EXPLOSION_SOURCE_DEFAULT") {
            "EXPLOSION_SOURCE_DEFAULT" | "0" => 0,
            "EXPLOSION_SOURCE_INCENDIARY_BOLT" | "1" => 1,
            "EXPLOSION_SOURCE_BOTTLE" | "2" => 2,
            "EXPLOSION_SOURCE_HOWLING_BOLT" | "3" => 3,
            "EXPLOSION_SOURCE_SPRINGRAZOR" | "4" => 4,
            "EXPLOSION_SOURCE_WHALE_OIL" | "5" => 5,
            "EXPLOSION_SOURCE_GRENADE" | "6" => 6,
            v => return Err(format!("unknown explosion source {v}")),
        };
        Ok(Self {
            damage: text("m_damageDecl")
                .map(|p| super::damage::Declaration::acquire(catalog, &p))
                .transpose()?,
            safe_radius: number(fields, "m_safeRadius", 0.)?,
            radius: number(fields, "m_damageRadius", 0.)?,
            full_radius: number(fields, "m_fullDamageRadius", 0.)?,
            player_radius_scale: number(fields, "m_damageRadiusScaleForPlayer", 1.)?,
            player_full_radius_scale: number(fields, "m_fullDamageRadiusScaleForPlayer", 0.5)?,
            player_damage_scale: number(fields, "m_damageAmountScaleForPlayer", 1.)?,
            angle: number(fields, "m_angle", 360.)?,
            collision_layers: crate::gameplay::player::decls::layer_mask(
                fields,
                "m_collisionLayers",
                0x240,
            )?,
            occlusion_layers: crate::gameplay::player::decls::layer_mask(
                fields,
                "m_occlusionLayers",
                0x23,
            )?,
            filter_inflictor: flag("m_filterInflictor")?,
            filter_attacker: flag("m_filterAttacker")?,
            filter_neutral: flag("m_filterNeutral")?,
            filter_witch: flag("m_filterWitch")?,
            filter_carried: flag("m_filterCarriedByPlayer")?,
            filter_doppelganger: flag("m_filterDoppelganger")?,
            attacker_always_player: flag("m_attackerIsAlwaysPlayer")?,
            ignore_gender: atom(fields, "m_ignoreGender").unwrap_or("5").to_owned(),
            impulse: number(fields, "m_impulse", 0.)?,
            impulse_z_offset: number(fields, "m_impulseDirZOffset", 0.)?,
            contact_material: text("m_contactMaterial"),
            ground_material: text("m_groundContactMaterial"),
            ground_distance: number(fields, "m_groundContactDistance", 0.)?,
            water_fx: text("m_waterSurfaceFX"),
            water_distance: number(fields, "m_waterSurfaceDistance", 0.)?,
            source,
        })
    }
}
fn path(name: &str) -> String {
    let name = name.replace('\\', "/").to_ascii_lowercase();
    if name.starts_with("generated/decls/explosion/") {
        name
    } else {
        format!(
            "generated/decls/explosion/{}.decl",
            name.trim_end_matches(".decl")
        )
    }
}
pub(crate) fn register_preload(
    catalog: &Catalog,
    name: &str,
    marker: u8,
    persistent: bool,
) -> Result<(), String> {
    catalog
        .explosion_declarations
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "explosion declaration registry is poisoned")?
        .register(&path(name), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.explosion_declarations.get() {
        owner
            .lock()
            .map_err(|_| "explosion declaration registry is poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.explosion_declarations.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "explosion", marker);
        owner.report(catalog.identity(), "explosion", "scope-released", |_| 0);
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.explosion_declarations.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "explosion", "catalog-retiring", |_| 0);
    }
}
