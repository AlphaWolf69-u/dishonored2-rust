use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; actual R16F history and point-sampled curves"]
fn fog_lookup_matches_installed_compute_programs() {
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
    let targets: Vec<_> = [7, 33, 1024, 16]
        .into_iter()
        .enumerate()
        .map(|(i, width)| {
            let height = if i == 3 { 16 } else { 1 };
            let bytes: Vec<u8> = if i == 3 {
                (0..height)
                    .flat_map(|y| {
                        (0..width).flat_map(move |x| {
                            (0..4).map(move |c| ((x * 13 + y * 7 + c * 17) % 256) as u8)
                        })
                    })
                    .collect()
            } else {
                (0..width)
                    .flat_map(|x| {
                        half::f16::from_f32(((x * 13 + i as u32 * 7) % 97) as f32 / 64.)
                            .to_le_bytes()
                    })
                    .collect()
            };
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("synthetic fog curve target"),
                    size: Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: if i == 3 {
                        TextureFormat::Rgba8Unorm
                    } else {
                        TextureFormat::R16Float
                    },
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &bytes,
            )
        })
        .collect();
    let views: Vec<_> = targets
        .iter()
        .map(|texture| texture.create_view(&default()))
        .collect();
    let owner = FogLookupPass::new(device, queue, 1.);
    assert!(FogLookupPass::shared(device, &targets[0], 1.).is_err());
    let mut curves = FogLookupPass::shared(device, &owner.curves[0].texture, 1.).unwrap();
    assert_eq!(curves.texture_id(), owner.texture_id());
    let array_id = curves.array_view().id();
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("fog curve numeric readback"),
        size: 16384,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut lines = include_str!("../../../../tests/fixtures/fog_lookup_blending.txt").lines();
    let mut frames = 0;
    let mut maximum = 0;
    while let Some(header) = lines.next() {
        let header: Vec<_> = header.split_whitespace().collect();
        assert_eq!(header.len(), 8);
        assert_eq!(header[0], "FRAME");
        assert_eq!(header[1].parse::<usize>().unwrap(), frames);
        let target = header[2].parse::<usize>().unwrap();
        let float = |text: &str| f32::from_bits(u32::from_str_radix(text, 16).unwrap());
        let weight = float(header[3]);
        let weights = Vec4::from_array(std::array::from_fn(|i| float(header[4 + i])));
        let mut encoder = device.create_command_encoder(&default());
        let plan = curves.record(
            device,
            queue,
            &mut encoder,
            &views[target],
            weight,
            Some(FogLookupOverrides {
                views: [&views[0], &views[1], &views[2], &views[3]],
                weights,
            }),
        );
        for (plane, index) in [plan.output, plan.final_image].into_iter().enumerate() {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &curves.curves[index].texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: curves.curves[index].layer,
                    },
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: plane as u64 * 8192,
                        bytes_per_row: Some(8192),
                        rows_per_image: Some(1),
                    },
                },
                Extent3d {
                    width: 4096,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for plane in 0..2 {
            let expected: Vec<_> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|word| u16::from_str_radix(word, 16).unwrap())
                .collect();
            assert_eq!(expected.len(), 4096);
            for (x, expected) in expected.into_iter().enumerate() {
                let at = plane * 8192 + x * 2;
                let actual = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
                // Synthetic outputs are nonnegative, so adjacent half words
                // measure one representable storage step without sign folding.
                assert_eq!(actual & 0x8000, 0, "frame{frames} plane{plane} x{x}");
                assert_eq!(expected & 0x8000, 0);
                let error = actual.abs_diff(expected);
                maximum = maximum.max(error);
                assert!(
                    error <= 1,
                    "frame{frames} plane{plane} x{x}: {actual:04x} != {expected:04x}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
        assert_eq!(
            curves.view().id(),
            curves.curves[plan.final_image].view.id()
        );
        assert_eq!(curves.array_view().id(), array_id);
        assert_eq!(curves.texture_id(), owner.texture_id());
        frames += 1;
    }
    assert_eq!(frames, 8);
    println!(
        "PASS8 fog-curve frames / 65536 base+final half words; maximum storage steps {maximum}"
    );
}
