//! Selected world-water colour program, before GPU bytecode admission.
use crate::render_program::{self, D3dVariantData, SerializedParameterBlock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorldWaterColourClass {
    ProbeHorizontal,
    Horizontal,
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

/// Program defaults and root material overrides, in publication order.
pub fn parameters(
    library: &mut crate::shader_cache::ProgramLibrary,
    bytes: &[u8],
    main: &str,
) -> Result<[render_program::SerializedParameterBlock; 2], String> {
    use render_program::{
        RenderParameterKind, SerializedParameter, SerializedParameterBlock,
        SerializedParameterValue,
    };
    let defaults = library
        .definition(main.as_bytes())?
        .ok_or("water MAIN definition absent")?
        .defaults
        .clone();
    let text = crate::entities::declaration_text(bytes);
    let numeric = crate::entities::material_float_parameters(&text, |name| {
        crate::rendering::water::FLOAT_PARAMETER_NAMES.contains(&name)
            || super::rays::FLOAT_PARAMETERS.contains(&name)
    })?;
    let images = crate::entities::material_resource_parameters(&text, |name| {
        crate::rendering::water::IMAGE_PARAMETER_NAMES.contains(&name)
    })?;
    let mut parameters: Vec<_> = numeric
        .into_iter()
        .map(|(name, words)| SerializedParameter {
            name: name.into_bytes(),
            kind: RenderParameterKind::Uniform,
            value: SerializedParameterValue::Words(words),
        })
        .collect();
    parameters.extend(crate::entities::material_integer_parameters(&text, |name| {
        super::rays::BOOLEAN_PARAMETERS.contains(&name)
    })?.into_iter().map(|(name, value)| SerializedParameter {
        name: name.into_bytes(),
        kind: RenderParameterKind::Uniform,
        value: SerializedParameterValue::Words([value as u32, 0, 0, 0]),
    }));
    parameters.extend(images.into_iter().map(|(name, image)| SerializedParameter {
        name: name.into_bytes(),
        kind: RenderParameterKind::Texture,
        value: SerializedParameterValue::Name(image.into_bytes()),
    }));
    Ok([
        defaults,
        SerializedParameterBlock {
            flags: 0,
            parameters,
        },
    ])
}
