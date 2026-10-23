use super::*;
use crate::assets::{
    archive::Catalog,
    camera_blur::{KernelBank, MaterialBank},
    installation,
};
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires GPU and installed archives; actual VP/FP composed camera-colour sequence"]
fn camera_colour_owner_matches_installed_sequence() {
    let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
    let kernels = KernelBank::load(&catalog).unwrap();
    let materials = MaterialBank::load(&catalog).unwrap();
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
    let kernel_buffers: Vec<_> = (0..6)
        .map(|i| BlurKernelBuffer::new(device, kernels.get(i).unwrap()))
        .collect();
    let texture = |extent: [u32; 2], format, usage| {
        device.create_texture(&TextureDescriptor {
            label: Some("camera colour sequence numeric target"),
            size: Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let image = |extent: [u32; 2], format, data: &[u8]| {
        device
            .create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("camera colour sequence controlled source"),
                    size: Extent3d {
                        width: extent[0],
                        height: extent[1],
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
                data,
            )
            .create_view(&default())
    };
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8 * 9 * 256,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut cases = 0;
    let mut channels = 0;
    let mut maximum = [0u32; 6];
    // Source modes isolate the adapter with exactly representable powers of
    // two. The original main-target storage conversion is a separate producer.
    for source_mode in 0..3 {
        let mut owner: Option<CameraColours> = None;
        let mut externals: Option<[Texture; 3]> = None;
        let mut lines = include_str!("../../../tests/fixtures/camera_colours.txt").lines();
        while let Some(header) = lines.next() {
            let words: Vec<_> = header.split_whitespace().collect();
            let u = |i: usize| words[i].parse::<u32>().unwrap();
            let f = |i: usize| words[i].parse::<f32>().unwrap();
            let frame = u(1) as usize;
            let w = u(2);
            let h = u(3);
            let stencil = u(4) != 0;
            let focus = f(5);
            let depth = f(6);
            let initial = u(7);
            let scale = [f(8), f(9)];
            let viewport = [u(10), u(11), u(12), u(13)];
            let scissor = [u(14), u(15), u(16), u(17)];
            let lowviewport = [u(18), u(19), u(20), u(21)];
            let lowscissor = [u(22), u(23), u(24), u(25)];
            let decode = |text: &str| {
                (0..text.len() / 2)
                    .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap())
                    .collect::<Vec<_>>()
            };
            let raw = decode(lines.next().unwrap().strip_prefix("SETTINGS ").unwrap());
            let settings = CameraSettings::from_words(std::array::from_fn(|i| {
                u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap())
            }));
            let base: Vec<_> = lines
                .next()
                .unwrap()
                .split_whitespace()
                .skip(1)
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let selected = [
                materials
                    .resolve(settings.blur_variant(DofPass::Near))
                    .unwrap(),
                materials
                    .resolve(settings.blur_variant(DofPass::Far))
                    .unwrap(),
            ];
            for m in selected {
                assert_eq!(
                    m.state.as_slice(),
                    base,
                    "installed material and native source state differ"
                );
            }
            let expected: Vec<_> = ["HALF", "LOW", "MASK", "NEAR", "ALPHA", "FAR", "DEPTH"]
                .map(|label| {
                    decode(
                        lines
                            .next()
                            .unwrap()
                            .strip_prefix(&format!("{label} "))
                            .unwrap(),
                    )
                })
                .into_iter()
                .collect();
            let config = CameraColourConfig {
                extent: [w, h],
                hdr_format: TextureFormat::Rg11b10Ufloat,
                depth_format: TextureFormat::Depth32FloatStencil8,
                stencil,
                host_source: source_mode != 0,
                material_states: [selected[0].state, selected[1].state],
            };
            let replaced = owner.as_ref().is_none_or(|o| !o.matches(&config));
            if replaced {
                owner = Some(CameraColours::new(device, config).unwrap());
                externals = Some([
                    texture(
                        [w, h],
                        TextureFormat::Rg11b10Ufloat,
                        TextureUsages::RENDER_ATTACHMENT
                            | TextureUsages::TEXTURE_BINDING
                            | TextureUsages::COPY_SRC,
                    ),
                    texture(
                        [w, h],
                        TextureFormat::Rg11b10Ufloat,
                        TextureUsages::RENDER_ATTACHMENT
                            | TextureUsages::TEXTURE_BINDING
                            | TextureUsages::COPY_SRC,
                    ),
                    texture(
                        [w, h],
                        TextureFormat::Depth32FloatStencil8,
                        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                    ),
                ]);
            }
            let owner = owner.as_mut().unwrap();
            let ext = externals.as_ref().unwrap();
            let views = ext.each_ref().map(|t| t.create_view(&default()));
            let sw = w * 2;
            let sh = h * 2;
            let units = [0.5, 2., 4.][frame % 3];
            let engine = [0.5, 2., 1.][frame % 3];
            let white = [4., 0.5, 2.][frame % 3];
            let gain = if source_mode == 0 {
                1.
            } else {
                units * if source_mode == 2 { engine * white } else { 1. }
            };
            let adaptation = (source_mode != 0).then(|| {
                device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("controlled current camera adaptation"),
                    contents: &[engine, 0., 0., 0., 0., 0., 0., 0.]
                        .into_iter()
                        .flat_map(f32::to_le_bytes)
                        .collect::<Vec<_>>(),
                    usage: BufferUsages::STORAGE,
                })
            });
            let mut data = Vec::new();
            for y in 0..sh as usize {
                for x in 0..sw as usize {
                    for (channel, v) in [
                        ((x * 7 + y * 3 + frame) % 23) as f32 / 4. - 0.5,
                        ((x * 3 + y * 11 + frame) % 19) as f32 / 3.,
                        ((x + y * 5 + frame) % 17) as f32 / 5.,
                        1.,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let value =
                            half::f16::from_f32(v).to_f32() * if channel < 3 { gain } else { 1. };
                        data.extend(half::f16::from_f32(value).to_bits().to_le_bytes());
                    }
                }
            }
            let hdr = image([sw, sh], TextureFormat::Rgba16Float, &data);
            let mask: Vec<_> = (0..h as usize)
                .flat_map(|y| {
                    (0..w as usize).flat_map(move |x| {
                        [
                            [0, 12, 13, 26, 128, 255, 1, 64][(x + 2 * y + frame) % 8],
                            if (x + 3 * y + frame) % 3 == 0 {
                                0
                            } else {
                                [1, 64, 255][(x + y) % 3]
                            },
                        ]
                    })
                })
                .collect();
            let mask = image([w, h], TextureFormat::Rg8Unorm, &mask);
            let focus = image(
                [1, 1],
                TextureFormat::Rgba32Float,
                &[focus, 0., 0., 1.]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            let refraction = image(
                [sw, sh],
                TextureFormat::Rg16Float,
                &vec![0; sw as usize * sh as usize * 4],
            );
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
            for (i, view) in views[..2].iter().enumerate() {
                if i == 0 && !replaced {
                    continue;
                }
                let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(if i == 0 {
                                default()
                            } else {
                                LinearRgba::new(0.5, 1., 2., 0.).into()
                            }),
                            store: StoreOp::Store,
                        },
                    })],
                    ..default()
                });
            }
            {
                let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &views[2],
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
            owner
                .record(
                    device,
                    queue,
                    &mut encoder,
                    CameraColourInputs {
                        settings: &settings,
                        materials: selected,
                        kernels: [
                            &kernel_buffers[selected[0].filter as usize],
                            &kernel_buffers[selected[1].filter as usize],
                        ],
                        source: if let Some(adaptation) = &adaptation {
                            CameraColourSource::Host {
                                image: &hdr,
                                extent: [sw, sh],
                                adaptation,
                                radiance_units: units,
                            }
                        } else {
                            CameraColourSource::Game {
                                image: &hdr,
                                extent: [sw, sh],
                            }
                        },
                        half_colour: &views[0],
                        high_mask: &mask,
                        refraction: &refraction,
                        depth_stencil: &views[2],
                        autofocus: &focus,
                        far_colour: &views[1],
                        projection: [
                            [1.5, 0., 0., 0.],
                            [0., 1.8, 0., 0.],
                            [0., 0., 0., -1.],
                            [0., 0., 0.1, 0.],
                        ],
                        near_clip: 0.1,
                        far_clip: 10.,
                        adapt_max: 8.,
                        engine_luminance: engine,
                        white_level: white,
                        compressed: source_mode == 2,
                        texel_scale: [1., 1.],
                        resolution_scale: scale,
                        position_to_texture: [1. / sw as f32, 1. / sh as f32],
                        high_viewport: viewport,
                        high_scissor: scissor,
                        low_viewport: lowviewport,
                        low_scissor: lowscissor,
                        vertices: &vertices,
                        vertex_range: 0..3,
                    },
                )
                .unwrap();
            let outputs = [
                &ext[0],
                owner.low_colour_texture(),
                owner.low_mask_texture(),
                owner.near_colour_texture(),
                owner.near_alpha_texture(),
                &ext[1],
                &ext[2],
            ];
            for (plane, t) in outputs.into_iter().enumerate() {
                encoder.copy_texture_to_buffer(
                    TexelCopyTextureInfo {
                        texture: t,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: if plane == 6 {
                            TextureAspect::DepthOnly
                        } else {
                            TextureAspect::All
                        },
                    },
                    TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: TexelCopyBufferLayout {
                            offset: plane as u64 * 9 * 256,
                            bytes_per_row: Some(256),
                            rows_per_image: Some(t.height()),
                        },
                    },
                    t.size(),
                );
            }
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &ext[2],
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::StencilOnly,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: 7 * 9 * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(h),
                    },
                },
                ext[2].size(),
            );
            let id = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(MapMode::Read, move |v| tx.send(v).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: Some(id),
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            for (plane, t) in outputs.into_iter().enumerate() {
                for y in 0..t.height() as usize {
                    for x in 0..t.width() as usize {
                        let pixel = y * t.width() as usize + x;
                        let at = plane * 9 * 256 + y * 256;
                        if plane == 6 {
                            let z = f32::from_le_bytes(
                                bytes[at + x * 4..at + x * 4 + 4].try_into().unwrap(),
                            );
                            let want = f32::from_le_bytes(
                                expected[plane][pixel * 8..pixel * 8 + 4]
                                    .try_into()
                                    .unwrap(),
                            );
                            assert_eq!(z, want, "depth case{frame} x{x} y{y}");
                            assert_eq!(
                                bytes[7 * 9 * 256 + y * 256 + x],
                                expected[plane][pixel * 8 + 4],
                                "stencil case{frame} x{x} y{y}"
                            );
                            channels += 2;
                        } else if plane == 2 || plane == 4 {
                            let bpp = if plane == 2 { 2 } else { 1 };
                            for c in 0..bpp {
                                let actual = bytes[at + x * bpp + c];
                                let want = expected[plane][pixel * bpp + c];
                                let error = actual.abs_diff(want) as u32;
                                maximum[plane] = maximum[plane].max(error);
                                assert!(error<=if plane==2{0}else{1},"byte source{source_mode} case{frame} plane{plane} x{x} y{y} channel{c}: {actual} != {want}");
                                channels += 1;
                            }
                        } else {
                            let actual = u32::from_le_bytes(
                                bytes[at + x * 4..at + x * 4 + 4].try_into().unwrap(),
                            );
                            let want = u32::from_le_bytes(
                                expected[plane][pixel * 4..pixel * 4 + 4]
                                    .try_into()
                                    .unwrap(),
                            );
                            for (shift, mask) in [(0, 2047), (11, 2047), (22, 1023)] {
                                let error =
                                    ((actual >> shift) & mask).abs_diff((want >> shift) & mask);
                                maximum[plane] = maximum[plane].max(error);
                                assert!(error<=1,"packed source{source_mode} case{frame} plane{plane} x{x} y{y} shift{shift}: {actual:08x} != {want:08x}, {error} steps");
                                channels += 1;
                            }
                        }
                    }
                }
            }
            drop(bytes);
            staging.unmap();
            cases += 1;
        }
    }
    assert_eq!(cases, 72);
    eprintln!("PASS {cases} composed camera colour frames, {channels} channels; maximum stored steps {maximum:?}");
}
