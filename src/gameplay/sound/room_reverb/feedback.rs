//! Four-line feedback unit (1411709b0/141170b90).
use crate::gameplay::sound::delay::Delay;

#[derive(Clone, Copy, Default)]
struct Damping {
    memory: f32,
    gain: f32,
    pole: f32,
}
impl Damping {
    fn configure(&mut self, length: u32, decay: f32, damping: f32, rate: u32) {
        // 141171c80 retains its history while replacing the coefficients.
        let amplitude = amplitude(length, decay, rate);
        let mut pole = damping_base(amplitude) * (1.0 - f64::from(damping) * f64::from(damping));
        let ceiling = f64::from_bits(0x3feff7ced916872b);
        if ceiling < pole { pole = ceiling; }
        self.gain = ((1.0 - pole) * amplitude) as f32;
        self.pole = pole as f32;
    }

    fn sample(&mut self, input: f32) -> f32 {
        self.memory = input * self.gain + self.pole * self.memory;
        self.memory
    }
}

fn amplitude(length: u32, decay: f32, rate: u32) -> f64 {
    10.0f64.powf((f64::from(length) * -3.0) / (f64::from(decay) * f64::from(rate)))
}
fn damping_base(amplitude: f64) -> f64 {
    // 1411be410 is log10; 1411b1560 is ln. Keep their operation order.
    amplitude.log10() * 20.0 * 10.0f64.ln() * 0.0125
}
fn limited_damping(longest: u32, decay: f32, damping: f32, rate: u32) -> f32 {
    // 141171ba0 limits the ratio before the per-line coefficient calculation.
    let base = damping_base(amplitude(longest, decay, rate));
    let mut difference = 1.0 - f64::from(damping) * f64::from(damping);
    if 1.0 < base * difference { difference = 1.0 / base; }
    (1.0 - difference).sqrt() as f32
}

pub struct FeedbackNetwork {
    lengths: [u32; 4],
    lines: [Delay; 4],
    damping: [Damping; 4],
}
impl FeedbackNetwork {
    pub fn new(lengths: [u32; 4], decay: f32, damping: f32, rate: u32) -> Result<Self, String> {
        let lines = [Delay::new(lengths[0])?, Delay::new(lengths[1])?, Delay::new(lengths[2])?, Delay::new(lengths[3])?];
        let mut result = Self { lengths: lengths.map(|length| length.max(4)), lines, damping: [Damping::default(); 4] };
        // Creation uses the requested lengths; 141171af0's later updates
        // reload the allocated lengths, including the four-sample minimum.
        result.configure_lengths(lengths, decay, damping, rate);
        Ok(result)
    }

    pub fn configure(&mut self, decay: f32, damping: f32, rate: u32) {
        self.configure_lengths(self.lengths, decay, damping, rate);
    }

    fn configure_lengths(&mut self, lengths: [u32; 4], decay: f32, damping: f32, rate: u32) {
        let ratio = limited_damping(lengths[3], decay, damping, rate);
        for (filter, length) in self.damping.iter_mut().zip(lengths) { filter.configure(length, decay, ratio, rate); }
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines { line.reset(); }
        for filter in &mut self.damping { filter.memory = 0.0; }
    }

    /// Accumulate into an existing output. Native SIMD groups and scalar
    /// wrap tails use different addition orders; preserve that partition.
    pub fn process_add(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), String> {
        if input.len() != output.len() { return Err("feedback reverb input/output frame counts differ".into()); }
        let mut start = 0;
        while start < input.len() {
            let chunk = self.lines.iter().fold(input.len() - start, |count, line| count.min(line.contiguous()));
            let grouped = chunk & !3;
            for offset in 0..chunk {
                let index = start + offset;
                let values: [f32; 4] = std::array::from_fn(|line| self.damping[line].sample(self.lines[line].current()));
                let [a, b, c, d] = values;
                let feedback = if offset < grouped {
                    output[index] = (a + c + output[index]) - (b + d);
                    let common = (a + b + c + d) * -0.5 + input[index];
                    [common + b, common + c, common + d, common + a]
                } else {
                    output[index] = 1.0 * a + 1.0 * c + -1.0 * b + -1.0 * d + output[index];
                    let common = -0.5 * a + -0.5 * c + -0.5 * b + -0.5 * d;
                    [b + common + input[index], c + common + input[index], d + common + input[index], a + common + input[index]]
                };
                for (line, value) in self.lines.iter_mut().zip(feedback) { line.sample(value); }
            }
            start += chunk;
        }
        Ok(())
    }
}
