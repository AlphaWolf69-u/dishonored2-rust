//! Source-owned container continuations. Source rendering remains in Runtime;
//! each voice and pending action retains its own reference to a shared stack.
use super::{Kind, Runtime, ScheduledAudio, RouteContext, sequence::{Next, Sequence}};
use crate::{assets::wwise::TransitionMode, gameplay::sound::containers::{continuation::{self, Entry, Registry, Stack}, timing}};

type Continuation = continuation::Source<Sequence, Next>;

#[derive(Clone, Copy)]
pub(super) struct Fade { pub(super) start: u64, pub(super) milliseconds: i32, pub(super) curve: u8 }
struct Voice {
    runtime: Runtime,
    continuation: Continuation,
    start: u64,
    created: u64,
    duration_sent: bool,
    stop_requested: bool,
    actual_stop: bool,
    ended: Option<u64>,
}
struct Pending { node: usize, stack: Option<Stack<Sequence>>, deadline: u64, fade: Option<Fade> }

#[derive(Default)]
pub(super) struct Flow {
    groups: Registry<Sequence>,
    voices: Vec<Voice>,
    pending: Vec<Pending>,
}
impl Flow {
    pub(super) fn play(&mut self, node: usize, start: u64, now: u64, audio: &ScheduledAudio,
        stack: Option<Stack<Sequence>>, fade: Option<Fade>) -> bool {
        match &audio.definition.nodes[node].kind {
            Kind::Missing | Kind::NoSource => false,
            Kind::Sound(_) => {
                let mut runtime = Runtime::new(node, start, &audio.definition, &audio.control.registrations);
                if let Some(fade) = fade {
                    runtime.fade = Some(super::super::fade::Envelope::fade_in(fade.start,
                        super::super::fade::frames(fade.milliseconds, audio.definition.rate.get()), fade.curve));
                }
                self.voices.push(Voice { runtime, continuation: Continuation::new(stack), start,
                    created: now / 1024 * 1024, duration_sent: false, stop_requested: false, actual_stop: false, ended: None });
                true
            },
            Kind::Container(container) => {
                let mut sequence = Sequence::new(node, &container.playback);
                let child = sequence.next(audio);
                let stack = if container.playback.continuous {
                    let stack = stack.unwrap_or_default();
                    stack.push(Entry::Sequence(sequence));
                    Some(stack)
                } else { stack };
                match child.filter(|&child| !matches!(audio.definition.nodes[child].kind, Kind::Missing)) {
                    Some(child) => self.play(child, start, now, audio, stack, fade),
                    None => {
                        if container.playback.continuous {
                            if let Some(stack) = &stack { drop(stack.pop()); }
                        }
                        self.failed(stack, now, audio)
                    },
                }
            },
            Kind::Layer(children) => {
                if children.len() == 1 { return self.play(children[0], start, now, audio, stack, fade); }
                if let Some(outer) = stack {
                    let session = self.groups.begin(audio.definition.nodes[node].id, outer, true);
                    let mut accepted = false;
                    for &child in children {
                        let branch = Stack::default();
                        branch.push(Entry::Group(session.branch()));
                        accepted |= self.play(child, start, now, audio, Some(branch), fade);
                    }
                    if accepted { true } else { self.failed(session.finish(), now, audio) }
                } else {
                    let mut accepted = false;
                    for &child in children { accepted |= self.play(child, start, now, audio, None, fade); }
                    accepted
                }
            },
        }
    }

    fn failed(&mut self, stack: Option<Stack<Sequence>>, now: u64, audio: &ScheduledAudio) -> bool {
        let mut source = Continuation::new(stack);
        source.prepare(false, |sequence| sequence.mode(audio), |sequence| sequence.continuation(audio));
        let Some(next) = source.next().copied() else { return false; };
        let deadline = timing::after_failed_start(now, next.mode, next.milliseconds, audio.definition.rate.get());
        self.pending.push(Pending { node: next.node, stack: source.stack(), deadline, fade: None });
        source.clear();
        true
    }

    pub(super) fn begin(&mut self, now: u64, audio: &ScheduledAudio) {
        for pending in std::mem::take(&mut self.pending) {
            if pending.deadline.saturating_sub(now) < 1024 {
                self.play(pending.node, pending.deadline, now, audio, pending.stack, pending.fade);
            } else { self.pending.push(pending); }
        }
    }

    pub(super) fn stop(&mut self, target: u32, milliseconds: i32, curve: u8, now: u64, audio: &ScheduledAudio) {
        self.pending.retain(|pending| !audio.definition.matches_actor(pending.node, target));
        for voice in &mut self.voices {
            if !audio.definition.matches_actor(voice.runtime.node, target) { continue; }
            let id = audio.definition.nodes[voice.runtime.node].id;
            voice.runtime.stop(id, milliseconds, curve, now, &audio.definition);
            voice.stop_requested = true;
            if milliseconds == 0 { voice.actual_stop = true; voice.continuation.clear(); voice.ended = Some(now); }
        }
    }

    pub(super) fn frame(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32],
        mut routing: Option<&mut RouteContext<'_>>) -> bool {
        for voice in &mut self.voices {
            if voice.ended.is_some() || now < voice.start { continue; }
            if voice.stop_requested && voice.runtime.fade.is_some_and(|fade| fade.value(now).is_none()) {
                voice.actual_stop = true;
                voice.continuation.clear();
                voice.ended = Some(now);
                continue;
            }
            if !voice.runtime.frame(now, audio, output, routing.as_deref_mut()) { voice.ended = Some(now); }
        }
        if now % 1024 == 1023 { self.notifications(now, audio); }
        !self.voices.is_empty() || !self.pending.is_empty()
    }

    /// Duration notifications precede source-end notifications in the first
    /// rendered quantum (141121990 / 1411287f0 / 1410e0080).
    fn notifications(&mut self, now: u64, audio: &ScheduledAudio) {
        let tick = now / 1024 * 1024;
        for voice in &mut self.voices {
            if voice.duration_sent || voice.created > tick { continue; }
            voice.duration_sent = true;
            if voice.actual_stop || voice.stop_requested { continue; }
            voice.continuation.prepare(true, |sequence| sequence.mode(audio), |sequence| sequence.continuation(audio));
            let Some(next) = voice.continuation.next().copied() else { continue; };
            let plan = match next.mode {
                TransitionMode::TriggerRate => {
                    let interval = (timing::samples_from_ms(f64::from(next.milliseconds), audio.definition.rate.get()) as i32).max(1024) as u32;
                    Some((tick + u64::from(interval) + voice.start % 1024, None))
                },
                // Crossfades remain guarded at Definition::decode until
                // source-readiness callback timing is established in-game.
                _ => None,
            };
            if let Some((deadline, fade)) = plan {
                self.pending.push(Pending { node: next.node, stack: voice.continuation.stack(), deadline, fade });
                voice.continuation.clear();
            }
        }
        for voice in &mut self.voices {
            let Some(ended) = voice.ended else { continue; };
            if voice.actual_stop { continue; }
            voice.continuation.prepare(false, |sequence| sequence.mode(audio), |sequence| sequence.continuation(audio));
            let Some(next) = voice.continuation.next().copied() else { continue; };
            if voice.stop_requested && matches!(next.mode, TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower) {
                voice.continuation.clear();
                continue;
            }
            let delay = if next.mode == TransitionMode::Delay {
                timing::samples_from_ms(f64::from(next.milliseconds), audio.definition.rate.get())
            } else { 0 };
            self.pending.push(Pending { node: next.node, stack: voice.continuation.stack(),
                deadline: timing::after_source_end(ended, next.mode, delay), fade: None });
            voice.continuation.clear();
        }
        self.voices.retain(|voice| voice.ended.is_none());
    }
}
