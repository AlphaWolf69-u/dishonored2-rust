//! 1bf9f0/1bfde0/1c0450/1c0530: bounded dynamic-decal ownership.
//! The caller owns FX notifications and render-world registration; renderer
//! objects survive slot eviction and are released only by explicit removal.
use crate::render_model::SurfaceHashIndex;
use std::num::NonZeroU64;

const CAPACITY: usize = 128;

pub struct DecalSlot<D, R> {
    pub description: Option<D>,
    pub renderer: Option<R>,
    pub registration: u32,
    pub registered: bool,
    owner: Option<NonZeroU64>,
}
impl<D, R> DecalSlot<D, R> {
    pub fn owner(&self) -> Option<NonZeroU64> {
        self.owner
    }
}

pub struct DecalPool<D, R> {
    slots: [DecalSlot<D, R>; CAPACITY],
    available: Vec<u32>,
    alive: Vec<u32>,
    hash: SurfaceHashIndex,
    limit: usize,
}
impl<D, R> DecalPool<D, R> {
    /// The caller supplies the admitted r_maxDynamicDecals value. Zero cannot
    /// satisfy 1bfde0's oldest-slot precondition, and storage has 128 records.
    pub fn new(limit: usize) -> Result<Self, String> {
        //1413635b0 registers limits 4.0 (40800000) and 128.0 (43000000).
        if !(4..=CAPACITY).contains(&limit) {
            return Err("dynamic decal limit must be between 4 and 128".into());
        }
        Ok(Self {
            slots: std::array::from_fn(|_| DecalSlot {
                description: None,
                renderer: None,
                registration: u32::MAX,
                registered: false,
                owner: None,
            }),
            available: (0..limit as u32).collect(),
            alive: Vec::with_capacity(limit),
            hash: SurfaceHashIndex::with_capacity(CAPACITY as u32, CAPACITY as u32, 0)?,
            limit,
        })
    }

    fn key(owner: NonZeroU64) -> u32 {
        (owner.get() >> 6) as u32
    }

    ///1bfde0 takes free[0] with swap removal, or evicts the oldest live record.
    ///2189d0's notification clears the previous FX owner's registration fields.
    pub fn insert(
        &mut self,
        owner: NonZeroU64,
        description: D,
        mut evict: impl FnMut(NonZeroU64),
    ) -> Result<u32, String> {
        let index = if self.available.is_empty() {
            let index = self.alive.remove(0);
            if let Some(previous) = self.slots[index as usize].owner {
                evict(previous);
                self.hash.remove(Self::key(previous), index)?;
            }
            index
        } else {
            self.available.swap_remove(0)
        };
        self.alive.push(index);
        let slot = &mut self.slots[index as usize];
        slot.description = Some(description);
        slot.owner = Some(owner);
        //1bfde0 deliberately preserves the renderer, registration and flag.
        self.hash.insert(Self::key(owner), index as i32)?;
        Ok(index)
    }

    ///1c0450 traverses the owner's hash bucket, including exact owner matching.
    ///Newest duplicate owners win because 00ad90 prepends their hash entries.
    pub fn find(&self, owner: NonZeroU64) -> Result<Option<u32>, String> {
        Ok(self
            .hash
            .bucket_chain(Self::key(owner))?
            .into_iter()
            .find(|&i| self.slots[i as usize].owner == Some(owner)))
    }

    pub fn slot(&self, index: u32) -> Option<&DecalSlot<D, R>> {
        self.slots.get(index as usize)
    }
    pub fn slot_mut(&mut self, index: u32) -> Option<&mut DecalSlot<D, R>> {
        self.slots.get_mut(index as usize)
    }
    pub fn alive(&self) -> &[u32] {
        &self.alive
    }
    pub fn limit(&self) -> usize {
        self.limit
    }

    ///1c0530 checks both owner and index, so an evicted FX cannot remove a
    ///replacement that reused its slot. Unregistration precedes resource drop.
    pub fn remove(
        &mut self,
        index: u32,
        owner: NonZeroU64,
        mut unregister: impl FnMut(&mut R),
    ) -> Result<bool, String> {
        if self.find(owner)? != Some(index) {
            return Ok(false);
        }
        if let Some(at) = self.alive.iter().position(|&i| i == index) {
            self.alive.remove(at);
        }
        let slot = &mut self.slots[index as usize];
        if let Some(renderer) = &mut slot.renderer {
            unregister(renderer);
        }
        slot.renderer = None;
        slot.owner = None;
        slot.registered = false;
        self.hash.remove(Self::key(owner), index)?;
        self.available.push(index);
        Ok(true)
    }

    ///1c06d0: optionally detach world entries before destroying committed
    ///objects. Unlike eviction, clearing does not notify FX owners via2189d0.
    pub fn clear(
        &mut self,
        new_limit: Option<usize>,
        remove_from_world: bool,
        mut unregister: impl FnMut(&mut R),
    ) -> Result<(), String> {
        if new_limit.is_some_and(|limit| !(4..=CAPACITY).contains(&limit)) {
            return Err("dynamic decal limit must be between 4 and 128".into());
        }
        for slot in &mut self.slots[..self.limit] {
            if remove_from_world {
                if let Some(renderer) = &mut slot.renderer {
                    unregister(renderer);
                }
            }
            if slot.owner.is_some() {
                slot.renderer = None;
                slot.owner = None;
                slot.registered = false;
            }
        }
        self.hash.clear();
        self.alive.clear();
        if let Some(limit) = new_limit {
            self.limit = limit;
        }
        self.available.clear();
        self.available.extend(0..self.limit as u32);
        Ok(())
    }

    ///1c00d0 delays a changed configured limit until every live owner is gone.
    pub fn reconfigure_when_empty(
        &mut self,
        limit: usize,
        unregister: impl FnMut(&mut R),
    ) -> Result<bool, String> {
        if limit != self.limit && self.available.len() == self.limit {
            self.clear(Some(limit), true, unregister)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
