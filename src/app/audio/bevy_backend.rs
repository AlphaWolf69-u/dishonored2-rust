//! One host output stream owns the sample clock and the shared bus histories.
//! ECS entities retain individual pause, stop and completion controls.
pub(super) mod buses;
use super::graph;
use buses::{Buses, FRAMES};
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};

#[derive(Resource, Default)]
pub(super) struct Engine {
    shared: Option<Arc<Mutex<Shared>>>,
}

struct Control { paused: AtomicBool, stopped: AtomicBool, finished: AtomicBool }

/// Despawning a sound entity also cancels the voice held by the output stream.
#[derive(Component)]
pub(super) struct VoiceHandle(Arc<Control>);
impl VoiceHandle {
    pub fn pause(&self, paused: bool) { self.0.paused.store(paused, Ordering::Relaxed); }
    pub fn finished(&self) -> bool { self.0.finished.load(Ordering::Relaxed) }
}
impl Drop for VoiceHandle {
    fn drop(&mut self) { self.0.stopped.store(true, Ordering::Relaxed); }
}

struct Voice { audio: graph::Routed, control: Arc<Control> }
struct Shared {
    buses: Buses,
    voices: Vec<Voice>,
    failed: bool,
    error: Option<String>,
}

impl Engine {
    pub fn new(mixer: &crate::gameplay::sound::mix::Mixer) -> Result<(Self, OutputAudio), String> {
        let base = crate::assets::installation::game_base().map_err(|error| error.to_string())?;
        let buses = Buses::new(mixer, &base)?;
        let shared = Arc::new(Mutex::new(Shared { buses, voices: Vec::new(), failed: false, error: None }));
        let output = OutputAudio { shared: shared.clone(), channels: ChannelCount::new(2).ok_or("invalid host stereo format")?,
            rate: SampleRate::new(48000).ok_or("invalid host processing rate")? };
        Ok((Self { shared: Some(shared) }, output))
    }

    pub fn submit(&self, audio: graph::ScheduledAudio) -> Result<VoiceHandle, String> {
        let shared = self.shared.as_ref().ok_or("shared audio output is unavailable")?;
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if shared.failed { return Err("shared audio output stopped after a processing error".into()); }
        let control = Arc::new(Control { paused: AtomicBool::new(false), stopped: AtomicBool::new(false), finished: AtomicBool::new(false) });
        shared.voices.push(Voice { audio: graph::Routed::new(audio)?, control: control.clone() });
        Ok(VoiceHandle(control))
    }

    pub fn update(&self, mixer: &crate::gameplay::sound::mix::Mixer) -> Result<(), String> {
        let Some(shared) = &self.shared else { return Ok(()) };
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = shared.error.take() { return Err(error); }
        shared.buses.update(mixer)
    }
}

impl Shared {
    fn block(&mut self) -> Result<[f32; FRAMES * 2], String> {
        self.buses.begin();
        for voice in &mut self.voices {
            if voice.control.stopped.load(Ordering::Relaxed) { voice.control.finished.store(true, Ordering::Relaxed); }
            else { voice.audio.begin(); }
        }
        for frame in 0..FRAMES {
            for voice in &mut self.voices {
                if voice.control.finished.load(Ordering::Relaxed) || voice.control.paused.load(Ordering::Relaxed) { continue; }
                if !voice.audio.frame(&mut self.buses, frame)? { voice.control.finished.store(true, Ordering::Relaxed); }
            }
        }
        self.voices.retain(|voice| !voice.control.finished.load(Ordering::Relaxed));
        Ok(*self.buses.finish()?)
    }
}

#[derive(Asset, TypePath, Clone)]
pub(super) struct OutputAudio {
    shared: Arc<Mutex<Shared>>,
    channels: ChannelCount,
    rate: SampleRate,
}

pub(super) struct Decoder {
    shared: Arc<Mutex<Shared>>,
    channels: ChannelCount,
    rate: SampleRate,
    block: [f32; FRAMES * 2],
    position: usize,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.position == self.block.len() {
            let mut shared = self.shared.lock().unwrap_or_else(|error| error.into_inner());
            if shared.failed { self.block.fill(0.0); }
            else {
                match shared.block() {
                    Ok(block) => self.block = block,
                    Err(error) => {
                        for voice in &shared.voices { voice.control.finished.store(true, Ordering::Relaxed); }
                        shared.voices.clear();
                        shared.error = Some(error);
                        shared.failed = true;
                        self.block.fill(0.0);
                    },
                }
            }
            self.position = 0;
        }
        let value = self.block[self.position];
        self.position += 1;
        Some(value)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> ChannelCount { self.channels }
    fn sample_rate(&self) -> SampleRate { self.rate }
    fn total_duration(&self) -> Option<Duration> { None }
}
impl Decodable for OutputAudio {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder { shared: self.shared.clone(), channels: self.channels, rate: self.rate,
            block: [0.0; FRAMES * 2], position: FRAMES * 2 }
    }
}

pub(super) fn retire(mut commands: Commands, voices: Query<(Entity, &VoiceHandle)>) {
    for (entity, voice) in &voices { if voice.finished() { commands.entity(entity).try_despawn(); } }
}
