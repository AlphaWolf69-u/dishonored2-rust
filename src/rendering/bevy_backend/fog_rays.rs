//! Radial sun-ray programs. Source colour-mask production and the same-view
//! fog stencil attachment remain the scene owner's responsibility.
use super::fog::FogParameters;
use bevy::{
    prelude::*,
    render::{render_resource::*, renderer::RenderDevice},
};

#[derive(Clone, Copy, Default, ShaderType)]
pub struct FogRayUniform {
    pub center: Vec4,
    pub controls: Vec4,
    pub extent: Vec4,
}
impl FogRayUniform {
    pub fn new(
        settings: crate::rendering::fog::rays::RaySettings,
        projection: crate::rendering::fog::rays::RayProjection,
        extent: UVec2,
    ) -> Self {
        Self {
            center: projection.transform2,
            controls: Vec4::new(settings.density, settings.decay, settings.weight, 0.),
            extent: Vec4::new(
                extent.x as f32,
                extent.y as f32,
                1. / extent.x as f32,
                1. / extent.y as f32,
            ),
        }
    }
}

pub struct FogRayPipelines {
    pub layout: BindGroupLayout,
    pub visibility: RenderPipeline,
    scattering: [RenderPipeline; 2],
}
impl FogRayPipelines {
    pub fn new(
        device: &RenderDevice,
        scattering_state: crate::render_state::RenderState,
        depth_format: TextureFormat,
    ) -> Result<Self, String> {
        Self::create(device, scattering_state, Some(depth_format), [1., 1.])
    }
    pub fn with_sampled_stencil(
        device: &RenderDevice,
        state: crate::render_state::RenderState,
        depth_scale: [f32; 2],
    ) -> Result<Self, String> {
        Self::create(device, state, None, depth_scale)
    }
    fn create(
        device: &RenderDevice,
        scattering_state: crate::render_state::RenderState,
        depth_format: Option<TextureFormat>,
        depth_scale: [f32; 2],
    ) -> Result<Self, String> {
        let source = format!(
            "{}\n{}\n{}\n{}",
            include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_rays.wgsl"),
            if depth_format.is_none() {
                include_str!("../../../assets/shaders/dh2_fog_rays_stencil.wgsl")
            } else {
                ""
            }
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("radial sun rays"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let mut entries = [FogRayUniform::min_size(), FogParameters::min_size()]
            .into_iter()
            .enumerate()
            .map(|(index, size)| BindGroupLayoutEntry {
                binding: index as u32,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(size),
                },
                count: None,
            })
            .collect::<Vec<_>>();
        entries.extend((2..=5).map(|binding| BindGroupLayoutEntry {
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
            binding: 6,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        });
        if depth_format.is_none() {
            entries.push(BindGroupLayoutEntry {
                binding: 7,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Uint,
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(Some("radial ray inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("radial ray program"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let prepared = crate::render_state::fog_draw_state(scattering_state);
        let target = crate::render_state::blend_state(&prepared).targets[0]
            .ok_or("radial rays have no colour target")?;
        let blend = super::pipeline_state::blend_target(target)?;
        let depth = depth_format
            .map(|format| super::shadow_pipeline::depth_stencil_gpu(&prepared, format))
            .transpose()?;
        let make = |visibility: bool, turbulence: bool| {
            let constants = [
                ("RAY_TURBULENCE", f64::from(u32::from(turbulence))),
                ("RAY_DEPTH_SCALE_X", f64::from(depth_scale[0])),
                ("RAY_DEPTH_SCALE_Y", f64::from(depth_scale[1])),
            ];
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(if visibility {
                    "radial ray blur visibility"
                } else {
                    "radial ray scattering"
                }),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("fog_ray_vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(if visibility {
                        "fog_ray_visibility_fragment"
                    } else {
                        if depth_format.is_some() {
                            "fog_ray_scatter_fragment"
                        } else {
                            "fog_ray_scatter_stencil"
                        }
                    }),
                    compilation_options: PipelineCompilationOptions {
                        constants: &constants,
                        ..default()
                    },
                    targets: &[Some(ColorTargetState {
                        format: if visibility {
                            TextureFormat::R8Unorm
                        } else {
                            TextureFormat::Rgba16Float
                        },
                        blend: if visibility { None } else { blend },
                        write_mask: if visibility {
                            ColorWrites::RED
                        } else {
                            ColorWrites::from_bits_truncate(u32::from(target.write_mask))
                        },
                    })],
                }),
                primitive: default(),
                depth_stencil: if visibility { None } else { depth.clone() },
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Ok(Self {
            visibility: make(true, false),
            scattering: [make(false, false), make(false, true)],
            layout,
        })
    }
    pub fn scattering(&self, turbulence: bool) -> &RenderPipeline {
        &self.scattering[usize::from(turbulence)]
    }
}
