//! Shared numeric GPU comparison for water inputs, with optional renderer noise.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

fn compare_inputs(
    source: &str,
    fixture: &str,
    query_words: usize,
    output_words: usize,
    mut encode: impl FnMut(&[u32]) -> Vec<u8>,
    with_noise: bool,
    exact: bool,
    storage_inputs: &[Vec<u8>],
) -> usize {
    let words = |line: &str| {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let frames = fixture
        .split("frame ")
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut lines = part.lines();
            (
                words(lines.next().unwrap()),
                lines.map(words).collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    assert!(!frames.is_empty());
    let count = frames[0].1.len();
    assert!(count > 0 && count % 64 == 0);
    let uniform_size = (frames[0].0.len() * 4) as u64;
    let output_size = (count * output_words * 4) as u64;
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
    let noise = with_noise.then(|| super::noise_textures::NoiseTextures::new(device, queue));
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("water input verification"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: default(),
        cache: None,
    });
    let make = |size, usage| {
        device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let uniform = make(uniform_size, BufferUsages::UNIFORM | BufferUsages::COPY_DST);
    let queries = make(
        (count * query_words * 4) as u64,
        BufferUsages::STORAGE | BufferUsages::COPY_DST,
    );
    let output = make(output_size, BufferUsages::STORAGE | BufferUsages::COPY_SRC);
    let staging = make(output_size, BufferUsages::COPY_DST | BufferUsages::MAP_READ);
    let mut entries = vec![
        BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        },
        BindGroupEntry {
            binding: 3,
            resource: queries.as_entire_binding(),
        },
        BindGroupEntry {
            binding: 4,
            resource: output.as_entire_binding(),
        },
    ];
    if let Some(noise) = &noise {
        entries.extend([
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(noise.permutation()),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(noise.gradients()),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::Sampler(noise.point_wrap()),
            },
        ]);
    }
    let bindings =
        device.create_bind_group(None, &pipeline.get_bind_group_layout(0).into(), &entries);
    let storage_buffers = storage_inputs
        .iter()
        .map(|contents| {
            device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents,
                usage: BufferUsages::STORAGE,
            })
        })
        .collect::<Vec<_>>();
    let storage_binding = (!storage_buffers.is_empty()).then(|| {
        let entries = storage_buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        device.create_bind_group(None, &pipeline.get_bind_group_layout(1).into(), &entries)
    });
    for (frame, (header, rows)) in frames.iter().enumerate() {
        assert_eq!(rows.len(), count);
        assert!(rows
            .iter()
            .all(|row| row.len() == query_words + output_words));
        let encoded = encode(header);
        assert_eq!(encoded.len() as u64, uniform_size);
        assert_eq!(
            encoded,
            header
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        let input = rows
            .iter()
            .flat_map(|row| row[..query_words].iter().flat_map(|v| v.to_le_bytes()))
            .collect::<Vec<_>>();
        queue.write_buffer(&uniform, 0, &encoded);
        queue.write_buffer(&queries, 0, &input);
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            if let Some(binding) = &storage_binding {
                pass.set_bind_group(1, binding, &[]);
            }
            pass.dispatch_workgroups((count / 64) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, output_size);
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
        let data = staging.slice(..).get_mapped_range();
        let mut maximum = 0f32;
        for (i, row) in rows.iter().enumerate() {
            for c in 0..output_words {
                let at = (i * output_words + c) * 4;
                let actual = f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                let expected = f32::from_bits(row[query_words + c]);
                let error = (actual - expected).abs();
                maximum = maximum.max(error);
                assert!(
                    if exact {
                        actual.to_bits() == expected.to_bits()
                    } else {
                        actual.is_finite() && error < 3e-5 * (1. + expected.abs())
                    },
                    "frame{frame} point{i} channel{c}: {actual} != {expected}; output words={:08x?}; expected words={:08x?}",
                    data[i*output_words*4..(i+1)*output_words*4].chunks_exact(4)
                        .map(|word| u32::from_le_bytes(word.try_into().unwrap())).collect::<Vec<_>>(),
                    &row[query_words..]
                );
            }
        }
        eprintln!("water input frame{frame} maximum error={maximum}");
        drop(data);
        staging.unmap();
    }
    frames.len()
}

pub fn compare_noise_inputs(
    source: &str,
    fixture: &str,
    query_words: usize,
    output_words: usize,
    encode: impl FnMut(&[u32]) -> Vec<u8>,
) -> usize {
    compare_inputs(
        source,
        fixture,
        query_words,
        output_words,
        encode,
        true,
        false,
        &[],
    )
}

pub fn compare_exact_inputs(
    source: &str,
    fixture: &str,
    query_words: usize,
    output_words: usize,
    encode: impl FnMut(&[u32]) -> Vec<u8>,
) -> usize {
    compare_inputs(
        source,
        fixture,
        query_words,
        output_words,
        encode,
        false,
        true,
        &[],
    )
}

pub fn compare_storage_inputs(
    source: &str,
    fixture: &str,
    query_words: usize,
    output_words: usize,
    storage_inputs: &[Vec<u8>],
) -> usize {
    compare_inputs(
        source,
        fixture,
        query_words,
        output_words,
        |header| header.iter().flat_map(|v| v.to_le_bytes()).collect(),
        false,
        false,
        storage_inputs,
    )
}
