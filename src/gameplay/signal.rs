//! Linked observer ownership shared by physics-world and mutable-shape signals.
//! Subscribers and callback keys are stable owned handles, not process pointers.
#[derive(Clone, Copy, Debug, Default)]
struct Link {
    target: Option<usize>,
    flags: u8,
}
#[derive(Clone, Copy, Debug)]
struct Slot<S, C> {
    next: Link,
    subscriber: S,
    callback: C,
}
pub struct Signal<S, C> {
    head: Link,
    slots: Vec<Option<Slot<S, C>>>,
    free: Vec<usize>,
}
impl<S, C> Default for Signal<S, C> {
    fn default() -> Self {
        Self {
            head: Link::default(),
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}
impl<S: Copy + Eq, C: Copy + Eq> Signal<S, C> {
    /// ae9d00 destroys the chain immediately outside emission. While emitting,
    /// mark every slot and let the traversal release it after callbacks return.
    pub fn destroy(&mut self) {
        let mut current = self.head.target;
        while let Some(index) = current {
            let slot = self.slots[index].as_mut().expect("missing signal slot");
            current = slot.next.target;
            if self.head.flags != 0 {
                slot.next.flags = (slot.next.flags & !2) | 1;
            } else {
                self.release_slot(index);
            }
        }
        if self.head.flags == 0 {
            self.head = Link::default();
        }
    }

    /// ce0550/ce05f0 prepend a new member slot while retaining signal flags.
    /// Duplicate subscriber/callback pairs remain distinct registrations.
    pub fn subscribe(&mut self, subscriber: S, callback: C) {
        let slot = Slot {
            next: Link {
                target: self.head.target,
                flags: 0,
            },
            subscriber,
            callback,
        };
        let index = if let Some(index) = self.free.pop() {
            self.slots[index] = Some(slot);
            index
        } else {
            self.slots.push(Some(slot));
            self.slots.len() - 1
        };
        self.head.target = Some(index);
    }
    fn link_mut(&mut self, slot: Option<usize>) -> &mut Link {
        match slot {
            None => &mut self.head,
            Some(index) => {
                &mut self.slots[index]
                    .as_mut()
                    .expect("signal link points to a destroyed slot")
                    .next
            }
        }
    }
    fn release_slot(&mut self, index: usize) {
        self.slots[index] = None;
        self.free.push(index);
    }

    /// fd1e00 -> ae99f0 removes the first matching member slot. During
    /// emission it marks that slot; traversal later owns physical removal.
    pub fn unsubscribe(&mut self, subscriber: S, callback: C) -> bool {
        let mut previous = None;
        let mut current = self.head.target;
        while let Some(index) = current {
            let slot = self.slots[index].as_ref().unwrap();
            let next = slot.next.target;
            if slot.subscriber == subscriber && slot.callback == callback {
                if self.head.flags != 0 {
                    let next = &mut self.slots[index].as_mut().unwrap().next;
                    next.flags = (next.flags & !2) | 1;
                } else {
                    self.link_mut(previous).target = next;
                    self.release_slot(index);
                }
                return true;
            }
            previous = Some(index);
            current = next;
        }
        false
    }

    /// c92be0/c96250 observer traversal: capture next before invoking, skip/remove
    /// marked slots, and retain mutations made by callbacks. A newly prepended
    /// observer is not part of the current traversal. Callback dispatch belongs
    /// to the world/resource owner; it receives this signal for in-flight edits.
    pub fn emit<A>(&mut self, args: &A, mut invoke: impl FnMut(&mut Self, S, C, &A)) {
        self.head.flags = (self.head.flags & !2) | 1;
        let mut previous = None;
        let mut current = self.head.target;
        while let Some(index) = current {
            let slot = *self.slots[index].as_ref().unwrap();
            let next = slot.next.target;
            if slot.next.flags == 0 {
                invoke(self, slot.subscriber, slot.callback, args);
                previous = Some(index);
            } else {
                self.link_mut(previous).target = next;
                self.release_slot(index);
            }
            current = next;
        }
        self.head.flags = 0;
    }
}

#[cfg(test)]
#[path = "signal/tests.rs"]
mod tests;
