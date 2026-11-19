//! Level-owned trigger bodies and component lifecycle. Render views borrow this
//! world; they do not build a second collision world for environment queries.
use crate::{
    archive,
    assets::entities::triggers::TriggerAssets,
    entities,
    gameplay::{
        collision_queries::CollisionQueries,
        component_state::{
            declaration::DeclarationScheduling,
            entity::{ComponentDeclaration, ComponentFactory, ComponentHandle, EntityComponents},
            lifecycle::ComponentLifecycleWorld,
            pool::ComponentPool,
            types::ComponentTypes,
            ComponentOwner, ComponentRegistry,
        },
        physics_actor::{ActorBody, PhysicsActor, PhysicsActorLinks},
        physics_world::{
            storage::WorldStorage, ComponentPublicationEvents, PublicationSignals, WorldInfo,
        },
        shape_point_query::ShapePointContact,
        trigger_body::{DeferredPhysicsState, TriggerComponent, TriggerComponentStore},
        water_component::WaterComponent,
        world_point_query::{
            level::LevelPointQueries, WorldPointQuery, WorldPointRequest, WorldQueryShape,
        },
    },
    rendering::environment_contacts::{ContactEntity, EnvironmentViewContacts},
};
use bevy::prelude::{default, Resource};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
};
#[macro_use]
mod publication;
#[macro_use]
mod rigid_family;
mod audio;
mod blink_queries;
mod player_queries;
pub(super) mod breakable;
pub(super) mod carrying_host;
pub(super) mod grapple_catch;
pub(in crate::app) mod grapple_release;
pub(super) mod carrying_resources;
pub(super) mod sound_volumes;
pub(super) mod damage_zone;
pub(in crate::app) mod occluded_sensor;
pub(super) mod damage;
mod loot;
pub(super) mod grenades;
mod ai_world;
pub(super) mod ai_sections;
pub(super) mod ai_managers;
mod ai_startup;
mod ai_cue_components;
mod ai_smart_objects;
mod ai_cue_events;
mod entity_activation;
mod crew;
mod crew_collectors;
mod crew_modules;
pub(super) mod document_hierarchy;
mod document_pickup;
mod document_rigid;
mod entity_messages;
mod heart_context;
mod heart_sensor;
mod entity_removal;
pub(super) mod explosions;
pub(super) mod explosion_effects;
mod interaction_components;
mod usable_static;
mod constraint;
mod constraint_removal;
mod flash_surfaces;
mod grenade_cooking;
pub(super) mod openable;
mod retargeting;
pub(in crate::app) mod fx_component;
pub(super) mod interaction_queries;
pub(super) mod level_bodies;
pub(super) mod player_reference;
mod equipment_reference;
pub(super) mod melee;
pub(super) mod preparation;
pub(super) mod projectiles;
pub(super) mod rigid_actor;
mod rigid_diagnostics;
pub(super) mod runtime_animation;
pub(super) mod runtime_entities;
pub(super) mod simulation;
mod startup;
pub(super) mod throwable;
mod trigger_events;
mod wind_bodies;
mod water_bodies;
pub(super) mod water_simulation;
mod water_impacts;
pub(super) mod water_publication;
pub(super) mod windblast;
use interaction_components::UsableComponent;
pub(super) mod lifetime;
pub(super) mod whale_oil_tank;

struct PhysicsEntity {
    /// Host slot reuse starts only after all native consumer acknowledgements.
    retired: bool,
    /// The actual Player owns its identity; the scene hosts its reference actor.
    external: bool,
    name: String,
    shared: crate::gameplay::entity_lifetime::EntityId,
    components: EntityComponents,
    class: usize,
    environment: Option<(usize, bool)>,
    flags: u8,
    entity_flags: u32,
    entity_flags_d7: u8,
    priority: i32,
    frame: crate::gameplay::reference_frame::PublishedFrame,
    timespace: u32,
}

#[derive(Resource)]
pub(super) struct PhysicsScene {
    pub(in crate::app) controller_feedback: Option<Arc<super::controller_feedback::Shared>>,
    owner_id: usize,
    contact_unbent_duration: Option<f32>,
    entity_flags_d4: Vec<u8>,
    /// Source-absence certificate for unhosted AITampering lookup in876ff0.
    /// One compact lane per actual header; None is never assumed absent.
    removal_tampering: Vec<Option<bool>>,
    post_updates: crate::gameplay::physics_world::update_queue::ObjectUpdateTask<usize>,
    post_frame: i32,
    pre_updates: crate::gameplay::physics_world::update_queue::ObjectUpdateTask<usize>,
    pre_frame: i32,
    rigid_pre_started: bool,
    rigid_post_started: bool,
    rigid_post_publication_cursor: [usize; 2],
    simulation: Option<simulation::PhysicsFrameDriver>,
    pre_path: Vec<usize>,
    body_commands: crate::gameplay::physics_world::commands::PhysicsCommands,
    rigid_post_path: Vec<usize>,
    body_activation_events:
        crate::gameplay::physics_world::body_activation_events::BodyActivationEvents,
    world: Arc<RwLock<LevelPointQueries>>,
    types: Arc<ComponentTypes>,
    grapple_dependency_ranges:[Option<[i32;2]>;5],
    factories: [u32; 61],
    classes: [i32; 61],
    entities: Vec<PhysicsEntity>,
    free_entities: Vec<usize>,
    shared_indices: HashMap<crate::gameplay::entity_lifetime::EntityId, usize>,
    declarations: HashMap<u64, ComponentDeclaration>,
    water_bindings: Vec<entities::water::WaterBinding>,
    water_declarations: Vec<crate::gameplay::water_component::startup::WaterDeclaration>,
    triggers: ComponentPool<TriggerComponent>,
    waters: ComponentPool<WaterComponent>,
    /// Wind payloads already live in the existing wind owner; these are their
    /// common component reference/lifecycle headers, not another wind field.
    winds: ComponentPool<ComponentOwner>,
    audios: ComponentPool<crate::gameplay::sound::component::AudioComponent>,
    usables: ComponentPool<UsableComponent>,
    static_usables: ComponentPool<UsableComponent>,
    constraints: ComponentPool<crate::gameplay::constraint::Component>,
    constraints_pending_enable: bool,
    static_usable_updates_pending: bool,
    usable_uses:std::collections::VecDeque<usable_static::AcceptedUse>,
    pickables: ComponentPool<crate::gameplay::pickable_component::PickableComponent>,
    readable_components: HashMap<String, crate::gameplay::pickable_component::CheckedComponent>,
    loots: ComponentPool<crate::gameplay::loot_component::LootComponent>,
    rigid_bodies: ComponentPool<super::rigid_body::RigidBodyComponent>,
    // Concrete subtype factories retain their own handles and generations.
    throwables: ComponentPool<super::rigid_body::RigidBodyComponent>,
    breakables: ComponentPool<breakable::BreakableComponent>,
    pub(in crate::app) throw_observation: Option<throwable::ThrowObservation>,
    pending_breakable_journals: breakable::PendingJournals,
    prepared_breakables: Vec<super::runtime_entities::PreparedBreakable>,
    prepared_sound_volumes: Vec<super::runtime_entities::PreparedSoundVolume>,
    prepared_openables: Vec<openable::Prepared>,
    openable_uses_pending: bool,
    openable_updates_pending: bool,
    sound_volumes: ComponentPool<crate::gameplay::sound::volume_control::Component>,
    damage_zones: ComponentPool<crate::gameplay::damage_zone::Component>,
    grenades: ComponentPool<crate::gameplay::grenade::Component>,
    ai_danger_zones: ComponentPool<crate::gameplay::ai_world::DangerComponent>,
    statistics: ComponentPool<crate::gameplay::ai_world::managers::Statistics>,
    timers: ComponentPool<crate::gameplay::ai_world::managers::Timers>,
    ai_cues: ComponentPool<crate::gameplay::ai_world::startup_managers::Cues>,
    ai_crews: ComponentPool<crate::gameplay::ai_world::startup_managers::CrewDispatch>,
    ai_dsm: ComponentPool<crate::gameplay::ai_world::startup_managers::Dsm>,
    ai_flee: ComponentPool<crate::gameplay::ai_world::startup_managers::Flee>,
    ai_coordination: ComponentPool<crate::gameplay::ai_world::startup_managers::Coordination>,
    ai_vision: ComponentPool<crate::gameplay::ai_world::startup_managers::Vision>,
    crew_cores: ComponentPool<crate::gameplay::ai_world::crew::Core>,
    crew_knowledge: ComponentPool<crate::gameplay::ai_world::crew::Knowledge>,
    crew_coordination: ComponentPool<crate::gameplay::ai_world::crew::Coordination>,
    crew_decisions: ComponentPool<crate::gameplay::ai_world::crew::Decision>,
    ai_crew_area: ComponentPool<crate::gameplay::ai_world::crew_collectors::Component>,
    ai_crew_combat: ComponentPool<crate::gameplay::ai_world::crew_collectors::Component>,
    ai_crew_notification: ComponentPool<crate::gameplay::ai_world::crew_collectors::Component>,
    ai_crew_search: ComponentPool<crate::gameplay::ai_world::crew_collectors::Component>,
    ai_crew_herd: ComponentPool<crate::gameplay::ai_world::crew_collectors::Component>,
    crew_chase: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_melee: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_search_module: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_link: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_ranged: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_fake: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_flank: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_hideout: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_tether: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    crew_alarm: ComponentPool<crate::gameplay::ai_world::crew_modules::Component>,
    observables: ComponentPool<crate::gameplay::ai_cues::Observable>,
    attention_sources: ComponentPool<crate::gameplay::ai_cues::AttentionSource>,
    investigate_spots: ComponentPool<crate::gameplay::ai_smart_objects::Component>,
    dynamic_search_spots: ComponentPool<crate::gameplay::ai_smart_objects::Component>,
    flash_surfaces: ComponentPool<flash_surfaces::Surface>,
    interactive_maps: ComponentPool<flash_surfaces::Map>,
    cooking_surfaces: ComponentPool<flash_surfaces::Surface>,
    openables: ComponentPool<openable::Component>,
    retargeters: ComponentPool<retargeting::Component>,
    fx_components: ComponentPool<fx_component::Component>,
    whale_oil_tanks: ComponentPool<whale_oil_tank::Component>,
    prepared_whale_oil_tanks: Vec<whale_oil_tank::Prepared>,
    whale_oil_tank_updates_pending: bool,
    fx_publications: Vec<crate::gameplay::pickable_component::CheckedComponent>,
    cue_actions:std::collections::VecDeque<ai_cue_events::Action>,
    cue_update_marker:Option<i32>,
    ///Host callback retry position in the actual clock's1/2/3 stage frame.
    ai_stage_frame:Option<(i32,u8)>,
    grenade_resources: HashMap<(u64, u32), grenades::Resources>,
    grenade_actions: std::collections::VecDeque<grenades::Action>,
    grenades_prepared: bool,
    damage_zone_effects: HashMap<(u64, u32), damage_zone::FxState>,
    projectiles: ComponentPool<super::projectiles::Component>,
    level_bodies: ComponentPool<level_bodies::PhysicsWorldComponent>,
    animations: ComponentPool<runtime_animation::AnimBaseComponent>,
    animation_audio_pending: bool,
    free_rigid_movement: Vec<usize>,
    rigid_wind_bodies: Vec<(ComponentHandle, u32)>,
    rigid_placements: HashMap<usize, usize>,
    initial_rigid_frames: HashMap<usize, crate::gameplay::reference_frame::ReferenceFrame>,
    document_hierarchy: document_hierarchy::DocumentHierarchy,
    factions: ComponentPool<super::faction_component::FactionComponent>,
    health: ComponentPool<damage::HealthComponent>,
    health_doors: ComponentPool<damage::DoorHealthComponent>,
    damage_player: Option<damage::PlayerDamageContext>,
    faction_session: crate::gameplay::faction_registry::SessionFactionRegistry,
    faction_manager: Option<crate::gameplay::faction_registry::ActiveFactionManager>,
    audio_bindings: Vec<entities::audio::AudioBinding>,
    audio_handles: Vec<ComponentHandle>,
    actors: PhysicsActorLinks,
    registry: Option<ComponentRegistry>,
    deferred: DeferredPhysicsState,
    signals: PublicationSignals<usize, usize>,
    contacts: crate::gameplay::water_contacts::WaterContactManager,
    water_notifications: std::collections::VecDeque<(crate::gameplay::entity_lifetime::EntityId,bool)>,
    water_simulation:water_simulation::WaterSimulation,
    water_impacts:std::collections::VecDeque<water_simulation::WaterImpact>,
    trigger_material: u16,
    trigger_range: [i32; 2],
    area_range: [i32; 2],
    dependency_ranges: [[i32; 2]; 2],
    queries: CollisionQueries<WorldPointRequest, ShapePointContact>,
    sensor_queries: Option<occluded_sensor::SensorQueries>,
    executor: WorldPointQuery,
    retirement: Option<lifetime::Retirement>,
    gameplay_component_drain: Option<lifetime::Retirement>,
}

impl PhysicsScene {
    pub(in crate::app) fn contact_unbent_duration(&self) -> Option<f32> { self.contact_unbent_duration }

    pub(in crate::app) fn publish_entity_timespace(&mut self,live:&crate::gameplay::entity_lifetime::LiveEntities,
        id:crate::gameplay::entity_lifetime::EntityId,space:u32)->Result<(),String>{
        self.entity_time_binding(live,id)?;
        if space>1{return Err("entity time transition selector is outside world/unbent".into());}
        let index=self.shared_indices[&id];
        self.entities[index].timespace=space;
        Ok(())
    }

    /// Raw entityD1 bit2 is the source's explicit time-sync exclusion.
    pub(in crate::app) fn entity_time_binding(&self, live: &crate::gameplay::entity_lifetime::LiveEntities,
        id: crate::gameplay::entity_lifetime::EntityId) -> Result<(u32,bool),String> {
        if live.get(id).is_none(){return Err("time-sync entity generation is stale".into());}
        let &index=self.shared_indices.get(&id).ok_or("time-sync entity has no canonical scene owner")?;
        let entity=self.entities.get(index).ok_or("time-sync entity slot is absent")?;
        if entity.retired || entity.shared != id{return Err("time-sync entity owner is retired".into());}
        Ok((entity.timespace,entity.entity_flags & 0x400 != 0))
    }

    pub(super) fn entity_frame(
        &self,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
        id: crate::gameplay::entity_lifetime::EntityId,
    ) -> Result<Option<crate::gameplay::reference_frame::ReferenceFrame>, String> {
        let Some((name, class)) = live.get(id) else {
            return Err("entity frame identity is stale".into());
        };
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let entity = self
            .entities
            .get(index)
            .ok_or("entity frame index is stale")?;
        if entity.class != class || !entity.name.eq_ignore_ascii_case(name) {
            return Err("entity frame owner differs".into());
        }
        if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
            return Ok(None);
        }
        if let Some(handle) = entity.components.reference_actor {
            let actor = self
                .actors
                .actors
                .get(handle.slot)
                .ok_or("entity reference actor absent")?;
            // The reference actor owns the current physics transform. No render
            // placement frame is substituted for an absent actor.
            return Ok(Some(actor.frame.current));
        }
        Ok(Some(entity.frame.pose))
    }

    pub(super) fn audio_component_live(&self, packed: u64, generation: u32) -> bool {
        let Ok(handle) = ComponentHandle::from_packed(packed) else {
            return false;
        };
        handle.class_id == self.classes[4]
            && self.audios.get(handle.slot).is_some_and(|audio| {
                audio.started
                    && audio.owner.entity.is_some()
                    && audio.owner.generation == generation
            })
    }

    pub(super) fn audio_owner_timespace(&self, packed: u64, generation: u32) -> Option<u32> {
        let handle = ComponentHandle::from_packed(packed).ok()?;
        if handle.class_id != self.classes[4] {
            return None;
        }
        let audio = self.audios.get(handle.slot)?;
        if !audio.started || audio.owner.generation != generation {
            return None;
        }
        self.entities
            .get(audio.owner.entity?)
            .map(|entity| entity.timespace)
    }

    pub(super) fn visit_audio(
        &mut self,
        mut visit: impl FnMut(
            ComponentHandle,
            &entities::audio::AudioBinding,
            &mut crate::gameplay::sound::component::AudioComponent,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        for index in 0..self.audios.chunk_count() * self.audios.items_per_chunk {
            let Some(audio) = self.audios.get_at(index) else {
                continue;
            };
            let handle = ComponentHandle {
                class_id: self.classes[4],
                slot: u16::try_from(index).map_err(|_| "audio slot exceeds pool identity")?,
            };
            let runtime = audio.runtime_definition.clone();
            let declaration = audio.owner.declaration;
            let binding = match runtime.as_deref() {
                Some(binding) => binding,
                None => self
                    .audio_bindings
                    .get(declaration.ok_or("audio instance declaration is absent")?)
                    .ok_or("audio instance definition is absent")?,
            };
            let audio = self
                .audios
                .get_at_mut(index)
                .ok_or("audio instance disappeared")?;
            visit(handle, binding, audio)?;
        }
        Ok(())
    }

    pub(super) fn point_queries(&self) -> Result<Arc<RwLock<LevelPointQueries>>, String> {
        let query_owner = self
            .world
            .read()
            .map_err(|_| "level physics query owner is poisoned".to_owned())?;
        query_owner.ready().map_err(str::to_owned)?;
        drop(query_owner);
        Ok(self.world.clone())
    }

    fn finish_point_queries(&mut self) -> Result<(), String> {
        let shared_world = self.world.clone();
        let world = shared_world
            .read()
            .map_err(|_| "level physics query owner is poisoned")?;
        let owners = world.resolve_wind_owners(|body| {
            let data =
                crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data)?;
            let entity = data.entity(|handle| self.owner(handle).ok())?;
            let entity = self
                .entities
                .get(entity)
                .ok_or("wind collision entity absent")?;
            let class = self.types.classes().hierarchy().type_id(entity.class)?;
            if !(self.trigger_range[0]..=self.trigger_range[1]).contains(&class) {
                return Ok(None);
            }
            if data.component.class_id != self.classes[0] {
                return Err("wind query trigger body has incompatible component owner".into());
            }
            let trigger = self
                .triggers
                .get(data.component.slot)
                .ok_or("wind collision trigger component absent")?;
            let Some(wind) = trigger.wind else {
                return Ok(None);
            };
            let entity = self
                .owner(wind)?
                .entity
                .ok_or("trigger wind has no entity")?;
            Ok(Some(
                self.entities
                    .get(entity)
                    .ok_or("trigger wind entity absent")?
                    .name
                    .clone(),
            ))
        });
        drop(world);
        // Publish through the existing owner after all component startup writes.
        // Once published, render/FX leases share the same immutable geometry.
        let mut query_owner = self
            .world
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *query_owner;
        world.geometry.release_immutable_sources();
        world.publish_wind_owners(owners);
        Ok(())
    }

    /// The point-query half of1fc710. This returns the selected body's packed
    /// AABB top (c85490), independently of the view's water component plane.
    /// The caller still owns the downward-ray fallback when this returns None.
    pub(super) fn particle_water_body_height(
        &mut self,
        position: [f32; 3],
    ) -> Result<Option<f32>, String> {
        self.world
            .read()
            .map_err(|_| "level physics query owner is poisoned")?
            .particle_water_body_height(&mut self.executor, position)
    }

    pub(super) fn load(
        catalog: &archive::Catalog,
        document: &entities::Document,
        map: &str,
        volumes: &[crate::rendering::environment::RuntimeEnvironmentVolume],
        texts: &mut crate::assets::declaration_texts::PhysicsSeed,
        shape_sources: &mut crate::physworld::source::ShapeSources,
        live: &mut crate::gameplay::entity_lifetime::LiveEntities,
        interactions: Option<&super::interactions::Interactions>,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents,
        models: &[entities::PlacedModel],
        joint_frames: &super::joint_frames::JointFrames,
        level_bodies: Option<&level_bodies::PreparedLevelBodies>,
        constraint_admission: &super::machinery::ConstraintAdmission,
    ) -> Result<Self, String> {
        let types = ComponentTypes::acquire(catalog)?;
        let constructor_tag = types.intern_name(Some(b"MAIN"))?;
        let factory = |name: &[u8]| {
            types
                .factories()
                .iter()
                .find(|p| {
                    types
                        .classes()
                        .metadata(p.class)
                        .and_then(|class| types.classes().name_text(class.name))
                        == Some(name)
                })
                .map(|p| p.pool_index)
                .ok_or_else(|| {
                    format!(
                        "physics component factory absent: {}",
                        String::from_utf8_lossy(name)
                    )
                })
        };
        // Pool32 is the installed reference-frame actor factory, audited with
        // the same executable metadata as its constructor and body interfaces.
        let factories = [
            factory(b"arkCpntTrigger")?,
            factory(b"arkCpntWater")?,
            factory(b"arkCpntWind")?,
            32,
            factory(b"arkCpntAudio")?,
            factory(b"arkCpntUsableWithStateList")?,
            factory(b"arkCpntPickable_Simple")?,
            factory(b"arkCpntLoot")?,
            factory(b"arkCpntRigidBody")?,
            factory(b"arkCpntFaction")?,
            factory(b"arkCpntHealth")?,
            factory(b"arkCpntBreakable")?,
            factory(b"arkCpntRigidBodyThrowable")?,
            factory(b"arkCpntProjectile")?,
            factory(b"arkCpntPhysicsWorld")?,
            factory(b"arkCpntAnimBase")?,
            factory(b"arkCpntHealthDoor")?,
            factory(b"arkCpntSoundVolumeControl")?,
            factory(b"arkCpntDamageZone")?,
            factory(b"arkCpntGrenade")?,
            factory(b"arkCpntAIDangerZone")?,
            factory(b"arkCpntAIStatistics")?,
            factory(b"arkCpntTimersManager")?,
            factory(b"arkCpntAIManager_Cues")?,
            factory(b"arkCpntAICrewDispatch")?,
            factory(b"arkCpntDSM")?,
            factory(b"arkCpntFleeManager")?,
            factory(b"arkCpntAIWorldCoordinationBB")?,
            factory(b"arkCpntAIVisionManager")?,
            factory(b"arkCpntAICrewCore")?,
            factory(b"arkCpntAICrewKnowledgeBB")?,
            factory(b"arkCpntAICrewCoordinationBB")?,
            factory(b"arkCpntAICrewDecision")?,
            factory(b"arkCpntAICrewCollector_Area")?,
            factory(b"arkCpntAICrewCollector_Combat")?,
            factory(b"arkCpntAICrewCollector_Notification")?,
            factory(b"arkCpntAICrewCollector_Search")?,
            factory(b"arkCpntAICrewCollector_Herd")?,
            factory(b"arkCpntAICrewModule_CombatChase")?,
            factory(b"arkCpntAICrewModule_CombatMelee")?,
            factory(b"arkCpntAICrewModule_Search")?,
            factory(b"arkCpntAICrewModule_Spot_Link")?,
            factory(b"arkCpntAICrewModule_CombatRanged")?,
            factory(b"arkCpntAICrewModule_CombatFake")?,
            factory(b"arkCpntAICrewModule_CombatFlank")?,
            factory(b"arkCpntAICrewModule_Spot_Hideout")?,
            factory(b"arkCpntAICrewModule_CombatTether")?,
            factory(b"arkCpntAICrewModule_CombatAlarm")?,
            factory(b"arkCpntObservable")?,
            factory(b"arkCpntAIAttentionSource")?,
            factory(b"arkCpntAIInvestigateSpot")?,
            factory(b"arkCpntAISearchSpot_Dynamic")?,
            factory(b"arkCpntFlashSurface")?,
            factory(b"arkCpntInteractiveMap")?,
            factory(b"arkCpntGrenadeCooking")?,
            factory(b"arkCpntOpenable")?,
            factory(b"arkCpntRetargeting")?,
            factory(b"arkCpntFX")?,
            factory(b"arkCpntUsableStatic")?,
            factory(b"arkCpntConstraint")?,
            factory(b"arkCpntWhaleOilTank")?,
        ];
        let names = types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .map(|&index| {
                let class = types
                    .classes()
                    .metadata(index)
                    .ok_or("physics class metadata absent")?;
                let name = types
                    .classes()
                    .name_text(class.name)
                    .ok_or("physics class name absent")?;
                Ok((name.to_vec(), index))
            })
            .collect::<Result<HashMap<_, _>, String>>()?;
        let hierarchy = types.classes().hierarchy();
        let entity_range = |name: &[u8]| {
            hierarchy.type_range(*names.get(name).ok_or("physics entity class absent")?)
        };
        let grapple_dependency_ranges=[b"arkCpntInteractable".as_slice(),b"arkCpntNavigation".as_slice(),
            b"arkMovable".as_slice(),b"arkCpntAIPhysicsReaction".as_slice(),b"arkCpntPhysicsSystemEvolved".as_slice()]
            .map(|name|names.get(name).map(|&index|hierarchy.type_range(index)).transpose());
        let [interactable_range,navigation_range,movable_range,reaction_range,evolved_range]=grapple_dependency_ranges;
        let grapple_dependency_ranges=[interactable_range?,navigation_range?,movable_range?,reaction_range?,evolved_range?];
        let trigger_range = entity_range(b"arkTrigger")?;
        let area_range = entity_range(b"arkAreaVolume")?;
        let mut classes = [0; 61];
        for i in 0..factories.len() {
            classes[i] = hierarchy.type_id(types.factory(factories[i])?.class)?;
        }
        let dependency_ranges = [
            hierarchy.type_range(types.factory(factories[1])?.class)?,
            hierarchy.type_range(types.factory(factories[2])?.class)?,
        ];
        let trace_shape_cpu = std::env::var("DH2_DEBUG_TRIGGER_SOURCE_CPU").as_deref() == Ok("1");
        let shape_cpu_start = if trace_shape_cpu {
            crate::loading::thread_cycles()
        } else {
            None
        };
        let geometry = TriggerAssets::load_with_sources(document, map, catalog, shape_sources)?;
        if let (Some(start), Some(end)) = (
            shape_cpu_start,
            if trace_shape_cpu {
                crate::loading::thread_cycles()
            } else {
                None
            },
        ) {
            eprintln!(
                "MEMORY trigger acquisition CPU: shapes={} cycles={}",
                geometry.shapes.len(),
                end.saturating_sub(start)
            );
        }
        let read = |name: &str| {
            catalog
                .read(name)
                .map(|v| v.map(|b| entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let water_bindings = entities::water::water_bindings(document, read)?;
        let audio_bindings = entities::audio::audio_bindings(document, catalog, read)?;
        let wind_scheduling = entities::component_scheduling(
            document,
            "cpntwind",
            DeclarationScheduling::from_base_flags(0),
            read,
        )?;
        let definitions = document
            .definitions
            .iter()
            .filter(|d| d.kind == "entity")
            .map(|d| (d.id.as_str(), d))
            .collect::<HashMap<_, _>>();
        let volume_indices = volumes
            .iter()
            .enumerate()
            .map(|(i, v)| (v.id.as_str(), (i, v.is_water_volume)))
            .collect::<HashMap<_, _>>();
        let constants = crate::assets::physics_material::MaterialConstants::acquire(catalog)?;
        let mut materials = crate::gameplay::materials::MaterialOwner::new(constants, 16)?;
        //0932d0 writes08b6d0's code-resource declaration into preset0+60.
        //The registered descriptor142283fc8 names default_preset; ordinary
        //surfaces with no physics override legitimately keep material ID0.
        let default_preset = materials.declaration_with_texts(catalog, "default_preset", texts)?;
        materials.bind_default_preset(default_preset)?;
        let declaration =
            materials.declaration_with_texts(catalog, "narrow_phase_trigger", texts)?;
        let trigger_material = materials.resolve(declaration, |_, _, _, _| {
            panic!("material subscriber before level initialization")
        })?;
        let water_declarations = water_bindings
            .iter()
            .map(|binding| {
                binding.startup(|name| materials.declaration_with_texts(catalog, name, texts))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let bounds = entities::physics::world_physics_bounds(document)?
            .ok_or("physics worldspawn bounds absent")?;
        static NEXT_WORLD: AtomicUsize = AtomicUsize::new(1);
        let owner_id = NEXT_WORLD.fetch_add(1, Ordering::Relaxed);
        let mut world = WorldStorage::new(WorldInfo::game_settings(bounds), owner_id, materials)?;
        //093a40: these actual userdata vtables all use06d040(null matrix)
        // at+20. Other body families must publish their real matrix policy.
        for index in [0, 8, 12, 14] {
            world.register_collision_class_without_matrix(classes[index]);
        }
        world.initialize_game_timespaces()?;
        let faction_session =
            crate::gameplay::faction_registry::SessionFactionRegistry::acquire(catalog)?;
        faction_session.acquire_all_registered(catalog)?;
        let faction_manager = Some(faction_session.snapshot()?.into_manager()?);
        let cvars = crate::platform::console::shared().read();
        let mut scene = Self {
            controller_feedback: None,
            owner_id,
            contact_unbent_duration: entities::physics::world_contact_unbent_duration(document)?,
            entity_flags_d4: Vec::new(),
            removal_tampering: Vec::new(),
            post_updates: Default::default(),
            post_frame: 0,
            pre_updates: Default::default(),
            pre_frame: 0,
            rigid_pre_started: false,
            rigid_post_started: false,
            rigid_post_publication_cursor: [0; 2],
            simulation: Some(simulation::PhysicsFrameDriver::new()),
            pre_path: Vec::new(),
            body_commands: crate::gameplay::physics_world::commands::PhysicsCommands::new()?,
            body_activation_events: Default::default(),
            world: Arc::new(RwLock::new(LevelPointQueries::new(world, geometry))),
            triggers: types.create_pool(factories[0], &cvars.cvars)?,
            waters: types.create_pool(factories[1], &cvars.cvars)?,
            winds: types.create_pool(factories[2], &cvars.cvars)?,
            audios: types.create_pool(factories[4], &cvars.cvars)?,
            usables: types.create_pool(factories[5], &cvars.cvars)?,
            static_usables: types.create_pool(factories[58], &cvars.cvars)?,
            constraints: types.create_pool(factories[59], &cvars.cvars)?,
            constraints_pending_enable: false,
            static_usable_updates_pending:false,
            usable_uses:Default::default(),
            pickables: types.create_pool(factories[6], &cvars.cvars)?,
            readable_components: HashMap::new(),
            loots: types.create_pool(factories[7], &cvars.cvars)?,
            rigid_bodies: types.create_pool(factories[8], &cvars.cvars)?,
            breakables: types.create_pool(factories[11], &cvars.cvars)?,
            throw_observation: None,
            pending_breakable_journals: Default::default(),
            prepared_breakables: Vec::new(),
            prepared_sound_volumes: Vec::new(),
            prepared_openables: Vec::new(),
            openable_uses_pending: false,
            openable_updates_pending: false,
            sound_volumes: types.create_pool(factories[17], &cvars.cvars)?,
            damage_zones: types.create_pool(factories[18], &cvars.cvars)?,
            grenades: types.create_pool(factories[19], &cvars.cvars)?,
            ai_danger_zones: types.create_pool(factories[20], &cvars.cvars)?,
            statistics: types.create_pool(factories[21], &cvars.cvars)?,
            timers: types.create_pool(factories[22], &cvars.cvars)?,
            ai_cues: types.create_pool(factories[23], &cvars.cvars)?,
            ai_crews: types.create_pool(factories[24], &cvars.cvars)?,
            ai_dsm: types.create_pool(factories[25], &cvars.cvars)?,
            ai_flee: types.create_pool(factories[26], &cvars.cvars)?,
            ai_coordination: types.create_pool(factories[27], &cvars.cvars)?,
            ai_vision: types.create_pool(factories[28], &cvars.cvars)?,
            crew_cores: types.create_pool(factories[29], &cvars.cvars)?,
            crew_knowledge: types.create_pool(factories[30], &cvars.cvars)?,
            crew_coordination: types.create_pool(factories[31], &cvars.cvars)?,
            crew_decisions: types.create_pool(factories[32], &cvars.cvars)?,
            ai_crew_area: types.create_pool(factories[33], &cvars.cvars)?,
            ai_crew_combat: types.create_pool(factories[34], &cvars.cvars)?,
            ai_crew_notification: types.create_pool(factories[35], &cvars.cvars)?,
            ai_crew_search: types.create_pool(factories[36], &cvars.cvars)?,
            ai_crew_herd: types.create_pool(factories[37], &cvars.cvars)?,
            crew_chase: types.create_pool(factories[38], &cvars.cvars)?,
            crew_melee: types.create_pool(factories[39], &cvars.cvars)?,
            crew_search_module: types.create_pool(factories[40], &cvars.cvars)?,
            crew_link: types.create_pool(factories[41], &cvars.cvars)?,
            crew_ranged: types.create_pool(factories[42], &cvars.cvars)?,
            crew_fake: types.create_pool(factories[43], &cvars.cvars)?,
            crew_flank: types.create_pool(factories[44], &cvars.cvars)?,
            crew_hideout: types.create_pool(factories[45], &cvars.cvars)?,
            crew_tether: types.create_pool(factories[46], &cvars.cvars)?,
            crew_alarm: types.create_pool(factories[47], &cvars.cvars)?,
            observables: types.create_pool(factories[48], &cvars.cvars)?,
            attention_sources: types.create_pool(factories[49], &cvars.cvars)?,
            investigate_spots: types.create_pool(factories[50], &cvars.cvars)?,
            dynamic_search_spots: types.create_pool(factories[51], &cvars.cvars)?,
            flash_surfaces: types.create_pool(factories[52], &cvars.cvars)?,
            interactive_maps: types.create_pool(factories[53], &cvars.cvars)?,
            cooking_surfaces: types.create_pool(factories[54], &cvars.cvars)?,
            openables: types.create_pool(factories[55], &cvars.cvars)?,
            retargeters: types.create_pool(factories[56], &cvars.cvars)?,
            fx_components: types.create_pool(factories[57], &cvars.cvars)?,
            whale_oil_tanks: types.create_pool(factories[60], &cvars.cvars)?,
            prepared_whale_oil_tanks: Vec::new(),
            whale_oil_tank_updates_pending:false,
            fx_publications: Vec::new(),
            cue_actions:Default::default(),
            cue_update_marker:None,
            ai_stage_frame:None,
            grenade_resources: Default::default(),
            grenade_actions: Default::default(),
            grenades_prepared: false,
            damage_zone_effects: HashMap::new(),
            throwables: types.create_pool(factories[12], &cvars.cvars)?,
            projectiles: types.create_pool(factories[13], &cvars.cvars)?,
            level_bodies: types.create_pool(factories[14], &cvars.cvars)?,
            animations: types.create_pool(factories[15], &cvars.cvars)?,
            animation_audio_pending: false,
            free_rigid_movement: Vec::new(),
            rigid_wind_bodies: Vec::new(),
            rigid_post_path: Vec::new(),
            rigid_placements: HashMap::new(),
            initial_rigid_frames: HashMap::new(),
            document_hierarchy: Default::default(),
            factions: types.create_pool(factories[9], &cvars.cvars)?,
            health: types.create_pool(factories[10], &cvars.cvars)?,
            health_doors: types.create_pool(factories[16], &cvars.cvars)?,
            damage_player: None,
            faction_session,
            faction_manager,
            audio_bindings,
            audio_handles: Vec::new(),
            actors: PhysicsActorLinks {
                actors: types.create_pool(factories[3], &cvars.cvars)?,
                bodies: Vec::new(),
            },
            types,
            grapple_dependency_ranges,
            factories,
            classes,
            entities: Vec::new(),
            free_entities: Vec::new(),
            shared_indices: HashMap::new(),
            declarations: HashMap::new(),
            water_bindings,
            water_declarations,
            registry: Some(ComponentRegistry::new()?),
            deferred: DeferredPhysicsState {
                world_flags: [0; 2],
                component_flag: 0,
            },
            signals: default(),
            contacts: crate::gameplay::water_contacts::WaterContactManager::new()?,
            water_notifications:Default::default(),
            water_simulation:Default::default(),
            water_impacts:Default::default(),
            trigger_material,
            trigger_range,
            area_range,
            dependency_ranges,
            queries: CollisionQueries::default(),
            sensor_queries: Some(occluded_sensor::SensorQueries::default()),
            executor: WorldPointQuery::new(u32::MAX)?,
            retirement: None,
            gameplay_component_drain: None,
        };
        drop(cvars);
        startup::install_activation_observers(&mut scene.signals, owner_id);
        scene.queries.set_submission_allowed(true);
        let mut registry = scene
            .registry
            .take()
            .ok_or("physics component registry absent")?;
        let preparation = (|| -> Result<(), String> {
            let mut entity_indices = HashMap::<String, usize>::new();
            let query_owner = scene.world.clone();
            let query_world = query_owner
                .read()
                .map_err(|_| "level physics query owner is poisoned")?;
            for index in 0..query_world.geometry.placements.len() {
                let placement = query_world.geometry.placements[index]
                    .as_ref()
                    .ok_or("prepared trigger placement absent")?;
                let existing = entity_indices.get(&placement.binding.entity_id).copied();
                let entity_index = existing.unwrap_or(scene.entities.len());
                let definition = definitions
                    .get(placement.binding.entity_id.as_str())
                    .ok_or("trigger entity declaration absent")?;
                let class = definition
                    .fields
                    .iter()
                    .find_map(|(k, v)| {
                        if k == "class" {
                            if let entities::Value::Atom(v) = v {
                                Some(v.trim_matches('"'))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .ok_or("trigger entity class name absent")?;
                let class = *names
                    .get(class.as_bytes())
                    .ok_or_else(|| format!("trigger entity class unsupported: {class}"))?;
                if existing.is_none() {
                    let shared = live.allocate_managed(
                        Arc::from(placement.binding.entity_id.as_str()),
                        class,
                        None,
                        0,
                        constructor_tag,
                    )?;
                    let entity = PhysicsEntity {
                        retired: false,
                        external: false,
                        name: placement.binding.entity_id.clone(),
                        shared,
                        components: EntityComponents::new(entity_index),
                        class,
                        environment: volume_indices
                            .get(placement.binding.entity_id.as_str())
                            .copied(),
                        flags: 0,
                        entity_flags: 0x001e_8008,
                        entity_flags_d7: 0,
                        priority: 0,
                        frame: crate::gameplay::reference_frame::PublishedFrame {
                            pose: crate::gameplay::reference_frame::ReferenceFrame {
                                position: placement.position,
                                basis: placement.shape.basis,
                            },
                            ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                                .published()
                        },
                        timespace: 0,
                    };
                    // Publish the allocation to its sole scene owner before any
                    // fallible factory callback, so cancellation can find it.
                    scene.entities.push(entity);
                    scene.entity_flags_d4.push(0);
                    let entity = &mut scene.entities[entity_index];
                    for (binding_index, binding) in scene
                        .water_bindings
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| b.entity_id == placement.binding.entity_id)
                    {
                        let declaration = scene.types.declaration(
                            factories[1],
                            binding_index,
                            binding.scheduling.clone(),
                        )?;
                        let handle = ComponentFactory {
                            declaration: &declaration,
                            pool: &mut scene.waters,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut entity.components,
                            declaration.scheduling.starts_on_spawn(),
                            |_, _, _| Ok(WaterComponent::default()),
                            |_, _, _| Ok(()),
                        )?
                        .ok_or("water component creation disabled during level load")?;
                        scene.declarations.insert(handle.packed(), declaration);
                    }
                    for (_, _, scheduling) in wind_scheduling
                        .iter()
                        .filter(|(id, _, _)| id == &placement.binding.entity_id)
                    {
                        let declaration =
                            scene
                                .types
                                .declaration(factories[2], index, scheduling.clone())?;
                        // Trigger dependency lookup only reads this common header.
                        let handle = ComponentFactory {
                            declaration: &declaration,
                            pool: &mut scene.winds,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut entity.components,
                            false,
                            |_, _, _| Ok(ComponentOwner::default()),
                            |_, _, _| Ok(()),
                        )?
                        .ok_or("wind reference creation disabled during level load")?;
                        scene.declarations.insert(handle.packed(), declaration);
                    }
                    entity_indices.insert(placement.binding.entity_id.clone(), entity_index);
                }
                let entity = &mut scene.entities[entity_index];
                let declaration = scene.types.declaration(
                    factories[0],
                    index,
                    placement.binding.scheduling.clone(),
                )?;
                let handle = ComponentFactory {
                    declaration: &declaration,
                    pool: &mut scene.triggers,
                    registry: &mut registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut entity.components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| TriggerComponent::new(&mut scene.actors.bodies),
                    |_, _, _| Ok(()),
                )?
                .ok_or("trigger component creation disabled during level load")?;
                scene.declarations.insert(handle.packed(), declaration);
            }
            // Audio components share the same entity headers, class metadata,
            // generation allocator and ordered lifecycle batch as physics owners.
            for index in 0..scene.audio_bindings.len() {
                let binding = &scene.audio_bindings[index];
                let entity_index = match entity_indices.get(&binding.entity_id).copied() {
                    Some(index) => index,
                    None => {
                        let definition = definitions
                            .get(binding.entity_id.as_str())
                            .ok_or("audio entity definition absent")?;
                        let class = definition
                            .fields
                            .iter()
                            .find_map(|(key, value)| {
                                if key != "class" {
                                    return None;
                                }
                                if let entities::Value::Atom(value) = value {
                                    Some(value.trim_matches('"'))
                                } else {
                                    None
                                }
                            })
                            .ok_or("audio entity class name absent")?;
                        let class = *names
                            .get(class.as_bytes())
                            .ok_or_else(|| format!("audio entity class unsupported: {class}"))?;
                        let index = scene.entities.len();
                        let shared = live.allocate_managed(
                            Arc::from(binding.entity_id.as_str()),
                            class,
                            None,
                            0,
                            constructor_tag,
                        )?;
                        scene.entities.push(PhysicsEntity {
                            retired: false,
                            external: false,
                            name: binding.entity_id.clone(),
                            shared,
                            components: EntityComponents::new(index),
                            class,
                            environment: volume_indices.get(binding.entity_id.as_str()).copied(),
                            flags: 0,
                            entity_flags: 0x001e_8008,
                            entity_flags_d7: 0,
                            priority: 0,
                            frame: crate::gameplay::reference_frame::PublishedFrame {
                                pose: binding.frame,
                                ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                                    .published()
                            },
                            timespace: 0,
                        });
                        scene.entity_flags_d4.push(0);
                        entity_indices.insert(binding.entity_id.clone(), index);
                        index
                    }
                };
                let declaration =
                    scene
                        .types
                        .declaration(factories[4], index, binding.scheduling.clone())?;
                let handle = ComponentFactory {
                    declaration: &declaration,
                    pool: &mut scene.audios,
                    registry: &mut registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut scene.entities[entity_index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(crate::gameplay::sound::component::AudioComponent::default()),
                    |_, _, _| Ok(()),
                )?
                .ok_or("audio component creation disabled during level load")?;
                scene.declarations.insert(handle.packed(), declaration);
                scene.audio_handles.push(handle);
            }
            drop(query_world);
            scene.admit_placed_flash_components(catalog,document,live,&mut registry,constructor_tag)?;
            if let Some(interactions) = interactions {
                scene.admit_interaction_components(
                    document,
                    interactions,
                    live,
                    &mut registry,
                    constructor_tag,
                )?;
                scene.prepare_document_rigids(
                    catalog,
                    document,
                    interactions,
                    models,
                    texts,
                    shape_sources,
                    &mut registry,
                    live,
                    constructor_tag,
                )?;
                scene.prepare_document_hierarchy(
                    document,
                    interactions,
                    models,
                    live,
                    &mut registry,
                    constructor_tag,
                    joint_frames,
                )?;
            }
            scene.admit_health_components(catalog, document, &mut registry)?;
            scene.prepare_placed_sound_volumes(catalog, document, models)?;
            scene.prepare_placed_breakables(catalog, document, models)?;
            scene.prepare_placed_openables(catalog, document, models, live, &mut registry)?;
            scene.prepare_placed_whale_oil_tanks(catalog,document,models,live)?;
            if let Some(prepared) = level_bodies {
                scene.admit_level_bodies(
                    document,
                    prepared,
                    live,
                    &mut registry,
                    constructor_tag,
                )?;
            }
            //The source reference lookup can resolve every constructed map
            //header before shared component Initialize. Publish this existing
            //identity index before constraints resolve their actual endpoints;
            //initialized/Spawn callbacks retain their subsequent order.
            for (index, entity) in scene.entities.iter().enumerate() {
                if scene.shared_indices.insert(entity.shared, index)
                    .is_some_and(|previous| previous != index) {
                    return Err("placed scene managed identity has multiple actual headers".into());
                }
            }
            scene.admit_placed_constraints(catalog, document, models,
                constraint_admission, live, &mut registry)?;
            // 8aa107/8aa152 finish the entity reflection/Spawn pass before the
            // later game-local shared component initialization/Start drain.
            for entity in &scene.entities {
                live.initialized(entity.shared)?;
                live.spawned(entity.shared)?;
            }
            registry.initialize_and_start(&mut interaction_components::Startup { world: None,
                scene: &mut scene,
                live,
                events: entity_events,
            })?;
            registry.drain_update_changes(&mut interaction_components::Startup { world: None,
                scene: &mut scene,
                live,
                events: entity_events,
            })?;
            if !scene.finish_constraint_admission(&mut registry, entity_events)? {
                return Err("placed constraint awaits actual endpoint body publication".into());
            }
            Ok(())
        })();
        // The registry owns the in-flight callback cursor even when startup
        // fails. Restore it before invoking the canonical release owner.
        scene.registry = Some(registry);
        let preparation = preparation.and_then(|()| scene.finish_point_queries())
            .and_then(|()| scene.capture_removal_components(catalog, document));
        if let Err(error) = preparation {
            return Err(
                match preparation::rollback(&mut scene, live, entity_events) {
                    Ok(()) => error,
                    Err(rollback) => {
                        format!("{error}; unpublished physics rollback incomplete: {rollback}")
                    }
                },
            );
        }
        Ok(scene)
    }

    /// Demand admission of actual componentless idEntity targets: objective
    /// markers and entity-qualified program variables share this base owner.
    /// Uses the same874c50/8a6180 identity and retirement roster as other
    /// admitted entity headers. Derived actors keep their real class owner;
    /// a reference to an unimplemented NPC/spawner never fabricates that actor.
    pub(super) fn admit_base_targets(
        &mut self,
        document: &entities::Document,
        requested: &std::collections::HashSet<String>,
        live: &mut crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        let tag = self.types.intern_name(Some(b"MAIN"))?;
        let class = self
            .types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .copied()
            .find(|&index| {
                self.types
                    .classes()
                    .metadata(index)
                    .is_some_and(|metadata| {
                        self.types.classes().name_text(metadata.name)
                            == Some(b"idEntity".as_slice())
                    })
            })
            .ok_or("base entity class metadata absent")?;
        let mut count = 0;
        for definition in document.definitions.iter().filter(|definition| {
            definition.kind == "entity" && requested.contains(&definition.id.to_ascii_lowercase())
        }) {
            if live.find(&definition.id).is_some() {
                continue;
            }
            let actual =
                definition
                    .fields
                    .iter()
                    .find_map(|(key, value)| match (key.as_str(), value) {
                        ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                        _ => None,
                    });
            if actual != Some("idEntity") {
                continue;
            }
            let edit =
                crate::gameplay::options::decl::scope(&definition.fields, "edit").unwrap_or(&[]);
            if crate::gameplay::options::decl::scope(edit, "m_componentDecls")
                .is_some_and(|fields| fields.iter().any(|(key, _)| key.starts_with("item_add[")||key.starts_with("item[")))
            {
                continue;
            }
            let pose = entities::spawn_frame(definition)?;
            let index = self.entities.len();
            let shared =
                live.allocate_managed(Arc::from(definition.id.as_str()), class, None, 0, tag)?;
            self.entities.push(PhysicsEntity {
                retired: false,
                external: false,
                name: definition.id.clone(),
                shared,
                components: EntityComponents::new(index),
                class,
                environment: None,
                flags: 0,
                entity_flags: 0x001e_8008,
                entity_flags_d7: 0,
                priority: 0,
                frame: crate::gameplay::reference_frame::PublishedFrame {
                    pose,
                    ..crate::gameplay::reference_frame::ReferenceFrameCache::default().published()
                },
                timespace: 0,
            });
            self.entity_flags_d4.push(0);
            live.initialized(shared)?;
            live.spawned(shared)?;
            self.shared_indices.insert(shared, index);
            count += 1;
        }
        eprintln!("BASE_ENTITY_DEMAND_LIFETIME admitted_base_entities={count}");
        Ok(())
    }

    fn water_component(&self, index: usize) -> Result<Option<ComponentHandle>, String> {
        let entity = self
            .entities
            .get(index)
            .ok_or("water query entity absent")?;
        entity
            .components
            .find_component(self.dependency_ranges[0], [0x20, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self
                            .waters
                            .get(handle.slot)
                            .ok_or("water query component absent")?
                            .owner
                            .flags,
                        start_phases: Some(
                            self.declarations
                                .get(&handle.packed())
                                .ok_or("water query declaration absent")?
                                .scheduling
                                .start_phases,
                        ),
                    },
                )
            })
    }

    /// 852ff0 / 8a7d30's Boolean projection for FX group eligibility.
    /// Any unstarted water component makes the native query return 0xff;
    /// the caller accepts only 1. Otherwise any started water contact admits
    /// the group. This Boolean result is independent of the distance sort.
    pub(super) fn water_at(&mut self, position: [f32; 3]) -> Result<bool, String> {
        let shared_world = self.world.clone();
        let world = shared_world
            .read()
            .map_err(|_| "level physics query owner is poisoned")?;
        let mut contacts = Vec::new();
        world.storage.point_query(
            &mut self.executor,
            &WorldPointRequest::water(position),
            |shape| {
                Ok(WorldQueryShape {
                    shapes: &world
                        .geometry
                        .shapes
                        .get(shape)
                        .and_then(Option::as_ref)
                        .ok_or("water query shape absent")?
                        .query_shapes,
                    materials: None,
                })
            },
            &mut contacts,
        )?;
        let mut found = false;
        for contact in contacts {
            let entity = crate::gameplay::collision_user_data::CollisionUserData::decode(
                contact.target.user_data,
            )?
            .entity(|handle| self.owner(handle).ok())?;
            if let Some(handle) = self.water_component(entity)? {
                let water = self
                    .waters
                    .get(handle.slot)
                    .ok_or("water query component absent")?;
                if water.owner.flags & 0xc < 8 {
                    return Ok(false);
                }
                found = true;
            }
        }
        Ok(found)
    }

    pub(super) fn resolve_view(
        &mut self,
        view: &mut EnvironmentViewContacts,
        position: [f32; 3],
    ) -> Result<bool, String> {
        view.query
            .submit(&mut self.queries, true, true, position)
            .map_err(|e| e.to_string())?;
        let hierarchy = self.types.classes().hierarchy();
        let mut head_water_surface: Option<f32> = None;
        let mut head_water_is_ocean = false;
        let mut head_water_current = None;
        let shared_world = self.world.clone();
        let world = shared_world
            .read()
            .map_err(|_| "level physics query owner is poisoned")?;
        let completed = view.resolve(
            &world.storage,
            &mut self.executor,
            &mut self.queries,
            |shape| {
                Ok(WorldQueryShape {
                    shapes: &world
                        .geometry
                        .shapes
                        .get(shape)
                        .and_then(Option::as_ref)
                        .ok_or("environment shape absent")?
                        .query_shapes,
                    materials: None,
                })
            },
            |handle| {
                (handle.class_id == self.classes[0])
                    .then(|| self.triggers.get(handle.slot).map(|v| &v.owner))
                    .flatten()
            },
            |index| {
                let entity = self
                    .entities
                    .get(index)
                    .ok_or("environment hit entity absent")?;
                let class = hierarchy.type_id(entity.class)?;
                let water = entity.components.find_component(
                    self.dependency_ranges[0],
                    [0x20, 0],
                    |h| {
                        Ok(
                            crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                                flags: self
                                    .waters
                                    .get(h.slot)
                                    .ok_or("environment water component absent")?
                                    .owner
                                    .flags,
                                start_phases: Some(
                                    self.declarations
                                        .get(&h.packed())
                                        .ok_or("environment water declaration absent")?
                                        .scheduling
                                        .start_phases,
                                ),
                            },
                        )
                    },
                )?;
                if let Some(component) = water.and_then(|h| self.waters.get(h.slot)) {
                    if component.owner.flags & 0xc >= 8 {
                        let declaration = component
                            .owner
                            .declaration
                            .ok_or("head water declaration absent")?;
                        head_water_is_ocean |= self
                            .water_declarations
                            .get(declaration)
                            .ok_or("head water declaration out of range")?
                            .is_ocean;
                        let height = 0.
                            - component
                                .plane()
                                .ok_or("head water component has no plane")?[3];
                        if head_water_surface.is_none_or(|prior| height > prior) {
                            head_water_surface = Some(height);
                            head_water_current = component
                                .flow_and_density()
                                .map(|value| [value[0], value[1], value[2]]);
                        }
                    }
                }
                Ok(ContactEntity {
                    trigger_derived: (self.trigger_range[0]..=self.trigger_range[1])
                        .contains(&class),
                    water_component_flags: water
                        .and_then(|h| self.waters.get(h.slot))
                        .map(|v| v.owner.flags),
                    environment: if (self.area_range[0]..=self.area_range[1]).contains(&class) {
                        entity.environment
                    } else {
                        None
                    },
                })
            },
        )?;
        view.head_water_surface = completed.then_some(head_water_surface).flatten();
        view.head_water_is_ocean = completed && head_water_is_ocean;
        view.head_water_current = if completed { head_water_current } else { None };
        Ok(completed)
    }

    fn reference_actor(
        &mut self,
        entity: usize,
        registry: &mut ComponentRegistry,
    ) -> Result<ComponentHandle, String> {
        if let Some(handle) = self.entities[entity].components.reference_actor {
            return Ok(handle);
        }
        let declaration = self.types.declaration(
            self.factories[3],
            entity,
            DeclarationScheduling::reference_frame(0),
        )?;
        let bodies = &mut self.actors.bodies;
        let free = &mut self.free_rigid_movement;
        let handle = ComponentFactory {
            declaration: &declaration,
            pool: &mut self.actors.actors,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[entity].components,
            false,
            |_, _, _| {
                // The inline default interface has the same lifetime as this
                // actor. Reuse only slots returned by acknowledged destruction.
                let body = ActorBody {
                    actor: None,
                    body_id: u32::MAX,
                };
                let embedded = if let Some(index) = free.pop() {
                    let slot = bodies
                        .get_mut(index)
                        .ok_or("free actor interface slot absent")?;
                    if slot.actor.is_some() || slot.body_id & 0x00ff_ffff != 0x00ff_ffff {
                        return Err("free actor interface is still attached".into());
                    }
                    *slot = body;
                    index
                } else {
                    let index = bodies.len();
                    bodies.push(body);
                    index
                };
                Ok(PhysicsActor::new(embedded))
            },
            |_, _, _| Err("reference actor requested immediate start before initialization".into()),
        )?
        .ok_or("reference actor creation disabled")?;
        self.declarations.insert(handle.packed(), declaration);
        self.entities[entity].components.reference_actor = Some(handle);
        Ok(handle)
    }
}

impl ComponentLifecycleWorld for PhysicsScene {
    fn priority(&self, handle: ComponentHandle) -> Result<i32, String> {
        let index = self
            .classes
            .iter()
            .position(|c| *c == handle.class_id)
            .ok_or("physics component class absent")?;
        self.types.initialization_rank(self.factories[index])
    }
    fn owner(&self, handle: ComponentHandle) -> Result<&ComponentOwner, String> {
        let owner = if handle.class_id == self.classes[0] {
            self.triggers.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[1] {
            self.waters.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[2] {
            self.winds.get(handle.slot)
        } else if handle.class_id == self.classes[3] {
            self.actors.actors.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[4] {
            self.audios.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[59] {
            self.constraints.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[58] {
            self.static_usables.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[5] {
            self.usables.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[6] {
            self.pickables.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[7] {
            self.loots.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[8] {
            self.rigid_bodies.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[9] {
            self.factions.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[10] {
            self.health.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[11] {
            self.breakables.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[12] {
            self.throwables.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[13] {
            self.projectiles.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[14] {
            self.level_bodies.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[15] {
            self.animations.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[16] {
            self.health_doors.get(handle.slot).map(|v| &v.base.owner)
        } else if handle.class_id == self.classes[17] {
            self.sound_volumes.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[19] {
            self.grenades.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[20] {
            self.ai_danger_zones.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[21] {
            self.statistics.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id == self.classes[22] {
            self.timers.get(handle.slot).map(|v| &v.owner)
        } else if handle.class_id==self.classes[48] {
            self.observables.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[49] {
            self.attention_sources.get(handle.slot).map(|v|&v.owner)
        } else if self.is_smart_object_class(handle.class_id) {
            self.smart_object(handle).ok().map(|v|&v.owner)
        } else if handle.class_id==self.classes[52] {
            self.flash_surfaces.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[53] {
            self.interactive_maps.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[54] {
            self.cooking_surfaces.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[55] {
            self.openables.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[56] {
            self.retargeters.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[57] {
            self.fx_components.get(handle.slot).map(|v|&v.owner)
        } else if handle.class_id==self.classes[60] {
            self.whale_oil_tanks.get(handle.slot).map(|v|&v.owner)
        } else if self.is_ai_startup_class(handle.class_id) {
            self.ai_startup_owner(handle)
        } else if self.is_crew_class(handle.class_id) {
            self.crew_owner(handle)
        } else if self.is_crew_collector_class(handle.class_id) {
            self.collector_owner(handle)
        } else if self.is_crew_module_class(handle.class_id) {
            self.module_owner(handle)
        } else if handle.class_id == self.classes[18] {
            self.damage_zones.get(handle.slot).map(|v| &v.owner)
        } else {
            None
        };
        owner.ok_or_else(|| "physics component owner absent".into())
    }
    fn owner_mut(&mut self, handle: ComponentHandle) -> Result<&mut ComponentOwner, String> {
        let owner = if handle.class_id == self.classes[0] {
            self.triggers.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[1] {
            self.waters.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[2] {
            self.winds.get_mut(handle.slot)
        } else if handle.class_id == self.classes[3] {
            self.actors
                .actors
                .get_mut(handle.slot)
                .map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[4] {
            self.audios.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[59] {
            self.constraints.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[58] {
            self.static_usables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[5] {
            self.usables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[6] {
            self.pickables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[7] {
            self.loots.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[8] {
            self.rigid_bodies.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[9] {
            self.factions.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[10] {
            self.health.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[11] {
            self.breakables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[12] {
            self.throwables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[13] {
            self.projectiles.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[14] {
            self.level_bodies.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[15] {
            self.animations.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[16] {
            self.health_doors.get_mut(handle.slot).map(|v| &mut v.base.owner)
        } else if handle.class_id == self.classes[17] {
            self.sound_volumes.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[19] {
            self.grenades.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[20] {
            self.ai_danger_zones.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[21] {
            self.statistics.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id == self.classes[22] {
            self.timers.get_mut(handle.slot).map(|v| &mut v.owner)
        } else if handle.class_id==self.classes[48] {
            self.observables.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[49] {
            self.attention_sources.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if self.is_smart_object_class(handle.class_id) {
            self.smart_object_mut(handle).ok().map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[52] {
            self.flash_surfaces.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[53] {
            self.interactive_maps.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[54] {
            self.cooking_surfaces.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[55] {
            self.openables.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[56] {
            self.retargeters.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[57] {
            self.fx_components.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if handle.class_id==self.classes[60] {
            self.whale_oil_tanks.get_mut(handle.slot).map(|v|&mut v.owner)
        } else if self.is_ai_startup_class(handle.class_id) {
            self.ai_startup_owner_mut(handle)
        } else if self.is_crew_class(handle.class_id) {
            self.crew_owner_mut(handle)
        } else if self.is_crew_collector_class(handle.class_id) {
            self.collector_owner_mut(handle)
        } else if self.is_crew_module_class(handle.class_id) {
            self.module_owner_mut(handle)
        } else if handle.class_id == self.classes[18] {
            self.damage_zones.get_mut(handle.slot).map(|v| &mut v.owner)
        } else {
            None
        };
        owner.ok_or_else(|| "physics component owner absent".into())
    }
    fn initialize(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let entity = self
            .owner(handle)?
            .entity
            .ok_or("physics component entity absent")?;
        if handle.class_id == self.classes[0] {
            let placement = self
                .owner(handle)?
                .declaration
                .ok_or("trigger declaration absent")?;
            let (position, collision) = {
                let world = self
                    .world
                    .read()
                    .map_err(|_| "level physics query owner is poisoned")?;
                let placement = world.geometry.placements[placement]
                    .as_ref()
                    .ok_or("trigger placement retired")?;
                (placement.position, placement.binding.collision)
            };
            let entity = &mut self.entities[entity];
            let mut level = startup::TriggerLevel {
                entity: &entity.components,
                index: placement,
                position,
                collision: &collision,
                material: self.trigger_material,
                ranges: self.dependency_ranges,
                declarations: &self.declarations,
                wind_class: self.classes[2],
                winds: &self.winds,
                flags: &mut entity.flags,
            };
            scene_publication_events!(self, registry, events);
            let mut query_owner = self
                .world
                .write()
                .map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            crate::gameplay::trigger_body::initialization::physics::TriggerPhysicsInitialization {
                world: &mut world.storage,
                geometry: &mut world.geometry,
                events: &mut events,
                level: &mut level,
            }
            .initialize(handle)
        } else if handle.class_id == self.classes[6] {
            let target = &mut self.entities[entity];
            self.pickables
                .get_mut(handle.slot)
                .ok_or("Pickable component absent")?
                .initialize(&mut target.entity_flags, &mut target.entity_flags_d7);
            Ok(())
        } else if handle.class_id == self.classes[3] {
            let pose = self.entities[entity].frame.pose;
            let actor = self
                .actors
                .actors
                .get_mut(handle.slot)
                .ok_or("reference actor absent")?;
            actor.set_transform(pose, true);
            for requests in &mut actor.requests {
                requests.enabled = true;
            }
            actor.owner.mark_initialized();
            Ok(())
        } else if handle.class_id == self.classes[4] {
            // 140708960 initializes the default sound slot and common owner.
            self.owner_mut(handle)?.mark_initialized();
            Ok(())
        } else {
            // Water588300 and the common wind owner initialization transition.
            self.owner_mut(handle)?.mark_initialized();
            Ok(())
        }
    }
    fn start(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let entity_index = self
            .owner(handle)?
            .entity
            .ok_or("physics component entity absent")?;
        let scheduling = self
            .declarations
            .get(&handle.packed())
            .ok_or("physics component scheduling absent")?
            .scheduling
            .clone();
        if handle.class_id == self.classes[19] {
            //4911b0 sets the actual initial pause request before7e7670.
            let component=self.grenades.get_mut(handle.slot).ok_or("Grenade Start payload absent")?;
            component.owner.request_update(u32::from(!component.armed()));
        }
        if self.is_health_class(handle.class_id) {
            self.entities[entity_index].entity_flags_d7 |= 2;
            let dead = self.entities[entity_index].entity_flags & 0x800 != 0;
            let component = self.health_payload_mut(handle)?;
            // 7181c0 preserves a previously dead owner's zero health when
            // the real component restarts; Start is not a resurrection.
            if component.owner.flags & (1 << 30) == 0 {
                component.current = if dead { 0.0 } else { component.maximum };
            }
        }
        if handle.class_id == self.classes[0] {
            let declaration = self
                .owner(handle)?
                .declaration
                .ok_or("trigger declaration absent")?;
            let keyframed = self
                .world
                .read()
                .map_err(|_| "level physics query owner is poisoned")?
                .geometry
                .placements[declaration]
                .as_ref()
                .ok_or("trigger placement retired")?
                .binding
                .collision
                .keyframed;
            let actor = if keyframed {
                Some(self.reference_actor(entity_index, registry)?)
            } else {
                None
            };
            scene_publication_events!(self, registry, events);
            let mut query_owner = self
                .world
                .write()
                .map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            crate::gameplay::trigger_body::startup::start_trigger(
                &mut world.storage,
                &mut world.geometry,
                &mut events,
                handle,
                &scheduling,
                self.entities[entity_index].priority,
                keyframed,
                |components, reference| {
                    let actor = actor.ok_or("keyframed trigger reference actor absent")?;
                    let body = components.trigger(reference)?.movement_body;
                    components.actors.attach_body(actor.slot as usize,body,0,|_,_|Err("reference actor queued motion before level initialization completed".into()))
                },
            )
        } else if handle.class_id == self.classes[1] {
            let binding = self
                .owner(handle)?
                .declaration
                .ok_or("water declaration absent")?;
            // All material declarations were resolved against this same owner
            // before lifecycle publication; no graphics material is substituted.
            let trigger_range = self.dependency_ranges_for_trigger()?;
            let declaration = &self.water_declarations[binding];
            let mut notify_contacts = false;
            let result = {
            let mut level = startup::WaterLevel {
                entity: &self.entities[entity_index].components,
                trigger_range,
                declarations: &self.declarations,
                scheduling: &scheduling,
                priority: self.entities[entity_index].priority,
                notify_contacts: &mut notify_contacts,
            };
            scene_publication_events!(self, registry, events);
            let mut query_owner = self
                .world
                .write()
                .map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            crate::gameplay::water_component::startup::physics::WaterPhysicsStartup {
                world: &mut world.storage,
                geometry: &mut world.geometry,
                events: &mut events,
                level: &mut level,
            }
            .start(handle, declaration)
            };
            result?;
            if notify_contacts { self.notify_water_contacts(handle, false)?; }
            Ok(())
        } else if handle.class_id == self.classes[4] {
            let audio = self
                .audios
                .get(handle.slot)
                .ok_or("audio component absent")?;
            let startup = if let Some(binding) = &audio.runtime_definition {
                binding.startup()
            } else {
                let declaration = audio.owner.declaration.ok_or("audio declaration absent")?;
                self.audio_bindings
                    .get(declaration)
                    .ok_or("audio startup binding absent")?
                    .startup()
            };
            let priority = self.entities[entity_index].priority;
            let audio = self
                .audios
                .get_mut(handle.slot)
                .ok_or("audio component absent")?;
            // 140708fc0 clears the special update counter before base admission.
            if audio.owner.flags & (1 << 30) != 0 {
                audio.owner.flags &= 0xf800003f;
            }
            let scheduled = crate::gameplay::component_state::prepare_admission(
                &mut audio.owner.flags,
                u8::from(scheduling.priority < 8),
                scheduling.priority,
                priority,
            );
            audio.startup = startup;
            audio.started = true;
            if scheduled {
                self.audios.schedule(handle.slot)?;
            }
            Ok(())
        } else {
            let priority = self.entities[entity_index].priority;
            let scheduled = crate::gameplay::component_state::prepare_admission(
                &mut self.owner_mut(handle)?.flags,
                u8::from(scheduling.priority < 8),
                scheduling.priority,
                priority,
            );
            if scheduled {
                if handle.class_id == self.classes[2] {
                    self.winds.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[3] {
                    self.actors.actors.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[59] {
                    self.constraints.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[58] {
                    self.static_usables.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[5] {
                    self.usables.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[6] {
                    self.pickables.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[7] {
                    self.loots.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[8] {
                    self.rigid_bodies.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[9] {
                    self.factions.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[10] {
                    self.health.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[11] {
                    self.breakables.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[13] {
                    self.projectiles.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[14] {
                    self.level_bodies.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[15] {
                    self.animations.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[16] {
                    self.health_doors.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[17] {
                    self.sound_volumes.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[19] {
                    self.grenades.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[20] {
                    self.ai_danger_zones.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[21] {
                    self.statistics.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[22] {
                    self.timers.schedule(handle.slot)?;
                } else if self.is_ai_startup_class(handle.class_id) {
                    self.schedule_ai_startup(handle)?;
                } else if self.is_crew_class(handle.class_id) {
                    self.schedule_crew(handle)?;
                } else if self.is_crew_collector_class(handle.class_id) {
                    self.schedule_crew_collector(handle)?;
                } else if self.is_crew_module_class(handle.class_id) {
                    self.schedule_crew_module(handle)?;
                } else if handle.class_id==self.classes[48] {
                    self.observables.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[49] {
                    self.attention_sources.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[50] {
                    self.investigate_spots.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[51] {
                    self.dynamic_search_spots.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[52] {
                    self.flash_surfaces.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[53] {
                    self.interactive_maps.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[54] {
                    self.cooking_surfaces.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[55] {
                    self.openables.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[56] {
                    self.retargeters.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[57] {
                    self.fx_components.schedule(handle.slot)?;
                } else if handle.class_id==self.classes[60] {
                    self.whale_oil_tanks.schedule(handle.slot)?;
                } else if handle.class_id == self.classes[18] {
                    self.damage_zones.schedule(handle.slot)?;
                } else {
                    return Err("scheduled physics component class is unsupported".into());
                }
            }
            Ok(())
        }
    }
    fn stop_queued(&mut self, _: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        // These level-load trigger/water callbacks are the audited0998b0 no-op.
        Ok(())
    }
    fn initialization_failed(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if self.release_missing_shape_component(handle, registry)? {
            return Ok(());
        }
        Err(format!(
            "level physics component initialization failed: {handle:?}"
        ))
    }
}

impl PhysicsScene {
    fn dependency_ranges_for_trigger(&self) -> Result<[i32; 2], String> {
        self.types
            .classes()
            .hierarchy()
            .type_range(self.types.factory(self.factories[0])?.class)
    }
}
