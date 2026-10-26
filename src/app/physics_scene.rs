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
        world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
    },
    rendering::environment_contacts::{ContactEntity, EnvironmentViewContacts},
};
use bevy::prelude::{default, Resource};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
mod startup;

struct PhysicsEntity {
    components: EntityComponents,
    class: usize,
    environment: Option<(usize, bool)>,
    flags: u8,
    priority: i32,
    pose: crate::gameplay::reference_frame::ReferenceFrame,
}

#[derive(Resource)]
pub(super) struct PhysicsScene {
    world: WorldStorage,
    geometry: TriggerAssets,
    types: ComponentTypes,
    factories: [u32; 5],
    classes: [i32; 5],
    entities: Vec<PhysicsEntity>,
    declarations: HashMap<u64, ComponentDeclaration>,
    water_bindings: Vec<entities::water::WaterBinding>,
    water_declarations: Vec<crate::gameplay::water_component::startup::WaterDeclaration>,
    triggers: ComponentPool<TriggerComponent>,
    waters: ComponentPool<WaterComponent>,
    /// Wind payloads already live in the existing wind owner; these are their
    /// common component reference/lifecycle headers, not another wind field.
    winds: ComponentPool<ComponentOwner>,
    audios: ComponentPool<crate::gameplay::sound::component::AudioComponent>,
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
}

impl PhysicsScene {
    pub(super) fn visit_audio(&mut self, mut visit: impl FnMut(ComponentHandle,
        &entities::audio::AudioBinding, &mut crate::gameplay::sound::component::AudioComponent) -> Result<(), String>) -> Result<(), String>
    {
        for (&handle, binding) in self.audio_handles.iter().zip(&self.audio_bindings) {
            let audio = self.audios.get_mut(handle.slot).ok_or("level audio component absent")?;
            visit(handle, binding, audio)?;
        }
        Ok(())
    }

    pub(super) fn query_snapshot(
        &self,
    ) -> Result<Arc<crate::gameplay::world_point_query::snapshot::PointQuerySnapshot>, String> {
        crate::gameplay::world_point_query::snapshot::PointQuerySnapshot::capture(
            &self.world.bodies,
            &self.world.broadphase.query_trees(),
            crate::gameplay::point_query::PointQuantization {
                offset: self.world.quantization.offset[..3].try_into().unwrap(),
                scale: self.world.quantization.inverse_scale[..3].try_into().unwrap(),
            },
            crate::gameplay::ray_query::RayQuantization {
                offset: self.world.quantization.offset[..3].try_into().unwrap(),
                scale: self.world.quantization.scale[..3].try_into().unwrap(),
            },
            self.geometry.shapes.iter().map(|shape| shape.query_shapes.clone()).collect(),
            |body| self.world.quantization.packed.unpack_bounds(body.bounds)[1][2],
        ).map(Arc::new)
    }

    /// The point-query half of1fc710. This returns the selected body's packed
    /// AABB top (c85490), independently of the view's water component plane.
    /// The caller still owns the downward-ray fallback when this returns None.
    pub(super) fn particle_water_body_height(
        &mut self,
        position: [f32; 3],
    ) -> Result<Option<f32>, String> {
        let hit = self.executor.closest_hit(
            &self.world.bodies,
            &self.world.broadphase.query_trees(),
            crate::gameplay::point_query::PointQuantization {
                offset: self.world.quantization.offset[..3].try_into().unwrap(),
                scale: self.world.quantization.inverse_scale[..3].try_into().unwrap(),
            },
            &WorldPointRequest::water(position),
            |shape| Ok(WorldQueryShape {
                shapes: &self.geometry.shapes.get(shape)
                    .ok_or("particle water-query shape absent")?.query_shapes,
                materials: None,
            }),
        )?;
        let Some((contact, _)) = hit else { return Ok(None) };
        let body = self.world.bodies.get(contact.target.body_id
            .ok_or("particle water-query hit has no body")?)
            .ok_or("particle water-query body absent")?;
        Ok(Some(self.world.quantization.packed.unpack_bounds(body.bounds)[1][2]))
    }

    pub(super) fn load(
        catalog: &archive::Catalog,
        document: &entities::Document,
        map: &str,
        volumes: &[crate::rendering::environment::EnvironmentVolume],
    ) -> Result<Self, String> {
        let base = crate::assets::installation::game_base().map_err(|e| e.to_string())?;
        let installed = crate::assets::class_metadata::ClassDeclarations::load(&base)?;
        let factory = |name: &[u8]| {
            installed
                .component_pools
                .iter()
                .find(|p| installed.classes[p.class].name == name)
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
        ];
        let names = installed
            .classes
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.clone(), i))
            .collect::<HashMap<_, _>>();
        let types = ComponentTypes::new(installed)?;
        let hierarchy = types.classes().hierarchy();
        let entity_range = |name: &[u8]| {
            hierarchy.type_range(*names.get(name).ok_or("physics entity class absent")?)
        };
        let trigger_range = entity_range(b"arkTrigger")?;
        let area_range = entity_range(b"arkAreaVolume")?;
        let mut classes = [0; 5];
        for i in 0..5 {
            classes[i] = hierarchy.type_id(types.factory(factories[i])?.class)?;
        }
        let dependency_ranges = [
            hierarchy.type_range(types.factory(factories[1])?.class)?,
            hierarchy.type_range(types.factory(factories[2])?.class)?,
        ];
        let geometry = TriggerAssets::load(document, map, catalog)?;
        let read = |name: &str| {
            catalog
                .read(name)
                .map(|v| v.map(|b| entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let water_bindings = entities::water::water_bindings(document, read)?;
        let audio_bindings = entities::audio::audio_bindings(document, read)?;
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
            .map(|(i, v)| (v.placement.id.as_str(), (i, v.is_water_volume)))
            .collect::<HashMap<_, _>>();
        let constants = Arc::new(crate::assets::physics_material::MaterialConstants::load(
            &base,
        )?);
        let mut materials = crate::gameplay::materials::MaterialOwner::new(constants, 16)?;
        let declaration = materials.declaration(catalog, "narrow_phase_trigger")?;
        let trigger_material = materials.resolve(declaration, |_, _, _, _| {
            panic!("material subscriber before level initialization")
        })?;
        let water_declarations = water_bindings
            .iter()
            .map(|binding| binding.startup(|name| materials.declaration(catalog, name)))
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
            world,
            geometry,
            triggers: types.create_pool(factories[0], &cvars.cvars)?,
            waters: types.create_pool(factories[1], &cvars.cvars)?,
            winds: types.create_pool(factories[2], &cvars.cvars)?,
            audios: types.create_pool(factories[4], &cvars.cvars)?,
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
        };
        drop(cvars);
        scene.queries.set_submission_allowed(true);
        let mut registry = scene
            .registry
            .take()
            .ok_or("physics component registry absent")?;
        let mut entity_indices = HashMap::<String, usize>::new();
        for index in 0..scene.geometry.placements.len() {
            let placement = &scene.geometry.placements[index];
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
                let mut entity = PhysicsEntity {
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
                    scene.entities.push(PhysicsEntity { components: EntityComponents::new(index), class,
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
        registry.initialize_and_start(&mut scene)?;
        scene.registry = Some(registry);
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
        let mut contacts = Vec::new();
        self.world.point_query(&mut self.executor, &WorldPointRequest::water(position), |shape| {
            Ok(WorldQueryShape {
                shapes: &self.geometry.shapes.get(shape).ok_or("water query shape absent")?.query_shapes,
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
        let completed = view.resolve(
            &self.world,
            &mut self.executor,
            &mut self.queries,
            |shape| {
                Ok(WorldQueryShape {
                    shapes: &self
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
                        head_water_surface=Some(head_water_surface.map_or(height,|prior|if prior<=height {height} else {prior}));
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
            let position = self.geometry.placements[placement].position;
            let collision = self.geometry.placements[placement].binding.collision;
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
            crate::gameplay::trigger_body::initialization::physics::TriggerPhysicsInitialization {
                world: &mut self.world,
                geometry: &mut self.geometry,
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
            let keyframed = self.geometry.placements[declaration]
                .binding
                .collision
                .keyframed;
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
            crate::gameplay::trigger_body::startup::start_trigger(
                &mut self.world,
                &mut self.geometry,
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
            crate::gameplay::water_component::startup::physics::WaterPhysicsStartup {
                world: &mut self.world,
                geometry: &mut self.geometry,
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
            crate::gameplay::component_state::prepare_admission(&mut audio.owner.flags,
                u8::from(scheduling.priority < 8), scheduling.priority, priority);
            audio.startup = startup;
            audio.started = true;
            Ok(())
        } else {
            let priority = self.entities[entity_index].priority;
            crate::gameplay::component_state::prepare_admission(
                &mut self.owner_mut(handle)?.flags,
                u8::from(scheduling.priority < 8),
                scheduling.priority,
                priority,
            );
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
