//! Installed PostFX/screenDisto material inputs. Image programs resolve to
//! their shipped compiled resources; no procedural replacement noise is made.
use crate::{
    archive::Catalog,
    assets::environment::{ParameterReader, ParameterSource},
    entities::EnvironmentRenderParameters,
    render_program::*,
    shader_cache::ProgramLibrary,
};
use std::sync::Arc;

const FLOATS: &[&str] = &[
    "disto1_strength",
    "disto1_panningspeed",
    "disto1_rotationspeed",
    "disto1_tilling",
    "disto2_strength",
    "disto2_panningspeed",
    "disto2_rotationspeed",
    "disto2_tilling",
    "opacity_panningspeed",
    "opacity_rotationspeed",
    "opacity_tilling",
    "material_seed",
    "particlealphatest",
    "postfx/ratioadaptation/clampcolor",
    "fx/opt/depthmodulation/startdepth",
    "fx/opt/depthmodulation/enddepth",
    "fx/opt/depthmodulation/startvalue",
    "fx/opt/depthmodulation/endvalue",
];
const IMAGES: [&str; 3] = ["fx/disto1", "fx/disto2", "fx/opacity"];

#[derive(Clone, Copy, Debug)]
pub struct DistortionLayer {
    pub tiling_speed: [f32; 4],
    pub strength: [f32; 4],
    pub rotation_seed: [f32; 4],
}
pub struct ScreenDistortionMaterial {
    pub name: String,
    pub layers: [DistortionLayer; 3],
    pub images: [Arc<crate::texture::InlineCompressedChain>; 3],
    /// Disto2, ratio adaptation, clamp colour, depth modulation.
    pub options: [u32; 4],
    pub clamp_colour: [f32; 4],
    /// Start/end depth, start/end modulation value.
    pub depth: [f32; 4],
    pub alpha_test: f32,
    pub state: crate::render_state::RenderState,
}
impl ScreenDistortionMaterial {
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
            .ok_or_else(|| format!("screen effect material absent: {path}"))?;
        let main = crate::material::authored_program_override(&bytes, "mainProgram")
            .ok_or("screen effect MAIN absent")?;
        if !main.eq_ignore_ascii_case("postfx/screendisto") {
            return Err(format!(
                "unsupported screen effect program {main} in {name}"
            ));
        }
        let definition = library
            .definition(main.as_bytes())?
            .ok_or("screen distortion definition absent")?;
        let defaults = definition.defaults.clone();
        let state = crate::render_state::material_render_state(
            &crate::entities::declaration_text(&bytes),
            definition.render_state(),
        )?;
        let text = crate::entities::declaration_text(&bytes);
        let mut parameters =
            crate::entities::material_float_parameters(&text, |n| FLOATS.contains(&n))?
                .into_iter()
                .map(|(name, words)| SerializedParameter {
                    name: name.into_bytes(),
                    kind: RenderParameterKind::Uniform,
                    value: SerializedParameterValue::Words(words),
                })
                .collect::<Vec<_>>();
        parameters.extend(
            crate::entities::material_resource_parameters(&text, |n| IMAGES.contains(&n))?
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
        let empty = EnvironmentRenderParameters::default();
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let scopes = parameters::ParameterScopes {
            view: &reader,
            blocks: &[&defaults, &material],
        };
        let options = SerializedParameterBlock {
            flags: 0,
            parameters: crate::material::material_option_parameters(&bytes, |n| {
                library.parameter_kinds.get(n.as_bytes()) == Some(&RenderParameterKind::Option)
            })?
            .into_iter()
            .map(|(name, value)| SerializedParameter {
                name: name.into_bytes(),
                kind: RenderParameterKind::Option,
                value: SerializedParameterValue::Words([value as u32, 0, 0, 0]),
            })
            .collect(),
        };
        let mut option = |n: &str| library.option_value(main.as_bytes(), &options, n.as_bytes());
        let settings = [
            option("fx/opt/uv_disto2")? as u32,
            option("PostFX/ratioAdaptation")? as u32,
            option("PostFX/ratioAdaptation/useClampColor")? as u32,
            option("fx/opt/DepthModulation")? as u32,
        ];
        let seed = scopes.scalar("material_seed")?;
        let layers = ["disto1", "disto2", "opacity"]
            .map(|prefix| -> Result<_, String> {
                let tile = scopes.vector(&format!("{prefix}_tilling"))?;
                let pan = scopes.vector(&format!("{prefix}_panningspeed"))?;
                Ok(DistortionLayer {
                    tiling_speed: [tile[0], tile[1], pan[0], pan[1]],
                    strength: if prefix == "opacity" {
                        [0.; 4]
                    } else {
                        scopes.vector(&format!("{prefix}_strength"))?
                    },
                    rotation_seed: [
                        scopes.scalar(&format!("{prefix}_rotationspeed"))?,
                        seed,
                        0.,
                        0.,
                    ],
                })
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "screen distortion layer count")?;
        let images = IMAGES
            .map(|name| -> Result<_, String> {
                let source = scopes.image_source(name)?;
                crate::material::load_2d_image(catalog, &source).map(Arc::new)
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "screen distortion image count")?;
        Ok(Self {
            name: name.into(),
            layers,
            images,
            options: settings,
            state,
            clamp_colour: scopes.vector("PostFX/ratioAdaptation/ClampColor")?,
            depth: [
                scopes.scalar("fx/opt/DepthModulation/StartDepth")?,
                scopes.scalar("fx/opt/DepthModulation/EndDepth")?,
                scopes.scalar("fx/opt/DepthModulation/StartValue")?,
                scopes.scalar("fx/opt/DepthModulation/EndValue")?,
            ],
            alpha_test: scopes.scalar("particleAlphaTest")?,
        })
    }
}
