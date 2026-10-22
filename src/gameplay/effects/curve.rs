//! 1cbd50's scalar max/min curve sampling. Interior keys use packed Bezier
//! triples [incoming offset, value, outgoing offset], not motion-curve keys.
#[derive(Default)]
pub struct EffectFloatCurve {
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}
#[derive(Default)]
pub struct EffectFloatCurvePair {
    pub maximum: EffectFloatCurve,
    pub minimum: EffectFloatCurve,
}
impl EffectFloatCurve {
    fn value(&self, index: usize) -> Result<f32, String> {
        self.values.get(index).copied().ok_or_else(|| {
            format!(
                "FX curve value index {index} exceeds {} stored values",
                self.values.len()
            )
        })
    }
    fn sample(&self, time: f32) -> Result<f32, String> {
        let n = self.times.len();
        if n == 0 {
            return Ok(0.);
        }
        let triples = n * 3 == self.values.len();
        if n == 1 || self.times[0] >= time {
            return self.value(usize::from(triples));
        }
        if time >= self.times[n - 1] {
            return self.value(if triples { (n - 1) * 3 + 1 } else { n - 1 });
        }
        // JBE stops for unordered times too. A binary partition or ordinary
        // time<=key test would change that boundary behavior.
        let mut index = 0;
        while index + 1 < n && time > self.times[index + 1] {
            index += 1;
        }
        let next = self
            .times
            .get(index + 1)
            .copied()
            .ok_or("FX curve has no enclosing key interval")?;
        let mut t = time - self.times[index];
        let interval = next - self.times[index];
        if interval > f32::from_bits(0x3400_0000) {
            t /= interval;
        }
        let u = 1. - t;
        let at = index * 3 + 1;
        let a = self.value(at)?;
        let out = self.value(at + 1)?;
        let incoming = self.value(at + 2)?;
        let b = self.value(at + 3)?;
        // Match the SIMD coefficient multiplication and horizontal reduction;
        // expanding the polynomial changes the rounded curve values.
        let u2 = u * u;
        let t3 = t * 3.;
        let products = [
            a * (u2 * u),
            (a + out) * (t3 * u2),
            (b + incoming) * ((t3 * t) * u),
            b * ((t * t) * t),
        ];
        Ok((products[3] + products[1]) + (products[2] + products[0]))
    }
}
impl EffectFloatCurvePair {
    pub fn sample(&self, time: f32, mix: f32) -> Result<f32, String> {
        let maximum = self.maximum.sample(time)?;
        if self.minimum.times.is_empty() {
            return Ok(maximum);
        }
        let minimum = self.minimum.sample(time)?;
        Ok((maximum - minimum) * mix + minimum)
    }
}
