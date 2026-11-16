//! Actual dynamic constructors8aecc0/874c50/784db0, sharing the level's
//! component factories, managed generations and canonical physics world.
use super::super::runtime_entities::{BodyDraft, BodyResources, SpawnOptions};
use super::*;
use crate::{
    assets::runtime_entity::{Definition, Kind},
    gameplay::{
        component_state::lifecycle::ComponentLifecycleWorld,
        entity_events::EntityEvents,
        entity_lifetime::{EntityId, LiveEntities},
    },
};
#[derive(Default)]
pub(in crate::app) struct Retirement {
    components: super::lifetime::Retirement,
    rollback_done: bool,
}
impl PhysicsScene {
    ///738740/738a20 require the published audio world and actual model bounds.
    ///Retain only resolved immutable data on the CPU preparation worker.
    pub(in crate::app) fn prepare_placed_sound_volumes(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        _models: &[entities::PlacedModel],
    ) -> Result<(), String> {
        use entities::scene_motion::{atom, child};
        if !self.prepared_sound_volumes.is_empty() {
            return Err("SoundVolumeControl plans already staged".into());
        }
        let declarations = document
            .definitions
            .iter()
            .filter(|d| d.kind == "component")
            .map(|d| (d.id.as_str(), d))
            .collect::<HashMap<_, _>>();
        let mut fields_cache = HashMap::new();
        let mut definitions =
            HashMap::<String, Arc<crate::gameplay::sound::volume_control::Definition>>::new();
        let mut read =
            |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
        for authored in document.definitions.iter().filter(|d| d.kind == "entity") {
            let Some(index) = self
                .entities
                .iter()
                .position(|entity| !entity.retired && entity.name == authored.id)
            else {
                continue;
            };
            for (slot, value) in child(child(&authored.fields, "edit"), "m_componentDecls")
                .iter()
                .filter(|(slot, _)| slot.starts_with("item_add["))
            {
                let Some(entry) = entities::scope(Some(value)) else {
                    continue;
                };
                let Some(fields) = entities::components::entry_fields(
                    entry,
                    "cpntsoundvolumecontrol",
                    &declarations,
                    &mut fields_cache,
                    &mut read,
                )?
                else {
                    continue;
                };
                let key = if let Some(name) = atom(entry, "mapComponentDecl") {
                    format!("map:{name}")
                } else if entities::field(entry, "inlineDecl").is_none() {
                    format!("decl:{}", atom(entry, "inheritedDecl").unwrap_or(""))
                } else {
                    format!("{}:{slot}", authored.id)
                };
                let definition = match definitions.get(&key) {
                    Some(definition) => definition.clone(),
                    None => {
                        let definition = Arc::new(
                            crate::gameplay::sound::volume_control::Definition::read(&fields)?,
                        );
                        definitions.insert(key, definition.clone());
                        definition
                    }
                };
                self.prepared_sound_volumes.push(
                    super::super::runtime_entities::PreparedSoundVolume {
                        actor: self.entities[index].shared,
                        definition,
                        scheduling: entities::components::scheduling(
                            &fields,
                            DeclarationScheduling::from_base_flags(0),
                        )?,
                        source: self.prepared_sound_volumes.len(),
                        handle: None,
                    },
                );
            }
        }
        Ok(())
    }
    pub(in crate::app) fn take_prepared_sound_volumes(
        &mut self,
    ) -> Vec<super::super::runtime_entities::PreparedSoundVolume> {
        std::mem::take(&mut self.prepared_sound_volumes)
    }
    pub(in crate::app) fn admit_prepared_sound_volume(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        plan: &mut super::super::runtime_entities::PreparedSoundVolume,
    ) -> Result<(), String> {
        if live.stage(plan.actor) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
            return Err("SoundVolumeControl parent is not spawned".into());
        }
        let &index = self
            .shared_indices
            .get(&plan.actor)
            .ok_or("SoundVolumeControl actual parent absent")?;
        let mut registry = self
            .registry
            .take()
            .ok_or("SoundVolumeControl registry borrowed")?;
        let result = (|| {
            let checked = match plan.handle {
                Some(checked) => checked,
                None => {
                    let declaration = self.types.declaration(
                        self.factories[17],
                        plan.source,
                        plan.scheduling.clone(),
                    )?;
                    let definition = plan.definition.clone();
                    let handle = ComponentFactory {
                        declaration: &declaration,
                        pool: &mut self.sound_volumes,
                        registry: &mut registry,
                        grow_one_at_a_time: false,
                    }
                    .create(
                        &mut self.entities[index].components,
                        declaration.scheduling.starts_on_spawn(),
                        |_, _, _| {
                            let mut component =
                                crate::gameplay::sound::volume_control::Component::new(
                                    Default::default(),
                                );
                            component.definition = Some(definition);
                            Ok(component)
                        },
                        |_, _, _| Ok(()),
                    )?
                    .ok_or("SoundVolumeControl component creation disabled")?;
                    self.declarations.insert(handle.packed(), declaration);
                    let checked = crate::gameplay::pickable_component::CheckedComponent {
                        handle,
                        generation: self.owner(handle)?.generation,
                    };
                    plan.handle = Some(checked);
                    checked
                }
            };
            if self.owner(checked.handle)?.generation != checked.generation {
                return Err("SoundVolumeControl actual generation changed".into());
            }
            self.run_runtime_startup(world, live, events, &mut registry)
        })();
        self.registry = Some(registry);
        result
    }
    ///Stage only typed bindings in the CPU preparation worker. Their actual
    ///Breakable component waits for canonical real chunk readiness in appWorld.
    pub(in crate::app) fn prepare_placed_breakables(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        models: &[entities::PlacedModel],
    ) -> Result<(), String> {
        if !self.prepared_breakables.is_empty() {
            return Err("placed Breakable source plans already staged".into());
        }
        let mut children_by_path = HashMap::<String, Arc<Definition>>::new();
        for (source, binding) in crate::assets::breakable::bindings(catalog, document)?
            .into_iter()
            .enumerate()
        {
            let Some(index) = self
                .entities
                .iter()
                .position(|entity| !entity.retired && entity.name == binding.entity_id)
            else {
                continue;
            };
            let authored = document
                .definitions
                .iter()
                .find(|definition| {
                    definition.kind == "entity" && definition.id == binding.entity_id
                })
                .ok_or("placed Breakable source entity absent")?;
            let scale = match models.iter().find(|model| model.id == binding.entity_id) {
                Some(model) => model.scale,
                None => entities::model_descriptor(authored, |path| {
                    crate::assets::declaration_texts::read_common_or_catalog(catalog, path)
                })?
                .map_or(self.entities[index].frame.scale, |model| model.scale),
            };
            let mut children = Vec::new();
            for (step, chunk, entity) in binding.definition.chunks() {
                let definition = match children_by_path.get(entity) {
                    Some(definition) => definition.clone(),
                    None => {
                        let definition = Definition::acquire(catalog, entity, None)?;
                        children_by_path.insert(entity.to_owned(), definition.clone());
                        definition
                    }
                };
                children.push(super::super::runtime_entities::PreparedBreakableChild {
                    step,
                    chunk,
                    definition,
                });
            }
            let actor = self.entities[index].shared;
            self.prepared_breakables
                .push(super::super::runtime_entities::PreparedBreakable {
                    actor,
                    definition: binding.definition,
                    scheduling: binding.scheduling,
                    source,
                    scale,
                    scope: Arc::from(format!("placed-breakable:{actor:?}:{source}")),
                    children,
                    created: Vec::new(),
                    handle: None,
                    requested: false,
                    installed: 0,
                });
        }
        Ok(())
    }
    pub(in crate::app) fn take_prepared_breakables(
        &mut self,
    ) -> Vec<super::super::runtime_entities::PreparedBreakable> {
        std::mem::take(&mut self.prepared_breakables)
    }
    pub(in crate::app) fn admit_prepared_breakable(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        plan: &mut super::super::runtime_entities::PreparedBreakable,
    ) -> Result<(), String> {
        if live.stage(plan.actor) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
            return Err("placed Breakable parent has no actual spawned header".into());
        }
        let &index = self
            .shared_indices
            .get(&plan.actor)
            .ok_or("placed Breakable parent is not admitted")?;
        self.entities[index].components.start_deferred =
            self.entities[index].entity_flags & 0x100 != 0;
        let mut registry = self
            .registry
            .take()
            .ok_or("placed Breakable registry borrowed")?;
        let result = (|| {
            let checked = match plan.handle {
                Some(checked) => checked,
                None => {
                    let handle = self.admit_breakable(
                        index,
                        plan.definition.clone(),
                        plan.scheduling.clone(),
                        plan.source,
                        &mut registry,
                    )?;
                    let checked = crate::gameplay::pickable_component::CheckedComponent {
                        handle,
                        generation: self.owner(handle)?.generation,
                    };
                    plan.handle = Some(checked);
                    checked
                }
            };
            if self.owner(checked.handle)?.generation != checked.generation {
                return Err("placed Breakable component generation changed".into());
            }
            while plan.installed < plan.created.len() {
                let (step, chunk, entity) = plan.created[plan.installed];
                self.install_breakable_chunk(
                    checked.handle,
                    checked.generation,
                    step,
                    chunk,
                    entity,
                )?;
                plan.installed += 1;
            }
            self.run_runtime_startup(world, live, events, &mut registry)
        })();
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn publish_runtime_resources(
        &mut self,
        drafts: Vec<BodyDraft>,
    ) -> Result<Vec<BodyResources>, String> {
        let mut query = self
            .world
            .write()
            .map_err(|_| "runtime physics world is poisoned")?;
        if !query.storage.materials.library.added.is_empty() {
            return Err(
                "runtime material publication requires its actual registered observer host".into(),
            );
        }
        drafts
            .into_iter()
            .map(|body| {
                Ok(BodyResources {
                    component: body.component,
                    variant: body.variant,
                    resources: Arc::new(
                        body.draft
                            .publish(&mut query.storage.materials, |_, _, _, _| {
                                unreachable!("material subscriber list was checked empty")
                            })?,
                    ),
                })
            })
            .collect()
    }
    pub(in crate::app) fn allocate_runtime_actor(
        &mut self,
        live: &mut LiveEntities,
        definition: &Definition,
        options: SpawnOptions,
    ) -> Result<EntityId, String> {
        if self.registry.is_none() {
            return Err("runtime entity factory registry is borrowed".into());
        }
        let scale = super::super::runtime_entities::entity_scale(definition, options.scale);
        if scale.iter().any(|value| !value.is_finite()) {
            return Err("runtime actor scale is nonfinite".into());
        }
        let index = self
            .free_entities
            .last()
            .copied()
            .unwrap_or(self.entities.len());
        if index < self.entities.len()
            && (!self.entities[index].retired
                || !self.entities[index].components.entries.values().is_empty())
        {
            return Err("free runtime entity header is still owned".into());
        }
        if index == self.entities.len() {
            self.entities
                .try_reserve(1)
                .map_err(|error| format!("runtime actor headers: {error}"))?;
            self.entity_flags_d4
                .try_reserve(1)
                .map_err(|error| format!("runtime actor flags: {error}"))?;
        }
        self.shared_indices
            .try_reserve(1)
            .map_err(|error| format!("runtime actor identities: {error}"))?;
        let tag = self.types.intern_name(Some(b"MAIN"))?;
        let id =
            live.allocate_managed_auto_name(&definition.entity, definition.class_index, 0, tag)?;
        let name = live
            .get(id)
            .ok_or("new managed actor disappeared")?
            .0
            .to_owned();
        if index < self.entities.len() {
            self.free_entities.pop();
        }
        let mut components = EntityComponents::new(index);
        //875d50 tests actual entity+d1 bit0 before queuing ANY component
        //Start. Initialize still constructs the real disabled bodies/resources.
        components.start_deferred = options.hidden_disabled;
        let entity = PhysicsEntity {
            name,
            shared: id,
            components,
            class: definition.class_index,
            environment: None,
            flags: 0,
            entity_flags: 0x001e8008 | if options.hidden_disabled { 0x101 } else { 0 },
            entity_flags_d7: 0,
            priority: 0,
            frame: crate::gameplay::reference_frame::PublishedFrame {
                pose: options.frame,
                scale,
                velocities: [[0.; 3]; 2],
            },
            timespace: 0,
            retired: false,
            external: false,
        };
        if index == self.entities.len() {
            self.entities.push(entity);
            self.entity_flags_d4.push(0);
        } else {
            self.entities[index] = entity;
            self.entity_flags_d4[index] = 0;
        }
        self.shared_indices.insert(id, index);
        self.certify_runtime_removal_absence(index);
        Ok(id)
    }
    pub(in crate::app) fn admit_runtime_actor(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        definition: Arc<Definition>,
        resources: &[BodyResources],
        items: &HashMap<usize, Arc<crate::assets::inventory_item::PickableItem>>,
        chunks: &[(usize, usize, usize, EntityId)],
        projectile: Option<crate::gameplay::projectiles::SpawnRequest>,
        append: bool,
        model_changed: bool,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("runtime managed actor is absent")?;
        let mut registry = self
            .registry
            .take()
            .ok_or("runtime component registry is borrowed")?;
        let result = (|| {
            let components = if append {
                &definition.stick_components
            } else {
                &definition.components
            };
            let offset = if append {
                definition.components.len()
            } else {
                0
            };
            let mut created = vec![None; components.len()];
            let mut created_rigid = false;
            for (local_index, component) in components.iter().enumerate() {
                let component_index = offset + local_index;
                if let Kind::Unhosted { class } = &component.kind {
                    static AI: std::sync::Once = std::sync::Once::new();
                    static AIM: std::sync::Once = std::sync::Once::new();
                    static OBS: std::sync::Once = std::sync::Once::new();
                    let once = match *class {
                        "arkCpntAIAttentionSource" => &AI,
                        "arkCpntAimAssist" => &AIM,
                        "arkCpntObservable" => &OBS,
                        _ => {
                            return Err("runtime unhosted capability is not source admitted".into());
                        }
                    };
                    once.call_once(||eprintln!("RUNTIME_ENTITY_UNHOSTED class={class} consumers=absent state=not-constructed"));
                    continue;
                }
                let declaration = self.types.declaration(
                    component.factory,
                    Arc::as_ptr(&definition) as usize + component_index,
                    (*component.scheduling).clone(),
                )?;
                let immediate = declaration.scheduling.starts_on_spawn();
                let handle = match &component.kind {
                    Kind::SoundVolume(definition) => ComponentFactory {
                        declaration: &declaration,
                        pool: &mut self.sound_volumes,
                        registry: &mut registry,
                        grow_one_at_a_time: false,
                    }
                    .create(
                        &mut self.entities[index].components,
                        immediate,
                        |_, _, _| {
                            let mut component =
                                crate::gameplay::sound::volume_control::Component::new(
                                    Default::default(),
                                );
                            component.definition = Some(definition.clone());
                            Ok(component)
                        },
                        |_, _, _| Ok(()),
                    )?,
                    Kind::Rigid(value) => {
                        let bodies = &mut self.actors.bodies;
                        let free = &mut self.free_rigid_movement;
                        ComponentFactory {
                            declaration: &declaration,
                            pool: &mut self.rigid_bodies,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut self.entities[index].components,
                            immediate,
                            |_, _, _| {
                                super::super::rigid_body::RigidBodyComponent::new(
                                    value.clone(),
                                    bodies,
                                    free,
                                )
                            },
                            |_, _, _| Ok(()),
                        )?
                    }
                    Kind::Throwable(value) => {
                        let bodies = &mut self.actors.bodies;
                        let free = &mut self.free_rigid_movement;
                        ComponentFactory {
                            declaration: &declaration,
                            pool: &mut self.throwables,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut self.entities[index].components,
                            immediate,
                            |_, _, _| {
                                super::super::rigid_body::RigidBodyComponent::new_throwable(
                                    value.clone(),
                                    bodies,
                                    free,
                                )
                            },
                            |_, _, _| Ok(()),
                        )?
                    }
                    Kind::Health(value) => {
                        let door =
                            component.class_index == self.types.factory(self.factories[16])?.class;
                        if !door
                            && component.class_index
                                != self.types.factory(self.factories[10])?.class
                        {
                            return Err("runtime derived Health has no actual payload pool".into());
                        }
                        let base = damage::HealthComponent {
                            owner: Default::default(),
                            receiver: value.receiver.clone(),
                            maximum: value.maximum,
                            current: 1.,
                            attacker: None,
                        };
                        if door {
                            ComponentFactory {
                                declaration: &declaration,
                                pool: &mut self.health_doors,
                                registry: &mut registry,
                                grow_one_at_a_time: false,
                            }
                            .create(
                                &mut self.entities[index].components,
                                immediate,
                                |_, _, _| Ok(damage::DoorHealthComponent { base }),
                                |_, _, _| Ok(()),
                            )?
                        } else {
                            ComponentFactory {
                                declaration: &declaration,
                                pool: &mut self.health,
                                registry: &mut registry,
                                grow_one_at_a_time: false,
                            }
                            .create(
                                &mut self.entities[index].components,
                                immediate,
                                |_, _, _| Ok(base),
                                |_, _, _| Ok(()),
                            )?
                        }
                    }
                    Kind::Breakable(value) => ComponentFactory {
                        declaration: &declaration,
                        pool: &mut self.breakables,
                        registry: &mut registry,
                        grow_one_at_a_time: false,
                    }
                    .create(
                        &mut self.entities[index].components,
                        immediate,
                        |_, _, _| Ok(breakable::BreakableComponent::new(value.clone())),
                        |_, _, _| Ok(()),
                    )?,
                    Kind::Projectile(_) => {
                        let prepared =
                            crate::assets::projectiles::Prepared::from_entity(definition.clone())?;
                        let bodies = &mut self.actors.bodies;
                        let free = &mut self.free_rigid_movement;
                        ComponentFactory {
                            declaration: &declaration,
                            pool: &mut self.projectiles,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut self.entities[index].components,
                            immediate,
                            |_, _, _| {
                                super::super::projectiles::Component::new(prepared, bodies, free)
                            },
                            |_, _, _| Ok(()),
                        )?
                    }
                    Kind::Pickable(value) => ComponentFactory {
                        declaration: &declaration,
                        pool: &mut self.pickables,
                        registry: &mut registry,
                        grow_one_at_a_time: false,
                    }
                    .create(
                        &mut self.entities[index].components,
                        immediate,
                        |_, _, _| {
                            Ok(crate::gameplay::pickable_component::PickableComponent::new(
                                value.clone(),
                            ))
                        },
                        |_, _, _| Ok(()),
                    )?,
                    Kind::Audio(value) => ComponentFactory {
                        declaration: &declaration,
                        pool: &mut self.audios,
                        registry: &mut registry,
                        grow_one_at_a_time: false,
                    }
                    .create(
                        &mut self.entities[index].components,
                        immediate,
                        |_, _, _| {
                            let mut audio =
                                crate::gameplay::sound::component::AudioComponent::default();
                            audio.runtime_definition = Some(value.clone());
                            Ok(audio)
                        },
                        |_, _, _| Ok(()),
                    )?,
                    Kind::ReferenceFrame => Some(self.reference_actor(index, &mut registry)?),
                    Kind::AnimBasic { default_alias } => {
                        let rig =
                            definition
                                .model
                                .as_ref()
                                .and_then(|model| match &model.geometry {
                                    crate::assets::equipment::attachment::Geometry::Animated {
                                        rig,
                                        ..
                                    } => Some(rig.clone()),
                                    crate::assets::equipment::attachment::Geometry::Rigid(_) => {
                                        None
                                    }
                                });
                        ComponentFactory {
                            declaration: &declaration,
                            pool: &mut self.animations,
                            registry: &mut registry,
                            grow_one_at_a_time: false,
                        }
                        .create(
                            &mut self.entities[index].components,
                            immediate,
                            |_, _, _| {
                                Ok(super::runtime_animation::AnimBaseComponent::new(
                                    rig,
                                    default_alias.clone(),
                                ))
                            },
                            |_, _, _| Ok(()),
                        )?
                    }
                    Kind::Unhosted { .. } => unreachable!(),
                }
                .ok_or("runtime component creation was disabled")?;
                if self.is_rigid_class(handle.class_id) {
                    let resource = resources
                        .iter()
                        .find(|resource| {
                            resource.component == component_index
                                && resource.variant == u8::from(append && model_changed)
                        })
                        .ok_or("runtime rigid prepared resource is absent")?;
                    self.rigid_component_mut(handle)?
                        .bind_prepared_resources(None, resource.resources.clone_refs())?;
                    created_rigid = true;
                }
                if handle.class_id == self.classes[6] {
                    self.pickables
                        .get_mut(handle.slot)
                        .ok_or("runtime Pickable payload disappeared")?
                        .item = items.get(&component_index).cloned();
                    let generation = self.pickables.get(handle.slot).unwrap().owner.generation;
                    self.readable_components.insert(
                        self.entities[index].name.clone(),
                        crate::gameplay::pickable_component::CheckedComponent {
                            handle,
                            generation,
                        },
                    );
                }
                self.declarations.insert(handle.packed(), declaration);
                created[local_index] = Some(handle);
            }
            self.reference_actor(index, &mut registry)?;
            for &(component, step, chunk, entity) in chunks {
                let handle = created
                    .get(component)
                    .copied()
                    .flatten()
                    .ok_or("runtime chunk actual component absent")?;
                if handle.class_id != self.classes[11] {
                    return Err("runtime chunk source owner is not actual Breakable".into());
                }
                self.install_breakable_chunk(
                    handle,
                    self.owner(handle)?.generation,
                    step,
                    chunk,
                    entity,
                )?;
            }
            if !append {
                if let Some(request) = projectile {
                    live.set_owner(id, request.attacker)?;
                    self.configure_projectile(id, request)?;
                }
                live.initialized(id)?;
                live.initialize_entity_context(id, self.entities[index].components.phase)?;
                live.spawned(id)?;
                live.publish_spawn_tag(id, "MAIN")?;
            } else if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
                return Err(
                    "runtime component append actor is not actual spawned generation".into(),
                );
            }
            self.run_runtime_startup(world, live, events, &mut registry)?;
            if created_rigid {
                if let Some(mut navigation) =
                    world.get_resource_mut::<super::super::NavigationWorld>()
                {
                    self.publish_entity_navigation_ownership(live, id, &mut navigation.0)?;
                }
            }
            Ok(())
        })();
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn entity_water_impact_pending(&self, id: EntityId) -> bool {
        self.water_impacts.iter().any(|impact| impact.owner == id)
    }
    pub(in crate::app) fn runtime_admission_idle(&self) -> bool {
        self.simulation
            .as_ref()
            .is_some_and(|driver| driver.is_idle())
    }
    pub(in crate::app) fn runtime_startup_owner(&self) -> Result<Option<EntityId>, String> {
        let creation = self
            .world
            .read()
            .map_err(|_| "runtime startup world poisoned")?
            .storage
            .disabled_component_creation_owner()
            .map(|(handle, _)| handle);
        let entered = creation.or(self
            .registry
            .as_ref()
            .ok_or("runtime startup registry borrowed")?
            .interrupted_startup_component()?);
        entered
            .map(|handle| {
                let index = self
                    .owner(handle)?
                    .entity
                    .ok_or("entered runtime component unlinked")?;
                Ok(self
                    .entities
                    .get(index)
                    .ok_or("entered runtime actor absent")?
                    .shared)
            })
            .transpose()
    }
    pub(in crate::app) fn prepare_runtime_rollback(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        state: &mut Retirement,
    ) -> Result<bool, String> {
        if state.rollback_done {
            return Ok(true);
        }
        if !self
            .simulation
            .as_ref()
            .is_some_and(|driver| driver.is_idle())
        {
            return Ok(false);
        }
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("runtime rollback header absent")?;
        super::preparation::rollback_runtime_entity(world, self, live, events, index)?;
        state.rollback_done = true;
        Ok(true)
    }
    /// Component/attachment drain only. The caller's canonical removal batch
    /// retains the managed identity until all other owners acknowledge release.
    pub(in crate::app) fn retire_runtime_actor(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        state: &mut Retirement,
        rollback: bool,
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(true);
        };
        if self.entities[index].external {
            return Err("external Player belongs to its actual shutdown owner".into());
        }
        if rollback && !self.prepare_runtime_rollback(world, live, events, id, state)? {
            return Ok(false);
        }
        super::lifetime::poll_entity_components(
            world,
            self,
            live,
            events,
            index,
            &mut state.components,
        )
    }
    pub(in crate::app) fn finish_runtime_retirement(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<(), String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(());
        };
        if live.get(id).is_some() {
            return Err("actor header reclamation precedes managed identity retirement".into());
        }
        if !self.entities[index].components.entries.values().is_empty() {
            return Err("actor header retains live components".into());
        }
        self.free_entities
            .try_reserve(1)
            .map_err(|error| format!("retired actor slots: {error}"))?;
        self.document_hierarchy.retire_entity_frames(index)?;
        self.shared_indices.remove(&id);
        self.rigid_placements.remove(&index);
        self.entities[index].retired = true;
        self.entities[index].name = String::new();
        self.free_entities.push(index);
        Ok(())
    }
}
