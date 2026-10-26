//! Attached-body mass-frame recomputation (c90a90).
use super::{
    body_storage::BodyStorage, motion_storage::MotionStorage, rigid_body::BodyMotion,
    shape_transform::ShapeMatrix,
};
use crate::assets::physworld::{
    mass::{combine_mass, MassElement, MassProperties, PrincipalMassProperties},
    rotation_quaternion,
};

/// Recompute the actual attached-body mass frame, then refresh cached frames
/// and preserve world angular velocity when the pose comparison changes.
/// mass_frame resolves caefc0 through the retained body/shape resource owner;
/// shape_bounds dispatches the selected shape's actual identity calcAabb.
pub fn recompute_motion_mass(
    bodies: &mut BodyStorage,
    motions: &mut MotionStorage,
    motion_id: u32,
    body_is_big_threshold: f32,
    mut mass_frame: impl FnMut(&BodyStorage, u32) -> Result<PrincipalMassProperties, String>,
    mut shape_bounds: impl FnMut(usize, ShapeMatrix) -> Result<[[f32; 4]; 2], String>,
) -> Result<(), String> {
    let motion = motions
        .get_mut(motion_id)
        .ok_or("mass recomputation motion is missing")?;
    let first = motion.first_body_id;
    if first & 0xff_ffff == 0xff_ffff {
        return Ok(());
    }
    let old_pose = motion.pose;
    let world_angular_velocity = motion.world_angular_velocity();
    let mut ids = Vec::new();
    let mut id = first;
    loop {
        let body = bodies
            .get(id)
            .ok_or("mass recomputation attachment ring contains a stale ID")?;
        ids.push(id);
        id = body.next_attached_body_id;
        if id == first {
            break;
        }
        if ids.len() >= bodies.capacity() {
            return Err("mass recomputation attachment ring does not close".into());
        }
    }
    let applied = if ids.len() == 1 {
        let frame = mass_frame(bodies, first)?;
        let body = bodies.get(first).unwrap();
        let orientation =
            rotation_quaternion([body.transform[0], body.transform[1], body.transform[2]]);
        BodyMotion::from_mass_frame(frame, orientation, body.transform[3])
    } else {
        let mut elements = Vec::with_capacity(ids.len());
        for &id in &ids {
            let properties = mass_frame(bodies, id)?.to_tensor();
            elements.push(MassElement {
                properties,
                transform: bodies.get(id).unwrap().transform,
            });
        }
        // The source clears this record and intentionally ignores combine's
        // status before conversion; preserve that path even for zero mass.
        let mut combined = MassProperties {
            volume: 0.,
            mass: 0.,
            center: [0.; 4],
            inertia: [[0.; 4]; 3],
        };
        let _ = combine_mass(&elements, &mut combined);
        BodyMotion::from_mass_frame(combined.to_principal(), [0., 0., 0., 1.], [0.; 4])
    };
    motion.pose = applied.pose;
    motion.inverse_mass = applied.inverse_mass;
    if bodies.get(first).unwrap().flags & 4 != 0 {
        motion.inverse_mass = [0; 4];
    }
    if !motion
        .pose
        .matches_pose(&old_pose, f32::from_bits(0x3a83_126f))
    {
        let identity = [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ];
        for id in ids {
            let body = bodies.get_mut(id).unwrap();
            body.attach_motion_frame(&motion.pose, None);
            body.update_motion_radius(
                motion.pose.center,
                shape_bounds(body.selected_shape(3)?, identity)?,
            );
            body.update_size_and_static_lod(body_is_big_threshold, true);
        }
        motion.set_world_angular_velocity(world_angular_velocity);
    }
    Ok(())
}

#[cfg(test)]
#[path = "motion_mass/tests.rs"]
mod tests;
