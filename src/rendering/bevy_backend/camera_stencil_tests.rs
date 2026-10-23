use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; original camera classifier shaders and native state fixtures"]
fn camera_stencil_matches_installed_shaders() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                        ..default()
                    },
                )),
                ..default()
            })
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
    let kinds = [
        CameraStencilKind::Near,
        CameraStencilKind::Far,
        CameraStencilKind::Autofocus,
        CameraStencilKind::Constant,
    ];
    let format = TextureFormat::Depth32FloatStencil8;
    let mut passes: Vec<_> = kinds
        .into_iter()
        .enumerate()
        .map(|(i, k)| {
            CameraStencilPass::new(
                device,
                k,
                format,
                crate::render_state::camera_blur_stencil_state(i != 0),
            )
            .unwrap()
        })
        .collect();
    let mask_bytes: Vec<_> = (0..8)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [
                    [0, 12, 13, 26, 128, 255, 1, 64][(x + 2 * y) % 8],
                    if (x + 3 * y) % 3 == 0 {
                        0
                    } else {
                        [1, 64, 255][(x + y) % 3]
                    },
                ]
            })
        })
        .collect();
    let mask = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("camera stencil controlled RG8 mask"),
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &mask_bytes,
    );
    let mask = mask.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 9 * 256 * 2,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut lines = include_str!("../../../tests/fixtures/camera_stencil.txt").lines();
    let mut cases = 0;
    let mut channels = 0;
    let mut marked = [0usize; 4];
    let mut skipped = [0usize; 4];
    while let Some(header) = lines.next() {
        let words: Vec<_> = header.split_whitespace().collect();
        let u = |i: usize| words[i].parse::<u32>().unwrap();
        let f = |i: usize| words[i].parse::<f32>().unwrap();
        let kind = u(1) as usize;
        let w = u(2);
        let h = u(3);
        let initial = u(4);
        let depth = f(5);
        let focus = f(6);
        let scale = [f(7), f(8)];
        let viewport = [u(9), u(10), u(11), u(12)];
        let scissor = [u(13), u(14), u(15), u(16)];
        let decode = |hex: &str| {
            (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect::<Vec<_>>()
        };
        let packet = decode(words[17]);
        let float = |i: usize| f32::from_le_bytes(packet[4 * i..4 * i + 4].try_into().unwrap());
        let parameters = CameraStencilParameters {
            projection: std::array::from_fn(|c| std::array::from_fn(|r| float(c * 4 + r))),
            near_clip: float(16),
            clip_distance: float(17),
        };
        assert_eq!(parameters.uniform_bytes().as_slice(), packet);
        let state: Vec<_> = lines
            .next()
            .unwrap()
            .split_whitespace()
            .skip(1)
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        assert_eq!(
            crate::render_state::camera_blur_stencil_state(kind != 0).as_slice(),
            state
        );
        let expected = decode(lines.next().unwrap().strip_prefix("STENCIL ").unwrap());
        assert_eq!(expected.len(), (w * h) as usize);
        let target = device.create_texture(&TextureDescriptor {
            label: Some("camera stencil numeric attachment"),
            size: Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let attachment = target.create_view(&default());
        let focus = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("camera autofocus numeric distance"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba32Float,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &[focus, 0., 0., 1.]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let focus = focus.create_view(&default());
        let points = [
            [-1f32, 1., 0., 0., 0.],
            [3., 1., 0., 2. * scale[0], 0.],
            [-1., -3., 0., 0., 2. * scale[1]],
        ];
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &points
                .into_iter()
                .flatten()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            usage: BufferUsages::VERTEX,
        });
        let mut encoder = device.create_command_encoder(&default());
        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &attachment,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(depth),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(initial),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        let source = match kind {
            0 => CameraStencilSource::Near(&mask),
            1 => CameraStencilSource::Far(&mask),
            2 => CameraStencilSource::Autofocus {
                mask: &mask,
                focus: &focus,
            },
            _ => CameraStencilSource::Constant,
        };
        passes[kind]
            .record(
                device,
                queue,
                &mut encoder,
                &parameters,
                source,
                CameraStencilTarget {
                    attachment: &attachment,
                    viewport,
                    scissor,
                    stencil_reference: if kind == 0 { 4 } else { 8 },
                },
                &vertices,
                0..3,
            )
            .unwrap();
        for (plane, aspect) in [TextureAspect::DepthOnly, TextureAspect::StencilOnly]
            .into_iter()
            .enumerate()
        {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: plane as u64 * 9 * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(h),
                    },
                },
                target.size(),
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for y in 0..h as usize {
            for x in 0..w as usize {
                let at = y * 256 + x * 4;
                assert_eq!(
                    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                    depth,
                    "classification changed depth in case{cases}"
                );
                let actual = bytes[9 * 256 + y * 256 + x];
                let want = expected[y * w as usize + x];
                assert_eq!(actual, want, "case{cases} kind{kind} x{x} y{y}");
                if u32::from(actual) != initial {
                    marked[kind] += 1;
                } else {
                    skipped[kind] += 1;
                }
                channels += 2;
            }
        }
        drop(bytes);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 32);
    for kind in 0..4 {
        assert!(
            marked[kind] > 0 && skipped[kind] > 0,
            "classifier{kind} did not exercise both admission outcomes"
        );
    }
    eprintln!("PASS {cases} native-state classifier cases, {channels} depth/stencil channels; changed{marked:?} retained{skipped:?}");
}
