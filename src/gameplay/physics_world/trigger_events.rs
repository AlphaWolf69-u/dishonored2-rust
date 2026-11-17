//! 779cb0 ->095730/18cf60 ->094700 actual body-event and overlap drain.
//! Host layout: each collision payload's numeric +60 list is keyed by its full
//! canonical body generation. It retains no resource or component references.
use super::{
    contact_modes::{TriggerEvent, TriggerTransition},
    storage::WorldStorage,
    BodyEventCallback,
};
use crate::gameplay::component_state::entity::ComponentHandle;
use std::collections::HashMap;
struct Overlap {
    trigger: u32,
    count: u32,
}
#[derive(Default)]
pub(crate) struct TriggerEvents {
    events: Vec<TriggerEvent>,
    event_cursor: usize,
    property_cursor: usize,
    overlaps: HashMap<u32, Vec<Overlap>>,
    dirty: Vec<u32>,
    dirty_cursor: usize,
    contact_cursor: Option<usize>,
}
impl TriggerEvents {
    pub(crate) fn push(&mut self, event: TriggerEvent) {
        self.events.push(event);
    }
    pub(crate) fn pending(&self) -> bool {
        !self.events.is_empty() || !self.dirty.is_empty()
    }
    /// Release a retired collision payload only after its source Exit drain.
    /// No numeric overlap may survive into a reused body-generation slot.
    pub(crate) fn forget_body(&mut self, body: u32) -> Result<(), String> {
        if self.pending() {
            return Err("body overlap retirement still has unacknowledged events".into());
        }
        if self
            .overlaps
            .get(&body)
            .is_some_and(|list| !list.is_empty())
            || self
                .overlaps
                .values()
                .any(|list| list.iter().any(|entry| entry.trigger == body))
        {
            return Err("body overlap retirement still has live source contacts".into());
        }
        self.overlaps.remove(&body);
        Ok(())
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn dispatch(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut crate::assets::entities::triggers::TriggerAssets,
        mut resolve_trigger: impl FnMut(ComponentHandle) -> Result<Option<u32>, String>,
        mut deliver: impl FnMut(
            &mut WorldStorage,
            &mut crate::assets::entities::triggers::TriggerAssets,
            TriggerTransition,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        //The source global/per-body property heads may contain no subscriptions.
        //No callbacks exist until a real owner creates its listener chain.
        if !world.body_events.has_listener_heads() {
            self.events.clear();
            self.event_cursor = 0;
            self.property_cursor = 0;
        }
        while let Some(&event) = self.events.get(self.event_cursor) {
            let bodies = &world.bodies;
            let overlaps = &mut self.overlaps;
            let dirty = &mut self.dirty;
            let property_cursor = &mut self.property_cursor;
            let mut ordinal = 0;
            world.body_events.dispatch(
                0,
                event.bodies,
                |properties, property, side| {
                    let current = ordinal;
                    ordinal += 1;
                    if current < *property_cursor {
                        return Ok(());
                    }
                    let signal = properties
                        .value_mut(property)
                        .ok_or("trigger event signal absent")?;
                    signal.emit_resumable(&event, |_, handle, callback, event| {
                        if callback != BodyEventCallback::TriggerContact {
                            return Err("unknown trigger event subscriber".into());
                        }
                        let Some(trigger) = resolve_trigger(handle)? else {
                            return Ok(());
                        };
                        if trigger != event.bodies[side] {
                            return Err("trigger callback body differs from event chain".into());
                        }
                        let other = event.bodies[1 - side];
                        let Some(record) = bodies.get(other).filter(|body| body.flags & 3 != 0)
                        else {
                            return Ok(());
                        };
                        let list = overlaps.entry(record.id).or_default();
                        let entered = event.flags >> 8 == 0;
                        if let Some(overlap) =
                            list.iter_mut().find(|entry| entry.trigger == trigger)
                        {
                            //18cf60 preserves the high pending-Enter bit while
                            //incrementing/decrementing only its31-bit reference count.
                            let count = if entered {
                                overlap.count.wrapping_add(1)
                            } else {
                                overlap.count.wrapping_sub(1)
                            };
                            overlap.count = (overlap.count & 0x8000_0000) | (count & 0x7fff_ffff);
                        } else {
                            list.push(Overlap {
                                trigger,
                                count: 0x8000_0001,
                            });
                        }
                        dirty.push(other); //01d6b0 is append, preserving duplicate jobs.
                        Ok(())
                    })?;
                    *property_cursor += 1;
                    Ok(())
                },
                |signal| {
                    signal.destroy();
                    Ok(())
                },
            )?;
            self.event_cursor += 1;
            self.property_cursor = 0;
        }
        self.events.clear();
        self.event_cursor = 0;
        while let Some(&other) = self.dirty.get(self.dirty_cursor) {
            let Some(body) = world
                .bodies
                .get(other)
                .filter(|body| body.flags & 3 != 0)
                .map(|body| (body.id, body.user_data, body.filter_info))
            else {
                self.overlaps.remove(&other);
                self.dirty_cursor += 1;
                self.contact_cursor = None;
                continue;
            };
            let list = self
                .overlaps
                .get_mut(&other)
                .ok_or("dirty trigger overlap payload absent")?;
            if self.contact_cursor.is_none() {
                self.contact_cursor = list.len().checked_sub(1);
            }
            while let Some(index) = self.contact_cursor {
                let overlap = list
                    .get_mut(index)
                    .ok_or("trigger overlap drain cursor invalid")?;
                if let Some(trigger) = world
                    .bodies
                    .get(overlap.trigger)
                    .filter(|body| body.flags & 3 != 0)
                    .map(|body| (body.id, body.user_data))
                {
                    let transition = |entered| TriggerTransition {
                        trigger: trigger.0,
                        other: body.0,
                        userdata: [trigger.1, body.1],
                        other_filter_info: body.2,
                        entered,
                    };
                    if overlap.count & 0x8000_0000 != 0 {
                        deliver(world, geometry, transition(true))?;
                        overlap.count &= 0x7fff_ffff;
                    }
                    if (overlap.count.wrapping_mul(2) as i32) < 1 {
                        deliver(world, geometry, transition(false))?;
                        list.swap_remove(index);
                    }
                } else {
                    list.swap_remove(index);
                }
                self.contact_cursor = index.checked_sub(1);
            }
            self.dirty_cursor += 1;
        }
        self.dirty.clear();
        self.dirty_cursor = 0;
        self.contact_cursor = None;
        self.overlaps
            .retain(|&body, _| world.bodies.get(body).is_some());
        Ok(())
    }
}
