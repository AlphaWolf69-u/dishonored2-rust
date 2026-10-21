use super::*;
use bevy::{prelude::*, render::RenderApp};

fn pixels(index: u32) -> Vec<u8> {
    (0..32)
        .flat_map(|z| {
            (0..32).flat_map(move |y| {
                (0..32).flat_map(move |x| {
                    [
                        (x * 17 + y * 3 + z * 7 + index * 37) % 256,
                        (x * 5 + y * 19 + z * 11 + index * 29) % 256,
                        (x * 13 + y * 7 + z * 23 + index * 41) % 256,
                        (x + y + z + index * 17) % 256,
                    ]
                    .map(|v| v as u8)
                })
            })
        })
        .collect()
}

#[test]
#[ignore = "requires graphics adapter; RGBA8 cube history and shader-backed overrides"]
fn colour_cube_history_matches_installed_compute_programs() {
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
    let originals: Vec<_> = (0..4)
        .map(|index| {
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("synthetic authored colour cube"),
                    size: Extent3d {
                        width: 32,
                        height: 32,
                        depth_or_array_layers: 32,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D3,
                    format: if index % 2 == 0 {
                        TextureFormat::Rgba8UnormSrgb
                    } else {
                        TextureFormat::Rgba8Unorm
                    },
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &pixels(index),
            )
        })
        .collect();
    let views: Vec<_> = originals
        .iter()
        .map(|t| t.create_view(&default()))
        .collect();
    let mut cubes = ColourCubePass::new(device, queue);
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 32 * 32,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut lines = include_str!("../../../../tests/fixtures/colour_cube.txt").lines();
    let mut frames = 0;
    let mut maximum = 0;
    while let Some(header) = lines.next() {
        let h: Vec<_> = header.split_whitespace().collect();
        assert_eq!(h.len(), 8);
        assert_eq!(h[0], "FRAME");
        assert_eq!(h[1].parse::<usize>().unwrap(), frames);
        let bits = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
        let target = h[2].parse::<usize>().unwrap();
        let weight = bits(h[3]);
        let weights = Vec4::from_array(std::array::from_fn(|i| bits(h[4 + i])));
        let mut encoder = device.create_command_encoder(&default());
        cubes.record(
            device,
            queue,
            &mut encoder,
            &views[target],
            weight,
            Some(CubeOverrides {
                views: [&views[0], &views[1], &views[2], &views[3]],
                weights,
            }),
        );
        encoder.copy_texture_to_buffer(
            cubes.cubes[cubes.final_index].texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(32),
                },
            },
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 32,
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
        let bytes = staging.slice(..).get_mapped_range();
        for sample in 0..96 {
            let v: Vec<usize> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|s| s.parse().unwrap())
                .collect();
            assert_eq!(v.len(), 7);
            let at = (v[2] * 32 + v[1]) * 256 + v[0] * 4;
            for c in 0..4 {
                let error = (i32::from(bytes[at + c]) - v[3 + c] as i32).abs();
                maximum = maximum.max(error);
                assert!(
                    error <= 1,
                    "frame{frames} sample{sample} channel{c}: {} != {}",
                    bytes[at + c],
                    v[3 + c]
                );
            }
        }
        drop(bytes);
        staging.unmap();
        frames += 1;
    }
    assert_eq!(frames, 12);
    eprintln!("12 colour-cube frames / 4608 channel checks, max byte error {maximum}");
}
