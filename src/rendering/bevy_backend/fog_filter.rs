//! Executable/shader-backed bilateral and Gaussian fog filtering pipelines.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
#[derive(Clone, Copy, Default, ShaderType)]
pub struct FogBlurUniform {
    pub ratio: Vec4,
    pub offset: Vec4,
    pub misc: Vec4,
}

/// Same-view attachments; radial visibility is distinct from fog shadowing.
pub struct FogFilterSources<'a> {
    pub color: &'a TextureView,
    pub scratch: &'a TextureView,
    pub depth: &'a TextureView,
    pub rays: Option<&'a TextureView>,
    pub extent: UVec2,
}

pub struct FogFilter {
    pipelines: FogFilterPipelines,
    uniforms: [UniformBuffer<FogBlurUniform>; 2],
    linear: Sampler,
    point: Sampler,
    bindings: Option<([TextureViewId; 4], [BindGroup; 2])>,
}
impl FogFilter {
    pub fn new(device: &RenderDevice) -> Self {
        let sampler = |filter| {
            device.create_sampler(&SamplerDescriptor {
                min_filter: filter,
                mag_filter: filter,
                ..default()
            })
        };
        Self {
            pipelines: FogFilterPipelines::new(device),
            uniforms: std::array::from_fn(|_| UniformBuffer::default()),
            linear: sampler(FilterMode::Linear),
            point: sampler(FilterMode::Nearest),
            bindings: None,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        settings: crate::platform::game_settings::FogRenderSettings,
        sources: FogFilterSources,
    ) {
        for (index, uniform) in self.uniforms.iter_mut().enumerate() {
            uniform.set(FogBlurUniform::new(settings, sources.extent, index == 0));
            uniform.write_buffer(device, queue);
        }
        // The common explicit layout retains the ray slot in plain variants.
        // That variant never samples it; bind existing depth without allocating
        // an invented visibility texture.
        let rays = sources.rays.unwrap_or(sources.depth);
        let ids = [
            sources.color.id(),
            sources.scratch.id(),
            sources.depth.id(),
            rays.id(),
        ];
        if self.bindings.as_ref().is_none_or(|old| old.0 != ids) {
            let bindings =
                std::array::from_fn(|index| {
                    let color = if index == 0 {
                        sources.color
                    } else {
                        sources.scratch
                    };
                    let textures = [color, sources.depth, rays];
                    let mut entries = vec![BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms[index].buffer().unwrap().as_entire_binding(),
                    }];
                    entries.extend(textures.into_iter().enumerate().map(|(i, view)| {
                        BindGroupEntry {
                            binding: i as u32 + 1,
                            resource: BindingResource::TextureView(view),
                        }
                    }));
                    entries.extend([&self.linear, &self.point].into_iter().enumerate().map(
                        |(i, sampler)| BindGroupEntry {
                            binding: i as u32 + 4,
                            resource: BindingResource::Sampler(sampler),
                        },
                    ));
                    device.create_bind_group(
                        Some("fog separable filter inputs"),
                        &self.pipelines.layout,
                        &entries,
                    )
                });
            self.bindings = Some((ids, bindings));
        }
        let bindings = &self.bindings.as_ref().unwrap().1;
        for (index, target) in [sources.scratch, sources.color].into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(if index == 0 {
                    "fog horizontal filter"
                } else {
                    "fog vertical filter"
                }),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(
                self.pipelines
                    .pipeline(settings.gaussian_blur, sources.rays.is_some()),
            );
            pass.set_bind_group(0, &bindings[index], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
impl FogBlurUniform {
    pub fn new(
        settings: crate::platform::game_settings::FogRenderSettings,
        extent: UVec2,
        horizontal: bool,
    ) -> Self {
        let prepared = crate::rendering::fog::filter::BlurParameters::new(settings);
        Self {
            ratio: prepared.ratio,
            offset: if horizontal {
                Vec4::new(1. / extent.x as f32, 0., 0., 0.)
            } else {
                Vec4::new(0., 1. / extent.y as f32, 0., 0.)
            },
            misc: Vec4::new(prepared.depth_range, prepared.ray_sharpness, 0., 0.),
        }
    }
}
pub struct FogFilterPipelines {
    pub layout: BindGroupLayout,
    pipelines: [RenderPipeline; 4],
}
impl FogFilterPipelines {
    pub fn new(device: &RenderDevice) -> Self {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("fog bilateral and Gaussian filter"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_fog_filter.wgsl").into(),
            ),
        });
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: Some(FogBlurUniform::min_size()),
            },
            count: None,
        }];
        entries.extend((1..=3).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }));
        entries.extend((4..=5).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        }));
        let layout = device.create_bind_group_layout(Some("fog filter inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("fog filter"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = std::array::from_fn(|index| {
            let constants = [
                ("FOG_GAUSSIAN", f64::from((index & 1) as u32)),
                ("FOG_GODRAYS", f64::from((index >> 1) as u32)),
            ];
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("fog depth-aware filter"),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some("fragment"),
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
    pub fn pipeline(&self, gaussian: bool, god_rays: bool) -> &RenderPipeline {
        &self.pipelines[usize::from(gaussian) + 2 * usize::from(god_rays)]
    }
}
