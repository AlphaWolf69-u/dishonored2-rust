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
#[ignore = "requires GPU; installed colour overlay and HDR range conversion"]
fn colour_output_matches_installed_shader() {
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
        let buffer = |size, usage| {
            device.create_buffer(&BufferDescriptor {
                label: None,
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(48, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
        let input = buffer(256 * 16, BufferUsages::STORAGE | BufferUsages::COPY_DST);
        let output = buffer(256 * 16, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
        let staging = buffer(256 * 16, BufferUsages::MAP_READ | BufferUsages::COPY_DST);
        let entry = include_str!("tests.wgsl");
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("colour output"),
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
                    resource: input.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        );
        let mut lines = include_str!("../../../../tests/fixtures/colour_output.txt").lines();
        let mut cases = 0;
        let mut nonfinite = 0;
        while let Some(header) = lines.next() {
            let words = header
                .split_whitespace()
                .skip(2)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 12);
            let params = ColourOutputParameters {
                tint_exposure: std::array::from_fn(|i| f32::from_bits(words[i])),
                white_level: std::array::from_fn(|i| f32::from_bits(words[4 + i])),
                options: words[8..12].try_into().unwrap(),
            };
            let bytes = params.uniform_bytes();
            assert_eq!(
                bytes.as_slice(),
                words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            queue.write_buffer(&uniform, 0, &bytes);
            let rows = (0..256)
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
            let bytes = staging.slice(..).get_mapped_range();
            let mut maximum = 0f32;
            for (i, row) in rows.iter().enumerate() {
                for c in 0..4 {
                    let at = i * 16 + c * 4;
                    let actual = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                    let expected = f32::from_bits(row[4 + c]);
                    if !expected.is_finite() {
                        assert!(
                            (expected.is_nan() && actual.is_nan()) || expected == actual,
                            "case{cases} row{i} channel{c}: {actual} != {expected}"
                        );
                        nonfinite += 1;
                        continue;
                    }
                    let error = (actual - expected).abs();
                    maximum = maximum.max(error);
                    assert!(
                        actual.is_finite() && error < 3e-6 * (1. + expected.abs()),
                        "case{cases} row{i} channel{c}: {actual} != {expected}"
                    );
                }
            }
            eprintln!("colour output case{cases} max error={maximum}");
            drop(bytes);
            staging.unmap();
            cases += 1;
        }
        assert_eq!(cases, 12);
        assert_eq!(nonfinite, 3072);
    }
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
}
