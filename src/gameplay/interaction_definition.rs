//! The source `arkInteractionDef` value and its process-lifetime UID owner.
use std::sync::atomic::{AtomicU32, Ordering};
#[path = "interaction_enums.rs"]
mod enums;

// 77c2d0 assigns DAT_14398e390 and then increments the same 32-bit counter.
// Construction can run on preparation workers, so the one host counter uses
// relaxed atomic ordering; copying a value does not allocate another UID.
static NEXT_UID: AtomicU32 = AtomicU32::new(0);
static NEXT_REQUEST_ID: AtomicU32 = AtomicU32::new(0);

/// Process-wide DAT_143994a30, shared by interaction requests and state
/// request records. Both 3b7aa0 and 681f00 advance this same counter.
pub(crate) fn next_state_request_id() -> u32 {
    let previous = NEXT_REQUEST_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        let next = value.wrapping_add(1) & 0x7fff_ffff;
        Some(if next == 0 { 1 } else { next })
    }).expect("state request counter update cannot fail");
    let next = previous.wrapping_add(1) & 0x7fff_ffff;
    if next == 0 { 1 } else { next }
}

pub(crate) const ALLOW_SWIMMING: u8 = 1 << 0;
pub(crate) const ALLOW_SHADOW_WALKING: u8 = 1 << 1;
pub(crate) const ALLOW_POSSESSION: u8 = 1 << 2;
pub(crate) const SEND_USE_NOTIFICATION: u8 = 1 << 3;
pub(crate) const FRONT_ONLY: u8 = 1 << 4;
pub(crate) const LOCKED: u8 = 1 << 5;
const SOURCE_FLAGS: u8 = 0x3f;

/// A sensor retains the identity of the original owned record, not a second
/// InteractionDef. The state index stays fixed through the phase-2 barrier.
#[derive(Clone, Copy)]
pub(crate) enum RecordKind { Pickable, Usable, UsableLocked, UsableState(usize) }
#[derive(Clone, Copy)]
pub(crate) struct RecordReference {
    pub component: crate::gameplay::pickable_component::CheckedComponent,
    pub kind: RecordKind,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct SensorSelection {
    pub target: Option<crate::gameplay::pickable_component::CheckedComponent>,
    pub primary: Option<RecordReference>,
    pub secondary: Option<RecordReference>,
    pub npc: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub npc_with_primary_cast: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub npc_collision_kind: u32,
    pub npc_occlusion_transparent: bool,
}

/// The defined fields copied by 681fb0. The high two bits of the native flag
/// byte are unspecified padding and are not exposed as a separate state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InteractionDef {
    pub name: u32,
    pub usage_text: u32,
    pub maximum_distance: f32,
    pub maximum_height: f32,
    pub occlusion_shape: i32,
    pub player_mode: i32,
    pub player_state: i32,
    flags: u8,
    pub icon: i32,
    pub uid: u32,
}
impl InteractionDef {
    /// 1415b5030 reflects these fields; declaration decoding starts with
    /// 77c2d0's constructed value and applies only authored overrides.
    pub(crate) fn from_fields(fields: &[(String, crate::assets::entities::Value)]) -> Result<Self, String> {
        use crate::assets::entities::Value;
        let mut result = Self::new();
        for (name, value) in fields {
            let Value::Atom(value) = value else { return Err(format!("interaction {name} is not scalar")); };
            let value = value.trim_matches('"');
            let number = || value.parse::<f32>().map_err(|_| format!("invalid interaction {name}: {value}"));
            let integer = || value.parse::<i32>().map_err(|_| format!("invalid interaction {name}: {value}"));
            let flag = match name.as_str() {
                "m_allowedWhileSwimming" => Some(ALLOW_SWIMMING),
                "m_allowedWhileShadowWalking" => Some(ALLOW_SHADOW_WALKING),
                "m_allowedWhilePossessing" => Some(ALLOW_POSSESSION),
                "m_sendUseNotification" => Some(SEND_USE_NOTIFICATION),
                "m_allowOnlyFromFront" => Some(FRONT_ONLY),
                "m_lockedInteraction" => Some(LOCKED),
                _ => None,
            };
            if let Some(flag) = flag {
                match value { "true" | "1" => result.flags |= flag, "false" | "0" => result.flags &= !flag,
                    _ => return Err(format!("invalid interaction {name}: {value}")) }
                continue;
            }
            match name.as_str() {
                "m_name" => result.name = if value.is_empty() { 0 } else { crate::assets::lang::key_id(value) },
                "m_usageText" => result.usage_text = if value.is_empty() { 0 } else { crate::assets::lang::key_id(value) },
                "m_maximumDistance" => result.maximum_distance = number()?,
                "m_maxHeight" => result.maximum_height = number()?,
                "m_occlusionShapeIndex" => result.occlusion_shape = integer()?,
                "m_playerMode" => result.player_mode = decode_enum(enums::PLAYER_MODES, value)?,
                "m_playerState" => result.player_state = decode_enum(enums::PLAYER_STATES, value)?,
                "m_interactionIcon" => result.icon = interaction_icon(value)?,
                _ => return Err(format!("unknown interaction field: {name}")),
            }
        }
        Ok(result)
    }
    /// Native constructor 77c2d0; UID wrap is the source's 32-bit wrap.
    pub(crate) fn new() -> Self {
        Self { name: 0, usage_text: 0, maximum_distance: 3.0, maximum_height: 1000.0,
            occlusion_shape: 0, player_mode: 14, player_state: 91,
            flags: ALLOW_SWIMMING | ALLOW_POSSESSION | SEND_USE_NOTIFICATION,
            icon: -1, uid: NEXT_UID.fetch_add(1, Ordering::Relaxed) }
    }
    pub(crate) fn flags(&self) -> u8 { self.flags }
    pub(crate) fn set_flags(&mut self, flags: u8) { self.flags = flags & SOURCE_FLAGS; }
    pub(crate) fn has(&self, flag: u8) -> bool { self.flags & flag != 0 }
}

/// One of the two inline request records constructed by 681f00. Its
/// InteractionDef is the actual 77c2d0 value, including its own UID.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StateRequestRecord {
    pub serial: u32,
    pub state: i32,
    pub result: i32,
    pub words: [u32; 2],
    pub interaction: InteractionDef,
}

/// The fixed pair belongs to its state instance. Allocate each request ID
/// and its interaction UID in native constructor order, not at preparation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StateRequestPair {
    pub records: [StateRequestRecord; 2],
}
impl StateRequestPair {
    pub(crate) fn new() -> Self {
        Self { records: std::array::from_fn(|_| StateRequestRecord {
            serial: next_state_request_id(), state: 0x5b, result: -1,
            words: [0; 2], interaction: InteractionDef::new(),
        }) }
    }
}

fn decode_enum(table: &[(&str, i32)], value: &str) -> Result<i32, String> {
    table.iter().find_map(|(name, id)| (*name == value).then_some(*id))
        .or_else(|| value.parse::<i32>().ok())
        .ok_or_else(|| format!("unknown interaction enum: {value}"))
}

pub(crate) fn interaction_icon(value: &str) -> Result<i32, String> {
    decode_enum(enums::INTERACTION_ICONS, value)
}

/// The same source enum table used by interaction declarations also names
/// arkDeclPlayerState::m_id for the ordered mode constructor.
pub(crate) fn player_state_id(value: &str) -> Result<i32, String> {
    decode_enum(enums::PLAYER_STATES, value)
}

/// Live inputs read by 3b80a0 for a candidate Interactable. The height
/// endpoints come from that entity's real 8780d0 bounds, in game coordinates.
pub(crate) struct CandidateContext {
    pub interactable: crate::gameplay::pickable_component::CheckedComponent,
    pub player_mode: i32,
    pub is_possessing: bool,
    pub camera: [f32; 3],
    pub point: [f32; 3],
    pub target_forward: [f32; 3],
    pub target_min_z: f32,
    pub target_max_z: f32,
}

/// 3b7aa0's temporary state-admission request. Its serial is distinct from
/// InteractionDef's persistent UID and advances only when a request is made.
pub(crate) struct InteractionRequest {
    pub serial: u32,
    pub interactable: crate::gameplay::pickable_component::CheckedComponent,
    pub definition: InteractionDef,
    pub skip_checks: bool,
    pub from_grapple: bool,
}
impl InteractionRequest {
    pub(crate) fn new(interactable: crate::gameplay::pickable_component::CheckedComponent,
        definition: InteractionDef, from_grapple: bool) -> Self {
        Self { serial: next_state_request_id(), interactable, definition,
            skip_checks: false, from_grapple }
    }
}

/// 3b80a0's source policy, distance, facing and height checks. For a non-null
/// player state the caller must run the real 803280/803460 admission checks;
/// a missing state host cannot silently admit a candidate.
pub(crate) fn admits_candidate(definition: &InteractionDef, context: &CandidateContext,
    mut state_allowed: impl FnMut(i32, i32, &InteractionRequest) -> Result<bool, String>) -> Result<bool, String> {
    if (context.player_mode == 1 && !definition.has(ALLOW_SWIMMING))
        || (context.player_mode == 2 && !definition.has(ALLOW_SHADOW_WALKING))
        || (context.is_possessing && !definition.has(ALLOW_POSSESSION)) { return Ok(false); }
    if definition.player_state != 91 {
        let mode = if definition.player_mode == 14 { context.player_mode } else { definition.player_mode };
        let request = InteractionRequest::new(context.interactable, *definition, false);
        if !state_allowed(mode, definition.player_state, &request)? { return Ok(false); }
    }
    let x = context.camera[0] - context.point[0];
    let y = context.camera[1] - context.point[1];
    let z = context.camera[2] - context.point[2];
    if !(y * y + x * x + z * z <= definition.maximum_distance * definition.maximum_distance) { return Ok(false); }
    if definition.has(FRONT_ONLY)
        && y * context.target_forward[1] + x * context.target_forward[0] + z * context.target_forward[2] < 0.0 {
        return Ok(false);
    }
    let height = (context.camera[2] - (context.target_min_z + context.target_max_z) * 0.5).abs();
    if definition.maximum_height <= height && height != definition.maximum_height { return Ok(false); }
    Ok(true)
}
