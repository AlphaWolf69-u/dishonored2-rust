//! 313730: composite prepared transparent blur/refraction over the HDR image.
//! Geometry, opacity, blurred colour and near alpha belong to the view task.
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct AlphaCompositeInputs<'a> {
    pub refraction: &'a TextureView,
    pub blurred: &'a TextureView,
    pub near_alpha: &'a TextureView,
    pub destination: &'a TextureView,
    pub depth_alpha: &'a TextureView,
    pub full_extent: [u32; 2],
    pub viewport: [u32; 4],
    pub scissor: [u32; 4],
    pub vertices: &'a Buffer,
    pub vertex_range: std::ops::Range<u32>,
    pub host: Option<AlphaHostInputs<'a>>,
}
pub struct AlphaHostInputs<'a> {
    pub adaptation: &'a Buffer,
    pub radiance_units: f32,
    pub white_level: f32,
    pub compressed: bool,
}
pub struct CameraAlphaComposite {
    pipeline: RenderPipeline,
    uniform: Buffer,
    sampler: Sampler,
    reference: u32,
    binding: Option<(([TextureViewId; 3], Option<BufferId>), BindGroup)>,
    extent: Option<[u32; 2]>,
    host_uniform: Option<Buffer>,
}
impl CameraAlphaComposite {
    pub fn new(
        device: &RenderDevice,
        colour: TextureFormat,
        depth: TextureFormat,
        stencil: bool,
        special_alpha: bool,
    ) -> Result<Self, String> {
        Self::with_colour_source(device, colour, depth, stencil, special_alpha, false)
    }
    pub fn with_host_colour(
        device: &RenderDevice,
        colour: TextureFormat,
        depth: TextureFormat,
        stencil: bool,
        special_alpha: bool,
    ) -> Result<Self, String> {
        Self::with_colour_source(device, colour, depth, stencil, special_alpha, true)
    }
    fn with_colour_source(
        device: &RenderDevice,
        colour: TextureFormat,
        depth: TextureFormat,
        stencil: bool,
        special_alpha: bool,
        host: bool,
    ) -> Result<Self, String> {
        if !depth.has_stencil_aspect() {
            return Err(
                "camera alpha composite requires the actual alpha depth/stencil target".into(),
            );
        }
        let (state, reference) =
            crate::render_state::camera_alpha_composite_state(stencil, special_alpha);
        let blend = crate::render_state::blend_state(&state).targets[0]
            .ok_or("camera alpha composite has no colour state")?;
        let source = concat!(
            include_str!("../../../assets/shaders/dh2_glass_distortion.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_camera_alpha_composite.wgsl")
        );
        let source = if host {
            let source = source.replace("return vec4(colour,alpha);", "return vec4(colour * alpha_host.x * select(1.0, alpha_adaptation[0].x * alpha_host.y, alpha_host.z != 0.0),alpha);");
            format!("@group(0) @binding(5) var<uniform> alpha_host:vec4<f32>;\n@group(0) @binding(6) var<storage,read> alpha_adaptation:array<vec4<f32>>;\n{source}")
        } else {
            source.to_owned()
        };
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera alpha composite"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, false);
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera alpha composite"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("alpha_vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: 20,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
                        VertexAttribute {
                            format: VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 12,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("alpha_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: colour,
                    blend: super::pipeline_state::blend_target(blend)?,
                    write_mask: ColorWrites::from_bits_truncate(u32::from(blend.write_mask)),
                })],
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
            depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(&state, depth)?),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            pipeline,
            reference,
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("camera alpha pixel scale"),
                size: 16,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            sampler: device.create_sampler(&SamplerDescriptor {
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                ..default()
            }),
            binding: None,
            extent: None,
            host_uniform: host.then(|| {
                device.create_buffer(&BufferDescriptor {
                    label: Some("camera alpha host radiance"),
                    size: 16,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            }),
        })
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: AlphaCompositeInputs<'_>,
    ) -> Result<(), String> {
        if inputs.host.is_some() != self.host_uniform.is_some() {
            return Err("camera alpha destination differs from its prepared colour adapter".into());
        }
        if let Some(host) = &inputs.host {
            if !host.radiance_units.is_finite()
                || host.radiance_units <= 0.
                || !host.white_level.is_finite()
                || host.white_level <= 0.
                || host.adaptation.size() < 32
            {
                return Err("camera alpha host radiance parameters are invalid".into());
            }
            queue.write_buffer(
                self.host_uniform.as_ref().unwrap(),
                0,
                &[
                    host.radiance_units,
                    host.white_level,
                    if host.compressed { 1. } else { 0. },
                    0.,
                ]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            );
        }
        if inputs.full_extent.contains(&0) {
            return Err("camera alpha composite extent is empty".into());
        }
        for region in [inputs.viewport, inputs.scissor] {
            if region[2] == 0
                || region[3] == 0
                || region[0]
                    .checked_add(region[2])
                    .is_none_or(|n| n > inputs.full_extent[0])
                || region[1]
                    .checked_add(region[3])
                    .is_none_or(|n| n > inputs.full_extent[1])
            {
                return Err("camera alpha composite region exceeds the HDR view".into());
            }
        }
        let images = [inputs.refraction, inputs.blurred, inputs.near_alpha];
        if images.iter().any(|v| v.id() == inputs.destination.id()) {
            return Err("camera alpha composite input aliases HDR destination".into());
        }
        if self.extent != Some(inputs.full_extent) {
            let values = [
                0.0f32,
                0.0,
                1.0 / inputs.full_extent[0] as f32,
                1.0 / inputs.full_extent[1] as f32,
            ];
            queue.write_buffer(
                &self.uniform,
                0,
                &values
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            self.extent = Some(inputs.full_extent);
        }
        let ids = images.map(TextureView::id);
        let key = (ids, inputs.host.as_ref().map(|h| h.adaptation.id()));
        if self.binding.as_ref().is_none_or(|v| v.0 != key) {
            let mut entries = vec![BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            entries.extend(images.into_iter().enumerate().map(|(i, v)| BindGroupEntry {
                binding: i as u32 + 1,
                resource: BindingResource::TextureView(v),
            }));
            entries.push(BindGroupEntry {
                binding: 4,
                resource: BindingResource::Sampler(&self.sampler),
            });
            if let Some(host) = &inputs.host {
                entries.extend([
                    BindGroupEntry {
                        binding: 5,
                        resource: self.host_uniform.as_ref().unwrap().as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: host.adaptation.as_entire_binding(),
                    },
                ]);
            }
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("camera alpha sources"),
                    &self.pipeline.get_bind_group_layout(0).into(),
                    &entries,
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera alpha composite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: inputs.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: inputs.depth_alpha,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
            }),
            ..default()
        });
        let [x, y, w, h] = inputs.viewport;
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        let [x, y, w, h] = inputs.scissor;
        pass.set_scissor_rect(x, y, w, h);
        pass.set_pipeline(&self.pipeline);
        pass.set_stencil_reference(self.reference);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_vertex_buffer(0, *inputs.vertices.slice(..));
        pass.draw(inputs.vertex_range, 0..1);
        Ok(())
    }
}
