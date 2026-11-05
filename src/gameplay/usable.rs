//! Runtime state for `arkCpntUsableWithStateList`.
use std::sync::Arc;
use crate::assets::entities::usable::DefinitionData;
use crate::gameplay::player::animation::{AnimBasicCommand, PlayMode};

/// State changes are returned to the animation and FX owners. They may keep
/// their own lightweight handles; the immutable declaration is shared.
#[derive(Default)]
pub struct Changes<'a> {
    pub animation: Option<AnimBasicCommand>,
    pub stop_effect: bool,
    pub start_effect: Option<&'a str>,
}

pub struct Usable {
    pub definition: Arc<DefinitionData>,
    pub state: usize,
    transition_remaining: Option<f32>,
}

fn command(index: u8, mode: PlayMode) -> AnimBasicCommand {
    AnimBasicCommand { idx: index.into(), weight: 1.0, mode, rate: 1.0, start: 0.0, play_on_end: false }
}

impl Usable {
    pub fn new(definition: Arc<DefinitionData>) -> Result<Self, String> {
        if definition.initial_step >= definition.steps.len() {
            return Err(format!("usable initial state {} outside {} states", definition.initial_step, definition.steps.len()));
        }
        Ok(Self { state: definition.initial_step, definition, transition_remaining: None })
    }

    /// 14077a880 initializes an idle pose and its active effect on spawn.
    pub fn initial(&self) -> Changes<'_> {
        let state = &self.definition.steps[self.state];
        Changes { animation: state.animation.map(|index| command(index, PlayMode::Loop)),
            stop_effect: false, start_effect: state.effect.as_deref() }
    }

    /// 14077aec0 wraps to the next state and ignores input during a transition.
    pub fn interact(&mut self) -> Changes<'_> {
        if self.transition_remaining.is_some() || self.definition.steps.is_empty() {
            return Changes::default();
        }
        let previous = self.state;
        self.state = (self.state + 1) % self.definition.steps.len();
        let from = &self.definition.steps[previous];
        let to = &self.definition.steps[self.state];
        let transitioning = from.transition_animation.is_some() || from.transition_min_duration > 0.0;
        self.transition_remaining = transitioning.then_some(from.transition_min_duration.max(0.0));
        Changes {
            animation: if transitioning { from.transition_animation.map(|index| command(index, PlayMode::AndStay)) }
                else { to.animation.map(|index| command(index, PlayMode::Loop)) },
            stop_effect: transitioning || !to.wait_transition_end_to_play_effect,
            start_effect: if to.wait_transition_end_to_play_effect { None } else { to.effect.as_deref() },
        }
    }

    /// 14077be20 waits for both the transition clip and minimum duration.
    pub fn tick(&mut self, dt: f32, animation_playing: bool) -> Changes<'_> {
        let Some(remaining) = self.transition_remaining.as_mut() else { return Changes::default() };
        *remaining -= dt.max(0.0);
        if *remaining > 0.0 || animation_playing { return Changes::default(); }
        self.transition_remaining = None;
        let state = &self.definition.steps[self.state];
        Changes {
            animation: state.animation.map(|index| command(index, PlayMode::Loop)),
            stop_effect: false,
            start_effect: state.wait_transition_end_to_play_effect.then(|| state.effect.as_deref()).flatten(),
        }
    }
}
