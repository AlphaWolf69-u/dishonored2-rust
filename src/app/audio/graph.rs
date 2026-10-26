//! Sample-clock playback of nested Wwise containers. Media is decoded before
//! submission; the audio worker only selects nodes and mixes validated PCM.
use super::pcm::PcmAudio;
use crate::{assets::wwise::{ContainerPlayback, MediaProperties, PlaybackNode, SelectionMode, TransitionMode},
    gameplay::sound::{containers::{transition_ms, Loops, Selection}, filters::{Kind as FilterKind, VoiceFilter},
        mix::{Mixer, power10}, properties::RandomProperties, resampler::{pitch_step, Resampler}, gain::GainRamp, Emitter}};
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{collections::HashMap, io, sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}}, time::Duration};

pub(super) type Selections = Arc<Mutex<HashMap<(u32, Option<Emitter>), Selection>>>;

struct Sound {
    pcm: PcmAudio,
    properties: Arc<MediaProperties>,
    spatial: Option<super::spatial::Voice>,
}
struct Container {
    children: Vec<usize>,
    weights: Vec<u32>,
    mode: SelectionMode,
    global: bool,
    reset: bool,
    avoid: u16,
    playback: ContainerPlayback,
}
enum Kind { Sound(usize), Layer(Vec<usize>), Container(Container) }
struct Node { id: u32, kind: Kind }

pub(super) struct Definition {
    nodes: Vec<Node>,
    sounds: Vec<Sound>,
    root: usize,
    channels: ChannelCount,
    rate: SampleRate,
}

fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }
fn random15() -> u16 { rand::random_range(0..=32767) }

impl Definition {
    pub fn required(node: &PlaybackNode) -> bool {
        match node {
            PlaybackNode::Sound { variant, .. } => variant.properties.nodes.iter()
                .find(|node| node.positioning_flags & 1 != 0).is_some_and(|node| node.positioning_3d.is_some())
                || variant.properties.nodes.iter().any(|node| {
                    [2, 3, 4].iter().any(|property| node.float(*property) != 0.0)
                        || node.curves.iter().any(|curve| matches!(curve.property, 2 | 3 | 4))
                        || node.ranges.iter().any(|range| matches!(range.property, 0 | 2 | 3 | 4 | 0x21)
                            && (range.min != 0.0 || range.max != 0.0))
                }),
            PlaybackNode::Layer { children, .. } => children.iter().any(Self::required),
            PlaybackNode::Container(container) => container.playback.continuous
                || container.children.iter().any(|(child, _)| !matches!(child, PlaybackNode::Sound { .. }) || Self::required(child)),
        }
    }

    pub fn decode(root: PlaybackNode, attenuations: &HashMap<u32, Arc<crate::assets::wwise::Attenuation>>) -> io::Result<Self> {
        fn node(source: PlaybackNode, nodes: &mut Vec<Node>, sounds: &mut Vec<Sound>,
            attenuations: &HashMap<u32, Arc<crate::assets::wwise::Attenuation>>) -> io::Result<usize> {
            let (id, kind) = match source {
                PlaybackNode::Sound { node, variant } => {
                    if variant.properties.nodes.iter().any(|node| node.ranges.iter().any(|range|
                        matches!(range.property, 5 | 0x17) && (range.min != 0.0 || range.max != 0.0))) {
                        return Err(invalid("scheduled Wwise voices require randomized output-bus gain lifetimes"));
                    }
                    let pcm = PcmAudio::from_wem(&variant.media)?;
                    if pcm.samples.is_empty() { return Err(invalid("scheduled Wwise sound contains no samples")); }
                    let sound = sounds.len();
                    let spatial = super::spatial::Voice::read(&variant.properties, attenuations).map_err(invalid)?;
                    sounds.push(Sound { pcm, properties: Arc::new(variant.properties), spatial });
                    (node, Kind::Sound(sound))
                },
                PlaybackNode::Layer { node: id, children } => {
                    let children = children.into_iter().map(|child| node(child, nodes, sounds, attenuations)).collect::<io::Result<_>>()?;
                    (id, Kind::Layer(children))
                },
                PlaybackNode::Container(container) => {
                    if container.playback.continuous {
                        if matches!(container.playback.transition, TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower) {
                            return Err(invalid("continuous Wwise crossfades require voice envelopes"));
                        }
                    }
                    let mut children = Vec::new();
                    let mut weights = Vec::new();
                    for (child, weight) in container.children {
                        children.push(node(child, nodes, sounds, attenuations)?);
                        weights.push(weight);
                    }
                    (container.node, Kind::Container(Container { children, weights, mode: container.mode,
                        global: container.global, reset: container.reset_each_play, avoid: container.avoid_repeat,
                        playback: container.playback }))
                },
            };
            let index = nodes.len();
            nodes.push(Node { id, kind });
            Ok(index)
        }
        let mut nodes = Vec::new();
        let mut sounds = Vec::new();
        let root = node(root, &mut nodes, &mut sounds, attenuations)?;
        let first = sounds.first().ok_or_else(|| invalid("Wwise playback graph has no media"))?;
        let spatial = sounds.iter().any(|sound| sound.spatial.is_some());
        let channels = if spatial { ChannelCount::new(2).ok_or_else(|| invalid("invalid stereo channel count"))? } else { first.pcm.channels };
        if channels.get() > 8 { return Err(invalid("scheduled Wwise graph exceeds eight output channels")); }
        // The original engine's DSP/output clock is 48 kHz (142427c24).
        // Each source's fixed-point resampler converts its own media rate.
        let rate = SampleRate::new(48000).ok_or_else(|| invalid("invalid output sample rate"))?;
        if sounds.iter().any(|sound| if sound.spatial.is_some() { sound.pcm.channels.get() != 1 } else { sound.pcm.channels != channels }) {
            return Err(invalid("scheduled Wwise graph requires channel format conversion"));
        }
        for entry in &nodes {
            if let Kind::Container(container) = &entry.kind {
                if container.playback.continuous && sounds.iter().any(|sound| sound.properties.nodes.iter()
                    .any(|node| node.id == entry.id && node.curves.iter().any(|curve| curve.property == 7))) {
                    return Err(invalid("scheduled Wwise transition RTPCs require live timing evaluation"));
                }
            }
        }
        Ok(Self { nodes, sounds, root, channels, rate })
    }

    pub fn root_properties(&self) -> Arc<MediaProperties> {
        let first = &self.sounds[0].properties;
        let nodes = first.nodes.iter().skip_while(|node| node.id != self.nodes[self.root].id).cloned().collect();
        Arc::new(MediaProperties { nodes, bus: first.bus })
    }

    pub fn prepare(self: &Arc<Self>, selections: Selections, emitter: Emitter, mixer: &Mixer,
        parameter: Option<(u32, f32)>, scene: &super::spatial::Scene, position: Option<super::spatial::Position>) -> Result<(ScheduledAudio, Playing), String>
    {
        let control = Arc::new(Control { volumes_db: (0..self.sounds.len()).map(|_| AtomicU32::new(0)).collect(),
            spatial: (0..self.sounds.len()).map(|_| [AtomicU32::new(1.0f32.to_bits()), AtomicU32::new(1.0f32.to_bits()), AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            pitches: (0..self.sounds.len()).map(|_| AtomicU32::new(0)).collect(),
            filters: (0..self.sounds.len()).map(|_| [AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            threshold_db: AtomicU32::new(mixer.volume_threshold_db.to_bits()), stops: Mutex::new(Vec::new()) });
        let playing = Playing { definition: self.clone(), control: control.clone(), parameter, position, emitter };
        playing.update(mixer, scene)?;
        Ok((ScheduledAudio { definition: self.clone(), control, selections, emitter }, playing))
    }
}

struct Control { spatial: Vec<[AtomicU32; 4]>, volumes_db: Vec<AtomicU32>, pitches: Vec<AtomicU32>, filters: Vec<[AtomicU32; 2]>,
    threshold_db: AtomicU32, stops: Mutex<Vec<(u32, i32, u8)>> }

#[derive(Component, Clone)]
pub(super) struct Playing {
    definition: Arc<Definition>,
    control: Arc<Control>,
    parameter: Option<(u32, f32)>,
    position: Option<super::spatial::Position>,
    emitter: Emitter,
}
impl Playing {
    pub fn stop(&self, target: u32, milliseconds: i32, curve: u8) {
        // An action can target a parent outside this Play action's subtree.
        let target = if self.definition.root_properties().nodes.iter().any(|node| node.id == target) {
            self.definition.nodes[self.definition.root].id
        } else { target };
        self.control.stops.lock().unwrap_or_else(|error| error.into_inner()).push((target, milliseconds, curve));
    }
    pub fn update(&self, mixer: &Mixer, scene: &super::spatial::Scene) -> Result<(), String> {
        for (sound, targets) in self.definition.sounds.iter().zip(&self.control.spatial) {
            if let Some(spatial) = &sound.spatial {
                let (gains, filters) = spatial.targets(scene, self.position.or_else(|| scene.emitter(self.emitter)))?;
                for (target, value) in targets.iter().zip([gains[0], gains[1], filters[0], filters[1]]) {
                    target.store(value.to_bits(), Ordering::Relaxed);
                }
            }
        }
        for (sound, volume) in self.definition.sounds.iter().zip(&self.control.volumes_db) {
            volume.store(mixer.volume_db_with_parameter(&sound.properties, self.parameter)?.to_bits(), Ordering::Relaxed);
        }
        for (sound, filters) in self.definition.sounds.iter().zip(&self.control.filters) {
            for (target, property) in filters.iter().zip([3, 4]) {
                target.store(mixer.source_property(&sound.properties, property, self.parameter)?.to_bits(), Ordering::Relaxed);
            }
        }
        for (sound, pitch) in self.definition.sounds.iter().zip(&self.control.pitches) {
            let cents = mixer.source_property(&sound.properties, 2, self.parameter)?;
            // Validate the complete authored random range before the worker
            // starts a voice; malformed pitch must remain a host-side error.
            let mut bounds = [cents; 2];
            for node in &sound.properties.nodes {
                if let Some(range) = node.ranges.iter().find(|range| range.property == 2) {
                    bounds[0] += range.min;
                    bounds[1] += range.max;
                }
            }
            for bound in bounds { pitch_step(bound, sound.pcm.sample_rate.get(), self.definition.rate.get())?; }
            pitch.store(cents.to_bits(), Ordering::Relaxed);
        }
        self.control.threshold_db.store(mixer.volume_threshold_db.to_bits(), Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Asset, TypePath, Clone)]
pub(super) struct ScheduledAudio {
    definition: Arc<Definition>,
    control: Arc<Control>,
    selections: Selections,
    emitter: Emitter,
}

enum State {
    Sound { frame: u64, random: RandomProperties, resampler: Option<Resampler>, gain: GainRamp, filters: Vec<[VoiceFilter; 2]> },
    Layer(Vec<Runtime>),
    Container { active: Vec<Runtime>, selection: Selection, loops: Loops, next: Option<u64>, initialized: bool, exhausted: bool },
    Done,
}
struct Runtime { node: usize, state: State, fade: Option<super::fade::Envelope> }

impl Runtime {
    fn new(node: usize, now: u64, definition: &Definition) -> Self {
        let state = match &definition.nodes[node].kind {
            // Random modifiers belong to each newly started source voice,
            // including when the same leaf overlaps itself in a sequence.
            Kind::Sound(index) => State::Sound { frame: 0,
                random: RandomProperties::sample(&definition.sounds[*index].properties, random15),
                resampler: None,
                gain: GainRamp::default(),
                filters: (0..definition.sounds[*index].pcm.channels.get()).map(|_| [
                    VoiceFilter::new(FilterKind::LowPass, definition.rate.get(), 1024, false),
                    VoiceFilter::new(FilterKind::HighPass, definition.rate.get(), 1024, false),
                ]).collect() },
            Kind::Layer(children) => State::Layer(children.iter().map(|child| Self::new(*child, now, definition)).collect()),
            Kind::Container(container) => State::Container { active: Vec::new(), selection: Selection::default(),
                loops: Loops::new(&container.playback, random15), next: Some(now), initialized: false, exhausted: false },
        };
        Self { node, state, fade: None }
    }

    fn halt_new(&mut self) {
        match &mut self.state {
            State::Container { active, next, exhausted, .. } => {
                *next = None;
                *exhausted = true;
                for child in active { child.halt_new(); }
            },
            State::Layer(children) => for child in children { child.halt_new(); },
            _ => {},
        }
    }

    fn stop(&mut self, target: u32, milliseconds: i32, curve: u8, now: u64, definition: &Definition) {
        if definition.nodes[self.node].id == target {
            if milliseconds <= 0 { self.state = State::Done; }
            else {
                let gain = self.fade.map_or(1.0, |fade| fade.value(now).unwrap_or(0.0));
                self.fade = Some(super::fade::Envelope::new(now, super::fade::frames(milliseconds, definition.rate.get()), gain, curve));
                self.halt_new();
            }
            return;
        }
        match &mut self.state {
            State::Layer(children) | State::Container { active: children, .. } => {
                for child in children { child.stop(target, milliseconds, curve, now, definition); }
            },
            _ => {},
        }
    }

    fn frame(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32]) -> bool {
        let gain = match self.fade { Some(fade) => match fade.value(now) { Some(gain) => gain, None => return false }, None => 1.0 };
        let mut local = [0.0; 8];
        let alive = self.frame_inner(now, audio, &mut local[..output.len()]);
        for (out, sample) in output.iter_mut().zip(local) { *out += sample * gain; }
        alive
    }

    fn frame_inner(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32]) -> bool {
        let definition = &audio.definition;
        let node = &definition.nodes[self.node];
        match (&mut self.state, &node.kind) {
            (State::Sound { frame, random, resampler, gain: ramp, filters }, Kind::Sound(index)) => {
                let sound = &definition.sounds[*index];
                if *frame % 1024 == 0 {
                    let cents = f32::from_bits(audio.control.pitches[*index].load(Ordering::Relaxed)) + random.pitch_cents;
                    let Ok(step) = pitch_step(cents, sound.pcm.sample_rate.get(), definition.rate.get()) else { return false };
                    match resampler {
                        Some(resampler) => resampler.set_step(step, true),
                        None => *resampler = Some(Resampler::new(step)),
                    }
                }
                let Some(resampler) = resampler else { return false };
                let channels = usize::from(sound.pcm.channels.get());
                let mut samples = [0.0; 8];
                for (channel, sample) in samples[..channels].iter_mut().enumerate() {
                    let Some(value) = resampler.sample(&sound.pcm.samples, channels, channel) else { return false };
                    *sample = value;
                }
                let volume = f32::from_bits(audio.control.volumes_db[*index].load(Ordering::Relaxed))
                    + random.volume_db + random.makeup_db;
                let threshold = f32::from_bits(audio.control.threshold_db.load(Ordering::Relaxed));
                let gain = if volume <= threshold { 0.0 } else { power10(volume * 0.05) };
                if *frame % 1024 == 0 {
                    let mut target = [gain; 8];
                    if sound.spatial.is_some() {
                        for (channel, target) in target[..output.len()].iter_mut().enumerate() {
                            *target *= f32::from_bits(audio.control.spatial[*index][channel].load(Ordering::Relaxed));
                        }
                    }
                    ramp.begin(target, 1024);
                }
                let gains = ramp.next_frame();
                let low_pass = f32::from_bits(audio.control.filters[*index][0].load(Ordering::Relaxed)) + random.low_pass;
                let high_pass = f32::from_bits(audio.control.filters[*index][1].load(Ordering::Relaxed)) + random.high_pass;
                let distance_low = f32::from_bits(audio.control.spatial[*index][2].load(Ordering::Relaxed));
                let distance_high = f32::from_bits(audio.control.spatial[*index][3].load(Ordering::Relaxed));
                for (sample, filters) in samples[..channels].iter_mut().zip(filters) {
                    if *frame % 1024 == 0 {
                        filters[0].set_target(low_pass.max(distance_low));
                        filters[1].set_target(high_pass.max(distance_high));
                    }
                    *sample = filters[0].process(*sample);
                    *sample = filters[1].process(*sample);
                }
                if sound.spatial.is_some() {
                    for (channel, out) in output.iter_mut().enumerate() {
                        *out += samples[0] * gains[channel];
                    }
                } else {
                    for ((out, sample), gain) in output.iter_mut().zip(samples).zip(gains) { *out += sample * gain; }
                }
                resampler.advance();
                *frame += 1;
                resampler.sample(&sound.pcm.samples, channels, 0).is_some()
            },
            (State::Layer(children), _) => {
                children.retain_mut(|child| child.frame(now, audio, output));
                !children.is_empty()
            },
            (State::Container { active, selection, loops, next, initialized, exhausted }, Kind::Container(container)) => {
                let key = (node.id, if container.global { None } else { Some(audio.emitter) });
                if next.is_some_and(|deadline| now >= deadline) && !*exhausted {
                    let selected = if container.playback.continuous {
                        if !*initialized && container.mode == SelectionMode::Sequence && !container.reset {
                            if let Some(shared) = audio.selections.lock().unwrap_or_else(|error| error.into_inner()).get(&key) {
                                selection.sequence = shared.sequence.clone();
                            }
                        }
                        let selected = selection.next(container.mode, &container.weights, container.avoid,
                            container.playback.restart_backward, Some(loops));
                        if container.mode == SelectionMode::Sequence && !container.reset {
                            audio.selections.lock().unwrap_or_else(|error| error.into_inner()).entry(key).or_default().sequence = selection.sequence.clone();
                        }
                        selected
                    } else {
                        let mut states = audio.selections.lock().unwrap_or_else(|error| error.into_inner());
                        let state = states.entry(key).or_default();
                        if container.reset && container.mode == SelectionMode::Sequence { state.sequence = Default::default(); }
                        state.next(container.mode, &container.weights, container.avoid, container.playback.restart_backward, None)
                    };
                    *initialized = true;
                    *next = None;
                    if let Some(index) = selected {
                        active.push(Self::new(container.children[index], now, definition));
                        if !container.playback.continuous { *exhausted = true; }
                        else if container.playback.transition == TransitionMode::TriggerRate {
                            let milliseconds = transition_ms(&container.playback, 0.0, random15);
                            *next = Some(now + frames(milliseconds, definition.rate));
                        }
                    } else { *exhausted = true; }
                }
                active.retain_mut(|child| child.frame(now, audio, output));
                if active.is_empty() && next.is_none() && !*exhausted {
                    let delay = if container.playback.transition == TransitionMode::Delay {
                        frames(transition_ms(&container.playback, 0.0, random15), definition.rate)
                    } else { 0 };
                    *next = Some(now + 1 + delay);
                }
                !*exhausted || !active.is_empty()
            },
            _ => false,
        }
    }
}

fn frames(milliseconds: f32, rate: SampleRate) -> u64 {
    // 1410ddd30 clamps Delay (3) and TriggerRate (5) continuation actions
    // to ten engine quanta. Zero authored intervals remain valid.
    ((f64::from(milliseconds) * f64::from(rate.get()) * 0.001 + 0.5) as u64).max(10 * 1024)
}

pub(super) struct Decoder {
    audio: ScheduledAudio,
    root: Runtime,
    frame: u64,
    output: [f32; 8],
    channel: usize,
    alive: bool,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let channels = usize::from(self.audio.definition.channels.get());
        if self.channel == channels {
            if !self.alive { return None; }
            self.output.fill(0.0);
            for (target, milliseconds, curve) in self.audio.control.stops.lock().unwrap_or_else(|error| error.into_inner()).drain(..) {
                self.root.stop(target, milliseconds, curve, self.frame, &self.audio.definition);
            }
            self.alive = self.root.frame(self.frame, &self.audio, &mut self.output[..channels]);
            self.frame += 1;
            self.channel = 0;
        }
        let sample = self.output[self.channel];
        self.channel += 1;
        Some(sample)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> ChannelCount { self.audio.definition.channels }
    fn sample_rate(&self) -> SampleRate { self.audio.definition.rate }
    fn total_duration(&self) -> Option<Duration> { None }
}
impl Decodable for ScheduledAudio {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder { audio: self.clone(), root: Runtime::new(self.definition.root, 0, &self.definition), frame: 0,
            output: [0.0; 8], channel: usize::from(self.definition.channels.get()), alive: true }
    }
}
