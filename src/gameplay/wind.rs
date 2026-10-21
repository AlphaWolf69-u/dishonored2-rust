//! World wind sampled by camera fog and physics: 77edf0, 77fec0, 780010/90/b0.
use crate::noise::PERMUTATION;
use bevy_math::Vec3;

pub use crate::assets::entities::wind::WindSettings;

pub struct Wind {
    settings: WindSettings,
    direction: Vec3,
    main_speed: f32,
    speed_variation: f32,
    pub previous_phase: f32,
    pub phase: f32,
}
impl Wind {
    pub fn new(settings: WindSettings) -> Self {
        let d = settings.direction;
        let squared = (d.y * d.y + d.x * d.x + d.z * d.z).max(f32::from_bits(0x00800000));
        let direction = d * (1. / squared.sqrt());
        Self {
            settings,
            direction,
            main_speed: settings.main_speed,
            speed_variation: settings.speed_variation,
            previous_phase: 0.,
            phase: 0.,
        }
    }
    pub fn set_speeds(&mut self, main: f32, variation: f32) {
        self.main_speed = main;
        self.speed_variation = variation;
    }
    pub fn reset_speeds(&mut self) {
        self.set_speeds(self.settings.main_speed, self.settings.speed_variation);
    }
    pub fn advance(&mut self, actor_delta: f32) {
        self.previous_phase = self.phase;
        self.phase = actor_delta * self.main_speed + self.phase;
    }
    pub fn velocity(&self, position: Vec3) -> Vec3 {
        let d = self.direction;
        let coordinate = (position.y * d.y + d.x * position.x + position.z * d.z) - self.phase;
        let noise = fractal_noise(
            self.settings.octaves,
            self.settings.persistence,
            self.settings.frequency,
            coordinate,
        );
        let speed = noise * self.speed_variation + self.main_speed;
        let mut seed = (speed * 1000.) as i64 as u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 10) & 0x7fff) as f32 * f32::from_bits(0x38000100) * 2. - 1.
        };
        let x = random();
        let y = random();
        let z = random();
        Vec3::new(
            (x * self.settings.noise_ratio + d.x) * speed,
            (y * self.settings.noise_ratio + d.y) * speed,
            (z * self.settings.noise_ratio + d.z) * speed,
        )
    }
}

/// 10de00: ridged accumulation modulates each next octave by the sum so far.
fn fractal_noise(octaves: i32, persistence: f32, mut frequency: f32, position: f32) -> f32 {
    let mut sum: f32 = 0.;
    let mut amplitude = 1.;
    for _ in 0..octaves {
        let sample = simplex_noise(frequency * position);
        frequency = frequency * 2.;
        let weighted = sample * amplitude;
        amplitude = amplitude * persistence;
        sum = sum + (1. - sum.abs()) * weighted;
    }
    sum
}

/// 10df10: the one-dimensional simplex kernel, including the nonpositive
/// lattice convention. The hash uses the standard Perlin permutation, also
/// published at https://mrl.cs.nyu.edu/~perlin/noise/ .
fn simplex_noise(mut position: f32) -> f32 {
    let mut lattice = position as i32;
    if position <= 0. {
        lattice = lattice.wrapping_sub(1);
    }
    position = position - lattice as f32;
    let next = position - 1.;
    let weight = 1. - position * position;
    let weight = weight * weight;
    let next_weight = 1. - next * next;
    let next_weight = next_weight * next_weight;
    let gradient = |index: i32| {
        let hash = PERMUTATION[(index as u32 & 255) as usize];
        let value = f32::from(hash & 7) + 1.;
        if hash & 8 != 0 {
            -value
        } else {
            value
        }
    };
    (gradient(lattice.wrapping_add(1)) * next * next_weight * next_weight
        + gradient(lattice) * position * weight * weight)
        * f32::from_bits(0x3eca3d71)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wind_matches_installed_noise_and_component_samples() {
        for (case, line) in include_str!("../../tests/fixtures/wind.txt")
            .lines()
            .enumerate()
        {
            let values = line
                .split_whitespace()
                .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 19);
            let v = &values;
            let settings = WindSettings {
                direction: Vec3::new(v[4], v[5], v[6]),
                main_speed: v[7],
                speed_variation: v[8],
                noise_ratio: v[9],
                frequency: v[3],
                persistence: v[2],
                octaves: v[1] as i32,
            };
            let wind = Wind {
                settings,
                direction: settings.direction,
                main_speed: v[7],
                speed_variation: v[8],
                phase: v[10],
                previous_phase: v[10],
            };
            let actual = [
                simplex_noise(v[0]),
                fractal_noise(
                    settings.octaves,
                    settings.persistence,
                    settings.frequency,
                    v[0],
                ),
            ]
            .into_iter()
            .chain(wind.velocity(Vec3::new(v[11], v[12], v[13])).to_array());
            for (field, (actual, expected)) in actual.zip(v[14..].iter().copied()).enumerate() {
                assert!(
                    (actual - expected).abs() <= 0.00003 * expected.abs().max(1.),
                    "wind case{case} field{field}: {actual} != {expected}"
                );
            }
        }
    }
}
