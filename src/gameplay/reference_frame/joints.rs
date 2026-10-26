//! Joint-bound frame composition, including model scale and frame normalization.
use super::{basis_product, quaternion_basis, transform_position, ReferenceFrame};
use crate::float::{add, divide, multiply, subtract};

pub struct JointFrames<'a> {
    pub joint_count: u16,
    pub channel_remaps: &'a [Vec<i16>],
    pub transforms: &'a [[f32; 16]],
    pub model_scale: [f32; 3],
}

/// Complete09b900 projection of a padded64-byte joint matrix.
pub fn matrix_frame(matrix: [f32; 16]) -> ReferenceFrame {
    ReferenceFrame {
        position: [matrix[12], matrix[13], matrix[14]],
        basis: std::array::from_fn(|i| [matrix[i * 4], matrix[i * 4 + 1], matrix[i * 4 + 2]]),
    }
}

///53d3f0 uses the phase channel's actual remap, not the skeleton joint index
/// as a pose-buffer index. A -1 request writes identity but returns false.
pub fn local_bound_frame(
    source: &JointFrames<'_>,
    channel: usize,
    joint: i16,
    offset: [f32; 3],
    orientation: [f32; 4],
    output: &mut ReferenceFrame,
) -> Result<bool, String> {
    if joint == -1 {
        *output = ReferenceFrame {
            position: [0.; 3],
            basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        };
        return Ok(false);
    }
    if joint < 0 || joint as u16 >= source.joint_count {
        return Ok(false);
    }
    let remap = source
        .channel_remaps
        .get(channel)
        .ok_or("joint frame channel is missing")?;
    if remap.is_empty() {
        return Ok(false);
    }
    let mapped = *remap
        .get(joint as usize)
        .ok_or("joint remap is incomplete")?;
    let matrix = usize::try_from(mapped)
        .ok()
        .and_then(|i| source.transforms.get(i))
        .ok_or("mapped joint transform is missing")?;
    let joint = matrix_frame(*matrix);
    let scaled = std::array::from_fn::<_, 3, _>(|i| multiply(source.model_scale[i], offset[i]));
    //53d3f0 sums the products before ADDSS(translation, sum). Ghidra's
    // flattened expression suggests a different grouping; retain the machine
    // sequence and its translation operand order.
    output.position = std::array::from_fn(|i| {
        add(
            joint.position[i],
            add(
                add(
                    multiply(joint.basis[1][i], scaled[1]),
                    multiply(joint.basis[0][i], scaled[0]),
                ),
                multiply(joint.basis[2][i], scaled[2]),
            ),
        )
    });
    output.basis = basis_product(quaternion_basis(orientation), joint.basis);
    Ok(true)
}

/// Complete00f2c0: normalize the first axis, rebuild the third by cross product,
/// then rebuild the second. Degenerate squared lengths clamp to MIN_POSITIVE.
pub fn normalize_basis(mut basis: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let inverse = |v: [f32; 3]| {
        let squared = add(
            add(multiply(v[0], v[0]), multiply(v[1], v[1])),
            multiply(v[2], v[2]),
        );
        let clamped = if squared <= f32::MIN_POSITIVE {
            f32::MIN_POSITIVE
        } else {
            squared
        };
        divide(1., clamped.sqrt())
    };
    let scale = inverse(basis[0]);
    basis[0] = basis[0].map(|v| multiply(v, scale));
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            subtract(multiply(a[1], b[2]), multiply(a[2], b[1])),
            subtract(multiply(a[2], b[0]), multiply(a[0], b[2])),
            subtract(multiply(a[0], b[1]), multiply(a[1], b[0])),
        ]
    };
    basis[2] = cross(basis[0], basis[1]);
    let scale = inverse(basis[2]);
    basis[2] = basis[2].map(|v| multiply(v, scale));
    basis[1] = cross(basis[2], basis[0]);
    let scale = inverse(basis[1]);
    basis[1] = basis[1].map(|v| multiply(v, scale));
    basis
}

///53c4f0: an all-zero origin override selects the relative model offset.
pub fn bound_origin(
    reference: [f32; 3],
    offset: [f32; 3],
    override_position: [f32; 3],
) -> [f32; 3] {
    if override_position == [0.; 3] {
        std::array::from_fn(|i| add(offset[i], reference[i]))
    } else {
        override_position
    }
}

///53d800 receives the published render-model world frame;53de90 receives its
/// actual current/predicted reference frame with bound_origin already applied.
pub fn world_bound_frame(
    source: &JointFrames<'_>,
    channel: usize,
    joint: i16,
    offset: [f32; 3],
    orientation: [f32; 4],
    world: impl FnOnce() -> Result<ReferenceFrame, String>,
    output: &mut ReferenceFrame,
) -> Result<bool, String> {
    if !local_bound_frame(source, channel, joint, offset, orientation, output)? {
        return Ok(false);
    }
    let world = world()?;
    output.position = transform_position(world, output.position);
    //0a55d0's non-aliasing in-place product is the shared00dce0 value product.
    output.basis = normalize_basis(basis_product(output.basis, world.basis));
    Ok(true)
}

#[cfg(test)]
#[path = "joints_tests.rs"]
mod tests;
