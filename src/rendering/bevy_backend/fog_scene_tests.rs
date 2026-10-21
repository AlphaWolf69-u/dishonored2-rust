//! Numerically exercise the actual Core3d atmosphere owner without a window.
use super::{
    fog::{FogCamera, FogCanvas, FogFrame, FogParameters, FogPlugin, FogSceneFrame},
    sky_mask::{SkyMaterial, SkyMesh},
};
use bevy::{
    asset::RenderAssetUsages,
    camera::{Exposure, RenderTarget},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    prelude::*,
    render::{
        extract_resource::ExtractResourcePlugin,
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
        texture::GpuImage,
        RenderApp, RenderPlugin,
    },
};

#[test]
#[ignore = "requires graphics adapter and installed clock controls; renders no window"]
fn atmosphere_owner_draws_sky_and_fog_and_restores_unfogged_frame() {
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
    app.add_plugins((
        crate::gameplay::game_clock_runtime::ClockRuntimePlugin,
        FogPlugin,
        ExtractResourcePlugin::<super::shadow_pass::ShadowSceneFrame>::default(),
        ExtractResourcePlugin::<super::shadow_sampling::ParallelSamplingBuffer>::default(),
    ));
    app.init_resource::<super::shadow_pass::ShadowSceneFrame>();
    app.sub_app_mut(RenderApp)
        .init_resource::<super::shadow_pass::ShadowSceneGpu>();
    app.finish();
    app.cleanup();
    crate::shadow_sampling::register_shaders(&mut app);
    // Conditional imports in the patched mesh shader still require their
    // modules, as in the client and submission_tests' existing fixture.
    let _indirect = app
        .world_mut()
        .resource_mut::<Assets<Shader>>()
        .add(Shader::from_wgsl(
            super::probe_weights::indirect_source(include_str!("../../../assets/shaders/dh2_indirect.wgsl")),
            "fog-scene-fixture/dh2_indirect.wgsl",
        ));
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
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |error| {
            eprintln!("Atmosphere owner GPU: {error}");
            capture.lock().unwrap().push(format!("{error}"));
        }));
    let image = |bytes: &[u8], format| {
        Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            bytes,
            format,
            RenderAssetUsages::default(),
        )
    };
    let diffuse = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(&[32, 64, 128, 255], TextureFormat::Rgba8Unorm));
    let black = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(&[0], TextureFormat::R8Unorm));
    let curve = app.world_mut().resource_mut::<Assets<Image>>().add(image(
        &half::f16::from_f32(0.5).to_bits().to_le_bytes(),
        TextureFormat::R16Float,
    ));
    let noise = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(&[128], TextureFormat::R8Unorm));
    let unavailable = app.world_mut().resource_mut::<Assets<Image>>().add(
        super::shadow_pipeline::shadow_depth_image(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureFormat::Depth24PlusStencil8,
        )
        .unwrap(),
    );
    let parallel = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(
            super::shadow_sampling::ParallelSamplingUniform::default(),
        ));
    app.insert_resource(super::shadow_sampling::ParallelSamplingBuffer {
        buffer: parallel,
        unavailable_depth: unavailable,
    });
    let shared = app
        .world_mut()
        .resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let mut buffers = world.resource_mut::<Assets<ShaderBuffer>>();
            let mut shared =
                super::fog_material::FogMaterialResources::new(&mut images, &mut buffers);
            images
                .get_mut(&shared.opaque_depth)
                .unwrap()
                .texture_descriptor
                .usage |= TextureUsages::COPY_SRC;
            shared
                .resize(
                    &mut images,
                    super::fog_targets::FogExtents::new(UVec2::splat(16), 1).unwrap(),
                )
                .unwrap();
            shared
        });
    let shared_handles = shared.clone();
    app.insert_resource(shared);
    let mut target = Image::new_target_texture(16, 16, TextureFormat::Rgba16Float, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(target);
    let camera = app
        .world_mut()
        .spawn((
            Camera3d {
                depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING)
                    .into(),
                ..default()
            },
            FogCamera,
            bevy::camera::Hdr,
            Camera {
                clear_color: Color::BLACK.into(),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            Projection::custom(super::camera_projection::GameProjection::new(
                90., 0.01, 7000.,
            )),
            Msaa::Off,
            bevy::render::view::NoIndirectDrawing,
            Tonemapping::None,
            DebandDither::Disabled,
            Exposure { ev100: 0. },
            Transform::IDENTITY,
        ))
        .id();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-10., -10., -2.], [30., -10., -2.], [-10., 30., -2.]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5, 0.5]; 3]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.; 4]; 3]);
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let mut sky = [Vec4::ZERO; 7];
    sky[0].x = 1.;
    sky[1] = Vec4::new(32., 64., 1., 0.);
    let state = crate::render_state::parse_render_state("{cull none}", None).unwrap();
    let material = app
        .world_mut()
        .resource_mut::<Assets<SkyMaterial>>()
        .add(SkyMaterial {
            sky,
            depth_offset: 1.,
            depth_uv: Vec4::new(1., 1., 0., 0.),
            clouds: false,
            project_far: true,
            state,
            images: [Some(diffuse), None, None, None, Some(black)],
        });
    app.world_mut().spawn((
        Mesh3d(mesh),
        SkyMesh { material, order: 0 },
        Transform::IDENTITY,
        Visibility::Visible,
    ));
    // A small opaque object leaves the centre sky sample intact, while making
    // the depth snapshot prove that it copied geometry rather than only zero.
    let occluder = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(0.2, 0.2, 0.2));
    let occluder_material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            unlit: true,
            base_color: Color::WHITE,
            ..default()
        });
    let occluder_entity = app
        .world_mut()
        .spawn((
            Mesh3d(occluder),
            MeshMaterial3d(occluder_material),
            // The 16x16 target preserves the game's 16:9-reference vertical FOV:
            // its visible half-width at z=-1 is 0.5625, not 1.0.
            Transform::from_xyz(-0.35, -0.35, -1.),
        ))
        .id();
    let fog = crate::rendering::fog::world::WorldFog {
        flags: 8,
        distance: Vec4::new(0., 0.01, 3.5, 0.),
        distance_start: Vec4::new(0.02, 0.03, 0.04, 1.),
        distance_end: Vec4::new(0.02, 0.03, 0.04, 1.),
        ..default()
    };
    let parameters = FogParameters::new(
        fog,
        crate::rendering::fog::world::WorldFogEffects {
            far_clip: 7000.,
            ..default()
        },
    );
    let mut settings = crate::platform::game_settings::fog_render_settings("").unwrap();
    settings.skip_blur = true;
    app.insert_resource(FogSceneFrame(Some(FogFrame {
        shadow:super::fog::FogShadowFrame {image:curve.clone(),extent:15000.,projected:false,temporal:false},
        areas: Vec::new(),
        area_states: None,
        light_canvas: None,
        revision: 1,
        camera,
        parameters,
        canvas: FogCanvas {
            s: Vec4::X,
            t: Vec4::Y,
            octaves: [Vec4::ZERO; 10],
        },
        eye: Vec3::ZERO,
        sky_masking_factor: 1.,
        curve,
        noise,
        settings,
        rays: crate::rendering::fog::rays::RaySettings {
            mask_size: 0.,
            direction: Some(Vec3::X),
            add_scattering: false,
            depth_start: 0.,
            threshold_bias: Vec3::ZERO,
            threshold_scale: Vec3::ONE,
            color: Vec4::ONE,
            density: 0.,
            decay: 1.,
            weight: 0.,
        },
        sun_sprite: None,
    })));
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("atmosphere owner numeric readback"),
        size: 16 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // Prove the snapshot independently of scene visibility/pipeline readiness.
    // The gameplay check below also requires non-clear opaque geometry.
    {
        let attachment = |label| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let source = attachment("snapshot nonzero source");
        let destination = attachment("snapshot numerical destination");
        let source_view = source.create_view(&default());
        let destination_view = destination.create_view(&default());
        let mut encoder = device.create_command_encoder(&default());
        drop(encoder.begin_render_pass(&RenderPassDescriptor {
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &source_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.375),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..default()
        }));
        super::fog_material_pass::FogDepthSnapshot::new(&device).record(
            &device,
            &mut encoder,
            &source_view,
            &destination_view,
        );
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &destination,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::DepthOnly,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(16),
                },
            },
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
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
        for y in 0..16 {
            for x in 0..16 {
                let at = y * 256 + x * 4;
                assert_eq!(
                    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                    0.375,
                    "opaque snapshot copy at {x},{y}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
    }
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
    let mut programs = crate::assets::shader_cache::ProgramLibrary::open(
        &base,
        crate::assets::shader_cache::ShaderCacheBackend::Pc,
        &catalog,
    )
    .unwrap();
    let sun_state = programs
        .definition(b"PostFX_Sun_Sprite")
        .unwrap()
        .unwrap()
        .render_state();
    let sun_image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(image(&[64, 32, 16, 128], TextureFormat::Rgba8Unorm));
    let sun_template = super::fog::SunSpriteFrame {
        parameters: crate::rendering::fog::sun_sprite::SunSpriteParameters {
            enabled: true,
            additive: true,
            direction: Some(Vec3::Y),
            size: 50.,
            color: Vec4::new(0.5, 0.75, 1., 0.25),
        },
        image: sun_image,
        state: sun_state,
    };
    let mut results = Vec::new();
    let area_geometry = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(8., 8., 8.));
    let area_frame = super::fog::AreaFogFrame {
        meshes: vec![area_geometry],
        world_from_local: Mat4::from_translation(Vec3::new(0., 0., -5.)),
        origin: Vec3::new(0., 5., 0.),
        bounds: [[-4., 1., -4.], [4., 9., 4.]],
        parameters: crate::rendering::fog::alpha::VolumeFog {
            volume_type: Vec3::X,
            distance: Vec4::new(0., 0.1, 2., 0.),
            color_start: Vec3::new(0.7, 0.2, 0.1),
            color_end: Vec3::new(0.7, 0.2, 0.1),
            density_factor: 1.,
            falloff: 0.03,
            scattering_direction: Vec3::Z,
            scattering_phase: 0.75,
            scattering_color: Vec3::ZERO,
            scattering_intensity: 0.,
        },
    };
    let area_states = super::fog::AreaFogStates {
        interval: programs
            .definition(b"atm/distanceFog/renderMinMax")
            .unwrap()
            .unwrap()
            .render_state(),
        radiance: programs
            .definition(b"atm/distanceFog/renderVolume")
            .unwrap()
            .unwrap()
            .render_state(),
    };
    let mut shared_ids = None;
    let control_readback = device.create_buffer(&BufferDescriptor {
        label: Some("shared fog material readiness"),
        size: 384,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for (mode, (flags, sun_mode, local_fog)) in [
        (0, None, false),
        (8, None, false),
        (0, None, false),
        (0, Some(true), false),
        (0, Some(false), false),
        (0, None, false),
        (0, None, true),
        (0, None, false),
        (0, None, true),
    ].into_iter().enumerate() {
        {
            let mut frame = app.world_mut().resource_mut::<FogSceneFrame>();
            let frame = frame.0.as_mut().unwrap();
            frame.areas = if local_fog {
                vec![area_frame.clone()]
            } else {
                Vec::new()
            };
            frame.area_states = Some(area_states);
        }
        let expected_object = if local_fog && mode != 8 {
            super::fog_objects::FogObjectParameters::selected(&area_frame, [1., 9., 5.])
        } else { super::fog_objects::FogObjectParameters::default() };
        let source_frame = app.world().resource::<FogSceneFrame>().0.as_ref().unwrap();
        let objects = super::fog_objects::FogObjectFrame {
            camera: source_frame.camera, revision: source_frame.revision,
            world_offset: 1, capacity: 1, updates: vec![(0, expected_object)],
        };
        app.world_mut().resource_mut::<super::fog_material::FogMaterialResources>().object_frame =
            Some(std::sync::Arc::new(objects));
        let active = flags != 0 || local_fog;
        app.world_mut()
            .resource_mut::<FogSceneFrame>()
            .0
            .as_mut()
            .unwrap()
            .sun_sprite = sun_mode.map(|additive| {
            let mut sun = sun_template.clone();
            sun.parameters.additive = additive;
            sun
        });
        app.world_mut()
            .resource_mut::<FogSceneFrame>()
            .0
            .as_mut()
            .unwrap()
            .parameters
            .flags
            .x = flags;
        for _ in 0..40 {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        println!(
            "Opaque fixture visibility {:?} / {:?}",
            app.world().get::<ViewVisibility>(occluder_entity),
            app.world().get::<InheritedVisibility>(occluder_entity)
        );
        let render = app.sub_app(RenderApp).world();
        for pipeline in render.resource::<PipelineCache>().pipelines() {
            if let CachedPipelineState::Err(error) = &pipeline.state {
                panic!("scene opaque shader: {error}");
            }
        }
        let images = render.resource::<RenderAssets<GpuImage>>();
        let ids = [
            &shared_handles.world,
            &shared_handles.canvas,
            &shared_handles.opaque_depth,
        ]
        .map(|handle| images.get(handle).unwrap().texture.id());
        if let Some(previous) = shared_ids {
            assert_eq!(
                previous, ids,
                "shared fog target allocations changed without a resize"
            );
        }
        shared_ids = Some(ids);
        let output = images.get(&target).unwrap();
        let mut encoder = device.create_command_encoder(&default());
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &output.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(16),
                },
            },
            Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let offset = 8 * 256 + 8 * 8;
        let pixel: [f32; 4] = std::array::from_fn(|channel| {
            half::f16::from_bits(u16::from_le_bytes(
                bytes[offset + channel * 2..offset + channel * 2 + 2]
                    .try_into()
                    .unwrap(),
            ))
            .to_f32()
        });
        assert!(
            pixel.iter().all(|value| value.is_finite()),
            "nonfinite atmosphere flags={flags}: {pixel:?}"
        );
        println!("Atmosphere owner flags={flags}: {pixel:?}");
        results.push(pixel);
        drop(bytes);
        staging.unmap();
        let parameters = render
            .resource::<RenderAssets<bevy::render::storage::GpuShaderBuffer>>()
            .get(&shared_handles.parameters)
            .unwrap();
        let mut encoder = device.create_command_encoder(&default());
        encoder.copy_buffer_to_buffer(&parameters.buffer, 0, &control_readback, 0, 256);
        let objects = render.resource::<RenderAssets<bevy::render::storage::GpuShaderBuffer>>()
            .get(&shared_handles.objects).unwrap();
        encoder.copy_buffer_to_buffer(&objects.buffer, 0, &control_readback, 256, 128);
        if active {
            let depth = images.get(&shared_handles.opaque_depth).unwrap();
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &depth.texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::DepthOnly,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(16),
                    },
                },
                Extent3d {
                    width: 16,
                    height: 16,
                    depth_or_array_layers: 1,
                },
            );
        }
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        control_readback
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let control = control_readback.slice(..).get_mapped_range();
        let ready = f32::from_le_bytes(control[..4].try_into().unwrap());
        assert_eq!(
            ready,
            if active { 1. } else { 0. },
            "shared fog material readiness flags={flags}"
        );
        if active {
            let offset = f32::from_le_bytes(control[12..16].try_into().unwrap());
            assert_eq!(offset, if local_fog {1.} else {-1.}, "object frame publication mode{mode}");
        }
        if local_fog {
            let expected = ShaderBuffer::from(expected_object);
            assert_eq!(expected.data.as_ref().unwrap().len(), 128);
            assert_eq!(&control[256..384], expected.data.as_ref().unwrap().as_slice(),
                "actual owner object upload/clearing mode{mode}");
        }
        drop(control);
        control_readback.unmap();
        if active {
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let depth = staging.slice(..).get_mapped_range();
            let mut populated = false;
            for y in 0..16 {
                for x in 0..16 {
                    let offset = y * 256 + x * 4;
                    let value = f32::from_le_bytes(depth[offset..offset + 4].try_into().unwrap());
                    assert!(value.is_finite() && (0.0..=1.0).contains(&value));
                    populated |= value > 0.;
                }
            }
            assert!(
                populated,
                "opaque fog depth snapshot did not copy the foreground geometry"
            );
            drop(depth);
            staging.unmap();
        }
    }
    assert!(
        results[0][0] > 0.01 && results[0][2] > results[0][0],
        "sky mesh was not drawn: {:?}",
        results[0]
    );
    assert!(
        (0..3).any(|channel| (results[1][channel] - results[0][channel]).abs() > 0.01),
        "world fog did not affect the actual camera target: {results:?}"
    );
    assert_eq!(
        results[0], results[2],
        "disabling fog must restore the same sky, without accumulated targets"
    );
    assert!(
        (0..3).all(|channel| results[3][channel] > results[0][channel]),
        "sun sprite did not reach the actual camera target: {results:?}"
    );
    assert_ne!(
        results[3], results[4],
        "authored sun blend change was ignored"
    );
    assert_eq!(
        results[0], results[5],
        "disabling the sun sprite left accumulated colour"
    );
    let alpha = 1. - (-1.6_f32).exp();
    let exposure = Exposure { ev100: 0. }.exposure();
    for (channel, color) in [0.7, 0.2, 0.1].into_iter().enumerate() {
        let expected = results[0][channel] * (1. - alpha) + color * alpha * 139.26 * exposure;
        assert!(
            (results[6][channel] - expected).abs() < 0.2,
            "closed area fog volume channel{channel}: {} != {expected}",
            results[6][channel]
        );
    }
    assert_eq!(
        results[0], results[7],
        "removing area fog left stale radiance"
    );
    assert!(
        errors.lock().unwrap().is_empty(),
        "atmosphere GPU validation: {:?}",
        errors.lock().unwrap()
    );
}
