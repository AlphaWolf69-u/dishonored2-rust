//!307530's horizontal/vertical expansion of the existing R8 ray mask.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct RayVisibilityFilter {
    extent: UVec2,
    _scratch: Texture,
    scratch: TextureView,
    pipeline: RenderPipeline,
    uniforms: [Buffer; 2],
    mirror: Sampler,
    bindings: Option<(TextureViewId, [BindGroup; 2])>,
    radius: Option<u32>,
}
impl RayVisibilityFilter {
    pub fn new(device: &RenderDevice, extent: UVec2) -> Self {
        let scratch = device.create_texture(&TextureDescriptor {
            label: Some("ray visibility scratch"),
            size: Extent3d {
                width: extent.x,
                height: extent.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = scratch.create_view(&default());
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ray visibility expansion"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_fog_ray_visibility_filter.wgsl")
                    .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("ray visibility expansion"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("visibility_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("visibility_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: ColorWrites::RED,
                })],
            }),
            primitive: PrimitiveState {
                cull_mode: None,
                ..default()
            },
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            extent,
            _scratch: scratch,
            scratch: view,
            pipeline,
            uniforms: std::array::from_fn(|_| {
                device.create_buffer(&BufferDescriptor {
                    label: Some("ray visibility parameters"),
                    size: 16,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            }),
            mirror: device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::MirrorRepeat,
                address_mode_v: AddressMode::MirrorRepeat,
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            }),
            bindings: None,
            radius: None,
        }
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        radius: f32,
    ) {
        if self.radius != Some(radius.to_bits()) {
            for (index, uniform) in self.uniforms.iter().enumerate() {
                let words = [
                    (self.extent.x as f32).to_bits(),
                    (self.extent.y as f32).to_bits(),
                    radius.to_bits(),
                    u32::from(index == 0),
                ];
                queue.write_buffer(
                    uniform,
                    0,
                    &words
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
            }
            self.radius = Some(radius.to_bits());
        }
        if self.bindings.as_ref().is_none_or(|v| v.0 != source.id()) {
            self.bindings = Some((
                source.id(),
                std::array::from_fn(|index| {
                    device.create_bind_group(
                        Some("ray visibility inputs"),
                        &self.pipeline.get_bind_group_layout(0).into(),
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: self.uniforms[index].as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: BindingResource::TextureView(if index == 0 {
                                    source
                                } else {
                                    &self.scratch
                                }),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: BindingResource::Sampler(&self.mirror),
                            },
                        ],
                    )
                }),
            ));
        }
        for (index, target) in [&self.scratch, source].into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(if index == 0 {
                    "horizontal ray visibility"
                } else {
                    "vertical ray visibility"
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bindings.as_ref().unwrap().1[index], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
