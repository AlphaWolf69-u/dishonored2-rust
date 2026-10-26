//! Partial-edit readers for sprite spawn and lifetime appearance. Orientation,
//! visibility, flipbook and material fields have separate producer owners.
use super::*;
use crate::gameplay::particles::sprite::{SpriteAppearance, SpriteSpawnDefinition};

pub fn spawn(fields: &Fields) -> Result<SpriteSpawnDefinition, String> {
    let mut result = SpriteSpawnDefinition::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_baseColor" => {
                for (name, value) in scope(value)? {
                    let channel = match name.as_str() {
                        "r" => 0,
                        "g" => 1,
                        "b" => 2,
                        "a" => 3,
                        _ => return Err(format!("unknown particle base-colour channel {name}")),
                    };
                    result.base_colour[channel] = number(value)?;
                }
            }
            "m_useSpawnMeshVertexColor" => result.use_spawn_mesh_colour = boolean(value)?,
            "m_initialColor" => particles::vector_parameter(&mut result.colour, scope(value)?)?,
            "m_initialAlpha" => particles::scalar_parameter(&mut result.alpha, scope(value)?)?,
            "m_initialSize" => particles::vector_parameter(&mut result.size, scope(value)?)?,
            "m_initialPivot" => particles::vector_parameter(&mut result.pivot, scope(value)?)?,
            "m_initialRotation" => {
                particles::scalar_parameter(&mut result.rotation, scope(value)?)?
            }
            // Runtime LOD sampling exists, but newly allocated reflected LOD
            // entry defaults still require their own constructor evidence.
            "m_initialColor_lod" | "m_initialAlpha_lod" | "m_initialSize_lod" => {
                return Err(format!(
                    "particle sprite {name} entry defaults are unresolved"
                ));
            }
            _ => return Err(format!("unknown particle sprite spawn field {name}")),
        }
    }
    Ok(result)
}

/// Read only the lifetime appearance portion of a sprite descriptor. This is
/// not an admission check for orientation/visibility/material/flipbook support.
pub fn appearance(fields: &Fields) -> Result<SpriteAppearance, String> {
    let mut result = SpriteAppearance::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_colorOverLife" => {
                particles::vector_parameter(&mut result.colour_over_life, scope(value)?)?
            }
            "m_alphaOverLife" => {
                particles::scalar_parameter(&mut result.alpha_over_life, scope(value)?)?
            }
            "m_sizeOverLife" => {
                particles::vector_parameter(&mut result.size_over_life, scope(value)?)?
            }
            "m_sizeOverVelocity" => {
                particles::vector_parameter(&mut result.size_over_velocity, scope(value)?)?
            }
            "m_velocityOffsetOverLife" => {
                particles::scalar_parameter(&mut result.velocity_offset_over_life, scope(value)?)?
            }
            "m_pivotOffsetOverLife" => {
                particles::vector_parameter(&mut result.pivot_offset_over_life, scope(value)?)?
            }
            "m_rotationOverLife" => {
                particles::scalar_parameter(&mut result.rotation_over_life, scope(value)?)?
            }
            "m_rotationRateOverLife" => {
                particles::scalar_parameter(&mut result.rotation_rate_over_life, scope(value)?)?
            }
            _ => {}
        }
    }
    Ok(result)
}
