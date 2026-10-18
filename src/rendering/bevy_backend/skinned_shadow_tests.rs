use super::*;
use bevy::prelude::*;
use bevy::render::{render_resource::*, renderer::RenderQueue, RenderApp};

#[test]
#[ignore = "requires GPU; compares four/eight weight shaders to actual installed DXBC stream output"]
fn native_skin_shadow_installed_dxbc_gpu_parity() {
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
    let bytes = |words: &[u32]| {
        words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let buffer = |words: &[u32], usage| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(words),
            usage,
        })
    };
    for fixture in [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/skin4_shadow_vertex_cases.txt"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/skin8_shadow_vertex_cases.txt"
        )),
    ] {
        let mut lines = fixture.lines();
        let meta = lines
            .next()
            .unwrap()
            .split_whitespace()
            .skip(1)
            .map(|v| v.parse::<usize>().unwrap())
            .collect::<Vec<_>>();
        let [influences, count, joints, offset] = meta.as_slice() else {
            panic!("bad skin fixture metadata")
        };
        let parse = |line: &str| {
            line.split_whitespace()
                .skip(1)
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>()
        };
        let vp = parse(lines.next().unwrap());
        let palette = parse(lines.next().unwrap());
        assert_eq!(palette.len(), joints * 12);
        let cases = lines.map(parse).collect::<Vec<_>>();
        assert_eq!(cases.len(), *count);
        assert!(cases.iter().all(|v| v.len() == 27));
        let rows = std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(vp[r * 4 + c])));
        let cb = crate::shadow_pass::native_skin_shadow_instance_cb(
            &rows,
            *offset as u32,
            f32::from_bits(vp[16]),
            [1.25, 0.75, -0.125, 0.25],
        );
        let constants = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &cb,
            usage: BufferUsages::UNIFORM,
        });
        let palette_gpu = buffer(&palette, BufferUsages::STORAGE);
        let input = buffer(
            &cases
                .iter()
                .flat_map(|v| v[..23].iter().copied())
                .collect::<Vec<_>>(),
            BufferUsages::STORAGE,
        );
        let source = format!(
            "const SKIN_INFLUENCES:u32={influences}u;\nconst SKIN_DITHER:bool=false;\n{}\n{}",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/shaders/dh2_shadow_skin.wgsl"
            )),
            r#"
            @group(2) @binding(0) var<storage,read> cases:array<u32>;
            @group(2) @binding(1) var<storage,read_write> result:array<vec4<f32>>;
            fn u4(at:u32)->vec4<u32>{return vec4(cases[at],cases[at+1u],cases[at+2u],cases[at+3u]);}
            @compute @workgroup_size(1) fn parity(@builtin(global_invocation_id) id:vec3<u32>){
                let at=id.x*23u;var input:Input;
                input.position=bitcast<vec3<f32>>(vec3(cases[at],cases[at+1u],cases[at+2u]));
                input.uv=bitcast<vec2<f32>>(vec2(cases[at+3u],cases[at+4u]));
                input.indices0=u4(at+7u);input.indices1=u4(at+11u);
                input.weights0=vec4<f32>(u4(at+15u))/255.0;input.weights1=vec4<f32>(u4(at+19u))/255.0;
                result[id.x]=skin_position(input);
            }"#
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline =
            device
                .wgpu_device()
                .create_compute_pipeline(&RawComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &shader,
                    entry_point: Some("parity"),
                    compilation_options: default(),
                    cache: None,
                });
        let size = (count * 16) as u64;
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = |index, buffers: &[&Buffer]| {
            device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(index),
                    entries: &buffers
                        .iter()
                        .enumerate()
                        .map(|(i, b)| BindGroupEntry {
                            binding: i as u32,
                            resource: b.as_entire_binding(),
                        })
                        .collect::<Vec<_>>(),
                })
        };
        let groups = [
            binding(0, &[&constants]),
            binding(1, &[&palette_gpu]),
            binding(2, &[&input, &output]),
        ];
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            for (i, group) in groups.iter().enumerate() {
                pass.set_bind_group(i as u32, group, &[]);
            }
            pass.dispatch_workgroups(*count as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(MapMode::Read, move |r| sender.send(r).unwrap());
        device
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let mut max_error = 0.0f64;
        for (i, actual) in mapped.chunks_exact(16).enumerate() {
            let case = &cases[i];
            let mut absolute_local = [0.0f64; 3];
            for j in 0..*influences {
                for r in 0..3 {
                    for c in 0..4 {
                        if case[15 + j] == 0 {
                            continue;
                        }
                        let joint = offset + case[7 + j] as usize;
                        let position = if c == 3 {
                            1.0
                        } else {
                            f64::from(f32::from_bits(case[c]))
                        };
                        absolute_local[r] +=
                            (f64::from(f32::from_bits(palette[joint * 12 + r * 4 + c]))
                                * position
                                * f64::from(case[15 + j])
                                / 255.0)
                                .abs();
                    }
                }
            }
            // Both shader APIs permit refactoring. Bound their independent
            // rounded expressions by gamma_80 times absolute product sums.
            let u = f64::from(f32::EPSILON) * 0.5;
            let gamma = 80.0 * u / (1.0 - 80.0 * u);
            for r in 0..4 {
                let value = f32::from_le_bytes(actual[r * 4..r * 4 + 4].try_into().unwrap());
                let expected = f32::from_bits(case[23 + r]);
                let magnitude = (0..3)
                    .map(|c| f64::from(rows[r][c]).abs() * absolute_local[c])
                    .sum::<f64>()
                    + f64::from(rows[r][3]).abs();
                let magnitude = if r == 2 {
                    magnitude * f64::from(f32::from_bits(vp[16])).abs()
                } else {
                    magnitude
                };
                let error = (f64::from(value) - f64::from(expected)).abs();
                max_error = max_error.max(error);
                assert!(
                    error <= 2.0 * gamma * magnitude,
                    "skin{influences} case{i} component{r}: {value} vs {expected}, bound {}",
                    2.0 * gamma * magnitude
                );
            }
        }
        drop(mapped);
        readback.unmap();
        // Validate the actual render pipelines and bind layouts, including
        // original UINT8/UNORM8 vertex formats and both cutout pixel families.
        let attributes = [
            (0, VertexFormat::Float32x3, 0),
            (1, VertexFormat::Float32x2, 12),
            (2, VertexFormat::Uint8x4, 20),
            (3, VertexFormat::Unorm8x4, 24),
            (4, VertexFormat::Uint8x4, 28),
            (5, VertexFormat::Unorm8x4, 32),
        ]
        .map(|(shader_location, format, offset)| VertexAttribute {
            shader_location,
            format,
            offset,
        });
        let texture = device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let image = texture.create_view(&TextureViewDescriptor::default());
        let sampler = native_shadow_alpha_sampler(device);
        for (alpha, dither) in [(false, false), (true, false), (true, true)] {
            let raster = NativeOpaqueShadowPipeline::with_program(
                device,
                adapter,
                &RawVertexBufferLayout {
                    array_stride: 36,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &attributes,
                },
                DepthBiasState::default(),
                TextureFormat::Depth24PlusStencil8,
                FrontFace::Ccw,
                None,
                NativeShadowDepthProgram::Skinned {
                    influences: *influences as u8,
                    alpha,
                    dither,
                },
                None,
            )
            .unwrap();
            raster
                .bind_prepared_model_state(device, &constants)
                .unwrap();
            raster
                .bind_skinned_instances(device, &palette_gpu, alpha.then_some((&image, &sampler)))
                .unwrap();
        }
        eprintln!("skin{influences}: {count} original positions within refactoring bound; maximum absolute error {max_error}; opaque/alpha/dither pipelines validated");
    }
    let cases = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/skin_shadow_dither_cases.txt"
    ))
    .lines()
    .map(|line| {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    })
    .collect::<Vec<_>>();
    assert_eq!(cases.len(), 128);
    let alphas = buffer(
        &cases.iter().map(|v| v[0]).collect::<Vec<_>>(),
        BufferUsages::STORAGE,
    );
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 4096,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 4096,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: None,
        source: ShaderSource::Wgsl(
            format!(
                "const SKIN_INFLUENCES:u32=4u; const SKIN_DITHER:bool=true;\n{}\n{}",
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/shaders/dh2_shadow_skin.wgsl"
                )),
                r#"
        @group(0) @binding(3) var<storage,read> test_alphas:array<f32>;
        @group(0) @binding(4) var<storage,read_write> test_results:array<u32>;
        @compute @workgroup_size(8) fn parity_pixel(@builtin(global_invocation_id) id:vec3<u32>){
            let p=vec2(id.x%8u,id.x/8u);
            test_results[id.x]=select(0u,1u,skin_dither_survives(test_alphas[p.y],p));
        }"#
            )
            .into(),
        ),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("parity_pixel"),
            compilation_options: default(),
            cache: None,
        });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 3,
                    resource: alphas.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(128, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4096);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| sender.send(r).unwrap());
    device
        .wgpu_device()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    for (i, word) in mapped.chunks_exact(4).enumerate() {
        assert_eq!(
            u32::from_le_bytes(word.try_into().unwrap()),
            (cases[i / 8][1] >> (i % 8)) & 1,
            "actual dither pixel x{} y{}",
            i % 8,
            i / 8
        );
    }
    drop(mapped);
    readback.unmap();
    eprintln!("hair/wings dither: all 1024 original pixel discard results match");
}
