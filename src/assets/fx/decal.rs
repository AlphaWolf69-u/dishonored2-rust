//! arkFXDecalDesc (reflection 143ae3730), defaults 217d50. Descriptor
//! acquisition is separate from decal material residency and live pool slots.
use super::*;
use crate::{
    gameplay::effects::{decal::DecalAnimationDefinition, vector_curve::EffectVectorCurvePair},
    rendering::decals::DecalAttachmentDefinition,
};
use std::sync::Arc;

pub struct DecalAsset {
    pub name: String,
    pub enabled: bool,
    pub declarations: Vec<Option<String>>,
    pub finish_loop_on_stop: bool,
    pub animation: Arc<DecalAnimationDefinition>,
    pub attachment: DecalAttachmentDefinition,
}

impl Default for DecalAsset {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            declarations: Vec::new(),
            finish_loop_on_stop: false,
            animation: Arc::new(Default::default()),
            attachment: Default::default(),
        }
    }
}

fn phase(name: &str) -> Option<usize> {
    match name {
        "m_durationCurve" => Some(0),
        "m_fadeinCurve" => Some(1),
        "m_fadeoutCurve" => Some(2),
        _ => None,
    }
}

fn vector_pair<const N: usize>(
    pair: &mut EffectVectorCurvePair<N>,
    fields: &Fields,
) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_maxCurve" => particles::vector_curve(&mut pair.maximum, scope(value)?)?,
            "m_minCurve" => particles::vector_curve(&mut pair.minimum, scope(value)?)?,
            _ => return Err(format!("unknown decal vector curve-pair field {name}")),
        }
    }
    Ok(())
}

pub fn decal(fields: &Fields) -> Result<DecalAsset, String> {
    let mut result = DecalAsset::default();
    for (name, value) in fields {
        let animation = Arc::get_mut(&mut result.animation).expect("unpublished decal definition");
        let attachment = &mut result.attachment;
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_decls" => edit_list(
                &mut result.declarations,
                scope(value)?,
                None,
                |slot, value| {
                    let name = atom(value)?;
                    *slot = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                        .then(|| name.to_owned());
                    Ok(())
                },
            )?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_looping" => animation.looping = boolean(value)?,
                        "m_finishLoopOnStop" => result.finish_loop_on_stop = boolean(value)?,
                        "m_delay" => bounds(&mut animation.timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut animation.timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut animation.timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut animation.timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown decal timing field {name}")),
                    }
                }
            }
            "m_position" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_trackOwnerPosition" => attachment.follow_position = boolean(value)?,
                        "m_trackOwnerOrientation" => {
                            attachment.follow_orientation = boolean(value)?
                        }
                        _ => return Err(format!("unknown decal attachment field {name}")),
                    }
                }
            }
            "m_projectionAxis" => {
                // 141a31c20: idMat3::normal_axis_t literal table.
                attachment.projection_axis = match atom(value)? {
                    "AXIS_FORWARD" | "0" => 0,
                    "AXIS_LEFT" | "1" => 1,
                    "AXIS_UP" | "2" => 2,
                    axis => return Err(format!("unknown decal projection axis {axis}")),
                };
            }
            "m_centering" => particles::vector(&mut attachment.centering, scope(value)?)?,
            "m_initialPosition" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_max" => {
                            particles::vector(&mut attachment.initial_position[0], scope(value)?)?
                        }
                        "m_min" => {
                            particles::vector(&mut attachment.initial_position[1], scope(value)?)?
                        }
                        _ => return Err(format!("unknown decal position range field {name}")),
                    }
                }
            }
            "m_initialRotation" => bounds(&mut attachment.initial_rotation, scope(value)?)?,
            "m_size" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_max" => particles::vector(&mut animation.size_range[0], scope(value)?)?,
                        "m_min" => particles::vector(&mut animation.size_range[1], scope(value)?)?,
                        "m_uniform" => animation.shared_size_randomness = boolean(value)?,
                        _ => {
                            let index = phase(name)
                                .ok_or_else(|| format!("unknown decal size field {name}"))?;
                            vector_pair(&mut animation.phases[index].size, scope(value)?)?;
                        }
                    }
                }
            }
            "m_color" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_max" => {
                            particles::vector(&mut animation.colour_range[0], scope(value)?)?
                        }
                        "m_min" => {
                            particles::vector(&mut animation.colour_range[1], scope(value)?)?
                        }
                        "m_uniform" => animation.shared_colour_randomness = boolean(value)?,
                        _ => {
                            let index = phase(name)
                                .ok_or_else(|| format!("unknown decal colour field {name}"))?;
                            vector_pair(&mut animation.phases[index].colour, scope(value)?)?;
                        }
                    }
                }
            }
            "m_alpha" | "m_projectionDepth" => {
                for (field, value) in scope(value)? {
                    let range = if name == "m_alpha" {
                        &mut animation.opacity_range
                    } else {
                        &mut animation.depth_range
                    };
                    match field.as_str() {
                        "m_max" => range[0] = number(value)?,
                        "m_min" => range[1] = number(value)?,
                        _ => {
                            let index = phase(field)
                                .ok_or_else(|| format!("unknown decal {name} field {field}"))?;
                            let curve = if name == "m_alpha" {
                                &mut animation.phases[index].opacity
                            } else {
                                &mut animation.phases[index].depth
                            };
                            curves(curve, scope(value)?)?;
                        }
                    }
                }
            }
            _ => return Err(format!("unknown decal FX descriptor field {name}")),
        }
    }
    Ok(result)
}

pub struct DecalDefinition {
    pub asset: crate::assets::fx::decal::DecalAsset,
    pub declarations: Vec<Option<Arc<crate::assets::decal::DecalMaterials<String>>>>,
}
impl DecalDefinition {
    pub fn load(
        fields: &[(String, Value)],
        catalog: &crate::archive::Catalog,
        resources: &mut crate::assets::decal::DecalResources,
    ) -> Result<Self, String> {
        let asset = crate::assets::fx::decal::decal(fields)?;
        let declarations = asset
            .declarations
            .iter()
            .map(|name| {
                name.as_deref()
                    .map(|name| {
                        resources
                            .resolve(catalog, name)?
                            .ok_or_else(|| format!("missing decal declaration {name}"))
                    })
                    .transpose()
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            asset,
            declarations,
        })
    }
}
