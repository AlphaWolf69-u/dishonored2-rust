//! 226380/2263f0's arkFXParticleSystemDesc. This describes an FX owner and its
//! declarations; per-stage simulation data belongs to the particledt loader.
use super::*;
use crate::gameplay::effects::{
    envelope::{ScalarEnvelope, VectorEnvelope},
    timing::EffectTimingRanges,
};

pub struct ParticlePosition {
    pub screen_particle: bool,
    pub track_position: bool,
    pub track_orientation: bool,
    pub offset: VectorEnvelope,
    pub worldspace: bool,
    pub randomize_each_loop: bool,
}
pub struct ParticleSystemAsset {
    pub name: String,
    pub enabled: bool,
    pub parameter_block: Option<String>,
    pub declarations: Vec<Option<String>>,
    pub wait_system_end: bool,
    pub timing: EffectTimingRanges,
    pub looping: bool,
    pub finish_loop_on_stop: bool,
    pub position: ParticlePosition,
    pub rate: ScalarEnvelope,
    pub auto_stop_emitter: bool,
    pub randomize_rate_each_loop: bool,
    pub colour: VectorEnvelope,
    pub alpha: ScalarEnvelope,
}
impl Default for ParticleSystemAsset {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            parameter_block: None,
            declarations: Vec::new(),
            wait_system_end: true,
            timing: EffectTimingRanges::default(),
            looping: false,
            finish_loop_on_stop: false,
            position: ParticlePosition {
                screen_particle: false,
                track_position: true,
                track_orientation: true,
                offset: VectorEnvelope::constant([0.; 3]),
                worldspace: false,
                randomize_each_loop: false,
            },
            rate: ScalarEnvelope::constant(1.),
            auto_stop_emitter: true,
            randomize_rate_each_loop: false,
            colour: VectorEnvelope::constant([1.; 3]),
            alpha: ScalarEnvelope::constant(1.),
        }
    }
}

fn reference(value: &Value) -> Result<Option<String>, String> {
    let text = atom(value)?;
    Ok((!text.is_empty() && !text.eq_ignore_ascii_case("NULL")).then(|| text.to_owned()))
}

fn scalar(envelope: &mut ScalarEnvelope, name: &str, value: &Value) -> Result<bool, String> {
    match name {
        "m_max" => envelope.range[0] = number(value)?,
        "m_min" => envelope.range[1] = number(value)?,
        "m_durationCurve" => curves(&mut envelope.duration, scope(value)?)?,
        "m_fadeinCurve" => curves(&mut envelope.fade_in, scope(value)?)?,
        "m_fadeoutCurve" => curves(&mut envelope.fade_out, scope(value)?)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn vector_curve_pair(
    pair: &mut crate::gameplay::effects::vector_curve::EffectVectorCurvePair<3>,
    fields: &Fields,
) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_maxCurve" => particles::vector_curve(&mut pair.maximum, scope(value)?)?,
            "m_minCurve" => particles::vector_curve(&mut pair.minimum, scope(value)?)?,
            _ => return Err(format!("unknown FX vector curve-pair field {name}")),
        }
    }
    Ok(())
}

fn vector(envelope: &mut VectorEnvelope, name: &str, value: &Value) -> Result<bool, String> {
    match name {
        "m_max" => particles::vector(&mut envelope.range[0], scope(value)?)?,
        "m_min" => particles::vector(&mut envelope.range[1], scope(value)?)?,
        "m_uniform" => envelope.uniform = boolean(value)?,
        "m_durationCurve" => vector_curve_pair(&mut envelope.duration, scope(value)?)?,
        "m_fadeinCurve" => vector_curve_pair(&mut envelope.fade_in, scope(value)?)?,
        "m_fadeoutCurve" => vector_curve_pair(&mut envelope.fade_out, scope(value)?)?,
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn particle_system(fields: &Fields) -> Result<ParticleSystemAsset, String> {
    let mut result = ParticleSystemAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_parmBlock" => result.parameter_block = reference(value)?,
            "m_decls" => edit_list(
                &mut result.declarations,
                scope(value)?,
                None,
                |slot, value| {
                    *slot = reference(value)?;
                    Ok(())
                },
            )?,
            "m_waitSystemEnd" => result.wait_system_end = boolean(value)?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_looping" => result.looping = boolean(value)?,
                        "m_finishLoopOnStop" => result.finish_loop_on_stop = boolean(value)?,
                        "m_delay" => bounds(&mut result.timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut result.timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut result.timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut result.timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown particle FX timing field {name}")),
                    }
                }
            }
            "m_position" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_screenParticle" => result.position.screen_particle = boolean(value)?,
                        "m_trackOwnerPosition" => result.position.track_position = boolean(value)?,
                        "m_trackOwnerOrientation" => {
                            result.position.track_orientation = boolean(value)?
                        }
                        "m_offset" => {
                            for (name, value) in scope(value)? {
                                match name.as_str() {
                                    "m_worldspace" => result.position.worldspace = boolean(value)?,
                                    "m_randomizeEachLoop" => {
                                        result.position.randomize_each_loop = boolean(value)?
                                    }
                                    _ if vector(&mut result.position.offset, name, value)? => {}
                                    _ => {
                                        return Err(format!(
                                            "unknown particle FX offset field {name}"
                                        ))
                                    }
                                }
                            }
                        }
                        _ => return Err(format!("unknown particle FX position field {name}")),
                    }
                }
            }
            "m_emitterRate" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_autoStopEmitter" => result.auto_stop_emitter = boolean(value)?,
                        "m_randomizeEachLoop" => result.randomize_rate_each_loop = boolean(value)?,
                        _ if scalar(&mut result.rate, name, value)? => {}
                        _ => return Err(format!("unknown particle FX emitter-rate field {name}")),
                    }
                }
            }
            "m_color" => {
                for (name, value) in scope(value)? {
                    if !vector(&mut result.colour, name, value)? {
                        return Err(format!("unknown particle FX colour field {name}"));
                    }
                }
            }
            "m_alpha" => {
                for (name, value) in scope(value)? {
                    if !scalar(&mut result.alpha, name, value)? {
                        return Err(format!("unknown particle FX alpha field {name}"));
                    }
                }
            }
            _ => return Err(format!("unresolved particle FX field {name}")),
        }
    }
    Ok(result)
}
