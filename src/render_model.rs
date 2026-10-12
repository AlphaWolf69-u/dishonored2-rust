//! Original render-model flag and finite query helpers.
use crate::tome::NoDepthAabbQuery;

/// FUN_141374630 registers r_multiSamples="0" (no multisampled FBO).
/// FUN_140347410 copies the setting into renderer state. This default must
/// not inherit Bevy's four-sample default.
pub const ORIGINAL_DEFAULT_MULTI_SAMPLES: i32 = 0;

/// FUN_140346720: the physical texture sample count is at least one.
pub fn original_renderer_sample_count(configured: i32) -> u32 {
    configured.max(1) as u32
}

/// Bounded view of one native packed-count array used by FUN_140341fb0.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceKeyArrayView<'a, T> {
    pub values: &'a [T],
    pub packed_count: u32,
}

/// The nested pointer-key arrays referenced from a surface key at +0x18.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceNestedKeyView<'a> {
    pub record_words: SurfaceKeyArrayView<'a, [u32; 4]>,
    pub short_keys: SurfaceKeyArrayView<'a, i16>,
}

/// Key fields observed by FUN_14033a590 and FUN_14033f020. This is a safe view,
/// not a claim that the native object is laid out as this Rust struct.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSortKeyView<'a> {
    pub signed_keys: [i64; 3],
    pub nested: SurfaceNestedKeyView<'a>,
    pub unsigned_key: u64,
    pub primary: f32,
    pub flags: [u8; 2],
    pub secondary: f32,
}

const SURFACE_ARRAY_COUNT_MASK: u32 = 0x00ff_ffff;

fn packed_surface_prefix<'a, T>(array: &SurfaceKeyArrayView<'a, T>) -> Result<&'a [T], String> {
    let count = (array.packed_count & SURFACE_ARRAY_COUNT_MASK) as usize;
    array.values.get(..count).ok_or_else(|| {
        "surface sort key packed count exceeds its supplied data".to_owned()
    })
}

/// FUN_140341fb0: equality of the nested short-key and four-word arrays.
/// Native count metadata uses only its low 24 bits.
pub fn surface_nested_key_equal(
    left: &SurfaceNestedKeyView<'_>,
    right: &SurfaceNestedKeyView<'_>,
) -> Result<bool, String> {
    let left_shorts = left.short_keys.packed_count & SURFACE_ARRAY_COUNT_MASK;
    let right_shorts = right.short_keys.packed_count & SURFACE_ARRAY_COUNT_MASK;
    if left_shorts != right_shorts
        || packed_surface_prefix(&left.short_keys)? != packed_surface_prefix(&right.short_keys)?
    {
        return Ok(false);
    }
    let left_records = left.record_words.packed_count & SURFACE_ARRAY_COUNT_MASK;
    let right_records = right.record_words.packed_count & SURFACE_ARRAY_COUNT_MASK;
    if left_records != right_records
        || packed_surface_prefix(&left.record_words)? != packed_surface_prefix(&right.record_words)?
    {
        return Ok(false);
    }
    Ok(true)
}

fn surface_float_order(left: f32, right: f32) -> i32 {
    if left != right {
        if left < right { -1 } else { 1 }
    } else {
        0
    }
}

fn surface_signed_order(left: i64, right: i64) -> u32 {
    if left < right { u32::MAX } else if left > right { 1 } else { 0 }
}

fn surface_unsigned_order(left: u64, right: u64) -> u32 {
    if left < right { u32::MAX } else if left > right { 1 } else { 0 }
}

/// FUN_14033a590: native draw-surface tie comparator. Nonzero flag bytes sort
/// first; float comparisons preserve COMISS unordered behavior (NaN => +1).
pub fn surface_key_compare(
    left: &SurfaceSortKeyView<'_>,
    right: &SurfaceSortKeyView<'_>,
) -> Result<u32, String> {
    if left.flags[0] != right.flags[0] {
        return Ok(if left.flags[0] != 0 { u32::MAX } else { 1 });
    }
    let order = surface_float_order(left.primary, right.primary);
    if order != 0 { return Ok(order as u32); }
    if left.flags[1] != right.flags[1] {
        return Ok(if left.flags[1] != 0 { u32::MAX } else { 1 });
    }
    for (left, right) in left.signed_keys.into_iter().zip(right.signed_keys) {
        let order = surface_signed_order(left, right);
        if order != 0 { return Ok(order); }
    }
    let order = surface_unsigned_order(left.unsigned_key, right.unsigned_key);
    if order != 0 { return Ok(order); }
    Ok(if surface_nested_key_equal(&left.nested, &right.nested)? { 0 } else { 1 })
}

/// FUN_14033e520: rightmost exact-key search used by the native sorted list.
/// It advances on compare <= 0 and returns -1 unless the final row compares equal.
pub fn surface_key_find_rightmost_equal(
    entries: Option<&[SurfaceSortKeyView<'_>]>,
    count: i32,
    target: &SurfaceSortKeyView<'_>,
) -> Result<i32, String> {
    let Some(entries) = entries else { return Ok(-1); };
    if count == 0 { return Ok(-1); }
    if entries.is_empty() {
        return Err("native surface key search reads entry zero from an empty slice".to_owned());
    }
    if count > 0 && count as usize > entries.len() {
        return Err("surface sort count exceeds its supplied entries".to_owned());
    }
    if count < 0 {
        return Ok(if surface_key_compare(&entries[0], target)? == 0 { 0 } else { -1 });
    }
    let mut index = 0i32;
    let mut remaining = count;
    while remaining > 1 {
        let half = remaining >> 1;
        let order = surface_key_compare(&entries[(index + half) as usize], target)? as i32;
        if order <= 0 { index += half; }
        remaining -= half;
    }
    let order = surface_key_compare(&entries[index as usize], target)?;
    Ok(if order == 0 { index } else { -1 })
}

/// FUN_14033f0f0: lower-bound insertion index; equal keys stay after the new row.
pub fn surface_key_lower_bound(
    entries: Option<&[SurfaceSortKeyView<'_>]>,
    count: i32,
    target: &SurfaceSortKeyView<'_>,
) -> Result<i32, String> {
    if count < 1 { return Ok(0); }
    let Some(entries) = entries else {
        return Err("native surface lower-bound search received a null list".to_owned());
    };
    if count as usize > entries.len() {
        return Err("surface sort count exceeds its supplied entries".to_owned());
    }
    let mut index = 0i32;
    let mut remaining = count;
    loop {
        let half = remaining >> 1;
        let order = surface_key_compare(&entries[(index + half) as usize], target)? as i32;
        if order < 0 { index += half; }
        remaining -= half;
        if half == 0 { return Ok(index + i32::from(order < 0)); }
    }
}

/// FUN_14033ecd0 (`SurfaceCompare<1>` slot +0x18) returns zero for every
/// signed count and valid key pair. This models only its observable index;
/// native object-pointer reads at +0x74 are not reproduced here.
pub const fn surface_compare_1_index_result() -> u32 { 0 }

/// Safe Rust view of FUN_1403119a0's pointer-bank lookup. The original
/// caller supplies `param_2`/`param_3` and a signed selector; an enabled
/// override uses the machine-code selector expression exactly. This models
/// the address calculation and loaded qword, not the caller's record layout.
pub fn surface_lookup_3119a0(
    default_bank: &[u64],
    override_bank: &[u64],
    flags49: u8,
    param2: u32,
    param3: u8,
    param4: i8,
) -> Result<u64, String> {
    let index = if param4 != -1 && flags49 & 0x40 != 0 {
        let selector = match param4 as u8 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            selector => usize::from(3u8 + u8::from(3 < selector.wrapping_sub(3))),
        };
        selector
            .checked_mul(3)
            .and_then(|row| row.checked_add(param3 as usize))
            .and_then(|row| row.checked_mul(0x16))
            .and_then(|row| row.checked_add(param2 as usize))
            .ok_or_else(|| "static surface override-bank index overflow".to_owned())?
    } else {
        (param3 as usize)
            .checked_mul(0x16)
            .and_then(|row| row.checked_add(param2 as usize))
            .ok_or_else(|| "static surface default-bank index overflow".to_owned())?
    };
    let bank = if param4 != -1 && flags49 & 0x40 != 0 {
        override_bank
    } else {
        default_bank
    };
    bank.get(index)
        .copied()
        .ok_or_else(|| format!("static surface pointer-bank index {index} is out of bounds"))
}

/// The material/mesh inputs already loaded by FUN_140321860. Flags and
/// pointer banks are native values, not inferred from transparency names.
pub struct DepthAlphaSurface<'a> {
    pub flags49: u8,
    pub default_programs: &'a [u64],
    pub override_programs: &'a [u64],
    pub selector: i8,
    pub has_optional_mesh: bool,
    pub special_vertex_format: bool,
    pub alternate_program: u64,
    pub draw_state: [u32; 9],
}

#[derive(Debug, PartialEq, Eq)]
pub struct DepthAlphaDrawPlan {
    pub program: u64,
    pub draw_state: [u32; 9],
    pub stencil_reference: u8,
    pub marks_alpha: bool,
}

/// FUN_140321860's three-pass program/state selection (140321ef3..14032219c).
/// Reuses the original pointer-bank rule; a null selected program skips the
/// draw without changing the alpha marker. Submission, list ownership and
/// material flag production remain the caller's responsibility.
pub fn prepare_depth_alpha_draw(
    surface: &DepthAlphaSurface<'_>, pass: u8,
) -> Result<Option<DepthAlphaDrawPlan>, String> {
    if pass > 2 { return Err("native depth-alpha pass must be 0, 1 or 2".to_owned()); }
    let alpha = surface.flags49 & 1 != 0;
    let program = if pass == 2 {
        if !alpha { return Ok(None); }
        surface.alternate_program
    } else {
        let slot = if pass == 0 && alpha { 4 } else { 6 }
            + u32::from(surface.special_vertex_format);
        surface_lookup_3119a0(surface.default_programs, surface.override_programs,
            surface.flags49, slot, u8::from(surface.has_optional_mesh), surface.selector)?
    };
    if program == 0 { return Ok(None); }
    let mut state = surface.draw_state;
    state[0] &= 0xfe1f_ffff;
    state[1] &= 0xfe1f_ffff;
    state[4] = state[4] & 0xff | 0xe80e_8000;
    state[5] = if pass == 2 { state[5] & 0xff80_80ef | 0x0080_8028 }
               else { state[5] & 0xff02_02ff | 0x0002_0228 };
    let marks_alpha = pass == 0 && alpha;
    if marks_alpha { state[0] = surface.draw_state[0] & 0xfe1f_fe23 | 0x0040_0023; }
    Ok(Some(DepthAlphaDrawPlan {program, draw_state: state,
        stencil_reference: if pass == 2 {0x80} else {2}, marks_alpha}))
}

fn surface_candidate_growth_capacity(capacity_word: u32, grow_one_at_a_time: bool) -> u32 {
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
pub fn reserve_surface_candidate_row_index(
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
        let requested = surface_candidate_growth_capacity(*capacity_word, grow_one_at_a_time);
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

/// One zero-initialized 0xa8-byte row from FUN_140339de0. Bytes not written
/// by its initializer stay deterministically zero here; whole-row equivalence
/// awaits confirming that downstream consumers ignore those gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSurfaceCandidateRow {
    pub bytes: [u8; 0xa8],
}

pub fn native_surface_candidate_row_default() -> NativeSurfaceCandidateRow {
    let mut bytes = [0; 0xa8];
    bytes[1] = 0xff;
    bytes[0x6c..0x70].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[0x74..0x7c].copy_from_slice(&0x7f7f_ffff_u64.to_le_bytes());
    bytes[0x94..0x98].copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
    NativeSurfaceCandidateRow { bytes }
}

impl Default for NativeSurfaceCandidateRow {
    fn default() -> Self {
        native_surface_candidate_row_default()
    }
}

/// Rust-owned 0xa8 surface candidate rows with the native packed count and
/// capacity header. Rust Vec supplies aligned storage and copy-on-growth; the
/// source allocator's debug tag/pool and uninitialized padding are separate.
/// FUN_14033a270/FUN_14033ad50 storage replacement: allocate defaults at the
/// requested size, copy only the live prefix, then release the old Vec. A
/// failed reserve leaves the original storage untouched, like the native call.
pub fn reallocate_surface_candidate_rows(
    rows: &mut Vec<NativeSurfaceCandidateRow>,
    live_count: u32,
    requested_capacity: u32,
) -> Result<(), String> {
    if requested_capacity > 0x00ff_ffff {
        return Err("native surface candidate capacity exceeds 24 bits".to_owned());
    }
    let copy_count = (live_count as usize).min(requested_capacity as usize);
    if copy_count > rows.len() {
        return Err("native surface live count exceeds supplied rows".to_owned());
    }
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(requested_capacity as usize)
        .map_err(|error| format!("native surface row allocation failed: {error}"))?;
    replacement.resize(requested_capacity as usize, NativeSurfaceCandidateRow::default());
    replacement[..copy_count].copy_from_slice(&rows[..copy_count]);
    *rows = replacement;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct NativeSurfaceCandidateRows {
    rows: Vec<NativeSurfaceCandidateRow>,
    count_word: u32,
    capacity_word: u32,
}

impl NativeSurfaceCandidateRows {
    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        if capacity > 0x00ff_ffff {
            return Err("native surface candidate capacity exceeds 24 bits".to_owned());
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(capacity as usize)
            .map_err(|error| format!("native surface row allocation failed: {error}"))?;
        rows.resize(capacity as usize, NativeSurfaceCandidateRow::default());
        Ok(Self {
            rows,
            count_word: 0,
            capacity_word: (capacity << 8) | u32::from(low_flags),
        })
    }

    pub fn append(
        &mut self,
        row: NativeSurfaceCandidateRow,
        grow_one_at_a_time: bool,
    ) -> Result<u32, String> {
        const COUNT_MASK: u32 = 0x00ff_ffff;
        let count = self.count_word & COUNT_MASK;
        let capacity = self.capacity_word >> 8;
        let mut allocation_succeeds = true;
        if count == capacity {
            let requested = surface_candidate_growth_capacity(
                self.capacity_word,
                grow_one_at_a_time,
            );
            if requested as usize > self.rows.len() {
                if reallocate_surface_candidate_rows(&mut self.rows, count, requested).is_err() {
                    allocation_succeeds = false;
                }
            }
        }
        let index = reserve_surface_candidate_row_index(
            &mut self.count_word,
            &mut self.capacity_word,
            grow_one_at_a_time,
            allocation_succeeds,
        );
        if index == u32::MAX {
            return Err("native surface candidate append failed".to_owned());
        }
        let slot = self
            .rows
            .get_mut(index as usize)
            .ok_or_else(|| "native surface candidate index exceeds allocated rows".to_owned())?;
        *slot = row;
        Ok(index)
    }

    pub fn rows(&self) -> &[NativeSurfaceCandidateRow] {
        &self.rows
    }

    pub fn count(&self) -> u32 {
        self.count_word & 0x00ff_ffff
    }

    pub fn capacity(&self) -> u32 {
        self.capacity_word >> 8
    }

    pub fn packed_header(&self) -> (u32, u32) {
        (self.count_word, self.capacity_word)
    }
}

/// FUN_140675c50: allocate and zero one native 8-byte pointer-list slot per capacity unit.
pub fn allocate_zeroed_native_surface_pointers(capacity: u32) -> Result<Vec<u64>, String> {
    if capacity > 0x00ff_ffff {
        return Err("native surface pointer capacity exceeds 24 bits".to_owned());
    }
    let mut slots = Vec::new();
    slots.try_reserve_exact(capacity as usize)
        .map_err(|error| format!("native surface pointer allocation failed: {error}"))?;
    slots.resize(capacity as usize, 0);
    Ok(slots)
}

/// FUN_1403a0000: allocate zeroed replacement slots, copy the live prefix,
/// and publish the native growth header. Caller supplies a growth capacity.
pub fn reallocate_native_surface_pointers(
    slots: &mut Vec<u64>,
    live_count: u32,
    capacity_word: &mut u32,
    new_capacity: u32,
) -> Result<(), String> {
    if new_capacity > 0x00ff_ffff || live_count > new_capacity || live_count as usize > slots.len() {
        return Err("native surface pointer reallocation metadata is out of range".to_owned());
    }
    let mut replacement = allocate_zeroed_native_surface_pointers(new_capacity)?;
    replacement[..live_count as usize].copy_from_slice(&slots[..live_count as usize]);
    *slots = replacement;
    *capacity_word = new_capacity.wrapping_shl(8) | 0x3f;
    Ok(())
}

/// Packed pointer list used by FUN_140160b50/FUN_14016a4a0 and the related
/// FUN_14033a9e0/FUN_140646e80 path. The logical capacity dispatch is shared;
/// Rust Vec replaces engine allocator ownership and OOM side effects.
#[derive(Clone, Debug, Default)]
pub struct NativeSurfacePointerList {
    slots: Vec<u64>,
    count_word: u32,
    capacity_word: u32,
}

impl NativeSurfacePointerList {
    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        Ok(Self {
            slots: allocate_zeroed_native_surface_pointers(capacity)?,
            count_word: 0,
            capacity_word: capacity.wrapping_shl(8) | u32::from(low_flags),
        })
    }

    /// FUN_14016a4a0/FUN_140646e80 packed-list capacity dispatch. Mode 3
    /// publishes the requested header even when its void allocator fails;
    /// `insert` detects the resulting storage/header mismatch before writing
    /// out of bounds. Allocator ownership/failure side effects differ by caller.
    pub fn set_capacity(&mut self, requested: i32, allocation_succeeds: bool) -> Result<bool, String> {
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
            return Err("native surface pointer capacity exceeds 24 bits".to_owned());
        }
        let old_capacity = self.capacity_word >> 8;
        if requested == old_capacity { return Ok(true); }
        let allocation_mode = (self.capacity_word >> 6) & 3;
        if allocation_mode == 3 && old_capacity < requested {
            let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
            if count > old_capacity || count as usize > self.slots.len() {
                return Err("native specialized surface pointer count exceeds storage".to_owned());
            }
            let flags = self.capacity_word as u8;
            if allocation_succeeds {
                let _ = reallocate_native_surface_pointers(
                    &mut self.slots,
                    count,
                    &mut self.capacity_word,
                    requested,
                );
            }
            self.capacity_word = requested.wrapping_shl(8) | u32::from(flags);
            return Ok(true);
        }
        if allocation_mode != 0 { return Ok(false); }
        let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        if !allocation_succeeds {
            self.slots = Vec::new();
            return Ok(false);
        }
        let mut replacement = match allocate_zeroed_native_surface_pointers(requested) {
            Ok(replacement) => replacement,
            Err(_) => {
                self.slots = Vec::new();
                return Ok(false);
            }
        };
        let copied = count.min(requested) as usize;
        if copied > self.slots.len() {
            return Err("native surface pointer count exceeds supplied storage".to_owned());
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
        let old_count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        let old_capacity = self.capacity_word >> 8;
        if old_count > old_capacity || old_capacity as usize != self.slots.len() {
            return Err("native surface pointer list metadata disagrees with storage".to_owned());
        }
        let index = if (requested_index as i32) < 0 { 0 } else { requested_index.min(old_count) };
        if old_count == old_capacity {
            let requested = surface_candidate_growth_capacity(self.capacity_word, grow_one_at_a_time);
            if requested <= old_capacity || requested > 0x00ff_ffff {
                return Err("native surface pointer growth exceeds packed capacity".to_owned());
            }
            if !self.set_capacity(requested as i32, true)? {
                return Err("native surface pointer allocator rejected growth mode".to_owned());
            }
            if self.capacity() as usize != self.slots.len() {
                return Err("native surface pointer allocation left a short backing array".to_owned());
            }
        }
        let appended = reserve_surface_candidate_row_index(
            &mut self.count_word,
            &mut self.capacity_word,
            grow_one_at_a_time,
            true,
        );
        if appended != old_count {
            return Err("native surface pointer count reservation failed".to_owned());
        }
        for destination in (index as usize..old_count as usize).rev() {
            self.slots[destination + 1] = self.slots[destination];
        }
        self.slots[index as usize] = value;
        Ok(index)
    }

    pub fn values(&self) -> &[u64] {
        self.slots.get(..(self.count_word & SURFACE_ARRAY_COUNT_MASK) as usize).unwrap_or(&[])
    }

    pub fn count(&self) -> u32 { self.count_word & SURFACE_ARRAY_COUNT_MASK }
    pub fn capacity(&self) -> u32 { self.capacity_word >> 8 }
    pub fn packed_header(&self) -> (u32, u32) { (self.count_word, self.capacity_word) }
}

/// FUN_140335360 stores 0x38-byte rows in this packed list. Fourteen raw
/// u32 lanes preserve the row bytes without guessing their mixed field types.
pub type NativeSurfaceBatchRow = [u32; 14];

/// FUN_1401d9090 returns the first qword of its native row pointer.
pub fn first_qword_of_native_surface_batch_row(row: &NativeSurfaceBatchRow) -> u64 {
    u64::from(row[0]) | (u64::from(row[1]) << 32)
}

fn allocate_zeroed_native_surface_rows(capacity: u32) -> Result<Vec<NativeSurfaceBatchRow>, String> {
    if capacity > 0x00ff_ffff {
        return Err("native surface row capacity exceeds 24 bits".to_owned());
    }
    let mut rows = Vec::new();
    rows.try_reserve_exact(capacity as usize)
        .map_err(|error| format!("native surface row allocation failed: {error}"))?;
    rows.resize(capacity as usize, [0; 14]);
    Ok(rows)
}

/// Packed 0x38-byte batch rows emitted by FUN_140335360 through
/// FUN_14033dd60/FUN_14033ac90. This models logical row storage; engine
/// allocator tags/ownership remain outside the Rust Vec representation.
#[derive(Clone, Debug, Default)]
pub struct NativeSurfaceBatchRowList {
    rows: Vec<NativeSurfaceBatchRow>,
    count_word: u32,
    capacity_word: u32,
}

impl NativeSurfaceBatchRowList {
    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        Ok(Self {
            rows: allocate_zeroed_native_surface_rows(capacity)?,
            count_word: 0,
            capacity_word: capacity.wrapping_shl(8) | u32::from(low_flags),
        })
    }

    /// FUN_14033dd60 dispatches packed row-list capacity changes. Its mode-3
    /// helper has no failure return and the caller publishes capacity anyway.
    pub fn set_capacity(&mut self, requested: i32, allocation_succeeds: bool) -> Result<bool, String> {
        if requested < 1 {
            if self.capacity_word as u8 & 0x40 == 0 {
                self.rows = Vec::new();
                self.capacity_word &= 0xff;
            }
            self.count_word &= 0xff00_0000;
            return Ok(true);
        }
        let requested = requested as u32;
        if requested > 0x00ff_ffff {
            return Err("native surface row capacity exceeds 24 bits".to_owned());
        }
        let old_capacity = self.capacity_word >> 8;
        if requested == old_capacity { return Ok(true); }
        let allocation_mode = (self.capacity_word >> 6) & 3;
        if allocation_mode == 3 && old_capacity < requested {
            let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
            if count > old_capacity || count as usize > self.rows.len() {
                return Err("native specialized surface row count exceeds storage".to_owned());
            }
            let flags = self.capacity_word as u8;
            if allocation_succeeds {
                if let Ok(mut replacement) = allocate_zeroed_native_surface_rows(requested) {
                    replacement[..count as usize].copy_from_slice(&self.rows[..count as usize]);
                    self.rows = replacement;
                }
            }
            self.capacity_word = requested.wrapping_shl(8) | u32::from(flags);
            return Ok(true);
        }
        if allocation_mode != 0 { return Ok(false); }
        let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        if !allocation_succeeds {
            self.rows = Vec::new();
            return Ok(false);
        }
        let mut replacement = match allocate_zeroed_native_surface_rows(requested) {
            Ok(replacement) => replacement,
            Err(_) => {
                self.rows = Vec::new();
                return Ok(false);
            }
        };
        let copied = count.min(requested) as usize;
        if copied > self.rows.len() {
            return Err("native surface row count exceeds supplied storage".to_owned());
        }
        replacement[..copied].copy_from_slice(&self.rows[..copied]);
        self.rows = replacement;
        self.capacity_word = requested.wrapping_shl(8) | (self.capacity_word & 0xff);
        if count > requested {
            self.count_word = (self.count_word & 0xff00_0000) | requested;
        }
        Ok(true)
    }

    /// Reserve and zero one row, growing with the native packed capacity rule.
    pub fn push_zeroed_row(&mut self, grow_one_at_a_time: bool, allocation_succeeds: bool) -> Result<u32, String> {
        let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        let capacity = self.capacity_word >> 8;
        if count > capacity || capacity as usize != self.rows.len() {
            return Err("native surface row-list metadata disagrees with storage".to_owned());
        }
        if count == capacity {
            let requested = surface_candidate_growth_capacity(self.capacity_word, grow_one_at_a_time);
            if requested <= capacity || requested > 0x00ff_ffff {
                return Err("native surface row growth exceeds packed capacity".to_owned());
            }
            if !self.set_capacity(requested as i32, allocation_succeeds)? {
                return Err("native surface row allocator rejected growth mode".to_owned());
            }
            if self.capacity_word >> 8 != self.rows.len() as u32 {
                return Err("native surface row allocation left a short backing array".to_owned());
            }
        }
        let index = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        self.rows[index as usize] = [0; 14];
        self.count_word = (self.count_word & 0xff00_0000) | (index + 1);
        Ok(index)
    }

    pub fn rows(&self) -> &[NativeSurfaceBatchRow] {
        self.rows.get(..(self.count_word & SURFACE_ARRAY_COUNT_MASK) as usize).unwrap_or(&[])
    }

    pub fn row_mut(&mut self, index: u32) -> Option<&mut NativeSurfaceBatchRow> {
        if index >= (self.count_word & SURFACE_ARRAY_COUNT_MASK) { return None; }
        self.rows.get_mut(index as usize)
    }

    pub fn first_qword(&self, index: u32) -> Option<u64> {
        self.rows().get(index as usize).map(first_qword_of_native_surface_batch_row)
    }

    pub fn count(&self) -> u32 { self.count_word & SURFACE_ARRAY_COUNT_MASK }
    pub fn capacity(&self) -> u32 { self.capacity_word >> 8 }
    pub fn packed_header(&self) -> (u32, u32) { (self.count_word, self.capacity_word) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSurfaceSortedInsert {
    Existing { index: u32 },
    Inserted { index: u32 },
}

/// Compose FUN_14033e520, FUN_14033f0f0 and FUN_14033a9e0: suppress an
/// existing equal surface key; otherwise insert its pointer at the native
/// lower bound while keeping the key and pointer arrays aligned.
pub fn insert_native_sorted_surface_pointer<'a>(
    sorted_keys: &mut Vec<SurfaceSortKeyView<'a>>,
    pointers: &mut NativeSurfacePointerList,
    target: SurfaceSortKeyView<'a>,
    pointer: u64,
    grow_one_at_a_time: bool,
) -> Result<NativeSurfaceSortedInsert, String> {
    if pointers.count() as usize != sorted_keys.len() {
        return Err("native surface key and pointer lists have different counts".to_owned());
    }
    let count = i32::try_from(sorted_keys.len())
        .map_err(|_| "native surface sorted list exceeds signed count".to_owned())?;
    let existing = surface_key_find_rightmost_equal(Some(sorted_keys), count, &target)?;
    if existing >= 0 {
        return Ok(NativeSurfaceSortedInsert::Existing { index: existing as u32 });
    }
    let index = surface_key_lower_bound(Some(sorted_keys), count, &target)? as usize;
    sorted_keys.try_reserve(1)
        .map_err(|error| format!("native surface key allocation failed: {error}"))?;
    pointers.insert(index as u32, pointer, grow_one_at_a_time)?;
    sorted_keys.insert(index, target);
    Ok(NativeSurfaceSortedInsert::Inserted { index: index as u32 })
}

/// HashIndex storage used by FUN_14000ad90. The native arrays use 0xffffffff
/// as the empty-chain sentinel; this safe view models logical bucket links,
/// not the engine allocator's tag/pool or owner lifetime.
#[derive(Clone, Debug)]
pub struct NativeSurfaceHashIndex {
    bucket_count: u32,
    chain_capacity: u32,
    growth_granularity: u32,
    bucket_mask: u32,
    cached_state_word: u32,
    bucket_heads: Vec<u32>,
    next_indices: Vec<u32>,
}

impl NativeSurfaceHashIndex {
    /// Construct the native reset state: both pointers refer to the sentinel.
    pub fn lazy(bucket_count: u32, chain_capacity: u32, growth_granularity: u32) -> Result<Self, String> {
        if bucket_count == 0 || bucket_count > i32::MAX as u32 || chain_capacity > i32::MAX as u32 {
            return Err("native surface HashIndex capacities are out of range".to_owned());
        }
        Ok(Self {
            bucket_count,
            chain_capacity,
            growth_granularity,
            bucket_mask: bucket_count - 1,
            cached_state_word: 0,
            bucket_heads: Vec::new(),
            next_indices: Vec::new(),
        })
    }

    pub fn with_capacity(bucket_count: u32, chain_capacity: u32, growth_granularity: u32) -> Result<Self, String> {
        let mut result = Self::lazy(bucket_count, chain_capacity, growth_granularity)?;
        result.reset(bucket_count, chain_capacity)?;
        Ok(result)
    }

    /// FUN_1400fe8b0 reset state: the sizes/mask are retained, arrays become
    /// the shared 0xffffffff sentinel, and the cached state word is cleared.
    pub fn clear(&mut self) {
        self.bucket_heads.clear();
        self.next_indices.clear();
        self.cached_state_word = 0;
    }

    /// FUN_1400fe7a0: replace both native arrays, fill them with 0xffffffff,
    /// and derive the bucket mask from hash capacity.
    pub fn reset(&mut self, bucket_count: u32, chain_capacity: u32) -> Result<(), String> {
        if bucket_count == 0 || bucket_count > i32::MAX as u32 || chain_capacity > i32::MAX as u32 {
            return Err("native surface HashIndex capacities are out of range".to_owned());
        }
        let mut buckets = Vec::new();
        buckets.try_reserve_exact(bucket_count as usize)
            .map_err(|error| format!("native surface hash allocation failed: {error}"))?;
        buckets.resize(bucket_count as usize, u32::MAX);
        let mut next = Vec::new();
        next.try_reserve_exact(chain_capacity as usize)
            .map_err(|error| format!("native surface chain allocation failed: {error}"))?;
        next.resize(chain_capacity as usize, u32::MAX);
        self.clear();
        self.bucket_count = bucket_count;
        self.chain_capacity = chain_capacity;
        self.bucket_mask = bucket_count - 1;
        self.bucket_heads = buckets;
        self.next_indices = next;
        self.cached_state_word = u32::MAX;
        Ok(())
    }

    /// FUN_1400fe970: grow only the next-index array, preserving old links and
    /// filling new slots with the native -1 sentinel.
    fn grow_chain(&mut self, requested: u32) -> Result<(), String> {
        if requested <= self.chain_capacity { return Ok(()); }
        let capacity = if self.growth_granularity == 0 {
            if requested < 0x4000_0001 { requested.checked_next_power_of_two().unwrap_or(0x7fff_ffff) }
            else { 0x7fff_ffff }
        } else {
            let granularity = self.growth_granularity;
            let rounded = requested.checked_sub(1)
                .and_then(|value| value.checked_add(granularity))
                .ok_or_else(|| "native surface chain capacity overflow".to_owned())?;
            rounded - rounded % granularity
        };
        if capacity > i32::MAX as u32 {
            return Err("native surface chain capacity exceeds signed native range".to_owned());
        }
        self.next_indices.try_reserve_exact((capacity - self.chain_capacity) as usize)
            .map_err(|error| format!("native surface chain growth failed: {error}"))?;
        self.next_indices.resize(capacity as usize, u32::MAX);
        self.chain_capacity = capacity;
        Ok(())
    }

    /// FUN_14000ad90: lazily reset sentinel storage, grow the chain array as
    /// needed, then prepend this index to the key's masked bucket chain.
    pub fn insert(&mut self, key: u32, index: i32) -> Result<(), String> {
        let index = u32::try_from(index)
            .map_err(|_| "native surface HashIndex insertion index is negative".to_owned())?;
        if self.bucket_heads.is_empty() {
            let requested = self.chain_capacity.max(index.checked_add(1)
                .ok_or_else(|| "native surface HashIndex index overflow".to_owned())?);
            self.reset(self.bucket_count, requested)?;
        } else if self.chain_capacity <= index {
            self.grow_chain(index.checked_add(1)
                .ok_or_else(|| "native surface HashIndex index overflow".to_owned())?)?;
        }
        let bucket = (key & self.bucket_mask) as usize;
        let head = self.bucket_heads.get_mut(bucket)
            .ok_or_else(|| "native surface HashIndex bucket exceeds storage".to_owned())?;
        let next = self.next_indices.get_mut(index as usize)
            .ok_or_else(|| "native surface HashIndex index exceeds storage".to_owned())?;
        *next = *head;
        *head = index;
        Ok(())
    }

    pub fn bucket_chain(&self, key: u32) -> Result<Vec<u32>, String> {
        if self.bucket_heads.is_empty() { return Ok(Vec::new()); }
        let bucket = (key & self.bucket_mask) as usize;
        let mut current = *self.bucket_heads.get(bucket)
            .ok_or_else(|| "native surface HashIndex bucket exceeds storage".to_owned())?;
        let mut result = Vec::new();
        while current != u32::MAX {
            if result.len() >= self.next_indices.len() {
                return Err("native surface HashIndex chain is cyclic".to_owned());
            }
            result.push(current);
            current = *self.next_indices.get(current as usize)
                .ok_or_else(|| "native surface HashIndex link exceeds storage".to_owned())?;
        }
        Ok(result)
    }

    pub fn bucket_heads(&self) -> &[u32] { &self.bucket_heads }
    pub fn next_indices(&self) -> &[u32] { &self.next_indices }
    pub fn bucket_mask(&self) -> u32 { self.bucket_mask }
    pub fn chain_capacity(&self) -> u32 { self.chain_capacity }
    pub fn cached_state_word(&self) -> u32 { self.cached_state_word }
}

/// FUN_140359bd0 input scalars after reading the original pointer graph. Field
/// names preserve observed offsets rather than asserting a native Rust layout.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeSurfaceCandidateSortFields {
    pub candidate_material_pointer: u64,
    pub owner_sort_value_1bb0: u64,
    pub surface_resource_value_00: u64,
    pub owner_pointer: u64,
    pub surface_resource_value_10: u64,
    pub sort_coverage_bits: u32,
    pub padding_32_33: [u8; 2],
    pub candidate_byte_38: i8,
    pub candidate_pointer_28: u64,
    pub surface_state_pointer_78: u64,
    pub surface_state_pointer_40: u64,
    pub surface_type_count: i32,
    pub owner_byte_1c31: u8,
}

/// FUN_140359bd0: create the seven-qword auxiliary sort payload copied into
/// candidate-row +0x40..+0x77. Padding bytes are zeroed; native stack padding
/// is unspecified and is not claimed to match.
pub fn native_surface_candidate_sort_payload(fields: &NativeSurfaceCandidateSortFields) -> [u8; 0x38] {
    let mut bytes = [0; 0x38];
    bytes[0x00..0x08].copy_from_slice(&fields.candidate_material_pointer.to_le_bytes());
    bytes[0x08..0x10].copy_from_slice(&fields.owner_sort_value_1bb0.to_le_bytes());
    bytes[0x10..0x18].copy_from_slice(&fields.surface_resource_value_00.to_le_bytes());
    bytes[0x18..0x20].copy_from_slice(&fields.owner_pointer.wrapping_add(0x1950).to_le_bytes());
    bytes[0x20..0x28].copy_from_slice(&fields.surface_resource_value_10.to_le_bytes());
    bytes[0x28..0x2c].copy_from_slice(&fields.sort_coverage_bits.to_le_bytes());
    bytes[0x2c..0x30].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[0x32..0x34].copy_from_slice(&fields.padding_32_33);
    let sort_flag = fields.candidate_byte_38 != 0
        && fields.candidate_pointer_28 != 0
        && fields.surface_state_pointer_78 == 0
        && fields.surface_state_pointer_40 == 0
        && fields.surface_type_count <= 1;
    bytes[0x30] = u8::from(sort_flag);
    bytes[0x31] = (fields.owner_byte_1c31 >> 1) & 1;
    bytes[0x34..0x38].copy_from_slice(&0x7f7f_ffff_u32.to_le_bytes());
    bytes
}

/// FUN_14033b350's observed 0xa8-byte candidate-row write branch.
#[derive(Clone, Copy, Debug)]
pub struct NativeSurfaceCandidateBranchFields {
    pub owner_byte_1c31: u8,
    pub global_byte_77b0: u8,
    pub context_pointer: u64,
    pub surface_container_pointer: u64,
    pub material_pointer: u64,
    pub selected_parameter_pointer: u64,
    pub override_parameter_pointer: u64,
    pub secondary_parameter_pointer: u64,
    /// f32 returned by FUN_1402c2ad0 -> FUN_1402db440 -> FUN_140342640.
    pub material_parameter_float: f32,
    pub auxiliary_payload: [u8; 0x38],
}

pub fn native_surface_candidate_branch_row(
    fields: &NativeSurfaceCandidateBranchFields,
) -> NativeSurfaceCandidateRow {
    let mut row = native_surface_candidate_row_default();
    row.bytes[0] = ((fields.owner_byte_1c31 >> 1) & 1) ^ fields.global_byte_77b0;
    row.bytes[1] = 0xff;
    row.bytes[0x08..0x10].copy_from_slice(&fields.context_pointer.to_le_bytes());
    row.bytes[0x10..0x18].copy_from_slice(&fields.surface_container_pointer.to_le_bytes());
    row.bytes[0x18..0x20].copy_from_slice(&fields.material_pointer.to_le_bytes());
    row.bytes[0x20..0x28].copy_from_slice(&fields.selected_parameter_pointer.to_le_bytes());
    row.bytes[0x28..0x30].copy_from_slice(&fields.override_parameter_pointer.to_le_bytes());
    row.bytes[0x30..0x38].copy_from_slice(&fields.secondary_parameter_pointer.to_le_bytes());
    row.bytes[0x38..0x40].fill(0);
    row.bytes[0x40..0x78].copy_from_slice(&fields.auxiliary_payload);
    row.bytes[0x78..0x7c].copy_from_slice(&fields.material_parameter_float.to_bits().to_le_bytes());
    row
}

/// FUN_14010a0d0's output for the default `param_4 == 0` clip constants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeAabbClipCullResult {
    pub outside_plane_mask: u8,
    pub output_byte: u8,
    pub culled: bool,
}

/// FUN_14010a0d0 with its default mode: transform the eight AABB corners and
/// accumulate the six initialized plane bits (1,2,4,8,16,32). The native
/// result is a cull flag: true if any plane has no corner in its half-space.
pub fn native_transformed_aabb_clip_cull(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> NativeAabbClipCullResult {
    let mut corners = [[0.0_f32; 4]; 8];
    for (index, corner) in corners.iter_mut().enumerate() {
        let x = if index & 1 == 0 { minimum[0] } else { maximum[0] };
        let y = if index & 2 == 0 { minimum[1] } else { maximum[1] };
        let z = if index & 4 == 0 { minimum[2] } else { maximum[2] };
        let xy0 = matrix[0] * x + matrix[3] + matrix[1] * y;
        let xy1 = matrix[4] * x + matrix[7] + matrix[5] * y;
        let xy2 = matrix[8] * x + matrix[11] + matrix[9] * y;
        let xy3 = matrix[12] * x + matrix[15] + matrix[13] * y;
        *corner = [
            matrix[2] * z + xy0,
            matrix[6] * z + xy1,
            matrix[10] * z + xy2,
            matrix[14] * z + xy3,
        ];
    }
    let plane_inside = [
        corners.iter().any(|corner| -corner[3] < corner[0]),
        corners.iter().any(|corner| corner[0] < corner[3]),
        corners.iter().any(|corner| -corner[3] < corner[1]),
        corners.iter().any(|corner| corner[1] < corner[3]),
        corners.iter().any(|corner| 0.0 < corner[2]),
        corners.iter().any(|corner| corner[2] < corner[3]),
    ];
    let mut inside_mask = 0_u8;
    for (bit, inside) in plane_inside.into_iter().enumerate() {
        if inside { inside_mask |= 1 << bit; }
    }
    let outside_plane_mask = inside_mask ^ 0x3f;
    NativeAabbClipCullResult {
        outside_plane_mask,
        output_byte: outside_plane_mask,
        culled: inside_mask != 0x3f,
    }
}

/// FUN_14033b280: return the selected input-surface indices and mode byte.
/// The count is the native packed header's low 24 bits; filtering preserves
/// original ascending order.
pub fn native_surface_candidate_iteration_indices(
    packed_count: u32,
    context_flags_18bc: u8,
    pass_filter: i32,
) -> Result<(i8, Vec<u32>), String> {
    let count = packed_count & 0x00ff_ffff;
    let mode = if count < 2 || context_flags_18bc & 0x10 != 0 { 0 } else { 1 };
    let mut indices = Vec::new();
    if pass_filter < 0 {
        indices.try_reserve_exact(count as usize)
            .map_err(|error| format!("native surface iteration allocation failed: {error}"))?;
        indices.extend(0..count);
    } else if (pass_filter as u32) < count {
        indices.push(pass_filter as u32);
    }
    Ok((mode, indices))
}

/// FUN_14033b280's allocation-free row traversal over its 0x80-byte
/// surfaceContainer records. `packed_count` is the native list header's +8
/// word; only its low 24 bits are the record count. The mode flag selects the
/// FUN_14033b350 branch but does not suppress row visits.
pub fn visit_native_surface_container_rows(
    rows: &[u8],
    packed_count: u32,
    context_flags_18bc: u8,
    pass_filter: i32,
    mut visit: impl FnMut(i8, u32, &[u8; 0x80]) -> Result<(), String>,
) -> Result<usize, String> {
    let count = packed_count & 0x00ff_ffff;
    let mode = if count < 2 || context_flags_18bc & 0x10 != 0 { 0 } else { 1 };
    let required = (count as usize)
        .checked_mul(0x80)
        .ok_or_else(|| "native surface container byte length overflow".to_owned())?;
    if required > rows.len() {
        return Err("native surface container is shorter than its packed record count".to_owned());
    }

    let mut visited = 0;
    for index in 0..count {
        if pass_filter >= 0 && pass_filter as u32 != index {
            continue;
        }
        let start = index as usize * 0x80;
        let row: &[u8; 0x80] = rows[start..start + 0x80].try_into().unwrap();
        visit(mode, index, row)?;
        visited += 1;
    }
    Ok(visited)
}

/// FUN_14033b350's optional DarkVision candidate branch gates. The category
/// is material+0x4a[7:5], produced from `rendering/darkvisionlayer`, not a
/// normal depth-pass or program ID. Pointer lookups and FUN_140109dd0 are
/// supplied as measured results; this helper does not select depth occluders.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeSurfaceCandidateBranchGates {
    pub material_pointer_present: bool,
    pub surface_pointer_30_present: bool,
    pub optional_pointer_78_value: Option<i32>,
    pub material_flags_4b: u8,
    pub mode: i8,
    pub surface_pointer_40_present: bool,
    pub helper_140109dd0_returns_nonzero: bool,
    pub initial_parameter_lookup_present: bool,
    pub surface_kind_pointer_present: bool,
    pub candidate_parameter_lookup_present: bool,
}

pub fn native_surface_candidate_category_key(
    gates: &NativeSurfaceCandidateBranchGates,
    material_flags_4a: u8,
    feature_bits: u32,
) -> Option<u32> {
    if !gates.material_pointer_present
        || !gates.surface_pointer_30_present
        || gates.optional_pointer_78_value.is_some_and(|value| value == 0)
        || gates.material_flags_4b & 0x0c == 4
        || (gates.mode != 0
            && !gates.surface_pointer_40_present
            && gates.helper_140109dd0_returns_nonzero)
        || !gates.initial_parameter_lookup_present
        || !gates.surface_kind_pointer_present
    {
        return None;
    }
    let category = material_flags_4a >> 5;
    if category == 0 || !gates.candidate_parameter_lookup_present {
        return None;
    }
    let enabled = feature_bits.wrapping_mul(2).wrapping_shr(u32::from(category)) & 1 != 0;
    enabled.then_some(u32::from(category) + 8)
}

/// Resolve FUN_14033b350's optional FUN_140109dd0 test using the exact default
/// clip mode, then apply the remaining observed material/category gates.
pub fn native_surface_candidate_category_key_with_bounds(
    gates: &NativeSurfaceCandidateBranchGates,
    material_flags_4a: u8,
    feature_bits: u32,
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> Option<u32> {
    let mut resolved = *gates;
    if resolved.mode != 0 && !resolved.surface_pointer_40_present {
        resolved.helper_140109dd0_returns_nonzero =
            native_transformed_aabb_clip_cull(matrix, minimum, maximum).culled;
    }
    native_surface_candidate_category_key(&resolved, material_flags_4a, feature_bits)
}

/// Inputs from one measured FUN_14033b350 surface branch. The caller must
/// supply source-backed fields; this does not infer runtime pointers or flags.
#[derive(Clone, Copy, Debug)]
pub struct NativeSurfaceCandidateInput {
    pub branch_fields: NativeSurfaceCandidateBranchFields,
    pub gates: NativeSurfaceCandidateBranchGates,
    pub material_flags_4a: u8,
    pub feature_bits: u32,
    pub clip_matrix: [f32; 16],
    pub bounds: [[f32; 3]; 2],
}

/// Port the observed candidate branch as one operation: apply eligibility,
/// write the 0xa8 row, append it, then prepend the row index to its category
/// HashIndex chain. Other FUN_14033b350 branches remain distinct.
pub fn append_native_surface_candidate_if_eligible(
    input: &NativeSurfaceCandidateInput,
    rows: &mut NativeSurfaceCandidateRows,
    hash_index: &mut NativeSurfaceHashIndex,
    grow_one_at_a_time: bool,
) -> Result<Option<u32>, String> {
    let Some(category_key) = native_surface_candidate_category_key_with_bounds(
        &input.gates,
        input.material_flags_4a,
        input.feature_bits,
        &input.clip_matrix,
        input.bounds[0],
        input.bounds[1],
    ) else {
        return Ok(None);
    };
    let row = native_surface_candidate_branch_row(&input.branch_fields);
    append_surface_candidate_to_hash(rows, hash_index, category_key, row, grow_one_at_a_time)
        .map(Some)
}

/// Append one candidate row, then insert its index into the native bucket
/// chain, matching FUN_14033b350's call order.
pub fn append_surface_candidate_to_hash(
    rows: &mut NativeSurfaceCandidateRows,
    hash_index: &mut NativeSurfaceHashIndex,
    category_key: u32,
    row: NativeSurfaceCandidateRow,
    grow_one_at_a_time: bool,
) -> Result<u32, String> {
    let index = rows.append(row, grow_one_at_a_time)?;
    hash_index.insert(category_key, index as i32)?;
    Ok(index)
}

/// FUN_140160b50's packed-vector insertion branch when count is below its
/// advertised capacity. Growth/allocation branches remain intentionally out
/// of this helper until their native allocator contract is ported.
pub fn insert_preallocated_surface_pointer(
    slots: &mut [u64],
    packed_count: &mut u32,
    capacity: u32,
    requested_index: i32,
    value: u64,
) -> Result<u32, String> {
    let count = *packed_count & SURFACE_ARRAY_COUNT_MASK;
    if count >= capacity {
        return Err("native surface pointer vector growth is required".to_owned());
    }
    if capacity as usize > slots.len() || count as usize >= slots.len() {
        return Err("native surface pointer vector metadata exceeds supplied storage".to_owned());
    }
    let index = if requested_index < 0 {
        0
    } else {
        (requested_index as u32).min(count)
    } as usize;
    for destination in (index..count as usize).rev() {
        slots[destination + 1] = slots[destination];
    }
    slots[index] = value;
    *packed_count = (*packed_count & !SURFACE_ARRAY_COUNT_MASK) | (count + 1);
    Ok(index as u32)
}

/// Safe view of one FUN_140335360 hash-chain node. The native node stores its
/// next pointer at +0 and the surface-container pointer at +8.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceHashChainNode {
    pub next: Option<usize>,
    pub surface_pointer: u64,
}

/// FUN_140335360's known HashIndex-chain branch: SurfaceCompare<1> returns
/// index zero, and FUN_140160b50 inserts each node's +8 pointer at the front.
/// This therefore reverses chain traversal and retains duplicates. The
/// branch-specific material selection, filtering, grouping, and allocator
/// growth paths remain separate.
pub fn build_prepass_hash_chain_pointer_list(
    nodes: &[SurfaceHashChainNode],
    head: Option<usize>,
    slots: &mut [u64],
    packed_count: &mut u32,
    capacity: u32,
) -> Result<(), String> {
    let mut current = head;
    let mut visited = 0usize;
    while let Some(index) = current {
        if visited >= nodes.len() {
            return Err("surface hash chain is cyclic".to_owned());
        }
        let node = nodes.get(index).ok_or_else(|| "surface hash node index is out of bounds".to_owned())?;
        let insertion_index = surface_compare_1_index_result() as i32;
        insert_preallocated_surface_pointer(
            slots,
            packed_count,
            capacity,
            insertion_index,
            node.surface_pointer,
        )?;
        current = node.next;
        visited += 1;
    }
    Ok(())
}

/// Full-growth version of FUN_140335360's HashIndex-chain branch. The native
/// caller uses FUN_140160b50/FUN_14016a4a0; this preserves front-insert order
/// and packed headers while Rust guards the native specialized-OOM overrun.
pub fn build_prepass_hash_chain_pointer_list_growing(
    nodes: &[SurfaceHashChainNode],
    head: Option<usize>,
    output: &mut NativeSurfacePointerList,
    grow_one_at_a_time: bool,
) -> Result<(), String> {
    let mut current = head;
    let mut visited = 0usize;
    while let Some(index) = current {
        if visited >= nodes.len() {
            return Err("surface hash chain is cyclic".to_owned());
        }
        let node = nodes.get(index).ok_or_else(|| "surface hash node index is out of bounds".to_owned())?;
        output.insert(surface_compare_1_index_result(), node.surface_pointer, grow_one_at_a_time)?;
        current = node.next;
        visited += 1;
    }
    Ok(())
}

/// Read all surface-container pointers in a HashIndex bucket, in the order
/// produced by the native front-insertion callback. HashIndex stores bucket
/// links here but no key comparison, so collisions remain in the same output;
/// exact-key filtering is a separate, not-yet-ported consumer operation.
pub fn native_prepass_surface_pointers_for_bucket(
    rows: &NativeSurfaceCandidateRows,
    hash_index: &NativeSurfaceHashIndex,
    bucket_key: u32,
) -> Result<Vec<u64>, String> {
    let chain = hash_index.bucket_chain(bucket_key)?;
    if chain.is_empty() {
        return Ok(Vec::new());
    }
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(chain.len())
        .map_err(|error| format!("native prepass chain allocation failed: {error}"))?;
    for (position, row_index) in chain.iter().copied().enumerate() {
        let row = rows.rows().get(row_index as usize)
            .ok_or_else(|| "native prepass hash index exceeds candidate rows".to_owned())?;
        let surface_pointer = u64::from_le_bytes(row.bytes[0x10..0x18].try_into().unwrap());
        nodes.push(SurfaceHashChainNode {
            next: (position + 1 < chain.len()).then_some(position + 1),
            surface_pointer,
        });
    }
    let capacity = u32::try_from(chain.len())
        .map_err(|_| "native prepass chain exceeds 32-bit count".to_owned())?;
    let mut slots = vec![0; chain.len()];
    let mut count = 0;
    build_prepass_hash_chain_pointer_list(&nodes, Some(0), &mut slots, &mut count, capacity)?;
    slots.truncate(count as usize);
    Ok(slots)
}

fn surface_key_compare_7(
    candidate: &SurfaceSortKeyView<'_>,
    target: &SurfaceSortKeyView<'_>,
) -> Result<u32, String> {
    let primary = surface_float_order(candidate.primary, target.primary);
    if primary != 0 { return Ok(primary as u32); }
    let secondary = surface_float_order(candidate.secondary, target.secondary);
    if secondary != 0 { return Ok(secondary as u32); }
    surface_key_compare(candidate, target)
}

/// FUN_14033f020: binary-search insertion point for the observed KeyCompare<7>.
/// `count` is its signed native count; malformed Rust views return an error
/// instead of reproducing the executable's out-of-bounds memory access.
pub fn surface_key_compare_7_insertion_index(
    entries: &[SurfaceSortKeyView<'_>],
    count: i32,
    target: &SurfaceSortKeyView<'_>,
) -> Result<u32, String> {
    if count < 1 { return Ok(0); }
    if count as usize > entries.len() {
        return Err("surface sort count exceeds its supplied entries".to_owned());
    }
    let mut low = 0i32;
    let mut remaining = count;
    loop {
        let half = remaining >> 1;
        let index = (low + half) as usize;
        let order = surface_key_compare_7(&entries[index], target)? as i32;
        let less = order < 0;
        if less { low += half; }
        remaining -= half;
        if half == 0 { return Ok((low + i32::from(less)) as u32); }
    }
}

/// FUN_1403386a0's ordered render-program selector, using observed raw offsets.
/// `state` must contain bytes through +0x2c7; IDs are preserved as `u32`.
pub fn static_surface_program_selector(
    state: &[u8],
    thresholds: &[f32; 10],
) -> Result<u32, String> {
    if state.len() < 0x2c8 {
        return Err("static surface state is shorter than offset +0x2c7".to_owned());
    }
    let resource = u64::from_le_bytes(state[0x100..0x108].try_into().unwrap());
    let flags48 = state[0x48];
    let flags4a = state[0x4a];
    let flags4b = state[0x4b];
    let value = f32::from_le_bytes(state[0x2c4..0x2c8].try_into().unwrap())
        + f32::from_le_bytes(state[0x2c0..0x2c4].try_into().unwrap());

    if resource != 0 && flags4a & 0x10 == 0 {
        return Ok(0x25);
    }
    if flags48 & 0x80 != 0 {
        return Ok(0x2a);
    }
    if flags4a & 0x10 != 0 {
        return Ok(0x36);
    }
    if (flags4b & 3).wrapping_sub(1) < 2 {
        return Ok(0x29);
    }
    if flags48 & 0x10 != 0 {
        return Ok(if matches!((flags4a >> 2) & 3, 0 | 3) {
            0x23
        } else {
            0x37
        });
    }
    if thresholds[0] > value {
        return Ok(if flags4b & 0x80 != 0 { 0x37 } else { 0x23 });
    }
    for (index, program) in [
        (1, 0x26), (2, 0x32), (3, 0x33), (5, 0x34),
        (4, 0x2d), (6, 0x37), (7, 0x35), (8, 0x38),
    ] {
        if thresholds[index] > value {
            return Ok(program);
        }
    }
    Ok(if thresholds[9] > value { 0x39 } else { 0x41 })
}

/// FUN_140338820's selector after its FUN_1402c2ac0 property-float lookup.
/// The caller supplies that f32 because the native property owner is not mapped.
pub fn parameterized_surface_program_selector(
    state: &[u8],
    value: f32,
    thresholds: &[f32; 10],
) -> Result<u32, String> {
    if state.len() < 0x108 {
        return Err("static surface state is shorter than offset +0x107".to_owned());
    }
    let flags48 = state[0x48];
    let flags4a = state[0x4a];
    if u64::from_le_bytes(state[0x100..0x108].try_into().unwrap()) != 0 && flags4a & 0x10 == 0 {
        return Ok(0x25);
    }
    if flags48 & 0x80 != 0 {
        return Ok(0x2a);
    }
    if flags48 & 0x10 != 0 {
        return Ok(if matches!((flags4a >> 2) & 3, 0 | 3) { 0x23 } else { 0x37 });
    }
    if value < thresholds[0] {
        return Ok(if state[0x4b] & 0x80 != 0 { 0x37 } else { 0x23 });
    }
    for (index, program) in [
        (1, 0x26), (2, 0x32), (3, 0x33), (5, 0x34),
        (4, 0x2d), (6, 0x37), (7, 0x35), (8, 0x38),
    ] {
        if value < thresholds[index] {
            return Ok(program);
        }
    }
    Ok(if value < thresholds[9] { 0x39 } else { 0x41 })
}

/// FUN_1402c2ac0 -> FUN_1402db420 prepares the lookup passed to FUN_140342640.
/// The global parameter ID is supplied by the caller; its native value is runtime-owned.
pub fn native_surface_parameter_float_lookup(
    parameter_object: &[u8],
    parameter_id: u64,
) -> Result<(u64, u64, u32), &'static str> {
    let owner = parameter_object.get(0x10..0x18)
        .ok_or("native parameter object is shorter than its owner pointer at +0x10")?;
    let property = u64::from_le_bytes(owner.try_into().unwrap()).wrapping_add(0x2d8);
    Ok((property, parameter_id, 0))
}

/// FUN_140319540 clears qwords 0 and 3 only when qword 0 is nonzero.
pub fn reset_native_depth_draw_cache_if_bound(cache: &mut [u64; 4]) {
    if cache[0] != 0 {
        cache[0] = 0;
        cache[3] = 0;
    }
}

fn native_indirect_pointer_plus(base: &[u8], addend: u64) -> Result<u64, &'static str> {
    let pointer = base.get(0x10..0x18)
        .ok_or("native object is shorter than its pointer at +0x10")?;
    let pointer = u64::from_le_bytes(pointer.try_into().unwrap());
    Ok(pointer.wrapping_add(addend))
}

/// FUN_1402c2ab0 loads a pointer at byte offset 0x10 and returns pointer + 0x58.
pub fn native_indirect_pointer_plus_0x58(base: &[u8]) -> Result<u64, &'static str> {
    native_indirect_pointer_plus(base, 0x58)
}

/// FUN_1402c2aa0 loads a pointer at byte offset 0x10 and returns pointer + 0x2d8.
pub fn native_indirect_pointer_plus_0x2d8(base: &[u8]) -> Result<u64, &'static str> {
    native_indirect_pointer_plus(base, 0x2d8)
}

/// FUN_1402bc340 passes its value as a one-qword array to FUN_1402bc370.
pub fn call_native_resource_array_bind_with_one(
    context: u64,
    value: u64,
    extra: u64,
    flags: u16,
    bind_many: impl FnOnce(u64, u32, &[u64; 1], u64, u16),
) {
    bind_many(context, 1, &[value], extra, flags);
}

/// FUN_1402fd8d0 reads its worker index at context +0xf8. Indices 0/1
/// invoke their no-argument workers; all others tail-dispatch the same context
/// to FUN_1402fe850. A short byte view is rejected instead of read out of bounds.
pub fn dispatch_native_depthpass_worker(
    context: &[u8],
    worker_0: impl FnOnce() -> u64,
    worker_1: impl FnOnce() -> u64,
    worker_other: impl FnOnce(&[u8]) -> u64,
) -> Result<u64, String> {
    let index = context.get(0xf8..0xfc)
        .ok_or_else(|| "native depth-pass context is shorter than its worker index".to_owned())?;
    let worker_index = i32::from_le_bytes(index.try_into().unwrap());
    Ok(match worker_index {
        0 => worker_0(),
        1 => worker_1(),
        _ => worker_other(context),
    })
}

/// Ports the early return in FUN_1402fdb60 when view+0x3a34 bit 0x10 is set:
/// mark renderer-state byte +0x12ad4 and return the original state pointer with
/// its low byte cleared. `None` means the worker continues through its main path.
pub fn native_depthpass_index0_early_exit(
    view: &[u8],
    renderer_state: &mut [u8],
    state_pointer: u64,
) -> Result<Option<u64>, &'static str> {
    let flags = *view.get(0x3a34)
        .ok_or("native view is shorter than its +0x3a34 flags byte")?;
    if flags & 0x10 == 0 {
        return Ok(None);
    }
    let completed = renderer_state.get_mut(0x12ad4)
        .ok_or("native renderer state is shorter than its +0x12ad4 completion byte")?;
    *completed = 1;
    Ok(Some(state_pointer & !0xff))
}

/// FUN_140109550 stages 64 source bytes, then writes four 16-byte chunks in
/// destination order. The writer callback preserves native pointer/alias order.
pub fn copy_native_64_to_four_destinations(
    source: &[u8; 64],
    mut write_destination: impl FnMut(usize, &[u8; 16]),
) {
    let staged = *source;
    for destination in 0..4 {
        let start = destination * 16;
        let chunk: &[u8; 16] = staged[start..start + 16].try_into().unwrap();
        write_destination(destination, chunk);
    }
}

/// FUN_1402ff110 reads four signed slot indices and copies a 64-byte payload
/// to `(index * 16) + 16` for each slot, without interpreting its contents.
pub fn write_native_depthpass_parameter_slots(
    context: &mut [u8],
    source: [u8; 64],
) -> Result<(), &'static str> {
    const FIRST_INDEX: usize = 0x123e8;
    let mut destinations = [0usize; 4];
    for (slot, destination) in destinations.iter_mut().enumerate() {
        let field = FIRST_INDEX + slot * 4;
        let bytes = context.get(field..field + 4)
            .ok_or("native depth-pass context lacks parameter slot indices")?;
        let index = i32::from_le_bytes(bytes.try_into().unwrap());
        let start = i64::from(index).checked_mul(16)
            .and_then(|offset| offset.checked_add(16))
            .ok_or("native depth-pass parameter slot offset overflow")?;
        let start = usize::try_from(start)
            .map_err(|_| "native depth-pass parameter slot index is negative")?;
        let end = start.checked_add(16)
            .ok_or("native depth-pass parameter slot range overflow")?;
        if end > context.len() {
            return Err("native depth-pass parameter slot exceeds context");
        }
        *destination = start;
    }
    copy_native_64_to_four_destinations(&source, |slot, chunk| {
        let start = destinations[slot];
        context[start..start + 16].copy_from_slice(chunk);
    });
    Ok(())
}

/// FUN_14033a810's ordered linked-chain list builder, not a depth sort.
/// `slots` models retained native storage; an empty record advances the
/// logical count without overwriting its old slot. Links use `id & mask`.
/// Inputs must be the original caller's valid, acyclic record/link arrays.
/// The render-graph producer/consumer remains to be integrated separately.
pub fn build_zprepass_list(
    slots: &mut Vec<Option<u32>>,
    logical_count: &mut usize,
    record_nonempty: &[bool],
    next: &[u32],
    mask: u32,
    head: u32,
    append: bool,
) {
    const END: u32 = u32::MAX;
    let prefix = if append { *logical_count } else { 0 };
    let mut id = head;
    let mut chain_count = 0;
    while id != END {
        chain_count += 1;
        id = next[(id & mask) as usize];
    }
    let requested = prefix + chain_count;
    if requested > 0x00ff_ffff {
        panic!("[sorted] zprepass renderlist is full");
    }
    if requested > slots.len() {
        slots.try_reserve_exact(requested - slots.len())
            .unwrap_or_else(|_| panic!("[sorted] zprepass renderlist is full"));
        slots.resize(requested, None);
    }
    *logical_count = requested;
    id = head;
    for slot in &mut slots[prefix..requested] {
        if record_nonempty[id as usize] {
            *slot = Some(id);
        }
        id = next[(id & mask) as usize];
    }
}

#[cfg(test)]
mod zprepass_list_tests {
    use super::{
        build_zprepass_list, copy_native_64_to_four_destinations,
        dispatch_native_depthpass_worker, native_depthpass_index0_early_exit,
        write_native_depthpass_parameter_slots,
    };

    #[test]
    fn native_depthpass_index0_flag_sets_completion_and_masks_return_pointer() {
        let mut view = vec![0; 0x3a35];
        view[0x3a34] = 0x10;
        let mut state = vec![0; 0x12ad5];
        let result = native_depthpass_index0_early_exit(
            &view, &mut state, 0x1234_5678_9abc_def7,
        ).unwrap();
        assert_eq!(result, Some(0x1234_5678_9abc_de00));
        assert_eq!(state[0x12ad4], 1);
    }

    #[test]
    fn native_depthpass_index0_without_flag_continues_without_writing_state() {
        let view = vec![0; 0x3a35];
        let mut state = vec![0; 0x12ad5];
        assert_eq!(native_depthpass_index0_early_exit(&view, &mut state, 0xff), Ok(None));
        assert_eq!(state[0x12ad4], 0);
        assert!(native_depthpass_index0_early_exit(&[], &mut state, 0).is_err());
        let mut short_state = vec![0; 0x12ad4];
        let mut flagged_view = vec![0; 0x3a35];
        flagged_view[0x3a34] = 0x10;
        assert!(native_depthpass_index0_early_exit(&flagged_view, &mut short_state, 0).is_err());
    }

    #[test]
    fn native_depthpass_payload_copy_preserves_bytes_and_destination_order() {
        let source: [u8; 64] = std::array::from_fn(|index| (index * 17) as u8);
        let mut chunks = [[0u8; 16]; 4];
        copy_native_64_to_four_destinations(&source, |index, chunk| chunks[index] = *chunk);
        for index in 0..4 {
            assert_eq!(chunks[index], source[index * 16..index * 16 + 16]);
        }

        let mut context = vec![0; 0x12400];
        for (slot, index) in [0i32, 1, 2, 3].into_iter().enumerate() {
            let field = 0x123e8 + slot * 4;
            context[field..field + 4].copy_from_slice(&index.to_le_bytes());
        }
        write_native_depthpass_parameter_slots(&mut context, source).unwrap();
        for index in 0..4 {
            let start = (index + 1) * 16;
            assert_eq!(&context[start..start + 16], &source[index * 16..index * 16 + 16]);
        }

        let mut aliased = vec![0; 0x123f8];
        for slot in 0..4 {
            let field = 0x123e8 + slot * 4;
            aliased[field..field + 4].copy_from_slice(&0i32.to_le_bytes());
        }
        write_native_depthpass_parameter_slots(&mut aliased, source).unwrap();
        assert_eq!(&aliased[16..32], &source[48..64]);
    }

    #[test]
    fn native_depthpass_dispatch_reads_index_and_routes_original_context() {
        let make_context = |index: i32| {
            let mut bytes = vec![0; 0xfc];
            bytes[0xf8..0xfc].copy_from_slice(&index.to_le_bytes());
            bytes
        };
        assert_eq!(dispatch_native_depthpass_worker(&make_context(0), || 10, || 11, |_| panic!("wrong worker")), Ok(10));
        assert_eq!(dispatch_native_depthpass_worker(&make_context(1), || 10, || 11, |_| panic!("wrong worker")), Ok(11));
        let context = make_context(2);
        let mut passed_same_context = false;
        assert_eq!(dispatch_native_depthpass_worker(&context, || 10, || 11, |other| { passed_same_context = std::ptr::eq(other, context.as_slice()); 12 }), Ok(12));
        assert!(passed_same_context);
        assert_eq!(dispatch_native_depthpass_worker(&make_context(-1), || 10, || 11, |_| 13), Ok(13));
        assert!(dispatch_native_depthpass_worker(&[0; 0xfb], || 10, || 11, |_| 13).is_err());
    }

    #[test]
    #[ignore = "requires local original executable capture from probe_original_zprepass_list.py"]
    fn matches_original_zprepass_list_cases() {
        let data = std::fs::read_to_string("analysis/native_zprepass_list_cases.txt").unwrap();
        let mut cases = 0;
        for line in data.lines() {
            let row: Vec<u32> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
            assert_eq!(row.len(), 116);
            let mut count = row[0] as usize;
            let live: Vec<bool> = row[3..35].iter().map(|&v| v != 0).collect();
            let mut slots: Vec<Option<u32>> = row[51..83].iter().copied().map(Some).collect();
            build_zprepass_list(&mut slots, &mut count, &live, &row[35..51], 15, row[2], row[1] != 0);
            assert_eq!(count, row[83] as usize, "case {cases}");
            assert_eq!(slots, row[84..116].iter().copied().map(Some).collect::<Vec<_>>(), "case {cases}");
            cases += 1;
        }
        assert_eq!(cases, 256);
    }

    fn surface_key(primary: f32, secondary: f32) -> super::SurfaceSortKeyView<'static> {
        const SHORTS: [i16; 0] = [];
        const RECORDS: [[u32; 4]; 0] = [];
        super::SurfaceSortKeyView {
            signed_keys: [0; 3],
            nested: super::SurfaceNestedKeyView {
                record_words: super::SurfaceKeyArrayView { values: &RECORDS, packed_count: 0 },
                short_keys: super::SurfaceKeyArrayView { values: &SHORTS, packed_count: 0 },
            },
            unsigned_key: 0,
            primary,
            flags: [0; 2],
            secondary,
        }
    }

    fn nested_surface_key<'a>(
        shorts: &'a [i16],
        short_count: u32,
        records: &'a [[u32; 4]],
        record_count: u32,
    ) -> super::SurfaceNestedKeyView<'a> {
        super::SurfaceNestedKeyView {
            short_keys: super::SurfaceKeyArrayView { values: shorts, packed_count: short_count },
            record_words: super::SurfaceKeyArrayView { values: records, packed_count: record_count },
        }
    }

    #[test]
    fn nested_surface_keys_compare_low_24_bit_lengths_and_exact_elements() {
        let left_short = [7i16, 9];
        let right_short = [7i16, 9];
        let left_records = [[1u32, 2, 3, 4]];
        let right_records = [[1u32, 2, 3, 4]];
        let left = nested_surface_key(&left_short, 0xaa00_0002, &left_records, 0xbb00_0001);
        let right = nested_surface_key(&right_short, 0x5500_0002, &right_records, 0x1100_0001);
        assert_eq!(super::surface_nested_key_equal(&left, &right), Ok(true));
        let different_length = nested_surface_key(&right_short, 0x5500_0001, &right_records, 0x1100_0001);
        assert_eq!(super::surface_nested_key_equal(&left, &different_length), Ok(false));
        let different_words = [[1u32, 2, 3, 5]];
        let different_record = nested_surface_key(&right_short, 0x5500_0002, &different_words, 0x1100_0001);
        assert_eq!(super::surface_nested_key_equal(&left, &different_record), Ok(false));
    }

    #[test]
    fn surface_key_comparator_preserves_native_field_precedence_and_nan() {
        let base = surface_key(1.0, 2.0);
        let mut other = base;
        other.flags[0] = 1;
        assert_eq!(super::surface_key_compare(&other, &base), Ok(u32::MAX));
        assert_eq!(super::surface_key_compare(&base, &other), Ok(1));
        other = base;
        other.primary = 0.0;
        other.flags[1] = 1;
        assert_eq!(super::surface_key_compare(&other, &base), Ok(u32::MAX));
        other = base;
        other.flags[1] = 1;
        assert_eq!(super::surface_key_compare(&base, &other), Ok(1));
        other = base;
        other.signed_keys[0] = -1;
        assert_eq!(super::surface_key_compare(&other, &base), Ok(u32::MAX));
        other = base;
        other.unsigned_key = u64::MAX;
        assert_eq!(super::surface_key_compare(&base, &other), Ok(u32::MAX));
        const DIFFERENT_RECORD: [[u32; 4]; 1] = [[4, 3, 2, 1]];
        other = base;
        other.nested.record_words = super::SurfaceKeyArrayView {
            values: &DIFFERENT_RECORD,
            packed_count: 1,
        };
        assert_eq!(super::surface_key_compare(&other, &base), Ok(1));
        other = base;
        other.primary = f32::NAN;
        assert_eq!(super::surface_key_compare(&other, &base), Ok(1));
        assert_eq!(super::surface_key_compare(&base, &other), Ok(1));
    }

    #[test]
    fn native_surface_key_search_and_lower_bound_preserve_duplicate_rules() {
        let entries = [surface_key(0.0, 0.0), surface_key(1.0, 0.0), surface_key(1.0, 2.0), surface_key(1.0, 3.0), surface_key(2.0, 0.0)];
        let zero = surface_key(0.0, 0.0);
        let one = surface_key(1.0, 0.0);
        let between = surface_key(1.5, 0.0);
        let two = surface_key(2.0, 0.0);
        let nan = surface_key(f32::NAN, 0.0);

        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 5, &zero), Ok(0));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 5, &one), Ok(3));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 5, &two), Ok(4));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 1, &zero), Ok(0));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 1, &one), Ok(-1));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 5, &between), Ok(-1));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 5, &nan), Ok(-1));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), 0, &one), Ok(-1));
        assert_eq!(super::surface_key_find_rightmost_equal(None, 5, &one), Ok(-1));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), -1, &zero), Ok(0));
        assert_eq!(super::surface_key_find_rightmost_equal(Some(&entries), -1, &one), Ok(-1));
        assert!(super::surface_key_find_rightmost_equal(Some(&entries), 6, &one).is_err());

        assert_eq!(super::surface_key_lower_bound(Some(&entries), 5, &zero), Ok(0));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 5, &one), Ok(1));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 5, &between), Ok(4));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 5, &two), Ok(4));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 1, &zero), Ok(0));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 1, &one), Ok(1));
        assert_eq!(super::surface_key_lower_bound(Some(&entries), 5, &nan), Ok(0));
        assert_eq!(super::surface_key_lower_bound(None, 0, &one), Ok(0));
        assert_eq!(super::surface_key_lower_bound(None, -1, &one), Ok(0));
        assert!(super::surface_key_lower_bound(None, 1, &one).is_err());
        assert!(super::surface_key_lower_bound(Some(&entries), 6, &one).is_err());
    }

    #[test]
    fn native_sorted_surface_insert_composes_search_lower_bound_and_pointer_growth() {
        let mut keys = Vec::new();
        let mut pointers = super::NativeSurfacePointerList::with_capacity(1, 0).unwrap();
        let key0 = surface_key(0.0, 0.0);
        let key1 = surface_key(1.0, 0.0);
        let key2 = surface_key(2.0, 0.0);
        let key_between = surface_key(1.5, 0.0);

        assert_eq!(super::insert_native_sorted_surface_pointer(&mut keys, &mut pointers, key0, 10, false), Ok(super::NativeSurfaceSortedInsert::Inserted { index: 0 }));
        assert_eq!(super::insert_native_sorted_surface_pointer(&mut keys, &mut pointers, key2, 20, false), Ok(super::NativeSurfaceSortedInsert::Inserted { index: 1 }));
        assert_eq!(super::insert_native_sorted_surface_pointer(&mut keys, &mut pointers, key2, 99, false), Ok(super::NativeSurfaceSortedInsert::Existing { index: 1 }));
        assert_eq!(super::insert_native_sorted_surface_pointer(&mut keys, &mut pointers, key_between, 15, false), Ok(super::NativeSurfaceSortedInsert::Inserted { index: 1 }));
        assert_eq!(super::insert_native_sorted_surface_pointer(&mut keys, &mut pointers, key1, 13, false), Ok(super::NativeSurfaceSortedInsert::Inserted { index: 1 }));

        assert_eq!(keys.iter().map(|key| key.primary).collect::<Vec<_>>(), [0.0, 1.0, 1.5, 2.0]);
        assert_eq!(pointers.values(), &[10, 13, 15, 20]);
        assert_eq!(pointers.count(), keys.len() as u32);
        assert!(pointers.capacity() >= pointers.count());
    }

    #[test]
    fn native_prepass_hash_chain_uses_full_pointer_growth_and_front_order() {
        let nodes = [
            super::SurfaceHashChainNode { next: Some(1), surface_pointer: 0x11 },
            super::SurfaceHashChainNode { next: None, surface_pointer: 0x22 },
        ];
        let mut output = super::NativeSurfacePointerList::with_capacity(0, 0).unwrap();
        super::build_prepass_hash_chain_pointer_list_growing(&nodes, Some(0), &mut output, true).unwrap();
        assert_eq!(output.values(), &[0x22, 0x11]);
        assert_eq!(output.count(), 2);
        assert_eq!(output.capacity(), 2);
        assert_eq!(output.packed_header().0 & 0xff00_0000, 0);
    }

    #[test]
    fn native_surface_iteration_matches_fun_14033b280_count_mode_and_filter() {
        assert_eq!(super::native_surface_candidate_iteration_indices(0xaa00_0003, 0, -1), Ok((1, vec![0, 1, 2])));
        assert_eq!(super::native_surface_candidate_iteration_indices(0xaa00_0003, 0x10, -1), Ok((0, vec![0, 1, 2])));
        assert_eq!(super::native_surface_candidate_iteration_indices(0xaa00_0003, 0, 1), Ok((1, vec![1])));
        assert_eq!(super::native_surface_candidate_iteration_indices(0xaa00_0003, 0, 3), Ok((1, vec![])));
        assert_eq!(super::native_surface_candidate_iteration_indices(0xaa00_0001, 0, -1), Ok((0, vec![0])));
    }

    #[test]
    fn surface_container_visits_native_stride_order_mode_and_filter() {
        let mut rows = vec![0; 3 * 0x80];
        for index in 0..3 {
            rows[index * 0x80] = index as u8;
            rows[(index + 1) * 0x80 - 1] = 0xa0 + index as u8;
        }
        for (packed_count, flags, filter) in [
            (0xaa00_0003, 0, -1),
            (0xaa00_0003, 0x10, -1),
            (0xaa00_0003, 0, 1),
            (0xaa00_0003, 0, 3),
            (0xaa00_0001, 0, -1),
            (0xaa00_0000, 0, -1),
        ] {
            let (mode, expected_indices) =
                super::native_surface_candidate_iteration_indices(packed_count, flags, filter)
                    .unwrap();
            let mut visited = Vec::new();
            let count = super::visit_native_surface_container_rows(
                &rows, packed_count, flags, filter,
                |actual_mode, index, row| {
                    assert_eq!(actual_mode, mode);
                    assert_eq!(row[0], index as u8);
                    assert_eq!(row[0x7f], 0xa0 + index as u8);
                    visited.push(index);
                    Ok(())
                },
            ).unwrap();
            assert_eq!(visited, expected_indices);
            assert_eq!(count, expected_indices.len());
        }
    }

    #[test]
    fn surface_container_rejects_truncation_before_visiting_and_propagates_error() {
        let mut visits = 0;
        assert!(super::visit_native_surface_container_rows(
            &[0; 0x7f], 1, 0, -1,
            |_, _, _| { visits += 1; Ok(()) },
        ).is_err());
        assert_eq!(visits, 0);
        let error = super::visit_native_surface_container_rows(
            &[0; 3 * 0x80], 3, 0, -1,
            |_, index, _| {
                visits += 1;
                if index == 1 { Err("callback error".to_owned()) } else { Ok(()) }
            },
        );
        assert_eq!(error, Err("callback error".to_owned()));
        assert_eq!(visits, 2);
    }

    #[test]
    fn transformed_aabb_clip_matches_original_plane_mask_and_cull_return() {
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [-0.5, -0.5, 0.1], [0.5, 0.5, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0, output_byte: 0, culled: false },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [-3.0, 0.0, 0.1], [-2.0, 1.0, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x01, output_byte: 0x01, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [2.0, 0.0, 0.1], [3.0, 1.0, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x02, output_byte: 0x02, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [0.0, -3.0, 0.1], [1.0, -2.0, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x04, output_byte: 0x04, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [0.0, 2.0, 0.1], [1.0, 3.0, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x08, output_byte: 0x08, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [-0.5, -0.5, -2.0], [0.5, 0.5, -1.0]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x10, output_byte: 0x10, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [-0.5, -0.5, 2.0], [0.5, 0.5, 3.0]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x20, output_byte: 0x20, culled: true },
        );
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [-1.0, -0.5, 0.1], [-1.0, 0.5, 0.9]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x01, output_byte: 0x01, culled: true },
        );
        let nan = f32::NAN;
        assert_eq!(
            super::native_transformed_aabb_clip_cull(&identity, [nan; 3], [nan; 3]),
            super::NativeAabbClipCullResult { outside_plane_mask: 0x3f, output_byte: 0x3f, culled: true },
        );
    }

    #[test]
    fn native_surface_candidate_category_key_matches_original_gates() {
        let mut gates = super::NativeSurfaceCandidateBranchGates {
            material_pointer_present: true,
            surface_pointer_30_present: true,
            optional_pointer_78_value: Some(1),
            material_flags_4b: 0,
            mode: 0,
            surface_pointer_40_present: false,
            helper_140109dd0_returns_nonzero: false,
            initial_parameter_lookup_present: true,
            surface_kind_pointer_present: true,
            candidate_parameter_lookup_present: true,
        };
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 1), Some(9));
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        gates.mode = 1;
        gates.helper_140109dd0_returns_nonzero = false;
        assert_eq!(super::native_surface_candidate_category_key_with_bounds(
            &gates, 0x20, 1, &identity, [-0.5, -0.5, 0.1], [0.5, 0.5, 0.9],
        ), Some(9));
        assert_eq!(super::native_surface_candidate_category_key_with_bounds(
            &gates, 0x20, 1, &identity, [-3.0, 0.0, 0.1], [-2.0, 1.0, 0.9],
        ), None);
        gates.mode = 0;
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0, u32::MAX), None);
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 0), None);
        gates.optional_pointer_78_value = Some(0);
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 1), None);
        gates.optional_pointer_78_value = None;
        gates.material_flags_4b = 4;
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 1), None);
        gates.material_flags_4b = 0;
        gates.mode = 1;
        gates.helper_140109dd0_returns_nonzero = true;
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 1), None);
        gates.surface_pointer_40_present = true;
        assert_eq!(super::native_surface_candidate_category_key(&gates, 0x20, 1), Some(9));
    }

    #[test]
    fn native_prepass_candidate_gate_row_hash_bucket_and_front_insert_compose() {
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let gates = super::NativeSurfaceCandidateBranchGates {
            material_pointer_present: true,
            surface_pointer_30_present: true,
            optional_pointer_78_value: Some(1),
            material_flags_4b: 0,
            mode: 0,
            surface_pointer_40_present: false,
            helper_140109dd0_returns_nonzero: false,
            initial_parameter_lookup_present: true,
            surface_kind_pointer_present: true,
            candidate_parameter_lookup_present: true,
        };
        let input = |pointer, material_flags_4a, feature_bits, accepted| {
            let mut input_gates = gates;
            input_gates.candidate_parameter_lookup_present = accepted;
            super::NativeSurfaceCandidateInput {
                branch_fields: super::NativeSurfaceCandidateBranchFields {
                    owner_byte_1c31: 0,
                    global_byte_77b0: 0,
                    context_pointer: 0x11,
                    surface_container_pointer: pointer,
                    material_pointer: 0x22,
                    selected_parameter_pointer: 0x33,
                    override_parameter_pointer: 0,
                    secondary_parameter_pointer: 0,
                    material_parameter_float: 0.0,
                    auxiliary_payload: [0; 0x38],
                },
                gates: input_gates,
                material_flags_4a,
                feature_bits,
                clip_matrix: identity,
                bounds: [[-0.5; 3], [0.5; 3]],
            }
        };
        let mut rows = super::NativeSurfaceCandidateRows::with_capacity(0, 0).unwrap();
        let mut hash = super::NativeSurfaceHashIndex::lazy(4, 0, 0).unwrap();
        assert_eq!(super::append_native_surface_candidate_if_eligible(
            &input(0x0a, 0x20, 1, true), &mut rows, &mut hash, false,
        ), Ok(Some(0)));
        assert_eq!(super::append_native_surface_candidate_if_eligible(
            &input(0x0b, 0x40, 2, true), &mut rows, &mut hash, false,
        ), Ok(Some(1)));
        assert_eq!(super::append_native_surface_candidate_if_eligible(
            &input(0x0c, 0x20, 1, true), &mut rows, &mut hash, false,
        ), Ok(Some(2)));
        assert_eq!(super::append_native_surface_candidate_if_eligible(
            &input(0x0d, 0x20, 1, false), &mut rows, &mut hash, false,
        ), Ok(None));
        assert_eq!(super::append_native_surface_candidate_if_eligible(
            &input(0x0e, 0xa0, 16, true), &mut rows, &mut hash, false,
        ), Ok(Some(3)));
        assert_eq!(rows.count(), 4);
        // Categories 9 and 13 collide in the four-bucket HashIndex. The
        // insertion callback reverses the bucket walk but does not compare keys.
        assert_eq!(super::native_prepass_surface_pointers_for_bucket(&rows, &hash, 9), Ok(vec![0x0a, 0x0c, 0x0e]));
        assert_eq!(super::native_prepass_surface_pointers_for_bucket(&rows, &hash, 10), Ok(vec![0x0b]));
        assert_eq!(super::native_prepass_surface_pointers_for_bucket(&rows, &hash, 11), Ok(Vec::new()));
    }

    #[test]
    fn native_surface_candidate_hash_index_matches_lazy_reset_growth_and_chain_order() {
        let mut rows = super::NativeSurfaceCandidateRows::with_capacity(1, 0).unwrap();
        let mut hash = super::NativeSurfaceHashIndex::lazy(8, 1, 0).unwrap();
        let row = super::NativeSurfaceCandidateRow::default();
        assert_eq!(super::append_surface_candidate_to_hash(&mut rows, &mut hash, 11, row, false), Ok(0));
        assert_eq!(hash.bucket_mask(), 7);
        assert_eq!(hash.cached_state_word(), u32::MAX);
        assert_eq!(hash.bucket_heads().len(), 8);
        assert_eq!(hash.next_indices(), &[u32::MAX]);
        assert_eq!(super::append_surface_candidate_to_hash(&mut rows, &mut hash, 19, row, false), Ok(1));
        assert_eq!(hash.chain_capacity(), 2);
        assert_eq!(hash.next_indices(), &[u32::MAX, 0]);
        assert_eq!(hash.bucket_chain(11), Ok(vec![1, 0]));
        assert_eq!(hash.bucket_chain(3), Ok(vec![1, 0]));
        hash.clear();
        assert_eq!(hash.cached_state_word(), 0);
        assert!(hash.bucket_heads().is_empty());
        assert_eq!(hash.bucket_chain(11), Ok(Vec::new()));
        assert_eq!(super::append_surface_candidate_to_hash(&mut rows, &mut hash, 11, row, false), Ok(2));
        assert_eq!(hash.bucket_chain(11), Ok(vec![2]));
    }

    #[test]
    fn native_surface_candidate_payload_and_row_match_original_offsets() {
        let sort = super::NativeSurfaceCandidateSortFields {
            candidate_material_pointer: 0x101,
            owner_sort_value_1bb0: 0x202,
            surface_resource_value_00: 0x303,
            owner_pointer: 0x4000,
            surface_resource_value_10: 0x404,
            sort_coverage_bits: 0x3f000000,
            padding_32_33: [0xa5, 0x5a],
            candidate_byte_38: 1,
            candidate_pointer_28: 2,
            surface_state_pointer_78: 0,
            surface_state_pointer_40: 0,
            surface_type_count: 1,
            owner_byte_1c31: 0x06,
        };
        let payload = super::native_surface_candidate_sort_payload(&sort);
        assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 0x101);
        assert_eq!(u64::from_le_bytes(payload[8..16].try_into().unwrap()), 0x202);
        assert_eq!(u64::from_le_bytes(payload[16..24].try_into().unwrap()), 0x303);
        assert_eq!(u64::from_le_bytes(payload[24..32].try_into().unwrap()), 0x5950);
        assert_eq!(u64::from_le_bytes(payload[32..40].try_into().unwrap()), 0x404);
        assert_eq!(u32::from_le_bytes(payload[40..44].try_into().unwrap()), 0x3f000000);
        assert_eq!(u32::from_le_bytes(payload[44..48].try_into().unwrap()), u32::MAX);
        assert_eq!(payload[48], 1);
        assert_eq!(payload[49], 1);
        assert_eq!(payload[50..52], [0xa5, 0x5a]);
        assert_eq!(u32::from_le_bytes(payload[52..56].try_into().unwrap()), 0x7f7fffff);

        let row = super::native_surface_candidate_branch_row(&super::NativeSurfaceCandidateBranchFields {
            owner_byte_1c31: 0x06,
            global_byte_77b0: 0x80,
            context_pointer: 0x505,
            surface_container_pointer: 0x606,
            material_pointer: 0x707,
            selected_parameter_pointer: 0x808,
            override_parameter_pointer: 0x909,
            secondary_parameter_pointer: 0xa0a,
            material_parameter_float: 0.5,
            auxiliary_payload: payload,
        });
        assert_eq!(row.bytes[0], 0x81);
        assert_eq!(row.bytes[1], 0xff);
        assert_eq!(u64::from_le_bytes(row.bytes[8..16].try_into().unwrap()), 0x505);
        assert_eq!(u64::from_le_bytes(row.bytes[16..24].try_into().unwrap()), 0x606);
        assert_eq!(u64::from_le_bytes(row.bytes[24..32].try_into().unwrap()), 0x707);
        assert_eq!(u64::from_le_bytes(row.bytes[32..40].try_into().unwrap()), 0x808);
        assert_eq!(u64::from_le_bytes(row.bytes[40..48].try_into().unwrap()), 0x909);
        assert_eq!(u64::from_le_bytes(row.bytes[48..56].try_into().unwrap()), 0xa0a);
        assert_eq!(row.bytes[0x38..0x40], [0; 8]);
        assert_eq!(u32::from_le_bytes(row.bytes[0x78..0x7c].try_into().unwrap()), 0.5_f32.to_bits());

        let mut gated = sort;
        gated.surface_state_pointer_40 = 1;
        assert_eq!(super::native_surface_candidate_sort_payload(&gated)[48], 0);
    }

    #[test]
    fn preallocated_surface_pointer_insert_matches_native_clamp_shift_and_count() {
        let mut slots = [11, 22, 33, 0];
        let mut count = 0xa500_0003;
        assert_eq!(super::insert_preallocated_surface_pointer(&mut slots, &mut count, 4, 1, 99), Ok(1));
        assert_eq!(slots, [11, 99, 22, 33]);
        assert_eq!(count, 0xa500_0004);
        assert_eq!(super::insert_preallocated_surface_pointer(&mut slots, &mut count, 4, -7, 88), Err("native surface pointer vector growth is required".to_owned()));

        let mut empty = [0; 2];
        let mut empty_count = 0;
        assert_eq!(super::insert_preallocated_surface_pointer(&mut empty, &mut empty_count, 2, i32::MIN, 7), Ok(0));
        assert_eq!(empty, [7, 0]);
        assert_eq!(empty_count, 1);

        let nodes = [
            super::SurfaceHashChainNode { next: Some(2), surface_pointer: 10 },
            super::SurfaceHashChainNode { next: None, surface_pointer: 30 },
            super::SurfaceHashChainNode { next: Some(1), surface_pointer: 10 },
        ];
        let mut prefixed = [20, 21, 22, 23, 0, 0, 0];
        let mut prefixed_count = 4;
        assert_eq!(
            super::build_prepass_hash_chain_pointer_list(
                &nodes, Some(0), &mut prefixed, &mut prefixed_count, 7,
            ),
            Ok(()),
        );
        assert_eq!(prefixed, [30, 10, 10, 20, 21, 22, 23]);
        assert_eq!(prefixed_count, 7);

        let mut full = [1, 2];
        let mut full_count = 2;
        assert_eq!(super::insert_preallocated_surface_pointer(&mut full, &mut full_count, 2, 100, 3), Err("native surface pointer vector growth is required".to_owned()));
    }

    #[test]
    fn native_surface_pointer_list_ports_allocator_modes_growth_and_capacity_reset() {
        let mut ordinary = super::NativeSurfacePointerList::with_capacity(1, 0).unwrap();
        assert_eq!(ordinary.insert(0, 10, false), Ok(0));
        assert_eq!(ordinary.insert(1, 20, false), Ok(1));
        assert_eq!(ordinary.capacity(), 2);
        assert_eq!(ordinary.values(), &[10, 20]);
        assert_eq!(ordinary.insert(u32::MAX, 5, false), Ok(0));
        assert_eq!(ordinary.values(), &[5, 10, 20]);
        assert_eq!(ordinary.capacity(), 3);

        let mut specialized = super::NativeSurfacePointerList::with_capacity(1, 0xc0).unwrap();
        assert_eq!(specialized.insert(0, 30, true), Ok(0));
        assert_eq!(specialized.insert(1, 40, true), Ok(1));
        assert_eq!(specialized.values(), &[30, 40]);
        assert_eq!(specialized.packed_header(), (2, (2 << 8) | 0xc0));

        let mut specialized_oom = super::NativeSurfacePointerList::with_capacity(1, 0xc0).unwrap();
        specialized_oom.insert(0, 45, true).unwrap();
        assert_eq!(specialized_oom.set_capacity(2, false), Ok(true));
        assert_eq!(specialized_oom.packed_header(), (1, (2 << 8) | 0xc0));
        assert_eq!(specialized_oom.values(), &[45]);
        assert!(specialized_oom.insert(1, 46, true).is_err());

        let mut denied = super::NativeSurfacePointerList::with_capacity(1, 0x40).unwrap();
        assert_eq!(denied.insert(0, 50, false), Ok(0));
        let before = denied.packed_header();
        assert!(denied.insert(1, 60, false).is_err());
        assert_eq!(denied.packed_header(), before);
        assert_eq!(denied.values(), &[50]);

        let mut failure = super::NativeSurfacePointerList::with_capacity(2, 0).unwrap();
        failure.insert(0, 70, false).unwrap();
        let before = failure.packed_header();
        assert_eq!(failure.set_capacity(4, false), Ok(false));
        assert_eq!(failure.packed_header(), before);
        assert_eq!(failure.count(), 1);
        assert!(failure.values().is_empty());

        let mut retained = super::NativeSurfacePointerList::with_capacity(2, 0x40).unwrap();
        retained.insert(0, 80, false).unwrap();
        assert_eq!(retained.set_capacity(0, true), Ok(true));
        assert_eq!(retained.packed_header(), (0, (2 << 8) | 0x40));
        assert!(retained.values().is_empty());
        let mut released = super::NativeSurfacePointerList::with_capacity(2, 0).unwrap();
        released.insert(0, 90, false).unwrap();
        assert_eq!(released.set_capacity(0, true), Ok(true));
        assert_eq!(released.packed_header(), (0, 0));
        assert_eq!(released.capacity(), 0);

        let mut slots = vec![11, 22];
        let mut capacity_word = (2 << 8) | 0x55;
        super::reallocate_native_surface_pointers(&mut slots, 2, &mut capacity_word, 4).unwrap();
        assert_eq!(slots, [11, 22, 0, 0]);
        assert_eq!(capacity_word, (4 << 8) | 0x3f);
        assert_eq!(super::allocate_zeroed_native_surface_pointers(3).unwrap(), [0, 0, 0]);
    }

    #[test]
    fn native_surface_batch_rows_preserve_0x38_bytes_across_capacity_modes() {
        let mut ordinary = super::NativeSurfaceBatchRowList::with_capacity(1, 0).unwrap();
        let first = ordinary.push_zeroed_row(false, true).unwrap();
        assert_eq!(first, 0);
        ordinary.row_mut(first).unwrap()[0] = 0x1234_5678;
        ordinary.row_mut(first).unwrap()[13] = 0x9abc_def0;
        let second = ordinary.push_zeroed_row(false, true).unwrap();
        assert_eq!(second, 1);
        assert_eq!(ordinary.capacity(), 2);
        assert_eq!(ordinary.rows()[0][0], 0x1234_5678);
        assert_eq!(ordinary.rows()[0][13], 0x9abc_def0);
        assert_eq!(ordinary.rows()[1], [0; 14]);
        assert_eq!(std::mem::size_of::<super::NativeSurfaceBatchRow>(), 0x38);

        let mut specialized = super::NativeSurfaceBatchRowList::with_capacity(1, 0xc0).unwrap();
        let first = specialized.push_zeroed_row(true, true).unwrap();
        specialized.row_mut(first).unwrap()[6] = 0xfeed_beef;
        let second = specialized.push_zeroed_row(true, true).unwrap();
        assert_eq!(second, 1);
        assert_eq!(specialized.packed_header(), (2, (2 << 8) | 0xc0));
        assert_eq!(specialized.rows()[0][6], 0xfeed_beef);
        assert_eq!(specialized.rows()[1], [0; 14]);

        let mut specialized_oom = super::NativeSurfaceBatchRowList::with_capacity(1, 0xc0).unwrap();
        specialized_oom.push_zeroed_row(true, true).unwrap();
        assert_eq!(specialized_oom.set_capacity(2, false), Ok(true));
        assert_eq!(specialized_oom.packed_header(), (1, (2 << 8) | 0xc0));
        assert!(specialized_oom.push_zeroed_row(true, true).is_err());

        let mut ordinary_oom = super::NativeSurfaceBatchRowList::with_capacity(1, 0).unwrap();
        ordinary_oom.push_zeroed_row(false, true).unwrap();
        let before = ordinary_oom.packed_header();
        assert_eq!(ordinary_oom.set_capacity(2, false), Ok(false));
        assert_eq!(ordinary_oom.packed_header(), before);
        assert_eq!(ordinary_oom.count(), 1);
        assert!(ordinary_oom.rows().is_empty());

        let mut denied = super::NativeSurfaceBatchRowList::with_capacity(1, 0x40).unwrap();
        denied.push_zeroed_row(false, true).unwrap();
        assert!(denied.push_zeroed_row(false, true).is_err());
        assert_eq!(denied.set_capacity(0, true), Ok(true));
        assert_eq!(denied.packed_header(), (0, (1 << 8) | 0x40));
    }

    #[test]
    fn surface_lookup_3119a0_matches_default_and_override_indexing() {
        let default_bank: Vec<u64> = (0..100).map(|index| 0x1000 + index).collect();
        let override_bank: Vec<u64> = (0..330).map(|index| 0x2000 + index).collect();
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, -1),
            Ok(0x1000 + 48),
        );
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0, 4, 2, 1),
            Ok(0x1000 + 48),
        );
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, 0),
            Ok(0x2000 + 48),
        );
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, 1),
            Ok(0x2000 + 114),
        );
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, 2),
            Ok(0x2000 + 180),
        );
        for selector in [3, 6] {
            assert_eq!(
                super::surface_lookup_3119a0(
                    &default_bank, &override_bank, 0x40, 4, 2, selector,
                ),
                Ok(0x2000 + 246),
            );
        }
        assert_eq!(
            super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, 7),
            Ok(0x2000 + 312),
        );
        assert!(super::surface_lookup_3119a0(&[], &[], 0, u32::MAX, u8::MAX, -1).is_err());
    }

    #[test]
    fn reserve_surface_candidate_row_index_matches_packed_growth_paths() {
        let mut count = 0xa500_0003;
        let mut capacity = 4 << 8;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, true),
            3,
        );
        assert_eq!(count, 0xa500_0004);
        assert_eq!(capacity, 4 << 8);

        count = 0x5a00_0004;
        capacity = 4 << 8;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, true),
            4,
        );
        assert_eq!(count, 0x5a00_0005);
        assert_eq!(capacity, 5 << 8);

        count = 4;
        capacity = (4 << 8) | 0xc0;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, true),
            4,
        );
        assert_eq!(capacity, (5 << 8) | 0x3f);

        count = 4;
        capacity = (4 << 8) | 0xc0;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, false),
            3,
        );
        assert_eq!(count, 4);
        assert_eq!(capacity, (4 << 8) | 0xc0);

        count = 4;
        capacity = 4 << 8;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, false),
            u32::MAX,
        );
        assert_eq!(count, 4);
        assert_eq!(capacity, 4 << 8);

        count = 4;
        capacity = (4 << 8) | 0x40;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, true, true),
            u32::MAX,
        );

        count = 5;
        capacity = 4 << 8;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, true),
            3,
        );
        assert_eq!(count, 5);

        count = 5;
        capacity = (5 << 8) | 3;
        assert_eq!(
            super::reserve_surface_candidate_row_index(&mut count, &mut capacity, false, true),
            5,
        );
        assert_eq!(capacity, 8 << 8 | 3);
    }

    #[test]
    fn native_candidate_row_initializer_matches_observed_fields() {
        let row = super::NativeSurfaceCandidateRow::default();
        assert_eq!(row.bytes[0], 0);
        assert_eq!(row.bytes[1], 0xff);
        assert_eq!(u32::from_le_bytes(row.bytes[0x6c..0x70].try_into().unwrap()), u32::MAX);
        assert_eq!(u64::from_le_bytes(row.bytes[0x74..0x7c].try_into().unwrap()), 0x7f7f_ffff);
        assert_eq!(u32::from_le_bytes(row.bytes[0x94..0x98].try_into().unwrap()), 0x3f80_0000);
        assert_eq!(&row.bytes[0x98..0xa4], &[0; 12]);
    }

    #[test]
    fn native_candidate_rows_append_and_preserve_data_across_growth() {
        let mut rows = super::NativeSurfaceCandidateRows::with_capacity(1, 0x3f).unwrap();
        let mut first = super::NativeSurfaceCandidateRow::default();
        first.bytes[8..16].copy_from_slice(&0x1122_3344_5566_7788_u64.to_le_bytes());
        let mut second = super::NativeSurfaceCandidateRow::default();
        second.bytes[8..16].copy_from_slice(&0x8877_6655_4433_2211_u64.to_le_bytes());
        assert_eq!(rows.append(first, false), Ok(0));
        assert_eq!(rows.append(second, false), Ok(1));
        assert_eq!((rows.count(), rows.capacity()), (2, 2));
        assert_eq!(rows.rows()[0].bytes[8..16], first.bytes[8..16]);
        assert_eq!(rows.rows()[1].bytes[8..16], second.bytes[8..16]);

        let mut mode_three = super::NativeSurfaceCandidateRows::with_capacity(1, 0xc0).unwrap();
        assert_eq!(mode_three.append(first, false), Ok(0));
        assert_eq!(mode_three.append(second, false), Ok(1));
        assert_eq!(mode_three.packed_header(), (2, (2 << 8) | 0x3f));
        assert_eq!(mode_three.rows()[0].bytes[8..16], first.bytes[8..16]);
    }

    #[test]
    fn native_candidate_reallocation_copies_only_live_prefix() {
        let mut rows = vec![
            super::NativeSurfaceCandidateRow::default(),
            super::NativeSurfaceCandidateRow::default(),
        ];
        rows[0].bytes[2] = 0x11;
        rows[1].bytes[2] = 0x22;
        super::reallocate_surface_candidate_rows(&mut rows, 1, 3).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].bytes[2], 0x11);
        assert_eq!(rows[1], super::NativeSurfaceCandidateRow::default());
        assert_eq!(rows[2], super::NativeSurfaceCandidateRow::default());
    }

    #[test]
    fn surface_compare_1_returns_observed_native_index() {
        assert_eq!(super::surface_compare_1_index_result(), 0);
    }

    #[test]
    fn keycompare_7_binary_search_uses_both_float_keys_and_tie_comparator() {
        let entries = [surface_key(1.0, 0.0), surface_key(3.0, 0.0), surface_key(5.0, 0.0)];
        assert_eq!(super::surface_key_compare_7_insertion_index(&entries, 3, &surface_key(4.0, 0.0)), Ok(2));
        assert_eq!(super::surface_key_compare_7_insertion_index(&entries, 3, &entries[1]), Ok(1));
        assert_eq!(super::surface_key_compare_7_insertion_index(&entries, 0, &entries[1]), Ok(0));
        assert_eq!(super::surface_key_compare_7_insertion_index(&entries, -1, &entries[1]), Ok(0));
        let secondary = [surface_key(3.0, 1.0), surface_key(3.0, 3.0)];
        assert_eq!(super::surface_key_compare_7_insertion_index(&secondary, 2, &surface_key(3.0, 2.0)), Ok(1));
        assert_eq!(super::surface_key_compare_7_insertion_index(&entries, 4, &entries[1]), Err("surface sort count exceeds its supplied entries".to_owned()));
    }

    fn selector_state(
        flags48: u8,
        flags4a: u8,
        flags4b: u8,
        resource: u64,
        first: f32,
        second: f32,
    ) -> Vec<u8> {
        let mut state = vec![0; 0x2c8];
        state[0x48] = flags48;
        state[0x4a] = flags4a;
        state[0x4b] = flags4b;
        state[0x100..0x108].copy_from_slice(&resource.to_le_bytes());
        state[0x2c0..0x2c4].copy_from_slice(&first.to_le_bytes());
        state[0x2c4..0x2c8].copy_from_slice(&second.to_le_bytes());
        state
    }

    #[test]
    fn static_surface_selector_preserves_native_flag_precedence() {
        let bands = [0.0; 10];
        let select = |state: Vec<u8>| super::static_surface_program_selector(&state, &bands);
        assert_eq!(select(selector_state(0, 0, 0, 1, 0.0, 0.0)), Ok(0x25));
        assert_eq!(select(selector_state(0x80, 0x10, 0, 1, 0.0, 0.0)), Ok(0x2a));
        assert_eq!(select(selector_state(0, 0x10, 0, 1, 0.0, 0.0)), Ok(0x36));
        for low in [1, 2] {
            assert_eq!(select(selector_state(0, 0, low, 0, 0.0, 0.0)), Ok(0x29));
        }
        assert_eq!(select(selector_state(0x10, 0, 0, 0, 0.0, 0.0)), Ok(0x23));
        assert_eq!(select(selector_state(0x10, 4, 0, 0, 0.0, 0.0)), Ok(0x37));

        let mut first_band = [0.0; 10];
        first_band[0] = 1.0;
        assert_eq!(
            super::static_surface_program_selector(
                &selector_state(0, 0, 0x80, 0, 0.0, 0.0),
                &first_band,
            ),
            Ok(0x37),
        );
    }

    #[test]
    fn static_surface_selector_uses_native_threshold_order_and_nan_comparisons() {
        let state = selector_state(0, 0, 0, 0, 1.0, 2.0);
        let ordered = [
            (1, 0x26), (2, 0x32), (3, 0x33), (5, 0x34), (4, 0x2d),
            (6, 0x37), (7, 0x35), (8, 0x38), (9, 0x39),
        ];
        for (index, expected) in ordered {
            let mut bands = [0.0; 10];
            bands[index] = 4.0;
            assert_eq!(
                super::static_surface_program_selector(&state, &bands),
                Ok(expected),
                "threshold index {index}",
            );
        }
        assert_eq!(super::static_surface_program_selector(&state, &[0.0; 10]), Ok(0x41));
        let mut below_sum = [0.0; 10];
        below_sum[0] = 2.5;
        assert_eq!(
            super::static_surface_program_selector(&state, &below_sum),
            Ok(0x41),
        );
        assert_eq!(
            super::static_surface_program_selector(
                &selector_state(0, 0, 0, 0, f32::NAN, 2.0),
                &[4.0; 10],
            ),
            Ok(0x41),
        );
        assert!(super::static_surface_program_selector(&[0; 0x2c7], &[0.0; 10]).is_err());
    }

    #[test]
    fn parameterized_surface_selector_preserves_flag_and_threshold_precedence() {
        let bands = [0.0; 10];
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0, 0, 0, 1, 0.0, 0.0), 5.0, &bands,
            ),
            Ok(0x25),
        );
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0x80, 0x10, 0, 1, 0.0, 0.0), 5.0, &bands,
            ),
            Ok(0x2a),
        );
        for (subtype, expected) in [(0, 0x23), (1, 0x37), (2, 0x37), (3, 0x23)] {
            assert_eq!(
                super::parameterized_surface_program_selector(
                    &selector_state(0x10, subtype << 2, 0, 0, 0.0, 0.0), 5.0, &bands,
                ),
                Ok(expected),
                "material subtype {subtype}",
            );
        }
        let ordered = [
            (0, 0x23), (1, 0x26), (2, 0x32), (3, 0x33), (5, 0x34),
            (4, 0x2d), (6, 0x37), (7, 0x35), (8, 0x38), (9, 0x39),
        ];
        for (index, expected) in ordered {
            let mut thresholds = [0.0; 10];
            thresholds[index] = 2.0;
            assert_eq!(
                super::parameterized_surface_program_selector(
                    &selector_state(0, 0, 0, 0, 0.0, 0.0), 1.0, &thresholds,
                ),
                Ok(expected),
                "threshold index {index}",
            );
        }
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0, 0, 0, 0, 0.0, 0.0), f32::NAN, &[2.0; 10],
            ),
            Ok(0x41),
        );
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0, 0x10, 0x80, 0, 0.0, 0.0), 1.0, &[2.0; 10],
            ),
            Ok(0x37),
        );
        assert!(super::parameterized_surface_program_selector(&[0; 0x107], 1.0, &bands).is_err());
        let mut parameter_object = [0; 0x18];
        parameter_object[0x10..0x18].copy_from_slice(&0x1000u64.to_le_bytes());
        assert_eq!(
            super::native_surface_parameter_float_lookup(&parameter_object, 0xfeed),
            Ok((0x12d8, 0xfeed, 0)),
        );
        assert!(super::native_surface_parameter_float_lookup(&parameter_object[..0x17], 0xfeed).is_err());
    }

    #[test]
    fn reset_preserves_empty_slots_and_chain_order() {
        let mut slots = vec![Some(91), Some(92), Some(93), Some(94)];
        let mut count = 4;
        // 2 -> 0 -> 1 -> END. The empty record 0 leaves slot 1 intact.
        build_zprepass_list(&mut slots, &mut count, &[false, true, true],
            &[1, u32::MAX, 0], 3, 2, false);
        assert_eq!(count, 3);
        assert_eq!(slots, [Some(2), Some(92), Some(1), Some(94)]);
    }

    #[test]
    fn append_and_masked_links_preserve_prefix() {
        let mut slots = vec![Some(17)];
        let mut count = 1;
        // Record 4 follows bucket 0, not next[4].
        build_zprepass_list(&mut slots, &mut count, &[true; 5],
            &[1, u32::MAX, u32::MAX, u32::MAX], 3, 4, true);
        assert_eq!(count, 3);
        assert_eq!(slots, [Some(17), Some(4), Some(1)]);
    }

    #[test]
    fn empty_chain_resets_or_retains_count_without_touching_storage() {
        let mut slots = vec![Some(31), Some(32)];
        let mut count = 2;
        build_zprepass_list(&mut slots, &mut count, &[], &[], 0, u32::MAX, true);
        assert_eq!(count, 2);
        build_zprepass_list(&mut slots, &mut count, &[], &[], 0, u32::MAX, false);
        assert_eq!(count, 0);
        assert_eq!(slots, [Some(31), Some(32)]);
    }
}

/// FUN_140872a50, 140872b1a: the qword store at +0xa8 also zeros
/// maxVisibleRange at +0xac. This is the constructor default, not a cutoff.
pub const INITIAL_MODEL_MAX_VISIBLE_RANGE: f32 = 0.0;

/// FUN_1401095f0: authored axes are stored by basis, not by matrix row.
pub fn affine_rows(position: [f32; 3], basis: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        if row == 3 { [0.0, 0.0, 0.0, 1.0] } else {
            [basis[0][row] * scale[0], basis[1][row] * scale[1], basis[2][row] * scale[2], position[row]]
        }
    })
}

/// FUN_140235c80 / FUN_1400fb8c0: scaled local-box center and absolute rotated extents.
pub fn transformed_bounds(local: [[f32; 3]; 2], position: [f32; 3], basis: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 3]; 2] {
    let center = std::array::from_fn::<_, 3, _>(|axis| (local[1][axis] + local[0][axis]) * scale[axis] * 0.5);
    let extent = std::array::from_fn::<_, 3, _>(|axis| scale[axis] * local[1][axis] - center[axis]);
    let rotated_extent = std::array::from_fn::<_, 3, _>(|axis| (extent[1] * basis[1][axis]).abs() + (extent[0] * basis[0][axis]).abs() + (extent[2] * basis[2][axis]).abs());
    let world_center = std::array::from_fn::<_, 3, _>(|axis| center[1] * basis[1][axis] + center[0] * basis[0][axis] + center[2] * basis[2][axis] + position[axis]);
    [std::array::from_fn(|axis| world_center[axis] - rotated_extent[axis]), std::array::from_fn(|axis| rotated_extent[axis] + world_center[axis])]
}

/// FUN_140237340: optional range region; nonpositive range uses its constructor region.
pub fn range_region(bounds: [[f32; 3]; 2], range: f32) -> [f32; 4] {
    if !(range > 0.0) { return [0.0, 0.0, 0.0, 32768.0]; }
    let center = std::array::from_fn::<_, 3, _>(|axis| (bounds[1][axis] + bounds[0][axis]) * 0.5);
    let mut extent = 0.0;
    for side in bounds {
        for axis in 0..3 {
            let value = side[axis] - center[axis];
            if extent <= value { extent = value; }
        }
    }
    [center[0], center[1], center[2], extent + range]
}

pub use crate::bmodel::accumulate_resource_bounds;

/// FUN_140268730: a missing static resource uses the observed zero bounds.
pub fn static_resource_bounds(resource: Option<&[[f32; 3]; 2]>) -> &[[f32; 3]; 2] {
    const EMPTY: [[f32; 3]; 2] = [[0.0; 3]; 2];
    resource.unwrap_or(&EMPTY)
}

/// FUN_14025da70: return the owned resource's bounds without modifying them.
pub fn resource_bounds(bounds: &[[f32; 3]; 2]) -> &[[f32; 3]; 2] { bounds }

/// FUN_14031ad30's finite no-depth selection prefix. The owning model
/// supplies prepared flags and bounds; they are not inferred from shape names.
#[derive(Clone, Copy)]
pub struct ModelQueryInput {
    pub frame_flags: u8,
    pub shape_flags: u8,
    pub region: [f32; 4],
    pub bounds: [[f32; 3]; 2],
}

pub fn model_eligible(frame_flags: u8, shape_flags: u8) -> bool {
    frame_flags & 4 != 0 || (shape_flags & 0x10 != 0 && frame_flags & 1 == 0)
}

pub fn in_model_region(camera: [f32; 3], region: [f32; 4]) -> bool {
    let radius = region[3];
    !(radius < radius.abs() || (0..3).any(|axis| radius < (camera[axis] - region[axis]).abs()))
}

pub fn model_visible(
    query: &NoDepthAabbQuery,
    camera: [f32; 3],
    viewport: [u32; 2],
    model: ModelQueryInput,
) -> Result<bool, String> {
    if !model_eligible(model.frame_flags, model.shape_flags) {
        return Ok(false);
    }
    if viewport.contains(&0) || camera.iter().chain(model.region.iter()).any(|value| !value.is_finite()) {
        return Err("unsupported nonfinite model region or empty viewport".to_owned());
    }
    if !in_model_region(camera, model.region) {
        return Ok(false);
    }
    let (visible, area) = query.visibility_and_area(model.bounds)?;
    Ok(visible && (model.frame_flags & 0x40 != 0 || !(area < 25.0 / (viewport[0] as f32 * viewport[1] as f32))))
}

/// FUN_140399620: the base render-model virtual callback has no side effects.
pub fn base_callback() -> u8 { 0 }

/// FUN_140234ba0: an overlapping mask does not add its other bits.
pub fn request_flags(flags: &mut u32, mask: u32) {
    if *flags & mask == 0 {
        *flags |= mask | 1;
    }
}

/// FUN_140236550: the dirty bit is set before the supplied mask is cleared.
pub fn clear_flags(flags: &mut u32, mask: u32) {
    if *flags & mask != 0 {
        *flags = (*flags | 1) & !mask;
    }
}

/// FUN_14031d0f0: preserve the native multiplication order and NaN propagation.
pub fn distance_fade(start: f32, end: f32, value: f32) -> f32 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    (3.0 - t * 2.0) * (t * t)
}

/// FUN_1403013e0: its shared padding input is DAT_1432ef64c, not a fixed margin.
pub fn square_patch_bounds(
    patch: [f32; 3],
    height_center: f32,
    height_extent: f32,
    spacing: f32,
) -> [[f32; 3]; 2] {
    let padding = spacing * 0.25;
    [
        [patch[0] - padding, patch[1] - padding, height_center - height_extent],
        [(patch[2] + patch[0]) + padding, (patch[2] + patch[1]) + padding, height_extent + height_center],
    ]
}

pub fn square_patch_visible(
    query: &NoDepthAabbQuery,
    patch: [f32; 3],
    height_center: f32,
    height_extent: f32,
    spacing: f32,
) -> Result<bool, String> {
    query.visible(square_patch_bounds(patch, height_center, height_extent, spacing))
}

/// FUN_1402370e0's surface inputs; property handle identity is not guessed.
#[derive(Clone, Copy)]
pub struct SurfaceFlags {
    pub byte_48: u8,
    pub byte_4b: u8,
    pub byte_4c: u8,
    pub parameter_14229d460: f32,
}

/// FUN_140342640's global-default branch; later matching owner keys win.
/// Registry selection is FUN_140310dd0; [u32; 4] copies match FUN_1403104a0.
pub fn parameter_float(
    name: &str,
    id: u32,
    flags: u32,
    defaults: &[[u32; 4]],
    overrides: &[(u16, [u32; 4])],
) -> Result<f32, String> {
    if flags & 0xf != 0 && flags & 0x30 != 0x30 {
        return Err(format!("idParmBlock::GetFloat: {name} is not a vector parm"));
    }
    let mut value = *defaults.get(id as usize).ok_or("parameter registry index out of bounds")?;
    for &(key, replacement) in overrides {
        if u32::from(key) == id { value = replacement; }
    }
    Ok(f32::from_bits(value[0]))
}

/// FUN_140342360: stable paired-slot removal; capacity and model dirty state are unchanged.
pub fn remove_parameter(overrides: &mut Vec<(u16, [u32; 4])>, id: u32) {
    overrides.retain(|&(key, _)| u32::from(key) != id);
}

/// An owned, preallocated idParmBlock. Entries follow the ID count, which
/// need not equal the packed value count. Allocation/growth policies are
/// deliberately not substituted with a generic Vec growth rule.
#[derive(Debug)]
pub struct ParameterBlock {
    pub entries: Vec<(u16, [u32; 4])>,
    pub value_count_word: u32,
    pub id_count_word: u32,
    pub value_capacity_word: u32,
    pub id_capacity_word: u32,
}

impl ParameterBlock {
    fn validate_preallocated(&self) -> Result<(), String> {
        if self.entries.len() != (self.id_count_word & 0xffffff) as usize {
            return Err("parameter ID count does not match initialized slots".into());
        }
        if self.entries.len() > (self.value_capacity_word >> 8) as usize
            || self.entries.len() > (self.id_capacity_word >> 8) as usize {
            return Err("native parameter-array growth is not implemented".into());
        }
        Ok(())
    }

    fn set_compacted_counts(&mut self) {
        let count = self.entries.len() as u32;
        self.value_count_word = (self.value_count_word & 0xff000000) | count;
        self.id_count_word = (self.id_count_word & 0xff000000) | count;
    }

    /// FUN_140343670's no-growth branch: the last duplicate wins and retained
    /// slots keep source order. With no duplicates, both count words stay intact.
    pub fn deduplicate(&mut self) -> Result<(), String> {
        self.validate_preallocated()?;
        if self.entries.len() > 512 {
            return Err("native duplicate marker array is limited to 512 entries".into());
        }
        let mut removed = [false; 512];
        for index in (0..self.entries.len()).rev() {
            removed[index] = (index + 1..self.entries.len()).any(|later|
                !removed[later] && self.entries[later].0 == self.entries[index].0);
        }
        if removed[..self.entries.len()].contains(&true) {
            let mut index = 0;
            self.entries.retain(|_| {
                let keep = !removed[index];
                index += 1;
                keep
            });
            self.set_compacted_counts();
        }
        Ok(())
    }

    /// FUN_140344540's no-growth branch repairs both counts even when the
    /// whitelist removes nothing. Its whitelist count is at +800 decimal.
    pub fn filter(&mut self, whitelist: &[u32]) -> Result<(), String> {
        self.validate_preallocated()?;
        self.entries.retain(|&(key, _)| whitelist.contains(&u32::from(key)));
        self.set_compacted_counts();
        Ok(())
    }

    pub fn float(&self, name: &str, id: u32, flags: u32, defaults: &[[u32; 4]]) -> Result<f32, String> {
        self.validate_preallocated()?;
        parameter_float(name, id, flags, defaults, &self.entries)
    }

    /// FUN_140310450 copies the value-count prefix to the ID+1 slots.
    /// The paired representation preserves native ID/value order and duplicates.
    pub fn write_entries_to_slots(&self, slots: &mut [[u32; 4]]) -> Result<(), String> {
        let count = (self.value_count_word & 0x00ff_ffff) as usize;
        if count > self.entries.len() {
            return Err("native parameter value count exceeds initialized entries".into());
        }
        for &(id, _) in &self.entries[..count] {
            if usize::from(id) + 1 >= slots.len() {
                return Err("native parameter ID exceeds destination slots".into());
            }
        }
        for &(id, value) in &self.entries[..count] {
            slots[usize::from(id) + 1] = value;
        }
        Ok(())
    }
}

fn native_parameter_slot_index(id: i32, slot_count: usize) -> Result<usize, String> {
    let index = usize::try_from(i64::from(id) + 1)
        .map_err(|_| "native parameter ID selects a negative slot".to_owned())?;
    if index >= slot_count {
        return Err("native parameter ID exceeds destination slots".into());
    }
    Ok(index)
}

/// FUN_140310720: zero the first qword and write an exact second qword.
pub fn write_native_parameter_u64(
    slots: &mut [[u32; 4]], id: i32, value: u64,
) -> Result<(), String> {
    let slot = native_parameter_slot_index(id, slots.len())?;
    slots[slot] = [0, 0, value as u32, (value >> 32) as u32];
    Ok(())
}

/// FUN_140310780: replicate the exact 32-bit word to all four lanes.
pub fn write_native_parameter_repeated_u32(
    slots: &mut [[u32; 4]], id: i32, value: u32,
) -> Result<(), String> {
    let slot = native_parameter_slot_index(id, slots.len())?;
    slots[slot] = [value; 4];
    Ok(())
}

/// FUN_140310a80: copy two raw qwords into the parameter slot at ID + 1.
pub fn write_native_parameter_4_words(
    slots: &mut [[u32; 4]], id: i32, value: [u32; 4],
) -> Result<(), String> {
    let slot = native_parameter_slot_index(id, slots.len())?;
    slots[slot] = value;
    Ok(())
}

/// FUN_140310920: zero-extend the word into the first qword and clear the second.
pub fn write_native_parameter_u32(
    slots: &mut [[u32; 4]], id: i32, value: u32,
) -> Result<(), String> {
    let slot = native_parameter_slot_index(id, slots.len())?;
    slots[slot] = [value, 0, 0, 0];
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeOptionalParameterValues {
    pub gate_at_0x38: u64,
    pub indirect_gate_at_0x78: u64,
    pub word_at_0x90: u32,
    pub words_at_0x94: [u32; 4],
    pub word_at_0x8c: u32,
}

/// FUN_1402afbf0's conditional writes, given the original pointer-loaded values
/// and the three runtime parameter IDs in native call order.
pub fn write_native_optional_parameter_values(
    slots: &mut [[u32; 4]], ids: [i32; 3], values: NativeOptionalParameterValues,
) -> Result<bool, String> {
    if values.gate_at_0x38 != 0 || values.indirect_gate_at_0x78 != 0 {
        return Ok(false);
    }
    for id in ids {
        native_parameter_slot_index(id, slots.len())?;
    }
    write_native_parameter_u32(slots, ids[0], values.word_at_0x90)?;
    write_native_parameter_4_words(slots, ids[1], values.words_at_0x94)?;
    write_native_parameter_u32(slots, ids[2], values.word_at_0x8c)?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialParameterGroup {
    Base,
    Effect(i8),
}

/// FUN_1402d4bc0: signed-byte selector; effect blocks live behind the pointer
/// at material+0x90, not inline at that address. Each native block is 0x20 bytes.
pub fn material_parameter_group(material_flags_49: u8, selector: i8) -> MaterialParameterGroup {
    if selector == -1 || material_flags_49 & 0x40 == 0 {
        MaterialParameterGroup::Base
    } else {
        MaterialParameterGroup::Effect(selector)
    }
}

pub struct MaterialParameterBlocks {
    pub base: ParameterBlock,
    pub effects: Vec<ParameterBlock>,
    pub flags_49: u8,
}

impl MaterialParameterBlocks {
    pub fn selected(&self, selector: i8) -> Result<&ParameterBlock, String> {
        match material_parameter_group(self.flags_49, selector) {
            MaterialParameterGroup::Base => Ok(&self.base),
            MaterialParameterGroup::Effect(index) => usize::try_from(index).ok()
                .and_then(|index| self.effects.get(index))
                .ok_or_else(|| "material effect selector outside owned block storage".into()),
        }
    }
}

/// FUN_1402d4be0 after the existing property getter's float return.
pub fn parameter_bit(value: f32) -> bool {
    value > f32::from_bits(0x3a83126f)
}

/// FUN_1402370e0: OR-reduction retains low flags and the other frame bits.
pub fn reduce_surface_flags(previous: [u8; 2], surfaces: &[SurfaceFlags]) -> [u8; 2] {
    let mut result = [previous[0] & 0x0f, previous[1] & 0xfe];
    for surface in surfaces {
        result[0] |= u8::from(surface.byte_4b & 0x0c != 4) << 4;
        result[0] |= (surface.byte_48 & 1) << 5;
        result[0] |= u8::from(parameter_bit(surface.parameter_14229d460)) << 6;
        result[0] |= (surface.byte_4c & 1) << 7;
        result[1] |= (surface.byte_4c >> 6) & 1;
    }
    result
}

/// FUN_14087c1f0's owned graph; missing render-model handles are skipped.
#[derive(Default)]
pub struct VisibilityNode {
    pub model_flags: Option<u32>,
    pub children: Vec<VisibilityNode>,
}

impl VisibilityNode {
    pub fn set_visible(&mut self, visible: bool) {
        if let Some(flags) = &mut self.model_flags {
            if visible {
                request_flags(flags, 2);
            } else {
                clear_flags(flags, 2);
            }
        }
        for child in &mut self.children {
            child.set_visible(visible);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_alpha_draw_matches_original_three_pass_dispatch() {
        // probe_original_depth_alpha_draw.py executes unmodified 321860
        // list/program/state branches with rendering/TLS callees intercepted.
        let default: Vec<u64> = (100..144).collect();
        let overrides: Vec<u64> = (1000..1330).collect();
        let fixture = include_str!("../analysis/original_depth_alpha_draw_cases.txt");
        let mut count = 0;
        for (index, line) in fixture.lines().enumerate() {
            let words: Vec<i64> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
            assert_eq!(words.len(), 26);
            let surface = DepthAlphaSurface {flags49: words[0] as u8,
                selector: words[1] as i8, has_optional_mesh: words[2] != 0,
                special_vertex_format: words[3] != 0, alternate_program: 77,
                default_programs: &default, override_programs: &overrides,
                draw_state: words[5..14].iter().map(|v|*v as u32).collect::<Vec<_>>().try_into().unwrap()};
            let actual = prepare_depth_alpha_draw(&surface, words[4] as u8).unwrap();
            assert_eq!(actual.is_some(), words[14] != 0, "native row{index}");
            if let Some(actual) = actual {
                assert_eq!(actual.program, words[15] as u64, "native program row{index}");
                assert_eq!(actual.stencil_reference, words[16] as u8, "native stencil row{index}");
                assert_eq!(actual.draw_state.as_slice(), words[17..26].iter().map(|v|*v as u32).collect::<Vec<_>>(), "native state row{index}");
            }
            count += 1;
        }
        assert_eq!(count, 1440);
    }

    #[test]
    fn depth_alpha_draw_selection_and_native_state_masks() {
        let default: Vec<u64> = (100..144).collect();
        let overrides: Vec<u64> = (1000..1330).collect();
        let mut surface = DepthAlphaSurface {flags49: 0, default_programs: &default,
            override_programs: &overrides, selector: -1, has_optional_mesh: false,
            special_vertex_format: false, alternate_program: 77, draw_state: [u32::MAX;9]};
        for flags in [0,1,0x40,0x41] {for optional in [false,true] {for special in [false,true] {
            surface.flags49 = flags; surface.has_optional_mesh = optional; surface.special_vertex_format = special;
            for selector in [-1,0,1,2,3,6,7,-128] {
                surface.selector = selector;
                for pass in 0..3 {
                    let plan = prepare_depth_alpha_draw(&surface, pass).unwrap();
                    if pass == 2 && flags & 1 == 0 {assert!(plan.is_none()); continue;}
                    let plan = plan.unwrap();
                    let slot = match (pass, flags & 1) {(0,1) => 4, _ => 6} + u64::from(special);
                    let expected_program = if pass == 2 {77} else if selector != -1 && flags & 0x40 != 0 {
                        let row = match selector {0=>0,1=>1,2=>2,3..=6=>3,_=>4};
                        1000+(row*3+u64::from(optional))*22+slot
                    } else {100+u64::from(optional)*22+slot};
                    assert_eq!(plan.program, expected_program);
                    assert_eq!(plan.stencil_reference, if pass==2 {0x80} else {2});
                    assert_eq!(plan.marks_alpha, pass==0 && flags&1!=0);
                    assert_eq!(plan.draw_state, [if plan.marks_alpha {0xfe5f_fe23} else {0xfe1f_ffff},
                        0xfe1f_ffff,u32::MAX,u32::MAX,0xe80e_80ff,
                        if pass==2 {0xff80_80ef} else {0xff02_02ff},u32::MAX,u32::MAX,u32::MAX]);
                }
            }
        }}}
        surface.flags49=1; surface.alternate_program=0;
        assert!(prepare_depth_alpha_draw(&surface,2).unwrap().is_none());
        surface.flags49=0; surface.selector=-1; surface.has_optional_mesh=false;
        surface.default_programs=&[0;44];
        assert!(prepare_depth_alpha_draw(&surface,0).unwrap().is_none());
        assert!(prepare_depth_alpha_draw(&surface,3).is_err());
        surface.default_programs=&[];
        assert!(prepare_depth_alpha_draw(&surface,0).is_err());
    }

    #[test]
    fn native_multisample_getters_match_original() {
        assert_eq!(original_renderer_sample_count(ORIGINAL_DEFAULT_MULTI_SAMPLES), 1);
        let fixture = include_str!("../analysis/native_multisample_cases.txt");
        for line in fixture.lines() {
            let words = line.split_whitespace().map(|word| word.parse::<i32>().unwrap()).collect::<Vec<_>>();
            assert_eq!(original_renderer_sample_count(words[0]), words[2] as u32);
            // Entire FUN_14036b620 is integer equality against the raw setting,
            // not the clamped texture count. Rust's existing equality suffices.
            assert_eq!((words[0] == words[1]) as i32, words[3]);
        }
    }

    #[test]
    fn native_depth_draw_cache_reset_matches_conditional_two_qword_clear() {
        let mut bound = [1, 2, 3, 4];
        reset_native_depth_draw_cache_if_bound(&mut bound);
        assert_eq!(bound, [0, 2, 3, 0]);
        let mut unbound = [0, 2, 3, 4];
        reset_native_depth_draw_cache_if_bound(&mut unbound);
        assert_eq!(unbound, [0, 2, 3, 4]);
    }

    #[test]
    fn native_indirect_pointer_helpers_apply_the_original_addends() {
        let mut object = [0; 0x18];
        object[0x10..0x18].copy_from_slice(&0x1234_5678_9abc_def0_u64.to_le_bytes());
        assert_eq!(native_indirect_pointer_plus_0x58(&object).unwrap(), 0x1234_5678_9abc_df48);
        assert_eq!(native_indirect_pointer_plus_0x2d8(&object).unwrap(), 0x1234_5678_9abc_e1c8);
        assert!(native_indirect_pointer_plus_0x58(&object[..0x17]).is_err());
    }

    #[test]
    fn native_optional_parameter_writes_match_the_guard_and_call_order() {
        let values = NativeOptionalParameterValues {
            gate_at_0x38: 0,
            indirect_gate_at_0x78: 0,
            word_at_0x90: 0x1111_2222,
            words_at_0x94: [0x3333_4444, 0x5555_6666, 0x7777_8888, 0x9999_aaaa],
            word_at_0x8c: 0xbbbb_cccc,
        };
        let mut slots = [[0xdead_beef; 4]; 5];
        assert!(write_native_optional_parameter_values(&mut slots, [0, 1, 2], values).unwrap());
        assert_eq!(slots[1], [0x1111_2222, 0, 0, 0]);
        assert_eq!(slots[2], values.words_at_0x94);
        assert_eq!(slots[3], [0xbbbb_cccc, 0, 0, 0]);
        let before = slots;
        assert!(!write_native_optional_parameter_values(&mut slots, [0, 1, 2], NativeOptionalParameterValues { gate_at_0x38: 1, ..values }).unwrap());
        assert_eq!(slots, before);
        assert!(!write_native_optional_parameter_values(&mut slots, [0, 1, 2], NativeOptionalParameterValues { indirect_gate_at_0x78: 1, ..values }).unwrap());
        assert_eq!(slots, before);
    }

    #[test]
    fn native_single_resource_bind_wraps_one_value_without_changing_arguments() {
        let mut observed = None;
        call_native_resource_array_bind_with_one(11, 22, 33, 0x4455, |context, count, values, extra, flags| {
            observed = Some((context, count, *values, extra, flags));
        });
        assert_eq!(observed, Some((11, 1, [22], 33, 0x4455)));
    }

    #[test]
    fn native_parameter_slot_writers_preserve_word_layout_and_order() {
        let entries = vec![
            (2, [1, 2, 3, 4]),
            (1, [5, 6, 7, 8]),
            (2, [9, 10, 11, 12]),
        ];
        let block = ParameterBlock {
            entries,
            value_count_word: 0xa500_0003,
            id_count_word: 3,
            value_capacity_word: 0x300,
            id_capacity_word: 0x300,
        };
        let mut slots = [[0xdead_beef; 4]; 5];
        block.write_entries_to_slots(&mut slots).unwrap();
        assert_eq!(slots[0], [0xdead_beef; 4]);
        assert_eq!(slots[2], [5, 6, 7, 8]);
        assert_eq!(slots[3], [9, 10, 11, 12]);
        assert_eq!(slots[4], [0xdead_beef; 4]);

        write_native_parameter_u64(&mut slots, 1, 0xfedc_ba98_7654_3210).unwrap();
        assert_eq!(slots[2], [0, 0, 0x7654_3210, 0xfedc_ba98]);
        write_native_parameter_repeated_u32(&mut slots, 2, 0x8000_1234).unwrap();
        assert_eq!(slots[3], [0x8000_1234; 4]);
        write_native_parameter_4_words(&mut slots, 3, [0x1020_3040, 0x5060_7080, 0x90a0_b0c0, 0xd0e0_f000]).unwrap();
        assert_eq!(slots[4], [0x1020_3040, 0x5060_7080, 0x90a0_b0c0, 0xd0e0_f000]);
        write_native_parameter_u32(&mut slots, 0, 0xdead_cafe).unwrap();
        assert_eq!(slots[1], [0xdead_cafe, 0, 0, 0]);
        write_native_parameter_u32(&mut slots, -1, 0x1234_5678).unwrap();
        assert_eq!(slots[0], [0x1234_5678, 0, 0, 0]);

        let before = slots;
        assert!(write_native_parameter_u64(&mut slots, -2, 1).is_err());
        assert_eq!(slots, before);
        let too_short = ParameterBlock {
            entries: vec![(0, [1; 4])],
            value_count_word: 2,
            id_count_word: 1,
            value_capacity_word: 0x100,
            id_capacity_word: 0x100,
        };
        assert!(too_short.write_entries_to_slots(&mut slots).is_err());
        assert_eq!(slots, before);
    }

    #[test]
    fn native_batch_row_first_qword_matches_the_native_accessor() {
        let mut row = [0; 14];
        row[0] = 0x7654_3210;
        row[1] = 0xfedc_ba98;
        assert_eq!(first_qword_of_native_surface_batch_row(&row), 0xfedc_ba98_7654_3210);
        let mut rows = NativeSurfaceBatchRowList::with_capacity(1, 0).unwrap();
        let index = rows.push_zeroed_row(false, true).unwrap();
        *rows.row_mut(index).unwrap() = row;
        assert_eq!(rows.first_qword(index), Some(0xfedc_ba98_7654_3210));
    }

    #[test]
    fn constructor_range_matches_original_qword_store() {
        let cases = include_str!("../analysis/original_model_info_default_cases.txt");
        let mut checked = 0;
        for line in cases.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            assert_eq!(fields.len(), 3);
            assert_eq!(u32::from_str_radix(fields[1], 16).unwrap(), 0x3fff0010);
            assert_eq!(u32::from_str_radix(fields[2], 16).unwrap(), INITIAL_MODEL_MAX_VISIBLE_RANGE.to_bits());
            checked += 1;
        }
        assert_eq!(checked, 4);
    }

    #[test]
    fn parameter_float_matches_original_global_defaults_and_overrides() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_parm_float_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |i: usize| fields[i].parse::<u32>().unwrap();
            if fields[0] == "copy" {
                let source = [u(1), u(2), u(3), u(4)];
                let copied = source;
                assert_eq!(copied, source);
            } else {
                let mut defaults = [[0; 4]; 12];
                defaults[u(1) as usize][0] = u(3);
                let overrides: Vec<_> = (0..u(4) as usize).map(|i| (u(5 + i * 2) as u16, [u(6 + i * 2), 0, 0, 0])).collect();
                assert_eq!(parameter_float("emissiveintensity", u(1), u(2), &defaults, &overrides).unwrap().to_bits(), u(fields.len() - 1), "{line}");
            }
            checked += 1;
        }
        assert_eq!(checked, 305);
        assert_eq!(parameter_float("bad", 0, 1, &[[0; 4]], &[]).unwrap_err(), "idParmBlock::GetFloat: bad is not a vector parm");
        assert!(parameter_float("missing", 1, 0, &[[0; 4]], &[]).is_err());
    }

    #[test]
    fn parameter_removal_matches_original_paired_arrays() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_parm_clear_cases.txt").lines() {
            let fields: Vec<u32> = line.split_whitespace().map(|value| value.parse().unwrap()).collect();
            let id = fields[0];
            let count = fields[2] as usize;
            let slot = |base: usize| (fields[base] as u16, std::array::from_fn(|lane| fields[base + 1 + lane]));
            let mut overrides: Vec<_> = (0..count).map(|index| slot(3 + index * 5)).collect();
            let capacity = overrides.capacity();
            remove_parameter(&mut overrides, id);
            assert_eq!(overrides.capacity(), capacity, "{line}");
            let base = 3 + count * 5;
            assert_eq!(fields[base] & 0xffffff, overrides.len() as u32, "{line}");
            assert_eq!(fields[base + 1] & 0xffffff, overrides.len() as u32, "{line}");
            let expected: Vec<_> = (0..fields[base + 2] as usize).map(|index| slot(base + 3 + index * 5)).collect();
            assert_eq!(overrides, expected, "{line}");
            assert_eq!(fields.len(), base + 3 + expected.len() * 5);
            if id < 6 {
                let defaults = [[0x3e800000, 0, 0, 0]; 6];
                assert_eq!(parameter_float("test", id, 0, &defaults, &overrides).unwrap().to_bits(), 0x3e800000);
            }
            checked += 1;
        }
        assert_eq!(checked, 168);
    }

    #[test]
    fn binding_compaction_and_selection_match_native() {
        let mut operations = [0; 3];
        for line in include_str!("../analysis/original_material_binding_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |index: usize| fields[index].parse::<u32>().unwrap();
            match fields[0] {
                "D" | "W" => {
                    let count = u(4) as usize;
                    let whitelist_count = u(5) as usize;
                    let whitelist: Vec<_> = (0..whitelist_count).map(|i| u(6 + i)).collect();
                    let base = 6 + whitelist_count;
                    let slot = |offset: usize| (u(offset) as u16, std::array::from_fn(|lane| u(offset + 1 + lane)));
                    let mut block = ParameterBlock {
                        entries: (0..count).map(|i| slot(base + 5 * i)).collect(),
                        value_count_word: u(1), id_count_word: u(2),
                        value_capacity_word: (u(3) << 8) | 0x13,
                        id_capacity_word: (u(3) << 8) | 0x3f,
                    };
                    let capacity = block.entries.capacity();
                    if fields[0] == "D" { block.deduplicate().unwrap(); }
                    else { block.filter(&whitelist).unwrap(); }
                    let expected = base + count * 5;
                    assert_eq!(block.value_count_word, u(expected), "{line}");
                    assert_eq!(block.id_count_word, u(expected + 1), "{line}");
                    let slots: Vec<_> = (0..u(expected + 2) as usize)
                        .map(|i| slot(expected + 3 + i * 5)).collect();
                    assert_eq!(block.entries, slots, "{line}");
                    assert_eq!(block.entries.capacity(), capacity);
                    assert_eq!(block.value_capacity_word, (u(3) << 8) | 0x13);
                    assert_eq!(block.id_capacity_word, (u(3) << 8) | 0x3f);
                    if let Some(&(id, value)) = block.entries.last() {
                        let defaults = vec![[0; 4]; usize::from(id) + 1];
                        assert_eq!(block.float("native", u32::from(id), 0, &defaults).unwrap().to_bits(), value[0]);
                    }
                    operations[0] += 1;
                }
                "S" => {
                    let group = material_parameter_group(u(1) as u8, fields[2].parse().unwrap());
                    if u(3) != 0 { assert_eq!(group, MaterialParameterGroup::Base); }
                    else if let MaterialParameterGroup::Effect(selector) = group {
                        assert_eq!(i32::from(selector) * 32, fields[4].parse::<i32>().unwrap());
                    } else { panic!("{line}"); }
                    operations[1] += 1;
                }
                "P" => operations[2] += 1,
                _ => panic!("{line}"),
            }
        }
        assert_eq!(operations, [630, 768, 65536]);
    }

    #[test]
    fn prepared_material_groups_feed_the_existing_property_getter() {
        let block = |word| ParameterBlock { entries: vec![(0, [word, 0, 0, 0])],
            value_count_word: 1, id_count_word: 1, value_capacity_word: 256, id_capacity_word: 256 };
        let mut material = MaterialParameterBlocks { base: block(0x3f000000),
            effects: vec![block(0x3f800000)], flags_49: 0x40 };
        assert_eq!(material.selected(-1).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 0.5);
        assert_eq!(material.selected(0).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 1.0);
        assert!(material.selected(-2).is_err());
        assert!(material.selected(1).is_err());
        material.flags_49 = 0;
        assert_eq!(material.selected(127).unwrap().float("value", 0, 0, &[[0; 4]]).unwrap(), 0.5);
        let mut unsupported = block(0);
        unsupported.value_capacity_word = 0;
        assert!(unsupported.deduplicate().is_err());
        assert!(unsupported.filter(&[]).is_err());
    }

    #[test]
    fn resource_union_matches_original() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_static_resource_bounds_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let count = fields[0].parse::<usize>().unwrap();
            let f = |index: usize| f32::from_bits(fields[index].parse().unwrap());
            let previous = std::array::from_fn(|side| std::array::from_fn(|axis| f(1 + side * 3 + axis)));
            let surfaces: Vec<_> = (0..count).map(|surface| std::array::from_fn(|side| std::array::from_fn(|axis| f(7 + surface * 6 + side * 3 + axis)))).collect();
            let actual = accumulate_resource_bounds(previous, &surfaces);
            let expected: [[f32; 3]; 2] = std::array::from_fn(|side| std::array::from_fn(|axis| f(7 + count * 6 + side * 3 + axis)));
            assert_eq!(actual.map(|row| row.map(f32::to_bits)), expected.map(|row| row.map(f32::to_bits)), "{line}");
            checked += 1;
        }
        assert_eq!(checked, 73);
    }

    #[test]
    fn helpers_match_installed_executable() {
        let rows = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.01, 1.0],
            [0.0, 0.0, -0.101, 0.0],
        ];
        let state = crate::tome::RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let query = NoDepthAabbQuery::from_state(&state).unwrap();
        let mut checked = 0;
        for line in include_str!("../analysis/original_render_model_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let u = |i: usize| fields[i].parse::<u32>().unwrap();
            let f = |i: usize| fields[i].parse::<f32>().unwrap();
            match fields[0] {
                "set" | "clear" => {
                    let mut flags = u(1);
                    if fields[0] == "set" {
                        request_flags(&mut flags, u(2));
                    } else {
                        clear_flags(&mut flags, u(2));
                    }
                    assert_eq!(flags, u(3), "{line}");
                }
                "fade" => {
                    let result = distance_fade(f32::from_bits(u(1)), f32::from_bits(u(2)), f32::from_bits(u(3)));
                    let expected = f32::from_bits(u(4));
                    if expected.is_nan() {
                        assert!(result.is_nan(), "{line}");
                    } else {
                        assert_eq!(result.to_bits(), expected.to_bits(), "{line}");
                    }
                }
                "tree" => {
                    let mut root = VisibilityNode {
                        model_flags: Some(u(1)),
                        children: vec![VisibilityNode {
                            model_flags: None,
                            children: vec![VisibilityNode { model_flags: Some(u(1)), children: vec![] }],
                        }],
                    };
                    root.set_visible(u(2) != 0);
                    assert_eq!(root.model_flags, Some(u(3)), "{line}");
                    assert_eq!(root.children[0].children[0].model_flags, Some(u(4)), "{line}");
                }
                "patch" => {
                    assert_eq!(square_patch_visible(&query, [f(1), f(2), f(3)], f(4), f(5), f(6)).unwrap(), u(7) != 0, "{line}");
                }
                kind => panic!("unknown native model-helper case {kind}"),
            }
            checked += 1;
        }
        assert_eq!(checked, 476);
    }

    #[test]
    fn registry_prefix_matches_installed_original_instructions() {
        let mut count = 0;
        for row in include_str!("../analysis/original_model_registry_cases.txt").lines() {
            let fields: Vec<&str> = row.split_whitespace().collect();
            match fields[0] {
                "resource" | "empty" => {
                    let bounds = std::array::from_fn(|side| std::array::from_fn(|axis| f32::from_bits(fields[1 + side * 3 + axis].parse().unwrap())));
                    let actual = if fields[0] == "empty" { static_resource_bounds(None) } else { static_resource_bounds(Some(resource_bounds(&bounds))) };
                    assert_eq!(actual.map(|side| side.map(f32::to_bits)), bounds.map(|side| side.map(f32::to_bits)));
                    if fields[0] == "resource" { assert!(std::ptr::eq(actual, &bounds)); }
                }
                "prefix" => {
                    let flags = fields[1].parse().unwrap();
                    let shape = fields[2].parse().unwrap();
                    let region = std::array::from_fn(|axis| f32::from_bits(fields[3 + axis].parse().unwrap()));
                    let camera = std::array::from_fn(|axis| f32::from_bits(fields[7 + axis].parse().unwrap()));
                    let accepted = model_eligible(flags, shape) && in_model_region(camera, region);
                    assert_eq!(accepted, fields[10] == "1", "native registry case {count}");
                }
                _ => panic!("unknown original registry case"),
            }
            count += 1;
        }
        assert_eq!(count, 16_388);
        // A prepared record stays eligible even when pending bit1 is set.
        assert!(model_eligible(5, 0));
        assert!(!model_eligible(1, 0x10));
    }

    #[test]
    fn geometry_matches_installed_original_operations() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_model_geometry_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let bits = |index: usize| fields[index].parse::<u32>().unwrap();
            let value = |index: usize| f32::from_bits(bits(index));
            let vector = |start: usize| std::array::from_fn(|axis| value(start + axis));
            let basis = |start: usize| std::array::from_fn(|axis| vector(start + axis * 3));
            let (actual, expected): (Vec<f32>, Vec<f32>) = match fields[0] {
                "base" => { assert_eq!(base_callback() as u32, bits(1)); checked += 1; continue; }
                "matrix" => (affine_rows(vector(1), basis(4), vector(13)).into_iter().flatten().collect(), (16..32).map(value).collect()),
                "bounds" | "static_bounds" => (transformed_bounds([vector(1), vector(4)], vector(7), basis(10), vector(19)).into_iter().flatten().collect(), (22..28).map(value).collect()),
                "region" => (range_region([vector(1), vector(4)], value(7)).to_vec(), (8..12).map(value).collect()),
                kind => panic!("unknown native model geometry case {kind}"),
            };
            assert_eq!(actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "{line}");
            checked += 1;
        }
        assert_eq!(checked, 355);
    }

    #[test]
    fn masked_requests_do_not_fill_partially_set_masks() {
        let mut flags = 2;
        request_flags(&mut flags, 6);
        assert_eq!(flags, 2);
        clear_flags(&mut flags, 6);
        assert_eq!(flags, 1);
        clear_flags(&mut flags, 1);
        assert_eq!(flags, 0);
    }

    #[test]
    fn visibility_reaches_children_without_parent_model() {
        let mut root = VisibilityNode {
            model_flags: None,
            children: vec![VisibilityNode { model_flags: Some(0), children: vec![] }],
        };
        root.set_visible(true);
        assert_eq!(root.children[0].model_flags, Some(3));
        root.set_visible(false);
        assert_eq!(root.children[0].model_flags, Some(1));
    }

    #[test]
    fn fade_matches_endpoints_and_unordered_machine_comparisons() {
        assert_eq!(distance_fade(0.0, 1.0, -1.0), 0.0);
        assert_eq!(distance_fade(0.0, 1.0, 0.5), 0.5);
        assert_eq!(distance_fade(0.0, 1.0, 2.0), 1.0);
        assert!(distance_fade(0.0, 0.0, 0.0).is_nan());
    }

    #[test]
    fn surface_reduction_clears_only_derived_bits_and_ors_all_surfaces() {
        let a = SurfaceFlags { byte_48: 0, byte_4b: 4, byte_4c: 0, parameter_14229d460: 0.0 };
        let b = SurfaceFlags { byte_48: 1, byte_4b: 0, byte_4c: 0x41, parameter_14229d460: 0.002 };
        assert_eq!(reduce_surface_flags([0xff, 0xff], &[]), [0x0f, 0xfe]);
        assert_eq!(reduce_surface_flags([0xff, 0xff], &[a]), [0x0f, 0xfe]);
        assert_eq!(reduce_surface_flags([0, 4], &[b, a]), [0xf0, 5]);
    }
}
