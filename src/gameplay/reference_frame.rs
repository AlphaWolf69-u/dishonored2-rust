//! Game reference-frame binding and the common7506d0 pose/velocity kernels.
use crate::float::{add, divide, multiply, subtract};
mod basis;
pub use basis::{basis_quaternion, invert_basis, relative_binding};

/// idVec3::ToMat3 (140112000): normalize +X along the input, construct the
/// horizontal left axis, then cross them for up. Vertical inputs use the
/// executable's explicit signed frame rather than a degenerate cross product.
pub fn direction_basis(direction: bevy_math::Vec3) -> bevy_math::Mat3 {
    use bevy_math::{Mat3, Vec3};
    let horizontal = direction.x * direction.x + direction.y * direction.y;
    if horizontal.abs() > f32::MIN_POSITIVE {
        let inverse_horizontal = 1.0 / horizontal.max(f32::MIN_POSITIVE).sqrt();
        let length = horizontal + direction.z * direction.z;
        let forward = direction * (1.0 / length.max(f32::MIN_POSITIVE).sqrt());
        let left = Vec3::new(-direction.y * inverse_horizontal, direction.x * inverse_horizontal, 0.0);
        Mat3::from_cols(forward, left, forward.cross(left))
    } else {
        let sign = if direction.z < 0.0 { -1.0 } else { 1.0 };
        Mat3::from_cols(Vec3::new(0.0, 0.0, sign), Vec3::X, Vec3::new(0.0, sign, 0.0))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ReferenceFrame {
    pub position: [f32; 3],
    pub basis: [[f32; 3]; 3],
}

///10e9e0 selects the installed normal-axis row bases; other enum values use
///the Z/identity basis. These are exact permutations, not trigonometric turns.
pub const fn normal_axis_basis(axis: i32) -> [[f32; 3]; 3] {
    match axis {
        0 => [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]],
        1 => [[0., 0., 1.], [1., 0., 0.], [0., 1., 0.]],
        _ => [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    }
}

/// The 21 published words shared by an entity and its reference-frame actor.
/// Prediction caches and completion markers stay with the actor alone.
#[derive(Clone, Copy, Debug)]
pub struct PublishedFrame {
    pub pose: ReferenceFrame,
    pub scale: [f32; 3],
    pub velocities: [[f32; 3]; 2],
}

impl Default for ReferenceFrame {
    /// 18df90/74f7b0 initialize both current and predicted frame this way.
    fn default() -> Self {
        Self {
            position: [0.; 3],
            basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }
    }
}

/// Current and lazily predicted reference frames (+30/+a4/+b0), selected by
/// the actor's update marker. Entity clock and frame-marker producers stay
/// with the world owner instead of using rendering elapsed time implicitly.
pub struct ReferenceFrameCache {
    pub current: ReferenceFrame,
    pub scale: [f32; 3],
    /// The published frame's velocities (+3c/+48), distinct from the actor's
    /// pending/resolved motion (+58) used to predict its next frame.
    pub published_velocities: [[f32; 3]; 2],
    pub predicted: ReferenceFrame,
    pub marker: i32,
}
impl Default for ReferenceFrameCache {
    /// 18df30 supplies the unset frame marker, independently of world time.
    fn default() -> Self {
        Self {
            current: ReferenceFrame::default(),
            scale: [1.; 3],
            published_velocities: [[0.; 3]; 2],
            predicted: ReferenceFrame::default(),
            marker: -1,
        }
    }
}
impl ReferenceFrameCache {
    pub fn published(&self) -> PublishedFrame {
        PublishedFrame {
            pose: self.current,
            scale: self.scale,
            velocities: self.published_velocities,
        }
    }
    /// Complete18e050: publish pose, velocities and frame marker, preserving
    /// scale and the independent predicted cache. This is word copying only.
    pub fn publish(&mut self, frame: ReferenceFrame, velocities: [[f32; 3]; 2], marker: i32) {
        self.current = frame;
        self.published_velocities = velocities;
        self.marker = marker;
    }

    /// Complete750550; valid linear requests advance the entity position.
    pub fn predict_position(
        &mut self,
        source: ReferenceFrame,
        motion: &mut super::physics_actor::ResolvedMotion,
        delta: f32,
    ) {
        self.predicted.position = source.position;
        if motion.valid[0] {
            self.predicted.position = std::array::from_fn(|i| {
                add(multiply(delta, motion.velocities[0][i]), source.position[i])
            });
        }
        motion.dirty[0] = false;
    }
    /// Complete7501d0, reusing the verified axis-angle and quaternion kernels.
    pub fn predict_basis(
        &mut self,
        source: ReferenceFrame,
        motion: &mut super::physics_actor::ResolvedMotion,
        delta: f32,
    ) {
        self.predicted.basis = source.basis;
        let angular = motion.velocities[1];
        if motion.valid[1]
            && angular
                .iter()
                .any(|&v| subtract(v, 0.).abs() > f32::from_bits(0x34000000))
        {
            let length = add(
                add(
                    multiply(angular[1], angular[1]),
                    multiply(angular[0], angular[0]),
                ),
                multiply(angular[2], angular[2]),
            );
            let inverse = super::constraint_solver::inverse_length(length);
            let axis = [
                multiply(angular[0], inverse),
                multiply(angular[1], inverse),
                multiply(angular[2], inverse),
                multiply(0., inverse),
            ];
            let angle = multiply(multiply(inverse, length), delta);
            let step = super::shape_transform::rotation::axis_angle_quaternion(axis, angle);
            let old = crate::assets::physworld::rotation_quaternion(
                source.basis.map(|v| [v[0], v[1], v[2], 0.]),
            );
            let q = [
                add(
                    add(
                        subtract(multiply(old[2], step[1]), multiply(step[2], old[1])),
                        multiply(old[0], step[3]),
                    ),
                    multiply(step[0], old[3]),
                ),
                add(
                    add(
                        subtract(multiply(old[0], step[2]), multiply(step[0], old[2])),
                        multiply(old[1], step[3]),
                    ),
                    multiply(step[1], old[3]),
                ),
                add(
                    add(
                        subtract(multiply(old[1], step[0]), multiply(step[1], old[0])),
                        multiply(old[2], step[3]),
                    ),
                    multiply(step[2], old[3]),
                ),
                subtract(
                    multiply(old[3], step[3]),
                    add(
                        add(multiply(old[1], step[1]), multiply(old[0], step[0])),
                        multiply(old[2], step[2]),
                    ),
                ),
            ];
            let q = crate::assets::physworld::normalize_body_quaternion(q);
            self.predicted.basis =
                crate::assets::physworld::quaternion_basis(q).map(|v| [v[0], v[1], v[2]]);
        }
        motion.dirty[1] = false;
    }
    ///750fe0: a completed current frame wins without consuming prediction dirt.
    pub fn position(
        &mut self,
        marker: i32,
        source: ReferenceFrame,
        motion: &mut super::physics_actor::ResolvedMotion,
        delta: f32,
    ) -> [f32; 3] {
        if self.marker == marker {
            return self.current.position;
        }
        if motion.dirty[0] {
            self.predict_position(source, motion, delta);
        }
        self.predicted.position
    }
    ///750f70 uses the independent angular prediction dirty flag.
    pub fn basis(
        &mut self,
        marker: i32,
        source: ReferenceFrame,
        motion: &mut super::physics_actor::ResolvedMotion,
        delta: f32,
    ) -> [[f32; 3]; 3] {
        if self.marker == marker {
            return self.current.basis;
        }
        if motion.dirty[1] {
            self.predict_basis(source, motion, delta);
        }
        self.predicted.basis
    }
}

/// 00dce0; shared by bound frames and local-shadow frame construction.
pub fn basis_product(left: [[f32; 3]; 3], right: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    // The unrolled MULSS/ADDSS sequence varies its operand order by lane.
    // Retain it: both operands can carry different stored NaN words.
    let [[a, b, c], [d, e, f], [g, h, i]] = left;
    let [[j, k, l], [m, n, o], [p, q, r]] = right;
    let dot = |[(a, b), (c, d), (e, f)]: [(f32, f32); 3]| {
        add(add(multiply(a, b), multiply(c, d)), multiply(e, f))
    };
    [
        [
            dot([(b, m), (j, a), (c, p)]),
            dot([(b, n), (k, a), (c, q)]),
            dot([(o, b), (a, l), (r, c)]),
        ],
        [
            dot([(e, m), (d, j), (f, p)]),
            dot([(e, n), (d, k), (f, q)]),
            dot([(d, l), (o, e), (r, f)]),
        ],
        [
            dot([(h, m), (g, j), (i, p)]),
            dot([(h, n), (g, k), (i, q)]),
            dot([(g, l), (o, h), (r, i)]),
        ],
    ]
}

/// 111410 produces the game's compact row basis, distinct from Havok's padded
/// aa0390 matrix. It does not normalize the supplied binding quaternion.
pub fn quaternion_basis(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = q;
    let x2 = add(x, x);
    let y2 = add(y, y);
    let z2 = add(z, z);
    let xx = multiply(x, x2);
    let wx = multiply(w, x2);
    [
        [
            subtract(1., add(multiply(z, z2), multiply(y, y2))),
            subtract(multiply(x, y2), multiply(w, z2)),
            add(multiply(w, y2), multiply(x, z2)),
        ],
        [
            add(multiply(w, z2), multiply(x, y2)),
            subtract(1., add(multiply(z, z2), xx)),
            subtract(multiply(y, z2), wx),
        ],
        [
            subtract(multiply(x, z2), multiply(w, y2)),
            add(wx, multiply(y, z2)),
            subtract(1., add(multiply(y, y2), xx)),
        ],
    ]
}

/// 10e5e0 forms a binding quaternion from the supplied axis and half-angle,
/// then uses 111410. The axis is already prepared by its owner; it is not
/// normalized here. This is separate from Havok's SIMD rotation approximation.
pub fn axis_angle_basis(axis: [f32; 3], angle: f32) -> [[f32; 3]; 3] {
    let half = multiply(angle, 0.5);
    let sine = (half as f64).sin() as f32;
    let cosine = (half as f64).cos() as f32;
    quaternion_basis([
        multiply(sine, axis[0]),
        multiply(sine, axis[1]),
        multiply(sine, axis[2]),
        cosine,
    ])
}

/// 110c70 uses shortest-arc normalized linear interpolation. Its endpoint
/// branches copy stored quaternions directly, without normalizing them.
pub fn interpolate_quaternion(first: [f32; 4], second: [f32; 4], mut fraction: f32) -> [f32; 4] {
    if fraction <= 0. {
        return first;
    }
    if !(fraction < 1.) || first == second {
        return second;
    }
    let inverse = subtract(1., fraction);
    let dot = add(
        add(
            add(multiply(second[0], first[0]), multiply(second[1], first[1])),
            multiply(first[2], second[2]),
        ),
        multiply(first[3], second[3]),
    );
    if dot < 0. {
        fraction = f32::from_bits(fraction.to_bits() ^ 0x8000_0000);
    }
    let value = [
        add(multiply(second[0], fraction), multiply(first[0], inverse)),
        add(multiply(fraction, second[1]), multiply(inverse, first[1])),
        add(multiply(fraction, second[2]), multiply(inverse, first[2])),
        add(multiply(inverse, first[3]), multiply(fraction, second[3])),
    ];
    let mut squared = add(
        add(
            add(multiply(value[0], value[0]), multiply(value[1], value[1])),
            multiply(value[2], value[2]),
        ),
        multiply(value[3], value[3]),
    );
    if squared <= f32::MIN_POSITIVE {
        squared = f32::MIN_POSITIVE;
    }
    let scale = divide(1., squared.sqrt());
    [
        multiply(value[0], scale),
        multiply(value[1], scale),
        multiply(value[2], scale),
        multiply(scale, value[3]),
    ]
}

/// aeb1b0 computes four copies of2*acos(abs(w)) through the packed asin kernel.
pub fn quaternion_angles(q: [f32; 4]) -> [f32; 4] {
    let absolute = f32::from_bits(q[3].to_bits() & 0x7fffffff);
    super::shape_transform::rotation::asin([absolute; 4]).map(|v| {
        let difference = subtract(f32::from_bits(0x3fc90fdb), v);
        add(difference, difference)
    })
}

/// Complete18ce70 scalar angle, with its small-angle precision fallback.
pub fn quaternion_angle(q: [f32; 4]) -> f32 {
    let angle = quaternion_angles(q)[0];
    if !(angle < f32::from_bits(0x3e79db23)) {
        return angle;
    }
    let length = add(
        add(multiply(q[1], q[1]), multiply(q[0], q[0])),
        multiply(q[2], q[2]),
    );
    let magnitude = multiply(super::constraint_solver::inverse_length(length), length);
    let angle = multiply(
        super::shape_transform::rotation::asin([magnitude; 4])[0],
        2.,
    );
    if angle < f32::from_bits(0x379268a9) {
        0.
    } else {
        angle
    }
}

/// Parent entity/joint selection supplies the actual parent frame. This kernel
/// composes the retained local binding; it does not approximate joint lookup.
pub fn bind(parent: ReferenceFrame, offset: [f32; 3], orientation: [f32; 4]) -> ReferenceFrame {
    ReferenceFrame {
        position: transform_position(parent, offset),
        basis: basis_product(quaternion_basis(orientation), parent.basis),
    }
}

/// Compose a raw phase/joint frame without quaternion conversion or basis
/// normalization, as required by 753c10's retained-binding update.
pub fn compose(parent: ReferenceFrame, local: ReferenceFrame) -> ReferenceFrame {
    ReferenceFrame {
        position: transform_position(parent, local.position),
        basis: basis_product(local.basis, parent.basis),
    }
}

pub(super) fn transform_position(parent: ReferenceFrame, offset: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        let x = multiply(offset[0], parent.basis[0][i]);
        let y = multiply(offset[1], parent.basis[1][i]);
        let pair = if i == 0 { add(y, x) } else { add(x, y) };
        add(
            add(pair, multiply(offset[2], parent.basis[2][i])),
            parent.position[i],
        )
    })
}

///7506d0 derives world linear and angular velocity after obtaining a bound pose.
pub fn velocities(previous: ReferenceFrame, next: ReferenceFrame, delta: f32) -> [[f32; 3]; 2] {
    let inverse = divide(1., delta);
    let linear = std::array::from_fn(|i| {
        multiply(subtract(next.position[i], previous.position[i]), inverse)
    });
    let convert = |basis: [[f32; 3]; 3]| {
        crate::assets::physworld::rotation_quaternion(basis.map(|v| [v[0], v[1], v[2], 0.]))
    };
    let old = convert(previous.basis);
    let new = convert(next.basis);
    let q = [
        add(
            subtract(
                subtract(multiply(new[2], old[1]), multiply(old[2], new[1])),
                multiply(new[3], old[0]),
            ),
            multiply(old[3], new[0]),
        ),
        add(
            subtract(
                subtract(multiply(new[0], old[2]), multiply(old[0], new[2])),
                multiply(new[3], old[1]),
            ),
            multiply(old[3], new[1]),
        ),
        add(
            subtract(
                subtract(multiply(new[1], old[0]), multiply(old[1], new[0])),
                multiply(new[3], old[2]),
            ),
            multiply(old[3], new[2]),
        ),
        add(
            add(multiply(old[0], new[0]), multiply(old[2], new[2])),
            add(multiply(old[1], new[1]), multiply(old[3], new[3])),
        ),
    ];
    let q = crate::assets::physworld::normalize_body_quaternion(q);
    let length = add(
        add(multiply(q[1], q[1]), multiply(q[0], q[0])),
        multiply(q[2], q[2]),
    );
    let angular = if length > f32::from_bits(0x28800000) {
        let speed = divide(quaternion_angle(q), delta);
        let inverse = super::constraint_solver::inverse_length(length);
        let sign = if q[3] < 0. { 0x80000000 } else { 0 };
        std::array::from_fn(|i| {
            multiply(
                f32::from_bits(multiply(inverse, q[i]).to_bits() ^ sign),
                speed,
            )
        })
    } else {
        [0.; 3]
    };
    [linear, angular]
}

pub mod joints;
#[cfg(test)]
#[path = "reference_frame/publication_tests.rs"]
mod publication_tests;
#[cfg(test)]
#[path = "reference_frame/tests.rs"]
mod tests;
