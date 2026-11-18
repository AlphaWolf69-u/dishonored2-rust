//! m_screenFades family storage inside the world FX owner (pool 19a6e0,
//! 32 slots, start 22a570). Started fades register in order (21d720 into
//! the view owner's +0xd8 list), which 14022a6b0 composes each frame.
use crate::gameplay::effects::{pool::EffectPoolLinks, screen_fade::{compose, ScreenFade}};
use std::sync::Arc;

pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<ScreenFade>>,
    order: Vec<u16>,
    frame: Option<[f32; 4]>,
}
impl Default for Systems {
    fn default() -> Self {
        Self { links: EffectPoolLinks::screen_fades(), slots: Vec::new(), order: Vec::new(), frame: None }
    }
}
impl Systems {
    pub(super) fn frame(&self) -> Option<[f32; 4]> { self.frame }

    pub(super) fn start(&mut self, head: &mut u16, asset: &Arc<crate::assets::fx::screen_fade::ScreenFadeAsset>,
        offsets: [f32; 4], seed: &mut u32) -> Result<(), String> {
        if !asset.enabled || !self.links.acquire(head)? { return Ok(()); }
        let index = *head;
        while self.slots.len() <= usize::from(index) { self.slots.push(None); }
        let fade = match ScreenFade::new(asset.clone(), offsets, seed) {
            Ok(fade) => fade,
            Err(error) => { self.links.release_head(head)?; return Err(error); }
        };
        if self.slots[usize::from(index)].is_some() { return Err("FX screen fade free slot retains a fade".into()); }
        self.slots[usize::from(index)] = Some(fade);
        self.order.push(index);
        Ok(())
    }

    pub(super) fn update(&mut self, head: &mut u16, age: f32, seed: &mut u32) -> Result<u32, String> {
        let slots = &mut self.slots;
        let order = &mut self.order;
        self.links.update_chain(head, |index| {
            let fade = slots.get_mut(usize::from(index)).and_then(Option::as_mut).ok_or("FX screen fade absent")?;
            fade.update(age, seed)?;
            if fade.active {
                Ok(true)
            } else {
                slots[usize::from(index)] = None;
                order.retain(|slot| *slot != index);
                Ok(false)
            }
        })
    }

    pub(super) fn stop(&mut self, mut head: u16, age: f32, looping_only: bool) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FX screen fade stop chain cycle".into()); }
            let fade = self.slots.get_mut(usize::from(head)).and_then(Option::as_mut).ok_or("FX screen fade stop absent")?;
            if !looping_only || fade.can_loop() { fade.stop(age); }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }

    pub(super) fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() { return Err("FX screen fade clear chain cycle".into()); }
            let index = *head;
            *self.slots.get_mut(usize::from(index)).ok_or("FX screen fade clear slot absent")? = None;
            self.order.retain(|slot| *slot != index);
            self.links.release_head(head)?;
        }
        Ok(())
    }

    pub(super) fn drained(&self) -> bool { self.links.allocated_count == 0 }

    pub(super) fn finish_frame(&mut self) {
        self.frame = compose(self.order.iter().filter_map(|&index| self.slots[usize::from(index)].as_ref()).map(|fade| fade.rgba));
    }

    pub(super) fn shutdown(&mut self) {
        self.order.clear();
        self.frame = None;
    }
}
