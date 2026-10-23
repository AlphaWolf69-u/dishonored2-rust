//! Camera controls registered by 14136f100..14136fd30. These startup readers
//! reuse the shared CVar conversions; live registry subscriptions are separate.
use super::{float_cvar, integer_cvar, quoted_setting, read_configuration};
use crate::rendering::camera_effects::ConsoleSettings;
#[derive(Clone, Debug)]
pub struct CameraRenderSettings {
    pub console: Option<ConsoleSettings>,
    pub autofocus: [f32; 4],
    pub near_clip: f32,
    pub far_clip: f32,
    pub opaque_enabled: bool,
    pub near_mask_gain: f32,
    pub stencil_enabled: bool,
}
pub fn camera_render_settings(source: &str) -> Result<CameraRenderSettings, String> {
    let integer = |name, default| -> Result<i32, String> {
        Ok(quoted_setting(source, name)?
            .map(integer_cvar)
            .unwrap_or(default))
    };
    let float = |name, default| -> Result<f32, String> {
        Ok(quoted_setting(source, name)?
            .map(float_cvar)
            .unwrap_or(default))
    };
    let console = if integer("r_DofDebugMode", 0)? != 0 {
        let mut values = [0.; 33];
        for (index, name, default) in [
            (3, "r_DofAutofocusSpeed", 0.2),
            (6, "r_DofFocalDist", 5.),
            (7, "r_DofFocusRange", 10000.),
            (8, "r_DofFocusRangeSharp", 10000.),
            // Debug's far fields are ordered differently from environment parameters.
            (9, "r_DofFocusRangeFar", 1.),
            (10, "r_DofFocusRangeFarSharp", 1.),
            (11, "r_DofFocusBias", 0.),
            (12, "r_DofFringChroma", 0.6),
            (13, "r_DofFringThreshold", 0.45),
            (14, "r_DofFringGain", 1.),
            (15, "r_DofDistoCoef", -0.25),
            (16, "r_DofDistoCubicCoef", 0.45),
            (17, "r_DofSharpenCOef", 0.),
            (18, "r_DofVignOutBorder", 0.5),
            (19, "r_DofVignInBorder", -1.),
            (20, "r_DofVignFadeFStop", 22.),
        ] {
            values[index] = float(name, default)?;
        }
        for (index, name, default) in [
            (2, "r_DofAutofocus", 0),
            (5, "r_DofAsymmetricLens", 1),
            (22, "r_DofUseFringing", 1),
            (23, "r_DofUseCatsEye", 0),
            (24, "r_DofUseDisto", 0),
            (25, "r_DofUseSharpen", 0),
            (26, "r_DofUseVignette", 0),
            (28, "r_DofUseNoise", 0),
        ] {
            values[index] = integer(name, default)? as f32;
        }
        Some(ConsoleSettings {
            values,
            kernel: integer("r_DofKernelType", 0)?,
            physical_focus: integer("r_DofAutoCompute", 0)? != 0,
            physical_lens: [
                float("r_DofCOC", 0.025)?,
                float("r_DofFocalLength", 0.35)?,
                float("r_DofFStop", 4.)?,
            ],
        })
    } else {
        None
    };
    Ok(CameraRenderSettings {
        console,
        autofocus: [
            float("r_DofAutofocusRadius", 8.)?,
            float("r_DofAutofocusSharpSpeed", 0.4)?,
            float("r_DofAutofocusSharpnessMin", 100.)?,
            float("r_DofAutofocusSharpnessMax", 1000.)?,
        ],
        near_clip: float("r_znear", 0.01)?,
        far_clip: float("r_zfar", 7000.)?,
        opaque_enabled: integer("r_dofOpaqueEnabled", 1)? != 0,
        // 14136fc40 registers 1432f3fd0; 3185e0 reads its float at +0x2c.
        near_mask_gain: float("r_dofBlurCoef", 1.0)?,
        stencil_enabled: integer("r_DofStencilEnabled", 1)? != 0,
    })
}
pub fn read_camera_render_settings() -> Result<CameraRenderSettings, String> {
    let (_, source) = read_configuration()?;
    camera_render_settings(source.as_deref().unwrap_or(""))
}
