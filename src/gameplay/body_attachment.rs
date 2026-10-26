//! Motion attachment (c92500), joining body storage, shape queries and bounds.
use super::{
    body_lod::BodyLodConfig, body_quality::BodyQualityLibrary, body_storage::BodyStorage,
    broadphase::PackedQuantization, collision_body::SweptBoundsStep, motion_storage::MotionStorage,
    shape_transform::ShapeMatrix,
};

/// Required world operations. These must resolve retained shapes and the
/// deactivation owner; no default/no-op implementation is provided.
pub trait AttachmentWorld {
    fn shape_flags(&mut self, shape: usize) -> Result<u16, String>;
    /// Selected shape's virtual +68, used only when its header has flag2.
    fn rotation_limit_measure(&mut self, shape: usize) -> Result<f32, String>;
    fn shape_bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String>;
    fn register_deactivation_body(&mut self, body_id: u32, motion_id: u32) -> Result<(), String>;
}

pub struct AttachmentSettings<'a> {
    pub qualities: &'a BodyQualityLibrary,
    pub lod: &'a BodyLodConfig,
    pub tolerance: [f32; 4],
    pub step: &'a SweptBoundsStep,
    pub packing: &'a PackedQuantization,
}

/// Complete c92500 sequencing. The world supplies its actual shape virtuals,
/// solver-info values and deactivation registration through AttachmentWorld.
pub fn attach_body_motion(
    bodies: &mut BodyStorage,
    motions: &mut MotionStorage,
    body_id: u32,
    motion_id: u32,
    orientation: Option<[f32; 4]>,
    settings: &AttachmentSettings<'_>,
    world: &mut impl AttachmentWorld,
) -> Result<(), String> {
    // Validate the preset before changing the attachment ring.
    let quality_id = bodies
        .get(body_id)
        .ok_or("attachment body ID is stale or missing")?
        .quality_id;
    let flags = settings
        .qualities
        .get(quality_id)
        .ok_or("body quality ID exceeds preset library")?
        .requested_flags;
    bodies.link_motion(body_id, motion_id, motions)?;
    let body = bodies.get_mut(body_id).unwrap();
    let motion = motions.get_mut(motion_id).unwrap();
    if motion.solver_id == 0x7fff_ffff && body.broadphase_id != u32::MAX {
        world.register_deactivation_body(body_id, motion_id)?;
    }
    if flags & 0x10000 != 0 {
        let shape = body.selected_shape(2)?;
        if world.shape_flags(shape)? & 2 != 0 {
            let measure = world.rotation_limit_measure(shape)? * 0.5;
            let minimum = f32::from_bits(0x3d4c_cccd);
            let measure = if measure > minimum { measure } else { minimum };
            let old = f32::from_bits(u32::from(motion.limits[1]) << 16);
            let limit = if measure < old { measure } else { old };
            motion.limits[1] = (limit.to_bits() >> 16) as u16;
        }
    }
    let lookahead = body.transform[3][3];
    if lookahead > 0. {
        let old = f32::from_bits(u32::from(motion.limits[0]) << 16);
        let limit = if old < lookahead { old } else { lookahead };
        motion.limits[0] = (limit.to_bits() >> 16) as u16;
    }
    body.attach_motion_frame(&motion.pose, orientation);
    if body.broadphase_id != u32::MAX {
        body.try_update_swept_bounds(
            motion,
            flags,
            settings.tolerance,
            settings.step,
            settings.packing,
            |shape, frame| world.shape_bounds(shape, frame),
        )?;
    }
    let identity = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    let local_bounds = world.shape_bounds(body.selected_shape(3)?, identity)?;
    body.update_motion_radius(motion.pose.center, local_bounds);
    body.update_tim_distance(
        motion.linear_velocity,
        settings.step.contact_distance_scale[0],
        settings.step.duration[0],
    );
    body.update_size_and_static_lod(settings.lod.body_is_big_threshold, true);
    body.update_speed_lod(
        motion.linear_velocity,
        settings.lod.fast_to_slow_threshold,
        settings.lod.slow_to_fast_threshold,
    );
    Ok(())
}

#[cfg(test)]
#[path = "body_attachment/tests.rs"]
mod tests;
