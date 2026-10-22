//! 226b10's scalar range sample and independent curve-mixing sample.
pub struct EffectParameterSample {
    pub scale: f32,
    pub curve_mix: f32,
}
/// 2178c0 and the first sample in 226b10 share this range evaluation.
pub fn sample_range(range: [f32; 2], seed: &mut u32) -> f32 {
    let fraction = crate::random::lcg_unit(seed);
    let first = super::normalize_small(range[0]);
    let second = super::normalize_small(range[1]);
    super::add((1. - fraction) * first, fraction * second)
}
impl EffectParameterSample {
    /// Endpoints retain their serialized order; this is not a sorted range.
    pub fn new(range: [f32; 2], seed: &mut u32) -> Self {
        Self {
            scale: sample_range(range, seed),
            curve_mix: crate::random::lcg_unit(seed),
        }
    }
}
