use super::*;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

fn parameters(words: &[u32]) -> (WaterRefractionParameters, WaterShadowParameters) {
    let floats = words.iter().map(|v| f32::from_bits(*v)).collect::<Vec<_>>();
    let v = |i| floats[i..i + 4].try_into().unwrap();
    (
        WaterRefractionParameters {
            clock_caustics: v(0),
            screen: v(4),
            unproject: v(8),
            light_level: v(12),
            extinction: v(16),
            hdr: v(20),
        },
        WaterShadowParameters {
            world_to_shadow: std::array::from_fn(|i| {
                floats[24 + i * 16..40 + i * 16].try_into().unwrap()
            }),
            view_row: v(104),
            slice_fars: v(108),
            sun_direction: v(112),
            shadow_size: v(116),
            bias: v(120),
            options: words[124..128].try_into().unwrap(),
        },
    )
}

#[test]
fn refraction_uniform_matches_source_input_packets() {
    let mut cases = 0;
    for header in include_str!("../../../../tests/fixtures/water_refraction.txt")
        .lines()
        .filter(|v| v.starts_with("case "))
    {
        let words = header
            .split_whitespace()
            .skip(2)
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 128);
        let (water, shadow) = parameters(&words);
        assert_eq!(
            uniform_bytes(&water, &shadow).as_slice(),
            words
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        cases += 1;
    }
    assert_eq!(cases, 6);
}

#[test]
#[ignore = "requires GPU; complete active refraction against installed shader bodies"]
fn refraction_matches_installed_shader() {
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
        let seed = |width, height, layers, values| {
            uploader.upload(device, queue, width, height, layers, values)
        };
        let depth = seed(
            16,
            12,
            1,
            (0..12)
                .flat_map(|y| {
                    (0..16).map(move |x| (1. / (1. + ((x * 3 + y * 7) % 27) as f64)) as f32)
                })
                .collect(),
        );
        let shadows = seed(
            4,
            4,
            5,
            (0..5)
                .flat_map(|layer| {
                    (0..4).flat_map(move |y| {
                        (0..4).map(move |x| 0.25 + ((x + y * 3 + layer) % 3) as f32 * 0.25)
                    })
                })
                .collect(),
        );
        let depth_view = depth.create_view(&default());
        let shadow_view = shadows.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        });
        let upload = |width, height, layers, format, data: &[u8]| {
            device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: None,
                    size: Extent3d {
                        width,
                        height,
                        depth_or_array_layers: layers,
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
        };
        let mut hdr_data = Vec::new();
        for y in 0..12 {
            for x in 0..16 {
                for value in [
                    0.2 + x as f64 * 0.1,
                    0.3 + y as f64 * 0.15,
                    0.4 + (x + y) as f64 * 0.05,
                    1.,
                ] {
                    hdr_data.extend(half::f16::from_f64(value).to_bits().to_le_bytes());
                }
            }
        }
        let hdr = upload(16, 12, 1, TextureFormat::Rgba16Float, &hdr_data);
        let hdr_view = hdr.create_view(&default());
        let mut caustic_data = Vec::new();
        for layer in 0..32 {
            for y in 0..8 {
                for x in 0..8 {
                    caustic_data.extend(
                        half::f16::from_f32(((x * 3 + y * 5 + layer * 7) % 31) as f32 / 32.)
                            .to_bits()
                            .to_le_bytes(),
                    );
                }
            }
        }
        let caustics = upload(8, 8, 32, TextureFormat::R16Float, &caustic_data);
        let caustic_view = caustics.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
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
        let clamp = sampler(AddressMode::ClampToEdge);
        let wrap = sampler(AddressMode::Repeat);
        let comparison = device.create_sampler(&SamplerDescriptor {
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::Repeat,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
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
        let uniform = buffer(512, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
        let queries = buffer(128 * 64, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let output = buffer(128 * 16, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let staging = buffer(128 * 16, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let entry = r#"
struct Params {water:WaterRefractionParameters,shadow:WaterShadowParameters,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var opaque_depth:texture_depth_2d;
@group(0) @binding(2) var hdr:texture_2d<f32>;
@group(0) @binding(3) var caustics:texture_2d_array<f32>;
@group(0) @binding(4) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(6) var linear_clamp:sampler;
@group(0) @binding(7) var trilinear_wrap:sampler;
@group(0) @binding(8) var shadow:texture_depth_2d_array;
@group(0) @binding(9) var comparison:sampler_comparison;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let a=queries[id.x*4u];let b=queries[id.x*4u+1u];let c=queries[id.x*4u+2u];let d=queries[id.x*4u+3u];
    let input=WaterRefractionInput(a.xyz,b.xyz,c.xyz,vec2(b.w,c.w),a.w,d.xyz);
    let value=water_refraction(p.water,input,p.shadow,opaque_depth,hdr,caustics,linear_clamp,trilinear_wrap,shadow,comparison);
    output[id.x]=vec4(value.color,value.water_depth);
}
"#;
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("water refraction"),
            source: ShaderSource::Wgsl(format!("{SHADER_SOURCE}\n{entry}").into()),
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
        let binding = device.create_bind_group(
            None,
            &pipeline.get_bind_group_layout(0).into(),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&depth_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&hdr_view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&caustic_view),
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
                    resource: BindingResource::Sampler(&clamp),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(&wrap),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::TextureView(&shadow_view),
                },
                BindGroupEntry {
                    binding: 9,
                    resource: BindingResource::Sampler(&comparison),
                },
            ],
        );
        let mut lines = include_str!("../../../../tests/fixtures/water_refraction.txt").lines();
        let mut cases = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let (water, shadow) = parameters(&words);
            queue.write_buffer(&uniform, 0, &uniform_bytes(&water, &shadow));
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
            let input = rows
                .iter()
                .flat_map(|row| row[..16].iter().flat_map(|v| v.to_le_bytes()))
                .collect::<Vec<_>>();
            queue.write_buffer(&queries, 0, &input);
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: None });
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
            let data = staging.slice(..).get_mapped_range();
            let mut maximum = [0f32; 2];
            for (i, row) in rows.iter().enumerate() {
                for c in 0..4 {
                    let at = i * 16 + c * 4;
                    let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[16 + c]);
                    let error = (actual - expected).abs();
                    let index = usize::from(c == 3);
                    maximum[index] = maximum[index].max(error);
                    assert!(
                        actual.is_finite() && error < 3e-5 * (1. + expected.abs()),
                        "case{cases} row{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!(
                "refraction case{cases}: max colour={}, depth={}",
                maximum[0], maximum[1]
            );
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 6);
    }
    device.poll(PollType::Poll).unwrap();
}
