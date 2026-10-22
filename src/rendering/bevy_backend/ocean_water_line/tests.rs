//! The complete mask-to-underwater-stencil chain, compared to installed HLSL.
use super::super::underwater_stencil::UnderwaterStencilParameters;
use super::*;
use bevy::render::RenderApp;

#[test]
#[ignore = "requires GPU; original wave-height mask and full/reduced underwater coverage"]
fn wave_water_line_drives_underwater_stencil() {
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
    let mut line = OceanWaterLine::new(device, UVec2::splat(8), state).unwrap();
    let mut stencil =
        UnderwaterStencilPass::new(device, TextureFormat::Depth32FloatStencil8, state).unwrap();
    let input = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ocean_water_line_inputs.bin"
    ));
    assert_eq!(input.len(), 320);
    let upload = |format, bytes: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("controlled wave mask input"),
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
            TextureDataOrder::LayerMajor,
            bytes,
        )
    };
    let inputs = [
        upload(TextureFormat::R8Unorm, &input[..64]),
        upload(TextureFormat::R16Float, &input[64..192]),
        upload(TextureFormat::R16Float, &input[192..]),
    ];
    let views = inputs.each_ref().map(|t| t.create_view(&default()));
    let targets = [32, 16].map(|size| {
        device.create_texture(&TextureDescriptor {
            label: Some("water-line actual alpha depth"),
            size: Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32FloatStencil8,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    });
    let attachments = targets.each_ref().map(|t| t.create_view(&default()));
    let positions = [
        [-1f32, 1., 0., 0., 0.],
        [3., 1., 0., 2., 0.],
        [-1., -3., 0., 0., 2.],
    ];
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &positions
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 26624,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ocean_water_line.txt"
    ));
    let mut rows = fixture.lines();
    let mut frames = 0;
    let mut pixels = 0;
    let mut identity = None;
    let decode = |line: &str, prefix: &str| {
        line.strip_prefix(prefix)
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    while let Some(header) = rows.next() {
        let words = header
            .strip_prefix("FRAME ")
            .unwrap()
            .split_whitespace()
            .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 52);
        let parameters = WaterLineParameters {
            world_from_clip: Mat4::from_cols_array(words[..16].try_into().unwrap()),
            viewport: Vec4::from_slice(&words[16..20]),
            water_level: Vec4::from_slice(&words[20..24]),
        };
        let surface = OceanSurfaceParameters {
            spatial: words[24..28].try_into().unwrap(),
            clock_view: words[28..32].try_into().unwrap(),
            camera: words[32..36].try_into().unwrap(),
            perlin: words[36..40].try_into().unwrap(),
            perlin_foam: words[40..44].try_into().unwrap(),
            shore: words[44..48].try_into().unwrap(),
            foam: words[48..52].try_into().unwrap(),
        };
        let mask = decode(rows.next().unwrap(), "MASK ");
        let full = decode(rows.next().unwrap(), "FULL ");
        let reduced = decode(rows.next().unwrap(), "REDUCED ");
        assert_eq!((mask.len(), full.len(), reduced.len()), (64, 1024, 256));
        for initial in [4u8, 6] {
            let mut encoder = device.create_command_encoder(&default());
            for attachment in &attachments {
                let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
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
            let target_params = [32, 16].map(|size| UnderwaterStencilParameters {
                world_from_clip: parameters.world_from_clip,
                water_matrix: Mat4::IDENTITY,
                viewport: Vec4::new(0., 0., size as f32, size as f32),
                resolution_scale: Vec4::ONE,
            });
            line.record_underwater(
                &mut stencil,
                device,
                queue,
                &mut encoder,
                parameters,
                &surface,
                WaterLineInputs {
                    amplitude_mask: &views[0],
                    shore_distance: &views[1],
                    collision_height: &views[2],
                },
                [
                    UnderwaterStencilTarget {
                        attachment: &attachments[0],
                        parameters: target_params[0],
                    },
                    UnderwaterStencilTarget {
                        attachment: &attachments[1],
                        parameters: target_params[1],
                    },
                ],
                &vertices,
                0..3,
            )
            .unwrap();
            let ids = (line.texture.id(), line.binding.as_ref().unwrap().1.id());
            if let Some(prior) = identity {
                assert_eq!(ids, prior, "changing time recreated water-line resources");
            }
            identity = Some(ids);
            let mut offset = 0;
            for (texture, aspect) in [
                (line.texture(), TextureAspect::All),
                (&targets[0], TextureAspect::DepthOnly),
                (&targets[0], TextureAspect::StencilOnly),
                (&targets[1], TextureAspect::DepthOnly),
                (&targets[1], TextureAspect::StencilOnly),
            ] {
                encoder.copy_texture_to_buffer(
                    TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect,
                    },
                    TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: TexelCopyBufferLayout {
                            offset,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(texture.height()),
                        },
                    },
                    texture.size(),
                );
                offset += u64::from(texture.height()) * 256;
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
            for (pixel, expected) in mask.iter().enumerate() {
                assert_eq!(
                    bytes[pixel / 8 * 256 + pixel % 8],
                    *expected,
                    "water-line frame{frames} pixel{pixel}"
                );
                pixels += 1;
            }
            let mut offset = 2048;
            for (size, coverage) in [(32, &full), (16, &reduced)] {
                for (pixel, covered) in coverage.iter().enumerate() {
                    let x = pixel % size;
                    let y = pixel / size;
                    let at = offset + y * 256 + x * 4;
                    assert_eq!(
                        f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                        0.375
                    );
                    let expected = if *covered != 0 && initial & 2 != 2 {
                        initial | 64
                    } else {
                        initial
                    };
                    assert_eq!(
                        bytes[offset + size * 256 + y * 256 + x],
                        expected,
                        "underwater frame{frames} initial{initial} size{size} pixel{pixel}"
                    );
                    pixels += 1;
                }
                offset += size * 256 * 2;
            }
            drop(bytes);
            staging.unmap();
        }
        frames += 1;
    }
    assert_eq!(frames, 3);
    println!("{pixels} exact water-line/underwater coverage pixels plus preserved depth across three wave times");
}
