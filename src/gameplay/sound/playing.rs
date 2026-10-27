//! Accepted event registrations and their pending-action/voice ownership.
//! The playing ID exists before actions execute (1410c96c0/1410e0fe0).
use crate::gameplay::sound::Emitter;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayingId(u32);

#[derive(Clone, Copy)]
pub struct Event {
    pub declaration: u32,
    pub object: Emitter,
    pub callbacks: u32,
}

#[derive(Clone, Copy)]
pub enum NotificationKind { SourceReady, EventPlaying, End }

#[derive(Clone, Copy)]
pub struct Notification {
    pub id: PlayingId,
    pub event: Event,
    pub kind: NotificationKind,
}

struct Registration {
    id: PlayingId,
    event: Event,
    completions: Arc<Mutex<Vec<Notification>>>,
    source_ready: AtomicBool,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // 1410e1290 reports end only after pending-action and voice ownership
        // are both gone. The registry keeps weak references so it cannot itself
        // prolong an event. The game-side callback consumes this queued packet.
        let mut notifications = self.completions.lock().unwrap_or_else(|error| error.into_inner());
        // c47a0 synthesizes EventPlaying before End when no Duration packet
        // arrived. This also covers stop-only and unsuccessful Play actions.
        if self.event.callbacks & 0x8000 != 0 && !self.source_ready.load(Ordering::Relaxed) {
            notifications.push(Notification { id: self.id, event: self.event, kind: NotificationKind::EventPlaying });
        }
        notifications.push(Notification { id: self.id, event: self.event, kind: NotificationKind::End });
    }
}

/// The accepted command owns the first ticket; every retained action/voice
/// owns a clone. Dropping one cannot finish the other owners' playback.
#[derive(Clone)]
pub struct Ticket(Arc<Registration>);
impl Ticket {
    pub fn id(&self) -> PlayingId { self.0.id }
    pub fn event(&self) -> Event { self.0.event }
    /// Source readiness is delivered by the actual audio processing phase,
    /// including delayed actions. Posting a command alone cannot emit it.
    pub fn source_ready(&self) {
        if self.0.event.callbacks & 8 == 0 { return; }
        self.0.source_ready.store(true, Ordering::Relaxed);
        self.0.completions.lock().unwrap_or_else(|error| error.into_inner())
            .push(Notification { id: self.id(), event: self.event(), kind: NotificationKind::SourceReady });
    }
}

#[derive(Default)]
pub struct PlayingEvents {
    next: u32,
    active: HashMap<PlayingId, Weak<Registration>>,
    completions: Arc<Mutex<Vec<Notification>>>,
}

impl PlayingEvents {
    /// The caller first resolves the event and accepts its command, as in
    /// 1410c96c0. This registers that command; it does not predict a future
    /// voice's success or allocate an ID for an unresolved request.
    pub fn register(&mut self, event: Event) -> Result<Ticket, String> {
        let id = self.next.checked_add(1).ok_or("audio playing IDs exhausted")?;
        let id = PlayingId(id);
        let registration = Arc::new(Registration {
            id, event, completions: self.completions.clone(), source_ready: AtomicBool::new(false),
        });
        self.active.insert(id, Arc::downgrade(&registration));
        self.next = id.0;
        Ok(Ticket(registration))
    }

    pub fn is_empty(&self) -> bool { self.active.is_empty() }

    /// Drain before FX liveness checks, retaining IDs until their actual end
    /// notification is delivered. No elapsed-time estimate retires an event.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        let completed = std::mem::take(&mut *self.completions.lock()
            .unwrap_or_else(|error| error.into_inner()));
        for notification in &completed {
            if matches!(notification.kind, NotificationKind::End) { self.active.remove(&notification.id); }
        }
        completed
    }
}
