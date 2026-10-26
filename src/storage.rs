//! Packed count/capacity storage shared by rendering and component ownership.
const COUNT_MASK: u32 = 0x00ff_ffff;

pub(crate) fn growth_capacity(capacity_word: u32, grow_one_at_a_time: bool) -> u32 {
    let capacity = capacity_word >> 8;
    let low_flags = capacity_word as u8;
    if grow_one_at_a_time {
        capacity.wrapping_add(1)
    } else if low_flags & 0x3f == 0x3f {
        if capacity == 0 {
            1
        } else if capacity < 0x4000_0000 {
            capacity * 2
        } else {
            0x7fff_ffff
        }
    } else {
        let step = 1u32 << (low_flags & 0x1f);
        let sum = capacity.wrapping_add(step);
        sum.wrapping_sub((step - 1) & sum)
    }
}

/// FUN_140339940's packed 24-bit count/capacity decision. Allocation/copy is
/// represented by `allocation_succeeds`; the native allocator and its memory
/// ownership are separate helpers and are not implemented here.
pub fn reserve_index(
    count_word: &mut u32,
    capacity_word: &mut u32,
    grow_one_at_a_time: bool,
    allocation_succeeds: bool,
) -> u32 {
    const COUNT_MASK: u32 = 0x00ff_ffff;
    let count = *count_word & COUNT_MASK;
    let capacity = *capacity_word >> 8;
    if count == capacity {
        let low_flags = *capacity_word as u8;
        let requested = growth_capacity(*capacity_word, grow_one_at_a_time);
        match low_flags & 0xc0 {
            0xc0 => {
                if allocation_succeeds {
                    *capacity_word = requested.wrapping_shl(8) | 0x3f;
                }
            }
            0 => {
                if !allocation_succeeds {
                    return u32::MAX;
                }
                *capacity_word = (*capacity_word & 0xff) | requested.wrapping_shl(8);
            }
            _ => return u32::MAX,
        }
    }
    let count = *count_word & COUNT_MASK;
    let capacity = *capacity_word >> 8;
    if count < capacity {
        *count_word = (*count_word & !COUNT_MASK) | (count + 1);
        count
    } else {
        capacity.wrapping_sub(1)
    }
}

/// FUN_140675c50: allocate and zero one native 8-byte pointer-list slot per capacity unit.
pub fn allocate_zeroed_pointers(capacity: u32) -> Result<Vec<u64>, String> {
    if capacity > 0x00ff_ffff {
        return Err("pointer capacity exceeds 24 bits".to_owned());
    }
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(capacity as usize)
        .map_err(|error| format!("pointer allocation failed: {error}"))?;
    slots.resize(capacity as usize, 0);
    Ok(slots)
}

/// FUN_1403a0000: allocate zeroed replacement slots, copy the live prefix,
/// and publish the native growth header. Caller supplies a growth capacity.
pub fn reallocate_pointers(
    slots: &mut Vec<u64>,
    live_count: u32,
    capacity_word: &mut u32,
    new_capacity: u32,
) -> Result<(), String> {
    if new_capacity > 0x00ff_ffff || live_count > new_capacity || live_count as usize > slots.len()
    {
        return Err("pointer reallocation metadata is out of range".to_owned());
    }
    let mut replacement = allocate_zeroed_pointers(new_capacity)?;
    replacement[..live_count as usize].copy_from_slice(&slots[..live_count as usize]);
    *slots = replacement;
    *capacity_word = new_capacity.wrapping_shl(8) | 0x3f;
    Ok(())
}

/// Packed pointer list used by FUN_140160b50/FUN_14016a4a0 and the related
/// FUN_14033a9e0/FUN_140646e80 path. The logical capacity dispatch is shared;
/// Rust Vec replaces engine allocator ownership and OOM side effects.
#[derive(Clone, Debug, Default)]
pub struct PointerList {
    slots: Vec<u64>,
    count_word: u32,
    capacity_word: u32,
}

impl PointerList {
    /// 75f060's empty owned-list constructor, including its packed count flags.
    pub fn owned() -> Self {
        Self::owned_with_flags(0x41)
    }

    /// The same owned list with a caller's source count-header flags. The
    /// expandable command queue's500c30 constructor uses0x05 instead of0x41.
    pub fn owned_with_flags(count_flags: u8) -> Self {
        Self {
            slots: Vec::new(),
            count_word: u32::from(count_flags) << 24,
            capacity_word: 0x3f,
        }
    }

    /// 60aee0 removes one live entry, preserving order and packed count flags.
    /// Its unused upper return-register bits have no logical Rust counterpart.
    pub fn remove_at(&mut self, index: i32) -> Result<bool, String> {
        if index < 0 || index as u32 >= self.count() {
            return Ok(false);
        }
        let count = self.count() as usize;
        if count > self.slots.len() {
            return Err("pointer list count exceeds storage".into());
        }
        self.slots
            .copy_within(index as usize + 1..count, index as usize);
        self.count_word = (self.count_word & !COUNT_MASK) | (count as u32 - 1);
        Ok(true)
    }

    /// 8918f0 appends in registration order. A denied allocation returns the
    /// source's invalid index; inconsistent host storage is reported separately.
    pub fn append(&mut self, value: u64, grow_one_at_a_time: bool) -> Result<u32, String> {
        let count = self.count();
        if count > self.capacity() || self.capacity() as usize != self.slots.len() {
            return Err("pointer list metadata disagrees with storage".into());
        }
        if count == self.capacity() {
            let requested = growth_capacity(self.capacity_word, grow_one_at_a_time);
            if requested > COUNT_MASK {
                return Err("pointer growth exceeds packed capacity".into());
            }
            if !self.set_capacity(requested as i32, true)? {
                return Ok(u32::MAX);
            }
        }
        if count < self.capacity() {
            let slot = self
                .slots
                .get_mut(count as usize)
                .ok_or("pointer allocation left a short backing array")?;
            *slot = value;
            self.count_word = (self.count_word & !COUNT_MASK) | (count + 1);
            Ok(count)
        } else {
            Ok(count.wrapping_sub(1))
        }
    }

    ///75f060 followed by16a4a0: preserve the owned list's count-header flags.
    pub fn owned_with_capacity(capacity: u32) -> Result<Self, String> {
        if capacity > COUNT_MASK {
            return Err("pointer capacity exceeds 24 bits".into());
        }
        let mut list = Self::owned();
        list.set_capacity(capacity as i32, true)?;
        Ok(list)
    }

    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        Ok(Self {
            slots: allocate_zeroed_pointers(capacity)?,
            count_word: 0,
            capacity_word: capacity.wrapping_shl(8) | u32::from(low_flags),
        })
    }

    /// FUN_14016a4a0/FUN_140646e80 packed-list capacity dispatch. Mode 3
    /// publishes the requested header even when its void allocator fails;
    /// `insert` detects the resulting storage/header mismatch before writing
    /// out of bounds. Allocator ownership/failure side effects differ by caller.
    pub fn set_capacity(
        &mut self,
        requested: i32,
        allocation_succeeds: bool,
    ) -> Result<bool, String> {
        if requested < 1 {
            if self.capacity_word as u8 & 0x40 == 0 {
                self.slots = Vec::new();
                self.capacity_word &= 0xff;
            }
            self.count_word &= 0xff00_0000;
            return Ok(true);
        }
        let requested = requested as u32;
        if requested > 0x00ff_ffff {
            return Err("pointer capacity exceeds 24 bits".to_owned());
        }
        let old_capacity = self.capacity_word >> 8;
        if requested == old_capacity {
            return Ok(true);
        }
        let allocation_mode = (self.capacity_word >> 6) & 3;
        if allocation_mode == 3 && old_capacity < requested {
            let count = self.count_word & COUNT_MASK;
            if count > old_capacity || count as usize > self.slots.len() {
                return Err("specialized pointer count exceeds storage".to_owned());
            }
            let flags = self.capacity_word as u8;
            if allocation_succeeds {
                let _ =
                    reallocate_pointers(&mut self.slots, count, &mut self.capacity_word, requested);
            }
            self.capacity_word = requested.wrapping_shl(8) | u32::from(flags);
            return Ok(true);
        }
        if allocation_mode != 0 {
            return Ok(false);
        }
        let count = self.count_word & COUNT_MASK;
        if !allocation_succeeds {
            self.slots = Vec::new();
            return Ok(false);
        }
        let mut replacement = match allocate_zeroed_pointers(requested) {
            Ok(replacement) => replacement,
            Err(_) => {
                self.slots = Vec::new();
                return Ok(false);
            }
        };
        let copied = count.min(requested) as usize;
        if copied > self.slots.len() {
            return Err("pointer count exceeds supplied storage".to_owned());
        }
        replacement[..copied].copy_from_slice(&self.slots[..copied]);
        self.slots = replacement;
        self.capacity_word = requested.wrapping_shl(8) | (self.capacity_word & 0xff);
        if count > requested {
            self.count_word = (self.count_word & 0xff00_0000) | requested;
        }
        Ok(true)
    }

    /// FUN_140160b50: grow when full, clamp the signed requested index, shift
    /// the packed live range right, increment its low-24 count, and write value.
    pub fn insert(
        &mut self,
        requested_index: u32,
        value: u64,
        grow_one_at_a_time: bool,
    ) -> Result<u32, String> {
        let old_count = self.count_word & COUNT_MASK;
        let old_capacity = self.capacity_word >> 8;
        if old_count > old_capacity || old_capacity as usize != self.slots.len() {
            return Err("pointer list metadata disagrees with storage".to_owned());
        }
        let index = if (requested_index as i32) < 0 {
            0
        } else {
            requested_index.min(old_count)
        };
        if old_count == old_capacity {
            let requested = growth_capacity(self.capacity_word, grow_one_at_a_time);
            if requested <= old_capacity || requested > 0x00ff_ffff {
                return Err("pointer growth exceeds packed capacity".to_owned());
            }
            if !self.set_capacity(requested as i32, true)? {
                return Err("pointer allocator rejected growth mode".to_owned());
            }
            if self.capacity() as usize != self.slots.len() {
                return Err("pointer allocation left a short backing array".to_owned());
            }
        }
        let appended = reserve_index(
            &mut self.count_word,
            &mut self.capacity_word,
            grow_one_at_a_time,
            true,
        );
        if appended != old_count {
            return Err("pointer count reservation failed".to_owned());
        }
        for destination in (index as usize..old_count as usize).rev() {
            self.slots[destination + 1] = self.slots[destination];
        }
        self.slots[index as usize] = value;
        Ok(index)
    }

    pub(crate) fn values_mut(&mut self) -> Result<&mut [u64], String> {
        self.slots
            .get_mut(..(self.count_word & COUNT_MASK) as usize)
            .ok_or("pointer list count exceeds its owned storage".into())
    }

    ///7e8a20 consumes a processed prefix, keeping entries appended by callbacks.
    /// A completely drained oversized owned buffer resets to the caller's floor.
    pub(crate) fn finish_prefix(&mut self, processed: u32, floor: u32) -> Result<bool, String> {
        let count = self.count();
        if processed > count {
            return Err("processed component prefix exceeds the queue".into());
        }
        if count == processed {
            let reset = self.capacity() > floor;
            if reset && self.capacity_word & 0x40 == 0 {
                self.slots.clear();
                self.slots.shrink_to_fit();
                self.capacity_word &= 0xff;
            }
            self.count_word &= !COUNT_MASK;
            if reset && self.capacity() < floor {
                self.set_capacity(floor as i32, true)?;
            }
            return Ok(reset);
        }
        self.slots
            .copy_within(processed as usize..count as usize, 0);
        self.count_word = (self.count_word & !COUNT_MASK) | (count - processed);
        Ok(false)
    }

    pub fn values(&self) -> &[u64] {
        self.slots
            .get(..(self.count_word & COUNT_MASK) as usize)
            .unwrap_or(&[])
    }

    pub fn count(&self) -> u32 {
        self.count_word & COUNT_MASK
    }
    pub fn capacity(&self) -> u32 {
        self.capacity_word >> 8
    }
    pub fn packed_header(&self) -> (u32, u32) {
        (self.count_word, self.capacity_word)
    }
}
