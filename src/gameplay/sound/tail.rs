//! Effect tail frames and buffer state (14116e950).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferState { DataReady, NoMoreData }

#[derive(Clone, Copy, Debug)]
pub struct BufferStatus {
    pub state: BufferState,
    pub valid_frames: u16,
}

pub struct Tail {
    remaining: u32,
    length: u32,
}
impl Default for Tail {
    fn default() -> Self { Self { remaining: u32::MAX, length: 0 } } // 14116dfa0
}
impl Tail {
    pub fn prepare(&mut self, channels: &mut [&mut [f32]], mut status: BufferStatus, duration: u32) -> Result<BufferStatus, String> {
        let capacity = channels.first().ok_or("effect tail has no channels")?.len();
        if capacity > usize::from(u16::MAX) || usize::from(status.valid_frames) > capacity
            || channels.iter().any(|channel| channel.len() != capacity)
        { return Err("effect tail buffer has inconsistent capacity or valid frames".into()); }
        if status.state != BufferState::NoMoreData {
            self.remaining = u32::MAX;
            return Ok(status);
        }
        if self.remaining == 0 && status.valid_frames == 0 { return Ok(status); }
        if self.remaining == u32::MAX || status.valid_frames != 0 {
            self.remaining = duration;
            self.length = duration;
        } else if self.length < duration {
            self.remaining = self.remaining.wrapping_sub(self.length).wrapping_add(duration);
            self.length = duration;
        }
        let padding = (capacity - usize::from(status.valid_frames)) as u32;
        self.remaining -= padding.min(self.remaining);
        if padding != 0 {
            for channel in channels { channel[usize::from(status.valid_frames)..].fill(0.0); }
            // The original fills the whole quantum, even on the final tail
            // block, rather than returning only the remaining tail samples.
            status.valid_frames = capacity as u16;
        }
        if self.remaining != 0 { status.state = BufferState::DataReady; }
        Ok(status)
    }
}
