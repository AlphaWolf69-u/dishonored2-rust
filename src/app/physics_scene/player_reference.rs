//! Real already constructed player -> canonical reference-frame actor bridge.
//! 874c50/74f5e0 create the ordinary reference actor; this record borrows the
//! player's managed identity and never creates another Player, proxy or body.
use super::*;
use crate::gameplay::{
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities, Stage},
    player::Player,
    reference_frame::ReferenceFrame,
};
impl PhysicsScene {
    pub(in crate::app) fn admit_player_reference(
        &mut self,
        player: &Player,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        timespace: u32,
    ) -> Result<(), String> {
        let id = player
            .managed_entity()
            .ok_or("constructed player has no managed identity")?;
        let (name, class) = live.get(id).ok_or("player reference generation is stale")?;
        if live.stage(id) != Some(Stage::Spawned) {
            return Err("player reference requires spawned real player".into());
        }
        let metadata = self
            .types
            .classes()
            .metadata(class)
            .ok_or("player reference class metadata is absent")?;
        if self.types.classes().name_text(metadata.name) != Some(b"arkPlayer") {
            return Err("player reference owner is not arkPlayer".into());
        }
        let index = if let Some(&index) = self.shared_indices.get(&id) {
            if !self.entities[index].external || self.entities[index].class != class {
                return Err("player identity already has a different scene owner".into());
            }
            index
        } else {
            let index = self.entities.len();
            let pose = ReferenceFrame {
                position: player.physics.position().to_array(),
                basis: player.physics.axis().to_cols_array_2d(),
            };
            self.entities.push(PhysicsEntity {
                name: name.to_owned(),
                shared: id,
                components: EntityComponents::new(index),
                class,
                environment: None,
                flags: 0,
                entity_flags: 0x001e_8008,
                entity_flags_d7: 0,
                priority: 0,
                timespace,
                external: true,
                retired: false,
                frame: crate::gameplay::reference_frame::PublishedFrame {
                    pose,
                    ..crate::gameplay::reference_frame::ReferenceFrameCache::default().published()
                },
            });
            self.entity_flags_d4.push(0);
            self.shared_indices.insert(id, index);
            index
        };
        let mut registry = self
            .registry
            .take()
            .ok_or("player reference component registry is absent")?;
        let result = (|| {
            self.reference_actor(index, &mut registry)?;
            registry.initialize_and_start(&mut interaction_components::Startup {
                scene: self,
                live,
                events,
            })?;
            registry.drain_update_changes(&mut interaction_components::Startup {
                scene: self,
                live,
                events,
            })
        })();
        self.registry = Some(registry);
        result
    }
    /// Called after actual Player.frame/view sampling. Only joints requested
    /// by real bound children are sampled from its existing body pose/cache.
    pub(in crate::app) fn publish_player_reference(
        &mut self,
        player: &Player,
        live: &LiveEntities,
        clock: &crate::gameplay::game_clock::GameClock,
    ) -> Result<(), String> {
        let id = player
            .managed_entity()
            .ok_or("player reference lost managed identity")?;
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("real player reference actor is not admitted")?;
        if !self.entities[index].external || live.get(id).is_none() {
            return Err("player reference record is stale".into());
        }
        let pose = ReferenceFrame {
            position: player.physics.position().to_array(),
            basis: player.physics.axis().to_cols_array_2d(),
        };
        let previous = self.entities[index].frame.pose;
        let delta = clock
            .entity_delta(self.entities[index].timespace)
            .ok_or("player reference timespace is absent")?;
        let velocities = if delta > 0. {
            crate::gameplay::reference_frame::velocities(previous, pose, delta)
        } else {
            [[0.; 3]; 2]
        };
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("player reference component is absent")?;
        self.actors
            .actors
            .get_mut(handle.slot)
            .ok_or("player reference actor retired")?
            .frame
            .publish(pose, velocities, clock.frame_marker());
        self.entities[index].frame.pose = pose;
        self.entities[index].frame.velocities = velocities;
        for (&(parent, joint), frame) in &mut self.document_hierarchy.external_joints {
            if parent == index {
                *frame = player.reference_local_joint(joint)?;
            }
        }
        self.request_post_physics_actor(
            handle.slot as usize,
            self.actors.actors[handle.slot].owner.generation,
        )?;
        Ok(())
    }
}
