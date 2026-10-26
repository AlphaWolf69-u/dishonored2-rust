//! Voice LPF/HPF frequency mapping, Butterworth coefficients and parameter
//! interpolation (141145ec0 / 141136670 / 1411367a0 / 141135f80 / 141136300).
use super::mix::power_from_bits;
mod biquad;
pub use biquad::Biquad;

#[derive(Clone, Copy, Debug)]
pub enum Kind { LowPass, HighPass }

/// 141145ec0. The ordinary voice curve is linear below control value 30,
/// then exponential. Feedback uses the lower, quantum-dependent range.
pub fn low_pass_frequency(value: f32, sample_rate: u32, quantum: u16, feedback: bool) -> f32 {
    let frequency = if feedback {
        let ceiling = ((sample_rate as f32 / f32::from(quantum) * 8.0) as i64 as u16) as f32 * 0.5;
        ceiling - (ceiling - 1.0) * 0.01 * value
    } else if value < 30.0 {
        (30.0 - value) * f32::from_bits(0x43d8aaab) + f32::from_bits(0x45dac000)
    } else {
        let bits = ((100.0 - value) * f32::from_bits(0x497e9fbf) + 1_065_353_216.0) as i64 as u32;
        power_from_bits(bits) * f32::from_bits(0x4186612a)
    };
    let maximum = sample_rate as f32 * f32::from_bits(0x3ee66666);
    if maximum >= frequency { frequency } else { maximum }
}

#[derive(Clone, Copy)]
struct Coefficients {
    b: [f32; 3],
    /// 141137100 stores the negated feedback coefficients.
    feedback: [f32; 2],
}
impl Coefficients {
    fn new(kind: Kind, value: f32, sample_rate: u32, quantum: u16, feedback: bool) -> Self {
        let value = match kind { Kind::LowPass => value, Kind::HighPass => 100.0 - value };
        let frequency = low_pass_frequency(value, sample_rate, quantum, feedback);
        let tangent = (frequency / sample_rate as f32 * std::f32::consts::PI).tan();
        let root_two = f32::from_bits(0x3fb504f3);
        match kind {
            Kind::LowPass => {
                let k = 1.0 / tangent;
                let square = k * k;
                let inverse = 1.0 / (k * root_two + 1.0 + square);
                Self { b: [inverse, inverse * 2.0, inverse],
                    feedback: [-((1.0 - square) * 2.0 * inverse), -(((1.0 - k * root_two) + square) * inverse)] }
            },
            Kind::HighPass => {
                let square = tangent * tangent;
                let sum = square + 1.0;
                let inverse = 1.0 / (sum + tangent * root_two);
                Self { b: [inverse, inverse * -2.0, inverse],
                    feedback: [(square - 1.0) * inverse * -2.0, -((sum - tangent * root_two) * inverse)] }
            },
        }
    }
}

/// One channel's filter state. Voice parameters change at quantum boundaries;
/// coefficients then advance through eight steps of 128 samples at 48 kHz.
pub struct VoiceFilter {
    kind: Kind,
    sample_rate: u32,
    quantum: u16,
    feedback: bool,
    start: f32,
    target: f32,
    step: u8,
    tail: u8,
    first: bool,
    bypass: bool,
    changed: bool,
    coefficients: Coefficients,
    input: [f32; 2],
    output: [f32; 2],
    sample: u16,
    interpolating: bool,
}
impl VoiceFilter {
    /// 141136a60's initial parameter flags and cleared channel history.
    pub fn new(kind: Kind, sample_rate: u32, quantum: u16, feedback: bool) -> Self {
        Self { kind, sample_rate, quantum, feedback, start: 0.0, target: 0.0, step: 8, tail: 0,
            first: true, bypass: true, changed: true, coefficients: Coefficients { b: [1.0, 0.0, 0.0], feedback: [0.0; 2] },
            input: [0.0; 2], output: [0.0; 2], sample: 0, interpolating: false }
    }

    pub fn set_target(&mut self, value: f32) {
        // 1410fdda0 clamps parameters before marking them for recomputation.
        let value = if value >= 0.0 { if value > 100.0 { 100.0 } else { value } } else { 0.0 };
        if value != self.target {
            self.start = (self.target - self.start) * 0.125 * f32::from(self.step) + self.start;
            self.target = value;
            self.changed = true;
        }
    }

    fn prepare(&mut self) {
        if !self.changed { return; }
        self.changed = false;
        if self.first {
            self.first = false;
            self.step = 8;
            self.start = self.target;
            self.bypass = self.target <= 0.1;
            if !self.bypass {
                self.tail = 0;
                self.coefficients = Coefficients::new(self.kind, self.target, self.sample_rate, self.quantum, self.feedback);
            }
        } else {
            self.bypass = self.start <= 0.1 && self.target <= 0.1;
            self.step = if self.bypass { 8 } else { self.tail = 0; 0 };
        }
    }

    pub fn process(&mut self, input: f32) -> f32 {
        if self.sample == 0 {
            self.prepare();
            self.interpolating = !self.bypass && self.step < 8;
        }
        if !self.bypass && self.step < 8 && self.sample % 128 == 0 {
            self.step += 1;
            let value = f32::from(self.step) * (self.target - self.start) * 0.125 + self.start;
            self.coefficients = Coefficients::new(self.kind, value, self.sample_rate, self.quantum, self.feedback);
        }
        let output = if self.bypass { input } else {
            let c = &self.coefficients;
            input * c.b[0] + self.input[1] * c.b[2] + c.b[1] * self.input[0]
                + self.output[1] * c.feedback[1] + c.feedback[0] * self.output[0]
        };
        self.input = [input, self.input[0]];
        self.output = [output, self.output[0]];
        self.sample += 1;
        if self.sample == self.quantum {
            self.sample = 0;
            if self.interpolating && self.step >= 8 {
                self.start = self.target;
                if self.target <= 0.1 { self.tail = 4; }
            } else if !self.bypass && self.tail != 0 {
                self.tail -= 1;
                if self.tail == 0 { self.bypass = true; }
            }
        }
        output
    }
}
