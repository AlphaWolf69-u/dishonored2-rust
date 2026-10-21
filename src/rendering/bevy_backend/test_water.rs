use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

// Owned numerical scene shared by complete reflection and surface comparisons.
// This is a test input builder, never a gameplay resource fallback.
pub fn compare(
    source: &str,
    entry: &str,
    fixture: &str,
    surface: bool,
    encode: impl Fn(&[u32]) -> Vec<u8>,
) {
    let query_words = if surface { 44 } else { 24 };
    let output_words = if surface { 4 } else { 8 };
    let texture_count = if surface { 8 } else { 6 };
    let query_binding = texture_count + 1;
    let output_binding = query_binding + 1;
    let sampler_start = output_binding + 1;
    let binding_count = if surface { 16 } else { 12 };
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
        let uploader = super::test_depth::DepthUploader::new(device);
        let buffer = |size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(
            if surface { 784 } else { 240 },
            BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        );
        let queries = buffer(
            128 * query_words as u64 * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
        );
        let output = buffer(
            128 * output_words as u64 * 4,
            BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        );
        let staging = buffer(
            128 * output_words as u64 * 4,
            BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        );
        let texture = |width, height, layers, mips, format, data: &[u8], dimension| {
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("reflection numeric input"),
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
        let cube = |cubes: u32, offset: f32| {
            let mut data = Vec::new();
            for face in 0..cubes * 6 {
                for y in 0..8 {
                    for x in 0..8 {
                        for v in [
                            offset + (face + 1) as f32 * 0.0625 + x as f32 * 0.015625,
                            offset + (face + 2) as f32 * 0.03125 + y as f32 * 0.0078125,
                            offset + (face + 3) as f32 * 0.015625 + (x + y) as f32 * 0.00390625,
                            1.,
                        ] {
                            data.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
                        }
                    }
                }
            }
            texture(
                8,
                8,
                cubes * 6,
                1,
                TextureFormat::Rgba16Float,
                &data,
                if cubes == 1 {
                    TextureViewDimension::Cube
                } else {
                    TextureViewDimension::CubeArray
                },
            )
        };
        let probes = cube(3, 0.);
        let sky = cube(1, 0.5);
        let mut hdr_data = Vec::new();
        let mut fog_data = Vec::new();
        for y in 0..32 {
            for x in 0..32 {
                for v in [
                    8. + x as f32 * 0.25,
                    10. + y as f32 * 0.125,
                    12. + (x + y) as f32 * 0.125,
                    1.,
                ] {
                    hdr_data.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
                }
                for v in [
                    x as f32 * 0.015625,
                    y as f32 * 0.03125,
                    (x + y) as f32 * 0.0078125,
                    [-0.25, 0., 0.25, 0.75, 1., 1.25][(x + y) % 6],
                ] {
                    fog_data.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
                }
            }
        }
        let hdr = texture(
            32,
            32,
            1,
            1,
            TextureFormat::Rgba16Float,
            &hdr_data,
            TextureViewDimension::D2,
        );
        let fog = texture(
            32,
            32,
            1,
            1,
            TextureFormat::Rgba16Float,
            &fog_data,
            TextureViewDimension::D2,
        );
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
        let linear = sampler(AddressMode::ClampToEdge);
        let wrap = sampler(AddressMode::Repeat);
        let extra = surface.then(|| {
            let data = (0..32)
                .flat_map(|layer| {
                    (0..8).flat_map(move |y| {
                        (0..8).flat_map(move |x| {
                            half::f16::from_f32(((x * 3 + y * 5 + layer * 7) % 31) as f32 / 32.)
                                .to_bits()
                                .to_le_bytes()
                        })
                    })
                })
                .collect::<Vec<_>>();
            let caustics = texture(
                8,
                8,
                32,
                1,
                TextureFormat::R16Float,
                &data,
                TextureViewDimension::D2Array,
            );
            let values = (0..5)
                .flat_map(|layer| {
                    (0..4).flat_map(move |y| {
                        (0..4).map(move |x| 0.25 + ((x + y * 3 + layer) % 3) as f32 * 0.25)
                    })
                })
                .collect();
            let shadow = uploader.upload(device, queue, 4, 4, 5, values);
            let view = shadow.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..default()
            });
            (caustics, (shadow, view))
        });
        let comparison = surface.then(|| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                compare: Some(CompareFunction::LessEqual),
                lod_max_clamp: 13.,
                ..default()
            })
        });
        let entries = (0..binding_count)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::COMPUTE,
                count: None,
                ty: match binding {
                    b if b >= 1 && b <= texture_count => BindingType::Texture {
                        sample_type: if binding == 1 || binding == 8 {
                            TextureSampleType::Depth
                        } else {
                            TextureSampleType::Float {
                                filterable: binding != 2,
                            }
                        },
                        view_dimension: match binding {
                            5 => TextureViewDimension::CubeArray,
                            6 => TextureViewDimension::Cube,
                            7 | 8 => TextureViewDimension::D2Array,
                            _ => TextureViewDimension::D2,
                        },
                        multisampled: false,
                    },
                    b if b >= sampler_start => BindingType::Sampler(if surface && b == 15 {
                        SamplerBindingType::Comparison
                    } else {
                        SamplerBindingType::Filtering
                    }),
                    _ => BindingType::Buffer {
                        ty: if binding == 0 {
                            BufferBindingType::Uniform
                        } else {
                            BufferBindingType::Storage {
                                read_only: binding == query_binding,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                },
            })
            .collect::<Vec<_>>();
        let layout = device.create_bind_group_layout(None, &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("complete water reflection"),
            source: ShaderSource::Wgsl(format!("{source}\n{entry}").into()),
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let mut lines = fixture.lines();
        let mut cases = 0;
        let mut hits = 0;
        let mut cutoffs = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), if surface { 196 } else { 60 });
            let bytes = encode(&words);
            assert_eq!(
                bytes,
                words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            queue.write_buffer(&uniform, 0, &bytes);
            let factor = if !surface && cases % 5 == 4 {
                1000.
            } else {
                1.
            };
            let depth = (0..32)
                .flat_map(|y| (0..32).map(move |x| factor * (2. + y as f64 * 0.5 + (x / 8) as f64)))
                .collect::<Vec<_>>();
            let full = uploader.upload(
                device,
                queue,
                32,
                32,
                1,
                depth.iter().map(|v| (1. / v) as f32).collect(),
            );
            let full_view = full.create_view(&default());
            let mut reduced = Vec::new();
            for mip in 1..5 {
                let step = 1 << mip;
                for y in 0..32 >> mip {
                    for x in 0..32 >> mip {
                        let v = (0..step)
                            .flat_map(|dy| {
                                (0..step)
                                    .map(move |dx| ((y * step + dy) * 32 + x * step + dx) as usize)
                            })
                            .map(|i| depth[i])
                            .collect::<Vec<_>>();
                        for a in [
                            v.iter().sum::<f64>() / v.len() as f64,
                            v.iter().copied().reduce(f64::min).unwrap(),
                            v.iter().copied().reduce(f64::max).unwrap(),
                            0.,
                        ] {
                            reduced.extend((a as f32).to_le_bytes());
                        }
                    }
                }
            }
            let pyramid = texture(
                16,
                16,
                1,
                4,
                TextureFormat::Rgba32Float,
                &reduced,
                TextureViewDimension::D2,
            );
            let mut views = vec![&full_view, &pyramid.1, &hdr.1, &fog.1, &probes.1, &sky.1];
            if let Some((caustics, shadow)) = &extra {
                views.extend([&caustics.1, &shadow.1]);
            }
            let bindings = (0..binding_count)
                .map(|binding| BindGroupEntry {
                    binding,
                    resource: match binding {
                        0 => uniform.as_entire_binding(),
                        b if b <= texture_count => {
                            BindingResource::TextureView(views[(b - 1) as usize])
                        }
                        b if b == query_binding => queries.as_entire_binding(),
                        b if b == output_binding => output.as_entire_binding(),
                        b if b == sampler_start => BindingResource::Sampler(&linear),
                        15 if surface => BindingResource::Sampler(comparison.as_ref().unwrap()),
                        _ => BindingResource::Sampler(&wrap),
                    },
                })
                .collect::<Vec<_>>();
            let group = device.create_bind_group(None, &layout, &bindings);
            let rows = (0..128)
                .map(|_| {
                    lines
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .map(|v| u32::from_str_radix(v, 16).unwrap())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert!(rows.iter().all(|r| r.len() == query_words + output_words));
            queue.write_buffer(
                &queries,
                0,
                &rows
                    .iter()
                    .flat_map(|r| r[..query_words].iter().flat_map(|v| v.to_le_bytes()))
                    .collect::<Vec<_>>(),
            );
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(2, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, output.size());
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
            let mut maximum = [0f32; 2];
            for (i, row) in rows.iter().enumerate() {
                if !surface {
                    hits += usize::from(f32::from_bits(row[28]) > 0.);
                    cutoffs += usize::from(
                        f32::from_bits(row[28]) > 0. && f32::from_bits(row[29]) > 6599.,
                    );
                }
                for c in 0..output_words {
                    let at = (i * output_words + c) * 4;
                    let value = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let reference = f32::from_bits(row[query_words + c]);
                    let error = (value - reference).abs();
                    maximum[c / 4] = maximum[c / 4].max(error);
                    assert!(
                        value.is_finite()
                            && reference.is_finite()
                            && error < 3e-5 * (1. + reference.abs()),
                        "case{cases} row{i} channel{c}: {value} != {reference}"
                    );
                }
            }
            eprintln!("water surface={surface} case{cases} max colour/diagnostic {maximum:?}");
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, if surface { 16 } else { 20 });
        if !surface {
            assert_eq!(hits, 572);
            assert_eq!(cutoffs, 184);
        }
    }
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
