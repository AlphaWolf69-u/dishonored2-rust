//! General effect biquad coefficients and four-sample expansion.
//! 14116e1b0 / 141137100 / 14116ea60 / 14116ece0.
use super::Coefficients;

#[derive(Clone)]
pub struct Biquad {
    coefficients: Coefficients,
    matrix: [f32; 32],
    input: [f32; 2],
    output: [f32; 2],
}
impl Biquad {
    pub fn new(kind: u32, rate: u32, frequency: f32, gain_db: f32, q: f32) -> Result<Self, String> {
        let coefficients = coefficients(kind, rate, frequency, gain_db, q)?;
        Ok(Self { matrix: matrix(coefficients), coefficients, input: [0.0; 2], output: [0.0; 2] })
    }

    pub fn configure(&mut self, kind: u32, rate: u32, frequency: f32, gain_db: f32, q: f32) -> Result<(), String> {
        let coefficients = coefficients(kind, rate, frequency, gain_db, q)?;
        self.matrix = matrix(coefficients);
        self.coefficients = coefficients;
        Ok(())
    }

    pub fn reset(&mut self) { self.input = [0.0; 2]; self.output = [0.0; 2]; }

    fn scalar(&mut self, sample: f32) -> f32 {
        let c = self.coefficients;
        let result = sample * c.b[0] + self.input[1] * c.b[2] + c.b[1] * self.input[0]
            + self.output[1] * c.feedback[1] + c.feedback[0] * self.output[0];
        self.input = [sample, self.input[0]];
        self.output = [result, self.output[0]];
        result
    }

    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        let alignment = samples.as_ptr() as usize & 15;
        let prefix = if alignment == 0 { 0 } else { ((16 - alignment) / 4).min(samples.len()) };
        for sample in &mut samples[..prefix] { *sample = self.scalar(*sample); }
        let mut chunks = samples[prefix..].chunks_exact_mut(4);
        for chunk in &mut chunks {
            let [x0, x1, x2, x3] = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let [x_1, x_2] = self.input;
            let [y_1, y_2] = self.output;
            let m = &self.matrix;
            let y0 = y_2 * m[28] + m[24] * y_1 + m[4] * x2 + m[0] * x0
                + m[20] * x_2 + m[16] * x_1 + x0 * m[12] + x1 * m[8];
            let y1 = y_2 * m[29] + m[25] * y_1 + m[5] * x2 + m[1] * x1
                + m[21] * x_2 + m[17] * x_1 + x0 * m[13] + x1 * m[9];
            let y2 = y_2 * m[30] + m[26] * y_1 + m[6] * x2 + m[2] * x2
                + m[22] * x_2 + m[18] * x_1 + x0 * m[14] + x1 * m[10];
            let y3 = y_2 * m[31] + m[27] * y_1 + m[7] * x2 + m[3] * x3
                + m[23] * x_2 + m[19] * x_1 + x0 * m[15] + x1 * m[11];
            chunk.copy_from_slice(&[y0, y1, y2, y3]);
            self.input = [x3, x2];
            self.output = [y3, y2];
        }
        for sample in chunks.into_remainder() { *sample = self.scalar(*sample); }
    }
}

fn matrix(coefficients: Coefficients) -> [f32; 32] {
    let [b0, b1, b2] = coefficients.b;
    let [a1, a2] = coefficients.feedback;
    let t1 = b0 * a1 + b1;
    let t2 = t1 * a1 + b2 + b0 * a2;
    let b1a2 = b1 * a2;
    let a1a2 = a1 * a2;
    let t3 = b1 * a1 + b2;
    let t4 = t3 * a1 + b1a2;
    let a1square = a1 * a1;
    [b0, b0, b0, b0, 0.0, 0.0, 0.0, t1,
        0.0, 0.0, t1, t2, 0.0, t1, t2, a1a2 * b0 + t2 * a1 + b1a2,
        b1, t3, t4, t4 * a1 + b2 * a2 + b1a2 * a1,
        b2, b2 * a1, b2 * a1 * a1 + b2 * a2, (b2 * 2.0 * a2 + b2 * a1 * a1) * a1,
        a1, a1square + a2, (a2 * 2.0 + a1square) * a1,
        (a2 * 3.0 + a1square) * a1 * a1 + a2 * a2,
        a2, a1a2, a1a2 * a1 + a2 * a2, (a2 * 2.0 * a2 + a1a2 * a1) * a1]
}

fn coefficients(kind: u32, rate: u32, mut frequency: f32, gain_db: f32, q: f32) -> Result<Coefficients, String> {
    let rate = rate as f32;
    let ceiling = rate * 0.5 * f32::from_bits(0x3f666666);
    if ceiling < frequency { frequency = ceiling; }
    let root_two = f32::from_bits(0x3fb504f3);
    let (b, a, denominator) = match kind {
        3 | 4 => {
            let tangent = ((frequency * std::f32::consts::PI) / rate).tan();
            let k = if kind == 3 { 1.0 / tangent } else { tangent };
            let sum = k * k + 1.0;
            let inverse = 1.0 / (sum + k * root_two);
            let b1 = inverse * if kind == 3 { 2.0 } else { -2.0 };
            let a1 = if kind == 3 { (1.0 - k * k) * b1 } else { -((k * k - 1.0) * b1) };
            ([inverse, b1, inverse], [a1, (sum - k * root_two) * inverse], 1.0)
        }
        0 | 2 => {
            let amplitude = crate::gameplay::sound::mix::power10(gain_db * 0.025);
            let angle = (frequency * std::f32::consts::TAU) / rate;
            let cosine = angle.cos();
            // The explicit zero multiplication preserves the source's invalid
            // amplitude branch as well as its ordinary unit shelf slope.
            let slope = ((1.0 / amplitude + amplitude) * 0.0 + 2.0).sqrt();
            let beta = slope * angle.sin() * 0.5 * amplitude.sqrt() * 2.0;
            let sum = amplitude + 1.0;
            let difference = amplitude - 1.0;
            if kind == 0 {
                let x = sum - difference * cosine;
                let y = difference * cosine + sum;
                ([(x + beta) * amplitude, (difference - sum * cosine) * amplitude * 2.0,
                    (x - beta) * amplitude], [(sum * cosine + difference) * -2.0, y - beta], y + beta)
            } else {
                let x = difference * cosine + sum;
                let y = sum - difference * cosine;
                ([(x + beta) * amplitude, (sum * cosine + difference) * amplitude * -2.0,
                    (x - beta) * amplitude], [(difference - sum * cosine) * 2.0, y - beta], y + beta)
            }
        }
        1 | 5 | 6 => {
            let angle = (frequency * std::f32::consts::TAU) / rate;
            let cosine = angle.cos() * -2.0;
            let amplitude = if kind == 1 { crate::gameplay::sound::mix::power10(gain_db * 0.025) } else { 1.0 };
            let alpha = angle.sin() / (q * 2.0);
            if kind == 1 {
                let scaled = alpha * amplitude;
                let inverse_scaled = alpha / amplitude;
                ([scaled + 1.0, cosine, 1.0 - scaled], [cosine, 1.0 - inverse_scaled], inverse_scaled + 1.0)
            } else if kind == 5 {
                ([alpha, 0.0, -alpha], [cosine, 1.0 - alpha], alpha + 1.0)
            } else {
                ([1.0, cosine, 1.0], [cosine, 1.0 - alpha], alpha + 1.0)
            }
        }
        _ => return Err(format!("unsupported equalizer filter type {kind}")),
    };
    let inverse = 1.0 / denominator;
    Ok(Coefficients { b: b.map(|value| inverse * value), feedback: a.map(|value| -(inverse * value)) })
}
