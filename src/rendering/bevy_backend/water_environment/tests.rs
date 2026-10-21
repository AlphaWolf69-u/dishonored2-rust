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
#[ignore = "requires GPU; compares installed water cube/probe sampling"]
fn water_environment_matches_installed_shader() {
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
    let cube = |cubes: u32, offset: f32| {
        let mut pixels = Vec::new();
        for face in 0..cubes * 6 {
            for y in 0..8 {
                for x in 0..8 {
                    for value in [
                        offset + (face + 1) as f32 * 0.0625 + x as f32 * 0.015625,
                        offset + (face + 2) as f32 * 0.03125 + y as f32 * 0.0078125,
                        offset + (face + 3) as f32 * 0.015625 + (x + y) as f32 * 0.00390625,
                        1.,
                    ] {
                        pixels.extend(half::f16::from_f32(value).to_bits().to_le_bytes());
                    }
                }
            }
        }
        let texture = device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("water environment numeric cube"),
                size: Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: cubes * 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &pixels,
        );
        let view = texture.create_view(&TextureViewDescriptor {
            dimension: Some(if cubes == 1 {
                TextureViewDimension::Cube
            } else {
                TextureViewDimension::CubeArray
            }),
            ..default()
        });
        (texture, view)
    };
    let probes = cube(3, 0.);
    let sky = cube(1, 0.5);
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
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
    let queries = buffer(128 * 64, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let output = buffer(128 * 16, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let staging = buffer(128 * 16, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[0.125f32, 0.25, 0.5, 20.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        usage: BufferUsages::UNIFORM,
    });
    let entries = (0..7)
        .map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::COMPUTE,
            count: None,
            ty: match binding {
                0 | 1 => BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: if binding == 0 {
                        TextureViewDimension::CubeArray
                    } else {
                        TextureViewDimension::Cube
                    },
                    multisampled: false,
                },
                4 | 5 => BindingType::Sampler(SamplerBindingType::Filtering),
                _ => BindingType::Buffer {
                    ty: if binding == 6 {
                        BufferBindingType::Uniform
                    } else {
                        BufferBindingType::Storage {
                            read_only: binding == 2,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            },
        })
        .collect::<Vec<_>>();
    let layout = device.create_bind_group_layout(None, &entries);
    let binding = device.create_bind_group(
        None,
        &layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&probes.1),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&sky.1),
            },
            BindGroupEntry {
                binding: 2,
                resource: queries.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::Sampler(&sampler),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::Sampler(&sampler),
            },
            BindGroupEntry {
                binding: 6,
                resource: uniform.as_entire_binding(),
            },
        ],
    );
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let entry = r#"
struct Query {position:vec4<f32>,direction:vec4<f32>,indices:vec4<u32>,weights:vec4<f32>,}
@group(0) @binding(0) var probes:texture_cube_array<f32>;
@group(0) @binding(1) var sky:texture_cube<f32>;
@group(0) @binding(2) var<storage,read> queries:array<Query>;
@group(0) @binding(3) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(4) var probe_sampler:sampler;
@group(0) @binding(5) var sky_sampler:sampler;
@group(0) @binding(6) var<uniform> sky_radius:vec4<f32>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let query=queries[id.x];let parameters=WaterEnvironment(sky_radius.xyz,sky_radius.w,MODE>=2u,MODE==3u,MODE==1u);
    output[id.x]=vec4(water_sky_color(parameters,query.position.xyz,query.direction.xyz,
        query.indices.xyz,query.weights.xyz,probes,probe_sampler,sky,sky_sampler),1.0);
}
"#;
    let rows = include_str!("../../../../tests/fixtures/water_environment.txt")
        .lines()
        .map(|row| {
            row.split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 512);
    assert!(rows.iter().all(|row| row.len() == 21));
    for mode in 0..4 {
        let rows = &rows[mode * 128..(mode + 1) * 128];
        assert!(rows.iter().all(|row| row[0] == mode as u32));
        let packet = rows
            .iter()
            .flat_map(|row| row[1..17].iter().flat_map(|v| v.to_le_bytes()))
            .collect::<Vec<_>>();
        queue.write_buffer(&queries, 0, &packet);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("water environment"),
            source: ShaderSource::Wgsl(
                format!("{SHADER_SOURCE}\nconst MODE:u32={mode}u;\n{entry}").into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
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
        let bytes = staging.slice(..).get_mapped_range();
        let mut maximum = 0f32;
        for (i, row) in rows.iter().enumerate() {
            for c in 0..4 {
                let address = i * 16 + c * 4;
                let value = f32::from_le_bytes(bytes[address..address + 4].try_into().unwrap());
                let reference = f32::from_bits(row[17 + c]);
                let error = (value - reference).abs();
                maximum = maximum.max(error);
                assert!(
                    error < 2e-6,
                    "mode{mode} row{i} channel{c}: {value} != {reference}"
                );
            }
        }
        eprintln!("water environment mode{mode} max error={maximum}");
        drop(bytes);
        staging.unmap();
    }
}
