//! arkDecal projection controls, resolved through the program/parameter owners.
//! The installed arkDecal source defines the four projection programs and the
//! FX opacity expression. No renderer-specific fallback material is substituted.
use crate::{
    assets::{
        archive::Catalog,
        entities,
        environment::{ParameterReader, ParameterSource},
    },
    rendering::render_program::{
        parameters::ParameterScopes, RenderParameterKind, SerializedParameter,
        SerializedParameterBlock, SerializedParameterValue,
    },
    shader_cache::ProgramLibrary,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ProjectionKind {
    None = 0,
    ClipModel = 1,
    CullModel = 2,
    DepthBox = 3,
}

#[derive(Clone, Debug)]
pub struct DecalProgram {
    pub kind: ProjectionKind,
    pub use_fx: bool,
    pub colour: [f32; 4],
    pub colour_adjust: [f32; 4],
    pub box_projection: [f32; 4],
    pub liquid_alpha_blur: f32,
    pub opacity: Option<String>,
    pub state: crate::render_state::RenderState,
}
impl DecalProgram {
    pub fn load(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        bytes: &[u8],
        main: &str,
        options: &SerializedParameterBlock,
    ) -> Result<Self, String> {
        let kind = match library.registered_option_value(options, b"decalType")? & 3 {
            0 => ProjectionKind::None,
            1 => ProjectionKind::ClipModel,
            2 => ProjectionKind::CullModel,
            3 => ProjectionKind::DepthBox,
            _ => unreachable!(),
        };
        let use_fx = library.option_value(main.as_bytes(), options, b"fx/opt/use_for_fx")? != 0;
        let names = [
            "color",
            "color_adjust",
            "decalboxprojection",
            "liquid_alpha_blur",
        ];
        let text = entities::declaration_text(bytes);
        let values = entities::material_float_parameters(&text, |name| names.contains(&name))?;
        let mut overrides = SerializedParameterBlock {
            flags: 0,
            parameters: values
                .into_iter()
                .map(|(name, words)| SerializedParameter {
                    name: name.into_bytes(),
                    kind: RenderParameterKind::Uniform,
                    value: SerializedParameterValue::Words(words),
                })
                .collect(),
        };
        let fallback = library
            .definition(main.as_bytes())?
            .ok_or("decal MAIN definition is absent")?
            .defaults
            .clone();
        let mut resolved = [[0.; 4]; 4];
        for (index, name) in names.iter().enumerate() {
            // 2da730: material override, selected program, registered default.
            resolved[index] = library
                .registered_float_vector(catalog, &overrides, name.as_bytes(), Some(&fallback))?
                .map(f32::from_bits);
        }
        let opacity = if use_fx {
            for (name, value) in
                entities::material_resource_parameters(&text, |name| name == "fx/opacity")?
            {
                overrides.parameters.push(SerializedParameter {
                    name: name.into_bytes(),
                    kind: RenderParameterKind::Texture,
                    value: SerializedParameterValue::Name(value.into_bytes()),
                });
            }
            let defaults = entities::EnvironmentRenderParameters::default();
            let reader = ParameterReader {
                catalog,
                parameters: &defaults,
            };
            Some(
                ParameterScopes {
                    view: &reader,
                    blocks: &[&fallback, &overrides],
                }
                .image_source("fx/opacity")?,
            )
        } else {
            None
        };
        let state = crate::render_state::material_render_state(
            &text,
            library
                .definition(main.as_bytes())?
                .ok_or("decal MAIN absent")?
                .render_state(),
        )?;
        Ok(Self {
            state,
            kind,
            use_fx,
            colour: resolved[0],
            colour_adjust: resolved[1],
            box_projection: resolved[2],
            liquid_alpha_blur: resolved[3][0],
            opacity,
        })
    }
}
