//! Standard FX parameter curves shared by particle rate, colour and placement.
//! Timing and stop/loop policy belong to the owning FX instance.
use super::parameters::{EffectParameterSample, EffectVectorParameterSample};
use super::{curve::EffectFloatCurvePair, vector_curve::EffectVectorCurvePair};
use crate::float::multiply;

#[derive(Clone, Copy)]
pub enum EnvelopePhase {
    Duration,
    FadeIn,
    FadeOut,
}

pub struct ScalarEnvelope {
    pub range: [f32; 2],
    pub duration: EffectFloatCurvePair,
    pub fade_in: EffectFloatCurvePair,
    pub fade_out: EffectFloatCurvePair,
}
impl ScalarEnvelope {
    /// 2262d0(value, 1): all phases start with a single constant-one key.
    /// Unlike an environment blend, this descriptor has no default fade ramp.
    pub fn constant(value: f32) -> Self {
        let curve = || EffectFloatCurvePair::constant(1.);
        Self {
            range: [value; 2],
            duration: curve(),
            fade_in: curve(),
            fade_out: curve(),
        }
    }

    /// 2269e0 checks all six curves, independently of the random range.
    pub fn is_constant(&self) -> Result<bool, String> {
        for pair in [&self.duration, &self.fade_in, &self.fade_out] {
            if !pair.maximum.is_constant()? || !pair.minimum.is_constant()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub struct VectorEnvelope {
    pub range: [[f32; 3]; 2],
    pub duration: EffectVectorCurvePair<3>,
    pub fade_in: EffectVectorCurvePair<3>,
    pub fade_out: EffectVectorCurvePair<3>,
    pub uniform: bool,
}
impl VectorEnvelope {
    /// 2263f0 initializes offset with zero, colour with one, and each phase's
    /// maximum curve to one. Minimum curves start empty; sampling is independent.
    pub fn constant(value: [f32; 3]) -> Self {
        let curve = || EffectVectorCurvePair::constant([1.; 3]);
        Self {
            range: [value; 2],
            duration: curve(),
            fade_in: curve(),
            fade_out: curve(),
            uniform: false,
        }
    }

    /// 226a70 uses the same six-curve admission for vector parameters.
    pub fn is_constant(&self) -> Result<bool, String> {
        for pair in [&self.duration, &self.fade_in, &self.fade_out] {
            if !pair.maximum.is_constant()? || !pair.minimum.is_constant()? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub struct ScalarEnvelopeSample {
    parameter: EffectParameterSample,
    duration_base: f32,
    pub current: f32,
}
impl ScalarEnvelopeSample {
    /// 227cd0's scalar preparation after 226b10 parameter sampling.
    pub fn new(definition: &ScalarEnvelope, seed: &mut u32) -> Result<Self, String> {
        let parameter = EffectParameterSample::new(definition.range, seed);
        let duration_base = multiply(
            definition.duration.sample(0., parameter.curve_mix)?,
            parameter.scale,
        );
        let current = multiply(
            definition.fade_in.sample(0., parameter.curve_mix)?,
            duration_base,
        );
        Ok(Self {
            parameter,
            duration_base,
            current,
        })
    }

    /// Duration/fade-in retain the current value. Fade-out uses that retained
    /// value without replacing it, so repeated ticks do not compound the fade.
    pub fn sample(
        &mut self,
        definition: &ScalarEnvelope,
        phase: EnvelopePhase,
        time: f32,
    ) -> Result<f32, String> {
        let (curve, base) = match phase {
            EnvelopePhase::Duration => (&definition.duration, self.parameter.scale),
            EnvelopePhase::FadeIn => (&definition.fade_in, self.duration_base),
            EnvelopePhase::FadeOut => (&definition.fade_out, self.current),
        };
        let value = multiply(curve.sample(time, self.parameter.curve_mix)?, base);
        if !matches!(phase, EnvelopePhase::FadeOut) {
            self.current = value;
        }
        Ok(value)
    }
}

pub struct VectorEnvelopeSample {
    parameter: EffectVectorParameterSample<3>,
    duration_base: [f32; 3],
    pub current: [f32; 3],
}
impl VectorEnvelopeSample {
    /// 224810's common vector preparation; the parameter sample is produced by
    /// 226bc0/226d30 in the caller's shared FX RNG order.
    pub fn new(definition: &VectorEnvelope, seed: &mut u32) -> Result<Self, String> {
        let parameter =
            EffectVectorParameterSample::new(definition.range, definition.uniform, seed)?;
        let duration = definition.duration.sample(0., parameter.curve_mix)?;
        let duration_base = std::array::from_fn(|i| multiply(duration[i], parameter.scale[i]));
        let fade = definition.fade_in.sample(0., parameter.curve_mix)?;
        let current = [
            multiply(duration_base[0], fade[0]),
            multiply(fade[1], duration_base[1]),
            multiply(fade[2], duration_base[2]),
        ];
        Ok(Self {
            parameter,
            duration_base,
            current,
        })
    }

    pub fn sample(
        &mut self,
        definition: &VectorEnvelope,
        phase: EnvelopePhase,
        time: f32,
    ) -> Result<[f32; 3], String> {
        let (curve, base) = match phase {
            EnvelopePhase::Duration => (&definition.duration, self.parameter.scale),
            EnvelopePhase::FadeIn => (&definition.fade_in, self.duration_base),
            EnvelopePhase::FadeOut => (&definition.fade_out, self.current),
        };
        let curve = curve.sample(time, self.parameter.curve_mix)?;
        let value = std::array::from_fn(|i| multiply(base[i], curve[i]));
        if !matches!(phase, EnvelopePhase::FadeOut) {
            self.current = value;
        }
        Ok(value)
    }
}
