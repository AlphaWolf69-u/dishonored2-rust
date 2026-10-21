//! Actual shader execution and numerical readback, without a game window.
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
fn ocean_surface_settings_pack_source_uniform_inputs() {
    use crate::rendering::ocean::{OceanSurfaceSettings, PerlinWaveSettings, ShoreWaveSettings};
    let mut frames = 0;
    for header in include_str!("../../../../tests/fixtures/ocean_surface.txt")
        .lines()
        .filter(|line| line.starts_with("frame "))
    {
        let words = header
            .strip_prefix("frame ")
            .unwrap()
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 28);
        let f = |i: usize| f32::from_bits(words[i]);
        let settings = OceanSurfaceSettings {
            scale: [f(0), f(1), f(2)],
            masks_extent: f(3),
            time_scale: f(5),
            normal_fade_start: f(11),
            normal_fade_factor: f(27),
            perlin: PerlinWaveSettings {
                blend_start: f(12),
                // BlendEnd belongs to vertex preparation, not this112-byte ABI.
                blend_end: f32::NAN,
                frequency: f(13),
                speed: f(14),
                amplitude: f(15),
                persistence: f(16),
                foam_threshold: f(17),
                foam_transition: f(18),
            },
            shore: ShoreWaveSettings {
                frequency: f(20),
                speed: f(21),
                amplitude: f(22),
                foam_luminance: f(23),
                foam_fade_distance: f(24),
                foam_fade_length: f(25),
            },
            foam_luminance: f(19),
            foam_bubbles_luminance: f(26),
            shore_distance_image: String::new(),
            wave_amplitude_image: String::new(),
            foam_image: String::new(),
        };
        let actual = OceanSurfaceParameters::from_settings(
            &settings,
            f(4),
            [f(8), f(9), f(10)],
            [f(6), f(7)],
        );
        let expected = words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(actual.uniform_bytes().as_slice(), expected);
        frames += 1;
    }
    assert_eq!(frames, 3);
}

#[test]
#[ignore = "requires graphics adapter; compares installed ocean surface shaders"]
fn ocean_surface_matches_installed_shaders() {
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
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |error| {
            captured.lock().unwrap().push(format!("{error}"));
        }));
    let noise = super::super::noise_textures::NoiseTextures::new(device, queue);
    let bytes = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
    assert_eq!(bytes.len(), 64 * 26);
    let mut offset = 0;
    let textures = [
        (TextureFormat::R32Float, 4),
        (TextureFormat::Rgba16Float, 8),
        (TextureFormat::Rgba16Float, 8),
        (TextureFormat::R16Float, 2),
        (TextureFormat::R32Float, 4),
    ]
    .map(|(format, stride)| {
        let size = Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("synthetic source ocean input grid"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let end = offset + 64 * stride;
        queue.write_texture(
            texture.as_image_copy(),
            &bytes[offset..end],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * stride as u32),
                rows_per_image: Some(8),
            },
            size,
        );
        offset = end;
        texture
    });
    assert_eq!(offset, bytes.len());
    let views = textures
        .each_ref()
        .map(|texture| texture.create_view(&default()));
    let linear_wrap = super::super::shadow_pipeline::shadow_alpha_sampler(device);
    let linear_clamp = device.create_sampler(&SamplerDescriptor {
        label: Some("ocean source bilinearClamp"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        lod_max_clamp: 13.0,
        ..default()
    });
    let source = format!(
        "{}\n{}",
        SHADER_SOURCE,
        r#"
@group(0) @binding(0) var<uniform> parameters: OceanSurfaceParameters;
@group(0) @binding(1) var permutation: texture_2d<f32>;
@group(0) @binding(2) var gradients: texture_2d<f32>;
@group(0) @binding(3) var point_wrap: sampler;
@group(0) @binding(4) var linear_wrap: sampler;
@group(0) @binding(5) var linear_clamp: sampler;
@group(0) @binding(6) var shore: texture_2d<f32>;
@group(0) @binding(7) var current: texture_2d<f32>;
@group(0) @binding(8) var previous: texture_2d<f32>;
@group(0) @binding(9) var collision: texture_2d<f32>;
@group(0) @binding(10) var foam: texture_2d<f32>;
@group(0) @binding(11) var<storage,read> points: array<vec4<f32>>;
@group(0) @binding(12) var<storage,read_write> output: array<vec4<f32>>;
@compute @workgroup_size(64,1,1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let a=points[id.x*2u];let b=points[id.x*2u+1u];
    var input=OceanSurfaceInput(a.xy,a.z,a.w,b.x,b.y!=0.0,b.z);
    let value=ocean_surface_displacement(parameters,input,permutation,gradients,point_wrap,
        linear_wrap,linear_clamp,shore,current,collision,foam);
    let normal=ocean_surface_normal(parameters,input,permutation,gradients,point_wrap,
        linear_wrap,linear_clamp,shore,current,collision,foam);
    input.prewarped=false;input.foam_texture=0.0;
    let history=ocean_surface_displacement(parameters,input,permutation,gradients,point_wrap,
        linear_wrap,linear_clamp,shore,previous,collision,foam);
    output[id.x*3u]=vec4(value.displacement,value.foam);
    output[id.x*3u+1u]=vec4(history.displacement,0.0);
    output[id.x*3u+2u]=vec4(normal.normal,normal.foam);
}
"#
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("shared ocean surface"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("ocean surface source comparison"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let buffer = |label, size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let uniform = buffer(
        "ocean surface controls",
        112,
        BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    );
    let points = buffer(
        "surface sample inputs",
        128 * 32,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
    );
    let output = buffer(
        "surface sample results",
        128 * 48,
        BufferUsages::STORAGE | BufferUsages::COPY_SRC,
    );
    let staging = buffer(
        "surface numeric readback",
        128 * 48,
        BufferUsages::COPY_DST | BufferUsages::MAP_READ,
    );
    let layout = pipeline.get_bind_group_layout(0).into();
    let bindings = device.create_bind_group(
        "surface fixture inputs",
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
                resource: BindingResource::Sampler(noise.point_wrap()),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::Sampler(&linear_wrap),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::Sampler(&linear_clamp),
            },
            BindGroupEntry {
                binding: 6,
                resource: BindingResource::TextureView(&views[0]),
            },
            BindGroupEntry {
                binding: 7,
                resource: BindingResource::TextureView(&views[1]),
            },
            BindGroupEntry {
                binding: 8,
                resource: BindingResource::TextureView(&views[2]),
            },
            BindGroupEntry {
                binding: 9,
                resource: BindingResource::TextureView(&views[3]),
            },
            BindGroupEntry {
                binding: 10,
                resource: BindingResource::TextureView(&views[4]),
            },
            BindGroupEntry {
                binding: 11,
                resource: points.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 12,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let mut lines = include_str!("../../../../tests/fixtures/ocean_surface.txt")
        .lines()
        .peekable();
    let parse = |s: &str| {
        s.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut frames = 0;
    while let Some(header) = lines.next() {
        let words = parse(header.strip_prefix("frame ").unwrap());
        assert_eq!(words.len(), 28);
        let group = |i: usize| std::array::from_fn(|j| f32::from_bits(words[i * 4 + j]));
        let parameters = OceanSurfaceParameters {
            spatial: group(0),
            clock_view: group(1),
            camera: group(2),
            perlin: group(3),
            perlin_foam: group(4),
            shore: group(5),
            foam: group(6),
        };
        let mut rows = Vec::new();
        while lines.peek().is_some_and(|line| !line.starts_with("frame ")) {
            rows.push(parse(lines.next().unwrap()));
        }
        assert_eq!(rows.len(), 128);
        assert!(rows.iter().all(|row| row.len() == 20));
        let data = rows
            .iter()
            .flat_map(|row| row[..8].iter().flat_map(|v| v.to_le_bytes()))
            .collect::<Vec<_>>();
        queue.write_buffer(&points, 0, &data);
        queue.write_buffer(&uniform, 0, &parameters.uniform_bytes());
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("surface fixture frame"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 128 * 48);
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
        let mapped = staging.slice(..).get_mapped_range();
        let mut maximum = [0.0_f32; 12];
        for (index, row) in rows.iter().enumerate() {
            for channel in 0..12 {
                let offset = index * 48 + channel * 4;
                let actual = f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap());
                let expected = f32::from_bits(row[8 + channel]);
                let error = (actual - expected).abs();
                maximum[channel] = maximum[channel].max(error);
                assert!(actual.is_finite() && error<=0.0003*(1.0+expected.abs()),
                    "surface frame{frames} case{index} channel{channel}: {actual} != {expected}, error{error}");
            }
        }
        eprintln!("ocean combined surface frame{frames} maximum errors={maximum:?}");
        drop(mapped);
        staging.unmap();
        frames += 1;
    }
    assert_eq!(frames, 3);
    assert!(
        errors.lock().unwrap().is_empty(),
        "GPU errors: {:?}",
        errors.lock().unwrap()
    );
}
