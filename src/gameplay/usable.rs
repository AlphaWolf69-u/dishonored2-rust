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
    pub stage_notification:Option<bool>,
}

pub struct Usable {
    pub definition: Arc<DefinitionData>,
    pub state: usize,
    transition_remaining: Option<f32>,
    pending_state: Option<usize>,
}

fn command(index: u8, mode: PlayMode) -> AnimBasicCommand {
    AnimBasicCommand { idx: index.into(), weight: 1.0, mode, rate: 1.0, start: 0.0, play_on_end: false }
}

impl Usable {
    pub fn new(definition: Arc<DefinitionData>) -> Result<Self, String> {
        if definition.initial_step >= definition.steps.len() {
            return Err(format!("usable initial state {} outside {} states", definition.initial_step, definition.steps.len()));
        }
        Ok(Self { state: definition.initial_step, definition, transition_remaining: None, pending_state:None })
    }

    /// 14077a880 initializes an idle pose and its active effect on spawn.
    pub fn initial(&self) -> Changes<'_> {
        let state = &self.definition.steps[self.state];
        if let Some(index) = state.animation {
            Changes { animation: Some(command(index, PlayMode::Loop)),
                stop_effect: true, start_effect: state.effect.as_deref(),stage_notification:None }
        } else if let Some(index) = state.transition_animation {
            // 14077b3b0 holds the first transition frame when no idle is
            // authored, and clears the current effect.
            Changes { animation: Some(command(index, PlayMode::StayOnFirstFrame)),
                stop_effect: true, start_effect: None,stage_notification:None }
        } else {
            Changes::default()
        }
    }

    ///77b3b0: a regular Start returns to the authored initial state;
    ///the component's retained bit30 resumes its previously selected state.
    pub fn start(&mut self, preserve_state: bool) -> Changes<'_> {
        if !preserve_state { self.state = self.definition.initial_step; }
        self.transition_remaining = None;
        self.pending_state = None;
        self.initial()
    }

    /// 14077aec0 wraps to the next state and ignores input during a transition.
    pub fn interact(&mut self) -> Changes<'_> {
        if self.transition_remaining.is_some() || self.definition.steps.is_empty() {
            return Changes::default();
        }
        self.set_state((self.state + 1) % self.definition.steps.len())
    }

    pub fn transitioning(&self) -> bool { self.transition_remaining.is_some() }

    /// 77b3b0 applies the requested authored state, also used by audiograph End.
    pub fn set_state(&mut self, next: usize) -> Changes<'_> {
        if next >= self.definition.steps.len() { return Changes::default(); }
        let previous = self.state;
        self.state = next;
        let from = &self.definition.steps[previous];
        let to = &self.definition.steps[self.state];
        let transitioning = from.transition_animation.is_some() || from.transition_min_duration > 0.0;
        self.transition_remaining = transitioning.then_some(from.transition_min_duration.max(0.0));
        Changes {
            animation: if transitioning { from.transition_animation.map(|index| command(index, PlayMode::AndStay)) }
                else { to.animation.map(|index| command(index, PlayMode::Loop)) },
            stop_effect: transitioning || !to.wait_transition_end_to_play_effect,
            start_effect: if to.wait_transition_end_to_play_effect { None } else { to.effect.as_deref() },
            stage_notification:Some(transitioning),
        }
    }

    ///77b920: Set Stage waits for an active transition; an instant change
    ///or a non-adjacent stage applies the actual destination idle/held pose.
    pub fn set_stage(&mut self,next:usize,instant:bool)->Changes<'_>{
        if next>=self.definition.steps.len(){return Changes::default();}
        if !instant {
            if next==self.state{return Changes::default();}
            if self.transitioning(){self.pending_state=Some(next);return Changes::default();}
            if (self.state+1)%self.definition.steps.len()==next{return self.set_state(next);}
        }
        self.state=next;self.transition_remaining=None;
        let mut change=self.initial();
        if change.animation.is_some(){change.stage_notification=Some(false);}
        change
    }

    /// 14077be20 waits for both the transition clip and minimum duration.
    pub fn tick(&mut self, dt: f32, animation_playing: bool) -> Changes<'_> {
        if dt<=0.{return Changes::default();}
        if self.transition_remaining.is_none(){
            if let Some(next)=self.pending_state.take(){return self.set_state(next);}
            return Changes::default();
        }
        let remaining=self.transition_remaining.as_mut().expect("transition was checked");
        *remaining -= dt.max(0.0);
        if *remaining > 0.0 || animation_playing { return Changes::default(); }
        self.transition_remaining = None;
        let state = &self.definition.steps[self.state];
        Changes {
            animation: state.animation.map(|index| command(index, PlayMode::Loop)),
            stop_effect: false,
            start_effect: state.wait_transition_end_to_play_effect.then(|| state.effect.as_deref()).flatten(),
            stage_notification:Some(false),
        }
    }
}
