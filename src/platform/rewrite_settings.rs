//! Explicit rendering improvements requested for the rewrite. These are
//! separate from the installed game's settings and authored environment data.
#[derive(Clone, Copy, Debug)]
pub struct RewriteShadowSettings {
    pub minimum_sun_caster_extent: f32,
}

impl RewriteShadowSettings {
    pub fn from_environment() -> Result<Self, String> {
        let minimum_sun_caster_extent = match std::env::var("DH2_SUN_SHADOW_MIN_EXTENT") {
            // Use authored area ranges unless an extension is explicitly requested.
            Err(std::env::VarError::NotPresent) => 0.0,
            Ok(value) => value
                .parse::<f32>()
                .map_err(|_| "DH2_SUN_SHADOW_MIN_EXTENT must be metres, or 0 for game settings")?,
            Err(error) => return Err(error.to_string()),
        };
        if !minimum_sun_caster_extent.is_finite() || minimum_sun_caster_extent < 0.0 {
            return Err("DH2_SUN_SHADOW_MIN_EXTENT must be finite and nonnegative".into());
        }
        Ok(Self {
            minimum_sun_caster_extent,
        })
    }
}
