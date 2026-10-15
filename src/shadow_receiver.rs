//! Native directional-shadow receiver initialization (FUN_140a707e0).
//!
//! This is the typed flag-0 ordinary-visibility path (original param_3 != 0).
//! It consumes the original camera/depth, global Tome bounds and dynamic AABBs;
//! it does not create Bevy shadow maps. The auxiliary plane-only path
//! (param_3 == 0), optional flags-0x20 cascade loop and other flag modes remain
//! explicitly unsupported. The auxiliary path does not initialize the depth
//! projection and must not be substituted for an unavailable ordinary state.

use crate::{
    render_shadow::{
        camera_aligned_receiver_rows, multiply_rows, receiver_intersection_bounds,
        receiver_inverse_rows, receiver_orthographic_rows, ShadowMatrix,
    },
    tome::shadow_caster::{
        extrude_native_receiver_planes, fill_receiver_depth_projection,
        native_normalized_frustum_planes, NativeReceiverDepthProjection,
    },
};

#[derive(Debug, Clone, Copy)]
pub struct NativeReceiverInput<'a> {
    /// Original param_9 numeric selector. Only zero is implemented.
    pub flags: u32,
    /// Ordinary visibility clip rows (state +0).
    pub camera_clip_rows: ShadowMatrix,
    pub camera_position: [f32; 3],
    /// Receiver's native ray direction (normalized with the original f32 sqrt).
    pub ray_direction: [f32; 3],
    /// Ordinary-state +0x59c maximum depth; +0x598/+0x590 validity are
    /// represented by Some. None is an unavailable ordinary state, NOT the
    /// separate auxiliary-view branch (which leaves the depth projection alone).
    pub maximum_depth: Option<f32>,
    pub maximum_distance: f32,
    /// Global Tome bounds (+0x14..+0x28) seed the native union. Dynamic
    /// bounds expand them; they do not replace the level bounds.
    pub world_bounds: [[f32; 3]; 2],
    /// Exact game-space bounds supplied to the initializer, in original order.
    pub dynamic_bounds: &'a [[[f32; 3]; 2]],
    /// Ordinary 64x64 visibility depth consumed by FUN_140a71f80.
    pub ordinary_depth: &'a [f32; 4096],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeReceiverStatus {
    /// Geometric receiver volume was empty; projection/mask were not initialized.
    Empty,
    Initialized,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeReceiverOutput {
    pub status: NativeReceiverStatus,
    pub distance: f32,
    pub finite_far: bool,
    pub cutoff: f32,
    pub receiver_rows: ShadowMatrix,
    pub bounds: [f32; 6],
    pub planes: [[f32; 4]; 32],
    pub plane_count: usize,
    pub far_plane: [f32; 4],
    pub far_plane_signs: [u8; 4],
}

/// FUN_140a707e0's NUMERIC FLAG-0 path. Existing projection mask contents are
/// preserved (the native routine max-rasterizes into caller-owned storage).
pub fn initialize_native_receiver(
    input: NativeReceiverInput<'_>,
    projection: &mut NativeReceiverDepthProjection,
) -> Result<NativeReceiverOutput, String> {
    if input.flags != 0 {
        return Err("FUN_140a707e0 flags other than numeric 0 are unsupported".into());
    }
    if input.maximum_depth.is_none() {
        return Err("native receiver ordinary depth state is unavailable".into());
    }
    if input
        .world_bounds
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
        || (0..3).any(|axis| input.world_bounds[0][axis] > input.world_bounds[1][axis])
    {
        return Err("invalid native receiver global Tome bounds".into());
    }
    if input
        .dynamic_bounds
        .iter()
        .flatten()
        .flatten()
        .any(|v| !v.is_finite())
        || input.ray_direction.iter().any(|v| !v.is_finite())
        || !input.maximum_distance.is_finite()
    {
        return Err("nonfinite native receiver input".into());
    }

    let mut distance = input.maximum_distance;
    let inverse = receiver_inverse_rows(input.camera_clip_rows);
    if let Some(maximum_depth) = input.maximum_depth {
        let point = unproject(inverse, [0.0, 0.0, maximum_depth]);
        let delta =
            std::array::from_fn::<_, 3, _>(|axis| point[axis] - input.camera_position[axis]);
        let depth_distance =
            ((delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2]).sqrt();
        if distance > depth_distance {
            distance = depth_distance;
        }
    }

    let ray_length = ((input.ray_direction[0] * input.ray_direction[0]
        + input.ray_direction[1] * input.ray_direction[1])
        + input.ray_direction[2] * input.ray_direction[2])
        .sqrt();
    if !(ray_length > 0.0) || !ray_length.is_finite() {
        return Err("invalid native receiver ray direction".into());
    }
    let inverse_length = 1.0 / ray_length;
    let direction = input.ray_direction.map(|value| value * inverse_length);
    let (mut camera_planes, finite_far) = native_normalized_frustum_planes(input.camera_clip_rows);
    let mut cutoff = 1.0;
    if distance < f32::MAX {
        // FUN_140a707e0 clips the frustum BEFORE extrusion and intersection.
        // Using the unchanged camera far plane instead expands the receiver
        // to the camera's full distance and produces the wrong depth mask.
        if !finite_far {
            camera_planes[1] =
                camera_planes[0].map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        }
        let normal = camera_planes[1];
        let point: [f32; 3] =
            std::array::from_fn(|axis| -normal[axis] * distance + input.camera_position[axis]);
        camera_planes[1][3] =
            -((normal[1] * point[1] + normal[0] * point[0]) + normal[2] * point[2]);

        // The near normal's SECOND normalization uses original double sqrt
        // and reciprocal, with f32 rounding before projecting the cutoff.
        let near = camera_planes[0];
        let length_squared = (near[1] * near[1] + near[0] * near[0]) + near[2] * near[2];
        let mut normalized = [near[0], near[1], near[2]];
        if length_squared != 0.0 {
            let reciprocal = 1.0f64 / f64::from(length_squared).sqrt();
            normalized = normalized.map(|value| (f64::from(value) * reciprocal) as f32);
        }
        let cutoff_point = std::array::from_fn::<_, 3, _>(|axis| {
            normalized[axis] * distance + input.camera_position[axis]
        });
        cutoff = project_depth(input.camera_clip_rows, cutoff_point);
    }
    let far_plane = camera_planes[1];
    // Native 0x80f0 lanes are -(0 < coefficient), not IEEE sign bits.
    let far_plane_signs = far_plane.map(|value| if 0.0 < value { 0xff } else { 0 });
    let mut planes = [[0.0; 4]; 32];
    let plane_count = extrude_native_receiver_planes(direction, &camera_planes, &mut planes);

    let bounds = union_bounds(input.world_bounds, input.dynamic_bounds);
    // Native a89050 receives original param_5, not the separately normalized
    // direction used for extrusion. Re-normalizing it here changes live rows.
    let aligned = camera_aligned_receiver_rows(input.camera_clip_rows, input.ray_direction);
    let clipped = receiver_intersection_bounds(aligned, camera_planes, bounds);
    if !(clipped[0] < clipped[3] && clipped[1] < clipped[4] && clipped[2] < clipped[5]) {
        return Ok(NativeReceiverOutput {
            status: NativeReceiverStatus::Empty,
            distance,
            finite_far,
            cutoff,
            receiver_rows: [[0.0; 4]; 4],
            bounds: clipped,
            planes,
            plane_count,
            far_plane,
            far_plane_signs,
        });
    }
    let receiver_rows = multiply_rows(receiver_orthographic_rows(clipped), aligned);
    fill_receiver_depth_projection(
        projection,
        input.ordinary_depth,
        inverse,
        receiver_rows,
        cutoff,
    );

    // FUN_140a70510 -> a719c0 -> a70200. a719c0 selects each endpoint
    // product per matrix lane, accumulates X then Y then Z, and adds translation.
    for object in input.dynamic_bounds {
        let mut minimum = [0.0; 3];
        let mut maximum = [0.0; 3];
        for axis in 0..3 {
            let row = receiver_rows[axis];
            let interval = |coefficient: f32, low: f32, high: f32| {
                let a = coefficient * low;
                let b = coefficient * high;
                if b < a {
                    (b, a)
                } else {
                    (a, b)
                }
            };
            let (min_x, max_x) = interval(row[0], object[0][0], object[1][0]);
            let (min_y, max_y) = interval(row[1], object[0][1], object[1][1]);
            let (min_z, max_z) = interval(row[2], object[0][2], object[1][2]);
            minimum[axis] = ((min_x + min_y) + min_z) + row[3];
            maximum[axis] = ((max_x + max_y) + max_z) + row[3];
        }
        crate::tome::shadow_caster::rasterize_receiver_depth_mask(
            &mut projection.mask,
            minimum,
            maximum,
        );
    }

    Ok(NativeReceiverOutput {
        status: NativeReceiverStatus::Initialized,
        distance,
        finite_far,
        cutoff,
        receiver_rows,
        bounds: clipped,
        planes,
        plane_count,
        far_plane,
        far_plane_signs,
    })
}

fn unproject(inverse: ShadowMatrix, point: [f32; 3]) -> [f32; 3] {
    let denominator = ((point[0] * inverse[3][0] + point[1] * inverse[3][1])
        + point[2] * inverse[3][2])
        + inverse[3][3];
    let reciprocal = 1.0f64 / f64::from(denominator);
    std::array::from_fn(|axis| {
        let row = inverse[axis];
        let numerator = ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3];
        (f64::from(numerator) * reciprocal) as f32
    })
}

fn project_depth(matrix: ShadowMatrix, point: [f32; 3]) -> f32 {
    let row = matrix[3];
    let w = ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3];
    let row = matrix[2];
    let z = ((point[0] * row[0] + point[1] * row[1]) + point[2] * row[2]) + row[3];
    (f64::from(z) / f64::from(w)) as f32
}

fn union_bounds(world: [[f32; 3]; 2], objects: &[[[f32; 3]; 2]]) -> [f32; 6] {
    let mut bounds = [
        world[0][0],
        world[0][1],
        world[0][2],
        world[1][0],
        world[1][1],
        world[1][2],
    ];
    for object in objects {
        for axis in 0..3 {
            if object[0][axis] < bounds[axis] {
                bounds[axis] = object[0][axis];
            }
            if bounds[axis + 3] < object[1][axis] {
                bounds[axis + 3] = object[1][axis];
            }
        }
    }
    bounds
}

#[cfg(test)]
#[path = "shadow_receiver_tests.rs"]
mod tests;
