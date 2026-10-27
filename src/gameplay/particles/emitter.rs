//! Continuous and burst emission from 1f39b0/1f1620. Particle allocation,
//! vector fields and animated spawn shapes remain explicit binding operations.
use super::{
    parameters::EmissionRate,
    pose::EmitterPose,
    timing::{CycleClock, StartDelay},
};
use crate::{
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct EmitterDefinition {
    pub cycles: u32,
    pub duration: [f32; 2],
    pub delay: [f32; 2],
    pub rate: EmissionRate,
    pub simulation_precision: f32,
}

impl EmitterDefinition {
    /// 1efcc0 uses 60 Hz*precision for integration, then further limits the
    /// emission slice by twice the largest authored base rate/key value.
    pub fn steps(&self) -> Result<[f32; 2], String> {
        let simulation = divide(1., multiply(self.simulation_precision, 60.));
        let mut maximum = f32::from_bits(0x3a83_126f);
        for curve in [&self.rate.curve.maximum, &self.rate.curve.minimum] {
            let count = curve.times.len();
            let tripled = curve.values.len() == count * 3;
            for i in 0..count {
                let value = *curve
                    .values
                    .get(if tripled { i * 3 + 1 } else { i })
                    .ok_or("particle rate key exceeds its value array")?;
                if maximum <= value {
                    maximum = value;
                }
            }
        }
        let [first, second] = self.rate.range.range;
        let range = if first <= second { second } else { first };
        let rate_step = divide(1., multiply(multiply(maximum, range), 2.));
        let emission = if rate_step < simulation {
            rate_step
        } else {
            simulation
        };
        if !(simulation.is_finite() && simulation > 0. && emission.is_finite() && emission > 0.) {
            return Err("particle precision/rate produces an invalid simulation step".into());
        }
        Ok([simulation, emission])
    }
}

pub struct EmissionContext {
    pub pose: ReferenceFrame,
    /// The remaining part of the source emission slice, passed to 1f4e00 for
    /// immediate integration of the newly accepted particle.
    pub remaining_delta: f32,
    pub cycle_fraction: f32,
    pub cycle_seed: u32,
    pub elapsed: f32,
    /// Initial stage-space position and velocity supplied by 1f1770.
    pub event: Option<([f32; 3], [f32; 3])>,
}

pub trait EmitterBindings {
    /// Return the actual remaining free-slot count after this spawn attempt.
    fn spawn(&mut self, context: EmissionContext, seed: &mut u32) -> Result<usize, String>;
    /// 1f4ae0 resets the velocity, force and acceleration field lists in order.
    fn reset_fields(&mut self, seed: &mut u32) -> Result<(), String>;
    /// 1fb190 updates the animated spawn-shape state after cycle advancement.
    fn advance_shape(
        &mut self,
        fraction: f32,
        cycle_seed: u32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String>;
}

pub struct ParticleEmitter {
    pub definition: Arc<EmitterDefinition>,
    pub pose: EmitterPose,
    pub elapsed: f32,
    pub cycle: CycleClock,
    pub delay: StartDelay,
    pub simulation_step: f32,
    pub(crate) seed: u32,
    cycle_seed: u32,
    emission_step: f32,
    unemitted_time: f32,
}

impl ParticleEmitter {
    /// The caller supplies the stage seed AFTER initial shape/field binding.
    /// 1fd7b0 samples delay before cycle duration, then resets the cycle seed
    /// and fields. The constructor's emission accumulator starts at FLT_MAX.
    pub fn new(
        definition: Arc<EmitterDefinition>,
        seed: u32,
        pose: ReferenceFrame,
        bindings: &mut impl EmitterBindings,
    ) -> Result<Self, String> {
        let [simulation_step, emission_step] = definition.steps()?;
        let mut seed = seed;
        let delay = StartDelay::new(definition.delay, &mut seed);
        let cycle = CycleClock::new(definition.duration, &mut seed);
        let cycle_seed = crate::random::lcg_step(&mut seed);
        bindings.reset_fields(&mut seed)?;
        Ok(Self {
            definition,
            pose: EmitterPose::new(pose),
            elapsed: 0.,
            cycle,
            delay,
            simulation_step,
            seed,
            cycle_seed,
            emission_step,
            unemitted_time: f32::MAX,
        })
    }

    /// The system's cycle reset does not clear particle age or the emission
    /// accumulator. It can restart emission while existing particles survive.
    pub fn reset(&mut self, bindings: &mut impl EmitterBindings) -> Result<(), String> {
        self.delay = StartDelay::new(self.definition.delay, &mut self.seed);
        self.cycle = CycleClock::new(self.definition.duration, &mut self.seed);
        self.cycle_seed = crate::random::lcg_step(&mut self.seed);
        bindings.reset_fields(&mut self.seed)
    }

    pub fn consume_delay(&mut self, delta: f32) -> f32 {
        self.delay.consume(delta)
    }

    /// Parameter preparation reads a copy of this seed; it does not consume
    /// the mutable spawn generator or change the cycle's retained seed.
    pub fn cycle_seed(&self) -> u32 {
        self.cycle_seed
    }

    /// 1f6160 updates cycle state/fields before the animated spawn shape. Pose
    /// history is advanced afterward by the calling emission path.
    pub(crate) fn advance_clock(
        &mut self,
        delta: f32,
        bindings: &mut impl EmitterBindings,
    ) -> Result<(), String> {
        self.elapsed = add(delta, self.elapsed);
        self.cycle.advance(
            delta,
            self.definition.cycles,
            self.definition.duration,
            &mut self.seed,
        )?;
        if self.cycle.restarted {
            self.cycle_seed = crate::random::lcg_step(&mut self.seed);
            bindings.reset_fields(&mut self.seed)?;
        }
        bindings.advance_shape(self.cycle.fraction, self.cycle_seed, delta, &mut self.seed)
    }

    /// 1f39b0 emits at most once per internal slice, resets rather than subtracts
    /// the period, then advances the clock. Rate is resampled from the cycle
    /// seed copy; only accepted spawn work consumes the mutable spawn seed.
    pub fn continuous(
        &mut self,
        mut delta: f32,
        mut free_slots: usize,
        lod: usize,
        multiplier: f32,
        bindings: &mut impl EmitterBindings,
    ) -> Result<usize, String> {
        while delta > 0. && free_slots != 0 && self.cycle.can_emit(self.definition.cycles) {
            let rate = self.definition.rate.sample(
                lod,
                self.cycle.fraction,
                self.cycle_seed,
                multiplier,
            )?;
            if rate >= f32::from_bits(0x3dcc_cccd)
                && subtract(self.unemitted_time, divide(1., rate)) >= 0.
            {
                self.unemitted_time = 0.;
                let context = EmissionContext {
                    pose: self.pose.sample(),
                    remaining_delta: delta,
                    cycle_fraction: self.cycle.fraction,
                    cycle_seed: self.cycle_seed,
                    elapsed: self.elapsed,
                    event: None,
                };
                free_slots = bindings.spawn(context, &mut self.seed)?;
            }
            let step = if self.emission_step <= delta {
                self.emission_step
            } else {
                delta
            };
            self.unemitted_time = add(step, self.unemitted_time);
            self.advance_clock(step, bindings)?;
            delta = subtract(delta, step);
            self.pose.advance(step);
        }
        Ok(free_slots)
    }

    /// 1f1770 consumes every queued event in order, sampling the two direction
    /// scales once from a cycle-seed copy. Rate and accepted particles consume
    /// the mutable spawn generator. This path advances even after cycles end.
    pub fn event_burst(
        &mut self,
        delta: f32,
        mut free_slots: usize,
        lod: usize,
        multiplier: f32,
        inputs: &[super::SpawnInput],
        use_location: bool,
        velocity: &super::parameters::ScalarParameter,
        normal: &super::parameters::ScalarParameter,
        bindings: &mut impl EmitterBindings,
    ) -> Result<usize, String> {
        if !inputs.is_empty() {
            let pose = self.pose.sample();
            let mut parameter_seed = self.cycle_seed;
            let velocity = velocity.sample(self.cycle.fraction, &mut parameter_seed)?;
            let normal = normal.sample(self.cycle.fraction, &mut parameter_seed)?;
            for input in inputs {
                let count = crate::float::truncate_i32(
                    add(
                        multiply(
                            self.definition.rate.range.sample(lod, &mut self.seed),
                            multiplier,
                        ),
                        0.5,
                    )
                    .floor(),
                );
                if count <= 0 {
                    continue;
                }
                let position = if use_location {
                    std::array::from_fn(|i| subtract(input.position[i], pose.position[i]))
                } else {
                    [0.; 3]
                };
                let direction = std::array::from_fn(|i| {
                    add(
                        multiply(velocity, input.directions[0][i]),
                        multiply(normal, input.directions[1][i]),
                    )
                });
                for _ in 0..count {
                    if free_slots == 0 {
                        break;
                    }
                    free_slots = bindings.spawn(
                        EmissionContext {
                            pose,
                            remaining_delta: delta,
                            cycle_fraction: self.cycle.fraction,
                            cycle_seed: self.cycle_seed,
                            elapsed: self.elapsed,
                            event: Some((position, direction)),
                        },
                        &mut self.seed,
                    )?;
                }
            }
        }
        self.advance_clock(delta, bindings)?;
        self.pose.advance(delta);
        Ok(free_slots)
    }

    /// 1f1620's burst path samples the raw rate range with the mutable seed;
    /// it does not evaluate the continuous rate curve. Clock advancement still
    /// occurs when an admitted burst finds no free slots.
    pub fn burst(
        &mut self,
        delta: f32,
        mut free_slots: usize,
        lod: usize,
        multiplier: f32,
        bindings: &mut impl EmitterBindings,
    ) -> Result<usize, String> {
        if self.cycle.can_emit(self.definition.cycles) {
            if self.cycle.restarted {
                let count = add(
                    multiply(
                        self.definition.rate.range.sample(lod, &mut self.seed),
                        multiplier,
                    ),
                    0.5,
                )
                .floor();
                let count = crate::float::truncate_i32(count);
                if count > 0 {
                    let pose = self.pose.sample();
                    for _ in 0..count {
                        if free_slots == 0 {
                            break;
                        }
                        free_slots = bindings.spawn(
                            EmissionContext {
                                pose,
                                remaining_delta: delta,
                                cycle_fraction: self.cycle.fraction,
                                cycle_seed: self.cycle_seed,
                                elapsed: self.elapsed,
                                event: None,
                            },
                            &mut self.seed,
                        )?;
                    }
                }
            }
            self.advance_clock(delta, bindings)?;
            self.pose.advance(delta);
        }
        Ok(free_slots)
    }
}

/// 1ffff0 samples warmup with FX-parameter endpoint order, independently from
/// the emitter's delay/duration generator and its mutable spawn seed.
pub fn warmup_delta(delta: f32, range: [f32; 2], system_seed: &mut u32) -> f32 {
    add(
        crate::gameplay::effects::parameters::sample_range(range, system_seed),
        delta,
    )
}
