//! Music's inline request manager: 140662b10, 140662fe0, 140663fe0,
//! 1406645e0. The original template has ten slots and events 15/16.
//!
//! Request handles are integers. Ghidra incorrectly renders the current-handle
//! comparison as floating point; 140664695 is an integer CMP/SETNZ.

const CAPACITY: usize = 10;
pub(super) const INVALID_REQUEST: i32 = -1;

#[derive(Clone, Copy)]
struct Entry {
    handle: i32,
    stage: i32,
    remaining: f32,
    lifetime: i32,
    flags: u8,
}

impl Entry {
    const EMPTY: Self = Self {
        handle: INVALID_REQUEST, stage: 0, remaining: 0.0, lifetime: 0, flags: 0,
    };
}

#[derive(Clone, Copy, Debug)]
pub enum Notification {
    /// Event 16 payload: request handle followed by operation 0, 1 or 2.
    Current(i32),
    Superseded(i32),
    Removed(i32),
}

/// This is mutable state belonging to one music owner. Definitions and sound
/// resources are retained by that owner, never copied into individual slots.
pub(super) struct Requests {
    entries: [Entry; CAPACITY],
    len: usize,
    expiring: usize,
    next_handle: i32,
    current: i32,
}

impl Default for Requests {
    fn default() -> Self {
        Self { entries: [Entry::EMPTY; CAPACITY], len: 0, expiring: 0,
            next_handle: INVALID_REQUEST, current: INVALID_REQUEST }
    }
}

impl Requests {
    /// 140662fe0 orders descending by signed music stage, inserting before
    /// existing equal stages. Full capacity returns -1 without consuming an ID.
    /// Lifetime 0 persists, 2 survives one update; other nonzero values count
    /// down the supplied duration, preserving the original integer dispatch.
    pub(super) fn insert(&mut self, stage: i32, duration: f32, lifetime: i32,
        notify: bool, additional_flag: bool) -> i32 {
        if self.len == CAPACITY { return INVALID_REQUEST; }
        self.next_handle = self.next_handle.wrapping_add(1);
        let at = self.entries[..self.len].iter().position(|entry| entry.stage <= stage)
            .unwrap_or(self.len);
        self.entries.copy_within(at..self.len, at + 1);
        self.entries[at] = Entry { handle: self.next_handle, stage, remaining: duration,
            lifetime, flags: u8::from(notify) | (u8::from(additional_flag) << 1) };
        self.len += 1;
        self.expiring += usize::from(lifetime != 0);
        self.next_handle
    }

    pub(super) fn stage(&self) -> Option<i32> {
        (self.len != 0).then(|| self.entries[0].stage)
    }

    pub(super) fn remove(&mut self, handle: i32, notify: &mut impl FnMut(Notification)) {
        if let Some(at) = self.entries[..self.len].iter().position(|entry| entry.handle == handle) {
            self.remove_at(at, notify);
        }
    }

    fn remove_at(&mut self, at: usize, notify: &mut impl FnMut(Notification)) {
        let entry = self.entries[at];
        self.expiring -= usize::from(entry.lifetime != 0);
        if entry.flags & 1 != 0 { notify(Notification::Removed(entry.handle)); }
        self.entries.copy_within(at + 1..self.len, at);
        self.len -= 1;
    }

    /// 1406645e0 expires backwards and tests a negative duration *before*
    /// decrementing. Zero therefore remains through the current update.
    /// The host submits notifications only while the original owner exists
    /// (1406dc560), preserving their order on the shared entity event queue.
    pub(super) fn update(&mut self, dt: f32, notify: &mut impl FnMut(Notification)) -> bool {
        if self.expiring != 0 {
            for at in (0..self.len).rev() {
                let entry = &mut self.entries[at];
                if entry.lifetime == 0 { continue; }
                // COMISS zero, remaining / JBE treats unordered as the
                // non-expired branch too; don't replace this with >= 0.
                if entry.remaining < 0.0 {
                    self.remove_at(at, notify);
                } else if entry.lifetime == 2 {
                    entry.remaining = -1.0;
                } else {
                    entry.remaining -= dt;
                }
            }
        }
        let next = if self.len == 0 { INVALID_REQUEST } else { self.entries[0].handle };
        let changed = self.current != next;
        if changed {
            if self.current >= 0 {
                if let Some(previous) = self.entries[..self.len].iter()
                    .find(|entry| entry.handle == self.current && entry.flags & 1 != 0) {
                    notify(Notification::Superseded(previous.handle));
                }
            }
            if next >= 0 && self.entries[0].flags & 1 != 0 {
                notify(Notification::Current(next));
            }
        }
        self.current = next;
        changed
    }
}
