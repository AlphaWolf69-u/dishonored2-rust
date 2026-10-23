use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; installed-source raster and RG8 stores"]
fn camera_masks_match_installed_shader() {
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
    let depth: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).map(move |x| [0.0001, 0.0004, 0.001, 0.005, 0.02, 0.1, 0.5, 1.][(x + 3 * y) % 8])
        })
        .collect();
    let depth =
        super::super::test_depth::DepthUploader::new(device).upload(device, queue, 8, 8, 1, depth);
    let make_image = |format, bytes: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("camera mask fixture"),
                size: Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            bytes,
        )
    };
    let near: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [((x * 3 + y * 7) % 11) as f32 / 10., 0., 0., 1.]
                    .into_iter()
                    .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
            })
        })
        .collect();
    let focus: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [3. + (x % 3) as f32, 6. + (y % 3) as f32, 0., 1.]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
        })
        .collect();
    let near = make_image(TextureFormat::Rgba16Float, &near);
    let focus = make_image(TextureFormat::Rgba32Float, &focus);
    let images = [
        depth.create_view(&default()),
        near.create_view(&default()),
        focus.create_view(&default()),
    ];
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("camera mask comparison"),
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut channels = 0;
    let mut maximum = 0f32;
    let mut store_distance = 0;
    for format in [
        TextureFormat::Rgba32Float,
        TextureFormat::Rg8Unorm,
        TextureFormat::R16Float,
    ] {
        let target = device.create_texture(&TextureDescriptor {
            label: Some("camera mask target"),
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&default());
        let mut pass = CameraFuzzinessPass::new(device, format);
        let mut lines = include_str!("../../../tests/fixtures/camera_fuzziness.txt").lines();
        let mut cases = 0;
        while let Some(header) = lines.next() {
            let hex = header.strip_prefix("FRAME ").unwrap();
            let packet: Vec<_> = (0..80)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            let f = |i: usize| f32::from_le_bytes(packet[i * 4..i * 4 + 4].try_into().unwrap());
            let u = |i: usize| u32::from_le_bytes(packet[i * 4..i * 4 + 4].try_into().unwrap());
            let parameters = CameraFuzzinessParameters {
                pass: match u(16) {
                    0 => DofPass::Alpha,
                    1 => DofPass::Near,
                    2 => DofPass::Far,
                    _ => unreachable!(),
                },
                near: [f(0), f(1)],
                alpha: [f(2), f(3), f(11)],
                far: [f(4), f(5), f(6), f(7)],
                inverse_z: [f(8), f(9), f(10)],
                centre: [f(12), f(13)],
                resolution_scale: [f(14), f(15)],
                autofocus: u(17) != 0,
            };
            assert_eq!(parameters.uniform_bytes().as_slice(), packet);
            let expected: Vec<_> = (0..64)
                .flat_map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
                        .collect::<Vec<_>>()
                })
                .collect();
            let store = lines.next().unwrap().strip_prefix("STORE ").unwrap();
            let store: Vec<_> = (0..128)
                .map(|i| u8::from_str_radix(&store[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            let half = lines.next().unwrap().strip_prefix("HALF ").unwrap();
            let half: Vec<_> = (0..64)
                .map(|i| {
                    u16::from_le_bytes(std::array::from_fn(|j| {
                        u8::from_str_radix(&half[i * 4 + j * 2..i * 4 + j * 2 + 2], 16).unwrap()
                    }))
                })
                .collect();
            let mut encoder = device.create_command_encoder(&default());
            pass.record(
                device,
                queue,
                &mut encoder,
                &parameters,
                CameraFuzzinessInputs {
                    alpha_depth: &images[0],
                    near_mask: &images[1],
                    autofocus: &images[2],
                },
                &view,
            );
            encoder.copy_texture_to_buffer(
                target.as_image_copy(),
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
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
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
            let data = staging.slice(..).get_mapped_range();
            for pixel in 0..64 {
                if format == TextureFormat::Rgba32Float {
                    for c in 0..4 {
                        let at = pixel / 8 * 256 + pixel % 8 * 16 + c * 4;
                        let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                        let want = expected[pixel * 4 + c];
                        let error = (actual - want).abs();
                        maximum = maximum.max(error);
                        assert!(
                            actual.is_finite() && error <= 0.00002 * want.abs().max(1.),
                            "case{cases} pixel{pixel} channel{c}: {actual} != {want}"
                        );
                        channels += 1;
                    }
                } else if format == TextureFormat::Rg8Unorm {
                    for c in 0..2 {
                        let actual = data[pixel / 8 * 256 + pixel % 8 * 2 + c];
                        let want = store[pixel * 2 + c];
                        let distance = actual.abs_diff(want);
                        store_distance = store_distance.max(distance);
                        assert!(
                            distance <= 1,
                            "RG8 case{cases} pixel{pixel} channel{c}: {actual} != {want}"
                        );
                        channels += 1;
                    }
                } else {
                    let at = pixel / 8 * 256 + pixel % 8 * 2;
                    let actual = u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
                    assert!(
                        actual.abs_diff(half[pixel]) <= 1,
                        "R16F case{cases} pixel{pixel}: {actual:04x} != {:04x}",
                        half[pixel]
                    );
                    channels += 1;
                }
            }
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 24);
    }
    assert_eq!(channels, 10752);
    eprintln!(
        "PASS 24 near/far/transparent mask cases; {channels} channels, maximum float error {maximum}, RG8 distance {store_distance}"
    );
}
