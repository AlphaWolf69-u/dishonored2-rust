//! Particle attractors (arkAttractorDesc). Each stage keeps one 0x68-byte
//! state per attractor at stage +0x1c8: 1cdff0 resets it when the stage is
//! created, 1ce190 moves it once per simulation chunk after the field
//! preparations, and the integration loop of 1f5620 adds 1cda40's pull,
//! scaled by m_strengthOverLife (1cdfe0), to each particle's accumulated
//! velocity before 1dbce0 advances it.
use super::space::ParticleSpace;
use crate::{
    assets::fx::particle_attractor::ParticleAttractor,
    assets::fx::particle_stage::SpawnVector,
    float::{add, divide, multiply, subtract},
};

pub struct AttractorState {
    position: [f32; 3],
    cut_off_squared: f32,
    strength: f32,
    maximum_strength: f32,
    target_radius: f32,
    tolerance_squared: f32,
    inverse_tolerance_squared: f32,
    deceleration: f32,
    /// +0x30: the stage seed at reset. Every preparation restarts its scalar
    /// samples from this copy, so their random mix stays fixed.
    seed: u32,
    /// +0x34: seconds since reset; the scalar parameters' sample time.
    time: f32,
    /// +0x38: integral of the sampled m_velocity.
    travelled: [f32; 3],
    /// +0x44/+0x50/+0x5c/+0x60: the random offset, its velocity towards the
    /// next random target, the time into the current period and the period.
    offset: [f32; 3],
    offset_velocity: [f32; 3],
    offset_elapsed: f32,
    offset_period: f32,
}

/// 1f6ca0: sample an arkVec3 and move it into the stage's space.
fn sample_vector(
    vector: &SpawnVector,
    space: &ParticleSpace,
    seed: &mut u32,
    time: f32,
) -> Result<[f32; 3], String> {
    Ok(space.transform(vector.value.sample(time, seed)?, vector.worldspace))
}

impl AttractorState {
    /// 1cdff0.
    pub fn new(definition: &ParticleAttractor, seed: u32) -> Self {
        let [first, second] = definition.random_period;
        let tolerance_squared = multiply(definition.tolerance_distance, definition.tolerance_distance);
        Self {
            position: [0.; 3],
            cut_off_squared: 0.,
            strength: 0.,
            maximum_strength: 0.,
            target_radius: 0.,
            tolerance_squared,
            inverse_tolerance_squared: divide(1., tolerance_squared),
            deceleration: definition.deceleration_factor,
            seed,
            time: 0.,
            travelled: [0.; 3],
            offset: [0.; 3],
            offset_velocity: [0.; 3],
            offset_elapsed: 0.,
            offset_period: if first <= 0. || second <= 0. { f32::MAX } else { 0. },
        }
    }

    /// 1ce190. `cycle_seed` is the copy of stage +0x278 the field
    /// preparations of the same chunk have already advanced.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        definition: &ParticleAttractor,
        origin: [f32; 3],
        space: &ParticleSpace,
        fraction: f32,
        cycle_seed: &mut u32,
        stage_seed: &mut u32,
        delta: f32,
    ) -> Result<(), String> {
        self.time = add(delta, self.time);
        let mut seed = self.seed;
        // 1ce2e0
        let cut_off = definition.cut_off_distance.sample(self.time, &mut seed)?;
        let cut_off = multiply(
            definition.cut_off_distance_over_cycle.sample_particle_rate(fraction, cycle_seed)?,
            cut_off,
        );
        self.cut_off_squared = multiply(cut_off, cut_off);
        let target = definition.target_radius.sample(self.time, &mut seed)?;
        self.target_radius = multiply(
            definition.target_radius_over_cycle.sample_particle_rate(fraction, cycle_seed)?,
            target,
        );
        let strength = definition.strength.sample(self.time, &mut seed)?;
        self.strength = multiply(
            definition.strength_over_cycle.sample_particle_rate(fraction, cycle_seed)?,
            strength,
        );
        self.maximum_strength = multiply(self.strength, definition.max_strength_factor);
        // 1cde40: the over-cycle velocity term reads m_velocity again.
        let velocity = sample_vector(&definition.velocity, space, &mut seed, self.time)?;
        let cycle_velocity = sample_vector(&definition.velocity, space, cycle_seed, fraction)?;
        let position = sample_vector(&definition.position, space, &mut seed, self.time)?;
        let cycle_position = sample_vector(&definition.position_over_cycle, space, cycle_seed, fraction)?;
        for i in 0..3 {
            self.travelled[i] = add(multiply(add(velocity[i], cycle_velocity[i]), delta), self.travelled[i]);
            let local = add(add(position[i], cycle_position[i]), self.travelled[i]);
            self.position[i] = add(origin[i], local);
        }
        self.advance_offset(definition, space, fraction, stage_seed, delta)?;
        for i in 0..3 {
            self.position[i] = add(self.offset[i], self.position[i]);
        }
        Ok(())
    }

    /// 1ce3d0: move the random offset towards its target; at the end of a
    /// period, 1ce080 draws the next period and target.
    fn advance_offset(
        &mut self,
        definition: &ParticleAttractor,
        space: &ParticleSpace,
        fraction: f32,
        seed: &mut u32,
        delta: f32,
    ) -> Result<(), String> {
        self.offset_elapsed = add(delta, self.offset_elapsed);
        if self.offset_elapsed < self.offset_period {
            self.move_offset(delta);
            return Ok(());
        }
        self.offset_elapsed = if self.offset_period <= 0. {
            0.
        } else {
            self.offset_elapsed % self.offset_period
        };
        self.move_offset(subtract(delta, self.offset_elapsed));
        // 1ce080
        self.offset_period = crate::gameplay::effects::timing::sample_range(definition.random_period, seed);
        let target = self.random_target(definition, space, fraction, seed)?;
        let inverse = divide(1., self.offset_period);
        for i in 0..3 {
            self.offset_velocity[i] = multiply(subtract(target[i], self.offset[i]), inverse);
        }
        self.move_offset(self.offset_elapsed);
        Ok(())
    }

    fn move_offset(&mut self, time: f32) {
        for i in 0..3 {
            self.offset[i] = add(multiply(time, self.offset_velocity[i]), self.offset[i]);
        }
    }

    /// 1cdbe0: a random direction scaled by m_randomRadius, plus the two
    /// random-offset vectors, all drawn from the stage seed.
    fn random_target(
        &self,
        definition: &ParticleAttractor,
        space: &ParticleSpace,
        fraction: f32,
        seed: &mut u32,
    ) -> Result<[f32; 3], String> {
        let radius = definition.random_radius.sample(self.time, seed)?;
        let radius = multiply(radius, definition.random_radius_over_cycle.sample_particle_rate(fraction, seed)?);
        let axis = |seed: &mut u32| subtract(multiply(crate::random::lcg_unit(seed), 2.), 1.);
        let x = axis(seed);
        let y = axis(seed);
        let z = axis(seed);
        let squared = add(add(multiply(y, y), multiply(x, x)), multiply(z, z)).max(f32::MIN_POSITIVE);
        let inverse = divide(1., squared.sqrt());
        let mut target = [x, y, z].map(|value| multiply(multiply(value, inverse), radius));
        let offset = sample_vector(&definition.random_offset, space, seed, self.time)?;
        target = std::array::from_fn(|i| add(offset[i], target[i]));
        let offset = sample_vector(&definition.random_offset_over_cycle, space, seed, fraction)?;
        Ok(std::array::from_fn(|i| add(target[i], offset[i])))
    }

    /// 1cda40: the pull on a particle at `position` moving with `velocity`,
    /// or None beyond the cut-off distance.
    pub fn pull(&self, position: [f32; 3], velocity: [f32; 3]) -> Option<[f32; 3]> {
        let mut d: [f32; 3] = std::array::from_fn(|i| subtract(self.position[i], position[i]));
        let mut squared = add(add(multiply(d[1], d[1]), multiply(d[0], d[0])), multiply(d[2], d[2]));
        if !(squared <= self.cut_off_squared) {
            return None;
        }
        if self.target_radius <= 0. {
            if f32::from_bits(0x3400_0000) < squared {
                let inverse = divide(1., squared.max(f32::MIN_POSITIVE).sqrt());
                d = d.map(|value| multiply(value, inverse));
            }
        } else {
            let length = squared.sqrt();
            let error = subtract(length, self.target_radius);
            squared = multiply(error, error);
            let inverse = divide(1., length.copysign(error));
            d = d.map(|value| multiply(value, inverse));
        }
        let denominator = if self.tolerance_squared <= squared { squared } else { self.tolerance_squared };
        let mut force = divide(self.strength, denominator);
        if self.maximum_strength <= force {
            force = self.maximum_strength;
        }
        let along = add(
            add(multiply(d[1], velocity[1]), multiply(d[0], velocity[0])),
            multiply(d[2], velocity[2]),
        );
        if along < 0. {
            force = multiply(force, self.deceleration);
        }
        if multiply(squared, self.inverse_tolerance_squared) < 1. {
            force = multiply(multiply(force, squared), self.inverse_tolerance_squared);
        }
        Some(d.map(|value| multiply(value, force)))
    }
}
