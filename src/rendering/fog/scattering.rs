//! lightscattering's ray integral, attenuation and canvas modulation.
//! Light-list ownership, ray/shape intervals and shadow taps remain callers.

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn saturate(value: f32) -> f32 {
    if value > 0. {
        value.min(1.)
    } else {
        0.
    }
}

/// InScatter's optimized arctangent identity, including BOTH source floors.
/// The original does not use a phase function or exponential falloff here.
pub fn in_scatter(start: [f32; 3], direction: [f32; 3], light: [f32; 3], length: f32) -> f32 {
    let q = std::array::from_fn(|i| start[i] - light[i]);
    let b = dot(direction, q);
    let c = dot(q, q);
    // The installed helper compiles both expressions to MAD. Preserving
    // contraction matters when the ray nearly passes through the light.
    let s = 1. / (-b).mul_add(b, c).max(0.0001).sqrt();
    s * (length / (s * b.mul_add(length, c)).max(0.0001)).atan()
}

#[derive(Clone, Copy)]
pub struct ScatteringAttenuation {
    pub position: [f32; 3],
    pub cutoff: f32,
    pub falloff: f32,
    pub spot_direction: [f32; 3],
    pub spot_parameters: [f32; 2],
    pub scatter_value: f32,
}
impl ScatteringAttenuation {
    /// Point, half-point and area types share the radius test. Only the
    /// source's LIGHT_TYPE_SPOT(2) applies its angular attenuation here.
    pub fn evaluate(
        &self,
        kind: u32,
        camera: [f32; 3],
        direction: [f32; 3],
        minimum: f32,
        length: f32,
    ) -> f32 {
        let middle = std::array::from_fn(|i| {
            self.position[i] - (camera[i] + direction[i] * (minimum + length * 0.5))
        });
        let square = dot(middle, middle);
        let cutoff = self.cutoff * self.cutoff;
        let t = saturate((square - cutoff) / (self.falloff * self.falloff - cutoff));
        let mut attenuation = t * t * (3. - 2. * t);
        if kind == 2 {
            let inverse = 1. / square.sqrt();
            let normalized = middle.map(|v| v * inverse);
            let cosine = dot(normalized, self.spot_direction);
            attenuation *= 1.
                - saturate(
                    (0.5 + 0.5 * cosine) * self.spot_parameters[0] + self.spot_parameters[1],
                );
        }
        attenuation * self.scatter_value
    }
}

/// ComputeTurbulence chooses a canvas per light and uses an unclamped lerp.
pub fn turbulence(first: f32, second: f32, use_second: bool, amount: f32) -> f32 {
    let canvas = if use_second { second } else { first };
    1. + (canvas - 1.) * amount
}

/// FakeRayleigh's phase. The installed global shader declaration's PI literal
/// rounds to the same f32 as the standard library constant.
pub fn rayleigh_phase(theta: f32) -> [f32; 3] {
    let phase = (3. / (16. * std::f32::consts::PI)) * (1. + theta * theta);
    [0.2 * phase, 0.5 * phase, 0.8 * phase]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scattering_helpers_match_installed_hlsl() {
        let reference: Vec<f32> = include_str!("../../../tests/fixtures/fog_light_scattering.txt")
            .split_whitespace()
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect();
        assert_eq!(reference.len(), 512 * 4);
        for id in 0..64 {
            // Inputs arrive through storage, as in real light records. They
            // are not GPU expressions fused across a synthetic call boundary.
            let r = &reference[id * 32..id * 32 + 32];
            let camera: [f32; 3] = r[0..3].try_into().unwrap();
            let direction: [f32; 3] = r[4..7].try_into().unwrap();
            let light = ScatteringAttenuation {
                position: r[8..11].try_into().unwrap(),
                cutoff: r[11],
                falloff: r[15],
                spot_direction: r[12..15].try_into().unwrap(),
                spot_parameters: r[16..18].try_into().unwrap(),
                scatter_value: r[18],
            };
            let length = r[3];
            let q = std::array::from_fn(|i| camera[i] - light.position[i]);
            let phase = rayleigh_phase(r[23]);
            let values = [
                in_scatter(camera, direction, light.position, length),
                light.evaluate(r[7] as u32, camera, direction, r[22], length),
                turbulence(r[20], r[21], id & 1 != 0, r[19]),
                dot(direction, q),
                phase[0],
                phase[1],
                phase[2],
                dot(q, q),
            ];
            for (channel, value) in values.into_iter().enumerate() {
                let expected = r[24 + channel];
                assert!(
                    (value - expected).abs() < 0.00003 + expected.abs() * 0.00002,
                    "scattering id{id} channel{channel}: {value} != {expected}"
                );
            }
        }
    }
}
