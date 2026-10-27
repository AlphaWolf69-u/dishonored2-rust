//! Installed particledt declarations. FX owners reference these simulation
//! declarations; they are distinct from the owner's timing and colour controls.
use super::particle_stage::ParticleStageAsset;
use super::*;

/// 1c3b80 constructor defaults; reflected layout from1559820.
pub struct ParticleVisibility {
    pub always_visible: bool,
    pub always_simulate: bool,
    pub always_update_cycles: bool,
    pub alpha_culling: bool,
    pub flush_on_occlusion: bool,
    /// Serialized extents retain [maximum, minimum] order.
    pub local_box: [[f32; 3]; 2],
    pub world_box: [[f32; 3]; 2],
    pub fixed_box: bool,
}
impl Default for ParticleVisibility {
    fn default() -> Self {
        Self {
            always_visible: false,
            always_simulate: false,
            always_update_cycles: false,
            alpha_culling: false,
            flush_on_occlusion: false,
            local_box: [[1.; 3], [-1.; 3]],
            world_box: [[0.; 3]; 2],
            fixed_box: false,
        }
    }
}
impl ParticleVisibility {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_alwaysVisible" => self.always_visible = boolean(value)?,
                "m_alwaysSimulate" => self.always_simulate = boolean(value)?,
                "m_alwaysUpdateCycles" => self.always_update_cycles = boolean(value)?,
                "m_alphaCulling" => self.alpha_culling = boolean(value)?,
                "m_flushOnOcclusion" => self.flush_on_occlusion = boolean(value)?,
                "m_fixedBoundingBox" => self.fixed_box = boolean(value)?,
                "m_localBoundingBox" | "m_worldBoundingBox" => {
                    let target = if name == "m_localBoundingBox" {
                        &mut self.local_box
                    } else {
                        &mut self.world_box
                    };
                    for (name, value) in scope(value)? {
                        let index = match name.as_str() {
                            "m_max" => 0,
                            "m_min" => 1,
                            _ => return Err(format!("unknown particle visibility extent {name}")),
                        };
                        particles::vector(&mut target[index], scope(value)?)?;
                    }
                }
                _ => return Err(format!("unknown particle visibility field {name}")),
            }
        }
        Ok(())
    }
}

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

pub struct ParticleDistance {
    pub alpha: crate::gameplay::effects::curve::EffectFloatCurve,
    pub size: crate::gameplay::effects::curve::EffectFloatCurve,
}
impl Default for ParticleDistance {
    /// 1c1720 -> 1c4050, after00d0b0 initializes tangent mode4.
    fn default() -> Self {
        let unit = || crate::gameplay::effects::curve::EffectFloatCurve {
            times: vec![0.],
            values: vec![0., 1., 0.],
        };
        Self {
            alpha: unit(),
            size: unit(),
        }
    }
}
impl ParticleDistance {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            let target = match name.as_str() {
                "m_alpha" => &mut self.alpha,
                "m_size" => &mut self.size,
                _ => return Err(format!("unknown particle distance field {name}")),
            };
            curve(target, scope(value)?)?;
        }
        Ok(())
    }
}

/// 1f9060 and1579b70: independent cycle/lifetime scalar wind modulation.
pub struct ParticleWind {
    pub enabled: bool,
    pub over_cycle: crate::gameplay::particles::parameters::ScalarParameter,
    pub over_life: crate::gameplay::particles::parameters::ScalarParameter,
}
impl Default for ParticleWind {
    fn default() -> Self {
        Self {
            enabled: false,
            over_cycle: crate::gameplay::particles::parameters::ScalarParameter::constant(1.),
            over_life: crate::gameplay::particles::parameters::ScalarParameter::constant(1.),
        }
    }
}
impl ParticleWind {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_enabled" => self.enabled = boolean(value)?,
                "m_windOverCycle" => {
                    particles::scalar_parameter(&mut self.over_cycle, scope(value)?)?
                }
                "m_windOverLife" => {
                    particles::scalar_parameter(&mut self.over_life, scope(value)?)?
                }
                _ => return Err(format!("unknown particle wind field {name}")),
            }
        }
        Ok(())
    }
}

/// 1f9060 initializes the same two scalar records as wind; 15794c0 names
/// them independently for linear and angular inherited velocity.
pub struct ParticleVelocityInheritance {
    pub enabled: bool,
    pub linear: crate::gameplay::particles::parameters::ScalarParameter,
    pub angular: crate::gameplay::particles::parameters::ScalarParameter,
}
impl Default for ParticleVelocityInheritance {
    fn default() -> Self {
        Self {
            enabled: false,
            linear: crate::gameplay::particles::parameters::ScalarParameter::constant(1.),
            angular: crate::gameplay::particles::parameters::ScalarParameter::constant(1.),
        }
    }
}
impl ParticleVelocityInheritance {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_enabled" => self.enabled = boolean(value)?,
                "m_linearVelocityOverCycle" => particles::scalar_parameter(&mut self.linear, scope(value)?)?,
                "m_angularVelocityOverCycle" => particles::scalar_parameter(&mut self.angular, scope(value)?)?,
                _ => return Err(format!("unknown particle velocity inheritance field {name}")),
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct ParticleDeclaration {
    pub visibility: ParticleVisibility,
    pub underwater: bool,
    pub handle_seen_from_underwater: bool,
    pub clipping: ParticleClipping,
    pub distance: ParticleDistance,
    pub wind: ParticleWind,
    pub velocity_inheritance: ParticleVelocityInheritance,
    pub stages: Vec<ParticleStageAsset>,
}

pub fn declaration(fields: &Fields) -> Result<ParticleDeclaration, String> {
    let mut result = ParticleDeclaration::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_visibility" => result.visibility.edit(scope(value)?)?,
            "m_underwater" => result.underwater = boolean(value)?,
            "m_handleSeenFromUnderwater" => result.handle_seen_from_underwater = boolean(value)?,
            "m_clipping" => result.clipping.edit(scope(value)?)?,
            "m_distance" => result.distance.edit(scope(value)?)?,
            "m_wind" => result.wind.edit(scope(value)?)?,
            "m_velocityInheritance" => result.velocity_inheritance.edit(scope(value)?)?,
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
