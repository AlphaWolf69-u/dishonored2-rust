//! arkFXRumbleDesc2281f0/2282b0, shared by the canonical FX declaration owner.
use super::*;
use crate::gameplay::effects::{envelope::ScalarEnvelope, timing::EffectTimingRanges};
use crate::gameplay::particles::parameters::ScalarParameter;

pub struct Asset {
    pub name: String,
    pub enabled: bool,
    pub looping: bool,
    pub keep_duration_on_stop: bool,
    pub timing: EffectTimingRanges,
    /// Magnitude, low/high motor, left/right trigger, in source RNG order.
    pub channels: [ScalarEnvelope; 5],
    pub distance_radius: [f32; 2],
    pub distance_fade: ScalarParameter,
    pub impulse: [f32; 2],
    pub impulse_fade: ScalarParameter,
}
fn envelope(parameter: &mut ScalarEnvelope, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_max" => parameter.range[0] = number(value)?,
            "m_min" => parameter.range[1] = number(value)?,
            "m_durationCurve" => curves(&mut parameter.duration, scope(value)?)?,
            "m_fadeinCurve" => curves(&mut parameter.fade_in, scope(value)?)?,
            "m_fadeoutCurve" => curves(&mut parameter.fade_out, scope(value)?)?,
            _ => return Err(format!("unknown rumble parameter field {name}")),
        }
    }
    Ok(())
}
pub fn descriptor(fields: &Fields) -> Result<Asset, String> {
    //2282b0→2262d0: all three maximum curves are constant one;
    //the motors' zero range is independent of those curve defaults.
    let mut result = Asset {
        name: String::new(),
        enabled: true,
        looping: false,
        keep_duration_on_stop: false,
        timing: Default::default(),
        channels: std::array::from_fn(|i| ScalarEnvelope::constant(if i == 0 { 1. } else { 0. })),
        distance_radius: [10.; 2],
        distance_fade: ScalarParameter {
            range: [1.; 2],
            curve: EffectFloatCurvePair {
                maximum: EffectFloatCurve {
                    times: vec![0., 1.],
                    values: vec![0., 1., 0., 0., 0., 0.],
                },
                minimum: Default::default(),
            },
        },
        impulse: [25.; 2],
        impulse_fade: ScalarParameter {
            range: [1.; 2],
            curve: EffectFloatCurvePair::constant(1.),
        },
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_timings" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_looping" => result.looping = boolean(value)?,
                        "m_finishLoopOnStop" => result.keep_duration_on_stop = boolean(value)?,
                        "m_delay" => bounds(&mut result.timing.delay, scope(value)?)?,
                        "m_fadein" => bounds(&mut result.timing.fade_in, scope(value)?)?,
                        "m_duration" => bounds(&mut result.timing.duration, scope(value)?)?,
                        "m_fadeout" => bounds(&mut result.timing.fade_out, scope(value)?)?,
                        _ => return Err(format!("unknown rumble timing field {name}")),
                    }
                }
            }
            "m_globalMagnitude" => envelope(&mut result.channels[0], scope(value)?)?,
            "m_lowMotor" => envelope(&mut result.channels[1], scope(value)?)?,
            "m_highMotor" => envelope(&mut result.channels[2], scope(value)?)?,
            "m_leftTrigger" => envelope(&mut result.channels[3], scope(value)?)?,
            "m_rightTrigger" => envelope(&mut result.channels[4], scope(value)?)?,
            "m_distanceRadius" => bounds(&mut result.distance_radius, scope(value)?)?,
            "m_distanceFade" => {
                super::particles::scalar_parameter(&mut result.distance_fade, scope(value)?)?
            }
            "m_impulse" => bounds(&mut result.impulse, scope(value)?)?,
            "m_impulseFade" => {
                super::particles::scalar_parameter(&mut result.impulse_fade, scope(value)?)?
            }
            _ => return Err(format!("unknown rumble descriptor field {name}")),
        }
    }
    Ok(result)
}
