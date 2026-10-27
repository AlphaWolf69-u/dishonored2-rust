//! Texture-backed particle vector fields (1578430,213530,214160).
use super::*;
use crate::gameplay::particles::parameters::VectorParameter;
use std::sync::Arc;

pub struct FieldPixels {
    pub width: i32,
    pub height: i32,
    pub rg: Vec<u8>,
}
impl FieldPixels {
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let chain = crate::assets::texture::compressed_inline_chain(bytes)?;
        if chain.info.format != crate::assets::texture::ImageFormat::Rg8 {
            return Err("particle vector field needs its authored two-channel byte image".into());
        }
        let width = i32::try_from(chain.info.width)
            .map_err(|_| "field texture width exceeds signed coordinates")?;
        let height = i32::try_from(chain.info.height)
            .map_err(|_| "field texture height exceeds signed coordinates")?;
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(2))
            .ok_or("field texture size overflow")?;
        let rg = chain
            .payload
            .get(..size)
            .ok_or("truncated particle field image")?
            .to_vec();
        Ok(Self { width, height, rg })
    }
}

pub struct FieldTexture {
    pub source: Option<String>,
    pub high: [f32; 2],
    pub low: [f32; 2],
    pub tiling: bool,
    pub fading: bool,
    pub fade_center: [f32; 2],
    pub fade_power: [f32; 2],
    pub pixels: Option<Arc<FieldPixels>>,
}
impl Default for FieldTexture {
    fn default() -> Self {
        Self {
            source: None,
            high: [1.; 2],
            low: [-1.; 2],
            tiling: false,
            fading: false,
            fade_center: [0.5; 2],
            fade_power: [4.; 2],
            pixels: None,
        }
    }
}
impl FieldTexture {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_texture" => {
                    let source = atom(value)?;
                    self.source = (!source.is_empty() && !source.eq_ignore_ascii_case("NULL"))
                        .then(|| source.to_owned());
                    self.pixels = None;
                }
                "m_highValue" => particles::vector(&mut self.high, scope(value)?)?,
                "m_lowValue" => particles::vector(&mut self.low, scope(value)?)?,
                "m_tiling" => self.tiling = boolean(value)?,
                "m_useFading" => self.fading = boolean(value)?,
                "m_fadeCenter" => particles::vector(&mut self.fade_center, scope(value)?)?,
                "m_fadePower" => particles::vector(&mut self.fade_power, scope(value)?)?,
                _ => return Err(format!("unknown particle field texture field {name}")),
            }
        }
        Ok(())
    }
}

pub struct ParticleField {
    pub enabled: bool,
    pub worldspace: bool,
    pub target_position: bool,
    /// XY, XZ, YZ projection textures, in native accumulation order.
    pub textures: [FieldTexture; 3],
    pub position: VectorParameter,
    pub position_cycle: VectorParameter,
    pub scale: VectorParameter,
    pub scale_cycle: VectorParameter,
    pub initial_rotation: [[f32; 3]; 2],
    pub cycle_reset: bool,
    pub rotation_rate: VectorParameter,
    pub rotation_rate_cycle: VectorParameter,
    pub rotation_order: usize,
    pub modes: [u8; 3],
    pub strength: VectorParameter,
    pub strength_cycle: VectorParameter,
    pub strength_life: VectorParameter,
    pub constant_strength: VectorParameter,
    pub constant_strength_cycle: VectorParameter,
    pub constant_strength_life: VectorParameter,
}
impl Default for ParticleField {
    fn default() -> Self {
        Self {
            enabled: true,
            worldspace: false,
            target_position: false,
            textures: std::array::from_fn(|_| FieldTexture::default()),
            position: VectorParameter::constant([0.; 3]),
            position_cycle: VectorParameter::constant([0.; 3]),
            scale: VectorParameter::constant([1.; 3]),
            scale_cycle: VectorParameter::constant([1.; 3]),
            initial_rotation: [[0.; 3]; 2],
            cycle_reset: false,
            rotation_rate: VectorParameter::constant([0.; 3]),
            rotation_rate_cycle: VectorParameter::constant([0.; 3]),
            rotation_order: 0,
            modes: [0; 3],
            strength: VectorParameter::constant([1.; 3]),
            strength_cycle: VectorParameter::constant([1.; 3]),
            strength_life: VectorParameter::constant([1.; 3]),
            constant_strength: VectorParameter::constant([0.; 3]),
            constant_strength_cycle: VectorParameter::constant([1.; 3]),
            constant_strength_life: VectorParameter::constant([1.; 3]),
        }
    }
}

pub fn field(fields: &Fields) -> Result<ParticleField, String> {
    let mut result = ParticleField::default();
    for (name, value) in fields {
        let parameter = match name.as_str() {
            "m_position" => Some(&mut result.position),
            "m_positionOverCycle" => Some(&mut result.position_cycle),
            "m_scale" => Some(&mut result.scale),
            "m_scaleOverCycle" => Some(&mut result.scale_cycle),
            "m_rotationRate" => Some(&mut result.rotation_rate),
            "m_rotationRateOverCycle" => Some(&mut result.rotation_rate_cycle),
            "m_strength" => Some(&mut result.strength),
            "m_strengthOverCycle" => Some(&mut result.strength_cycle),
            "m_strengthOverLife" => Some(&mut result.strength_life),
            "m_constantStrength" => Some(&mut result.constant_strength),
            "m_constantStrengthOverCycle" => Some(&mut result.constant_strength_cycle),
            "m_constantStrengthOverLife" => Some(&mut result.constant_strength_life),
            _ => None,
        };
        if let Some(parameter) = parameter {
            particles::vector_parameter(parameter, scope(value)?)?;
            continue;
        }
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_worldspace" => result.worldspace = boolean(value)?,
            "m_useTargetPosition" => result.target_position = boolean(value)?,
            "m_textures" => {
                for (name, value) in scope(value)? {
                    let index = match name.as_str() {
                        "m_xy" => 0,
                        "m_xz" => 1,
                        "m_yz" => 2,
                        _ => return Err(format!("unknown field texture plane {name}")),
                    };
                    result.textures[index].edit(scope(value)?)?;
                }
            }
            "m_initialRotation" => {
                for (name, value) in scope(value)? {
                    let index = match name.as_str() {
                        "m_max" => 0,
                        "m_min" => 1,
                        "m_cycleReset" => {
                            result.cycle_reset = boolean(value)?;
                            continue;
                        }
                        _ => return Err(format!("unknown field initial rotation field {name}")),
                    };
                    particles::vector(&mut result.initial_rotation[index], scope(value)?)?;
                }
            }
            "m_rotationOrder" => {
                result.rotation_order = match atom(value)? {
                    "idAngles::XYZ" => 0,
                    "idAngles::YZX" => 1,
                    "idAngles::ZXY" => 2,
                    "idAngles::XZY" => 3,
                    "idAngles::YXZ" => 4,
                    "idAngles::ZYX" => 5,
                    mode => return Err(format!("unknown field rotation order {mode}")),
                }
            }
            "m_modes" => {
                for (name, value) in scope(value)? {
                    let index = match name.as_str() {
                        "m_X" => 0,
                        "m_Y" => 1,
                        "m_Z" => 2,
                        _ => return Err(format!("unknown field axis {name}")),
                    };
                    result.modes[index] = match atom(value)? {
                        "NORMAL" => 0,
                        "ABSOLUTE_POSITIVE" => 1,
                        "ABSOLUTE_NEGATIVE" => 2,
                        mode => return Err(format!("unknown field axis mode {mode}")),
                    };
                }
            }
            _ => return Err(format!("unknown particle vector field {name}")),
        }
    }
    Ok(result)
}
