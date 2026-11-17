//! Instanced particle colour submission. The scene owner supplies effective
//! draw state, clipping, selected volume fog and current view resources.
use super::fog_objects::FogObjectParameters;
use crate::rendering::particles::{
    geometry::SpriteGrid, material::ParticleMaterial, ParticleInstance,
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
    },
};
use std::{collections::HashMap, sync::Arc};
mod transparency;

/// The existing installed-material sampler image, shared with ordinary surfaces.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct ParticleMaterialSampler(pub Handle<Image>);

#[derive(Clone, Copy, ShaderType)]
struct TransparencyParameters {
    weights: Vec4,
    parameters: Vec4,
    mode: UVec4,
}

#[derive(Clone, Copy, ShaderType)]
struct ViewParameters {
    clip_from_game_world: Mat4,
    game_world_from_clip: Mat4,
    camera_time: Vec4,
    inverse_z: Vec4,
    exposure_extent: Vec4,
    scene_grid: UVec4,
    sun_direction: Vec4,
    sun_colour: Vec4,
}
#[derive(Clone, Copy, ShaderType)]
struct FogViewParameters {
    world_position: Vec3,
    exposure: f32,
}
#[derive(Clone, Copy, ShaderType)]
struct TextureLayer {
    tiling_pan: Vec4,
    strength: Vec4,
    rotation_seed: Vec4,
}
#[derive(Clone, Copy, ShaderType)]
struct MaterialParameters {
    layers: [TextureLayer; 4],
    colour_adjust: Vec4,
    depth_fade_lookup: Vec4,
    radial_exposure_fog: Vec4,
    distort_options: UVec4,
    colour_options: UVec4,
    fade_options: UVec4,
    animation_options: UVec4,
    surface_options: UVec4,
    macro_fresnel: Vec4,
    fresnel_uv: Vec4,
    lighting_parameters: Vec4,
    lighting_dielectric: Vec4,
    lighting_metal_diffuse: Vec4,
    shadow_options: UVec4,
    surface_map_options: UVec4,
    surface_ranges: Vec4,
    lighting_uv: Vec4,
    normal_options: UVec4,
    detail_normal: Vec4,
    distortion_fog: Vec4,
    alpha_test: f32,
    pixel_options: UVec4,
    pixel_parameters: Vec4,
    emissive: Vec4,
    emissive_clip: Vec4,
    pixel_effects: UVec4,
    fuzz: Vec4,
    translucency_phase: Vec4,
    translucency_ramp: [Vec4; 3],
    /// particle/liquid: (flowmap_strength, liquid_outer_alpha,
    /// liquid_inner_alpha, fresnel_power), (fresnel_intensity,
    /// liquid_Disto_amount, liquid_Disto_reflx, liquid_rflx_intensity),
    /// fresnel_color.
    liquid: [Vec4; 3],
    /// particle/liquid use_flow_map, use_flow_mask, use_reflexion.
    liquid_options: UVec4,
    /// particle/dissolve (distoStrength, alpha_Adjust.xyz).
    dissolve: Vec4,
    /// particle/dissolve tile_alpha, use_mesh.
    dissolve_options: UVec4,
    /// particle/fire (fire_distortion, lookup_adjust, fire_intensity,
    /// fire_light_intensity), (fire_smoke_color.xyz, fire_light_color).
    fire: [Vec4; 2],
    /// particle/fire use_lookup_light.
    fire_options: UVec4,
    /// particle/add_liquid (distoStrength.xy, liquid_Threshold).
    add_liquid: Vec4,
    /// particle/add_liquid fade_threshold, no_opacity_map.
    add_liquid_options: UVec4,
    /// particle/water and blood (liquid_Threshold, liquid_inner_alpha,
    /// liquid_Disto_amount, liquid_default_alpha).
    water: Vec4,
    /// liquid_nm_thresholds.
    water_thresholds: Vec4,
    /// liquid_alpha_fb_animated, use_mesh.
    water_options: UVec4,
}
impl MaterialParameters {
    fn resolve(source: &ParticleMaterial) -> Result<Self, String> {
        let lighting_mode = source.option("fx/opt/use_lighting")?;
        if lighting_mode < 0 {
            return Err(format!(
                "particle lighting mode {lighting_mode} is not connected"
            ));
        }
        let option = |name: &str| source.option(name).map(|v| v as u32);
        let scalar = |name: &str| source.uniform(name).map(|v| v[0]);
        let mut layers = [TextureLayer {
            tiling_pan: Vec4::ZERO,
            strength: Vec4::ZERO,
            rotation_seed: Vec4::ZERO,
        }; 4];
        for (layer, prefix) in layers.iter_mut().zip(["disto1", "disto2", "opacity"]) {
            let tile = source.uniform(&format!("{prefix}_Tilling"))?;
            let pan = source.uniform(&format!("{prefix}_PanningSpeed"))?;
            layer.tiling_pan = Vec4::new(tile[0], tile[1], pan[0], pan[1]);
            layer.rotation_seed = Vec4::new(
                scalar(&format!("{prefix}_RotationSpeed"))?,
                scalar("material_seed")?,
                0.,
                0.,
            );
            if prefix != "opacity" {
                layer.strength = Vec4::from_array(source.uniform(&format!("{prefix}_Strength"))?);
            }
        }
        if source.option("fx/opt/opacity_scroll_mask")? != 0 {
            let tile = source.uniform("opacity_Scroll_Mask_Tilling")?;
            let pan = source.uniform("opacity_Scroll_Mask_PanningSpeed")?;
            layers[3].tiling_pan = Vec4::new(tile[0], tile[1], pan[0], pan[1]);
            layers[3].rotation_seed = Vec4::new(
                scalar("opacity_Scroll_Mask_RotationSpeed")?,
                scalar("material_seed")?,
                0.,
                0.,
            );
        }
        let soft = option("fx/opt/use_softcontact")?;
        if soft > 3 {
            return Err(format!("unknown particle soft contact mode {soft}"));
        }
        let macro_uv = if source.option("fx/opt/use_macroUV")? != 0 {
            source.uniform("macroUvAmount")?
        } else {
            [0.; 4]
        };
        let fresnel = source.option("fx/opt/fresnel")? != 0;
        let pixel = source.lighting.as_ref().and_then(|lighting| lighting.pixel.as_ref());
        let (liquid, liquid_options) = if source.liquid {
            let reflexion = option("fx/opt/use_reflexion")?;
            let (reflx, rflx) = if reflexion != 0 {
                (scalar("liquid_Disto_reflx")?, scalar("liquid_rflx_intensity")?)
            } else {
                (0., 0.)
            };
            ([
                Vec4::new(scalar("flowmap_strength")?, scalar("liquid_outer_alpha")?,
                    scalar("liquid_inner_alpha")?, scalar("fresnel_power")?),
                Vec4::new(scalar("fresnel_intensity")?, scalar("liquid_Disto_amount")?, reflx, rflx),
                Vec4::from_array(source.uniform("fresnel_color")?),
            ], UVec4::new(option("fx/opt/use_flow_map")?, option("fx/opt/use_flow_mask")?, reflexion, 0))
        } else {
            ([Vec4::ZERO; 3], UVec4::ZERO)
        };
        let (dissolve, dissolve_options) = if source.dissolve {
            let adjust = source.uniform("alpha_Adjust")?;
            (Vec4::new(scalar("distoStrength")?, adjust[0], adjust[1], adjust[2]),
                UVec4::new(option("fx/opt/tile_alpha")?, option("fx/opt/use_mesh")?, 0, 0))
        } else {
            (Vec4::ZERO, UVec4::ZERO)
        };
        let (fire, fire_options) = if source.fire {
            let smoke = source.uniform("fire_smoke_color")?;
            ([Vec4::new(scalar("fire_distortion")?, scalar("lookup_adjust")?, scalar("fire_intensity")?,
                    scalar("fire_light_intensity")?),
                Vec4::new(smoke[0], smoke[1], smoke[2], scalar("fire_light_color")?)],
                UVec4::new(option("fx/opt/use_lookup_light")?, 0, 0, 0))
        } else {
            ([Vec4::ZERO; 2], UVec4::ZERO)
        };
        let (add_liquid, add_liquid_options) = if source.add_liquid {
            let strength = source.uniform("distoStrength")?;
            (Vec4::new(strength[0], strength[1], scalar("liquid_Threshold")?, 0.),
                UVec4::new(option("fx/opt/fade_threshold")?, option("fx/opt/no_opacity_map")?, 0, 0))
        } else {
            (Vec4::ZERO, UVec4::ZERO)
        };
        let (water, water_thresholds, water_options) = if source.water {
            (Vec4::new(scalar("liquid_Threshold")?, scalar("liquid_inner_alpha")?,
                scalar("liquid_Disto_amount")?, scalar("liquid_default_alpha")?),
                Vec4::from_array(source.uniform("liquid_nm_thresholds")?),
                UVec4::new(option("fx/opt/liquid_alpha_fb_animated")?, option("fx/opt/use_mesh")?, 0, 0))
        } else {
            (Vec4::ZERO, Vec4::ZERO, UVec4::ZERO)
        };
        Ok(Self {
            water,
            water_thresholds,
            water_options,
            liquid,
            liquid_options,
            dissolve,
            dissolve_options,
            fire,
            fire_options,
            add_liquid,
            add_liquid_options,
            distortion_fog: if source.distortion || source.add_liquid {Vec4::new(scalar("fogStart")?,scalar("fogEnd")?,scalar("fogScale")?,0.)}
                else if source.scrolling_water {Vec4::new(0.,0.,0.,scalar("liquid_Disto_amount")?)} else {Vec4::ZERO},
            pixel_effects: pixel.map_or(UVec4::ZERO, |p| UVec4::new(
                u32::from(p.fuzz.is_some()), u32::from(p.fuzz_mask), u32::from(p.translucency.is_some()),
                u32::from(p.translucency.as_ref().is_some_and(|t| t.vertex_alpha)))),
            fuzz: pixel.and_then(|p| p.fuzz).map_or(Vec4::ZERO, |f| Vec4::new(f[0], f[1], 0., 0.)),
            translucency_phase: pixel.and_then(|p| p.translucency.as_ref()).map_or(Vec4::ZERO, |t| Vec4::from_array(t.phase)),
            translucency_ramp: pixel.and_then(|p| p.translucency.as_ref()).map_or([Vec4::ZERO; 3], |t| t.ramp.map(Vec4::from_array)),
            pixel_options: pixel.map_or(UVec4::ZERO, |p| UVec4::from_array(p.maps.map(u32::from))),
            pixel_parameters: pixel.map_or(Vec4::ZERO, |p| Vec4::new(p.sheen[0], p.sheen[1], u32::from(p.self_shadowed_bump) as f32, 0.)),
            emissive: pixel.map_or(Vec4::ZERO, |p| Vec4::from_array(p.emissive)),
            emissive_clip: pixel.map_or(Vec4::ZERO, |p| Vec4::new(p.emissive_clip[0], p.emissive_clip[1], u32::from(p.exposure_emissive) as f32, 0.)),
            alpha_test: scalar("particleAlphaTest")?,
            normal_options: UVec4::new(
                u32::from(source.lighting.as_ref().is_some_and(|light| light.bump)),
                u32::from(source.lighting.as_ref().is_some_and(|light| light.detail_bump.is_some())),
                u32::from(source.option("twoSidedLighting")? != 0), 0),
            detail_normal: source.lighting.as_ref().and_then(|light| light.detail_bump)
                .map_or(Vec4::ZERO, |detail| Vec4::new(detail[0], detail[1], 0., 0.)),
            surface_map_options: source.lighting.as_ref().map_or(UVec4::ZERO, |light| {
                UVec4::new(
                    u32::from(light.maps[0]),
                    u32::from(light.maps[1]),
                    u32::from(light.maps[2]),
                    0,
                )
            }),
            surface_ranges: source
                .lighting
                .as_ref()
                .map_or(Vec4::ZERO, |light| Vec4::new(light.metallic[0], 0., 0., 0.)),
            lighting_uv: source
                .lighting
                .as_ref()
                .map_or(Vec4::ZERO, |light| Vec4::from_array(light.uv)),
            shadow_options: source.lighting.as_ref().map_or(UVec4::ZERO, |light| {
                // isAlpha pixel particles receive the parallel light directly
                // with neutral AO; an opaque particle surface reads the screen
                // shadow mask built from its zPrePass (route 0, 2d5210 /
                // lightContext), as the capture's ro_texscreenshadows shows.
                let clouds = light.pixel.as_ref().is_none_or(|p| p.translucency.is_none());
                UVec4::new(light.shadow_flags | (u32::from(clouds) << 4), light.shadow_filtering as u32,
                    u32::from(!source.opaque), 0)
            }),
            lighting_parameters: source.lighting.as_ref().map_or(Vec4::ZERO, |light| {
                Vec4::new(
                    light.gloss[0],
                    light.gloss[1],
                    light.metallic[1],
                    u32::from(light.gloss_in_alpha) as f32,
                )
            }),
            lighting_dielectric: source
                .lighting
                .as_ref()
                .map_or(Vec4::ZERO, |light| Vec4::from_array(light.dielectric)),
            lighting_metal_diffuse: source
                .lighting
                .as_ref()
                .map_or(Vec4::ZERO, |light| Vec4::from_array(light.metal_diffuse)),
            surface_options: UVec4::new(
                option("fx/opt/use_macroUV")?,
                option("fx/opt/fresnel")?,
                option("fx/opt/fresnel/useNormalMap")?,
                option("fx/opt/fresnel/useLUT")?,
            ),
            macro_fresnel: Vec4::new(
                macro_uv[0],
                macro_uv[1],
                if fresnel {
                    scalar("particleFresnelThreshold")?
                } else {
                    0.
                },
                if fresnel {
                    scalar("particleFresnelStrength")?
                } else {
                    0.
                },
            ),
            fresnel_uv: if fresnel && source.option("fx/opt/fresnel/useNormalMap")? != 0 {
                Vec4::from_array(source.uniform("Layer0_ScaleBias")?)
            } else {
                Vec4::ZERO
            },
            layers,
            colour_adjust: Vec4::from_array(source.uniform("color_Adjust")?),
            depth_fade_lookup: Vec4::new(
                scalar("depth_Threshold")?,
                scalar("particleDistanceFadeIn")?,
                scalar("particleDistanceFadeOut")?,
                scalar("lookup_adjust")?,
            ),
            radial_exposure_fog: Vec4::new(
                scalar("radial_RampPow")?,
                scalar("fx/auto_exposure_amount")?,
                scalar("particleFogScatterPower")?,
                if source.distortion { option("fx/opt/distblur_constant_offset")? as f32 } else { 0. },
            ),
            distort_options: UVec4::new(
                option("fx/opt/uv_disto1")?,
                option("fx/opt/uv_disto2")?,
                option("fx/opt/uv_disto1_uniform")?,
                option("fx/opt/uv_disto2_uniform")?,
            ),
            colour_options: UVec4::new(
                option("fx/opt/uv_Disto2xDisto1")?,
                option("fx/opt/use_diffuse")?,
                option("fx/opt/use_lookup")?,
                option("fx/opt/radial_ramp")?,
            ),
            fade_options: UVec4::new(
                option("fx/opt/use_disto_has_mask")?,
                option("fx/opt/view_dependent_fade")?,
                soft,
                option("fx/opt/no_alpha_lookup_modulate")?,
            ),
            animation_options: UVec4::new(
                option("fx/opt/use_flipbook")?,
                option("fx/opt/flipbook_is_mask")?,
                option("fx/opt/opacity_scroll_mask")?,
                lighting_mode as u32,
            ),
        })
    }
}

/// Which program of a particle material a GPU material specializes.
/// The ordinary FX pass draws isAlpha materials; a non-isAlpha (blend ONE
/// ZERO) material is a model surface drawn in the world zPrePass and the
/// opaque colour pass (1401fbad0 publishes particle renderers as dynamic
/// model surfaces), as the original's capture shows: GREATER_EQUAL depth
/// writes in the prepass, then an EQUAL depth test without writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParticlePhase {
    Colour,
    Masking,
    OpaqueDepth,
    OpaqueColour,
}

/// 1402fcff0's per-model zPrePass partition for a particle surface. The FX
/// render model has no model depth hack (the capture's prepass draw keeps
/// stencil disabled), so only the source state's depth fields apply.
pub fn opaque_depth_state(state: crate::render_state::RenderState) -> crate::render_state::RenderState {
    crate::render_state::main_depth_surface_state(state, 0.0).0
}

/// The opaque colour pass after the zPrePass: EQUAL depth, no depth writes
/// (capture events 67930 and 69112 for model and particle surfaces alike).
pub fn opaque_colour_state(mut state: crate::render_state::RenderState) -> crate::render_state::RenderState {
    state[5] = (state[5] & !0x1f) | 0x8 | 2;
    state
}

pub struct ParticleDraw {
    pub material: Arc<ParticleMaterial>,
    pub masking_factor: f32,
    /// Shared per-object SH/IBL record selected at the particle root's origin.
    pub lighting: Option<[[f32; 4]; 9]>,
    /// The selected colour-pass state, after the owning cache's overrides.
    pub state: crate::render_state::RenderState,
    pub stencil_reference: u32,
    pub grid_level: u32,
    pub mesh: Option<(String, Arc<crate::assets::particle_model::ParticleModel>)>,
    pub instances: Vec<ParticleInstance>,
    /// Non-instanced particle geometry (vertex format 4, a triangle strip),
    /// drawn with one instance record for the draw's clip/fade/record data.
    pub strip: Vec<crate::gameplay::particles::trail::TrailVertex>,
    pub clip_plane: Vec4,
    pub clip_fade_distance: f32,
    pub fog: FogObjectParameters,
    pub transparency: crate::rendering::particles::transparency::TransparencyParameters,
}
pub struct ParticleFogInputs<'a> {
    pub parameters: &'a Buffer,
    pub world: &'a TextureView,
    pub canvas: &'a TextureView,
    pub curve: &'a TextureView,
    pub depth: &'a TextureView,
}
pub struct ParticleLightingInputs<'a> {
    pub probes: &'a Buffer,
    pub words: &'a Buffer,
    pub cubes: &'a TextureView,
    pub sampler: &'a Sampler,
    pub planes: &'a Buffer,
}
pub struct ParticleDirectLightingInputs<'a> {
    pub pixel: Option<ParticlePixelLightingInputs<'a>>,
    pub lights: &'a Buffer,
    pub pool: &'a Buffer,
    pub heads: &'a Buffer,
    pub shadows: &'a Buffer,
    pub parallel: &'a Buffer,
    pub parallel_depth: &'a TextureView,
    pub local: Option<(&'a Buffer, &'a TextureView)>,
    pub grid: crate::tiled_lighting::TileGrid,
    pub sun: Option<([f32; 3], [f32; 3])>,
}
pub struct ParticlePixelLightingInputs<'a> {
    pub screen: &'a TextureView,
    pub cloud: &'a TextureView,
    pub cloud_sampler: &'a Sampler,
    pub area: Option<&'a TextureView>,
}
/// The zPrePass needs only the view, its time and the depth attachment.
pub struct ParticleDepthInputs<'a> {
    pub surface_sampler: Option<&'a Sampler>,
    pub matrices: &'a crate::rendering::view_history::ViewMatrices,
    pub local_time: f32,
    pub extent: [u32; 2],
    pub inverse_z: [f32; 3],
    pub adaptation: &'a Buffer,
    pub depth: &'a TextureView,
}
pub struct ParticleViewInputs<'a> {
    pub distortion_sources: &'a super::screen_effects::ScreenEffectSources,
    pub surface_sampler: Option<&'a Sampler>,
    pub direct: Option<ParticleDirectLightingInputs<'a>>,
    pub lighting: Option<ParticleLightingInputs<'a>>,
    pub matrices: &'a crate::rendering::view_history::ViewMatrices,
    pub local_time: f32,
    pub camera_exposure: f32,
    pub extent: [u32; 2],
    pub inverse_z: [f32; 3],
    pub adaptation: &'a Buffer,
    pub colour: &'a TextureView,
    pub depth: &'a TextureView,
    pub fog: ParticleFogInputs<'a>,
    pub compressed_hdr: bool,
    pub white_level: f32,
    pub resolve_state: crate::render_state::RenderState,
}
pub(super) struct GeometryGpu {
    vertices: Buffer,
    indices: Buffer,
    count: u32,
}
struct MaterialGpu {
    source: Arc<ParticleMaterial>,
    parameters: UniformBuffer<MaterialParameters>,
    binding: BindGroup,
    pipeline: RenderPipeline,
}

/// Host device cache of immutable shader/layout/pipeline objects. The original
/// global 1402c9f00/1402ca440 cache supplies serialized shader data; retaining
/// compiled backend objects here is an explicit host reuse improvement.
#[derive(Clone)]
pub(super) struct ParticlePrograms {
    formats: (TextureFormat, TextureFormat),
    view_layout: BindGroupLayout,
    material_layout: BindGroupLayout,
    fog_layout: BindGroupLayout,
    lighting_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    shader: ShaderModule,
    /// The zPrePass programs use only the view and material groups.
    depth_layout: PipelineLayout,
    pipelines: HashMap<(crate::render_state::RenderState, bool, bool, bool, bool, i32, u8, ParticlePhase, bool), RenderPipeline>,
}
impl ParticlePrograms {
    fn new(device: &RenderDevice, formats: (TextureFormat, TextureFormat)) -> Self {
        let view_layout = device.create_bind_group_layout(
            Some("particle view"),
            &[
                buffer_entry(0, true),
                buffer_entry(1, false),
                buffer_entry(2, false),
                buffer_entry(3, false),
                buffer_entry(4, true),
                buffer_entry(5, false),
                buffer_entry(6, false),
            ],
        );
        let material_layout = device.create_bind_group_layout(
            Some("particle material"),
            &std::iter::once(buffer_entry(0, true))
                .chain(
                    (1..=5)
                        .chain(10..=13)
                        .chain(15..=17)
                        .chain(19..=20)
                        .chain(21..=27)
                        .map(|b| texture_entry(b, TextureViewDimension::D2, false)),
                )
                .chain((6..=8).chain([14, 18]).map(sampler_entry))
                .chain(std::iter::once(texture_entry(28, TextureViewDimension::D3, false)))
                .chain(std::iter::once(texture_entry(
                    9,
                    TextureViewDimension::D3,
                    false,
                )))
                .collect::<Vec<_>>(),
        );
        let fog_layout = device.create_bind_group_layout(
            Some("particle shared fog"),
            &[
                buffer_entry(161, false),
                texture_entry(162, TextureViewDimension::D2, false),
                texture_entry(163, TextureViewDimension::D2, false),
                texture_entry(164, TextureViewDimension::D2Array, false),
                texture_entry(165, TextureViewDimension::D2, true),
                sampler_entry(166),
                buffer_entry(167, false),
            ],
        );
        let lighting_layout = device.create_bind_group_layout(
            Some("particle scene lighting"),
            &[
                buffer_entry(100, false),
                buffer_entry(119, false),
                buffer_entry(158, false),
                texture_entry(156, TextureViewDimension::CubeArray, false),
                sampler_entry(157),
                buffer_entry(120, false),
                texture_entry(121, TextureViewDimension::D2Array, true),
                texture_entry(123, TextureViewDimension::D2, false),
                texture_entry(124, TextureViewDimension::D2, false),
                sampler_entry(125),
                texture_entry(133, TextureViewDimension::D2Array, false),
                BindGroupLayoutEntry {
                    binding: 122,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Comparison),
                    count: None,
                },
                buffer_entry(126, false),
                texture_entry(127, TextureViewDimension::D2Array, true),
                BindGroupLayoutEntry {
                    binding: 128,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Comparison),
                    count: None,
                },
                buffer_entry(129, false),
                buffer_entry(130, false),
                buffer_entry(131, false),
                buffer_entry(132, false),
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("particle colour"),
            bind_group_layouts: &[
                Some(&view_layout),
                Some(&material_layout),
                Some(&fog_layout),
                Some(&lighting_layout),
            ],
            immediate_size: 0,
        });
        let depth_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("particle depth"),
            bind_group_layouts: &[Some(&view_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let source = particle_shader_source();
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("particle colour"),
            source: ShaderSource::Wgsl(source.into()),
        });
        Self {formats, view_layout, material_layout, fog_layout, lighting_layout,
            pipeline_layout, depth_layout, shader, pipelines: HashMap::new()}
    }
}

/// The FX colour pass shader: shared lighting and fog modules with the
/// particle programs.
pub fn particle_shader_source() -> String {
    let fog = include_str!("../../../assets/shaders/dh2_fog_material.wgsl")
        .lines()
        .filter(|line| !line.trim_start().starts_with("#import"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("@group(#{MATERIAL_BIND_GROUP})", "@group(2)");
    // Directives are stripped; only the particle-surface instancing branch
    // (never defined for the FX pass) is removed with its body.
    let shared_source = |source: &str| {
        let mut instanced = None;
        source
            .lines()
            .filter(|line| {
                let directive = line.trim_start();
                match (directive, instanced) {
                    ("#ifdef DH2_PARTICLE_INSTANCES", _) => instanced = Some(true),
                    ("#else", Some(true)) => instanced = Some(false),
                    ("#endif", Some(_)) => instanced = None,
                    _ => return instanced != Some(true) && !directive.starts_with('#'),
                }
                false
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let lighting_source = [
        super::probe_weights::SHADER_SOURCE.to_owned(),
        shared_source(include_str!("../../../assets/shaders/dh2_shadow.wgsl")),
        shared_source(include_str!("../../../assets/shaders/dh2_tlf_list.wgsl")),
        shared_source(include_str!(
            "../../../assets/shaders/dh2_light_scattering.wgsl"
        )),
        shared_source(include_str!("../../../assets/shaders/dh2_indirect.wgsl")),
        shared_source(include_str!("../../../assets/shaders/dh2_reflection.wgsl")),
        shared_source(include_str!("../../../assets/shaders/dh2_fresnel.wgsl")),
        shared_source(include_str!("../../../assets/shaders/dh2_ward.wgsl")),
        include_str!("../../../assets/shaders/dh2_area_light.wgsl").to_owned(),
        shared_source(include_str!(
            "../../../assets/shaders/dh2_parallel_receiver.wgsl"
        )),
        shared_source(include_str!(
            "../../../assets/shaders/dh2_local_receiver.wgsl"
        )),
        include_str!("../../../assets/shaders/dh2_particle_lighting.wgsl").to_owned(),
    ]
    .join("\n")
    .replace("@group(#{MATERIAL_BIND_GROUP})", "@group(3)");
    let source = format!(
        "{lighting_source}\n{}\n{}\n{}\n{fog}",
        include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
        include_str!("../../../assets/shaders/dh2_fog_alpha.wgsl"),
        include_str!("../../../assets/shaders/dh2_particles.wgsl")
    );
    source
}

/// Mutable level buffers and material bindings, separate from device programs.
pub struct ParticlePass {
    view: UniformBuffer<ViewParameters>,
    fog_view: UniformBuffer<FogViewParameters>,
    instances: Buffer,
    objects: Buffer,
    transparency_parameters: Buffer,
    root_lighting: Buffer,
    transparency: Option<transparency::TransparencyPass>,
    distortion: Option<([u32; 2], bool, super::screen_effects::passes::DistortionPasses, super::water_hdr::WaterHdr)>,
    reported_mesh_submission: bool,
    max_instances: usize,
    max_draws: usize,
    /// The six shared sprite grids (1dc5b0), one owner for every pass instance.
    grids: Arc<Vec<GeometryGpu>>,
    meshes: HashMap<String, (GeometryGpu, Option<(BufferId, BindGroup)>)>,
    programs: Arc<ParticlePrograms>,
    wrap: Sampler,
    clamp: Sampler,
    mirror: Sampler,
    bilinear: Sampler,
    _unused_volume: Texture,
    unused_volume_view: TextureView,
    /// 2D stand-in for texture slots a material's program never samples
    /// (a flipbook material has no opacity image).
    _unused_image: Texture,
    unused_image_view: TextureView,
    _unused_shadow: Texture,
    unused_shadow_view: TextureView,
    comparison: Sampler,
    _unused_cube: Texture,
    unused_cube_view: TextureView,
    /// Keyed by material, state, phase and strip geometry.
    materials: HashMap<(String, crate::render_state::RenderState, ParticlePhase, bool), MaterialGpu>,
    /// r_particleAlphaTestDepth and r_particleAlphaTestLit: the opaque
    /// passes' particleAlphaTest (140321860 / 14031dce0 set the parameter
    /// from these cvars per pass; capture: 0.035 prepass, 0.01569 colour).
    opaque_alpha_tests: [f32; 2],
    // Host: pipeline specialization is independent of image/material binding
    // residency (1402c9f00/1402ca440 keep prepared shader programs separately).
    pipeline_requests: usize,
    pipeline_creations: usize,
    binding_refreshes: usize,
    material_sampler: Option<SamplerId>,
    view_bindings: Option<(BufferId, Vec<BindGroup>)>,
    fog_binding: Option<((BufferId, [TextureViewId; 4]), BindGroup)>,
    lighting_binding: Option<(([BufferId; 9], [TextureViewId; 6], [SamplerId; 2]), BindGroup)>,
    colour_format: TextureFormat,
    depth_format: TextureFormat,
    instance_bytes: Vec<u8>,
    /// bevy_backend: the frame's strip vertices in one buffer, created at
    /// the first strip draw, grown to the next power of two and released
    /// with the pass. The original appends to its per-renderer dynamic
    /// idTriangles (1402107b0) within the frame's vertex budget.
    strip_vertices: Option<Buffer>,
    strip_bytes: Vec<u8>,
}
fn buffer_entry(binding: u32, uniform: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Buffer {
            ty: if uniform {
                BufferBindingType::Uniform
            } else {
                BufferBindingType::Storage { read_only: true }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn texture_entry(
    binding: u32,
    dimension: TextureViewDimension,
    depth: bool,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Texture {
            sample_type: if depth {
                TextureSampleType::Depth
            } else {
                TextureSampleType::Float { filterable: true }
            },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Sampler(SamplerBindingType::Filtering),
        count: None,
    }
}
impl ParticlePass {
    /// Both opaque programs of every non-isAlpha material, before play.
    pub fn prepare_opaque_materials<'a>(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        materials: impl Iterator<Item = &'a Arc<ParticleMaterial>>,
        surface_sampler: &Sampler,
    ) -> Result<usize, String> {
        if self.material_sampler != Some(surface_sampler.id()) {
            self.rebind_materials(images, device, queue, surface_sampler)?;
            self.material_sampler = Some(surface_sampler.id());
        }
        let mut created = 0;
        for source in materials.filter(|material| material.opaque) {
            for (phase, state) in [(ParticlePhase::OpaqueDepth, opaque_depth_state(source.state)),
                (ParticlePhase::OpaqueColour, opaque_colour_state(source.state))] {
                let key = (source.name.clone(), state, phase, false);
                if self.materials.contains_key(&key) { continue; }
                let material = self.material(images, device, queue, source.clone(), state, false, surface_sampler, phase, false)?;
                self.materials.insert(key, material);
                created += 1;
            }
        }
        Ok(created)
    }

    /// The cvar values the opaque passes bind as particleAlphaTest. A change
    /// rebuilds the opaque materials, whose parameters carry the value.
    pub fn set_opaque_alpha_tests(&mut self, values: [f32; 2]) {
        if values != self.opaque_alpha_tests {
            self.opaque_alpha_tests = values;
            self.materials.retain(|key, _| !matches!(key.2, ParticlePhase::OpaqueDepth | ParticlePhase::OpaqueColour));
        }
    }

    /// The GPU binding is a cache, not an effect owner. Call at effect/level
    /// retirement after definitions and live instances have released their
    /// references; active sources keep their existing bindings and pipelines.
    pub(super) fn retire_unused_materials(&mut self) {
        let trace = super::world_effects::trace_lifecycle();
        let before = self.materials.len();
        let mut cached = HashMap::<usize, usize>::new();
        for material in self.materials.values() {
            *cached.entry(Arc::as_ptr(&material.source) as usize).or_default() += 1;
        }
        self.materials.retain(|_, material| {
            Arc::strong_count(&material.source) > cached[&(Arc::as_ptr(&material.source) as usize)]
        });
        if trace { eprintln!("FX_LIFECYCLE gpu_materials_retired={} retained={}", before - self.materials.len(), self.materials.len()); }
    }

    /// Raise the instance/draw capacity in place. Only the capacity-sized
    /// storage and the bind groups that reference it are rebuilt; the shader,
    /// pipelines and level materials are kept.
    pub fn grow(&mut self, device: &RenderDevice, max_instances: usize, max_draws: usize) {
        if max_instances <= self.max_instances && max_draws <= self.max_draws {
            return;
        }
        let max_instances = max_instances.max(self.max_instances);
        let max_draws = max_draws.max(self.max_draws);
        let storage = |label, size| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        self.instances = storage("particle instances", (max_instances * 96) as u64);
        self.objects = storage("particle volume fog", (max_draws * FogObjectParameters::SIZE) as u64);
        self.transparency_parameters = storage("particle transparency parameters", (max_draws * 48) as u64);
        self.root_lighting = storage("particle root probe records", (max_draws * 9 * 16) as u64);
        self.max_instances = max_instances;
        self.max_draws = max_draws;
        self.instance_bytes.reserve((max_instances * 96).saturating_sub(self.instance_bytes.len()));
        self.view_bindings = None;
        self.fog_binding = None;
        self.lighting_binding = None;
        for (_, binding) in self.meshes.values_mut() {
            *binding = None;
        }
    }
    fn geometry_binding(&self,device:&RenderDevice,vertices:&Buffer,adaptation:&Buffer)->BindGroup {
        device.create_bind_group(Some("particle geometry view"),&self.programs.view_layout,&[
            BindGroupEntry{binding:0,resource:self.view.binding().unwrap()},
            BindGroupEntry{binding:1,resource:self.instances.as_entire_binding()},
            BindGroupEntry{binding:2,resource:vertices.as_entire_binding()},
            BindGroupEntry{binding:3,resource:adaptation.as_entire_binding()},
            BindGroupEntry{binding:4,resource:self.fog_view.binding().unwrap()},
            BindGroupEntry{binding:5,resource:self.transparency_parameters.as_entire_binding()},
            BindGroupEntry{binding:6,resource:self.root_lighting.as_entire_binding()},
        ])
    }
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        max_instances: usize,
        max_draws: usize,
        colour_format: TextureFormat,
        depth_format: TextureFormat,
    ) -> Result<Self, String> {
        Self::new_with_programs(device, queue, max_instances, max_draws, colour_format, depth_format, None)
    }
    pub(super) fn reusable_programs(&self) -> Arc<ParticlePrograms> { self.programs.clone() }
    /// The sprite grids, shared with another pass instance of the same device.
    pub(super) fn shared_grids(&self) -> Arc<Vec<GeometryGpu>> { self.grids.clone() }
    pub(super) fn new_with_programs(
        device: &RenderDevice, queue: &RenderQueue, max_instances: usize, max_draws: usize,
        colour_format: TextureFormat, depth_format: TextureFormat, cached: Option<Arc<ParticlePrograms>>,
    ) -> Result<Self, String> {
        Self::new_with_shared(device, queue, max_instances, max_draws, colour_format, depth_format, cached, None)
    }
    pub(super) fn new_with_shared(
        device: &RenderDevice, queue: &RenderQueue, max_instances: usize, max_draws: usize,
        colour_format: TextureFormat, depth_format: TextureFormat, cached: Option<Arc<ParticlePrograms>>,
        grids: Option<Arc<Vec<GeometryGpu>>>,
    ) -> Result<Self, String> {
        if max_instances == 0 || max_draws == 0 {
            return Err("particle GPU capacity must be nonzero".into());
        }
        let reused = cached.as_ref().is_some_and(|programs| programs.formats == (colour_format, depth_format));
        let programs = cached
            .filter(|programs| programs.formats == (colour_format, depth_format))
            .unwrap_or_else(|| Arc::new(ParticlePrograms::new(device, (colour_format, depth_format))));
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY particle program acquisition: reused={reused} cached_pipelines={} formats={colour_format:?}/{depth_format:?}", programs.pipelines.len());
        }
        let storage = |label, size| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let instances = storage("particle instances", (max_instances * 96) as u64);
        let objects = storage(
            "particle volume fog",
            (max_draws * FogObjectParameters::SIZE) as u64,
        );
        let transparency_parameters =
            storage("particle transparency parameters", (max_draws * 48) as u64);
        let grids = grids.unwrap_or_else(|| Arc::new(sprite_grids(device)));
        let sampler = |mode, mipmap| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: mipmap,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        let mut view = UniformBuffer::from(ViewParameters {
            clip_from_game_world: Mat4::IDENTITY,
            game_world_from_clip: Mat4::IDENTITY,
            camera_time: Vec4::ZERO,
            inverse_z: Vec4::ZERO,
            exposure_extent: Vec4::ONE,
            scene_grid: UVec4::ZERO,
            sun_direction: Vec4::ZERO,
            sun_colour: Vec4::ZERO,
        });
        view.write_buffer(device, queue);
        let mut fog_view = UniformBuffer::from(FogViewParameters {
            world_position: Vec3::ZERO,
            exposure: 1.,
        });
        fog_view.write_buffer(device, queue);
        // Required only by the host's fixed binding layout when no authored
        // volume is used. The disabled shader branch cannot sample this image.
        let unused_volume = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("unused particle volume binding"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D3,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &[0; 4],
        );
        let unused_volume_view = unused_volume.create_view(&default());
        let unused_image = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("unused particle image binding"),
                size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &[0; 4],
        );
        let unused_image_view = unused_image.create_view(&default());
        // Dormant fixed-layout binding for unlit scenes with no probe network.
        let unused_cube = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("unused particle reflection binding"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &[0; 24],
        );
        let unused_cube_view = unused_cube.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::CubeArray),
            ..default()
        });
        let unused_shadow = device.create_texture(&TextureDescriptor {
            label: Some("unused particle shadow binding"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let unused_shadow_view = unused_shadow.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        let root_lighting = storage("particle root probe records", (max_draws * 9 * 16) as u64);
        Ok(Self {
            view,
            fog_view,
            instances,
            objects,
            transparency_parameters,
            root_lighting,
            transparency: None,
            distortion: None,
            reported_mesh_submission: false,
            max_instances,
            max_draws,
            grids,
            meshes: HashMap::new(),
            programs,
            wrap: sampler(AddressMode::Repeat, MipmapFilterMode::Linear),
            clamp: sampler(AddressMode::ClampToEdge, MipmapFilterMode::Nearest),
            mirror: sampler(AddressMode::MirrorRepeat, MipmapFilterMode::Nearest),
            bilinear: sampler(AddressMode::Repeat, MipmapFilterMode::Nearest),
            _unused_volume: unused_volume,
            unused_volume_view,
            _unused_image: unused_image,
            unused_image_view,
            _unused_shadow: unused_shadow,
            unused_shadow_view,
            comparison: super::shadow_pipeline::shadow_comparison_sampler(device),
            _unused_cube: unused_cube,
            unused_cube_view,
            materials: HashMap::new(),
            opaque_alpha_tests: [0.035, 0.01569],
            pipeline_requests: 0,
            pipeline_creations: 0,
            binding_refreshes: 0,
            material_sampler: None,
            view_bindings: None,
            fog_binding: None,
            lighting_binding: None,
            colour_format,
            depth_format,
            instance_bytes: Vec::with_capacity(max_instances * 96),
            strip_vertices: None,
            strip_bytes: Vec::new(),
        })
    }

    /// Image/sampler residency can change without new shader programs or
    /// uniforms (host descriptor side of 14005feb0 / 1402c9f00 ownership).
    fn material_binding(
        &self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: &ParticleMaterial,
        parameters: &UniformBuffer<MaterialParameters>,
        anisotropic_sampler: &Sampler,
    ) -> Result<BindGroup, String> {
        let mut views = HashMap::new();
        for (name, image) in &source.images {
            let view = images.acquire(
                device,
                queue,
                image,
                &TextureDescriptor {
                    label: Some(&source.name),
                    size: Extent3d {
                        width: image.info.width,
                        height: image.info.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: image.info.mip_count,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: super::image_formats::gpu_image_format(image.info.format),
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
            )?;
            views.insert(name.as_str(), view);
        }
        // The material omits fx/opacity only when its program never samples
        // it (flipbook volume or radial ramp).
        let opacity = views.get("fx/opacity").unwrap_or(&self.unused_image_view);
        let volume_view = if let Some(volume) = &source.volume {
            let view = images.acquire(
                device,
                queue,
                volume,
                &TextureDescriptor {
                    label: Some(&source.name),
                    size: Extent3d {
                        width: volume.info.width,
                        height: volume.info.height,
                        depth_or_array_layers: volume.depth,
                    },
                    mip_level_count: volume.info.mip_count,
                    sample_count: 1,
                    dimension: TextureDimension::D3,
                    format: super::image_formats::gpu_image_format(volume.info.format),
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
            )?;
            Some(view)
        } else {
            None
        };
        let distortion_volume_view = if let Some(volume) = &source.distortion_volume {
            let view = images.acquire(
                device,
                queue,
                volume,
                &TextureDescriptor {
                    label: Some(&source.name),
                    size: Extent3d {
                        width: volume.info.width,
                        height: volume.info.height,
                        depth_or_array_layers: volume.depth,
                    },
                    mip_level_count: volume.info.mip_count,
                    sample_count: 1,
                    dimension: TextureDimension::D3,
                    format: super::image_formats::gpu_image_format(volume.info.format),
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
            )?;
            Some(view)
        } else {
            None
        };
        let surface_sampler = if source.option("fx/opt/use_mirror_sampler")? != 0 {
            &self.mirror
        } else if source.option("fx/opt/use_clamp_sampler")? != 0 {
            &self.clamp
        } else if source.option("fx/opt/use_bilinear_sampler")? != 0 {
            &self.bilinear
        } else {
            &self.wrap
        };
        let binding = device.create_bind_group(
            Some("particle material"),
            &self.programs.material_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: parameters.binding().ok_or("particle uniforms absent")?,
                },
                // particle/fire reuses the slots: fire_ball (1), fire_light
                // (2), fire_smoke (3), lookup_light (4), lookup_fire (5).
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(
                        if source.fire { views.get("fx/fire_ball").unwrap_or(opacity) } else { opacity },
                    ),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(
                        views.get(if source.fire { "fx/fire_light" } else if source.liquid { "fx/flowmap" } else { "fx/disto1" })
                            .unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(
                        views.get(if source.fire { "fx/fire_smoke" } else { "fx/disto2" }).unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(
                        views.get(if source.fire { "fx/lookup_light" } else { "fx/diffuse" }).unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(
                        views.get(if source.dissolve || source.fire { "fx/lookup_fire" } else { "fx/lookup" }).unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.wrap),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(surface_sampler),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::Sampler(&self.clamp),
                },
                BindGroupEntry {
                    binding: 15,
                    resource: BindingResource::TextureView(
                        views.get("glossmap").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 16,
                    resource: BindingResource::TextureView(
                        views.get("metallicmap").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 17,
                    resource: BindingResource::TextureView(
                        views.get("specularmap").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 18,
                    resource: BindingResource::Sampler(anisotropic_sampler),
                },
                BindGroupEntry {
                    binding: 19,
                    resource: BindingResource::TextureView(
                        views.get(if source.scrolling_water || source.liquid || source.water || source.fire { "fx/normals" } else { "bumpmap" }).unwrap_or(opacity)),
                },
                BindGroupEntry {
                    binding: 20,
                    resource: BindingResource::TextureView(views.get("detailbumpmap").unwrap_or(opacity)),
                },
                BindGroupEntry { binding: 21, resource: BindingResource::TextureView(views.get("diffuselookup").unwrap_or(opacity)) },
                BindGroupEntry { binding: 22, resource: BindingResource::TextureView(views.get("sheenmap").unwrap_or(opacity)) },
                BindGroupEntry { binding: 23, resource: BindingResource::TextureView(views.get("emissivemap").unwrap_or(opacity)) },
                BindGroupEntry { binding: 24, resource: BindingResource::TextureView(views.get("emissivemaskmap").unwrap_or(opacity)) },
                BindGroupEntry { binding: 25, resource: BindingResource::TextureView(views.get("fuzz/mask").unwrap_or(opacity)) },
                BindGroupEntry { binding: 26, resource: BindingResource::TextureView(views.get("translucencymap").unwrap_or(opacity)) },
                BindGroupEntry { binding: 27, resource: BindingResource::TextureView(views.get("selfshadowedbumpmap").unwrap_or(opacity)) },
                BindGroupEntry {
                    binding: 14,
                    resource: BindingResource::Sampler(&self.bilinear),
                },
                BindGroupEntry {
                    binding: 11,
                    resource: BindingResource::TextureView(
                        views.get(if source.liquid { "fx/reflexion" } else { "fx/macrouvmap" }).unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 12,
                    resource: BindingResource::TextureView(
                        views.get("particlefresnelnormalmap").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 13,
                    resource: BindingResource::TextureView(
                        views.get("particlefresnellut").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 10,
                    resource: BindingResource::TextureView(
                        views.get(if source.liquid { "fx/liquid_mask" } else { "fx/opacity_scroll_mask" }).unwrap_or(opacity),
                    ),
                },
                BindGroupEntry { binding: 28, resource: BindingResource::TextureView(
                    distortion_volume_view.as_ref().unwrap_or(&self.unused_volume_view)) },
                BindGroupEntry {
                    binding: 9,
                    resource: BindingResource::TextureView(
                        volume_view.as_ref().unwrap_or(&self.unused_volume_view),
                    ),
                },
            ],
        );
        Ok(binding)
    }

    fn rebind_materials(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        sampler: &Sampler,
    ) -> Result<(), String> {
        // Move the existing allocation out briefly, so immutable layout/sampler
        // fields and mutable bindings can be borrowed independently.
        let mut materials = std::mem::take(&mut self.materials);
        let result = (|| {
            for material in materials.values_mut() {
                material.binding = self.material_binding(images, device, queue,
                    &material.source, &material.parameters, sampler)?;
                self.binding_refreshes += 1;
            }
            Ok(())
        })();
        self.materials = materials;
        result
    }

    pub fn refresh_scene_bindings(&mut self, images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice, queue: &RenderQueue, sampler: &Sampler) -> Result<(), String> {
        let mut materials = std::mem::take(&mut self.materials);
        let result = (|| {
            for material in materials.values_mut() {
                if material.source.images.values().any(|image| images.changed_paths.contains(&image.path)) {
                    material.binding = self.material_binding(images, device, queue,
                        &material.source, &material.parameters, sampler)?;
                    self.binding_refreshes += 1;
                }
            }
            Ok(())
        })();
        self.materials = materials;
        result
    }

    fn material(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: Arc<ParticleMaterial>,
        state: crate::render_state::RenderState,
        accumulation: bool,
        anisotropic_sampler: &Sampler,
        phase: ParticlePhase,
        strip: bool,
    ) -> Result<MaterialGpu, String> {
        let masking = phase == ParticlePhase::Masking;
        let opaque = matches!(phase, ParticlePhase::OpaqueDepth | ParticlePhase::OpaqueColour);
        if opaque != source.opaque {
            return Err(format!("particle {} program does not match the material's isAlpha class: {}",
                if opaque { "opaque" } else { "alpha" }, source.name));
        }
        if opaque && !(source.water || source.liquid) {
            return Err(format!("opaque particle zPrePass is connected for the water and liquid programs only: {}", source.name));
        }
        if masking && (!source.masking_uses_main || source.distortion) {
            return Err(format!("particle masking program requires separate preparation: {}", source.name));
        }
        let accumulation = accumulation && !source.distortion;
        let mut resolved = MaterialParameters::resolve(&source)?;
        match phase {
            ParticlePhase::OpaqueDepth => resolved.alpha_test = self.opaque_alpha_tests[0],
            ParticlePhase::OpaqueColour => resolved.alpha_test = self.opaque_alpha_tests[1],
            _ => {}
        }
        let mut parameters = UniformBuffer::from(resolved);
        parameters.write_buffer(device, queue);
        let binding = self.material_binding(images, device, queue, &source, &parameters, anisotropic_sampler)?;
        let lighting_mode = if source.distortion || source.liquid || source.dissolve || source.add_liquid { 0 } else { source.option("fx/opt/use_lighting")? };
        let water_options = if source.scrolling_water {
            u8::from(source.option("fx/opt/water_alpha_mask_spec")? != 0)
                | (u8::from(source.option("fx/opt/use_mesh")? != 0) << 1)
        } else { 0 };
        let pipeline_key = (state, accumulation, source.distortion, source.scrolling_water, source.liquid, lighting_mode, water_options | (u8::from(source.dissolve) << 2) | (u8::from(source.add_liquid) << 3) | (u8::from(source.water) << 4) | (u8::from(source.fire) << 5), phase, strip);
        self.pipeline_requests += 1;
        let pipeline = if let Some(pipeline) = self.programs.pipelines.get(&pipeline_key) {
            pipeline.clone()
        } else {
            let blends = crate::render_state::blend_state(&state);
            let formats = [
                if masking {
                    TextureFormat::R8Unorm
                } else if accumulation || source.distortion {
                    TextureFormat::Rgba16Float
                } else {
                    self.colour_format
                },
                TextureFormat::Rg16Float,
            ];
            let targets = (0..if phase == ParticlePhase::OpaqueDepth { 0 } else if accumulation { 2 } else { 1 })
                .map(|index| {
                    let blend = blends.targets[index].ok_or("particle colour target absent")?;
                    Ok(Some(ColorTargetState {
                        format: formats[index],
                        blend: super::pipeline_state::blend_target(blend)?,
                        write_mask: ColorWrites::from_bits_truncate(blend.write_mask as u32),
                    }))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let depth = crate::render_state::depth_stencil_state(&state);
            if depth.depth_write != (phase == ParticlePhase::OpaqueDepth) {
                return Err("particle colour state must disable depth writes; the zPrePass must write".into());
            }
            if depth.stencil_enabled && !self.depth_format.has_stencil_aspect() {
                return Err(
                    "particle alpha colour state requires the gameplay stencil attachment".into(),
                );
            }
            let (front_face, cull_mode) = super::shadow_pipeline::shadow_mesh_raster(&state, false);
            let constants = [("particle_lighting_mode", f64::from(lighting_mode)),
                ("particle_distortion_mode", f64::from(u32::from(source.distortion))),
                ("particle_scrolling_water", f64::from(u32::from(source.scrolling_water))),
                ("particle_liquid", f64::from(u32::from(source.liquid))),
                ("particle_dissolve", f64::from(u32::from(source.dissolve))),
                ("particle_add_liquid", f64::from(u32::from(source.add_liquid))),
                ("particle_fire", f64::from(u32::from(source.fire))),
                ("particle_water", f64::from(u32::from(source.water))),
                ("particle_water_alpha_mask_spec", f64::from(u32::from(water_options & 1 != 0))),
                ("particle_water_mesh", f64::from(u32::from(water_options & 2 != 0))),
                ("particle_masking", f64::from(u32::from(masking))),
                ("particle_opaque", f64::from(u32::from(opaque)))];
            // Vertex format 4 (1402d0dc0 list +0x178).
            let strip_attributes = [
                VertexAttribute { format: VertexFormat::Float32x3, offset: 0, shader_location: 0 },
                VertexAttribute { format: VertexFormat::Float16x4, offset: 0xc, shader_location: 1 },
                VertexAttribute { format: VertexFormat::Snorm8x4, offset: 0x14, shader_location: 2 },
                VertexAttribute { format: VertexFormat::Snorm8x4, offset: 0x18, shader_location: 3 },
                VertexAttribute { format: VertexFormat::Unorm8x4, offset: 0x1c, shader_location: 4 },
                VertexAttribute { format: VertexFormat::Float16x4, offset: 0x20, shader_location: 5 },
            ];
            let strip_layout = [RawVertexBufferLayout {
                array_stride: 40,
                step_mode: VertexStepMode::Vertex,
                attributes: &strip_attributes,
            }];
            let started = super::screen_effects::passes::trace_pipeline_start();
            let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(match (strip, phase == ParticlePhase::OpaqueDepth) {
                    (true, true) => "particle strip depth",
                    (true, false) => "particle strip colour",
                    (false, true) => "particle sprite depth",
                    (false, false) => "particle sprite colour",
                }),
                layout: Some(if phase == ParticlePhase::OpaqueDepth { &self.programs.depth_layout } else { &self.programs.pipeline_layout }),
                vertex: RawVertexState {
                    module: &self.programs.shader,
                    entry_point: Some(match (strip, phase == ParticlePhase::OpaqueDepth) {
                        (true, true) => "particle_strip_depth_vertex",
                        (true, false) => "particle_strip_vertex",
                        (false, true) => "particle_depth_vertex",
                        (false, false) => "particle_vertex",
                    }),
                    compilation_options: PipelineCompilationOptions { constants: &constants, ..default() },
                    buffers: if strip { &strip_layout } else { &[] },
                },
                fragment: Some(RawFragmentState {
                    module: &self.programs.shader,
                    entry_point: Some(if phase == ParticlePhase::OpaqueDepth {
                        "particle_depth_fragment"
                    } else if source.distortion {
                        "particle_distortion_fragment"
                    } else if accumulation {
                        "particle_accumulation_fragment"
                    } else {
                        "particle_fragment"
                    }),
                    compilation_options: PipelineCompilationOptions { constants: &constants, ..default() },
                    targets: &targets,
                }),
                // bevy_backend: the original draws trail strips through the
                // shared index buffer 1402d1ae0(registry, 2); a triangle
                // strip gives the same triangles without the index buffer.
                primitive: PrimitiveState {
                    topology: if strip { PrimitiveTopology::TriangleStrip } else { PrimitiveTopology::TriangleList },
                    front_face,
                    cull_mode,
                    ..default()
                },
                depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(
                    &state,
                    self.depth_format,
                )?),
                multisample: default(),
                multiview_mask: None,
                cache: None,
            });
            super::screen_effects::passes::trace_pipeline_end(started,"particle_create_render_pipeline",&source.name);
            self.pipeline_creations += 1;
            Arc::make_mut(&mut self.programs).pipelines.insert(pipeline_key, pipeline.clone());
            pipeline
        };
        Ok(MaterialGpu {
            source,
            parameters,
            binding,
            pipeline,
        })
    }

    /// The cached materials were built with another surface sampler.
    pub fn sampler_changed(&self, sampler: &Sampler) -> bool {
        self.material_sampler != Some(sampler.id())
    }

    /// Create the GPU material of every (material, draw state) the level's
    /// particles can use, as the draw path derives the state: the transparency
    /// mode from the cvars and both water stencil variants. Returns how many
    /// were created.
    pub fn prepare_materials<'a>(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        materials: impl Iterator<Item = &'a Arc<ParticleMaterial>>,
        cvars: &crate::platform::cvars::CVars,
        surface_sampler: &Sampler,
    ) -> Result<usize, String> {
        self.prepare_materials_bounded(images, device, queue, materials, &Default::default(), cvars, surface_sampler, None)
            .map(|(created, _)| created)
    }

    /// Host loading scheduling: acquire at most the requested number of new
    /// variants before returning to the UI. Cached variants consume no quota.
    /// The loading gate stays closed until both water states of every authored
    /// material exist. Ordinary gameplay retains the unbounded path.
    pub fn prepare_materials_bounded<'a>(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        materials: impl Iterator<Item = &'a Arc<ParticleMaterial>>,
        strips: &std::collections::HashSet<String>,
        cvars: &crate::platform::cvars::CVars,
        surface_sampler: &Sampler,
        max_created: Option<usize>,
    ) -> Result<(usize, bool), String> {
        if self.material_sampler != Some(surface_sampler.id()) {
            self.rebind_materials(images, device, queue, surface_sampler)?;
            self.material_sampler = Some(surface_sampler.id());
        }
        let mut created = 0;
        let transparencies = crate::rendering::particles::transparency::ParticleTransparency::new(cvars);
        for material in materials {
            if material.opaque { continue; }
            let mut transparency = transparencies.get(material.state)?;
            if material.distortion { transparency.mode = 0; }
            // Trail materials also get their strip pipelines before play.
            let strip_variants: &[bool] = if strips.contains(&material.name) { &[false, true] } else { &[false] };
            for (water, &strip) in [false, true].into_iter().flat_map(|water| strip_variants.iter().map(move |strip| (water, strip))) {
                let (state, _) = crate::render_state::alpha_surface_draw_state(material.state, transparency.mode != 0, water);
                let key = (material.name.clone(), state, ParticlePhase::Colour, strip);
                if self.materials.contains_key(&key) { continue; }
                if max_created.is_some_and(|limit| created >= limit) {
                    return Ok((created, false));
                }
                let gpu = self.material(images, device, queue, material.clone(), state, transparency.mode != 0, surface_sampler, ParticlePhase::Colour, strip)?;
                self.materials.insert(key, gpu);
                created += 1;
            }
        }
        Ok((created, true))
    }

    /// Prewarm only material identities requested by the mask owner. Colour
    /// and mask variants share source/image leases but have distinct states.
    pub fn prepare_mask_materials<'a>(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        materials: impl Iterator<Item = &'a Arc<ParticleMaterial>>,
        surface_sampler: &Sampler,
        max_created: Option<usize>,
    ) -> Result<(usize, bool), String> {
        if self.material_sampler != Some(surface_sampler.id()) {
            self.rebind_materials(images, device, queue, surface_sampler)?;
            self.material_sampler = Some(surface_sampler.id());
        }
        let mut created = 0;
        for source in materials {
            let state = crate::render_state::masking_surface_draw_state(source.state);
            let key = (source.name.clone(), state, ParticlePhase::Masking, false);
            if self.materials.contains_key(&key) { continue; }
            if max_created.is_some_and(|limit| created >= limit) { return Ok((created, false)); }
            let material = self.material(images, device, queue, source.clone(), state, false, surface_sampler, ParticlePhase::Masking, false)?;
            self.materials.insert(key, material);
            created += 1;
        }
        Ok((created, true))
    }

    pub fn record(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleViewInputs<'_>,
        draws: &[ParticleDraw],
    ) -> Result<(), String> {
        self.record_mode(images, device, queue, encoder, input, draws, ParticlePhase::Colour)
    }

    /// The opaque colour pass of non-isAlpha particle surfaces, after the
    /// world's opaque pass and before atmosphere: EQUAL against the depth
    /// their zPrePass wrote. `input.colour`/`input.depth` are the view's
    /// lit target and its depth. Runs on its own pass instance so its
    /// uploads never share buffers with the alpha pass of the same frame.
    pub fn record_opaque(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleViewInputs<'_>,
        draws: &[ParticleDraw],
    ) -> Result<(), String> {
        self.record_mode(images, device, queue, encoder, input, draws, ParticlePhase::OpaqueColour)
    }

    /// One record per instance (1eaa40's layout of the rewrite's particle
    /// buffer): centre/axes, packed parameters, clip plane and clip fade.
    fn upload_instances(&mut self, queue: &RenderQueue, draws: &[ParticleDraw]) {
        self.instance_bytes.clear();
        for (tag, draw) in draws.iter().enumerate() {
            for instance in &draw.instances {
                for v in [
                    Vec3::from_array(instance.position).extend(instance.axes[0][0]),
                    Vec3::from_array(instance.axes[1]).extend(instance.axes[0][1]),
                    Vec3::from_array(instance.axes[2]).extend(instance.axes[0][2]),
                ] {
                    self.instance_bytes
                        .extend(v.to_array().into_iter().flat_map(f32::to_le_bytes));
                }
                self.instance_bytes.extend(
                    [
                        instance.parameters.colour,
                        instance.parameters.flipbook_seed_uv,
                        instance.parameters.size,
                        tag as u32,
                    ]
                    .into_iter()
                    .flat_map(u32::to_le_bytes),
                );
                self.instance_bytes.extend(
                    draw.clip_plane
                        .to_array()
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
                self.instance_bytes.extend(
                    [draw.clip_fade_distance, if draw.mesh.is_some() {1.} else {0.}, draw.masking_factor, 0.]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
        }
        if !self.instance_bytes.is_empty() {
            queue.write_buffer(&self.instances, 0, &self.instance_bytes);
        }
    }

    /// The frame's strip vertices (vertex format 4) in one buffer.
    fn upload_strips(&mut self, device: &RenderDevice, queue: &RenderQueue, draws: &[ParticleDraw]) {
        self.strip_bytes.clear();
        for draw in draws {
            for vertex in &draw.strip {
                self.strip_bytes.extend(vertex.position.iter().flat_map(|v| v.to_le_bytes()));
                self.strip_bytes.extend(vertex.texcoord.iter().flat_map(|v| v.to_le_bytes()));
                self.strip_bytes.extend(vertex.normal.to_le_bytes());
                self.strip_bytes.extend(vertex.tangent.to_le_bytes());
                self.strip_bytes.extend(vertex.colour.to_le_bytes());
                self.strip_bytes.extend(vertex.texcoord5.iter().flat_map(|v| v.to_le_bytes()));
            }
        }
        if !self.strip_bytes.is_empty() {
            let needed = self.strip_bytes.len() as u64;
            if self.strip_vertices.as_ref().is_none_or(|buffer| buffer.size() < needed) {
                self.strip_vertices = Some(device.create_buffer(&BufferDescriptor {
                    label: Some("particle strip vertices"),
                    size: needed.next_power_of_two(),
                    usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            queue.write_buffer(self.strip_vertices.as_ref().unwrap(), 0, &self.strip_bytes);
        }
    }

    /// The world zPrePass of non-isAlpha particle surfaces (1402fcff0's model
    /// partition): depth only, before the screen shadows and the opaque
    /// fog-depth snapshot consume the prepass depth. `depth` is the view's
    /// depth attachment.
    pub fn record_opaque_depth(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleDepthInputs<'_>,
        draws: &[ParticleDraw],
    ) -> Result<(), String> {
        if draws.is_empty() {
            return Ok(());
        }
        if draws.iter().any(|draw| !draw.material.opaque) {
            return Err("particle zPrePass draws must be opaque surfaces".into());
        }
        let count = draws.iter().map(|d| d.instances.len()).sum::<usize>();
        if count > self.max_instances || draws.len() > self.max_draws {
            return Err("particle draw exceeds admitted declaration capacity".into());
        }
        let sampler = input.surface_sampler.unwrap_or(&self.wrap).clone();
        if self.material_sampler != Some(sampler.id()) {
            self.rebind_materials(images, device, queue, &sampler)?;
            self.material_sampler = Some(sampler.id());
        }
        for draw in draws {
            let strip = !draw.strip.is_empty();
            let key = (draw.material.name.clone(), opaque_depth_state(draw.material.state), ParticlePhase::OpaqueDepth, strip);
            if !self.materials.contains_key(&key) {
                let material = self.material(images, device, queue, draw.material.clone(), key.1, false,
                    &sampler, ParticlePhase::OpaqueDepth, strip)?;
                self.materials.insert(key, material);
            }
        }
        // The colour pass of the same frame rewrites these values; the
        // zPrePass programs read only the view projection, camera and time.
        self.view.set(ViewParameters {
            game_world_from_clip: Mat4::IDENTITY,
            scene_grid: UVec4::ZERO,
            sun_direction: Vec4::ZERO,
            sun_colour: Vec4::ZERO,
            clip_from_game_world: Mat4::from_cols_array_2d(&input.matrices.view_projection).transpose(),
            camera_time: Vec3::from_array(input.matrices.position).extend(input.local_time),
            inverse_z: Vec3::from_array(input.inverse_z).extend(0.),
            exposure_extent: Vec4::new(0., 0., input.extent[0] as f32, input.extent[1] as f32),
        });
        self.view.write_buffer(device, queue);
        self.upload_instances(queue, draws);
        if count == 0 {
            return Ok(());
        }
        self.upload_strips(device, queue, draws);
        if self.view_bindings.as_ref().is_none_or(|v| v.0 != input.adaptation.id()) {
            let bindings = self.grids.iter()
                .map(|grid| self.geometry_binding(device, &grid.vertices, input.adaptation)).collect();
            self.view_bindings = Some((input.adaptation.id(), bindings));
        }
        for draw in draws {
            let Some((source, model)) = &draw.mesh else { continue };
            if !self.meshes.contains_key(source) {
                self.meshes.insert(source.clone(), (mesh_geometry(device, model)?, None));
            }
            if self.meshes[source].1.as_ref().is_none_or(|entry| entry.0 != input.adaptation.id()) {
                let binding = self.geometry_binding(device, &self.meshes[source].0.vertices, input.adaptation);
                self.meshes.get_mut(source).unwrap().1 = Some((input.adaptation.id(), binding));
            }
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("particle zPrePass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: input.depth,
                depth_ops: Some(Operations { load: LoadOp::Load, store: StoreOp::Store }),
                stencil_ops: self.depth_format.has_stencil_aspect()
                    .then_some(Operations { load: LoadOp::Load, store: StoreOp::Store }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut first = 0u32;
        let mut strip_first = 0u32;
        for draw in draws {
            let level = draw.grid_level.min(5) as usize;
            let strip = draw.strip.len() as u32;
            let material = &self.materials[&(draw.material.name.clone(), opaque_depth_state(draw.material.state),
                ParticlePhase::OpaqueDepth, strip != 0)];
            pass.set_pipeline(&material.pipeline);
            if strip != 0 {
                pass.set_bind_group(0, &self.view_bindings.as_ref().unwrap().1[0], &[]);
                pass.set_bind_group(1, &material.binding, &[]);
                pass.set_vertex_buffer(0, *self.strip_vertices.as_ref().unwrap().slice(u64::from(strip_first) * 40..));
                pass.draw(0..strip, first..first + 1);
                first += draw.instances.len() as u32;
                strip_first += strip;
                continue;
            }
            let (geometry, binding) = if let Some((source, _)) = &draw.mesh {
                let cached = &self.meshes[source]; (&cached.0, &cached.1.as_ref().unwrap().1)
            } else { (&self.grids[level], &self.view_bindings.as_ref().unwrap().1[level]) };
            pass.set_bind_group(0, binding, &[]);
            pass.set_bind_group(1, &material.binding, &[]);
            pass.set_index_buffer(*geometry.indices.slice(..), IndexFormat::Uint16);
            let instances = draw.instances.len() as u32;
            pass.draw_indexed(0..geometry.count, 0, first..first + instances);
            first += instances;
        }
        Ok(())
    }

    /// 1403222f0's mask submission shares the ordinary particle buffers and
    /// image bindings. The caller supplies the cleared R8 target and scene
    /// depth; source material state is preserved except for depth writes.
    pub fn record_mask(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleViewInputs<'_>,
        draws: &[ParticleDraw],
    ) -> Result<(), String> {
        if !draws.iter().any(|draw| draw.masking_factor > 0.0) { return Ok(()); }
        if draws.iter().any(|draw| draw.masking_factor > 0.0 && draw.material.distortion) {
            return Err("distortion particle masking specialization is not connected".into());
        }
        self.record_mode(images, device, queue, encoder, input, draws, ParticlePhase::Masking)
    }

    fn record_mode(
        &mut self,
        images: &mut super::particle_images::ParticleImagesGpu,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleViewInputs<'_>,
        draws: &[ParticleDraw],
        phase: ParticlePhase,
    ) -> Result<(), String> {
        if draws.is_empty() {
            return Ok(());
        }
        let masking = phase == ParticlePhase::Masking;
        let opaque = phase == ParticlePhase::OpaqueColour;
        if draws.iter().any(|draw| draw.material.opaque != opaque) {
            return Err("particle draws of both isAlpha classes share one pass".into());
        }
        let draw_state = |draw: &ParticleDraw| match phase {
            ParticlePhase::Masking => crate::render_state::masking_surface_draw_state(draw.material.state),
            ParticlePhase::OpaqueColour => opaque_colour_state(draw.material.state),
            _ => draw.state,
        };
        let accumulation = phase == ParticlePhase::Colour && draws.iter().find(|draw| !draw.material.distortion)
            .is_some_and(|draw| draw.transparency.mode != 0);
        if phase == ParticlePhase::Colour && draws
            .iter()
            .any(|draw| !draw.material.distortion && (draw.transparency.mode != 0) != accumulation)
        {
            return Err("particle transparency mode changed within a frame".into());
        }
        if accumulation
            && self
                .transparency
                .as_ref()
                .is_none_or(|p| !p.matches(input.extent, input.compressed_hdr, input.resolve_state))
        {
            self.transparency = Some(transparency::TransparencyPass::new(
                device,
                queue,
                input.extent,
                input.compressed_hdr,
                self.colour_format,
                self.depth_format,
                input.resolve_state,
            )?);
        }
        let count = draws.iter().map(|d| d.instances.len()).sum::<usize>();
        if count > self.max_instances || draws.len() > self.max_draws {
            return Err("particle draw exceeds admitted declaration capacity".into());
        }
        if input.surface_sampler.is_none()
            && draws.iter().any(|draw| {
                draw.material
                    .lighting
                    .as_ref()
                    .is_some_and(|light| light.bump || light.maps[1] || light.maps[2]
                        || light.pixel.as_ref().is_some_and(|p| p.self_shadowed_bump || p.maps[2] || p.maps[3]
                            || p.translucency.as_ref().is_some_and(|t| !t.vertex_alpha)))
            })
        {
            return Err("particle surface maps await the canonical anisotropic sampler".into());
        }
        let sampler_id = input.surface_sampler.unwrap_or(&self.wrap).id();
        if self.material_sampler != Some(sampler_id) {
            let sampler = input.surface_sampler.unwrap_or(&self.wrap).clone();
            self.rebind_materials(images, device, queue, &sampler)?;
            self.material_sampler = Some(sampler_id);
        }
        for draw in draws {
            if masking && !(draw.masking_factor > 0.0) { continue; }
            let state = draw_state(draw);
            let key = (draw.material.name.clone(), state, phase, !draw.strip.is_empty());
            if !self.materials.contains_key(&key) {
                let surface_sampler = input.surface_sampler.unwrap_or(&self.wrap).clone();
                let material = self.material(
                        images,
                        device,
                        queue,
                        draw.material.clone(),
                        state,
                        accumulation,
                        &surface_sampler,
                        phase,
                        !draw.strip.is_empty(),
                    )?;
                self.materials.insert(key, material);
            }
        }
        let position = input.matrices.position;
        let sun = input
            .direct
            .as_ref()
            .and_then(|v| v.sun)
            .unwrap_or(([0.; 3], [0.; 3]));
        let mut inverse_game = Mat4::IDENTITY.to_cols_array_2d();
        if draws.iter().any(|draw| draw.material.distortion || draw.material.lighting.as_ref().is_some_and(|light| light.pixel.is_some()))
            && !crate::render_shadow::invert_rows(&input.matrices.view_projection, &mut inverse_game) {
            return Err("particle pixel lighting requires an invertible view projection".into());
        }
        self.view.set(ViewParameters {
            game_world_from_clip: Mat4::from_cols_array_2d(&inverse_game).transpose(),
            scene_grid: input.direct.as_ref().map_or(UVec4::ZERO, |v| {
                UVec4::new(
                    v.grid.lighting_shift,
                    v.grid.dimensions[0],
                    v.grid.dimensions[1],
                    0,
                )
            }),
            sun_direction: Vec3::from_array(sun.0).extend(0.),
            sun_colour: Vec3::from_array(sun.1).extend(0.),
            clip_from_game_world: Mat4::from_cols_array_2d(&input.matrices.view_projection)
                .transpose(),
            camera_time: Vec3::from_array(position).extend(input.local_time),
            inverse_z: Vec3::from_array(input.inverse_z).extend(0.),
            exposure_extent: Vec4::new(
                139.26 * input.camera_exposure,
                input.camera_exposure,
                input.extent[0] as f32,
                input.extent[1] as f32,
            ),
        });
        self.view.write_buffer(device, queue);
        self.fog_view.set(FogViewParameters {
            world_position: Vec3::new(position[0], position[2], -position[1]),
            exposure: input.camera_exposure,
        });
        self.fog_view.write_buffer(device, queue);
        self.upload_instances(queue, draws);
        if count == 0 {
            return Ok(());
        }
        self.upload_strips(device, queue, draws);
        let transparency = ShaderBuffer::from(
            draws
                .iter()
                .map(|draw| TransparencyParameters {
                    weights: Vec4::from_array(draw.transparency.weights),
                    parameters: Vec4::from_array(draw.transparency.parameters),
                    mode: UVec4::new(draw.transparency.mode as u32, 0, 0, 0),
                })
                .collect::<Vec<_>>(),
        );
        queue.write_buffer(
            &self.transparency_parameters,
            0,
            transparency
                .data
                .as_ref()
                .ok_or("particle transparency records absent")?,
        );
        let root_lighting = ShaderBuffer::from(
            draws
                .iter()
                .map(|draw| draw.lighting.unwrap_or([[0.; 4]; 9]).map(Vec4::from_array))
                .collect::<Vec<_>>(),
        );
        queue.write_buffer(
            &self.root_lighting,
            0,
            root_lighting
                .data
                .as_ref()
                .ok_or("particle root probe records absent")?,
        );
        if draws.iter().any(|d| d.lighting.is_some()) && input.lighting.is_none() {
            return Err("lit particle scene GPU resources unavailable".into());
        }
        let lighting = input.lighting.as_ref();
        let probes = lighting.map_or(&self.instances, |v| v.probes);
        let words = lighting.map_or(&self.instances, |v| v.words);
        let planes = lighting.map_or(&self.instances, |v| v.planes);
        let cubes = lighting.map_or(&self.unused_cube_view, |v| v.cubes);
        let reflection_sampler = lighting.map_or(&self.wrap, |v| v.sampler);
        let direct = input.direct.as_ref();
        if draws.iter().any(|d| {
            d.material
                .option("fx/opt/use_lighting")
                .is_ok_and(|v| v == 1 || v >= 3)
        }) && direct.is_none()
        {
            return Err("particle local-light inputs unavailable".into());
        }
        let directional = direct.map_or(&self.instances, |d| d.parallel);
        let directional_depth = direct.map_or(&self.unused_shadow_view, |d| d.parallel_depth);
        let local = direct
            .and_then(|d| d.local)
            .map_or(&self.instances, |d| d.0);
        let local_depth = direct
            .and_then(|d| d.local)
            .map_or(&self.unused_shadow_view, |d| d.1);
        let lights = direct.map_or(&self.instances, |d| d.lights);
        let pool = direct.map_or(&self.instances, |d| d.pool);
        let heads = direct.map_or(&self.instances, |d| d.heads);
        let shadows = direct.map_or(&self.instances, |d| d.shadows);
        let pixel = direct.and_then(|d| d.pixel.as_ref());
        if draws.iter().any(|draw| draw.material.lighting.as_ref().is_some_and(|light| light.pixel.is_some()))
            && pixel.is_none() {
            return Err("particle pixel-lighting scene inputs unavailable".into());
        }
        let screen = pixel.map_or(input.fog.world, |p| p.screen);
        let cloud = pixel.map_or(input.fog.world, |p| p.cloud);
        let cloud_sampler = pixel.map_or(&self.clamp, |p| p.cloud_sampler);
        // A frame with no textured area lights uses the shader's slice255
        // branch. The fixed layout's array binding is then never sampled.
        let area = pixel.and_then(|p| p.area).unwrap_or(input.fog.curve);
        let lighting_key = (
            [
                probes.id(),
                words.id(),
                planes.id(),
                directional.id(),
                local.id(),
                lights.id(),
                pool.id(),
                heads.id(),
                shadows.id(),
            ],
            [cubes.id(), directional_depth.id(), local_depth.id(), screen.id(), cloud.id(), area.id()],
            [reflection_sampler.id(), cloud_sampler.id()],
        );
        if self
            .lighting_binding
            .as_ref()
            .is_none_or(|v| v.0 != lighting_key)
        {
            self.lighting_binding = Some((
                lighting_key,
                device.create_bind_group(
                    Some("particle shared scene lighting"),
                    &self.programs.lighting_layout,
                    &[
                        BindGroupEntry {
                            binding: 100,
                            resource: probes.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 119,
                            resource: words.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 158,
                            resource: planes.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 156,
                            resource: BindingResource::TextureView(cubes),
                        },
                        BindGroupEntry {
                            binding: 120,
                            resource: directional.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 121,
                            resource: BindingResource::TextureView(directional_depth),
                        },
                        BindGroupEntry {
                            binding: 122,
                            resource: BindingResource::Sampler(&self.comparison),
                        },
                        BindGroupEntry { binding: 123, resource: BindingResource::TextureView(screen) },
                        BindGroupEntry { binding: 124, resource: BindingResource::TextureView(cloud) },
                        BindGroupEntry { binding: 125, resource: BindingResource::Sampler(cloud_sampler) },
                        BindGroupEntry { binding: 133, resource: BindingResource::TextureView(area) },
                        BindGroupEntry {
                            binding: 126,
                            resource: local.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 127,
                            resource: BindingResource::TextureView(local_depth),
                        },
                        BindGroupEntry {
                            binding: 128,
                            resource: BindingResource::Sampler(&self.comparison),
                        },
                        BindGroupEntry {
                            binding: 129,
                            resource: lights.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 130,
                            resource: pool.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 131,
                            resource: heads.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 132,
                            resource: shadows.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 157,
                            resource: BindingResource::Sampler(reflection_sampler),
                        },
                    ],
                ),
            ));
        }
        let objects = ShaderBuffer::from(draws.iter().map(|d| d.fog).collect::<Vec<_>>());
        queue.write_buffer(
            &self.objects,
            0,
            objects.data.as_ref().ok_or("particle fog records absent")?,
        );
        if self
            .view_bindings
            .as_ref()
            .is_none_or(|v| v.0 != input.adaptation.id())
        {
            let bindings = self
                .grids
                .iter()
                .map(|grid| {
                    self.geometry_binding(device, &grid.vertices, input.adaptation)
                })
                .collect();
            self.view_bindings = Some((input.adaptation.id(), bindings));
        }
        for draw in draws {
            let Some((source,model))=&draw.mesh else {continue};
            if !self.meshes.contains_key(source) {
                self.meshes.insert(source.clone(),(mesh_geometry(device,model)?,None));
            }
            if self.meshes[source].1.as_ref().is_none_or(|entry|entry.0!=input.adaptation.id()) {
                let binding=self.geometry_binding(device,&self.meshes[source].0.vertices,input.adaptation);
                self.meshes.get_mut(source).unwrap().1=Some((input.adaptation.id(),binding));
            }
        }
        let fog_key = (
            input.fog.parameters.id(),
            [
                input.fog.world.id(),
                input.fog.canvas.id(),
                input.fog.curve.id(),
                input.fog.depth.id(),
            ],
        );
        if self.fog_binding.as_ref().is_none_or(|v| v.0 != fog_key) {
            self.fog_binding = Some((
                fog_key,
                device.create_bind_group(
                    Some("particle shared fog"),
                    &self.programs.fog_layout,
                    &[
                        BindGroupEntry {
                            binding: 161,
                            resource: input.fog.parameters.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 162,
                            resource: BindingResource::TextureView(input.fog.world),
                        },
                        BindGroupEntry {
                            binding: 163,
                            resource: BindingResource::TextureView(input.fog.canvas),
                        },
                        BindGroupEntry {
                            binding: 164,
                            resource: BindingResource::TextureView(input.fog.curve),
                        },
                        BindGroupEntry {
                            binding: 165,
                            resource: BindingResource::TextureView(input.fog.depth),
                        },
                        BindGroupEntry {
                            binding: 166,
                            resource: BindingResource::Sampler(&self.wrap),
                        },
                        BindGroupEntry {
                            binding: 167,
                            resource: self.objects.as_entire_binding(),
                        },
                    ],
                ),
            ));
        }
        let colour_attachments = if accumulation {
            let targets = self.transparency.as_ref().unwrap();
            vec![
                Some(RenderPassColorAttachment {
                    view: &targets.accumulation_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                }),
                Some(RenderPassColorAttachment {
                    view: &targets.opacity_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(bevy::color::LinearRgba::new(1., 0., 0., 1.).into()),
                        store: StoreOp::Store,
                    },
                }),
            ]
        } else {
            vec![Some(RenderPassColorAttachment {
                view: input.colour,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })]
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(if masking { "particle masking" } else if opaque { "particle opaque colour" } else { "particle colour" }),
            color_attachments: &colour_attachments,
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: input.depth,
                depth_ops: None,
                stencil_ops: self
                    .depth_format
                    .has_stencil_aspect()
                    .then_some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(2, &self.fog_binding.as_ref().unwrap().1, &[]);
        pass.set_bind_group(3, &self.lighting_binding.as_ref().unwrap().1, &[]);
        let mut first = 0u32;
        let mut strip_first = 0u32;
        for draw in draws {
            let strip = draw.strip.len() as u32;
            if draw.material.distortion || (masking && !(draw.masking_factor > 0.0)) {
                first += draw.instances.len() as u32; strip_first += strip; continue;
            }
            let level = draw.grid_level.min(5) as usize;
            let material = &self.materials[&(draw.material.name.clone(), draw_state(draw), phase, strip != 0)];
            pass.set_pipeline(&material.pipeline);
            pass.set_stencil_reference(if masking || opaque { 0 } else { draw.stencil_reference });
            if strip != 0 {
                pass.set_bind_group(0, &self.view_bindings.as_ref().unwrap().1[0], &[]);
                pass.set_bind_group(1, &material.binding, &[]);
                pass.set_vertex_buffer(0, *self.strip_vertices.as_ref().unwrap().slice(u64::from(strip_first) * 40..));
                pass.draw(0..strip, first..first + 1);
                first += draw.instances.len() as u32;
                strip_first += strip;
                continue;
            }
            let (geometry,binding)=if let Some((source,_))=&draw.mesh {
                let cached=&self.meshes[source]; (&cached.0,&cached.1.as_ref().unwrap().1)
            } else {(&self.grids[level],&self.view_bindings.as_ref().unwrap().1[level])};
            pass.set_bind_group(0, binding, &[]);
            pass.set_bind_group(1, &material.binding, &[]);
            pass.set_index_buffer(*geometry.indices.slice(..), IndexFormat::Uint16);
            let count = draw.instances.len() as u32;
            pass.draw_indexed(0..geometry.count, 0, first..first + count);
            first += count;
        }
        drop(pass);
        if phase == ParticlePhase::Colour && !self.reported_mesh_submission {
            let meshes=draws.iter().filter(|draw|draw.mesh.is_some()&&!draw.material.distortion).collect::<Vec<_>>();
            if !meshes.is_empty() {
                bevy::log::info!(draws=meshes.len(),instances=meshes.iter().map(|draw|draw.instances.len()).sum::<usize>(),
                    "Particle mesh colour submission active");
                self.reported_mesh_submission=true;
            }
        }
        if accumulation {
            self.transparency
                .as_mut()
                .unwrap()
                .record_resolve(device, queue, encoder, &input)?;
        }
        if phase == ParticlePhase::Colour && draws.iter().any(|draw| draw.material.distortion) {
            if self.distortion.as_ref().is_none_or(|(extent, compressed, _, _)|
                *extent != input.extent || *compressed != input.compressed_hdr) {
                let format = if input.compressed_hdr { TextureFormat::Rg11b10Ufloat } else { TextureFormat::Rgba16Float };
                self.distortion = Some((input.extent, input.compressed_hdr,
                    super::screen_effects::passes::DistortionPasses::new(device, queue, input.distortion_sources, input.extent, 0, format)?,
                    super::water_hdr::WaterHdr::new(device, queue, input.extent, input.compressed_hdr, self.colour_format)?));
            }
            let (_, _, vectors, hdr) = self.distortion.as_mut().unwrap();
            vectors.record_vectors(device, queue, encoder, input.distortion_sources, &[], None, input.local_time, input.depth, input.inverse_z, None)?;
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("particle distortion vectors"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &vectors.vector_view, depth_slice: None, resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: input.depth, depth_ops: None,
                    stencil_ops: self.depth_format.has_stencil_aspect().then_some(Operations {load: LoadOp::Load, store: StoreOp::Store}),
                }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_bind_group(2, &self.fog_binding.as_ref().unwrap().1, &[]);
            pass.set_bind_group(3, &self.lighting_binding.as_ref().unwrap().1, &[]);
            let mut first = 0;
            let mut strip_first = 0u32;
            for draw in draws {
                let count = draw.instances.len() as u32;
                let strip = draw.strip.len() as u32;
                if draw.material.distortion && strip != 0 {
                    let material = &self.materials[&(draw.material.name.clone(), draw.state, ParticlePhase::Colour, true)];
                    pass.set_pipeline(&material.pipeline);
                    pass.set_stencil_reference(draw.stencil_reference);
                    pass.set_bind_group(0, &self.view_bindings.as_ref().unwrap().1[0], &[]);
                    pass.set_bind_group(1, &material.binding, &[]);
                    pass.set_vertex_buffer(0, *self.strip_vertices.as_ref().unwrap().slice(u64::from(strip_first) * 40..));
                    pass.draw(0..strip, first..first + 1);
                } else if draw.material.distortion {
                    let level = draw.grid_level.min(5) as usize;
                    let material = &self.materials[&(draw.material.name.clone(), draw.state, ParticlePhase::Colour, false)];
                    pass.set_pipeline(&material.pipeline);
                    pass.set_stencil_reference(draw.stencil_reference);
                    let (geometry,binding)=if let Some((source,_))=&draw.mesh {
                        let cached=&self.meshes[source]; (&cached.0,&cached.1.as_ref().unwrap().1)
                    } else {(&self.grids[level],&self.view_bindings.as_ref().unwrap().1[level])};
                    pass.set_bind_group(0, binding, &[]);
                    pass.set_bind_group(1, &material.binding, &[]);
                    pass.set_index_buffer(*geometry.indices.slice(..), IndexFormat::Uint16);
                    pass.draw_indexed(0..geometry.count, 0, first..first+count);
                }
                first += count;
                strip_first += strip;
            }
            drop(pass);
            let prepared = hdr.prepare(device, queue, encoder, input.colour, input.extent,
                input.camera_exposure, input.white_level, input.adaptation)?;
            vectors.record_resolve(device, encoder, prepared.source(), prepared.colour());
            prepared.finish_source(encoder);
        }
        Ok(())
    }
}

/// The shared sprite grid levels 0-5 as instanced draw geometry.
fn sprite_grids(device: &RenderDevice) -> Vec<GeometryGpu> {
    let mut grids = Vec::new();
    for level in 0..6 {
        let grid = SpriteGrid::new(level);
        let vertices = grid
            .vertices
            .iter()
            .flat_map(|v| {
                v.position
                    .into_iter()
                    .chain([0.])
                    .chain(v.uv)
                    .chain([0., 0.])
                    .chain([1., 0., 0., 0.])
                    .chain([0., 1., 0., 0.])
                    .chain([0., 0., 1., 0.])
                    .chain([1.; 4])
            })
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>();
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("particle grid vertices"),
            contents: &vertices,
            usage: BufferUsages::STORAGE,
        });
        let bytes = grid
            .indices
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        let indices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("particle grid indices"),
            contents: &bytes,
            usage: BufferUsages::INDEX,
        });
        grids.push(GeometryGpu {
            vertices,
            indices,
            count: grid.indices.len() as u32,
        });
    }
    grids
}

/// 232480 uploads PMODEL draw vertices; 2320b0 supplies its shared quad indices.
fn mesh_geometry(device:&RenderDevice,model:&crate::assets::particle_model::ParticleModel)->Result<GeometryGpu,String> {
    if model.vertices.len()>65536 || model.vertices.len()%4!=0 || model.vertices.is_empty() {
        return Err("particle render mesh must contain complete u16-indexed quads".into());
    }
    let mut bytes=Vec::with_capacity(model.vertices.len()*96);
    for vertex in &model.vertices {
        let half=|at| half::f16::from_bits(u16::from_le_bytes([vertex.attributes[at],vertex.attributes[at+1]])).to_f32();
        let snorm=|at| ((vertex.attributes[at] as i8 as f32)/127.).max(-1.);
        let normal=Vec3::new(snorm(8),snorm(9),snorm(10));
        let tangent=Vec3::new(snorm(12),snorm(13),snorm(14));
        let bitangent=normal.cross(tangent)*snorm(15);
        bytes.extend(vertex.position.into_iter().chain([0.]).chain([half(0),half(2),half(4),half(6)])
            .chain(normal.to_array()).chain([snorm(11)]).chain(tangent.to_array()).chain([snorm(15)])
            .chain(bitangent.to_array()).chain([0.]).chain(vertex.colour().map(|v|f32::from(v)/255.))
            .flat_map(f32::to_le_bytes));
    }
    let vertices=device.create_buffer_with_data(&BufferInitDescriptor{label:Some("particle mesh vertices"),contents:&bytes,usage:BufferUsages::STORAGE});
    let indices=(0..model.vertices.len()).step_by(4).flat_map(|base|[0,2,3,0,3,1].map(|i|(base+i) as u16))
        .flat_map(u16::to_le_bytes).collect::<Vec<_>>();
    let count=indices.len() as u32/2;
    let indices=device.create_buffer_with_data(&BufferInitDescriptor{label:Some("particle mesh indices"),contents:&indices,usage:BufferUsages::INDEX});
    Ok(GeometryGpu{vertices,indices,count})
}

impl Drop for ParticlePass {
    fn drop(&mut self) {
        if self.pipeline_requests != 0
            && std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY particle pipeline release: requests={} unique={} reused={} binding_refreshes={} created={}",
                self.pipeline_requests, self.programs.pipelines.len(),
                self.pipeline_requests.saturating_sub(self.pipeline_creations), self.binding_refreshes, self.pipeline_creations);
        }
    }
}

