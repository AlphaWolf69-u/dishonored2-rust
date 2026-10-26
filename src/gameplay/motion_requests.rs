//! Ordered actor velocity requests and expiry (74fb40/753980/752060).
//! Actor construction supplies the verified empty-manager defaults.

#[derive(Clone, Copy, Debug)]
pub struct MotionRequestInput {
    pub velocity: [f32; 3],
    pub priority: i32,
    pub remaining: f32,
    /// Source0 persists,2 lasts one evaluation; other nonzero values count down.
    pub lifetime: i32,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionRequest {
    pub input: MotionRequestInput,
    pub id: u32,
}

/// Used by the general list and the player's existing one-frame projection.
pub fn takes_precedence(new_priority: i32, old_priority: i32) -> bool {
    new_priority >= old_priority
}

#[derive(Clone, Debug)]
pub struct MotionRequestList {
    entries: Vec<MotionRequest>,
    pub timed_count: i32,
    pub next_id: u32,
    /// 753980 compares and stores the first ID as integer words (CMP/MOV).
    pub cached_id: u32,
    pub enabled: bool,
}

impl Default for MotionRequestList {
    /// 74f5e0/74f530: empty, disabled, no timed requests, unset IDs. The first
    /// insertion wraps the ID to zero; starting at zero would skip that ID.
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            timed_count: 0,
            next_id: u32::MAX,
            cached_id: u32::MAX,
            enabled: false,
        }
    }
}

impl MotionRequestList {
    pub fn from_parts(
        entries: Vec<MotionRequest>,
        timed_count: i32,
        next_id: u32,
        cached_id: u32,
        enabled: bool,
    ) -> Result<Self, String> {
        if entries.len() > 10 {
            return Err("actor request list exceeds its ten-request limit".into());
        }
        Ok(Self {
            entries,
            timed_count,
            next_id,
            cached_id,
            enabled,
        })
    }
    pub fn entries(&self) -> &[MotionRequest] {
        &self.entries
    }
    pub fn selected(&self) -> Option<&MotionRequest> {
        self.entries.first()
    }
    pub fn has_enabled_motion(&self) -> bool {
        self.enabled && self.entries.iter().any(|entry| entry.input.remaining >= 0.)
    }
    /// 74fb40 sorts signed priorities, placing new requests before tied ones.
    /// Native ffffffff remains the return both on capacity rejection and ID wrap.
    pub fn push(&mut self, mut input: MotionRequestInput) -> u32 {
        if self.entries.len() == 10 {
            return u32::MAX;
        }
        self.next_id = self.next_id.wrapping_add(1);
        input.flags &= 3;
        if input.lifetime != 0 {
            self.timed_count = self.timed_count.wrapping_add(1);
        }
        let at = self
            .entries
            .iter()
            .position(|entry| takes_precedence(input.priority, entry.input.priority))
            .unwrap_or(self.entries.len());
        // 751430's meaningful insert/copy behavior is Vec::insert; allocation
        // headers, unused bytes and process allocator ownership stay in Rust.
        self.entries.insert(
            at,
            MotionRequest {
                input,
                id: self.next_id,
            },
        );
        self.next_id
    }
    /// Complete752060 boolean result. Timed-count ownership belongs to the
    /// caller (753980), not to this ordered removal primitive.
    pub fn remove_at(&mut self, index: i32) -> bool {
        if index < 0 || index as usize >= self.entries.len() {
            return false;
        }
        self.entries.remove(index as usize);
        true
    }
    /// 753980 iterates backwards, preserving relative order when entries expire.
    /// A request that crosses zero is removed on the next evaluation.
    pub fn advance(&mut self, delta: f32) -> bool {
        if self.timed_count > 0 {
            for index in (0..self.entries.len()).rev() {
                let entry = &mut self.entries[index].input;
                if entry.lifetime == 0 {
                    continue;
                }
                // COMISS(0, remaining)/JBE admits unordered NaNs as well.
                if !(entry.remaining < 0.) {
                    entry.remaining = if entry.lifetime == 2 {
                        -1.
                    } else {
                        crate::float::subtract(entry.remaining, delta)
                    };
                } else {
                    self.timed_count = self.timed_count.wrapping_sub(1);
                    self.remove_at(index as i32);
                }
            }
        }
        let id = self.entries.first().map_or(u32::MAX, |entry| entry.id);
        let changed = self.cached_id != id;
        self.cached_id = id;
        changed
    }
}

#[cfg(test)]
#[path = "motion_requests/tests.rs"]
mod tests;
#[cfg(test)]
pub(crate) use tests::snapshot as request_snapshot;
