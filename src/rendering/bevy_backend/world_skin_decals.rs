//! m_skinDecals family storage inside the world FX owner (pool 19a760, 64
//! slots). 22e8e0 arms a slot with its start time plus a sampled delay;
//! 22ea00 counts it down and, once due, queues a decal (22e970) on the FX
//! record's skinned target (+0xb8, set by 1404b1d80 from 140878470). The
//! slot then disarms and the chain update (220180) releases it. FX stop
//! leaves armed slots running; clear (21e6f0) releases them.
use crate::gameplay::effects::pool::EffectPoolLinks;

struct SkinDecal {
    remaining: f32,
    armed: bool,
}

pub(super) struct Systems {
    links: EffectPoolLinks,
    slots: Vec<Option<SkinDecal>>,
}
impl Default for Systems {
    fn default() -> Self { Self { links: EffectPoolLinks::skin_decals(), slots: Vec::new() } }
}
impl Systems {
    /// 222f80 bit 0x20: an enabled descriptor with a texture (22dcb0) and a
    /// free slot (222240) samples its delay (one draw) after the start time.
    pub(super) fn start(&mut self, head: &mut u16, asset: &crate::assets::fx::SkinDecalAsset,
        offsets: [f32; 4], seed: &mut u32) -> Result<(), String> {
        if !asset.enabled || !asset.textures.iter().any(|texture| !texture.is_empty() && texture != "NULL") {
            return Ok(());
        }
        if !self.links.acquire(head)? { return Ok(()); }
        let index = usize::from(*head);
        while self.slots.len() <= index { self.slots.push(None); }
        if self.slots[index].is_some() { return Err("FX skin decal free slot retains a decal".into()); }
        let delay = crate::gameplay::effects::parameters::sample_range(asset.delay, seed);
        self.slots[index] = Some(SkinDecal { remaining: crate::float::add(delay, offsets[0]), armed: true });
        Ok(())
    }

    /// 220180/22ea00. `skin_target` is the record's skinned render model.
    pub(super) fn update(&mut self, head: &mut u16, delta: f32, skin_target: bool) -> Result<u32, String> {
        let slots = &mut self.slots;
        self.links.update_chain(head, |index| {
            let decal = slots.get_mut(usize::from(index)).and_then(Option::as_mut).ok_or("FX skin decal absent")?;
            if !decal.armed {
                slots[usize::from(index)] = None;
                return Ok(false);
            }
            if decal.remaining <= 0. {
                if skin_target {
                    return Err("FX skin decal on a skinned target: projection 22e180 and paint pass 14025e8f0 are not ported".into());
                }
                decal.armed = false;
            }
            decal.remaining = crate::float::subtract(decal.remaining, delta);
            Ok(true)
        })
    }

    /// 21e6f0: release the whole chain.
    pub(super) fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        while *head != u16::MAX {
            let index = usize::from(*head);
            self.links.release_head(head)?;
            if let Some(slot) = self.slots.get_mut(index) { *slot = None; }
        }
        Ok(())
    }

    pub(super) fn drained(&self) -> bool { self.links.allocated_count == 0 }
}
