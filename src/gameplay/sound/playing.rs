//! Accepted event registrations and their pending-action/voice ownership.
//! The playing ID exists before actions execute (1410c96c0/1410e0fe0).
use crate::gameplay::sound::Emitter;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, AtomicU32, Ordering}}};

/// One Wwise PostEvent ID namespace shared by ordinary and FX producers.
#[derive(Clone, Default)]
pub(crate) struct PlayingIds(Arc<AtomicU32>);
impl PlayingIds {
    pub(crate) fn allocate(&self) -> u32 {
        loop {
            let id = self.0.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            if id != 0 { return id; }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlayingId(u32);
impl PlayingId {
    pub(crate) fn value(self) -> u32 { self.0 }
}

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
    /// c6030: sixteen cookies per actual sound object. Keep admission until
    /// c47a0 consumes End, including the gap after the last voice drops.
    object_counts: HashMap<Emitter, u8>,
    completions: Arc<Mutex<Vec<Notification>>>,
    // Host observation copies only packets consumed by this canonical owner.
    observer: Option<Arc<Mutex<Vec<Notification>>>>,
}

impl PlayingEvents {
    /// The caller first resolves the event and accepts its command, as in
    /// 1410c96c0. This registers that command; it does not predict a future
    /// voice's success or allocate an ID for an unresolved request.
    pub fn register(&mut self, event: Event) -> Result<Ticket, String> {
        let id = self.next.checked_add(1).ok_or("audio playing IDs exhausted")?;
        self.register_with_id(id, event)
    }

    pub(crate) fn can_register_object(&self, object: Emitter) -> bool {
        self.object_counts.get(&object).copied().unwrap_or(0) < 16
    }

    /// c4550 stores the actual accepted PostEvent return ID in its cookie.
    pub(crate) fn register_with_id(&mut self, id: u32, event: Event) -> Result<Ticket, String> {
        if id == 0 { return Err("zero is not a playing ID".into()); }
        let id = PlayingId(id);
        if self.active.contains_key(&id) { return Err("audio playing ID is already retained".into()); }
        if !self.can_register_object(event.object) { return Err("sound object cookie pool is full".into()); }
        self.active.try_reserve(1).map_err(|error| format!("playing event allocation: {error}"))?;
        self.object_counts.try_reserve(1).map_err(|error| format!("sound object cookie allocation: {error}"))?;
        let registration = Arc::new(Registration {
            id, event, completions: self.completions.clone(), source_ready: AtomicBool::new(false),
        });
        self.active.insert(id, Arc::downgrade(&registration));
        *self.object_counts.entry(event.object).or_default() += 1;
        self.next = self.next.max(id.0);
        Ok(Ticket(registration))
    }

    pub fn is_empty(&self) -> bool { self.active.is_empty() }

    pub(crate) fn observe_notifications(&mut self, observer: Arc<Mutex<Vec<Notification>>>) {
        self.observer = Some(observer);
    }
    pub(crate) fn observation(&self) -> (Vec<PlayingId>, Vec<(Emitter, u8)>) {
        (self.active.keys().copied().collect(), self.object_counts.iter().map(|(&object, &count)| (object, count)).collect())
    }

    /// True while any pending action or playing voice retains this
    /// declaration reference. The registry itself owns only weak tickets.
    pub fn references_object(&self, object: Emitter) -> bool {
        self.object_counts.contains_key(&object)
    }

    pub fn references_declaration(&self, declaration: u32) -> bool {
        self.active.values().filter_map(Weak::upgrade).any(|entry| entry.event.declaration == declaration)
    }

    /// Drain before FX liveness checks, retaining IDs until their actual end
    /// notification is delivered. No elapsed-time estimate retires an event.
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        let completed = std::mem::take(&mut *self.completions.lock()
            .unwrap_or_else(|error| error.into_inner()));
        for notification in &completed {
            if matches!(notification.kind, NotificationKind::End) && self.active.remove(&notification.id).is_some() {
                if let Some(count) = self.object_counts.get_mut(&notification.event.object) {
                    *count -= 1;
                    if *count == 0 { self.object_counts.remove(&notification.event.object); }
                }
            }
        }
        if let Some(observer) = &self.observer {
            observer.lock().unwrap_or_else(|error| error.into_inner()).extend_from_slice(&completed);
        }
        completed
    }
}
