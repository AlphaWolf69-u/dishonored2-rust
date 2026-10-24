//! Wind drag on rigid bodies: arkPhysicsShapeProperty's aero faces
//! (14008b830, built from the shape's hkGeometry) and the wind manager's
//! per-frame velocity change (14077f050 -> 14007a460 -> 14018d020).
//! Lane order and operation order follow the installed SSE code.

/// One 48-byte face: unit normal (w 0) and centroid with the triangle area
/// in w. The serialized triangle indices are not used by the drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AeroFace {
    pub normal: [f32; 4],
    pub center: [f32; 4],
}

/// The packed rsqrtps estimate plus 14008b830's single Newton step.
fn refined_inverse_length(squared: f32) -> f32 {
    if squared <= 0. {
        return 0.;
    }
    #[cfg(target_arch = "x86_64")]
    let estimate = unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(squared)))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let estimate = 1. / squared.sqrt();
    (3. - estimate * squared * estimate) * estimate * 0.5
}

/// 14008b830: triangles in shape space, in hkGeometry order. Degenerate
/// (zero-cross) triangles are dropped, as the original pops their record.
pub fn aero_faces(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>) -> Vec<AeroFace> {
    triangles
        .into_iter()
        .filter_map(|[a, b, c]| {
            let z = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
            let x = (b[1] - a[1]) * (c[2] - a[2]) - (c[1] - a[1]) * (b[2] - a[2]);
            let y = (b[2] - a[2]) * (c[0] - a[0]) - (c[2] - a[2]) * (b[0] - a[0]);
            let squared = z * z + (y * y + x * x);
            if squared == 0. {
                return None;
            }
            let inverse = refined_inverse_length(squared);
            let third = f32::from_bits(0x3eaaaaab);
            Some(AeroFace {
                normal: [inverse * x, inverse * y, inverse * z, 0.],
                center: [
                    (b[0] + a[0] + c[0]) * third,
                    (b[1] + a[1] + c[1]) * third,
                    (b[2] + a[2] + c[2]) * third,
                    inverse * squared * 0.5,
                ],
            })
        })
        .collect()
}

/// hknpBody transform columns; each w lane holds the body-space mass center.
pub type BodyColumns = [[f32; 4]; 3];

fn combine(c: &BodyColumns, v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| (v[1] * c[1][i] + v[0] * c[0][i]) + v[2] * c[2][i])
}

/// The columns' transpose applied to `v` (world to body space).
fn transpose_apply(c: &BodyColumns, v: [f32; 4]) -> [f32; 4] {
    let t = [
        [c[0][0], c[1][0], c[2][0], c[2][1]],
        [c[0][1], c[1][1], c[2][1], c[2][3]],
        [c[0][2], c[1][2], c[2][2], c[2][3]],
    ];
    std::array::from_fn(|i| (v[1] * t[1][i] + v[0] * t[0][i]) + v[2] * t[2][i])
}

fn shuffle_yzx(v: [f32; 4]) -> [f32; 4] {
    [v[1], v[2], v[0], v[3]]
}

/// Lanes of `shufps(c9)` applied to `shufps(c9)(a) * b - a * shufps(c9)(b)`.
fn cross_lanes(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (sa, sb) = (shuffle_yzx(a), shuffle_yzx(b));
    shuffle_yzx(std::array::from_fn(|i| sa[i] * b[i] - a[i] * sb[i]))
}

/// The motion state 14077f050 reads: center of mass (+0x00), orientation
/// (+0x10) and packed inverse inertia/mass halves (+0x20).
#[derive(Clone, Copy, Debug)]
pub struct MotionMass {
    pub center: [f32; 4],
    pub orientation: [f32; 4],
    pub inverse_mass: [u16; 4],
}

/// 14077f050 for one wind-registered body. `wind` is 77edf0's velocity at
/// the motion center; `linear` is the body's current world velocity. Returns
/// the world linear and angular velocity changes the original adds through
/// setBodyVelocity, or None below its thresholds for an inactive body.
pub fn wind_velocity_change(
    columns: &BodyColumns,
    faces: &[AeroFace],
    motion: &MotionMass,
    linear: [f32; 4],
    wind: [f32; 3],
    speed_factor: f32,
    seconds: f32,
    active: bool,
) -> Option<([f32; 4], [f32; 4])> {
    let air = (0. - f32::from_bits(0x3f9ccccd)) * 0.5;
    let local_wind = transpose_apply(
        columns,
        [wind[0] * speed_factor, wind[1] * speed_factor, wind[2] * speed_factor, 0.],
    );
    let local_linear = transpose_apply(columns, linear);
    let center = [columns[0][3], columns[1][3], columns[2][3], columns[2][3]];
    let mut force = [0.; 4];
    let mut torque = [0.; 4];
    for face in faces {
        let offset: [f32; 4] = std::array::from_fn(|i| face.center[i] - center[i]);
        let swept = cross_lanes(offset, local_linear);
        let relative: [f32; 4] =
            std::array::from_fn(|i| ((swept[i] + local_linear[i]) - local_wind[i]) * face.normal[i]);
        let negative = 0. - ((relative[1] + relative[0]) + relative[2]);
        if negative >= 0. {
            continue;
        }
        let push: [f32; 4] = std::array::from_fn(|i| {
            face.center[3] * air * negative * negative * face.normal[i]
        });
        for i in 0..4 {
            force[i] += push[i];
        }
        let moment = cross_lanes(push, offset);
        for i in 0..4 {
            torque[i] += moment[i];
        }
    }
    let force = combine(columns, force);
    let torque = combine(columns, torque);
    let (linear_change, angular_change) = velocity_change(motion, force, torque, seconds);
    let squared = |v: [f32; 4]| {
        let s: [f32; 4] = std::array::from_fn(|i| v[i] * v[i]);
        (s[1] + s[0]) + s[2]
    };
    (active
        || squared(linear_change) > f32::from_bits(0x38d1b717)
        || squared(angular_change) > f32::from_bits(0x3d800000))
    .then_some((linear_change, angular_change))
}

fn half(word: u16) -> f32 {
    f32::from_bits(u32::from(word) << 16)
}

/// Havok's rotation of `v` into the frame `q` (`_setRotatedInverseDir`)
/// before the final doubling, as inlined by 14007a460.
fn half_rotate_inverse(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, w] = q;
    let ww = w * w - 0.5;
    let dot = (qy * v[1] + qx * v[0]) + qz * v[2];
    [
        (dot * qx + ww * v[0]) + (qz * v[1] - v[2] * qy) * w,
        (dot * qy + ww * v[1]) + (qx * v[2] - v[0] * qz) * w,
        (dot * qz + ww * v[2]) + (qy * v[0] - v[1] * qx) * w,
    ]
}

/// Havok's rotation of `v` out of the frame `q` (`_setRotatedDir`) before
/// the final doubling, as inlined by 14007a460 and 140c85dc0.
fn half_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, w] = q;
    let ww = w * w - 0.5;
    let dot = (qy * v[1] + qx * v[0]) + qz * v[2];
    [
        (dot * qx + ww * v[0]) + (v[2] * qy - qz * v[1]) * w,
        (dot * qy + ww * v[1]) + (v[0] * qz - qx * v[2]) * w,
        (dot * qz + ww * v[2]) + (v[1] * qx - qy * v[0]) * w,
    ]
}

/// Motion-space vector to world space.
pub fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    half_rotate(q, v).map(|value| value + value)
}

/// World vector to motion space.
pub fn rotate_inverse(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    half_rotate_inverse(q, v).map(|value| value + value)
}

/// 14007a460: world force and torque over `seconds` to world velocity
/// changes through the motion's packed principal inverse inertia and mass.
pub fn velocity_change(
    motion: &MotionMass,
    force: [f32; 4],
    torque: [f32; 4],
    seconds: f32,
) -> ([f32; 4], [f32; 4]) {
    let t: [f32; 3] = std::array::from_fn(|i| torque[i] * seconds);
    let local = half_rotate_inverse(motion.orientation, t);
    let b: [f32; 3] =
        std::array::from_fn(|i| (local[i] + local[i]) * half(motion.inverse_mass[i]));
    let world = rotate(motion.orientation, b);
    let inverse_mass = half(motion.inverse_mass[3]);
    (
        std::array::from_fn(|i| inverse_mass * force[i] * seconds),
        [world[0], world[1], world[2], 0.],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aero_faces_keep_outward_normals_centroids_and_areas() {
        let faces = aero_faces([
            [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]],
            [[0., 0., 0.], [1., 0., 0.], [2., 0., 0.]],
        ]);
        assert_eq!(faces.len(), 1);
        let face = faces[0];
        assert!((face.normal[2] - 1.).abs() < 1e-6 && face.normal[0] == 0.);
        assert!((face.center[0] - 2. / 3.).abs() < 1e-6);
        assert!((face.center[3] - 2.).abs() < 1e-5);
    }

    #[test]
    fn wind_into_a_plate_pushes_along_the_wind() {
        let columns = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
        // A unit plate facing -x, offset in y so the push also twists it.
        let faces = [AeroFace {
            normal: [-1., 0., 0., 0.],
            center: [0., 0.5, 0., 1.],
        }];
        let motion = MotionMass {
            center: [0.; 4],
            orientation: [0., 0., 0., 1.],
            // inverse inertia 1 on all axes, inverse mass 0.5.
            inverse_mass: [0x3f80, 0x3f80, 0x3f80, 0x3f00],
        };
        let (linear, angular) =
            wind_velocity_change(&columns, &faces, &motion, [0.; 4], [2., 0., 0.], 1., 0.1, true)
                .unwrap();
        // F = -0.6125 * area * d^2 * n with d = -wind.x * n.x = 2.
        let expected = 0.6125 * 4.;
        assert!((linear[0] - 0.5 * expected * 0.1).abs() < 1e-5, "{linear:?}");
        assert!((angular[2] + 0.5 * expected * 0.1).abs() < 1e-5, "{angular:?}");
        // Leeward faces receive nothing.
        assert!(wind_velocity_change(
            &columns, &faces, &motion, [0.; 4], [-2., 0., 0.], 1., 0.1, false
        )
        .is_none());
    }
}
