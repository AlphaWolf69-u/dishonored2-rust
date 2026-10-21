//! Compare production vertex calculations to installed bodies; numeric buffers only.
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
fn ocean_vertex_uniform_matches_source_fixture() {
    let mut count = 0;
    for header in include_str!("../../../../tests/fixtures/ocean_vertex.txt")
        .lines()
        .filter(|v| v.starts_with("case "))
    {
        let words = header
            .split_whitespace()
            .skip(3)
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 72);
        let f = |i| f32::from_bits(words[i]);
        let p = OceanVertexParameters {
            view: OceanVertexView {
                clip_from_world: words[28..44]
                    .iter()
                    .map(|v| f32::from_bits(*v))
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap(),
                previous_clip_from_world: words[44..60]
                    .iter()
                    .map(|v| f32::from_bits(*v))
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap(),
                view_row: [f(60), f(61), f(62), f(63)],
                jitter_relative_to_previous: [f(69), f(70)],
            },
            patch_transform: [f(64), f(65), f(66), f(67)],
            perlin_blend_end: f(68),
        };
        let actual = p
            .uniform_bytes()
            .chunks_exact(4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(actual, words[28..]);
        count += 1;
    }
    assert_eq!(count, 9);
}

const ENTRY: &str = r#"
struct Params {surface:OceanSurfaceParameters,vertex:OceanVertexParameters,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var permutation:texture_2d<f32>;
@group(0) @binding(2) var gradients:texture_2d<f32>;
@group(0) @binding(3) var shore_map:texture_2d<f32>;
@group(0) @binding(4) var displacement:texture_2d<f32>;
@group(0) @binding(5) var previous:texture_2d<f32>;
@group(0) @binding(6) var collision_map:texture_2d<f32>;
@group(0) @binding(7) var foam_mask:texture_2d<f32>;
@group(0) @binding(8) var<storage,read> points:array<vec4<f32>>;
@group(0) @binding(9) var amplitude_mask:texture_2d<f32>;
@group(0) @binding(10) var point_wrap:sampler;
@group(0) @binding(11) var linear_wrap:sampler;
@group(0) @binding(12) var linear_clamp:sampler;
@group(0) @binding(13) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let vertex=ocean_mesh_vertex(p.surface,p.vertex,points[id.x].xy,MODE!=0u,
        permutation,gradients,point_wrap,linear_wrap,linear_clamp,shore_map,displacement,
        previous,collision_map,foam_mask,amplitude_mask);
    output[id.x*5u]=vertex.clip;output[id.x*5u+1u]=vertex.previous_clip;
    var world=vec4(0.0);var motion=vec4(0.0);
    if MODE==0u {world=vec4(vertex.world,0.0);}
    if MODE==1u {motion=screen_motion_vector(vertex.clip,vertex.previous_clip,p.vertex.perlin_end_jitter.yz);}
    output[id.x*5u+2u]=world;output[id.x*5u+3u]=vertex.legacy;output[id.x*5u+4u]=motion;
}
"#;

#[test]
#[ignore = "requires GPU; compares all source ocean vertex/depth modes"]
fn ocean_vertex_matches_installed_shaders() {
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
    let noise = super::super::noise_textures::NoiseTextures::new(device, queue);
    let upload = |format, bytes: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("ocean vertex fixture image"),
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
    let data = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
    assert_eq!(data.len(), 1664);
    let textures = [
        upload(TextureFormat::R32Float, &data[..256]),
        upload(TextureFormat::Rgba16Float, &data[256..768]),
        upload(TextureFormat::Rgba16Float, &data[768..1280]),
        upload(TextureFormat::R16Float, &data[1280..1408]),
        upload(TextureFormat::R32Float, &data[1408..]),
        upload(
            TextureFormat::R32Float,
            include_bytes!("../../../../tests/fixtures/ocean_height_query_mask.bin"),
        ),
    ];
    let views = textures.each_ref().map(|v| v.create_view(&default()));
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
    let mut entries = Vec::new();
    for binding in 0..14 {
        let ty = match binding {
            0 => BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(288),
            },
            8 | 13 => BindingType::Buffer {
                ty: BufferBindingType::Storage {
                    read_only: binding == 8,
                },
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(16),
            },
            10..=12 => BindingType::Sampler(SamplerBindingType::Filtering),
            _ => BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
        };
        entries.push(BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::COMPUTE,
            ty,
            count: None,
        });
    }
    let layout = device.create_bind_group_layout(Some("ocean vertex fixture inputs"), &entries);
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipelines: Vec<_> = (0..3)
        .map(|mode| {
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("ocean vertex calculations"),
                source: ShaderSource::Wgsl(
                    format!("{SHADER_SOURCE}\nconst MODE:u32={mode}u;\n{ENTRY}").into(),
                ),
            });
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: None,
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: default(),
                cache: None,
            })
        })
        .collect();
    let buffer = |size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let uniform = buffer(288, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    let positions = buffer(128 * 16, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let output = buffer(128 * 80, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let staging = buffer(
        output.size(),
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let binding = device.create_bind_group(
        Some("ocean vertex fixture"),
        &layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(noise.permutation()),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(noise.gradients()),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&views[0]),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::TextureView(&views[1]),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::TextureView(&views[2]),
            },
            BindGroupEntry {
                binding: 6,
                resource: BindingResource::TextureView(&views[3]),
            },
            BindGroupEntry {
                binding: 7,
                resource: BindingResource::TextureView(&views[4]),
            },
            BindGroupEntry {
                binding: 8,
                resource: positions.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 9,
                resource: BindingResource::TextureView(&views[5]),
            },
            BindGroupEntry {
                binding: 10,
                resource: BindingResource::Sampler(noise.point_wrap()),
            },
            BindGroupEntry {
                binding: 11,
                resource: BindingResource::Sampler(&wrap),
            },
            BindGroupEntry {
                binding: 12,
                resource: BindingResource::Sampler(&clamp),
            },
            BindGroupEntry {
                binding: 13,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let mut input = include_str!("../../../../tests/fixtures/ocean_vertex.txt").lines();
    let mut cases = 0;
    while let Some(header) = input.next() {
        let fields = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(fields[0], "case");
        let mode = fields[1].parse::<usize>().unwrap();
        let frame = fields[2].parse::<usize>().unwrap();
        let controls = fields[3..]
            .iter()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 288);
        queue.write_buffer(&uniform, 0, &controls);
        let mut points = Vec::new();
        let mut expected = Vec::new();
        for _ in 0..128 {
            let words = input
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 24);
            points.extend(words[..4].iter().flat_map(|v| v.to_le_bytes()));
            expected.extend(words[4..].iter().map(|v| f32::from_bits(*v)));
        }
        queue.write_buffer(&positions, 0, &points);
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&pipelines[mode]);
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
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let mapped = staging.slice(..).get_mapped_range();
        let mut maximum = [0f32; 5];
        for (i, (bytes, wanted)) in mapped.chunks_exact(4).zip(expected).enumerate() {
            let actual = f32::from_le_bytes(bytes.try_into().unwrap());
            let error = (actual - wanted).abs();
            maximum[(i / 4) % 5] = maximum[(i / 4) % 5].max(error);
            assert!(
                actual.is_finite() && error <= 0.0003 * (1. + wanted.abs()),
                "mode{mode} frame{frame} component{i}: {actual} != {wanted}"
            );
        }
        eprintln!("ocean vertex mode{mode} frame{frame} max errors={maximum:?}");
        drop(mapped);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 9);
}
