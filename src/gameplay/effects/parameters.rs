//! 226b10's scalar range sample and independent curve-mixing sample.
pub struct EffectParameterSample {
    pub scale: f32,
    pub curve_mix: f32,
}
/// 2178c0 and the first sample in 226b10 share this range evaluation.
pub fn sample_range(range: [f32; 2], seed: &mut u32) -> f32 {
    let fraction = crate::random::lcg_unit(seed);
    let first = super::normalize_small(range[0]);
    let second = super::normalize_small(range[1]);
    super::add((1. - fraction) * first, fraction * second)
}
impl EffectParameterSample {
    /// Endpoints retain their serialized order; this is not a sorted range.
    pub fn new(range: [f32; 2], seed: &mut u32) -> Self {
        Self {
            scale: sample_range(range, seed),
            curve_mix: crate::random::lcg_unit(seed),
        }
    }
}

///218060/218120 and226bc0/226d30 normalize tiny endpoints before weighting.
///Vec2's independent second product retains fraction*endpoint operand order.
fn vector_range<const N: usize>(range: [[f32; N]; 2], mix: [f32; N], uniform: bool) -> [f32; N] {
    use crate::float::{add, multiply, subtract};
    std::array::from_fn(|i| {
        let first = super::normalize_small(range[0][i]);
        let second = super::normalize_small(range[1][i]);
        let product = if N == 2 && !uniform {
            multiply(mix[i], second)
        } else {
            multiply(second, mix[i])
        };
        add(multiply(subtract(1., mix[i]), first), product)
    })
}

pub struct EffectVectorParameterSample<const N: usize> {
    pub scale: [f32; N],
    pub curve_mix: [f32; N],
}
impl<const N: usize> EffectVectorParameterSample<N> {
    ///Uniform mode consumes one shared range sample followed by one shared
    ///curve-mix sample. Independent mode consumes all N range samples BEFORE
    ///the N curve-mix samples, rather than interleaving scalar constructors.
    pub fn new(range: [[f32; N]; 2], uniform: bool, seed: &mut u32) -> Result<Self, String> {
        if N != 2 && N != 3 {
            return Err("FX vector parameter requires two or three channels".into());
        }
        let fractions = if uniform {
            [crate::random::lcg_unit(seed); N]
        } else {
            std::array::from_fn(|_| crate::random::lcg_unit(seed))
        };
        let scale = vector_range(range, fractions, uniform);
        let curve_mix = if uniform {
            [crate::random::lcg_unit(seed); N]
        } else {
            std::array::from_fn(|_| crate::random::lcg_unit(seed))
        };
        Ok(Self { scale, curve_mix })
    }
}

///1da100 weights the position range in the opposite endpoint order from the
///standard vector parameter. Fractions are supplied by the parent's shared RNG.
pub fn position_range(range: [[f32; 3]; 2], mix: [f32; 3]) -> [f32; 3] {
    use crate::float::{add, multiply, subtract};
    std::array::from_fn(|i| {
        add(
            multiply(subtract(1., mix[i]), super::normalize_small(range[1][i])),
            multiply(mix[i], super::normalize_small(range[0][i])),
        )
    })
}
