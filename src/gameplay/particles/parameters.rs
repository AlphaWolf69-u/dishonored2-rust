//! Particle parameter sampling. Shared FX curves are reused, but particle
//! range arithmetic and RNG ordering differ from cached screen-FX parameters.
use crate::{
    float::{add, multiply, subtract},
    gameplay::effects::{curve::EffectFloatCurvePair, vector_curve::EffectVectorCurvePair},
    random::lcg_unit,
};

pub struct RateParameter {
    pub range: [f32; 2],
    pub lods: Vec<[f32; 2]>,
}
impl RateParameter {
    /// 1f88a0 selects a one-based LOD, capped at the authored list length.
    /// 1f7340 samples its range with one RNG step. Only LOD ranges normalize
    /// tiny endpoints; the base range retains its raw subtraction path.
    pub fn sample(&self, lod: usize, seed: &mut u32) -> f32 {
        let mix = lcg_unit(seed);
        let selected = lod.min(self.lods.len());
        if selected == 0 {
            add(
                multiply(mix, subtract(self.range[0], self.range[1])),
                self.range[1],
            )
        } else {
            let [maximum, minimum] =
                self.lods[selected - 1].map(crate::gameplay::effects::normalize_small);
            add(multiply(subtract(1., mix), minimum), multiply(maximum, mix))
        }
    }
}

pub struct ScalarParameter {
    /// Serialized maximum followed by minimum, without sorting endpoints.
    pub range: [f32; 2],
    pub curve: EffectFloatCurvePair,
}
impl ScalarParameter {
    /// 1cd130's constant scalar parameter: cubic key at zero with value one.
    pub fn constant(value: f32) -> Self {
        Self {
            range: [value; 2],
            curve: EffectFloatCurvePair::constant(1.),
        }
    }

    /// 1cbcc0 always advances twice, including constant ranges/curves.
    pub fn sample(&self, age: f32, seed: &mut u32) -> Result<f32, String> {
        let range_mix = lcg_unit(seed);
        let curve_mix = lcg_unit(seed);
        Ok(multiply(
            self.curve.sample(age, curve_mix)?,
            add(
                multiply(range_mix, subtract(self.range[0], self.range[1])),
                self.range[1],
            ),
        ))
    }
}

pub struct VectorParameter<const N: usize = 3> {
    pub range: [[f32; N]; 2],
    pub curve: EffectVectorCurvePair<N>,
    pub uniform: bool,
}
impl<const N: usize> VectorParameter<N> {
    /// 1ef520's vector parameter initialization; callers supply its explicit
    /// range default (zero for acceleration, one for velocity-over-life).
    pub fn constant(value: [f32; N]) -> Self {
        Self {
            range: [value; 2],
            uniform: false,
            curve: EffectVectorCurvePair::constant([1.; N]),
        }
    }

    /// 1cc280's uniform mode consumes curve mixing BEFORE range mixing.
    /// 1cc0d0 instead consumes XYZ range samples, then XYZ curve samples.
    /// 1ed1a0/1ed050 use the same ordering for two-component sprite parameters,
    /// with a distinct multiply/add order on the independent path.
    pub fn sample(&self, age: f32, seed: &mut u32) -> Result<[f32; N], String> {
        if N != 2 && N != 3 {
            return Err("particle vector parameter requires two or three channels".into());
        }
        let (range_mix, curve_mix) = if self.uniform {
            let curve_mix = lcg_unit(seed);
            ([lcg_unit(seed); N], [curve_mix; N])
        } else {
            (
                std::array::from_fn(|_| lcg_unit(seed)),
                std::array::from_fn(|_| lcg_unit(seed)),
            )
        };
        let curve = self.curve.sample(age, curve_mix)?;
        Ok(std::array::from_fn(|i| {
            let difference = subtract(self.range[0][i], self.range[1][i]);
            let scale = if N == 2 && !self.uniform {
                add(self.range[1][i], multiply(range_mix[i], difference))
            } else {
                add(multiply(difference, range_mix[i]), self.range[1][i])
            };
            multiply(scale, curve[i])
        }))
    }
}
