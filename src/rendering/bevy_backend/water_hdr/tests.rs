use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU; original HDR conversion and packed storage reference"]
fn water_hdr_keeps_source_storage_and_host_alpha() {
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
    {
        let extent = Extent3d {
            width: 13,
            height: 7,
            depth_or_array_layers: 1,
        };
        let hosts = std::array::from_fn::<_, 2, _>(|_| {
            device.create_texture(&TextureDescriptor {
                label: Some("HDR reference host"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC
                    | TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let views = hosts.each_ref().map(|v| v.create_view(&default()));
        let adaptation = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 32,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 4 * 7 * 256,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut owners = [false, true].map(|compressed| {
            WaterHdr::new(
                device,
                queue,
                [13, 7],
                compressed,
                TextureFormat::Rgba16Float,
            )
            .unwrap()
        });
        let ids = owners.each_ref().map(|o| [o.source.id(), o.colour.id()]);
        assert!(owners[0].matches([13, 7], false, TextureFormat::Rgba16Float));
        assert!(!owners[0].matches([7, 13], false, TextureFormat::Rgba16Float));
        assert!(!owners[0].matches([13, 7], true, TextureFormat::Rgba16Float));
        assert!(!owners[0].matches([13, 7], false, TextureFormat::Rgba32Float));
        assert!(WaterHdr::new(device, queue, [0, 7], true, TextureFormat::Rgba16Float).is_err());
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(
                r#"
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
 let uv=vec2(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
 if(p.x<2.0 || p.x>=10.0 || p.y<1.0 || p.y>=5.0) {discard;}
 return vec4(0.13,2.6,12.7,0.375);
}
"#
                .into(),
            ),
        });
        let overlays = [TextureFormat::Rgba16Float, TextureFormat::Rg11b10Ufloat].map(|format| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: default(),
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        });
        let hex = |s: &str| -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        };
        let copy = |encoder: &mut CommandEncoder, texture: &Texture, plane: u64| {
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: plane * 7 * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(7),
                    },
                },
                extent,
            );
        };
        let mut lines = include_str!("../../../../tests/fixtures/water_hdr.txt").lines();
        let mut frames = 0;
        let mut host_steps = 0u16;
        while let Some(header) = lines.next() {
            let packet = hex(header.strip_prefix("FRAME ").unwrap());
            let word = |at| u32::from_le_bytes(packet[at..at + 4].try_into().unwrap());
            let compressed = word(0) != 0;
            let bank = usize::from(compressed);
            assert_eq!([word(4), word(8)], [13, 7]);
            let camera = f32::from_bits(word(12));
            let white = f32::from_bits(word(16));
            let pixels = hex(lines.next().unwrap().strip_prefix("INPUT ").unwrap());
            let expected_source = hex(lines.next().unwrap().strip_prefix("SOURCE ").unwrap());
            let expected_colour = hex(lines.next().unwrap().strip_prefix("COLOUR ").unwrap());
            let expected_host = hex(lines.next().unwrap().strip_prefix("HOST ").unwrap());
            let selected = frames % 2;
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &hosts[selected],
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                &pixels,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(13 * 8),
                    rows_per_image: Some(7),
                },
                extent,
            );
            queue.write_buffer(
                &adaptation,
                0,
                &[
                    word(20),
                    0x45000000,
                    0x46000000,
                    0x47000000,
                    0x48000000,
                    0,
                    0,
                    0,
                ]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
            );
            let owner = &mut owners[bank];
            let mut encoder = device.create_command_encoder(&default());
            assert!(owner
                .prepare(
                    device,
                    queue,
                    &mut encoder,
                    &views[selected],
                    [1, 1],
                    camera,
                    white,
                    &adaptation
                )
                .is_err());
            assert!(owner
                .prepare(
                    device,
                    queue,
                    &mut encoder,
                    &views[selected],
                    [13, 7],
                    0.0,
                    white,
                    &adaptation
                )
                .is_err());
            if compressed {
                assert!(owner
                    .prepare(
                        device,
                        queue,
                        &mut encoder,
                        &views[selected],
                        [13, 7],
                        camera,
                        0.0,
                        &adaptation
                    )
                    .is_err());
            }
            let frame = owner
                .prepare(
                    device,
                    queue,
                    &mut encoder,
                    &views[selected],
                    [13, 7],
                    camera,
                    white,
                    &adaptation,
                )
                .unwrap();
            assert_eq!(
                frame.format(),
                if compressed {
                    TextureFormat::Rg11b10Ufloat
                } else {
                    TextureFormat::Rgba16Float
                }
            );
            copy(&mut encoder, &frame.owner.source, 0);
            copy(&mut encoder, &frame.owner.colour, 1);
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("synthetic water write"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: frame.colour(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        },
                    })],
                    ..default()
                });
                pass.set_pipeline(&overlays[bank]);
                pass.draw(0..3, 0..1);
            }
            copy(&mut encoder, &frame.owner.colour, 2);
            frame.finish(&mut encoder);
            copy(&mut encoder, &hosts[selected], 3);
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let mapped = staging.slice(..).get_mapped_range();
            let tight = |plane: usize, bpp: usize| {
                (0..7)
                    .flat_map(|y| {
                        mapped[plane * 7 * 256 + y * 256..plane * 7 * 256 + y * 256 + 13 * bpp]
                            .iter()
                            .copied()
                    })
                    .collect::<Vec<_>>()
            };
            let bpp = if compressed { 4 } else { 8 };
            assert_eq!(
                tight(0, bpp),
                expected_source,
                "frame{frames} immutable source"
            );
            assert_eq!(
                tight(1, bpp),
                expected_source,
                "frame{frames} initial working copy"
            );
            assert_eq!(
                tight(2, bpp),
                expected_colour,
                "frame{frames} working colour"
            );
            let host = tight(3, 8);
            for (channel, (actual, expected)) in host
                .chunks_exact(2)
                .zip(expected_host.chunks_exact(2))
                .enumerate()
            {
                let a = u16::from_le_bytes(actual.try_into().unwrap());
                let e = u16::from_le_bytes(expected.try_into().unwrap());
                if channel % 4 == 3 {
                    assert_eq!(a, e, "frame{frames} host alpha");
                } else {
                    let av = half::f16::from_bits(a).to_f32();
                    let ev = half::f16::from_bits(e).to_f32();
                    if !ev.is_finite() {
                        assert!((ev.is_nan() && av.is_nan()) || av == ev);
                    } else {
                        let steps = a.abs_diff(e);
                        host_steps = host_steps.max(steps);
                        assert!(
                            av.is_finite() && steps <= 1,
                            "frame{frames} channel{channel}: {av} != {ev}"
                        );
                    }
                }
            }
            drop(mapped);
            staging.unmap();
            assert_eq!([owner.source.id(), owner.colour.id()], ids[bank]);
            assert_eq!(
                owner.encode_bindings.iter().filter(|v| v.is_some()).count(),
                if frames % 12 == 0 { 1 } else { 2 }
            );
            frames += 1;
        }
        assert_eq!(frames, 24);
        println!("24 native HDR frames: source/copy/working native bytes EXACT, host alpha EXACT, host RGB <= {host_steps} half steps");
    }
    // Let the backend retire buffers/pipelines after the owned test scene is
    // dropped, before destroying its device and allocator.
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
