//! Actual six AIWorld manager pools and source lifecycle hooks.
use super::*;
use crate::{
    assets::ai_world::startup_managers::Definition,
    gameplay::{
        ai_world::{Slot, Worlds, startup_managers as payload},
        entity_events::EntityEvents,
        entity_lifetime::{EntityId, LiveEntities},
        pickable_component::CheckedComponent,
    },
};
use bevy::prelude::World;
impl PhysicsScene {
    fn initialize_dsm_sources(
        &mut self,
        world: &mut World,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        use payload::NavigationInstances;
        if self
            .ai_dsm
            .get(handle.slot)
            .ok_or("DSM payload absent")?
            .initialized
        {
            return Ok(());
        }
        let (id, checked) = self.ai_startup_identity(handle)?;
        let area = live.ai_world_index(id)?;
        let area =
            u16::try_from(area).map_err(|_| "DSM AIWorld area exceeds original navmesh ordinal")?;
        let graph = world
            .get_resource::<NavigationInstances>()
            .ok_or("DSM Initialize requires actual admitted navigation resource")?
            .graph(area)?;
        let count = graph.cells.len();
        let mut access = std::collections::HashMap::with_capacity(count);
        let mut areas = Vec::with_capacity(count);
        for (index, cell) in graph.cells.iter().enumerate() {
            //5bc4d0 overwrites duplicate quantized keys with the later index.
            access.insert(cell.key, index as u16);
            if let Some(reference) = graph.area_volume(cell)? {
                return Err(format!(
                    "DSM serialized non-null AreaVolume pointer needs its actual reflected reference decoder: {reference}"
                ));
            }
            areas.push(None);
        }
        let mut static_values = vec![255; count];
        let maximum_z = crate::platform::console::shared()
            .read()
            .cvars
            .get("ark_ai_dsm_flee_neighbor_max_z")
            .ok_or("DSM original neighbor Z cvar absent")?
            .float;
        let height = |index: usize| {
            let cell = &graph.cells[index];
            cell.key[2] as f32 * 2. + graph.grid_origin[2] + cell.offset
        };
        //5bdb30's default8-neighbor static influence initialization.
        for (index, value) in static_values.iter_mut().enumerate() {
            let mut degree = 0;
            let mut flat = true;
            for neighbor in graph.neighbors(index)? {
                degree += 1;
                flat &= !(maximum_z <= (height(index) - height(neighbor as usize)).abs());
            }
            if degree < 8 && flat {
                *value = ((1. - degree as f32 / 8.) * 254.) as u8;
            }
        }
        let parent = self
            .owner(handle)?
            .entity
            .ok_or("DSM parent header absent")?;
        let domain = self.entities[parent].timespace;
        let group = *[3usize, 4, 2]
            .get(domain as usize)
            .ok_or("DSM entity clock domain is outside original enum")?;
        let now = world
            .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("DSM Initialize clock owner absent")?
            .clock()
            .map_err(str::to_owned)?
            .time(group)
            .ok_or("DSM actual clock group absent")?;
        let component = self.ai_dsm.get_mut(handle.slot).unwrap();
        component.cell_access = access;
        component.areas = areas;
        if count > 0 {
            component.valid = vec![u32::MAX; count.div_ceil(32)];
            //5bab80's fresh branch finishes with5b9a80: source cells with
            //invalid nav-faceFFFFFFFF are not valid influence recipients.
            for (index,cell) in graph.cells.iter().enumerate(){
                if cell.nav_face_id==u32::MAX{component.valid[index>>5]&=!(1u32<<(index&31));}
            }
            component.touched = vec![0; count.div_ceil(32)];
            component.search_values = vec![127; count];
            component.static_values = static_values;
            component.chase_values = vec![255; count];
            component.sensor_values = std::array::from_fn(|_| vec![127; count]);
            component.last_search_time = now;
            component.last_regeneration_time = now;
            component.sensor_times = [now; 8];
            if area == 0 {
                world
                    .resource_mut::<NavigationInstances>()
                    .register(checked)?;
                component.navigation_listener = true;
            }
        }
        component.graph = Some(graph);
        component.initialized = true;
        Ok(())
    }
    fn start_dsm_sources(
        &mut self,
        _world: &mut World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let graph = self
            .ai_dsm
            .get(handle.slot)
            .ok_or("DSM payload absent")?
            .graph
            .clone()
            .ok_or("DSM Start precedes graph Initialize")?;
        let mut areas = std::mem::take(&mut self.ai_dsm.get_mut(handle.slot).unwrap().areas);
        let mut hits = Vec::new();
        let result: Result<(), String> = (|| {
            for (index, cell) in graph.cells.iter().enumerate() {
                let position = [
                    cell.key[0] as f32 + graph.grid_origin[0] + 0.5,
                    cell.key[1] as f32 + graph.grid_origin[1] + 0.5,
                    cell.key[2] as f32 * 2. + graph.grid_origin[2] + cell.offset,
                ];
                {
                    let owner = self
                        .world
                        .read()
                        .map_err(|_| "DSM collision query owner poisoned")?;
                    owner.point_hits(
                        &mut self.executor,
                        &WorldPointRequest::environment(position),
                        &mut hits,
                    )?;
                }
                for hit in &hits {
                    let user = crate::gameplay::collision_user_data::CollisionUserData::decode(
                        hit.target.user_data,
                    )?;
                    let entity = user.entity(|component| self.owner(component).ok())?;
                    let class = self
                        .types
                        .classes()
                        .hierarchy()
                        .type_id(self.entities[entity].class)?;
                    if (self.area_range[0]..=self.area_range[1]).contains(&class) {
                        areas[index] = Some(self.entities[entity].shared);
                        break;
                    }
                }
            }
            Ok(())
        })();
        let component = self.ai_dsm.get_mut(handle.slot).unwrap();
        component.areas = areas;
        result?;
        component.started = true;
        Ok(())
    }
    fn deinitialize_dsm_sources(
        &mut self,
        world: &mut World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (_, checked) = self.ai_startup_identity(handle)?;
        let component = self
            .ai_dsm
            .get_mut(handle.slot)
            .ok_or("DSM payload absent")?;
        if component.started {
            return Err("DSM Deinitialize precedes Stop".into());
        }
        if component.navigation_listener {
            world
                .resource_mut::<payload::NavigationInstances>()
                .unregister(checked)?;
            component.navigation_listener = false;
        }
        component.graph = None;
        component.clear_spot_influence();
        component.cell_access = Default::default();
        component.valid = Vec::new();
        component.touched = Vec::new();
        component.areas = Vec::new();
        component.search_values = Vec::new();
        component.static_values = Vec::new();
        component.chase_values = Vec::new();
        component.sensor_values = std::array::from_fn(|_| Vec::new());
        component.initialized = false;
        Ok(())
    }
    pub(super) fn dispatch_ai_startup(
        &mut self,
        live: &LiveEntities,
        _registry: &mut ComponentRegistry,
        callback: crate::gameplay::entity_events::Listener,
        event: &crate::gameplay::entity_events::Event,
        commands: &mut crate::gameplay::entity_events::Commands,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
        worlds: Option<&Worlds>,
    ) -> Result<(), String> {
        use crate::gameplay::entity_events::{Listener, Payload};
        let Listener::AIWorldStartup {
            world: id,
            handle,
            generation,
            handler,
        } = callback
        else {
            return Err("AI startup dispatcher received a different callback owner".into());
        };
        let (parent, checked) = self.ai_startup_identity(handle)?;
        if parent != id || checked.generation != generation || live.get(id).is_none() {
            return Err("AI startup callback generation is stale".into());
        }
        if event.id != handler {
            return Err("AI startup callback ID differs from registered handler".into());
        }
        if handle.class_id == self.classes[23] {
            return self.dispatch_ai_cues_event(live, commands, handle, event, clock, worlds);
        }
        if handle.class_id == self.classes[26] {
            if handler != 0x6b {
                return Err("Flee callback is not source6B".into());
            }
            let Some(source) = event.target.filter(|source| live.get(*source).is_some()) else {
                return Ok(());
            };
            if live.ai_world_index(source)? != live.ai_world_index(id)? {
                return Ok(());
            }
            let Payload::Bytes(bytes) = &event.payload else {
                return Err("Flee6B requires actual source position payload".into());
            };
            if bytes.len() < 12 {
                return Err("Flee6B source position payload is truncated".into());
            }
            let position = std::array::from_fn(|axis| {
                f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())
            });
            let time = clock
                .ok_or("Flee6B clock owner absent")?
                .clock()
                .map_err(str::to_owned)?
                .time(3)
                .ok_or("Flee6B actual global clock group absent")?;
            let component = self.ai_flee.get_mut(handle.slot).unwrap();
            if !component.initialized {
                return Err("Flee6B has no initialized receiver".into());
            }
            component.events.push(payload::FleeEvent {
                entity: source,
                position,
                time,
                time_domain: 0,
                valid: true,
            });
            return Ok(());
        }
        if handle.class_id == self.classes[28] {
            if event.target != Some(id) {
                return Err("Vision callback target differs from its real AIWorld".into());
            }
            let Payload::AIObservable {
                entity,
                component,
                category,
            } = &event.payload
            else {
                return Err("Vision callback requires actual Observable reference".into());
            };
            if live.get(*entity).is_none() {
                return Err("Vision callback Observable parent generation is stale".into());
            }
            if self.owner(component.handle)?.generation != component.generation {
                return Err("Vision callback Observable component generation is stale".into());
            }
            let vision = self.ai_vision.get_mut(handle.slot).unwrap();
            if !vision.initialized {
                return Err("Vision callback has no initialized receiver".into());
            }
            let roster = vision
                .rosters
                .get_mut(*category as usize)
                .ok_or("Vision callback Observable category is outside original eight rosters")?;
            let reference = payload::ObservableRef {
                entity: *entity,
                component: *component,
            };
            match handler {
                0x1be => roster.push(reference),
                0x1bf => {
                    if let Some(index) = roster.iter().position(|entry| *entry == reference) {
                        roster.remove(index);
                    }
                }
                _ => return Err("Vision callback is not source1BE/1BF".into()),
            }
            return Ok(());
        }
        Err("AI startup class has no source callback for this message".into())
    }
    pub(super) fn is_ai_startup_class(&self, class: i32) -> bool {
        self.classes[23..29].contains(&class)
    }
    pub(super) fn ai_startup_owner(&self, handle: ComponentHandle) -> Option<&ComponentOwner> {
        if handle.class_id == self.classes[23] {
            return self.ai_cues.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[24] {
            return self.ai_crews.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[25] {
            return self.ai_dsm.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[26] {
            return self.ai_flee.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[27] {
            return self.ai_coordination.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[28] {
            return self.ai_vision.get(handle.slot).map(|v| &v.owner);
        }
        None
    }
    pub(super) fn ai_startup_owner_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Option<&mut ComponentOwner> {
        if handle.class_id == self.classes[23] {
            return self.ai_cues.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[24] {
            return self.ai_crews.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[25] {
            return self.ai_dsm.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[26] {
            return self.ai_flee.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[27] {
            return self
                .ai_coordination
                .get_mut(handle.slot)
                .map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[28] {
            return self.ai_vision.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        None
    }
    pub(super) fn schedule_ai_startup(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id == self.classes[23] {
            return self.ai_cues.schedule(handle.slot);
        }
        if handle.class_id == self.classes[24] {
            return self.ai_crews.schedule(handle.slot);
        }
        if handle.class_id == self.classes[25] {
            return self.ai_dsm.schedule(handle.slot);
        }
        if handle.class_id == self.classes[26] {
            return self.ai_flee.schedule(handle.slot);
        }
        if handle.class_id == self.classes[27] {
            return self.ai_coordination.schedule(handle.slot);
        }
        if handle.class_id == self.classes[28] {
            return self.ai_vision.schedule(handle.slot);
        }
        Err("unknown AI startup class".into())
    }
    pub(super) fn create_ai_startup(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        match &*definition {
            Definition::Cues(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_cues,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::Cues::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Crew(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_crews,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::CrewDispatch::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Dsm(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_dsm,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::Dsm::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Flee(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_flee,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::Flee::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Coordination(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_coordination,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::Coordination::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Vision(_) => ComponentFactory {
                declaration,
                pool: &mut self.ai_vision,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| payload::Vision::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
        }
    }
    pub(super) fn release_ai_startup_binding(
        &mut self,
        index: usize,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id == self.classes[23] {
            return self.entities[index].components.release(
                handle,
                self.ai_cues
                    .get_mut(handle.slot)
                    .ok_or("Cues pool payload absent")?,
            );
        }
        if handle.class_id == self.classes[24] {
            return self.entities[index].components.release(
                handle,
                self.ai_crews
                    .get_mut(handle.slot)
                    .ok_or("Crew pool payload absent")?,
            );
        }
        if handle.class_id == self.classes[25] {
            return self.entities[index].components.release(
                handle,
                self.ai_dsm
                    .get_mut(handle.slot)
                    .ok_or("Dsm pool payload absent")?,
            );
        }
        if handle.class_id == self.classes[26] {
            return self.entities[index].components.release(
                handle,
                self.ai_flee
                    .get_mut(handle.slot)
                    .ok_or("Flee pool payload absent")?,
            );
        }
        if handle.class_id == self.classes[27] {
            return self.entities[index].components.release(
                handle,
                self.ai_coordination
                    .get_mut(handle.slot)
                    .ok_or("Coordination pool payload absent")?,
            );
        }
        if handle.class_id == self.classes[28] {
            return self.entities[index].components.release(
                handle,
                self.ai_vision
                    .get_mut(handle.slot)
                    .ok_or("Vision pool payload absent")?,
            );
        }
        Err("unknown AI startup release class".into())
    }
    pub(super) fn reclaim_ai_startup(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id == self.classes[23] {
            self.ai_cues.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[24] {
            self.ai_crews.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[25] {
            self.ai_dsm.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[26] {
            self.ai_flee.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[27] {
            self.ai_coordination.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[28] {
            self.ai_vision.release(handle.slot)?;
            return Ok(());
        }
        Err("unknown AI startup reclaim class".into())
    }
    fn ai_startup_identity(
        &self,
        handle: ComponentHandle,
    ) -> Result<(EntityId, CheckedComponent), String> {
        let owner = self
            .ai_startup_owner(handle)
            .ok_or("AI startup component absent")?;
        let parent = owner
            .entity
            .and_then(|index| self.entities.get(index))
            .ok_or("AI startup parent absent")?
            .shared;
        Ok((
            parent,
            CheckedComponent {
                handle,
                generation: owner.generation,
            },
        ))
    }
    pub(in crate::app) fn coordination(
        &self,
        id: EntityId,
        checked: CheckedComponent,
    ) -> Result<&payload::Coordination, String> {
        let (parent, actual) = self.ai_startup_identity(checked.handle)?;
        if parent != id
            || actual.generation != checked.generation
            || checked.handle.class_id != self.classes[27]
        {
            return Err("AI coordination callback generation differs".into());
        }
        self.ai_coordination
            .get(checked.handle.slot)
            .filter(|component| component.initialized)
            .ok_or_else(|| "AI coordination is not initialized".into())
    }
    pub(in crate::app) fn coordination_mut(
        &mut self,
        id: EntityId,
        checked: CheckedComponent,
    ) -> Result<&mut payload::Coordination, String> {
        let (parent, actual) = self.ai_startup_identity(checked.handle)?;
        if parent != id
            || actual.generation != checked.generation
            || checked.handle.class_id != self.classes[27]
        {
            return Err("AI coordination callback generation differs".into());
        }
        let component = self
            .ai_coordination
            .get_mut(checked.handle.slot)
            .ok_or("AI coordination payload absent")?;
        if !component.initialized {
            return Err("AI coordination is not initialized".into());
        }
        Ok(component)
    }
}

fn listener(
    id: EntityId,
    checked: CheckedComponent,
    handler: u16,
) -> crate::gameplay::entity_events::Listener {
    crate::gameplay::entity_events::Listener::AIWorldStartup {
        world: id,
        handle: checked.handle,
        generation: checked.generation,
        handler,
    }
}
impl PhysicsScene {
    pub(super) fn initialize_ai_startup(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.ai_startup_identity(handle)?;
        live.get(id)
            .ok_or("AI startup parent generation is stale")?;
        let area = live.ai_world_index(id)?;
        if handle.class_id == self.classes[23] {
            if self.ai_cues.get(handle.slot).unwrap().initialized {
                return Ok(());
            }
            let definition = self.ai_cues.get(handle.slot).unwrap().definition.clone();
            let Definition::Cues(definition) = &*definition else {
                unreachable!()
            };
            for (category, pool) in definition.pools.iter().enumerate() {
                for index in 0..pool.maximum {
                    if index < self.ai_cues.get(handle.slot).unwrap().pools[category].configured {
                        continue;
                    }
                    let child = if let Some(child) = self.ai_cues.get(handle.slot).unwrap().pools
                        [category]
                        .children
                        .get(index)
                        .copied()
                    {
                        child
                    } else {
                        let source = pool
                            .entity
                            .as_deref()
                            .ok_or("positive cue count has no actual child source")?;
                        let child =
                            super::super::runtime_entities::prepared_child(world, id, source)?;
                        let child = super::super::runtime_entities::spawn_in_registry(
                            world,
                            self,
                            live,
                            events,
                            registry,
                            child,
                            super::super::runtime_entities::SpawnOptions {
                                frame: crate::gameplay::reference_frame::ReferenceFrame::default(),
                                scale: None,
                                hidden_disabled: true,
                            },
                            Some(
                                format!(
                                    "Cue_{}_{}_{}",
                                    crate::assets::ai_world::startup_managers::CUE_NAMES[category],
                                    area,
                                    index
                                )
                                .into(),
                            ),
                            id,
                        )?;
                        self.ai_cues.get_mut(handle.slot).unwrap().pools[category]
                            .children
                            .push(child);
                        child
                    };
                    live.set_ai_world_index(child, area)?;
                    super::super::runtime_entities::set_ai_cue_category(
                        world,
                        live,
                        child,
                        category as i32,
                    )?;
                    self.ai_cues.get_mut(handle.slot).unwrap().pools[category].configured =
                        index + 1;
                }
            }
            let component = self.ai_cues.get_mut(handle.slot).unwrap();
            component.validate_children()?;
            component.initialized = true;
            world
                .resource_mut::<Worlds>()
                .bind_component(id, Slot::Cues, checked)?;
        } else if handle.class_id == self.classes[24] {
            if self.ai_crews.get(handle.slot).unwrap().initialized {
                return Ok(());
            }
            let definition = self.ai_crews.get(handle.slot).unwrap().definition.clone();
            let Definition::Crew(definition) = &*definition else {
                unreachable!()
            };
            let child =
                super::super::runtime_entities::prepared_child(world, id, &definition.entity)?;
            let frame = self.entities[self.owner(handle)?.entity.ok_or("Crew parent absent")?]
                .frame
                .pose;
            for ordinal in 0..8 {
                if ordinal < self.ai_crews.get(handle.slot).unwrap().configured {
                    continue;
                }
                let child = if let Some(child) = self
                    .ai_crews
                    .get(handle.slot)
                    .unwrap()
                    .children
                    .get(ordinal)
                    .copied()
                {
                    child
                } else {
                    let child = super::super::runtime_entities::spawn_in_registry(
                        world,
                        self,
                        live,
                        events,
                        registry,
                        child.clone(),
                        super::super::runtime_entities::SpawnOptions {
                            frame,
                            scale: None,
                            hidden_disabled: false,
                        },
                        None,
                        id,
                    )?;
                    self.ai_crews
                        .get_mut(handle.slot)
                        .unwrap()
                        .children
                        .push(child);
                    child
                };
                live.set_ai_world_index(child, area)?;
                let index = *self
                    .shared_indices
                    .get(&child)
                    .ok_or("Crew child managed header absent")?;
                self.entity_flags_d4[index] |= 4;
                self.ai_crews.get_mut(handle.slot).unwrap().configured = ordinal + 1;
            }
            self.ai_crews.get_mut(handle.slot).unwrap().initialized = true;
        } else if handle.class_id == self.classes[25] {
            self.initialize_dsm_sources(world, live, handle)?;
            world
                .resource_mut::<Worlds>()
                .bind_component(id, Slot::Dsm, checked)?;
        } else if handle.class_id == self.classes[26] {
            let component = self.ai_flee.get_mut(handle.slot).unwrap();
            if component.initialized {
                return Ok(());
            }
            events.register(0x6b, None, listener(id, checked, 0x6b));
            component.initialized = true;
        } else if handle.class_id == self.classes[27] {
            if self.ai_coordination.get(handle.slot).unwrap().initialized {
                return Ok(());
            }
            world
                .resource_mut::<Worlds>()
                .bind_component(id, Slot::Coordination, checked)?;
            self.ai_coordination
                .get_mut(handle.slot)
                .unwrap()
                .initialized = true;
        } else if handle.class_id == self.classes[28] {
            if self.ai_vision.get(handle.slot).unwrap().initialized {
                return Ok(());
            }
            world
                .resource_mut::<Worlds>()
                .bind_component(id, Slot::Vision, checked)?;
            for event in [0x1be, 0x1bf] {
                events.register(event, Some(id), listener(id, checked, event));
            }
            self.ai_vision.get_mut(handle.slot).unwrap().initialized = true;
        } else {
            return Err("unknown AI startup Initialize class".into());
        }
        self.owner_mut(handle)?.mark_initialized();
        Ok(())
    }
    pub(super) fn start_ai_startup(
        &mut self,
        world: &mut World,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.ai_startup_identity(handle)?;
        if handle.class_id == self.classes[23] {
            let component = self.ai_cues.get_mut(handle.slot).unwrap();
            if !component.initialized {
                return Err("Cues Start precedes Initialize".into());
            }
            if !component.started {
                for event in [0x73, 0x74, 0x75, 0x76] {
                    events.register(event, None, listener(id, checked, event));
                }
                component.started = true;
            }
        } else if handle.class_id == self.classes[24] {
            let component = self.ai_crews.get_mut(handle.slot).unwrap();
            if !component.initialized {
                return Err("Crew Start precedes Initialize".into());
            }
            component.requester = Some(id);
            component.request_next = -1;
            component.request_active = true;
            component.started = true;
        } else if handle.class_id == self.classes[25] {
            self.start_dsm_sources(world, handle)?;
        } else if handle.class_id == self.classes[26] {
            let worlds = world
                .get_resource::<Worlds>()
                .ok_or("Flee Start has no canonical AIWorld owner")?;
            let dsm = worlds
                .component(id, Slot::Dsm)?
                .ok_or("Flee Start requires initialized DSM")?;
            let component = self
                .ai_dsm
                .get(dsm.handle.slot)
                .ok_or("Flee Start DSM payload absent")?;
            if component.owner.generation != dsm.generation || !component.initialized {
                return Err("Flee Start DSM generation is stale".into());
            }
            let empty = component
                .graph
                .as_ref()
                .ok_or("Flee Start DSM has no admitted graph")?
                .cells
                .is_empty();
            self.ai_flee.get_mut(handle.slot).unwrap().started = true;
            if empty {
                registry.pause_updates(handle, self)?;
            }
        } else if handle.class_id == self.classes[27] {
            self.ai_coordination.get_mut(handle.slot).unwrap().started = true;
        } else if handle.class_id == self.classes[28] {
            self.ai_vision.get_mut(handle.slot).unwrap().started = true;
        } else {
            return Err("unknown AI startup Start class".into());
        }
        Ok(())
    }
    pub(super) fn stop_ai_startup(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.ai_startup_identity(handle)?;
        if handle.class_id == self.classes[23] {
            let component = self.ai_cues.get_mut(handle.slot).unwrap();
            if component.started {
                for event in [0x76, 0x73, 0x74, 0x75] {
                    events.unregister(event, None, listener(id, checked, event));
                }
                component.started = false;
            }
        } else if handle.class_id == self.classes[24] {
            let c = self.ai_crews.get_mut(handle.slot).unwrap();
            c.requester = None;
            c.request_next = -1;
            c.request_active = false;
            c.started = false;
        } else if handle.class_id == self.classes[25] {
            self.ai_dsm.get_mut(handle.slot).unwrap().started = false;
        } else if handle.class_id == self.classes[26] {
            let c = self.ai_flee.get_mut(handle.slot).unwrap();
            c.events = Vec::new();
            c.started = false;
        } else if handle.class_id == self.classes[27] {
            let c = self.ai_coordination.get_mut(handle.slot).unwrap();
            c.reset();
            c.started = false;
        } else if handle.class_id == self.classes[28] {
            self.ai_vision.get_mut(handle.slot).unwrap().started = false;
        } else {
            return Err("unknown AI startup Stop class".into());
        }
        Ok(())
    }
    pub(super) fn deinitialize_ai_startup(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.ai_startup_identity(handle)?;
        if handle.class_id == self.classes[23] {
            if self.ai_cues.get(handle.slot).unwrap().started {
                return Err("Cues Deinitialize precedes Stop".into());
            }
            self.deinitialize_ai_cue_children(world, live, events, handle)?;
            for pool in &mut self.ai_cues.get_mut(handle.slot).unwrap().pools {
                pool.configured = 0;
            }
            world
                .resource_mut::<Worlds>()
                .unbind_component(id, Slot::Cues, checked)?;
            self.ai_cues.get_mut(handle.slot).unwrap().initialized = false;
        } else if handle.class_id == self.classes[24] {
            let component = self.ai_crews.get_mut(handle.slot).unwrap();
            if component.started {
                return Err("Crew Deinitialize precedes Stop".into());
            }
            while component.retire_cursor < component.children.len() {
                let child = component.children[component.retire_cursor];
                if live.get(child).is_some() {
                    super::super::entity_functions::enqueue_removal_with_live(world, live, child, 0.)?;
                }
                component.retire_cursor += 1;
            }
            component.children.clear();
            component.configured = 0;
            component.retire_cursor = 0;
            component.initialized = false;
        } else if handle.class_id == self.classes[25] {
            self.deinitialize_dsm_sources(world, handle)?;
            world
                .resource_mut::<Worlds>()
                .unbind_component(id, Slot::Dsm, checked)?;
        } else if handle.class_id == self.classes[26] {
            let component = self.ai_flee.get_mut(handle.slot).unwrap();
            events.unregister(0x6b, None, listener(id, checked, 0x6b));
            component.initialized = false;
        } else if handle.class_id == self.classes[27] {
            world
                .resource_mut::<Worlds>()
                .unbind_component(id, Slot::Coordination, checked)?;
            self.ai_coordination
                .get_mut(handle.slot)
                .unwrap()
                .initialized = false;
        } else if handle.class_id == self.classes[28] {
            for event in [0x1be, 0x1bf] {
                events.unregister(event, Some(id), listener(id, checked, event));
            }
            world
                .resource_mut::<Worlds>()
                .unbind_component(id, Slot::Vision, checked)?;
            self.ai_vision.get_mut(handle.slot).unwrap().initialized = false;
        } else {
            return Err("unknown AI startup Deinitialize class".into());
        }
        Ok(())
    }
}
