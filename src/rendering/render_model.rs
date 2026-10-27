//! Original render-model flag and finite query helpers.
use crate::tome::NoDepthAabbQuery;
pub use crate::storage::{
    allocate_zeroed_pointers as allocate_zeroed_surface_pointers,
    reallocate_pointers as reallocate_surface_pointers,
    reserve_index as reserve_surface_candidate_row_index, PointerList as SurfacePointerList,
};
use crate::storage::growth_capacity as surface_candidate_growth_capacity;


/// FUN_141374630 registers r_multiSamples="0" (no multisampled FBO).
/// FUN_140347410 copies the setting into renderer state. This default must
/// not inherit Bevy's four-sample default.
pub const DEFAULT_MULTI_SAMPLES: i32 = 0;

/// FUN_140346720: the physical texture sample count is at least one.
pub fn renderer_sample_count(configured: i32) -> u32 {
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
    array
        .values
        .get(..count)
        .ok_or_else(|| "surface sort key packed count exceeds its supplied data".to_owned())
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
        if left < right {
            -1
        } else {
            1
        }
    } else {
        0
    }
}

fn surface_signed_order(left: i64, right: i64) -> u32 {
    if left < right {
        u32::MAX
    } else if left > right {
        1
    } else {
        0
    }
}

fn surface_unsigned_order(left: u64, right: u64) -> u32 {
    if left < right {
        u32::MAX
    } else if left > right {
        1
    } else {
        0
    }
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
    if order != 0 {
        return Ok(order as u32);
    }
    if left.flags[1] != right.flags[1] {
        return Ok(if left.flags[1] != 0 { u32::MAX } else { 1 });
    }
    for (left, right) in left.signed_keys.into_iter().zip(right.signed_keys) {
        let order = surface_signed_order(left, right);
        if order != 0 {
            return Ok(order);
        }
    }
    let order = surface_unsigned_order(left.unsigned_key, right.unsigned_key);
    if order != 0 {
        return Ok(order);
    }
    Ok(if surface_nested_key_equal(&left.nested, &right.nested)? {
        0
    } else {
        1
    })
}

/// FUN_14033e520: rightmost exact-key search used by the native sorted list.
/// It advances on compare <= 0 and returns -1 unless the final row compares equal.
pub fn surface_key_find_rightmost_equal(
    entries: Option<&[SurfaceSortKeyView<'_>]>,
    count: i32,
    target: &SurfaceSortKeyView<'_>,
) -> Result<i32, String> {
    let Some(entries) = entries else {
        return Ok(-1);
    };
    if count == 0 {
        return Ok(-1);
    }
    if entries.is_empty() {
        return Err("surface key search reads entry zero from an empty slice".to_owned());
    }
    if count > 0 && count as usize > entries.len() {
        return Err("surface sort count exceeds its supplied entries".to_owned());
    }
    if count < 0 {
        return Ok(if surface_key_compare(&entries[0], target)? == 0 {
            0
        } else {
            -1
        });
    }
    let mut index = 0i32;
    let mut remaining = count;
    while remaining > 1 {
        let half = remaining >> 1;
        let order = surface_key_compare(&entries[(index + half) as usize], target)? as i32;
        if order <= 0 {
            index += half;
        }
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
    if count < 1 {
        return Ok(0);
    }
    let Some(entries) = entries else {
        return Err("surface lower-bound search received a null list".to_owned());
    };
    if count as usize > entries.len() {
        return Err("surface sort count exceeds its supplied entries".to_owned());
    }
    let mut index = 0i32;
    let mut remaining = count;
    loop {
        let half = remaining >> 1;
        let order = surface_key_compare(&entries[(index + half) as usize], target)? as i32;
        if order < 0 {
            index += half;
        }
        remaining -= half;
        if half == 0 {
            return Ok(index + i32::from(order < 0));
        }
    }
}

/// FUN_14033ecd0 (`SurfaceCompare<1>` slot +0x18) returns zero for every
/// signed count and valid key pair. This models only its observable index;
/// native object-pointer reads at +0x74 are not reproduced here.
pub const fn surface_compare_1_index_result() -> u32 {
    0
}

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
    surface: &DepthAlphaSurface<'_>,
    pass: u8,
) -> Result<Option<DepthAlphaDrawPlan>, String> {
    if pass > 2 {
        return Err("depth-alpha pass must be 0, 1 or 2".to_owned());
    }
    let alpha = surface.flags49 & 1 != 0;
    let program = if pass == 2 {
        if !alpha {
            return Ok(None);
        }
        surface.alternate_program
    } else {
        let slot =
            if pass == 0 && alpha { 4 } else { 6 } + u32::from(surface.special_vertex_format);
        surface_lookup_3119a0(
            surface.default_programs,
            surface.override_programs,
            surface.flags49,
            slot,
            u8::from(surface.has_optional_mesh),
            surface.selector,
        )?
    };
    if program == 0 {
        return Ok(None);
    }
    let draw = depth_alpha_draw_state(surface.draw_state, surface.flags49, pass)?;
    Ok(Some(DepthAlphaDrawPlan {
        program,
        draw_state: draw.state,
        stencil_reference: draw.reference,
        marks_alpha: draw.marks_alpha,
    }))
}

pub struct DepthAlphaDrawState {
    pub state: [u32; 9],
    pub reference: u8,
    pub marks_alpha: bool,
}
///321860's state overrides after the source owner admitted a real program.
///Backend draws reuse this without constructing synthetic pointer banks.
pub fn depth_alpha_draw_state(base: [u32; 9], flags49: u8, pass: u8) -> Result<DepthAlphaDrawState, String> {
    if pass > 2 { return Err("depth-alpha pass must be 0, 1 or 2".into()); }
    let mut state = base;
    state[0] &= 0xfe1f_ffff;
    state[1] &= 0xfe1f_ffff;
    state[4] = state[4] & 0xff | 0xe80e_8000;
    state[5] = if pass == 2 {
        state[5] & 0xff80_80ef | 0x0080_8028
    } else {
        state[5] & 0xff02_02ff | 0x0002_0228
    };
    let marks_alpha = pass == 0 && flags49 & 1 != 0;
    if marks_alpha {
        state[0] = base[0] & 0xfe1f_fe23 | 0x0040_0023;
    }
    Ok(DepthAlphaDrawState {
        state,
        reference: if pass == 2 { 0x80 } else { 2 },
        marks_alpha,
    })
}

// Shared declaration logic also serves the asset-audit tools; no renderer dependency.
pub use crate::material::{blend_enabled, shadow_program_source, ShadowProgramSource};

/// FUN_140329120's dynamic-model program/state preparation only. The existing
/// pointer-bank selector is identical; shadow slots are 10/11. Bounds testing,
/// material production and draw submission remain the caller's responsibility.
/// `special_vertex_format` is the same native +0x90 template comparison used
/// by depth-alpha; `alternate_program` is not used by this pass. The context
/// byte is the MVP determinant sign written by FUN_14010b310, not an enabled
/// renderer flag. Prefer `prepare_shadow_model_draw_for_mvp` with actual rows.
#[derive(Debug, PartialEq, Eq)]
pub struct ShadowModelDrawPlan {
    pub program: u64,
    pub draw_state: [u32; 9],
}

/// FUN_140268b10 selects `m_staticModelPerLOD[slot]`: values at least 2 use
/// slot 2, while negative values are left unchanged by the original branch.
/// Callers must prove the returned slot is nonnegative before indexing.
pub const fn static_model_lod_slot(slot: i32) -> i32 {
    if slot < 2 {
        slot
    } else {
        2
    }
}

/// Port of FUN_140268780: slots 1 and 2 at native offsets `+0x1c70` and
/// `+0x1c78` are tested for non-null; slot 0 is deliberately not considered.
pub fn static_model_has_lod1_or_2<T>(model_per_lod: &[Option<T>; 3]) -> bool {
    model_per_lod[1].is_some() || model_per_lod[2].is_some()
}

/// State change requested by FUN_140268b10. `surface_list_byte_offset` is
/// relative to the native per-LOD surface-list base at `+0x1c90`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticModelLodSwitch<T> {
    pub slot: usize,
    pub selected_model: T,
    pub surface_list_byte_offset: usize,
    pub rebuild_surfaces: bool,
    pub callback_vtable_offset: usize,
    pub callback_argument: i32,
}

/// Port of FUN_140268b10's model-pointer change decision. The native function
/// updates active model `+0x1c50`, selects the matching surface list at
/// `+0x1c90 + slot*0x10`, rebuilds surface/bounds records, then calls vtable
/// `+0x28` with argument 0. This returns that source-backed action; the owner
/// still applies pointers and invokes the callbacks. Invalid negative slots
/// are declined instead of performing native unchecked pointer arithmetic.
pub fn static_model_lod_switch<T: Copy + Eq>(
    requested_slot: i32,
    model_per_lod: &[Option<T>; 3],
    active_model: Option<T>,
) -> Option<StaticModelLodSwitch<T>> {
    let slot = usize::try_from(static_model_lod_slot(requested_slot)).ok()?;
    let selected_model = model_per_lod[slot]?;
    if active_model == Some(selected_model) {
        return None;
    }
    Some(StaticModelLodSwitch {
        slot,
        selected_model,
        surface_list_byte_offset: slot * 0x10,
        rebuild_surfaces: true,
        callback_vtable_offset: 0x28,
        callback_argument: 0,
    })
}

/// FUN_14087c730 refreshes static-model LOD only when `+0xd4 & 1` and
/// `+0x294 != 0`.
pub const fn static_model_lod_refresh_required(flags_d4: u8, actual_level: i32) -> bool {
    flags_d4 & 1 != 0 && actual_level != 0
}

/// FUN_14087c730 registers with the dynamic LOD manager only when all three
/// native gates are clear: `+0xd2 & 0x40`, `+0xd4 & 1`, and `+0xd1 & 1`.
pub const fn lod_manager_registration_allowed(
    flags_d2: u8,
    flags_d4: u8,
    flags_d1: u8,
) -> bool {
    flags_d2 & 0x40 == 0 && flags_d4 & 1 == 0 && flags_d1 & 1 == 0
}

/// Destination/action after FUN_14064cd70 has evaluated model and child gates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LodRegistrationAction {
    PositionList,
    BoundsList,
    SetLevel8,
    NoAction,
}

/// Ports FUN_14064cd70's final classification branch. The caller must provide
/// the outcomes of its native model/child eligibility checks; this does not
/// perform those checks or push into the manager's lists.
pub const fn lod_registration_action(
    has_model_pointer: bool,
    model_variant_eligible: bool,
    child_eligible: bool,
) -> LodRegistrationAction {
    let model_eligible = has_model_pointer && model_variant_eligible;
    if model_eligible || child_eligible {
        if has_model_pointer {
            LodRegistrationAction::BoundsList
        } else {
            LodRegistrationAction::PositionList
        }
    } else if !has_model_pointer {
        LodRegistrationAction::SetLevel8
    } else {
        LodRegistrationAction::NoAction
    }
}

/// FUN_1408792e0 subtracts the integer at object offset +0x17c, clamps to
/// 0..4, then FUN_140268b10 caps the static-model array slot at 2.
pub fn relative_static_model_lod_slot(actual_level: i32, field_0x17c: i32) -> i32 {
    static_model_lod_slot(actual_level.wrapping_sub(field_0x17c).clamp(0, 4))
}

/// Results of FUN_1408792e0's three virtual render-model predicates. These
/// labels mirror the native predicate arguments, not inferred C++ class names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LodModelPredicate {
    Predicate2,
    Predicate3,
    Predicate7,
    Other,
}

/// Model-side candidate gate in FUN_14064cd70. Predicate 2 uses the low-24-bit
/// count at `[model + 0x1fe8] + 0x70`; predicate 3 checks this model's LOD slots
/// 1/2, and predicate 7 accepts any listed model with either slot present.
/// The caller resolves the native virtual predicate and list into these inputs.
pub fn lod_model_variant_eligible<T>(
    predicate: LodModelPredicate,
    predicate2_raw_count: u32,
    predicate3_model_per_lod: &[Option<T>; 3],
    predicate7_models_per_lod: &[[Option<T>; 3]],
) -> bool {
    match predicate {
        LodModelPredicate::Predicate2 => predicate2_raw_count & 0x00ff_ffff > 1,
        LodModelPredicate::Predicate3 => {
            static_model_has_lod1_or_2(predicate3_model_per_lod)
        }
        LodModelPredicate::Predicate7 => predicate7_models_per_lod
            .iter()
            .any(static_model_has_lod1_or_2),
        LodModelPredicate::Other => false,
    }
}

/// Child-side gate in FUN_14064cd70. The flag path requires both low bits
/// encoded in mask 0x0c and a negative signed value; otherwise native falls
/// back to the child's vtable +0x60 result.
pub const fn lod_child_registration_eligible(
    field_0x1c: i32,
    vtable_0x60_returns_nonzero: bool,
) -> bool {
    (((field_0x1c as u8) & 0x0c) == 0x0c && field_0x1c < 0) || vtable_0x60_returns_nonzero
}

/// Observable model-selection action from FUN_1408792e0. Applying model
/// pointers, surfaces, and virtual callbacks remains the native owner's job.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LodModelAction {
    None,
    Predicate2 {
        selected_index: i32,
        callback_argument: i32,
    },
    Predicate3 {
        static_model_slot: i32,
    },
    Predicate7 {
        static_model_slot: i32,
        model_count: u32,
    },
}

/// Exact source-level dispatch in FUN_1408792e0. `has_active_model` models its
/// null check; counts are the native low-24-bit list counts. Predicate2 stores
/// an index capped to count-1 and calls vtable +0x28 with argument 4; Predicate3
/// selects one static-model slot; Predicate7 applies the slot to every entry.
pub fn lod_model_action(
    actual_level: i32,
    field_0x17c: i32,
    has_active_model: bool,
    predicate: LodModelPredicate,
    predicate2_model_count: u32,
    predicate7_model_count: u32,
) -> LodModelAction {
    if actual_level >= 6 || !has_active_model {
        return LodModelAction::None;
    }
    let requested_index = actual_level.wrapping_sub(field_0x17c).clamp(0, 4);
    match predicate {
        LodModelPredicate::Predicate2 => {
            let last_index = ((predicate2_model_count & 0x00ff_ffff) as i32).wrapping_sub(1);
            LodModelAction::Predicate2 {
                selected_index: requested_index.min(last_index),
                callback_argument: 4,
            }
        }
        LodModelPredicate::Predicate3 => LodModelAction::Predicate3 {
            static_model_slot: static_model_lod_slot(requested_index),
        },
        LodModelPredicate::Predicate7 => {
            let model_count = predicate7_model_count & 0x00ff_ffff;
            if model_count == 0 {
                LodModelAction::None
            } else {
                LodModelAction::Predicate7 {
                    static_model_slot: static_model_lod_slot(requested_index),
                    model_count,
                }
            }
        }
        LodModelPredicate::Other => LodModelAction::None,
    }
}

const LOD_MASK_TO_LEVEL: [u8; 16] = [0, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0];

/// Per-view values prepared by FUN_14064cff0. Distance fields are already
/// squared exactly as in the native batch input; the native CVar defaults and
/// selected preset are owned by a different path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodViewParameters {
    pub primary_position: [f32; 3],
    pub alternate_position: [f32; 3],
    pub use_alternate_position: bool,
    pub fov_sin_half_squared: f32,
    pub level_distances_squared: [f32; 4],
    pub far_distance_squared: f32,
    pub force_visible_distance_squared: f32,
}

/// Numeric fields read by FUN_14064c1a0. This is a value snapshot, not a claim
/// about the native object's type or layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodBoundsInput {
    pub minimum: [f32; 3],
    pub maximum: [f32; 3],
    pub flags_1c30: u8,
    pub flags_1c31: u8,
    pub bit_1_at_0xd0: bool,
    pub current_optimal_level: i32,
}

/// Numeric fields read by FUN_14064c3a0 from its position-list entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodPositionInput {
    pub position: [f32; 3],
    pub current_optimal_level: i32,
}

/// One evaluator result. The native caller ORs `needs_update` into its batch
/// dirty byte; it later publishes the new optimal level through FUN_14087bb70.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LodDecision {
    pub optimal_level: i32,
    pub needs_update: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LodLevelUpdate {
    pub optimal_level: i32,
    pub actual_level: i32,
    pub actual_changed: bool,
}

/// FUN_14087bb70 calls each child's virtual +0x68 only when its +0x1c field
/// satisfies `(byte_field & 0x0c) > 7 && signed_field < 0`, and actual changes.
pub const fn lod_child_update_args(
    previous_actual: i32,
    actual: i32,
    child_field_1c: i32,
) -> Option<LodChildUpdate> {
    if previous_actual != actual && child_field_1c < 0 && (child_field_1c as u8 & 0x0c) > 7 {
        Some(LodChildUpdate {
            previous_actual,
            actual,
        })
    } else {
        None
    }
}

/// Previous/current actual LOD levels passed to the child's virtual +0x68.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LodChildUpdate {
    pub previous_actual: i32,
    pub actual: i32,
}

/// Pure state update from FUN_14087bb70: store `optimal` and set actual to
/// min(optimal, requested_max); object rebuilds/notifications are separate.
pub const fn lod_level_update(
    previous_actual: i32,
    optimal: i32,
    requested_max: i32,
) -> LodLevelUpdate {
    let actual_level = if optimal < requested_max {
        optimal
    } else {
        requested_max
    };
    LodLevelUpdate {
        optimal_level: optimal,
        actual_level,
        actual_changed: previous_actual != actual_level,
    }
}

/// FUN_14064cff0 computes this from its view FOV before launching either batch.
pub fn lod_fov_sin_half_squared(fov_radians: f32) -> f32 {
    let sine = ((fov_radians * 0.5) as f64).sin() as f32;
    sine * sine
}

fn lod_distance_squared(point: [f32; 3], camera: [f32; 3]) -> f32 {
    let dy = point[1] - camera[1];
    let dx = point[0] - camera[0];
    let dz = point[2] - camera[2];
    dy * dy + dx * dx + dz * dz
}

fn lod_min_view_distance_squared(point: [f32; 3], view: &LodViewParameters) -> f32 {
    let primary = lod_distance_squared(point, view.primary_position);
    if view.use_alternate_position {
        let alternate = lod_distance_squared(point, view.alternate_position);
        if alternate <= primary {
            return alternate;
        }
    }
    primary
}

fn lod_select_level(
    fov_scaled_distance_squared: f32,
    level_distances_squared: [f32; 4],
    far_distance_squared: f32,
) -> i32 {
    let mut mask = 0usize;
    for (index, threshold) in level_distances_squared.into_iter().enumerate() {
        mask |= usize::from(fov_scaled_distance_squared < threshold) << index;
    }
    if mask != 0 {
        LOD_MASK_TO_LEVEL[mask] as i32
    } else if far_distance_squared < fov_scaled_distance_squared {
        5
    } else {
        4
    }
}

/// FUN_14064c1a0: bounds-aware camera-distance evaluator. Native bounds and
/// flag names are unresolved; the fields deliberately retain source offsets.
pub fn lod_level_for_bounds(
    view: &LodViewParameters,
    input: LodBoundsInput,
) -> LodDecision {
    let center = [
        (input.minimum[0] + input.maximum[0]) * 0.5,
        (input.minimum[1] + input.maximum[1]) * 0.5,
        (input.minimum[2] + input.maximum[2]) * 0.5,
    ];
    let mut distance_squared = lod_min_view_distance_squared(center, view);
    let extent_y = input.maximum[1] - input.minimum[1];
    let extent_x = input.maximum[0] - input.minimum[0];
    let largest_horizontal_extent = if extent_x <= extent_y {
        extent_y
    } else {
        extent_x
    };
    let half_extent = largest_horizontal_extent * 0.5;
    distance_squared = distance_squared - half_extent * largest_horizontal_extent * 0.5;

    let special_flags = input.flags_1c30 & 1 == 0 && input.flags_1c31 & 0x0c != 0;
    let normal_path = (special_flags || distance_squared <= view.force_visible_distance_squared)
        && !input.bit_1_at_0xd0;
    let optimal_level = if normal_path {
        lod_select_level(
            distance_squared * view.fov_sin_half_squared,
            view.level_distances_squared,
            view.far_distance_squared,
        )
    } else if view.far_distance_squared < distance_squared {
        7
    } else {
        6
    };
    LodDecision {
        optimal_level,
        needs_update: optimal_level != input.current_optimal_level,
    }
}

/// FUN_14064c3a0: direct-position camera-distance evaluator.
pub fn lod_level_for_position(
    view: &LodViewParameters,
    input: LodPositionInput,
) -> LodDecision {
    let distance_squared = lod_min_view_distance_squared(input.position, view);
    let optimal_level = lod_select_level(
        distance_squared * view.fov_sin_half_squared,
        view.level_distances_squared,
        view.far_distance_squared,
    );
    LodDecision {
        optimal_level,
        needs_update: optimal_level != input.current_optimal_level,
    }
}

pub fn prepare_shadow_model_draw(
    surface: &DepthAlphaSurface<'_>,
    bias: [f32; 3],
    frustum_margin: f32,
    model_mvp_flipped: bool,
) -> Result<Option<ShadowModelDrawPlan>, String> {
    let program = surface_lookup_3119a0(
        surface.default_programs,
        surface.override_programs,
        surface.flags49,
        10 + u32::from(surface.special_vertex_format),
        u8::from(surface.has_optional_mesh),
        surface.selector,
    )?;
    if program == 0 {
        return Ok(None);
    }
    let state = selected_shadow_model_draw_state(
        surface.flags49,
        surface.draw_state,
        bias,
        frustum_margin,
        model_mvp_flipped,
    );
    Ok(Some(ShadowModelDrawPlan {
        program,
        draw_state: state,
    }))
}

/// The state half of FUN_140329120, after its actual pointer-bank lookup.
/// Callers with an independently resolved cached program use the same state
/// encoder without inventing native executable program pointers.
pub fn selected_shadow_model_draw_state(
    flags49: u8,
    mut state: [u32; 9],
    bias: [f32; 3],
    frustum_margin: f32,
    model_mvp_flipped: bool,
) -> [u32; 9] {
    state[0] = state[0] & 0xfe1f_f802 | 2;
    // COMISS 0,margin / SETA: NaN is NOT negative. The decompiler's
    // `0 <= margin` expression loses this unordered branch.
    state[5] = state[5] & 0xefff_fff9
        | 1
        | (u32::from(!(frustum_margin < 0.0)) << 28)
        | (u32::from(!model_mvp_flipped) << 27);
    state[6..].fill(0);
    if flags49 & 2 == 0 {
        // CVTTSS2SI returns integer-indefinite, not Rust's saturating cast,
        // for NaN/overflow. The original reorders clamp and slope words.
        state[6] = if (-2147483648.0..2147483648.0).contains(&bias[0]) {
            bias[0] as i32 as u32
        } else {
            0x8000_0000
        };
        state[7] = bias[2].to_bits();
        state[8] = bias[1].to_bits();
    }
    state
}

/// The native 329120 call sequence publishes the MVP rows/handedness before
/// preparing this draw state. Keep the matrix-derived flag paired with its draw.
pub fn prepare_shadow_model_draw_for_mvp(
    surface: &DepthAlphaSurface<'_>,
    bias: [f32; 3],
    frustum_margin: f32,
    mvp: &crate::render_shadow::ShadowMatrix,
) -> Result<Option<ShadowModelDrawPlan>, String> {
    prepare_shadow_model_draw(
        surface,
        bias,
        frustum_margin,
        crate::render_shadow::model_mvp_winding_flipped(mvp),
    )
}

/// One zero-initialized 0xa8-byte row from FUN_140339de0. Bytes not written
/// by its initializer stay deterministically zero here; whole-row equivalence
/// awaits confirming that downstream consumers ignore those gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceCandidateRow {
    pub bytes: [u8; 0xa8],
}

pub fn surface_candidate_row_default() -> SurfaceCandidateRow {
    let mut bytes = [0; 0xa8];
    bytes[1] = 0xff;
    bytes[0x6c..0x70].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[0x74..0x7c].copy_from_slice(&0x7f7f_ffff_u64.to_le_bytes());
    bytes[0x94..0x98].copy_from_slice(&0x3f80_0000_u32.to_le_bytes());
    SurfaceCandidateRow { bytes }
}

impl Default for SurfaceCandidateRow {
    fn default() -> Self {
        surface_candidate_row_default()
    }
}

/// Rust-owned 0xa8 surface candidate rows with the native packed count and
/// capacity header. Rust Vec supplies aligned storage and copy-on-growth; the
/// source allocator's debug tag/pool and uninitialized padding are separate.
/// FUN_14033a270/FUN_14033ad50 storage replacement: allocate defaults at the
/// requested size, copy only the live prefix, then release the old Vec. A
/// failed reserve leaves the original storage untouched, like the native call.
pub fn reallocate_surface_candidate_rows(
    rows: &mut Vec<SurfaceCandidateRow>,
    live_count: u32,
    requested_capacity: u32,
) -> Result<(), String> {
    if requested_capacity > 0x00ff_ffff {
        return Err("surface candidate capacity exceeds 24 bits".to_owned());
    }
    let copy_count = (live_count as usize).min(requested_capacity as usize);
    if copy_count > rows.len() {
        return Err("surface live count exceeds supplied rows".to_owned());
    }
    let mut replacement = Vec::new();
    replacement
        .try_reserve_exact(requested_capacity as usize)
        .map_err(|error| format!("surface row allocation failed: {error}"))?;
    replacement.resize(
        requested_capacity as usize,
        SurfaceCandidateRow::default(),
    );
    replacement[..copy_count].copy_from_slice(&rows[..copy_count]);
    *rows = replacement;
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct SurfaceCandidateRows {
    rows: Vec<SurfaceCandidateRow>,
    count_word: u32,
    capacity_word: u32,
}

impl SurfaceCandidateRows {
    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        if capacity > 0x00ff_ffff {
            return Err("surface candidate capacity exceeds 24 bits".to_owned());
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(capacity as usize)
            .map_err(|error| format!("surface row allocation failed: {error}"))?;
        rows.resize(capacity as usize, SurfaceCandidateRow::default());
        Ok(Self {
            rows,
            count_word: 0,
            capacity_word: (capacity << 8) | u32::from(low_flags),
        })
    }

    pub fn append(
        &mut self,
        row: SurfaceCandidateRow,
        grow_one_at_a_time: bool,
    ) -> Result<u32, String> {
        const COUNT_MASK: u32 = 0x00ff_ffff;
        let count = self.count_word & COUNT_MASK;
        let capacity = self.capacity_word >> 8;
        let mut allocation_succeeds = true;
        if count == capacity {
            let requested =
                surface_candidate_growth_capacity(self.capacity_word, grow_one_at_a_time);
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
            return Err("surface candidate append failed".to_owned());
        }
        let slot = self
            .rows
            .get_mut(index as usize)
            .ok_or_else(|| "surface candidate index exceeds allocated rows".to_owned())?;
        *slot = row;
        Ok(index)
    }

    pub fn rows(&self) -> &[SurfaceCandidateRow] {
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

/// FUN_140335360 stores 0x38-byte rows in this packed list. Fourteen raw
/// u32 lanes preserve the row bytes without guessing their mixed field types.
pub type SurfaceBatchRow = [u32; 14];

/// FUN_1401d9090 returns the first qword of its native row pointer.
pub fn first_qword_of_surface_batch_row(row: &SurfaceBatchRow) -> u64 {
    u64::from(row[0]) | (u64::from(row[1]) << 32)
}

fn allocate_zeroed_surface_rows(
    capacity: u32,
) -> Result<Vec<SurfaceBatchRow>, String> {
    if capacity > 0x00ff_ffff {
        return Err("surface row capacity exceeds 24 bits".to_owned());
    }
    let mut rows = Vec::new();
    rows.try_reserve_exact(capacity as usize)
        .map_err(|error| format!("surface row allocation failed: {error}"))?;
    rows.resize(capacity as usize, [0; 14]);
    Ok(rows)
}

/// Packed 0x38-byte batch rows emitted by FUN_140335360 through
/// FUN_14033dd60/FUN_14033ac90. This models logical row storage; engine
/// allocator tags/ownership remain outside the Rust Vec representation.
#[derive(Clone, Debug, Default)]
pub struct SurfaceBatchRowList {
    rows: Vec<SurfaceBatchRow>,
    count_word: u32,
    capacity_word: u32,
}

impl SurfaceBatchRowList {
    pub fn with_capacity(capacity: u32, low_flags: u8) -> Result<Self, String> {
        Ok(Self {
            rows: allocate_zeroed_surface_rows(capacity)?,
            count_word: 0,
            capacity_word: capacity.wrapping_shl(8) | u32::from(low_flags),
        })
    }

    /// FUN_14033dd60 dispatches packed row-list capacity changes. Its mode-3
    /// helper has no failure return and the caller publishes capacity anyway.
    pub fn set_capacity(
        &mut self,
        requested: i32,
        allocation_succeeds: bool,
    ) -> Result<bool, String> {
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
            return Err("surface row capacity exceeds 24 bits".to_owned());
        }
        let old_capacity = self.capacity_word >> 8;
        if requested == old_capacity {
            return Ok(true);
        }
        let allocation_mode = (self.capacity_word >> 6) & 3;
        if allocation_mode == 3 && old_capacity < requested {
            let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
            if count > old_capacity || count as usize > self.rows.len() {
                return Err("specialized surface row count exceeds storage".to_owned());
            }
            let flags = self.capacity_word as u8;
            if allocation_succeeds {
                if let Ok(mut replacement) = allocate_zeroed_surface_rows(requested) {
                    replacement[..count as usize].copy_from_slice(&self.rows[..count as usize]);
                    self.rows = replacement;
                }
            }
            self.capacity_word = requested.wrapping_shl(8) | u32::from(flags);
            return Ok(true);
        }
        if allocation_mode != 0 {
            return Ok(false);
        }
        let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        if !allocation_succeeds {
            self.rows = Vec::new();
            return Ok(false);
        }
        let mut replacement = match allocate_zeroed_surface_rows(requested) {
            Ok(replacement) => replacement,
            Err(_) => {
                self.rows = Vec::new();
                return Ok(false);
            }
        };
        let copied = count.min(requested) as usize;
        if copied > self.rows.len() {
            return Err("surface row count exceeds supplied storage".to_owned());
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
    pub fn push_zeroed_row(
        &mut self,
        grow_one_at_a_time: bool,
        allocation_succeeds: bool,
    ) -> Result<u32, String> {
        let count = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        let capacity = self.capacity_word >> 8;
        if count > capacity || capacity as usize != self.rows.len() {
            return Err("surface row-list metadata disagrees with storage".to_owned());
        }
        if count == capacity {
            let requested =
                surface_candidate_growth_capacity(self.capacity_word, grow_one_at_a_time);
            if requested <= capacity || requested > 0x00ff_ffff {
                return Err("surface row growth exceeds packed capacity".to_owned());
            }
            if !self.set_capacity(requested as i32, allocation_succeeds)? {
                return Err("surface row allocator rejected growth mode".to_owned());
            }
            if self.capacity_word >> 8 != self.rows.len() as u32 {
                return Err("surface row allocation left a short backing array".to_owned());
            }
        }
        let index = self.count_word & SURFACE_ARRAY_COUNT_MASK;
        self.rows[index as usize] = [0; 14];
        self.count_word = (self.count_word & 0xff00_0000) | (index + 1);
        Ok(index)
    }

    pub fn rows(&self) -> &[SurfaceBatchRow] {
        self.rows
            .get(..(self.count_word & SURFACE_ARRAY_COUNT_MASK) as usize)
            .unwrap_or(&[])
    }

    pub fn row_mut(&mut self, index: u32) -> Option<&mut SurfaceBatchRow> {
        if index >= (self.count_word & SURFACE_ARRAY_COUNT_MASK) {
            return None;
        }
        self.rows.get_mut(index as usize)
    }

    pub fn first_qword(&self, index: u32) -> Option<u64> {
        self.rows()
            .get(index as usize)
            .map(first_qword_of_surface_batch_row)
    }

    pub fn count(&self) -> u32 {
        self.count_word & SURFACE_ARRAY_COUNT_MASK
    }
    pub fn capacity(&self) -> u32 {
        self.capacity_word >> 8
    }
    pub fn packed_header(&self) -> (u32, u32) {
        (self.count_word, self.capacity_word)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceSortedInsert {
    Existing { index: u32 },
    Inserted { index: u32 },
}

/// Compose FUN_14033e520, FUN_14033f0f0 and FUN_14033a9e0: suppress an
/// existing equal surface key; otherwise insert its pointer at the native
/// lower bound while keeping the key and pointer arrays aligned.
pub fn insert_sorted_surface_pointer<'a>(
    sorted_keys: &mut Vec<SurfaceSortKeyView<'a>>,
    pointers: &mut SurfacePointerList,
    target: SurfaceSortKeyView<'a>,
    pointer: u64,
    grow_one_at_a_time: bool,
) -> Result<SurfaceSortedInsert, String> {
    if pointers.count() as usize != sorted_keys.len() {
        return Err("surface key and pointer lists have different counts".to_owned());
    }
    let count = i32::try_from(sorted_keys.len())
        .map_err(|_| "surface sorted list exceeds signed count".to_owned())?;
    let existing = surface_key_find_rightmost_equal(Some(sorted_keys), count, &target)?;
    if existing >= 0 {
        return Ok(SurfaceSortedInsert::Existing {
            index: existing as u32,
        });
    }
    let index = surface_key_lower_bound(Some(sorted_keys), count, &target)? as usize;
    sorted_keys
        .try_reserve(1)
        .map_err(|error| format!("surface key allocation failed: {error}"))?;
    pointers.insert(index as u32, pointer, grow_one_at_a_time)?;
    sorted_keys.insert(index, target);
    Ok(SurfaceSortedInsert::Inserted {
        index: index as u32,
    })
}

/// HashIndex storage used by FUN_14000ad90. The native arrays use 0xffffffff
/// as the empty-chain sentinel; this safe view models logical bucket links,
/// not the engine allocator's tag/pool or owner lifetime.
#[derive(Clone, Debug)]
pub struct SurfaceHashIndex {
    bucket_count: u32,
    chain_capacity: u32,
    growth_granularity: u32,
    bucket_mask: u32,
    cached_state_word: u32,
    bucket_heads: Vec<u32>,
    next_indices: Vec<u32>,
}

impl SurfaceHashIndex {
    /// Construct the native reset state: both pointers refer to the sentinel.
    pub fn lazy(
        bucket_count: u32,
        chain_capacity: u32,
        growth_granularity: u32,
    ) -> Result<Self, String> {
        if bucket_count == 0 || bucket_count > i32::MAX as u32 || chain_capacity > i32::MAX as u32 {
            return Err("surface HashIndex capacities are out of range".to_owned());
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

    pub fn with_capacity(
        bucket_count: u32,
        chain_capacity: u32,
        growth_granularity: u32,
    ) -> Result<Self, String> {
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
            return Err("surface HashIndex capacities are out of range".to_owned());
        }
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(bucket_count as usize)
            .map_err(|error| format!("surface hash allocation failed: {error}"))?;
        buckets.resize(bucket_count as usize, u32::MAX);
        let mut next = Vec::new();
        next.try_reserve_exact(chain_capacity as usize)
            .map_err(|error| format!("surface chain allocation failed: {error}"))?;
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
        if requested <= self.chain_capacity {
            return Ok(());
        }
        let capacity = if self.growth_granularity == 0 {
            if requested < 0x4000_0001 {
                requested.checked_next_power_of_two().unwrap_or(0x7fff_ffff)
            } else {
                0x7fff_ffff
            }
        } else {
            let granularity = self.growth_granularity;
            let rounded = requested
                .checked_sub(1)
                .and_then(|value| value.checked_add(granularity))
                .ok_or_else(|| "surface chain capacity overflow".to_owned())?;
            rounded - rounded % granularity
        };
        if capacity > i32::MAX as u32 {
            return Err("surface chain capacity exceeds signed native range".to_owned());
        }
        self.next_indices
            .try_reserve_exact((capacity - self.chain_capacity) as usize)
            .map_err(|error| format!("surface chain growth failed: {error}"))?;
        self.next_indices.resize(capacity as usize, u32::MAX);
        self.chain_capacity = capacity;
        Ok(())
    }

    /// FUN_14000ad90: lazily reset sentinel storage, grow the chain array as
    /// needed, then prepend this index to the key's masked bucket chain.
    pub fn insert(&mut self, key: u32, index: i32) -> Result<(), String> {
        let index = u32::try_from(index)
            .map_err(|_| "surface HashIndex insertion index is negative".to_owned())?;
        if self.bucket_heads.is_empty() {
            let requested = self.chain_capacity.max(
                index
                    .checked_add(1)
                    .ok_or_else(|| "surface HashIndex index overflow".to_owned())?,
            );
            self.reset(self.bucket_count, requested)?;
        } else if self.chain_capacity <= index {
            self.grow_chain(
                index
                    .checked_add(1)
                    .ok_or_else(|| "surface HashIndex index overflow".to_owned())?,
            )?;
        }
        let bucket = (key & self.bucket_mask) as usize;
        let head = self
            .bucket_heads
            .get_mut(bucket)
            .ok_or_else(|| "surface HashIndex bucket exceeds storage".to_owned())?;
        let next = self
            .next_indices
            .get_mut(index as usize)
            .ok_or_else(|| "surface HashIndex index exceeds storage".to_owned())?;
        *next = *head;
        *head = index;
        Ok(())
    }

    /// 021300: unlink this index from its bucket, then clear its chain link.
    /// A missing index still has its link cleared; lazy sentinel storage is
    /// left untouched. The other entries retain their insertion order.
    pub fn remove(&mut self, key: u32, index: u32) -> Result<(), String> {
        if self.bucket_heads.is_empty() {
            return Ok(());
        }
        let index = index as usize;
        let next = *self.next_indices.get(index)
            .ok_or("surface HashIndex removal index exceeds storage")?;
        let bucket = (key & self.bucket_mask) as usize;
        let mut current = self.bucket_heads[bucket];
        if current == index as u32 {
            self.bucket_heads[bucket] = next;
        } else {
            let mut visited = 0;
            while current != u32::MAX {
                visited += 1;
                if visited > self.next_indices.len() {
                    return Err("surface HashIndex chain is cyclic".into());
                }
                let link = self.next_indices.get_mut(current as usize)
                    .ok_or("surface HashIndex link exceeds storage")?;
                if *link == index as u32 {
                    *link = next;
                    break;
                }
                current = *link;
            }
        }
        self.next_indices[index] = u32::MAX;
        Ok(())
    }

    pub fn bucket_chain(&self, key: u32) -> Result<Vec<u32>, String> {
        if self.bucket_heads.is_empty() {
            return Ok(Vec::new());
        }
        let bucket = (key & self.bucket_mask) as usize;
        let mut current = *self
            .bucket_heads
            .get(bucket)
            .ok_or_else(|| "surface HashIndex bucket exceeds storage".to_owned())?;
        let mut result = Vec::new();
        while current != u32::MAX {
            if result.len() >= self.next_indices.len() {
                return Err("surface HashIndex chain is cyclic".to_owned());
            }
            result.push(current);
            current = *self
                .next_indices
                .get(current as usize)
                .ok_or_else(|| "surface HashIndex link exceeds storage".to_owned())?;
        }
        Ok(result)
    }

    pub fn bucket_heads(&self) -> &[u32] {
        &self.bucket_heads
    }
    pub fn next_indices(&self) -> &[u32] {
        &self.next_indices
    }
    pub fn bucket_mask(&self) -> u32 {
        self.bucket_mask
    }
    pub fn chain_capacity(&self) -> u32 {
        self.chain_capacity
    }
    pub fn cached_state_word(&self) -> u32 {
        self.cached_state_word
    }
}

/// FUN_140359bd0 input scalars after reading the original pointer graph. Field
/// names preserve observed offsets rather than asserting a native Rust layout.
#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceCandidateSortFields {
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
pub fn surface_candidate_sort_payload(
    fields: &SurfaceCandidateSortFields,
) -> [u8; 0x38] {
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
pub struct SurfaceCandidateBranchFields {
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

pub fn surface_candidate_branch_row(
    fields: &SurfaceCandidateBranchFields,
) -> SurfaceCandidateRow {
    let mut row = surface_candidate_row_default();
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
pub struct AabbClipCullResult {
    pub outside_plane_mask: u8,
    pub output_byte: u8,
    pub culled: bool,
}

/// FUN_14010a0d0 with its default mode: transform the eight AABB corners and
/// accumulate the six initialized plane bits (1,2,4,8,16,32). The native
/// result is a cull flag: true if any plane has no corner in its half-space.
pub fn transformed_aabb_clip_cull(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> AabbClipCullResult {
    transformed_aabb_clip_cull_at_depth(matrix, minimum, maximum, 0.0)
}

/// FUN_140109e40's explicit near-depth threshold. Directional model draws
/// use this when their retained projection depth offset is negative.
pub fn transformed_aabb_clip_cull_at_depth(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
    near_depth: f32,
) -> AabbClipCullResult {
    transformed_aabb_clip_cull_bounds(matrix, minimum, maximum, near_depth, false)
}

///10a0d0 mode1 uses zero, rather than -W, for the X/Y lower planes.
///Local model collection tests the light's unit-volume matrix in this mode.
pub fn unit_aabb_clip_cull(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> AabbClipCullResult {
    transformed_aabb_clip_cull_bounds(matrix, minimum, maximum, 0.0, true)
}

#[inline]
fn transformed_aabb_clip_cull_bounds(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
    near_depth: f32,
    unit_xy: bool,
) -> AabbClipCullResult {
    #[cfg(target_arch = "x86_64")]
    {
        // SSE is part of the x86-64 baseline. Each lane is one original
        // corner; preserve 109e40/10a0d0's multiply/add order and ordered
        // comparisons, including NaNs and exact plane boundaries.
        unsafe { aabb_clip_cull_sse(matrix, minimum, maximum, near_depth, unit_xy) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        aabb_clip_cull_scalar(matrix, minimum, maximum, near_depth, unit_xy)
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn aabb_clip_cull_sse(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
    near_depth: f32,
    unit_xy: bool,
) -> AabbClipCullResult {
    use core::arch::x86_64::*;
    // All array accesses have fixed, in-range indices; these operations use
    // registers, with no alignment assumptions or changes to the FP mode.
    unsafe {
        let x = _mm_setr_ps(minimum[0], maximum[0], minimum[0], maximum[0]);
        let y = _mm_setr_ps(minimum[1], minimum[1], maximum[1], maximum[1]);
        let xy: [__m128; 4] = std::array::from_fn(|row| {
            let at = row * 4;
            _mm_add_ps(
                _mm_add_ps(
                    _mm_mul_ps(_mm_set1_ps(matrix[at]), x),
                    _mm_set1_ps(matrix[at + 3]),
                ),
                _mm_mul_ps(_mm_set1_ps(matrix[at + 1]), y),
            )
        });
        let mut inside = [0; 6];
        for z in [minimum[2], maximum[2]] {
            let z = _mm_set1_ps(z);
            let c: [__m128; 4] = std::array::from_fn(|row| {
                _mm_add_ps(_mm_mul_ps(_mm_set1_ps(matrix[row * 4 + 2]), z), xy[row])
            });
            let lower = if unit_xy {
                _mm_setzero_ps()
            } else {
                _mm_xor_ps(c[3], _mm_set1_ps(-0.0))
            };
            let tests = [
                _mm_cmplt_ps(lower, c[0]),
                _mm_cmplt_ps(c[0], c[3]),
                _mm_cmplt_ps(lower, c[1]),
                _mm_cmplt_ps(c[1], c[3]),
                _mm_cmplt_ps(_mm_set1_ps(near_depth), c[2]),
                _mm_cmplt_ps(c[2], c[3]),
            ];
            for (inside, test) in inside.iter_mut().zip(tests) {
                *inside |= _mm_movemask_ps(test);
            }
        }
        let inside_mask = inside
            .into_iter()
            .enumerate()
            .fold(0u8, |mask, (bit, value)| {
                mask | (u8::from(value != 0) << bit)
            });
        let outside_plane_mask = inside_mask ^ 0x3f;
        AabbClipCullResult {
            outside_plane_mask,
            output_byte: outside_plane_mask,
            culled: inside_mask != 0x3f,
        }
    }
}

#[cfg(any(test, not(target_arch = "x86_64")))]
fn aabb_clip_cull_scalar(
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
    near_depth: f32,
    unit_xy: bool,
) -> AabbClipCullResult {
    let mut corners = [[0.0_f32; 4]; 8];
    for (index, corner) in corners.iter_mut().enumerate() {
        let x = if index & 1 == 0 {
            minimum[0]
        } else {
            maximum[0]
        };
        let y = if index & 2 == 0 {
            minimum[1]
        } else {
            maximum[1]
        };
        let z = if index & 4 == 0 {
            minimum[2]
        } else {
            maximum[2]
        };
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
        corners
            .iter()
            .any(|corner| (if unit_xy { 0.0 } else { -corner[3] }) < corner[0]),
        corners.iter().any(|corner| corner[0] < corner[3]),
        corners
            .iter()
            .any(|corner| (if unit_xy { 0.0 } else { -corner[3] }) < corner[1]),
        corners.iter().any(|corner| corner[1] < corner[3]),
        corners.iter().any(|corner| near_depth < corner[2]),
        corners.iter().any(|corner| corner[2] < corner[3]),
    ];
    let mut inside_mask = 0_u8;
    for (bit, inside) in plane_inside.into_iter().enumerate() {
        if inside {
            inside_mask |= 1 << bit;
        }
    }
    let outside_plane_mask = inside_mask ^ 0x3f;
    AabbClipCullResult {
        outside_plane_mask,
        output_byte: outside_plane_mask,
        culled: inside_mask != 0x3f,
    }
}

/// FUN_14033b280: return the selected input-surface indices and mode byte.
/// The count is the native packed header's low 24 bits; filtering preserves
/// original ascending order.
pub fn surface_candidate_iteration_indices(
    packed_count: u32,
    context_flags_18bc: u8,
    pass_filter: i32,
) -> Result<(i8, Vec<u32>), String> {
    let count = packed_count & 0x00ff_ffff;
    let mode = if count < 2 || context_flags_18bc & 0x10 != 0 {
        0
    } else {
        1
    };
    let mut indices = Vec::new();
    if pass_filter < 0 {
        indices
            .try_reserve_exact(count as usize)
            .map_err(|error| format!("surface iteration allocation failed: {error}"))?;
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
pub fn visit_surface_container_rows(
    rows: &[u8],
    packed_count: u32,
    context_flags_18bc: u8,
    pass_filter: i32,
    mut visit: impl FnMut(i8, u32, &[u8; 0x80]) -> Result<(), String>,
) -> Result<usize, String> {
    let count = packed_count & 0x00ff_ffff;
    let mode = if count < 2 || context_flags_18bc & 0x10 != 0 {
        0
    } else {
        1
    };
    let required = (count as usize)
        .checked_mul(0x80)
        .ok_or_else(|| "surface container byte length overflow".to_owned())?;
    if required > rows.len() {
        return Err("surface container is shorter than its packed record count".to_owned());
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
pub struct SurfaceCandidateBranchGates {
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

pub fn surface_candidate_category_key(
    gates: &SurfaceCandidateBranchGates,
    material_flags_4a: u8,
    feature_bits: u32,
) -> Option<u32> {
    if !gates.material_pointer_present
        || !gates.surface_pointer_30_present
        || gates
            .optional_pointer_78_value
            .is_some_and(|value| value == 0)
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
    let enabled = feature_bits
        .wrapping_mul(2)
        .wrapping_shr(u32::from(category))
        & 1
        != 0;
    enabled.then_some(u32::from(category) + 8)
}

/// Resolve FUN_14033b350's optional FUN_140109dd0 test using the exact default
/// clip mode, then apply the remaining observed material/category gates.
pub fn surface_candidate_category_key_with_bounds(
    gates: &SurfaceCandidateBranchGates,
    material_flags_4a: u8,
    feature_bits: u32,
    matrix: &[f32; 16],
    minimum: [f32; 3],
    maximum: [f32; 3],
) -> Option<u32> {
    let mut resolved = *gates;
    if resolved.mode != 0 && !resolved.surface_pointer_40_present {
        resolved.helper_140109dd0_returns_nonzero =
            transformed_aabb_clip_cull(matrix, minimum, maximum).culled;
    }
    surface_candidate_category_key(&resolved, material_flags_4a, feature_bits)
}

/// Inputs from one measured FUN_14033b350 surface branch. The caller must
/// supply source-backed fields; this does not infer runtime pointers or flags.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceCandidateInput {
    pub branch_fields: SurfaceCandidateBranchFields,
    pub gates: SurfaceCandidateBranchGates,
    pub material_flags_4a: u8,
    pub feature_bits: u32,
    pub clip_matrix: [f32; 16],
    pub bounds: [[f32; 3]; 2],
}

/// Port the observed candidate branch as one operation: apply eligibility,
/// write the 0xa8 row, append it, then prepend the row index to its category
/// HashIndex chain. Other FUN_14033b350 branches remain distinct.
pub fn append_surface_candidate_if_eligible(
    input: &SurfaceCandidateInput,
    rows: &mut SurfaceCandidateRows,
    hash_index: &mut SurfaceHashIndex,
    grow_one_at_a_time: bool,
) -> Result<Option<u32>, String> {
    let Some(category_key) = surface_candidate_category_key_with_bounds(
        &input.gates,
        input.material_flags_4a,
        input.feature_bits,
        &input.clip_matrix,
        input.bounds[0],
        input.bounds[1],
    ) else {
        return Ok(None);
    };
    let row = surface_candidate_branch_row(&input.branch_fields);
    append_surface_candidate_to_hash(rows, hash_index, category_key, row, grow_one_at_a_time)
        .map(Some)
}

/// Append one candidate row, then insert its index into the native bucket
/// chain, matching FUN_14033b350's call order.
pub fn append_surface_candidate_to_hash(
    rows: &mut SurfaceCandidateRows,
    hash_index: &mut SurfaceHashIndex,
    category_key: u32,
    row: SurfaceCandidateRow,
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
        return Err("surface pointer vector growth is required".to_owned());
    }
    if capacity as usize > slots.len() || count as usize >= slots.len() {
        return Err("surface pointer vector metadata exceeds supplied storage".to_owned());
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
        let node = nodes
            .get(index)
            .ok_or_else(|| "surface hash node index is out of bounds".to_owned())?;
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
    output: &mut SurfacePointerList,
    grow_one_at_a_time: bool,
) -> Result<(), String> {
    let mut current = head;
    let mut visited = 0usize;
    while let Some(index) = current {
        if visited >= nodes.len() {
            return Err("surface hash chain is cyclic".to_owned());
        }
        let node = nodes
            .get(index)
            .ok_or_else(|| "surface hash node index is out of bounds".to_owned())?;
        output.insert(
            surface_compare_1_index_result(),
            node.surface_pointer,
            grow_one_at_a_time,
        )?;
        current = node.next;
        visited += 1;
    }
    Ok(())
}

/// Read all surface-container pointers in a HashIndex bucket, in the order
/// produced by the native front-insertion callback. HashIndex stores bucket
/// links here but no key comparison, so collisions remain in the same output;
/// exact-key filtering is a separate, not-yet-ported consumer operation.
pub fn prepass_surface_pointers_for_bucket(
    rows: &SurfaceCandidateRows,
    hash_index: &SurfaceHashIndex,
    bucket_key: u32,
) -> Result<Vec<u64>, String> {
    let chain = hash_index.bucket_chain(bucket_key)?;
    if chain.is_empty() {
        return Ok(Vec::new());
    }
    let mut nodes = Vec::new();
    nodes
        .try_reserve_exact(chain.len())
        .map_err(|error| format!("prepass chain allocation failed: {error}"))?;
    for (position, row_index) in chain.iter().copied().enumerate() {
        let row = rows
            .rows()
            .get(row_index as usize)
            .ok_or_else(|| "prepass hash index exceeds candidate rows".to_owned())?;
        let surface_pointer = u64::from_le_bytes(row.bytes[0x10..0x18].try_into().unwrap());
        nodes.push(SurfaceHashChainNode {
            next: (position + 1 < chain.len()).then_some(position + 1),
            surface_pointer,
        });
    }
    let capacity = u32::try_from(chain.len())
        .map_err(|_| "prepass chain exceeds 32-bit count".to_owned())?;
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
    if primary != 0 {
        return Ok(primary as u32);
    }
    let secondary = surface_float_order(candidate.secondary, target.secondary);
    if secondary != 0 {
        return Ok(secondary as u32);
    }
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
    if count < 1 {
        return Ok(0);
    }
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
        if less {
            low += half;
        }
        remaining -= half;
        if half == 0 {
            return Ok((low + i32::from(less)) as u32);
        }
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
        (1, 0x26),
        (2, 0x32),
        (3, 0x33),
        (5, 0x34),
        (4, 0x2d),
        (6, 0x37),
        (7, 0x35),
        (8, 0x38),
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
        return Ok(if matches!((flags4a >> 2) & 3, 0 | 3) {
            0x23
        } else {
            0x37
        });
    }
    if value < thresholds[0] {
        return Ok(if state[0x4b] & 0x80 != 0 { 0x37 } else { 0x23 });
    }
    for (index, program) in [
        (1, 0x26),
        (2, 0x32),
        (3, 0x33),
        (5, 0x34),
        (4, 0x2d),
        (6, 0x37),
        (7, 0x35),
        (8, 0x38),
    ] {
        if value < thresholds[index] {
            return Ok(program);
        }
    }
    Ok(if value < thresholds[9] { 0x39 } else { 0x41 })
}

/// FUN_1402c2ac0 -> FUN_1402db420 prepares the lookup passed to FUN_140342640.
/// The global parameter ID is supplied by the caller; its native value is runtime-owned.
pub fn surface_parameter_float_lookup(
    parameter_object: &[u8],
    parameter_id: u64,
) -> Result<(u64, u64, u32), &'static str> {
    let owner = parameter_object
        .get(0x10..0x18)
        .ok_or("parameter object is shorter than its owner pointer at +0x10")?;
    let property = u64::from_le_bytes(owner.try_into().unwrap()).wrapping_add(0x2d8);
    Ok((property, parameter_id, 0))
}

/// FUN_140319540 clears qwords 0 and 3 only when qword 0 is nonzero.
pub fn reset_depth_draw_cache_if_bound(cache: &mut [u64; 4]) {
    if cache[0] != 0 {
        cache[0] = 0;
        cache[3] = 0;
    }
}

fn indirect_pointer_plus(base: &[u8], addend: u64) -> Result<u64, &'static str> {
    let pointer = base
        .get(0x10..0x18)
        .ok_or("object is shorter than its pointer at +0x10")?;
    let pointer = u64::from_le_bytes(pointer.try_into().unwrap());
    Ok(pointer.wrapping_add(addend))
}

/// FUN_1402c2ab0 loads a pointer at byte offset 0x10 and returns pointer + 0x58.
pub fn indirect_pointer_plus_0x58(base: &[u8]) -> Result<u64, &'static str> {
    indirect_pointer_plus(base, 0x58)
}

/// FUN_1402c2aa0 loads a pointer at byte offset 0x10 and returns pointer + 0x2d8.
pub fn indirect_pointer_plus_0x2d8(base: &[u8]) -> Result<u64, &'static str> {
    indirect_pointer_plus(base, 0x2d8)
}

/// FUN_1402bc340 passes its value as a one-qword array to FUN_1402bc370.
pub fn call_resource_array_bind_with_one(
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
pub fn dispatch_depthpass_worker(
    context: &[u8],
    worker_0: impl FnOnce() -> u64,
    worker_1: impl FnOnce() -> u64,
    worker_other: impl FnOnce(&[u8]) -> u64,
) -> Result<u64, String> {
    let index = context
        .get(0xf8..0xfc)
        .ok_or_else(|| "depth-pass context is shorter than its worker index".to_owned())?;
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
pub fn depthpass_index0_early_exit(
    view: &[u8],
    renderer_state: &mut [u8],
    state_pointer: u64,
) -> Result<Option<u64>, &'static str> {
    let flags = *view
        .get(0x3a34)
        .ok_or("view is shorter than its +0x3a34 flags byte")?;
    if flags & 0x10 == 0 {
        return Ok(None);
    }
    let completed = renderer_state
        .get_mut(0x12ad4)
        .ok_or("renderer state is shorter than its +0x12ad4 completion byte")?;
    *completed = 1;
    Ok(Some(state_pointer & !0xff))
}

/// FUN_140109550 stages 64 source bytes, then writes four 16-byte chunks in
/// destination order. The writer callback preserves native pointer/alias order.
pub fn copy_64_to_four_destinations(
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
pub fn write_depthpass_parameter_slots(
    context: &mut [u8],
    source: [u8; 64],
) -> Result<(), &'static str> {
    const FIRST_INDEX: usize = 0x123e8;
    let mut destinations = [0usize; 4];
    for (slot, destination) in destinations.iter_mut().enumerate() {
        let field = FIRST_INDEX + slot * 4;
        let bytes = context
            .get(field..field + 4)
            .ok_or("depth-pass context lacks parameter slot indices")?;
        let index = i32::from_le_bytes(bytes.try_into().unwrap());
        let start = i64::from(index)
            .checked_mul(16)
            .and_then(|offset| offset.checked_add(16))
            .ok_or("depth-pass parameter slot offset overflow")?;
        let start = usize::try_from(start)
            .map_err(|_| "depth-pass parameter slot index is negative")?;
        let end = start
            .checked_add(16)
            .ok_or("depth-pass parameter slot range overflow")?;
        if end > context.len() {
            return Err("depth-pass parameter slot exceeds context");
        }
        *destination = start;
    }
    copy_64_to_four_destinations(&source, |slot, chunk| {
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
        slots
            .try_reserve_exact(requested - slots.len())
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
        build_zprepass_list, copy_64_to_four_destinations, dispatch_depthpass_worker,
        depthpass_index0_early_exit, write_depthpass_parameter_slots,
    };

    #[test]
    fn depthpass_index0_flag_sets_completion_and_masks_return_pointer() {
        let mut view = vec![0; 0x3a35];
        view[0x3a34] = 0x10;
        let mut state = vec![0; 0x12ad5];
        let result =
            depthpass_index0_early_exit(&view, &mut state, 0x1234_5678_9abc_def7).unwrap();
        assert_eq!(result, Some(0x1234_5678_9abc_de00));
        assert_eq!(state[0x12ad4], 1);
    }

    #[test]
    fn depthpass_index0_without_flag_continues_without_writing_state() {
        let view = vec![0; 0x3a35];
        let mut state = vec![0; 0x12ad5];
        assert_eq!(
            depthpass_index0_early_exit(&view, &mut state, 0xff),
            Ok(None)
        );
        assert_eq!(state[0x12ad4], 0);
        assert!(depthpass_index0_early_exit(&[], &mut state, 0).is_err());
        let mut short_state = vec![0; 0x12ad4];
        let mut flagged_view = vec![0; 0x3a35];
        flagged_view[0x3a34] = 0x10;
        assert!(depthpass_index0_early_exit(&flagged_view, &mut short_state, 0).is_err());
    }

    #[test]
    fn depthpass_payload_copy_preserves_bytes_and_destination_order() {
        let source: [u8; 64] = std::array::from_fn(|index| (index * 17) as u8);
        let mut chunks = [[0u8; 16]; 4];
        copy_64_to_four_destinations(&source, |index, chunk| chunks[index] = *chunk);
        for index in 0..4 {
            assert_eq!(chunks[index], source[index * 16..index * 16 + 16]);
        }

        let mut context = vec![0; 0x12400];
        for (slot, index) in [0i32, 1, 2, 3].into_iter().enumerate() {
            let field = 0x123e8 + slot * 4;
            context[field..field + 4].copy_from_slice(&index.to_le_bytes());
        }
        write_depthpass_parameter_slots(&mut context, source).unwrap();
        for index in 0..4 {
            let start = (index + 1) * 16;
            assert_eq!(
                &context[start..start + 16],
                &source[index * 16..index * 16 + 16]
            );
        }

        let mut aliased = vec![0; 0x123f8];
        for slot in 0..4 {
            let field = 0x123e8 + slot * 4;
            aliased[field..field + 4].copy_from_slice(&0i32.to_le_bytes());
        }
        write_depthpass_parameter_slots(&mut aliased, source).unwrap();
        assert_eq!(&aliased[16..32], &source[48..64]);
    }

    #[test]
    fn depthpass_dispatch_reads_index_and_routes_context() {
        let make_context = |index: i32| {
            let mut bytes = vec![0; 0xfc];
            bytes[0xf8..0xfc].copy_from_slice(&index.to_le_bytes());
            bytes
        };
        assert_eq!(
            dispatch_depthpass_worker(
                &make_context(0),
                || 10,
                || 11,
                |_| panic!("wrong worker")
            ),
            Ok(10)
        );
        assert_eq!(
            dispatch_depthpass_worker(
                &make_context(1),
                || 10,
                || 11,
                |_| panic!("wrong worker")
            ),
            Ok(11)
        );
        let context = make_context(2);
        let mut passed_same_context = false;
        assert_eq!(
            dispatch_depthpass_worker(
                &context,
                || 10,
                || 11,
                |other| {
                    passed_same_context = std::ptr::eq(other, context.as_slice());
                    12
                }
            ),
            Ok(12)
        );
        assert!(passed_same_context);
        assert_eq!(
            dispatch_depthpass_worker(&make_context(-1), || 10, || 11, |_| 13),
            Ok(13)
        );
        assert!(dispatch_depthpass_worker(&[0; 0xfb], || 10, || 11, |_| 13).is_err());
    }

    #[test]
    fn matches_zprepass_list_cases() {
        let data = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/zprepass_list_cases.txt"
        ))
        .unwrap();
        let mut cases = 0;
        for line in data.lines() {
            let row: Vec<u32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            assert_eq!(row.len(), 116);
            let mut count = row[0] as usize;
            let live: Vec<bool> = row[3..35].iter().map(|&v| v != 0).collect();
            let mut slots: Vec<Option<u32>> = row[51..83].iter().copied().map(Some).collect();
            build_zprepass_list(
                &mut slots,
                &mut count,
                &live,
                &row[35..51],
                15,
                row[2],
                row[1] != 0,
            );
            assert_eq!(count, row[83] as usize, "case {cases}");
            assert_eq!(
                slots,
                row[84..116].iter().copied().map(Some).collect::<Vec<_>>(),
                "case {cases}"
            );
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
                record_words: super::SurfaceKeyArrayView {
                    values: &RECORDS,
                    packed_count: 0,
                },
                short_keys: super::SurfaceKeyArrayView {
                    values: &SHORTS,
                    packed_count: 0,
                },
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
            short_keys: super::SurfaceKeyArrayView {
                values: shorts,
                packed_count: short_count,
            },
            record_words: super::SurfaceKeyArrayView {
                values: records,
                packed_count: record_count,
            },
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
        let different_length =
            nested_surface_key(&right_short, 0x5500_0001, &right_records, 0x1100_0001);
        assert_eq!(
            super::surface_nested_key_equal(&left, &different_length),
            Ok(false)
        );
        let different_words = [[1u32, 2, 3, 5]];
        let different_record =
            nested_surface_key(&right_short, 0x5500_0002, &different_words, 0x1100_0001);
        assert_eq!(
            super::surface_nested_key_equal(&left, &different_record),
            Ok(false)
        );
    }

    #[test]
    fn surface_key_comparator_preserves_field_precedence_and_nan() {
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
    fn surface_key_search_and_lower_bound_preserve_duplicate_rules() {
        let entries = [
            surface_key(0.0, 0.0),
            surface_key(1.0, 0.0),
            surface_key(1.0, 2.0),
            surface_key(1.0, 3.0),
            surface_key(2.0, 0.0),
        ];
        let zero = surface_key(0.0, 0.0);
        let one = surface_key(1.0, 0.0);
        let between = surface_key(1.5, 0.0);
        let two = surface_key(2.0, 0.0);
        let nan = surface_key(f32::NAN, 0.0);

        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 5, &zero),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 5, &one),
            Ok(3)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 5, &two),
            Ok(4)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 1, &zero),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 1, &one),
            Ok(-1)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 5, &between),
            Ok(-1)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 5, &nan),
            Ok(-1)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), 0, &one),
            Ok(-1)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(None, 5, &one),
            Ok(-1)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), -1, &zero),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_find_rightmost_equal(Some(&entries), -1, &one),
            Ok(-1)
        );
        assert!(super::surface_key_find_rightmost_equal(Some(&entries), 6, &one).is_err());

        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 5, &zero),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 5, &one),
            Ok(1)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 5, &between),
            Ok(4)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 5, &two),
            Ok(4)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 1, &zero),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 1, &one),
            Ok(1)
        );
        assert_eq!(
            super::surface_key_lower_bound(Some(&entries), 5, &nan),
            Ok(0)
        );
        assert_eq!(super::surface_key_lower_bound(None, 0, &one), Ok(0));
        assert_eq!(super::surface_key_lower_bound(None, -1, &one), Ok(0));
        assert!(super::surface_key_lower_bound(None, 1, &one).is_err());
        assert!(super::surface_key_lower_bound(Some(&entries), 6, &one).is_err());
    }

    #[test]
    fn sorted_surface_insert_composes_search_lower_bound_and_pointer_growth() {
        let mut keys = Vec::new();
        let mut pointers = super::SurfacePointerList::with_capacity(1, 0).unwrap();
        let key0 = surface_key(0.0, 0.0);
        let key1 = surface_key(1.0, 0.0);
        let key2 = surface_key(2.0, 0.0);
        let key_between = surface_key(1.5, 0.0);

        assert_eq!(
            super::insert_sorted_surface_pointer(&mut keys, &mut pointers, key0, 10, false),
            Ok(super::SurfaceSortedInsert::Inserted { index: 0 })
        );
        assert_eq!(
            super::insert_sorted_surface_pointer(&mut keys, &mut pointers, key2, 20, false),
            Ok(super::SurfaceSortedInsert::Inserted { index: 1 })
        );
        assert_eq!(
            super::insert_sorted_surface_pointer(&mut keys, &mut pointers, key2, 99, false),
            Ok(super::SurfaceSortedInsert::Existing { index: 1 })
        );
        assert_eq!(
            super::insert_sorted_surface_pointer(
                &mut keys,
                &mut pointers,
                key_between,
                15,
                false
            ),
            Ok(super::SurfaceSortedInsert::Inserted { index: 1 })
        );
        assert_eq!(
            super::insert_sorted_surface_pointer(&mut keys, &mut pointers, key1, 13, false),
            Ok(super::SurfaceSortedInsert::Inserted { index: 1 })
        );

        assert_eq!(
            keys.iter().map(|key| key.primary).collect::<Vec<_>>(),
            [0.0, 1.0, 1.5, 2.0]
        );
        assert_eq!(pointers.values(), &[10, 13, 15, 20]);
        assert_eq!(pointers.count(), keys.len() as u32);
        assert!(pointers.capacity() >= pointers.count());
    }

    #[test]
    fn prepass_hash_chain_uses_full_pointer_growth_and_front_order() {
        let nodes = [
            super::SurfaceHashChainNode {
                next: Some(1),
                surface_pointer: 0x11,
            },
            super::SurfaceHashChainNode {
                next: None,
                surface_pointer: 0x22,
            },
        ];
        let mut output = super::SurfacePointerList::with_capacity(0, 0).unwrap();
        super::build_prepass_hash_chain_pointer_list_growing(&nodes, Some(0), &mut output, true)
            .unwrap();
        assert_eq!(output.values(), &[0x22, 0x11]);
        assert_eq!(output.count(), 2);
        assert_eq!(output.capacity(), 2);
        assert_eq!(output.packed_header().0 & 0xff00_0000, 0);
    }

    #[test]
    fn surface_iteration_matches_fun_14033b280_count_mode_and_filter() {
        assert_eq!(
            super::surface_candidate_iteration_indices(0xaa00_0003, 0, -1),
            Ok((1, vec![0, 1, 2]))
        );
        assert_eq!(
            super::surface_candidate_iteration_indices(0xaa00_0003, 0x10, -1),
            Ok((0, vec![0, 1, 2]))
        );
        assert_eq!(
            super::surface_candidate_iteration_indices(0xaa00_0003, 0, 1),
            Ok((1, vec![1]))
        );
        assert_eq!(
            super::surface_candidate_iteration_indices(0xaa00_0003, 0, 3),
            Ok((1, vec![]))
        );
        assert_eq!(
            super::surface_candidate_iteration_indices(0xaa00_0001, 0, -1),
            Ok((0, vec![0]))
        );
    }

    #[test]
    fn surface_container_visits_stride_order_mode_and_filter() {
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
                super::surface_candidate_iteration_indices(packed_count, flags, filter)
                    .unwrap();
            let mut visited = Vec::new();
            let count = super::visit_surface_container_rows(
                &rows,
                packed_count,
                flags,
                filter,
                |actual_mode, index, row| {
                    assert_eq!(actual_mode, mode);
                    assert_eq!(row[0], index as u8);
                    assert_eq!(row[0x7f], 0xa0 + index as u8);
                    visited.push(index);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(visited, expected_indices);
            assert_eq!(count, expected_indices.len());
        }
    }

    #[test]
    fn surface_container_rejects_truncation_before_visiting_and_propagates_error() {
        let mut visits = 0;
        assert!(
            super::visit_surface_container_rows(&[0; 0x7f], 1, 0, -1, |_, _, _| {
                visits += 1;
                Ok(())
            },)
            .is_err()
        );
        assert_eq!(visits, 0);
        let error =
            super::visit_surface_container_rows(&[0; 3 * 0x80], 3, 0, -1, |_, index, _| {
                visits += 1;
                if index == 1 {
                    Err("callback error".to_owned())
                } else {
                    Ok(())
                }
            });
        assert_eq!(error, Err("callback error".to_owned()));
        assert_eq!(visits, 2);
    }

    #[test]
    fn vectorized_aabb_clip_preserves_scalar_masks_for_float_edge_cases() {
        let mut seed = 0xc9e354a7u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let values = [
                0.0,
                -0.0,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::from_bits(0x7fc01234),
                f32::from_bits(1),
                -f32::from_bits(1),
            ];
            if seed & 15 == 0 {
                values[(seed as usize >> 4) % values.len()]
            } else {
                f32::from_bits(seed)
            }
        };
        for _ in 0..16384 {
            let matrix = std::array::from_fn(|_| next());
            let minimum = std::array::from_fn(|_| next());
            let maximum = std::array::from_fn(|_| next());
            let near = next();
            for unit in [false, true] {
                assert_eq!(
                    super::transformed_aabb_clip_cull_bounds(
                        &matrix, minimum, maximum, near, unit
                    ),
                    super::aabb_clip_cull_scalar(&matrix, minimum, maximum, near, unit)
                );
            }
        }
    }

    #[test]
    fn transformed_aabb_clip_matches_plane_mask_and_cull_return() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [-0.5, -0.5, 0.1], [0.5, 0.5, 0.9]),
            super::AabbClipCullResult {
                outside_plane_mask: 0,
                output_byte: 0,
                culled: false
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [-3.0, 0.0, 0.1], [-2.0, 1.0, 0.9]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x01,
                output_byte: 0x01,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [2.0, 0.0, 0.1], [3.0, 1.0, 0.9]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x02,
                output_byte: 0x02,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [0.0, -3.0, 0.1], [1.0, -2.0, 0.9]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x04,
                output_byte: 0x04,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [0.0, 2.0, 0.1], [1.0, 3.0, 0.9]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x08,
                output_byte: 0x08,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(
                &identity,
                [-0.5, -0.5, -2.0],
                [0.5, 0.5, -1.0]
            ),
            super::AabbClipCullResult {
                outside_plane_mask: 0x10,
                output_byte: 0x10,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [-0.5, -0.5, 2.0], [0.5, 0.5, 3.0]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x20,
                output_byte: 0x20,
                culled: true
            },
        );
        assert_eq!(
            super::transformed_aabb_clip_cull(
                &identity,
                [-1.0, -0.5, 0.1],
                [-1.0, 0.5, 0.9]
            ),
            super::AabbClipCullResult {
                outside_plane_mask: 0x01,
                output_byte: 0x01,
                culled: true
            },
        );
        let nan = f32::NAN;
        assert_eq!(
            super::transformed_aabb_clip_cull(&identity, [nan; 3], [nan; 3]),
            super::AabbClipCullResult {
                outside_plane_mask: 0x3f,
                output_byte: 0x3f,
                culled: true
            },
        );
    }

    #[test]
    fn surface_candidate_category_key_matches_gates() {
        let mut gates = super::SurfaceCandidateBranchGates {
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
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 1),
            Some(9)
        );
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        gates.mode = 1;
        gates.helper_140109dd0_returns_nonzero = false;
        assert_eq!(
            super::surface_candidate_category_key_with_bounds(
                &gates,
                0x20,
                1,
                &identity,
                [-0.5, -0.5, 0.1],
                [0.5, 0.5, 0.9],
            ),
            Some(9)
        );
        assert_eq!(
            super::surface_candidate_category_key_with_bounds(
                &gates,
                0x20,
                1,
                &identity,
                [-3.0, 0.0, 0.1],
                [-2.0, 1.0, 0.9],
            ),
            None
        );
        gates.mode = 0;
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0, u32::MAX),
            None
        );
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 0),
            None
        );
        gates.optional_pointer_78_value = Some(0);
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 1),
            None
        );
        gates.optional_pointer_78_value = None;
        gates.material_flags_4b = 4;
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 1),
            None
        );
        gates.material_flags_4b = 0;
        gates.mode = 1;
        gates.helper_140109dd0_returns_nonzero = true;
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 1),
            None
        );
        gates.surface_pointer_40_present = true;
        assert_eq!(
            super::surface_candidate_category_key(&gates, 0x20, 1),
            Some(9)
        );
    }

    #[test]
    fn prepass_candidate_gate_row_hash_bucket_and_front_insert_compose() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let gates = super::SurfaceCandidateBranchGates {
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
            super::SurfaceCandidateInput {
                branch_fields: super::SurfaceCandidateBranchFields {
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
        let mut rows = super::SurfaceCandidateRows::with_capacity(0, 0).unwrap();
        let mut hash = super::SurfaceHashIndex::lazy(4, 0, 0).unwrap();
        assert_eq!(
            super::append_surface_candidate_if_eligible(
                &input(0x0a, 0x20, 1, true),
                &mut rows,
                &mut hash,
                false,
            ),
            Ok(Some(0))
        );
        assert_eq!(
            super::append_surface_candidate_if_eligible(
                &input(0x0b, 0x40, 2, true),
                &mut rows,
                &mut hash,
                false,
            ),
            Ok(Some(1))
        );
        assert_eq!(
            super::append_surface_candidate_if_eligible(
                &input(0x0c, 0x20, 1, true),
                &mut rows,
                &mut hash,
                false,
            ),
            Ok(Some(2))
        );
        assert_eq!(
            super::append_surface_candidate_if_eligible(
                &input(0x0d, 0x20, 1, false),
                &mut rows,
                &mut hash,
                false,
            ),
            Ok(None)
        );
        assert_eq!(
            super::append_surface_candidate_if_eligible(
                &input(0x0e, 0xa0, 16, true),
                &mut rows,
                &mut hash,
                false,
            ),
            Ok(Some(3))
        );
        assert_eq!(rows.count(), 4);
        // Categories 9 and 13 collide in the four-bucket HashIndex. The
        // insertion callback reverses the bucket walk but does not compare keys.
        assert_eq!(
            super::prepass_surface_pointers_for_bucket(&rows, &hash, 9),
            Ok(vec![0x0a, 0x0c, 0x0e])
        );
        assert_eq!(
            super::prepass_surface_pointers_for_bucket(&rows, &hash, 10),
            Ok(vec![0x0b])
        );
        assert_eq!(
            super::prepass_surface_pointers_for_bucket(&rows, &hash, 11),
            Ok(Vec::new())
        );
    }

    #[test]
    fn surface_candidate_hash_index_matches_lazy_reset_growth_and_chain_order() {
        let mut rows = super::SurfaceCandidateRows::with_capacity(1, 0).unwrap();
        let mut hash = super::SurfaceHashIndex::lazy(8, 1, 0).unwrap();
        let row = super::SurfaceCandidateRow::default();
        assert_eq!(
            super::append_surface_candidate_to_hash(&mut rows, &mut hash, 11, row, false),
            Ok(0)
        );
        assert_eq!(hash.bucket_mask(), 7);
        assert_eq!(hash.cached_state_word(), u32::MAX);
        assert_eq!(hash.bucket_heads().len(), 8);
        assert_eq!(hash.next_indices(), &[u32::MAX]);
        assert_eq!(
            super::append_surface_candidate_to_hash(&mut rows, &mut hash, 19, row, false),
            Ok(1)
        );
        assert_eq!(hash.chain_capacity(), 2);
        assert_eq!(hash.next_indices(), &[u32::MAX, 0]);
        assert_eq!(hash.bucket_chain(11), Ok(vec![1, 0]));
        assert_eq!(hash.bucket_chain(3), Ok(vec![1, 0]));
        hash.clear();
        assert_eq!(hash.cached_state_word(), 0);
        assert!(hash.bucket_heads().is_empty());
        assert_eq!(hash.bucket_chain(11), Ok(Vec::new()));
        assert_eq!(
            super::append_surface_candidate_to_hash(&mut rows, &mut hash, 11, row, false),
            Ok(2)
        );
        assert_eq!(hash.bucket_chain(11), Ok(vec![2]));
    }

    #[test]
    fn surface_candidate_payload_and_row_match_offsets() {
        let sort = super::SurfaceCandidateSortFields {
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
        let payload = super::surface_candidate_sort_payload(&sort);
        assert_eq!(u64::from_le_bytes(payload[0..8].try_into().unwrap()), 0x101);
        assert_eq!(
            u64::from_le_bytes(payload[8..16].try_into().unwrap()),
            0x202
        );
        assert_eq!(
            u64::from_le_bytes(payload[16..24].try_into().unwrap()),
            0x303
        );
        assert_eq!(
            u64::from_le_bytes(payload[24..32].try_into().unwrap()),
            0x5950
        );
        assert_eq!(
            u64::from_le_bytes(payload[32..40].try_into().unwrap()),
            0x404
        );
        assert_eq!(
            u32::from_le_bytes(payload[40..44].try_into().unwrap()),
            0x3f000000
        );
        assert_eq!(
            u32::from_le_bytes(payload[44..48].try_into().unwrap()),
            u32::MAX
        );
        assert_eq!(payload[48], 1);
        assert_eq!(payload[49], 1);
        assert_eq!(payload[50..52], [0xa5, 0x5a]);
        assert_eq!(
            u32::from_le_bytes(payload[52..56].try_into().unwrap()),
            0x7f7fffff
        );

        let row = super::surface_candidate_branch_row(
            &super::SurfaceCandidateBranchFields {
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
            },
        );
        assert_eq!(row.bytes[0], 0x81);
        assert_eq!(row.bytes[1], 0xff);
        assert_eq!(
            u64::from_le_bytes(row.bytes[8..16].try_into().unwrap()),
            0x505
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[16..24].try_into().unwrap()),
            0x606
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[24..32].try_into().unwrap()),
            0x707
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[32..40].try_into().unwrap()),
            0x808
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[40..48].try_into().unwrap()),
            0x909
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[48..56].try_into().unwrap()),
            0xa0a
        );
        assert_eq!(row.bytes[0x38..0x40], [0; 8]);
        assert_eq!(
            u32::from_le_bytes(row.bytes[0x78..0x7c].try_into().unwrap()),
            0.5_f32.to_bits()
        );

        let mut gated = sort;
        gated.surface_state_pointer_40 = 1;
        assert_eq!(super::surface_candidate_sort_payload(&gated)[48], 0);
    }

    #[test]
    fn preallocated_surface_pointer_insert_matches_clamp_shift_and_count() {
        let mut slots = [11, 22, 33, 0];
        let mut count = 0xa500_0003;
        assert_eq!(
            super::insert_preallocated_surface_pointer(&mut slots, &mut count, 4, 1, 99),
            Ok(1)
        );
        assert_eq!(slots, [11, 99, 22, 33]);
        assert_eq!(count, 0xa500_0004);
        assert_eq!(
            super::insert_preallocated_surface_pointer(&mut slots, &mut count, 4, -7, 88),
            Err("surface pointer vector growth is required".to_owned())
        );

        let mut empty = [0; 2];
        let mut empty_count = 0;
        assert_eq!(
            super::insert_preallocated_surface_pointer(
                &mut empty,
                &mut empty_count,
                2,
                i32::MIN,
                7
            ),
            Ok(0)
        );
        assert_eq!(empty, [7, 0]);
        assert_eq!(empty_count, 1);

        let nodes = [
            super::SurfaceHashChainNode {
                next: Some(2),
                surface_pointer: 10,
            },
            super::SurfaceHashChainNode {
                next: None,
                surface_pointer: 30,
            },
            super::SurfaceHashChainNode {
                next: Some(1),
                surface_pointer: 10,
            },
        ];
        let mut prefixed = [20, 21, 22, 23, 0, 0, 0];
        let mut prefixed_count = 4;
        assert_eq!(
            super::build_prepass_hash_chain_pointer_list(
                &nodes,
                Some(0),
                &mut prefixed,
                &mut prefixed_count,
                7,
            ),
            Ok(()),
        );
        assert_eq!(prefixed, [30, 10, 10, 20, 21, 22, 23]);
        assert_eq!(prefixed_count, 7);

        let mut full = [1, 2];
        let mut full_count = 2;
        assert_eq!(
            super::insert_preallocated_surface_pointer(&mut full, &mut full_count, 2, 100, 3),
            Err("surface pointer vector growth is required".to_owned())
        );
    }

    #[test]
    fn surface_pointer_list_ports_allocator_modes_growth_and_capacity_reset() {
        let mut ordinary = super::SurfacePointerList::with_capacity(1, 0).unwrap();
        assert_eq!(ordinary.insert(0, 10, false), Ok(0));
        assert_eq!(ordinary.insert(1, 20, false), Ok(1));
        assert_eq!(ordinary.capacity(), 2);
        assert_eq!(ordinary.values(), &[10, 20]);
        assert_eq!(ordinary.insert(u32::MAX, 5, false), Ok(0));
        assert_eq!(ordinary.values(), &[5, 10, 20]);
        assert_eq!(ordinary.capacity(), 3);

        let mut specialized = super::SurfacePointerList::with_capacity(1, 0xc0).unwrap();
        assert_eq!(specialized.insert(0, 30, true), Ok(0));
        assert_eq!(specialized.insert(1, 40, true), Ok(1));
        assert_eq!(specialized.values(), &[30, 40]);
        assert_eq!(specialized.packed_header(), (2, (2 << 8) | 0xc0));

        let mut specialized_oom = super::SurfacePointerList::with_capacity(1, 0xc0).unwrap();
        specialized_oom.insert(0, 45, true).unwrap();
        assert_eq!(specialized_oom.set_capacity(2, false), Ok(true));
        assert_eq!(specialized_oom.packed_header(), (1, (2 << 8) | 0xc0));
        assert_eq!(specialized_oom.values(), &[45]);
        assert!(specialized_oom.insert(1, 46, true).is_err());

        let mut denied = super::SurfacePointerList::with_capacity(1, 0x40).unwrap();
        assert_eq!(denied.insert(0, 50, false), Ok(0));
        let before = denied.packed_header();
        assert!(denied.insert(1, 60, false).is_err());
        assert_eq!(denied.packed_header(), before);
        assert_eq!(denied.values(), &[50]);

        let mut failure = super::SurfacePointerList::with_capacity(2, 0).unwrap();
        failure.insert(0, 70, false).unwrap();
        let before = failure.packed_header();
        assert_eq!(failure.set_capacity(4, false), Ok(false));
        assert_eq!(failure.packed_header(), before);
        assert_eq!(failure.count(), 1);
        assert!(failure.values().is_empty());

        let mut retained = super::SurfacePointerList::with_capacity(2, 0x40).unwrap();
        retained.insert(0, 80, false).unwrap();
        assert_eq!(retained.set_capacity(0, true), Ok(true));
        assert_eq!(retained.packed_header(), (0, (2 << 8) | 0x40));
        assert!(retained.values().is_empty());
        let mut released = super::SurfacePointerList::with_capacity(2, 0).unwrap();
        released.insert(0, 90, false).unwrap();
        assert_eq!(released.set_capacity(0, true), Ok(true));
        assert_eq!(released.packed_header(), (0, 0));
        assert_eq!(released.capacity(), 0);

        let mut slots = vec![11, 22];
        let mut capacity_word = (2 << 8) | 0x55;
        super::reallocate_surface_pointers(&mut slots, 2, &mut capacity_word, 4).unwrap();
        assert_eq!(slots, [11, 22, 0, 0]);
        assert_eq!(capacity_word, (4 << 8) | 0x3f);
        assert_eq!(
            super::allocate_zeroed_surface_pointers(3).unwrap(),
            [0, 0, 0]
        );
    }

    #[test]
    fn surface_batch_rows_preserve_0x38_bytes_across_capacity_modes() {
        let mut ordinary = super::SurfaceBatchRowList::with_capacity(1, 0).unwrap();
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
        assert_eq!(std::mem::size_of::<super::SurfaceBatchRow>(), 0x38);

        let mut specialized = super::SurfaceBatchRowList::with_capacity(1, 0xc0).unwrap();
        let first = specialized.push_zeroed_row(true, true).unwrap();
        specialized.row_mut(first).unwrap()[6] = 0xfeed_beef;
        let second = specialized.push_zeroed_row(true, true).unwrap();
        assert_eq!(second, 1);
        assert_eq!(specialized.packed_header(), (2, (2 << 8) | 0xc0));
        assert_eq!(specialized.rows()[0][6], 0xfeed_beef);
        assert_eq!(specialized.rows()[1], [0; 14]);

        let mut specialized_oom = super::SurfaceBatchRowList::with_capacity(1, 0xc0).unwrap();
        specialized_oom.push_zeroed_row(true, true).unwrap();
        assert_eq!(specialized_oom.set_capacity(2, false), Ok(true));
        assert_eq!(specialized_oom.packed_header(), (1, (2 << 8) | 0xc0));
        assert!(specialized_oom.push_zeroed_row(true, true).is_err());

        let mut ordinary_oom = super::SurfaceBatchRowList::with_capacity(1, 0).unwrap();
        ordinary_oom.push_zeroed_row(false, true).unwrap();
        let before = ordinary_oom.packed_header();
        assert_eq!(ordinary_oom.set_capacity(2, false), Ok(false));
        assert_eq!(ordinary_oom.packed_header(), before);
        assert_eq!(ordinary_oom.count(), 1);
        assert!(ordinary_oom.rows().is_empty());

        let mut denied = super::SurfaceBatchRowList::with_capacity(1, 0x40).unwrap();
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
                super::surface_lookup_3119a0(&default_bank, &override_bank, 0x40, 4, 2, selector,),
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
    fn candidate_row_initializer_matches_observed_fields() {
        let row = super::SurfaceCandidateRow::default();
        assert_eq!(row.bytes[0], 0);
        assert_eq!(row.bytes[1], 0xff);
        assert_eq!(
            u32::from_le_bytes(row.bytes[0x6c..0x70].try_into().unwrap()),
            u32::MAX
        );
        assert_eq!(
            u64::from_le_bytes(row.bytes[0x74..0x7c].try_into().unwrap()),
            0x7f7f_ffff
        );
        assert_eq!(
            u32::from_le_bytes(row.bytes[0x94..0x98].try_into().unwrap()),
            0x3f80_0000
        );
        assert_eq!(&row.bytes[0x98..0xa4], &[0; 12]);
    }

    #[test]
    fn candidate_rows_append_and_preserve_data_across_growth() {
        let mut rows = super::SurfaceCandidateRows::with_capacity(1, 0x3f).unwrap();
        let mut first = super::SurfaceCandidateRow::default();
        first.bytes[8..16].copy_from_slice(&0x1122_3344_5566_7788_u64.to_le_bytes());
        let mut second = super::SurfaceCandidateRow::default();
        second.bytes[8..16].copy_from_slice(&0x8877_6655_4433_2211_u64.to_le_bytes());
        assert_eq!(rows.append(first, false), Ok(0));
        assert_eq!(rows.append(second, false), Ok(1));
        assert_eq!((rows.count(), rows.capacity()), (2, 2));
        assert_eq!(rows.rows()[0].bytes[8..16], first.bytes[8..16]);
        assert_eq!(rows.rows()[1].bytes[8..16], second.bytes[8..16]);

        let mut mode_three = super::SurfaceCandidateRows::with_capacity(1, 0xc0).unwrap();
        assert_eq!(mode_three.append(first, false), Ok(0));
        assert_eq!(mode_three.append(second, false), Ok(1));
        assert_eq!(mode_three.packed_header(), (2, (2 << 8) | 0x3f));
        assert_eq!(mode_three.rows()[0].bytes[8..16], first.bytes[8..16]);
    }

    #[test]
    fn candidate_reallocation_copies_only_live_prefix() {
        let mut rows = vec![
            super::SurfaceCandidateRow::default(),
            super::SurfaceCandidateRow::default(),
        ];
        rows[0].bytes[2] = 0x11;
        rows[1].bytes[2] = 0x22;
        super::reallocate_surface_candidate_rows(&mut rows, 1, 3).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].bytes[2], 0x11);
        assert_eq!(rows[1], super::SurfaceCandidateRow::default());
        assert_eq!(rows[2], super::SurfaceCandidateRow::default());
    }

    #[test]
    fn surface_compare_1_returns_observed_index() {
        assert_eq!(super::surface_compare_1_index_result(), 0);
    }

    #[test]
    fn keycompare_7_binary_search_uses_both_float_keys_and_tie_comparator() {
        let entries = [
            surface_key(1.0, 0.0),
            surface_key(3.0, 0.0),
            surface_key(5.0, 0.0),
        ];
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&entries, 3, &surface_key(4.0, 0.0)),
            Ok(2)
        );
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&entries, 3, &entries[1]),
            Ok(1)
        );
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&entries, 0, &entries[1]),
            Ok(0)
        );
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&entries, -1, &entries[1]),
            Ok(0)
        );
        let secondary = [surface_key(3.0, 1.0), surface_key(3.0, 3.0)];
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&secondary, 2, &surface_key(3.0, 2.0)),
            Ok(1)
        );
        assert_eq!(
            super::surface_key_compare_7_insertion_index(&entries, 4, &entries[1]),
            Err("surface sort count exceeds its supplied entries".to_owned())
        );
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
    fn static_surface_selector_preserves_flag_precedence() {
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
    fn static_surface_selector_uses_threshold_order_and_nan_comparisons() {
        let state = selector_state(0, 0, 0, 0, 1.0, 2.0);
        let ordered = [
            (1, 0x26),
            (2, 0x32),
            (3, 0x33),
            (5, 0x34),
            (4, 0x2d),
            (6, 0x37),
            (7, 0x35),
            (8, 0x38),
            (9, 0x39),
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
        assert_eq!(
            super::static_surface_program_selector(&state, &[0.0; 10]),
            Ok(0x41)
        );
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
                &selector_state(0, 0, 0, 1, 0.0, 0.0),
                5.0,
                &bands,
            ),
            Ok(0x25),
        );
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0x80, 0x10, 0, 1, 0.0, 0.0),
                5.0,
                &bands,
            ),
            Ok(0x2a),
        );
        for (subtype, expected) in [(0, 0x23), (1, 0x37), (2, 0x37), (3, 0x23)] {
            assert_eq!(
                super::parameterized_surface_program_selector(
                    &selector_state(0x10, subtype << 2, 0, 0, 0.0, 0.0),
                    5.0,
                    &bands,
                ),
                Ok(expected),
                "material subtype {subtype}",
            );
        }
        let ordered = [
            (0, 0x23),
            (1, 0x26),
            (2, 0x32),
            (3, 0x33),
            (5, 0x34),
            (4, 0x2d),
            (6, 0x37),
            (7, 0x35),
            (8, 0x38),
            (9, 0x39),
        ];
        for (index, expected) in ordered {
            let mut thresholds = [0.0; 10];
            thresholds[index] = 2.0;
            assert_eq!(
                super::parameterized_surface_program_selector(
                    &selector_state(0, 0, 0, 0, 0.0, 0.0),
                    1.0,
                    &thresholds,
                ),
                Ok(expected),
                "threshold index {index}",
            );
        }
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0, 0, 0, 0, 0.0, 0.0),
                f32::NAN,
                &[2.0; 10],
            ),
            Ok(0x41),
        );
        assert_eq!(
            super::parameterized_surface_program_selector(
                &selector_state(0, 0x10, 0x80, 0, 0.0, 0.0),
                1.0,
                &[2.0; 10],
            ),
            Ok(0x37),
        );
        assert!(super::parameterized_surface_program_selector(&[0; 0x107], 1.0, &bands).is_err());
        let mut parameter_object = [0; 0x18];
        parameter_object[0x10..0x18].copy_from_slice(&0x1000u64.to_le_bytes());
        assert_eq!(
            super::surface_parameter_float_lookup(&parameter_object, 0xfeed),
            Ok((0x12d8, 0xfeed, 0)),
        );
        assert!(
            super::surface_parameter_float_lookup(&parameter_object[..0x17], 0xfeed)
                .is_err()
        );
    }

    #[test]
    fn reset_preserves_empty_slots_and_chain_order() {
        let mut slots = vec![Some(91), Some(92), Some(93), Some(94)];
        let mut count = 4;
        // 2 -> 0 -> 1 -> END. The empty record 0 leaves slot 1 intact.
        build_zprepass_list(
            &mut slots,
            &mut count,
            &[false, true, true],
            &[1, u32::MAX, 0],
            3,
            2,
            false,
        );
        assert_eq!(count, 3);
        assert_eq!(slots, [Some(2), Some(92), Some(1), Some(94)]);
    }

    #[test]
    fn append_and_masked_links_preserve_prefix() {
        let mut slots = vec![Some(17)];
        let mut count = 1;
        // Record 4 follows bucket 0, not next[4].
        build_zprepass_list(
            &mut slots,
            &mut count,
            &[true; 5],
            &[1, u32::MAX, u32::MAX, u32::MAX],
            3,
            4,
            true,
        );
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

#[path = "render_model/owner_flags.rs"]
mod owner_flags;
pub use owner_flags::{
    collect_local_model_casters, mark_model_owner_dirty, model_caster_sort_distance,
    model_is_dynamic_caster, model_owner_bounds_flags, model_shadow_radial_sort_key,
    point_clip_mask, prepare_model_caster_info, set_actor_dynamic_caster,
    set_actors_dynamic_caster_for_view, set_model_dynamic_caster_flag,
    set_model_dynamic_caster_with_children, sort_model_shadow_records, ActorCasterAccess,
    LocalModelCasterRecord, LocalShadowCacheControls, ModelOwnerDirtyQueues, ModelCasterInfo,
    INITIAL_MODEL_CONTEXT_FLAGS_18BC,
};

/// FUN_1401095f0: authored axes are stored by basis, not by matrix row.
pub fn affine_rows(position: [f32; 3], basis: [[f32; 3]; 3], scale: [f32; 3]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        if row == 3 {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            [
                basis[0][row] * scale[0],
                basis[1][row] * scale[1],
                basis[2][row] * scale[2],
                position[row],
            ]
        }
    })
}

/// FUN_140235c80 / FUN_1400fb8c0: scaled local-box center and absolute rotated extents.
pub fn transformed_bounds(
    local: [[f32; 3]; 2],
    position: [f32; 3],
    basis: [[f32; 3]; 3],
    scale: [f32; 3],
) -> [[f32; 3]; 2] {
    let center = std::array::from_fn::<_, 3, _>(|axis| {
        (local[1][axis] + local[0][axis]) * scale[axis] * 0.5
    });
    let extent = std::array::from_fn::<_, 3, _>(|axis| scale[axis] * local[1][axis] - center[axis]);
    let rotated_extent = std::array::from_fn::<_, 3, _>(|axis| {
        (extent[1] * basis[1][axis]).abs()
            + (extent[0] * basis[0][axis]).abs()
            + (extent[2] * basis[2][axis]).abs()
    });
    let world_center = std::array::from_fn::<_, 3, _>(|axis| {
        center[1] * basis[1][axis]
            + center[0] * basis[0][axis]
            + center[2] * basis[2][axis]
            + position[axis]
    });
    [
        std::array::from_fn(|axis| world_center[axis] - rotated_extent[axis]),
        std::array::from_fn(|axis| rotated_extent[axis] + world_center[axis]),
    ]
}

/// FUN_140237340: optional range region; nonpositive range uses its constructor region.
pub fn range_region(bounds: [[f32; 3]; 2], range: f32) -> [f32; 4] {
    if !(range > 0.0) {
        return [0.0, 0.0, 0.0, 32768.0];
    }
    let center = std::array::from_fn::<_, 3, _>(|axis| (bounds[1][axis] + bounds[0][axis]) * 0.5);
    let mut extent = 0.0;
    for side in bounds {
        for axis in 0..3 {
            let value = side[axis] - center[axis];
            if extent <= value {
                extent = value;
            }
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
pub fn resource_bounds(bounds: &[[f32; 3]; 2]) -> &[[f32; 3]; 2] {
    bounds
}

/// FUN_14031ad30's finite no-depth selection prefix. The owning model
/// supplies prepared flags and bounds; they are not inferred from shape names.
#[derive(Clone, Copy)]
pub struct ModelQueryInput {
    pub frame_flags: u8,
    pub shape_flags: u8,
    pub region: [f32; 4],
    pub bounds: [[f32; 3]; 2],
}

mod area_gather;
pub use area_gather::{area_model_visible, ModelFadeInput};

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
    Ok(model_visible_area(query, camera, viewport, model)?.is_some())
}

/// FUN_14031ad30 retains the flag-0 bounds-query area for every accepted
/// model and forwards it to each surface material's images via 14031a510.
pub fn model_visible_area(
    query: &NoDepthAabbQuery,
    camera: [f32; 3],
    viewport: [u32; 2],
    model: ModelQueryInput,
) -> Result<Option<f32>, String> {
    if !model_eligible(model.frame_flags, model.shape_flags) {
        return Ok(None);
    }
    if viewport.contains(&0)
        || camera
            .iter()
            .chain(model.region.iter())
            .any(|value| !value.is_finite())
    {
        return Err("unsupported nonfinite model region or empty viewport".to_owned());
    }
    if !in_model_region(camera, model.region) {
        return Ok(None);
    }
    let (visible, area) = query.visibility_and_area(model.bounds)?;
    Ok((visible
        && (model.frame_flags & 0x40 != 0
            || !(area < 25.0 / (viewport[0] as f32 * viewport[1] as f32))))
        .then_some(area))
}

/// FUN_140399620: the base render-model virtual callback has no side effects.
pub fn base_callback() -> u8 {
    0
}

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

/// 1fddc0 selects particle water predicates from the declaration and current
/// view. Reuse the ordinary model flag helpers, including their dirty bit.
pub fn update_particle_water_flags(
    flags: &mut u32,
    underwater: bool,
    handle_seen_from_underwater: bool,
    view_underwater: bool,
) {
    if underwater {
        request_flags(flags, 2);
    } else {
        clear_flags(flags, 2);
        if handle_seen_from_underwater {
            request_flags(flags, 4);
            clear_flags(flags, 8);
            return;
        }
        if view_underwater {
            clear_flags(flags, 4);
            request_flags(flags, 8);
            return;
        }
    }
    clear_flags(flags, 4);
    clear_flags(flags, 8);
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
        [
            patch[0] - padding,
            patch[1] - padding,
            height_center - height_extent,
        ],
        [
            (patch[2] + patch[0]) + padding,
            (patch[2] + patch[1]) + padding,
            height_extent + height_center,
        ],
    ]
}

pub fn square_patch_visible(
    query: &NoDepthAabbQuery,
    patch: [f32; 3],
    height_center: f32,
    height_extent: f32,
    spacing: f32,
) -> Result<bool, String> {
    query.visible(square_patch_bounds(
        patch,
        height_center,
        height_extent,
        spacing,
    ))
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
        return Err(format!(
            "idParmBlock::GetFloat: {name} is not a vector parm"
        ));
    }
    let mut value = *defaults
        .get(id as usize)
        .ok_or("parameter registry index out of bounds")?;
    for &(key, replacement) in overrides {
        if u32::from(key) == id {
            value = replacement;
        }
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
            || self.entries.len() > (self.id_capacity_word >> 8) as usize
        {
            return Err("parameter-array growth is not implemented".into());
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
            return Err("duplicate marker array is limited to 512 entries".into());
        }
        let mut removed = [false; 512];
        for index in (0..self.entries.len()).rev() {
            removed[index] = (index + 1..self.entries.len())
                .any(|later| !removed[later] && self.entries[later].0 == self.entries[index].0);
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
        self.entries
            .retain(|&(key, _)| whitelist.contains(&u32::from(key)));
        self.set_compacted_counts();
        Ok(())
    }

    pub fn float(
        &self,
        name: &str,
        id: u32,
        flags: u32,
        defaults: &[[u32; 4]],
    ) -> Result<f32, String> {
        self.validate_preallocated()?;
        parameter_float(name, id, flags, defaults, &self.entries)
    }

    /// FUN_140310620 restores registered defaults for the ID-count prefix.
    /// Value count/capacity and unused trailing IDs do not affect this operation.
    pub fn restore_defaults(
        &self,
        slots: &mut [[u32; 4]],
        defaults: &[[u32; 4]],
    ) -> Result<(), String> {
        let count = (self.id_count_word & 0x00ff_ffff) as usize;
        let entries = self
            .entries
            .get(..count)
            .ok_or("parameter ID count exceeds initialized entries")?;
        for &(id, _) in entries {
            if usize::from(id) >= defaults.len() || usize::from(id) + 1 >= slots.len() {
                return Err("parameter ID exceeds default or destination slots".into());
            }
        }
        for &(id, _) in entries {
            slots[usize::from(id) + 1] = defaults[usize::from(id)];
        }
        Ok(())
    }

    /// FUN_140310450 copies the ID-count prefix to the ID+1 slots.
    /// The paired representation preserves native ID/value order and duplicates.
    pub fn write_entries_to_slots(&self, slots: &mut [[u32; 4]]) -> Result<(), String> {
        let count = (self.id_count_word & 0x00ff_ffff) as usize;
        if count > self.entries.len() {
            return Err("parameter ID count exceeds initialized entries".into());
        }
        for &(id, _) in &self.entries[..count] {
            if usize::from(id) + 1 >= slots.len() {
                return Err("parameter ID exceeds destination slots".into());
            }
        }
        for &(id, value) in &self.entries[..count] {
            slots[usize::from(id) + 1] = value;
        }
        Ok(())
    }
}

fn parameter_slot_index(id: i32, slot_count: usize) -> Result<usize, String> {
    let index = usize::try_from(i64::from(id) + 1)
        .map_err(|_| "parameter ID selects a negative slot".to_owned())?;
    if index >= slot_count {
        return Err("parameter ID exceeds destination slots".into());
    }
    Ok(index)
}

/// FUN_140310720: zero the first qword and write an exact second qword.
pub fn write_parameter_u64(
    slots: &mut [[u32; 4]],
    id: i32,
    value: u64,
) -> Result<(), String> {
    let slot = parameter_slot_index(id, slots.len())?;
    slots[slot] = [0, 0, value as u32, (value >> 32) as u32];
    Ok(())
}

/// FUN_140310780: replicate the exact 32-bit word to all four lanes.
pub fn write_parameter_repeated_u32(
    slots: &mut [[u32; 4]],
    id: i32,
    value: u32,
) -> Result<(), String> {
    let slot = parameter_slot_index(id, slots.len())?;
    slots[slot] = [value; 4];
    Ok(())
}

/// FUN_140310a80: copy two raw qwords into the parameter slot at ID + 1.
pub fn write_parameter_4_words(
    slots: &mut [[u32; 4]],
    id: i32,
    value: [u32; 4],
) -> Result<(), String> {
    let slot = parameter_slot_index(id, slots.len())?;
    slots[slot] = value;
    Ok(())
}

/// FUN_140310920: zero-extend the word into the first qword and clear the second.
pub fn write_parameter_u32(
    slots: &mut [[u32; 4]],
    id: i32,
    value: u32,
) -> Result<(), String> {
    let slot = parameter_slot_index(id, slots.len())?;
    slots[slot] = [value, 0, 0, 0];
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionalParameterValues {
    pub gate_at_0x38: u64,
    pub indirect_gate_at_0x78: u64,
    pub word_at_0x90: u32,
    pub words_at_0x94: [u32; 4],
    pub word_at_0x8c: u32,
}

/// FUN_1402afbf0's conditional writes, given the original pointer-loaded values
/// and the three runtime parameter IDs in native call order.
pub fn write_optional_parameter_values(
    slots: &mut [[u32; 4]],
    ids: [i32; 3],
    values: OptionalParameterValues,
) -> Result<bool, String> {
    if values.gate_at_0x38 != 0 || values.indirect_gate_at_0x78 != 0 {
        return Ok(false);
    }
    for id in ids {
        parameter_slot_index(id, slots.len())?;
    }
    write_parameter_u32(slots, ids[0], values.word_at_0x90)?;
    write_parameter_4_words(slots, ids[1], values.words_at_0x94)?;
    write_parameter_u32(slots, ids[2], values.word_at_0x8c)?;
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
            MaterialParameterGroup::Effect(index) => usize::try_from(index)
                .ok()
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

/// Values propagated by FUN_14087e0a0 along the entity's ordered child list.
/// These are resolved actor fields, not values inferred from material names.
#[derive(Clone, Copy, Debug)]
pub struct InheritedModelDepthState {
    /// Zero dispatches vtable +0x98; nonzero dispatches +0x90. This does not
    /// assign unverified show/hide meaning to those virtual methods.
    pub dispatch_flags: u8,
    pub model_mask: u8,
    pub depth_hack: f32,
}

/// FUN_14087e040 resolves the actual parent-owner link. An absent parent or
/// parent model uses the original 0/1/0 input, not replacement actor defaults.
pub fn parent_model_depth_state(
    parent: Option<(u8, Option<(u8, f32)>)>,
) -> InheritedModelDepthState {
    let (dispatch_flags, model_mask, depth_hack) = match parent {
        None => (0, 1, 0.0),
        Some((flags, None)) => ((flags >> 1) & 1, 1, 0.0),
        Some((flags, Some((model_flags, depth)))) => {
            ((flags >> 1) & 1, (model_flags >> 3) & 1, depth)
        }
    };
    InheritedModelDepthState {
        dispatch_flags,
        model_mask,
        depth_hack,
    }
}

/// FUN_14087b760 writes only on float inequality, then requests the owner's
/// hierarchy refresh. +0/-0 compare equal; an unchanged NaN still refreshes.
pub fn set_local_model_depth_hack(local: &mut f32, value: f32) -> bool {
    if value != *local {
        *local = value;
        true
    } else {
        false
    }
}

/// FUN_14087e0a0's scalar owner resolution. A nonzero local value replaces
/// its inherited value (including negative/NaN); zero and signed zero inherit.
/// The model owner stores the result at +0x18b8 and replaces only bit 3 at
/// +0x18bc. The caller performs the ordered +0x90/+0x98 virtual callback and
/// walks its actual child list; neither lifecycle nor graph links are guessed.
pub fn resolve_inherited_model_depth_state(
    inherited: InheritedModelDepthState,
    local_depth_hack: f32,
    actor_flags_d0: u8,
    actor_mask_100: u8,
) -> InheritedModelDepthState {
    let depth_hack = if local_depth_hack != 0.0 {
        local_depth_hack
    } else {
        inherited.depth_hack
    };
    let mut model_mask = actor_mask_100 & 1;
    if depth_hack != 0.0 {
        model_mask &= inherited.model_mask;
    }
    InheritedModelDepthState {
        dispatch_flags: inherited.dispatch_flags | (actor_flags_d0 & 1),
        model_mask,
        depth_hack,
    }
}

/// FUN_14087e0a0's model mutation/parameter action. Its parameter predicate
/// is also used by FUN_140227c70: remove, then override only positive
/// non-unit depth hacks. 1.0 is the native default (DAT_141f697b0).
/// Parameter-array allocation and virtual lifecycle remain the actual owner's
/// responsibility; an action is not permission to fabricate a parameter ID.
pub fn publish_model_depth_state(
    model_flags_18bc: &mut u8,
    model_depth_hack: &mut f32,
    resolved: InheritedModelDepthState,
) -> Option<f32> {
    *model_flags_18bc = (*model_flags_18bc & 0xf7) | (resolved.model_mask << 3);
    *model_depth_hack = resolved.depth_hack;
    (resolved.depth_hack > 0.0 && resolved.depth_hack != 1.0).then_some(resolved.depth_hack)
}

/// FUN_1400596a0: copy the selected 0x50-byte clock record's +0x18 value.
/// This accessor establishes neither the time unit nor the clock's producer.
/// An invalid owned record index is unavailable rather than a native out-of-bounds read.
pub fn game_clock_group_time(records: &[[u8; 0x50]], group: usize) -> Option<f64> {
    records
        .get(group)
        .map(|record| f64::from_le_bytes(record[0x18..0x20].try_into().unwrap()))
}

/// FUN_140875cf0: reset the entity's group to zero, including model-less entities.
/// An attached model receives the double-to-float offset and renderer group 3:
/// DAT_1422952d8 contains the u32 mapping [3, 4, 2, 0], indexed with RAX*4.
/// Only bits 4..6 change; no offset is subtracted from the model's selected clock.
pub fn reset_entity_shader_clock(
    entity_group: &mut u32,
    entity_clock: f64,
    model: Option<(&mut f32, &mut u8)>,
) {
    *entity_group = 0;
    if let Some((offset, flags)) = model {
        *offset = (entity_clock - 0.0) as f32;
        *flags = (*flags & !0x70) | 0x30;
    }
}

/// FUN_140319560, 14031970d..14031975b: the owner's time-group selector.
/// This is only the model parameter prefix, not that function's complete
/// matrix/material publication. Clock group initialization belongs to its owner.
pub fn model_shader_time_group(model_flags_1c31: u8) -> usize {
    usize::from(model_flags_1c31 >> 4 & 7)
}

/// FUN_140319560: CVTPS2PD(offset), ADDSD(selected renderer clock),
/// CVTPD2PS, then the selected clock's unchanged delta. Do not round the clock
/// to f32 first or replace it with an unrelated elapsed/zero value.
pub fn model_shader_local_time(
    model_offset: f32,
    selected_clock: f64,
    selected_delta: f32,
) -> [f32; 2] {
    [
        (f64::from(model_offset) + selected_clock) as f32,
        selected_delta,
    ]
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
#[path = "render_model/tests.rs"]
mod tests;
