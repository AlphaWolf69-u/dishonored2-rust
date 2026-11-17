//! Concrete Investigate934840 and DynamicSearch5af530, with their shared
//! AISmartObject5b5450/5b5750 lifecycle and canonical AIWorld dependencies.
use super::*;
use crate::{
    assets::ai_smart_objects::Definition,
    gameplay::{
        ai_smart_objects::{Component, Slot as RuntimeSlot},
        ai_world::{Slot, Worlds},
        entity_events::{EntityEvents, Event, Listener},
        entity_lifetime::{EntityId, LiveEntities},
        pickable_component::CheckedComponent,
    },
};
use bevy::prelude::World;

impl PhysicsScene {
    pub(super) fn is_smart_object_class(&self, class: i32) -> bool {
        self.classes[50] == class || self.classes[51] == class
    }
    pub(super) fn smart_object(&self, h: ComponentHandle) -> Result<&Component, String> {
        if h.class_id == self.classes[50] {
            self.investigate_spots.get(h.slot)
        } else if h.class_id == self.classes[51] {
            self.dynamic_search_spots.get(h.slot)
        } else {
            None
        }
        .ok_or_else(|| "AISmartObject concrete payload absent".into())
    }
    pub(super) fn smart_object_mut(
        &mut self,
        h: ComponentHandle,
    ) -> Result<&mut Component, String> {
        if h.class_id == self.classes[50] {
            self.investigate_spots.get_mut(h.slot)
        } else if h.class_id == self.classes[51] {
            self.dynamic_search_spots.get_mut(h.slot)
        } else {
            None
        }
        .ok_or_else(|| "AISmartObject concrete payload absent".into())
    }
    fn smart_identity(
        &self,
        h: ComponentHandle,
    ) -> Result<(usize, EntityId, CheckedComponent), String> {
        let owner = &self.smart_object(h)?.owner;
        let index = owner.entity.ok_or("AISmartObject entity absent")?;
        Ok((
            index,
            self.entities[index].shared,
            CheckedComponent {
                handle: h,
                generation: owner.generation,
            },
        ))
    }
    fn smart_listener(&self, h: ComponentHandle) -> Result<Listener, String> {
        Ok(Listener::Component {
            owner: self.owner_id,
            handle: h,
            generation: self.smart_object(h)?.owner.generation,
            handler: 0x66,
        })
    }
    pub(super) fn create_smart_object(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        let pool = match &*definition {
            Definition::Investigate { .. } => &mut self.investigate_spots,
            Definition::SearchDynamic { .. } => &mut self.dynamic_search_spots,
        };
        ComponentFactory {
            declaration,
            pool,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| Ok(Component::new(definition.clone())),
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn initialize_smart_object(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let (_, id, _) = self.smart_identity(h)?;
        if live.get(id).is_none() {
            return Err("AISmartObject Initialize entity is stale".into());
        }
        if self.smart_object(h)?.initialized {
            return Ok(());
        }
        let definition = self.smart_object(h)?.definition.clone();
        let mut slots = Vec::with_capacity(definition.common().slots.len());
        for slot in &definition.common().slots {
            if slot.maximum_users < -1 {
                return Err("AISmartObject m_maxUsers is below source sentinel".into());
            }
            slots.push(RuntimeSlot {
                users: if slot.maximum_users == -1 {
                    Vec::new()
                } else {
                    Vec::with_capacity(slot.maximum_users as usize)
                },
                position_override: None,
                face_to: slot
                    .face_to_entity
                    .as_deref()
                    .and_then(|name| live.find(name)),
            });
        }
        let fallback_slot = definition
            .common()
            .slots
            .iter()
            .position(|slot| slot.fallback);
        let listener = self.smart_listener(h)?;
        events.register(0x66, Some(id), listener);
        let component = self.smart_object_mut(h)?;
        component.slots = slots;
        component.fallback_slot = fallback_slot;
        component.initialized = true;
        component.owner.mark_initialized(); //5b5450 first invokes base7e7400.
        Ok(())
    }
    ///5b4660: position overrides are in the same authored entity space as
    ///the slot offset. Rotation is optional; owner translation always applies.
    fn smart_slot_position(&self, h: ComponentHandle, slot: usize) -> Result<[f32; 3], String> {
        let (index, _, _) = self.smart_identity(h)?;
        let component = self.smart_object(h)?;
        let offset = component
            .slots
            .get(slot)
            .ok_or("AISmartObject slot is absent")?
            .position_override
            .unwrap_or(component.definition.common().slots[slot].offset);
        let frame = &self.entities[index].frame;
        let mut position = frame.pose.position;
        if component.definition.common().offset_in_entity_space {
            for axis in 0..3 {
                for j in 0..3 {
                    position[j] += offset[axis] * frame.pose.basis[axis][j];
                }
            }
        } else {
            for j in 0..3 {
                position[j] += offset[j];
            }
        }
        Ok(position)
    }
    fn smart_world_dependencies(
        &self,
        world: &World,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<(EntityId, CheckedComponent, CheckedComponent), String> {
        let worlds = world
            .get_resource::<Worlds>()
            .ok_or("SearchSpot has no canonical AIWorld owner")?;
        let recipient = worlds.recipient(live, id)?;
        let coordination = worlds
            .component(recipient, Slot::Coordination)?
            .ok_or("SearchSpot Coordination dependency absent")?;
        let dsm = worlds
            .component(recipient, Slot::Dsm)?
            .ok_or("SearchSpot DSM dependency absent")?;
        let actual = self.owner(dsm.handle)?;
        if dsm.handle.class_id != self.classes[25]
            || actual.generation != dsm.generation
            || actual.entity.map(|i| self.entities[i].shared) != Some(recipient)
        {
            return Err("SearchSpot DSM dependency generation differs".into());
        }
        Ok((recipient, coordination, dsm))
    }
    pub(super) fn start_smart_object(
        &mut self,
        world: Option<&World>,
        live: &LiveEntities,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let (index, id, checked) = self.smart_identity(h)?;
        if self.smart_object(h)?.started {
            return Ok(());
        }
        if h.class_id == self.classes[51] {
            let world = world.ok_or("SearchSpot Start requires published world")?;
            let (recipient, coordination, dsm) = self.smart_world_dependencies(world, live, id)?;
            if !self.smart_object(h)?.registered {
                self.coordination_mut(recipient, coordination)?
                    .add_search_spot(checked);
                self.smart_object_mut(h)?.registered = true;
            }
            if self.smart_object(h)?.influence_cell.is_none() {
                let position = self.smart_slot_position(h, 0)?;
                let influence = match &*self.smart_object(h)?.definition {
                    Definition::SearchDynamic { influence, .. } => *influence,
                    _ => unreachable!(),
                };
                let cell = self
                    .ai_dsm
                    .get_mut(dsm.handle.slot)
                    .ok_or("SearchSpot DSM payload absent")?
                    .add_search_influence(position, influence)?;
                self.smart_object_mut(h)?.influence_cell = cell;
                if cell.is_none() {
                    bevy::log::warn!(
                        "SearchSpot {} has no DSM cell at {:?}",
                        self.entities[index].name,
                        position
                    );
                }
            }
            let snapshot = self.entities[index].frame.pose.position;
            //5af810->5bed20 uses default domain0 (group3), not local
            //entity time and not its per-entity offset.
            let clock = world
                .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                .ok_or("SearchSpot source clock absent")?;
            let time = clock
                .clock()?
                .time(3)
                .ok_or("SearchSpot default time group absent")?;
            let space = 0;
            let component = self.smart_object_mut(h)?;
            component.position_snapshot = snapshot;
            component.deadline = Some((
                time + dynamic_cvar("ark_ai_search_spot_dynamic_poscheck_periodicity")? as f64,
                space,
            ));
        }
        self.smart_object_mut(h)?.started = true;
        Ok(())
    }
    ///Factory5af720 stage0,5af8f0: only the actual accepted update prefix.
    pub(in crate::app) fn update_dynamic_search_spots(
        &mut self,
        world: &World,
        live: &LiveEntities,
    ) -> Result<(), String> {
        let count = self.dynamic_search_spots.roster.sorted_start as usize;
        if count == 0 {
            return Ok(());
        }
        let clock = world
            .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("DynamicSearch stage0 clock absent")?
            .clock()?;
        let marker = clock.frame_marker();
        let interval = dynamic_cvar("ark_ai_search_spot_dynamic_poscheck_periodicity")?;
        let tolerance = dynamic_cvar("ark_ai_search_spot_dynamic_poscheck_tolerance")?;
        for cursor in 0..count {
            let h = ComponentHandle {
                class_id: self.classes[51],
                slot: self.dynamic_search_spots.roster.indices[cursor],
            };
            let (index, id, _) = self.smart_identity(h)?;
            if self.smart_object(h)?.updated_frame == Some(marker) {
                continue;
            }
            if !self.smart_object(h)?.started {
                return Err("stopped DynamicSearch remains in accepted stage0 roster".into());
            }
            let space = self.entities[index].timespace;
            let group = *[3, 4, 2, 0]
                .get(space as usize)
                .ok_or("DynamicSearch entity timespace invalid")?;
            let now = clock
                .time(group)
                .ok_or("DynamicSearch current time group absent")?;
            let deadline = self
                .smart_object(h)?
                .deadline
                .ok_or("DynamicSearch Start deadline absent")?
                .0;
            if deadline <= now && deadline != now {
                let position = self.entities[index].frame.pose.position;
                let prior = self.smart_object(h)?.position_snapshot;
                let delta = [
                    prior[0] - position[0],
                    prior[1] - position[1],
                    prior[2] - position[2],
                ];
                if tolerance * tolerance
                    < delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]
                {
                    let (_, _, dsm) = self.smart_world_dependencies(world, live, id)?;
                    if let Some(cell) = self.smart_object(h)?.influence_cell {
                        self.ai_dsm
                            .get_mut(dsm.handle.slot)
                            .ok_or("DynamicSearch DSM absent")?
                            .remove_search_influence(cell)?;
                        self.smart_object_mut(h)?.influence_cell = None;
                    }
                    let slot_position = self.smart_slot_position(h, 0)?;
                    let influence = match &*self.smart_object(h)?.definition {
                        Definition::SearchDynamic { influence, .. } => *influence,
                        _ => unreachable!(),
                    };
                    let cell = self
                        .ai_dsm
                        .get_mut(dsm.handle.slot)
                        .ok_or("DynamicSearch DSM absent")?
                        .add_search_influence(slot_position, influence)?;
                    if cell.is_none() {
                        bevy::log::warn!(
                            "DynamicSearch {} has no DSM cell at {:?}",
                            self.entities[index].name,
                            slot_position
                        );
                    }
                    let component = self.smart_object_mut(h)?;
                    component.influence_cell = cell;
                    component.position_snapshot = position;
                }
                self.smart_object_mut(h)?.deadline = Some((now + interval as f64, space));
            }
            self.smart_object_mut(h)?.updated_frame = Some(marker);
        }
        Ok(())
    }
    pub(super) fn stop_smart_object(
        &mut self,
        world: Option<&World>,
        live: &LiveEntities,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let (_, id, checked) = self.smart_identity(h)?;
        //No NPC-user admission exists in the bounded Cue startup closure.
        //An admitted user must go through5b71a0/5b77e0, never be dropped.
        if self
            .smart_object(h)?
            .slots
            .iter()
            .any(|slot| !slot.users.is_empty())
        {
            return Err(
                "AISmartObject Stop requires actual registered-user stop/remove owner".into(),
            );
        }
        self.smart_object_mut(h)?.active = false;
        if h.class_id == self.classes[51]
            && (self.smart_object(h)?.registered || self.smart_object(h)?.influence_cell.is_some())
        {
            let world = world.ok_or("SearchSpot Stop requires canonical AIWorld owner")?;
            let (recipient, coordination, dsm) = self.smart_world_dependencies(world, live, id)?;
            if self.smart_object(h)?.registered {
                self.coordination_mut(recipient, coordination)?
                    .remove_search_spot(checked);
                self.smart_object_mut(h)?.registered = false;
            }
            if let Some(cell) = self.smart_object(h)?.influence_cell {
                self.ai_dsm
                    .get_mut(dsm.handle.slot)
                    .ok_or("SearchSpot DSM payload absent")?
                    .remove_search_influence(cell)?;
                self.smart_object_mut(h)?.influence_cell = None;
            }
        }
        let component = self.smart_object_mut(h)?;
        component.started = false;
        component.deadline = None;
        Ok(())
    }
    pub(super) fn deinitialize_smart_object(
        &mut self,
        events: &mut EntityEvents,
        h: ComponentHandle,
    ) -> Result<(), String> {
        let (_, id, _) = self.smart_identity(h)?;
        let component = self.smart_object(h)?;
        if component.started || component.registered || component.influence_cell.is_some() {
            return Err("AISmartObject Deinit precedes Stop".into());
        }
        events.unregister(0x66, Some(id), self.smart_listener(h)?);
        let component = self.smart_object_mut(h)?;
        component.slots = Vec::new();
        component.fallback_slot = None;
        component.initialized = false;
        Ok(())
    }
    pub(super) fn dispatch_smart_object_event(
        &mut self,
        live: &LiveEntities,
        listener: Listener,
        event: &Event,
    ) -> Result<(), String> {
        let Listener::Component {
            owner,
            handle,
            generation,
            handler: 0x66,
        } = listener
        else {
            return Err("AISmartObject listener class differs".into());
        };
        let (_, id, checked) = self.smart_identity(handle)?;
        if owner != self.owner_id
            || generation != checked.generation
            || event.target != Some(id)
            || event.id != 0x66
            || live.get(id).is_none()
        {
            return Err("AISmartObject callback generation differs".into());
        }
        //5b5840 only touches existing users' actual Midnight handles. Empty
        //runtime user lists are the real constructor state, not a fake NPC query.
        if self
            .smart_object(handle)?
            .slots
            .iter()
            .any(|s| !s.users.is_empty())
        {
            return Err("AISmartObject66 requires its actual user Midnight handles".into());
        }
        Ok(())
    }
}

fn dynamic_cvar(name: &str) -> Result<f32, String> {
    crate::platform::console::shared()
        .read()
        .cvars
        .get(name)
        .map(|v| v.float)
        .ok_or_else(|| format!("SearchSpot cvar {name} absent"))
}
