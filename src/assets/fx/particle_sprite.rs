//! Partial-edit sprite descriptors using 1ec240/1f8a70/1f8900's defaults.
use super::*;
use crate::gameplay::particles::{
    orientation::{OrientationMode, ParticleOrientation},
    sprite::{AngularFade, SpriteAppearance, SpriteFlipbook, SpriteSpawnDefinition, TextureFlip},
};

/// The complete reflected sprite descriptor. Material resolution and derived
/// geometric bounds belong to the rendering producer, rather than the reader.
pub struct ParticleSpriteAsset {
    pub enabled: bool,
    pub name: String,
    pub material: Option<String>,
    pub orientation: ParticleOrientation,
    pub spawn: SpriteSpawnDefinition,
    pub appearance: SpriteAppearance,
    pub minimal_distance: f32,
    pub query_alpha: EffectFloatCurve,
    pub query_size: EffectFloatCurve,
    pub angular_fade: AngularFade,
    pub flipbook: SpriteFlipbook,
    pub tessellation_distance: f32,
}
impl Default for ParticleSpriteAsset {
    fn default() -> Self {
        Self {
            enabled: true,
            name: String::new(),
            material: None,
            orientation: ParticleOrientation::default(),
            spawn: SpriteSpawnDefinition::default(),
            appearance: SpriteAppearance::default(),
            minimal_distance: -f32::MAX,
            query_alpha: EffectFloatCurvePair::constant(1.).maximum,
            query_size: EffectFloatCurvePair::constant(1.).maximum,
            angular_fade: AngularFade::new([30., 15.]),
            flipbook: SpriteFlipbook::default(),
            tessellation_distance: 0.,
        }
    }
}

fn orientation(result: &mut ParticleOrientation, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_mode" => {
                result.mode = match atom(value)?.rsplit("::").next().unwrap_or("") {
                    "SIMULATED" => OrientationMode::Emitter,
                    "VIEW" => OrientationMode::FaceCamera,
                    "VIEW_PLANAR" => OrientationMode::CameraAxes,
                    "VELOCITY_PLANAR" => OrientationMode::VelocityZ,
                    "VELOCITY_PERPENDICULAR" => OrientationMode::VelocityX,
                    "VECTOR_PLANAR" => OrientationMode::CustomZ,
                    "VECTOR_PERPENDICULAR" => OrientationMode::CustomX,
                    mode => return Err(format!("unknown particle orientation {mode}")),
                }
            }
            "m_viewAsOrientationAxis" => result.secondary_uses_view_direction = boolean(value)?,
            "m_primaryAxis" | "m_secondaryAxis" => {
                let (axis, worldspace) = if name == "m_primaryAxis" {
                    (&mut result.primary, &mut result.primary_worldspace)
                } else {
                    (&mut result.secondary, &mut result.secondary_worldspace)
                };
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_value" => particles::vector_parameter(axis, scope(value)?)?,
                        "m_worldspace" => *worldspace = boolean(value)?,
                        _ => return Err(format!("unknown particle orientation axis field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown particle orientation field {name}")),
        }
    }
    Ok(())
}

fn integer(value: &Value) -> Result<i32, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid particle integer: {e}"))
}

fn flipbook(result: &mut SpriteFlipbook, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_textureFlipU" | "m_textureFlipV" => {
                let flip = match atom(value)?.rsplit("::").next().unwrap_or("") {
                    "NONE" => TextureFlip::None,
                    "RANDOM" => TextureFlip::Random,
                    "ALWAYS" => TextureFlip::Always,
                    flip => return Err(format!("unknown particle texture flip {flip}")),
                };
                result.flips[usize::from(name == "m_textureFlipV")] = flip;
            }
            "m_type" => {
                result.cycle_rate = match atom(value)?.rsplit("::").next().unwrap_or("") {
                    "SINGLE_CYCLE" => false,
                    "CYCLE_RATE" => true,
                    mode => return Err(format!("unknown particle animation type {mode}")),
                }
            }
            "m_startFrame" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_framesRounding" => result.initial.divisions = integer(value)?,
                        "m_max" => result.initial.range[0] = number(value)?,
                        "m_min" => result.initial.range[1] = number(value)?,
                        _ => return Err(format!("unknown particle initial frame field {name}")),
                    }
                }
            }
            "m_rate" => particles::scalar_parameter(&mut result.rate, scope(value)?)?,
            _ => return Err(format!("unknown particle flipbook field {name}")),
        }
    }
    Ok(())
}

pub fn sprite(fields: &Fields) -> Result<ParticleSpriteAsset, String> {
    let mut result = ParticleSpriteAsset::default();
    result.appearance = appearance(fields)?;
    let mut angles = result.angular_fade.angles;
    for (name, value) in fields {
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_material" => {
                let path = atom(value)?;
                result.material = (!path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
                    .then(|| path.to_owned());
            }
            "m_spawn" => result.spawn = spawn(scope(value)?)?,
            "m_orientation" => orientation(&mut result.orientation, scope(value)?)?,
            "m_flipBook" => flipbook(&mut result.flipbook, scope(value)?)?,
            "m_angularFade" => bounds(&mut angles, scope(value)?)?,
            // Keep the installed declaration's misspelling.
            "m_visbility" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_minimalDistance" => result.minimal_distance = number(value)?,
                        _ => return Err(format!("unknown particle visibility field {name}")),
                    }
                }
            }
            "m_queryVisibility" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_alpha" => curve(&mut result.query_alpha, scope(value)?)?,
                        "m_size" => curve(&mut result.query_size, scope(value)?)?,
                        _ => return Err(format!("unknown particle query visibility field {name}")),
                    }
                }
            }
            "m_tessellation" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_distance" => result.tessellation_distance = number(value)?,
                        _ => return Err(format!("unresolved particle tessellation field {name}")),
                    }
                }
            }
            "m_colorOverLife"
            | "m_alphaOverLife"
            | "m_sizeOverLife"
            | "m_sizeOverVelocity"
            | "m_velocityOffsetOverLife"
            | "m_pivotOffsetOverLife"
            | "m_rotationOverLife"
            | "m_rotationRateOverLife" => {}
            // Derived caches are recomputed from editable fields, as in the
            // descriptor's post-load callbacks.
            "m_angleFade_DotScale" | "m_angleFade_DotMin" | "m_maxBoundingRadius" => {}
            _ => return Err(format!("unknown particle sprite field {name}")),
        }
    }
    result.angular_fade = AngularFade::new(angles);
    Ok(result)
}

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
