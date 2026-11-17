//! Shared32-slot quad family19a3f0/19f710, registration21d720/22a8b0.
use crate::{
    assets::fx::screen_quad::{ScreenQuadAsset, ScreenQuadStage},
    gameplay::effects::{
        pool::EffectPoolLinks,
        screen_quad::{Draw, Frame, Runtime},
    },
};
use std::sync::Arc;
pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<Runtime>>,
    order: Vec<u16>,
}
impl Default for Systems {
    fn default() -> Self {
        Self {
            links: EffectPoolLinks::screen_quads(),
            slots: Vec::new(),
            order: Vec::with_capacity(32),
        }
    }
}
impl Systems {
    pub fn start(
        &mut self,
        head: &mut u16,
        asset: Arc<ScreenQuadAsset>,
        offsets: [f32; 4],
        view_mask: u8,
        seed: &mut u32,
    ) -> Result<(), String> {
        if !asset.enabled || asset.material.is_none() || !self.links.acquire(head)? {
            return Ok(());
        }
        let index = usize::from(*head);
        while self.slots.len() <= index {
            self.slots.push(None);
        }
        match Runtime::new(asset, offsets, view_mask, seed) {
            Ok(runtime) => {
                self.slots[index] = Some(runtime);
                self.order.push(*head);
                Ok(())
            }
            Err(error) => {
                self.links.release_head(head)?;
                Err(error)
            }
        }
    }
    pub fn update(&mut self, head: &mut u16, age: f32, seed: &mut u32) -> Result<u32, String> {
        let slots = &mut self.slots;
        let order = &mut self.order;
        self.links.update_chain(head, |index| {
            let slot = slots
                .get_mut(usize::from(index))
                .ok_or("quad slot absent")?;
            let runtime = slot.as_mut().ok_or("quad runtime absent")?;
            if !runtime.blend.active {
                *slot = None;
                order.retain(|entry| *entry != index);
                return Ok(false);
            }
            runtime.blend.update(age, runtime.blend.view_mask, seed)?;
            Ok(true)
        })
    }
    pub fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("quad stop chain cycle".into());
            }
            let runtime = self
                .slots
                .get_mut(usize::from(head))
                .and_then(Option::as_mut)
                .ok_or("quad stop runtime absent")?;
            if !looping_only || runtime.blend.can_loop() {
                runtime.blend.stop(age);
            }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }
    pub fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("quad clear chain cycle".into());
            }
            *self
                .slots
                .get_mut(usize::from(*head))
                .ok_or("quad clear slot absent")? = None;
            self.order.retain(|entry| *entry != *head);
            self.links.release_head(head)?;
        }
        Ok(())
    }
    ///21d790 registers the actual matching-view runtime values, including
    ///zero blend; no source material IO or parameter parsing occurs here.
    pub fn frame(&self, view_mask: u8) -> Frame {
        let mut frame = Frame::default();
        // RenderView ctor 043800 initializes five 043540 arrays with flags
        // 0x440: four inline entries per stage. 88c210 evicts the oldest
        // entry before appending a fifth. Count first so only the surviving
        // leases are copied, retaining their original registration order.
        let mut remaining = [0_u8; 5];
        for &index in &self.order {
            let Some(runtime) = self.slots[usize::from(index)].as_ref() else {
                continue;
            };
            if runtime.blend.view_mask & view_mask != 0
                && runtime.asset.stage != ScreenQuadStage::Never
            {
                remaining[runtime.asset.stage as usize] += 1;
            }
        }
        let mut at = 0;
        for &index in &self.order {
            let Some(runtime) = self.slots[usize::from(index)].as_ref() else {
                continue;
            };
            if runtime.blend.view_mask & view_mask == 0
                || runtime.asset.stage == ScreenQuadStage::Never
            {
                continue;
            }
            let remaining = &mut remaining[runtime.asset.stage as usize];
            *remaining -= 1;
            if *remaining >= 4 {
                continue;
            }
            frame.draws[at] = Some(Draw {
                asset: runtime.asset.clone(),
                blend: runtime.blend.blend,
            });
            at += 1;
        }
        frame
    }
    pub fn drained(&self) -> bool {
        self.links.allocated_count == 0
    }
    pub fn shutdown(&mut self) {
        self.slots.clear();
        self.order.clear();
    }
}
