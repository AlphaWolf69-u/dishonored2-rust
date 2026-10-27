//! Partial-edit particle movement parameters. Stage/emitter/sprite descriptor
//! fields have their own readers; this module does not discard them as motion.
use super::*;
use crate::gameplay::{
    effects::vector_curve::*,
    particles::{parameters::*, MotionParameters},
};

pub(super) fn vector<const N: usize>(value: &mut [f32; N], fields: &Fields) -> Result<(), String> {
    for (name, field) in fields {
        let index = match name.as_str() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => return Err(format!("unknown particle vector field {name}")),
        };
        *value
            .get_mut(index)
            .ok_or_else(|| format!("particle vector channel {name} exceeds {N} channels"))? =
            number(field)?;
    }
    Ok(())
}

pub(super) fn vector_curve<const N: usize>(
    curve: &mut EffectVectorCurve<N>,
    fields: &Fields,
) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_times" => float_list(&mut curve.times, scope(value)?)?,
            "m_values" => edit_list(&mut curve.values, scope(value)?, [0.; N], |slot, value| {
                vector(slot, scope(value)?)
            })?,
            "m_loopMode" | "m_interpolationMode" | "m_tangentsIn" | "m_tangentsOut" => {}
            _ => return Err(format!("unknown particle vector curve field {name}")),
        }
    }
    Ok(())
}

/// Reflected colour/size LOD lists contain only MinMax ranges. New colour
/// and mesh-size entries are one (1ecbb0); sprite size is .25 (1ecc00).
pub(super) fn vector_lods(
    values: &mut Vec<[[f32; 3]; 2]>, fields: &Fields, initial: [f32; 3],
) -> Result<(), String> {
    edit_list(values, fields, [initial; 2], |slot, value| {
        for (name, value) in scope(value)? {
            let target = match name.as_str() {
                "m_max" => &mut slot[0],
                "m_min" => &mut slot[1],
                _ => return Err(format!("unknown particle LOD range field {name}")),
            };
            vector(target, scope(value)?)?;
        }
        Ok(())
    })
}
/// 1ec230 initializes both extrema of sprite/mesh alpha LOD entries to one.
pub(super) fn alpha_lods(values: &mut Vec<[f32; 2]>, fields: &Fields) -> Result<(), String> {
    edit_list(values, fields, [1.; 2], |slot, value| bounds(slot, scope(value)?))
}

pub fn scalar_parameter(parameter: &mut ScalarParameter, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_max" => parameter.range[0] = number(value)?,
            "m_min" => parameter.range[1] = number(value)?,
            "m_curve" => curves(&mut parameter.curve, scope(value)?)?,
            _ => return Err(format!("unknown particle scalar parameter field {name}")),
        }
    }
    Ok(())
}

pub fn vector_parameter<const N: usize>(
    parameter: &mut VectorParameter<N>,
    fields: &Fields,
) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_max" => vector(&mut parameter.range[0], scope(value)?)?,
            "m_min" => vector(&mut parameter.range[1], scope(value)?)?,
            "m_uniform" => parameter.uniform = boolean(value)?,
            "m_curve" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_maxCurve" => vector_curve(&mut parameter.curve.maximum, scope(value)?)?,
                        "m_minCurve" => vector_curve(&mut parameter.curve.minimum, scope(value)?)?,
                        _ => {
                            return Err(format!("unknown particle vector curve-pair field {name}"))
                        }
                    }
                }
            }
            _ => return Err(format!("unknown particle vector parameter field {name}")),
        }
    }
    Ok(())
}

/// Read only the movement portion of a stage. The complete stage reader must
/// consume the other fields; this is not an admission check for an entire FX.
pub fn motion_parameters(fields: &Fields) -> Result<MotionParameters, String> {
    let mut result = MotionParameters::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_velocityOverLife" => vector_parameter(&mut result.velocity, scope(value)?)?,
            "m_linearAccelerationOverLife" => {
                scalar_parameter(&mut result.linear_acceleration, scope(value)?)?
            }
            "m_accelerationOverLife" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_value" => vector_parameter(&mut result.acceleration, scope(value)?)?,
                        "m_worldspace" => result.acceleration_worldspace = boolean(value)?,
                        // 141573cd0: acceleration_t+118 is a standard
                        // scalar curve parameter, not a plain gravity float.
                        "m_gravity" => scalar_parameter(&mut result.gravity, scope(value)?)?,
                        _ => return Err(format!("unresolved particle acceleration field {name}")),
                    }
                }
            }
            _ => {}
        }
    }
    Ok(result)
}
