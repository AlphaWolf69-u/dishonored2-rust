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
}

pub struct OceanQueryQueue {
    pending: Vec<[f32; 2]>,
    records: Box<[QueryRecord; OCEAN_QUERY_CAPACITY]>,
    next_slot: usize,
    submitted_counts: [usize; 4],
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
        self.pending.clear();
        Ok(*count)
    }
    pub fn submitted_count(&self, ring: usize) -> Option<usize> {
        self.submitted_counts.get(ring).copied()
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
        if handle.is_invalid() {
            return Ok(None);
        }
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
                .ok_or("ocean query result storage is not available")?;
            destination.copy_from_slice(values);
        }
        self.records[slot] = QueryRecord::default();
        *handle = OceanQueryHandle::INVALID;
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

#[cfg(test)]
#[path = "queries/tests.rs"]
mod tests;
