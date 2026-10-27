//! Authored periodic velocity orientation (1faec0, 1fb190, 1fadb0).
use super::{parameters::ScalarParameter, spawn::rotate};
use crate::float::{add, divide, multiply, subtract};

pub struct PeriodicOrientation {
    pub angle: ScalarParameter,
    pub period: ScalarParameter,
}
impl Default for PeriodicOrientation {
    /// 1f8c00 initializes both scalar parameters to constant zero.
    fn default() -> Self {
        Self {
            angle: ScalarParameter::constant(0.),
            period: ScalarParameter::constant(0.),
        }
    }
}
impl PeriodicOrientation {
    /// 1f9390 tests endpoints directly, independently of their curves.
    fn enabled(&self) -> bool {
        self.period.range[0] != 0.
            && self.period.range[1] != 0.
            && (self.angle.range[0] != self.angle.range[1] || self.angle.range[0] != 0.)
    }
    fn sample_period(&self, fraction: f32, seed: &mut u32) -> Result<f32, String> {
        let period = self.period.sample(fraction, seed)?;
        let minimum = f32::from_bits(0x38d1_b717);
        Ok(if period <= minimum { minimum } else { period })
    }
}

#[derive(Default)]
pub struct PeriodicOrientationState {
    current: [f32; 4],
    elapsed: f32,
    period: f32,
    start: [f32; 4],
    end: [f32; 4],
}

fn normalize(value: &mut [f32; 3]) -> f32 {
    let squared = add(
        add(multiply(value[1], value[1]), multiply(value[0], value[0])),
        multiply(value[2], value[2]),
    );
    let clamped = if squared <= f32::MIN_POSITIVE {
        f32::MIN_POSITIVE
    } else {
        squared
    };
    let reciprocal = divide(1., clamped.sqrt());
    *value = value.map(|v| multiply(v, reciprocal));
    multiply(reciprocal, squared)
}

fn random_axis(seed: &mut u32) -> [f32; 3] {
    let mut axis =
        std::array::from_fn(|_| subtract(multiply(crate::random::lcg_unit(seed), 2.), 1.));
    normalize(&mut axis);
    axis
}

impl PeriodicOrientationState {
    pub fn new(
        definition: &PeriodicOrientation,
        cycle_seed: u32,
        seed: &mut u32,
    ) -> Result<Self, String> {
        // The executable leaves current axis/angle unwritten until 1fb190.
        // Host memory safety: zero rotation for this otherwise indeterminate
        // first use. Endpoints and all subsequent updates follow game code.
        let mut state = Self::default();
        if definition.enabled() {
            let mut parameter_seed = cycle_seed;
            state.period = definition.sample_period(0., &mut parameter_seed)?;
            let start = random_axis(seed);
            state.start = [
                start[0],
                start[1],
                start[2],
                definition.angle.sample(0., &mut parameter_seed)?,
            ];
            let end = random_axis(seed);
            state.end = [
                end[0],
                end[1],
                end[2],
                definition.angle.sample(0., &mut parameter_seed)?,
            ];
        }
        Ok(state)
    }

    pub fn advance(
        &mut self,
        definition: &PeriodicOrientation,
        fraction: f32,
        cycle_seed: u32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String> {
        if !(f32::from_bits(0x3400_0000) < self.period) {
            return Ok(());
        }
        self.elapsed = add(self.elapsed, delta);
        let mut parameter_seed = cycle_seed;
        if self.period <= self.elapsed {
            loop {
                self.elapsed = subtract(self.elapsed, self.period);
                self.period = definition.sample_period(fraction, &mut parameter_seed)?;
                if !(self.period <= self.elapsed) {
                    break;
                }
            }
            self.start = self.end;
            let end = random_axis(seed);
            self.end = [
                end[0],
                end[1],
                end[2],
                definition.angle.sample(fraction, &mut parameter_seed)?,
            ];
        }
        let fraction = divide(self.elapsed, self.period);
        self.current = std::array::from_fn(|i| {
            add(
                multiply(
                    subtract(1., fraction),
                    crate::gameplay::effects::normalize_small(self.start[i]),
                ),
                multiply(
                    crate::gameplay::effects::normalize_small(self.end[i]),
                    fraction,
                ),
            )
        });
        Ok(())
    }

    pub fn apply(&self, velocity: [f32; 3]) -> [f32; 3] {
        let epsilon = f32::from_bits(0x3400_0000);
        if !(epsilon < self.period) {
            return velocity;
        }
        let [x, y, z, angle] = self.current;
        let mut axis = [
            subtract(multiply(z, velocity[1]), multiply(y, velocity[2])),
            subtract(multiply(x, velocity[2]), multiply(z, velocity[0])),
            subtract(multiply(y, velocity[0]), multiply(x, velocity[1])),
        ];
        if epsilon < normalize(&mut axis) {
            rotate(velocity, axis, angle)
        } else {
            velocity
        }
    }
}
