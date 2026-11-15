//! Immutable arkProjectile projections. Constructors 784db0/623800 and
//! inherited component admission 87a4b0. Geometry, materials and damage retain
//! their canonical owners; this level table keeps only weak entity leases.
use crate::{archive::Catalog, entities::{self, Value}, gameplay::damage::{atom, number}};
use std::{collections::HashMap, sync::{Arc, Weak}};

#[derive(Debug)]
pub struct Declaration {
    pub entity: String,
    pub model: Option<String>,
    pub material: Option<String>,
    pub linear_velocity: f32,
    pub gravity_multiplier: f32,
    pub roll_speed: f32,
    pub npc_detection_extent: f32,
    pub contact_impulse: f32,
    pub maximum_range: f32,
    pub collision_layers: u32,
    pub damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub sleep_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub falloff_minimum: f32,
    pub falloff_maximum: f32,
    pub required_stick_angle: f32,
    pub required_npc_stick_angle: f32,
    pub can_break: bool,
    pub break_chance_stickable: f32,
    pub break_chance_not_stickable: f32,
    pub break_chance_npc: f32,
    pub bounce_angle: f32,
    pub side_hit_restitution: f32,
    pub bounce_impulse_factor: f32,
    pub bounce_impulse_random_angle: f32,
    pub lifespan_after_hit: f32,
    pub lifespan_if_visible: f32,
    pub pass_through_water: bool,
}

#[derive(Default)]
pub struct Sources {
    catalog: Option<u64>,
    entities: HashMap<String, Weak<Declaration>>,
}
impl Sources {
    pub fn acquire(&mut self, catalog: &Catalog, entity: &str) -> Result<Arc<Declaration>, String> {
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("projectile sources belong to another catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = entity.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(value) = self.entities.get(&key).and_then(Weak::upgrade) { return Ok(value); }
        }
        self.entities.retain(|_, lease| lease.strong_count() != 0);
        // Source handoff is temporary: no archive access occurs in flight.
        let mut texts = HashMap::<String, Option<String>>::new();
        let mut read = |path: &str| {
            if let Some(text) = texts.get(path) { return Ok(text.clone()); }
            let text = crate::assets::declaration_texts::read_common_or_catalog(catalog, path)?;
            texts.insert(path.to_owned(), text.clone());
            Ok::<_, String>(text)
        };
        let chain = entities::player::entity_chain(&key, &mut read)?;
        let mut class = None;
        let mut model = None;
        for fields in &chain {
            if let Some(value) = atom(fields, "class") { class = Some(value); }
            let info = child(child(fields, "edit"), "renderModelInfo");
            if let Some(value) = text(info, "model") { model = Some(value); }
        }
        if class != Some("arkProjectile") { return Err(format!("{entity}: projectile class {:?} has no admitted runtime", class)); }
        let fields = entities::player::entity_component(&key, "cpntprojectile", &mut read)?
            .ok_or_else(|| format!("{entity}: arkProjectile has no projectile component"))?;
        let damage = child(&fields, "m_damageSettings");
        let falloff = child(damage, "m_falloff");
        let stick = child(&fields, "m_stickSettings");
        let breaking = child(&fields, "m_breakSettings");
        let bounce = child(&fields, "m_bounceSettings");
        let acquire_damage = |key| text(damage, key).map(|name| crate::assets::damage::Declaration::acquire(catalog, &name)).transpose();
        let declaration = Arc::new(Declaration {
            entity: key.clone(), model, material: text(&fields, "m_material"),
            linear_velocity: number(&fields, "m_linearVelocity", 0.)?,
            gravity_multiplier: number(&fields, "m_gravityMultiplier", 0.)?,
            roll_speed: number(&fields, "m_rollSpeed", 0.)?,
            npc_detection_extent: number(&fields, "m_npcDetectionExtent", 0.)?,
            contact_impulse: number(&fields, "m_contactImpulse", 30.)?,
            maximum_range: number(&fields, "m_maxRange", 0.)?,
            collision_layers: crate::gameplay::player::decls::layer_mask(&fields, "m_collisionLayers", 0x4144be)?,
            damage: acquire_damage("m_damage")?, sleep_damage: acquire_damage("m_sleepDamage")?,
            falloff_minimum: number(falloff, "m_minValue", 0.)?,
            falloff_maximum: number(falloff, "m_maxValue", 0.)?,
            required_stick_angle: number(stick, "m_requiredVelocityAngle", 0.)?,
            required_npc_stick_angle: number(stick, "m_requiredVelocityAngleOnNPC", 0.)?,
            can_break: boolean(breaking, "m_canBreakOnHit", false)?,
            break_chance_stickable: number(breaking, "m_chanceToBreakOnStickableSurface", 0.)?,
            break_chance_not_stickable: number(breaking, "m_chanceToBreakOnNotStickableSurface", 100.)?,
            break_chance_npc: number(breaking, "m_chanceToBreakOnNPC", 0.)?,
            bounce_angle: number(bounce, "m_requiredVelocityAngle", 45.)?,
            side_hit_restitution: number(bounce, "m_speedRestitutionOnSideHit", 5.)?,
            bounce_impulse_factor: number(bounce, "m_contactImpulseFactor", f32::from_bits(0x3ba3d70a))?,
            bounce_impulse_random_angle: number(bounce, "m_contactImpulseRandomAngle", 45.)?,
            lifespan_after_hit: number(&fields, "m_lifespanAfterHit", 120.)?,
            lifespan_if_visible: number(&fields, "m_lifespanIfStillVisible", 10.)?,
            pass_through_water: boolean(&fields, "m_passThroughWater", true)?,
        });
        if !catalog.has_overlays() { self.entities.insert(key, Arc::downgrade(&declaration)); }
        Ok(declaration)
    }
}
fn child<'a>(fields: &'a [(String, Value)], key: &str) -> &'a [(String, Value)] {
    entities::scope(entities::field(fields, key)).unwrap_or(&[])
}
fn text(fields: &[(String, Value)], key: &str) -> Option<String> {
    atom(fields, key).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned)
}
fn boolean(fields: &[(String, Value)], key: &str, default: bool) -> Result<bool, String> {
    match atom(fields, key) { None => Ok(default), Some("true" | "1") => Ok(true), Some("false" | "0") => Ok(false), Some(value) => Err(format!("invalid {key}: {value}")) }
}
