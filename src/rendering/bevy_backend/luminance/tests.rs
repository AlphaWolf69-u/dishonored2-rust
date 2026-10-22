use super::*;
use bevy::{prelude::*, render::RenderApp};
#[test]
#[ignore = "requires GPU; actual installed shader mip-chain comparison"]
fn luminance_pyramid_matches_installed_downsample_chain() {
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
    let pipelines = LuminancePipelines::new(device);
    let mut lines = include_str!("../../../../tests/fixtures/luminance.txt")
        .lines()
        .peekable();
    let mut frames = 0;
    let mut channels = 0;
    let mut maximum = 0;
    while let Some(frame) = lines.next() {
        let words: Vec<_> = frame.split_whitespace().collect();
        assert_eq!(words[0], "FRAME");
        let width = words[2].parse::<u32>().unwrap();
        let height = words[3].parse::<u32>().unwrap();
        let quality = words[4].parse::<i32>().unwrap();
        let bits: Vec<_> = (0..4)
            .map(|i| u8::from_str_radix(&words[5][i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let exposure = f32::from_le_bytes(bits.try_into().unwrap());
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let values = [
                    (((x * 5 + y * 3) % 17) as i32 - 3) as f64 * 0.125,
                    ((x * 7 + y) % 19) as f64 * 0.25,
                    ((x + y * 11) % 23) as f64 * 0.125,
                    1.0,
                ];
                for value in values {
                    pixels.extend(
                        half::f16::from_f64(value * 139.26 * exposure as f64)
                            .to_bits()
                            .to_le_bytes(),
                    );
                }
            }
        }
        let full = UVec2::new(width, height);
        let source = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: None,
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
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
        let mut pyramid = LuminancePyramid::new(device, queue, full, quality).unwrap();
        assert!(pyramid.matches(full, quality));
        assert!(!pyramid.matches(full + UVec2::ONE, quality));
        let mut encoder = device.create_command_encoder(&default());
        pyramid.record(
            device,
            queue,
            &mut encoder,
            &pipelines,
            &source,
            exposure,
            0..2,
        );
        assert!(pyramid.levels.iter().take(2).all(|v| v.bindings.len() == 1));
        assert!(pyramid.levels.iter().skip(2).all(|v| v.bindings.is_empty()));
        queue.submit([encoder.finish()]);
        let mut encoder = device.create_command_encoder(&default());
        pyramid.record(
            device,
            queue,
            &mut encoder,
            &pipelines,
            &source,
            exposure,
            2..pyramid.level_count(),
        );
        let mut comparisons = Vec::new();
        let mut offset = 0;
        for level in 0..pyramid.level_count() {
            let extent = pyramid.extent(level).unwrap();
            let header: Vec<_> = lines.next().unwrap().split_whitespace().collect();
            assert_eq!(header[0], "LEVEL");
            assert_eq!(header[1].parse::<usize>().unwrap(), level);
            assert_eq!(
                extent,
                UVec2::new(header[2].parse().unwrap(), header[3].parse().unwrap())
            );
            let pitch = (extent.x * 8).div_ceil(256) * 256;
            let expected: Vec<_> = (0..extent.x * extent.y)
                .map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|v| u16::from_str_radix(v, 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect();
            comparisons.push((level, extent, pitch, offset, expected));
            offset += u64::from(pitch * extent.y);
        }
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: offset,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for (level, extent, pitch, offset, _) in &comparisons {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: pyramid.texture(),
                    mip_level: *level as u32,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: *offset,
                        bytes_per_row: Some(*pitch),
                        rows_per_image: Some(extent.y),
                    },
                },
                Extent3d {
                    width: extent.x,
                    height: extent.y,
                    depth_or_array_layers: 1,
                },
            );
        }
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
        for (level, extent, pitch, offset, expected) in comparisons {
            for (pixel, values) in expected.iter().enumerate() {
                for (channel, &expected) in values.iter().enumerate() {
                    let at = offset as usize
                        + (pixel / extent.x as usize) * pitch as usize
                        + (pixel % extent.x as usize) * 8
                        + channel * 2;
                    let actual = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
                    let distance = actual.abs_diff(expected);
                    maximum = maximum.max(distance);
                    assert!(distance<=1,"frame{frames} level{level} pixel{pixel} channel{channel}: {actual:04x} != {expected:04x}, {distance} half steps");
                    channels += 1;
                }
            }
        }
        drop(bytes);
        staging.unmap();
        frames += 1;
    }
    assert_eq!(frames, 4);
    eprintln!("{frames} luminance chains / {channels} channels, maximum {maximum} half steps");
}
