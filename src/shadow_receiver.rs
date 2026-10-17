//! Native directional-shadow receiver initialization (FUN_140a707e0).
//!
//! This is the typed flag-0 ordinary-visibility path (original param_3 != 0).
//! It consumes the original camera/depth, global Tome bounds and dynamic AABBs;
//! it does not create Bevy shadow maps. The auxiliary flag-0 plane-only path
//! (param_3 == 0) is separate and never initializes the depth projection.
//! Optional flags-0x20 cascade loops and other flag modes remain unsupported;
//! auxiliary success is not ordinary visibility/depth availability.

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
    /// Original `FUN_140330c10` receiver input forwarded through
    /// `FUN_140a70710` to `FUN_140a707e0` param_8. The caller reads
    /// `plVar20[1]+0xafc+DAT_14397a2f8*4` (captured count 4 selects +0xb0c);
    /// if null, the receiver uses `DAT_141ab3828`. The count/index is the
    /// `r_shadowParallelSliceCnt` value updated from the selected preset by
    /// `FUN_14050ff40`; the capture reads 4. FUN_140351780 supplies the same
    /// &DAT_143308e20 owner to preparation and gather-results slot1, so this
    /// is the last UPDATED far, not the authored CVar. DAT_141ab3828 is
    /// verified FLT_MAX (bytes ff ff 7f 7f), the null-distance fallback.
    pub maximum_distance: f32,
    /// Global Tome bounds (+0x14..+0x28) seed the native union. Dynamic
    /// bounds expand them; they do not replace the level bounds.
    pub world_bounds: [[f32; 3]; 2],
    /// Exact game-space bounds supplied to the initializer, in original order.
    pub dynamic_bounds: &'a [[[f32; 3]; 2]],
    /// Ordinary 64x64 visibility depth from `DAT_1432fc640[1]+0x590`
    /// (valid flag `+0x598`). `FUN_140330c10` installs this view-state pointer
    /// in slot2 of `auStack_190c8`; `FUN_140a70710` forwards it to
    /// `FUN_140a707e0`, which selects it as `local_c80`. `FUN_140a71f80` then
    /// forwards `local_c80` as the hidden fifth argument to `FUN_140a6f640`;
    /// its 4096 samples feed the native reducer before projection. The source
    /// writer for this global depth pointer and alias to `lVar18+0x50` remain
    /// unresolved.
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

/// Auxiliary view resolved by the native +0x1b4 dirty-view update.
/// camera_position is auxiliary +0x140, not ordinary-state +0x4a0.
#[derive(Debug, Clone, Copy)]
pub struct NativeAuxiliaryReceiverInput {
    pub camera_clip_rows: ShadowMatrix,
    pub camera_position: [f32; 3],
    pub ray_direction: [f32; 3],
    pub maximum_distance: f32,
}

/// Plane-only result: no ordinary bitmap/receiver rows/depth mask initialized.
/// Source: chunk_000523.c:6876..6910,7029,7336 (FUN_140a707e0 param_3==0).
#[derive(Debug, Clone, Copy)]
pub struct NativeAuxiliaryReceiverOutput {
    pub distance: f32,
    pub finite_far: bool,
    pub cutoff: f32,
    pub camera_planes: [[f32; 4]; 6],
    pub planes: [[f32; 4]; 32],
    pub plane_count: usize,
    pub far_plane: [f32; 4],
    pub far_plane_signs: [u8; 4],
}

/// Numeric flag-0 auxiliary path. None is native status 3 for an infinite-far
/// view with no finite distance. Success cannot imply ordinary depth readiness.
pub fn initialize_auxiliary_receiver(
    input: NativeAuxiliaryReceiverInput,
) -> Result<Option<NativeAuxiliaryReceiverOutput>, String> {
    let output = receiver_planes(input)?;
    if !output.finite_far && !(output.distance < f32::MAX) {
        Ok(None)
    } else {
        Ok(Some(output))
    }
}

fn receiver_planes(
    input: NativeAuxiliaryReceiverInput,
) -> Result<NativeAuxiliaryReceiverOutput, String> {
    if !input.maximum_distance.is_finite() {
        return Err("nonfinite native receiver maximum distance".into());
    }
    let distance = input.maximum_distance;
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
    Ok(NativeAuxiliaryReceiverOutput {
        distance, finite_far, cutoff, camera_planes, planes, plane_count,
        far_plane, far_plane_signs,
    })
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

    let NativeAuxiliaryReceiverOutput {
        finite_far, cutoff, camera_planes, planes, plane_count,
        far_plane, far_plane_signs, ..
    } = receiver_planes(NativeAuxiliaryReceiverInput {
        camera_clip_rows: input.camera_clip_rows,
        camera_position: input.camera_position,
        ray_direction: input.ray_direction,
        maximum_distance: distance,
    })?;

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
