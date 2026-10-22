use super::super::tone_mapping::{ToneMappingParameters, ToneMappingPass};
use super::*;
use crate::rendering::tone_mapping::{ToneCurve, ToneSettings};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

fn pixels(image: usize, host: f32) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let values = match image {
                0 => [
                    (x as f32 - 2.0) / 64.0,
                    (y + 1) as f32 / 64.0,
                    ((x * 3 + y * 5) % 13) as f32 / 64.0,
                    1.0,
                ],
                1 => [
                    ((x * 5 + y * 3) % 17) as f32 / 256.0,
                    ((x * 7 + y) % 19) as f32 / 256.0,
                    ((x + y * 11) % 23) as f32 / 256.0,
                    1.0,
                ],
                4 => [
                    ((x * 7 + y * 2) % 11) as f32 / 128.0,
                    ((x + y * 3) % 17) as f32 / 128.0,
                    ((x * 5 + y) % 13) as f32 / 128.0,
                    1.0,
                ],
                _ => [
                    ((x * 3 + y * 5 + image * 7) % 19) as f32 / 16.0,
                    ((x * 7 + y + image * 3) % 17) as f32 / 16.0,
                    ((x + y * 11 + image * 5) % 23) as f32 / 16.0,
                    1.0,
                ],
            };
            for value in values {
                if image == 0 {
                    data.extend((value * host).to_le_bytes());
                } else {
                    data.extend(half::f16::from_f32(value).to_bits().to_le_bytes());
                }
            }
        }
    }
    data
}
#[test]
#[ignore = "requires GPU; actual fused display against installed composition HLSL"]
fn fused_lens_display_matches_installed_composition() {
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
    let image = |format, usage, data: Option<Vec<u8>>| {
        let descriptor = TextureDescriptor {
            label: None,
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        if let Some(data) = data {
            device.create_texture_with_data(queue, &descriptor, TextureDataOrder::MipMajor, &data)
        } else {
            device.create_texture(&descriptor)
        }
    };
    let hdr = image(
        TextureFormat::Rgba32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        None,
    );
    let hdr_view = hdr.create_view(&default());
    let images: Vec<_> = (1..5)
        .map(|i| {
            image(
                TextureFormat::Rgba16Float,
                TextureUsages::TEXTURE_BINDING,
                Some(pixels(i, 1.0)),
            )
        })
        .collect();
    let views: Vec<_> = images.iter().map(|i| i.create_view(&default())).collect();
    let poison: Vec<_> = (0..64)
        .flat_map(|_| {
            [f32::NAN, f32::NAN, f32::NAN, 1.0]
                .into_iter()
                .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
        })
        .collect();
    let poison = image(
        TextureFormat::Rgba16Float,
        TextureUsages::TEXTURE_BINDING,
        Some(poison),
    )
    .create_view(&default());
    let cube = device
        .create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D3,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &[0, 0, 0, 255],
        )
        .create_view(&default());
    let target = image(
        TextureFormat::Rgba32Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        None,
    );
    let target_view = target.create_view(&default());
    let exposure = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 32,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 2048,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut pass = ToneMappingPass::new_with_lens(device, TextureFormat::Rgba32Float);
    let curve = ToneCurve::new(ToneSettings {
        overbright: 1.0,
        black: 0.0,
        white: 1.0,
        junction: 0.18,
        toe: 0.5,
        shoulder: 0.5,
    });
    let mut lines = include_str!("../../../../tests/fixtures/lens_composite.txt").lines();
    let mut frames = 0;
    let mut channels = 0;
    let mut maximum = 0.0_f32;
    while let Some(line) = lines.next() {
        let packet = line.strip_prefix("FRAME ").unwrap();
        let words: Vec<_> = (0..32)
            .map(|i| {
                u32::from_le_bytes(std::array::from_fn(|j| {
                    u8::from_str_radix(&packet[i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
                }))
            })
            .collect();
        let floats = |start| std::array::from_fn(|i| f32::from_bits(words[start + i]));
        let parameters = LensCompositeParameters {
            viewport: floats(0),
            pixel: floats(4),
            controls: floats(8),
            streak: floats(12),
            tint: floats(16),
            flags: words[20..24].try_into().unwrap(),
            options: words[24..28].try_into().unwrap(),
        };
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &hdr,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &pixels(0, parameters.pixel[3]),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(128),
                rows_per_image: Some(8),
            },
            extent,
        );
        let mut adaptation = [0; 32];
        adaptation[0..4].copy_from_slice(&words[28].to_le_bytes());
        queue.write_buffer(&exposure, 0, &adaptation);
        let tone = ToneMappingParameters {
            curve,
            gamma: 1.0,
            brightness: 1.0,
            exposure: f32::from_bits(words[29]) / parameters.pixel[3],
            use_curve: false,
            // The BW program must bypass colour grading even with a resident cube.
            use_colour_cube: parameters.flags[3] != 0,
        };
        let lens = LensCompositeInputs {
            parameters: &parameters,
            bloom: if parameters.tint[3] > 0.0 {
                &poison
            } else {
                &views[0]
            },
            dirt_from: &views[1],
            dirt_to: &views[2],
            flares: &views[3],
        };
        let mut encoder = device.create_command_encoder(&default());
        pass.record_with_lens(
            device,
            queue,
            &mut encoder,
            &tone,
            &hdr_view,
            &cube,
            &exposure,
            &target_view,
            &lens,
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
        for pixel in 0..64 {
            for (channel, raw) in lines.next().unwrap().split_whitespace().enumerate() {
                let expected = f32::from_bits(u32::from_str_radix(raw, 16).unwrap());
                let at = pixel / 8 * 256 + pixel % 8 * 16 + channel * 4;
                let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                let error = (actual - expected).abs();
                maximum = maximum.max(error);
                assert!(
                    actual.is_finite() && error < 3e-5 * (1.0 + expected.abs()),
                    "frame{frames} pixel{pixel} channel{channel}: {actual} != {expected}"
                );
                channels += 1;
            }
        }
        drop(bytes);
        staging.unmap();
        frames += 1;
    }
    assert_eq!((frames, channels), (42, 10752));
    eprintln!(
        "Fused lens display: {frames} native frames, {channels} channels; maximum error {maximum}"
    );
}
