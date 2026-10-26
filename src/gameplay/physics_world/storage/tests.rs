use super::*;
use crate::{
    assets::{
        archive::Catalog,
        class_metadata::ClassDeclarations,
        entities::{self, triggers::TriggerAssets},
    },
    gameplay::{
        collision_user_data::CollisionUserData,
        component_state::{
            declaration::DeclarationScheduling,
            entity::{ComponentFactory, EntityComponents},
            types::ComponentTypes,
            ComponentRegistry,
        },
        physics_world::{ComponentPublicationEvents, PublicationSignals},
        trigger_body::{
            initialization::{
                physics::{TriggerLevelInitialization, TriggerPhysicsInitialization},
                LoadedTriggerShape, TriggerDependency,
            },
            DeferredPhysicsState, TriggerComponent, TriggerComponentStore,
        },
        water_component::{
            startup::physics::{WaterLevelStartup, WaterPhysicsStartup},
            WaterComponent,
        },
    },
};

/// Explicit trigger/water composition. Dependency lookup uses the entity's real
/// entries and pool owners; no water/wind instance is fabricated from a name.
struct InstalledTriggerLevel<'a> {
    entity: &'a EntityComponents,
    placement: usize,
    position: [f32; 3],
    collision: crate::gameplay::trigger_body::TriggerCollisionSettings,
    material: u16,
    dependency_ranges: [[i32; 2]; 2],
    scheduling: &'a DeclarationScheduling,
    water_scheduling: Option<&'a DeclarationScheduling>,
    flags: u8,
}
impl<F> TriggerLevelInitialization<TriggerAssets, F> for InstalledTriggerLevel<'_> {
    fn load_shape(
        &mut self,
        entity: usize,
        geometry: &mut TriggerAssets,
    ) -> Result<Option<LoadedTriggerShape>, String> {
        assert_eq!(entity, self.entity.identity);
        Ok(Some(LoadedTriggerShape {
            shape: geometry.shape_resources[self.placement],
            basis: geometry.placements[self.placement].shape.basis,
        }))
    }
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
        components: &TriggerComponentStore<'_>,
    ) -> Result<Option<crate::gameplay::component_state::entity::ComponentHandle>, String> {
        assert_eq!(entity, self.entity.identity);
        self.entity.find_component(
            self.dependency_ranges[usize::from(kind == TriggerDependency::Wind)],
            [0x20, 0],
            |handle| {
                let owner = components
                    .component_owner(handle)
                    .ok_or("lookup component owner is missing")?;
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: owner.flags,
                        start_phases: Some(if handle.class_id == components.water_class {
                            self.water_scheduling
                                .ok_or("water scheduling is missing")?
                                .start_phases
                        } else {
                            self.scheduling.start_phases
                        }),
                    },
                )
            },
        )
    }
    fn position(&self, entity: usize) -> Result<[f32; 3], String> {
        assert_eq!(entity, self.entity.identity);
        Ok(self.position)
    }
    fn collision_settings(
        &self,
        declaration: usize,
    ) -> Result<crate::gameplay::trigger_body::TriggerCollisionSettings, String> {
        assert_eq!(declaration, self.placement);
        Ok(self.collision)
    }
    fn trigger_material(
        &self,
        materials: &crate::gameplay::materials::MaterialOwner<usize, usize>,
    ) -> Result<u16, String> {
        assert!(materials.library.get(self.material).is_some());
        Ok(self.material)
    }
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String> {
        assert_eq!(entity, self.entity.identity);
        self.flags |= 4;
        Ok(())
    }
    fn report_missing_shape(&mut self, _: usize) -> Result<(), String> {
        panic!("installed trigger resource was already loaded")
    }
    fn release_failed(
        &mut self,
        _: crate::gameplay::component_state::entity::ComponentHandle,
        _: &mut WorldStorage,
        _: &mut TriggerAssets,
        _: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String> {
        panic!("installed trigger resource was already loaded")
    }
}

struct InstalledWaterLevel<'a> {
    entity: &'a EntityComponents,
    trigger_range: [i32; 2],
    scheduling: &'a DeclarationScheduling,
    contacts: &'a mut crate::gameplay::water_contacts::WaterContactManager,
    notifications: usize,
}
struct StartupContacts<'a, 'component> {
    components: &'a TriggerComponentStore<'component>,
}
impl crate::gameplay::water_contacts::WaterDensityWorld for StartupContacts<'_, '_> {
    fn prepare_body(&mut self, _: u32) -> Result<(), String> {
        panic!("new level has no retained rigid-body contacts")
    }
    fn density_override(&self, _: u32) -> Result<f32, String> {
        panic!("new level has no retained rigid-body contacts")
    }
    fn body_mass(&self, _: u32) -> Result<f32, String> {
        panic!("new level has no retained rigid-body contacts")
    }
    fn shape_volume(&self, _: u32) -> Result<Option<[f32; 4]>, String> {
        panic!("new level has no retained rigid-body contacts")
    }
    fn body_label(&self, _: u32) -> Result<String, String> {
        panic!("new level has no retained rigid-body contacts")
    }
}
impl crate::gameplay::water_contacts::WaterContactWorld for StartupContacts<'_, '_> {
    fn water_state(
        &self,
        reference: crate::gameplay::component_state::entity::ComponentHandle,
    ) -> Result<crate::gameplay::water_contacts::WaterContactState, String> {
        if reference.class_id != self.components.water_class {
            return Err("contact component is not water".into());
        }
        Ok(self
            .components
            .waters
            .get(reference.slot)
            .ok_or("water contact component is missing")?
            .into())
    }
    fn deactivate_body(&mut self, _: u32) -> Result<(), String> {
        panic!("new level has no retained rigid-body contacts")
    }
}
impl<F> WaterLevelStartup<TriggerAssets, F> for InstalledWaterLevel<'_> {
    fn priorities(
        &self,
        _: crate::gameplay::component_state::entity::ComponentHandle,
    ) -> Result<[i32; 2], String> {
        Ok([self.scheduling.priority, 0])
    }
    fn trigger(
        &mut self,
        entity: usize,
        components: &TriggerComponentStore<'_>,
    ) -> Result<crate::gameplay::component_state::entity::ComponentHandle, String> {
        assert_eq!(entity, self.entity.identity);
        self.entity
            .find_component(self.trigger_range, [0x20, 0], |handle| {
                let owner = &components.trigger(handle)?.owner;
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: owner.flags,
                        start_phases: Some(self.scheduling.start_phases),
                    },
                )
            })?
            .ok_or_else(|| "water has no trigger component".into())
    }
    fn material_registered(
        &mut self,
        _: &mut crate::gameplay::signal::Signal<usize, usize>,
        _: usize,
        _: usize,
        _: u16,
    ) {
        panic!("initial material library has no subscribers")
    }
    fn notify_contacts(
        &mut self,
        reference: crate::gameplay::component_state::entity::ComponentHandle,
        _: &mut WorldStorage,
        _: &mut TriggerAssets,
        events: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String> {
        self.notifications += 1;
        self.contacts.notify_component(
            reference,
            &mut StartupContacts {
                components: &events.components,
            },
        )
    }
}

fn unexpected_subscriber(
    _: &mut crate::gameplay::signal::Signal<usize, usize>,
    _: usize,
    _: usize,
    _: crate::gameplay::body_activation::ActivationOwners<'_>,
    _: crate::gameplay::physics_world::PublicationEvent<'_>,
    _: &mut TriggerComponentStore<'_>,
) -> Result<(), String> {
    panic!("empty world signals have no subscribers")
}

#[test]
#[ignore = "requires installed Edge of the World archives and executable metadata"]
fn installed_triggers_create_in_the_shared_world_buffers() {
    run_installed_trigger_world("eotw_p_lowchaos");
}

#[test]
#[ignore = "requires installed water-volume levels and executable metadata"]
fn installed_water_volumes_publish_from_shared_world_queries() {
    for level in ["bunker_backyard_p", "dust_return_p", "madinventor_p"] {
        run_installed_trigger_world(level);
    }
}

fn run_installed_trigger_world(level_name: &str) {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let level = crate::assets::level::LevelResources::resolve(&catalog, level_name).unwrap();
    let bytes = level.read_entities(&catalog).unwrap().unwrap();
    let document = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
    let map = level
        .entities
        .strip_prefix("maps/")
        .unwrap()
        .strip_suffix(".entities")
        .unwrap();
    let mut geometry = TriggerAssets::load(&document, map, &catalog).unwrap();
    let installed = ClassDeclarations::load(&base).unwrap();
    let factory = |name: &[u8]| {
        installed
            .component_pools
            .iter()
            .find(|p| installed.classes[p.class].name == name)
            .unwrap()
            .pool_index
    };
    let trigger_index = factory(b"arkCpntTrigger");
    let water_index = factory(b"arkCpntWater");
    let wind_index = factory(b"arkCpntWind");
    let entity_classes = installed
        .classes
        .iter()
        .enumerate()
        .map(|(index, class)| (class.name.clone(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let trigger_entity_class = entity_classes[b"arkTrigger".as_slice()];
    let area_entity_class = entity_classes[b"arkAreaVolume".as_slice()];
    let documents = document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity")
        .map(|d| (d.id.as_str(), d))
        .collect::<std::collections::HashMap<_, _>>();
    let placement_classes: Vec<_> = geometry
        .placements
        .iter()
        .map(|p| {
            let definition = documents[p.binding.entity_id.as_str()];
            let class = definition
                .fields
                .iter()
                .find_map(|(name, value)| {
                    if name == "class" {
                        if let entities::Value::Atom(class) = value {
                            return Some(class);
                        }
                    }
                    None
                })
                .unwrap();
            *entity_classes
                .get(class.trim_matches('"').as_bytes())
                .unwrap_or_else(|| {
                    panic!(
                        "unresolved entity class {class:?} on {}",
                        p.binding.entity_id
                    )
                })
        })
        .collect();
    let volumes =
        crate::rendering::environment::load_environment_volumes(&document, &catalog).unwrap();
    let water_bindings = entities::water::water_bindings(&document, |name| {
        catalog
            .read(name)
            .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
            .map_err(|e| e.to_string())
    })
    .unwrap();
    let volume_indices = volumes
        .iter()
        .enumerate()
        .map(|(i, v)| (v.placement.id.as_str(), i))
        .collect::<std::collections::HashMap<_, _>>();
    let types = ComponentTypes::new(installed).unwrap();
    assert!(
        types.initialization_rank(water_index).unwrap()
            > types.initialization_rank(trigger_index).unwrap()
    );
    let dependency_ranges = [water_index, wind_index].map(|index| {
        types
            .classes()
            .hierarchy()
            .type_range(types.factory(index).unwrap().class)
            .unwrap()
    });
    let cvars = crate::platform::cvars::CVars::new();
    let mut triggers = types
        .create_pool::<TriggerComponent>(trigger_index, &cvars)
        .unwrap();
    let mut waters = types
        .create_pool::<WaterComponent>(water_index, &cvars)
        .unwrap();
    let mut registry = ComponentRegistry::new().unwrap();
    let mut actors = crate::gameplay::physics_actor::PhysicsActorLinks {
        actors: types.create_pool(32, &cvars).unwrap(),
        bodies: Vec::new(),
    };
    let mut deferred = DeferredPhysicsState {
        world_flags: [0; 2],
        component_flag: 0,
    };
    let trigger_class = types
        .declaration(trigger_index, 0, DeclarationScheduling::from_base_flags(0))
        .unwrap()
        .class_id;
    let water_class = types
        .declaration(water_index, 0, DeclarationScheduling::from_base_flags(0))
        .unwrap()
        .class_id;
    let mut signals = PublicationSignals::<usize, usize>::default();
    let mut config = WorldInfo::default();
    config.broad_phase_aabb = entities::physics::world_physics_bounds(&document)
        .unwrap()
        .unwrap();
    let constants = std::sync::Arc::new(
        crate::assets::physics_material::MaterialConstants::load(&base).unwrap(),
    );
    let mut materials =
        crate::gameplay::materials::MaterialOwner::<usize, usize>::new(constants, 16).unwrap();
    let trigger_material = materials
        .declaration(&catalog, "narrow_phase_trigger")
        .unwrap();
    let trigger_material_id = materials
        .resolve(trigger_material, |_, _, _, _| {
            panic!("initial library has no subscribers")
        })
        .unwrap();
    let mut world = WorldStorage::new(config, 0, materials).unwrap();
    let mut water_contacts = crate::gameplay::water_contacts::WaterContactManager::new().unwrap();
    let trigger_range = types
        .classes()
        .hierarchy()
        .type_range(types.factory(trigger_index).unwrap().class)
        .unwrap();
    let mut water_handles = Vec::new();
    let mut started_water = 0;
    let mut handles = Vec::new();
    for i in 0..geometry.placements.len() {
        let mut entity = EntityComponents::new(i);
        let water_binding = water_bindings
            .iter()
            .enumerate()
            .find(|(_, b)| b.entity_id == geometry.placements[i].binding.entity_id);
        let water_reference = water_binding.map(|(index, binding)| {
            let declaration = types
                .declaration(water_index, index, binding.scheduling.clone())
                .unwrap();
            let handle = ComponentFactory {
                declaration: &declaration,
                pool: &mut waters,
                registry: &mut registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut entity,
                false,
                |_, _, _| Ok(WaterComponent::default()),
                |_, _, _| panic!("creation did not request water start"),
            )
            .unwrap()
            .unwrap();
            //588300 is the actual water initializer, the common phase transition.
            waters
                .get_mut(handle.slot)
                .unwrap()
                .owner
                .mark_initialized();
            handle
        });
        water_handles.push(water_reference);
        let declaration = types
            .declaration(trigger_index, i, DeclarationScheduling::from_base_flags(0))
            .unwrap();
        let handle = ComponentFactory {
            declaration: &declaration,
            pool: &mut triggers,
            registry: &mut registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entity,
            false,
            |_, _, _| TriggerComponent::new(&mut actors.bodies),
            |_, _, _| panic!("creation did not request component start"),
        )
        .unwrap()
        .unwrap();
        // Use the constructor scheduling fields as an explicit composition
        // input. This test does not claim to decode all authored scheduling.
        let placement = &geometry.placements[i];
        let keyframed = placement.binding.collision.keyframed;
        let position = placement.position;
        let collision = placement.binding.collision;
        let reference_actor = if keyframed {
            let actor_declaration = types
                .declaration(32, i, DeclarationScheduling::reference_frame(0))
                .unwrap();
            let actor = ComponentFactory {
                declaration: &actor_declaration,
                pool: &mut actors.actors,
                registry: &mut registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut entity,
                false,
                |_, _, _| {
                    let embedded = actors.bodies.len();
                    actors
                        .bodies
                        .push(crate::gameplay::physics_actor::ActorBody {
                            actor: None,
                            body_id: u32::MAX,
                        });
                    Ok(crate::gameplay::physics_actor::PhysicsActor::new(embedded))
                },
                |_, _, _| panic!("unrequested reference-frame start"),
            )
            .unwrap()
            .unwrap();
            entity.reference_actor = Some(actor);
            Some(actor)
        } else {
            None
        };
        let mut events = ComponentPublicationEvents {
            components: TriggerComponentStore {
                trigger_class,
                water_class,
                triggers: &mut triggers,
                waters: &mut waters,
                actors: &mut actors,
                deferred: &mut deferred,
            },
            signals: &mut signals,
            dispatch: unexpected_subscriber,
        };
        let mut level = InstalledTriggerLevel {
            entity: &entity,
            placement: i,
            position,
            collision,
            material: trigger_material_id,
            dependency_ranges,
            scheduling: &declaration.scheduling,
            water_scheduling: water_binding.map(|(_, b)| &b.scheduling),
            flags: 0,
        };
        TriggerPhysicsInitialization {
            world: &mut world,
            geometry: &mut geometry,
            events: &mut events,
            level: &mut level,
        }
        .initialize(handle)
        .unwrap_or_else(|e| panic!("trigger{i}: {e}"));
        assert_eq!(level.flags & 4, u8::from(keyframed) * 4);
        let movement_body = events.components.trigger(handle).unwrap().movement_body;
        let body = events.components.actors.bodies[movement_body].body_id;
        if let Some(water) = water_reference {
            let binding = water_binding.unwrap().1;
            let startup = binding
                .startup(|name| world.materials.declaration(&catalog, name))
                .unwrap();
            let mut water_level = InstalledWaterLevel {
                entity: &entity,
                trigger_range,
                scheduling: &binding.scheduling,
                contacts: &mut water_contacts,
                notifications: 0,
            };
            WaterPhysicsStartup {
                world: &mut world,
                geometry: &mut geometry,
                events: &mut events,
                level: &mut water_level,
            }
            .start(water, &startup)
            .unwrap();
            assert_eq!(water_level.notifications, 1);
            let component = events.components.waters.get(water.slot).unwrap();
            assert_eq!(component.owner.flags & 12, 8);
            assert_eq!(
                component.flow_and_density(),
                Some([
                    binding.current[0],
                    binding.current[1],
                    binding.current[2],
                    binding.density
                ])
            );
            let created = world.bodies.get(body).unwrap();
            let bounds = crate::gameplay::physics_world::PublicationGeometry::bounds(
                &mut geometry,
                created.shape,
                created.transform,
            )
            .unwrap();
            assert_eq!(
                component.plane().unwrap()[3].to_bits(),
                crate::float::subtract(0., bounds[1][2]).to_bits()
            );
            if let Some(material) = startup.materials[usize::from(startup.is_ocean)] {
                let expected = world
                    .materials
                    .resolve(material, |_, _, _, _| {
                        panic!("no initial material subscribers")
                    })
                    .unwrap();
                assert_eq!(created.material_id, expected);
            }
            started_water += 1;
        }
        crate::gameplay::trigger_body::startup::start_trigger(
            &mut world,
            &mut geometry,
            &mut events,
            handle,
            &declaration.scheduling,
            0, // Explicit composition input; priority8 disables automatic updates.
            keyframed,
            |components, reference| {
                let actor = reference_actor.expect("only keyframed triggers bind an actor");
                let movement = components.trigger(reference)?.movement_body;
                components
                    .actors
                    .attach_body(actor.slot as usize, movement, 0, |_, _| {
                        panic!("new actor has no enabled motion requests")
                    })
            },
        )
        .unwrap();
        assert_eq!(
            events.components.trigger(handle).unwrap().owner.flags & 0xc,
            8
        );
        if let Some(actor) = reference_actor {
            assert_eq!(
                events.components.actors.actors[actor.slot as usize].current_body,
                movement_body
            );
            assert_eq!(
                events.components.actors.bodies[movement_body].actor,
                Some(actor.slot as usize)
            );
        }
        let signal = world
            .body_events
            .get_or_create(world.bodies.capacity(), body, 0)
            .unwrap();
        let mut callbacks = Vec::new();
        signal.emit(&(), |_, subscriber, callback, _| {
            callbacks.push((subscriber, callback))
        });
        assert_eq!(
            callbacks,
            [(handle, super::super::BodyEventCallback::TriggerContact)]
        );
        let created = world.bodies.get(body).unwrap();
        assert_eq!(
            created.user_data,
            CollisionUserData { component: handle }.encode()
        );
        assert_ne!(created.broadphase_id, u32::MAX);
        assert!(world.motions.get(created.motion_id).is_some());
        assert!(world.deactivation.state(created.motion_id).is_some());
        handles.push((handle, body));
    }
    assert!(!handles.is_empty());
    assert_eq!(
        handles.len(),
        geometry.placements.len(),
        "every installed trigger must be constructed"
    );
    assert_eq!(world.bodies.active_count() as usize, handles.len() + 1);
    let mut queries = CollisionQueries::default();
    queries.set_submission_allowed(true);
    let mut submitted = Vec::new();
    for (_, body) in handles.iter().step_by(7) {
        let bounds = world
            .quantization
            .packed
            .unpack_bounds(world.bodies.get(*body).unwrap().bounds);
        let point = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
        submitted.push(
            queries
                .submit(
                    WorldPointRequest {
                        point,
                        hit: crate::gameplay::collision_hit::ShapeHitInfo {
                            body_id: None,
                            material_id: 0,
                            shape_key: u32::MAX,
                            filter_info: 0,
                            user_data: 0,
                        },
                        filter: None,
                    },
                    true,
                )
                .unwrap(),
        );
    }
    world
        .resolve_point_queries(
            &mut WorldPointQuery::new(u32::MAX).unwrap(),
            &mut queries,
            |shape| {
                Ok(WorldQueryShape {
                    shapes: &geometry.shapes[shape].query_shapes,
                    materials: None,
                })
            },
        )
        .unwrap();
    let mut hit_count = 0;
    for mut id in submitted {
        let mut hits = Vec::new();
        assert!(queries.take_completed(&mut id, &mut hits).unwrap());
        for hit in hits {
            let body = world.bodies.get(hit.target.body_id.unwrap()).unwrap();
            assert_eq!(hit.target.user_data, body.user_data);
            let component = CollisionUserData::decode(hit.target.user_data)
                .unwrap()
                .component;
            assert!(triggers.get(component.slot).unwrap().owner.entity.is_some());
            hit_count += 1;
        }
    }
    assert!(
        hit_count > 0,
        "queries must execute actual installed geometry"
    );
    println!(
        "{} installed triggers admitted; {hit_count} canonical geometry query hits",
        handles.len()
    );
    // Consume real filtered camera requests through the same body, component
    // and geometry owners; class admission comes from installed RTTI ranges.
    let mut view = crate::rendering::environment_contacts::EnvironmentViewContacts::new(0.);
    let mut executor = WorldPointQuery::new(u32::MAX).unwrap();
    let hierarchy = types.classes().hierarchy();
    let trigger_range = hierarchy.type_range(trigger_entity_class).unwrap();
    let area_range = hierarchy.type_range(area_entity_class).unwrap();
    let mut area_views = 0;
    let mut water_views = 0;
    let mut material_views = 0;
    for (_, body) in &handles {
        let bounds = world
            .quantization
            .packed
            .unpack_bounds(world.bodies.get(*body).unwrap().bounds);
        let point = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
        view.query.submit(&mut queries, true, true, point).unwrap();
        assert!(view
            .resolve(
                &world,
                &mut executor,
                &mut queries,
                |shape| Ok(WorldQueryShape {
                    shapes: &geometry.shapes[shape].query_shapes,
                    materials: None
                }),
                |handle| (handle.class_id == trigger_class)
                    .then(|| &triggers.get(handle.slot).unwrap().owner),
                |entity| {
                    let class = hierarchy.type_id(placement_classes[entity])?;
                    let environment = if (area_range[0]..=area_range[1]).contains(&class) {
                        volume_indices
                            .get(geometry.placements[entity].binding.entity_id.as_str())
                            .map(|&index| (index, volumes[index].is_water_volume))
                    } else {
                        None
                    };
                    Ok(crate::rendering::environment_contacts::ContactEntity {
                        trigger_derived: (trigger_range[0]..=trigger_range[1]).contains(&class),
                        water_component_flags: water_handles[entity]
                            .map(|handle| waters.get(handle.slot).unwrap().owner.flags),
                        environment,
                    })
                }
            )
            .unwrap());
        let publication = view
            .selection
            .water_publication(|index| {
                Ok(volumes[index]
                    .surface_material
                    .as_deref()
                    .map(std::sync::Arc::from))
            })
            .unwrap();
        area_views += usize::from(!view.selection.areas.is_empty());
        water_views += usize::from(view.selection.water.is_some());
        material_views += usize::from(publication.surface_material.is_some());
        if publication.surface_material.is_some() {
            assert_eq!(
                publication.surface_height.unwrap().to_bits(),
                view.selection.water_level.to_bits()
            );
        }
    }
    assert!(
        area_views > 0,
        "actual camera queries must reach installed environment profiles"
    );
    if volumes.iter().any(|v| v.is_water_volume) {
        assert!(
            water_views > 0,
            "{level_name}: installed water volumes must be reached by camera queries"
        );
        assert!(
            material_views > 0,
            "{level_name}: the water volume must publish its real material"
        );
    } else {
        assert_eq!(
            water_views, 0,
            "ordinary areas must not become water volumes"
        );
    }
    println!("Canonical view queries: {area_views} ordinary areas, {water_views} water volumes, {material_views} surface materials");
    println!(
        "{started_water} installed water components started through shared physics/material owners"
    );
    assert_eq!(
        started_water,
        water_bindings.len(),
        "every authored water component must start through its actual trigger"
    );
    world.release(&mut geometry).unwrap();
}
