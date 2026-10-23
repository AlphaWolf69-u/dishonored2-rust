//! Collision-query slot ownership (point submissions 094d90/08fc60), completion,
//! cancellation and ordered result transfer. Geometry execution is separate.
//! The exclusive owner linearizes the original pool/queue operations; it does
//! not implement the physics world's worker threads or shape algorithms.
use std::collections::VecDeque;

pub const QUERY_CAPACITY: usize = 0x1000;
const INDEX_MASK: u32 = 0x1fff;
const GENERATION_STEP: u32 = 0x2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryId(u32);
impl QueryId {
    pub const INVALID: Self = Self(u32::MAX);
    pub fn bits(self) -> u32 {
        self.0
    }
    pub fn is_valid(self) -> bool {
        self != Self::INVALID
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    OutdatedId,
    OutsideSubmissionPhase,
}
impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutdatedId => "collision query accessed with outdated or corrupted id",
            Self::OutsideSubmissionPhase => "collision query submitted outside its valid phase",
        })
    }
}
impl std::error::Error for QueryError {}

/// The source slot embeds one result. A second hit moves the whole list to an
/// owned allocation; retrieval transfers that allocation rather than copying it.
enum HitStorage<T: Copy> {
    Inline(Option<T>),
    Owned(Vec<T>),
}
impl<T: Copy> Default for HitStorage<T> {
    fn default() -> Self {
        Self::Inline(None)
    }
}

pub struct QueryHits<T: Copy> {
    storage: HitStorage<T>,
}
impl<T: Copy> Default for QueryHits<T> {
    fn default() -> Self {
        Self {
            storage: HitStorage::default(),
        }
    }
}
impl<T: Copy> QueryHits<T> {
    /// 098b20 appends without sorting, deduplication or changing query distance.
    pub fn push(&mut self, hit: T) {
        match &mut self.storage {
            HitStorage::Inline(None) => self.storage = HitStorage::Inline(Some(hit)),
            HitStorage::Inline(Some(first)) => self.storage = HitStorage::Owned(vec![*first, hit]),
            HitStorage::Owned(hits) => hits.push(hit),
        }
    }
    pub fn as_slice(&self) -> &[T] {
        match &self.storage {
            HitStorage::Inline(Some(hit)) => std::slice::from_ref(hit),
            HitStorage::Inline(None) => &[],
            HitStorage::Owned(hits) => hits,
        }
    }
    fn take_into(&mut self, output: &mut Vec<T>) {
        match std::mem::take(&mut self.storage) {
            HitStorage::Inline(hit) => {
                output.clear();
                output.extend(hit);
            }
            HitStorage::Owned(hits) => *output = hits,
        }
    }
}

struct Slot<Request, Hit: Copy> {
    id: QueryId,
    flags: u8,
    request: Option<Request>,
    hits: QueryHits<Hit>,
}

pub struct CollisionQueries<Request, Hit: Copy> {
    slots: Box<[Slot<Request, Hit>]>,
    next_free: Box<[u32]>,
    free_head: u32,
    pending: VecDeque<usize>,
    submission_allowed: bool,
}
impl<Request, Hit: Copy> Default for CollisionQueries<Request, Hit> {
    fn default() -> Self {
        // 08f940 initializes ids0..4095 and the ascending free chain ending4096.
        Self {
            slots: (0..QUERY_CAPACITY)
                .map(|i| Slot {
                    id: QueryId(i as u32),
                    flags: 0,
                    request: None,
                    hits: QueryHits::default(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            next_free: (1..=QUERY_CAPACITY as u32)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            free_head: 0,
            pending: VecDeque::with_capacity(QUERY_CAPACITY),
            submission_allowed: true,
        }
    }
}
impl<Request, Hit: Copy> CollisionQueries<Request, Hit> {
    pub fn set_submission_allowed(&mut self, allowed: bool) {
        self.submission_allowed = allowed;
    }
    pub fn submit(
        &mut self,
        request: Request,
        world_available: bool,
    ) -> Result<QueryId, QueryError> {
        if !world_available {
            return Ok(QueryId::INVALID);
        }
        let index = (self.free_head & INDEX_MASK) as usize;
        if index == QUERY_CAPACITY {
            return Ok(QueryId::INVALID);
        }
        // The source reports a fatal error here. Return an error without leaking
        // a slot when the host violates the same submission-phase contract.
        if !self.submission_allowed {
            return Err(QueryError::OutsideSubmissionPhase);
        }
        // 098650's linearized successful compare/exchange. Keep the ABA tag;
        // release0921a0 preserves it while pushing the freed index.
        self.free_head =
            (self.free_head & !INDEX_MASK).wrapping_add(GENERATION_STEP) | self.next_free[index];
        let slot = &mut self.slots[index];
        slot.id.0 = slot.id.0.wrapping_add(GENERATION_STEP);
        slot.flags &= !7;
        slot.request = Some(request);
        slot.hits = QueryHits::default();
        self.pending.push_back(index);
        Ok(slot.id)
    }
    fn index(&self, id: QueryId) -> Result<usize, QueryError> {
        let index = (id.0 & INDEX_MASK) as usize;
        self.slots
            .get(index)
            .filter(|slot| slot.id == id && slot.request.is_some())
            .map(|_| index)
            .ok_or(QueryError::OutdatedId)
    }
    fn release(&mut self, index: usize) {
        let slot = &mut self.slots[index];
        // 090ac0 invalidates the id again on destruction, before pool reuse.
        slot.id.0 = slot.id.0.wrapping_add(GENERATION_STEP);
        slot.request = None;
        slot.hits = QueryHits::default();
        self.next_free[index] = self.free_head & INDEX_MASK;
        self.free_head = (self.free_head & !INDEX_MASK) | index as u32;
    }
    /// 091a50: pending cancellation defers reclamation until the worker dequeues
    /// it. A completed query is released immediately. The caller loses its id.
    pub fn cancel(&mut self, id: &mut QueryId) -> Result<(), QueryError> {
        let index = self.index(*id)?;
        if self.slots[index].flags & 2 != 0 {
            self.release(index);
        } else {
            self.slots[index].flags |= 1;
        }
        *id = QueryId::INVALID;
        Ok(())
    }
    /// 0998c0/096610: FIFO work, skip cancelled jobs, mark completed only after
    /// their actual executor has returned. Appending hits leaves their order intact.
    pub fn resolve_pending(&mut self, mut execute: impl FnMut(&Request, &mut QueryHits<Hit>)) {
        while let Some(index) = self.pending.pop_front() {
            if self.slots[index].flags & 1 != 0 {
                self.release(index);
                continue;
            }
            let slot = &mut self.slots[index];
            execute(slot.request.as_ref().unwrap(), &mut slot.hits);
            slot.flags |= 2;
        }
    }
    /// 092630: pending queries leave both the caller's id and result buffer
    /// untouched. Completed queries consume the id and transfer ordered results.
    pub fn take_completed(
        &mut self,
        id: &mut QueryId,
        output: &mut Vec<Hit>,
    ) -> Result<bool, QueryError> {
        let index = self.index(*id)?;
        if self.slots[index].flags & 2 == 0 {
            return Ok(false);
        }
        self.slots[index].hits.take_into(output);
        self.release(index);
        *id = QueryId::INVALID;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "collision_queries/tests.rs"]
mod tests;
