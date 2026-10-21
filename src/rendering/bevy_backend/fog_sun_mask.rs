//! Projected sun colour-mask program and its explicit radiance transport.
use super::fog::{FogParameters, FogView};
use bevy::{
    prelude::*,
    render::{render_resource::*, renderer::RenderDevice},
};

#[derive(Clone, Copy, ShaderType)]
pub struct FogSunMaskUniform {
    pub transform1: Vec4,
    pub transform2: Vec4,
    pub threshold_bias: Vec4,
    pub threshold_scale: Vec4,
    pub color: Vec4,
    pub projection: Vec4,
    pub viewport: Vec4,
    pub game_world_from_view: Mat4,
}
impl FogSunMaskUniform {
    pub fn sprite(
        settings: crate::rendering::fog::sun_sprite::SunSpriteParameters,
        projection: crate::rendering::fog::rays::RayProjection,
        view: FogView,
        extent: UVec2,
    ) -> Self {
        Self {
            transform1: projection.transform1,
            transform2: projection.transform2,
            color: settings.color,
            projection: Vec4::new(0., 0., view.ray_scale.z, 1. / (139.26 * view.unproject.w)),
            viewport: Vec4::new(extent.x as f32, extent.y as f32, 0., 0.),
            // These fields belong to the scattering mask's vertex/fragment
            // programs; the sprite variant never reads them.
            threshold_bias: Vec4::ZERO,
            threshold_scale: Vec4::ZERO,
            game_world_from_view: Mat4::IDENTITY,
        }
    }
    /// `game_world_from_view` maps camera vectors to the game's Z-up basis.
    /// The scene HDR target contains game radiance *139.26*camera exposure.
    pub fn new(
        settings: crate::rendering::fog::rays::RaySettings,
        projection: crate::rendering::fog::rays::RayProjection,
        view: FogView,
        game_world_from_view: Mat4,
        extent: UVec2,
    ) -> Self {
        Self {
            transform1: projection.transform1,
            transform2: projection.transform2,
            threshold_bias: settings.threshold_bias.extend(0.),
            threshold_scale: settings.threshold_scale.extend(0.),
            color: settings.color,
            projection: Vec4::new(
                view.ray_scale.x,
                view.ray_scale.y,
                view.ray_scale.z,
                1. / (139.26 * view.unproject.w),
            ),
            viewport: Vec4::new(extent.x as f32, extent.y as f32, settings.depth_start, 0.),
            game_world_from_view,
        }
    }
}
pub struct FogSunProgram {
    pub(super) module: ShaderModule,
    pub layout: BindGroupLayout,
    pub(super) pipeline_layout: PipelineLayout,
}
impl FogSunProgram {
    pub fn new(device: &RenderDevice) -> Self {
        let source = format!(
            "{}\n{}\n{}",
            include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_sun_mask.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_sun_sprite.wgsl")
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("sun colour mask"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let mut entries = [FogSunMaskUniform::min_size(), FogParameters::min_size()]
            .into_iter()
            .enumerate()
            .map(|(i, size)| BindGroupLayoutEntry {
                binding: i as u32,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(size),
                },
                count: None,
            })
            .collect::<Vec<_>>();
        entries.extend((2..=4).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }));
        entries.push(BindGroupLayoutEntry {
            binding: 5,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        });
        let layout = device.create_bind_group_layout(Some("sun colour-mask inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("sun colour mask"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        Self {
            module,
            layout,
            pipeline_layout,
        }
    }
}

pub struct FogSunMaskPipelines {
    pub layout: BindGroupLayout,
    pipelines: [RenderPipeline; 3],
}
impl FogSunMaskPipelines {
    pub fn new(device: &RenderDevice) -> Self {
        Self::with_program(device, &FogSunProgram::new(device))
    }
    pub fn with_program(device: &RenderDevice, program: &FogSunProgram) -> Self {
        let module = program.module.clone();
        let pipeline_layout = program.pipeline_layout.clone();
        let layout = program.layout.clone();
        let pipelines = std::array::from_fn(|index| {
            let constants = [
                ("SUN_SCATTERING", f64::from(u32::from(index != 0))),
                ("SUN_SHADOWING", f64::from(u32::from(index == 2))),
            ];
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("projected sun colour mask"),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("fog_sun_vertex"),
                    compilation_options: PipelineCompilationOptions {
                        constants: &constants,
                        ..default()
                    },
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some("fog_sun_fragment"),
                    compilation_options: PipelineCompilationOptions {
                        constants: &constants,
                        ..default()
                    },
                    targets: &[Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self { layout, pipelines }
    }
    pub fn pipeline(&self, scattering: bool, shadow: bool) -> &RenderPipeline {
        &self.pipelines[if scattering {
            1 + usize::from(shadow)
        } else {
            0
        }]
    }
}
