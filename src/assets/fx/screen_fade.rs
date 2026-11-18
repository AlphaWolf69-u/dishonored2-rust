//! arkFXScreenFadeDesc (constructor 14022a170: standard
//! timings, m_color a vector parameter of zero with constant-one curves,
//! m_alpha 219ef0(0, 0, 1): zero with the environment-style fade ramps).
use super::{boolean, bounds, curves, descriptor_list, number, scope, Fields};
use crate::gameplay::effects::{
    envelope::{ScalarEnvelope, VectorEnvelope},
    environment::EnvironmentEffectDefinition,
};

pub struct ScreenFadeAsset {
    pub name: String,
    pub enabled: bool,
    pub looping: bool,
    pub keep_duration_on_stop: bool,
    pub timing: crate::gameplay::effects::timing::EffectTimingRanges,
    pub colour: VectorEnvelope,
    pub alpha: ScalarEnvelope,
}

fn colour(value: &mut VectorEnvelope, fields: &Fields) -> Result<(), String> {
    let vector_curves = |pair: &mut crate::gameplay::effects::vector_curve::EffectVectorCurvePair<3>, fields: &Fields| -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_maxCurve" => super::particles::vector_curve(&mut pair.maximum, scope(value)?)?,
                "m_minCurve" => super::particles::vector_curve(&mut pair.minimum, scope(value)?)?,
                _ => return Err(format!("unknown FX colour curve field {name}")),
            }
        }
        Ok(())
    };
    for (name, field) in fields {
        match name.as_str() {
            "m_max" => super::particles::vector(&mut value.range[0], scope(field)?)?,
            "m_min" => super::particles::vector(&mut value.range[1], scope(field)?)?,
            "m_uniform" => value.uniform = boolean(field)?,
            "m_durationCurve" => vector_curves(&mut value.duration, scope(field)?)?,
            "m_fadeinCurve" => vector_curves(&mut value.fade_in, scope(field)?)?,
            "m_fadeoutCurve" => vector_curves(&mut value.fade_out, scope(field)?)?,
            _ => return Err(format!("unknown FX screen fade colour field {name}")),
        }
    }
    Ok(())
}

fn alpha(value: &mut ScalarEnvelope, fields: &Fields) -> Result<(), String> {
    for (name, field) in fields {
        match name.as_str() {
            "m_max" => value.range[0] = number(field)?,
            "m_min" => value.range[1] = number(field)?,
            "m_durationCurve" => curves(&mut value.duration, scope(field)?)?,
            "m_fadeinCurve" => curves(&mut value.fade_in, scope(field)?)?,
            "m_fadeoutCurve" => curves(&mut value.fade_out, scope(field)?)?,
            _ => return Err(format!("unknown FX screen fade alpha field {name}")),
        }
    }
    Ok(())
}

pub fn screen_fade(fields: &Fields) -> Result<ScreenFadeAsset, String> {
    let ramps = EnvironmentEffectDefinition::default();
    let mut fade = ScreenFadeAsset {
        name: String::new(),
        enabled: true,
        looping: false,
        keep_duration_on_stop: false,
        timing: Default::default(),
        colour: VectorEnvelope::constant([0.; 3]),
        alpha: ScalarEnvelope {
            range: [0.; 2],
            duration: ramps.duration.clone(),
            fade_in: ramps.fade_in.clone(),
            fade_out: ramps.fade_out.clone(),
        },
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => fade.name = super::atom(value)?.to_owned(),
            "m_enabled" => fade.enabled = boolean(value)?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_looping" => fade.looping = boolean(value)?,
                        "m_finishLoopOnStop" => fade.keep_duration_on_stop = boolean(value)?,
                        "m_delay" => bounds(&mut fade.timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut fade.timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut fade.timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut fade.timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown FX timing field {name}")),
                    }
                }
            }
            "m_color" => colour(&mut fade.colour, scope(value)?)?,
            "m_alpha" => alpha(&mut fade.alpha, scope(value)?)?,
            _ => return Err(format!("unknown FX screen fade field {name}")),
        }
    }
    Ok(fade)
}

pub(super) fn list(target: &mut Vec<std::sync::Arc<ScreenFadeAsset>>, fields: &Fields) -> Result<(), String> {
    descriptor_list(target, fields, |fields| screen_fade(fields).map(std::sync::Arc::new))
}
