//! Movement-body frame publication shared with ordinary post-physics traversal.
use super::{ActorUpdatePhase, PhysicsActorLinks, post_update::PostPhysicsWorld};
use crate::{
    float::{add, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};

/// 8086a0. Velocity of an offset center of mass rotating about the body origin.
pub fn center_rotation_velocity(
    position: [f32; 3],
    center: [f32; 3],
    angular: [f32; 3],
    delta: f32,
) -> [f32; 3] {
    let squared = add(
        add(multiply(angular[1], angular[1]), multiply(angular[0], angular[0])),
        multiply(angular[2], angular[2]),
    );
    if squared <= f32::from_bits(0x2880_0000) { // 1423cce40 = 2^-46
        return [0.; 3];
    }
    let inverse = super::super::constraint_solver::inverse_length(squared);
    let [x, y, z] = angular.map(|value| multiply(value, inverse));
    let axis = [x, y, z, 0.];
    let angle = multiply(multiply(inverse, squared), delta);
    let rotation = super::super::shape_transform::rotation::axis_angle_basis(axis, angle);
    let offset: [f32; 3] = std::array::from_fn(|i| subtract(center[i], position[i]));
    let rotated: [f32; 3] = std::array::from_fn(|i| {
        add(
            add(multiply(rotation[0][i], offset[0]), multiply(rotation[1][i], offset[1])),
            multiply(rotation[2][i], offset[2]),
        )
    });
    let inverse_delta = super::super::constraint_solver::reciprocal(delta);
    std::array::from_fn(|i| multiply(subtract(rotated[i], offset[i]), inverse_delta))
}

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
