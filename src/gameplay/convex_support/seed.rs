//! Initial penetration simplex construction and degenerate contacts (c23b50).
use super::{
    distance::ConvexDistanceSolution,
    faces::reciprocal_sqrt_estimate,
    penetration::{ConvexSupportPoint, PenetrationSupport},
};
use crate::gameplay::{constraint_solver::reciprocal, shape_transform::inverse_rotate_vector};

pub struct PenetrationSeed {
    pub points: [ConvexSupportPoint; 4],
    pub count: usize,
}

pub enum SeedOutcome {
    Ready,
    Contact(ConvexDistanceSolution),
    /// Status7 requests the caller's retry path; it is not a separation result.
    Failed,
}

pub(super) fn subtract(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(super) fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
pub(super) fn cross(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        b[2] * a[1] - a[2] * b[1],
        b[0] * a[2] - a[0] * b[2],
        b[1] * a[0] - a[1] * b[0],
        b[3] * a[3] - a[3] * b[3],
    ]
}
pub(super) fn inverse_length(squared: f32) -> f32 {
    let r = reciprocal_sqrt_estimate(squared);
    if squared <= 0. {
        0.
    } else {
        (3. - r * squared * r) * (r * 0.5)
    }
}
pub(super) fn normalize(v: [f32; 4]) -> [f32; 4] {
    let factor = inverse_length(dot(v, v));
    v.map(|v| v * factor)
}
pub(super) fn perpendicular(v: [f32; 4]) -> [f32; 4] {
    if v[1] * v[1] + v[0] * v[0] < (v[0] * v[0] + v[2] * v[2]) + 0. {
        [-v[2], 0., v[0], 0.]
    } else {
        [-v[1], v[0], 0., 0.]
    }
}
fn contact(distance: [f32; 4], normal: [f32; 4], witness: [f32; 4]) -> SeedOutcome {
    SeedOutcome::Contact(ConvexDistanceSolution {
        status: 1,
        distance,
        normal,
        witness,
    })
}

impl PenetrationSeed {
    /// Retains feature packets even when a degenerate simplex requests a retry.
    pub fn build(
        &mut self,
        support: &PenetrationSupport<'_>,
        epsilon: f32,
        tolerance: f32,
    ) -> Result<SeedOutcome, String> {
        let mut attempted = -1;
        loop {
            match self.count {
                0 => {
                    let translation = support.relative[3];
                    let direction = if dot(translation, translation) > epsilon {
                        inverse_rotate_vector(support.relative, translation)
                    } else {
                        [0., 1., 0., 0.]
                    };
                    self.points[0] = support.point(direction)?;
                    self.count = 1;
                    let squared = dot(self.points[0].difference, self.points[0].difference);
                    if !(squared > epsilon) {
                        let distance = if squared <= 0. {
                            0.
                        } else {
                            inverse_length(squared) * squared
                        };
                        return Ok(contact(
                            [distance; 4],
                            normalize(direction),
                            self.points[0].query,
                        ));
                    }
                    attempted = 0;
                }
                1 => {
                    let v = self.points[0].difference;
                    if !(dot(v, v) > epsilon) {
                        if attempted >= 0 {
                            return Ok(SeedOutcome::Failed);
                        }
                        self.count = 0;
                        continue;
                    }
                    attempted = 1;
                    let a = support.point(v)?;
                    let b = support.point(v.map(|v| -v))?;
                    let da = subtract(a.difference, v);
                    let db = subtract(b.difference, v);
                    self.points[1] = if dot(da, da) >= dot(db, db) { a } else { b };
                    self.count = 2;
                }
                2 => {
                    let line = subtract(self.points[0].difference, self.points[1].difference);
                    if !(dot(line, line) > epsilon) {
                        if attempted >= 1 {
                            return Ok(SeedOutcome::Failed);
                        }
                        self.count = 1;
                        continue;
                    }
                    attempted = 2;
                    let axis = perpendicular(line);
                    let secondary = cross(line, axis);
                    let origin = self.points[0].difference;
                    let (a, pa) = support.extent(normalize(axis), origin)?;
                    let (b, pb) = support.extent(normalize(secondary), origin)?;
                    // UCOMISS secondary,primary / JNC: unordered takes primary.
                    self.points[2] = if b >= a { pb } else { pa };
                    self.count = 3;
                }
                3 => {
                    let [a, b, c, _] = self.points.map(|p| p.difference);
                    let normal = cross(subtract(a, b), subtract(b, c));
                    let squared = dot(normal, normal);
                    // UCOMISS/JZ also takes the degenerate path for unordered.
                    if squared == 0. || squared.is_nan() {
                        if attempted < 2 {
                            self.count = 2;
                            continue;
                        }
                        let normal = normalize(perpendicular(subtract(a, b)));
                        let length = (a[1].abs() + a[0].abs()) + a[2].abs();
                        let weight =
                            reciprocal((length + length) + f32::from_bits(0x3400_0000)) * length;
                        let witness = std::array::from_fn(|i| {
                            (self.points[1].query[i] - self.points[0].query[i]) * weight
                                + self.points[0].query[i]
                        });
                        return Ok(contact([0.; 4], normal, witness));
                    }
                    attempted = 3;
                    let normal = normalize(normal);
                    let (extent, fourth) = support.extent(normal, a)?;
                    if !(extent >= tolerance) {
                        return Ok(contact([0.; 4], normal, self.triangle_witness()));
                    }
                    self.points[3] = fourth;
                    self.count = 4;
                }
                4 => {
                    let [a, b, c, d] = self.points.map(|p| p.difference);
                    let normal = cross(subtract(a, b), subtract(b, c));
                    let volume = dot(subtract(d, a), normal);
                    if volume * volume > dot(normal, normal) * epsilon {
                        return Ok(SeedOutcome::Ready);
                    }
                    if attempted >= 3 {
                        return Ok(SeedOutcome::Failed);
                    }
                    self.count = 3;
                }
                _ => return Ok(SeedOutcome::Failed),
            }
        }
    }

    fn triangle_witness(&self) -> [f32; 4] {
        let [a, b, c, _] = self.points.map(|p| p.difference);
        let n = cross(subtract(b, a), subtract(c, a));
        let weight = |u: [f32; 4], v: [f32; 4]| {
            ((u[1] * n[2] - u[2] * n[1]) * v[0] + (u[2] * n[0] - u[0] * n[2]) * v[1])
                + (u[0] * n[1] - u[1] * n[0]) * v[2]
        };
        let weights = [weight(c, b), weight(a, c), weight(b, a)];
        let denominator = (weights[1] + weights[0]) + weights[2];
        let inverse = if denominator == 0. {
            0.
        } else {
            reciprocal(denominator)
        };
        let weights = weights.map(|w| inverse * w);
        std::array::from_fn(|i| {
            (weights[1] * self.points[1].query[i] + weights[0] * self.points[0].query[i])
                + weights[2] * self.points[2].query[i]
        })
    }
}
