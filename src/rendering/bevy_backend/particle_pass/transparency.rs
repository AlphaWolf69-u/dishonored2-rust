//! 31dce0's weighted-blended resolve of the alpha list. The accumulation and
//! revealage targets are the ALPHA_LIT owner's (alpha_depth_scene), shared by
//! the dynamicAlpha/worldAlpha model draws and the particle draws.
use super::super::water_hdr::WaterHdr;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Copy, ShaderType)]
struct Parameters {
    limits: Vec4,
}

pub(super) struct TransparencyPass {
    hdr: WaterHdr,
    parameters: UniformBuffer<Parameters>,
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    binding: Option<((BufferId, TextureViewId, TextureViewId), BindGroup)>,
    extent: [u32; 2],
    compressed: bool,
    resolve_state: crate::render_state::RenderState,
}
impl TransparencyPass {
    pub fn matches(
        &self,
        extent: [u32; 2],
        compressed: bool,
        state: crate::render_state::RenderState,
    ) -> bool {
        self.extent == extent && self.compressed == compressed && self.resolve_state == state
    }
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        extent: [u32; 2],
        compressed: bool,
        host_format: TextureFormat,
        depth_format: TextureFormat,
        resolve_state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        let mut parameters = UniformBuffer::from(Parameters { limits: Vec4::ZERO });
        parameters.write_buffer(device, queue);
        let texture_binding = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let buffer_binding = |binding, uniform| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
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
        };
        let layout = device.create_bind_group_layout(
            Some("particle resolve"),
            &[
                texture_binding(0),
                texture_binding(1),
                buffer_binding(2, true),
                buffer_binding(3, false),
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("particle resolve"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("particle transparency resolve"),
            source: ShaderSource::Wgsl(
                concat!(
                    include_str!("../../../../assets/shaders/dh2_colour_output.wgsl"),
                    "\n",
                    include_str!(
                        "../../../../assets/shaders/dh2_particle_transparency_resolve.wgsl"
                    )
                )
                .into(),
            ),
        });
        let mut state = resolve_state;
        // 31eefb..31ef12: resolve uses stencil reference4 and mask4, separate
        // from each alpha draw's reference44/4 and mask40.
        state[5] = (state[5] & 0xff04_04ff) | 0x0004_0420;
        state[4] = (state[4] & 0xff) | 0x4004_0000;
        let blend = crate::render_state::blend_state(&state).targets[0]
            .ok_or("particle resolve colour target absent")?;
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("particle transparency resolve"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("resolve_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("resolve_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: if compressed {
                        TextureFormat::Rg11b10Ufloat
                    } else {
                        TextureFormat::Rgba16Float
                    },
                    blend: super::super::pipeline_state::blend_target(blend)?,
                    write_mask: ColorWrites::from_bits_truncate(blend.write_mask as u32),
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: Some(super::super::pipeline_state::depth_stencil_gpu(
                &state,
                depth_format,
            )?),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            hdr: WaterHdr::new(device, queue, extent, compressed, host_format)?,
            parameters,
            layout,
            pipeline,
            binding: None,
            extent,
            compressed,
            resolve_state,
        })
    }
    pub fn record_resolve(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: &super::ParticleViewInputs<'_>,
        targets: &super::AlphaListTargets<'_>,
    ) -> Result<(), String> {
        self.parameters.set(Parameters {
            limits: Vec4::new(
                input.white_level,
                if self.compressed { 1. } else { 0. },
                0.,
                0.,
            ),
        });
        self.parameters.write_buffer(device, queue);
        let key = (input.adaptation.id(), targets.accumulation.id(), targets.opacity.id());
        if self.binding.as_ref().is_none_or(|b| b.0 != key) {
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("particle resolve"),
                    &self.layout,
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(targets.accumulation),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(targets.opacity),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: self.parameters.binding().unwrap(),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: input.adaptation.as_entire_binding(),
                        },
                    ],
                ),
            ));
        }
        let hdr = self.hdr.prepare(
            device,
            queue,
            encoder,
            input.colour,
            input.extent,
            input.camera_exposure,
            input.white_level,
            input.adaptation,
        )?;
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("particle transparency resolve"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: hdr.colour(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: input.depth,
                    depth_ops: None,
                    stencil_ops: Some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
            pass.set_stencil_reference(4);
            pass.draw(0..3, 0..1);
        }
        hdr.finish(encoder);
        Ok(())
    }
}
