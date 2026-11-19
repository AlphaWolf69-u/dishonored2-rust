//! Usable lifecycle headers over the existing interaction/animation/FX owners.
//! 77b3b0/77b610 retain one mutable usable state, not a second state machine.
use super::*;
use crate::gameplay::{
    component_state::ComponentInstance, entity_lifetime::LiveEntities, name_pool::NameId,
};

pub(super) struct UsableComponent {
    pub owner: ComponentOwner,
    pub actor: usize,
    pub(super) static_state: Option<super::usable_static::State>,
}
impl ComponentInstance for UsableComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}

impl PhysicsScene {
    ///76e8b0 captures mode1 PhysicsBase/PhysicsSystemEvolved references at
    ///Start. These are weak checked component generations, not resource leases.
    pub(super) fn start_pickable_physics_references(&mut self,handle:ComponentHandle)->Result<(),String> {
        use crate::gameplay::pickable_component::{CheckedComponent,PickableReferences};
        let index=self.owner(handle)?.entity.ok_or("Pickable Start entity absent")?;
        let checked=|candidate:Option<ComponentHandle>|->Result<Option<CheckedComponent>,String>{
            candidate.map(|handle|self.owner(handle).map(|o|CheckedComponent{handle,generation:o.generation})).transpose()
        };
        let physics_base=checked(self.document_rigid(index)?)?;
        let physics_system=checked(self.grapple_dependency(index,4,[0x20,0])?)?;
        let refs=self.pickables.get_mut(handle.slot).ok_or("Pickable Start payload absent")?
            .references.get_or_insert_with(PickableReferences::default);
        refs.physics_base=physics_base;
        refs.physics_system=physics_system;
        Ok(())
    }
    pub(super) fn admit_interaction_components(
        &mut self,
        document: &entities::Document,
        interactions: &super::super::interactions::Interactions,
        live: &mut LiveEntities,
        registry: &mut ComponentRegistry,
        constructor_tag: NameId,
    ) -> Result<(), String> {
        let definitions: HashMap<_, _> = document
            .definitions
            .iter()
            .filter(|entry| entry.kind == "entity")
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut indices: HashMap<_, _> = self
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.name.clone(), index))
            .collect();
        let names: HashMap<_, _> = self
            .types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .map(|&index| {
                let metadata = self
                    .types
                    .classes()
                    .metadata(index)
                    .ok_or("usable class metadata absent")?;
                Ok((
                    self.types
                        .classes()
                        .name_text(metadata.name)
                        .ok_or("usable class name absent")?
                        .to_vec(),
                    index,
                ))
            })
            .collect::<Result<_, String>>()?;
        for (actor_index, actor) in interactions.actors.iter().enumerate() {
            let index = if let Some(&index) = indices.get(&actor.entity) {
                index
            } else {
                let definition = definitions
                    .get(actor.entity.as_str())
                    .ok_or("usable entity declaration absent")?;
                let class = definition
                    .fields
                    .iter()
                    .find_map(|(key, value)| match (key.as_str(), value) {
                        ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                        _ => None,
                    })
                    .ok_or("usable entity class absent")?;
                let class = *names
                    .get(class.as_bytes())
                    .ok_or("usable entity class is not registered")?;
                let index = self.entities.len();
                let shared = live.allocate_managed(
                    Arc::from(actor.entity.as_str()),
                    class,
                    None,
                    0,
                    constructor_tag,
                )?;
                self.entities.push(PhysicsEntity {
                    retired: false,
                    external: false,
                    name: actor.entity.clone(),
                    shared,
                    components: EntityComponents::new(index),
                    class,
                    environment: None,
                    flags: 0,
                    entity_flags: 0x001e_8008,
                    entity_flags_d7: 0,
                    priority: 0,
                    frame: crate::gameplay::reference_frame::PublishedFrame {
                        pose: actor.frame,
                        ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                            .published()
                    },
                    timespace: 0,
                });
                self.entity_flags_d4.push(0);
                indices.insert(actor.entity.clone(), index);
                index
            };
            let is_static=actor.definition.static_definition.is_some();
            let factory=if is_static{58}else{5};
            let locked=if is_static&&self.throwable_class_is(self.entities[index].class,b"arkUsable")? {
                let edit=definitions.get(actor.entity.as_str()).and_then(|def|entities::scope(entities::field(&def.fields,"edit"))).unwrap_or(&[]);
                entities::scene_motion::atom(edit,"m_locked")==Some("true")
            }else{false};
            let declaration = self.types.declaration(self.factories[factory],actor_index,actor.scheduling.clone())?;
            let handle = ComponentFactory {
                declaration: &declaration,
                pool: if is_static {&mut self.static_usables}else{&mut self.usables},
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                declaration.scheduling.starts_on_spawn(),
                |_, _, _| {
                    let mut owner=ComponentOwner::default();
                    if is_static{owner.request_update(1);} //4cd0f0.
                    Ok(UsableComponent {
                        owner,
                        actor: actor_index,
                        static_state:is_static.then(||super::usable_static::State::new(actor.definition.clone(),locked)),
                    })
                },
                |_, _, _| Ok(()),
            )?
            .ok_or("usable component creation disabled")?;
            self.declarations.insert(handle.packed(), declaration);
        }
        for binding in &interactions.readable {
            let definition = definitions
                .get(binding.entity.as_str())
                .ok_or("readable entity declaration absent")?;
            let index = if let Some(&index) = indices.get(&binding.entity) {
                index
            } else {
                let class = definition
                    .fields
                    .iter()
                    .find_map(|(key, value)| match (key.as_str(), value) {
                        ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                        _ => None,
                    })
                    .ok_or("readable entity class absent")?;
                let class = *names
                    .get(class.as_bytes())
                    .ok_or("readable entity class is not registered")?;
                let index = self.entities.len();
                // A failed parse must not leave an identity outside the scene's
                // rollback roster.
                let pose = entities::spawn_frame(definition)?;
                let shared = live.allocate_managed(
                    Arc::from(binding.entity.as_str()),
                    class,
                    None,
                    0,
                    constructor_tag,
                )?;
                self.entities.push(PhysicsEntity {
                    retired: false,
                    external: false,
                    name: binding.entity.clone(),
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
                        ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                            .published()
                    },
                    timespace: 0,
                });
                self.entity_flags_d4.push(0);
                indices.insert(binding.entity.clone(), index);
                index
            };
            let item = interactions.acquire_readable_item(&binding.item)?;
            let declaration = self.types.declaration(
                self.factories[6],
                Arc::as_ptr(&binding.component) as usize,
                binding.component.scheduling.clone(),
            )?;
            let handle = ComponentFactory {
                declaration: &declaration,
                pool: &mut self.pickables,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                declaration.scheduling.starts_on_spawn(),
                |_, _, _| {
                    let mut component = crate::gameplay::pickable_component::PickableComponent::new(
                        binding.component.clone(),
                    );
                    component.item = Some(item);
                    Ok(component)
                },
                |_, _, _| Ok(()),
            )?
            .ok_or("Pickable component creation disabled")?;
            let generation = self
                .pickables
                .get(handle.slot)
                .ok_or("new Pickable component absent")?
                .owner
                .generation;
            self.readable_components.insert(
                binding.entity.clone(),
                crate::gameplay::pickable_component::CheckedComponent { handle, generation },
            );
            self.declarations.insert(handle.packed(), declaration);
            if let Some(definition) = &binding.loot {
                let declaration = self.types.declaration(
                    self.factories[7],
                    Arc::as_ptr(definition) as usize,
                    definition.scheduling.clone(),
                )?;
                let handle = ComponentFactory {
                    declaration: &declaration,
                    pool: &mut self.loots,
                    registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| {
                        Ok(crate::gameplay::loot_component::LootComponent::new(definition.clone()))
                    },
                    |_, _, _| Ok(()),
                )?
                .ok_or("Loot component creation disabled")?;
                self.declarations.insert(handle.packed(), declaration);
            }
            if let Some((name, scheduling)) = &binding.faction {
                let definition = self.faction_session.definition(name)?;
                let declaration = self.types.declaration(
                    self.factories[9],
                    Arc::as_ptr(&definition) as usize,
                    scheduling.clone(),
                )?;
                let handle = ComponentFactory {
                    declaration: &declaration,
                    pool: &mut self.factions,
                    registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| {
                        let mut component =
                            super::super::faction_component::FactionComponent::default();
                        component.set_definition(definition.clone())?;
                        Ok(component)
                    },
                    |_, _, _| Ok(()),
                )?
                .ok_or("Faction component creation disabled")?;
                self.declarations.insert(handle.packed(), declaration);
            }
        }
        for binding in &interactions.carrying {
            // READ_NOTE and holding can be authored on the same concrete
            // component. Preserve the factory generation already admitted.
            if self.readable_components.contains_key(&binding.entity) {
                continue;
            }
            let definition = definitions
                .get(binding.entity.as_str())
                .ok_or("carried entity declaration absent")?;
            let index = if let Some(&index) = indices.get(&binding.entity) {
                index
            } else {
                let class = definition
                    .fields
                    .iter()
                    .find_map(|(key, value)| match (key.as_str(), value) {
                        ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                        _ => None,
                    })
                    .ok_or("carried entity class absent")?;
                let class = *names
                    .get(class.as_bytes())
                    .ok_or("carried entity class is not registered")?;
                let pose = entities::spawn_frame(definition)?;
                let index = self.entities.len();
                let shared = live.allocate_managed(
                    Arc::from(binding.entity.as_str()),
                    class,
                    None,
                    0,
                    constructor_tag,
                )?;
                self.entities.push(PhysicsEntity {
                    retired: false,
                    external: false,
                    name: binding.entity.clone(),
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
                        ..crate::gameplay::reference_frame::ReferenceFrameCache::default()
                            .published()
                    },
                    timespace: 0,
                });
                self.entity_flags_d4.push(0);
                indices.insert(binding.entity.clone(), index);
                index
            };
            let item = if binding.definition.item.is_empty() {
                None
            } else {
                Some(interactions.acquire_readable_item(&binding.definition.item)?)
            };
            let declaration = self.types.declaration(
                self.factories[6],
                Arc::as_ptr(&binding.definition) as usize,
                binding.scheduling.clone(),
            )?;
            let handle = ComponentFactory {
                declaration: &declaration,
                pool: &mut self.pickables,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                declaration.scheduling.starts_on_spawn(),
                |_, _, _| {
                    let mut component = crate::gameplay::pickable_component::PickableComponent::new(
                        binding.definition.clone(),
                    );
                    component.item = item;
                    Ok(component)
                },
                |_, _, _| Ok(()),
            )?
            .ok_or("carried Pickable component creation disabled")?;
            let generation = self
                .pickables
                .get(handle.slot)
                .ok_or("new carried Pickable absent")?
                .owner
                .generation;
            self.readable_components.insert(
                binding.entity.clone(),
                crate::gameplay::pickable_component::CheckedComponent { handle, generation },
            );
            self.declarations.insert(handle.packed(), declaration);
        }
        Ok(())
    }

    pub(super) fn start_usable_backend(&self, world: &mut bevy::prelude::World,
        handle: ComponentHandle) -> Result<(), String> {
        let component = if handle.class_id==self.classes[58]{self.static_usables.get(handle.slot)}else{self.usables.get(handle.slot)}.ok_or("Usable Start component absent")?;
        let preserve_state = component.owner.flags & (1 << 30) != 0;
        if !world.contains_resource::<super::super::interactions::Interactions>() {
            return Err("Usable Start interaction owner absent".into());
        }
        world.resource_scope(|world, mut interactions: bevy::prelude::Mut<super::super::interactions::Interactions>| {
            let mut navigation = world.get_resource_mut::<super::super::NavigationWorld>()
                .ok_or("Usable Start collision owner absent")?;
            interactions.start_actor(component.actor, preserve_state, &mut navigation.0)
        })
    }

    ///3b8430 requests the Interactable base on the contacted actor, not
    ///just its Pickable subclass. Resolve the actual started Usable generation.
    pub(in crate::app) fn interaction_usable(
        &self,
        live: &LiveEntities,
        id: crate::gameplay::entity_lifetime::EntityId,
    ) -> Result<Option<usize>, String> {
        if live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&id) else { return Ok(None); };
        let entity = &self.entities[index];
        if entity.retired || entity.entity_flags & 1 != 0 { return Ok(None); }
        let handle = entity.components.find_component([self.classes[5]; 2], [0x28, 0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                flags: self.owner(handle)?.flags,
                start_phases: self.declarations.get(&handle.packed())
                    .map(|declaration| declaration.scheduling.start_phases),
            })
        })?;
        handle.map(|handle| self.usables.get(handle.slot).map(|component| component.actor)
            .ok_or_else(|| "interaction Usable generation absent".to_owned())).transpose()
    }

    pub(in crate::app) fn readable_identity(
        &self,
        entity: &str,
        live: &LiveEntities,
    ) -> Result<
        Option<(
            crate::gameplay::entity_lifetime::EntityId,
            crate::gameplay::pickable_component::CheckedComponent,
        )>,
        String,
    > {
        let Some(&reference) = self.readable_components.get(entity) else {
            return Ok(None);
        };
        let owner = self.owner(reference.handle)?;
        if owner.generation != reference.generation {
            return Err("readable component generation is stale".into());
        }
        let entity = self
            .entities
            .get(owner.entity.ok_or("readable component is unlinked")?)
            .ok_or("readable entity slot is absent")?;
        if live.stage(entity.shared) != Some(crate::gameplay::entity_lifetime::Stage::Spawned)
            || owner.flags & 0xf != 0xa
            || entity.entity_flags & 3 != 0
            || self
                .pickables
                .get(reference.handle.slot)
                .is_some_and(|component| component.interaction_flags & 0xa != 0)
        {
            return Ok(None);
        }
        Ok(Some((entity.shared, reference)))
    }

    pub(in crate::app) fn readable_item(
        &self,
        entity: &str,
        live: &LiveEntities,
    ) -> Result<
        Option<(
            (
                crate::gameplay::entity_lifetime::EntityId,
                crate::gameplay::pickable_component::CheckedComponent,
            ),
            Arc<crate::assets::inventory_item::PickableItem>,
        )>,
        String,
    > {
        let Some(identity) = self.readable_identity(entity, live)? else {
            return Ok(None);
        };
        let item = self
            .pickables
            .get(identity.1.handle.slot)
            .and_then(|component| component.item.as_ref())
            .ok_or("readable Pickable item lease is absent")?;
        Ok(Some((identity, item.clone())))
    }

    /// 76e750/76e2b0 use the same live component, including props with no
    /// inventoryItem. Its immutable declaration cannot change during a grab.
    pub(in crate::app) fn carry_pickable(
        &self,
        entity: &str,
        live: &LiveEntities,
    ) -> Result<
        Option<(
            crate::gameplay::entity_lifetime::EntityId,
            Arc<crate::gameplay::pickable_component::PickableDefinition>,
        )>,
        String,
    > {
        let Some((id, reference)) = self.readable_identity(entity, live)? else {
            return Ok(None);
        };
        let component = self
            .pickables
            .get(reference.handle.slot)
            .ok_or("carried Pickable disappeared")?;
        if !component.interaction_enabled
            || !component.definition.usable_by_player
            || component.definition.hold.is_none()
        {
            return Ok(None);
        }
        Ok(Some((id, component.definition.clone())))
    }

    /// 8780d0 reads the current bounds of the contacted actor's render owner.
    /// Level setup publishes one source per rendered placement, in placement
    /// order. A rigid pickup already carries that placement's index.
    pub(in crate::app) fn interaction_bounds(
        &self,
        live: &LiveEntities,
        id: crate::gameplay::entity_lifetime::EntityId,
        runtime: Option<&super::super::world_models::WorldModels>,
        sources: &super::super::shadow_updates::ModelShadowSources,
        placements: &super::super::RenderSourcePlacements,
    ) -> Result<Option<[[f32; 3]; 2]>, String> {
        if let Some(bounds) = runtime.and_then(|models| models.interaction_bounds(id)) {
            return Ok(Some(bounds));
        }
        if let Some(placement) = self.shared_indices.get(&id).and_then(|i| self.rigid_placements.get(i)) {
            let index = sources.0.binary_search_by_key(placement, |source| source.placement);
            if let Ok(index) = index {
                return Ok(Some(sources.0[index].bounds));
            }
        }
        super::super::world_models::entity_bounds_from_parts(live, id, None, Some(sources), placements)
    }

    /// 8780d0's current entity bounds and pose for 3b80a0. A null render
    /// owner returns the entity origin as both bounds, as in the source.
    pub(in crate::app) fn carry_context(
        &self,
        entity: &str,
        live: &LiveEntities,
        sources: &super::super::shadow_updates::ModelShadowSources,
    ) -> Result<
        Option<(
            crate::gameplay::pickable_component::CheckedComponent,
            crate::gameplay::reference_frame::ReferenceFrame,
            [f32; 2],
        )>,
        String,
    > {
        let Some((id, reference)) = self.readable_identity(entity, live)? else {
            return Ok(None);
        };
        let component = self
            .pickables
            .get(reference.handle.slot)
            .ok_or("carry context Pickable absent")?;
        if !component.interaction_enabled
            || !component.definition.usable_by_player
            || component.definition.hold.is_none()
        {
            return Ok(None);
        }
        let Some(frame) = self.entity_frame(live, id)? else {
            return Ok(None);
        };
        let index = component
            .owner
            .entity
            .ok_or("carry context entity absent")?;
        let bounds = if let Some(&placement) = self.rigid_placements.get(&index) {
            let bounds = sources
                .0
                .iter()
                .find(|source| source.placement == placement)
                .ok_or("carry render bounds owner absent")?
                .bounds;
            [bounds[0][2], bounds[1][2]]
        } else {
            [frame.position[2]; 2]
        };
        Ok(Some((reference, frame, bounds)))
    }
}

impl crate::gameplay::component_state::updates::ComponentUpdateWorld for PhysicsScene {
    fn update_remove(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if handle.class_id==self.classes[32]||handle.class_id==self.classes[38]{return Err("Crew update removal requires the canonical borrowed lifecycle/event host".into())}
        registry.base_update_remove(handle, self)
    }
    fn update_add(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if handle.class_id==self.classes[32]{return Err("Crew update admission requires the canonical borrowed event host".into())}
        registry.base_update_add(handle, self)
    }
    fn set_update_scheduled(
        &mut self,
        handle: ComponentHandle,
        scheduled: bool,
    ) -> Result<(), String> {
        self.set_component_update_scheduled(handle, scheduled)
    }
}

pub(super) struct Startup<'a> {
    pub world: Option<&'a mut bevy::prelude::World>,
    pub scene: &'a mut PhysicsScene,
    pub live: &'a mut LiveEntities,
    pub events: &'a mut crate::gameplay::entity_events::EntityEvents,
}
impl Startup<'_> {
    /// Host preparation errors need the authored owner before rollback retires it.
    /// Format only on failure; successful component startup retains no report.
    fn failure(&self, phase: &str, handle: ComponentHandle, error: String) -> String {
        let entity = self
            .scene
            .owner(handle)
            .ok()
            .and_then(|owner| owner.entity)
            .and_then(|index| self.scene.entities.get(index));
        match entity {
            Some(entity) => format!("{phase} component {handle:?} on {}: {error}", entity.name),
            None => format!("{phase} component {handle:?}: {error}"),
        }
    }
}
impl ComponentLifecycleWorld for Startup<'_> {
    fn priority(&self, handle: ComponentHandle) -> Result<i32, String> {
        self.scene.priority(handle)
    }
    fn owner(&self, handle: ComponentHandle) -> Result<&ComponentOwner, String> {
        self.scene.owner(handle)
    }
    fn owner_mut(&mut self, handle: ComponentHandle) -> Result<&mut ComponentOwner, String> {
        self.scene.owner_mut(handle)
    }
    fn initialize(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let result = (|| {
            if handle.class_id == self.scene.classes[59] {
                return self.scene.initialize_constraint(self.live, self.events, registry, handle);
            }
            if handle.class_id == self.scene.classes[57] {
                return self.scene.initialize_fx_component(self.live, handle);
            }
            if handle.class_id == self.scene.classes[60] {
                return self.scene.initialize_whale_oil_tank(self.world.as_deref_mut().ok_or("WhaleOilTank Initialize requires world")?,self.live,self.events,registry,handle);
            }
            if handle.class_id == self.scene.classes[55] {
                return self.scene.initialize_openable(
                    self.world.as_deref_mut().ok_or("Openable Initialize requires published world")?,
                    self.live, self.events, registry, handle,
                );
            }
            if handle.class_id == self.scene.classes[56] {
                return self.scene.initialize_retargeting(handle);
            }
            if self.scene.is_flash_class(handle.class_id) {
                return self.scene.initialize_flash_component(self.live,self.events,handle);
            }
            if self.scene.is_smart_object_class(handle.class_id) {
                return self.scene.initialize_smart_object(self.live,self.events,handle);
            }
            if handle.class_id==self.scene.classes[48]||handle.class_id==self.scene.classes[49] {
                return self.scene.initialize_cue_component(self.live,handle);
            }
            if self.scene.is_ai_startup_class(handle.class_id) {
                return self.scene.initialize_ai_startup(
                    self.world.as_deref_mut().ok_or("AI manager Initialize requires published world")?,
                    self.live,self.events,registry,handle,
                );
            }
            if handle.class_id==self.scene.classes[21] {
                return self.scene.initialize_statistics(handle);
            }
            if handle.class_id==self.scene.classes[22] {
                return self.scene.initialize_timers(self.world.as_deref_mut(),self.live,handle);
            }
            if self.scene.is_crew_class(handle.class_id) {
                self.scene.initialize_crew(handle)
            } else if self.scene.is_crew_collector_class(handle.class_id) {
                self.scene.initialize_crew_collector(handle)
            } else if self.scene.is_crew_module_class(handle.class_id) {
                self.scene.initialize_crew_module(handle)
            } else if handle.class_id == self.scene.classes[20] {
                self.scene.initialize_ai_danger(self.world.as_deref_mut().ok_or("AI danger Initialize requires published world")?,self.live,handle)
            } else if handle.class_id == self.scene.classes[19] {
                self.scene.initialize_grenade(handle,self.live)
            } else if handle.class_id == self.scene.classes[18] {
                self.scene.initialize_damage_zone(handle,self.live)
            } else if handle.class_id == self.scene.classes[17] {
                self.scene.initialize_sound_volume(
                    self.world.as_deref_mut().ok_or("sound volume Initialize requires its published world")?,
                    self.live, handle,
                )
            } else if self.scene.is_rigid_class(handle.class_id) {
                self.scene
                    .initialize_placed_rigid(self.live, handle, registry, self.events)?;
                if self.scene.owner(handle)?.entity.is_none(){return Ok(());}
                self.scene
                    .finish_rigid_instigator_initialize(self.live, handle)?;
                if handle.class_id == self.scene.classes[12] {
                    self.scene.initialize_throwable_component(handle)?;
                }
                Ok(())
            } else if handle.class_id == self.scene.classes[7] {
                self.scene.initialize_loot(handle)
            } else if handle.class_id == self.scene.classes[11] {
                self.scene.initialize_breakable(handle)
            } else if handle.class_id == self.scene.classes[13] {
                self.scene.initialize_projectile(handle)
            } else if handle.class_id == self.scene.classes[14] {
                self.scene.initialize_level_bodies(handle, registry)
            } else if handle.class_id == self.scene.classes[15] {
                self.scene.initialize_animation(handle)
            } else if handle.class_id == self.scene.classes[9] {
                self.scene
                    .factions
                    .get_mut(handle.slot)
                    .ok_or("Faction component absent")?
                    .initialize(&self.scene.faction_session)
            } else {
                self.scene.initialize(handle, registry)
            }
        })();
        result.map_err(|error| self.failure("Initialize", handle, error))
    }
    fn start(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let result = (|| {
            if handle.class_id == self.scene.classes[57] {
                self.scene.start(handle, registry)?;
                return self.scene.start_fx_component(self.live, self.events, handle);
            }
            if handle.class_id == self.scene.classes[60] {
                self.scene.start(handle,registry)?;
                return self.scene.start_whale_oil_tank(handle);
            }
            if handle.class_id == self.scene.classes[55] {
                self.scene.start(handle, registry)?;
                return self.scene.start_openable(
                    self.world.as_deref_mut().ok_or("Openable Start requires published world")?,
                    self.live, self.events, registry, handle,
                );
            }
            if handle.class_id == self.scene.classes[4] {
                //708340 can create objects after an acknowledged708620 Stop
                //on this same component generation. Retain pending Stops.
                if let Some(world)=self.world.as_deref_mut() {
                    super::super::audio::resume_component_object(world,crate::gameplay::sound::Emitter::Component {
                        handle:handle.packed(),generation:self.scene.audios.get(handle.slot)
                            .ok_or("audio Start component absent")?.owner.generation,
                    })?;
                }
                self.scene.start(handle, registry)?;
                return self.scene.start_audio_events(self.live, self.events, handle);
            }
            if self.scene.is_smart_object_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                return self.scene.start_smart_object(self.world.as_deref(),self.live,handle);
            }
            if self.scene.is_flash_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                return self.scene.start_flash_component(handle);
            }
            if handle.class_id==self.scene.classes[48]||handle.class_id==self.scene.classes[49] {
                self.scene.start(handle,registry)?;
                return self.scene.start_cue_component(self.world.as_deref(),self.live,self.events,handle);
            }
            if self.scene.is_ai_startup_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                return self.scene.start_ai_startup(
                    self.world.as_deref_mut().ok_or("AI manager Start requires published world")?,
                    self.events,registry,handle,
                );
            }
            if handle.class_id==self.scene.classes[21] {
                self.scene.start(handle,registry)?;
                return self.scene.start_statistics(self.events,handle);
            }
            if handle.class_id==self.scene.classes[22] {
                self.scene.start(handle,registry)?;
                return self.scene.start_timers(self.live,handle);
            }
            if self.scene.is_crew_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                self.scene.start_crew(self.events,handle)
            } else if self.scene.is_crew_collector_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                self.scene.start_crew_collector(self.events,registry,handle)
            } else if self.scene.is_crew_module_class(handle.class_id) {
                self.scene.start(handle,registry)?;
                self.scene.start_crew_module(self.events,registry,handle)
            } else if handle.class_id == self.scene.classes[20] {
                self.scene.start(handle,registry)?;
                self.scene.start_ai_danger(handle,self.events)
            } else if handle.class_id == self.scene.classes[19] {
                self.scene.start(handle,registry)?;
                self.scene.start_grenade(self.world.as_deref_mut(),self.live,self.events,handle)
            } else if handle.class_id==self.scene.classes[59] {
                self.scene.start(handle,registry)?;
                self.scene.start_constraint(handle)
            } else if handle.class_id==self.scene.classes[58] {
                self.scene.start(handle,registry)?;
                self.scene.start_static_usable(self.live,self.events,handle)?;
                if let Some(world)=self.world.as_deref_mut(){self.scene.start_usable_backend(world,handle)?;}
                Ok(())
            } else if handle.class_id == self.scene.classes[5] {
                self.scene.start(handle, registry)?;
                self.scene.start_usable_events(self.live,self.events,handle)?;
                //Preparation has not published Interactions yet. Its initial
                //state/body queue already belongs to the prepared level.
                if let Some(world) = self.world.as_deref_mut() {
                    self.scene.start_usable_backend(world, handle)?;
                }
                Ok(())
            } else if handle.class_id==self.scene.classes[6] {
                self.scene.start_pickable_physics_references(handle)?;
                self.scene.start(handle,registry)?;
                if let Some(world)=self.world.as_deref_mut(){self.scene.heart_pickable_started(world,handle)?;}
                Ok(())
            } else if handle.class_id == self.scene.classes[18] {
                self.scene.start(handle,registry)?;
                self.scene.start_damage_zone(self.world.as_deref_mut().ok_or("DamageZone Start requires published world")?,self.live,handle)
            } else if handle.class_id == self.scene.classes[17] {
                self.scene.start(handle, registry)?;
                self.scene.sound_volumes.get_mut(handle.slot).ok_or("sound volume Start payload absent")?.started = true;
                Ok(())
            } else if self.scene.is_rigid_class(handle.class_id) {
                self.scene
                    .start_placed_rigid(self.live, handle, registry, self.events)?;
                if handle.class_id == self.scene.classes[12] {
                    self.scene
                        .start_throwable_component(handle, registry, self.events)?;
                }
                Ok(())
            } else if handle.class_id == self.scene.classes[7] {
                self.scene.start(handle,registry)?;
                self.scene.start_loot(self.live,self.events,registry,handle)
            } else if handle.class_id == self.scene.classes[11] {
                self.scene.start(handle, registry)?;
                self.scene.start_breakable(handle, self.events)
            } else if handle.class_id == self.scene.classes[13] {
                self.scene.start(handle, registry)?;
                self.scene.start_projectile(handle,self.events)
            } else if handle.class_id == self.scene.classes[15] {
                self.scene.start(handle, registry)?;
                self.scene.start_animation(handle)
            } else if handle.class_id == self.scene.classes[9] {
                let index = self
                    .scene
                    .owner(handle)?
                    .entity
                    .ok_or("Faction entity absent")?;
                let id = self.scene.entities[index].shared;
                if self.live.stage(id) != Some(crate::gameplay::entity_lifetime::Stage::Spawned) {
                    return Err("Faction Start entity is not spawned".into());
                }
                self.scene
                    .factions
                    .get(handle.slot)
                    .ok_or("Faction component absent")?
                    .preflight_start(
                        id,
                        self.scene
                            .faction_manager
                            .as_ref()
                            .ok_or("Faction manager absent")?
                            .manager(),
                    )?;
                self.scene.start(handle, registry)?;
                self.scene
                    .factions
                    .get_mut(handle.slot)
                    .ok_or("Faction component absent")?
                    .start(
                        id,
                        self.scene
                            .faction_manager
                            .as_mut()
                            .ok_or("Faction manager absent")?
                            .manager_mut(),
                        self.events,
                    )
            } else {
                self.scene.start(handle, registry)
            }
        })();
        result.map_err(|error| self.failure("Start", handle, error))
    }
    fn stop_queued(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.scene.stop_queued(handle, registry)
    }
    fn initialization_failed(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.scene.initialization_failed(handle, registry)
    }
}

impl crate::gameplay::component_state::updates::ComponentUpdateWorld for Startup<'_> {
    fn update_remove(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if self.scene.is_crew_module_class(handle.class_id){self.scene.update_remove_crew_module(handle,registry,self.live,self.events)?;}
        registry.base_update_remove(handle, self)?;
        self.scene.crew_update_subscription(handle,self.events,false)
    }
    fn update_add(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        registry.base_update_add(handle, self)?;
        self.scene.crew_update_subscription(handle,self.events,true)
    }
    fn set_update_scheduled(
        &mut self,
        handle: ComponentHandle,
        scheduled: bool,
    ) -> Result<(), String> {
        self.scene.set_component_update_scheduled(handle, scheduled)
    }
}
impl PhysicsScene {
    pub(super) fn set_component_update_scheduled(
        &mut self,
        handle: ComponentHandle,
        scheduled: bool,
    ) -> Result<(), String> {
        macro_rules! membership {
            ($pool:expr) => {
                if scheduled {
                    $pool.schedule(handle.slot)
                } else {
                    $pool.unschedule(handle.slot)
                }
            };
        }
        if handle.class_id == self.classes[0] {
            membership!(self.triggers)
        } else if handle.class_id == self.classes[1] {
            membership!(self.waters)
        } else if handle.class_id == self.classes[2] {
            membership!(self.winds)
        } else if handle.class_id == self.classes[3] {
            membership!(self.actors.actors)
        } else if handle.class_id == self.classes[4] {
            membership!(self.audios)
        } else if handle.class_id == self.classes[59] {
            membership!(self.constraints)
        } else if handle.class_id == self.classes[58] {
            membership!(self.static_usables)
        } else if handle.class_id == self.classes[5] {
            membership!(self.usables)
        } else if handle.class_id == self.classes[6] {
            membership!(self.pickables)
        } else if handle.class_id == self.classes[7] {
            membership!(self.loots)
        } else if self.is_rigid_class(handle.class_id) {
            membership!(self.rigid_pool_mut(handle.class_id)?)
        } else if handle.class_id == self.classes[9] {
            membership!(self.factions)
        } else if handle.class_id == self.classes[10] {
            membership!(self.health)
        } else if handle.class_id == self.classes[11] {
            membership!(self.breakables)
        } else if handle.class_id == self.classes[13] {
            membership!(self.projectiles)
        } else if handle.class_id == self.classes[14] {
            membership!(self.level_bodies)
        } else if handle.class_id == self.classes[15] {
            membership!(self.animations)
        } else if handle.class_id == self.classes[16] {
            membership!(self.health_doors)
        } else if handle.class_id == self.classes[17] {
            membership!(self.sound_volumes)
        } else if handle.class_id == self.classes[19] {
            membership!(self.grenades)
        } else if handle.class_id == self.classes[20] {
            membership!(self.ai_danger_zones)
        } else if handle.class_id == self.classes[21] {
            membership!(self.statistics)
        } else if handle.class_id == self.classes[22] {
            membership!(self.timers)
        } else if handle.class_id == self.classes[23] {
            membership!(self.ai_cues)
        } else if handle.class_id == self.classes[24] {
            membership!(self.ai_crews)
        } else if handle.class_id == self.classes[25] {
            membership!(self.ai_dsm)
        } else if handle.class_id == self.classes[26] {
            membership!(self.ai_flee)
        } else if handle.class_id == self.classes[27] {
            membership!(self.ai_coordination)
        } else if handle.class_id == self.classes[28] {
            membership!(self.ai_vision)
        } else if handle.class_id==self.classes[29] {
            membership!(self.crew_cores)
        } else if handle.class_id==self.classes[30] {
            membership!(self.crew_knowledge)
        } else if handle.class_id==self.classes[31] {
            membership!(self.crew_coordination)
        } else if handle.class_id==self.classes[32] {
            membership!(self.crew_decisions)
        } else if handle.class_id==self.classes[33] {
            membership!(self.ai_crew_area)
        } else if handle.class_id==self.classes[34] {
            membership!(self.ai_crew_combat)
        } else if handle.class_id==self.classes[35] {
            membership!(self.ai_crew_notification)
        } else if handle.class_id==self.classes[36] {
            membership!(self.ai_crew_search)
        } else if handle.class_id==self.classes[37] {
            membership!(self.ai_crew_herd)
        } else if handle.class_id==self.classes[38] {
            membership!(self.crew_chase)
        } else if handle.class_id==self.classes[39] {
            membership!(self.crew_melee)
        } else if handle.class_id==self.classes[40] {
            membership!(self.crew_search_module)
        } else if handle.class_id==self.classes[41] {
            membership!(self.crew_link)
        } else if handle.class_id==self.classes[42] {
            membership!(self.crew_ranged)
        } else if handle.class_id==self.classes[43] {
            membership!(self.crew_fake)
        } else if handle.class_id==self.classes[44] {
            membership!(self.crew_flank)
        } else if handle.class_id==self.classes[45] {
            membership!(self.crew_hideout)
        } else if handle.class_id==self.classes[46] {
            membership!(self.crew_tether)
        } else if handle.class_id==self.classes[47] {
            membership!(self.crew_alarm)
        } else if handle.class_id==self.classes[48] {
            membership!(self.observables)
        } else if handle.class_id==self.classes[49] {
            membership!(self.attention_sources)
        } else if handle.class_id==self.classes[50] {
            membership!(self.investigate_spots)
        } else if handle.class_id==self.classes[51] {
            membership!(self.dynamic_search_spots)
        } else if handle.class_id==self.classes[52] {
            membership!(self.flash_surfaces)
        } else if handle.class_id==self.classes[53] {
            membership!(self.interactive_maps)
        } else if handle.class_id==self.classes[54] {
            membership!(self.cooking_surfaces)
        } else if handle.class_id==self.classes[55] {
            membership!(self.openables)
        } else if handle.class_id==self.classes[56] {
            membership!(self.retargeters)
        } else if handle.class_id==self.classes[57] {
            membership!(self.fx_components)
        } else if handle.class_id==self.classes[60] {
            membership!(self.whale_oil_tanks)
        } else if handle.class_id == self.classes[18] {
            membership!(self.damage_zones)
        } else {
            Err("component update class unsupported".into())
        }
    }
}

impl PhysicsScene {
    /// Drain the shared source component registry for dynamically admitted actors.
    /// The caller retains the registry cursor on failure; no second startup queue.
    pub(in crate::app) fn run_runtime_startup(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut crate::gameplay::entity_events::EntityEvents,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let mut host = Startup { world: Some(world), scene: self, live, events };
        registry.initialize_and_start(&mut host)?;
        registry.drain_update_changes(&mut host)?;
        if !host.scene.finish_constraint_admission(registry, host.events)? {
            return Err("constraint startup awaits actual endpoint body publication".into());
        }
        Ok(())
    }
    pub(in crate::app) fn run_runtime_startup_with_registry(
        &mut self,
        live: &mut LiveEntities,
        events: &mut crate::gameplay::entity_events::EntityEvents,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let mut host = Startup {
            world: None,
            scene: self,
            live,
            events,
        };
        registry.initialize_and_start(&mut host)?;
        registry.drain_update_changes(&mut host)?;
        if !host.scene.finish_constraint_admission(registry, host.events)? {
            return Err("constraint startup awaits actual endpoint body publication".into());
        }
        Ok(())
    }
    fn initialize_level_bodies(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        scene_publication_events!(self, registry, events);
        self.level_bodies
            .get_mut(handle.slot)
            .ok_or("PhysicsWorld component absent")?
            .initialize(handle, &mut world.storage, &mut world.geometry, &mut events)
    }
}
