use super::*;
use bevy::prelude::*;
use bevy::render::{render_resource::*, renderer::RenderQueue, RenderApp};

#[test]
#[ignore = "requires a GPU and original VS14200 numeric stream-output fixture"]
fn native_vegetation_shadow_vs_matches_original_stream_output_gpu() {
    // The SM5 port is checked BIT-EXACT against the installed shader by the
    // Python oracle's --compiled-port mode. Modern WGSL math lowering differs
    // slightly even on this same GPU under both Vulkan and DX12: measured
    // maximum 5.9604645e-7, including identity-MVP cases. Use the existing
    // shadow GPU tests' 1e-6 float tolerance, not a production position bias.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/shadow_vegetation_cases.txt"
    );
    let raw =
        std::fs::read_to_string(path).expect("run analysis/probe_original_shadow_vegetation.py");
    let cases: Vec<Vec<u32>> = raw
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect()
        })
        .collect();
    assert_eq!(cases.len(), 256);
    assert!(cases.iter().all(|case| case.len() == 70));
    // Exercise the scene owner's encoder against every native input record,
    // not just a separately packed CB in the direct shader harness.
    for case in &cases {
        let vector = |at: usize| std::array::from_fn(|lane| f32::from_bits(case[at + lane]));
        let wind = crate::shadow_pass::NativeVegetationShadowParameters {
            main_variation: vector(4),
            main_frequency: vector(8),
            main_amplitude: vector(12),
            minimum_wind: vector(16),
            leaf_frequency: vector(20),
            leaf_amplitude: vector(24),
            branch_amplitude: vector(28),
        };
        let cb = crate::shadow_pass::native_vegetation_shadow_instance_cb(
            vector(0),
            &wind,
            &[vector(40), vector(36), vector(32), vector(44)],
            [f32::from_bits(case[48]), f32::from_bits(case[49])],
            f32::from_bits(case[50]),
        );
        assert_eq!(
            cb.as_slice(),
            case[..52]
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>()
        );
    }
    let input_bytes: Vec<u8> = cases
        .iter()
        .flatten()
        .flat_map(|word| word.to_le_bytes())
        .collect();
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
    let source = format!(
        "{}\n{}\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow_wind.wgsl"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow_vegetation.wgsl"
        )),
        r#"
        @group(0) @binding(3) var<storage, read> cases: array<u32>;
        @group(0) @binding(4) var<storage, read_write> result: array<f32>;
        fn vector(at: u32) -> vec4<f32> {
            return bitcast<vec4<f32>>(vec4<u32>(cases[at], cases[at+1u], cases[at+2u], cases[at+3u]));
        }
        @compute @workgroup_size(64) fn probe(@builtin(global_invocation_id) id: vec3<u32>) {
            if id.x >= arrayLength(&cases)/70u { return; }
            let at = id.x*70u;
            let p = VegetationShadowCB(vector(at), vector(at+4u), vector(at+8u), vector(at+12u),
                vector(at+16u), vector(at+20u), vector(at+24u), vector(at+28u),
                vector(at+32u), vector(at+36u), vector(at+40u), vector(at+44u), vector(at+48u));
            let color = vector(at+52u).xyz;
            let normal = vector(at+55u).xy;
            let position = vector(at+57u).xyz;
            let uv = vector(at+60u).xy;
            let clip = vegetation_shadow_clip(p, position, normal, color);
            for (var component=0u; component<4u; component++) {
                result[id.x*8u+component] = uv[component%2u];
                result[id.x*8u+4u+component] = clip[component];
            }
        }
    "#
    );
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("actual vegetation shadow VS stream-output parity"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("vegetation shadow VS parity"),
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: default(),
            cache: None,
        });
    let input = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("original synthetic vertex/CB inputs"),
        contents: &input_bytes,
        usage: BufferUsages::STORAGE,
    });
    let size = (cases.len() * 8 * 4) as u64;
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
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 3,
                    resource: input.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups((cases.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let mut max_abs = 0.0f32;
    for (index, actual) in bytes.chunks_exact(4).enumerate() {
        let actual = u32::from_le_bytes(actual.try_into().unwrap());
        let expected = cases[index / 8][62 + index % 8];
        if index % 8 < 4 {
            assert_eq!(
                actual,
                expected,
                "raw UV case {} component {}",
                index / 8,
                index % 8
            );
        } else {
            let error = (f32::from_bits(actual) - f32::from_bits(expected)).abs();
            assert!(
                error.is_finite() && error <= 0.000001,
                "clip case {} component {}: {} != {}",
                index / 8,
                index % 8,
                f32::from_bits(actual),
                f32::from_bits(expected)
            );
            max_abs = max_abs.max(error);
        }
    }
    drop(bytes);
    readback.unmap();
    println!(
        "VS14200: {} native cases, raw UV exact, clip max_abs={max_abs:e}",
        cases.len()
    );
    draw_vegetation_cutout(
        device,
        world.resource::<RenderAdapter>(),
        queue,
        &cases,
        &output,
        &readback,
    );
}

const VEGETATION_ATTRIBUTES: [VertexAttribute; 4] = [
    VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 20,
        shader_location: 0,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 32,
        shader_location: 1,
    },
    VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 2,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 12,
        shader_location: 3,
    },
];

#[test]
fn vegetation_shadow_layout_requires_native_inputs_and_complete_cb() {
    let mut attributes = VEGETATION_ATTRIBUTES;
    fn layout(attributes: &[VertexAttribute]) -> RawVertexBufferLayout<'_> {
        RawVertexBufferLayout {
            array_stride: 40,
            step_mode: VertexStepMode::Vertex,
            attributes,
        }
    }
    assert!(validate_vegetation_vertex_layout(&layout(&attributes)).is_ok());
    assert!(validate_vegetation_vertex_layout(&layout(&attributes[..2])).is_err());
    attributes[2].format = VertexFormat::Float32x2;
    assert!(validate_vegetation_vertex_layout(&layout(&attributes)).is_err());
    attributes = VEGETATION_ATTRIBUTES;
    attributes[3].offset = 36;
    assert!(validate_vegetation_vertex_layout(&layout(&attributes)).is_err());
    assert_eq!(
        NativeShadowDepthProgram::Vegetation.model_uniform_bytes(),
        208
    );
    assert_eq!(NativeShadowDepthProgram::Opaque.model_uniform_bytes(), 80);
    assert_eq!(
        NativeShadowDepthProgram::RigidAlpha.model_uniform_bytes(),
        80
    );
}

fn draw_vegetation_cutout(
    device: &RenderDevice,
    adapter: &RenderAdapter,
    queue: &RenderQueue,
    cases: &[Vec<u32>],
    output: &Buffer,
    readback: &Buffer,
) {
    let layout = RawVertexBufferLayout {
        array_stride: 40,
        step_mode: VertexStepMode::Vertex,
        attributes: &VEGETATION_ATTRIBUTES,
    };
    let pipeline = NativeOpaqueShadowPipeline::new_vegetation_with_raster_state(
        device,
        adapter,
        &layout,
        DepthBiasState::default(),
        TextureFormat::Depth16Unorm,
        FrontFace::Cw,
        None,
    )
    .unwrap();
    // Fixture case16 has an identity projection. Preserve all actual input
    // bytes, including raw color/normal values, rather than forward material RGB.
    let base = 16 * 8;
    let vertex_bytes: Vec<u8> = cases[base..base + 8]
        .iter()
        .flat_map(|case| case[52..62].iter())
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("native vegetation vertex inputs"),
        contents: &vertex_bytes,
        usage: BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[0, 0, 3, 0, 7, 0],
        usage: BufferUsages::INDEX,
    });
    let model_bytes: Vec<u8> = cases[base][..52]
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect();
    let model = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("original vegetation 208-byte CB"),
        contents: &model_bytes,
        usage: BufferUsages::UNIFORM,
    });
    let binding = pipeline.bind_prepared_model_state(device, &model).unwrap();
    let rigid = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &model_bytes[..80],
        usage: BufferUsages::UNIFORM,
    });
    assert!(pipeline
        .bind_prepared_model_state(device, &rigid)
        .unwrap_err()
        .contains("208"));
    let sampler = native_shadow_alpha_sampler(device);
    let depths = NativeShadowDepthArray::new_with_format(
        device,
        Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 2,
        },
        TextureFormat::Depth16Unorm,
        "vegetation alpha threshold fixture",
    )
    .unwrap();
    let textures: Vec<_> = [0x3800u16, 0x37ffu16]
        .into_iter()
        .map(|alpha| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("exact half / predecessor alpha"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let rgba: Vec<u8> = [0x3c00u16, 0x3c00, 0x3c00, alpha]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect();
            queue.write_texture(
                texture.as_image_copy(),
                &rgba,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(8),
                    rows_per_image: Some(1),
                },
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            texture
        })
        .collect();
    let views: Vec<_> = textures
        .iter()
        .map(|texture| texture.create_view(&default()))
        .collect();
    let materials: Vec<_> = views
        .iter()
        .map(|view| {
            pipeline
                .bind_vegetation_material(device, view, &sampler)
                .unwrap()
        })
        .collect();
    assert!(pipeline
        .bind_alpha_material(device, &views[0], &sampler, &model)
        .is_err());
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    for (layer, material) in materials.iter().enumerate() {
        pipeline
            .draw_layer(
                &mut encoder,
                &depths,
                layer as u32,
                Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                },
                None,
                &[NativeOpaqueShadowDraw {
                    model_state_offset: 0,
                    model_state: &binding,
                    program_resources: Some(material),
                    vertex_buffer: &vertices,
                    index_buffer: &indices,
                    index_format: IndexFormat::Uint16,
                    indices: 0..3,
                    base_vertex: 0,
                    instances: 0..1,
                    scissor: NativeShadowScissor {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 32,
                    },
                }],
            )
            .unwrap();
    }
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("vegetation target validation"), source: ShaderSource::Wgsl(r#"
            @group(0) @binding(0) var depth: texture_depth_2d_array;
            @group(0) @binding(1) var<storage, read_write> result: array<f32>;
            @compute @workgroup_size(8,8) fn read_depth(@builtin(global_invocation_id) id: vec3<u32>) {
                let size = textureDimensions(depth);
                if any(id.xy >= size) || id.z >= textureNumLayers(depth) { return; }
                result[(id.z*size.y+id.y)*size.x+id.x] = textureLoad(depth,vec2<i32>(id.xy),i32(id.z),0);
            }
        "#.into()),
    });
    let reader = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("read_depth"),
            compilation_options: default(),
            cache: None,
        });
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &reader.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&depths.sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&reader);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(4, 4, 2);
    }
    encoder.copy_buffer_to_buffer(output, 0, readback, 0, 32 * 32 * 2 * 4);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |result| {
        tx.send(result).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(10)),
        })
        .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .unwrap()
        .unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let pixels: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert!(
        pixels[..1024].iter().any(|depth| *depth < 1.0),
        "alpha==0.5 must cast"
    );
    assert!(
        pixels[..1024].contains(&1.0),
        "fixture must not cover the entire layer"
    );
    assert!(
        pixels[1024..].iter().all(|depth| *depth == 1.0),
        "alpha below 0.5 must discard"
    );
    println!(
        "vegetation indexed D16 draw: alpha==0.5 casts; predecessor discards; 208-byte CB enforced"
    );
    drop(bytes);
    readback.unmap();
}
