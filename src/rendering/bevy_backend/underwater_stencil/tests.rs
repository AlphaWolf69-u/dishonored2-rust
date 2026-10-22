use super::*;
use bevy::render::RenderApp;

#[test]
#[ignore = "requires GPU; tests both underwater shader branches and full/reduced stencil/depth preservation"]
fn underwater_marking_preserves_depth_and_other_stencil_bits() {
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
    let state = crate::render_state::parse_render_state(
        "{ depthEnable 0 depthfunc ALWAYS cull none }",
        None,
    )
    .unwrap();
    let format = TextureFormat::Depth32FloatStencil8;
    let mut renderer = UnderwaterStencilPass::new(device, format, state).unwrap();
    assert!(UnderwaterStencilPass::new(device, TextureFormat::Depth32Float, state).is_err());
    // Independent original frame6387 CreateDepthStencilState chunk1502/p85204.
    let native = crate::render_state::depth_stencil_state(
        &crate::render_state::underwater_stencil_state(state),
    );
    assert!(native.depth_enabled && !native.depth_write && native.stencil_enabled);
    assert_eq!(
        (
            native.depth_compare,
            native.stencil_read_mask,
            native.stencil_write_mask
        ),
        (8, 2, 64)
    );
    for face in [native.front, native.back] {
        assert_eq!(
            (face.fail, face.depth_fail, face.pass, face.compare),
            (1, 1, 3, 6)
        );
    }
    let targets = [8, 4].map(|size| {
        device.create_texture(&TextureDescriptor {
            label: Some("underwater stencil numeric target"),
            size: Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    });
    let attachments = targets.each_ref().map(|t| t.create_view(&default()));
    let mask_values = [0u16, 0x3400, 0x3800, 0x3a00, 0x3c00, 0x3a00, 0x3800, 0x3400];
    let mask = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("controlled water-line coverage"),
            size: Extent3d {
                width: 8,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R16Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &mask_values
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mask = mask.create_view(&default());
    let positions = [
        [-1f32, 1., 0., 0., 0.],
        [3., 1., 0., 2., 0.],
        [-1., -3., 0., 0., 2.],
    ];
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("underwater source clip triangle"),
        contents: &positions
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 6144,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let coverage = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/underwater_stencil_coverage.txt"
    ))
    .split_whitespace()
    .map(|word| word.parse::<u8>().unwrap())
    .collect::<Vec<_>>();
    assert_eq!(coverage.len(), 160);
    let mut pixels = 0;
    for volume in [false, true] {
        let mut binding_ids = None;
        for initial in [0u8, 2, 4, 6, 64, 66, 68, 70] {
            let mut encoder = device.create_command_encoder(&default());
            for attachment in &attachments {
                let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("initialize independent numeric depth/stencil"),
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: attachment,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(0.375),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: Some(Operations {
                            load: LoadOp::Clear(u32::from(initial)),
                            store: StoreOp::Store,
                        }),
                    }),
                    ..default()
                });
            }
            let parameters = [8, 4].map(|size| UnderwaterStencilParameters {
                world_from_clip: Mat4::IDENTITY,
                water_matrix: if size == 8 {
                    Mat4::from_cols(
                        Vec4::new(1., 0., 0.02, 0.),
                        Vec4::Y,
                        Vec4::Z,
                        Vec4::new(0., 0., 0.01, 1.),
                    )
                } else {
                    Mat4::from_cols(
                        Vec4::X,
                        Vec4::new(0., 1., -0.02, 0.),
                        Vec4::Z,
                        Vec4::new(0., 0., 0.02, 1.),
                    )
                },
                viewport: Vec4::new(0., 0., size as f32, size as f32),
                resolution_scale: if size == 8 {
                    Vec4::ONE
                } else {
                    Vec4::new(0.5, 1., 1., 1.)
                },
            });
            renderer
                .record(
                    device,
                    queue,
                    &mut encoder,
                    if volume {
                        UnderwaterStencilSource::Volume
                    } else {
                        UnderwaterStencilSource::Ocean(&mask)
                    },
                    [
                        UnderwaterStencilTarget {
                            attachment: &attachments[0],
                            parameters: parameters[0],
                        },
                        UnderwaterStencilTarget {
                            attachment: &attachments[1],
                            parameters: parameters[1],
                        },
                    ],
                    &vertices,
                    0..3,
                )
                .unwrap();
            let ids = renderer
                .bindings
                .each_ref()
                .map(|b| b.as_ref().unwrap().1.id());
            if let Some(previous) = binding_ids {
                assert_eq!(
                    ids, previous,
                    "unchanged inputs rebuilt underwater bindings"
                );
            }
            binding_ids = Some(ids);
            let mut offset = 0;
            for target in &targets {
                for aspect in [TextureAspect::DepthOnly, TextureAspect::StencilOnly] {
                    encoder.copy_texture_to_buffer(
                        TexelCopyTextureInfo {
                            texture: target,
                            mip_level: 0,
                            origin: Origin3d::ZERO,
                            aspect,
                        },
                        TexelCopyBufferInfo {
                            buffer: &staging,
                            layout: TexelCopyBufferLayout {
                                offset,
                                bytes_per_row: Some(256),
                                rows_per_image: Some(target.height()),
                            },
                        },
                        target.size(),
                    );
                    offset += u64::from(target.height()) * 256;
                }
            }
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            let mut offset = 0;
            for (index, size) in [8usize, 4].into_iter().enumerate() {
                for y in 0..size {
                    for x in 0..size {
                        let at = offset + y * 256 + x * 4;
                        assert_eq!(
                            f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                            0.375,
                            "underwater mark changed scene depth"
                        );
                        let covered = coverage[usize::from(volume) * 80
                            + if index == 0 { 0 } else { 64 }
                            + y * size
                            + x]
                            != 0;
                        let expected = if covered && initial & 2 != 2 {
                            initial | 64
                        } else {
                            initial
                        };
                        let actual = bytes[offset + size * 256 + y * 256 + x];
                        assert_eq!(
                            actual, expected,
                            "volume={volume} initial={initial} target={index} x={x} y={y}"
                        );
                        pixels += 1;
                    }
                }
                offset += size * 256 * 2;
            }
            drop(bytes);
            staging.unmap();
        }
    }
    println!("{pixels} original-rule underwater stencil/depth pixels; both source branches and target sizes");
}
