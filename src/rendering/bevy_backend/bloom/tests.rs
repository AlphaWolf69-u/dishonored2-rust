use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; compares actual raster pass with installed HLSL"]
fn bloom_upsample_matches_installed_shader() {
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
    let pixels: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [
                    ((x * 5 + y * 3) % 17) as f32 * 0.125,
                    ((x * 7 + y) % 19) as f32 * 0.25,
                    ((x + y * 11) % 23) as f32 * 0.5,
                    ((x * 3 + y * 5) % 13) as f32 * 0.0625,
                ]
                .into_iter()
                .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            })
        })
        .collect();
    let extent = Extent3d {
        width: 8,
        height: 8,
        depth_or_array_layers: 1,
    };
    let source = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("bloom oracle source"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &pixels,
    );
    let source = source.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let frames: Vec<_> = include_str!("../../../../tests/fixtures/bloom.txt")
        .lines()
        .collect();
    let mut channel_count = 0;
    for format in [TextureFormat::Rgba32Float, TextureFormat::Rgba16Float] {
        let mut bloom = BloomUpsamplePass::new(device, format);
        let target = device.create_texture(&TextureDescriptor {
            label: Some("bloom comparison target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&default());
        let mut maximum = 0.0_f32;
        let mut maximum_half_steps = 0;
        for (frame_index, frame) in frames.chunks_exact(65).enumerate() {
            let packet = frame[0].strip_prefix("FRAME ").unwrap();
            let values: Vec<_> = (0..12)
                .map(|i| {
                    let bytes: Vec<_> = (0..4)
                        .map(|j| {
                            u8::from_str_radix(&packet[i * 8 + j * 2..i * 8 + j * 2 + 2], 16)
                                .unwrap()
                        })
                        .collect();
                    f32::from_le_bytes(bytes.try_into().unwrap())
                })
                .collect();
            let parameters = BloomParameters {
                pixel_scale: Vec4::from_slice(&values[0..4]),
                viewport: Vec4::from_slice(&values[4..8]),
                blur: Vec4::from_slice(&values[8..12]),
            };
            let mut encoder = device.create_command_encoder(&default());
            bloom.record(device, queue, &mut encoder, parameters, &source, &view);
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
            staging
                .slice(..)
                .map_async(MapMode::Read, move |v| tx.send(v).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            for (pixel, row) in frame[1..].iter().enumerate() {
                let first = if format == TextureFormat::Rgba32Float {
                    0
                } else {
                    4
                };
                for (channel, word) in row.split_whitespace().skip(first).take(4).enumerate() {
                    let expected = f32::from_bits(u32::from_str_radix(word, 16).unwrap());
                    let width = if format == TextureFormat::Rgba32Float {
                        4
                    } else {
                        2
                    };
                    let offset = (pixel / 8) * 256 + (pixel % 8) * 4 * width + channel * width;
                    let (actual, expected, tolerance) = if width == 4 {
                        (
                            f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()),
                            expected,
                            0.00002,
                        )
                    } else {
                        let actual_bits =
                            u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                        let expected_bits = half::f16::from_f32(expected).to_bits();
                        let steps = actual_bits.abs_diff(expected_bits);
                        maximum_half_steps = maximum_half_steps.max(steps);
                        assert!(steps <= 1, "FP16 frame{frame_index} pixel{pixel} channel{channel}: {steps} representable steps");
                        (
                            half::f16::from_bits(u16::from_le_bytes(
                                bytes[offset..offset + 2].try_into().unwrap(),
                            ))
                            .to_f32(),
                            half::f16::from_f32(expected).to_f32(),
                            expected.abs() * 0.001 + 0.00001,
                        )
                    };
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(actual.is_finite() && error<=tolerance,
                        "{format:?} frame{frame_index} pixel{pixel} channel{channel}: {actual} != {expected}");
                    channel_count += 1;
                }
            }
            drop(bytes);
            staging.unmap();
        }
        eprintln!("24 bloom raster frames {format:?}, maximum error {maximum}");
        if format == TextureFormat::Rgba16Float {
            eprintln!("Maximum FP16 distance from D3D11 render stores: {maximum_half_steps} representable steps");
        }
    }
    assert_eq!(channel_count, 12288);
}

#[test]
#[ignore = "requires GPU; compares chained stores with installed bloom HLSL"]
fn bloom_chain_preserves_scheduled_draws_and_uniforms() {
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
    let pixels: Vec<u8> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [
                    ((x * 5 + y * 3) % 17) as f32 * 0.125,
                    ((x * 7 + y) % 19) as f32 * 0.25,
                    ((x + y * 11) % 23) as f32 * 0.5,
                    ((x * 3 + y * 5) % 13) as f32 * 0.0625,
                ]
                .into_iter()
                .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            })
        })
        .collect();
    let extent = Extent3d {
        width: 8,
        height: 8,
        depth_or_array_layers: 1,
    };
    let input = device
        .create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &pixels,
        )
        .create_view(&default());
    let plan = crate::rendering::bloom::BloomSchedule::new(
        &[[8, 8], [4, 4], [2, 2], [1, 1]],
        0,
        0,
        0,
        [1., 0.875],
    )
    .unwrap()
    .unwrap();
    let mut chain = BloomChain::new(device, plan).unwrap();
    let output = chain.output().id();
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 4096,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut lines = include_str!("../../../../tests/fixtures/bloom_chain.txt").lines();
    let mut frames = 0;
    let mut channels = 0;
    while let Some(header) = lines.next() {
        let veil = header.strip_prefix("CHAIN ").unwrap().parse().unwrap();
        let mut views = Vec::new();
        let mut expected = Vec::new();
        for _ in 0..2 {
            let packet = lines.next().unwrap().strip_prefix("DRAW ").unwrap();
            let values: Vec<f32> = (0..12)
                .map(|i| {
                    f32::from_le_bytes(std::array::from_fn(|j| {
                        u8::from_str_radix(&packet[i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
                    }))
                })
                .collect();
            let view = BloomViewParameters::full_target([8, 8], [1.0, 0.875], [0.0, 0.0]);
            assert_eq!(view.pixel_scale, Vec4::from_slice(&values[0..4]));
            assert_eq!(view.viewport, Vec4::from_slice(&values[4..8]));
            views.push(view);
            expected.push(
                (0..64)
                    .map(|_| {
                        lines
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .map(|w| u16::from_str_radix(w, 16).unwrap())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let mut encoder = device.create_command_encoder(&default());
        chain
            .record(device, queue, &mut encoder, &input, &views, veil)
            .unwrap();
        for (index, step) in chain.test_textures().enumerate() {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: step,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: index as u64 * 2048,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(8),
                    },
                },
                extent,
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for step in 0..2 {
            for pixel in 0..64 {
                for channel in 0..4 {
                    let offset = step * 2048 + pixel / 8 * 256 + pixel % 8 * 8 + channel * 2;
                    let actual = u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                    let reference = expected[step][pixel][channel];
                    assert!(actual.abs_diff(reference)<=1,"frame{frames} step{step} pixel{pixel} channel{channel}: {actual:04x} != {reference:04x}");
                    channels += 1;
                }
            }
        }
        drop(bytes);
        staging.unmap();
        assert_eq!(chain.output().id(), output);
        frames += 1;
    }
    assert_eq!((frames, channels), (3, 1536));
}
