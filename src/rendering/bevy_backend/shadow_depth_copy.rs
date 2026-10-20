//! Native32db10's cropped/rescaled depthCopy draw. WGPU texture copies cannot
//! replace this path: the native shader transports an authored source region
//! to the current destination viewport and can merge with LESS_EQUAL depth.
use crate::shadow_pipeline::{depth_stencil_gpu, ShadowDepthArray};
use bevy::render::{render_resource::*, renderer::RenderDevice};

pub struct ShadowDepthCopy {
    pipeline: RenderPipeline,
    sampler: Sampler,
    format: TextureFormat,
}

/// Native32db10 overrides, retaining the actual depthCopy RPD's other bits.
pub fn copy_draw_state(
    mut source: crate::render_state::RenderState,
    merge: bool,
) -> crate::render_state::RenderState {
    source[0] &= 0xfe1f_ffff;
    source[5] = (source[5] & 0xfcff_fff8) | (if merge { 3 } else { 7 }) | 8;
    source
}

impl ShadowDepthCopy {
    pub fn new(
        device: &RenderDevice,
        format: TextureFormat,
        source: crate::render_state::RenderState,
        merge: bool,
    ) -> Result<Self, String> {
        let state = copy_draw_state(source, merge);
        let raster = crate::render_state::raster_state(&state);
        if raster.cull_mode != 1
            || raster.fill_mode != 3
            || raster.antialiased_line
            || raster.multisample
            || !raster.depth_clip
        {
            return Err("depthCopy raster state requires unsupported nonstandard fullscreen behavior".into());
        }
        let depth = depth_stencil_gpu(&state, format)?;
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("depthCopy cropped array draw"),
            source: ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/shaders/dh2_shadow_depth_copy.wgsl"
                ))
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("depthCopy viewport transfer"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: PrimitiveState {
                front_face: if raster.front_counter_clockwise {
                    FrontFace::Ccw
                } else {
                    FrontFace::Cw
                },
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth),
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("depthCopy POINT sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_max_clamp: 13.0,
            ..Default::default()
        });
        Ok(Self {
            pipeline,
            sampler,
            format,
        })
    }

    pub fn copy(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        source: &ShadowDepthArray,
        source_layer: u32,
        source_rect: [i32; 4],
        destination: &ShadowDepthArray,
        destination_layer: u32,
        destination_rect: [i32; 4],
    ) -> Result<(), String> {
        let validate = |target: &ShadowDepthArray, layer, rect: [i32; 4]| {
            layer < target.size.depth_or_array_layers
                && rect[0] >= 0
                && rect[1] >= 0
                && rect[2] >= rect[0]
                && rect[3] >= rect[1]
                && rect[2] < (target.size.width as i32)
                && rect[3] < (target.size.height as i32)
        };
        if source.texture.id() == destination.texture.id()
            || destination.texture.format() != self.format
            || !validate(source, source_layer, source_rect)
            || !validate(destination, destination_layer, destination_rect)
        {
            return Err(
                "depthCopy source/destination region is invalid or aliases its attachment"
                    .into(),
            );
        }
        let mut bytes = Vec::with_capacity(32);
        for value in [
            (source_rect[2] - source_rect[0] + 1) as f32 / source.size.width as f32,
            (source_rect[3] - source_rect[1] + 1) as f32 / source.size.height as f32,
            source_rect[0] as f32 / source.size.width as f32,
            source_rect[1] as f32 / source.size.height as f32,
        ] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(source_layer.to_le_bytes());
        bytes.extend([0u8; 12]);
        let constants = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("depthCopy source crop/slice"),
            contents: &bytes,
            usage: BufferUsages::UNIFORM,
        });
        let group = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: constants.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&source.sampled),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
        let has_stencil = self.format.has_stencil_aspect();
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("depthCopy rectangle"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &destination.layers[destination_layer as usize],
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: has_stencil.then_some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
            }),
            ..Default::default()
        });
        let [x, y, x1, y1] = destination_rect;
        let (width, height) = ((x1 - x + 1) as u32, (y1 - y + 1) as u32);
        pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
        pass.set_scissor_rect(x as u32, y as u32, width, height);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_stencil_reference(0);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        prelude::*,
        render::{renderer::RenderQueue, RenderApp},
    };
    #[test]
    #[ignore = "requires GPU; verifies native cropped POINT depth copies, array slices and LESS_EQUAL merging"]
    fn depth_copy_preserves_crops_slices_and_merge_gpu() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.finish();
        app.cleanup();
        let world = app.sub_app_mut(RenderApp).world_mut();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        for format in [
            TextureFormat::Depth16Unorm,
            TextureFormat::Depth24PlusStencil8,
        ] {
            let source = ShadowDepthArray::new_with_format(
                device,
                Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 2,
                },
                format,
                "copy source pattern",
            )
            .unwrap();
            let target = ShadowDepthArray::new_with_format(
                device,
                Extent3d {
                    width: 10,
                    height: 8,
                    depth_or_array_layers: 2,
                },
                format,
                "copy destination regions",
            )
            .unwrap();
            let mut state = [0; 9];
            crate::render_state::reset_render_state(&mut state);
            let replace = ShadowDepthCopy::new(device, format, state, false).unwrap();
            let merge = ShadowDepthCopy::new(device, format, state, true).unwrap();
            let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:None,source:ShaderSource::Wgsl(r#"
                @vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{
                    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4<f32>(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
                }
                @fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32{return floor(p.x)/16.0+floor(p.y)/32.0;}
            "#.into())});
            let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: default(),
                    targets: &[],
                }),
                primitive: default(),
                depth_stencil: Some(DepthStencilState {
                    format,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::Always),
                    stencil: default(),
                    bias: default(),
                }),
                multisample: default(),
                multiview_mask: None,
                cache: None,
            });
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            for (array, clear, draw_pattern) in [(&source, 0.75, true), (&target, 0.5, false)] {
                for layer in 0..2 {
                    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: None,
                        color_attachments: &[],
                        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                            view: &array.layers[layer],
                            depth_ops: Some(Operations {
                                load: LoadOp::Clear(clear),
                                store: StoreOp::Store,
                            }),
                            stencil_ops: format.has_stencil_aspect().then_some(Operations {
                                load: LoadOp::Clear(0),
                                store: StoreOp::Store,
                            }),
                        }),
                        ..default()
                    });
                    if draw_pattern && layer == 0 {
                        pass.set_pipeline(&pipeline);
                        pass.draw(0..3, 0..1);
                    }
                }
            }
            replace
                .copy(
                    device,
                    &mut encoder,
                    &source,
                    0,
                    [1, 2, 6, 6],
                    &target,
                    0,
                    [2, 1, 7, 4],
                )
                .unwrap();
            merge
                .copy(
                    device,
                    &mut encoder,
                    &source,
                    0,
                    [0, 1, 7, 7],
                    &target,
                    1,
                    [1, 2, 8, 6],
                )
                .unwrap();
            replace
                .copy(
                    device,
                    &mut encoder,
                    &source,
                    1,
                    [0, 0, 7, 7],
                    &target,
                    0,
                    [0, 6, 9, 7],
                )
                .unwrap();
            assert!(replace
                .copy(
                    device,
                    &mut encoder,
                    &target,
                    0,
                    [0, 0, 2, 2],
                    &target,
                    1,
                    [0, 0, 2, 2]
                )
                .is_err());
            let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor{label:None,source:ShaderSource::Wgsl(r#"
                @group(0) @binding(0) var depths:texture_depth_2d_array;
                @group(0) @binding(1) var<storage,read_write> pixels:array<f32>;
                @compute @workgroup_size(1) fn read(@builtin(global_invocation_id) i:vec3<u32>){pixels[i.x]=textureLoad(depths,vec2<i32>(i32(i.x%10u),i32((i.x/10u)%8u)),i32(i.x/80u),0);}
            "#.into())});
            let pipeline =
                device
                    .wgpu_device()
                    .create_compute_pipeline(&RawComputePipelineDescriptor {
                        label: None,
                        layout: None,
                        module: &shader,
                        entry_point: Some("read"),
                        compilation_options: default(),
                        cache: None,
                    });
            let output = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 640,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 640,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(&target.sampled),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: output.as_entire_binding(),
                        },
                    ],
                });
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(160, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 640);
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(MapMode::Read, move |r| sender.send(r).unwrap());
            device
                .wgpu_device()
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            receiver.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range();
            let quantization = if format == TextureFormat::Depth16Unorm {
                1.0 / 65535.0
            } else {
                1.0 / 16777215.0
            };
            for (i, bytes) in mapped.chunks_exact(4).enumerate() {
                let actual = f32::from_le_bytes(bytes.try_into().unwrap());
                let (x, y, layer) = (i % 10, (i / 10) % 8, i / 80);
                let mut expected = 0.5;
                if layer == 0 && (2..=7).contains(&x) && (1..=4).contains(&y) {
                    let source_x = x - 2 + 1;
                    let source_y = (((y - 1) as f32 + 0.5) * 5.0 / 4.0).floor() as usize + 2;
                    expected = source_x as f32 / 16.0 + source_y as f32 / 32.0;
                }
                if layer == 1 && (1..=8).contains(&x) && (2..=6).contains(&y) {
                    let source_x = x - 1;
                    let source_y = (((y - 2) as f32 + 0.5) * 7.0 / 5.0).floor() as usize + 1;
                    expected = 0.5f32.min(source_x as f32 / 16.0 + source_y as f32 / 32.0);
                }
                if layer == 0 && y >= 6 {
                    expected = 0.75;
                }
                assert!(
                    (actual - expected).abs() <= quantization * 1.01,
                    "depthCopy {format:?} layer{layer} ({x},{y}): {actual} vs {expected}"
                );
            }
            drop(mapped);
            readback.unmap();
        }
    }
}
