//! Equipment's already owned entity -> shared reference actor bridge.
//! 619380 and 8ae9b0 copy the real parent's frame before registration;
//! 7545a0/753c10 then traverse those same bound entities through 8783d0.
//! Animation/models and the managed identity remain with their existing owners.
use super::*;
use crate::gameplay::{
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities, Stage},
    game_clock::GameClock,
    reference_frame::{relative_binding, PublishedFrame, ReferenceFrame},
};

impl PhysicsScene {
    pub(in crate::app) fn publish_equipment_runtime_flags(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        flags: u8,
    ) -> Result<(), String> {
        live.get(id).ok_or("equipment flag generation is stale")?;
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("equipment flag header absent")?;
        if !self.entities[index].external || self.entities[index].retired {
            return Err("equipment flags targeted another Scene owner".into());
        }
        *self
            .entity_flags_d4
            .get_mut(index)
            .ok_or("equipment flag storage absent")? = flags;
        Ok(())
    }
    pub(in crate::app) fn equipment_parent_frame(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<ReferenceFrame, String> {
        self.entity_frame(live, id)?
            .ok_or_else(|| "equipment birth parent has no actual reference frame".into())
    }
    pub(in crate::app) fn admit_equipment_reference(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        frame: ReferenceFrame,
        timespace: u32,
    ) -> Result<(), String> {
        let (name, class) = live
            .get(id)
            .ok_or("equipment reference generation is stale")?;
        if live.stage(id) != Some(Stage::Spawned) || timespace > 1 {
            return Err(
                "equipment reference requires actual spawned identity and clock selector".into(),
            );
        }
        let index = if let Some(&index) = self.shared_indices.get(&id) {
            let owner = &self.entities[index];
            if owner.retired || !owner.external || owner.class != class || owner.name != name {
                return Err("equipment identity already has a different Scene owner".into());
            }
            index
        } else {
            let index = self
                .free_entities
                .last()
                .copied()
                .unwrap_or(self.entities.len());
            if index < self.entities.len()
                && (!self.entities[index].retired
                    || !self.entities[index].components.entries.values().is_empty())
            {
                return Err("equipment reference free header still has an owner".into());
            }
            if index == self.entities.len() {
                self.entities
                    .try_reserve(1)
                    .map_err(|e| format!("equipment headers: {e}"))?;
                self.entity_flags_d4
                    .try_reserve(1)
                    .map_err(|e| format!("equipment flags: {e}"))?;
            }
            self.shared_indices
                .try_reserve(1)
                .map_err(|e| format!("equipment identity projection: {e}"))?;
            let owner = PhysicsEntity {
                retired: false,
                external: true,
                name: name.to_owned(),
                shared: id,
                components: EntityComponents::new(index),
                class,
                environment: None,
                flags: 0,
                entity_flags: 0x001e_8008,
                entity_flags_d7: 0,
                priority: 0,
                frame: PublishedFrame {
                    pose: frame,
                    scale: [1.; 3],
                    velocities: [[0.; 3]; 2],
                },
                timespace,
            };
            if index == self.entities.len() {
                self.entities.push(owner);
                self.entity_flags_d4.push(0);
            } else {
                self.free_entities.pop();
                self.entities[index] = owner;
                self.entity_flags_d4[index] = 0;
            }
            // Publish before fallible component acquisition: a failed birth
            // remains discoverable by the equipment owner's teardown journal.
            self.shared_indices.insert(id, index);
            index
        };
        let mut registry = self
            .registry
            .take()
            .ok_or("equipment reference registry borrowed")?;
        let result = (|| {
            self.reference_actor(index, &mut registry)?;
            self.run_runtime_startup_with_registry(live, events, &mut registry)
        })();
        self.registry = Some(registry);
        result
    }

    /// Use the source's common752900 reciprocal binding owner instead of a
    /// second hierarchy in the equipment table. The actual binding/animation
    /// owner continues to publish exact world frames, including its real tags.
    pub(in crate::app) fn bind_equipment_reference(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        parent: EntityId,
    ) -> Result<(), String> {
        let frame = self.equipment_parent_frame(live, id)?;
        let parent_frame = self.equipment_parent_frame(live, parent)?;
        let (offset, orientation) = relative_binding(parent_frame, frame);
        self.bind_entity(
            live,
            id,
            parent,
            super::throwable::BindingPose {
                joint: -1,
                offset,
                orientation,
                parent_placement: None,
            },
            events,
        )
    }

    pub(in crate::app) fn publish_equipment_reference(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        frame: ReferenceFrame,
        timespace: u32,
        clock: &GameClock,
    ) -> Result<(), String> {
        if live.get(id).is_none() || timespace > 1 {
            return Err("equipment frame generation or clock selector is stale".into());
        }
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("equipment frame has no Scene owner")?;
        if self.entities[index].retired || !self.entities[index].external {
            return Err("equipment frame targeted a different Scene owner".into());
        }
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("equipment reference actor absent")?;
        let delta = clock
            .entity_delta(timespace)
            .ok_or("equipment clock selector unavailable")?;
        let velocities = if delta > 0. {
            crate::gameplay::reference_frame::velocities(
                self.entities[index].frame.pose,
                frame,
                delta,
            )
        } else {
            [[0.; 3]; 2]
        };
        let actor = self
            .actors
            .actors
            .get_mut(handle.slot)
            .ok_or("equipment reference actor retired")?;
        actor.frame.publish(frame, velocities, clock.frame_marker());
        // This projection borrows the binding owner's finished world frame.
        // Keep the ordinary parent-local relation coherent without sampling a
        // second skeleton or having generic post-update replace that frame.
        if let Some(parent) = actor.binding.entity {
            let parent_frame = self
                .entities
                .get(parent)
                .ok_or("equipment parent header absent")?
                .frame
                .pose;
            let (offset, orientation) = relative_binding(parent_frame, frame);
            actor.binding.offset = offset;
            actor.binding.orientation = orientation;
        }
        let generation = actor.owner.generation;
        self.entities[index].frame.pose = frame;
        self.entities[index].frame.velocities = velocities;
        self.entities[index].timespace = timespace;
        self.request_post_physics_actor(handle.slot as usize, generation).map(|_| ())
    }

    pub(in crate::app) fn detach_equipment_reference(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
    ) -> Result<(), String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(());
        };
        if !self.entities[index].external {
            return Err("equipment detach targeted another Scene owner".into());
        }
        self.detach_document(live, index)
    }

    /// Shared component registry acknowledgements precede actor/header reuse.
    /// Equipment keeps its actual identity, clock, animation, audio and GPU
    /// retirement receipts; this bridge retires only its reference projection.
    pub(in crate::app) fn retire_equipment_reference(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        state: &mut super::lifetime::Retirement,
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(true);
        };
        if !self.entities[index].external {
            return Err("equipment retirement targeted another Scene owner".into());
        }
        if !super::lifetime::poll_entity_components(world, self, live, events, index, state)? {
            return Ok(false);
        }
        Ok(true)
    }

    /// The equipment owner's final managed-ID acknowledgement precedes reuse
    /// of its Scene header, just as for ordinary runtime entities.
    pub(in crate::app) fn finish_equipment_reference(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<(), String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(());
        };
        if live.get(id).is_some() {
            return Err(
                "equipment header reclamation precedes its managed identity retirement".into(),
            );
        }
        if !self.entities[index].external
            || !self.entities[index].components.entries.values().is_empty()
        {
            return Err("equipment header retains a different or live component owner".into());
        }
        self.free_entities
            .try_reserve(1)
            .map_err(|e| format!("equipment header retirement: {e}"))?;
        self.document_hierarchy.retire_entity_frames(index)?;
        self.shared_indices.remove(&id);
        self.entities[index].retired = true;
        self.entities[index].name = String::new();
        self.free_entities.push(index);
        Ok(())
    }
}
