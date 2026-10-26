//! Host audio output for already decoded samples. The playback task only
//! iterates PCM; all container/codec errors are returned by the asset reader.
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{io, sync::Arc, time::Duration};

#[derive(Asset, TypePath, Clone)]
pub(super) struct PcmAudio {
    pub(super) samples: Arc<[f32]>,
    pub(super) channels: ChannelCount,
    pub(super) sample_rate: SampleRate,
    pub(super) loop_points: Option<[u32; 2]>,
    fade: Option<super::fade::Control>,
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
        Ok(Self { samples: audio.samples.into(), channels, sample_rate, loop_points: audio.loop_points, fade: None })
    }

    pub fn controlled(&self) -> (Self, super::fade::Control) {
        let control = super::fade::Control::default();
        let mut audio = self.clone();
        audio.fade = Some(control.clone());
        (audio, control)
    }
}

pub(super) struct PcmDecoder {
    audio: PcmAudio,
    position: usize,
    envelope: Option<super::fade::Envelope>,
    gain: f32,
}

impl Iterator for PcmDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let channels = usize::from(self.audio.channels.get());
        if self.position % channels == 0 {
            let frame = (self.position / channels) as u64;
            if let Some((milliseconds, curve)) = self.audio.fade.as_ref().and_then(|control| control.take()) {
                let from = self.envelope.and_then(|envelope| envelope.value(frame)).unwrap_or(self.gain);
                self.envelope = Some(super::fade::Envelope::new(frame, super::fade::frames(milliseconds, self.audio.sample_rate.get()), from, curve));
            }
            self.gain = match self.envelope { Some(envelope) => envelope.value(frame)?, None => 1.0 };
        }
        let sample = *self.audio.samples.get(self.position)?;
        self.position += 1;
        Some(sample * self.gain)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.audio.samples.len() - self.position;
        (if self.audio.fade.is_some() { 0 } else { remaining }, Some(remaining))
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
    fn decoder(&self) -> PcmDecoder { PcmDecoder { audio: self.clone(), position: 0, envelope: None, gain: 1.0 } }
}
