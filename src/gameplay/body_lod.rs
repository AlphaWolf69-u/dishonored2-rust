//! Shape-query LOD configuration and variants, not visual mesh distances.
#[derive(Clone, Copy, Debug)]
pub struct BodyLodConfig {
    pub register_default_config: bool,
    pub auto_build_on_dynamic_body_added: bool,
    pub auto_build_on_mesh_body_added: bool,
    pub accuracy: f32,
    pub slow_to_fast_threshold: f32,
    pub fast_to_slow_threshold: f32,
    pub body_is_big_threshold: f32,
    pub average_velocity_gain: f32,
}
impl Default for BodyLodConfig {
    /// cc87b0. Names/offsets follow the executable's hknpLodManagerCinfo fields.
    fn default() -> Self {
        Self {
            register_default_config: false,
            auto_build_on_dynamic_body_added: false,
            auto_build_on_mesh_body_added: false,
            accuracy: f32::from_bits(0x3d4c_cccd),
            slow_to_fast_threshold: 4.,
            fast_to_slow_threshold: 1.,
            body_is_big_threshold: 4.,
            average_velocity_gain: f32::from_bits(0x3dcc_cccd),
        }
    }
}
pub struct LodVariants {
    table: [[u8; 8]; 8],
}
impl LodVariants {
    /// cc87f0's complete variant-table setup. Automatic shape construction and
    /// body-added subscriptions remain with the physics world owner.
    pub fn new(config: BodyLodConfig) -> Self {
        let mut variants = Self { table: [[0; 8]; 8] };
        variants.set_matching(0, 0, 0, 0, 2, 2);
        if config.register_default_config {
            variants.set_matching(4, 0, 4, 4, 3, 2);
            variants.set_matching(1, 1, 0, 0, 3, 3);
            variants.set_matching(1, 1, 1, 1, 4, 4);
            variants.set_matching(1, 1, 4, 4, 4, 3);
        }
        variants
    }
    /// cc8a90 -> cc89d0: apply in the original nested order, including the
    /// diagonal's second reversed write. Rule order determines precedence.
    pub fn set_matching(
        &mut self,
        query_mask: u32,
        query_value: u32,
        target_mask: u32,
        target_value: u32,
        query_variant: u8,
        target_variant: u8,
    ) {
        for query in 0..8 {
            if query as u32 & query_mask != query_value {
                continue;
            }
            for target in 0..8 {
                if target as u32 & target_mask == target_value {
                    self.table[query][target] = (query_variant << 4) | target_variant;
                    self.table[target][query] = (target_variant << 4) | query_variant;
                }
            }
        }
    }
    pub fn table(&self) -> &[[u8; 8]; 8] {
        &self.table
    }
}

#[cfg(test)]
#[path = "body_lod/tests.rs"]
mod tests;
