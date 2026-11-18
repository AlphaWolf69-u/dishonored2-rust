//! Screen-quad program inputs. Material definitions lease the canonical FX
//! image identities; they do not own another decoded texture/image registry.
use crate::{
    archive::Catalog,
    assets::environment::{ParameterReader, ParameterSource},
    entities::EnvironmentRenderParameters,
    render_program::*,
    shader_cache::ProgramLibrary,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Program {
    Distortion,
    Blend,
}
pub struct ScreenQuadMaterial {
    pub name: String,
    pub program: Program,
    pub state: crate::render_state::RenderState,
    pub layers: [crate::rendering::screen_distortion::DistortionLayer; 3],
    pub images: BTreeMap<String, Arc<crate::rendering::particles::image_resources::ParticleImage>>,
    pub options: [u32; 4],
    pub colour: [f32; 4],
    pub clamp_colour: [f32; 4],
    pub depth: [f32; 4],
    pub alpha_test: f32,
    /// screenblend fx/opt/use_diffuse: TextureColor's RGB is fx/diffuse at
    /// the opacity UV.
    pub diffuse: bool,
}
impl ScreenQuadMaterial {
    pub fn load(
        catalog: &Catalog,
        library: &mut ProgramLibrary,
        resources: &mut crate::rendering::particles::definition::ParticleResources,
        name: &str,
    ) -> Result<Self, String> {
        let bytes = crate::material::declaration(catalog, name)?;
        let main = crate::material::authored_program_override(&bytes, "mainProgram")
            .ok_or("screen quad MAIN absent")?;
        let program = match main.to_ascii_lowercase().as_str() {
            "postfx/screendisto" => Program::Distortion,
            "postfx/screenblend" => Program::Blend,
            _ => {
                return Err(format!(
                    "screen quad program not implemented: {main} ({name})"
                ));
            }
        };
        let definition = library
            .definition(main.as_bytes())?
            .ok_or("screen quad program definition absent")?;
        let defaults = definition.defaults.clone();
        let text = crate::entities::declaration_text(&bytes);
        let state = crate::render_state::material_render_state(&text, definition.render_state())?;
        let floats = crate::entities::material_float_parameters(&text, |name| {
            [
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
                "color_adjust",
                "postfx/ratioadaptation/clampcolor",
                "fx/opt/depthmodulation/startdepth",
                "fx/opt/depthmodulation/enddepth",
                "fx/opt/depthmodulation/startvalue",
                "fx/opt/depthmodulation/endvalue",
            ]
            .contains(&name)
        })?;
        let mut parameters = floats
            .into_iter()
            .map(|(name, words)| SerializedParameter {
                name: name.into_bytes(),
                kind: RenderParameterKind::Uniform,
                value: SerializedParameterValue::Words(words),
            })
            .collect::<Vec<_>>();
        parameters.extend(
            crate::entities::material_resource_parameters(&text, |n| {
                [
                    "fx/disto1",
                    "fx/disto2",
                    "fx/opacity",
                    "fx/diffuse",
                    "fx/lookup",
                ]
                .contains(&n)
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
        let mut option = |name: &str| {
            library
                .option_value(main.as_bytes(), &options, name.as_bytes())
                .map(|v| v as u32)
        };
        let settings = if program == Program::Distortion {
            [
                option("fx/opt/uv_disto2")?,
                option("PostFX/ratioAdaptation")?,
                option("PostFX/ratioAdaptation/useClampColor")?,
                option("fx/opt/DepthModulation")?,
            ]
        } else {
            // These are independent source program branches. Preserve enabled
            // variants explicitly rather than drawing them as the default.
            [
                option("fx/opt/uv_disto1")?,
                option("fx/opt/uv_disto2")?,
                option("fx/opt/use_clamp_sampler")?,
                option("fx/opt/no_opacity_map")?,
            ]
        };
        let diffuse = program == Program::Blend && option("fx/opt/use_diffuse")? != 0;
        if program == Program::Blend {
            for name in [
                "fx/opt/radial_ramp",
                "fx/opt/use_lookup",
                "fx/opt/uv_Disto2xDisto1",
                "fx/opt/use_disto_has_mask",
                "fx/opt/uv_disto1_uniform",
                "fx/opt/uv_disto2_uniform",
            ] {
                if option(name)? != 0 {
                    return Err(format!(
                        "screenblend branch requires its compiled shader port: {name}"
                    ));
                }
            }
        }
        let empty = EnvironmentRenderParameters::default();
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let scopes = parameters::ParameterScopes {
            view: &reader,
            blocks: &[&defaults, &material],
        };
        let seed = scopes.scalar("material_seed")?;
        let mut images = BTreeMap::new();
        for (index, name) in ["fx/disto1", "fx/disto2", "fx/opacity"]
            .into_iter()
            .enumerate()
        {
            let enabled = program == Program::Distortion
                || match index {
                    0 => settings[0] != 0,
                    1 => settings[1] != 0,
                    _ => settings[3] == 0,
                };
            let source = if enabled {
                scopes.image_source(name)?
            } else {
                "_white".to_owned()
            };
            images.insert(name.to_owned(), resources.image(catalog, &source)?);
        }
        let diffuse_source = if diffuse { scopes.image_source("fx/diffuse")? } else { "_white".to_owned() };
        images.insert("fx/diffuse".to_owned(), resources.image(catalog, &diffuse_source)?);
        let layers = ["disto1", "disto2", "opacity"]
            .map(|prefix| -> Result<_, String> {
                let tile = scopes.vector(&format!("{prefix}_tilling"))?;
                let pan = scopes.vector(&format!("{prefix}_panningspeed"))?;
                Ok(crate::rendering::screen_distortion::DistortionLayer {
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
            .map_err(|_| "screen quad layer count")?;
        Ok(Self {
            name: name.into(),
            program,
            state,
            layers,
            images,
            options: settings,
            colour: if program == Program::Blend {
                scopes.vector("color_Adjust")?
            } else {
                [1.; 4]
            },
            clamp_colour: if program == Program::Distortion {
                scopes.vector("PostFX/ratioAdaptation/ClampColor")?
            } else {
                [0.; 4]
            },
            depth: if program == Program::Distortion {
                [
                    scopes.scalar("fx/opt/DepthModulation/StartDepth")?,
                    scopes.scalar("fx/opt/DepthModulation/EndDepth")?,
                    scopes.scalar("fx/opt/DepthModulation/StartValue")?,
                    scopes.scalar("fx/opt/DepthModulation/EndValue")?,
                ]
            } else {
                [0.; 4]
            },
            alpha_test: scopes.scalar("particleAlphaTest")?,
            diffuse,
        })
    }
}
