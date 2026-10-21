use super::*;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

#[test]
#[ignore = "requires GPU; complete installed fog-shadow variants and temporal sampling"]
fn fog_shadow_matches_installed_shader() {
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
        let uploader = super::super::test_depth::DepthUploader::new(device);
        let full = uploader.upload(
            device,
            queue,
            32,
            24,
            1,
            (0..24)
                .flat_map(|y| (0..32).map(move |x| ((x + y * 3) % 31) as f32 / 32.))
                .collect(),
        );
        let depth = full.create_view(&default());
        let projected = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("fog numeric height image"),
                size: Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::R32Float,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &(0..8)
                .flat_map(|y| {
                    (0..8).flat_map(move |x| (((x * 13 + y * 7) % 31) as f32 * 150.).to_le_bytes())
                })
                .collect::<Vec<_>>(),
        );
        let projected_view = projected.create_view(&default());
        let shadow = uploader.upload(
            device,
            queue,
            4,
            4,
            5,
            (0..5)
                .flat_map(|l| {
                    (0..4).flat_map(move |y| {
                        (0..4).map(move |x| 0.25 + ((x + y * 3 + l) % 3) as f32 * 0.25)
                    })
                })
                .collect(),
        );
        let atlas = shadow.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        let linear = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            lod_max_clamp: 13.,
            ..default()
        });
        let comparison = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            // Installed shadowHardwarePCFSampler uses strict Less, including
            // fog samples exactly on a caster. The edge fixtures exercise it.
            compare: Some(CompareFunction::Less),
            lod_max_clamp: 13.,
            ..default()
        });
        let buffer = |size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(144, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
        let records = buffer(416, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let queries = buffer(128 * 16, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let output = buffer(128 * 16, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let staging = buffer(128 * 16, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("complete fog shadow"),
            source: ShaderSource::Wgsl(
                format!("{SHADER_SOURCE}\n{}", include_str!("tests.wgsl")).into(),
            ),
        });
        let layout = device.create_bind_group_layout(
            None,
            &(0..8)
                .chain([15])
                .map(|binding| BindGroupLayoutEntry {
                    binding,
                    visibility: ShaderStages::COMPUTE,
                    count: None,
                    ty: match binding {
                        1..=3 => BindingType::Texture {
                            sample_type: if binding == 2 {
                                TextureSampleType::Float { filterable: true }
                            } else {
                                TextureSampleType::Depth
                            },
                            view_dimension: if binding == 3 {
                                TextureViewDimension::D2Array
                            } else {
                                TextureViewDimension::D2
                            },
                            multisampled: false,
                        },
                        6 | 7 => BindingType::Sampler(if binding == 7 {
                            SamplerBindingType::Comparison
                        } else {
                            SamplerBindingType::Filtering
                        }),
                        _ => BindingType::Buffer {
                            ty: if binding == 0 {
                                BufferBindingType::Uniform
                            } else {
                                BufferBindingType::Storage {
                                    read_only: binding != 5,
                                }
                            },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                    },
                })
                .collect::<Vec<_>>(),
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let binding = device.create_bind_group(
            None,
            &layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&depth),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&projected_view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&atlas),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: queries.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: output.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&linear),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(&comparison),
                },
                BindGroupEntry {
                    binding: 15,
                    resource: records.as_entire_binding(),
                },
            ],
        );
        let mut lines = include_str!("../../../../tests/fixtures/fog_shadow.txt").lines();
        let mut cases = 0;
        let mut over_one = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 140);
            let f = |offset: usize| std::array::from_fn(|i| f32::from_bits(words[offset + i]));
            let params = FogShadowParameters {
                viewport: f(0),
                resolution_projected: f(4),
                jitter: f(8),
                options: words[12..16].try_into().unwrap(),
            };
            let mut bytes = params.uniform_bytes().to_vec();
            bytes.extend(words[16..36].iter().flat_map(|v| v.to_le_bytes()));
            assert_eq!(
                bytes,
                words[..36]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            queue.write_buffer(&uniform, 0, &bytes);
            queue.write_buffer(
                &records,
                0,
                &words[36..]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            );
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
            assert!(rows.iter().all(|r| r.len() == 8));
            queue.write_buffer(
                &queries,
                0,
                &rows
                    .iter()
                    .flat_map(|r| r[..4].iter().flat_map(|v| v.to_le_bytes()))
                    .collect::<Vec<_>>(),
            );
            let mut encoder = device.create_command_encoder(&default());
            {
                let mut pass = encoder.begin_compute_pass(&default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &binding, &[]);
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
            let mut maximum = 0f32;
            for (i, row) in rows.iter().enumerate() {
                over_one += usize::from(f32::from_bits(row[4]) > 1.);
                for c in 0..4 {
                    let at = i * 16 + c * 4;
                    let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[4 + c]);
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        actual.is_finite() && expected.is_finite() && error < 3e-5,
                        "case{cases} row{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!("fog shadow case{cases} max error={maximum}");
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 31);
        assert!(over_one > 0);
    }
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
