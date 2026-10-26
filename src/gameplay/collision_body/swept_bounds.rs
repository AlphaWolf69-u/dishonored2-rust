use super::CollisionBody;
use crate::gameplay::{
    broadphase::PackedQuantization, motion::Motion, shape_transform::ShapeMatrix,
};

/// The three solver-info inputs consumed by 1280e20 (+20, +30 and +60).
/// Supplied by the world step, independently of rendering frame time.
pub struct SweptBoundsStep {
    pub duration: [f32; 4],
    pub rotation_test_scale: f32,
    pub contact_distance_scale: [f32; 4],
}
impl SweptBoundsStep {
    /// The three fields consumed by body bounds, projected from c9f510's
    /// defined constructor output. c8c250 retains its 16-ms duration when
    /// reapplying the world's solver settings (MOVAPS at c8c480).
    pub fn initial() -> Self {
        Self {
            duration: [f32::from_bits(0x3c83_126f); 4],
            rotation_test_scale: f32::from_bits(0x4279_ffff),
            contact_distance_scale: [f32::from_bits(0x454c_cb33); 4],
        }
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

impl CollisionBody {
    /// Complete 1280e20. calc_bounds must dispatch the selected shape's actual
    /// calcAabb; a compound's transformed local AABB is only valid for that type.
    pub fn update_swept_bounds(
        &mut self,
        motion: &Motion,
        requested_quality_flags: u32,
        tolerance: [f32; 4],
        step: &SweptBoundsStep,
        packing: &PackedQuantization,
        mut calc_bounds: impl FnMut(usize, ShapeMatrix) -> [[f32; 4]; 2],
    ) -> Result<(), String> {
        self.try_update_swept_bounds(
            motion,
            requested_quality_flags,
            tolerance,
            step,
            packing,
            |shape, matrix| Ok(calc_bounds(shape, matrix)),
        )
    }

    /// Fallible resource dispatch for the same routine. A missing shape must
    /// stop publication instead of supplying fabricated bounds.
    pub fn try_update_swept_bounds(
        &mut self,
        motion: &Motion,
        requested_quality_flags: u32,
        tolerance: [f32; 4],
        step: &SweptBoundsStep,
        packing: &PackedQuantization,
        calc_bounds: impl FnMut(usize, ShapeMatrix) -> Result<[[f32; 4]; 2], String>,
    ) -> Result<(), String> {
        let shape = self.selected_shape(3)?;
        let (bounds, _) = self.expanded_bounds(shape, motion, requested_quality_flags, tolerance, step, calc_bounds)?;
        let packed = packing.pack_bounds(bounds, [0.; 4]);
        if packed != self.bounds {
            self.status |= 0x10;
            self.bounds = packed;
        }
        Ok(())
    }

    pub(crate) fn expanded_bounds(
        &mut self,
        shape: usize,
        motion: &Motion,
        requested_quality_flags: u32,
        tolerance: [f32; 4],
        step: &SweptBoundsStep,
        mut calc_bounds: impl FnMut(usize, ShapeMatrix) -> Result<[[f32; 4]; 2], String>,
    ) -> Result<([[f32; 4]; 2], super::MotionExpansion), String> {
        let mut bounds = calc_bounds(shape, self.transform)?;
        let expansion = self.motion_expansion(motion, tolerance, step.duration);
        let contact =
            (expansion.travel[0] + expansion.angular_padding[0]) * step.contact_distance_scale[0];
        self.max_contact_distance =
            crate::float::truncate_i32(min(max(contact, 0.), 65535.)) as u16;
        let velocity = motion.angular_velocity;
        let angular_squared =
            (velocity[1] * velocity[1] + velocity[0] * velocity[0]) + velocity[2] * velocity[2];
        if requested_quality_flags & 0x380 != 0x80
            && requested_quality_flags & 0x20000 != 0
            && (f32::from_bits(0x3c33_abb1) * step.rotation_test_scale) * step.rotation_test_scale
                < angular_squared
        {
            let predicted = calc_bounds(shape, self.predicted_shape_frame(motion, step.duration))?;
            for i in 0..4 {
                let padding = max(self.transform[3][3], tolerance[i] * 0.5);
                bounds[0][i] = min(bounds[0][i], predicted[0][i]) - padding;
                bounds[1][i] = max(bounds[1][i], predicted[1][i]) + padding;
            }
        } else {
            let d = expansion.displacement;
            let (low, high) = if requested_quality_flags & 0x380 == 0x80 {
                let squared = (d[1] * d[1] + d[0] * d[0]) + d[2] * d[2];
                let half_length =
                    (crate::gameplay::constraint_solver::inverse_length(squared) * squared) * 0.5;
                (
                    d.map(|v| v * 0.5 - half_length),
                    d.map(|v| v * 0.5 + half_length),
                )
            } else {
                (d.map(|v| min(0., v)), d.map(|v| max(0., v)))
            };
            for i in 0..4 {
                bounds[0][i] = (min(low[i], expansion.cage_displacement[i])
                    - expansion.angular_padding[i])
                    + bounds[0][i];
                bounds[1][i] = (max(high[i], expansion.cage_displacement[i])
                    + expansion.angular_padding[i])
                    + bounds[1][i];
            }
        }
        Ok((bounds, expansion))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn swept_bounds_match_executed_routine() {
        for (case, line) in include_str!("../../../tests/fixtures/swept_body_bounds.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let v = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let mut body = CollisionBody::default();
            body.transform = std::array::from_fn(|i| v(i * 4));
            body.motion_to_body_orientation = v(24);
            body.packed_radius = w[40] as u16;
            let motion = Motion {
                pose: crate::gameplay::rigid_body::BodyPose {
                    center: v(16),
                    orientation: v(20),
                },
                linear_velocity: v(28),
                angular_velocity: v(32),
                linear_velocity_cage: std::array::from_fn(|i| w[36 + i] as u16),
                integration_factor: w[39] as u16,
                ..Default::default()
            };
            let local = [v(41), v(45)];
            let quantization =
                crate::gameplay::broadphase::BroadphaseQuantization::new([v(49), v(53)]);
            let step = SweptBoundsStep {
                duration: v(61),
                rotation_test_scale: f32::from_bits(w[65]),
                contact_distance_scale: v(66),
            };
            // Layout: tolerance57, duration61, scale65, contact66, flags70.
            body.status = w[71] as u16;
            body.bounds.words = w[72..76].try_into().unwrap();
            let calc = |_: usize, matrix| {
                crate::gameplay::shape_transform::transform_bounds(matrix, local)
            };
            body.update_swept_bounds(&motion, w[70], v(57), &step, &quantization.packed, calc)
                .unwrap();
            let actual = [
                u32::from(body.max_contact_distance),
                u32::from(body.status),
                body.bounds.words[0],
                body.bounds.words[1],
                body.bounds.words[2],
                body.bounds.words[3],
            ];
            assert_eq!(actual, w[76..], "case {case}");
            body.status = w[71] as u16;
            body.update_swept_bounds(&motion, w[70], v(57), &step, &quantization.packed, calc)
                .unwrap();
            assert_eq!(u32::from(body.status), w[71], "repeat case {case}");
        }
    }
}
