//! Actual pooled cue requests458490/458600/458730/459500 and update459250.
use super::*;
use crate::assets::ai_world::startup_managers::Definition;
use crate::gameplay::{
    ai_world::{Worlds, startup_managers::CuePerception},
    entity_events::{Commands, EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
    pickable_component::CheckedComponent,
    reference_frame::ReferenceFrame,
};
use bevy::prelude::World;

///bevy_backend: retained synchronous callback while canonical Actor/World
///borrows become available in the same-frame component update scope.
pub(super) struct Action {
    parent: CheckedComponent,
    instigator: EntityId,
    category: usize,
    operation: u16,
    frame: ReferenceFrame,
    now: f64,
    perceivers: Vec<EntityId>,
    midnight_handle: u32,
    culprit_kind: u32,
    selected: Option<EntityId>,
    phase: u8,
}
impl PhysicsScene {
    pub(super) fn deinitialize_ai_cue_children(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let generation = self.owner(handle)?.generation;
        if self
            .cue_actions
            .iter()
            .any(|a| a.parent.handle == handle && a.parent.generation == generation)
        {
            return Err("Cues Deinit precedes accepted callback drain".into());
        }
        loop {
            let component = self
                .ai_cues
                .get(handle.slot)
                .ok_or("Cues retiring payload absent")?;
            let [pool, index] = component.retire_cursor;
            if pool >= 25 {
                break;
            }
            let Some(&child) = component.pools[pool].children.get(index) else {
                let component = self.ai_cues.get_mut(handle.slot).unwrap();
                component.retire_cursor = [pool + 1, 0];
                component.retire_phase = 0;
                continue;
            };
            if live.get(child).is_none() {
                let component = self.ai_cues.get_mut(handle.slot).unwrap();
                component.retire_cursor[1] += 1;
                component.retire_phase = 0;
                continue;
            }
            match component.retire_phase {
                0 => self.set_entity_enabled(world, live, events, child, false)?,
                1 => {
                    let source =
                        super::super::runtime_entities::with_ai_cue(world, live, child, |c| {
                            Ok(c.instigator)
                        })?;
                    if let Some(instigator) = source.filter(|&id| live.get(id).is_some()) {
                        events.post(Event {
                            id: 0x78,
                            target: Some(instigator),
                            payload: Payload::Entity(child),
                        });
                    }
                }
                2 => super::super::entity_functions::enqueue_removal(world, child, 0.)?,
                _ => {
                    let component = self.ai_cues.get_mut(handle.slot).unwrap();
                    component.retire_phase = 0;
                    component.retire_cursor[1] += 1;
                    continue;
                }
            }
            self.ai_cues.get_mut(handle.slot).unwrap().retire_phase += 1;
        }
        let component = self.ai_cues.get_mut(handle.slot).unwrap();
        for pool in &mut component.pools {
            pool.children = Vec::new();
            pool.used = 0;
        }
        component.retire_cursor = [0; 2];
        component.retire_phase = 0;
        Ok(())
    }
    pub(super) fn dispatch_ai_cues_event(
        &mut self,
        live: &LiveEntities,
        _commands: &mut Commands,
        handle: ComponentHandle,
        event: &Event,
        clock: Option<&ClockRuntime>,
        worlds: Option<&Worlds>,
    ) -> Result<(), String> {
        let owner = self.owner(handle)?;
        if handle.class_id != self.classes[23] {
            return Err("cue event has another component class".into());
        }
        let index = owner.entity.ok_or("Cues callback has no parent")?;
        let parent = self.entities[index].shared;
        let checked = CheckedComponent {
            handle,
            generation: owner.generation,
        };
        let component = self
            .ai_cues
            .get(handle.slot)
            .ok_or("Cues callback payload absent")?;
        if !component.started {
            return Err("Cues callback owner stopped".into());
        }
        let instigator = event
            .target
            .ok_or("cue request has no actual instigator target")?;
        let (_, instigator_class) = live
            .get(instigator)
            .ok_or("cue request instigator is stale")?;
        let (category, position, perceivers, midnight_handle) = match &event.payload {
            Payload::AICueRequest {
                kind,
                position,
                perceivers,
                midnight_handle,
            } => (
                *kind as usize,
                *position,
                perceivers.clone(),
                *midnight_handle,
            ),
            Payload::Bytes(bytes) => {
                let required = if event.id == 0x75 { 4 } else { 16 };
                if bytes.len() != required {
                    return Err("cue request packet size differs from source".into());
                }
                let kind = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
                let position = if event.id == 0x75 {
                    [0.; 3]
                } else {
                    std::array::from_fn(|lane| {
                        f32::from_le_bytes(bytes[4 + lane * 4..8 + lane * 4].try_into().unwrap())
                    })
                };
                if event.id == 0x76 {
                    return Err("cue76 requires owned managed-NPC list payload".into());
                }
                (kind, position, Vec::new(), 0)
            }
            _ => return Err("Cues callback lacks source request payload".into()),
        };
        if !(0x73..=0x76).contains(&event.id) || category >= 25 {
            return Err("cue request category/message out of source range".into());
        }
        let Definition::Cues(definition) = &*component.definition else {
            return Err("Cues callback declaration class differs".into());
        };
        let mut frame = self
            .entity_frame(live, instigator)?
            .ok_or("cue instigator current frame absent")?;
        if event.id != 0x75 {
            if live.ai_world_index(instigator)? != live.ai_world_index(parent)? {
                return Ok(());
            }
            frame.position = position;
            frame.position[2] += definition.pools[category].z_offset;
            let worlds = worlds.ok_or("cue request needs canonical AIWorld area owner")?;
            if worlds.area_at(frame.position) != live.ai_world_index(parent)? as usize {
                return Ok(());
            }
        }
        let now = clock
            .ok_or("cue request clock owner absent")?
            .entity_time(parent)
            .map_err(str::to_owned)?;
        let culprit_kind = if self.throwable_class_is(instigator_class, b"arkPlayer")? {
            1
        } else if self.throwable_class_is(instigator_class, b"arkNPC")? {
            2
        } else if self.throwable_class_is(instigator_class, b"arkSwarmEntity")? {
            3
        } else {
            4
        };
        self.cue_actions
            .try_reserve(1)
            .map_err(|e| format!("cue callback journal: {e}"))?;
        self.cue_actions.push_back(Action {
            parent: checked,
            instigator,
            category,
            operation: event.id,
            frame,
            now,
            perceivers,
            midnight_handle,
            culprit_kind,
            selected: None,
            phase: 0,
        });
        Ok(())
    }
    fn cue_coordination_handle(&self, parent: ComponentHandle) -> Result<ComponentHandle, String> {
        let index = self.owner(parent)?.entity.ok_or("Cues parent absent")?;
        self.cue_dependency(index, b"arkCpntAIWorldCoordinationBB", [0x28, 0])?
            .map(|c| c.handle)
            .ok_or("Cues actual CoordinationBB dependency absent".into())
    }
    fn set_cue_frame(
        &mut self,
        world:&World,
        live: &mut LiveEntities,
        events:&mut EntityEvents,
        id: EntityId,
        frame: ReferenceFrame,
    ) -> Result<(), String> {
        live.get(id).ok_or("pooled Cue generation stale")?;
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("pooled Cue header absent")?;
        //4590b0 uses87d1e0→87d200, not ordinary753c10 publication.
        //The existing changed-pose boundary avoids an unrelated entity scan.
        if self.entities[index].frame.pose.position!=frame.position {
            world.get_resource::<crate::gameplay::ai_world::Worlds>().ok_or("Cue SetTransform AIWorld owner absent")?
                .assign_transformed(live,events,id,frame.position)?;
        }
        self.entities[index].frame.pose = frame;
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("pooled Cue reference actor absent")?;
        let actor = self
            .actors
            .actors
            .get_mut(handle.slot)
            .ok_or("pooled Cue reference actor stale")?;
        actor.set_transform(frame, false);
        let generation = actor.owner.generation;
        //Both a newly queued phase and an already retained phase acknowledge
        //this actual frame publication; false never means a lost update.
        self.request_post_physics_actor(handle.slot as usize, generation)?;
        Ok(())
    }
    fn drain_ai_cue_actions(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        while let Some(action) = self.cue_actions.front() {
            let parent = action.parent;
            if self.owner(parent.handle)?.generation != parent.generation {
                return Err("accepted Cues callback generation retired".into());
            }
            let category = action.category;
            if action.phase == 0 {
                let selected = if action.operation == 0x75 {
                    let mut selected = None;
                    for &child in
                        &self.ai_cues.get(parent.handle.slot).unwrap().pools[category].children
                    {
                        if super::super::runtime_entities::with_ai_cue(world, live, child, |c| {
                            Ok(c.instigator == Some(action.instigator))
                        })? {
                            selected = Some(child);
                            break;
                        }
                    }
                    selected
                } else {
                    let pool = &self.ai_cues.get(parent.handle.slot).unwrap().pools[category];
                    let child = *pool
                        .children
                        .get(pool.used)
                        .ok_or("cue request has no source-prewarmed pool slot")?;
                    super::super::runtime_entities::with_ai_cue(world, live, child, |_| Ok(()))?;
                    let pool =
                        &mut self.ai_cues.get_mut(parent.handle.slot).unwrap().pools[category];
                    pool.used = (pool.used + 1) % pool.children.len();
                    Some(child)
                };
                let action = self.cue_actions.front_mut().unwrap();
                action.selected = selected;
                action.phase = 1;
            }
            let Some(child) = self.cue_actions.front().unwrap().selected else {
                self.cue_actions.pop_front();
                continue;
            };
            let phase = self.cue_actions.front().unwrap().phase;
            match phase {
                1 => {
                    let action = self.cue_actions.front().unwrap();
                    if action.operation != 0x75 {
                        let (frame, source, kind) =
                            (action.frame, action.instigator, action.culprit_kind);
                        self.set_cue_frame(world,live,events,child,frame)?;
                        let component = self.ai_cues.get(parent.handle.slot).unwrap();
                        let Definition::Cues(def) = &*component.definition else {
                            unreachable!()
                        };
                        let full = def.pools[category].full_perception;
                        super::super::runtime_entities::with_ai_cue_mut(world, live, child, |c| {
                            c.instigator = Some(source);
                            c.instigator_kind = kind;
                            c.gives_full_perception = full;
                            Ok(())
                        })?;
                    }
                }
                2 => {
                    let action = self.cue_actions.front().unwrap();
                    let operation = action.operation;
                    if operation == 0x76 {
                        let coord = self.cue_coordination_handle(parent.handle)?;
                        let bb = self
                            .ai_coordination
                            .get_mut(coord.slot)
                            .ok_or("cue CoordinationBB payload absent")?;
                        if !bb.cue_perception.iter().any(|row| row.cue == child) {
                            bb.cue_perception.push(CuePerception {
                                cue: child,
                                perceivers: action.perceivers.clone(),
                                midnight_handle: action.midnight_handle,
                            });
                            bb.cue_perception_dirty = true;
                        }
                    }
                    let deadline = if operation == 0x75 {
                        action.now
                    } else if operation == 0x73 {
                        let Definition::Cues(def) =
                            &*self.ai_cues.get(parent.handle.slot).unwrap().definition
                        else {
                            unreachable!()
                        };
                        action.now + f64::from(def.pools[category].duration)
                    } else {
                        f64::MAX
                    };
                    super::super::runtime_entities::with_ai_cue_mut(world, live, child, |c| {
                        c.deadline = deadline;
                        Ok(())
                    })?;
                }
                3 => {
                    if self.cue_actions.front().unwrap().operation != 0x75 {
                        self.set_entity_enabled(world, live, events, child, true)?;
                    }
                }
                4 => {
                    let action = self.cue_actions.front().unwrap();
                    if action.operation != 0x75 {
                        events.post(Event {
                            id: 0x77,
                            target: Some(action.instigator),
                            payload: Payload::AICueGranted {
                                cue: child,
                                instigator: action.instigator,
                            },
                        });
                    }
                }
                _ => {
                    self.cue_actions.pop_front();
                    continue;
                }
            }
            self.cue_actions.front_mut().unwrap().phase += 1;
        }
        Ok(())
    }
    pub(in crate::app) fn finish_ai_cues(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        advance: bool,
    ) -> Result<(), String> {
        self.drain_ai_cue_actions(world, live, events)?;
        if !advance {
            return Ok(());
        }
        let clock = world
            .get_resource::<ClockRuntime>()
            .ok_or("Cues update clock absent")?;
        if !clock.advanced() {
            return Ok(());
        }
        let marker = clock.clock().map_err(str::to_owned)?.frame_marker();
        if self.cue_update_marker == Some(marker) {
            return Ok(());
        }
        for position in 0..self.ai_cues.roster.sorted_start as usize {
            let slot = self.ai_cues.roster.indices[position];
            let component = self
                .ai_cues
                .get(slot)
                .ok_or("scheduled Cues payload absent")?;
            if !component.started {
                continue;
            }
            let handle = ComponentHandle {
                class_id: self.classes[23],
                slot,
            };
            let parent =
                self.entities[component.owner.entity.ok_or("Cues update parent absent")?].shared;
            let now = world
                .resource::<ClockRuntime>()
                .entity_time(parent)
                .map_err(str::to_owned)?;
            let coord = self.cue_coordination_handle(handle)?;
            let bb = self
                .ai_coordination
                .get(coord.slot)
                .ok_or("cue CoordinationBB absent")?;
            let rows = if bb.cue_perception_dirty {
                bb.cue_perception.len()
            } else {
                0
            };
            for index in (0..rows).rev() {
                let row = &self.ai_coordination.get(coord.slot).unwrap().cue_perception[index];
                let child = row.cue;
                if row.midnight_handle != 0 {
                    //Installed459302 passes this word in EDX to the global
                    //7d63f0 Midnight manager, not the row's managed-NPC list.
                    return Err(
                        "cue perception Midnight handle requires actual global7d5860 scene owner"
                            .into(),
                    );
                }
                let child_index = *self
                    .shared_indices
                    .get(&child)
                    .ok_or("cue perception actor header absent")?;
                if self.entities[child_index].entity_flags & 0x100 != 0 {
                    let bb = self.ai_coordination.get_mut(coord.slot).unwrap();
                    bb.cue_perception.remove(index);
                    bb.cue_perception_dirty = !bb.cue_perception.is_empty();
                }
            }
            for category in 0..25 {
                let count = self.ai_cues.get(slot).unwrap().pools[category]
                    .children
                    .len();
                for child_index in 0..count {
                    let child =
                        self.ai_cues.get(slot).unwrap().pools[category].children[child_index];
                    let header = *self
                        .shared_indices
                        .get(&child)
                        .ok_or("cue update actor header absent")?;
                    if self.entities[header].entity_flags & 0x100 != 0 {
                        continue;
                    }
                    let (deadline, instigator) =
                        super::super::runtime_entities::with_ai_cue(world, live, child, |c| {
                            Ok((c.deadline, c.instigator))
                        })?;
                    if deadline < now {
                        self.set_entity_enabled(world, live, events, child, false)?;
                        if let Some(instigator) = instigator.filter(|&id| live.get(id).is_some()) {
                            events.post(Event {
                                id: 0x78,
                                target: Some(instigator),
                                payload: Payload::Entity(child),
                            });
                        }
                    }
                }
            }
        }
        self.cue_update_marker = Some(marker);
        Ok(())
    }
}
