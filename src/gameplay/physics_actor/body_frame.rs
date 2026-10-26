//! Movement-body frame publication shared with ordinary post-physics traversal.
use super::{post_update::PostPhysicsWorld, ActorUpdatePhase, PhysicsActorLinks};
use crate::{
    float::{add, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};

/// 80b7c0 shifts the cached motion velocities to the body's published origin.
/// Angular input is already in world space, not the motion's principal axes.
pub fn body_origin_velocities(
    position: [f32; 3],
    motion_center: [f32; 3],
    linear: [f32; 3],
    angular: [f32; 3],
) -> [[f32; 3]; 2] {
    let offset: [f32; 3] = std::array::from_fn(|i| subtract(position[i], motion_center[i]));
    let linear = [
        add(
            subtract(
                multiply(angular[2], offset[1]),
                multiply(offset[2], angular[1]),
            ),
            linear[0],
        ),
        add(
            subtract(
                multiply(angular[0], offset[2]),
                multiply(offset[0], angular[2]),
            ),
            linear[1],
        ),
        add(
            subtract(
                multiply(angular[1], offset[0]),
                multiply(offset[1], angular[0]),
            ),
            linear[2],
        ),
    ];
    [linear, angular]
}

impl PhysicsActorLinks {
    /// 754ea0 publishes into the actor cache before deciding whether the
    /// owning entity requires a post-physics traversal. Scale is preserved.
    pub fn publish_body_frame(
        &mut self,
        actor: usize,
        pose: ReferenceFrame,
        velocities: [[f32; 3]; 2],
        notify_entity: bool,
        world: &mut impl PostPhysicsWorld,
    ) -> Result<(), String> {
        let record = self
            .actors
            .get_at(actor)
            .ok_or("body publication actor is missing")?;
        let entity = record.owner.entity;
        self.actors[actor]
            .frame
            .publish(pose, velocities, world.publication_frame());
        if notify_entity {
            let previous =
                world.entity_frame(entity.ok_or("body publication actor has no owning entity")?)?;
            if previous.pose.position == pose.position
                && previous.pose.basis == pose.basis
                && previous.velocities == velocities
            {
                self.actors[actor].flags &= !1;
            } else {
                self.request_phase_update(
                    actor,
                    world.post_queue_frame(),
                    ActorUpdatePhase::PostPhysics,
                    |actors, actor| world.enqueue_post(actors, actor),
                )?;
            }
        }
        Ok(())
    }
}
