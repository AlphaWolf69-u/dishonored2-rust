//! FSDisto family storage inside the existing world FX owner.19a250/19f4a0,
//!221c40,21f070 and8911f0. No renderer/image/resource manager lives here.
use crate::{
    assets::fx::screen_distortion::{Blend, Prepared},
    gameplay::effects::{pool::EffectPoolLinks, screen_distortion::{Frame, ParameterSample, Runtime}},
};
use std::sync::Arc;

#[derive(Default)]
struct Slot {
    runtime: Option<Runtime>,
    prepared: Option<Arc<Prepared>>,
    values: [[f32; 4]; 16],
    scratch: Vec<ParameterSample>,
}
struct Submission { slot: u16, blend: f32 }
pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Slot>,
    submissions: Vec<Submission>,
    frame: Frame,
}
impl Default for Systems {
    fn default() -> Self {
        Self { links: EffectPoolLinks::screen_distortion(), slots: Vec::new(),
            submissions: Vec::with_capacity(64), frame: Frame::default() }
    }
}
impl Systems {
    pub(super) fn begin_frame(&mut self) { self.submissions.clear(); }
    pub(super) fn frame(&self) -> Frame { self.frame }
    fn submit(&mut self, index: u16) {
        let slot = &mut self.slots[usize::from(index)];
        let Some(runtime) = slot.runtime.as_ref() else { return };
        if !runtime.visible { return; }
        for (target, value) in slot.values.iter_mut().zip(runtime.values()) { *target = value; }
        //21d290 retains the runtime parm-block pointer and this blend. If a
        //block is updated again before8911f0, both rows see its latest values.
        self.submissions.push(Submission { slot: index, blend: runtime.blend });
    }
    pub(super) fn start(&mut self, head: &mut u16, definition: Arc<Prepared>, offsets: [f32; 4], seed: &mut u32) -> Result<(), String> {
        if !definition.asset.enabled || !self.links.acquire(head)? { return Ok(()); }
        let index = *head;
        while self.slots.len() <= usize::from(index) { self.slots.push(Slot::default()); }
        let scratch = std::mem::take(&mut self.slots[usize::from(index)].scratch);
        let runtime = match Runtime::new_reusing(definition.asset.clone(), offsets, seed, scratch) {
            Ok(value) => value,
            Err(error) => { self.links.release_head(head)?; return Err(error); }
        };
        let slot = &mut self.slots[usize::from(index)];
        if slot.runtime.is_some() { return Err("FSDisto free slot retains a runtime".into()); }
        slot.prepared = Some(definition);
        slot.runtime = Some(runtime);
        self.submit(index);
        Ok(())
    }
    pub(super) fn update(&mut self, head: &mut u16, age: f32, visible: bool, seed: &mut u32) -> Result<u32, String> {
        let slots = &mut self.slots;
        let submissions = &mut self.submissions;
        self.links.update_chain(head, |index| {
            let slot = slots.get_mut(usize::from(index)).ok_or("FSDisto update slot absent")?;
            let runtime = slot.runtime.as_mut().ok_or("FSDisto runtime absent")?;
            if runtime.active {
                runtime.update(age, visible, seed)?;
                if runtime.visible {
                    for (target, value) in slot.values.iter_mut().zip(runtime.values()) { *target = value; }
                    submissions.push(Submission { slot: index, blend: runtime.blend });
                }
                Ok(true)
            } else {
                slot.scratch = runtime.take_samples();
                slot.runtime = None;
                //The current view's submissions still borrow this block's
                //compiled bindings; finish_frame retires that final lease.
                Ok(false)
            }
        })
    }
    pub(super) fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FSDisto stop chain cycle".into()); }
            let runtime = self.slots.get_mut(usize::from(head)).and_then(|slot| slot.runtime.as_mut())
                .ok_or("FSDisto stop runtime absent")?;
            if !looping_only || runtime.can_loop() { runtime.stop(age); }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }
    pub(super) fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FSDisto clear chain cycle".into()); }
            let slot = self.slots.get_mut(usize::from(*head)).ok_or("FSDisto clear slot absent")?;
            if let Some(mut runtime) = slot.runtime.take() { slot.scratch = runtime.take_samples(); }
            self.links.release_head(head)?;
        }
        Ok(())
    }
    pub(super) fn drained(&self) -> bool { self.links.allocated_count == 0 }

    ///8911f0 resolves per-parameter defaults, per-row blend and declaration
    ///min/max/average policy. Options use source1411b5ad8(ceilf), then>0.
    pub(super) fn finish_frame(&mut self) {
        let mut defaults = [[0.; 4]; 23];
        let mut sum = [[0.; 4]; 23];
        let mut maximum = [[-f32::MAX; 4]; 23];
        let mut minimum = [[f32::MAX; 4]; 23];
        let mut count = [0u32; 23];
        let mut blend = [Blend::Average; 23];
        let mut option = [false; 23];
        for submission in &self.submissions {
            let slot = &self.slots[usize::from(submission.slot)];
            let Some(prepared) = slot.prepared.as_ref() else { continue };
            defaults = prepared.defaults;
            for (binding, value) in prepared.bindings.iter().zip(slot.values) {
                let Some(binding) = binding else { continue };
                let id = binding.slot;
                count[id] += 1;
                blend[id] = binding.blend;
                option[id] = binding.option;
                for lane in 0..4 {
                    let default = crate::gameplay::effects::normalize_small(binding.default[lane]);
                    let value = crate::gameplay::effects::normalize_small(value[lane]);
                    let value = default * (1. - submission.blend) + value * submission.blend;
                    sum[id][lane] += value;
                    if maximum[id][lane] <= value { maximum[id][lane] = value; }
                    if value <= minimum[id][lane] { minimum[id][lane] = value; }
                }
            }
        }
        for id in 0..23 {
            if count[id] == 0 { continue; }
            for lane in 0..4 {
                let value = match blend[id] { Blend::Minimum => minimum[id][lane], Blend::Maximum => maximum[id][lane],
                    Blend::Average => sum[id][lane] / count[id] as f32 };
                defaults[id][lane] = if option[id] { if crate::float::truncate_i32(value.ceil()) > 0 { 1. } else { 0. } } else { value };
            }
        }
        self.frame = Frame::from_values(defaults);
        for slot in &mut self.slots { if slot.runtime.is_none() { slot.prepared = None; } }
    }
    pub(super) fn shutdown(&mut self) {
        self.submissions.clear();
        self.frame = Frame::default();
        for slot in &mut self.slots { if slot.runtime.is_none() { slot.prepared = None; } }
    }
}
