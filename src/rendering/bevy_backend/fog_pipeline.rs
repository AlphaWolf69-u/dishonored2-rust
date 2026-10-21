//! Pipelines for the original signed interior depth, canvas, shadow march,
//! world fog and depth-aware upsample passes. Resource lifetime and scheduling
//! belong to the fog view owner; these pipelines do not allocate frame targets.
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub struct FogPipelines {
    module: ShaderModule,
    pub interior_view_layout: BindGroupLayout,
    pub interior_model_layout: BindGroupLayout,
    pub depth: RenderPipeline,
    pub interior: RenderPipeline,
    pub canvas: RenderPipeline,
    pub shadow: RenderPipeline,
    pub world: RenderPipeline,
    pub upscale: RenderPipeline,
    pub upscale_slope: RenderPipeline,
}
impl FogPipelines {
    pub fn new(device: &RenderDevice, hdr_format: TextureFormat) -> Self {
        let source = format!(
            "{}\n{}\n{}\n{}",
            include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_shadow.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog.wgsl")
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("fog depth, canvas, shadow, world and upscale"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let make = |name: &str,
                    vertex: &str,
                    fragment: &str,
                    format: TextureFormat,
                    blend: Option<BlendState>,
                    interior: bool| {
            let attributes = [VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }];
            let vertices = [RawVertexBufferLayout {
                array_stride: 12,
                step_mode: VertexStepMode::Vertex,
                attributes: &attributes,
            }];
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(name),
                layout: None,
                vertex: RawVertexState {
                    module: &module,
                    entry_point: Some(vertex),
                    compilation_options: default(),
                    buffers: if interior { &vertices } else { &[] },
                },
                fragment: Some(RawFragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState {
                    front_face: FrontFace::Cw,
                    cull_mode: None,
                    ..default()
                },
                depth_stencil: None,
                multisample: MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent::REPLACE,
        };
        let world_blend = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::SrcAlpha,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
        };
        let composite = BlendState {
            color: BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            alpha: BlendComponent {
                src_factor: BlendFactor::Zero,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
        };
        let interior_view_layout = device.create_bind_group_layout(
            Some("fog interior view"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(super::fog::FogView::min_size()),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Depth,
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 7,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let interior_model_layout = device.create_bind_group_layout(
            Some("fog interior transforms"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(64),
                },
                count: None,
            }],
        );
        let pipelines = Self {
            module: module.clone(),
            interior_view_layout,
            interior_model_layout,
            depth: make(
                "fog linear depth min/max",
                "fog_vertex",
                "fog_depth_fragment",
                TextureFormat::Rg16Float,
                None,
                false,
            ),
            interior: make(
                "interior signed depth",
                "fog_interior_vertex",
                "fog_interior_fragment",
                TextureFormat::R16Float,
                Some(additive),
                true,
            ),
            canvas: make(
                "dynamic canvas",
                "fog_vertex",
                "fog_canvas_fragment",
                TextureFormat::R8Unorm,
                None,
                false,
            ),
            shadow: make(
                "fog shadow march",
                "fog_vertex",
                "fog_shadow_fragment",
                TextureFormat::R8Unorm,
                None,
                false,
            ),
            world: make(
                "world fog",
                "fog_vertex",
                "fog_world_fragment",
                TextureFormat::Rgba16Float,
                Some(world_blend),
                false,
            ),
            upscale: make(
                "fog nearest-depth upscale",
                "fog_vertex",
                "fog_upscale_fragment",
                hdr_format,
                Some(composite),
                false,
            ),
            upscale_slope: make(
                "fog slope-aware upscale",
                "fog_vertex",
                "fog_upscale_slope_fragment",
                hdr_format,
                Some(composite),
                false,
            ),
        };
        pipelines
    }

    pub fn interior_mesh(
        &self,
        device: &RenderDevice,
        vertex: &bevy::mesh::VertexBufferLayout,
        front_face: FrontFace,
    ) -> RenderPipeline {
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("fog interior mesh"),
            bind_group_layouts: &[
                Some(&self.interior_view_layout),
                Some(&self.interior_model_layout),
            ],
            immediate_size: 0,
        });
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("fog shared mesh signed depth"),
            layout: Some(&layout),
            vertex: RawVertexState {
                module: &self.module,
                entry_point: Some("fog_interior_mesh_vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: vertex.array_stride,
                    step_mode: vertex.step_mode,
                    attributes: &vertex.attributes,
                }],
            },
            fragment: Some(RawFragmentState {
                module: &self.module,
                entry_point: Some("fog_interior_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R16Float,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent::REPLACE,
                    }),
                    write_mask: ColorWrites::RED,
                })],
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode: None,
                ..default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}
