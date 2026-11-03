//! arkDeclFade (colour at +0x58, opacity +0xd0), sampled by 140167170.
use super::*;
use crate::gameplay::effects::{curve::EffectFloatCurve, vector_curve::EffectVectorCurvePair};
#[derive(Default)]
pub struct Fade {
    pub colour: EffectVectorCurvePair<3>,
    pub opacity: EffectFloatCurve,
}
impl Fade {
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields = crate::entities::parse_declaration(text).map_err(|e| e.to_string())?;
        let edit = fields
            .iter()
            .find(|(name, _)| name == "edit")
            .map(|(_, v)| scope(v))
            .transpose()?
            .unwrap_or(&fields);
        let mut result = Self::default();
        if let Some((_, fields)) = edit.iter().find(|(name, _)| name == "m_fadeCurves") {
            for (name, value) in scope(fields)? {
                match name.as_str() {
                    "m_color" => {
                        particles::vector_curve(&mut result.colour.maximum, scope(value)?)?
                    }
                    "m_opacity" => curve(&mut result.opacity, scope(value)?)?,
                    _ => return Err(format!("unknown fade curve {name}")),
                }
            }
        }
        Ok(result)
    }
    pub fn rgba(&self, time: f32) -> Result<[f32; 4], String> {
        let rgb = self.colour.sample(time, [0.; 3])?;
        let opacity = self.opacity.sample_visibility(time)?;
        Ok([
            rgb[0].clamp(0., 1.),
            rgb[1].clamp(0., 1.),
            rgb[2].clamp(0., 1.),
            opacity.clamp(0., 1.),
        ])
    }
}
