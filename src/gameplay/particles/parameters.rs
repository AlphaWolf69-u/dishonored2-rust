//! Particle parameter sampling. Shared FX curves are reused, but particle
//! range arithmetic and RNG ordering differ from cached screen-FX parameters.
use crate::{
    float::{add, multiply, subtract},
    gameplay::effects::{curve::EffectFloatCurvePair, vector_curve::EffectVectorCurvePair},
    random::lcg_unit,
};

#[derive(Clone)]
pub struct RateParameter {
    pub range: [f32; 2],
    pub lods: Vec<[f32; 2]>,
}

#[derive(Clone)]
pub struct EmissionRate {
    pub range: RateParameter,
    pub curve: EffectFloatCurvePair,
}
impl EmissionRate {
    /// 1f72f0 includes every authored LOD range when estimating burst capacity.
    pub fn maximum_burst(&self) -> f32 {
        let mut maximum = self.range.range[0];
        for value in std::iter::once(&self.range.range[1]).chain(self.range.lods.iter().flatten()) {
            if maximum <= *value {
                maximum = *value;
            }
        }
        maximum
    }

    /// 1f7260 applies the maximum stored rate-curve key to that range bound.
    pub fn maximum_rate(&self) -> Result<f32, String> {
        Ok(multiply(
            self.maximum_burst(),
            self.curve.maximum_key_value(0.)?,
        ))
    }
    /// 1f39b0 samples from a COPY of the cycle seed at each emission slice.
    /// The caller's mutable spawn seed is reserved for accepted emissions.
    pub fn sample(
        &self,
        lod: usize,
        age: f32,
        cycle_seed: u32,
        multiplier: f32,
    ) -> Result<f32, String> {
        let mut seed = cycle_seed;
        let range = self.range.sample(lod, &mut seed);
        let curve = self.curve.sample_particle_rate(age, &mut seed)?;
        Ok(multiply(multiply(range, curve), multiplier))
    }
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
    /// 1ea4b0 selects the capped, one-based LOD range and 1cd690 curve path.
    /// LOD0 uses the ordinary two-sample scalar parameter implementation.
    pub fn sample_lod(
        &self,
        age: f32,
        lod: usize,
        ranges: &[[f32; 2]],
        seed: &mut u32,
    ) -> Result<f32, String> {
        let selected = lod.min(ranges.len());
        if selected == 0 {
            return self.sample(age, seed);
        }
        let range = crate::gameplay::effects::timing::sample_range(ranges[selected - 1], seed);
        Ok(multiply(self.curve.sample_particle_rate(age, seed)?, range))
    }
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

impl VectorParameter<3> {
    /// 1ea300 uses normalized LOD endpoints and conditionally sampled curve
    /// mixtures, rather than the base parameter's always-consumed curve RNG.
    pub fn sample_lod(
        &self,
        age: f32,
        lod: usize,
        ranges: &[[[f32; 3]; 2]],
        seed: &mut u32,
    ) -> Result<[f32; 3], String> {
        let selected = lod.min(ranges.len());
        if selected == 0 {
            return self.sample(age, seed);
        }
        let mix = if self.uniform {
            [lcg_unit(seed); 3]
        } else {
            std::array::from_fn(|_| lcg_unit(seed))
        };
        let range = crate::gameplay::effects::parameters::position_range(ranges[selected - 1], mix);
        let curve = self.curve.sample_particle_lod(age, seed)?;
        Ok([
            multiply(range[0], curve[0]),
            multiply(curve[1], range[1]),
            multiply(curve[2], range[2]),
        ])
    }
}

pub struct VectorParameter<const N: usize = 3> {
    pub range: [[f32; N]; 2],
    pub curve: EffectVectorCurvePair<N>,
    pub uniform: bool,
}
impl VectorParameter<3> {
    /// 1e9940: accumulate both curve key magnitudes, then multiply each by
    /// the larger serialized range endpoint. The endpoint is not absolutized.
    pub fn maximum_magnitude(&self) -> Result<[f32; 3], String> {
        let mut maximum = [0.; 3];
        self.curve.minimum.accumulate_key_magnitudes(&mut maximum)?;
        self.curve.maximum.accumulate_key_magnitudes(&mut maximum)?;
        Ok(std::array::from_fn(|i| {
            let endpoint = if self.range[0][i] <= self.range[1][i] {
                self.range[1][i]
            } else { self.range[0][i] };
            multiply(endpoint, maximum[i])
        }))
    }
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
