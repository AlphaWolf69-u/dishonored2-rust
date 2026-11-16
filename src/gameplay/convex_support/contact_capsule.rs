//! cb4830's capsule contact polytope, completed by ca3970's oriented box.
//! The retained scratch owns topology only; no asset, body or query cache.
use super::faces::reciprocal_sqrt_estimate;
use crate::{
    assets::physworld::polytope::{ConvexPolytope, PolytopeFace},
    gameplay::shape_transform::{rotate_vector, transform_point},
};

/// cb4830 initializes103, then ca3970 sets40 and180: the final header is1c3.
pub(crate) const CAPSULE_FLAGS: u16 = 0x1c3;

pub(crate) struct CapsulePolytope {
    shape: ConvexPolytope,
}
impl Default for CapsulePolytope {
    fn default() -> Self {
        Self {
            shape: ConvexPolytope {
                vertices: vec![[0.; 4]; 8],
                planes: vec![[0.; 4]; 6],
                faces: (0..6)
                    .map(|index| PolytopeFace {
                        first_index: index * 4,
                        num_indices: 4,
                        min_half_angle: 4,
                    })
                    .collect(),
                // ca3970's six installed little-endian topology words. cb4830
                // overrides only each face's angle byte, retaining this winding.
                indices: vec![
                    2, 6, 4, 0, 1, 5, 7, 3, 1, 0, 4, 5, 7, 6, 2, 3, 3, 2, 0, 1, 7, 5, 4, 6,
                ],
                radius: 0.,
                connectivity: None,
            },
        }
    }
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
impl CapsulePolytope {
    pub(crate) fn prepare(
        &mut self,
        endpoints: &[[f32; 4]],
        radius: f32,
    ) -> Result<&ConvexPolytope, String> {
        let a = *endpoints
            .first()
            .ok_or("capsule contact first endpoint absent")?;
        let b = *endpoints
            .get(1)
            .ok_or("capsule contact second endpoint absent")?;
        let delta: [f32; 4] = std::array::from_fn(|i| b[i] - a[i]);
        let squared = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
        let estimate = reciprocal_sqrt_estimate(squared);
        let inverse = if squared <= 0. {
            0.
        } else {
            (3. - estimate * squared * estimate) * estimate * 0.5
        };
        let axis = delta.map(|value| value * inverse);
        let xy = axis[1] * axis[1] + axis[0] * axis[0];
        let xz = (axis[0] * axis[0] + axis[2] * axis[2]) + 0.;
        let alternate = xy < xz;
        let perpendicular = if alternate {
            [-axis[2], 0., axis[0], 0.]
        } else {
            [-axis[1], axis[0], 0., 0.]
        };
        let perpendicular_squared = if xy > xz { xy } else { xz };
        let estimate = reciprocal_sqrt_estimate(perpendicular_squared);
        // The original does not mask a zero perpendicular's RSQRT. Preserve
        // its degeneracy rather than replacing a zero-length capsule by a ball.
        let inverse_perpendicular =
            (3. - perpendicular_squared * estimate * estimate) * estimate * 0.5;
        let side = perpendicular.map(|value| value * inverse_perpendicular);
        let cross = [
            (perpendicular[2] * axis[1] - axis[2] * perpendicular[1]) * inverse_perpendicular,
            (perpendicular[0] * axis[2] - axis[0] * perpendicular[2]) * inverse_perpendicular,
            (perpendicular[1] * axis[0] - axis[1] * perpendicular[0]) * inverse_perpendicular,
            (axis[3] * 0. - axis[3] * 0.) * inverse_perpendicular,
        ];
        let center = std::array::from_fn(|i| delta[i] * 0.5 + a[i]);
        let frame = [axis, side, cross, center];
        // DAT141ab3810 bits3c23d70a=.01, DAT141ab3764=.5. Subtraction
        // preserves cb4830's radius rounding instead of multiplying by .99.
        let margin = radius * f32::from_bits(0x3c23d70a);
        let extent = inverse * squared * 0.5 + margin;
        self.shape.radius = radius - margin;
        let minimum = [-extent, -margin, -margin, -0.];
        let maximum = [extent, margin, margin, 0.];
        for (index, vertex) in self.shape.vertices.iter_mut().enumerate() {
            //141e846a0 selects the minimum for each set XYZ mask bit;
            //141e84690 clears W before the ordinary aa06c0 transform.
            let point = std::array::from_fn(|axis| {
                if axis == 3 {
                    0.
                } else if index & (1 << axis) != 0 {
                    minimum[axis]
                } else {
                    maximum[axis]
                }
            });
            *vertex = transform_point(frame, point);
            vertex[3] = f32::from_bits(0x3f000000 | index as u32);
        }
        let local_planes = [
            [1., 0., 0., -maximum[0]],
            [-1., 0., 0., minimum[0]],
            [0., 1., 0., -maximum[1]],
            [0., -1., 0., minimum[1]],
            [0., 0., 1., -maximum[2]],
            [0., 0., -1., minimum[2]],
        ];
        let negative_center = [-center[0], -center[1], -center[2]];
        let offsets =
            [axis, side, cross].map(|axis| dot([axis[0], axis[1], axis[2]], negative_center));
        for (out, local) in self.shape.planes.iter_mut().zip(local_planes) {
            *out = rotate_vector(frame, local);
            out[3] = ((local[0] * offsets[0] + local[2] * offsets[2]) + local[1] * offsets[1])
                + local[3];
        }
        Ok(&self.shape)
    }
}
