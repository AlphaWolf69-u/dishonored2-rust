//! 2189f0/218b80/218d10: phase samples supplied to the decal render binding.
use super::{
    curve::EffectFloatCurvePair,
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
pub struct DecalPhaseSample {
    pub value: DecalSample,
    ///218d10 clears activation only when time > 1, including its exact boundary.
    pub expired: bool,
}
impl DecalAnimation {
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
