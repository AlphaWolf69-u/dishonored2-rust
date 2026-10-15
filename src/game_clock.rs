//! Original five-group game clock (140059480–140059990).
//!
//! The platform counter and engine/CVar decisions are explicit inputs. This is
//! not a replacement of those decisions with Bevy elapsed time. The renderer's
//! gameplay view-clock transfer is implemented below; camera construction and
//! entity clock/lifecycle ownership remain separate. Numeric fixtures execute
//! the original update and transfer blocks on owned clock storage.

pub const GROUP_COUNT: usize = 5;
pub const GROUP_STRIDE: usize = 0x50;
pub const STATE_SIZE: usize = 0x1b0;
/// Native constructor step and read-only DAT_1422813c8 fixed step.
pub const DEFAULT_STEP: f32 = f32::from_bits(0x3d08_8889);
const QUANTUM: f32 = f32::from_bits(0x3c08_8889);

/// Values read at the native update's platform/engine/CVar call boundaries.
#[derive(Clone, Copy, Debug)]
pub struct ClockStepInputs {
    /// FUN_140157db0: QueryPerformanceCounter minus the platform's origin.
    pub counter_since_origin: i64,
    /// FUN_140157da0: the platform's stored QueryPerformanceFrequency.
    pub counter_frequency: i64,
    /// PTR_142274420 -> idCommonLocal vtable 141ac4d88, +0x188 ->
    /// FUN_14003fbe0: its +0x1ed8 counter equals zero. See the predicate below.
    pub variable_step: bool,
    pub force_fixed_step: i32, // g_gameFixedDt, DAT_142bc5558
    pub fixed_step: f32,       // DAT_1422813c8
    pub jitter_window: f32,    // g_physicsStepTolerance, DAT_142bc55dc
    /// Historical field name: this is observed smoothed FPS, NOT a frame cap.
    /// FUN_140044ec0 computes DAT_1425ae8f0 from the measured microseconds.
    pub rate_cap: i32,
    pub grow_step_limit: i32, // g_limitPhysicStepVariation, DAT_142bc5658
}

/// FUN_14003fbe0, idCommonLocal virtual +0x188. The common constructor
/// 14003e4c0 initializes this counter to zero. 140048940 sets it to two before
/// time-stream work; 140048a50 advances that work and decrements the counter
/// (or clears it on DAT_1425b2258), invoking/clearing +0x1ee0 on the >=2 path.
/// Those stream/transition lifecycles are not replaced by this scalar getter.
pub fn allows_variable_step(common_counter: i32) -> bool {
    common_counter == 0
}

/// Registered defaults: FUN_141355b80/141355c20/141355bd0. Configuration
/// overrides are resolved by their actual names, never by a guessed preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockControls {
    pub force_fixed_step: i32,
    pub physics_step_tolerance: f32,
    pub limit_step_variation: i32,
}

impl Default for ClockControls {
    fn default() -> Self {
        Self {
            force_fixed_step: 0,
            physics_step_tolerance: 0.2,
            limit_step_variation: 0,
        }
    }
}

/// FUN_140157f40's cached integer QPC-to-microsecond scale. Multiplication
/// wraps before division; replacing this with Duration/floating seconds loses
/// the original integer rounding. The platform must supply a positive QPF.
#[derive(Clone, Copy, Debug, Default)]
pub struct CounterMicroseconds {
    divisor: u64,
}

impl CounterMicroseconds {
    pub fn read(&mut self, counter_since_origin: i64, frequency: i64) -> Option<u64> {
        if self.divisor == 0 {
            if frequency <= 0 {
                return None;
            }
            self.divisor = (frequency as u64).wrapping_shl(10) / 1_000_000;
        }
        (self.divisor != 0).then(|| (counter_since_origin as u64).wrapping_shl(10) / self.divisor)
    }
}

/// Numeric prefix of FUN_140044ec0. The original frame calls it before game
/// updates (FUN_1400450d0). Scheduler-mode changes and debug printing in that
/// function are separate owner actions, not implemented by this estimator.
#[derive(Clone, Copy, Debug)]
pub struct FrameTimingEstimator {
    previous_microseconds: Option<u64>,
    smoothed_microseconds: f32,
    frame_microseconds: u64,
    truncated_microseconds: i32,
    frame_rate: i32,
}

impl Default for FrameTimingEstimator {
    fn default() -> Self {
        Self {
            previous_microseconds: None,
            smoothed_microseconds: f32::from_bits(0x4682_3555),
            frame_microseconds: 0,
            truncated_microseconds: 0,
            frame_rate: 0,
        }
    }
}

impl FrameTimingEstimator {
    pub fn frame_rate(&self) -> i32 {
        self.frame_rate
    }
    pub fn frame_microseconds(&self) -> u64 {
        self.frame_microseconds
    }

    pub fn update<E>(&mut self, mut read: impl FnMut() -> Result<u64, E>) -> Result<(), E> {
        let previous = match self.previous_microseconds {
            Some(previous) => previous,
            None => {
                let previous = read()?;
                self.previous_microseconds = Some(previous);
                previous
            }
        };
        let current = read()?;
        self.frame_microseconds = current.wrapping_sub(previous);
        let signed = self.frame_microseconds as i64;
        let mut elapsed = signed as f32;
        if signed < 0 {
            elapsed += f32::from_bits(0x5f80_0000);
        }
        let tiny = f32::from_bits(0x2193_92ef);
        if f32::from_bits(elapsed.to_bits() & 0x7fff_ffff) <= tiny {
            elapsed = 0.0;
        }
        if f32::from_bits(self.smoothed_microseconds.to_bits() & 0x7fff_ffff) <= tiny {
            self.smoothed_microseconds = 0.0;
        }
        self.smoothed_microseconds = self.smoothed_microseconds * f32::from_bits(0x3f4c_cccd)
            + elapsed * f32::from_bits(0x3e4c_cccd);
        self.truncated_microseconds = native_truncate_i32(self.smoothed_microseconds);
        self.frame_rate = native_truncate_i32(1_000_000.0 / self.smoothed_microseconds + 0.5);
        self.previous_microseconds = Some(current);
        Ok(())
    }
}

/// Byte-exact owned state. Native padding/other fields survive initialization
/// and updates, instead of being silently assigned invented field meanings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeGameClock {
    bytes: [u8; STATE_SIZE],
}

impl NativeGameClock {
    pub fn from_bytes(bytes: [u8; STATE_SIZE]) -> Self {
        Self { bytes }
    }
    pub fn as_bytes(&self) -> &[u8; STATE_SIZE] {
        &self.bytes
    }

    fn u32(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap())
    }
    fn u64(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.bytes[at..at + 8].try_into().unwrap())
    }
    fn f32(&self, at: usize) -> f32 {
        f32::from_bits(self.u32(at))
    }
    fn f64(&self, at: usize) -> f64 {
        f64::from_bits(self.u64(at))
    }
    fn put32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(&mut self, at: usize, value: u64) {
        self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn put_float(&mut self, at: usize, value: f32) {
        self.put32(at, value.to_bits());
    }
    fn put_double(&mut self, at: usize, value: f64) {
        self.put64(at, value.to_bits());
    }
    fn copy32(&mut self, to: usize, from: usize) {
        self.put32(to, self.u32(from));
    }
    fn copy64(&mut self, to: usize, from: usize) {
        self.put64(to, self.u64(from));
    }

    /// FUN_140059480. Writes only fields touched by the native constructor.
    pub fn initialize(&mut self) {
        self.put32(0, 0);
        for group in 0..GROUP_COUNT {
            let base = group * GROUP_STRIDE;
            for offset in [0x08, 0x18, 0x30, 0x38, 0x50] {
                self.put64(base + offset, 0);
            }
            for offset in [0x10, 0x14, 0x20, 0x48] {
                self.put_float(base + offset, 1.0);
            }
            self.put64(base + 0x24, 0x3f80_0000);
            self.put_double(base + 0x40, -1.0);
        }
        self.put32(0x198, 0);
        self.put32(0x19c, 12);
        self.put64(0x1a0, 0x0100_0000);
        self.put64(0x1a8, 0);
        self.put_double(0x18, f64::from(DEFAULT_STEP) + 0.0);
    }

    /// FUN_1400596a0; reuse the already verified raw-record getter.
    pub fn time(&self, group: usize) -> Option<f64> {
        let records = self.bytes[..GROUP_COUNT * GROUP_STRIDE]
            .as_chunks::<GROUP_STRIDE>()
            .0;
        crate::render_model::game_clock_group_time(records, group)
    }

    /// FUN_140059680.
    pub fn delta(&self, group: usize) -> Option<f32> {
        (group < GROUP_COUNT).then(|| self.f32(group * GROUP_STRIDE + 0x28))
    }

    /// FUN_1400596e0.
    pub fn previous_time(&self, group: usize) -> Option<f64> {
        (group < GROUP_COUNT).then(|| self.f64(group * GROUP_STRIDE + 0x08))
    }

    /// FUN_1400596c0 / FUN_140059720.
    pub fn rates(&self, group: usize) -> Option<[f32; 2]> {
        (group < GROUP_COUNT).then(|| {
            let base = group * GROUP_STRIDE;
            [self.f32(base + 0x20), self.f32(base + 0x24)]
        })
    }

    /// FUN_140059740: output order and the >= 0x34000000 change test.
    pub fn rate_change(&self, group: usize) -> Option<([f32; 2], bool)> {
        (group < GROUP_COUNT).then(|| {
            let base = group * GROUP_STRIDE;
            let current = self.f32(base + 0x24);
            let previous = self.f32(base + 0x14);
            (
                [current, previous],
                (current - previous).abs() >= f32::from_bits(0x3400_0000),
            )
        })
    }

    /// FUN_140059890. The deadline uses clock 2, not the selected clock.
    pub fn set_rate_target(
        &mut self,
        group: usize,
        value: f32,
        duration: f32,
    ) -> Result<(), String> {
        if group >= GROUP_COUNT {
            return Err(format!(
                "native time group {group} outside 0..{GROUP_COUNT}"
            ));
        }
        let base = group * GROUP_STRIDE;
        self.put_float(base + 0x48, value);
        self.put_double(base + 0x40, f64::from(duration) + self.f64(0xb8));
        Ok(())
    }

    /// FUN_1400598d0; the next update consumes the one-shot +0x1a1 flag.
    pub fn set_time(&mut self, time: f64, delta: f32) {
        self.put_double(0x18, time);
        self.put_float(0x28, delta);
        self.bytes[0x1a1] = 1;
        self.put_double(0x08, time - f64::from(delta));
        for group in 1..GROUP_COUNT {
            let base = group * GROUP_STRIDE;
            self.copy64(base + 0x08, 0x08);
            self.copy64(base + 0x18, 0x18);
            self.copy32(base + 0x28, 0x28);
        }
    }

    /// FUN_140059980. Deliberately separate from the update.
    pub fn increment_frame_counter(&mut self) {
        self.put32(0, self.u32(0).wrapping_add(1));
    }

    /// FUN_1400592d0. The callback is the original virtual read/write boundary;
    /// this does not claim to implement the save file's outer format or endian
    /// conversion. Writing callbacks must leave their input fields unchanged.
    pub fn transfer_groups<E>(
        &mut self,
        writing: bool,
        mut transfer: impl FnMut(&mut [u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        for group in 0..GROUP_COUNT {
            let base = 8 + group * GROUP_STRIDE;
            for (offset, size) in [
                (0, 8),
                (8, 4),
                (12, 4),
                (16, 8),
                (24, 4),
                (28, 4),
                (32, 4),
                (40, 8),
                (48, 8),
                (56, 8),
                (64, 4),
            ] {
                transfer(&mut self.bytes[base + offset..base + offset + size])?;
            }
            if !writing {
                self.copy64(base + 0x48, base + 0x10);
            }
        }
        Ok(())
    }

    /// FUN_140059780, including post-read synchronization of clocks 1 and 2.
    pub fn transfer_state<E>(
        &mut self,
        writing: bool,
        mut transfer: impl FnMut(&mut [u8]) -> Result<(), E>,
    ) -> Result<(), E> {
        transfer(&mut self.bytes[..4])?;
        self.transfer_groups(writing, &mut transfer)?;
        for (offset, size) in [(0x1a0, 1), (0x1a1, 1), (0x1a2, 1), (0x1a4, 4)] {
            transfer(&mut self.bytes[offset..offset + size])?;
        }
        if !writing {
            self.bytes[0x1a3] = 0;
            let a = self.f64(0x68);
            let b = self.f64(0xb8);
            if b <= a && a != b {
                self.copy64(0xb8, 0x68);
            } else {
                self.copy64(0x68, 0xb8);
            }
        }
        Ok(())
    }

    /// FUN_140059990. All external state is explicit; no rate, pause, or
    /// platform-clock policy is replaced by a guessed game-thread default.
    pub fn step(&mut self, input: ClockStepInputs) -> bool {
        if self.bytes[0x1a1] == 1 {
            self.bytes[0x1a1] = 0;
            return true;
        }
        let counter_time = input.counter_since_origin as f64 / input.counter_frequency as f64;
        if self.bytes[0x1a3] == 0 {
            for group in 0..GROUP_COUNT {
                let base = group * GROUP_STRIDE;
                self.copy64(base + 0x08, base + 0x18);
                self.copy32(base + 0x10, base + 0x20);
                self.copy32(base + 0x14, base + 0x24);
            }
            self.put_double(0x18, counter_time + self.f64(0x1a8));
        } else {
            self.bytes[0x1a3] = 0;
            self.put_double(0x1a8, self.f64(0x18) - counter_time);
        }
        if 0.0 <= self.f64(0x90) {
            self.put_double(0x90, -1.0);
            self.copy32(0x70, 0x98);
            self.copy32(0x74, 0x98);
        }
        if !input.variable_step || input.force_fixed_step != 0 || self.bytes[0x1a2] != 0 {
            let delta = if !input.variable_step {
                DEFAULT_STEP
            } else if input.force_fixed_step != 0 {
                input.fixed_step
            } else {
                self.f32(0x1a4)
            };
            self.put_double(0x08, self.f64(0x18) - f64::from(delta));
            self.put32(0x19c, 12);
            self.bytes[0x1a2] = 0;
        }
        self.put_float(0x28, (self.f64(0x18) - self.f64(0x08)) as f32);
        self.put_float(0x78, self.f32(0x74) * self.f32(0x28));
        self.put_double(0x68, f64::from(self.f32(0x78)) + self.f64(0x58));
        let mut unquantized = (self.f64(0x68) - self.f64(0xb8)) as f32;
        let excess = unquantized - 0.5;
        if 0.0 < excess {
            self.put_double(0x58, self.f64(0x58) - f64::from(excess));
            self.put_double(0x68, self.f64(0x68) - f64::from(excess));
            unquantized = 0.5;
        }
        for base in [0xa0, 0xf0, 0x140] {
            let deadline = self.f64(base + 0x40);
            if 0.0 <= deadline {
                if deadline <= self.f64(0x68) {
                    self.put_double(base + 0x40, -1.0);
                    self.copy32(base + 0x20, base + 0x48);
                } else {
                    let rate = self.f32(base + 0x20);
                    let fraction = self.f32(0x78) / (deadline - self.f64(0x58)) as f32;
                    self.put_float(
                        base + 0x20,
                        fraction * (self.f32(base + 0x48) - rate) + rate,
                    );
                }
            }
        }
        self.bytes[0x1a0] = u8::from(self.f32(0x160) < 1.0 || self.f32(0x110) < 1.0);
        let effective_rate = self.f32(0xc0) * self.f32(0x74);
        self.put_float(0xc4, effective_rate);
        let mut max_rate = 0.0;
        for rate in [self.f32(0x110), self.f32(0x160)] {
            if max_rate <= rate {
                max_rate = rate;
            }
        }
        let raw_steps = ((unquantized * max_rate) * effective_rate) * 120.0;
        let mut count = native_truncate_i32((raw_steps + 0.5).floor());
        if input.jitter_window > 0.0 && input.rate_cap > 0 {
            let expected = ((120.0 / input.rate_cap as f32) * max_rate) * effective_rate;
            if expected > 0.5 && (0.5 - expected % 1.0).abs() * 2.0 > 0.5 {
                let previous = self.u32(0x198) as i32;
                if count > previous {
                    count = native_truncate_i32(((raw_steps - input.jitter_window) + 0.5).floor());
                } else if count < previous {
                    count = native_truncate_i32(((input.jitter_window + raw_steps) + 0.5).floor());
                }
            }
        }
        count = if count < 0 {
            0
        } else {
            count.min(self.u32(0x19c) as i32).min(12)
        };
        self.put32(0x198, count as u32);
        self.put32(
            0x19c,
            if input.grow_step_limit == 0 {
                12
            } else {
                count.wrapping_add(1)
            } as u32,
        );
        self.put_float(0xc8, (count as f32 * QUANTUM) / max_rate);
        self.put_double(0xb8, f64::from(self.f32(0xc8)) + self.f64(0xa8));
        for base in [0xf0, 0x140] {
            let rate = self.f32(base + 0x20);
            let applied = if rate < f32::from_bits(0x3d4c_cccd) {
                if rate <= f32::from_bits(0x3ccc_cccd) {
                    0.0
                } else {
                    f32::from_bits(0x3d4c_cccd)
                }
            } else {
                rate
            };
            self.put_float(base + 0x24, applied * self.f32(0xc4));
            self.put_float(base + 0x28, applied * self.f32(0xc8));
            self.put_double(
                base + 0x18,
                f64::from(self.f32(base + 0x28)) + self.f64(base + 0x18),
            );
        }
        count > 0
    }
}

/// Five clocks transferred in identity order, not entity mapping [3,4,2,0].
/// FUN_1408ac710's clock loop calls 1408a8a00/1408a8980, forwarding to the
/// manager's time/delta getters. Seconds remain unchanged (including raw bits).
/// This is that clock block, NOT 8ac710's other gameplay/scope operations.
#[derive(Clone, Copy, Debug)]
pub struct GameplayViewClocks {
    times: [f64; GROUP_COUNT],
    deltas: [f32; GROUP_COUNT],
}

impl GameplayViewClocks {
    pub fn from_clock(clock: &NativeGameClock) -> Self {
        Self {
            times: std::array::from_fn(|group| {
                clock
                    .time(group)
                    .expect("owned native clock has five groups")
            }),
            deltas: std::array::from_fn(|group| {
                clock
                    .delta(group)
                    .expect("owned native clock has five groups")
            }),
        }
    }

    /// 1408ac744..1408ac795: game-result +0..0x28 doubles, +0x28..0x3c
    /// floats. Unrelated result fields/tail are preserved.
    pub fn write_game_result(&self, destination: &mut [u8]) -> Result<(), String> {
        if destination.len() < 0x3c {
            return Err("game clock result needs 0x3c bytes".into());
        }
        for group in 0..GROUP_COUNT {
            destination[group * 8..group * 8 + 8].copy_from_slice(&self.times[group].to_le_bytes());
            let delta = 0x28 + group * 4;
            destination[delta..delta + 4].copy_from_slice(&self.deltas[group].to_le_bytes());
        }
        Ok(())
    }

    /// 14005c3a8..14005c3ed overwrites the copied camera prefix's clocks.
    /// Prefix starts at batch entry +0x30; callers pass the prefix itself.
    /// Camera/viewport/other prefix fields are NOT fabricated or cleared.
    pub fn write_view_prefix(&self, prefix: &mut [u8]) -> Result<(), String> {
        if prefix.len() < 0x44 {
            return Err("gameplay view clock prefix needs 0x44 bytes".into());
        }
        for group in 0..GROUP_COUNT {
            let time = 0x08 + group * 8;
            prefix[time..time + 8].copy_from_slice(&self.times[group].to_le_bytes());
            let delta = 0x30 + group * 4;
            prefix[delta..delta + 4].copy_from_slice(&self.deltas[group].to_le_bytes());
        }
        Ok(())
    }

    /// 140319560 reads the identity-ordered renderer snapshot after the
    /// original 35dc60/374e40 copies. Preserve double addition before f32
    /// conversion; invalid native group bits are unavailable, not clamped.
    pub fn model_local_time(&self, model_offset: f32, model_flags_1c31: u8) -> Option<[f32; 2]> {
        let group = crate::render_model::model_shader_time_group(model_flags_1c31);
        Some(crate::render_model::model_shader_local_time(
            model_offset,
            *self.times.get(group)?,
            *self.deltas.get(group)?,
        ))
    }
}

// CVTTSS2SI returns INT_MIN for NaN/out-of-range; Rust's saturating cast alone
// would change the native step-count branches.
fn native_truncate_i32(value: f32) -> i32 {
    if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_view_clock_transfer_matches_original_machine_blocks() {
        fn bytes(text: &str) -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
                .collect()
        }
        let mut count = 0;
        for line in include_str!("../analysis/original_gameplay_view_clock_cases.txt").lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            assert_eq!(words.len(), 12);
            let mut state = [0xa5; STATE_SIZE];
            for group in 0..GROUP_COUNT {
                let time = u64::from_str_radix(words[group], 16).unwrap();
                let delta = u32::from_str_radix(words[5 + group], 16).unwrap();
                state[group * GROUP_STRIDE + 0x18..group * GROUP_STRIDE + 0x20]
                    .copy_from_slice(&time.to_le_bytes());
                state[group * GROUP_STRIDE + 0x28..group * GROUP_STRIDE + 0x2c]
                    .copy_from_slice(&delta.to_le_bytes());
            }
            let values = GameplayViewClocks::from_clock(&NativeGameClock::from_bytes(state));
            let mut game_result = [0x6b; 0x50];
            values.write_game_result(&mut game_result).unwrap();
            assert_eq!(&game_result[..0x3c], bytes(words[10]), "{line}");
            assert_eq!(&game_result[0x3c..], &[0x6b; 0x14]);
            let mut prefix = [0xa5; 0x50];
            values.write_view_prefix(&mut prefix).unwrap();
            assert_eq!(prefix.as_slice(), bytes(words[11]), "{line}");
            count += 1;
        }
        assert_eq!(count, 525);
    }

    #[test]
    fn gameplay_clocks_select_manager_record_without_entity_remapping() {
        let clocks = GameplayViewClocks {
            times: [16_777_216.75, 11.0, 22.0, 33.0, 44.0],
            deltas: [-0.0, 0.125, 0.25, 0.5, 1.0],
        };
        for flags in 0..=u8::MAX {
            let group = crate::render_model::model_shader_time_group(flags);
            match clocks.model_local_time(0.5, flags) {
                Some(time) => {
                    assert!(group < GROUP_COUNT);
                    assert_eq!(
                        time[0].to_bits(),
                        ((clocks.times[group] + 0.5) as f32).to_bits()
                    );
                    assert_eq!(time[1].to_bits(), clocks.deltas[group].to_bits());
                }
                None => assert!(group >= GROUP_COUNT),
            }
        }
        assert_eq!(clocks.model_local_time(0.5, 0).unwrap()[0], 16_777_218.0);
        assert_eq!(clocks.model_local_time(0.5, 0x30).unwrap(), [33.5, 0.5]);
        let mut short = [0xa5; 0x3b];
        assert!(clocks.write_game_result(&mut short).is_err());
        assert!(clocks.write_view_prefix(&mut short).is_err());
        assert_eq!(short, [0xa5; 0x3b]);
    }

    #[test]
    fn constructor_reanchors_and_quantizes_first_update() {
        let mut clock = NativeGameClock::from_bytes([0xa5; STATE_SIZE]);
        clock.initialize();
        assert_eq!(clock.bytes[4..8], [0xa5; 4]);
        for group in 0..GROUP_COUNT {
            assert_eq!(clock.rates(group), Some([1.0, 1.0]));
            assert_eq!(clock.delta(group), Some(0.0));
        }
        let inputs = ClockStepInputs {
            counter_since_origin: 123456,
            counter_frequency: 1000,
            variable_step: true,
            force_fixed_step: 0,
            fixed_step: DEFAULT_STEP,
            jitter_window: 0.0,
            rate_cap: 0,
            grow_step_limit: 0,
        };
        assert!(clock.step(inputs));
        assert_eq!(clock.u32(0x198), 4);
        assert_eq!(clock.time(3), Some(f64::from(4.0 * QUANTUM)));
        assert_eq!(clock.bytes[0x1a3], 0);
        assert_eq!(clock.time(GROUP_COUNT), None);
    }

    #[test]
    fn setting_time_and_serialization_do_not_guess_outer_format() {
        let mut clock = NativeGameClock::from_bytes([0; STATE_SIZE]);
        clock.initialize();
        clock.set_time(25.0, 0.125);
        assert_eq!(clock.previous_time(4), Some(24.875));
        let mut saved = Vec::new();
        clock
            .transfer_state(true, |field| {
                saved.extend_from_slice(field);
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(saved.len(), 331);
        let mut restored = NativeGameClock::from_bytes([0; STATE_SIZE]);
        restored.initialize();
        let mut cursor = 0;
        restored
            .transfer_state(false, |field| {
                field.copy_from_slice(&saved[cursor..cursor + field.len()]);
                cursor += field.len();
                Ok::<_, ()>(())
            })
            .unwrap();
        assert_eq!(restored.time(3), Some(25.0));
        assert_eq!(restored.delta(3), Some(0.125));
        assert_eq!(restored.bytes[0x1a3], 0);
        assert_eq!(restored.rate_change(3), Some(([1.0, 1.0], false)));
        assert!(restored.set_rate_target(5, 1.0, 0.0).is_err());
    }

    #[test]
    fn platform_and_frame_timing_match_original_executable() {
        fn word(text: &str) -> u64 {
            u64::from_str_radix(text, 16).unwrap()
        }
        let mut counts = [0; 2];
        for (line, text) in include_str!("../analysis/original_game_clock_cases.txt")
            .lines()
            .enumerate()
        {
            let fields: Vec<_> = text.split_whitespace().collect();
            match fields[0] {
                "micro" => {
                    counts[0] += 1;
                    let mut scale = CounterMicroseconds {
                        divisor: word(fields[3]),
                    };
                    assert_eq!(
                        scale.read(word(fields[1]) as i64, word(fields[2]) as i64),
                        Some(word(fields[4])),
                        "native microseconds {line}"
                    );
                    assert_eq!(scale.divisor, word(fields[5]));
                }
                "timing" => {
                    counts[1] += 1;
                    let initialized = fields[1] == "1";
                    let current = word(fields[4]);
                    let mut timing = FrameTimingEstimator {
                        previous_microseconds: initialized.then(|| word(fields[2])),
                        smoothed_microseconds: f32::from_bits(word(fields[3]) as u32),
                        ..Default::default()
                    };
                    let mut calls = 0;
                    timing
                        .update(|| {
                            calls += 1;
                            Ok::<_, ()>(current)
                        })
                        .unwrap();
                    assert_eq!(calls, if initialized { 1 } else { 2 });
                    assert_eq!(
                        (
                            timing.frame_microseconds,
                            timing.smoothed_microseconds.to_bits(),
                            timing.truncated_microseconds as u32,
                            timing.frame_rate as u32
                        ),
                        (
                            word(fields[5]),
                            word(fields[6]) as u32,
                            word(fields[7]) as u32,
                            word(fields[8]) as u32
                        ),
                        "native frame estimator {line}"
                    );
                    assert_eq!(timing.previous_microseconds, Some(current));
                }
                _ => (),
            }
        }
        assert_eq!(counts, [256, 1000]);
        assert_eq!(CounterMicroseconds::default().read(1, 0), None);
        assert_eq!(CounterMicroseconds::default().read(1, 1), None);
    }

    #[test]
    fn clock_state_matches_original_executable() {
        fn raw<const N: usize>(text: &str) -> [u8; N] {
            assert_eq!(text.len(), N * 2);
            std::array::from_fn(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
        }
        fn word(text: &str) -> u32 {
            u32::from_str_radix(text, 16).unwrap()
        }
        fn float(text: &str) -> f32 {
            f32::from_bits(word(text))
        }
        fn double(text: &str) -> f64 {
            f64::from_bits(u64::from_str_radix(text, 16).unwrap())
        }
        let mut counts = [0; 7];
        for (line, text) in include_str!("../analysis/original_game_clock_cases.txt")
            .lines()
            .enumerate()
        {
            let fields: Vec<_> = text.split_whitespace().collect();
            if matches!(fields[0], "micro" | "timing") {
                continue;
            }
            if fields[0] == "gate" {
                counts[6] += 1;
                let counter = i32::from_str_radix(fields[1], 16).unwrap();
                assert_eq!(allows_variable_step(counter), fields[2] == "1");
                // FUN_1400406c0 is satisfied by the evidenced standard integer
                // comparison; no second wrapper around `<` is necessary.
                assert_eq!(counter < 2, fields[3] == "1");
                continue;
            }
            let (before, expected) = match fields[0] {
                "init" | "inc" => (1, 2),
                "set" => (3, 4),
                "target" => (4, 5),
                "io" => (3, 5),
                "step" => (9, 11),
                other => panic!("unknown original fixture {other}"),
            };
            let mut clock = NativeGameClock::from_bytes(raw(fields[before]));
            match fields[0] {
                "init" => {
                    counts[0] += 1;
                    clock.initialize();
                }
                "inc" => {
                    counts[1] += 1;
                    clock.increment_frame_counter();
                }
                "set" => {
                    counts[2] += 1;
                    clock.set_time(double(fields[1]), float(fields[2]));
                }
                "target" => {
                    counts[3] += 1;
                    clock
                        .set_rate_target(
                            word(fields[1]) as usize,
                            float(fields[2]),
                            float(fields[3]),
                        )
                        .unwrap();
                }
                "io" => {
                    counts[4] += 1;
                    let groups_only = fields[1] == "1";
                    let writing = fields[2] == "1";
                    let payload: Vec<_> = fields[4]
                        .as_bytes()
                        .chunks_exact(2)
                        .map(|pair| {
                            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
                        })
                        .collect();
                    let mut actual = Vec::new();
                    let mut cursor = 0;
                    let mut transfer = |field: &mut [u8]| {
                        if writing {
                            actual.extend_from_slice(field);
                        } else {
                            field.copy_from_slice(&payload[cursor..cursor + field.len()]);
                        }
                        cursor += field.len();
                        Ok::<_, ()>(())
                    };
                    if groups_only {
                        clock.transfer_groups(writing, &mut transfer).unwrap();
                    } else {
                        clock.transfer_state(writing, &mut transfer).unwrap();
                    }
                    assert_eq!(cursor, payload.len(), "native I/O length case {line}");
                    if writing {
                        assert_eq!(actual, payload, "native I/O order case {line}");
                    }
                }
                "step" => {
                    counts[5] += 1;
                    let input = ClockStepInputs {
                        counter_since_origin: i64::from_str_radix(fields[1], 16).unwrap(),
                        counter_frequency: i64::from_str_radix(fields[2], 16).unwrap(),
                        variable_step: word(fields[3]) != 0,
                        force_fixed_step: word(fields[4]) as i32,
                        fixed_step: float(fields[5]),
                        jitter_window: float(fields[6]),
                        rate_cap: word(fields[7]) as i32,
                        grow_step_limit: word(fields[8]) as i32,
                    };
                    assert_eq!(
                        clock.step(input),
                        fields[10] == "1",
                        "native update result case {line}"
                    );
                }
                _ => unreachable!(),
            }
            let expected: [u8; STATE_SIZE] = raw(fields[expected]);
            let differences: Vec<_> = clock
                .bytes
                .iter()
                .zip(&expected)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(12)
                .map(|(at, (a, b))| format!("{at:#x}: {a:02x} != {b:02x}"))
                .collect();
            assert!(
                differences.is_empty(),
                "original {} case {line}: {}",
                fields[0],
                differences.join(", ")
            );
        }
        assert_eq!(counts, [32, 140, 140, 140, 96, 1800, 7]);
    }
}
