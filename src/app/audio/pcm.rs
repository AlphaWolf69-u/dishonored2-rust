//! Host audio output for already decoded samples. The playback task only
//! iterates PCM; all container/codec errors are returned by the asset reader.
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{io, sync::Arc, time::Duration};

#[derive(Asset, TypePath, Clone)]
pub(super) struct PcmAudio {
    samples: Arc<[f32]>,
    channels: ChannelCount,
    sample_rate: SampleRate,
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
        Ok(Self { samples: audio.samples.into(), channels, sample_rate })
    }
}

pub(super) struct PcmDecoder {
    audio: PcmAudio,
    position: usize,
}

impl Iterator for PcmDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let sample = *self.audio.samples.get(self.position)?;
        self.position += 1;
        Some(sample)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.audio.samples.len() - self.position;
        (remaining, Some(remaining))
    }
}

impl Source for PcmDecoder {
    fn current_span_len(&self) -> Option<usize> { Some(self.audio.samples.len() - self.position) }
    fn channels(&self) -> ChannelCount { self.audio.channels }
    fn sample_rate(&self) -> SampleRate { self.audio.sample_rate }
    fn total_duration(&self) -> Option<Duration> {
        let frames = self.audio.samples.len() as u64 / u64::from(self.audio.channels.get());
        let rate = u64::from(self.audio.sample_rate.get());
        Some(Duration::new(frames / rate, ((frames % rate) * 1_000_000_000 / rate) as u32))
    }
}

impl Decodable for PcmAudio {
    type Decoder = PcmDecoder;
    fn decoder(&self) -> PcmDecoder { PcmDecoder { audio: self.clone(), position: 0 } }
}
