use super::*;
use bevy::{prelude::*, render::RenderApp};

fn source_pixels(image: u32) -> Vec<u8> {
    (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                let values = if image == 0 {
                    [
                        ((x * 5 + y * 3) % 17) as f32 * 0.125,
                        ((x * 7 + y) % 19) as f32 * 0.25,
                        ((x + y * 11) % 23) as f32 * 0.5,
                        ((x * 3 + y * 5) % 13) as f32 * 0.0625,
                    ]
                } else {
                    [
                        ((x * 3 + y * 5 + image * 7) % 19) as f32 / 16.,
                        ((x * 7 + y + image * 3) % 17) as f32 / 16.,
                        ((x + y * 11 + image * 5) % 23) as f32 / 16.,
                        1.,
                    ]
                };
                values
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            })
        })
        .collect()
}
fn extent() -> Extent3d {
    Extent3d {
        width: 8,
        height: 8,
        depth_or_array_layers: 1,
    }
}

#[test]
#[ignore = "requires GPU; compares float results and packed D3D11 render stores"]
fn flare_passes_match_installed_shaders() {
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
    let sources: Vec<_> = (0..3)
        .map(|i| {
            device
                .create_texture_with_data(
                    queue,
                    &TextureDescriptor {
                        label: Some("flare oracle image"),
                        size: extent(),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: TextureFormat::Rgba16Float,
                        usage: TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    TextureDataOrder::MipMajor,
                    &source_pixels(i),
                )
                .create_view(&default())
        })
        .collect();
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let frames: Vec<_> = include_str!("../../../../tests/fixtures/lens_flare.txt")
        .lines()
        .collect();
    let mut channels = 0;
    for format in [TextureFormat::Rgba32Float, TextureFormat::Rg11b10Ufloat] {
        let first = FlarePass::new(device, format, FlareStage::Threshold);
        let mut passes = [
            first.sibling(FlareStage::Threshold),
            first.sibling(FlareStage::Generate),
            first.sibling(FlareStage::Blur),
        ];
        let target = device.create_texture(&TextureDescriptor {
            label: Some("flare oracle target"),
            size: extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&default());
        let mut maximum = 0.0_f32;
        let mut packed_steps = 0;
        for (frame_index, frame) in frames.chunks_exact(65).enumerate() {
            let packet = frame[0].strip_prefix("FRAME ").unwrap();
            let values: Vec<_> = (0..20)
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
            let p = FlareParameters {
                view: Vec4::from_slice(&values[0..4]),
                threshold: Vec4::from_slice(&values[4..8]),
                ghost: Vec4::from_slice(&values[8..12]),
                controls: Vec4::from_slice(&values[12..16]),
                direction: Vec4::from_slice(&values[16..20]),
            };
            let mut encoder = device.create_command_encoder(&default());
            passes[frame_index / 16].record(
                device,
                queue,
                &mut encoder,
                p,
                [&sources[0], &sources[1], &sources[2]],
                &view,
                [8, 8],
                [8, 8],
            );
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
                extent(),
            );
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging.slice(..).map_async(MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            for (pixel, row) in frame[1..].iter().enumerate() {
                let expected: Vec<_> = row
                    .split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect();
                if format == TextureFormat::Rgba32Float {
                    for channel in 0..4 {
                        let offset = (pixel / 8) * 256 + (pixel % 8) * 16 + channel * 4;
                        let actual =
                            f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                        let expected = f32::from_bits(expected[channel]);
                        let error = (actual - expected).abs();
                        maximum = maximum.max(error);
                        assert!(actual.is_finite() && error<=0.00002+expected.abs()*0.00002,
                            "frame{frame_index} pixel{pixel} channel{channel}: {actual} != {expected}");
                        channels += 1;
                    }
                } else {
                    let offset = (pixel / 8) * 256 + (pixel % 8) * 4;
                    let actual = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                    for (shift, mask) in [(0, 0x7ff), (11, 0x7ff), (22, 0x3ff)] {
                        let distance =
                            ((actual >> shift) & mask).abs_diff((expected[4] >> shift) & mask);
                        packed_steps = packed_steps.max(distance);
                        assert!(distance<=1,"packed frame{frame_index} pixel{pixel} shift{shift}: {actual:08x} != {:08x} ({distance} ULP)",expected[4]);
                        channels += 1;
                    }
                }
            }
            drop(bytes);
            staging.unmap();
        }
        eprintln!("48 flare frames {format:?}: maximum float error {maximum}, packed component distance {packed_steps} ULP");
    }
    assert_eq!(channels, 21504);
}

#[test]
#[ignore = "requires GPU; checks four queued draws with reused packed targets"]
fn flare_chain_preserves_stores_and_parameters() {
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
    let pixels: Vec<_> = (0..16)
        .flat_map(|y| {
            (0..16).flat_map(move |x| {
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
    let texture = |size, data: &[u8]| {
        device
            .create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("flare chain oracle image"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba16Float,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                data,
            )
            .create_view(&default())
    };
    let source = texture(
        Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        &pixels,
    );
    let profiles = [
        texture(extent(), &source_pixels(1)),
        texture(extent(), &source_pixels(2)),
    ];
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8 * 4,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut comparisons = 0;
    let mut maximum_steps = 0;
    for (case, part) in include_str!("../../../../tests/fixtures/lens_flare_chain.txt")
        .split("CHAIN ")
        .filter(|p| !p.trim().is_empty())
        .enumerate()
    {
        let mut lines = part.lines();
        let packet = lines.next().unwrap();
        let values: Vec<_> = (0..20)
            .map(|i| {
                let bytes: Vec<_> = (0..4)
                    .map(|j| {
                        u8::from_str_radix(&packet[i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
                    })
                    .collect();
                f32::from_le_bytes(bytes.try_into().unwrap())
            })
            .collect();
        let parameters = FlareParameters {
            view: Vec4::from_slice(&values[0..4]),
            threshold: Vec4::from_slice(&values[4..8]),
            ghost: Vec4::from_slice(&values[8..12]),
            controls: Vec4::from_slice(&values[12..16]),
            direction: Vec4::from_slice(&values[16..20]),
        };
        let expected: Vec<_> = lines
            .filter(|v| !v.starts_with("DRAW "))
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert_eq!(expected.len(), 256);
        let mut chain = FlareChain::new(device, [16, 16], [8, 8], [values[2], values[3]]).unwrap();
        let output_id = chain.output().id();
        let temporary_id = chain.temporary_view.id();
        for repeat in 0..2 {
            let mut encoder = device.create_command_encoder(&default());
            assert!(!chain.record_observed(
                device,
                queue,
                &mut encoder,
                &source,
                [&profiles[0], &profiles[1]],
                parameters,
                0x200000,
                |_, _, _| panic!("disabled flare recorded a draw")
            ));
            let mut draws = 0;
            assert!(chain.record_observed(
                device,
                queue,
                &mut encoder,
                &source,
                [&profiles[0], &profiles[1]],
                parameters,
                0,
                |encoder, index, texture| {
                    assert_eq!(index, draws);
                    draws += 1;
                    encoder.copy_texture_to_buffer(
                        TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: Origin3d::ZERO,
                            aspect: TextureAspect::All,
                        },
                        TexelCopyBufferInfo {
                            buffer: &staging,
                            layout: TexelCopyBufferLayout {
                                offset: index as u64 * 256 * 8,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(8),
                            },
                        },
                        extent(),
                    );
                }
            ));
            assert_eq!(draws, 4);
            assert_eq!(chain.output().id(), output_id);
            assert_eq!(chain.temporary_view.id(), temporary_id);
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging.slice(..).map_async(MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            for draw in 0..4 {
                for pixel in 0..64 {
                    let offset = draw * 256 * 8 + (pixel / 8) * 256 + (pixel % 8) * 4;
                    let actual = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                    let expected = expected[draw * 64 + pixel];
                    for (shift, mask) in [(0, 0x7ff), (11, 0x7ff), (22, 0x3ff)] {
                        let distance =
                            ((actual >> shift) & mask).abs_diff((expected >> shift) & mask);
                        maximum_steps = maximum_steps.max(distance);
                        assert!(distance<=1,"chain{case} repeat{repeat} draw{draw} pixel{pixel} shift{shift}: {actual:08x} != {expected:08x} ({distance} ULP)");
                        comparisons += 1;
                    }
                }
            }
            drop(bytes);
            staging.unmap();
        }
    }
    assert_eq!(comparisons, 12288);
    eprintln!("8 four-pass flare chains, twice each: {comparisons} packed components, maximum {maximum_steps} ULP");
}
