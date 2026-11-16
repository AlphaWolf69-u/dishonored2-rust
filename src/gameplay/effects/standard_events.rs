//! Shared standard-event cursor216c00/216b80/21a870/21a730.
//! Event construction and consumption remain with their canonical managers.

#[derive(Clone, Copy, Debug)]
pub struct StandardEventTimings {
    /// Serialized maximum, then minimum, as in arkStandardEventTimings.
    pub delay: [f32; 2],
    pub cycles: [i32; 2],
    pub period: [f32; 2],
}
impl Default for StandardEventTimings {
    ///2242d0: delay0, one cycle, period1.
    fn default() -> Self {
        Self {
            delay: [0.; 2],
            cycles: [1; 2],
            period: [1.; 2],
        }
    }
}

pub struct Schedule {
    timings: StandardEventTimings,
    next: f32,
    remaining: u32,
    period: f32,
    live: bool,
}
impl Schedule {
    pub fn new(timings: &StandardEventTimings, offset: f32, seed: &mut u32) -> Self {
        let remaining = if timings.cycles == [0, 0] {
            0
        } else {
            let unit = crate::random::lcg_unit(seed);
            let count = (unit * (timings.cycles[0] as f32 - timings.cycles[1] as f32)
                + timings.cycles[1] as f32) as i32;
            count.max(1) as u32
        };
        let next = crate::float::add(super::timing::sample_range(timings.delay, seed), offset);
        let period = super::timing::sample_range(timings.period, seed);
        Self {
            timings: *timings,
            next,
            remaining,
            period,
            live: true,
        }
    }
    pub fn live(&self) -> bool {
        self.live
    }
    pub fn stop(&mut self) {
        self.live = false;
    }
    pub fn advance(
        &mut self,
        now: f32,
        seed: &mut u32,
        mut emit: impl FnMut() -> Result<(), String>,
    ) -> Result<bool, String> {
        self.advance_with_seed(now, seed, |_| emit())
    }
    /// Some standard events consume their position/declaration random inputs
    /// while emitting; those samples precede the next period sample.
    pub fn advance_with_seed(
        &mut self,
        now: f32,
        seed: &mut u32,
        mut emit: impl FnMut(&mut u32) -> Result<(), String>,
    ) -> Result<bool, String> {
        if !self.live {
            return Ok(false);
        }
        //21a730's comparison is strictly earlier, including an exact0 start.
        while self.next < now {
            emit(seed)?;
            if self.remaining != 0 {
                self.remaining -= 1;
                if self.remaining == 0 {
                    self.live = false;
                    return Ok(false);
                }
            }
            self.next = crate::float::add(self.next, self.period);
            self.period = super::timing::sample_range(self.timings.period, seed);
            //DAT141ab3b04 bits3a83126f; a short period yields until the next
            //update instead of introducing a host tick/count cap.
            if !(f32::from_bits(0x3a83126f) <= self.period) {
                break;
            }
        }
        Ok(true)
    }
}
