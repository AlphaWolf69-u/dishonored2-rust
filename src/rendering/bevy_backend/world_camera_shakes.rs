//! m_cameraShakes family storage inside the world FX owner (pool 19a010,
//! 32 slots, start 22d700). Every live shake adds into one view
//! accumulator per frame (14022db30 reset, 14022d9e0 add).
use crate::gameplay::effects::{camera_shake::{CameraShake, ShakeAccumulator}, pool::EffectPoolLinks};
use std::sync::Arc;

pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<CameraShake>>,
    accumulator: ShakeAccumulator,
    frame: ShakeAccumulator,
}
impl Default for Systems {
    fn default() -> Self {
        Self { links: EffectPoolLinks::camera_shakes(), slots: Vec::new(), accumulator: Default::default(), frame: Default::default() }
    }
}
impl Systems {
    pub(super) fn frame(&self) -> ShakeAccumulator { self.frame }

    pub(super) fn begin_frame(&mut self) { self.accumulator = ShakeAccumulator::default(); }

    pub(super) fn start(&mut self, head: &mut u16, asset: &Arc<crate::assets::fx::CameraShakeAsset>, offsets: [f32; 4],
        seed: &mut u32) -> Result<(), String> {
        if !asset.enabled || !self.links.acquire(head)? { return Ok(()); }
        let index = usize::from(*head);
        while self.slots.len() <= index { self.slots.push(None); }
        let shake = match CameraShake::new(asset.clone(), offsets, seed) {
            Ok(shake) => shake,
            Err(error) => { self.links.release_head(head)?; return Err(error); }
        };
        if self.slots[index].is_some() { return Err("FX camera shake free slot retains a shake".into()); }
        self.slots[index] = Some(shake);
        Ok(())
    }

    /// Shakes add only when the FX is visible in the view (+0xed & +0x154)
    /// and a view camera exists.
    pub(super) fn update(&mut self, head: &mut u16, age: f32, visible: bool, seed: &mut u32,
        camera: Option<crate::gameplay::reference_frame::ReferenceFrame>, origin: [f32; 3]) -> Result<u32, String> {
        let slots = &mut self.slots;
        let accumulator = &mut self.accumulator;
        self.links.update_chain(head, |index| {
            let shake = slots.get_mut(usize::from(index)).and_then(Option::as_mut).ok_or("FX camera shake absent")?;
            let mut scratch = ShakeAccumulator::default();
            // approximate_: the FX instance's impulse (+0xb8) is not
            // published; zero samples the impulse fade at its start.
            let target = if visible && camera.is_some() { &mut *accumulator } else { &mut scratch };
            let view = camera.map_or(([0.; 3], [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]), |camera| (camera.position, camera.basis));
            shake.update(age, seed, view, origin, 0., target)?;
            if shake.active {
                Ok(true)
            } else {
                slots[usize::from(index)] = None;
                Ok(false)
            }
        })
    }

    pub(super) fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FX camera shake stop chain cycle".into()); }
            let shake = self.slots.get_mut(usize::from(head)).and_then(Option::as_mut).ok_or("FX camera shake stop absent")?;
            if !looping_only || shake.can_loop() { shake.stop(age); }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }

    pub(super) fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FX camera shake clear chain cycle".into()); }
            *self.slots.get_mut(usize::from(*head)).ok_or("FX camera shake clear slot absent")? = None;
            self.links.release_head(head)?;
        }
        Ok(())
    }

    pub(super) fn drained(&self) -> bool { self.links.allocated_count == 0 }

    pub(super) fn finish_frame(&mut self) { self.frame = self.accumulator; }

    pub(super) fn shutdown(&mut self) {
        self.frame = ShakeAccumulator::default();
        self.accumulator = ShakeAccumulator::default();
    }
}
