//! Ordinary decal-FX family: 222f80 start, 2191f0 update, 2191d0 stop,
//! 218980 removal. Projection records and materials stay with their renderer.
use super::{DecalDescription, DecalEffect, DecalPool};
use crate::{
    assets::{decal::DecalMaterials, fx::decal::DecalDefinition},
    gameplay::{
        effects::{parameters::reference_index, pool::EffectPoolLinks},
        reference_frame::ReferenceFrame,
    },
};
use std::{num::NonZeroU64, sync::Arc};

struct Playing {
    definition: Arc<DecalDefinition>,
    effect: DecalEffect,
}

pub struct DecalSystems<M, R> {
    links: EffectPoolLinks,
    slots: Vec<Option<Playing>>,
    pool: DecalPool<DecalDescription<M>, R>,
}

impl<M, R> DecalSystems<M, R> {
    pub(crate) fn allocated_count(&self) -> u16 { self.links.allocated_count }

    pub fn new(projection_limit: usize) -> Result<Self, String> {
        let links = EffectPoolLinks::decals();
        Ok(Self {
            slots: (0..links.next.len()).map(|_| None).collect(),
            links,
            pool: DecalPool::new(projection_limit)?,
        })
    }

    /// Logical identity of the stable family slot, in place of the executable's
    /// FX-entry address. Six low zero bits preserve the pool's pointer hash.
    fn owner(slot: u16) -> NonZeroU64 {
        NonZeroU64::new((u64::from(slot) + 1) << 6).unwrap()
    }

    pub fn pool(&self) -> &DecalPool<DecalDescription<M>, R> {
        &self.pool
    }
    pub fn pool_mut(&mut self) -> &mut DecalPool<DecalDescription<M>, R> {
        &mut self.pool
    }

    pub fn start(
        &mut self,
        head: &mut u16,
        definition: Arc<DecalDefinition>,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Result<(), String> {
        // 22dcb0 only checks presence here; 218620 selects a declaration on
        // the first active sample. Moving that random draw to startup changes
        // every subsequent effect that shares the manager's random stream.
        if !definition.asset.enabled || !definition.declarations.iter().any(Option::is_some) {
            return Ok(());
        }
        if !self.links.acquire(head)? {
            return Ok(());
        }
        match DecalEffect::new(
            definition.asset.animation.clone(),
            &definition.asset.attachment,
            offsets,
            seed,
        ) {
            Ok(effect) => self.slots[usize::from(*head)] = Some(Playing { definition, effect }),
            Err(error) => {
                self.links.release_head(head)?;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn stop(&mut self, mut head: u16, now: f32) {
        while head != u16::MAX {
            if let Some(playing) = self.slots[usize::from(head)].as_mut() {
                playing
                    .effect
                    .animation
                    .timing
                    .begin_fade_out(playing.definition.asset.finish_loop_on_stop, now);
            }
            head = self.links.next[usize::from(head)];
        }
    }

    pub fn clear(
        &mut self,
        head: &mut u16,
        mut unregister: impl FnMut(&mut R),
    ) -> Result<(), String> {
        while *head != u16::MAX {
            let slot = *head;
            if let Some(playing) = self.slots[usize::from(slot)].take() {
                playing.effect.binding.detach(
                    Self::owner(slot),
                    &mut self.pool,
                    &mut unregister,
                )?;
            }
            self.links.release_head(head)?;
        }
        Ok(())
    }

    /// The material resolver supplies its shared, already prepared reference
    /// set. No file parsing, GPU allocation or material conversion occurs here.
    pub fn update(
        &mut self,
        head: &mut u16,
        now: f32,
        parent: ReferenceFrame,
        seed: &mut u32,
        mut resolve: impl FnMut(&Arc<DecalMaterials<String>>) -> Result<M, String>,
        mut unregister: impl FnMut(&mut R),
    ) -> Result<u32, String> {
        let Self { links, slots, pool } = self;
        links.update_chain(head, |slot| {
            let mut playing = slots[usize::from(slot)]
                .take()
                .ok_or("live decal link has no state")?;
            if !playing.effect.binding.active {
                playing
                    .effect
                    .binding
                    .detach(Self::owner(slot), pool, &mut unregister)?;
                return Ok(false);
            }
            let mut evicted = None;
            let result = (|| {
                let effect = &mut playing.effect;
                if let Some(sample) = effect.binding.update(&mut effect.animation, now, seed)? {
                    effect.binding.apply(
                        Self::owner(slot),
                        pool,
                        &effect.attachment,
                        parent,
                        sample,
                        || {
                            let index = reference_index(&playing.definition.declarations, seed)
                                .ok_or("admitted decal has no declaration")?;
                            resolve(playing.definition.declarations[index].as_ref().unwrap())
                        },
                        |owner| evicted = Some(owner),
                    )?;
                }
                // A decal becoming inactive during this update stays linked
                // until the next update, exactly as the shared family walker.
                Ok(true)
            })();
            slots[usize::from(slot)] = Some(playing);
            if let Some(owner) = evicted {
                let index = usize::try_from((owner.get() >> 6) - 1)
                    .map_err(|_| "decal owner index overflow")?;
                let previous = slots
                    .get_mut(index)
                    .and_then(Option::as_mut)
                    .ok_or("evicted decal has no owning FX state")?;
                previous.effect.binding.retire();
            }
            result
        })
    }
}
