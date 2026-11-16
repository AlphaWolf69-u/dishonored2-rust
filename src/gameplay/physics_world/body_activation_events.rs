//! Per-body state and activation signals owned by the game-world wrapper.
//! 1400941e0/140094130/140094280/140094460/1400943d0.
use std::collections::HashMap;
use crate::gameplay::{component_state::entity::ComponentHandle, signal::Signal};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActivationSubscriber {
    pub handle: ComponentHandle,
    pub generation: u32,
    ///0: component8097f0;1: player-instigator world notifier852730.
    pub callback: u8,
}

struct Transition { body: u32, active: bool, prepared: bool }

#[derive(Default)]
pub(crate) struct BodyActivationEvents {
    // Native states: active0, inactive1, removed2, destroyed3. Compact host
    // storage keeps signal allocations only for bodies with actual listeners.
    states: Vec<u8>,
    listeners: HashMap<u32, Signal<ActivationSubscriber, u8>>,
    // Host observer errors can suspend traversal. Retain its exact identity
    // and preparation boundary; no native state is published before callbacks.
    pending: Option<Transition>,
}

impl BodyActivationEvents {
    fn slot(&mut self, body: u32) -> Result<usize, String> {
        let slot = (body & 0x00ff_ffff) as usize;
        if slot == 0x00ff_ffff { return Err("activation event body is invalid".into()); }
        if self.states.len() <= slot { self.states.resize(slot + 1, 3); }
        Ok(slot)
    }

    pub(crate) fn subscribe(&mut self, body: u32, subscriber: ActivationSubscriber) -> Result<(), String> {
        let slot = self.slot(body)?;
        if self.states[slot] > 1 { return Err("activation listener body has not been published to the wrapper".into()); }
        self.listeners.entry(body & 0x00ff_ffff).or_default().subscribe(subscriber, 0);
        Ok(())
    }

    ///852730 subscribes to the lower body's real signal even before it is
    ///published. The caller validates the actual body ID and component lease.
    pub(crate) fn subscribe_instigator(&mut self, body: u32, subscriber: ActivationSubscriber) -> Result<(), String> {
        self.slot(body)?;
        if subscriber.callback != 1 { return Err("instigator body callback differs".into()); }
        self.listeners.entry(body & 0x00ff_ffff).or_default().subscribe(subscriber, 0);
        Ok(())
    }
    pub(crate) fn unsubscribe(&mut self, body: u32, subscriber: ActivationSubscriber) -> Result<(), String> {
        self.slot(body)?;
        if let Some(signal) = self.listeners.get_mut(&(body & 0x00ff_ffff)) {
            signal.unsubscribe(subscriber, 0);
            if signal.is_empty() { self.listeners.remove(&(body & 0x00ff_ffff)); }
        }
        Ok(())
    }

    /// 1400941e0 reads the body's actual active bit after publication.
    pub(crate) fn added(&mut self, body: u32, flags: u32) -> Result<(), String> {
        let slot = self.slot(body)?;
        self.states[slot] = u8::from(flags & 8 == 0);
        Ok(())
    }

    /// 140094460 marks removal without firing an activation transition.
    pub(crate) fn removed(&mut self, body: u32) -> Result<(), String> {
        let slot = self.slot(body)?;
        self.states[slot] = 2;
        Ok(())
    }

    /// 1400943d0 destroys the signal before the body slot can be reused.
    pub(crate) fn destroyed(&mut self, body: u32) -> Result<(), String> {
        let slot = self.slot(body)?;
        if let Some(mut signal) = self.listeners.remove(&(body & 0x00ff_ffff)) { signal.destroy(); }
        if self.pending.as_ref().is_some_and(|pending| pending.body == body) { self.pending = None; }
        self.states[slot] = 3;
        Ok(())
    }

    /// Source transition admission and deactivation preparation. Keeping this
    /// separate from emit permits both phases to borrow the same live rigid
    /// pool without two simultaneous mutable closure captures.
    pub(crate) fn begin_transition(&mut self, body: u32, active: bool,
        mut before_deactivation: impl FnMut(u32) -> Result<(), String>,
    ) -> Result<bool, String> {
        let slot = self.slot(body)?;
        let expected = u8::from(active);
        if let Some(pending) = &self.pending {
            if pending.body != body {
                // The outer world signal resumes its original batch. Earlier
                // bodies already published their new state and must be skipped
                // before reaching this body's retained listener cursor.
                if self.states[slot] != expected { return Ok(false); }
                return Err("another body activation signal is incomplete".into());
            }
            if pending.active != active { return Err("body activation direction differs from its suspended signal".into()); }
            if self.states[slot] != expected { return Err("body activation state changed during a suspended signal".into()); }
        } else {
            if self.states[slot] != expected { return Ok(false); }
            self.pending = Some(Transition { body, active, prepared: false });
        }
        if !self.pending.as_ref().expect("transition retained").prepared {
            if !active { before_deactivation(body)?; }
            self.pending.as_mut().expect("transition retained").prepared = true;
        }
        Ok(true)
    }

    /// 140094130/140094280 publish the new state only after listeners return.
    /// Signal's cursor skips earlier successful listeners after a host error.
    pub(crate) fn emit_transition(&mut self, body: u32, active: bool,
        mut dispatch: impl FnMut(ActivationSubscriber, u32, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let pending = self.pending.as_ref().ok_or("activation signal was not prepared")?;
        if pending.body != body || pending.active != active || !pending.prepared {
            return Err("activation signal differs from its prepared transition".into());
        }
        let slot = self.slot(body)?;
        if self.states[slot] != u8::from(active) { return Err("prepared activation state changed".into()); }
        if let Some(signal) = self.listeners.get_mut(&(body & 0x00ff_ffff)) {
            signal.emit_resumable(&(body, active), |_, subscriber, _, &(body, active)| dispatch(subscriber, body, active))?;
        }
        self.states[slot] = u8::from(!active);
        self.pending = None;
        Ok(())
    }
}
