//! Front/mono/LFE room-effect processing (141168ff0/141169450/14116ab60).
use super::{tone::ToneControls, Block, EarlyReflections, InputMix, LateReverb, Parameters};
use crate::assets::sound_effects::ReflectionPatterns;
use std::sync::Arc;

pub struct RoomReverb {
    parameters: Parameters,
    structural: Parameters,
    previous: Parameters,
    rate: u32,
    mask: u32,
    send: bool,
    patterns: Arc<ReflectionPatterns>,
    input: InputMix,
    early: Option<EarlyReflections>,
    late: LateReverb,
    tone: ToneControls,
}
impl RoomReverb {
    pub fn new(mut parameters: Parameters, patterns: Arc<ReflectionPatterns>, rate: u32,
        speaker_mask: u32, send: bool) -> Result<Self, String>
    {
        if !matches!(speaker_mask, 3 | 4 | 8 | 11 | 12) {
            return Err(format!("room reverb speaker mask {speaker_mask:#x} requires its surround processing path"));
        }
        if rate == 0 { return Err("room reverb requires a nonzero sample rate".into()); }
        if send { parameters.dry_level = 0.0; }
        let input = parameters.input_mix(speaker_mask);
        let early = parameters.early_reflection_unit(&patterns, rate, speaker_mask)?;
        let late = LateReverb::new(&parameters, rate)?;
        let tone = ToneControls::new(&parameters, early.is_some(), rate)?;
        parameters.rtpc_dirty = false;
        parameters.room_dirty = false;
        Ok(Self { structural: parameters.clone(), previous: parameters.clone(), parameters,
            rate, mask: speaker_mask, send, patterns, input, early, late, tone })
    }

    pub fn parameters(&self) -> &Parameters { &self.parameters }
    pub fn set_parameter(&mut self, id: u16, bytes: &[u8]) -> Result<(), String> { self.parameters.set(id, bytes) }

    fn prepare(&mut self) -> Result<(), String> {
        if self.parameters.room_dirty {
            self.late.configure_room(&self.structural, &self.parameters)?;
            if self.structural.early_reflections != self.parameters.early_reflections
                || self.structural.early_pattern != self.parameters.early_pattern
                || self.structural.early_room_size != self.parameters.early_room_size
            {
                self.early = self.parameters.early_reflection_unit(&self.patterns, self.rate, self.mask)?;
            }
            self.tone.configure(&self.structural, &self.parameters, self.early.is_some(), self.rate)?;
            self.input = self.parameters.input_mix(self.mask);
            self.structural = self.parameters.clone();
            self.parameters.room_dirty = false;
        }
        if self.parameters.rtpc_dirty {
            self.late.configure_rtpc(&self.previous, &self.parameters);
            self.tone.configure_rtpc(&self.previous, &self.parameters, self.early.is_some(), self.rate)?;
            if self.send { self.parameters.dry_level = 0.0; }
            self.parameters.rtpc_dirty = false;
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        if let Some(early) = &mut self.early { early.reset(); }
        self.late.reset();
        self.tone.reset();
    }

    /// Wwise planar channel order: non-LFE mask bits ascending, then LFE.
    /// The effect owner supplies valid audio/tail frames in multiples of four.
    pub fn process(&mut self, channels: &mut [&mut [f32]]) -> Result<(), String> {
        if channels.len() != self.mask.count_ones() as usize { return Err("room reverb channels do not match the speaker mask".into()); }
        let count = channels[0].len();
        if count % 4 != 0 || channels.iter().any(|channel| channel.len() != count) {
            return Err("room reverb requires equal channel lengths divisible by four".into());
        }
        self.prepare()?;
        let mut mono = Block([0.0; 256]);
        let mut late_left = Block([0.0; 256]);
        let mut late_right = Block([0.0; 256]);
        let mut early_left = Block([0.0; 256]);
        let mut early_right = Block([0.0; 256]);
        let mut previous = self.previous.clone();
        for start in (0..count).step_by(256) {
            let frames = (count - start).min(256);
            let mono = &mut mono.0[..frames];
            for (offset, sample) in mono.iter_mut().enumerate() {
                let mut frame = [0.0; 3];
                for (index, channel) in channels.iter().enumerate() { frame[index] = channel[start + offset]; }
                *sample = self.input.frame(&frame[..channels.len()])?;
            }
            self.tone.input(mono);
            let early_left = &mut early_left.0[..frames];
            let early_right = &mut early_right.0[..frames];
            if let Some(early) = &mut self.early {
                early.process(mono, early_left, early_right)?;
                self.tone.early(early_left, early_right);
            }
            let late_left = &mut late_left.0[..frames];
            let late_right = &mut late_right.0[..frames];
            let tone = &mut self.tone;
            self.late.process(mono, late_left, late_right, |samples| { tone.late(samples); Ok(()) })?;
            let old_width = width(previous.stereo_width, true);
            let new_width = width(self.parameters.stereo_width, false);
            let mut channel = 0;
            for bit in [1, 2, 4, 8] {
                if self.mask & bit == 0 { continue; }
                let output = &mut channels[channel][start..start + frames];
                let (old_pan, new_pan) = match bit {
                    1 => (old_width, new_width),
                    2 => ([old_width[1], old_width[0]], [new_width[1], new_width[0]]),
                    _ => ([f32::from_bits(0x3f3504e6); 2], [f32::from_bits(0x3f3504e6); 2]),
                };
                let old_level = if bit == 8 { previous.reverb_level * previous.channel_levels[3] } else { previous.reverb_level };
                let level = if bit == 8 { self.parameters.reverb_level * self.parameters.channel_levels[3] } else { self.parameters.reverb_level };
                mix(output, late_left, late_right,
                    [previous.dry_level, old_level * old_pan[0], old_level * old_pan[1]],
                    [self.parameters.dry_level, level * new_pan[0], level * new_pan[1]]);
                if self.early.is_some() && bit != 8 {
                    mix(output, early_left, early_right,
                        [1.0, previous.early_level * old_pan[0], previous.early_level * old_pan[1]],
                        [1.0, self.parameters.early_level * new_pan[0], self.parameters.early_level * new_pan[1]]);
                }
                channel += 1;
            }
            previous = self.parameters.clone();
        }
        // 141169450 leaves its audio snapshot untouched on zero valid frames.
        if count != 0 { self.previous = self.parameters.clone(); }
        Ok(())
    }
}

fn width(value: f32, previous: bool) -> [f32; 2] {
    let main = value * f32::from_bits(0x3bb60b61) * f32::from_bits(0x3e95f634) + f32::from_bits(0x3f3504e6);
    let complement = 1.0 - main * main;
    let cross = if previous {
        if complement <= 0.0 { 0.0 } else { complement.sqrt() }
    } else if 0.0 < complement { complement.sqrt() } else { 0.0 };
    [main, cross]
}

/// 1411721d0's constant/ramped three-input mix. The variable-gain path's
/// first lane adds the dry term first; the other lanes add the wet term first.
fn mix(output: &mut [f32], left: &[f32], right: &[f32], mut old: [f32; 3], new: [f32; 3]) {
    if old == new {
        for ((sample, left), right) in output.iter_mut().zip(left).zip(right) { *sample = left * old[1] + *sample * old[0] + right * old[2]; }
        return;
    }
    let inverse = 1.0 / output.len() as f32;
    let steps: [f32; 3] = std::array::from_fn(|index| (new[index] - old[index]) * inverse);
    for (index, ((sample, left), right)) in output.iter_mut().zip(left).zip(right).enumerate() {
        *sample = if index & 3 == 0 { old[0] * *sample + old[1] * left + old[2] * right }
            else { old[1] * left + old[0] * *sample + old[2] * right };
        for (gain, step) in old.iter_mut().zip(steps) { *gain += step; }
    }
}
