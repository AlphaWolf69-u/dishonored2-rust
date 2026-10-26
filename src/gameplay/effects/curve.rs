//! 1cbd50's scalar max/min curve sampling. Interior keys use packed Bezier
//! triples [incoming offset, value, outgoing offset], not motion-curve keys.
#[derive(Clone, Default)]
pub struct EffectFloatCurve {
    pub times: Vec<f32>,
    pub values: Vec<f32>,
}
#[derive(Clone, Default)]
pub struct EffectFloatCurvePair {
    pub maximum: EffectFloatCurve,
    pub minimum: EffectFloatCurve,
}

///1d1ae0 supplies a vector curve's enclosing times, extrapolating the first
///or last interval for indices outside the key list. Empty/single-key lists
///have their own source branches; no synthetic interval width is inserted.
pub fn curve_key_times(times: &[f32], index: i32) -> [f32; 2] {
    use crate::float::{add, multiply, subtract};
    match times.len() {
        0 => [0.; 2],
        1 => [times[0]; 2],
        count => {
            let key = |index: i32| {
                if index < 0 {
                    add(
                        multiply(subtract(times[1], times[0]), index as f32),
                        times[0],
                    )
                } else if (index as usize) < count {
                    times[index as usize]
                } else {
                    let steps = index.wrapping_sub(count as i32).wrapping_add(1);
                    add(
                        multiply(subtract(times[count - 1], times[count - 2]), steps as f32),
                        times[count - 1],
                    )
                }
            };
            [key(index), key(index.wrapping_add(1))]
        }
    }
}
impl EffectFloatCurve {
    /// 1ee9b0's inline query-visibility curves use the same sequential cubic
    /// reduction as 1cd690's minimum path, including endpoint admission.
    pub fn sample_visibility(&self, time: f32) -> Result<f32, String> {
        self.sample_particle_rate(time, true)
    }

    /// 1cd690's scalar particle-rate path. It has different cubic reduction
    /// orders for maximum and minimum, independently of 1cbd50's SIMD sampler.
    fn sample_particle_rate(&self, time: f32, minimum: bool) -> Result<f32, String> {
        use crate::float::{add, divide, multiply, subtract};
        let count = self.times.len();
        if count == 0 {
            return Ok(0.);
        }
        let triples = self.values.len() == count * 3;
        if count == 1 || time <= self.times[0] {
            return self.value(usize::from(triples));
        }
        if !(time < self.times[count - 1]) {
            return self.value(if triples {
                (count - 1) * 3 + 1
            } else {
                count - 1
            });
        }
        let mut index = 0;
        while index + 1 < count && !(time <= self.times[index + 1]) {
            index += 1;
        }
        let mut t = subtract(time, self.times[index]);
        let interval = subtract(
            *self
                .times
                .get(index + 1)
                .ok_or("particle curve has no enclosing interval")?,
            self.times[index],
        );
        if interval > f32::from_bits(0x3400_0000) {
            t = divide(t, interval);
        }
        let u = subtract(1., t);
        let at = index * 3 + 1;
        let a = self.value(at)?;
        let out = self.value(at + 1)?;
        let incoming = self.value(at + 2)?;
        let b = self.value(at + 3)?;
        let first = multiply(multiply(multiply(a, u), u), u);
        let second = multiply(multiply(multiply(multiply(add(a, out), t), 3.), u), u);
        let third = multiply(multiply(multiply(multiply(add(b, incoming), t), 3.), t), u);
        let last = multiply(multiply(multiply(b, t), t), t);
        Ok(if minimum {
            add(add(add(last, second), third), first)
        } else {
            add(add(last, second), add(third, first))
        })
    }

    /// 2266c0's FX constant-curve classification. Its reference is the first
    /// stored float, including the incoming tangent in a tripled curve.
    pub fn is_constant(&self) -> Result<bool, String> {
        let count = self.times.len();
        if count <= 1 {
            return Ok(true);
        }
        let tripled = self.values.len() == count * 3;
        let reference = self.value(0)?;
        for i in 1..count {
            if reference != self.value(if tripled { i * 3 + 1 } else { i })? {
                return Ok(false);
            }
        }
        if tripled {
            for i in 0..count {
                if self.value(i * 3)? != 0. || self.value(i * 3 + 2)? != 0. {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

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
    /// 1d0f00 accumulates a bound over stored key values, minimum curve first.
    /// Tangents are skipped even when the curve stores Bezier triples.
    pub fn maximum_key_value(&self, mut maximum: f32) -> Result<f32, String> {
        for curve in [&self.minimum, &self.maximum] {
            let triples = curve.values.len() == curve.times.len() * 3;
            for index in 0..curve.times.len() {
                let value = *curve
                    .values
                    .get(if triples { index * 3 + 1 } else { index })
                    .ok_or("particle curve key exceeds stored value count")?;
                if maximum <= value {
                    maximum = value;
                }
            }
        }
        Ok(maximum)
    }
    /// 1cd690 consumes a random step only when a minimum curve exists. The
    /// range reduction multiplies by the raw 15-bit integer before its scale.
    pub fn sample_particle_rate(&self, time: f32, seed: &mut u32) -> Result<f32, String> {
        use crate::float::{add, multiply, subtract};
        let maximum = self.maximum.sample_particle_rate(time, false)?;
        if self.minimum.times.is_empty() {
            return Ok(maximum);
        }
        let state = crate::random::lcg_step(seed);
        let minimum = self.minimum.sample_particle_rate(time, true)?;
        Ok(add(
            multiply(
                multiply(subtract(maximum, minimum), ((state >> 10) & 0x7fff) as f32),
                f32::from_bits(0x3800_0100),
            ),
            minimum,
        ))
    }

    /// Mode4 single-key initialization used by scalar FX parameters.
    pub fn constant(value: f32) -> Self {
        Self {
            maximum: EffectFloatCurve {
                times: vec![0.],
                values: vec![0., value, 0.],
            },
            minimum: EffectFloatCurve::default(),
        }
    }

    pub fn sample(&self, time: f32, mix: f32) -> Result<f32, String> {
        let maximum = self.maximum.sample(time)?;
        if self.minimum.times.is_empty() {
            return Ok(maximum);
        }
        let minimum = self.minimum.sample(time)?;
        Ok((maximum - minimum) * mix + minimum)
    }
}
