//! hknp per-motion deactivation: the motion-properties profile (140c9f230),
//! the 32-byte motion state (140ce90e0) and the final integrator's stillness
//! check (140ce91c0). Group decisions live with the world that owns the
//! constraint links (140cdc150).
use super::constraint_solver::reciprocal;

fn rsqrt_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value.sqrt()
    }
}

fn rcp_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value
    }
}

/// CVTTPS2DQ followed by the XOR with `2^31 <= v`: overflow saturates up.
fn truncate(value: f32) -> i32 {
    let limit = 2_147_483_648f32;
    if value >= limit {
        i32::MAX
    } else if value.is_nan() || value < -limit {
        i32::MIN
    } else {
        value as i32
    }
}

fn exponent(value: f32) -> u8 {
    (value.to_bits() >> 23) as u8
}

/// Motion properties +0x28..+0x3c.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeactivationProfile {
    pub linear_squared: f32,
    pub angular_squared: f32,
    pub inverse_scale: f32,
    pub coherence_high: i16,
    pub coherence_low: i16,
    pub checks: u8,
    pub slow: u8,
    pub still: u8,
    pub spike_margin: u8,
    pub spike: u8,
}

impl DeactivationProfile {
    /// 140c9f230 followed by 140c9f320(…, .8125, -.8, 3, .016).
    pub fn new(linear: f32, angular: f32, checks: u8) -> Self {
        let mut angular = angular * f32::from_bits(0x3ea2_f983);
        let limit = f32::from_bits(0x3f19_999a);
        if limit <= angular {
            angular = limit;
        }
        let mut threshold = linear * 20. * f32::from_bits(0x3d0f_5c29);
        let shift = 4i32 - checks as i32;
        if shift >= 0 {
            threshold *= (1i32 << (shift & 31)) as f32;
        }
        let still = exponent(threshold * threshold);
        let half = threshold * 0.5;
        let quantum = f32::from_bits(0x46ff_fe00);
        Self {
            linear_squared: linear * linear,
            angular_squared: angular * angular,
            inverse_scale: 1. / (linear * 20.),
            coherence_high: (f32::from_bits(0x3f50_0000) * quantum) as i64 as i16,
            coherence_low: (f32::from_bits(0xbf4c_cccd) * quantum) as i64 as i16,
            checks,
            slow: exponent(half * half).min(still),
            still,
            spike_margin: 6,
            spike: exponent(f32::from_bits(0x3c83_126f) * f32::from_bits(0x3c83_126f)),
        }
    }
}

impl Default for DeactivationProfile {
    /// c9f050 -> 140c9f230(DAT_141ab3814 = .05, DAT_141ec4664 = pi/20, 4);
    /// every live Dust profile carries these values.
    fn default() -> Self {
        Self::new(f32::from_bits(0x3d4c_cccd), f32::from_bits(0x3e20_d97c), 4)
    }
}

/// hknpDeactivationState activation mode (+0xe): 140c87ca0 sets it.
pub const MODE_DEACTIVATE: u8 = 0;
pub const MODE_ALWAYS_ACTIVE: u8 = 1;
pub const MODE_KEEP_DEACTIVATED: u8 = 2;

/// The 32-byte state at deactivation manager +0x348 (one per motion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionDeactivation(pub [u8; 32]);

fn position_scale(profile: &DeactivationProfile) -> (f32, f32) {
    let inverse = reciprocal(profile.inverse_scale);
    (inverse, inverse * f32::from_bits(0x3a80_0000))
}

fn quantize_position(center: [f32; 4], profile: &DeactivationProfile) -> u32 {
    let scale = 1024. * profile.inverse_scale;
    let q: [u32; 3] = std::array::from_fn(|i| truncate(scale * center[i]) as u32 & 0x3ff);
    q[0] | q[1] << 10 | q[2] << 20
}

fn quantize_orientation(orientation: [f32; 4]) -> [u8; 4] {
    let scale = reciprocal(f32::from_bits(0x3f8c_cccd)) * 128.;
    orientation.map(|q| {
        truncate(q * scale)
            .wrapping_add(127)
            .clamp(i16::MIN as i32, i16::MAX as i32)
            .clamp(0, 255) as u8
    })
}

impl MotionDeactivation {
    /// 140ce90e0(state, motionId & 3, mode).
    pub fn new(motion_id: u32, mode: u8) -> Self {
        let mut state = [0u8; 32];
        state[10] = (motion_id & 3) as u8;
        state[4..8].copy_from_slice(&quantize_orientation([0., 0., 0., 1.]));
        state[14] = mode;
        state[16..20].copy_from_slice(&(-1i32).to_le_bytes());
        Self(state)
    }
    pub fn mode(&self) -> u8 {
        self.0[14]
    }
    pub fn set_mode(&mut self, mode: u8) {
        self.0[14] = mode;
    }
    pub fn island(&self) -> i32 {
        i32::from_le_bytes(self.0[16..20].try_into().unwrap())
    }
    pub fn set_island(&mut self, island: i32) {
        self.0[16..20].copy_from_slice(&island.to_le_bytes());
    }
    /// 140c91230 activation: a counter of 0xff ("never") is kept.
    pub fn restart(&mut self) {
        if self.0[11] != 0xff {
            self.0[11] = 0;
        }
    }

    fn requantize(&mut self, center: [f32; 4], orientation: [f32; 4], profile: &DeactivationProfile) {
        self.0[0..4].copy_from_slice(&quantize_position(center, profile).to_le_bytes());
        self.0[4..8].copy_from_slice(&quantize_orientation(orientation));
    }

    /// 140ce91c0 after the final integration of an active motion. `linear` is
    /// the new motion +0x60 velocity, `previous` the velocity before it.
    /// Returns the candidate bit the group decision consumes.
    pub fn check(
        &mut self,
        center: [f32; 4],
        orientation: [f32; 4],
        linear: [f32; 4],
        previous: [f32; 4],
        profile: &DeactivationProfile,
    ) -> bool {
        let s = &mut self.0;
        let mut counter = s[11] as u32;
        if counter == 0xff {
            return false;
        }
        let mut frame = s[10] as u32 + 1;
        let full = profile.checks.wrapping_shl(2) as u32;
        let mut interval = 4u32;
        let mut level = 0u32;
        if counter != 0 {
            let square = linear.map(|v| v * v);
            let speed = (square[1] + square[0]) + square[2];
            level = ((counter - 1) >> 2).min(profile.checks as u32);
            interval = 4 << level;
            let restart = frame - 1 == interval * 2;
            let magnitude = (speed.to_bits() & 0x7f80_0000) >> 23;
            if !(speed == 0. || speed.is_nan()) {
                let inverse = rsqrt_estimate(speed);
                let past = previous.map(|v| v * v);
                let past = (past[1] + past[0]) + past[2];
                let past_inverse = if past <= 0. { 0. } else { rsqrt_estimate(past) };
                let c: [f32; 3] =
                    std::array::from_fn(|i| (past_inverse * previous[i]) * (linear[i] * inverse));
                let cosine = (c[1] + c[0]) + c[2];
                let samples = (16i32 << level) as f32;
                let t = f32::from_bits(0x46ff_fe00) * (cosine * rcp_estimate(samples)) + 0.5;
                let old = i16::from_le_bytes([s[8], s[9]]) as i32;
                let sum = old.wrapping_add(truncate(t)).clamp(i16::MIN as i32, i16::MAX as i32);
                s[8..10].copy_from_slice(&(sum as i16).to_le_bytes());
            }
            let slot = (((counter - 1) >> 1) & 1) as usize;
            let mut current = s[12 + slot] as u32;
            let other = s[13 - slot] as u32;
            if counter <= full {
                current = if restart { magnitude } else { current.max(magnitude) };
            }
            if counter > 2
                && magnitude > profile.spike as u32
                && magnitude > profile.spike_margin as u32 + other
            {
                counter = 0;
            }
            s[12 + slot] = current as u8;
        }
        if frame & (interval - 1) == 0 {
            let reset = 'test: {
                let packed = u32::from_le_bytes(s[0..4].try_into().unwrap());
                let (inverse, unit) = position_scale(profile);
                let scale = 1024. * profile.inverse_scale;
                let reference = [packed & 0x3ff, packed >> 10 & 0x3ff, packed >> 20 & 0x3ff];
                let d: [f32; 3] = std::array::from_fn(|i| {
                    let current = (truncate(center[i] * scale) as u32 & 0x3ff) as i32 as f32 * unit;
                    let d = (reference[i] as i32 as f32 * unit - current).abs();
                    if d < inverse * 0.5 {
                        d
                    } else {
                        inverse - d
                    }
                });
                let d = d.map(|v| v * v);
                if (d[1] + d[0]) + d[2] > profile.linear_squared {
                    break 'test true;
                }
                let tenths = f32::from_bits(0x3f8c_cccd);
                let unpack = reciprocal(128.) * tenths;
                let pack = reciprocal(tenths) * 128.;
                let e: [f32; 4] = std::array::from_fn(|i| {
                    let current = truncate(orientation[i] * pack) as f32 * unpack;
                    let e = (s[4 + i] as i32 - 127) as f32 * unpack - current;
                    e * e
                });
                if (e[3] + e[1]) + (e[2] + e[0]) > profile.angular_squared {
                    break 'test true;
                }
                if counter == 0 {
                    s[8..10].fill(0);
                    s[12..14].fill(0);
                    frame = 0;
                } else if counter & 3 == 0 {
                    let slow = s[12] < profile.slow && s[13] < profile.slow;
                    let still = s[12] < profile.still && s[13] < profile.still;
                    let coherence = i16::from_le_bytes([s[8], s[9]]) as i32;
                    let high = 0x7fff - ((0x7fff - profile.coherence_high as i32) >> level);
                    if coherence > high {
                        if counter == full && !still {
                            break 'test true;
                        }
                    } else if coherence < profile.coherence_low as i32 || slow {
                        counter = full;
                    }
                    let (c, o) = (quantize_position(center, profile), quantize_orientation(orientation));
                    s[0..4].copy_from_slice(&c.to_le_bytes());
                    s[4..8].copy_from_slice(&o);
                    frame = 0;
                    s[12] = if counter < full { 0 } else { s[13] };
                    s[8..10].fill(0);
                }
                counter = (counter + 1).min(0xfe);
                false
            };
            if reset {
                self.requantize(center, orientation, profile);
                counter = 0;
            }
        }
        let s = &mut self.0;
        s[10] = frame as u8;
        s[11] = counter as u8;
        (counter & 0xff) as i32 > profile.checks as i32 * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> &'static str {
        include_str!("../../tests/fixtures/motion_deactivation.txt")
    }
    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }
    fn float(word: &str) -> f32 {
        f32::from_bits(u32::from_str_radix(word, 16).unwrap())
    }

    #[test]
    fn profile_matches_motion_properties_packer() {
        let mut count = 0;
        for line in fixture().lines().filter(|l| l.starts_with("profile ")) {
            let f: Vec<&str> = line.split(' ').collect();
            let p = DeactivationProfile::new(float(f[1]), float(f[2]), f[3].parse().unwrap());
            let mut out = Vec::new();
            out.extend(p.linear_squared.to_le_bytes());
            out.extend(p.angular_squared.to_le_bytes());
            out.extend(p.inverse_scale.to_le_bytes());
            out.extend(p.coherence_high.to_le_bytes());
            out.extend(p.coherence_low.to_le_bytes());
            out.extend([p.checks, p.slow, p.still, p.spike_margin, p.spike]);
            assert_eq!(out, bytes(f[4]), "{line}");
            count += 1;
        }
        assert_eq!(count, 64);
        let p = DeactivationProfile::default();
        assert_eq!((p.linear_squared, p.inverse_scale), (f32::from_bits(0x3b23_d70b), 1.));
        assert_eq!((p.coherence_high, p.coherence_low), (0x67ff, 0x999bu16 as i16));
        assert_eq!([p.checks, p.slow, p.still, p.spike_margin, p.spike], [4, 0x73, 0x75, 6, 0x73]);
    }

    #[test]
    fn state_init_matches_executable() {
        for line in fixture().lines().filter(|l| l.starts_with("init ")) {
            let f: Vec<&str> = line.split(' ').collect();
            let state = MotionDeactivation::new(f[1].parse().unwrap(), f[2].parse().unwrap());
            let expected = bytes(f[3]);
            for i in (0..8).chain([10, 11, 14]).chain(16..20) {
                assert_eq!(state.0[i], expected[i], "{line} byte {i}");
            }
        }
    }

    #[test]
    fn check_matches_executable() {
        let mut count = 0;
        for line in fixture().lines().filter(|l| l.starts_with("check ")) {
            let f: Vec<&str> = line.split(' ').collect();
            let mut state = MotionDeactivation(bytes(f[1]).try_into().unwrap());
            let v: Vec<f32> = f[2..18].iter().map(|w| float(w)).collect();
            let lane = |i: usize| -> [f32; 4] { std::array::from_fn(|j| v[i * 4 + j]) };
            let profile = DeactivationProfile::new(
                f32::from_bits(0x3d4c_cccd),
                f32::from_bits(0x3e20_d97c),
                f[18].parse().unwrap(),
            );
            let candidate = state.check(lane(0), lane(1), lane(2), lane(3), &profile);
            assert_eq!(candidate, f[19] == "-1", "{line}");
            assert_eq!(state.0.to_vec(), bytes(f[20]), "{line}");
            count += 1;
        }
        assert!(count > 10_000);
    }
}
