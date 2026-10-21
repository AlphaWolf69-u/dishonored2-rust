use super::*;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

fn parameters(words: &[u32]) -> ScreenRayParameters {
    ScreenRayParameters {
        camera_to_clip: std::array::from_fn(|i| f32::from_bits(words[i])),
        unproject: std::array::from_fn(|i| f32::from_bits(words[16 + i])),
        extent: words[20..24].try_into().unwrap(),
    }
}
#[test]
fn screen_ray_uniform_matches_source_packets() {
    let mut cases = 0;
    for header in include_str!("../../../../tests/fixtures/screen_ray.txt")
        .lines()
        .filter(|v| v.starts_with("case "))
    {
        let words = header
            .split_whitespace()
            .skip(2)
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 24);
        assert_eq!(
            parameters(&words).uniform_bytes().as_slice(),
            words
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        cases += 1;
    }
    assert_eq!(cases, 4);
}

#[test]
#[ignore = "requires GPU; hierarchical rays against original shader functions"]
fn screen_rays_match_installed_shader() {
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
        let buffer = |size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(96, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
        let queries = buffer(128 * 64, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let output = buffer(128 * 32, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let staging = buffer(128 * 32, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let entry = r#"
struct Params {camera_to_clip:mat4x4<f32>,unproject:vec4<f32>,extent:vec4<u32>,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var full_depth:texture_depth_2d;
@group(0) @binding(2) var reduced_depth:texture_2d<f32>;
@group(0) @binding(3) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let a=queries[id.x*4u];let b=queries[id.x*4u+1u];let c=queries[id.x*4u+2u];let d=queries[id.x*4u+3u];
    let hit=screen_ray_trace(a.xyz,b.xyz,bitcast<u32>(b.w),full_depth,reduced_depth,p.extent.xy,p.unproject.xyz,c,d.x,d.y,d.z,p.camera_to_clip);
    output[id.x*2u]=vec4(hit.fade,hit.depth,hit.uv);output[id.x*2u+1u]=vec4(hit.debug,0.0);
}
"#;
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("hierarchical screen rays"),
            source: ShaderSource::Wgsl(format!("{SHADER_SOURCE}\n{entry}").into()),
        });
        let layout = device.create_bind_group_layout(
            None,
            &(0..5)
                .map(|binding| BindGroupLayoutEntry {
                    binding,
                    visibility: ShaderStages::COMPUTE,
                    count: None,
                    ty: match binding {
                        1 | 2 => BindingType::Texture {
                            sample_type: if binding == 1 {
                                TextureSampleType::Depth
                            } else {
                                TextureSampleType::Float { filterable: false }
                            },
                            view_dimension: TextureViewDimension::D2,
                            multisampled: false,
                        },
                        _ => BindingType::Buffer {
                            ty: if binding == 0 {
                                BufferBindingType::Uniform
                            } else {
                                BufferBindingType::Storage {
                                    read_only: binding == 3,
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
        let mut lines = include_str!("../../../../tests/fixtures/screen_ray.txt").lines();
        let mut cases = 0;
        let mut nans = 0;
        let mut hits = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let params = parameters(&words);
            queue.write_buffer(&uniform, 0, &params.uniform_bytes());
            let [width, height, _, _] = params.extent;
            let linear = (0..height)
                .flat_map(|y| (0..width).map(move |x| 2. + y as f64 * 0.5 + (x / 8) as f64))
                .collect::<Vec<_>>();
            let full = uploader.upload(
                device,
                queue,
                width,
                height,
                1,
                linear.iter().map(|v| (1. / v) as f32).collect(),
            );
            let full_view = full.create_view(&default());
            let mut reduced = Vec::new();
            for mip in 1..5 {
                let step = 1 << mip;
                for y in 0..height >> mip {
                    for x in 0..width >> mip {
                        let values = (0..step)
                            .flat_map(|dy| {
                                (0..step).map(move |dx| {
                                    ((y * step + dy) * width + x * step + dx) as usize
                                })
                            })
                            .map(|i| linear[i])
                            .collect::<Vec<_>>();
                        for value in [
                            values.iter().sum::<f64>() / values.len() as f64,
                            values.iter().copied().reduce(f64::min).unwrap(),
                            values.iter().copied().reduce(f64::max).unwrap(),
                            0.,
                        ] {
                            reduced.extend((value as f32).to_le_bytes());
                        }
                    }
                }
            }
            let pyramid = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("screen ray input hierarchy"),
                    size: Extent3d {
                        width: width >> 1,
                        height: height >> 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 4,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba32Float,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::MipMajor,
                &reduced,
            );
            let pyramid_view = pyramid.create_view(&default());
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
                        resource: BindingResource::TextureView(&full_view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&pyramid_view),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: queries.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: output.as_entire_binding(),
                    },
                ],
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
            queue.write_buffer(
                &queries,
                0,
                &rows
                    .iter()
                    .flat_map(|row| row[..16].iter().flat_map(|v| v.to_le_bytes()))
                    .collect::<Vec<_>>(),
            );
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
            let mut maximum = [0f32; 4];
            for (i, row) in rows.iter().enumerate() {
                hits += usize::from(f32::from_bits(row[16]) > 0.);
                for c in 0..8 {
                    let at = i * 32 + c * 4;
                    let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[16 + c]);
                    if expected.is_nan() {
                        assert!(
                            actual.is_nan(),
                            "case{cases} row{i} channel{c}: expected sourceNaN, got{actual}"
                        );
                        nans += 1;
                        continue;
                    }
                    let error = (actual - expected).abs();
                    maximum[c.min(3)] = maximum[c.min(3)].max(error);
                    assert!(
                        actual.is_finite() && error < 3e-5 * (1. + expected.abs()),
                        "case{cases} row{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!("screen ray case{cases}: fade/depth/u/other max errors={maximum:?}");
            drop(data);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 4);
        assert_eq!(hits, 94);
        assert_eq!(nans, 144);
    }
    device.poll(PollType::Poll).unwrap();
}
