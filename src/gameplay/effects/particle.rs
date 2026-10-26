//! Particle FX preparation and publication (227cd0, 226f60, 227190).
//! The system owner allocates emitters and applies the returned publication;
//! this controller does not select a declaration or draw synthetic particles.
use super::{
    envelope::{EnvelopePhase, ScalarEnvelopeSample, VectorEnvelopeSample},
    timing::EffectTimings,
};
use crate::{
    assets::fx::particle_system::ParticleSystemAsset,
    float::{add, multiply, subtract},
    gameplay::reference_frame::ReferenceFrame,
};
use std::sync::Arc;

pub struct ParticleEffectFrame {
    pub owner: ReferenceFrame,
    pub camera: Option<ReferenceFrame>,
    /// The current local pm_screenParticleOffset value, already resolved by
    /// the config/console owner. The registered default is not repeated here.
    pub screen_offset: f32,
    pub view_mask: u8,
    pub allowed_mask: u8,
    /// The particle render-model's +1c30 bit0 also requests a +78 bounds
    /// publication, independently of this FX instance's first visible update.
    pub bounds_dirty: bool,
    pub delta: f32,
}

pub struct ParticlePublication {
    pub pose: ReferenceFrame,
    pub rate: f32,
    pub colour: [f32; 4],
    pub visible: bool,
    pub refresh_bounds: bool,
    pub stop: bool,
    pub simulation_delta: f32,
}

pub struct ParticleEffect {
    pub definition: Arc<ParticleSystemAsset>,
    pub timing: EffectTimings,
    pub active: bool,
    rate: ScalarEnvelopeSample,
    colour: VectorEnvelopeSample,
    alpha: ScalarEnvelopeSample,
    offset: VectorEnvelopeSample,
    constant_curves: bool,
    zero_offset: bool,
    first_update: bool,
    auto_stop_pending: bool,
    pose: ReferenceFrame,
    published_rate: f32,
    published_colour: [f32; 4],
    published_offset: [f32; 3],
}

impl ParticleEffect {
    /// 227cd0 consumes the system seed, initializes rate/colour/alpha/offset,
    /// then samples shared timings. System setup receives its own seed value;
    /// it must not consume the FX owner's later parameter/timing samples.
    pub fn new<T>(
        definition: Arc<ParticleSystemAsset>,
        offsets: [f32; 4],
        seed: &mut u32,
        prepare_system: impl FnOnce(&ParticleSystemAsset, u32) -> Result<T, String>,
    ) -> Result<(Self, T), String> {
        let system_seed = crate::random::lcg_step(seed);
        let system = prepare_system(&definition, system_seed)?;
        let rate = ScalarEnvelopeSample::new(&definition.rate, seed)?;
        let colour = VectorEnvelopeSample::new(&definition.colour, seed)?;
        let alpha = ScalarEnvelopeSample::new(&definition.alpha, seed)?;
        let offset = VectorEnvelopeSample::new(&definition.position.offset, seed)?;
        // 226930's derived flag depends on curves, not whether ranges differ.
        let constant_curves = definition.position.offset.is_constant()?
            && definition.rate.is_constant()?
            && definition.colour.is_constant()?
            && definition.alpha.is_constant()?;
        let zero_offset = constant_curves
            && (offset.current.iter().fold(0, |bits, v| bits | v.to_bits()) & 0x7fff_ffff) == 0;
        let published_rate = rate.current;
        let published_colour = [
            colour.current[0],
            colour.current[1],
            colour.current[2],
            alpha.current,
        ];
        let published_offset = offset.current;
        let timing = EffectTimings::new(&definition.timing, offsets, seed);
        Ok((
            Self {
                auto_stop_pending: definition.auto_stop_emitter,
                definition,
                timing,
                active: true,
                rate,
                colour,
                alpha,
                offset,
                constant_curves,
                zero_offset,
                first_update: true,
                pose: ReferenceFrame::default(),
                published_rate,
                published_colour,
                published_offset,
            },
            system,
        ))
    }

    pub fn stop(&mut self, now: f32) {
        self.timing
            .begin_fade_out(self.definition.finish_loop_on_stop, now);
    }

    /// 227a90 resamples only the requested offset/rate parameters on a loop.
    /// Colour/alpha and the pending automatic-stop flag retain their state.
    fn restart_loop(&mut self, seed: &mut u32) -> Result<(), String> {
        if self.definition.position.randomize_each_loop {
            self.offset = VectorEnvelopeSample::new(&self.definition.position.offset, seed)?;
            self.published_offset = self.offset.current;
        }
        if self.definition.randomize_rate_each_loop {
            self.rate = ScalarEnvelopeSample::new(&self.definition.rate, seed)?;
            self.published_rate = self.rate.current;
        }
        Ok(())
    }

    /// `system_finished` is the system's +1c9c bit3 result (1fca90). The FX
    /// pool retires inactive instances before their next update, not midway
    /// through this publication. Delay has no publication at all.
    pub fn update(
        &mut self,
        now: f32,
        frame: &ParticleEffectFrame,
        system_finished: bool,
        seed: &mut u32,
    ) -> Result<Option<ParticlePublication>, String> {
        if !self.active || !(self.timing.delay_end <= now) {
            return Ok(None);
        }
        let (phase, time) = if self.timing.duration_end <= now {
            if !self.definition.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                (
                    EnvelopePhase::FadeOut,
                    multiply(
                        subtract(now, self.timing.duration_end),
                        self.timing.fade_out_rate,
                    ),
                )
            } else {
                self.timing
                    .restart(&self.definition.timing, self.timing.duration_end, seed);
                self.restart_loop(seed)?;
                if self.timing.fade_in_end <= now {
                    if !(now < self.timing.duration_end) {
                        return Ok(None);
                    }
                    (
                        EnvelopePhase::Duration,
                        multiply(
                            subtract(now, self.timing.fade_in_end),
                            self.timing.duration_rate,
                        ),
                    )
                } else {
                    (
                        EnvelopePhase::FadeIn,
                        multiply(
                            subtract(now, self.timing.delay_end),
                            self.timing.fade_in_rate,
                        ),
                    )
                }
            }
        } else if self.timing.fade_in_end <= now {
            if !(now < self.timing.duration_end) {
                return Ok(None);
            }
            (
                EnvelopePhase::Duration,
                multiply(
                    subtract(now, self.timing.fade_in_end),
                    self.timing.duration_rate,
                ),
            )
        } else {
            (
                EnvelopePhase::FadeIn,
                multiply(
                    subtract(now, self.timing.delay_end),
                    self.timing.fade_in_rate,
                ),
            )
        };
        let mut stop = false;
        if matches!(phase, EnvelopePhase::FadeOut) && time > 1. {
            if self.definition.wait_system_end {
                stop = true;
                self.active = !system_finished;
            } else {
                self.active = false;
            }
        } else if !self.constant_curves {
            let rate = self.rate.sample(&self.definition.rate, phase, time)?;
            // 2278d0 evaluates rate's fade curve, then submits the retained
            // current rate. Automatic stop is applied separately below.
            self.published_rate = if matches!(phase, EnvelopePhase::FadeOut) {
                self.rate.current
            } else {
                rate
            };
            let colour = self.colour.sample(&self.definition.colour, phase, time)?;
            let alpha = self.alpha.sample(&self.definition.alpha, phase, time)?;
            self.published_colour = [colour[0], colour[1], colour[2], alpha];
            self.published_offset =
                self.offset
                    .sample(&self.definition.position.offset, phase, time)?;
        }
        if matches!(phase, EnvelopePhase::FadeOut) && self.auto_stop_pending {
            self.auto_stop_pending = false;
            self.published_rate = 0.;
        }
        self.publish(frame, stop).map(Some)
    }

    fn publish(
        &mut self,
        frame: &ParticleEffectFrame,
        stop: bool,
    ) -> Result<ParticlePublication, String> {
        let visible = frame.view_mask & frame.allowed_mask != 0;
        let mut refresh_bounds = false;
        if !visible {
            self.first_update = true;
        } else {
            let position = &self.definition.position;
            if position.screen_particle {
                let camera = frame
                    .camera
                    .ok_or("screen particle FX has no camera context")?;
                self.pose.basis = camera.basis;
                self.pose.position = std::array::from_fn(|i| {
                    add(
                        multiply(frame.screen_offset, camera.basis[0][i]),
                        camera.position[i],
                    )
                });
                if !self.zero_offset {
                    for i in 0..3 {
                        if position.worldspace {
                            self.pose.position[i] =
                                add(self.pose.position[i], self.published_offset[i]);
                        } else {
                            let order = if i == 0 { [1, 0, 2] } else { [0, 1, 2] };
                            for axis in order {
                                self.pose.position[i] = add(
                                    self.pose.position[i],
                                    multiply(self.published_offset[axis], self.pose.basis[axis][i]),
                                );
                            }
                        }
                    }
                }
            } else {
                if self.first_update || position.track_orientation {
                    self.pose.basis = frame.owner.basis;
                }
                if self.first_update || position.track_position {
                    self.pose.position = if self.zero_offset {
                        frame.owner.position
                    } else {
                        std::array::from_fn(|i| {
                            let offset = if position.worldspace {
                                self.published_offset[i]
                            } else {
                                add(
                                    add(
                                        multiply(self.published_offset[1], self.pose.basis[1][i]),
                                        multiply(self.published_offset[0], self.pose.basis[0][i]),
                                    ),
                                    multiply(self.published_offset[2], self.pose.basis[2][i]),
                                )
                            };
                            add(offset, frame.owner.position[i])
                        })
                    };
                }
            }
            refresh_bounds = self.first_update || frame.bounds_dirty;
            self.first_update = false;
        }
        Ok(ParticlePublication {
            pose: self.pose,
            rate: self.published_rate,
            colour: self.published_colour,
            visible,
            refresh_bounds,
            stop,
            simulation_delta: if visible { frame.delta } else { 0. },
        })
    }
}
