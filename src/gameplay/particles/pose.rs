//! Particle emitter pose history (1d9df0..1d9fd0). Position interpolation is
//! independent of the normalized quaternion interpolation used for orientation.
use crate::{
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::{
        basis_quaternion, interpolate_quaternion, quaternion_basis, ReferenceFrame,
    },
};

pub struct EmitterPose {
    pub sampled: ReferenceFrame,
    start: [f32; 3],
    displacement: [f32; 3],
    previous_orientation: [f32; 4],
    target_orientation: [f32; 4],
    fraction: f32,
    rate: f32,
}

impl EmitterPose {
    /// Start from the actual first published owner/camera pose. The first-frame
    /// warmup path sets both history endpoints to this pose before emission.
    pub fn new(pose: ReferenceFrame) -> Self {
        let orientation = basis_quaternion(pose.basis);
        Self {
            sampled: pose,
            start: pose.position,
            displacement: [0.; 3],
            previous_orientation: orientation,
            target_orientation: orientation,
            fraction: 0.,
            rate: 0.,
        }
    }

    /// 1d9ec0 receives the result of 110080's reference-frame conversion.
    pub fn set_orientation(&mut self, orientation: [f32; 4]) {
        self.target_orientation = orientation;
    }
    pub fn set_basis(&mut self, basis: [[f32; 3]; 3]) {
        self.set_orientation(basis_quaternion(basis));
    }
    /// 1d9ee0, called independently of the position history update.
    pub fn commit_orientation(&mut self) {
        self.previous_orientation = self.target_orientation;
    }
    /// 1d9f00 changes the start position without clearing other history fields.
    pub fn set_position(&mut self, position: [f32; 3]) {
        self.start = position;
    }
    /// 1d9eb0 resets only the interpolation fraction.
    pub fn reset_fraction(&mut self) {
        self.fraction = 0.;
    }

    /// 1d9f90 discards translation interpolation across a >10 metre jump.
    pub fn target_position(&mut self, target: [f32; 3]) {
        self.displacement = std::array::from_fn(|i| subtract(target[i], self.start[i]));
        let [x, y, z] = self.displacement;
        if add(add(multiply(y, y), multiply(x, x)), multiply(z, z)) > 100. {
            self.displacement = [0.; 3];
            self.start = target;
        }
    }

    /// 1d9f20 collapses both endpoints for tiny/nonpositive frame durations;
    /// it does not insert a synthetic minimum duration or reset the fraction.
    pub fn set_duration(&mut self, delta: f32) {
        if delta > f32::from_bits(0x3400_0000) {
            self.rate = divide(1., delta);
        } else {
            self.rate = 0.;
            self.start = [
                add(self.start[0], self.displacement[0]),
                add(self.displacement[1], self.start[1]),
                add(self.start[2], self.displacement[2]),
            ];
            self.displacement = [0.; 3];
            self.commit_orientation();
        }
    }

    /// 1d9e00 samples before emission; translation can extrapolate while the
    /// orientation helper retains its own endpoint clamps.
    pub fn sample(&mut self) -> ReferenceFrame {
        self.sampled = ReferenceFrame {
            position: std::array::from_fn(|i| {
                add(multiply(self.fraction, self.displacement[i]), self.start[i])
            }),
            basis: quaternion_basis(interpolate_quaternion(
                self.previous_orientation,
                self.target_orientation,
                self.fraction,
            )),
        };
        self.sampled
    }

    /// 1d9df0 advances after the corresponding emitter clock step.
    pub fn advance(&mut self, delta: f32) {
        self.fraction = add(multiply(delta, self.rate), self.fraction);
    }
}
