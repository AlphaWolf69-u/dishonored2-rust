//! Actual colour draw versus cached VS14069/PS14070, including implicit mips.
use super::*;
use bevy::{
    prelude::*,
    render::{renderer::RenderQueue, RenderApp},
};

fn floats<const N: usize>(words: &[u32], offset: usize) -> [f32; N] {
    std::array::from_fn(|i| f32::from_bits(words[offset + i]))
}
fn parameters(w: &[u32]) -> OceanColourView {
    OceanColourView {
        surface: OceanSurfaceParameters {
            spatial: floats(w, 0),
            clock_view: floats(w, 4),
            camera: floats(w, 8),
            perlin: floats(w, 12),
            perlin_foam: floats(w, 16),
            shore: floats(w, 20),
            foam: floats(w, 24),
        },
        vertex: OceanVertexParameters {
            view: super::super::ocean_vertex::OceanVertexView {
                clip_from_world: floats(w, 28),
                previous_clip_from_world: floats(w, 44),
                view_row: floats(w, 60),
                jitter_relative_to_previous: floats(w, 69),
            },
            patch_transform: floats(w, 64),
            perlin_blend_end: f32::from_bits(w[68]),
        },
        reflection: WaterReflectionParameters {
            view_from_world: floats(w, 72),
            camera_to_clip: floats(w, 88),
            unproject: floats(w, 104),
            thickness: floats(w, 108),
            trace: floats(w, 112),
            screen_hdr: floats(w, 116),
            options: w[120..124].try_into().unwrap(),
        },
        refraction: WaterRefractionParameters {
            clock_caustics: floats(w, 124),
            screen: floats(w, 128),
            unproject: floats(w, 132),
            light_level: floats(w, 136),
            extinction: floats(w, 140),
            hdr: floats(w, 144),
        },
        shadow: WaterShadowParameters {
            world_to_shadow: std::array::from_fn(|i| floats(w, 148 + i * 16)),
            view_row: floats(w, 228),
            slice_fars: floats(w, 232),
            sun_direction: floats(w, 236),
            shadow_size: floats(w, 240),
            bias: floats(w, 244),
            options: w[248..252].try_into().unwrap(),
        },
        water: WaterSurfaceParameters {
            underwater_visibility_boost: f32::from_bits(w[252]),
            shore_foam_luminance: f32::from_bits(w[253]),
            use_sun: w[256] != 0,
            disable_ssr_on_rough_water: w[257] != 0,
        },
        colour: OceanColourParameters {
            projected: floats(w, 260),
            water: floats(w, 264),
            light: floats(w, 268),
            sun: floats(w, 272),
            pixel_to_texture: floats(w, 276),
        },
        output: ColourOutputParameters {
            tint_exposure: floats(w, 280),
            white_level: floats(w, 284),
            options: w[288..292].try_into().unwrap(),
        },
        sky_radius: floats(w, 292),
        use_sky_cube: w[296] != 0,
    }
}

#[test]
#[ignore = "requires GPU; complete ocean colour draw against actual cached shader pixels"]
fn ocean_colour_raster_matches_cached_stages() {
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
    {
        let noise = super::super::noise_textures::NoiseTextures::new(device, queue);
        let uploader = super::super::test_depth::DepthUploader::new(device);
        let upload = |width, height, layers, mips, format, data: &[u8], dimension| {
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("ocean colour raster numeric inputs"),
                    size: Extent3d {
                        width,
                        height,
                        depth_or_array_layers: layers,
                    },
                    mip_level_count: mips,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                data,
            );
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(dimension),
                ..default()
            });
            (texture, view)
        };
        let base = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
        let extra = include_bytes!("../../../../tests/fixtures/ocean_colour_inputs.bin");
        let waves = [
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::R32Float,
                &base[..256],
                TextureViewDimension::D2,
            ),
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::Rgba16Float,
                &base[256..768],
                TextureViewDimension::D2,
            ),
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::Rgba16Float,
                &base[768..1280],
                TextureViewDimension::D2,
            ),
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::R16Float,
                &base[1280..1408],
                TextureViewDimension::D2,
            ),
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::R32Float,
                &base[1408..],
                TextureViewDimension::D2,
            ),
            upload(
                8,
                8,
                1,
                1,
                TextureFormat::R32Float,
                &extra[..256],
                TextureViewDimension::D2,
            ),
        ];
        let projected = upload(
            8,
            8,
            1,
            1,
            TextureFormat::R32Float,
            &extra[256..512],
            TextureViewDimension::D2,
        );
        let foam_depth = upload(
            8,
            8,
            1,
            1,
            TextureFormat::R32Float,
            &extra[768..1024],
            TextureViewDimension::D2,
        );
        let foam = upload(
            8,
            8,
            1,
            4,
            TextureFormat::Rgba16Float,
            include_bytes!("../../../../tests/fixtures/ocean_colour_foam_mips.bin"),
            TextureViewDimension::D2,
        );
        let half_pixels = |values: Vec<f32>| {
            values
                .into_iter()
                .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
                .collect::<Vec<_>>()
        };
        let hdr_data = (0..24)
            .flat_map(|y| {
                (0..32).flat_map(move |x| {
                    [
                        0.2 + x as f32 * 0.0625,
                        0.3 + y as f32 * 0.0625,
                        0.4 + (x + y) as f32 * 0.03125,
                        1.,
                    ]
                })
            })
            .collect();
        let hdr = upload(
            32,
            24,
            1,
            1,
            TextureFormat::Rgba16Float,
            &half_pixels(hdr_data),
            TextureViewDimension::D2,
        );
        let fog_data = (0..24)
            .flat_map(|y| {
                (0..32).flat_map(move |x| {
                    [
                        x as f32 / 128.,
                        y as f32 / 128.,
                        (x + y) as f32 / 256.,
                        0.25,
                    ]
                })
            })
            .collect();
        let fog = upload(
            32,
            24,
            1,
            1,
            TextureFormat::Rgba16Float,
            &half_pixels(fog_data),
            TextureViewDimension::D2,
        );
        let linear = (0..24)
            .flat_map(|y| (0..32).map(move |x| 500. + y as f64 * 10. + x as f64 * 2.))
            .collect::<Vec<_>>();
        let opaque = uploader.upload(
            device,
            queue,
            32,
            24,
            1,
            linear.iter().map(|v| (1. / v) as f32).collect(),
        );
        let opaque_view = opaque.create_view(&default());
        let mut reduced_data = Vec::new();
        for m in 1..5 {
            let step = 1 << m;
            for y in 0..24 >> m {
                for x in 0..32 >> m {
                    let v = (0..step)
                        .flat_map(|dy| {
                            (0..step).map(move |dx| ((y * step + dy) * 32 + x * step + dx) as usize)
                        })
                        .map(|i| linear[i])
                        .collect::<Vec<_>>();
                    for a in [
                        v.iter().sum::<f64>() / v.len() as f64,
                        v.iter().copied().reduce(f64::min).unwrap(),
                        v.iter().copied().reduce(f64::max).unwrap(),
                        0.,
                    ] {
                        reduced_data.extend((a as f32).to_le_bytes());
                    }
                }
            }
        }
        let reduced = upload(
            16,
            12,
            1,
            4,
            TextureFormat::Rgba32Float,
            &reduced_data,
            TextureViewDimension::D2,
        );
        let caustic_data = (0..32)
            .flat_map(|l| {
                (0..8).flat_map(move |y| {
                    (0..8).map(move |x| ((x * 3 + y * 5 + l * 7) % 31) as f32 / 32.)
                })
            })
            .collect();
        let caustics = upload(
            8,
            8,
            32,
            1,
            TextureFormat::R16Float,
            &half_pixels(caustic_data),
            TextureViewDimension::D2Array,
        );
        let shadow_values = (0..5)
            .flat_map(|l| {
                (0..4).flat_map(move |y| {
                    (0..4).map(move |x| 0.25 + ((x + y * 3 + l) % 3) as f32 * 0.25)
                })
            })
            .collect();
        let shadow = uploader.upload(device, queue, 4, 4, 5, shadow_values);
        let shadow_view = shadow.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        // The cached source variant uses uniform sky, but the production
        // resource layout still requires correctly shaped unused cube views.
        let sky = upload(
            1,
            1,
            6,
            1,
            TextureFormat::Rgba16Float,
            &half_pixels(vec![0.125, 0.25, 0.5, 1.].repeat(6)),
            TextureViewDimension::Cube,
        );
        let probes = sky.0.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::CubeArray),
            ..default()
        });
        let sampler = |address| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: MipmapFilterMode::Nearest,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        let wrap = sampler(AddressMode::Repeat);
        let clamp = sampler(AddressMode::ClampToEdge);
        let comparison = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            compare: Some(CompareFunction::LessEqual),
            lod_max_clamp: 13.,
            ..default()
        });
        let mesh = OceanMeshBuffers::new(
            device,
            crate::rendering::ocean::OceanGeometry {
                vertices: vec![
                    [-1., -1.],
                    [-1., 1.],
                    [1., -1.],
                    [1., 1.],
                    [-0.6, -0.3],
                    [0.5, -0.2],
                    [0.1, 0.8],
                ],
                indices: vec![0, 1, 2, 3, 4, 5, 6],
                records: vec![crate::rendering::ocean::OceanGeometryRecord {
                    vertex_count: 4,
                    strip_primitive_count: 2,
                    strip_index_offset: 0,
                    edge_vertex_count: 7,
                    edge_triangle_count: 1,
                    edge_index_offset: 4,
                }],
            },
        );
        let target = |format| {
            device.create_texture(&TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: 32,
                    height: 24,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let colour = target(TextureFormat::Rgba16Float);
        let colour_view = colour.create_view(&default());
        let depth = target(TextureFormat::Depth32FloatStencil8);
        let depth_view = depth.create_view(&default());
        let buffer = |size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let plane = 256 * 24u64;
        let staging = buffer(plane * 3, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let colour_offset = 1200u64.next_multiple_of(alignment);
        let depth_offset = 288u64.next_multiple_of(alignment);
        let colour_uniform = buffer(
            colour_offset + 1200,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let depth_uniform = buffer(
            depth_offset + 288,
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let mut lines = include_str!("../../../../tests/fixtures/ocean_colour_raster.txt").lines();
        let mut cases = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 300);
            let view = parameters(&words);
            let bytes = view.uniform_bytes();
            assert_eq!(
                bytes.as_slice(),
                words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            queue.write_buffer(&colour_uniform, colour_offset, &bytes);
            queue.write_buffer(
                &depth_uniform,
                depth_offset,
                &super::super::ocean_depth::uniform_bytes(&view.surface, &view.vertex),
            );
            let base = [
                0x01e008a9, 0x01e00802, 0x01e00802, 0x01e00802, 0xe00e0000, 0x30ffff1e, 0, 0, 0,
            ];
            let input = |depth_pass| crate::rendering::ocean::OceanDrawInputs {
                depth_pass,
                eye_height: 300.,
                surface_height: 0.3,
                height_extent: 500.,
                wireframe: false,
                mirrored: cases % 2 != 0,
            };
            let (ds, dr) = crate::rendering::ocean::ocean_draw_state(base, input(true));
            let (cs, cr) = crate::rendering::ocean::ocean_draw_state(base, input(false));
            let depth_program = super::super::ocean_depth::OceanDepthProgram::new(
                device,
                TextureFormat::Depth32FloatStencil8,
                ds,
                dr,
                true,
            )
            .unwrap();
            let colour_program = OceanColourProgram::new(
                device,
                TextureFormat::Depth32FloatStencil8,
                TextureFormat::Rgba16Float,
                cs,
                cr,
            )
            .unwrap();
            let depth_parameters = depth_program.parameter_binding(device, &depth_uniform);
            let colour_parameters = colour_program.parameter_binding(device, &colour_uniform);
            let images = [
                noise.permutation(),
                noise.gradients(),
                &waves[0].1,
                &waves[1].1,
                &waves[2].1,
                &waves[3].1,
                &waves[4].1,
                &waves[5].1,
            ];
            let samplers = [noise.point_wrap(), &wrap, &clamp];
            let depth_waves = depth_program.texture_binding(device, images, samplers);
            let colour_waves = colour_program.wave_binding(device, images, samplers);
            let scene = colour_program.scene_binding(
                device,
                [
                    &projected.1,
                    &foam.1,
                    &foam_depth.1,
                    &opaque_view,
                    &reduced.1,
                    &hdr.1,
                    &fog.1,
                    &caustics.1,
                    &shadow_view,
                    &sky.1,
                    &probes,
                ],
                &comparison,
            );
            let mut encoder = device.create_command_encoder(&default());
            {
                let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &colour_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(
                                bevy::color::LinearRgba::new(0.03125, 0.0625, 0.125, 0.25).into(),
                            ),
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(0.),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: Some(Operations {
                            load: LoadOp::Clear(0x80),
                            store: StoreOp::Store,
                        }),
                    }),
                    ..default()
                });
            }
            depth_program
                .record(
                    &mut encoder,
                    &depth_view,
                    None,
                    [1., 2., 30., 20.],
                    [3, 4, 26, 17],
                    &mesh,
                    &depth_waves,
                    &[OceanPatchDraw {
                        record: 0,
                        parameters: &depth_parameters,
                        parameter_offset: depth_offset as u32,
                    }],
                )
                .unwrap();
            colour_program
                .record(
                    &mut encoder,
                    &depth_view,
                    &colour_view,
                    [1., 2., 30., 20.],
                    [3, 4, 26, 17],
                    &mesh,
                    &colour_waves,
                    &scene,
                    &[OceanPatchDraw {
                        record: 0,
                        parameters: &colour_parameters,
                        parameter_offset: colour_offset as u32,
                    }],
                )
                .unwrap();
            for (texture, aspect, offset) in [
                (&colour, TextureAspect::All, 0),
                (&depth, TextureAspect::DepthOnly, plane),
                (&depth, TextureAspect::StencilOnly, plane * 2),
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
                            rows_per_image: Some(24),
                        },
                    },
                    Extent3d {
                        width: 32,
                        height: 24,
                        depth_or_array_layers: 1,
                    },
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
                    timeout: None,
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let data = staging.slice(..).get_mapped_range();
            let mut changed = 0;
            let mut maximum = 0u16;
            let mut max_depth = 0f32;
            for i in 0..768 {
                let row = lines
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(row.len(), 6);
                let x = i % 32;
                let y = i / 32;
                let address = y * 256;
                let actual_depth = f32::from_le_bytes(
                    data[plane as usize + address + x * 4..plane as usize + address + x * 4 + 4]
                        .try_into()
                        .unwrap(),
                );
                let expected_depth = f32::from_bits(row[0]);
                let error = (actual_depth - expected_depth).abs();
                max_depth = max_depth.max(error);
                assert!(
                    error < 6e-7,
                    "case{cases} pixel{i} depth {actual_depth} != {expected_depth}"
                );
                assert_eq!(
                    u32::from(data[plane as usize * 2 + address + x]),
                    row[1],
                    "case{cases} pixel{i} stencil"
                );
                let mut rgba = [0u16; 4];
                for c in 0..4 {
                    let at = address + x * 8 + c * 2;
                    let value = u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
                    rgba[c] = value;
                    let expected = row[2 + c] as u16;
                    let delta = value.abs_diff(expected);
                    maximum = maximum.max(delta);
                    assert!(half::f16::from_bits(value).is_finite()&&delta<=1,"case{cases} pixel{i} channel{c}: half {value:04x} != {expected:04x}, delta{delta}");
                }
                changed += usize::from(rgba != [0x2800, 0x2c00, 0x3000, 0x3400]);
            }
            assert_eq!(changed, 106, "case{cases} drawn pixels");
            eprintln!("ocean colour raster case{cases}: changed={changed}, max half ULP={maximum}, max depth={max_depth}");
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 4);
    }
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
