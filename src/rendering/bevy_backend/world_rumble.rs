//! The existing FX manager's32 rumble slots19a650/19fa50 and229f10 max mixer.
use crate::{
    assets::fx::rumble::Asset,
    gameplay::effects::{pool::EffectPoolLinks, rumble::Runtime},
};
use std::sync::Arc;

pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<Runtime>>,
    listener: Option<[f32; 3]>,
    frame: [f32; 4],
    completed: Option<(u32, [f32; 4])>,
}
impl Default for Systems {
    fn default() -> Self {
        Self {
            links: EffectPoolLinks::rumble(),
            slots: Vec::new(),
            listener: None,
            frame: [0.; 4],
            completed: None,
        }
    }
}
impl Systems {
    pub fn begin_frame(&mut self, listener: Option<[f32; 3]>) {
        self.listener = listener;
        self.frame = [0.; 4];
    }
    pub fn completed(&self) -> Option<(u32, [f32; 4])> {
        self.completed
    }
    pub fn finish_frame(&mut self, frame: u32) {
        self.completed = Some((frame, self.frame));
    }
    pub fn start(
        &mut self,
        head: &mut u16,
        definition: Arc<Asset>,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Result<(), String> {
        if !definition.enabled || !self.links.acquire(head)? {
            return Ok(());
        }
        let index = usize::from(*head);
        while self.slots.len() <= index {
            self.slots.push(None);
        }
        match Runtime::new(definition, offsets, seed) {
            Ok(runtime) => {
                self.slots[index] = Some(runtime);
                Ok(())
            }
            Err(error) => {
                self.links.release_head(head)?;
                Err(error)
            }
        }
    }
    pub fn update(
        &mut self,
        head: &mut u16,
        age: f32,
        delta: f32,
        position: [f32; 3],
        impulse: f32,
        seed: &mut u32,
    ) -> Result<u32, String> {
        let slots = &mut self.slots;
        let frame = &mut self.frame;
        let listener = self.listener;
        self.links.update_chain(head, |index| {
            let slot = slots
                .get_mut(usize::from(index))
                .ok_or("rumble slot absent")?;
            let runtime = slot.as_mut().ok_or("rumble runtime absent")?;
            if !runtime.active {
                *slot = None;
                return Ok(false);
            }
            if let Some(values) = runtime.update(age, delta, listener, position, impulse, seed)? {
                for (current, incoming) in frame.iter_mut().zip(values) {
                    //229f10: comparison preserves the existing value on NaN.
                    if *current <= incoming {
                        *current = incoming;
                    }
                }
            }
            Ok(true)
        })
    }
    pub fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("rumble stop chain cycle".into());
            }
            let runtime = self
                .slots
                .get_mut(usize::from(head))
                .and_then(Option::as_mut)
                .ok_or("rumble stop runtime absent")?;
            if !looping_only || runtime.can_loop() {
                runtime.stop(age);
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
                return Err("rumble clear chain cycle".into());
            }
            *self
                .slots
                .get_mut(usize::from(*head))
                .ok_or("rumble clear slot absent")? = None;
            self.links.release_head(head)?;
        }
        Ok(())
    }
    pub fn drained(&self) -> bool {
        self.links.allocated_count == 0
    }
    pub fn shutdown(&mut self) {
        self.slots.clear();
        self.frame = [0.; 4];
        self.listener = None;
        self.completed = None;
    }
}
