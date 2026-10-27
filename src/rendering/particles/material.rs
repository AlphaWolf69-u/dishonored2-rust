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
    "twoSidedLighting",
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
    "fx/opt/fresnel/useNormalMap",
    "fx/opt/fresnel/useLUT",
    "fx/opt/use_macroUV",
];
const UNIFORMS: &[&str] = &[
    "fx/auto_exposure_amount",
    "particleFogScatterPower",
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

pub struct ParticleLightingMaterial {
    pub pixel: Option<ParticlePixelMaterial>,
    pub gloss: [f32; 2],
    pub metallic: [f32; 2],
    /// Active gloss, metallic and specular texture reads, in that order.
    pub maps: [bool; 3],
    pub bump: bool,
    /// Detail normals are read only inside the base bump-map branch.
    pub detail_bump: Option<[f32; 2]>,
    pub uv: [f32; 4],
    pub gloss_in_alpha: bool,
    pub dielectric: [f32; 4],
    pub metal_diffuse: [f32; 4],
    pub shadow_flags: u32,
    pub shadow_filtering: i32,
}

pub struct ParticlePixelMaterial {
    pub self_shadowed_bump: bool,
    pub fuzz: Option<[f32; 2]>,
    pub fuzz_mask: bool,
    pub translucency: Option<ParticleTranslucencyMaterial>,
    pub sheen: [f32; 2],
    /// Diffuse lookup, sheen, emissive and emissive-mask texture reads.
    pub maps: [bool; 4],
    pub emissive: [f32; 4],
    pub emissive_clip: [f32; 2],
    pub exposure_emissive: bool,
}

pub struct ParticleTranslucencyMaterial {
    pub vertex_alpha: bool,
    /// Distortion, power, intensity and minimum ramp position.
    pub phase: [f32; 4],
    /// Inner, medium and outer colours; inner.w carries the ramp scale.
    pub ramp: [[f32; 4]; 3],
}

pub struct ParticleMaterial {
    pub name: String,
    pub distortion: bool,
    pub state: crate::render_state::RenderState,
    /// Material+4a high three bits, consumed by declaration analysis1c4460.
    pub darkvision_layer: u8,
    pub options: BTreeMap<String, i32>,
    pub uniforms: BTreeMap<String, [f32; 4]>,
    pub images: BTreeMap<String, Arc<crate::texture::InlineCompressedChain>>,
    pub volume: Option<Arc<crate::texture::InlineVolumeChain>>,
    pub distortion_volume: Option<Arc<crate::texture::InlineVolumeChain>>,
    pub lighting: Option<ParticleLightingMaterial>,
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
        let distortion = program.eq_ignore_ascii_case("particle/dist_blur");
        if !distortion && !program.eq_ignore_ascii_case("particle/add_default") {
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
        let mut options = OPTIONS
            .iter()
            .map(|name| {
                library
                    .option_value(program.as_bytes(), &overrides, name.as_bytes())
                    .map(|value| (name.to_ascii_lowercase(), value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        if distortion {
            options.insert("fx/opt/distblur_constant_offset".into(), library.option_value(
                program.as_bytes(), &overrides, b"fx/opt/distblur_constant_offset")?);
        }
        let empty = EnvironmentRenderParameters::default();
        let darkvision_layer =
            library.registered_option_value(&overrides, b"rendering/darkVisionLayer")? as u8 & 7;
        let reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let scopes = parameters::ParameterScopes {
            view: &reader,
            blocks: &[&defaults, &material],
        };
        let scroll_uniforms: &[&str] = if options["fx/opt/opacity_scroll_mask"] != 0 {
            &[
                "opacity_Scroll_Mask_Tilling",
                "opacity_Scroll_Mask_PanningSpeed",
                "opacity_Scroll_Mask_RotationSpeed",
            ]
        } else {
            &[]
        };
        let mut extra_uniforms = scroll_uniforms.to_vec();
        if distortion { extra_uniforms.extend(["fogStart", "fogEnd", "fogScale"]); }
        if options["fx/opt/use_macrouv"] != 0 {
            extra_uniforms.push("macroUvAmount");
        }
        if options["fx/opt/fresnel"] != 0 {
            extra_uniforms.extend(["particleFresnelThreshold", "particleFresnelStrength"]);
            if options["fx/opt/fresnel/usenormalmap"] != 0 {
                extra_uniforms.push("Layer0_ScaleBias");
            }
        }
        let uniforms = UNIFORMS
            .iter()
            .chain(extra_uniforms.iter())
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
            ("fx/opacity_scroll_mask", Some("fx/opt/opacity_scroll_mask")),
            ("fx/macroUvMap", Some("fx/opt/use_macrouv")),
            (
                "particleFresnelNormalMap",
                Some("fx/opt/fresnel/usenormalmap"),
            ),
            ("particleFresnelLUT", Some("fx/opt/fresnel/uselut")),
        ] {
            if option.is_some_and(|name| options[name] == 0)
                || (image.starts_with("particleFresnel") && options["fx/opt/fresnel"] == 0)
            {
                continue;
            }
            let source = scopes.image_source(image)?;
            images.insert(
                image.to_ascii_lowercase(),
                Arc::new(crate::material::load_2d_image(catalog, &source)?),
            );
        }
        let lighting = if !distortion && options["fx/opt/use_lighting"] != 0 {
            let mut read_option =
                |name: &str| library.option_value(program.as_bytes(), &overrides, name.as_bytes());
            // These owners still need the shared surface-map evaluator. Never
            // admit them as untextured lighting or silently drop a map/layer.
            for name in [
                "extraLayer",
                "useParallaxMapping",
                "staticDecalCount",
                "hasAnimatedEmissive",
                "hasDynamicDecals",
            ] {
                if read_option(name)? != 0 {
                    return Err(format!(
                        "particle lighting surface input {name} is not connected"
                    ));
                }
            }
            let gloss = scopes.vector("wardRoughness")?;
            let pixel = if options["fx/opt/use_lighting"] == 1 {
                if !crate::material::shadow_is_alpha(
                    state[0],
                    read_option("isDecal")? != 0,
                    read_option("decalType")? as u32,
                    read_option("skyType")? as u32,
                ) {
                    return Err("opaque particle pixel-lighting inputs are not connected".into());
                }
                let self_shadowed_bump = read_option("useSelfShadowedBumpMap")? != 0;
                let fuzz = if read_option("fuzz/enable")? != 0 {
                    Some([
                        scopes.vector("fuzz/fresnel")?[0],
                        scopes.vector("fuzz/intensity")?[0],
                    ])
                } else {
                    None
                };
                let fuzz_mask = fuzz.is_some() && read_option("fuzz/hasMask")? != 0;
                let translucency = if read_option("translucency/enable")? != 0 {
                    let mut ramp = [
                        scopes.vector("translucency/ramp/innerColor")?,
                        scopes.vector("translucency/ramp/mediumColor")?,
                        scopes.vector("translucency/ramp/outerColor")?,
                    ];
                    ramp[0][3] = scopes.vector("translucency/scale")?[0];
                    Some(ParticleTranslucencyMaterial {
                        vertex_alpha: read_option("translucencyUseVertexColor")? != 0,
                        phase: [
                            scopes.vector("translucency/distortion")?[0],
                            scopes.vector("translucency/power")?[0],
                            scopes.vector("translucency/intensity")?[0],
                            scopes.vector("translucency/min")?[0],
                        ],
                        ramp,
                    })
                } else {
                    None
                };
                // BRDF_particles includes standard Ward; translucency_hq only
                // executes in BRDF_skin, independently of this material flag.
                for (image, enabled) in [
                    ("selfShadowedBumpMap", self_shadowed_bump),
                    ("fuzz/mask", fuzz_mask),
                    (
                        "translucencyMap",
                        translucency.as_ref().is_some_and(|t| !t.vertex_alpha),
                    ),
                ] {
                    if enabled {
                        images.insert(
                            image.to_ascii_lowercase(),
                            Arc::new(crate::material::load_2d_image(
                                catalog,
                                &scopes.image_source(image)?,
                            )?),
                        );
                    }
                }
                let maps = [
                    read_option("hasDiffuseLookup")? != 0,
                    read_option("hasSheenMap")? != 0,
                    read_option("hasEmissiveMap")? != 0,
                    read_option("hasEmissiveMaskMap")? != 0,
                ];
                for (image, enabled) in [
                    "diffuseLookup",
                    "sheenMap",
                    "emissiveMap",
                    "emissiveMaskMap",
                ]
                .into_iter()
                .zip(maps)
                {
                    if enabled {
                        images.insert(
                            image.to_ascii_lowercase(),
                            Arc::new(crate::material::load_2d_image(
                                catalog,
                                &scopes.image_source(image)?,
                            )?),
                        );
                    }
                }
                let colour = scopes.vector("emissiveColorConstant")?;
                let clip = scopes.vector("emissiveClipParms")?;
                Some(ParticlePixelMaterial {
                    self_shadowed_bump,
                    fuzz,
                    fuzz_mask,
                    translucency,
                    sheen: [
                        scopes.vector("sheen")?[0],
                        scopes.vector("sheenImportance")?[0],
                    ],
                    maps,
                    emissive: [
                        colour[0],
                        colour[1],
                        colour[2],
                        scopes.vector("emissiveIntensity")?[0],
                    ],
                    emissive_clip: [clip[0], clip[1]],
                    exposure_emissive: read_option("whaleOilEmissivePATCH")? != 0,
                })
            } else {
                None
            };
            let metallic = scopes.vector("metallicMinMax")?;
            let gloss_in_alpha = read_option("glossInDiffuseAlpha")? != 0;
            let bump = read_option("hasBumpMap")? != 0;
            let detail_bump = if bump && read_option("hasDetailBumpMap")? != 0 {
                Some([
                    scopes.vector("detailBumpmapTiling")?[0],
                    scopes.vector("detailBumpmapIntensity")?[0],
                ])
            } else {
                None
            };
            let maps = [
                read_option("hasGlossMap")? != 0 && !gloss_in_alpha,
                read_option("hasMetallicMap")? != 0,
                read_option("hasSpecularMap")? != 0,
            ];
            for (name, enabled) in ["glossMap", "metallicMap", "specularMap"]
                .into_iter()
                .zip(maps)
            {
                if enabled {
                    images.insert(
                        name.to_ascii_lowercase(),
                        Arc::new(crate::material::load_2d_image(
                            catalog,
                            &scopes.image_source(name)?,
                        )?),
                    );
                }
            }
            for (name, enabled) in [("bumpmap", bump), ("detailBumpmap", detail_bump.is_some())] {
                if enabled {
                    images.insert(
                        name.to_ascii_lowercase(),
                        Arc::new(crate::material::load_2d_image(
                            catalog,
                            &scopes.image_source(name)?,
                        )?),
                    );
                }
            }
            let needs_uv = bump || pixel.is_some() || maps.into_iter().any(|enabled| enabled);
            Some(ParticleLightingMaterial {
                pixel,
                maps,
                bump,
                detail_bump,
                uv: if needs_uv {
                    scopes.vector("Layer0_ScaleBias")?
                } else {
                    [0.; 4]
                },
                gloss: [gloss[0], gloss[1]],
                metallic: [metallic[0], metallic[1]],
                gloss_in_alpha,
                dielectric: scopes.vector("specularTintDielectric")?,
                metal_diffuse: scopes.vector("diffuseAlbedoMetallic")?,
                shadow_flags: u32::from(read_option("shadow/useNormalOffset")? != 0)
                    | (u32::from(read_option("shadow/useSlopeScale")? != 0) << 1)
                    | (u32::from(read_option("shadow/useSlopeScaleBias")? != 0) << 2)
                    | (u32::from(read_option("shadow/slopeScaleIsPostProjection")? != 0) << 3),
                shadow_filtering: read_option("shadowFilteringQuality")?,
            })
        } else {
            None
        };
        let volume = if options["fx/opt/use_flipbook"] != 0 {
            Some(Arc::new(crate::material::load_3d_image(
                catalog,
                &scopes.image_source("fx/flipbook")?,
            )?))
        } else {
            None
        };
        let distortion_volume = if distortion && options["fx/opt/use_flipbook"] != 0 {
            Some(Arc::new(crate::material::load_3d_image(catalog, &scopes.image_source("fx/flipbooknormals")?)?))
        } else { None };
        Ok(Self {
            name: name.to_owned(),
            distortion,
            state,
            darkvision_layer,
            options,
            uniforms,
            images,
            volume,
            distortion_volume,
            lighting,
        })
    }
}
