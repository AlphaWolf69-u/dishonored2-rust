//! 2189f0/218b80/218d10: phase samples supplied to the decal render binding.
use super::{
    curve::EffectFloatCurvePair,
    parameters::{EffectParameterSample, EffectVectorParameterSample},
    timing::{EffectTimingRanges, EffectTimings},
    vector_curve::EffectVectorCurvePair,
};
use crate::float::{multiply, subtract};
use std::sync::Arc;

pub struct DecalPhaseCurves {
    pub size: EffectVectorCurvePair<2>,
    pub depth: EffectFloatCurvePair,
    pub colour: EffectVectorCurvePair<3>,
    pub opacity: EffectFloatCurvePair,
}
pub struct DecalCurveMix {
    pub size: [f32; 2],
    pub depth: f32,
    pub colour: [f32; 3],
    pub opacity: f32,
}
#[derive(Clone, Copy)]
pub struct DecalSample {
    pub scale: [f32; 3],
    pub colour: [f32; 4],
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DecalPhase {
    FadeIn,
    Duration,
    FadeOut,
}

pub struct DecalAnimationDefinition {
    pub timing: EffectTimingRanges,
    pub looping: bool,
    pub size_range: [[f32; 2]; 2],
    pub shared_size_randomness: bool,
    pub depth_range: [f32; 2],
    pub colour_range: [[f32; 3]; 2],
    pub shared_colour_randomness: bool,
    pub opacity_range: [f32; 2],
    /// Phase declaration storage order: duration, fade-in, fade-out.
    pub phases: [DecalPhaseCurves; 3],
}

pub struct DecalAnimation {
    pub definition: Arc<DecalAnimationDefinition>,
    pub timing: EffectTimings,
    pub mix: DecalCurveMix,
    /// Distinct retained values supplied by the FX initialization routines.
    pub fade_in_base: DecalSample,
    pub duration_base: DecalSample,
    pub current: DecalSample,
}

impl Default for DecalAnimationDefinition {
    ///217c80 constructs mode4 curves, then217d50 resets the descriptor.
    ///Size fades0->1 and1->0; colour, depth and opacity remain constant one.
    fn default() -> Self {
        fn vector<const N: usize>(keys: &[(f32, [f32; N])]) -> EffectVectorCurvePair<N> {
            let mut result = EffectVectorCurvePair::default();
            for &(time, value) in keys {
                result.maximum.append_key(time, value, 4);
            }
            result
        }
        let scalar = || EffectFloatCurvePair {
            maximum: super::curve::EffectFloatCurve {
                times: vec![0.],
                values: vec![0., 1., 0.],
            },
            minimum: Default::default(),
        };
        let phase = |size| DecalPhaseCurves {
            size,
            depth: scalar(),
            colour: vector(&[(0., [1.; 3])]),
            opacity: scalar(),
        };
        Self {
            timing: EffectTimingRanges::default(),
            looping: false,
            size_range: [[1.; 2]; 2],
            shared_size_randomness: false,
            depth_range: [1.; 2],
            colour_range: [[1.; 3]; 2],
            shared_colour_randomness: false,
            opacity_range: [1.; 2],
            phases: [
                phase(vector(&[(0., [1.; 2])])),
                phase(vector(&[(0., [0.; 2]), (1., [1.; 2])])),
                phase(vector(&[(0., [1.; 2]), (1., [0.; 2])])),
            ],
        }
    }
}
pub struct DecalPhaseSample {
    pub value: DecalSample,
    ///218d10 clears activation only when time > 1, including its exact boundary.
    pub expired: bool,
}
impl DecalAnimation {
    ///218e80 initializes all parameter bases in source RNG order. Attachment
    ///sampling belongs to the render binding and occurs between depth and
    ///colour, not before or after the full set of animation parameters.
    pub fn new<T>(
        definition: Arc<DecalAnimationDefinition>,
        offsets: [f32; 4],
        seed: &mut u32,
        prepare_attachment: impl FnOnce(&mut u32) -> Result<T, String>,
    ) -> Result<(Self, T), String> {
        let timing = EffectTimings::new(&definition.timing, offsets, seed);
        let size = EffectVectorParameterSample::new(
            definition.size_range,
            definition.shared_size_randomness,
            seed,
        )?;
        //217fb0 prepares both vector bases using the actual duration/fade-in curves.
        let size_duration = definition.phases[0].size.sample(0., size.curve_mix)?;
        let size_fade_base =
            std::array::from_fn::<_, 2, _>(|i| multiply(size_duration[i], size.scale[i]));
        let size_fade = definition.phases[1].size.sample(0., size.curve_mix)?;
        let current_size = [
            multiply(size_fade[0], size_fade_base[0]),
            multiply(size_fade_base[1], size_fade[1]),
        ];
        let depth = EffectParameterSample::new(definition.depth_range, seed);
        let depth_fade_base = multiply(
            definition.phases[0].depth.sample(0., depth.curve_mix)?,
            depth.scale,
        );
        let current_depth = multiply(
            definition.phases[1].depth.sample(0., depth.curve_mix)?,
            depth_fade_base,
        );
        let attachment = prepare_attachment(seed)?;
        let colour = EffectVectorParameterSample::new(
            definition.colour_range,
            definition.shared_colour_randomness,
            seed,
        )?;
        //224810 has its own lane multiplication order, shared with other FX.
        let colour_duration = definition.phases[0].colour.sample(0., colour.curve_mix)?;
        let colour_fade_base =
            std::array::from_fn::<_, 3, _>(|i| multiply(colour_duration[i], colour.scale[i]));
        let colour_fade = definition.phases[1].colour.sample(0., colour.curve_mix)?;
        let current_colour = [
            multiply(colour_fade_base[0], colour_fade[0]),
            multiply(colour_fade[1], colour_fade_base[1]),
            multiply(colour_fade[2], colour_fade_base[2]),
        ];
        let opacity = EffectParameterSample::new(definition.opacity_range, seed);
        let opacity_fade_base = multiply(
            definition.phases[0].opacity.sample(0., opacity.curve_mix)?,
            opacity.scale,
        );
        let current_opacity = multiply(
            definition.phases[1].opacity.sample(0., opacity.curve_mix)?,
            opacity_fade_base,
        );
        let state = Self {
            definition,
            timing,
            mix: DecalCurveMix {
                size: size.curve_mix,
                depth: depth.curve_mix,
                colour: colour.curve_mix,
                opacity: opacity.curve_mix,
            },
            duration_base: DecalSample {
                scale: [size.scale[0], size.scale[1], depth.scale],
                colour: [
                    colour.scale[0],
                    colour.scale[1],
                    colour.scale[2],
                    opacity.scale,
                ],
            },
            fade_in_base: DecalSample {
                scale: [size_fade_base[0], size_fade_base[1], depth_fade_base],
                colour: [
                    colour_fade_base[0],
                    colour_fade_base[1],
                    colour_fade_base[2],
                    opacity_fade_base,
                ],
            },
            current: DecalSample {
                scale: [current_size[0], current_size[1], current_depth],
                colour: [
                    current_colour[0],
                    current_colour[1],
                    current_colour[2],
                    current_opacity,
                ],
            },
        };
        Ok((state, attachment))
    }

    ///2183a0 selects one phase per update, including only one loop restart
    ///when the clock skips several cycles. Delay retains the previous sample.
    pub fn update(&mut self, now: f32, seed: &mut u32) -> Result<Option<DecalPhaseSample>, String> {
        if self.timing.delay_end > now {
            return Ok(None);
        }
        if now >= self.timing.duration_end {
            if !self.definition.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let time = multiply(
                    subtract(now, self.timing.duration_end),
                    self.timing.fade_out_rate,
                );
                return self.sample(DecalPhase::FadeOut, time).map(Some);
            }
            self.timing
                .restart(&self.definition.timing, self.timing.duration_end, seed);
        }
        // JBE takes the duration branch for unordered fade-in bounds. Its
        // subsequent JBE returns for unordered duration bounds (2183fb/469).
        if self.timing.fade_in_end > now {
            let time = multiply(
                subtract(now, self.timing.delay_end),
                self.timing.fade_in_rate,
            );
            self.sample(DecalPhase::FadeIn, time).map(Some)
        } else if self.timing.duration_end > now {
            let time = multiply(
                subtract(now, self.timing.fade_in_end),
                self.timing.duration_rate,
            );
            self.sample(DecalPhase::Duration, time).map(Some)
        } else {
            Ok(None)
        }
    }

    /// The scheduler supplies the phase and its normalized time. Fade-out
    /// multiplies the last retained sample without accumulating it each tick.
    pub fn sample(&mut self, phase: DecalPhase, time: f32) -> Result<DecalPhaseSample, String> {
        let (index, base) = match phase {
            DecalPhase::Duration => (0, self.duration_base),
            DecalPhase::FadeIn => (1, self.fade_in_base),
            DecalPhase::FadeOut => (2, self.current),
        };
        let curves = &self.definition.phases[index];
        let size = curves.size.sample(time, self.mix.size)?;
        let depth = curves.depth.sample(time, self.mix.depth)?;
        let colour = curves.colour.sample(time, self.mix.colour)?;
        let opacity = curves.opacity.sample(time, self.mix.opacity)?;
        let red = if phase == DecalPhase::Duration {
            multiply(base.colour[0], colour[0])
        } else {
            multiply(colour[0], base.colour[0])
        };
        let value = DecalSample {
            scale: [
                multiply(base.scale[0], size[0]),
                multiply(base.scale[1], size[1]),
                multiply(depth, base.scale[2]),
            ],
            colour: [
                red,
                multiply(base.colour[1], colour[1]),
                multiply(base.colour[2], colour[2]),
                multiply(opacity, base.colour[3]),
            ],
        };
        if phase != DecalPhase::FadeOut {
            self.current = value;
        }
        Ok(DecalPhaseSample {
            value,
            expired: phase == DecalPhase::FadeOut && time > 1.,
        })
    }
}
