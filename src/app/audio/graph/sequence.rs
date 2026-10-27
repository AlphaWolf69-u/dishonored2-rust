//! Playback-local container traversal shared by source continuations and the renderer.
use super::{ContainerPlayback, Kind, Loops, ScheduledAudio, Selection, SelectionMode};
use crate::assets::wwise::TransitionMode;

#[derive(Clone, Copy)]
pub(super) struct Next {
    pub(super) node: usize,
    pub(super) mode: TransitionMode,
    pub(super) milliseconds: f32,
}

pub(super) struct Sequence {
    pub(super) node: usize,
    selection: Selection,
    loops: Option<Loops>,
    pub(super) initialized: bool,
}
impl Sequence {
    pub(super) fn new(node: usize, playback: &ContainerPlayback) -> Self {
        // 14111a450 sends step playback directly to 14111b6e0. Only the
        // continuous path (14111b7b0) samples a playback-local loop limit.
        Self { node, selection: Selection::default(), loops: playback.continuous.then(|| Loops::new(playback, super::random15)), initialized: false }
    }
    pub(super) fn next(&mut self, audio: &ScheduledAudio) -> Option<usize> {
        let node = &audio.definition.nodes[self.node];
        let Kind::Container(container) = &node.kind else { return None; };
        let Self { selection, loops, initialized, .. } = self;
        let key = (node.id, if container.global { None } else { Some(audio.emitter) });
        let selected = if container.playback.continuous {
            if !*initialized && container.mode == SelectionMode::Sequence && !container.reset {
                if let Some(shared) = audio.selections.lock().unwrap_or_else(|error| error.into_inner()).get(&key) {
                    selection.sequence = shared.sequence.clone();
                }
            }
            let selected = selection.next(container.mode, &container.weights, container.avoid,
                container.playback.restart_backward, loops.as_mut());
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
        selected.map(|index| container.children[index])
    }

    pub(super) fn mode(&self, audio: &ScheduledAudio) -> TransitionMode {
        match &audio.definition.nodes[self.node].kind {
            Kind::Container(container) => container.playback.transition,
            _ => TransitionMode::Disabled,
        }
    }

    /// 14112f0a0 samples transition time only after a global actor lookup
    /// succeeds. Missing actors pop this sequence entry without a Play call.
    pub(super) fn continuation(&mut self, audio: &ScheduledAudio) -> Option<Next> {
        self.timed_next(audio, false)
    }

    /// 1410ddd30 samples transition time in every mode, even when the
    /// failed-start action ultimately uses only its ten-quantum minimum.
    pub(super) fn failed_continuation(&mut self, audio: &ScheduledAudio) -> Option<Next> {
        self.timed_next(audio, true)
    }

    fn timed_next(&mut self, audio: &ScheduledAudio, all_modes: bool) -> Option<Next> {
        let node = self.next(audio)?;
        if matches!(audio.definition.nodes[node].kind, Kind::Missing) { return None; }
        let Kind::Container(container) = &audio.definition.nodes[self.node].kind else { return None; };
        let mode = container.playback.transition;
        let milliseconds = if all_modes || matches!(mode, TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower |
            TransitionMode::Delay | TransitionMode::TriggerRate) {
            let seconds = f32::from_bits(audio.control.transitions[self.node].load(std::sync::atomic::Ordering::Relaxed));
            super::transition_ms(&container.playback, seconds, super::random15)
        } else { 0.0 };
        Some(Next { node, mode, milliseconds })
    }

}
