//! Sixteen canonical FOV slots19a2f0/19f570; registration/remove21b360/21b340.
use crate::{assets::fx::fov::Asset, gameplay::effects::{fov::{Runtime, Sample}, pool::EffectPoolLinks}};
use std::sync::Arc;

pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<Runtime>>,
    order: Vec<u16>,
}
impl Default for Systems {
    fn default() -> Self { Self { links: EffectPoolLinks::fov(), slots: Vec::new(), order: Vec::with_capacity(16) } }
}
impl Systems {
    pub fn start(&mut self, head: &mut u16, definition: Arc<Asset>, offsets: [f32; 4], seed: &mut u32) -> Result<(), String> {
        if !definition.enabled || !self.links.acquire(head)? { return Ok(()); }
        let index = usize::from(*head);
        while self.slots.len() <= index { self.slots.push(None); }
        match Runtime::new(definition, offsets, seed) {
            Ok(runtime) => { self.slots[index] = Some(runtime); self.order.push(*head); Ok(()) },
            Err(error) => { self.links.release_head(head)?; Err(error) },
        }
    }
    pub fn update(&mut self, head: &mut u16, age: f32, seed: &mut u32) -> Result<u32, String> {
        let slots = &mut self.slots;
        let order = &mut self.order;
        self.links.update_chain(head, |index| {
            let slot = slots.get_mut(usize::from(index)).ok_or("FOV slot absent")?;
            let runtime = slot.as_mut().ok_or("FOV runtime absent")?;
            if !runtime.active { *slot = None; order.retain(|registered| *registered != index); return Ok(false); }
            runtime.update(age, seed)?;
            Ok(true)
        })
    }
    pub fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FOV stop chain cycle".into()); }
            let runtime = self.slots.get_mut(usize::from(head)).and_then(Option::as_mut).ok_or("FOV stop runtime absent")?;
            if !looping_only || runtime.can_loop() { runtime.stop(age); }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }
    pub fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FOV clear chain cycle".into()); }
            *self.slots.get_mut(usize::from(*head)).ok_or("FOV clear slot absent")? = None;
            self.order.retain(|registered| *registered != *head);
            self.links.release_head(head)?;
        }
        Ok(())
    }
    pub fn samples(&self) -> impl Iterator<Item = Sample> + '_ {
        self.order.iter().filter_map(|index| self.slots[usize::from(*index)].as_ref()).map(|runtime|
            Sample { value: runtime.value, blend: runtime.blend, percentage: runtime.definition.percentage })
    }
    pub fn drained(&self) -> bool { self.links.allocated_count == 0 }
    pub fn shutdown(&mut self) { self.slots.clear(); self.order.clear(); }
}
