use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; actual installed vertex/pixel blur and queued pairs"]
fn near_mask_blur_matches_installed_shader() {
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
    let descriptor = TextureDescriptor {
        label: Some("near-mask blur fixture"),
        size: Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::R16Float,
        usage: TextureUsages::TEXTURE_BINDING
            | TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let pixels: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                half::f16::from_f32(((x * 7 + y * 3) % 19) as f32 / 8.)
                    .to_bits()
                    .to_le_bytes()
            })
        })
        .collect();
    let source =
        device.create_texture_with_data(queue, &descriptor, TextureDataOrder::MipMajor, &pixels);
    let source_view = source.create_view(&default());
    let temporary = device.create_texture(&descriptor);
    let temporary_view = temporary.create_view(&default());
    let output = device.create_texture(&descriptor);
    let output_view = output.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("near-mask blur readback"),
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut passes = CameraMaskBlurPass::pair(device, TextureFormat::R16Float);
    let mut lines = include_str!("../../../tests/fixtures/camera_mask_blur.txt").lines();
    let mut draws = 0;
    let mut channels = 0;
    let mut maximum = 0;
    while let Some(header) = lines.next() {
        let words: Vec<_> = header.split_whitespace().collect();
        let pair = words[0] == "PAIR";
        let gain = words[if pair { 1 } else { 2 }].parse().unwrap();
        let parameters = NearMaskBlur::separable([8, 8], gain).unwrap();
        let hex = if pair {
            words[2]
        } else {
            lines.next().unwrap().strip_prefix("STORE ").unwrap()
        };
        let expected: Vec<_> = (0..64)
            .map(|i| {
                u16::from_le_bytes(std::array::from_fn(|j| {
                    u8::from_str_radix(&hex[i * 4 + j * 2..i * 4 + j * 2 + 2], 16).unwrap()
                }))
            })
            .collect();
        let mut encoder = device.create_command_encoder(&default());
        if pair {
            passes[0].record(
                device,
                queue,
                &mut encoder,
                &parameters[0],
                [1., 1.],
                &source_view,
                &temporary_view,
            );
            passes[1].record(
                device,
                queue,
                &mut encoder,
                &parameters[1],
                [1., 1.],
                &temporary_view,
                &output_view,
            );
            draws += 2;
        } else {
            let axis: usize = words[1].parse().unwrap();
            passes[axis].record(
                device,
                queue,
                &mut encoder,
                &parameters[axis],
                [1., 1.],
                &source_view,
                &output_view,
            );
            draws += 1;
        }
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            descriptor.size,
        );
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
        for pixel in 0..64 {
            let at = pixel / 8 * 256 + pixel % 8 * 2;
            let actual = u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
            let distance = actual.abs_diff(expected[pixel]);
            maximum = maximum.max(distance);
            assert!(
                distance <= if pair { 2 } else { 1 },
                "draw{draws} pixel{pixel} pair{pair}: {actual:04x} != {:04x}, {distance} half steps",
                expected[pixel]
            );
            channels += 1;
        }
        drop(data);
        staging.unmap();
    }
    assert_eq!(draws, 14);
    assert_eq!(channels, 704);
    eprintln!(
        "PASS near-mask blur: {draws} native draws, {channels} channels, maximum {maximum} half steps; H/V queued together"
    );
}
