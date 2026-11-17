//!idEventReceiver883740/883a10/883b00/884b70/884c20/883760.
use super::*;
impl LiveEntities {
    fn receiver(&self, id: EntityId) -> Result<&Entry, String> {
        let slot = self
            .slots
            .get(id.0 as u32 as usize)
            .ok_or("event receiver slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 {
            return Err("event receiver generation stale".into());
        }
        slot.entry
            .as_deref()
            .ok_or_else(|| "event receiver retired".into())
    }
    fn receiver_mut(&mut self, id: EntityId) -> Result<&mut Entry, String> {
        let slot = self
            .slots
            .get_mut(id.0 as u32 as usize)
            .ok_or("event receiver slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 {
            return Err("event receiver generation stale".into());
        }
        slot.entry
            .as_deref_mut()
            .ok_or_else(|| "event receiver retired".into())
    }
    ///883a10 appends only absent observers, then883b00 maintains the
    ///reciprocal subject. Both capacities are prepared before mutation.
    pub(crate) fn observe_events(
        &mut self,
        subject: EntityId,
        observer: EntityId,
    ) -> Result<(), String> {
        let incoming = !self
            .receiver(subject)?
            .event_observers
            .as_ref()
            .is_some_and(|list| list.contains(&observer));
        let outgoing = !self
            .receiver(observer)?
            .event_subjects
            .as_ref()
            .is_some_and(|list| list.contains(&subject));
        if incoming {
            self.receiver_mut(subject)?
                .event_observers
                .get_or_insert_with(|| Box::new(Vec::new()))
                .try_reserve(1)
                .map_err(|error| format!("event observer list: {error}"))?;
        }
        if outgoing {
            self.receiver_mut(observer)?
                .event_subjects
                .get_or_insert_with(|| Box::new(Vec::new()))
                .try_reserve(1)
                .map_err(|error| format!("event subject list: {error}"))?;
        }
        if incoming {
            self.receiver_mut(subject)?
                .event_observers
                .as_mut()
                .unwrap()
                .push(observer);
        }
        if outgoing {
            self.receiver_mut(observer)?
                .event_subjects
                .as_mut()
                .unwrap()
                .push(subject);
        }
        Ok(())
    }
    ///884b70/884c20 use60aee0's stable removal, preserving notice order.
    pub(crate) fn unobserve_events(
        &mut self,
        subject: EntityId,
        observer: EntityId,
    ) -> Result<(), String> {
        self.receiver(subject)?;
        self.receiver(observer)?;
        if let Some(list) = self.receiver_mut(subject)?.event_observers.as_mut() {
            if let Some(index) = list.iter().position(|entry| *entry == observer) {
                list.remove(index);
            }
        }
        if let Some(list) = self.receiver_mut(observer)?.event_subjects.as_mut() {
            if let Some(index) = list.iter().position(|entry| *entry == subject) {
                list.remove(index);
            }
        }
        Ok(())
    }
    ///883d10 snapshots before callbacks and traverses in reverse. Callback
    ///unregistration therefore cannot invalidate the current dispatch list.
    pub(crate) fn event_observer_snapshot(
        &self,
        subject: EntityId,
    ) -> Result<Vec<EntityId>, String> {
        let mut snapshot = Vec::new();
        if let Some(source) = self.receiver(subject)?.event_observers.as_ref() {
            snapshot
                .try_reserve_exact(source.len())
                .map_err(|error| format!("event observer snapshot: {error}"))?;
            snapshot.extend(source.iter().rev().copied());
        }
        Ok(snapshot)
    }
    ///883d10 asks each receiver's virtual+20 before invoking virtual+10.
    ///The sole currently admitted generic observer is7854d0's actual Trigger
    ///touch observer:8dc740's Remove12c handler does not subscribe to the
    ///PlayerEquipmentExploded notice7e. A future observer owner must provide
    ///its actual filter here, rather than silently accepting unknown callbacks.
    pub(crate) fn equipment_exploded_observer_matches(
        &self,
        observer: EntityId,
    ) -> Result<bool, String> {
        let receiver = self.receiver(observer)?;
        if receiver.trigger_touching.is_some() {
            Ok(false)
        } else {
            Err("playerEquipmentExploded has an unported generic notice receiver".into())
        }
    }
    ///883d10/notice81: the admitted generic observer is the actual Trigger
    ///touch owner. Its8e5500 accepts129..12c, then8d8740; neither filter
    ///accepts81. Unknown receiver families require their actual callback.
    pub(crate) fn grappled_observer_matches(&self, observer: EntityId) -> Result<bool, String> {
        let receiver = self.receiver(observer)?;
        if receiver.trigger_touching.is_some() {
            Ok(false)
        } else {
            Err("grappled has an unported generic notice receiver".into())
        }
    }
    ///785690: actual native Trigger entity list, independent of components.
    pub(crate) fn trigger_is_touching(
        &self,
        trigger: EntityId,
        other: EntityId,
    ) -> Result<bool, String> {
        Ok(self
            .receiver(trigger)?
            .trigger_touching
            .as_ref()
            .is_some_and(|list| list.contains(&other)))
    }
    ///7854d0 appends once and registers its generic notice observer even
    ///when the entity was already touching.
    pub(crate) fn trigger_touch(
        &mut self,
        trigger: EntityId,
        other: EntityId,
    ) -> Result<(), String> {
        self.receiver(other)?;
        let list = self
            .receiver_mut(trigger)?
            .trigger_touching
            .get_or_insert_with(|| Box::new(Vec::new()));
        let append = !list.contains(&other);
        if append {
            list.try_reserve(1)
                .map_err(|error| format!("Trigger touching list: {error}"))?;
        }
        self.observe_events(other, trigger)?;
        if append {
            self.receiver_mut(trigger)?
                .trigger_touching
                .as_mut()
                .unwrap()
                .push(other);
        }
        Ok(())
    }
    ///7855d0 removes the generic pair before7b8270's swap removal.
    pub(crate) fn trigger_untouch(
        &mut self,
        trigger: EntityId,
        other: EntityId,
    ) -> Result<(), String> {
        self.unobserve_events(other, trigger)?;
        if let Some(list) = self.receiver_mut(trigger)?.trigger_touching.as_mut() {
            if let Some(index) = list.iter().position(|entry| *entry == other) {
                list.swap_remove(index);
            }
        }
        Ok(())
    }
    ///883760 unlinks outgoing subjects first, then incoming observers, in
    ///reverse order. Clearing each side retains no cross-world references.
    pub(super) fn detach_event_observers(&mut self, id: EntityId) -> Option<()> {
        while let Some(subject) = self
            .receiver(id)
            .ok()?
            .event_subjects
            .as_ref()
            .and_then(|list| list.last())
            .copied()
        {
            self.unobserve_events(subject, id).ok()?;
        }
        while let Some(observer) = self
            .receiver(id)
            .ok()?
            .event_observers
            .as_ref()
            .and_then(|list| list.last())
            .copied()
        {
            self.unobserve_events(id, observer).ok()?;
        }
        Some(())
    }
}
