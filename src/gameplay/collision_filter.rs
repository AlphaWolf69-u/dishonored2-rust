//! arkCollisionFilterInfo words and the two pair rules that read them.
//!
//! A filter word is `layer | collidesWith << 5`: the low five bits name the
//! arkCollisionLayer_t of the body or query, the rest is the set of layers it
//! collides with. Level bodies carry authored words (hknpBodyCinfo
//! collisionFilterInfo); runtime bodies build theirs from a layer
//! (FUN_14081a5d0) and an arkCollisionSettings_t (FUN_14081a5f0).

/// arkCollisionLayer_t (enum registration order 0..26).
pub mod layer {
    pub const WORLD_LOW: u32 = 0;
    pub const WORLD_LOW_HIGH: u32 = 1;
    pub const WORLD_HIGH: u32 = 2;
    pub const RIGID_BODY: u32 = 3;
    pub const DEBRIS: u32 = 4;
    pub const MOVER: u32 = 5;
    pub const PLAYER: u32 = 6;
    pub const PLAYER_HITBOX: u32 = 7;
    pub const PLAYER_DEAD: u32 = 8;
    pub const NPC: u32 = 9;
    pub const NPC_BODY_ANIMATED: u32 = 10;
    pub const PROJECTILE: u32 = 11;
    pub const INTERACTABLE: u32 = 15;
    pub const TRIGGER: u32 = 16;
}

const LAYER_NAMES: [&str; 27] = [
    "COLLISION_LAYER_WORLD_LOW",
    "COLLISION_LAYER_WORLD_LOW_HIGH",
    "COLLISION_LAYER_WORLD_HIGH",
    "COLLISION_LAYER_RIGID_BODY",
    "COLLISION_LAYER_DEBRIS",
    "COLLISION_LAYER_MOVER",
    "COLLISION_LAYER_PLAYER",
    "COLLISION_LAYER_PLAYER_HITBOX",
    "COLLISION_LAYER_PLAYER_DEAD",
    "COLLISION_LAYER_NPC",
    "COLLISION_LAYER_NPC_BODY_ANIMATED",
    "COLLISION_LAYER_PROJECTILE",
    "COLLISION_LAYER_CLOTH",
    "COLLISION_LAYER_CLOTH_COLLIDER",
    "COLLISION_LAYER_NPC_BODY_RAGDOLL",
    "COLLISION_LAYER_INTERACTABLE",
    "COLLISION_LAYER_TRIGGER",
    "COLLISION_LAYER_VISION",
    "COLLISION_LAYER_PLAYER_PHYSICS",
    "COLLISION_LAYER_DECAL_QUERY",
    "COLLISION_LAYER_WEAPON_MELEE",
    "COLLISION_LAYER_CHAIN",
    "COLLISION_LAYER_TRIGGER_WATER",
    "COLLISION_LAYER_PICKUP",
    "COLLISION_LAYER_24",
    "COLLISION_LAYER_GHOST",
    "COLLISION_LAYER_HIDDEN",
];

/// The collides-with set of each layer (DAT_141c764d0).
pub const LAYER_COLLIDES_WITH: [u32; 27] = [
    0x2e4778, 0x3e4f78, 0x1e4838, 0x5f4d7f, 0x5b4c3f, 0x1a4f7f, 0x41c62b, 0x300800, 0x802b,
    0x430263, 0x534c7b, 0x4044be, 0x2000, 0x1000, 0x534c7f, 0x140, 0x4658, 0x200463f, 0xf, 0x3f,
    0x44be, 0x83, 0x4e58, 0x800000, 0, 0x20000, 0,
];

/// A decl's arkCollisionLayer_t value.
pub fn layer_by_name(name: &str) -> Option<u32> {
    LAYER_NAMES.iter().position(|n| *n == name).map(|i| i as u32)
}

/// An arkCollisionLayerMask_t/Bits_t decl member (`m_NPC`, `m_WORLD_LOW`).
pub fn layer_member(member: &str) -> Option<u32> {
    let name = member.strip_prefix("m_")?;
    LAYER_NAMES
        .iter()
        .position(|n| n.strip_prefix("COLLISION_LAYER_") == Some(name))
        .map(|i| i as u32)
}

/// FUN_14081a5d0: a layer's filter word.
pub fn layer_filter(layer: u32) -> u32 {
    LAYER_COLLIDES_WITH[layer as usize] << 5 | layer & 31
}

/// arkCollisionSettings_t::arkBlockFlags_t (eBLOCK_* registration values).
pub mod block {
    pub const NPC: u16 = 1;
    pub const PLAYER: u16 = 2;
    pub const WEAPON: u16 = 4;
    pub const VISION: u16 = 8;
    pub const DYNAMIC: u16 = 0x10;
    pub const BLOCKED_BY_STATIC: u16 = 0x20;
    pub const DEBRIS_AND_PARTICLES: u16 = 0x40;
    pub const RAGDOLL: u16 = 0x80;
    /// eBLOCK_DEFAULT, the arkCollisionSettings_t constructor value (FUN_140088f70).
    pub const DEFAULT: u16 = 0x7f;
    pub const ALL: u16 = 0xffff;
}

/// FUN_14081a5f0: remove the collides-with layers a body's block flags leave
/// out. The default leaves the word unchanged; no flags keep only CLOTH and
/// PLAYER_PHYSICS.
pub fn apply_block_flags(filter: u32, flags: u16) -> u32 {
    if flags == block::DEFAULT {
        return filter;
    }
    if flags == 0 {
        return filter & 0x820001f;
    }
    let keep = |set: bool, layers: &[u32]| -> u32 {
        if set {
            u32::MAX
        } else {
            !layers.iter().fold(0, |m, l| m | 1 << (l + 5))
        }
    };
    let has = |bit: u16| flags & bit != 0;
    filter
        & keep(has(block::NPC), &[9, 10])
        & keep(has(block::NPC) || has(block::RAGDOLL), &[14])
        & keep(has(block::PLAYER), &[6])
        & keep(has(block::WEAPON), &[11, 20])
        & keep(has(block::VISION), &[17])
        & keep(has(block::DYNAMIC), &[3])
        & keep(has(block::BLOCKED_BY_STATIC), &[0, 1, 2])
        & keep(has(block::DEBRIS_AND_PARTICLES), &[4])
}

/// arkCollisionFilter's body and shape-key pair test (FUN_140099610 /
/// FUN_140099670): each word's collides-with set holds the other's layer. A
/// query word of zero accepts every target.
pub fn filters_collide(query: u32, target: u32) -> bool {
    query == 0 || (target >> 5 >> (query & 31)) & 1 != 0 && (query >> 5 >> (target & 31)) & 1 != 0
}

/// The filter a world query runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryFilter {
    /// The world's arkCollisionFilter with the query body's word (the
    /// character proxy's own casts).
    World(u32),
    /// An arkCollisionQueryFilter (arkPhysicsWorld's async and synchronous
    /// queries).
    Layers(CollisionQueryFilter),
}

impl QueryFilter {
    /// Every target (FUN_140099610's zero-word shortcut); the rewrite's
    /// diagnostics query with it.
    pub const ALL: Self = Self::World(0);

    pub fn layers(layers: u32, required_flags: u32) -> Self {
        Self::Layers(CollisionQueryFilter { layers, required_flags })
    }

    pub fn accepts(self, target: u32) -> bool {
        match self {
            Self::World(query) => filters_collide(query, target),
            Self::Layers(filter) => filter.accepts(target),
        }
    }
}

/// arkCollisionQueryFilter's body/shape-pair mask predicate (0996a0/0996d0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionQueryFilter {
    pub layers: u32,
    pub required_flags: u32,
}

impl CollisionQueryFilter {
    pub fn accepts(self, body_filter_info: u32) -> bool {
        (self.layers >> (body_filter_info & 31)) & 1 != 0
            && ((body_filter_info >> 5) & self.required_flags) == self.required_flags
    }

    /// 0996d0 selects the second shape's filter word when its target flag is
    /// nonzero; the body predicate0996a0 applies exactly the same mask rule.
    pub fn accepts_pair(self, first: u32, second: u32, target_is_second: bool) -> bool {
        self.accepts(if target_is_second { second } else { first })
    }
}

#[cfg(test)]
#[path = "collision_filter/tests.rs"]
mod tests;
