use super::*;
use bevy::{prelude::*, render::RenderApp};
fn pixels(image: usize) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            if image == 1 || image == 6 {
                let v = if image == 1 {
                    [0.0001f32, 0.0004, 0.001, 0.005, 0.02, 0.1, 0.5, 1.][(x + 3 * y) % 8]
                } else {
                    3. + ((x + y) % 5) as f32
                };
                data.extend(v.to_le_bytes());
            } else {
                let values = if image == 3 {
                    [((x + 3 * y) % 9) as f32 / 8., 0., 0., 1.]
                } else {
                    [
                        ((x * 3 + y * 5 + image * 7) % 19) as f32 / 8.,
                        ((x * 7 + y + image * 3) % 17) as f32 / 8.,
                        ((x + y * 11 + image * 5) % 23) as f32 / 8.,
                        1.,
                    ]
                };
                for v in values {
                    data.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
                }
            }
        }
    }
    data
}
#[test]
#[ignore = "requires GPU; compares installed camera-combine HLSL with synthetic textures"]
fn camera_combine_matches_game_shader() {
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
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let extent = Extent3d {
        width: 8,
        height: 8,
        depth_or_array_layers: 1,
    };
    let textures: Vec<_> = (0..7)
        .map(|i| {
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: None,
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: if i == 1 || i == 6 {
                        TextureFormat::R32Float
                    } else {
                        TextureFormat::Rgba16Float
                    },
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &pixels(i),
            )
        })
        .collect();
    let views: Vec<_> = textures.iter().map(|t| t.create_view(&default())).collect();
    let adaptation = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 32,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let target = device.create_texture(&TextureDescriptor {
        label: None,
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba32Float,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let output = target.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut pass = CameraCombinePass::new(device, TextureFormat::Rgba32Float);
    let mut lines = include_str!("../../../tests/fixtures/camera_combine.txt").lines();
    let mut frames = 0;
    let mut maximum = 0f32;
    while let Some(header) = lines.next() {
        let raw = header.strip_prefix("FRAME ").unwrap();
        let bytes: Vec<u8> = (0..raw.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(bytes.len(), 160);
        let f = |i: usize| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[128..132].try_into().unwrap());
        let params = CameraCombineParameters {
            optics: crate::rendering::camera_effects::CombineParameters {
                camera: [0.; 4],
                near: [0.; 2],
                far: std::array::from_fn(|i| f(8 + i)),
                distortion: std::array::from_fn(|i| f(12 + i)),
                vignette: std::array::from_fn(|i| f(i)),
                noise: std::array::from_fn(|i| f(24 + i)),
                centre: [f(16), f(17)],
                options: std::array::from_fn(|i| flags & (1 << i) != 0),
            },
            scattering_colour: [f(4), f(5), f(6)],
            white_level: f(7),
            resolution_scale: [f(18), f(19)],
            inverse_z_time: std::array::from_fn(|i| f(20 + i)),
            host_radiance_units: None,
            compressed: bytes[132] != 0,
            exposure_entry: 0,
        };
        assert_eq!(params.uniform_bytes().as_slice(), &bytes[..144]);
        let mut adaptation_bytes = [0; 32];
        adaptation_bytes[..16].copy_from_slice(&bytes[144..]);
        queue.write_buffer(&adaptation, 0, &adaptation_bytes);
        let mut encoder = device.create_command_encoder(&default());
        pass.record(
            device,
            queue,
            &mut encoder,
            &params,
            CameraCombineInputs {
                images: std::array::from_fn(|i| &views[i]),
                adaptation: &adaptation,
            },
            &output,
            [8, 8],
        )
        .unwrap();
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            extent,
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        for pixel in 0..64 {
            for (channel, raw) in lines.next().unwrap().split_whitespace().enumerate() {
                let expected = f32::from_bits(u32::from_str_radix(raw, 16).unwrap());
                let at = pixel / 8 * 256 + pixel % 8 * 16 + channel * 4;
                let actual = f32::from_le_bytes(mapped[at..at + 4].try_into().unwrap());
                let error = (actual - expected).abs();
                maximum = maximum.max(error);
                assert!(
                    actual.is_finite() && error < 3e-5 * (1. + expected.abs()),
                    "frame{frames} pixel{pixel} channel{channel}: {actual} != {expected}"
                );
            }
        }
        drop(mapped);
        staging.unmap();
        frames += 1;
    }
    assert_eq!(frames, 64);
    println!("{frames} camera frames /4096 pixels; max absolute error {maximum}; noise branch not covered");
}
