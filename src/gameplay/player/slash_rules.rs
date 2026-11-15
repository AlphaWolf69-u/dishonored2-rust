//! Mutable rule-selector state attached to one real player state machine.
//! 140895820 refreshes condition results; it does not reset action cursors.
//! 1408920c0/390 implement the retained per-rule sequence position.
use crate::assets::slash_rules::{Boolean, Condition, Definition, Outcome};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Context {
    pub is_generic_fatality: bool,
    pub is_focus_blink_attack: bool,
    pub is_focused: bool,
    pub is_throw_hit_available: bool,
    pub is_blink_attack: bool,
    pub current_posture: usize,
    pub last_slash_type: i32,
    pub time_since_last_slash: f32,
}
impl Context {
    fn matches(&self, condition: &Condition) -> bool {
        match *condition {
            Condition::Boolean(variable, operator, expected) => operator.compare(match variable {
                Boolean::GenericFatality => self.is_generic_fatality,
                Boolean::FocusBlinkAttack => self.is_focus_blink_attack,
                Boolean::Focused => self.is_focused,
                Boolean::ThrowHitAvailable => self.is_throw_hit_available,
                Boolean::BlinkAttack => self.is_blink_attack,
            }, expected),
            Condition::LastSlash(operator, expected) => operator.compare(self.last_slash_type, expected),
            Condition::Posture(operator, expected) => operator.compare(self.current_posture, expected),
            Condition::TimeSinceLastSlash(operator, expected) => operator.compare(self.time_since_last_slash, expected),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    pub definition: Arc<Definition>,
    /// One counter per retained declaration rule. Native single-action
    /// instances do not advance a sequence object at all.
    sequences: Vec<usize>,
    exhausted: Vec<bool>,
}
impl Runtime {
    pub fn new(definition: Arc<Definition>) -> Self {
        let count = definition.rules.len();
        Self { definition, sequences: vec![0; count], exhausted: vec![false; count] }
    }
    /// Refresh the condition epoch and select the first admitted outcome.
    /// Original sequence cursors advance on evaluation, before animation
    /// admission; a later scene failure does not roll the selector back.
    pub fn select(&mut self, context: &Context) -> Option<&Outcome> {
        for (index, rule) in self.definition.rules.iter().enumerate() {
            if self.exhausted[index] || !rule.conditions.iter().all(|condition| context.matches(condition)) { continue; }
            let sequence = self.sequences[index];
            let outcome = &rule.outcomes[sequence % rule.outcomes.len()];
            if rule.outcomes.len() > 1 {
                self.sequences[index] = sequence.wrapping_add(1);
                if rule.once && self.sequences[index] >= rule.outcomes.len() { self.exhausted[index] = true; }
            } else if rule.once { self.exhausted[index] = true; }
            return Some(outcome);
        }
        None
    }
}
