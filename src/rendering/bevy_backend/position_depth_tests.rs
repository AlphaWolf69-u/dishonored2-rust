//! Exercise the real scene allocator, indexed/nonindexed mesh slices, changing
//! view/model matrices, reverse depth and the water pass's masked stencil write.
use super::*;
use bevy::{asset::RenderAssetUsages, mesh::Indices, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; submits scene water depth without a window"]
fn water_depth_submits_shared_scene_meshes() {
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
    let make = |indexed| {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-1f32, -1., 0.], [3., -1., 0.], [-1., 3., 0.]],
        );
        if indexed {
            mesh.insert_indices(Indices::U32(vec![0, 1, 2]));
        }
        mesh
    };
    let indexed = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(make(true));
    let plain = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(make(false));
    for _ in 0..8 {
        app.update();
    }
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let allocator = world.resource::<MeshAllocator>();
    let meshes = world.resource::<RenderAssets<RenderMesh>>();
    assert!(meshes.get(&indexed).is_some() && meshes.get(&plain).is_some());
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |e| {
            capture.lock().unwrap().push(format!("{e}"));
        }));
    let format = TextureFormat::Depth24PlusStencil8;
    let target = device.create_texture(&TextureDescriptor {
        label: Some("water depth mesh test"),
        size: Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let attachment = target.create_view(&default());
    let sampled = target.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    });
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("water depth numeric readback"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var source:texture_depth_2d;
@group(0) @binding(1) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(1) fn read(@builtin(global_invocation_id) i:vec3<u32>) {
    output[i.y*8u+i.x]=textureLoad(source,vec2<i32>(i.xy),0);
}
"#
            .into(),
        ),
    });
    let read = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("read"),
        compilation_options: default(),
        cache: None,
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 2560,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(
        None,
        &read.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&sampled),
            },
            BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let mut water =
        PositionDepthPass::new(device, format, crate::render_state::water_depth_state(), 4)
            .unwrap();
    let mut bindings = None;
    for (case, (left, view_z)) in [(0.25, 0.), (0.625, 0.), (0.625, -0.5)]
        .into_iter()
        .enumerate()
    {
        water
            .prepare(
                device,
                queue,
                Mat4::from_translation(Vec3::new(0., 0., view_z)),
                [
                    PositionInstance {
                        mesh: indexed.id(),
                        world_from_local: Mat4::from_translation(Vec3::new(0., 0., left)),
                    },
                    PositionInstance {
                        mesh: plain.id(),
                        world_from_local: Mat4::from_scale_rotation_translation(
                            Vec3::new(0.5, 1., 1.),
                            Quat::IDENTITY,
                            Vec3::new(0.5, 0., 0.75),
                        ),
                    },
                ],
                allocator,
                meshes,
            )
            .unwrap();
        let ids = (
            water.binding.as_ref().unwrap().1.id(),
            water.meshes.draws()[0].models.id(),
        );
        if let Some(prior) = bindings {
            assert_eq!(ids, prior, "steady water draw replaced persistent bindings");
        }
        bindings = Some(ids);
        let mut encoder = device.create_command_encoder(&default());
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &attachment,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.5),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(0xa0),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        water.record(&mut encoder, &attachment).unwrap();
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&read);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(8, 8, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 256);
        let mut copy = target.as_image_copy();
        copy.aspect = TextureAspect::StencilOnly;
        encoder.copy_texture_to_buffer(
            copy,
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 512,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for y in 0..8 {
            for x in 0..8 {
                let offset = (y * 8 + x) * 4;
                let depth = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let expected = if case == 2 {
                    0.5
                } else if x >= 4 {
                    0.75
                } else {
                    left.max(0.5)
                };
                assert!(
                    (depth - expected).abs() < 0.000001,
                    "water case{case} pixel{x}/{y}: {depth} != {expected}"
                );
                assert_eq!(
                    bytes[512 + y * 256 + x],
                    if expected > 0.5 { 0xa4 } else { 0xa0 },
                    "water stencil case{case} pixel{x}/{y}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
    }
    assert!(
        errors.lock().unwrap().is_empty(),
        "position depth GPU validation: {:?}",
        errors.lock().unwrap()
    );
}
