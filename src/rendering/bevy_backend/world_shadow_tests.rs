use super::*;
use bevy::prelude::*;
use bevy::render::{render_resource::*, renderer::RenderQueue, RenderApp};

#[test]
#[ignore = "requires graphics adapter; compares actual installed D3D11 world VS with WGSL"]
fn native_world_shadow_vertex_installed_dxbc_gpu_parity() {
    let mut rows = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/world_shadow_vertex_cases.txt"
    ))
    .lines();
    let meta: Vec<_> = rows
        .next()
        .unwrap()
        .split_whitespace()
        .skip(1)
        .map(|s| s.parse::<usize>().unwrap())
        .collect();
    let [count, matrix_count, offset] = meta.as_slice() else {
        panic!("bad fixture header")
    };
    let words = |line: &str, tag: &str| -> Vec<u32> {
        let mut parts = line.split_whitespace();
        assert_eq!(parts.next(), Some(tag));
        parts.map(|s| u32::from_str_radix(s, 16).unwrap()).collect()
    };
    let bytes =
        |words: &[u32]| -> Vec<u8> { words.iter().flat_map(|word| word.to_le_bytes()).collect() };
    let projection = words(rows.next().unwrap(), "VP");
    let mut view = Vec::new();
    for row in (0..4).rev() {
        view.extend_from_slice(&projection[row * 4..row * 4 + 4]);
    }
    view.extend([*offset as u32, 0, 0, 0]);
    view.extend([0; 4]); // Unused opaque UV variables in the shared world record.
    let matrices: Vec<_> = (0..*matrix_count)
        .flat_map(|_| words(rows.next().unwrap(), "MATRIX"))
        .collect();
    let indices: Vec<_> = (0..*offset + *count)
        .flat_map(|_| words(rows.next().unwrap(), "INDEX"))
        .collect();
    let mut positions = Vec::new();
    let mut expected = Vec::new();
    let mut roundoff_bounds = Vec::new();
    for instance in 0..*count {
        let fields = words(rows.next().unwrap(), "CASE");
        assert_eq!(fields.len(), 8);
        let model = (indices[offset + instance] & 0x00ff_ffff) as usize;
        let mut absolute_world = [0.0_f64; 3];
        for row in 0..3 {
            absolute_world[row] = (0..4)
                .map(|column| {
                    (f64::from(f32::from_bits(fields[column]))
                        * f64::from(f32::from_bits(matrices[model * 12 + row * 4 + column])))
                    .abs()
                })
                .sum();
        }
        // The actual DXBC declares refactoringAllowed. D3D11 VS and Vulkan
        // compute can reassociate/contract its expression differently: this
        // is not bit-exact across APIs. Per component the scalar expression
        // contains three 4-term model dots, three products and three sums
        // (27 rounded operations). Bound BOTH evaluations by gamma_27 times
        // the absolute product sum, rather than inventing an epsilon.
        let u = f64::from(f32::EPSILON) / 2.0;
        let gamma = 27.0 * u / (1.0 - 27.0 * u);
        for row in 0..4 {
            let magnitude: f64 = (0..3)
                .map(|column| {
                    f64::from(f32::from_bits(projection[row * 4 + column])).abs()
                        * absolute_world[column]
                })
                .sum::<f64>()
                + f64::from(f32::from_bits(projection[row * 4 + 3])).abs();
            roundoff_bounds.push(2.0 * gamma * magnitude);
        }
        // The live pipeline selects NativeVegetationPosition, which retains
        // these original local values independently of the forward mesh.
        positions.extend_from_slice(&fields[..4]);
        expected.extend_from_slice(&fields[4..]);
    }
    assert!(rows.next().is_none());
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
    let world = app.sub_app_mut(RenderApp).world_mut();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let adapter = world.resource::<RenderAdapter>();
    let buffer = |label, words: &[u32], usage| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: &bytes(words),
            usage,
        })
    };
    let view = buffer("native world VP/offset", &view, BufferUsages::UNIFORM);
    let matrices = buffer(
        "native 48-byte matrix records",
        &matrices,
        BufferUsages::STORAGE,
    );
    let indices = buffer("native visible ID remap", &indices, BufferUsages::STORAGE);
    let positions = buffer(
        "original local shadow coordinates",
        &positions,
        BufferUsages::STORAGE,
    );
    let vertex_attributes = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let raster = NativeOpaqueShadowPipeline::new_world_with_raster_state(
        device,
        adapter,
        &RawVertexBufferLayout {
            array_stride: 12,
            step_mode: VertexStepMode::Vertex,
            attributes: &vertex_attributes,
        },
        DepthBiasState::default(),
        TextureFormat::Depth24PlusStencil8,
        FrontFace::Ccw,
        Some(Face::Back),
    )
    .unwrap();
    raster
        .bind_world_instances(device, &indices, &matrices)
        .unwrap();
    raster.bind_prepared_model_state(device, &view).unwrap();
    let wind_attributes = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 20,
            shader_location: 2,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 32,
            shader_location: 3,
        },
    ];
    let wind_pipeline = NativeOpaqueShadowPipeline::new_world_vegetation_with_raster_state(
        device,
        adapter,
        &RawVertexBufferLayout {
            array_stride: 40,
            step_mode: VertexStepMode::Vertex,
            attributes: &wind_attributes,
        },
        DepthBiasState::default(),
        TextureFormat::Depth24PlusStencil8,
        FrontFace::Ccw,
        Some(Face::Back),
    )
    .unwrap();
    assert_eq!(wind_pipeline.program.model_uniform_bytes(), 208);
    let source = format!(
        "{}\n{}\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow_world_project.wgsl"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow_world.wgsl"
        )),
        r#"
        @group(2) @binding(0) var<storage, read> test_positions: array<vec4<f32>>;
        @group(2) @binding(1) var<storage, read_write> test_results: array<vec4<f32>>;
        @compute @workgroup_size(1) fn parity(@builtin(global_invocation_id) id: vec3<u32>) {
            test_results[id.x] = world_shadow_position(test_positions[id.x].xyz, id.x);
        }
    "#
    );
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("original world shadow projection parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("parity"),
            compilation_options: default(),
            cache: None,
        });
    let size = (*count * 16) as u64;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let binding = |group, buffers: &[&Buffer]| {
        device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(group),
                entries: &buffers
                    .iter()
                    .enumerate()
                    .map(|(binding, buffer)| BindGroupEntry {
                        binding: binding as u32,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            })
    };
    let groups = [
        binding(0, &[&view]),
        binding(1, &[&indices, &matrices]),
        binding(2, &[&positions, &output]),
    ];
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        for (index, group) in groups.iter().enumerate() {
            pass.set_bind_group(index as u32, group, &[]);
        }
        pass.dispatch_workgroups(*count as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| sender.send(result).unwrap());
    device
        .wgpu_device()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let mut differing_words = 0;
    let mut maximum_absolute_error = 0.0_f64;
    for (index, (actual, expected)) in mapped.chunks_exact(4).zip(expected).enumerate() {
        let actual = u32::from_le_bytes(actual.try_into().unwrap());
        let error = (f64::from(f32::from_bits(actual)) - f64::from(f32::from_bits(expected))).abs();
        differing_words += usize::from(actual != expected);
        maximum_absolute_error = maximum_absolute_error.max(error);
        assert!(error <= roundoff_bounds[index],
            "installed VS position {} component {} exceeds IEEE reassociation bound: {} vs {}; error={error} bound={}",
            index/4, index%4, f32::from_bits(actual), f32::from_bits(expected), roundoff_bounds[index]);
    }
    eprintln!("installed world VS vs Vulkan WGSL: {differing_words}/{} differing words; max absolute error {maximum_absolute_error}; all within gamma_27 refactoring bound",count*4);
    drop(mapped);
    readback.unmap();
}
