//! Packed view depth and the shared AO/sun X/Y filter. No light atlas input:
//! these stages remain available while directional-shadow ownership changes.
use super::screen_shadow_pipeline::{
    screen_shader, DwaoBlurPipeline, DwaoBlurUniform, ScreenShadowUniform,
};
use bevy::{
    color::LinearRgba,
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
    },
};

pub(super) struct ScreenVisibilityFilter {
    extent: [u32; 2],
    pub depth: TextureView,
    attachment: TextureView,
    _intermediate: Texture,
    intermediate: TextureView,
    parameters: Buffer,
    bindings: [BindGroup; 2],
    pack: RenderPipeline,
    pack_binding: BindGroup,
    blur: [DwaoBlurPipeline; 2],
    offsets: [BindGroup; 2],
}
fn uniform(device: &RenderDevice, value: impl Into<ShaderBuffer>) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("screen visibility filter parameters"),
        contents: &value.into().data.unwrap(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}
impl ScreenVisibilityFilter {
    pub fn new(
        device: &RenderDevice,
        depth: &Texture,
        output: &TextureView,
        extent: [u32; 2],
        state: [u32; 9],
        depth_default: Option<&TextureView>,
    ) -> Result<Self, String> {
        if depth.sample_count() != 1
            || [depth.width(), depth.height()] != extent
            || extent.contains(&0)
        {
            return Err("visibility filter requires matching single-sample view depth".into());
        }
        let uniform_entry = BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(192),
            },
        };
        let texture_entry = |binding, sample_type| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Texture {
                sample_type,
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
        };
        let layout = device.create_bind_group_layout(
            Some("screen visibility filter inputs"),
            &[
                uniform_entry.clone(),
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
                },
                texture_entry(8, TextureSampleType::Float { filterable: false }),
            ],
        );
        let pack_layout = device.create_bind_group_layout(
            Some("screen packed depth input"),
            &[
                uniform_entry,
                if depth_default.is_some() {
                    texture_entry(9, TextureSampleType::Float { filterable: false })
                } else {
                    texture_entry(2, TextureSampleType::Depth)
                },
            ],
        );
        let module = screen_shader(device);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("screen packed depth"),
            bind_group_layouts: &[Some(&pack_layout)],
            immediate_size: 0,
        });
        let pack = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("screen packed depth"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("screen_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some(if depth_default.is_some() {
                    "screen_pack_default_depth"
                } else {
                    "screen_pack_depth"
                }),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: ColorWrites::GREEN | ColorWrites::BLUE | ColorWrites::ALPHA,
                })],
            }),
            primitive: default(),
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        let blur_depth = crate::shadow_pipeline::depth_stencil_gpu(&state, depth.format())?;
        let blur = std::array::from_fn(|i| {
            DwaoBlurPipeline::with_layout(
                device,
                &layout,
                &module,
                ColorTargetState {
                    format: if i == 0 {
                        TextureFormat::Rgba16Float
                    } else {
                        TextureFormat::Rgba8Unorm
                    },
                    blend: None,
                    write_mask: ColorWrites::ALL,
                },
                blur_depth.clone(),
            )
        });
        let intermediate = device.create_texture(&TextureDescriptor {
            label: Some("AO and sun visibility intermediate"),
            size: Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
            dimension: TextureDimension::D2,
            mip_level_count: 1,
            sample_count: 1,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let intermediate_view = intermediate.create_view(&default());
        let sampled_depth = depth.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..default()
        });
        let attachment = depth.create_view(&default());
        let parameters = device.create_buffer(&BufferDescriptor {
            label: Some("screen visibility view"),
            size: 192,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let point = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..default()
        });
        let pack_binding = device.create_bind_group(
            Some("screen packed depth"),
            &pack_layout,
            &[
                BindGroupEntry {
                    binding: 1,
                    resource: parameters.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: if depth_default.is_some() { 9 } else { 2 },
                    resource: BindingResource::TextureView(depth_default.unwrap_or(&sampled_depth)),
                },
            ],
        );
        let bindings = [output, &intermediate_view].map(|input| {
            device.create_bind_group(
                Some("screen visibility filter"),
                &layout,
                &[
                    BindGroupEntry {
                        binding: 1,
                        resource: parameters.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(&point),
                    },
                    BindGroupEntry {
                        binding: 8,
                        resource: BindingResource::TextureView(input),
                    },
                ],
            )
        });
        let offsets = std::array::from_fn(|i| {
            blur[i].bind(
                device,
                &uniform(
                    device,
                    DwaoBlurUniform::from_owner(
                        extent.map(|v| v as f32),
                        if i == 0 { [1.0, 0.0] } else { [0.0, 1.0] },
                        1.0,
                        0.1,
                        0.0,
                    )
                    .unwrap(),
                ),
            )
        });
        Ok(Self {
            extent,
            depth: sampled_depth,
            attachment,
            _intermediate: intermediate,
            intermediate: intermediate_view,
            parameters,
            bindings,
            pack,
            pack_binding,
            blur,
            offsets,
        })
    }
    pub fn scratch(&self) -> &TextureView {
        &self.intermediate
    }
    pub fn pack(
        &self,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        output: &TextureView,
        unproject: Vec4,
        scale: [f32; 2],
    ) {
        // Only unprojection and scale are read by these entries. The other
        // host transport lanes are explicitly unused, not invented camera data.
        let view = ScreenShadowUniform {
            inverse_view_projection: [Vec4::ZERO; 4],
            unproject,
            camera_time: Vec4::ZERO,
            depth_scale_step_hack: Vec4::new(scale[0], scale[1], 0.0, 0.0),
            target: UVec4::new(self.extent[0], self.extent[1], 0, 0),
            blur: Vec4::ZERO,
            cloud_motion: Vec4::ZERO,
            cloud_density: Vec4::ZERO,
            options: UVec4::ZERO,
        };
        queue.write_buffer(&self.parameters, 0, &ShaderBuffer::from(view).data.unwrap());
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("screen depth and neutral visibility"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::WHITE.into()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&self.pack);
        pass.set_bind_group(0, &self.pack_binding, &[]);
        pass.draw(0..3, 0..1);
    }
    pub fn filter(&self, encoder: &mut CommandEncoder, output: &TextureView) {
        let viewport = [
            0.0,
            0.0,
            self.extent[0] as f32,
            self.extent[1] as f32,
            0.0,
            1.0,
        ];
        self.blur[0].draw(
            encoder,
            &self.bindings[0],
            &self.offsets[0],
            &self.intermediate,
            &self.attachment,
            Operations {
                load: LoadOp::Clear(LinearRgba::new(0.0, 0.0, 0.0, 0.0)),
                store: StoreOp::Store,
            },
            viewport,
        );
        self.blur[1].draw(
            encoder,
            &self.bindings[1],
            &self.offsets[1],
            output,
            &self.attachment,
            Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            viewport,
        );
    }
}

#[cfg(test)]
#[path = "screen_visibility_tests.rs"]
mod tests;
