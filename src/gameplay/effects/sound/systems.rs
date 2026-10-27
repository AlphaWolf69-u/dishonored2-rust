//! Shared sound-family storage and ordered start/update/retirement.
use super::{
    handle::{SoundHandle, SoundObjectWorld},
    sample_position, SoundEvent, SoundInstance,
};
use crate::gameplay::effects::pool::EffectPoolLinks;
use bevy_math::{Mat3, Vec3};

pub struct SoundSystems<O, P> {
    links: EffectPoolLinks,
    slots: Vec<Option<SoundInstance<O, P>>>,
}
impl<O, P> Default for SoundSystems<O, P> {
    fn default() -> Self {
        let links = EffectPoolLinks::sounds();
        Self {
            slots: (0..links.next.len()).map(|_| None).collect(),
            links,
        }
    }
}
impl<O: Copy, P: Copy + PartialEq> SoundSystems<O, P> {
    /// 222f80 samples the position before attempting pool acquisition. A full
    /// pool consumes those three samples, but not 230160's start-delay sample.
    pub fn start(
        &mut self,
        head: &mut u16,
        position: [Vec3; 2],
        start: SoundEvent,
        stop: SoundEvent,
        start_age: f32,
        allow_virtualization: bool,
        seed: &mut u32,
    ) -> Result<(), String> {
        if start.event.is_none() && stop.event.is_none() {
            return Ok(());
        }
        let position = sample_position(position, seed);
        if self.links.acquire(head)? {
            self.slots[usize::from(*head)] = Some(SoundInstance::new(
                position,
                start,
                stop,
                start_age,
                allow_virtualization,
                seed,
            ));
        }
        Ok(())
    }

    /// Audio callbacks resolve their retained slot before delivering the real
    /// playing-ID completion or event-info packet to this handle.
    pub fn handle_mut(&mut self, slot: u16) -> Option<&mut SoundHandle<O, P>> {
        self.slots
            .get_mut(usize::from(slot))?
            .as_mut()
            .map(|slot| &mut slot.handle)
    }

    /// 220340 checks liveness before update. A slot finishing during update
    /// remains counted until the next traversal, as with the particle family.
    pub fn update<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        head: &mut u16,
        world: &mut W,
        age: f32,
        origin: Vec3,
        axis: Mat3,
        context: &W::Context,
    ) -> Result<u32, String> {
        let slots = &mut self.slots;
        self.links.update_chain(head, |index| {
            let slot = &mut slots[usize::from(index)];
            let sound = slot.as_mut().ok_or("FX sound pool slot is absent")?;
            if sound.is_alive() {
                sound.update(world, age, origin, axis, context)?;
                Ok(true)
            } else {
                sound.handle.release(world)?;
                *slot = None;
                Ok(false)
            }
        })
    }

    pub fn stop(&mut self, mut head: u16, age: f32, seed: &mut u32) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("FX sound stop chain contains a cycle".into());
            }
            let sound = self
                .slots
                .get_mut(usize::from(head))
                .and_then(Option::as_mut)
                .ok_or("FX sound stop slot is absent")?;
            sound.request_stop(age, seed);
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }

    pub fn clear<W: SoundObjectWorld<Object = O, Playing = P>>(
        &mut self,
        head: &mut u16,
        world: &mut W,
    ) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("FX sound clear chain contains a cycle".into());
            }
            let slot = self
                .slots
                .get_mut(usize::from(*head))
                .ok_or("FX sound clear index is invalid")?;
            if let Some(sound) = slot.as_mut() {
                sound.handle.release(world)?;
            }
            *slot = None;
            self.links.release_head(head)?;
        }
        Ok(())
    }
}
