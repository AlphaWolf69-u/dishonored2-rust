use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; compares all colour downscale modes with installed HLSL"]
fn colour_downscale_matches_installed_shader() {
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
    let pixels: Vec<_> = (0_i32..8)
        .flat_map(|y| {
            (0_i32..8).flat_map(move |x| {
                let values = match (x + y) % 4 {
                    0 => [0.0, 0.0, 0.0, 0.0],
                    1 => [0.25, 0.5, 1.0, 0.75],
                    _ => [
                        ((x * 5 + y * 3) % 17 - 4) as f32 * 0.125,
                        ((x * 7 + y) % 19 - 3) as f32 * 0.25,
                        ((x + y * 11) % 23 - 2) as f32 * 0.5,
                        ((x * 3 + y * 5) % 13) as f32 * 0.0625,
                    ],
                };
                values
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
    let source = device
        .create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("colour downscale oracle image"),
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
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let frames: Vec<_> = include_str!("../../../../tests/fixtures/colour_downscale.txt")
        .lines()
        .collect();
    let mut channels = 0;
    for format in [
        TextureFormat::Rgba32Float,
        TextureFormat::Rgba16Float,
        TextureFormat::Rg11b10Ufloat,
    ] {
        let mut draw = ColourDownscalePass::new(device, format);
        let mut host_draw = ColourDownscalePass::new_with_host_source(device, format);
        let mut hdr_owners = [
            HdrDownscale::new(device, [8, 8], false).unwrap(),
            HdrDownscale::new(device, [8, 8], true).unwrap(),
        ];
        let owner_ids = hdr_owners.each_ref().map(|v| v.output().id());
        assert!(hdr_owners[0].matches([8, 8], false));
        assert!(!hdr_owners[0].matches([16, 8], false));
        assert!(!hdr_owners[0].matches([8, 8], true));
        let target = device.create_texture(&TextureDescriptor {
            label: Some("colour downscale oracle target"),
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
        let mut packed_steps = 0;
        for (frame_index, frame) in frames.chunks_exact(65).enumerate() {
            let packet = frame[0].strip_prefix("FRAME ").unwrap();
            let words: Vec<_> = (0..16)
                .map(|i| {
                    let bytes: Vec<_> = (0..4)
                        .map(|j| {
                            u8::from_str_radix(&packet[i * 8 + j * 2..i * 8 + j * 2 + 2], 16)
                                .unwrap()
                        })
                        .collect();
                    u32::from_le_bytes(bytes.try_into().unwrap())
                })
                .collect();
            let values: Vec<_> = words.iter().copied().map(f32::from_bits).collect();
            let mut p = DownscaleParameters {
                texel: Vec4::from_slice(&values[0..4]),
                colour: Vec4::from_slice(&values[4..8]),
                limits: Vec4::from_slice(&values[8..12]),
                options: UVec4::from_slice(&words[12..16]),
            };
            let mut encoder = device.create_command_encoder(&default());
            let use_owner = words[14] != 0
                && format
                    == if words[13] != 0 {
                        TextureFormat::Rg11b10Ufloat
                    } else {
                        TextureFormat::Rgba16Float
                    };
            if words[14] != 0 {
                let engine = values[9];
                // Poison the CPU field: this path must consume the GPU factor.
                p.limits.y = 0.03125;
                let adaptation = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("HDR source GPU adaptation"),
                    contents: &[engine, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
                        .into_iter()
                        .flat_map(f32::to_le_bytes)
                        .collect::<Vec<_>>(),
                    usage: BufferUsages::STORAGE,
                });
                if use_owner {
                    let owner = &mut hdr_owners[words[13] as usize];
                    let camera_exposure = values[11] / 139.26;
                    assert_eq!((139.26 * camera_exposure).to_bits(), values[11].to_bits());
                    owner
                        .record(
                            device,
                            queue,
                            &mut encoder,
                            &source,
                            [8, 8],
                            camera_exposure,
                            values[8],
                            values[10],
                            &adaptation,
                        )
                        .unwrap();
                    assert_eq!(owner.output().id(), owner_ids[words[13] as usize]);
                } else {
                    host_draw.record_host(
                        device,
                        queue,
                        &mut encoder,
                        p,
                        &source,
                        &view,
                        [0, 0, 8, 8],
                        [0, 0, 8, 8],
                        &adaptation,
                    );
                }
            } else {
                draw.record(
                    device,
                    queue,
                    &mut encoder,
                    p,
                    &source,
                    &view,
                    [0, 0, 8, 8],
                    [0, 0, 8, 8],
                );
            }
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: if use_owner {
                        &hdr_owners[words[13] as usize].texture
                    } else {
                        &target
                    },
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
                } else if format == TextureFormat::Rgba16Float {
                    for channel in 0..4 {
                        let offset = (pixel / 8) * 256 + (pixel % 8) * 8 + channel * 2;
                        let actual =
                            u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap());
                        let expected =
                            half::f16::from_f32(f32::from_bits(expected[channel])).to_bits();
                        let distance = actual.abs_diff(expected);
                        packed_steps = packed_steps.max(distance as u32);
                        assert!(distance<=1,"half frame{frame_index} pixel{pixel} channel{channel}: {actual:04x} != {expected:04x}");
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
        eprintln!("72 colour-downscale frames {format:?}: maximum float error {maximum}, packed component distance {packed_steps} ULP");
    }
    assert_eq!(channels, 50688);
}
