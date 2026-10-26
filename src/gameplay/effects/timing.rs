//! 216cf0/216e30 timing preparation and 216f80's stop request.
#[derive(Default)]
pub struct EffectTimingRanges {
    /// Pairs retain the source declaration's storage order.
    pub delay: [f32; 2],
    pub fade_in: [f32; 2],
    pub duration: [f32; 2],
    pub fade_out: [f32; 2],
}
pub struct EffectTimings {
    pub duration_offset: f32,
    pub fade_out_offset: f32,
    pub delay_end: f32,
    pub fade_in_rate: f32,
    pub fade_in_end: f32,
    pub duration_rate: f32,
    pub duration_end: f32,
    pub fade_out_rate: f32,
    pub fading_out: bool,
}
/// 216cf0/216e30 and particle/sound delay initializers share this sample.
pub(crate) fn sample_range(range: [f32; 2], seed: &mut u32) -> f32 {
    let fraction = crate::random::lcg_unit(seed);
    let first = super::normalize_small(range[0]);
    let second = super::normalize_small(range[1]);
    // Timing's endpoint weighting differs from 226b10's scalar parameter.
    super::add((1. - fraction) * second, first * fraction)
}
impl EffectTimings {
    /// Offsets are delay, fade-in, duration and fade-out, supplied by the parent
    /// effect. Initialization consumes four random samples even for fixed times.
    pub fn new(ranges: &EffectTimingRanges, offsets: [f32; 4], seed: &mut u32) -> Self {
        let delay_end = super::add(sample_range(ranges.delay, seed), offsets[0]);
        let fade_in = super::add(sample_range(ranges.fade_in, seed), offsets[1]);
        let mut state = Self {
            duration_offset: offsets[2],
            fade_out_offset: offsets[3],
            delay_end,
            fade_in_rate: 1. / (fade_in + f32::from_bits(0x3400_0000)),
            fade_in_end: 0.,
            duration_rate: 0.,
            duration_end: 0.,
            fade_out_rate: 0.,
            fading_out: false,
        };
        state.restart(ranges, super::add(fade_in, delay_end), seed);
        state
    }
    /// Restart only the duration/fade-out interval. Earlier phase data and the
    /// stop flag survive; this is not a fresh initialization of the effect.
    pub fn restart(&mut self, ranges: &EffectTimingRanges, start: f32, seed: &mut u32) {
        self.fade_in_end = start;
        let duration = super::add(sample_range(ranges.duration, seed), self.duration_offset);
        self.duration_end = super::add(duration, self.fade_in_end);
        self.duration_rate = 1. / (duration + f32::from_bits(0x3400_0000));
        let fade_out = super::add(sample_range(ranges.fade_out, seed), self.fade_out_offset);
        self.fade_out_rate = 1. / (fade_out + f32::from_bits(0x3400_0000));
    }
    pub fn begin_fade_out(&mut self, keep_duration_end: bool, now: f32) {
        if !self.fading_out {
            self.fading_out = true;
            if !keep_duration_end {
                self.duration_end = now;
            }
        }
    }
}
