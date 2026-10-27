//! Source-owned container continuations. Source rendering remains in Runtime;
//! each voice and pending action retains its own reference to a shared stack.
use super::{Kind, Runtime, ScheduledAudio, RouteContext, sequence::{Next, Sequence}};
use crate::{assets::wwise::TransitionMode, gameplay::sound::containers::{continuation::{self, Entry, Registry, Stack}, timing}};
use super::super::fade::{Envelope, Transition};

type Continuation = continuation::Source<Sequence, Next>;

#[derive(Clone, Copy)]
pub(super) struct Fade { pub(super) start: u64, pub(super) milliseconds: i32, pub(super) curve: u8 }
struct Voice {
    id: u64,
    runtime: Runtime,
    continuation: Continuation,
    start: u64,
    created: u64,
    duration_sent: bool,
    stop_requested: bool,
    actual_stop: bool,
    ended: Option<u64>,
    duration_ms: Option<f32>,
}
struct Crossfade { voice: u64, milliseconds: i32, curve: u8 }
struct Pending { node: usize, stack: Option<Stack<Sequence>>, deadline: u64,
    fade: Option<Fade>, inherited: Option<Transition>, crossfade: Option<Crossfade> }

#[derive(Default)]
pub(super) struct Flow {
    groups: Registry<Sequence>,
    voices: Vec<Voice>,
    pending: Vec<Pending>,
    next_voice: u64,
}
impl Flow {
    pub(super) fn play(&mut self, node: usize, start: u64, now: u64, audio: &ScheduledAudio,
        stack: Option<Stack<Sequence>>, fade: Option<Fade>, inherited: Option<Transition>) -> bool {
        match &audio.definition.nodes[node].kind {
            Kind::Missing | Kind::NoSource => false,
            Kind::Sound(_) => {
                let mut runtime = Runtime::new(node, start, &audio.definition, &audio.control.registrations);
                runtime.fade = inherited;
                if let Some(fade) = fade {
                    runtime.fade = Some(Transition::new(Envelope::fade_in(fade.start,
                        super::super::fade::frames(fade.milliseconds, audio.definition.rate.get()), fade.curve)));
                }
                let id = self.next_voice;
                self.next_voice = self.next_voice.wrapping_add(1);
                self.voices.push(Voice { id, runtime, continuation: Continuation::new(stack), start,
                    created: now / 1024 * 1024, duration_sent: false, stop_requested: false,
                    actual_stop: false, ended: None, duration_ms: None });
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
                    Some(child) => self.play(child, start, now, audio, stack, fade, inherited),
                    None => {
                        if container.playback.continuous {
                            if let Some(stack) = &stack { drop(stack.pop()); }
                        }
                        self.failed(stack, now, audio, inherited, fade)
                    },
                }
            },
            Kind::Layer(children) => {
                if children.len() == 1 { return self.play(children[0], start, now, audio, stack, fade, inherited); }
                if let Some(outer) = stack {
                    let session = self.groups.begin(audio.definition.nodes[node].id, outer, true);
                    let mut accepted = false;
                    for &child in children {
                        let branch = Stack::default();
                        branch.push(Entry::Group(session.branch()));
                        accepted |= self.play(child, start, now, audio, Some(branch), fade, inherited.clone());
                    }
                    if accepted { true } else { self.failed(session.finish(), now, audio, inherited, fade) }
                } else {
                    let mut accepted = false;
                    for &child in children { accepted |= self.play(child, start, now, audio, None, fade, inherited.clone()); }
                    accepted
                }
            },
        }
    }

    fn failed(&mut self, stack: Option<Stack<Sequence>>, now: u64, audio: &ScheduledAudio,
        inherited: Option<Transition>, fade: Option<Fade>) -> bool {
        let mut source = Continuation::new(stack);
        source.prepare(false, |sequence| sequence.mode(audio), |sequence| sequence.failed_continuation(audio));
        let Some(next) = source.next().copied() else { return false; };
        let deadline = timing::after_failed_start(now, next.mode, next.milliseconds, audio.definition.rate.get());
        self.pending.push(Pending { node: next.node, stack: source.stack(), deadline, fade, inherited, crossfade: None });
        source.clear();
        true
    }

    pub(super) fn begin(&mut self, now: u64, audio: &ScheduledAudio) {
        for mut pending in std::mem::take(&mut self.pending) {
            // 1410ea040 cancels a continuation when its retained stop-gain
            // transition completes; a completed fade-in only unlinks it.
            if let Some(gain) = &pending.inherited {
                if gain.value(now).is_none() { continue; }
                if !gain.active(now) { pending.inherited = None; }
            }
            if pending.deadline.saturating_sub(now) < 1024 {
                if let Some(fade) = &mut pending.fade { fade.start = now; }
                if let Some(crossfade) = pending.crossfade {
                    // 14111c850 falls back to its zero-ms descriptor if the
                    // previous PBI has ended (14111ce70 clears that pointer).
                    if let Some(voice) = self.voices.iter_mut().find(|voice| voice.id == crossfade.voice && voice.ended.is_none()) {
                        let id = audio.definition.nodes[voice.runtime.node].id;
                        voice.runtime.stop(id, crossfade.milliseconds, crossfade.curve, now, &audio.definition);
                        voice.stop_requested = true;
                        pending.fade = (crossfade.milliseconds != 0).then_some(Fade { start: now,
                            milliseconds: crossfade.milliseconds, curve: crossfade.curve });
                    }
                }
                self.play(pending.node, pending.deadline, now, audio, pending.stack, pending.fade, pending.inherited);
            } else { self.pending.push(pending); }
        }
        for voice in &mut self.voices {
            if voice.duration_ms.is_none() { voice.duration_ms = voice.runtime.duration_ms(audio); }
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
            if voice.runtime.fade_value(now).is_none() {
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
                TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower => {
                    timing::crossfade(next.mode, voice.duration_ms.unwrap_or(0.0), next.milliseconds, audio.definition.rate.get())
                        .map(|plan| (tick + u64::from(plan.delay_samples), Some(Crossfade { voice: voice.id,
                            milliseconds: plan.milliseconds, curve: plan.curve })))
                },
                _ => None,
            };
            if let Some((deadline, crossfade)) = plan {
                let inherited = if crossfade.is_none() { voice.runtime.fade.as_ref().filter(|fade| fade.active(now)).cloned() } else { None };
                self.pending.push(Pending { node: next.node, stack: voice.continuation.stack(), deadline,
                    fade: None, inherited, crossfade });
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
                deadline: timing::after_source_end(ended, next.mode, delay), fade: None,
                inherited: voice.runtime.fade.as_ref().filter(|fade| fade.active(now)).cloned(), crossfade: None });
            voice.continuation.clear();
        }
        self.voices.retain(|voice| voice.ended.is_none());
    }
}
