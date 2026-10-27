//! Render-mesh descriptors (1571800) and defaults (1e8ba0), distinct from
//! the PMODEL sources used to choose spawn positions and initial velocity.
use super::*;
use crate::gameplay::particles::{
    mesh_appearance::{MeshAppearance, MeshSpawnDefinition},
    orientation::{OrientationMode, ParticleOrientation},
    sprite::SpriteFlipbook,
};
use std::sync::Arc;

pub struct ParticleMeshAsset {
    pub enabled: bool,
    pub name: String,
    pub source: Option<String>,
    pub model: Option<Arc<crate::assets::particle_model::ParticleModel>>,
    pub material: Option<String>,
    pub orientation: ParticleOrientation,
    pub spawn: MeshSpawnDefinition,
    pub appearance: MeshAppearance,
    pub query_alpha: EffectFloatCurve,
    pub query_size: EffectFloatCurve,
    pub rotation_order: usize,
    pub use_flipbook: bool,
    pub flipbook: SpriteFlipbook,
}
impl Default for ParticleMeshAsset {
    fn default() -> Self {
        let mut orientation = ParticleOrientation::default();
        orientation.mode = OrientationMode::CustomZ;
        orientation.secondary_uses_view_direction = false;
        orientation.secondary =
            crate::gameplay::particles::parameters::VectorParameter::constant([0., 0., 1.]);
        Self {
            enabled: true,
            name: String::new(),
            source: None,
            model: None,
            material: None,
            orientation,
            spawn: Default::default(),
            appearance: Default::default(),
            query_alpha: EffectFloatCurvePair::constant(1.).maximum,
            query_size: EffectFloatCurvePair::constant(1.).maximum,
            rotation_order: 0,
            use_flipbook: true,
            flipbook: Default::default(),
        }
    }
}
impl ParticleMeshAsset {
    /// 1e9a80: model radius times spawn/LOD and lifetime/velocity size maxima.
    pub fn bounding_radius(&self) -> Result<f32, String> {
        use crate::float::multiply;
        let maximum = |a: f32, b: f32| if a <= b { b } else { a };
        let component_maximum = |v: [f32; 3]| maximum(v[0], maximum(v[1], v[2]));
        let Some(model) = &self.model else {
            return Ok(0.);
        };
        let mut spawn = component_maximum(self.spawn.size.maximum_magnitude()?);
        for lod in &self.spawn.size_lods {
            spawn = maximum(
                spawn,
                component_maximum(std::array::from_fn(|i| maximum(lod[0][i], lod[1][i]))),
            );
        }
        let mut radius = 0f32;
        for vertex in &model.vertices {
            let p = vertex.position;
            radius = radius.max((p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt());
        }
        Ok(multiply(
            multiply(
                component_maximum(self.appearance.size_over_velocity.maximum_magnitude()?),
                component_maximum(self.appearance.size_over_life.maximum_magnitude()?),
            ),
            multiply(radius, spawn),
        ))
    }
}
pub fn mesh(fields: &Fields) -> Result<ParticleMeshAsset, String> {
    let mut result = ParticleMeshAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_mesh" | "m_material" => {
                let text = atom(value)?;
                let target = if name == "m_mesh" {
                    &mut result.source
                } else {
                    &mut result.material
                };
                *target = (!text.is_empty() && !text.eq_ignore_ascii_case("NULL"))
                    .then(|| text.to_owned());
            }
            "m_orientation" => {
                super::particle_sprite::orientation(&mut result.orientation, scope(value)?)?
            }
            "m_spawn" => spawn(&mut result.spawn, scope(value)?)?,
            "m_queryVisibility" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_alpha" => curve(&mut result.query_alpha, scope(value)?)?,
                        "m_size" => curve(&mut result.query_size, scope(value)?)?,
                        _ => return Err(format!("unknown mesh query visibility {name}")),
                    }
                }
            }
            "m_colorOverLife" => {
                particles::vector_parameter(&mut result.appearance.colour_over_life, scope(value)?)?
            }
            "m_alphaOverLife" => {
                particles::scalar_parameter(&mut result.appearance.alpha_over_life, scope(value)?)?
            }
            "m_sizeOverLife" => {
                particles::vector_parameter(&mut result.appearance.size_over_life, scope(value)?)?
            }
            "m_sizeOverVelocity" => particles::vector_parameter(
                &mut result.appearance.size_over_velocity,
                scope(value)?,
            )?,
            "m_velocityOffsetOverLife" => particles::scalar_parameter(
                &mut result.appearance.velocity_offset_over_life,
                scope(value)?,
            )?,
            "m_pivotOffsetOverLife" => particles::vector_parameter(
                &mut result.appearance.pivot_offset_over_life,
                scope(value)?,
            )?,
            "m_rotationOverLife" => particles::vector_parameter(
                &mut result.appearance.rotation_over_life,
                scope(value)?,
            )?,
            "m_rotationRateOverLife" => particles::vector_parameter(
                &mut result.appearance.rotation_rate_over_life,
                scope(value)?,
            )?,
            "m_rotationOrder" => {
                result.rotation_order = match atom(value)? {
                    "idAngles::XYZ" => 0,
                    "idAngles::YZX" => 1,
                    "idAngles::ZXY" => 2,
                    "idAngles::XZY" => 3,
                    "idAngles::YXZ" => 4,
                    "idAngles::ZYX" => 5,
                    mode => return Err(format!("unknown particle mesh rotation order {mode}")),
                }
            }
            "m_useFlipbook" => result.use_flipbook = boolean(value)?,
            "m_flipBook" => super::particle_sprite::flipbook(&mut result.flipbook, scope(value)?)?,
            "m_maxBoundingRadius" => {}
            _ => return Err(format!("unknown particle mesh field {name}")),
        }
    }
    Ok(result)
}
fn spawn(result: &mut MeshSpawnDefinition, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_baseColor" => {
                for (name, value) in scope(value)? {
                    let i = match name.as_str() {
                        "r" => 0,
                        "g" => 1,
                        "b" => 2,
                        "a" => 3,
                        _ => return Err(format!("unknown mesh colour {name}")),
                    };
                    result.base_colour[i] = number(value)?;
                }
            }
            "m_useSpawnMeshVertexColor" => result.use_spawn_mesh_colour = boolean(value)?,
            "m_initialColor_lod" => particles::vector_lods(&mut result.colour_lods, scope(value)?, [1.; 3])?,
            "m_initialAlpha_lod" => particles::alpha_lods(&mut result.alpha_lods, scope(value)?)?,
            "m_initialSize_lod" => particles::vector_lods(&mut result.size_lods, scope(value)?, [1.; 3])?,
            "m_initialColor" => particles::vector_parameter(&mut result.colour, scope(value)?)?,
            "m_initialAlpha" => particles::scalar_parameter(&mut result.alpha, scope(value)?)?,
            "m_initialSize" => particles::vector_parameter(&mut result.size, scope(value)?)?,
            "m_initialPivot" => particles::vector_parameter(&mut result.pivot, scope(value)?)?,
            "m_initialRotation" => {
                particles::vector_parameter(&mut result.rotation, scope(value)?)?
            }
            _ => return Err(format!("unresolved mesh spawn field {name}")),
        }
    }
    Ok(())
}
