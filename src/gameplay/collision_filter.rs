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

#[cfg(test)]
#[path = "collision_filter/tests.rs"]
mod tests;
