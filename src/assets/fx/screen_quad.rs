//! 21d410's screen-quad descriptor, using the shared scalar blend envelope.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ScreenQuadStage {
    ShadowWalkEnd = 0,
    MotionVectorBegin = 1,
    MotionVectorEnd = 2,
    BeforeToneMap = 3,
    AfterToneMap = 4,
    #[default]
    Never = 5,
}

pub struct ScreenQuadAsset {
    pub name: String,
    pub enabled: bool,
    pub material: Option<String>,
    pub stage: ScreenQuadStage,
    pub blend: EnvironmentEffectDefinition,
}

pub fn screen_quad(fields: &[(String, Value)]) -> Result<ScreenQuadAsset, String> {
    // 21d410 and 21a000 both initialize their standard parameter via
    // 219ef0(1, 0, 1), and their standard timing via 224300. Reuse that
    // partial-edit reader rather than introducing a second envelope format.
    let common = fields
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "m_material" | "m_stage"))
        .cloned()
        .collect::<Vec<_>>();
    let common = environment_effect(&common)?;
    let mut result = ScreenQuadAsset {
        name: common.name,
        enabled: common.enabled,
        material: None,
        stage: ScreenQuadStage::Never,
        blend: common.blend,
    };
    for (name, value) in fields {
        match name.as_str() {
            "m_material" => {
                let name = atom(value)?;
                result.material = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                    .then(|| name.to_owned());
            }
            "m_stage" => {
                result.stage = match atom(value)? {
                    "SHADOWWALK_END" => ScreenQuadStage::ShadowWalkEnd,
                    "MOTIONVECTOR_BEGIN" => ScreenQuadStage::MotionVectorBegin,
                    "MOTIONVECTOR_END" => ScreenQuadStage::MotionVectorEnd,
                    "BEFORE_TONEMAP" => ScreenQuadStage::BeforeToneMap,
                    "AFTER_TONEMAP" => ScreenQuadStage::AfterToneMap,
                    "NEVER" => ScreenQuadStage::Never,
                    name => return Err(format!("unknown screen-quad stage {name}")),
                }
            }
            _ => {}
        }
    }
    Ok(result)
}
