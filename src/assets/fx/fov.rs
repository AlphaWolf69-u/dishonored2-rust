//! arkFXFovDesc21b130; schema1503e80 (+30 scalar FOV,+278 percentage).
use super::*;
use crate::gameplay::effects::{envelope::ScalarEnvelope, environment::EnvironmentEffectDefinition};

pub struct Asset {
    pub name: String,
    pub enabled: bool,
    pub common: std::sync::Arc<EnvironmentEffectDefinition>,
    pub value: ScalarEnvelope,
    pub percentage: bool,
}
pub fn descriptor(fields: &Fields) -> Result<Asset, String> {
    let common = environment_effect_fields(fields.iter().filter(|(name, _)|
        !matches!(name.as_str(), "m_fov" | "m_percentageMode")))?;
    let defaults = EnvironmentEffectDefinition::default();
    let mut result = Asset { name: common.name, enabled: common.enabled, common: common.blend,
        value: ScalarEnvelope { range: [90.; 2], duration: defaults.duration,
            fade_in: defaults.fade_in, fade_out: defaults.fade_out }, percentage: false };
    for (name, value) in fields {
        match name.as_str() {
            "m_fov" => for (name, value) in scope(value)? {
                match name.as_str() {
                    "m_max" => result.value.range[0] = number(value)?,
                    "m_min" => result.value.range[1] = number(value)?,
                    "m_durationCurve" => curves(&mut result.value.duration, scope(value)?)?,
                    "m_fadeinCurve" => curves(&mut result.value.fade_in, scope(value)?)?,
                    "m_fadeoutCurve" => curves(&mut result.value.fade_out, scope(value)?)?,
                    _ => return Err(format!("unknown FOV parameter field {name}")),
                }
            },
            "m_percentageMode" => result.percentage = boolean(value)?,
            _ => {},
        }
    }
    Ok(result)
}
