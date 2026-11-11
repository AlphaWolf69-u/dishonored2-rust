//! hkfxFreelistBodyAllocator (vtable 141f1deb8): a free list over a fixed
//! body capacity. States: the next free index, -1 allocated, -2 being
//! allocated, -3 the end of the free list.
const ALLOCATED: i32 = -1;
const ALLOCATING: i32 = -2;
const END: i32 = -3;

pub struct BodyAllocator {
    /// +0x18, one entry per body (+0x20 capacity).
    states: Vec<i32>,
    /// +0x28/+0x30: the allocated indices in ascending order (140ff8230).
    allocated: Vec<i32>,
    /// +0x38.
    count: i32,
    /// +0x3c: allocated or released since the indices were rebuilt.
    dirty: bool,
    /// +0x40 free-list head, +0x44 the steal cursor.
    head: i32,
    cursor: i32,
}

impl BodyAllocator {
    /// 140ff8410 / 140ff85c0.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity as i32;
        let mut states: Vec<i32> = (1..=capacity).collect();
        if let Some(last) = states.last_mut() {
            *last = END;
        }
        Self { states, allocated: Vec::new(), count: 0, dirty: false, head: 0, cursor: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.states.len()
    }

    pub fn count(&self) -> i32 {
        self.count
    }

    pub fn is_allocated(&self, body: i32) -> bool {
        self.states.get(body as usize).is_some_and(|&state| state == ALLOCATED)
    }

    /// The allocated indices of the last rebuild. The original asserts that
    /// no allocation or release happened since (hkfxBodyAllocator.inl:0x1d).
    pub fn allocated(&self) -> Result<&[i32], String> {
        if self.dirty {
            return Err("Havok FX bodies changed since the allocated indices were rebuilt".into());
        }
        Ok(&self.allocated)
    }

    /// 140ff8440: up to `requested` bodies (at most the capacity). Once the
    /// free list is exhausted, `steal` takes allocated bodies round-robin from
    /// the cursor. Returns the bodies (free ones first) and the position of
    /// the first stolen one.
    pub fn allocate(&mut self, requested: usize, steal: bool) -> (Vec<i32>, Option<usize>) {
        let capacity = self.states.len() as i32;
        let count = requested.min(self.states.len());
        let mut bodies = Vec::with_capacity(count);
        let mut first_stolen = None;
        let mut popped = 0;
        for position in 0..count {
            let mut body = self.head;
            if body == END {
                if steal {
                    let mut cursor = self.cursor;
                    while self.states[cursor as usize] != ALLOCATED {
                        cursor = (cursor + 1) % capacity;
                        self.cursor = cursor;
                    }
                    body = self.cursor;
                    first_stolen.get_or_insert(position);
                    self.cursor = (body + 1) % capacity;
                }
            } else {
                popped += 1;
                self.head = self.states[body as usize];
                self.states[body as usize] = ALLOCATING;
            }
            bodies.push(body);
        }
        for &body in &bodies[..popped] {
            self.states[body as usize] = ALLOCATED;
        }
        self.count += popped as i32;
        self.dirty = true;
        if !steal {
            // Without stealing the exhausted positions hold the end marker;
            // the original reports allocated + stolen, which excludes them.
            bodies.truncate(popped);
        }
        (bodies, first_stolen)
    }

    /// 140ff8560: pushes each allocated body back onto the free list.
    pub fn release(&mut self, bodies: &[i32]) {
        let mut released = 0;
        for &body in bodies {
            if self.states[body as usize] == ALLOCATED {
                released += 1;
                self.states[body as usize] = self.head;
                self.head = body;
            }
        }
        self.count -= released;
        self.dirty = true;
    }

    /// 140ff8230.
    pub fn update_allocated(&mut self) {
        if self.dirty {
            self.allocated.clear();
            self.allocated.extend(
                self.states.iter().enumerate().filter(|(_, &state)| state == ALLOCATED).map(|(body, _)| body as i32),
            );
            self.dirty = false;
        }
    }
}
