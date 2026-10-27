//! Accepted event registrations and their pending-action/voice ownership.
//! The playing ID exists before actions execute (1410c96c0/1410e0fe0).
use crate::gameplay::sound::Emitter;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct PlayingId(u32);

#[derive(Clone, Copy)]
pub(super) struct Event {
    pub declaration: u32,
    pub object: Emitter,
}

#[derive(Clone, Copy)]
pub(super) struct Completion {
    pub id: PlayingId,
    pub event: Event,
}

struct Registration {
    id: PlayingId,
    event: Event,
    completions: Arc<Mutex<Vec<Completion>>>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // 1410e1290 reports end only after pending-action and voice ownership
        // are both gone. The registry keeps weak references so it cannot itself
        // prolong an event. The game-side callback consumes this queued packet.
        self.completions.lock().unwrap_or_else(|error| error.into_inner())
            .push(Completion { id: self.id, event: self.event });
    }
}

/// The accepted command owns the first ticket; every retained action/voice
/// owns a clone. Dropping one cannot finish the other owners' playback.
#[derive(Clone)]
pub(super) struct Ticket(Arc<Registration>);
impl Ticket {
    pub fn id(&self) -> PlayingId { self.0.id }
    pub fn event(&self) -> Event { self.0.event }
}

#[derive(Default)]
pub(super) struct PlayingEvents {
    next: u32,
    active: HashMap<PlayingId, Weak<Registration>>,
    completions: Arc<Mutex<Vec<Completion>>>,
}

impl PlayingEvents {
    /// The caller first resolves the event and accepts its command, as in
    /// 1410c96c0. This registers that command; it does not predict a future
    /// voice's success or allocate an ID for an unresolved request.
    pub fn register(&mut self, event: Event) -> Result<Ticket, String> {
        let id = self.next.checked_add(1).ok_or("audio playing IDs exhausted")?;
        let id = PlayingId(id);
        let registration = Arc::new(Registration {
            id, event, completions: self.completions.clone(),
        });
        self.active.insert(id, Arc::downgrade(&registration));
        self.next = id.0;
        Ok(Ticket(registration))
    }

    pub fn is_empty(&self) -> bool { self.active.is_empty() }

    /// Drain before FX liveness checks, retaining IDs until their actual end
    /// notification is delivered. No elapsed-time estimate retires an event.
    pub fn take_completed(&mut self) -> Vec<Completion> {
        let completed = std::mem::take(&mut *self.completions.lock()
            .unwrap_or_else(|error| error.into_inner()));
        for completion in &completed { self.active.remove(&completion.id); }
        completed
    }
}
