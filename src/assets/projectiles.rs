//! Immutable arkProjectile projections. Constructors 784db0/623800 and
//! inherited component admission 87a4b0. Geometry, materials and damage retain
//! their canonical owners; this level table keeps only weak entity leases.
use crate::{
    archive::Catalog,
    entities::{self, Value},
    gameplay::damage::{atom, number},
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

#[derive(Debug)]
pub struct Declaration {
    pub entity: String,
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
    pub stick: StickSettings,
    pub deflection: Deflection,
    pub minimum_unbent_distance: f32,
    pub maximum_unbent_distance: f32,
    pub maximum_homing_angle: f32,
    pub stop_trail_on_hit: bool,
    pub switch_trail_on_time_suspended: bool,
    pub can_be_uncanny_deflected: bool,
    pub uncanny_offset_npc: f32,
    pub uncanny_offset_player: f32,
    pub sleep_headshot_time: f32,
    pub sleep_in_combat_time: f32,
    pub sleep_not_in_combat_time: f32,
    pub dont_explode_on_characters: bool,
    pub pinned_limb_forward_offset: f32,
    pub witness_missile_cue_on_impact: bool,
    pub effects: Effects,
}

/// One prepared source lease captured before a real weapon consumes a round.
/// The canonical entity factory owns body instantiation; the world-model
/// owner publishes this same model lease and acknowledges actual retirement.
pub struct Prepared {
    pub declaration: Arc<Declaration>,
    pub bounds_min: bevy_math::Vec3,
    pub bounds_max: bevy_math::Vec3,
    ///638ae0 queries the base model before639970 applies637e40's visual
    ///override. Final model bounds above still serve Initialize626eb0.
    pub sight_bounds: [[f32; 3]; 2],
    pub(crate) entity: Arc<crate::assets::runtime_entity::Definition>,
}
impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedProjectile")
            .field("entity", &self.declaration.entity)
            .finish_non_exhaustive()
    }
}
impl Prepared {
    pub(crate) fn from_entity(
        entity: Arc<crate::assets::runtime_entity::Definition>,
    ) -> Result<Arc<Self>, String> {
        Self::project(entity, None)
    }
    fn project(
        entity: Arc<crate::assets::runtime_entity::Definition>,
        sight_bounds: Option<[[f32; 3]; 2]>,
    ) -> Result<Arc<Self>, String> {
        let declaration = entity
            .projectile
            .clone()
            .ok_or("entity has no actual Projectile component")?;
        let model = entity
            .model
            .as_ref()
            .ok_or("projectile has no authored model")?;
        if entity.throwable.is_none() {
            return Err("projectile has no required Throwable component".into());
        }
        let bounds = model.geometry.bounds();
        Ok(Arc::new(Self {
            declaration,
            bounds_min: bevy_math::Vec3::from_array(bounds[0]),
            bounds_max: bevy_math::Vec3::from_array(bounds[1]),
            sight_bounds: sight_bounds.unwrap_or(bounds),
            entity,
        }))
    }
    pub(crate) fn from_weapon_entity(
        base: &crate::assets::runtime_entity::Definition,
        entity: Arc<crate::assets::runtime_entity::Definition>,
    ) -> Result<Arc<Self>, String> {
        let sight_bounds = base
            .model
            .as_ref()
            .ok_or("weapon projectile has no base render model")?
            .geometry
            .bounds();
        Self::project(entity, Some(sight_bounds))
    }
}

#[derive(Debug)]
pub struct StickSettings {
    pub can_recover: bool,
    pub components_to_attach: Vec<String>,
    pub minimum_yaw: f32,
    pub maximum_yaw: f32,
    pub random_yaw: bool,
    pub minimum_pitch: f32,
    pub maximum_pitch: f32,
    pub random_pitch: bool,
    pub minimum_sink: f32,
    pub sink: f32,
    pub sink_npc: f32,
    pub sink_random_range: f32,
    pub sink_pinned_limb: f32,
    pub model_change: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deflection {
    None,
    Deflect,
    Break,
}
impl Deflection {
    fn read(value: &str) -> Result<Self, String> {
        match value {
            "NO_DEFLECTION" | "0" => Ok(Self::None),
            "DEFLECT" | "1" => Ok(Self::Deflect),
            "BREAK" | "2" => Ok(Self::Break),
            _ => Err(format!("unknown projectile deflection type {value}")),
        }
    }
}
#[derive(Debug)]
pub struct Effects {
    pub fired_sound: Option<String>,
    pub trail: Option<String>,
    pub time_suspended_trail: Option<String>,
    pub break_fx: Option<String>,
    pub explosion: Option<String>,
    pub uncanny_npc: Option<String>,
    pub uncanny_player: Option<String>,
}
impl Declaration {
    /// Startup reservations use the existing FX owner; no flight-time I/O.
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        [
            &self.effects.trail,
            &self.effects.time_suspended_trail,
            &self.effects.break_fx,
            &self.effects.explosion,
            &self.effects.uncanny_npc,
            &self.effects.uncanny_player,
        ]
        .into_iter()
        .filter_map(|path| path.as_deref())
    }
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
            if let Some(value) = self.entities.get(&key).and_then(Weak::upgrade) {
                return Ok(value);
            }
        }
        self.entities.retain(|_, lease| lease.strong_count() != 0);
        // Source handoff is temporary: no archive access occurs in flight.
        let mut texts = HashMap::<String, Option<String>>::new();
        let mut read = |path: &str| {
            if let Some(text) = texts.get(path) {
                return Ok(text.clone());
            }
            let text = crate::assets::declaration_texts::read_common_or_catalog(catalog, path)?;
            texts.insert(path.to_owned(), text.clone());
            Ok::<_, String>(text)
        };
        let chain = entities::player::entity_chain(&key, &mut read)?;
        let mut class = None;
        for fields in &chain {
            if let Some(value) = atom(fields, "class") {
                class = Some(value.to_owned());
            }
        }
        if class.as_deref() != Some("arkProjectile") {
            return Err(format!(
                "{entity}: projectile class {:?} has no admitted runtime",
                class
            ));
        }
        let fields = entities::player::entity_component(&key, "cpntprojectile", &mut read)?
            .ok_or_else(|| format!("{entity}: arkProjectile has no projectile component"))?;
        let declaration = Declaration::from_resolved(catalog, &key, &fields)?;
        if !catalog.has_overlays() {
            self.entities.insert(key, Arc::downgrade(&declaration));
        }
        Ok(declaration)
    }
}
impl Declaration {
    pub(crate) fn from_resolved(
        catalog: &Catalog,
        entity: &str,
        fields: &[(String, Value)],
    ) -> Result<Arc<Self>, String> {
        let damage = child(&fields, "m_damageSettings");
        let falloff = child(damage, "m_falloff");
        let stick = child(&fields, "m_stickSettings");
        let breaking = child(&fields, "m_breakSettings");
        let bounce = child(&fields, "m_bounceSettings");
        let acquire_damage = |key| {
            text(damage, key)
                .map(|name| crate::assets::damage::Declaration::acquire(catalog, &name))
                .transpose()
        };
        let declaration = Arc::new(Declaration {
            entity: entity.to_owned(),
            material: text(&fields, "m_material"),
            linear_velocity: number(&fields, "m_linearVelocity", 0.)?,
            gravity_multiplier: number(&fields, "m_gravityMultiplier", 0.)?,
            roll_speed: number(&fields, "m_rollSpeed", 0.)?,
            npc_detection_extent: number(&fields, "m_npcDetectionExtent", 0.)?,
            contact_impulse: number(&fields, "m_contactImpulse", 30.)?,
            maximum_range: number(&fields, "m_maxRange", 0.)?,
            collision_layers: crate::gameplay::player::decls::layer_mask(
                &fields,
                "m_collisionLayers",
                0x4144be,
            )?,
            damage: acquire_damage("m_damage")?,
            sleep_damage: acquire_damage("m_sleepDamage")?,
            falloff_minimum: number(falloff, "m_minValue", 0.)?,
            falloff_maximum: number(falloff, "m_maxValue", 0.)?,
            required_stick_angle: number(stick, "m_requiredVelocityAngle", 0.)?,
            required_npc_stick_angle: number(stick, "m_requiredVelocityAngleOnNPC", 0.)?,
            can_break: boolean(breaking, "m_canBreakOnHit", false)?,
            break_chance_stickable: number(breaking, "m_chanceToBreakOnStickableSurface", 0.)?,
            break_chance_not_stickable: number(
                breaking,
                "m_chanceToBreakOnNotStickableSurface",
                100.,
            )?,
            break_chance_npc: number(breaking, "m_chanceToBreakOnNPC", 0.)?,
            bounce_angle: number(bounce, "m_requiredVelocityAngle", 45.)?,
            side_hit_restitution: number(bounce, "m_speedRestitutionOnSideHit", 5.)?,
            bounce_impulse_factor: number(
                bounce,
                "m_contactImpulseFactor",
                f32::from_bits(0x3ba3d70a),
            )?,
            bounce_impulse_random_angle: number(bounce, "m_contactImpulseRandomAngle", 45.)?,
            lifespan_after_hit: number(&fields, "m_lifespanAfterHit", 120.)?,
            lifespan_if_visible: number(&fields, "m_lifespanIfStillVisible", 10.)?,
            pass_through_water: boolean(&fields, "m_passThroughWater", true)?,
            stick: StickSettings {
                minimum_yaw: number(stick, "m_minYaw", 0.)?,
                maximum_yaw: number(stick, "m_maxYaw", 0.)?,
                random_yaw: boolean(stick, "m_randomYaw", false)?,
                minimum_pitch: number(stick, "m_minPitch", 0.)?,
                maximum_pitch: number(stick, "m_maxPitch", 0.)?,
                random_pitch: boolean(stick, "m_randomPitch", false)?,
                minimum_sink: number(stick, "m_minSinkDist", 0.)?,
                sink: number(stick, "m_sinkDist", 0.)?,
                sink_npc: number(stick, "m_sinkDistOnNPC", 0.)?,
                sink_random_range: number(stick, "m_sinkDistRandomRange", 0.)?,
                sink_pinned_limb: number(stick, "m_sinkDistWithPinnedLimb", 0.)?,
                model_change: text(stick, "m_modelChange"),
                can_recover: boolean(stick, "m_canRecover", true)?,
                components_to_attach: declaration_list(stick, "m_componentsToAttach")?,
            },
            deflection: Deflection::read(
                atom(&fields, "m_deflectionType").unwrap_or("NO_DEFLECTION"),
            )?,
            minimum_unbent_distance: number(
                &fields,
                "m_minUnbentDistance",
                f32::from_bits(0x3eb33333),
            )?,
            maximum_unbent_distance: number(
                &fields,
                "m_maxUnbentDistance",
                f32::from_bits(0x3ecccccd),
            )?,
            maximum_homing_angle: number(&fields, "m_maxHomingAngle", 300.)?,
            stop_trail_on_hit: boolean(&fields, "m_stopTrailFxOnHit", false)?,
            switch_trail_on_time_suspended: boolean(
                &fields,
                "m_switchTrailFxOnTimeSuspended",
                false,
            )?,
            can_be_uncanny_deflected: boolean(&fields, "m_canBeUncannyDeflected", false)?,
            uncanny_offset_npc: number(&fields, "m_uncannyDeflectionNpcOffset", 0.3)?,
            uncanny_offset_player: number(&fields, "m_uncannyDeflectionPlayerOffset", 0.3)?,
            sleep_headshot_time: number(damage, "m_headshotSleepTimer", 0.)?,
            sleep_in_combat_time: number(damage, "m_inCombatSleepTimer", 0.)?,
            sleep_not_in_combat_time: number(damage, "m_notInCombatSleepTimer", 0.)?,
            dont_explode_on_characters: boolean(&fields, "m_dontExplodeOnCharacters", false)?,
            pinned_limb_forward_offset: number(&fields, "m_pinnedLimbForwardOffset", 0.)?,
            witness_missile_cue_on_impact: boolean(
                &fields,
                "m_spawnWitnessMissileCueOnImpact",
                true,
            )?,
            effects: Effects {
                fired_sound: text(&fields, "m_firedSoundEvent"),
                trail: text(&fields, "m_trailFx"),
                time_suspended_trail: text(&fields, "m_bendtimeTrailFx"),
                break_fx: text(breaking, "m_breakFx"),
                explosion: text(&fields, "m_explosionFx"),
                uncanny_npc: text(&fields, "m_uncannyDeflectionNpcFx"),
                uncanny_player: text(&fields, "m_uncannyDeflectionPlayerFx"),
            },
        });
        Ok(declaration)
    }
}
fn child<'a>(fields: &'a [(String, Value)], key: &str) -> &'a [(String, Value)] {
    entities::scope(entities::field(fields, key)).unwrap_or(&[])
}
fn text(fields: &[(String, Value)], key: &str) -> Option<String> {
    atom(fields, key)
        .filter(|value| !value.is_empty() && *value != "NULL")
        .map(str::to_owned)
}
fn boolean(fields: &[(String, Value)], key: &str, default: bool) -> Result<bool, String> {
    match atom(fields, key) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid {key}: {value}")),
    }
}
fn declaration_list(fields: &[(String, Value)], key: &str) -> Result<Vec<String>, String> {
    let entries = child(fields, key);
    let count = atom(entries, "num")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| format!("invalid {key} declaration count"))?;
    let mut declarations = Vec::new();
    declarations
        .try_reserve(count)
        .map_err(|error| format!("{key} declarations: {error}"))?;
    for index in 0..count {
        let item = format!("item[{index}]");
        let name = atom(entries, &item)
            .ok_or_else(|| format!("missing {key}.{item} declaration reference"))?;
        if !name.is_empty() && !name.eq_ignore_ascii_case("NULL") {
            declarations.push(name.to_owned());
        }
    }
    Ok(declarations)
}
