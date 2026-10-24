//! arkCollisionQueryFilter's body/shape-pair mask predicate (0996a0/0996d0).

#[derive(Clone, Copy, Debug)]
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
