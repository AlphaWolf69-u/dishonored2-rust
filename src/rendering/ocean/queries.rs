//! CPU query metadata from 300e20/3032b0/302840/3034a0/3034c0.
//! Renderer ring indices and completed GPU results must come from their owner.
pub const OCEAN_QUERY_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct OceanQueryHandle {
    pub slot: u16,
    pub ring: u16,
}
impl OceanQueryHandle {
    pub const INVALID: Self = Self {
        slot: u16::MAX,
        ring: u16::MAX,
    };
    pub fn is_invalid(self) -> bool {
        self == Self::INVALID
    }
}

#[derive(Clone, Copy, Default)]
struct QueryRecord {
    offset: u16,
    count: u16,
    // Host: Bevy game/render ticks are independent. A ring's previous
    // initialized tail is not the result of this registration.
    generation: u64,
}

pub struct OceanQueryQueue {
    pending: Vec<[f32; 2]>,
    records: Box<[QueryRecord; OCEAN_QUERY_CAPACITY]>,
    next_slot: usize,
    submitted_counts: [usize; 4],
    submitted_generations: [u64; 4],
    completed_generations: [u64; 4],
}
impl Default for OceanQueryQueue {
    fn default() -> Self {
        Self::new()
    }
}
impl OceanQueryQueue {
    /// 300e20 initializes all records, cursor, pending count and four counts to0.
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(OCEAN_QUERY_CAPACITY),
            records: Box::new([QueryRecord::default(); OCEAN_QUERY_CAPACITY]),
            next_slot: 0,
            submitted_counts: [0; 4],
            submitted_generations: [0; 4],
            completed_generations: [0; 4],
        }
    }

    /// Caller serializes registration through &mut/its existing queue lock.
    /// A non-fitting request returns the source's invalid handle unchanged state.
    pub fn register(
        &mut self,
        current_ring: usize,
        points: &[[f32; 2]],
    ) -> Result<OceanQueryHandle, String> {
        if current_ring >= 4 {
            return Err("ocean query ring is outside four slots".into());
        }
        if points.len() > OCEAN_QUERY_CAPACITY - self.pending.len() {
            return Ok(OceanQueryHandle::INVALID);
        }
        let original_slot = self.next_slot;
        let mut searched = 0;
        // The shipping code copies subsequent records into the ORIGINAL slot
        // while advancing its cursor. Preserve this unusual occupied-slot path;
        // replacing it with a conventional free-list changes returned results.
        while self.records[original_slot].count != 0 && searched < OCEAN_QUERY_CAPACITY {
            self.next_slot = (self.next_slot + 1) & 1023;
            self.records[original_slot] = self.records[self.next_slot];
            searched += 1;
        }
        if searched == OCEAN_QUERY_CAPACITY {
            return Err("ocean query table has no free handle slot".into());
        }
        self.records[original_slot] = QueryRecord {
            offset: self.pending.len() as u16,
            count: points.len() as u16,
            generation: self.submitted_generations[(current_ring + 1) & 3] + 1,
        };
        self.pending.extend_from_slice(points);
        let handle = OceanQueryHandle {
            slot: self.next_slot as u16,
            ring: ((current_ring + 1) & 3) as u16,
        };
        self.next_slot = (self.next_slot + 1) & 1023;
        Ok(handle)
    }

    pub fn pending_points(&self) -> &[[f32; 2]] {
        &self.pending
    }

    /// 3030f0 records this count even for an empty submission, then clears the
    /// pending count. This records issued commands, not CPU readback readiness.
    pub fn record_submission(&mut self, current_ring: usize) -> Result<usize, String> {
        let count = self
            .submitted_counts
            .get_mut(current_ring)
            .ok_or("ocean query ring is outside four slots")?;
        *count = self.pending.len();
        self.submitted_generations[current_ring] += 1;
        self.pending.clear();
        Ok(*count)
    }
    pub fn submitted_count(&self, ring: usize) -> Option<usize> {
        self.submitted_counts.get(ring).copied()
    }

    /// Called only after the selected submission has actually been mapped
    /// and copied into its canonical CPU ring (304c40 host handoff).
    pub fn mark_completed(&mut self, ring: usize) -> Result<(), String> {
        let generation = *self.submitted_generations.get(ring).ok_or("ocean completion ring is out of range")?;
        self.completed_generations[ring] = generation;
        Ok(())
    }

    pub fn is_ready(&self, handle: OceanQueryHandle) -> Result<bool, String> {
        if handle.is_invalid() { return Ok(true); }
        let record = self.records.get(usize::from(handle.slot)).ok_or("ocean query handle slot is out of range")?;
        let completed = *self.completed_generations.get(usize::from(handle.ring)).ok_or("ocean query handle ring is out of range")?;
        if record.count == 0 { return Ok(true); }
        if completed > record.generation {
            return Err(format!("ocean query result expired before consumption: slot={} ring={} expected={} completed={completed}", handle.slot, handle.ring, record.generation));
        }
        Ok(completed == record.generation)
    }

    /// 302840 copies packed height/normal float4s and consumes the handle. The
    /// source does not test GPU readiness here: provide actual completed rings.
    /// Rust bounds errors prevent reading uninitialized or out-of-range storage.
    pub fn take(
        &mut self,
        handle: &mut OceanQueryHandle,
        completed: [&[[f32; 4]]; 4],
        output: &mut [[f32; 4]],
    ) -> Result<Option<usize>, String> {
        let count = self.peek(*handle, completed, output)?;
        if count.is_some() { self.release(handle)?; }
        Ok(count)
    }

    /// Read a completed CPU ring without consuming its handle. The renderer
    /// uses this only to publish the value that gameplay will consume next.
    pub fn peek(
        &self,
        handle: OceanQueryHandle,
        completed: [&[[f32; 4]]; 4],
        output: &mut [[f32; 4]],
    ) -> Result<Option<usize>, String> {
        if handle.is_invalid() { return Ok(None); }
        if !self.is_ready(handle)? { return Ok(None); }
        let slot = usize::from(handle.slot);
        let record = *self
            .records
            .get(slot)
            .ok_or("ocean query handle slot is out of range")?;
        let count = usize::from(record.count);
        let ring = completed
            .get(usize::from(handle.ring))
            .ok_or("ocean query handle ring is out of range")?;
        let destination = output
            .get_mut(..count)
            .ok_or("ocean query output is smaller than its request")?;
        if count > 0 {
            let offset = usize::from(record.offset);
            let values = ring
                .get(offset..offset + count)
                .ok_or_else(|| format!("completed ocean query storage is inconsistent: slot={} ring={} generation={} offset={offset} count={count} available={}",
                    handle.slot, handle.ring, record.generation, ring.len()))?;
            destination.copy_from_slice(values);
        }
        Ok(Some(count))
    }

    pub fn release(&mut self, handle: &mut OceanQueryHandle) -> Result<(), String> {
        if !handle.is_invalid() {
            *self
                .records
                .get_mut(usize::from(handle.slot))
                .ok_or("ocean query handle slot is out of range")? = QueryRecord::default();
            *handle = OceanQueryHandle::INVALID;
        }
        Ok(())
    }
    /// 3034a0 clears handles only, retaining cursor, pending points and counts.
    pub fn reset_handles(&mut self) {
        self.records.fill(QueryRecord::default());
    }
}

/// The game's single ocean query manager (DAT_1432eb1b8) as its clients
/// share it: the queue, CPU copies of the four completed readback rings,
/// the ring new registrations join, and whether an ocean is active
/// (140302d20). The renderer submits and reads it back once per frame.
#[derive(Default)]
pub struct SharedOceanQueries {
    pub queue: OceanQueryQueue,
    pub completed: [Vec<[f32; 4]>; 4],
    pub register_ring: usize,
    pub active: bool,
}

pub type OceanQueryLink = std::sync::Arc<std::sync::Mutex<SharedOceanQueries>>;

/// The process's one manager, as the game's global (DAT_1432eb1b8).
pub fn ocean_query_manager() -> &'static OceanQueryLink {
    static MANAGER: std::sync::OnceLock<OceanQueryLink> = std::sync::OnceLock::new();
    MANAGER.get_or_init(Default::default)
}

/// A client's multi-point ocean query (a Havok particle stage's +0x6e8,
/// 14064e5d0/14064e640): one point per body slot, a float4 result
/// (height, normal) per point, and the handles of each of four frames.
pub struct OceanMultiQuery {
    pub positions: Vec<[f32; 2]>,
    pub results: Vec<[f32; 4]>,
    rings: [Vec<OceanQueryHandle>; 4],
    ring: usize,
}

impl OceanMultiQuery {
    /// 1401e5c00: positions at the origin, heights at -FLT_MAX.
    pub fn new(capacity: usize) -> Self {
        Self {
            positions: vec![[0.; 2]; capacity],
            results: vec![[-f32::MAX, 0., 0., 0.]; capacity],
            rings: Default::default(),
            ring: 0,
        }
    }

    /// 14064e640 with an active ocean: this ring's results from four frames
    /// ago fill the result slots in handle order (14064de80, 32 per
    /// handle), then every position is registered again in batches of 32
    /// (14064e040; a batch that does not fit keeps an invalid handle).
    pub fn exchange(&mut self, shared: &mut SharedOceanQueries) -> Result<(), String> {
        if self.positions.is_empty() || !shared.active {
            for ring in &mut self.rings {
                for handle in ring.iter_mut() { shared.queue.release(handle)?; }
                ring.clear();
            }
            return Ok(());
        }
        // Keep the whole source ring until its actual render submission is
        // available; clearing it on pending would leak live handle records.
        for &handle in &self.rings[self.ring] {
            if !shared.queue.is_ready(handle)? { return Ok(()); }
        }
        let SharedOceanQueries { queue, completed, register_ring, .. } = shared;
        let completed = completed.each_ref().map(Vec::as_slice);
        let mut offset = 0;
        for handle in &mut self.rings[self.ring] {
            if handle.is_invalid() {
                continue;
            }
            let output = self.results.get_mut(offset..).ok_or("ocean query results overflow")?;
            queue.take(handle, completed, output)?;
            offset += 32;
        }
        self.rings[self.ring].clear();
        for batch in self.positions.chunks(32) {
            let handle = queue.register(*register_ring, batch)?;
            self.rings[self.ring].push(handle);
        }
        self.ring = (self.ring + 1) & 3;
        Ok(())
    }
}

#[cfg(test)]
#[path = "queries/tests.rs"]
mod tests;
