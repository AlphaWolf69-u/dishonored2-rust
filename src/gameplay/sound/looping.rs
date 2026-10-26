//! Source-frame looping, independent of random/sequence container cycles.
use crate::assets::wwise::MediaProperties;

/// Version-113 AkPropID_Loop is 0x3a. It is an integer union member,
/// unlike pitch/gain properties. Zero is infinite and one is a single pass.
pub fn source_count(properties: &MediaProperties) -> Result<u16, String> {
    let source = properties
        .nodes
        .first()
        .ok_or("sound source has no property node")?;
    if source.ranges.iter().any(|range| {
        range.property == 0x3a && (range.min.to_bits() != 0 || range.max.to_bits() != 0)
    }) {
        return Err("randomized source loop counts require integer property sampling".into());
    }
    let value = source
        .values
        .iter()
        .find(|(property, _)| *property == 0x3a)
        .map_or(1, |(_, value)| *value);
    u16::try_from(value).map_err(|_| "source loop count exceeds the decoder's 16-bit count".into())
}

#[derive(Clone, Copy, Debug)]
pub struct SourceLoop {
    start: u64,
    end: u64,
    frames: u64,
    count: u16,
}
impl SourceLoop {
    /// 1411425c0/141142d50: inclusive file endpoints become exclusive buffer
    /// bounds; absent loop points use the entire media. Finite playback keeps
    /// the intro once, repeats the loop region, and plays the final tail once.
    pub fn new(points: Option<[u32; 2]>, frames: usize, count: u16) -> Result<Self, String> {
        u32::try_from(frames).map_err(|_| "source frame count exceeds the decoder's 32-bit range")?;
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
