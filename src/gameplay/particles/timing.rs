//! 1d9b70/1d9c20's particle emitter cycle counter. This is independent of the
//! parent FX group's delay/fade timings and the lifetime of emitted particles.
use crate::{
    float::{add, divide, multiply, subtract},
    random::lcg_unit,
};

/// The values stored at simulator+284 through +294. The emitter's start/reset
/// owner supplies the initial cycle; this type does not invent a start policy.
pub struct CycleClock {
    pub cycle: u32,
    pub elapsed: f32,
    pub fraction: f32,
    pub duration: f32,
    pub restarted: bool,
}

impl CycleClock {
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
                    let mix = lcg_unit(seed);
                    let minimum = crate::gameplay::effects::normalize_small(range[1]);
                    let maximum = crate::gameplay::effects::normalize_small(range[0]);
                    self.cycle = self.cycle.wrapping_add(1);
                    self.duration =
                        add(multiply(subtract(1., mix), minimum), multiply(maximum, mix));
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
