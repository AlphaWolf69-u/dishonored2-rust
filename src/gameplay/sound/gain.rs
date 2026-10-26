//! Output matrix gain ramps (14113ef50 / 141144890).
//! The mixer interpolates the product of voice gain and speaker gain.

pub struct GainRamp {
    target: Option<[f32; 8]>,
    lanes: [[f32; 8]; 8],
    four_steps: [f32; 8],
    frame: usize,
}
impl Default for GainRamp {
    fn default() -> Self { Self { target: None, lanes: [[0.0; 8]; 8], four_steps: [0.0; 8], frame: 0 } }
}
impl GainRamp {
    pub fn begin(&mut self, target: [f32; 8], frames: u16) {
        let start = self.target.unwrap_or(target);
        self.target = Some(target);
        self.frame = 0;
        let inverse = 1.0 / f32::from(frames);
        for channel in 0..8 {
            let step = (target[channel] - start[channel]) * inverse;
            let four = step * 4.0;
            let a = start[channel];
            let b = a + step;
            let c = step * 2.0 + a;
            let d = step * 3.0 + a;
            self.lanes[channel] = [a, b, c, d, a + four, b + four, c + four, d + four];
            self.four_steps[channel] = four;
        }
    }

    pub fn next_frame(&mut self) -> [f32; 8] {
        let lane = self.frame & 7;
        let result = std::array::from_fn(|channel| self.lanes[channel][lane]);
        self.frame += 1;
        if self.frame & 7 == 0 {
            for (lanes, four) in self.lanes.iter_mut().zip(self.four_steps) {
                for value in lanes { *value = *value + four + four; }
            }
        }
        result
    }
}
