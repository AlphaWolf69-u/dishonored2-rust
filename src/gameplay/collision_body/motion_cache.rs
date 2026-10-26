//! Full12816e0 body-frame, motion-history and retained-bounds update.
use super::{CollisionBody, SweptBoundsStep};
use crate::{
    float::{add, multiply, subtract, truncate_i32},
    gameplay::{
        body_lod::BodyLodConfig,
        broadphase::{PackedBodyBounds, PackedQuantization},
        motion::Motion,
        shape_transform::ShapeMatrix,
    },
};

/// The owning world's+750/+770 cache settings and optional+950 LOD manager,
/// alongside its shared solver step and packed-bounds conversion.
pub struct BodyCacheSettings<'a> {
    pub bounds_tolerance: [f32; 4],
    pub cache_margin: f32,
    pub lod: Option<BodyLodConfig>,
    pub step: &'a SweptBoundsStep,
    pub packing: &'a PackedQuantization,
}

fn max(first: f32, second: f32) -> f32 {
    if first > second {
        first
    } else {
        second
    }
}
fn min(first: f32, second: f32) -> f32 {
    if first < second {
        first
    } else {
        second
    }
}
fn squared(v: [f32; 4]) -> f32 {
    add(
        add(multiply(v[1], v[1]), multiply(v[0], v[0])),
        multiply(v[2], v[2]),
    )
}
fn historical_length(value: f32) -> f32 {
    let value = add(value, f32::from_bits(0x2880_0000));
    // Machine MULPS computes estimate*0.5 before multiplying the correction;
    // use the already verified inverse-length kernel with that exact grouping.
    multiply(
        crate::gameplay::constraint_solver::inverse_length(value),
        value,
    )
}
fn quantized_distance(value: f32) -> u16 {
    // CVTTPS2DQ xor -(2^31<=value), then signed integer clamps.
    let integer = truncate_i32(value) ^ if value >= 2147483648. { -1 } else { 0 };
    integer.clamp(0, 65535) as u16
}
fn words(bounds: PackedBodyBounds) -> [u64; 2] {
    [
        u64::from(bounds.words[0]) | (u64::from(bounds.words[1]) << 32),
        u64::from(bounds.words[2]) | (u64::from(bounds.words[3]) << 32),
    ]
}

impl CollisionBody {
    /// The source captures the selected channel-3 shape before frame work.
    /// None means its actual null state, never an unreadable/missing resource.
    /// Geometry errors remain errors; a null shape leaves all body fields intact.
    pub fn update_from_motion(
        &mut self,
        motion: &Motion,
        shape: Option<usize>,
        quality_flags: u32,
        settings: &BodyCacheSettings<'_>,
        calc_bounds: impl FnMut(usize, ShapeMatrix) -> Result<[[f32; 4]; 2], String>,
    ) -> Result<bool, String> {
        let step = settings.step;
        let old_angle = historical_length(squared(std::array::from_fn(|i| {
            multiply(step.duration[i], motion.previous_step_angular_velocity[i])
        })));
        let angular_distance = multiply(
            old_angle,
            f32::from_bits(u32::from(self.packed_radius) << 16),
        );
        let bias = f32::from_bits(0x3f7f_be77);
        let tim_angle = quantized_distance(min(
            add(multiply(old_angle, f32::from_bits(0x4722_f8e0)), bias),
            65535.,
        ));
        let angular_tim = quantized_distance(add(
            multiply(angular_distance, step.contact_distance_scale[0]),
            bias,
        ));
        let previous_linear = historical_length(squared(motion.previous_step_linear_velocity));
        let max_tim = quantized_distance(add(
            multiply(
                add(
                    multiply(previous_linear, step.duration[0]),
                    angular_distance,
                ),
                step.contact_distance_scale[0],
            ),
            bias,
        ));
        let b = self.motion_to_body_orientation;
        let m = motion.pose.orientation;
        let q = [
            add(
                subtract(
                    subtract(multiply(m[2], b[1]), multiply(b[2], m[1])),
                    multiply(m[3], b[0]),
                ),
                multiply(b[3], m[0]),
            ),
            add(
                subtract(
                    subtract(multiply(m[0], b[2]), multiply(b[0], m[2])),
                    multiply(m[3], b[1]),
                ),
                multiply(b[3], m[1]),
            ),
            add(
                subtract(
                    subtract(multiply(m[1], b[0]), multiply(b[1], m[0])),
                    multiply(m[3], b[2]),
                ),
                multiply(b[3], m[2]),
            ),
            // SHUFPS4e/ADDPs then SHUFPSb1/ADDPS reduces as two pairs.
            add(
                add(multiply(b[0], m[0]), multiply(b[2], m[2])),
                add(multiply(b[1], m[1]), multiply(b[3], m[3])),
            ),
        ];
        let basis = crate::assets::physworld::quaternion_basis(q);
        let local = [
            self.transform[0][3],
            self.transform[1][3],
            self.transform[2][3],
        ];
        let position = std::array::from_fn(|i| {
            subtract(
                motion.pose.center[i],
                add(
                    add(
                        multiply(local[1], basis[1][i]),
                        multiply(local[0], basis[0][i]),
                    ),
                    multiply(local[2], basis[2][i]),
                ),
            )
        });
        let Some(shape) = shape else {
            return Ok(false);
        };
        self.set_transform([basis[0], basis[1], basis[2], position]);
        self.tim_angle = tim_angle;
        self.max_tim_distance = max_tim;
        self.angular_tim_distance = angular_tim;
        let (mut bounds, expansion) = self.expanded_bounds(
            shape,
            motion,
            quality_flags,
            settings.bounds_tolerance,
            step,
            calc_bounds,
        )?;
        if let Some(lod) = settings.lod {
            let old = f32::from_bits(u32::from(self.packed_average_surface_velocity) << 16);
            let speed = expansion.speed_bound[0];
            let average = max(
                add(
                    multiply(lod.average_velocity_gain, subtract(speed, old)),
                    old,
                ),
                speed,
            );
            self.packed_average_surface_velocity =
                ((average.to_bits() as i32) >> 16).clamp(i16::MIN as i32, i16::MAX as i32) as u16;
            let fast = self.lod_flags & 1 != 0;
            let threshold = if fast {
                lod.fast_to_slow_threshold
            } else {
                lod.slow_to_fast_threshold
            };
            // COMISS/JBE admits unordered inputs to the slow branch too.
            if !(average > threshold) {
                if fast {
                    self.status |= 0x40;
                    self.lod_flags &= !1;
                }
            } else if !fast {
                self.status |= 0x40;
                self.lod_flags |= 1;
            }
        }
        let mut packed = settings.packing.pack_bounds(bounds, [0.; 4]);
        let margin = settings.cache_margin;
        if margin > 0. {
            let velocity = max(
                squared(motion.previous_step_linear_velocity),
                squared(motion.linear_velocity),
            );
            let threshold = multiply(step.rotation_test_scale, margin);
            let new = words(packed);
            let old = words(self.bounds);
            let mask = 0x8000008000800000u64;
            // UCOMISS/JNC skips retention only for ordered >=; unordered
            // velocity inputs still enter the source's cached-bounds path.
            if !(velocity >= multiply(threshold, threshold))
                && (new[0].wrapping_sub(0x10001000001) | 0x7ffef07ffe7ffef0u64.wrapping_sub(new[1]))
                    & mask
                    == 0
            {
                if (old[1].wrapping_sub(new[1]) | new[0].wrapping_sub(old[0])) & mask == 0 {
                    packed = self.bounds;
                } else {
                    for i in 0..4 {
                        bounds[0][i] = subtract(bounds[0][i], margin);
                        bounds[1][i] = add(bounds[1][i], margin);
                    }
                    packed = settings.packing.pack_bounds(bounds, [0.; 4]);
                }
            }
        }
        if packed != self.bounds {
            self.status |= 0x10;
            self.bounds = packed;
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "motion_cache/tests.rs"]
mod tests;
