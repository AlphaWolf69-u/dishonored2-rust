//! Source-frame looping, independent of random/sequence container cycles.
use crate::assets::wwise::MediaProperties;

/// Version-113 AkPropID_Loop is 0x3a. It is an integer union member,
/// unlike pitch/gain properties. Zero is infinite and one is a single pass.
/// Bank enum: https://github.com/bnnm/wwiser/blob/master/wwiser/parser/wdefs.py
/// Source decoder count/interval: 1411425c0, 141142d50 and 1411606d0.
#[derive(Clone, Copy, Debug)]
pub struct LoopCount {
    pub count: u32,
    pub modifiers: Option<[i32; 2]>,
}
impl LoopCount {
    pub fn read(properties: &MediaProperties) -> Result<Self, String> {
        let source = properties
            .nodes
            .first()
            .ok_or("sound source has no property node")?;
        let value = source
            .values
            .iter()
            .find(|(property, _)| *property == 0x3a)
            .map_or(1, |(_, value)| *value);
        let modifiers = source
            .ranges
            .iter()
            .find(|range| range.property == 0x3a)
            .map(|range| [range.min.to_bits() as i32, range.max.to_bits() as i32]);
        Ok(Self {
            count: value,
            modifiers,
        })
    }

    /// 141122630 adds the signed integer modifier before the caller narrows
    /// to its 16-bit decoder counter. This also applies when the base is zero.
    pub fn sample(self, mut random15: impl FnMut() -> u16) -> u16 {
        let Some([minimum, maximum]) = self.modifiers else {
            return self.count as u16;
        };
        let difference = maximum.wrapping_sub(minimum);
        let offset = if difference == 0 {
            0
        } else {
            (f64::from(random15()) * f64::from_bits(0x3f00002000400080) * f64::from(difference)
                + 0.5) as i32
        };
        self.count.wrapping_add(minimum.wrapping_add(offset) as u32) as u16
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SourceLoop {
    start: u64,
    end: u64,
    frames: u64,
    count: u16,
}
impl SourceLoop {
    pub fn with_count(mut self, count: u16) -> Self {
        self.count = count;
        self
    }

    /// 141149850: finite decoder duration, before the PBI pitch adjustment
    /// in 1411287f0. Zero denotes an infinite source, not a zero-length clip.
    /// Keep the float conversions and multiplication order of the decoder.
    pub fn duration_ms(self, sample_rate: u32) -> f32 {
        if self.count == 0 { return 0.0; }
        let repeated = (self.end - self.start) as f32 * f32::from(self.count - 1);
        (repeated + self.frames as f32) * 1000.0 / sample_rate as f32
    }
    /// 1411425c0/141142d50: inclusive file endpoints become exclusive buffer
    /// bounds; absent loop points use the entire media. Finite playback keeps
    /// the intro once, repeats the loop region, and plays the final tail once.
    pub fn new(points: Option<[u32; 2]>, frames: usize, count: u16) -> Result<Self, String> {
        u32::try_from(frames)
            .map_err(|_| "source frame count exceeds the decoder's 32-bit range")?;
        let [start, end] = points
            .map(|[start, end]| [u64::from(start), u64::from(end)])
            .unwrap_or([0, frames as u64]);
        if start >= end || end > frames as u64 {
            return Err("source loop interval exceeds media".into());
        }
        Ok(Self {
            start,
            end,
            frames: frames as u64,
            count,
        })
    }

    pub fn frame(self, frame: u64) -> Option<usize> {
        let frame = if self.count == 1 || frame < self.end {
            frame
        } else if self.count == 0 {
            self.start + (frame - self.end) % (self.end - self.start)
        } else {
            let repeated = (self.end - self.start) * u64::from(self.count - 1);
            if frame - self.end < repeated {
                self.start + (frame - self.end) % (self.end - self.start)
            } else {
                frame - repeated
            }
        };
        (frame < self.frames)
            .then(|| usize::try_from(frame).ok())
            .flatten()
    }
}
