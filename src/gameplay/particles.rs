//! CPU particle state shared by the FX emitters. Descriptor loading and sprite
//! submission have separate owners; these routines never infer missing inputs.
pub mod emitter;
pub mod orientation;
pub mod sprite;
pub mod parameters;
pub mod pose;
pub mod space;
pub mod stage;
pub mod timing;

use crate::float::{add, divide, multiply, subtract};

/// The eight floats stored by 1dbbe0/1dbce0. Age is normalized by lifetime.
#[derive(Clone, Copy, Debug)]
pub struct ParticleState {
    pub position: [f32; 3],
    pub age: f32,
    pub velocity: [f32; 3],
    pub inverse_lifetime: f32,
}

/// Evaluated in 1dbce0's order, at the newly advanced particle age. Vector
/// acceleration has already been transformed to the stage's coordinate space.
pub struct MotionSample {
    pub linear_acceleration: f32,
    pub acceleration: [f32; 3],
    pub gravity: [f32; 3],
    pub acceleration_field: Option<[f32; 3]>,
    pub velocity_scale: [f32; 3],
}

/// 1ef520's movement descriptors. Source velocity and vector fields are
/// independent stage producers and are passed to integration by their owner.
pub struct MotionParameters {
    pub velocity: parameters::VectorParameter,
    pub acceleration: parameters::VectorParameter,
    pub acceleration_worldspace: bool,
    pub gravity: parameters::ScalarParameter,
    pub linear_acceleration: parameters::ScalarParameter,
}
impl Default for MotionParameters {
    fn default() -> Self {
        Self {
            velocity: parameters::VectorParameter::constant([1.; 3]),
            acceleration: parameters::VectorParameter::constant([0.; 3]),
            acceleration_worldspace: false,
            gravity: parameters::ScalarParameter::constant(0.),
            linear_acceleration: parameters::ScalarParameter::constant(0.),
        }
    }
}
impl MotionParameters {
    /// 1dbce0 evaluates these four parameters in this order even when a range
    /// is constant; skipping zero acceleration would change later RNG samples.
    pub fn sample(
        &self,
        age: f32,
        seed: &mut u32,
        space: &space::ParticleSpace,
        acceleration_field: Option<[f32; 3]>,
    ) -> Result<MotionSample, String> {
        let linear_acceleration = self.linear_acceleration.sample(age, seed)?;
        let acceleration = space.transform(
            self.acceleration.sample(age, seed)?,
            self.acceleration_worldspace,
        );
        let gravity = space.gravity(self.gravity.sample(age, seed)?);
        let velocity_scale = self.velocity.sample(age, seed)?;
        Ok(MotionSample {
            linear_acceleration,
            acceleration,
            gravity,
            acceleration_field,
            velocity_scale,
        })
    }
}

impl ParticleState {
    /// 1dbbe0 writes zero age and the reciprocal of the sampled lifetime.
    /// The emitter retains responsibility for its minimum-lifetime admission.
    pub fn new(position: [f32; 3], velocity: [f32; 3], lifetime: f32) -> Self {
        Self {
            position,
            age: 0.,
            velocity,
            inverse_lifetime: divide(1., lifetime),
        }
    }

    /// 1dbce0: advance age, sample motion, accumulate acceleration, then move
    /// with the resulting velocity. Expired particles leave their stored state
    /// untouched. Age exactly one remains alive until the next positive step.
    pub fn advance(
        &mut self,
        delta: f32,
        loop_life: bool,
        base_velocity: [f32; 3],
        source_velocity: [f32; 3],
        accumulated_velocity: &mut [f32; 3],
        sample: impl FnOnce(f32) -> Result<MotionSample, String>,
    ) -> Result<bool, String> {
        let mut age = add(multiply(delta, self.inverse_lifetime), self.age);
        if age > 1. {
            if !loop_life {
                return Ok(false);
            }
            age = subtract(age, age.floor());
        }
        self.age = age;
        // The source clamps squared length to FLT_MIN, not an arbitrary
        // epsilon or normalize-or-zero threshold (DAT_141ae3c60).
        let [x, y, z] = self.velocity;
        let mut squared = add(add(multiply(y, y), multiply(x, x)), multiply(z, z));
        if squared <= f32::MIN_POSITIVE {
            squared = f32::MIN_POSITIVE;
        }
        let reciprocal_length = divide(1., squared.sqrt());
        let motion = sample(age)?;
        for i in 0..3 {
            let directional = multiply(
                multiply(
                    multiply(self.velocity[i], reciprocal_length),
                    motion.linear_acceleration,
                ),
                delta,
            );
            accumulated_velocity[i] = add(directional, accumulated_velocity[i]);
            let mut acceleration = add(motion.acceleration[i], motion.gravity[i]);
            if let Some(field) = motion.acceleration_field {
                acceleration = add(acceleration, field[i]);
            }
            accumulated_velocity[i] = add(multiply(acceleration, delta), accumulated_velocity[i]);
            self.velocity[i] = add(
                add(
                    multiply(motion.velocity_scale[i], base_velocity[i]),
                    source_velocity[i],
                ),
                accumulated_velocity[i],
            );
            let displacement = if i == 2 {
                multiply(self.velocity[i], delta)
            } else {
                multiply(delta, self.velocity[i])
            };
            self.position[i] = add(displacement, self.position[i]);
        }
        Ok(true)
    }
}

/// 1f1f70's stage-space admission. Bounds reject unordered coordinates; plane
/// tests reject only a negative signed distance, matching the separate branches.
pub fn contains(position: [f32; 3], bounds: [[f32; 3]; 2], planes: &[[f32; 4]]) -> bool {
    (0..3).all(|i| bounds[0][i] <= position[i] && position[i] <= bounds[1][i])
        && planes.iter().all(|p| {
            let distance = add(
                add(
                    add(multiply(position[0], p[0]), multiply(position[1], p[1])),
                    multiply(position[2], p[2]),
                ),
                p[3],
            );
            !(distance < 0.)
        })
}

/// 1f3590's queued input to child-stage spawning. 1f1770 combines the two
/// direction channels with the stage's independent velocity parameters.
#[derive(Clone, Copy, Debug)]
pub struct SpawnInput {
    pub position: [f32; 3],
    pub directions: [[f32; 3]; 2],
}

impl SpawnInput {
    /// Convert a world input to the receiver's space before enqueueing it.
    /// The basis contains the three rows stored at stage+1c, +28 and +34.
    pub fn in_stage_space(self, worldspace: bool, origin: [f32; 3], basis: [[f32; 3]; 3]) -> Self {
        if worldspace {
            return self;
        }
        let local: [f32; 3] = std::array::from_fn(|i| subtract(self.position[i], origin[i]));
        let direction = |v: [f32; 3]| {
            std::array::from_fn(|i| {
                add(
                    add(multiply(v[0], basis[i][0]), multiply(v[1], basis[i][1])),
                    multiply(v[2], basis[i][2]),
                )
            })
        };
        // The position path sums Y before X on rows zero and two.
        let position = std::array::from_fn(|i| {
            let pair = if i == 1 {
                add(
                    multiply(local[0], basis[i][0]),
                    multiply(local[1], basis[i][1]),
                )
            } else {
                add(
                    multiply(local[1], basis[i][1]),
                    multiply(local[0], basis[i][0]),
                )
            };
            add(pair, multiply(local[2], basis[i][2]))
        });
        Self {
            position,
            directions: self.directions.map(direction),
        }
    }
}

/// 1d9b70's cycle admission. Zero means unlimited cycles. The final cycle
/// remains admitted while its elapsed time is strictly below its duration.
pub fn can_emit(cycles: u32, current: u32, age: f32, end: f32) -> bool {
    cycles == 0 || current < cycles - 1 || (age <= end && age != end)
}
