//! Visible sun sprite: authored texture, persistent uniforms and two blend modes.
use super::fog_sun_mask::{FogSunMaskUniform, FogSunProgram};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct SunSpritePass {
    pub state: crate::render_state::RenderState,
    pipelines: [RenderPipeline; 2],
    layout: BindGroupLayout,
    uniform: UniformBuffer<FogSunMaskUniform>,
    sampler: Sampler,
    binding: Option<(TextureViewId, BufferId, BindGroup)>,
}
impl SunSpritePass {
    pub fn new(
        device: &RenderDevice,
        program: &FogSunProgram,
        state: crate::render_state::RenderState,
        hdr: TextureFormat,
        depth: TextureFormat,
        initial: FogSunMaskUniform,
    ) -> Result<Self, String> {
        let layout = device.create_bind_group_layout(
            Some("sun sprite inputs"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(FogSunMaskUniform::min_size()),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 5,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("sun sprite"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |additive| -> Result<RenderPipeline, String> {
            let prepared = crate::render_state::sun_sprite_draw_state(state, additive);
            let blend = crate::render_state::blend_state(&prepared).targets[0]
                .ok_or("sun sprite has no colour output")?;
            let (front_face, cull_mode) =
                super::shadow_pipeline::shadow_mesh_raster(&prepared, false);
            Ok(device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("visible sun sprite"),
                layout: Some(&pipeline_layout),
                vertex: RawVertexState {
                    module: &program.module,
                    entry_point: Some("fog_sun_vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &program.module,
                    entry_point: Some("fog_sun_sprite_fragment"),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format: hdr,
                        blend: super::pipeline_state::blend_target(blend)?,
                        write_mask: ColorWrites::from_bits_truncate(u32::from(blend.write_mask)),
                    })],
                }),
                primitive: PrimitiveState {
                    front_face,
                    cull_mode,
                    ..default()
                },
                depth_stencil: Some(super::shadow_pipeline::depth_stencil_gpu(&prepared, depth)?),
                multisample: default(),
                multiview_mask: None,
                cache: None,
            }))
        };
        Ok(Self {
            state,
            pipelines: [make(false)?, make(true)?],
            layout,
            uniform: UniformBuffer::from(initial),
            sampler: device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            }),
            binding: None,
        })
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        uniform: FogSunMaskUniform,
        image: &TextureView,
    ) {
        self.uniform.set(uniform);
        self.uniform.write_buffer(device, queue);
        let buffer = self.uniform.buffer().unwrap();
        if self
            .binding
            .as_ref()
            .is_none_or(|old| old.0 != image.id() || old.1 != buffer.id())
        {
            let bind = device.create_bind_group(
                None,
                &self.layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(image),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            );
            self.binding = Some((image.id(), buffer.id(), bind));
        }
    }
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        hdr: &TextureView,
        depth: &TextureView,
        additive: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("visible sun sprite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            ..default()
        });
        pass.set_pipeline(&self.pipelines[usize::from(additive)]);
        pass.set_bind_group(
            0,
            &self
                .binding
                .as_ref()
                .expect("sun sprite must be prepared")
                .2,
            &[],
        );
        pass.draw(0..6, 0..1);
    }
}
