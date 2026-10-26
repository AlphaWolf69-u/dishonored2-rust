//! arkCollisionFilterInfo words and the two pair rules that read them.
//!
//! A filter word is `layer | collidesWith << 5`: the low five bits name the
//! arkCollisionLayer_t of the body or query, the rest is the set of layers it
//! collides with. Level bodies carry authored words (hknpBodyCinfo
//! collisionFilterInfo); runtime bodies build theirs from a layer
//! (`filter_info`) and their collision settings (`shape_tags::material_filter`).

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

/// An arkCollisionLayerMask_t/Bits_t decl member (`m_NPC`, `m_WORLD_LOW`).
pub fn layer_member(member: &str) -> Option<u32> {
    let name = member.strip_prefix("m_")?;
    COLLISION_LAYERS
        .iter()
        .position(|n| n.strip_prefix("COLLISION_LAYER_") == Some(name))
        .map(|i| i as u32)
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
            Self::World(query) => pair_collides(query, target),
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

/// The COLLISION_LAYER enum (registered by 1419f0xxx), in value order.
pub const COLLISION_LAYERS: [&str; 27] = [
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

/// 141c764d0: the layers each layer accepts.
const LAYER_MASKS: [u32; 27] = [
    0x002e4778, 0x003e4f78, 0x001e4838, 0x005f4d7f, 0x005b4c3f, 0x001a4f7f, 0x0041c62b, 0x00300800,
    0x0000802b, 0x00430263, 0x00534c7b, 0x004044be, 0x00002000, 0x00001000, 0x00534c7f, 0x00000140,
    0x00004658, 0x0200463f, 0x0000000f, 0x0000003f, 0x000044be, 0x00000083, 0x00004e58, 0x00800000,
    0x00000000, 0x00020000, 0x00000000,
];

pub fn layer_index(name: &str) -> Option<u32> {
    COLLISION_LAYERS.iter().position(|l| *l == name).map(|i| i as u32)
}

/// 14081a5d0: a body's filter info, accepted layers above the layer.
pub fn filter_info(layer: u32) -> u32 {
    LAYER_MASKS[layer as usize] << 5 | layer & 31
}

/// arkCollisionFilter 140099610: a zero first word always collides;
/// otherwise each side must accept the other's layer.
pub fn pair_collides(first: u32, second: u32) -> bool {
    first == 0 || ((second >> 5) >> (first & 31) & 1 != 0 && (first >> 5) >> (second & 31) & 1 != 0)
}

/// Whether any layer's bodies can collide with this layer's.
pub fn layer_collides(layer: u32) -> bool {
    (0..COLLISION_LAYERS.len() as u32).any(|other| pair_collides(filter_info(layer), filter_info(other)))
}

#[cfg(test)]
#[path = "collision_filter/tests.rs"]
mod tests;
