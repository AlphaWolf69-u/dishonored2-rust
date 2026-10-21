//! Numeric shader verification; no game window or visual inspection.
use super::*;
use bevy::{prelude::*, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; compares installed noise shader outputs"]
fn ocean_noise_matches_installed_shaders() {
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
    let noise = NoiseTextures::new(device, queue);
    assert_eq!(noise.texture_bytes(), 256 * 257 * 4);
    let rows = include_str!("../../../../tests/fixtures/ocean_noise.txt")
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 128);
    assert!(rows.iter().all(|row| row.len() == 12));
    let points = rows
        .iter()
        .flat_map(|row| row[..4].iter().flat_map(|v| v.to_le_bytes()))
        .collect::<Vec<_>>();
    let input = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("source noise inputs"),
        contents: &points,
        usage: BufferUsages::STORAGE,
    });
    let size = rows.len() as u64 * 32;
    let output = device.create_buffer(&BufferDescriptor {
        label: Some("noise output"),
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("numeric noise readback"),
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let source = format!(
        "{}\n{}",
        include_str!("../../../../assets/shaders/dh2_ocean_noise.wgsl"),
        r#"
@group(0) @binding(0) var permutation: texture_2d<f32>;
@group(0) @binding(1) var gradients: texture_2d<f32>;
@group(0) @binding(2) var point_wrap: sampler;
@group(0) @binding(3) var<storage,read> points: array<vec4<f32>>;
@group(0) @binding(4) var<storage,read_write> output: array<vec4<f32>>;
@compute @workgroup_size(64,1,1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = points[id.x];
    let quality = i32(id.x % 3u);
    let octaves = 1 + i32(id.x % 5u);
    let g = noise_gradient_three(permutation, gradients, point_wrap, p.xyz, quality);
    let f = noise_fractal_three(permutation, gradients, point_wrap, p.xyz,
        octaves, p.w, vec3(2.054, 2.042, 1.3), 0.5, quality);
    let v = noise_voronoi_two(p.xy, 0.7);
    let s = noise_shore_voronoi(p.xy);
    output[id.x*2u] = vec4(g, f, v.xy);
    output[id.x*2u+1u] = vec4(v.z, s, noise_curve_five(p.w));
}
"#
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("ocean shared noise"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("numeric source noise comparison"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let layout = pipeline.get_bind_group_layout(0).into();
    let bindings = device.create_bind_group(
        "noise fixture inputs",
        &layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(noise.permutation()),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(noise.gradients()),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(noise.point_wrap()),
            },
            BindGroupEntry {
                binding: 3,
                resource: input.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 4,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("noise fixture"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(2, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, size);
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
    let mut maximum = [0.0_f32; 8];
    for (index, row) in rows.iter().enumerate() {
        for channel in 0..8 {
            let offset = index * 32 + channel * 4;
            let actual = f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap());
            let expected = f32::from_bits(row[4 + channel]);
            let error = (actual - expected).abs();
            maximum[channel] = maximum[channel].max(error);
            assert!(
                actual.is_finite() && error <= 0.0002 * (1.0 + expected.abs()),
                "noise case{index} channel{channel}: {actual} != {expected}, error{error}"
            );
        }
    }
    eprintln!("ocean noise maximum errors={maximum:?}");
    drop(mapped);
    staging.unmap();
    assert!(
        errors.lock().unwrap().is_empty(),
        "GPU errors: {:?}",
        errors.lock().unwrap()
    );
}
