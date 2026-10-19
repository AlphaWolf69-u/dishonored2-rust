use super::*;
use bevy::{
    ecs::system::RunSystemOnce,
    prelude::*,
    render::{render_resource::*, renderer::PendingCommandBuffers},
};

fn gpu_image(array: &NativeShadowDepthArray, sampler: Sampler) -> GpuImage {
    GpuImage {
        texture: array.texture.clone(),
        texture_view: array.sampled.clone(),
        sampler,
        texture_descriptor: TextureDescriptor {
            label: None,
            size: array.size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: array.texture.format(),
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        texture_view_descriptor: None,
        had_data: false,
    }
}

#[test]
#[ignore = "requires GPU; retained atlas versus fresh copies across owner and allocation changes"]
fn retained_local_atlas_matches_fresh_submissions_gpu() {
    struct RestoreEnvironment(Option<std::ffi::OsString>);
    impl Drop for RestoreEnvironment {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("DH2_REUSE_LOCAL_SHADOW_ATLAS", value),
                None => std::env::remove_var("DH2_REUSE_LOCAL_SHADOW_ATLAS"),
            }
        }
    }
    let _restore = RestoreEnvironment(std::env::var_os("DH2_REUSE_LOCAL_SHADOW_ATLAS"));
    std::env::set_var("DH2_REUSE_LOCAL_SHADOW_ATLAS", "1");
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
    let handles: Vec<_> = (0..4)
        .map(|_| {
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::default())
        })
        .collect();
    let world = app.sub_app_mut(RenderApp).world_mut();
    world.init_resource::<NativeLocalShadowCompletion>();
    // GPU resources normally initialize in the render schedule; this fixture
    // invokes the owner systems directly without running that schedule.
    world.init_resource::<bevy::render::mesh::allocator::MeshAllocatorSettings>();
    world.init_resource::<MeshAllocator>();
    world.init_resource::<RenderAssets<RenderMesh>>();
    world.init_resource::<RenderAssets<GpuShaderBuffer>>();
    world.insert_resource(bevy::pbr::GlobalClusterableObjectMeta::new(
        BufferBindingType::Storage { read_only: true },
    ));
    world.init_resource::<PendingCommandBuffers>();

    let device = world.resource::<RenderDevice>().clone();
    let queue = world.resource::<RenderQueue>().clone();
    let sampler = crate::shadow_pipeline::native_shadow_comparison_sampler(&device);
    let probe_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("atlas numerical readback"),
        source: ShaderSource::Wgsl(
            r#"
        @group(0) @binding(0) var atlas:texture_depth_2d_array;
        @group(0) @binding(1) var<storage,read_write> pixels:array<u32>;
        @compute @workgroup_size(8,8) fn main(@builtin(global_invocation_id) p:vec3<u32>) {
            pixels[p.y*8u+p.x]=bitcast<u32>(textureLoad(atlas,vec2<i32>(p.xy),0,0));
        }
    "#
            .into(),
        ),
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
        size: 256,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 512 + 4096,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for format in [
        TextureFormat::Depth16Unorm,
        TextureFormat::Depth24PlusStencil8,
    ] {
        let size = Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        };
        let mut atlases: Vec<_> = (0..2)
            .map(|_| {
                NativeShadowDepthArray::new_with_format(&device, size, format, "test atlas")
                    .unwrap()
            })
            .collect();
        let caches: Vec<_> = (0..2)
            .map(|_| {
                NativeShadowDepthArray::new_with_format(&device, size, format, "test cache")
                    .unwrap()
            })
            .collect();
        let mut owners: Vec<_> = (0..2)
            .map(|i| {
                let mut owner = LocalGpu::default();
                owner.arrays = Some((
                    NativeShadowDepthArray::from_owned_texture(
                        atlases[i].texture.clone(),
                        "owner atlas",
                    )
                    .unwrap(),
                    NativeShadowDepthArray::from_owned_texture(
                        caches[i].texture.clone(),
                        "owner cache",
                    )
                    .unwrap(),
                ));
                owner.valid.insert(1, false);
                owner
            })
            .collect();
        // Seed actual cache pixels, so a skipped/incorrect copy cannot pass by
        // comparing two blank textures. Atlas background must remain at 1.0.
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        for cache in &caches {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &cache.layers[0],
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.25),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: format.has_stencil_aspect().then_some(Operations {
                        load: LoadOp::Clear(7),
                        store: StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        queue.submit([encoder.finish()]);
        for i in 0..2 {
            let mut images = world.resource_mut::<RenderAssets<GpuImage>>();
            images.insert(handles[i * 2].id(), gpu_image(&atlases[i], sampler.clone()));
            images.insert(
                handles[i * 2 + 1].id(),
                gpu_image(&caches[i], sampler.clone()),
            );
        }
        let mut copy_state = [0; 9];
        crate::render_state::reset_native_render_state(&mut copy_state);
        let mut reuse_frames = 0;
        for stage in 0..12 {
            let rect = if stage < 2 {
                [2, 2, 5, 5]
            } else {
                [0, 0, 3, 3]
            };
            if stage == 5 || stage == 10 {
                for i in 0..2 {
                    atlases[i] = NativeShadowDepthArray::new_with_format(
                        &device,
                        size,
                        format,
                        "replacement atlas",
                    )
                    .unwrap();
                    world
                        .resource_mut::<RenderAssets<GpuImage>>()
                        .insert(handles[i * 2].id(), gpu_image(&atlases[i], sampler.clone()));
                }
            }
            for i in 0..2 {
                let mut owner = std::mem::take(&mut owners[i]);
                // Reference always executes the original fresh clear/copy path.
                if i == 0 {
                    owner.retained_atlas = None;
                }
                if stage == 3 {
                    owner.valid.clear();
                }
                world.insert_resource(owner);
                world.insert_resource(NativeLocalShadowSceneFrame(Some(
                    NativeLocalShadowSubmission {
                        revision: stage + 1,
                        atlas: handles[i * 2].clone(),
                        cache: handles[i * 2 + 1].clone(),
                        atlas_extent: [8, 8, 1],
                        cache_extent: [8, 8, 1],
                        depth_32_bits: format.has_stencil_aspect(),
                        copy_state,
                        layers: vec![NativeShadowLayer {
                            cache: NativeShadowCacheOp::None,
                            ops: NativeShadowLayerOps {
                                depth: NativeShadowLoad::Clear(1.),
                                stencil: NativeStencilLoad::Clear(0),
                            },
                            instances: Vec::new(),
                        }],
                        regions: vec![NativeLocalShadowRegion {
                            atlas: AtlasAllocation {
                                id: 2,
                                slice: 0,
                                rect,
                            },
                            cache: Some(AtlasAllocation {
                                id: 1,
                                slice: 0,
                                rect: [0, 0, 3, 3],
                            }),
                            static_instances: 0,
                            contains_models: false,
                            static_prepared: true,
                        }],
                        receiver: Ok(NativeLocalShadowReceiver {
                            bias: [0.; 3],
                            lights: Vec::new(),
                            shadows: Vec::new(),
                        }),
                    },
                )));
                if stage >= 8 {
                    let mut frame = world.resource_mut::<NativeLocalShadowSceneFrame>();
                    let frame = frame.0.as_mut().unwrap();
                    frame.layers.clear();
                    frame.regions.clear();
                }
                world.run_system_once(prepare).unwrap();
                world.run_system_once(draw).unwrap();
                let pending = world.resource_mut::<PendingCommandBuffers>().take();
                let reused = i == 1 && matches!(stage, 1 | 7 | 9 | 11);
                assert_eq!(pending.is_empty(), reused, "stage={stage} owner={i}");
                queue.submit(pending);
                let done = world
                    .resource::<NativeLocalShadowCompletion>()
                    .0
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap();
                assert_eq!(
                    done.0,
                    stage + 1,
                    "reuse must publish the current completion revision"
                );
                owners[i] = world.remove_resource::<LocalGpu>().unwrap();
                if reused {
                    reuse_frames += 1;
                }
                if i == 1 {
                    assert_eq!(owners[i].atlas_reused, reuse_frames);
                }
            }
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            for i in 0..2 {
                let binding = device
                    .wgpu_device()
                    .create_bind_group(&BindGroupDescriptor {
                        label: None,
                        layout: &probe.get_bind_group_layout(0),
                        entries: &[
                            BindGroupEntry {
                                binding: 0,
                                resource: BindingResource::TextureView(&atlases[i].sampled),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: output.as_entire_binding(),
                            },
                        ],
                    });
                {
                    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
                    pass.set_pipeline(&probe);
                    pass.set_bind_group(0, &binding, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &readback, i as u64 * 256, 256);
                if format.has_stencil_aspect() {
                    encoder.copy_texture_to_buffer(
                        TexelCopyTextureInfo {
                            texture: &atlases[i].texture,
                            mip_level: 0,
                            origin: Origin3d::ZERO,
                            aspect: TextureAspect::StencilOnly,
                        },
                        TexelCopyBufferInfo {
                            buffer: &readback,
                            layout: TexelCopyBufferLayout {
                                offset: 512 + i as u64 * 2048,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(8),
                            },
                        },
                        size,
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
                &bytes[..256],
                &bytes[256..512],
                "depth differs stage={stage} format={format:?}"
            );
            if stage < 3 {
                let values: Vec<_> = bytes[..256]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                assert_eq!(
                    values.iter().filter(|&&v| v < 0.5).count(),
                    16,
                    "must copy actual cached depth, stage={stage}"
                );
                assert_eq!(
                    values.iter().filter(|&&v| v == 1.).count(),
                    48,
                    "must preserve clear background"
                );
            }
            if format.has_stencil_aspect() {
                for y in 0..8 {
                    assert_eq!(
                        &bytes[512 + y * 256..512 + y * 256 + 8],
                        &bytes[2560 + y * 256..2560 + y * 256 + 8]
                    );
                }
            }
            drop(bytes);
            readback.unmap();
        }
        assert_eq!(reuse_frames, 4);
    }
}
