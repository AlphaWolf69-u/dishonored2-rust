//! 88e220's numeric/boolean overrides after ordinary environment tracks.
//! Image parameters use environment_images instead of this weighted average.
use crate::float::{add, multiply};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterKind {
    Boolean,
    Signed,
    Unsigned,
    Float,
}
struct Entry<K> {
    key: K,
    kind: ParameterKind,
    baseline: [u32; 4],
    target: [f32; 4],
    influence: f32,
    weight: f32,
}
pub struct ParameterOverrides<K> {
    entries: Vec<Entry<K>>,
}
impl<K> Default for ParameterOverrides<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}
impl<K: PartialEq> ParameterOverrides<K> {
    /// Explicit entries retain their order and duplicates. Their values have
    /// already been converted to floats by the external parameter producer.
    pub fn push_override(
        &mut self,
        key: K,
        kind: ParameterKind,
        baseline: [u32; 4],
        target: [f32; 4],
        weight: f32,
    ) {
        self.entries.push(Entry {
            key,
            kind,
            baseline,
            target,
            influence: 1.,
            weight,
        });
    }
    /// Only environments that actually supply this parameter participate.
    /// Blend weight and averaging influence are independent, unclamped inputs.
    pub fn add_environment(
        &mut self,
        key: K,
        kind: ParameterKind,
        baseline: [u32; 4],
        value: [u32; 4],
        weight: f32,
        influence: f32,
    ) {
        let entry = if let Some(index) = self.entries.iter().position(|e| e.key == key) {
            &mut self.entries[index]
        } else {
            self.entries.push(Entry {
                key,
                kind,
                baseline,
                target: [0.; 4],
                influence: 0.,
                weight: 0.,
            });
            self.entries.last_mut().unwrap()
        };
        for i in 0..if kind == ParameterKind::Boolean { 1 } else { 4 } {
            let value = match kind {
                ParameterKind::Boolean | ParameterKind::Unsigned => value[i] as f32,
                ParameterKind::Signed => value[i] as i32 as f32,
                ParameterKind::Float => f32::from_bits(value[i]),
            };
            let product = if kind == ParameterKind::Float {
                multiply(influence, value)
            } else {
                multiply(value, influence)
            };
            entry.target[i] = add(product, entry.target[i]);
        }
        entry.weight = add(multiply(influence, weight), entry.weight);
        entry.influence = add(influence, entry.influence);
    }
    pub fn finish(self) -> impl Iterator<Item = (K, [u32; 4])> {
        self.entries.into_iter().map(|e| {
            // MAXSS retains incoming NaN/equal operands, unlike f32::max.
            let epsilon = f32::from_bits(0x38d1_b717);
            let divisor = if epsilon > e.influence {
                epsilon
            } else {
                e.influence
            };
            let reciprocal = 1. / divisor;
            let fraction = multiply(reciprocal, e.weight).clamp(0., 1.);
            let value = match e.kind {
                ParameterKind::Boolean => {
                    let candidate = u32::from(multiply(reciprocal, e.target[0]) >= 0.5);
                    let threshold = if e.baseline[0] == 0 && candidate != 0 {
                        f32::from_bits(0x3586_37bd)
                    } else {
                        f32::from_bits(0x3f7f_be77)
                    };
                    let value = if fraction < threshold {
                        e.baseline[0]
                    } else {
                        candidate
                    };
                    [value, 0, 0, 0]
                }
                ParameterKind::Float => std::array::from_fn(|i| {
                    let start = f32::from_bits(e.baseline[i]);
                    add(
                        multiply(multiply(reciprocal, e.target[i]) - start, fraction),
                        start,
                    )
                    .to_bits()
                }),
                ParameterKind::Signed => std::array::from_fn(|i| {
                    let start = e.baseline[i] as i32;
                    let target = integer(reciprocal * e.target[i]);
                    let delta = target.wrapping_sub(start) as f32;
                    start.wrapping_add(integer((delta * fraction).floor())) as u32
                }),
                ParameterKind::Unsigned => std::array::from_fn(|i| {
                    let start = e.baseline[i];
                    let target = wide_integer_word(reciprocal * e.target[i]);
                    if target < start {
                        start.wrapping_sub(wide_integer_word(
                            ((start - target) as f32 * fraction).ceil(),
                        ))
                    } else {
                        start.wrapping_add(wide_integer_word(
                            ((target - start) as f32 * fraction).floor(),
                        ))
                    }
                }),
            };
            (e.key, value)
        })
    }
}
fn integer(value: f32) -> i32 {
    crate::entities::integer_word(value.to_bits(), 0x30) as i32
}
// CVTTSS2SI RAX's indefinite result has zero low 32 bits. Rust's saturating
// cast differs for positive overflow; keep the executable's stored word.
fn wide_integer_word(value: f32) -> u32 {
    if (-9223372036854775808.0..9223372036854775808.0).contains(&value) {
        value as i64 as u32
    } else {
        0
    }
}

#[cfg(test)]
#[path = "environment_parameter_overrides/tests.rs"]
mod tests;
