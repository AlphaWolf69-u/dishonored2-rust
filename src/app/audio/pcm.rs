//! Decoded source media shared by playback instances. Container and codec
//! errors are returned before submission to the output worker.
use bevy::audio::{ChannelCount, SampleRate};
use std::{io, sync::Arc};

#[derive(Clone)]
pub(super) struct PcmAudio {
    pub(super) samples: Arc<[f32]>,
    pub(super) channels: ChannelCount,
    pub(super) sample_rate: SampleRate,
    pub(super) loop_points: Option<[u32; 2]>,
}

impl PcmAudio {
    pub fn from_wem(bytes: &[u8]) -> io::Result<Self> {
        let audio = crate::assets::wwise::decode_wem_pcm(bytes)?;
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid decoded Wwise sample format");
        let channels = ChannelCount::new(audio.channels).ok_or_else(invalid)?;
        let sample_rate = SampleRate::new(audio.sample_rate).ok_or_else(invalid)?;
        if audio.samples.len() % usize::from(channels.get()) != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "partial decoded Wwise sample frame"));
        }
        Ok(Self { samples: audio.samples.into(), channels, sample_rate, loop_points: audio.loop_points })
    }
}
