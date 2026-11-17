//! Shared FX-light family pool:19a510/19f8b0,222f80,21fa00,21e3c0.
//! The host renderer retains its allocation per real pool slot across reuse;
//! this owner holds only source runtime, links and its last published sample.
use crate::{
    assets::fx::light::LightAsset,
    gameplay::{
        effects::{
            light::{LightEffect, LightSample},
            pool::EffectPoolLinks,
        },
        reference_frame::ReferenceFrame,
    },
};
use std::sync::Arc;

#[derive(Default)]
struct Slot {
    runtime: Option<LightEffect>,
    sample: Option<LightSample>,
    ///226060 creates renderer state only on the first admission of a slot.
    ///The retained disabled allocation survives source chain release.
    allocated: bool,
}
pub(super) struct LightSystems {
    links: EffectPoolLinks,
    slots: Vec<Slot>,
}
impl Default for LightSystems {
    fn default() -> Self {
        let links = EffectPoolLinks::lights();
        Self {
            //bevy_backend: source links retain all128 identities, but empty
            //typed runtime records need storage only at the first demanded slot.
            slots: Vec::new(),
            links,
        }
    }
}
impl LightSystems {
    pub(super) fn start(
        &mut self,
        head: &mut u16,
        asset: Arc<LightAsset>,
        offsets: [f32; 4],
        owner: ReferenceFrame,
        seed: &mut u32,
    ) -> Result<(), String> {
        //222f80 checks enabled, then221f40 BEFORE226060 samples timing/curves.
        //A refused native pool acquisition consumes no light-family RNG.
        if !asset.enabled || !self.links.acquire(head)? {
            return Ok(());
        }
        let runtime = match LightEffect::new(asset, offsets, owner, seed) {
            Ok(runtime) => runtime,
            Err(error) => {
                //Host validation failed before renderer publication. Restore
                //the actual acquired link rather than strand an empty entry.
                self.links.release_head(head)?;
                return Err(error);
            }
        };
        let required = usize::from(*head) + 1;
        if self.slots.len() < required { self.slots.resize_with(required, Slot::default); }
        let slot = self
            .slots
            .get_mut(usize::from(*head))
            .ok_or("FX light slot absent")?;
        if slot.runtime.is_some() {
            return Err("FX light free slot still owns a runtime".into());
        }
        slot.runtime = Some(runtime);
        slot.sample = None;
        slot.allocated = true;
        Ok(())
    }

    pub(super) fn update(
        &mut self,
        head: &mut u16,
        age: f32,
        owner: ReferenceFrame,
        seed: &mut u32,
    ) -> Result<u32, String> {
        let slots = &mut self.slots;
        self.links.update_chain(head, |index| {
            let slot = slots
                .get_mut(usize::from(index))
                .ok_or("FX light update slot absent")?;
            let runtime = slot.runtime.as_mut().ok_or("FX light runtime absent")?;
            //21fa00 checks liveness BEFORE226290. A terminal update retains
            //the last renderer state and chain count until next traversal.
            if runtime.is_alive() {
                if let Some(sample) = runtime.update(age, owner, seed)? {
                    slot.sample = Some(sample);
                }
                Ok(true)
            } else {
                //224eb0 withdraws the enabled state, not the retained render
                //allocation. The host publishes None for this same real slot.
                slot.sample = None;
                slot.runtime = None;
                Ok(false)
            }
        })
    }

    ///223c20/220c00 stops every light;223dd0/2215e0 only looping entries.
    pub(super) fn stop(
        &mut self,
        mut head: u16,
        age: f32,
        looping_only: bool,
    ) -> Result<(), String> {
        let mut visited = 0;
        while head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("FX light stop chain cycle".into());
            }
            let runtime = self
                .slots
                .get_mut(usize::from(head))
                .and_then(|slot| slot.runtime.as_mut())
                .ok_or("FX light stop slot absent")?;
            if !looping_only || runtime.can_loop() {
                runtime.stop(age);
            }
            head = self.links.next[usize::from(head)];
        }
        Ok(())
    }

    ///21e3c0 hard-clears a whole chain before returning its links to the pool.
    pub(super) fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        let mut visited = 0;
        while *head != u16::MAX {
            visited += 1;
            if visited > self.slots.len() {
                return Err("FX light clear chain cycle".into());
            }
            let slot = self
                .slots
                .get_mut(usize::from(*head))
                .ok_or("FX light clear slot absent")?;
            slot.runtime = None;
            slot.sample = None;
            self.links.release_head(head)?;
        }
        Ok(())
    }

    ///Borrowed on the actual host publication phase. No rebuilt snapshot,
    ///per-frame allocation, source conversion or second playback clock.
    pub(crate) fn publication(&self) -> impl Iterator<Item = (u16, Option<&LightSample>)> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.allocated)
            .map(|(index, slot)| (index as u16, slot.sample.as_ref()))
    }
    pub(super) fn drained(&self) -> bool {
        self.links.allocated_count == 0
    }
}
