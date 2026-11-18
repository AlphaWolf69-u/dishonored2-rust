//! FX camera shake (start 14022d700, oscillators 14022d640, phase update
//! 14022aec0 -> 14022d240 fade-in / 14022d0a0 duration / 14022d3e0
//! fade-out, channel sums 14022ca90/14022c900/14022cc20, composition
//! 14022c6d0 with 14022c430/22afd0/22b8b0/22c190/22c500, accumulation
//! 14022d9e0) and its application to the view (14088c5e0).
use super::{parameters::{sample_range, EffectParameterSample}, timing::EffectTimings};
use crate::assets::fx::{CameraShakeAsset, StandardParameter};
use std::sync::Arc;

/// One sampled standard parameter: sample, curve mix, its duration start
/// value and fade-in start value, and the current value.
struct Sampled {
    sample: EffectParameterSample,
    main: f32,
    current: f32,
}

impl Sampled {
    /// 140226b10 then the duration and fade-in curves at 0.
    fn new(parameter: &StandardParameter, seed: &mut u32) -> Result<Self, String> {
        let sample = EffectParameterSample::new(parameter.scale, seed);
        let main = parameter.duration.sample(0., sample.curve_mix)? * sample.scale;
        let current = parameter.fade_in.sample(0., sample.curve_mix)? * main;
        Ok(Self { sample, main, current })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { FadeIn, Duration, FadeOut }

/// The view owner's accumulation (+8): position offset, angular and
/// direction quaternions (x, y, z, w), reset each frame (14022db30).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeAccumulator {
    pub offset: [f32; 3],
    pub angular: [f32; 4],
    pub direction: [f32; 4],
}

impl Default for ShakeAccumulator {
    fn default() -> Self {
        Self { offset: [0.; 3], angular: [0., 0., 0., 1.], direction: [0., 0., 0., 1.] }
    }
}

/// The FX camera shakes of the last FX update, for the view (14022dac0).
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone, Copy, Default)]
pub struct FxShakeFrame(pub ShakeAccumulator);

/// 140110990: idQuat product.
pub fn quaternion_product(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        (ax * bw + bx * aw + ay * bz) - az * by,
        (ay * bw + by * aw + az * bx) - bz * ax,
        (az * bw + bz * aw + by * ax) - ay * bx,
        ((bw * aw - ax * bx) - by * ay) - az * bz,
    ]
}

/// 140111410: quaternion to a row-major 3x3 matrix.
pub fn quaternion_matrix(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let xx = x * x2;
    let wx = w * x2;
    [
        [1. - (z * z2 + y * y2), x * y2 - w * z2, w * y2 + x * z2],
        [w * z2 + x * y2, 1. - (z * z2 + xx), y * z2 - wx],
        [x * z2 - w * y2, wx + y * z2, 1. - (y * y2 + xx)],
    ]
}

impl ShakeAccumulator {
    /// 14022d9e0.
    fn add(&mut self, offset: [f32; 3], angular: [f32; 4], direction: [f32; 4]) {
        for i in 0..3 {
            self.offset[i] += offset[i];
        }
        self.angular = quaternion_product(self.angular, angular);
        self.direction = quaternion_product(self.direction, direction);
    }

    /// 14088c5e0: the view origin moves by the offset and its axes (rows)
    /// become angular x axes x direction.
    pub fn apply(&self, origin: &mut [f32; 3], axes: &mut [[f32; 3]; 3]) {
        for i in 0..3 {
            origin[i] += self.offset[i];
        }
        let product = |a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]| -> [[f32; 3]; 3] {
            std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
        };
        *axes = product(&product(&quaternion_matrix(self.angular), axes), &quaternion_matrix(self.direction));
    }
}

pub struct CameraShake {
    asset: Arc<CameraShakeAsset>,
    timing: EffectTimings,
    /// +0x3c: the shake's own seed (curve sampling draws from a copy).
    seed: u32,
    magnitude: Sampled,
    /// Amplitude and frequency per oscillator, per channel.
    oscillators: [Vec<(Sampled, Sampled)>; 8],
    distance_radius: f32,
    impulse_radius: f32,
    pub active: bool,
}

/// Curves sampled at fraction `t` for the phase.
fn phase_curve(parameter: &StandardParameter, phase: Phase, t: f32, mix: f32) -> Result<f32, String> {
    match phase {
        Phase::FadeIn => parameter.fade_in.sample(t, mix),
        Phase::Duration => parameter.duration.sample(t, mix),
        Phase::FadeOut => parameter.fade_out.sample(t, mix),
    }
}

impl CameraShake {
    /// 14022d700.
    pub fn new(asset: Arc<CameraShakeAsset>, offsets: [f32; 4], seed: &mut u32) -> Result<Self, String> {
        crate::random::lcg_step(seed);
        let own_seed = *seed;
        let timing = EffectTimings::new(&asset.timing, offsets, seed);
        let magnitude = Sampled::new(&asset.magnitude, seed)?;
        let mut oscillators: [Vec<(Sampled, Sampled)>; 8] = Default::default();
        for (channel, list) in asset.channels.iter().enumerate() {
            for oscillator in list {
                let amplitude = Sampled::new(&oscillator.amplitude, seed)?;
                let frequency = Sampled::new(&oscillator.frequency, seed)?;
                oscillators[channel].push((amplitude, frequency));
            }
        }
        let distance_radius = sample_range([asset.distance_radius[1], asset.distance_radius[0]], seed);
        let impulse_radius = sample_range([asset.impulse[1], asset.impulse[0]], seed);
        Ok(Self { asset, timing, seed: own_seed, magnitude, oscillators, distance_radius, impulse_radius, active: true })
    }

    pub fn can_loop(&self) -> bool {
        self.active && self.asset.looping
    }

    pub fn stop(&mut self, now: f32) {
        self.timing.begin_fade_out(self.asset.keep_duration_on_stop, now);
    }

    /// 14022c900/ca90/cc20: one channel's sum of amplitude x cos(2 pi f age).
    fn channel(&mut self, channel: usize, phase: Phase, t: f32, age: f32) -> Result<f32, String> {
        let mut sum = 0.;
        for (index, oscillator) in self.asset.channels[channel].iter().enumerate() {
            let (amplitude, frequency) = &mut self.oscillators[channel][index];
            let (a, f) = match phase {
                Phase::Duration => (
                    phase_curve(&oscillator.amplitude, phase, t, amplitude.sample.curve_mix)? * amplitude.sample.scale,
                    phase_curve(&oscillator.frequency, phase, t, frequency.sample.curve_mix)? * frequency.sample.scale,
                ),
                Phase::FadeIn => (
                    phase_curve(&oscillator.amplitude, phase, t, amplitude.sample.curve_mix)? * amplitude.main,
                    phase_curve(&oscillator.frequency, phase, t, frequency.sample.curve_mix)? * frequency.main,
                ),
                Phase::FadeOut => (
                    phase_curve(&oscillator.amplitude, phase, t, amplitude.sample.curve_mix)? * amplitude.current,
                    phase_curve(&oscillator.frequency, phase, t, frequency.sample.curve_mix)? * frequency.current,
                ),
            };
            if phase != Phase::FadeOut {
                amplitude.current = a;
                frequency.current = f;
            }
            let tau = std::f32::consts::TAU;
            let mut angle = f * tau * age;
            if angle < 0. || tau <= angle {
                angle -= (angle * f32::from_bits(0x3e22_f983)).floor() * tau;
            }
            sum += angle.cos() * a;
        }
        Ok(sum)
    }

    /// 14022aec0 and its phase functions for one frame at `age`, adding
    /// to `accumulator`. `camera` is the view position and its axes (rows),
    /// `origin` the FX position, `impulse` the FX instance's impulse (+0xb8).
    pub fn update(&mut self, age: f32, seed: &mut u32, camera: ([f32; 3], [[f32; 3]; 3]), origin: [f32; 3],
        impulse: f32, accumulator: &mut ShakeAccumulator) -> Result<(), String> {
        if !self.active || age < self.timing.delay_end {
            return Ok(());
        }
        let (phase, t) = if self.timing.duration_end <= age {
            if !self.asset.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                (Phase::FadeOut, (age - self.timing.duration_end) * self.timing.fade_out_rate)
            } else {
                self.timing.restart(&self.asset.timing, self.timing.duration_end, seed);
                if self.timing.fade_in_end <= age {
                    (Phase::Duration, (age - self.timing.fade_in_end) * self.timing.duration_rate)
                } else {
                    (Phase::FadeIn, (age - self.timing.delay_end) * self.timing.fade_in_rate)
                }
            }
        } else if self.timing.fade_in_end <= age {
            (Phase::Duration, (age - self.timing.fade_in_end) * self.timing.duration_rate)
        } else {
            (Phase::FadeIn, (age - self.timing.delay_end) * self.timing.fade_in_rate)
        };
        let mut t = t;
        let magnitude = match phase {
            Phase::Duration => {
                let value = self.asset.magnitude.duration.sample(t, self.magnitude.sample.curve_mix)? * self.magnitude.sample.scale;
                self.magnitude.current = value;
                value
            }
            Phase::FadeIn => {
                let value = self.asset.magnitude.fade_in.sample(t, self.magnitude.sample.curve_mix)? * self.magnitude.main;
                self.magnitude.current = value;
                value
            }
            Phase::FadeOut => {
                if 1. < t {
                    self.active = false;
                    t = 1.;
                }
                self.asset.magnitude.fade_out.sample(t, self.magnitude.sample.curve_mix)? * self.magnitude.current
            }
        };
        let mut channels = [0.; 8];
        for (channel, value) in channels.iter_mut().enumerate() {
            *value = self.channel(channel, phase, t, age)?;
        }
        // 14022c6d0: the direction to the FX and its distance.
        let (position, axes) = camera;
        let delta: [f32; 3] = std::array::from_fn(|i| origin[i] - position[i]);
        let squared = delta.iter().map(|v| v * v).sum::<f32>();
        let inverse = 1. / squared.max(f32::from_bits(0x0080_0000)).sqrt();
        let direction = delta.map(|v| v * inverse);
        let distance = inverse * squared;
        let mut local_seed = self.seed;
        let _ = &mut local_seed;
        let distance_fade = self.asset.distance_fade.sample(distance / self.distance_radius.max(1.), 0.)?;
        let impulse_fade = self.asset.impulse_fade.sample(impulse / self.impulse_radius.max(1.), 0.)?;
        let scale = magnitude * distance_fade * impulse_fade;
        let [px, py, pz, po] = [channels[0], channels[1], channels[2], channels[3]];
        let offset = std::array::from_fn(|i| (direction[i] * po + px * axes[0][i] + py * axes[1][i] + pz * axes[2][i]) * scale);
        let radians = f32::from_bits(0x3c8e_fa35) * scale;
        let angles = [channels[4] * radians, channels[5] * radians, channels[6] * radians];
        let angular = euler_quaternion(angles, self.asset.rotation_order);
        let direction_rotation = toward(direction, channels[7] * radians);
        accumulator.add(offset, angular, direction_rotation);
        Ok(())
    }
}

/// 14022c190: half angles about the principal axes in the rotate order.
fn euler_quaternion(angles: [f32; 3], order: u32) -> [f32; 4] {
    const ORDERS: [usize; 13] = [0, 1, 2, 0, 1, 2, 0, 2, 1, 0, 2, 1, 0];
    let order = (order as usize).min(5);
    let axis = |index: usize| {
        let half = angles[index] * 0.5;
        let (s, c) = half.sin_cos();
        let mut q = [0., 0., 0., c];
        q[index] = s;
        q
    };
    let mut q = axis(ORDERS[order * 2]);
    q = quaternion_product(q, axis(ORDERS[order * 2 + 1]));
    quaternion_product(q, axis(ORDERS[order * 2 + 2]))
}

/// 14022c500: a rotation by `angle` about the axis perpendicular to the
/// FX direction and Z (or X when the direction is within 0.9 of Z).
fn toward(direction: [f32; 3], angle: f32) -> [f32; 4] {
    let [x, y, z] = direction;
    let axis = if f32::from_bits(0x3f66_6666) <= z.abs() { [0., -z, y] } else { [y, -x, 0.] };
    let squared = axis.iter().map(|v| v * v).sum::<f32>().max(f32::from_bits(0x0080_0000));
    let inverse = 1. / squared.sqrt();
    let (s, c) = angle.sin_cos();
    [axis[0] * inverse * s, axis[1] * inverse * s, axis[2] * inverse * s, c]
}
