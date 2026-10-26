//! 1d9b70/1d9c20's particle emitter cycle counter. This is independent of the
//! parent FX group's delay/fade timings and the lifetime of emitted particles.
use crate::{
    float::{add, divide, subtract},
    gameplay::effects::timing::sample_range,
};

/// The values stored at simulator+284 through +294. The emitter's start/reset
/// owner resets the counter before resetting its cycle seed and vector fields.
pub struct CycleClock {
    pub cycle: u32,
    pub elapsed: f32,
    pub fraction: f32,
    pub duration: f32,
    pub restarted: bool,
}

impl CycleClock {
    /// 1d9b90 samples duration once, before 1f3bf0's cycle seed/vector-field
    /// reset. Initialization does not consume a delay or warmup sample.
    pub fn new(duration: [f32; 2], seed: &mut u32) -> Self {
        Self {
            cycle: 0,
            elapsed: 0.,
            fraction: 0.,
            duration: sample_range(duration, seed),
            restarted: true,
        }
    }
    pub fn can_emit(&self, cycles: u32) -> bool {
        super::can_emit(cycles, self.cycle, self.elapsed, self.duration)
    }

    /// `range` follows the descriptor's maximum/minimum order. The source
    /// advances the shared RNG for every crossed cycle, including fixed ranges.
    pub fn advance(
        &mut self,
        delta: f32,
        cycles: u32,
        range: [f32; 2],
        seed: &mut u32,
    ) -> Result<(), String> {
        self.elapsed = add(delta, self.elapsed);
        if cycles == 0 || self.cycle < cycles - 1 || self.elapsed < self.duration {
            self.restarted = self.duration <= self.elapsed;
            if self.restarted {
                loop {
                    self.elapsed = subtract(self.elapsed, self.duration);
                    self.cycle = self.cycle.wrapping_add(1);
                    self.duration = sample_range(range, seed);
                    if !(self.duration <= self.elapsed && self.can_emit(cycles)) {
                        break;
                    }
                    // A malformed unlimited emitter would spin forever in
                    // the source. Report it rather than fabricate a duration.
                    if cycles == 0 && self.duration <= 0. && range[0] <= 0. && range[1] <= 0. {
                        return Err("unlimited particle cycle has no positive duration".into());
                    }
                }
            }
        }
        if self.duration <= self.elapsed {
            self.elapsed = self.duration;
        }
        self.fraction = if self.duration <= f32::from_bits(0x3400_0000) {
            1.
        } else {
            divide(self.elapsed, self.duration)
        };
        Ok(())
    }
}

pub struct StartDelay {
    remaining: f32,
}
impl StartDelay {
    /// 1f3c30 samples the descriptor delay independently of its cycle duration.
    pub fn new(range: [f32; 2], seed: &mut u32) -> Self {
        Self {
            remaining: sample_range(range, seed),
        }
    }

    /// 1f2240 removes only the part of this frame still covered by the delay.
    /// Its separate comparisons preserve the source's unordered-input branch.
    pub fn consume(&mut self, delta: f32) -> f32 {
        let consumed = if delta <= self.remaining {
            delta
        } else {
            self.remaining
        };
        self.remaining = subtract(self.remaining, consumed);
        subtract(delta, consumed)
    }

    pub fn remaining(&self) -> f32 {
        self.remaining
    }
}
