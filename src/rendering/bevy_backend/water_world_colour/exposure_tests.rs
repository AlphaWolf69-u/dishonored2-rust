//! The production water parameter bridge reads a changing GPU adaptation
//! record; CPU luminance lanes are poisoned. Colour results use native fixtures.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

#[test]
#[ignore = "requires graphics adapter; water view-exposure GPU binding"]
fn world_water_uses_gpu_exposure_for_all_hdr_operations() {
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
    let buffer = |size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let uniform = buffer(1008, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    let adaptation = buffer(32, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let input = buffer(256 * 16, BufferUsages::STORAGE | BufferUsages::COPY_DST);
    let output = buffer(256 * 32, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let staging = buffer(
        output.size(),
        BufferUsages::MAP_READ | BufferUsages::COPY_DST,
    );
    let entry = r#"
@group(0) @binding(0) var<uniform> parameters:WorldProbeColourParameters;
@group(0) @binding(1) var<storage,read> adaptation:array<vec4<f32>,2>;
@group(0) @binding(2) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let p=water_world_colour_with_exposure(parameters,adaptation[0].x);
    let q=queries[id.x];
    output[2u*id.x]=vec4(colour_output(p.output,q.rgb),q.a);
    output[2u*id.x+1u]=vec4(p.reflection.screen_hdr.z,p.refraction.hdr.x,
        p.output.tint_exposure.w,p.reflection.screen_hdr.w);
}
"#;
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("water GPU adaptation contract"),
        source: ShaderSource::Wgsl(
            format!(
                "{}\n{}\n{}\n{entry}",
                super::super::water_colour::shader_source(),
                super::super::colour_output::SHADER_SOURCE,
                include_str!("../../../../assets/shaders/dh2_water_world_colour_parameters.wgsl"),
            )
            .into(),
        ),
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
                resource: adaptation.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: input.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let words = |line: &str| {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut lines = include_str!("../../../../tests/fixtures/colour_output.txt").lines();
    let mut cases = 0;
    let mut nonfinite = 0;
    let mut maximum = 0.0f32;
    while let Some(header) = lines.next() {
        let header = words(
            &header
                .split_whitespace()
                .skip(2)
                .collect::<Vec<_>>()
                .join(" "),
        );
        assert_eq!(header.len(), 12);
        let mut data = vec![0u8; 1008];
        for (i, word) in header.iter().enumerate() {
            data[928 + i * 4..932 + i * 4].copy_from_slice(&word.to_le_bytes());
        }
        // Three independent bad CPU values must all be replaced from the GPU.
        for (offset, word) in [(184, 0x7fc01234u32), (288, 0xc2b60000), (940, 0x449a4000)] {
            data[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        // The bridge must preserve white level, not overwrite adjacent lanes.
        data[188..192].copy_from_slice(&header[4].to_le_bytes());
        queue.write_buffer(&uniform, 0, &data);
        queue.write_buffer(
            &adaptation,
            0,
            &[
                header[3], 0x45000000, 0x46000000, 0x47000000, 0x48000000, 0, 0, 0,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        );
        let rows = (0..256)
            .map(|_| words(lines.next().unwrap()))
            .collect::<Vec<_>>();
        assert!(rows.iter().all(|row| row.len() == 8));
        queue.write_buffer(
            &input,
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
            pass.dispatch_workgroups(4, 1, 1);
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
        let mapped = staging.slice(..).get_mapped_range();
        for (i, row) in rows.iter().enumerate() {
            for c in 0..4 {
                let at = i * 32 + c * 4;
                let actual = f32::from_le_bytes(mapped[at..at + 4].try_into().unwrap());
                let expected = f32::from_bits(row[4 + c]);
                if !expected.is_finite() {
                    assert!((expected.is_nan() && actual.is_nan()) || expected == actual);
                    nonfinite += 1;
                } else {
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        actual.is_finite() && error < 3e-6 * (1.0 + expected.abs()),
                        "case{cases} row{i} channel{c}: {actual} != {expected}"
                    );
                }
                let at = i * 32 + 16 + c * 4;
                let word = u32::from_le_bytes(mapped[at..at + 4].try_into().unwrap());
                assert_eq!(
                    word,
                    header[if c == 3 { 4 } else { 3 }],
                    "GPU exposure lane{c}"
                );
            }
        }
        drop(mapped);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 12);
    assert_eq!(nonfinite, 3072);
    println!("12 GPU exposure frames, 12288 native colour channels, max finite error={maximum}");
}
