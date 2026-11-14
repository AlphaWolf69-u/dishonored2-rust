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
        world_point_query::{level::LevelPointQueries, WorldPointQuery, WorldPointRequest, WorldQueryShape},
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
mod startup;
mod interaction_components;
use interaction_components::UsableComponent;
pub(super) mod lifetime;

struct PhysicsEntity {
    name: String,
    shared: crate::gameplay::entity_lifetime::EntityId,
    components: EntityComponents,
    class: usize,
    environment: Option<(usize, bool)>,
    flags: u8,
    priority: i32,
    pose: crate::gameplay::reference_frame::ReferenceFrame,
}

#[derive(Resource)]
pub(super) struct PhysicsScene {
    world: Arc<RwLock<LevelPointQueries>>,
    types: Arc<ComponentTypes>,
    factories: [u32; 6],
    classes: [i32; 6],
    entities: Vec<PhysicsEntity>,
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
    audio_bindings: Vec<entities::audio::AudioBinding>,
    audio_handles: Vec<ComponentHandle>,
    actors: PhysicsActorLinks,
    registry: Option<ComponentRegistry>,
    deferred: DeferredPhysicsState,
    signals: PublicationSignals<usize, usize>,
    contacts: crate::gameplay::water_contacts::WaterContactManager,
    trigger_material: u16,
    trigger_range: [i32; 2],
    area_range: [i32; 2],
    dependency_ranges: [[i32; 2]; 2],
    queries: CollisionQueries<WorldPointRequest, ShapePointContact>,
    executor: WorldPointQuery,
    retirement: Option<lifetime::Retirement>,
}

impl PhysicsScene {
    pub(super) fn entity_frame(&self, live: &crate::gameplay::entity_lifetime::LiveEntities,
        id: crate::gameplay::entity_lifetime::EntityId)
        -> Result<Option<crate::gameplay::reference_frame::ReferenceFrame>, String> {
        let Some((name, class)) = live.get(id) else { return Err("entity frame identity is stale".into()); };
        let Some(&index) = self.shared_indices.get(&id) else { return Ok(None); };
        let entity = self.entities.get(index).ok_or("entity frame index is stale")?;
        if entity.class != class || !entity.name.eq_ignore_ascii_case(name) { return Err("entity frame owner differs".into()); }
        if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) { return Ok(None); }
        if let Some(handle) = entity.components.reference_actor {
            let actor = self.actors.actors.get(handle.slot).ok_or("entity reference actor absent")?;
            // The reference actor owns the current physics transform. No render
            // placement frame is substituted for an absent actor.
            return Ok(Some(actor.frame.current));
        }
        Ok(Some(entity.pose))
    }

    pub(super) fn audio_component_live(&self, packed: u64, generation: u32) -> bool {
        let Ok(handle) = ComponentHandle::from_packed(packed) else { return false };
        handle.class_id == self.classes[4] && self.audios.get(handle.slot)
            .is_some_and(|audio| audio.started && audio.owner.entity.is_some() && audio.owner.generation == generation)
    }

    pub(super) fn visit_audio(&mut self, mut visit: impl FnMut(ComponentHandle,
        &entities::audio::AudioBinding, &mut crate::gameplay::sound::component::AudioComponent) -> Result<(), String>) -> Result<(), String>
    {
        for (&handle, binding) in self.audio_handles.iter().zip(&self.audio_bindings) {
            let audio = self.audios.get_mut(handle.slot).ok_or("level audio component absent")?;
            visit(handle, binding, audio)?;
        }
        Ok(())
    }

    pub(super) fn point_queries(&self) -> Result<Arc<RwLock<LevelPointQueries>>, &str> {
        self.world.read().map_err(|_| "level physics query owner is poisoned")?.ready()?;
        Ok(self.world.clone())
    }

    fn finish_point_queries(&mut self) -> Result<(), String> {
        let shared_world = self.world.clone();
        let world = shared_world.read().map_err(|_| "level physics query owner is poisoned")?;
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
        let mut query_owner = self.world.write().map_err(|_| "level physics query owner is poisoned")?;
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
        self.world.read().map_err(|_| "level physics query owner is poisoned")?.particle_water_body_height(&mut self.executor, position)
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
    ) -> Result<Self, String> {
        let types = ComponentTypes::acquire(catalog)?;
        let constructor_tag = types.intern_name(Some(b"MAIN"))?;
        let factory = |name: &[u8]| {
            types
                .factories()
                .iter()
                .find(|p| types.classes().metadata(p.class)
                    .and_then(|class| types.classes().name_text(class.name)) == Some(name))
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
        ];
        let names = types.classes().hierarchy().registration_order().iter().map(|&index| {
            let class = types.classes().metadata(index).ok_or("physics class metadata absent")?;
            let name = types.classes().name_text(class.name).ok_or("physics class name absent")?;
            Ok((name.to_vec(), index))
        }).collect::<Result<HashMap<_, _>, String>>()?;
        let hierarchy = types.classes().hierarchy();
        let entity_range = |name: &[u8]| {
            hierarchy.type_range(*names.get(name).ok_or("physics entity class absent")?)
        };
        let trigger_range = entity_range(b"arkTrigger")?;
        let area_range = entity_range(b"arkAreaVolume")?;
        let mut classes = [0; 6];
        for i in 0..6 {
            classes[i] = hierarchy.type_id(types.factory(factories[i])?.class)?;
        }
        let dependency_ranges = [
            hierarchy.type_range(types.factory(factories[1])?.class)?,
            hierarchy.type_range(types.factory(factories[2])?.class)?,
        ];
        let trace_shape_cpu = std::env::var("DH2_DEBUG_TRIGGER_SOURCE_CPU").as_deref() == Ok("1");
        let shape_cpu_start = if trace_shape_cpu { crate::loading::thread_cycles() } else { None };
        let geometry = TriggerAssets::load_with_sources(document, map, catalog, shape_sources)?;
        if let (Some(start), Some(end)) = (shape_cpu_start, if trace_shape_cpu { crate::loading::thread_cycles() } else { None }) {
            eprintln!("MEMORY trigger acquisition CPU: shapes={} cycles={}", geometry.shapes.len(), end.saturating_sub(start));
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
        let declaration = materials.declaration_with_texts(catalog, "narrow_phase_trigger", texts)?;
        let trigger_material = materials.resolve(declaration, |_, _, _, _| {
            panic!("material subscriber before level initialization")
        })?;
        let water_declarations = water_bindings
            .iter()
            .map(|binding| binding.startup(|name| materials.declaration_with_texts(catalog, name, texts)))
            .collect::<Result<Vec<_>, String>>()?;
        let bounds = entities::physics::world_physics_bounds(document)?
            .ok_or("physics worldspawn bounds absent")?;
        static NEXT_WORLD: AtomicUsize = AtomicUsize::new(1);
        let world = WorldStorage::new(
            WorldInfo::game_settings(bounds),
            NEXT_WORLD.fetch_add(1, Ordering::Relaxed),
            materials,
        )?;
        let cvars = crate::platform::console::shared().read();
        let mut scene = Self {
            world: Arc::new(RwLock::new(LevelPointQueries::new(world, geometry))),
            triggers: types.create_pool(factories[0], &cvars.cvars)?,
            waters: types.create_pool(factories[1], &cvars.cvars)?,
            winds: types.create_pool(factories[2], &cvars.cvars)?,
            audios: types.create_pool(factories[4], &cvars.cvars)?,
            usables: types.create_pool(factories[5], &cvars.cvars)?,
            audio_bindings,
            audio_handles: Vec::new(),
            actors: PhysicsActorLinks {
                actors: types.create_pool(factories[3], &cvars.cvars)?,
                bodies: Vec::new(),
            },
            types,
            factories,
            classes,
            entities: Vec::new(),
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
            trigger_material,
            trigger_range,
            area_range,
            dependency_ranges,
            queries: CollisionQueries::default(),
            executor: WorldPointQuery::new(u32::MAX)?,
            retirement: None,
        };
        drop(cvars);
        scene.queries.set_submission_allowed(true);
        let mut registry = scene
            .registry
            .take()
            .ok_or("physics component registry absent")?;
        let mut entity_indices = HashMap::<String, usize>::new();
        let query_owner = scene.world.clone();
        let query_world = query_owner.read().map_err(|_| "level physics query owner is poisoned")?;
        for index in 0..query_world.geometry.placements.len() {
            let placement = &query_world.geometry.placements[index];
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
                let shared = live.allocate_managed(Arc::from(placement.binding.entity_id.as_str()), class, None, 0, constructor_tag)?;
                let mut entity = PhysicsEntity {
                    name: placement.binding.entity_id.clone(),
                    shared,
                    components: EntityComponents::new(entity_index),
                    class,
                    environment: volume_indices
                        .get(placement.binding.entity_id.as_str())
                        .copied(),
                    flags: 0,
                    priority: 0,
                    pose: crate::gameplay::reference_frame::ReferenceFrame {
                        position: placement.position,
                        basis: placement.shape.basis,
                    },
                };
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
                scene.entities.push(entity);
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
                    let definition = definitions.get(binding.entity_id.as_str()).ok_or("audio entity definition absent")?;
                    let class = definition.fields.iter().find_map(|(key, value)| {
                        if key != "class" { return None; }
                        if let entities::Value::Atom(value) = value { Some(value.trim_matches('"')) } else { None }
                    }).ok_or("audio entity class name absent")?;
                    let class = *names.get(class.as_bytes()).ok_or_else(|| format!("audio entity class unsupported: {class}"))?;
                    let index = scene.entities.len();
                    let shared = live.allocate_managed(Arc::from(binding.entity_id.as_str()), class, None, 0, constructor_tag)?;
                    scene.entities.push(PhysicsEntity { name: binding.entity_id.clone(), shared, components: EntityComponents::new(index), class,
                        environment: volume_indices.get(binding.entity_id.as_str()).copied(), flags: 0, priority: 0, pose: binding.frame });
                    entity_indices.insert(binding.entity_id.clone(), index);
                    index
                }
            };
            let declaration = scene.types.declaration(factories[4], index, binding.scheduling.clone())?;
            let handle = ComponentFactory { declaration: &declaration, pool: &mut scene.audios,
                registry: &mut registry, grow_one_at_a_time: false }.create(
                    &mut scene.entities[entity_index].components, declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(crate::gameplay::sound::component::AudioComponent::default()), |_, _, _| Ok(()),
                )?.ok_or("audio component creation disabled during level load")?;
            scene.declarations.insert(handle.packed(), declaration);
            scene.audio_handles.push(handle);
        }
        drop(query_world);
        if let Some(interactions) = interactions {
            scene.admit_usable_components(document, interactions, live, &mut registry, constructor_tag)?;
        }
        for entity in &scene.entities { live.initialized(entity.shared)?; }
        registry.initialize_and_start(&mut scene)?;
        for (index, entity) in scene.entities.iter().enumerate() {
            live.spawned(entity.shared)?;
            scene.shared_indices.insert(entity.shared, index);
        }
        scene.registry = Some(registry);
        scene.finish_point_queries()?;
        Ok(scene)
    }

    fn water_component(&self, index: usize) -> Result<Option<ComponentHandle>, String> {
        let entity = self.entities.get(index).ok_or("water query entity absent")?;
        entity.components.find_component(self.dependency_ranges[0], [0x20, 0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                flags: self.waters.get(handle.slot).ok_or("water query component absent")?.owner.flags,
                start_phases: Some(self.declarations.get(&handle.packed())
                    .ok_or("water query declaration absent")?.scheduling.start_phases),
            })
        })
    }

    /// 852ff0 / 8a7d30's Boolean projection for FX group eligibility.
    /// Any unstarted water component makes the native query return 0xff;
    /// the caller accepts only 1. Otherwise any started water contact admits
    /// the group. This Boolean result is independent of the distance sort.
    pub(super) fn water_at(&mut self, position: [f32; 3]) -> Result<bool, String> {
        let shared_world = self.world.clone();
        let world = shared_world.read().map_err(|_| "level physics query owner is poisoned")?;
        let mut contacts = Vec::new();
        world.storage.point_query(&mut self.executor, &WorldPointRequest::water(position), |shape| {
            Ok(WorldQueryShape {
                shapes: &world.geometry.shapes.get(shape).ok_or("water query shape absent")?.query_shapes,
                materials: None,
            })
        }, &mut contacts)?;
        let mut found = false;
        for contact in contacts {
            let entity = crate::gameplay::collision_user_data::CollisionUserData::decode(contact.target.user_data)?
                .entity(|handle| self.owner(handle).ok())?;
            if let Some(handle) = self.water_component(entity)? {
                let water = self.waters.get(handle.slot).ok_or("water query component absent")?;
                if water.owner.flags & 0xc < 8 { return Ok(false); }
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
        let world = shared_world.read().map_err(|_| "level physics query owner is poisoned")?;
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
                if let Some(component)=water.and_then(|h|self.waters.get(h.slot)) {
                    if component.owner.flags & 0xc >= 8 {
                        let declaration=component.owner.declaration.ok_or("head water declaration absent")?;
                        head_water_is_ocean |= self.water_declarations.get(declaration).ok_or("head water declaration out of range")?.is_ocean;
                        let height=0.-component.plane().ok_or("head water component has no plane")?[3];
                        if head_water_surface.is_none_or(|prior| height > prior) {
                            head_water_surface = Some(height);
                            head_water_current = component.flow_and_density().map(|value| [value[0], value[1], value[2]]);
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
        view.head_water_surface=completed.then_some(head_water_surface).flatten();
        view.head_water_is_ocean=completed && head_water_is_ocean;
        view.head_water_current=if completed { head_water_current } else { None };
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
                let embedded = bodies.len();
                bodies.push(ActorBody {
                    actor: None,
                    body_id: u32::MAX,
                });
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
        } else if handle.class_id == self.classes[5] {
            self.usables.get(handle.slot).map(|v| &v.owner)
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
        } else if handle.class_id == self.classes[5] {
            self.usables.get_mut(handle.slot).map(|v| &mut v.owner)
        } else {
            None
        };
        owner.ok_or_else(|| "physics component owner absent".into())
    }
    fn initialize(
        &mut self,
        handle: ComponentHandle,
        _: &mut ComponentRegistry,
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
                let world = self.world.read().map_err(|_| "level physics query owner is poisoned")?;
                (world.geometry.placements[placement].position, world.geometry.placements[placement].binding.collision)
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
            let mut events = ComponentPublicationEvents {
                components: TriggerComponentStore {
                    trigger_class: self.classes[0],
                    water_class: self.classes[1],
                    triggers: &mut self.triggers,
                    waters: &mut self.waters,
                    actors: &mut self.actors,
                    deferred: &mut self.deferred,
                },
                signals: &mut self.signals,
                dispatch: startup::dispatch_observer,
            };
            let mut query_owner = self.world.write().map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            crate::gameplay::trigger_body::initialization::physics::TriggerPhysicsInitialization {
                world: &mut world.storage,
                geometry: &mut world.geometry,
                events: &mut events,
                level: &mut level,
            }
            .initialize(handle)
        } else if handle.class_id == self.classes[3] {
            let pose = self.entities[entity].pose;
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
        if handle.class_id == self.classes[0] {
            let declaration = self
                .owner(handle)?
                .declaration
                .ok_or("trigger declaration absent")?;
            let keyframed = self.world.read().map_err(|_| "level physics query owner is poisoned")?
                .geometry.placements[declaration].binding.collision.keyframed;
            let actor = if keyframed {
                Some(self.reference_actor(entity_index, registry)?)
            } else {
                None
            };
            let mut events = ComponentPublicationEvents {
                components: TriggerComponentStore {
                    trigger_class: self.classes[0],
                    water_class: self.classes[1],
                    triggers: &mut self.triggers,
                    waters: &mut self.waters,
                    actors: &mut self.actors,
                    deferred: &mut self.deferred,
                },
                signals: &mut self.signals,
                dispatch: startup::dispatch_observer,
            };
            let mut query_owner = self.world.write().map_err(|_| "level physics query owner is poisoned")?;
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
            let mut level = startup::WaterLevel {
                entity: &self.entities[entity_index].components,
                trigger_range,
                declarations: &self.declarations,
                scheduling: &scheduling,
                priority: self.entities[entity_index].priority,
                contacts: &mut self.contacts,
            };
            let mut events = ComponentPublicationEvents {
                components: TriggerComponentStore {
                    trigger_class: self.classes[0],
                    water_class: self.classes[1],
                    triggers: &mut self.triggers,
                    waters: &mut self.waters,
                    actors: &mut self.actors,
                    deferred: &mut self.deferred,
                },
                signals: &mut self.signals,
                dispatch: startup::dispatch_observer,
            };
            let mut query_owner = self.world.write().map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            crate::gameplay::water_component::startup::physics::WaterPhysicsStartup {
                world: &mut world.storage,
                geometry: &mut world.geometry,
                events: &mut events,
                level: &mut level,
            }
            .start(handle, declaration)
        } else if handle.class_id == self.classes[4] {
            let declaration = self.owner(handle)?.declaration.ok_or("audio declaration absent")?;
            let startup = self.audio_bindings.get(declaration).ok_or("audio startup binding absent")?.startup();
            let priority = self.entities[entity_index].priority;
            let audio = self.audios.get_mut(handle.slot).ok_or("audio component absent")?;
            // 140708fc0 clears the special update counter before base admission.
            if audio.owner.flags & (1 << 30) != 0 { audio.owner.flags &= 0xf800003f; }
            let scheduled = crate::gameplay::component_state::prepare_admission(&mut audio.owner.flags,
                u8::from(scheduling.priority < 8), scheduling.priority, priority);
            audio.startup = startup;
            audio.started = true;
            if scheduled { self.audios.schedule(handle.slot)?; }
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
                if handle.class_id == self.classes[2] { self.winds.schedule(handle.slot)?; }
                else if handle.class_id == self.classes[3] { self.actors.actors.schedule(handle.slot)?; }
                else if handle.class_id == self.classes[5] { self.usables.schedule(handle.slot)?; }
                else { return Err("scheduled physics component class is unsupported".into()); }
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
        _: &mut ComponentRegistry,
    ) -> Result<(), String> {
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
