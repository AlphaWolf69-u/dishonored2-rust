//! ScreenDisto vector accumulation and 326930's two colour resolves.
use super::{sources::ScreenEffectSources, QuadDraw};
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::collections::HashMap;

struct MaterialGpu {
    _images: [Texture; 3],
    views: [TextureView; 3],
    binding: Option<(TextureViewId, BindGroup)>,
    pipeline: usize,
}
pub struct DistortionPasses {
    pub vectors: Texture,
    pub vector_view: TextureView,
    vector_extent: [u32; 2],
    uniform: Buffer,
    stride: usize,
    layout: BindGroupLayout,
    materials: Vec<MaterialGpu>,
    pipelines: Vec<RenderPipeline>,
    wrap: Sampler,
    clamp: Sampler,
    mirror: Sampler,
    resolve: RenderPipeline,
    resolve_uniform: [Buffer; 2],
    resolve_binding: Option<([TextureViewId; 2], [BindGroup; 2])>,
}
fn pipeline(
    device: &RenderDevice,
    module: &ShaderModule,
    layout: Option<&PipelineLayout>,
    label: &str,
    vertex: &str,
    fragment: &str,
    format: TextureFormat,
    state: &crate::render_state::RenderState,
) -> Result<RenderPipeline, String> {
    let target = crate::render_state::blend_state(state).targets[0]
        .ok_or("screen effect target state absent")?;
    Ok(device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some(label),
        layout,
        vertex: RawVertexState {
            module,
            entry_point: Some(vertex),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: super::super::pipeline_state::blend_target(target)?,
                write_mask: ColorWrites::from_bits_truncate(target.write_mask as u32),
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
    }))
}
impl DistortionPasses {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        sources: &ScreenEffectSources,
        extent: [u32; 2],
        downsample: u32,
        colour_format: TextureFormat,
    ) -> Result<Self, String> {
        let vector_extent = extent.map(|v| (v >> downsample).max(1));
        let vectors = device.create_texture(&TextureDescriptor {
            label: Some("screen distortion vectors"),
            size: Extent3d {
                width: vector_extent[0],
                height: vector_extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let vector_view = vectors.create_view(&default());
        let stride = 224u64.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) as usize;
        // The view has four entries per stage; the two vector stages share a field.
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("screen distortion material parameters"),
            size: (stride * 8) as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entries = std::iter::once(BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: BufferSize::new(224),
            },
            count: None,
        })
        .chain((1..=3).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }))
        .chain([
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Depth,
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 6,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ])
        .collect::<Vec<_>>();
        let layout = device.create_bind_group_layout(Some("screen distortion inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("screen distortion"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("screen distortion"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_screen_distortion.wgsl").into(),
            ),
        });
        let mut pipeline_indices = HashMap::new();
        let mut pipelines = Vec::new();
        let mut materials = Vec::new();
        for source in &sources.materials {
            let index = if let Some(&index) = pipeline_indices.get(&source.state) {
                index
            } else {
                let index = pipelines.len();
                pipelines.push(pipeline(
                    device,
                    &shader,
                    Some(&pipeline_layout),
                    "screen distortion field",
                    "screen_distortion_vertex",
                    "screen_distortion_fragment",
                    TextureFormat::Rgba16Float,
                    &source.state,
                )?);
                pipeline_indices.insert(source.state, index);
                index
            };
            let images = source.images.each_ref().map(|image| {
                let format = super::super::image_formats::gpu_image_format(image.info.format);
                device.create_texture_with_data(
                    queue,
                    &TextureDescriptor {
                        label: Some("screen effect installed image"),
                        size: Extent3d {
                            width: image.info.width,
                            height: image.info.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: image.info.mip_count,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format,
                        usage: TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    TextureDataOrder::LayerMajor,
                    &image.payload,
                )
            });
            let views = images.each_ref().map(|t| t.create_view(&default()));
            materials.push(MaterialGpu {
                _images: images,
                views,
                binding: None,
                pipeline: index,
            });
        }
        let sampler = |address, mipmap| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: mipmap,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        let wrap = sampler(AddressMode::Repeat, MipmapFilterMode::Linear);
        let clamp = sampler(AddressMode::ClampToEdge, MipmapFilterMode::Nearest);
        let mirror = sampler(AddressMode::MirrorRepeat, MipmapFilterMode::Nearest);
        let resolve_module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("motion distortion resolve"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_motion_distortion.wgsl").into(),
            ),
        });
        let resolve = pipeline(
            device,
            &resolve_module,
            None,
            "motion distortion resolve",
            "motion_distortion_vertex",
            "motion_distortion_fragment",
            colour_format,
            &sources.resolve_state,
        )?;
        let resolve_uniform = [1., 4.].map(|scale| {
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some("motion distortion view parameters"),
                size: 32,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &buffer,
                0,
                &[
                    1.,
                    1.,
                    1. / extent[0] as f32,
                    1. / extent[1] as f32,
                    scale,
                    0.,
                    0.,
                    0.,
                ]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            );
            buffer
        });
        Ok(Self {
            vectors,
            vector_view,
            vector_extent,
            uniform,
            stride,
            layout,
            materials,
            pipelines,
            wrap,
            clamp,
            mirror,
            resolve,
            resolve_uniform,
            resolve_binding: None,
        })
    }
    pub fn record_vectors(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        sources: &ScreenEffectSources,
        draws: &[QuadDraw],
        time: f32,
        depth: &TextureView,
        unproject: [f32; 3],
    ) -> Result<(), String> {
        if draws.len() > 8 {
            return Err("screen vector stage capacity exceeded".into());
        }
        let mut bytes = vec![0; self.stride * draws.len()];
        for (draw, chunk) in draws.iter().zip(bytes.chunks_exact_mut(self.stride)) {
            let material = &sources.materials[draw.material];
            let mut floats = material
                .layers
                .iter()
                .flat_map(|l| {
                    l.tiling_speed
                        .into_iter()
                        .chain(l.strength)
                        .chain(l.rotation_seed)
                })
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>();
            floats.extend(material.options.into_iter().flat_map(u32::to_le_bytes));
            floats.extend(
                [
                    self.vector_extent[0] as f32,
                    self.vector_extent[1] as f32,
                    time,
                    draw.blend,
                ]
                .into_iter()
                .chain(material.clamp_colour)
                .chain(material.depth)
                .chain([
                    unproject[0],
                    unproject[1],
                    unproject[2],
                    material.alpha_test,
                ])
                .flat_map(f32::to_le_bytes),
            );
            chunk[..224].copy_from_slice(&floats);
            let gpu = &mut self.materials[draw.material];
            if gpu.binding.as_ref().is_none_or(|b| b.0 != depth.id()) {
                gpu.binding = Some((
                    depth.id(),
                    device.create_bind_group(
                        Some("screen distortion material"),
                        &self.layout,
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: BindingResource::Buffer(BufferBinding {
                                    buffer: &self.uniform,
                                    offset: 0,
                                    size: BufferSize::new(224),
                                }),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: BindingResource::TextureView(&gpu.views[0]),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: BindingResource::TextureView(&gpu.views[1]),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: BindingResource::TextureView(&gpu.views[2]),
                            },
                            BindGroupEntry {
                                binding: 4,
                                resource: BindingResource::Sampler(&self.wrap),
                            },
                            BindGroupEntry {
                                binding: 5,
                                resource: BindingResource::TextureView(depth),
                            },
                            BindGroupEntry {
                                binding: 6,
                                resource: BindingResource::Sampler(&self.clamp),
                            },
                        ],
                    ),
                ));
            }
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.uniform, 0, &bytes);
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("screen distortion vectors"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.vector_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(default()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        for (index, draw) in draws.iter().enumerate() {
            let material = &self.materials[draw.material];
            pass.set_pipeline(&self.pipelines[material.pipeline]);
            pass.set_bind_group(
                0,
                &material.binding.as_ref().unwrap().1,
                &[(index * self.stride) as u32],
            );
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }
    pub fn record_resolve(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        source: &TextureView,
        colour: &TextureView,
    ) {
        let key = [source.id(), colour.id()];
        if self.resolve_binding.as_ref().is_none_or(|v| v.0 != key) {
            let layout = self.resolve.get_bind_group_layout(0).into();
            let bindings = [0, 1].map(|index| {
                let image = [source, colour][index];
                device.create_bind_group(
                    Some("motion distortion resolve inputs"),
                    &layout,
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: self.resolve_uniform[index].as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::TextureView(&self.vector_view),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::TextureView(image),
                        },
                        BindGroupEntry {
                            binding: 3,
                            resource: BindingResource::Sampler(&self.clamp),
                        },
                        BindGroupEntry {
                            binding: 4,
                            resource: BindingResource::Sampler(&self.mirror),
                        },
                    ],
                )
            });
            self.resolve_binding = Some((key, bindings));
        }
        // 327270 calls 326930 with scales 1 then 4 and swapped scene targets.
        for (index, target) in [colour, source].into_iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("motion distortion colour resolve"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &self.resolve_binding.as_ref().unwrap().1[index], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
