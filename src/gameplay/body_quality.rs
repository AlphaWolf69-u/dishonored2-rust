//! Physics body quality presets (cc8390), distinct from graphics quality.
#[derive(Clone, Copy, Debug)]
pub struct BodyQuality {
    pub priority: u32,
    pub supported_flags: u32,
    pub requested_flags: u32,
    pub contact_caching_relative_movement_threshold: f32,
}
impl Default for BodyQuality {
    /// cc82e0 initializes the meaningful fields after reference ownership.
    fn default() -> Self {
        Self {
            priority: 0,
            supported_flags: u32::MAX,
            requested_flags: 0,
            contact_caching_relative_movement_threshold: f32::from_bits(0x3dcc_cccd),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct BodyQualityConfig {
    pub enable_welding_for_default_objects: bool,
    pub enable_welding_for_critical_objects: bool,
    pub enable_contact_caching: bool,
}
impl Default for BodyQualityConfig {
    /// cc8310; the world supplies its own three actual configuration switches.
    fn default() -> Self {
        Self {
            enable_welding_for_default_objects: false,
            enable_welding_for_critical_objects: true,
            enable_contact_caching: false,
        }
    }
}
pub struct BodyQualityLibrary {
    qualities: [BodyQuality; 32],
}
impl BodyQualityLibrary {
    /// cc8330 -> cc8390, replacing reference headers with owned records.
    pub fn new(config: BodyQualityConfig) -> Self {
        let caching = if config.enable_contact_caching {
            0x400
        } else {
            0
        };
        let mut q = std::array::from_fn::<_, 32, _>(|i| BodyQuality {
            priority: i as u32 * 10,
            supported_flags: (u32::MAX & !0x400) | caching,
            ..BodyQuality::default()
        });
        q[0].supported_flags &= !0x800;
        for (i, requested) in [(1, 0x25808), (2, 0x25a08)] {
            q[i].supported_flags &= 0xfffe_ffaf;
            q[i].requested_flags |= caching | requested;
            q[i].contact_caching_relative_movement_threshold = f32::from_bits(0x3e4c_cccd);
        }
        q[3].requested_flags |= caching | 0x20800;
        q[3].requested_flags |= if config.enable_welding_for_default_objects {
            0x80
        } else {
            0x20008
        };
        q[4].requested_flags |= caching | 0x20880;
        q[6].requested_flags |= caching | 0x20a08;
        q[5].requested_flags = (q[5].requested_flags & !0x400) | 0x20902;
        q[5].supported_flags &= !0x400;
        q[7].requested_flags |= caching | 0x10040;
        q[7].contact_caching_relative_movement_threshold = f32::from_bits(0x3c23_d70a);
        if config.enable_welding_for_critical_objects {
            q[7].requested_flags |= 0x82;
        }
        for (i, requested) in [(8, 0x100), (9, 0x80), (10, 0x10180), (11, 0x10102)] {
            q[i].supported_flags &= !0x400;
            q[i].requested_flags = (q[i].requested_flags & !0x400) | requested;
        }
        Self { qualities: q }
    }
    pub fn get(&self, id: u8) -> Option<&BodyQuality> {
        self.qualities.get(usize::from(id))
    }
    pub fn qualities(&self) -> &[BodyQuality; 32] {
        &self.qualities
    }
}

#[cfg(test)]
#[path = "body_quality/tests.rs"]
mod tests;
