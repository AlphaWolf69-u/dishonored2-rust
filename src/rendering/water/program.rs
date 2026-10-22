//! Selected world-water colour program, before GPU bytecode admission.
use crate::render_program::{D3dVariantData, SerializedParameterBlock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldWaterColourClass {
    ProbeHorizontal,
}

#[derive(Clone, Copy, Debug)]
pub struct WaterColourOptions {
    pub horizontal: bool,
    pub sunlight: bool,
    pub reflections: bool,
    pub sky_cube: bool,
    pub disable_rough_reflections: bool,
}

#[derive(Clone, Debug)]
pub struct WorldWaterColourSource {
    pub program: String,
    pub variant_hash: u32,
    pub variant: D3dVariantData,
    /// Effective material and constructor specialization overrides.
    pub options: SerializedParameterBlock,
    pub colour_options: WaterColourOptions,
    /// Admission depends on the actual installed stage bytecode, independently
    /// of program naming and material options. Unsupported ABIs stay explicit.
    pub class: Result<WorldWaterColourClass, String>,
    /// Selected program defaults, then material values. A different selected
    /// program must not inherit MAIN's uniform/image defaults.
    pub parameters: [SerializedParameterBlock; 2],
}

impl WorldWaterColourSource {
    pub fn settings(
        &self,
        view: &impl crate::assets::environment::ParameterSource,
    ) -> Result<super::WaterSettings, String> {
        super::WaterSettings::resolve(&crate::render_program::parameters::ParameterScopes {
            view,
            blocks: &[&self.parameters[0], &self.parameters[1]],
        })
    }
}
