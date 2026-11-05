//! Track clip timing (141159840 / 141159fd0 / 1411588a0).
//! Encoded media stays with the bank/source owner; these records hold no bytes.

/// The 40-byte serialized track record consumed by 141159840.
#[derive(Clone, Copy, Debug)]
pub struct SourceClip {
    pub track: u32,
    pub source: u32,
    pub position_ms: f64,
    pub trim_begin_ms: f64,
    pub trim_end_ms: f64,
    pub source_duration_ms: f64,
}

/// The six words retained in the track's clip array by 1411588a0.
#[derive(Clone, Copy, Debug)]
pub struct Clip {
    pub track: u32,
    pub source: u32,
    pub start: i32,
    pub duration: i32,
    pub source_duration: i32,
    pub source_offset: i32,
}

fn rounded_sample(value: f64) -> i32 {
    let rounded = (if value > 0.0 { 0.5 } else { -0.5 }) + value;
    // CVTTSD2SI returns integer-indefinite on invalid input; Rust saturates.
    if rounded >= -2147483648.0 && rounded < 2147483648.0 {
        rounded as i32
    } else {
        i32::MIN
    }
}

impl SourceClip {
    /// Uses the actual engine rate (142427c24). No music-specific rate is
    /// chosen. Fully trimmed clips are skipped before creating a record.
    pub fn prepare(self, sample_rate: u32) -> Result<Option<Clip>, String> {
        let duration_ms = (self.trim_end_ms + self.source_duration_ms) - self.trim_begin_ms;
        if duration_ms <= 0.0 {
            return Ok(None);
        }
        let rate = f64::from(sample_rate);
        let source_duration = rounded_sample(rate * self.source_duration_ms * 0.001);
        // Host: diagnose invalid bank data instead of the source's integer
        // divide exception. Do not fabricate a replacement duration.
        if source_duration == 0 {
            return Err("music clip source duration rounds to zero samples".into());
        }
        let offset = rounded_sample(rate * self.trim_begin_ms * 0.001);
        let mut source_offset = offset
            .checked_rem(source_duration)
            .ok_or("music clip source offset division overflows")?;
        if source_offset < 0 {
            source_offset = source_offset.wrapping_add(source_duration);
        }
        Ok(Some(Clip {
            track: self.track,
            source: self.source,
            // This multiplication's operand order differs from the other
            // three conversions in 1411588a0; keep it separate.
            start: rounded_sample((self.position_ms + self.trim_begin_ms) * rate * 0.001),
            duration: rounded_sample(rate * duration_ms * 0.001),
            source_duration,
            source_offset,
        }))
    }
}
