use super::*;
use bevy::{prelude::*, render::RenderApp};
#[test]
#[ignore = "requires GPU; actual autofocus pass against installed compute execution"]
fn autofocus_matches_game_shader() {
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
    let descriptor = |width, height, format, usage| TextureDescriptor {
        label: None,
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    };
    let mut depth = Vec::new();
    for y in 0..16 {
        for x in 0..16 {
            let bits = [
                0x3800u16, 0x3c00, 0x4400, 0x4800, 0x4c00, 0x5000, 0x5800, 0x6000,
            ][(x * 3 + y * 5) % 8];
            depth.extend(bits.to_le_bytes());
        }
    }
    let depth = device.create_texture_with_data(
        queue,
        &descriptor(
            16,
            16,
            TextureFormat::R16Float,
            TextureUsages::TEXTURE_BINDING,
        ),
        TextureDataOrder::MipMajor,
        &depth,
    );
    let previous = device.create_texture(&descriptor(
        1,
        1,
        TextureFormat::Rg32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
    ));
    let current = device.create_texture(&descriptor(
        1,
        1,
        TextureFormat::Rg32Float,
        TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
    ));
    let depth_view = depth.create_view(&default());
    let previous_view = previous.create_view(&default());
    let current_view = current.create_view(&default());
    let read = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut pass = AutofocusPass::new(device);
    let mut count = 0;
    let mut maximum = 0f32;
    for row in include_str!("../../../tests/fixtures/autofocus.txt")
        .lines()
        .filter(|s| !s.starts_with('#'))
    {
        let words: Vec<u32> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 22);
        let f = |i| f32::from_bits(words[i]);
        let parameters = AutofocusParameters {
            centre: [f(0), f(1)],
            resolution_scale: [f(2), f(3)],
            texel: [f(4), f(5)],
            radius: f(6),
            far_clip: f(7),
            speeds: [f(8), f(9), f(10), f(11)],
            local_delta: f(13),
        };
        let previous_bytes: Vec<u8> = words[16..18].iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_texture(
            previous.as_image_copy(),
            &previous_bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
            },
            previous.size(),
        );
        let mut encoder = device.create_command_encoder(&default());
        pass.record(
            device,
            queue,
            &mut encoder,
            parameters,
            &depth_view,
            &previous_view,
            &current_view,
        )
        .unwrap();
        encoder.copy_texture_to_buffer(
            current.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &read,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            current.size(),
        );
        let id = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(id),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = read.slice(..).get_mapped_range();
        for i in 0..2 {
            let actual = f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
            let expected = f(18 + i);
            let error = (actual - expected).abs();
            maximum = maximum.max(error);
            assert!(
                actual.is_finite() && error < 3e-5 * (1. + expected.abs()),
                "case{count} lane{i}: {actual} != {expected}"
            );
        }
        drop(bytes);
        read.unmap();
        count += 1;
    }
    assert_eq!(count, 128);
    println!("{count} autofocus states, max absolute error {maximum}");
}
