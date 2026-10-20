use super::*;
use bevy::{
    prelude::*,
    render::{render_resource::*, RenderApp},
};

#[test]
#[ignore = "requires GPU; serial/parallel directional cache refresh, restore, empty and replacement parity"]
fn cached_directional_slices_preserve_depth_stencil_and_generations_gpu() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
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
    let device = world.resource::<RenderDevice>().clone();
    let queue = world.resource::<RenderQueue>().clone();
    let adapter = world.resource::<RenderAdapter>();
    let probe_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("directional depth parity readback"),
        source: ShaderSource::Wgsl(r#"
            @group(0) @binding(0) var atlas:texture_depth_2d_array;
            @group(0) @binding(1) var<storage,read_write> pixels:array<u32>;
            @compute @workgroup_size(8,8,1) fn main(@builtin(global_invocation_id) p:vec3<u32>) {
                pixels[p.z*64u+p.y*8u+p.x]=bitcast<u32>(textureLoad(atlas,vec2<i32>(p.xy),i32(p.z),0));
            }
        "#.into()),
    });
    let probe = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &probe_shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 1024,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 18432,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(VEGETATION_POSITION, vec![[0f32; 3]; 6]);
    let mut layouts = bevy::mesh::MeshVertexBufferLayouts::default();
    let layout = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
    let indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[0u32, 1, 2, 3, 4, 5]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::INDEX,
    });
    let constants = shadow_instance_cb(
        &[
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ],
        1.,
    );
    let ring = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &constants,
        usage: BufferUsages::UNIFORM,
    });
    for depth_32_bits in [false, true] {
        let format = parallel_target_format(depth_32_bits);
        let dimensions = ParallelShadowTargetDimensions {
            width: 8,
            height: 8,
            layers: 4,
            depth_32_bits,
        };
        let make_target = || {
            let array = ShadowDepthArray::new_with_format(
                &device,
                Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 4,
                },
                format,
                "directional test atlas",
            )
            .unwrap();
            CachedTarget::new(&device, dimensions, array).unwrap()
        };
        let mut targets = [make_target(), make_target()];
        let mut gpu = ShadowSceneGpu::default();
        let mut draw_state = [0; 9];
        crate::render_state::reset_render_state(&mut draw_state);
        draw_state[5] = (draw_state[5] & !7) | 1;
        let key = PipelineKey {
            layout: hash_layout(&layout),
            depth_32_bits,
            front_face: FrontFace::Ccw,
            cull_mode: None,
            program: ShadowProgram::RigidOpaque,
            alpha_uv: None,
            draw_state,
            stencil_reference: 0,
        };
        let pipeline = ensure_pipeline(&mut gpu, key, layout.clone(), &device, adapter).unwrap();
        ensure_uniform_binding(gpu.pipelines.by_id_mut(pipeline), &ring, &device).unwrap();
        let mut finalization = bevy::render::renderer::PendingCommandBuffers::default();
        finalization.set_parallel_finalization(true);
        finalization.set_finalization_threads(3);
        let mut initial_static = None;
        for stage in 0..11 {
            if stage == 8 {
                for target in &mut targets {
                    target.cache_valid.fill(false);
                }
            }
            if stage == 9 {
                targets = [make_target(), make_target()];
            }
            let static_count = usize::from(stage < 6 || stage >= 8);
            let cache = match stage {
                0 | 1 => ShadowCacheOp::Retained {
                    generation: 1,
                    static_instances: 1,
                },
                2 | 5 => ShadowCacheOp::Retained {
                    generation: 2,
                    static_instances: 1,
                },
                3 => ShadowCacheOp::Restore,
                4 => ShadowCacheOp::Store {
                    static_instances: 1,
                },
                6 | 7 => ShadowCacheOp::Retained {
                    generation: 3,
                    static_instances: 0,
                },
                _ => ShadowCacheOp::Retained {
                    generation: 3,
                    static_instances: 1,
                },
            };
            let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &[
                    [-0.9f32, -0.9, 0.2 + stage as f32 * 0.04],
                    [-0.1, -0.9, 0.2 + stage as f32 * 0.04],
                    [-0.9, 0.9, 0.2 + stage as f32 * 0.04],
                    [0.1, -0.9, 0.4 + stage as f32 * 0.02],
                    [0.9, -0.9, 0.4 + stage as f32 * 0.02],
                    [0.1, 0.9, 0.4 + stage as f32 * 0.02],
                ]
                .into_iter()
                .flatten()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
                usage: BufferUsages::VERTEX,
            });
            let allocation = Arc::new(ShadowMeshAllocation {
                layout: layout.clone(),
                layout_key: key.layout,
                vertex: vertices,
                base_vertex: 0,
                index: indices.clone(),
                indices: 0..6,
                index_format: IndexFormat::Uint32,
            });
            let mut records = Vec::new();
            let layers: Vec<_> = (0..4)
                .map(|layer| {
                    let skip = stage == 10;
                    let cached = layer == 1 || layer == 2;
                    if !skip && stage != 7 {
                        let include_static = !cached || (stage != 3 && static_count != 0);
                        for draw in usize::from(!include_static)..2 {
                            let mut geometry = ShadowMeshAllocation {
                                layout: allocation.layout.clone(),
                                layout_key: allocation.layout_key,
                                vertex: allocation.vertex.clone(),
                                base_vertex: 0,
                                index: allocation.index.clone(),
                                indices: draw as u32 * 3..draw as u32 * 3 + 3,
                                index_format: IndexFormat::Uint32,
                            };
                            // Keep the cached test geometry source independent of its saved GPU surface.
                            geometry.base_vertex = 0;
                            records.push(PreparedShadowRecord {
                                layer,
                                scissor: ShadowScissor {
                                    x: 0,
                                    y: 0,
                                    width: 8,
                                    height: 8,
                                },
                                pipeline,
                                uniform_offset: 0,
                                instances: 0..1,
                                allocation: Arc::new(geometry),
                                resources: None,
                            });
                        }
                    }
                    ShadowLayer {
                        cache: if cached && !skip {
                            cache
                        } else {
                            ShadowCacheOp::None
                        },
                        ops: ShadowLayerOps {
                            depth: if skip {
                                ShadowLoad::Load
                            } else {
                                ShadowLoad::Clear(1.)
                            },
                            stencil: if skip {
                                StencilLoad::Load
                            } else {
                                StencilLoad::Clear(50 + stage * 10 + layer as u32)
                            },
                        },
                        instances: Vec::new(),
                    }
                })
                .collect();
            for (mode, target) in targets.iter_mut().enumerate() {
                target.validate_cache_sources(&layers).unwrap();
                let mut commands = Vec::new();
                if mode == 0 {
                    let mut encoder = device.create_command_encoder(&default());
                    for (layer_index, layer) in layers.iter().enumerate() {
                        let publication = encode_layer(
                            &mut encoder,
                            target,
                            layer_index,
                            layer_index,
                            layer,
                            &records,
                            &gpu.pipelines,
                            None,
                        )
                        .unwrap();
                        assert!(publication.complete);
                        publication.publish(target, layer_index, layer_index);
                    }
                    commands.push(encoder.finish());
                } else {
                    for (layer_index, result) in
                        record_layers(&device, target, 0, &layers, &records, &gpu.pipelines, None)
                    {
                        let (publication, encoder) = result.unwrap();
                        assert!(publication.complete);
                        if let Some(encoder) = encoder {
                            finalization.push_encoder(encoder);
                        }
                        publication.publish(target, layer_index, layer_index);
                    }
                    commands.extend(finalization.take());
                }
                queue.submit(commands);
            }
            assert_eq!(targets[0].cache_valid, targets[1].cache_valid);
            assert_eq!(targets[0].cache_generation, targets[1].cache_generation);
            assert_eq!(targets[0].layer_valid, targets[1].layer_valid);
            let generation = [
                Some(1),
                Some(1),
                Some(2),
                Some(2),
                None,
                Some(2),
                Some(3),
                Some(3),
                Some(3),
                Some(3),
                Some(3),
            ][stage as usize];
            for target in &targets {
                assert_eq!(target.cache_valid, [true, true]);
                assert_eq!(target.cache_generation, [generation, generation]);
                assert!(target.layer_valid.iter().all(|&valid| valid));
            }

            let mut encoder = device.create_command_encoder(&default());
            for (mode, target) in targets.iter().enumerate() {
                let binding = device
                    .wgpu_device()
                    .create_bind_group(&BindGroupDescriptor {
                        label: None,
                        layout: &probe.get_bind_group_layout(0),
                        entries: &[
                            BindGroupEntry {
                                binding: 0,
                                resource: BindingResource::TextureView(&target.array.sampled),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: output.as_entire_binding(),
                            },
                        ],
                    });
                {
                    let mut pass = encoder.begin_compute_pass(&default());
                    pass.set_pipeline(&probe);
                    pass.set_bind_group(0, &binding, &[]);
                    pass.dispatch_workgroups(1, 1, 4);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, mode as u64 * 1024, 1024);
                if format.has_stencil_aspect() {
                    encoder.copy_texture_to_buffer(
                        TexelCopyTextureInfo {
                            texture: &target.array.texture,
                            mip_level: 0,
                            origin: Origin3d::ZERO,
                            aspect: TextureAspect::StencilOnly,
                        },
                        TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: TexelCopyBufferLayout {
                                offset: 2048 + mode as u64 * 8192,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(8),
                            },
                        },
                        target.array.size,
                    );
                }
            }
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            device.poll(PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range();
            assert_eq!(
                &bytes[..1024],
                &bytes[1024..2048],
                "depth stage={stage} format={format:?}"
            );
            if format.has_stencil_aspect() {
                assert_eq!(
                    &bytes[2048..10240],
                    &bytes[10240..18432],
                    "stencil stage={stage}"
                );
                if stage < 3 {
                    assert_eq!(bytes[2048 + 2048], if stage < 2 { 51 } else { 71 });
                }
            }
            let values: Vec<_> = bytes[..1024]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            if stage == 0 {
                assert!(
                    values.iter().filter(|&&v| v < 1.).count() > 16,
                    "fixture must draw real geometry"
                );
                initial_static = Some(values[64 + 4 * 8 + 1].to_bits());
                assert!(values[64 + 4 * 8 + 1] < 1.);
                assert!(
                    values[64 + 4 * 8 + 5] < 1.,
                    "fixture must draw dynamic geometry too"
                );
            }
            if stage == 1 {
                assert_eq!(
                    Some(values[64 + 4 * 8 + 1].to_bits()),
                    initial_static,
                    "retained generation must ignore changed static input"
                );
            }
            if stage == 2 {
                assert_ne!(
                    Some(values[64 + 4 * 8 + 1].to_bits()),
                    initial_static,
                    "new generation must refresh static depth"
                );
            }
            if stage == 8 || stage == 9 {
                assert!(
                    values[64 + 4 * 8 + 1] < 1.,
                    "invalidated or replaced cache must materialize static geometry"
                );
            }
            if stage == 6 {
                assert_eq!(
                    values[64 + 4 * 8 + 1],
                    1.,
                    "empty static refresh must clear preceding static geometry"
                );
                assert!(
                    values[64 + 4 * 8 + 5] < 1.,
                    "dynamic geometry must follow the empty static-cache store"
                );
            }
            if stage == 7 {
                assert!(
                    values.iter().all(|&v| v == 1.),
                    "empty retained cache must remain clear"
                );
            }
            drop(bytes);
            readback.unmap();
        }
    }
}
