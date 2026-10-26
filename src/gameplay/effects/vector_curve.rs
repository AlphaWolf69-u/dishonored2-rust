//! 1ed2a0/1cc3b0: two/three-component FX curves and per-channel min/max mixing.
//! Their cubic reduction differs from 1cbd50's scalar SIMD reduction.
use crate::float::{add, divide, multiply, subtract};

#[derive(Default)]
pub struct EffectVectorCurve<const N: usize> {
    pub times: Vec<f32>,
    /// Packed incoming offset, value, outgoing offset vectors for each key.
    pub values: Vec<[f32; N]>,
}
#[derive(Default)]
pub struct EffectVectorCurvePair<const N: usize> {
    pub maximum: EffectVectorCurve<N>,
    pub minimum: EffectVectorCurve<N>,
}
impl<const N: usize> EffectVectorCurve<N> {
    ///00d550/00d760 append logical key/value data. Mode4 constructors use
    ///cubic triples; an existing tripled curve retains that representation.
    ///Allocation and inline-storage flags remain with Rust's Vec owner.
    pub fn append_key(&mut self, time: f32, value: [f32; N], interpolation_mode: i32) {
        let tripled = !self.times.is_empty() && self.values.len() == self.times.len() * 3;
        self.times.push(time);
        if !tripled && interpolation_mode < 4 {
            self.values.push(value);
        } else {
            self.values.extend([[0.; N], value, [0.; N]]);
        }
    }

    fn value(&self, index: usize) -> Result<[f32; N], String> {
        self.values.get(index).copied().ok_or_else(|| {
            format!(
                "FX vector curve value index {index} exceeds {} stored values",
                self.values.len()
            )
        })
    }

    fn sample(&self, time: f32, minimum: bool) -> Result<[f32; N], String> {
        let count = self.times.len();
        if count == 0 {
            return Ok([0.; N]);
        }
        let triples = self.values.len() == count * 3;
        if count == 1 || time <= self.times[0] {
            return self.value(usize::from(triples));
        }
        // In these vector implementations unordered time takes the last key,
        // rather than the scalar sampler's interior-interval branch.
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
        let [previous, next] = if N == 2 {
            super::curve::curve_key_times(&self.times, index as i32)
        } else {
            [
                self.times[index],
                *self
                    .times
                    .get(index + 1)
                    .ok_or("FX vector curve has no enclosing interval")?,
            ]
        };
        let mut t = subtract(time, previous);
        let interval = subtract(next, previous);
        if interval > f32::from_bits(0x3400_0000) {
            t = divide(t, interval);
        }
        let u = subtract(1., t);
        let u3 = multiply(multiply(u, u), u);
        let t3 = multiply(multiply(t, t), t);
        let outgoing = multiply(multiply(multiply(t, 3.), u), u);
        let incoming = if N == 2 && minimum {
            multiply(multiply(multiply(t, t), 3.), u)
        } else {
            multiply(multiply(multiply(t, 3.), t), u)
        };
        // Vec3's interior path always indexes cubic triples. Vec2 additionally
        // retains the source's nontripled indexing branch; neither is linear.
        let at = if N == 3 || triples {
            index * 3 + 1
        } else {
            index
        };
        let a = self.value(at)?;
        let out = self.value(at + 1)?;
        let into = self.value(at + 2)?;
        let b = self.value(at + 3)?;
        Ok(std::array::from_fn(|i| {
            let first = if N == 2 {
                multiply(a[i], u3)
            } else {
                multiply(u3, a[i])
            };
            let second = multiply(outgoing, add(out[i], a[i]));
            let pair = if N == 2 {
                add(first, second)
            } else {
                add(second, first)
            };
            let tangent = if N == 2 && i == 1 {
                add(into[i], b[i])
            } else {
                add(b[i], into[i])
            };
            let last = if N == 3 || (minimum && i == 1) {
                multiply(t3, b[i])
            } else {
                multiply(b[i], t3)
            };
            add(add(pair, multiply(incoming, tangent)), last)
        }))
    }
}
impl<const N: usize> EffectVectorCurvePair<N> {
    /// Mode4 single-key initialization shared by vector FX parameters.
    pub fn constant(value: [f32; N]) -> Self {
        Self {
            maximum: EffectVectorCurve {
                times: vec![0.],
                values: vec![[0.; N], value, [0.; N]],
            },
            minimum: EffectVectorCurve::default(),
        }
    }

    pub fn sample(&self, time: f32, mix: [f32; N]) -> Result<[f32; N], String> {
        if N != 2 && N != 3 {
            return Err("FX vector curve requires two or three channels".into());
        }
        let maximum = self.maximum.sample(time, false)?;
        if self.minimum.times.is_empty() {
            return Ok(maximum);
        }
        let minimum = self.minimum.sample(time, true)?;
        Ok(std::array::from_fn(|i| {
            add(
                multiply(subtract(maximum[i], minimum[i]), mix[i]),
                minimum[i],
            )
        }))
    }
}

impl EffectVectorCurve<3> {
    /// 2267a0 retains the first stored vector as its reference, then checks
    /// incoming/outgoing tangents separately when the curve uses triples.
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
                if self.value(i * 3)? != [0.; 3] || self.value(i * 3 + 2)? != [0.; 3] {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
