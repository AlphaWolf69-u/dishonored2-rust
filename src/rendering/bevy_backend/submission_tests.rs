#![cfg(feature = "game")]
use bevy::prelude::*;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
    RenderApp,
};

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct ClusterFixtureExtension {}
impl bevy::pbr::MaterialExtension for ClusterFixtureExtension {
    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        let flag = bevy::shader::ShaderDefVal::UInt("DH2_PROJECT_FAR".into(), 0);
        descriptor.vertex.shader_defs.push(flag.clone());
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push(flag);
        }
        Ok(())
    }
}
type ClusterFixtureMaterial =
    bevy::pbr::ExtendedMaterial<StandardMaterial, ClusterFixtureExtension>;

#[test]
#[ignore = "requires a GPU; compares serial and chunked passes after light and cluster-grid changes"]
fn parallel_camera_chunks_preserve_lighting_after_light_and_grid_changes_gpu() {
    use bevy::{
        asset::RenderAssetUsages,
        camera::{Exposure, RenderTarget},
        core_pipeline::tonemapping::{DebandDither, Tonemapping},
        light::{
            cluster::{ClusterConfig, ClusterFarZMode, ClusterZConfig, GlobalClusterSettings},
            GlobalAmbientLight,
        },
        render::{
            render_asset::RenderAssets, texture::GpuImage, view::NoIndirectDrawing, RenderPlugin,
        },
    };
    let run = |gpu_clustering: bool, parallel: bool, serial_camera: bool| {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.add_plugins(bevy::pbr::MaterialPlugin::<ClusterFixtureMaterial>::default());
        app.finish();
        app.cleanup();
        // Conditional imports in the patched PBR shaders still need their
        // registered modules, just as in the game client.
        crate::shadow_sampling::register_shaders(&mut app);
        super::fog_material::register_shaders(&mut app);
        let _indirect = app
            .world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .add(bevy::shader::Shader::from_wgsl(
                super::probe_weights::indirect_source(include_str!("../../../assets/shaders/dh2_indirect.wgsl")),
                "cluster-fixture/dh2_indirect.wgsl",
            ));

        app.world_mut().insert_resource(GlobalAmbientLight {
            brightness: 0.0,
            ..default()
        });
        if !gpu_clustering {
            app.world_mut()
                .resource_mut::<GlobalClusterSettings>()
                .gpu_clustering = None;
        }
        let device = app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderDevice>()
            .clone();
        let queue = app
            .sub_app(RenderApp)
            .world()
            .resource::<RenderQueue>()
            .clone();
        app.world_mut()
            .resource_mut::<bevy::diagnostic::FrameCount>()
            .0 = 300;
        if parallel {
            app.sub_app_mut(RenderApp).add_systems(bevy::render::RenderStartup,
                (|mut pending: ResMut<bevy::render::renderer::PendingCommandBuffers>| {
                    pending.set_finalization_threads(1);
                }).after(bevy::render::init_gpu_resource::<bevy::render::renderer::PendingCommandBuffers>));
            app.sub_app_mut(RenderApp)
                .insert_resource(bevy::render::renderer::ParallelCommandFinalization);
            app.sub_app_mut(RenderApp)
                .insert_resource(bevy::render::render_phase::CameraDrawChunking::new(1));
        }
        if serial_camera {
            app.sub_app_mut(RenderApp)
                .edit_schedule(bevy::core_pipeline::Core3d, |schedule| {
                    schedule.set_executor(bevy::ecs::schedule::SingleThreadedExecutor::new());
                });
        }

        let mut image = Image::new_uninit(
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.usage |=
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(2.0, 2.0, 2.0));
        let material = app
            .world_mut()
            .resource_mut::<Assets<ClusterFixtureMaterial>>()
            .add(ClusterFixtureMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 1.0,
                    reflectance: 0.0,
                    ..default()
                },
                extension: ClusterFixtureExtension {},
            });
        app.world_mut()
            .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
        // Distinct geometry sharing material/buffers exercises indirect draws
        // with different mesh-uniform indices and transforms.
        let second_mesh = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::new(0.7, 0.9, 0.6));
        app.world_mut().spawn((
            Mesh3d(second_mesh),
            MeshMaterial3d(material),
            Transform::from_xyz(-1.3, 0.0, 0.1),
        ));
        // A second specialization exercises ordered batches spanning pipelines.
        let masked_material = app
            .world_mut()
            .resource_mut::<Assets<ClusterFixtureMaterial>>()
            .add(ClusterFixtureMaterial {
                base: StandardMaterial {
                    base_color: Color::srgba(0.6, 0.8, 1.0, 0.9),
                    alpha_mode: AlphaMode::Mask(0.5),
                    perceptual_roughness: 1.0,
                    reflectance: 0.0,
                    ..default()
                },
                extension: ClusterFixtureExtension {},
            });
        app.world_mut().spawn((
            Mesh3d(mesh),
            MeshMaterial3d(masked_material),
            Transform::from_xyz(1.2, -0.4, -0.4).with_scale(Vec3::splat(0.65)),
        ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                bevy::camera::Hdr,
                bevy::camera::ShadowLodOrigin,
                bevy::core_pipeline::prepass::DepthPrepass,
                Camera {
                    clear_color: Color::BLACK.into(),
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                Msaa::Off,
                Tonemapping::None,
                DebandDither::Disabled,
                Exposure { ev100: 5.0 },
                NoIndirectDrawing,
                Transform::from_xyz(0.0, 1.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id();
        let first = app
            .world_mut()
            .spawn((
                PointLight {
                    intensity: 5000.0,
                    range: 12.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(1.0, 3.0, 3.0),
                Visibility::Visible,
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                PointLight {
                    intensity: 3000.0,
                    range: 20.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(2.0, 1.0, 1.0),
                Visibility::Hidden,
            ))
            .id();
        let ids = |app: &mut App| {
            let world = app.sub_app_mut(RenderApp).world_mut();
            let mut query = world.query::<&bevy::pbr::ViewClusterBindings>();
            let bindings = query.single(world).unwrap();
            let first = match bindings.clusterable_object_index_lists_binding().unwrap() {
                BindingResource::Buffer(b) => b.buffer.clone(),
                _ => unreachable!(),
            };
            let second = match bindings.offsets_and_counts_binding().unwrap() {
                BindingResource::Buffer(b) => b.buffer.clone(),
                _ => unreachable!(),
            };
            (first, second, bindings.n_indices())
        };
        let read_pixels = |app: &App| {
            let world = app.sub_app(RenderApp).world();
            let texture = &world
                .resource::<RenderAssets<GpuImage>>()
                .get(image.id())
                .unwrap()
                .texture;
            let readback = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 4096,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(16),
                    },
                },
                texture.size(),
            );
            let submission = queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(MapMode::Read, move |result| send.send(result).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(20)),
                })
                .unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range();
            let pixels: Vec<u8> = mapped
                .chunks_exact(256)
                .flat_map(|row| row[..64].iter().copied())
                .collect();
            drop(mapped);
            readback.unmap();
            pixels
        };
        let mut outputs = Vec::new();
        for stage in 0..4 {
            let dimensions = match stage {
                0 | 3 => UVec3::new(2, 2, 4),
                1 => UVec3::ONE,
                _ => UVec3::new(4, 3, 8),
            };
            app.world_mut()
                .entity_mut(camera)
                .insert(ClusterConfig::XYZ {
                    dimensions,
                    z_config: ClusterZConfig {
                        first_slice_depth: 1.0,
                        far_z_mode: ClusterFarZMode::Constant(50.0),
                    },
                    dynamic_resizing: false,
                });
            *app.world_mut().get_mut::<Visibility>(first).unwrap() = if stage == 1 {
                Visibility::Hidden
            } else {
                Visibility::Visible
            };
            *app.world_mut().get_mut::<Visibility>(second).unwrap() = if stage == 2 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            app.world_mut()
                .get_mut::<Transform>(first)
                .unwrap()
                .translation = if stage == 2 {
                Vec3::new(-2.0, 3.0, 3.0)
            } else {
                Vec3::new(1.0, 3.0, 3.0)
            };
            for _ in 0..if stage == 0 { 140 } else { 20 } {
                app.update();
                device
                    .poll(PollType::Wait {
                        submission_index: None,
                        timeout: Some(std::time::Duration::from_secs(20)),
                    })
                    .unwrap();
            }
            {
                let world = app.world_mut();
                let mut q = world.query_filtered::<&ViewVisibility, With<Mesh3d>>();
                let visible = q.iter(world).filter(|v| v.get()).count();
                let render = app.sub_app(RenderApp).world();
                let pipelines = render.resource::<PipelineCache>();
                for pipeline in pipelines.pipelines() {
                    if let CachedPipelineState::Err(error) = &pipeline.state {
                        panic!("clustering fixture pipeline failed: {error:?}");
                    }
                }
                assert_eq!(
                    pipelines.waiting_pipelines().count(),
                    0,
                    "fixture shaders must be ready before comparing pixels"
                );

                eprintln!(
                    "CLUSTER_STAGE gpu={gpu_clustering} stage={stage} visible={visible} pipelines={} waiting={}",
                    pipelines.pipelines().count(),
                    pipelines.waiting_pipelines().count()
                );
            }
            let before = ids(&mut app);
            app.update();
            let after = ids(&mut app);

            assert_eq!(
                before.2, after.2,
                "cluster index counts must reset each frame"
            );
            assert_ne!(
                before.0, after.0,
                "both paths allocate ordinary cluster buffers"
            );
            if parallel {
                assert!(
                    app.sub_app(RenderApp)
                        .world()
                        .resource::<bevy::render::renderer::PendingCommandBuffers>()
                        .parallel_finalized_encoders()
                        > 0,
                    "fixture must finalize real camera encoders in parallel"
                );
                assert!(
                    app.sub_app(RenderApp)
                        .world()
                        .resource::<bevy::render::render_phase::CameraDrawChunking>()
                        .recorded_chunks()
                        > 0,
                    "fixture must execute the chunked camera path"
                );
            }
            outputs.push(read_pixels(&app));
        }
        eprintln!(
            "CLUSTER_FIXTURE gpu={gpu_clustering} maxima={:?} alpha={:?}",
            outputs
                .iter()
                .map(|pixels| pixels
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter())
                    .copied()
                    .max())
                .collect::<Vec<_>>(),
            outputs.iter().map(|pixels| pixels[3]).collect::<Vec<_>>()
        );
        assert!(
            outputs[0]
                .chunks_exact(4)
                .any(|p| p[..3].iter().any(|&v| v > 8)),
            "test geometry must be lit"
        );
        assert!(
            outputs[1].chunks_exact(4).all(|p| p[..3] == [0, 0, 0]),
            "removed lights must leave no stale illumination"
        );
        assert_ne!(
            outputs[0], outputs[2],
            "changed lights must affect the output"
        );
        assert_eq!(
            outputs[0], outputs[3],
            "restoring lights and grid must restore pixels"
        );
        outputs
    };
    for gpu_clustering in [false, true] {
        let direct = run(gpu_clustering, false, false);
        assert_eq!(
            direct,
            run(gpu_clustering, true, false),
            "parallel command finalization must preserve all render passes and lighting"
        );
        assert_eq!(
            direct,
            run(gpu_clustering, true, true),
            "render-thread camera scheduling must preserve all render passes and lighting"
        );
    }
}

#[test]
#[ignore = "requires a GPU; verifies ready/recorded command submission order during parallel finalization"]
fn parallel_command_finalization_preserves_mixed_buffer_submission_order_gpu() {
    use bevy::render::renderer::PendingCommandBuffers;
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
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>().clone();
    let queue = render.resource::<RenderQueue>().clone();
    for (parallel, threads) in [(false, 0), (true, 0), (true, 1), (true, 3), (true, 4)] {
        for through_context in [false, true] {
            let source = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &11u32.to_le_bytes(),
                usage: BufferUsages::COPY_SRC,
            });
            let value = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 12,
                usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let observed = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 8,
                usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 12,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut pending = PendingCommandBuffers::default();
            pending.set_parallel_finalization(parallel);
            pending.set_finalization_threads(threads);
            if through_context {
                use bevy::{
                    ecs::system::RunSystemOnce,
                    render::renderer::{ParallelCommandFinalization, RenderContext},
                };
                let world = app.sub_app_mut(RenderApp).world_mut();
                world.insert_resource(pending);
                if parallel {
                    world.insert_resource(ParallelCommandFinalization);
                } else {
                    world.remove_resource::<ParallelCommandFinalization>();
                }
                let (source, value, observed) = (source.clone(), value.clone(), observed.clone());
                world
                    .run_system_once(
                        move |device: Res<RenderDevice>, mut context: RenderContext| {
                            context
                                .command_encoder()
                                .copy_buffer_to_buffer(&source, 0, &value, 0, 4);
                            let mut second = device.create_command_encoder(&default());
                            second.copy_buffer_to_buffer(&value, 0, &observed, 0, 4);
                            context.add_command_encoder(second);
                            context.command_encoder().clear_buffer(&value, 0, Some(4));
                            let mut fourth = device.create_command_encoder(&default());
                            fourth.copy_buffer_to_buffer(&value, 0, &observed, 4, 4);
                            context.add_command_buffer(fourth.finish());
                        },
                    )
                    .unwrap();
                pending = world.remove_resource::<PendingCommandBuffers>().unwrap();
            } else {
                let mut first = device.create_command_encoder(&default());
                first.copy_buffer_to_buffer(&source, 0, &value, 0, 4);
                pending.push([first.finish()]);
                let mut second = device.create_command_encoder(&default());
                second.copy_buffer_to_buffer(&value, 0, &observed, 0, 4);
                pending.push_encoder(second);
                let mut third = device.create_command_encoder(&default());
                third.clear_buffer(&value, 0, Some(4));
                pending.push([third.finish()]);
                let mut fourth = device.create_command_encoder(&default());
                fourth.copy_buffer_to_buffer(&value, 0, &observed, 4, 4);
                pending.push_encoder(fourth);
            }
            assert_eq!(pending.len(), 4);
            let mut commands = pending.take();
            assert!(pending.is_empty());
            assert_eq!(
                pending.parallel_finalized_encoders(),
                if parallel {
                    if through_context {
                        3
                    } else {
                        2
                    }
                } else {
                    0
                }
            );
            let mut copy = device.create_command_encoder(&default());
            copy.copy_buffer_to_buffer(&value, 0, &readback, 0, 4);
            copy.copy_buffer_to_buffer(&observed, 0, &readback, 4, 8);
            commands.push(copy.finish());
            let submission = queue.submit(commands);
            let (send, receive) = std::sync::mpsc::channel();
            readback
                .slice(..)
                .map_async(MapMode::Read, move |result| send.send(result).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(20)),
                })
                .unwrap();
            receive.recv().unwrap().unwrap();
            let mapped = readback.slice(..).get_mapped_range();
            let actual: Vec<_> = mapped
                .chunks_exact(4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                .collect();
            assert_eq!(
                actual,
                vec![0, 11, 0],
                "ready and recorded command buffers were reordered"
            );
            drop(mapped);
            readback.unmap();
        }
    }
}
