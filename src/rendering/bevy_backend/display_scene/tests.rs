//! Real Core3d replacement/readiness path; numeric pixels only, no window.
use super::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::{Exposure, Hdr, RenderTarget},
    core_pipeline::tonemapping::DebandDither,
    render::{render_resource::*, storage::ShaderBuffer, RenderPlugin},
};

fn mode(app: &mut App, camera: Entity) -> Tonemapping {
    let world = app.sub_app_mut(RenderApp).world_mut();
    let mut q = world.query::<(&MainEntity, &Tonemapping)>();
    *q.iter(world)
        .find(|(main, _)| main.id() == camera)
        .unwrap()
        .1
}
fn pixel(app: &App, target: &Handle<Image>) -> [u8; 4] {
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let image = world
        .resource::<RenderAssets<GpuImage>>()
        .get(target)
        .unwrap();
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        image.texture.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &staging,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(8),
            },
        },
        Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
    );
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(MapMode::Read, move |v| tx.send(v).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = staging.slice(..).get_mapped_range();
    let out = data[..4].try_into().unwrap();
    drop(data);
    staging.unmap();
    out
}
fn assert_pixel(actual: [u8; 4], expected: [u8; 4]) {
    for i in 0..4 {
        assert!(
            (i32::from(actual[i]) - i32::from(expected[i])).abs() <= 1,
            "{actual:?} != {expected:?}"
        );
    }
}
fn adaptation_factor(app: &App, handle: &Handle<ShaderBuffer>) -> f32 {
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let exposure = world
        .resource::<RenderAssets<GpuShaderBuffer>>()
        .get(handle)
        .unwrap();
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 32,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&exposure.buffer, 0, &staging, 0, 32);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(MapMode::Read, move |v| tx.send(v).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = staging.slice(..).get_mapped_range();
    let factor = f32::from_le_bytes(bytes[..4].try_into().unwrap());
    drop(bytes);
    staging.unmap();
    factor
}
#[test]
#[ignore = "requires graphics adapter; real display owner, input readiness and GPU exposure"]
fn display_owner_waits_for_inputs_replaces_tone_pass_and_restores_fallback() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..Default::default()
            })
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..Default::default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.add_plugins((super::super::auto_exposure::ExposurePlugin, DisplayPlugin));
    app.finish();
    app.cleanup();
    let device = app
        .sub_app(RenderApp)
        .world()
        .resource::<RenderDevice>()
        .clone();
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let report = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |error| {
            report.lock().unwrap().push(error.to_string())
        }));
    let mut image = Image::new_target_texture(8, 8, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    let cube = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .reserve_handle();
    let factor = 139.26 * Exposure { ev100: 0. }.exposure();
    let adaptation = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![factor, 1., 0., 0., 0., 0., 0., 0.]));
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Hdr,
            Msaa::Off,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::linear_rgb(0.125, 0.25, 0.5)),
                ..Default::default()
            },
            RenderTarget::Image(target.clone().into()),
            Transform::IDENTITY,
            Exposure { ev100: 0. },
            Tonemapping::Reinhard,
            DebandDither::Disabled,
            MaterialExposure(adaptation.clone()),
        ))
        .id();
    let frame = DisplayFrame {
        camera,
        parameters: ToneMappingParameters {
            curve: crate::rendering::tone_mapping::ToneCurve::new(
                crate::rendering::tone_mapping::ToneSettings {
                    overbright: 1.,
                    black: 0.,
                    white: 4.,
                    junction: 0.5,
                    toe: 0.6,
                    shoulder: 0.5,
                },
            ),
            gamma: 1.,
            brightness: 1.,
            exposure: 1.,
            use_curve: false,
            use_colour_cube: false,
        },
        cube: cube.clone(),
        weight: 1.,
        custom_ev: 0.,
        overrides: None,
    };
    app.world_mut().resource_mut::<DisplaySceneFrame>().0 = Some(frame.clone());
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(mode(&mut app, camera), Tonemapping::Reinhard);
    assert!(app
        .sub_app(RenderApp)
        .world()
        .resource::<DisplayGpu>()
        .current
        .is_none());
    app.world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(
            cube.id(),
            Image::new(
                Extent3d {
                    width: 32,
                    height: 32,
                    depth_or_array_layers: 32,
                },
                TextureDimension::D3,
                super::super::colour_cube::identity_pixels(),
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            ),
        )
        .unwrap();
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(mode(&mut app, camera), Tonemapping::None);
    assert_eq!(
        app.sub_app(RenderApp)
            .world()
            .resource::<DisplayGpu>()
            .current
            .as_ref()
            .unwrap()
            .cube_submissions,
        1,
        "unchanged archive colour tables must not be recomputed each frame"
    );
    assert_pixel(pixel(&app, &target), [99, 137, 188, 255]);
    // The current GPU adaptation state, not a CPU snapshot or constant, owns
    // the multiplier. Halving it must be applied exactly once in this frame.
    {
        let render = app.sub_app(RenderApp).world();
        let exposure = render
            .resource::<RenderAssets<GpuShaderBuffer>>()
            .get(&adaptation)
            .unwrap();
        render.resource::<RenderQueue>().write_buffer(
            &exposure.buffer,
            0,
            &(factor * 0.5).to_le_bytes(),
        );
    }
    app.update();
    assert_pixel(pixel(&app, &target), [71, 99, 137, 255]);
    let missing = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .reserve_handle();
    app.world_mut()
        .resource_mut::<DisplaySceneFrame>()
        .0
        .as_mut()
        .unwrap()
        .cube = missing;
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(mode(&mut app, camera), Tonemapping::Reinhard);
    app.world_mut().resource_mut::<DisplaySceneFrame>().0 = None;
    app.update();
    assert_eq!(mode(&mut app, camera), Tonemapping::Reinhard);
    // Activate the actual GPU exposure producer and compare each completed
    // frame with its own adaptation result. A stale multiplier or wrong pass
    // order cannot satisfy this check while adaptation is changing.
    app.world_mut()
        .entity_mut(camera)
        .insert(super::super::auto_exposure::ExposureParameters {
            range: Vec4::new(0.03, 2000., 0., 1.),
            adaptation: Vec4::new(0., 1., 0.99, 0.99),
            camera: Vec4::new(30., 0., 0.75, 0.),
        });
    let mut frame = frame;
    frame.custom_ev = 0.75;
    app.world_mut().resource_mut::<DisplaySceneFrame>().0 = Some(frame);
    let mut factors = Vec::new();
    for _ in 0..3 {
        app.update();
        let factor = adaptation_factor(&app, &adaptation);
        factors.push(factor);
        let scale = factor * 2.0_f32.powf(-0.75) / (139.26 * Exposure { ev100: 0. }.exposure());
        let rgb = [0.125, 0.25, 0.5].map(|v| {
            let v = (v * scale).clamp(0., 1.);
            let s = if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            };
            (s * 255.).round() as u8
        });
        assert_pixel(pixel(&app, &target), [rgb[0], rgb[1], rgb[2], 255]);
        assert_eq!(mode(&mut app, camera), Tonemapping::None);
    }
    assert!(
        factors.windows(2).any(|p| (p[1] - p[0]).abs() > 0.0001),
        "exposure did not change: {factors:?}"
    );
    assert!(
        errors.lock().unwrap().is_empty(),
        "GPU validation: {:?}",
        errors.lock().unwrap()
    );
}
