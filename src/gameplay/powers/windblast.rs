//! Windblast's complete power-component dispatch, 140448780/140448590.
//! The spawned cpntWindblast entity owns the blast, damage, force and FX.
//! A host without that entity component must return false from spawn.
use crate::{assets::entities::Value, gameplay::options::decl::atom};

#[derive(Clone, Debug)]
pub struct Settings {
    variants: [Option<String>; 4],
    pub hold_minimal_duration_for_shockwave: f32,
}
impl Settings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let path = |name| atom(fields, name).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL")).map(str::to_owned);
        let hold = match atom(fields, "m_holdMinimalDurationForShockwave") {
            None => 0.5, // 1404484a0, decl +0x198
            Some(value) => value.trim_end_matches('f').parse::<f32>().ok().filter(|v| v.is_finite())
                .ok_or_else(|| format!("invalid windblast hold duration: {value}"))?,
        };
        Ok(Self { variants: [path("m_windblastBasicEntityDef"), path("m_windblastGreaterEntityDef"),
            path("m_windblastShockwaveEntityDef"), path("m_windblastGreaterShockwaveEntityDef")],
            hold_minimal_duration_for_shockwave: hold })
    }
    /// The two upgrade attribute values are compared with their original
    /// bases by the attribute owner, not treated as arbitrary boolean tags.
    /// 140448780 chooses the 2-bit greater/shockwave variant before spawning.
    pub fn entity(&self, greater: bool, shockwave: bool) -> Option<&str> {
        self.variants[usize::from(greater) | (usize::from(shockwave) << 1)].as_deref()
    }
    /// The native spawn uses the player camera's full position and matrix,
    /// assigns the player entity as owner, then calls world attachment.
    /// Payment follows a successful spawn; a missing decl is a failed cast.
    pub fn cast(&self, greater: bool, shockwave: bool, mut spawn: impl FnMut(&str) -> bool) -> bool {
        self.entity(greater, shockwave).is_some_and(|entity| spawn(entity))
    }
}
