//! Mutable-shape observer ownership (cdfc70/cdfe20 and ce03a0..ce0530).
//! The world/resource owner routes callbacks by stable owner and entry handles.
use super::signal::Signal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ShapeObserver {
    pub owner: usize,
    pub entry: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShapeCallback {
    Mutated,
    Destroyed,
}
#[derive(Default)]
pub(crate) struct ShapeSignals {
    pub mutated: Signal<ShapeObserver, ShapeCallback>,
    pub destroyed: Signal<ShapeObserver, ShapeCallback>,
}
pub(crate) trait ShapeSignalStore {
    /// Resolve the mutable shape's actual signal pair. An immutable or missing
    /// shape is an ownership error here; c92be0 checks virtual18 before calling.
    fn signals_mut(&mut self, shape: usize) -> Result<&mut ShapeSignals, String>;
}
#[derive(Default)]
struct Registration {
    shape: Option<usize>,
    bodies: Vec<u32>,
    status: u8,
}
/// The query-world owner must release these observers before releasing its
/// registrations or the corresponding shape-signal resources (cdfcb0).
pub(crate) struct ShapeRegistry {
    owner: usize,
    registrations: Vec<Registration>,
    active: Vec<usize>,
    recycled: Vec<usize>,
    pending: bool,
}
impl ShapeRegistry {
    /// cdfc70. The owner key replaces this manager's process-pointer identity.
    pub fn new(owner: usize) -> Self {
        Self {
            owner,
            registrations: Vec::new(),
            active: Vec::new(),
            recycled: Vec::new(),
            pending: false,
        }
    }
    pub fn has_pending_changes(&self) -> bool {
        self.pending
    }
    fn entry_mut(&mut self, observer: ShapeObserver) -> Result<&mut Registration, String> {
        if observer.owner != self.owner {
            return Err("shape observer belongs to another world".into());
        }
        self.registrations
            .get_mut(observer.entry)
            .ok_or("shape observer entry is missing".into())
    }
    /// cdfe20 groups by root shape, reuses the last pooled registration, and
    /// appends every body ID. ce0430 resets status and subscribes both callbacks
    /// only when binding a newly active registration.
    pub fn register_body(
        &mut self,
        shape: usize,
        body: u32,
        signals: &mut impl ShapeSignalStore,
    ) -> Result<(), String> {
        let entry = if let Some(&entry) = self
            .active
            .iter()
            .find(|&&i| self.registrations[i].shape == Some(shape))
        {
            entry
        } else {
            let entry = if let Some(entry) = self.recycled.pop() {
                entry
            } else {
                self.registrations.push(Registration::default());
                self.registrations.len() - 1
            };
            self.active.push(entry);
            self.registrations[entry].shape = Some(shape);
            self.registrations[entry].status = 0;
            let observer = ShapeObserver {
                owner: self.owner,
                entry,
            };
            let pair = signals.signals_mut(shape)?;
            pair.mutated.subscribe(observer, ShapeCallback::Mutated);
            pair.destroyed.subscribe(observer, ShapeCallback::Destroyed);
            entry
        };
        self.registrations[entry].bodies.push(body);
        Ok(())
    }
    /// cdffd0 removes the first matching body ID by swapping the last ID into
    /// its slot. An empty registration detaches and moves to the recycle pool.
    pub fn remove_body(
        &mut self,
        shape: usize,
        body: u32,
        signals: &mut impl ShapeSignalStore,
    ) -> Result<(), String> {
        let active_index = self
            .active
            .iter()
            .position(|&entry| self.registrations[entry].shape == Some(shape))
            .ok_or("removed body has no shape registration")?;
        let entry = self.active[active_index];
        let body_index = self.registrations[entry]
            .bodies
            .iter()
            .position(|&id| id == body)
            .ok_or("removed body is missing from its shape registration")?;
        self.registrations[entry].bodies.swap_remove(body_index);
        if self.registrations[entry].bodies.is_empty() {
            let pair = signals.signals_mut(shape)?;
            self.detach(
                ShapeObserver {
                    owner: self.owner,
                    entry,
                },
                shape,
                &mut pair.mutated,
                &mut pair.destroyed,
            )?;
            self.active.swap_remove(active_index);
            self.recycled.push(entry);
        }
        Ok(())
    }
    /// ce0510 ORs the flags, including a zero-valued notification, and marks
    /// the manager for processing. Flag interpretation belongs to that processor.
    pub fn mutated(&mut self, observer: ShapeObserver, flags: u8) -> Result<(), String> {
        self.entry_mut(observer)?.status |= flags;
        self.pending = true;
        Ok(())
    }
    /// ce04a0 detaches both callbacks before clearing the shape reference.
    /// Separate channels permit destruction callbacks to unsubscribe themselves
    /// through the currently emitting signal without borrowing it again.
    fn detach(
        &mut self,
        observer: ShapeObserver,
        shape: usize,
        mutated: &mut Signal<ShapeObserver, ShapeCallback>,
        destroyed: &mut Signal<ShapeObserver, ShapeCallback>,
    ) -> Result<(), String> {
        let entry = self.entry_mut(observer)?;
        if entry.shape != Some(shape) {
            return Err("shape observer signal owner does not match its registration".into());
        }
        mutated.unsubscribe(observer, ShapeCallback::Mutated);
        destroyed.unsubscribe(observer, ShapeCallback::Destroyed);
        entry.shape = None;
        Ok(())
    }
    /// ce0530 retains body IDs and flags after detaching; pending processing
    /// owns their later cleanup. Do not fabricate bounds for a destroyed shape.
    pub fn destroyed(
        &mut self,
        observer: ShapeObserver,
        shape: usize,
        mutated: &mut Signal<ShapeObserver, ShapeCallback>,
        destroyed: &mut Signal<ShapeObserver, ShapeCallback>,
    ) -> Result<(), String> {
        self.detach(observer, shape, mutated, destroyed)?;
        self.pending = true;
        Ok(())
    }
    /// cdfcb0/ce03d0 release active entries, then pooled entries. The resource
    /// owner supplies each live shape's signal pair before dropping that shape.
    pub fn release(mut self, signals: &mut impl ShapeSignalStore) -> Result<(), String> {
        let order: Vec<usize> = self.active.iter().chain(&self.recycled).copied().collect();
        for entry in order {
            if let Some(shape) = self.registrations[entry].shape {
                let pair = signals.signals_mut(shape)?;
                self.detach(
                    ShapeObserver {
                        owner: self.owner,
                        entry,
                    },
                    shape,
                    &mut pair.mutated,
                    &mut pair.destroyed,
                )?;
            }
            self.registrations[entry].bodies.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "shape_registry/tests.rs"]
mod tests;
