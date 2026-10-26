//! Installed particledt declarations. FX owners reference these simulation
//! declarations; they are distinct from the owner's timing and colour controls.
use super::particle_stage::ParticleStageAsset;
use super::*;

pub struct ParticleClipping {
    /// Authored order is [maximum, minimum], unlike simulation contains().
    pub bounding_box: [[f32; 3]; 2],
    pub use_ocean_plane: bool,
    pub invert_ocean_plane: bool,
    pub water_volume_raycast_distance: f32,
    pub water_volume_offset: f32,
    pub ocean_plane_offset: f32,
    pub rendering_offset: f32,
    pub rendering_fade_distance: f32,
}
impl Default for ParticleClipping {
    /// 1c3f80 supplies the +/-1e30 clipping box, not +/-FLT_MAX.
    fn default() -> Self {
        Self {
            bounding_box: [
                [f32::from_bits(0x7149_f2ca); 3],
                [f32::from_bits(0xf149_f2ca); 3],
            ],
            use_ocean_plane: false,
            invert_ocean_plane: false,
            water_volume_raycast_distance: 1.,
            water_volume_offset: 0.,
            ocean_plane_offset: 0.,
            rendering_offset: 0.,
            rendering_fade_distance: 0.,
        }
    }
}
impl ParticleClipping {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_boundingBox" => {
                    for (name, value) in scope(value)? {
                        let index = match name.as_str() {
                            "m_max" => 0,
                            "m_min" => 1,
                            _ => return Err(format!("unknown particle clipping box field {name}")),
                        };
                        particles::vector(&mut self.bounding_box[index], scope(value)?)?;
                    }
                }
                "m_useOceanPlane" => self.use_ocean_plane = boolean(value)?,
                "m_invertOceanPlane" => self.invert_ocean_plane = boolean(value)?,
                "m_waterVolumeRayCastDistance" => {
                    self.water_volume_raycast_distance = number(value)?
                }
                "m_waterVolumeOffset" => self.water_volume_offset = number(value)?,
                "m_oceanPlaneOffset" => self.ocean_plane_offset = number(value)?,
                "m_renderingOffset" => self.rendering_offset = number(value)?,
                "m_renderingFadeDistance" => self.rendering_fade_distance = number(value)?,
                _ => return Err(format!("unresolved particle clipping field {name}")),
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct ParticleDeclaration {
    pub underwater: bool,
    pub handle_seen_from_underwater: bool,
    pub clipping: ParticleClipping,
    pub stages: Vec<ParticleStageAsset>,
}

pub fn declaration(fields: &Fields) -> Result<ParticleDeclaration, String> {
    let mut result = ParticleDeclaration::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_underwater" => result.underwater = boolean(value)?,
            "m_handleSeenFromUnderwater" => result.handle_seen_from_underwater = boolean(value)?,
            "m_clipping" => result.clipping.edit(scope(value)?)?,
            "m_stages" => {
                for (name, value) in scope(value)? {
                    if name == "num" {
                        let count = atom(value)?
                            .parse::<usize>()
                            .map_err(|e| format!("particle stage count: {e}"))?;
                        result
                            .stages
                            .resize_with(count, ParticleStageAsset::default);
                    } else {
                        let index = name
                            .strip_prefix("item[")
                            .and_then(|v| v.strip_suffix(']'))
                            .ok_or_else(|| format!("unknown particle stage array field {name}"))?
                            .parse::<usize>()
                            .map_err(|e| format!("particle stage index: {e}"))?;
                        let slot = result.stages.get_mut(index).ok_or_else(|| {
                            format!("particle stage {index} exceeds declared count")
                        })?;
                        let fields = scope(value)?;
                        // Disabled producers remain in place for child-stage
                        // references, but are not instantiated or admitted.
                        if fields.iter().any(|(name, value)| {
                            name == "m_enabled"
                                && matches!(value, Value::Atom(value) if value == "false")
                        }) {
                            slot.enabled = false;
                        } else {
                            *slot = particle_stage::stage(fields)?;
                        }
                    }
                }
            }
            _ => return Err(format!("unresolved particle declaration producer {name}")),
        }
    }
    Ok(result)
}
