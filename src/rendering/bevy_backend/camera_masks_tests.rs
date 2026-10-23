use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; composed camera masks against installed raster shaders"]
fn camera_mask_owner_matches_installed_chain() {
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
    let depth_view = depth.create_view(&default());
    let pixels: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [3. + (x % 3) as f32, 6. + (y % 3) as f32, 0., 1.]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
        })
        .collect();
    let focus = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("camera-mask focus fixture"),
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::MipMajor,
        &pixels,
    );
    let focus_view = focus.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("camera-mask chain comparison"),
        size: 256 * 16,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut owner: Option<CameraMasks> = None;
    let mut lines = include_str!("../../../tests/fixtures/camera_masks.txt").lines();
    let mut cases = 0;
    let mut channels = 0;
    let mut maximum_half = 0;
    let mut maximum_byte = 0;
    while let Some(header) = lines.next() {
        let words: Vec<_> = header.split_whitespace().collect();
        let extent = [words[1].parse().unwrap(), words[2].parse().unwrap()];
        let gain = words[3].parse().unwrap();
        let scale = words[4].parse().unwrap();
        let raw = words[5];
        let settings = CameraSettings::from_words(std::array::from_fn(|i| {
            u32::from_le_bytes(std::array::from_fn(|j| {
                u8::from_str_radix(&raw[i * 8 + j * 2..i * 8 + j * 2 + 2], 16).unwrap()
            }))
        }));
        let region: Vec<u32> = lines
            .next()
            .unwrap()
            .strip_prefix("REGION ")
            .unwrap()
            .split_whitespace()
            .map(|n| n.parse().unwrap())
            .collect();
        let decode = |text: &str| {
            (0..text.len() / 2)
                .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
                .collect::<Vec<_>>()
        };
        let expected_near = decode(lines.next().unwrap().strip_prefix("NEAR ").unwrap());
        let expected_far = decode(lines.next().unwrap().strip_prefix("FAR ").unwrap());
        if owner.as_ref().is_none_or(|v| !v.matches(extent)) {
            owner = Some(CameraMasks::new(device, extent).unwrap());
        }
        let owner = owner.as_mut().unwrap();
        let mut encoder = device.create_command_encoder(&default());
        owner.record(
            device,
            queue,
            &mut encoder,
            CameraMaskInputs {
                settings: &settings,
                alpha_depth: &depth_view,
                autofocus: &focus_view,
                inverse_z: [100., 0.001, -1.],
                resolution_scale: [scale, scale],
                near_mask_gain: gain,
                far_clip: 7000.,
                near_viewport: region[0..4].try_into().unwrap(),
                near_scissor: region[4..8].try_into().unwrap(),
                far_viewport: region[8..12].try_into().unwrap(),
                far_scissor: region[12..16].try_into().unwrap(),
            },
        );
        for (plane, texture) in [owner.near_texture(), owner.far_texture()]
            .into_iter()
            .enumerate()
        {
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: plane as u64 * 256 * 8,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(texture.height()),
                    },
                },
                Extent3d {
                    width: texture.width(),
                    height: texture.height(),
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        staging.slice(..).map_async(MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = staging.slice(..).get_mapped_range();
        let near = owner.near_texture();
        for y in 0..near.height() as usize {
            for x in 0..near.width() as usize {
                let at = y * 256 + x * 2;
                let expected = (y * near.width() as usize + x) * 2;
                let actual = u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
                let want =
                    u16::from_le_bytes(expected_near[expected..expected + 2].try_into().unwrap());
                let distance = actual.abs_diff(want);
                maximum_half = maximum_half.max(distance);
                assert!(
                    distance <= 3,
                    "near case{cases} pixel{x},{y}: {actual:04x} != {want:04x}, {distance} half steps"
                );
                channels += 1;
            }
        }
        let far = owner.far_texture();
        for y in 0..far.height() as usize {
            for x in 0..far.width() as usize {
                for c in 0..2 {
                    let actual = data[256 * 8 + y * 256 + x * 2 + c];
                    let want = expected_far[(y * far.width() as usize + x) * 2 + c];
                    let distance = actual.abs_diff(want);
                    maximum_byte = maximum_byte.max(distance);
                    assert!(
                        distance <= 1,
                        "far case{cases} pixel{x},{y} channel{c}: {actual} != {want}"
                    );
                    channels += 1;
                }
            }
        }
        drop(data);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 24);
    eprintln!(
        "PASS {cases} ordered camera-mask frames /{channels} channels, near {maximum_half} half steps, far {maximum_byte} bytes; flags, autofocus, clipping and target resize"
    );
}
