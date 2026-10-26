//! Installed particle/add_default inputs and specialization defaults. Texture
//! image programs resolve through the same archive path as other materials.
use crate::{
    archive::Catalog,
    assets::environment::{ParameterReader, ParameterSource},
    entities::EnvironmentRenderParameters,
    render_program::*,
    shader_cache::ProgramLibrary,
};
use std::{collections::BTreeMap, sync::Arc};

const OPTIONS: &[&str] = &[
    "fx/opt/radial_ramp",
    "fx/opt/use_lookup",
    "fx/opt/use_diffuse",
    "fx/opt/uv_disto1",
    "fx/opt/uv_disto2",
    "fx/opt/uv_Disto2xDisto1",
    "fx/opt/uv_disto1_uniform",
    "fx/opt/uv_disto2_uniform",
    "fx/opt/use_clamp_sampler",
    "fx/opt/use_mirror_sampler",
    "fx/opt/use_bilinear_sampler",
    "fx/opt/use_disto_has_mask",
    "fx/opt/use_flipbook",
    "fx/opt/flipbook_is_mask",
    "fx/opt/no_alpha_lookup_modulate",
    "fx/opt/opacity_scroll_mask",
    "fx/opt/use_softcontact",
    "fx/opt/view_dependent_fade",
    "fx/opt/use_lighting",
    "fx/opt/fresnel",
    "fx/opt/use_macroUV",
];
const UNIFORMS: &[&str] = &[
    "material_seed",
    "color_Adjust",
    "radial_RampPow",
    "lookup_adjust",
    "disto1_Strength",
    "disto1_Tilling",
    "disto1_PanningSpeed",
    "disto1_RotationSpeed",
    "disto2_Strength",
    "disto2_Tilling",
    "disto2_PanningSpeed",
    "disto2_RotationSpeed",
    "opacity_Tilling",
    "opacity_PanningSpeed",
    "opacity_RotationSpeed",
    "particleDistanceFadeIn",
    "particleDistanceFadeOut",
    "depth_Threshold",
    "particleAlphaTest",
];

pub struct ParticleMaterial {
    pub name: String,
    pub state: crate::render_state::RenderState,
    pub options: BTreeMap<String, i32>,
    pub uniforms: BTreeMap<String, [f32; 4]>,
    pub images: BTreeMap<String, Arc<crate::texture::InlineCompressedChain>>,
}
impl ParticleMaterial {
    pub fn option(&self, name: &str) -> Result<i32, String> {
        self.options
            .get(&name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| format!("particle option {name} was not resolved"))
    }
    pub fn uniform(&self, name: &str) -> Result<[f32; 4], String> {
        self.uniforms
            .get(&name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| format!("particle uniform {name} was not resolved"))
    }
    pub fn load(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        name: &str,
    ) -> Result<Self, String> {
        let path = format!(
            "generated/decls/material/{}.decl",
            name.to_ascii_lowercase()
        );
        let bytes = catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("particle material absent: {path}"))?;
        let program = crate::material::authored_program_override(&bytes, "mainProgram")
            .ok_or_else(|| format!("particle main program absent: {name}"))?;
        if !program.eq_ignore_ascii_case("particle/add_default") {
            return Err(format!("particle program {program} is not connected"));
        }
        let definition = library
            .definition(program.as_bytes())?
            .ok_or("particle program definition absent")?;
        let defaults = definition.defaults.clone();
        let text = crate::entities::declaration_text(&bytes);
        let state = crate::render_state::material_render_state(&text, definition.render_state())?;
        let mut parameters = crate::entities::material_float_parameters(&text, |name| {
            library.parameter_kinds.get(name.as_bytes()) == Some(&RenderParameterKind::Uniform)
        })?
        .into_iter()
        .map(|(name, words)| SerializedParameter {
            name: name.into_bytes(),
            kind: RenderParameterKind::Uniform,
            value: SerializedParameterValue::Words(words),
        })
        .collect::<Vec<_>>();
        parameters.extend(
            crate::entities::material_resource_parameters(&text, |name| {
                library.parameter_kinds.get(name.as_bytes()) == Some(&RenderParameterKind::Texture)
            })?
            .into_iter()
            .map(|(name, value)| SerializedParameter {
                name: name.into_bytes(),
                kind: RenderParameterKind::Texture,
                value: SerializedParameterValue::Name(value.into_bytes()),
            }),
        );
        let material = SerializedParameterBlock {
            flags: 0,
            parameters,
        };
        let overrides = SerializedParameterBlock {
            flags: 0,
            parameters: crate::material::material_option_parameters(&bytes, |name| {
                library.parameter_kinds.get(name.as_bytes()) == Some(&RenderParameterKind::Option)
            })?
            .into_iter()
            .map(|(name, value)| SerializedParameter {
                name: name.into_bytes(),
                kind: RenderParameterKind::Option,
                value: SerializedParameterValue::Words([value as u32, 0, 0, 0]),
            })
            .collect(),
        };
        let options = OPTIONS
            .iter()
            .map(|name| {
                library
                    .option_value(program.as_bytes(), &overrides, name.as_bytes())
                    .map(|value| (name.to_ascii_lowercase(), value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let empty = EnvironmentRenderParameters::default();
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let scopes = parameters::ParameterScopes {
            view: &reader,
            blocks: &[&defaults, &material],
        };
        let uniforms = UNIFORMS
            .iter()
            .map(|name| {
                scopes
                    .vector(name)
                    .map(|value| (name.to_ascii_lowercase(), value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut images = BTreeMap::new();
        for (image, option) in [
            ("fx/opacity", None),
            ("fx/disto1", Some("fx/opt/uv_disto1")),
            ("fx/disto2", Some("fx/opt/uv_disto2")),
            ("fx/diffuse", Some("fx/opt/use_diffuse")),
            ("fx/lookup", Some("fx/opt/use_lookup")),
        ] {
            if option.is_some_and(|name| options[name] == 0) {
                continue;
            }
            let source = scopes.image_source(image)?;
            images.insert(
                image.to_owned(),
                Arc::new(crate::material::load_2d_image(catalog, &source)?),
            );
        }
        Ok(Self {
            name: name.to_owned(),
            state,
            options,
            uniforms,
            images,
        })
    }
}
