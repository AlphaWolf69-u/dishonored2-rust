//! Queued component update membership, 7e7800/7e79c0 and 7ea6e0.
//! The live typed component pools own actual roster insertion/removal. Queued
//! handles use the same registry and are retired before their slots are freed.
use super::{entity::ComponentHandle, lifecycle::{sort_batch, ComponentLifecycleWorld}, ComponentRegistry};
use crate::storage::PointerList;

pub trait ComponentUpdateWorld: ComponentLifecycleWorld {
    /// Dispatch the real component virtuals +0x50 and +0x58, including any
    /// component-specific override. Generic components call the base helpers.
    fn update_remove(&mut self, component: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String>;
    fn update_add(&mut self, component: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String>;
    /// The component's real +0x88 pool dispatch, 48ff60 / 48fe10.
    fn set_update_scheduled(&mut self, component: ComponentHandle, scheduled: bool) -> Result<(), String>;
}

fn cancel(list: &mut PointerList, cursor: i32, component: ComponentHandle) -> Result<bool, String> {
    let index = list.values().iter().position(|&value| value == component.packed())
        .map_or(-1, |index| index as i32);
    if cursor <= index { list.remove_at(index)?; Ok(true) } else { Ok(false) }
}

impl ComponentRegistry {
    /// A processed prefix still owns its pointers until its batch is retired.
    /// Never recycle the component slot while either queue retains its handle.
    pub fn ensure_no_update_references(&self, component: ComponentHandle) -> Result<(), String> {
        let identity = component.packed();
        if self.pending_update_remove.values().contains(&identity)
            || self.pending_update_add.values().contains(&identity)
        {
            return Err("component retirement is waiting for its update batch".into());
        }
        Ok(())
    }

    fn push_update(&mut self, component: ComponentHandle, add: bool) -> Result<(), String> {
        let list = if add { &mut self.pending_update_add } else { &mut self.pending_update_remove };
        let before = list.count();
        match list.append_preserving(component.packed(), false) {
            Ok(index) if index == before && list.count() == before + 1 => {},
            Ok(_) => return Err("component update queue rejected the append".into()),
            Err(error) => return Err(error),
        }
        let count = list.count() as i32;
        self.maximum_registered = self.maximum_registered.max(count);
        Ok(())
    }

    /// 7e7800: increment the 21-bit pause count, queueing the first removal.
    pub fn pause_updates(&mut self, component: ComponentHandle, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        let mut flags = world.owner(component)?.flags;
        let result = self.pause_update_flags(component, &mut flags);
        world.owner_mut(component)?.flags = flags;
        result
    }

    /// The same 7e7800 operation with the actual typed owner's flags already
    /// borrowed by a physics publication callback.
    pub(crate) fn pause_update_flags(&mut self, component: ComponentHandle, flags_out: &mut u32) -> Result<(), String> {
        const COUNT: u32 = 0x07ff_ffc0;
        let previous = *flags_out;
        let mut flags = (previous & !COUNT) | ((previous.wrapping_add(0x40)) & COUNT);
        *flags_out = flags;
        if flags & COUNT == 0x40 && (flags >> 4 & 3) != 1 {
            if flags >> 4 & 3 == 3 && cancel(&mut self.pending_update_add, self.processed_update_add_count, component)? {
                flags = (flags & !0x10) | 0x20;
                *flags_out = flags;
            }
            if flags & 0x30 == 0 {
                flags = (flags & !0x20) | 0x10;
                *flags_out = flags;
                if let Err(error) = self.push_update(component, false) {
                    *flags_out = previous;
                    return Err(error);
                }
            }
        }
        *flags_out = flags;
        Ok(())
    }

    /// 7e79c0: decrement only a nonzero pause count, queueing the last resume.
    pub fn resume_updates(&mut self, component: ComponentHandle, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        let mut flags = world.owner(component)?.flags;
        let result = self.resume_update_flags(component, &mut flags);
        world.owner_mut(component)?.flags = flags;
        result
    }

    /// The same 7e79c0 operation against the already-borrowed typed owner.
    pub(crate) fn resume_update_flags(&mut self, component: ComponentHandle, flags_out: &mut u32) -> Result<(), String> {
        const COUNT: u32 = 0x07ff_ffc0;
        let previous = *flags_out;
        if previous & COUNT == 0 { return Ok(()); }
        let mut flags = (previous & !COUNT) | (previous.wrapping_sub(0x40) & COUNT);
        *flags_out = flags;
        if flags & COUNT == 0 && (flags >> 4 & 3) != 3 {
            if flags >> 4 & 3 == 1 && cancel(&mut self.pending_update_remove, self.processed_update_remove_count, component)? {
                flags &= !0x30;
                *flags_out = flags;
            }
            if flags & 0x30 == 0x20 {
                *flags_out = flags | 0x30;
                if let Err(error) = self.push_update(component, true) {
                    *flags_out = previous;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Base virtual +0x50 (7e75b0); overrides remain with the typed owner.
    pub fn base_update_remove(&mut self, component: ComponentHandle, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        let previous = world.owner(component)?.flags;
        world.owner_mut(component)?.flags = (previous & !0x10) | 0x20;
        if let Err(error) = world.set_update_scheduled(component, false) {
            world.owner_mut(component)?.flags = previous;
            return Err(error);
        }
        let flags = world.owner(component)?.flags;
        if flags & 0x07ff_ffc0 == 0 {
            world.owner_mut(component)?.flags = flags | 0x30;
            if let Err(error) = self.push_update(component, true) {
                world.set_update_scheduled(component, true)?;
                world.owner_mut(component)?.flags = previous;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Base virtual +0x58 (7e7610); overrides remain with the typed owner.
    pub fn base_update_add(&mut self, component: ComponentHandle, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        let previous = world.owner(component)?.flags;
        world.owner_mut(component)?.flags = previous & !0x30;
        if let Err(error) = world.set_update_scheduled(component, true) {
            world.owner_mut(component)?.flags = previous;
            return Err(error);
        }
        let flags = world.owner(component)?.flags;
        if flags & 0x07ff_ffc0 != 0 {
            world.owner_mut(component)?.flags = (flags & !0x20) | 0x10;
            if let Err(error) = self.push_update(component, false) {
                world.set_update_scheduled(component, false)?;
                world.owner_mut(component)?.flags = previous;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Base virtual +0x30 (7e7750) removes scheduled membership or cancels
    /// the appropriate pending operation before the component slot can retire.
    pub fn stop_update_ownership(&mut self, component: ComponentHandle, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        let previous = world.owner(component)?.flags;
        let mut flags = previous & !0x0c;
        world.owner_mut(component)?.flags = flags;
        if flags & 0x1000_0000 != 0 { return Ok(()); }
        if flags & 0x30 < 0x20 {
            if let Err(error) = world.set_update_scheduled(component, false) {
                world.owner_mut(component)?.flags = previous;
                return Err(error);
            }
        }
        match flags >> 4 & 3 {
            1 => { cancel(&mut self.pending_update_remove, self.processed_update_remove_count, component)?; }
            3 => { cancel(&mut self.pending_update_add, self.processed_update_add_count, component)?; }
            _ => {}
        }
        flags = world.owner(component)?.flags;
        world.owner_mut(component)?.flags = flags & 0xf800_000f;
        Ok(())
    }

    /// 7ea6e0: sorted removals first, then additions in insertion order.
    /// Callbacks can enqueue more work; an interrupted host callback leaves
    /// the captured batch and its completed prefix available for retry.
    pub fn drain_update_changes(&mut self, world: &mut impl ComponentUpdateWorld) -> Result<(), String> {
        while self.pending_update_remove.count() != 0 || self.pending_update_add.count() != 0 {
            while self.pending_update_remove.count() != 0 {
                if self.processed_update_remove_count == 0 {
                    self.processed_update_remove_count = self.pending_update_remove.count() as i32;
                    if let Err(error) = sort_batch(&mut self.pending_update_remove, world) {
                        self.processed_update_remove_count = 0;
                        return Err(error);
                    }
                }
                while self.update_remove_next < self.processed_update_remove_count as u32 {
                    let value = *self.pending_update_remove.values().get(self.update_remove_next as usize)
                        .ok_or("component update-removal cursor exceeds its batch")?;
                    world.update_remove(ComponentHandle::from_packed(value)?, self)?;
                    self.update_remove_next += 1;
                }
                if self.pending_update_remove.finish_prefix(self.processed_update_remove_count as u32, 16)? {
                    self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                }
                self.processed_update_remove_count = 0;
                self.update_remove_next = 0;
            }
            while self.pending_update_add.count() != 0 {
                if self.processed_update_add_count == 0 {
                    self.processed_update_add_count = self.pending_update_add.count() as i32;
                }
                while self.update_add_next < self.processed_update_add_count as u32 {
                    let value = *self.pending_update_add.values().get(self.update_add_next as usize)
                        .ok_or("component update-addition cursor exceeds its batch")?;
                    world.update_add(ComponentHandle::from_packed(value)?, self)?;
                    self.update_add_next += 1;
                }
                if self.pending_update_add.finish_prefix(self.processed_update_add_count as u32, 16)? {
                    self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                }
                self.processed_update_add_count = 0;
                self.update_add_next = 0;
            }
        }
        Ok(())
    }
}
