use super::*;
use bevy::prelude::*;
use bevy::render::{
    render_resource::*,
    renderer::{RenderAdapter, RenderQueue},
    RenderApp,
};

#[test]
fn prepared_shadow_raster_preserves_faces_after_index_conversion() {
    for cull_bits in 0..4 {
        for ccw in [false, true] {
            let mut state = [0; 9];
            state[5] = (cull_bits << 24) | (u32::from(ccw) << 27);
            let expected_cull = match cull_bits {
                0 => None,
                1 => Some(Face::Front),
                _ => Some(Face::Back),
            };
            for reversed in [false, true] {
                let (front, cull) = shadow_mesh_raster(&state, reversed);
                assert_eq!(cull, expected_cull);
                for triangle_ccw in [false, true] {
                    let imported_ccw = triangle_ccw ^ reversed;
                    assert_eq!(
                        imported_ccw == (front == FrontFace::Ccw),
                        triangle_ccw == ccw
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires GPU; verifies inherited shadow depth/write/stencil and imported winding by pixel readback"]
fn prepared_shadow_state_controls_actual_depth_pixels() {
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
    let adapter = world.resource::<RenderAdapter>();
    let queue = world.resource::<RenderQueue>();
    let cases = 5 + 2 * 2 * 2;
    let target = ShadowDepthArray::new(
        device,
        Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: cases,
        },
        "prepared native shadow state pixel cases",
    )
    .unwrap();
    let attributes = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let layout = RawVertexBufferLayout {
        array_stride: 12,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    };
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("prepared native state fullscreen triangle"),
        contents: &[-1.0_f32, -1.0, 0.5, 3.0, -1.0, 0.5, -1.0, 3.0, 0.5]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("prepared native state triangle indices"),
        contents: &[0_u16, 1, 2, 0, 2, 1]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::INDEX,
    });
    let identity =
        std::array::from_fn(|row| std::array::from_fn(|col| if row == col { 1.0 } else { 0.0 }));
    let model = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("prepared native state actual model constants"),
        contents: &crate::shadow_pass::shadow_instance_cb(&identity, 1.0),
        usage: BufferUsages::UNIFORM,
    });
    let mut state = [0; 9];
    crate::render_state::reset_render_state(&mut state);
    // Native bitfield values come from the executable-verified state decoder:
    // compare=1 LESS, compare=4 GREATER; word5 bits3/4/5 enable depth/write/stencil.
    state[5] = (state[5] & !7) | 1;
    let mut states = vec![state; 5];
    states[1][5] = (state[5] & !7) | 4;
    states[2][5] &= !16;
    states[3][5] &= !8;
    states[4][5] |= 32;
    // Compare NEVER on both stencil faces; no fragment can write depth.
    states[4][4] &= !((7 << 17) | (7 << 29));
    // Every native front convention / FRONT or BACK cull combination is
    // submitted with both original and reversed index streams.
    for ccw in [false, true] {
        for cull_bits in [1, 2] {
            for _reversed in [false, true] {
                let mut raster = state;
                raster[5] = (raster[5] & !((3 << 24) | (1 << 27)))
                    | (cull_bits << 24)
                    | (u32::from(ccw) << 27);
                states.push(raster);
            }
        }
    }
    let pipelines: Vec<_> = states
        .iter()
        .enumerate()
        .map(|(case, state)| {
            let (front, cull) = if case < 5 {
                (FrontFace::Ccw, None)
            } else {
                shadow_mesh_raster(state, (case - 5) % 2 != 0)
            };
            ShadowDepthPipeline::new_prepared_shadow_depth(
                device,
                adapter,
                &layout,
                state,
                91,
                target.texture.format(),
                front,
                cull,
                ShadowDepthProgram::Opaque,
            )
            .unwrap()
        })
        .collect();
    let bindings: Vec<_> = pipelines
        .iter()
        .map(|pipeline| pipeline.bind_prepared_model_state(device, &model).unwrap())
        .collect();
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("prepared state depth/write/stencil cases"),
    });
    for (layer, (pipeline, binding)) in pipelines.iter().zip(&bindings).enumerate() {
        pipeline
            .draw_layer(
                &mut encoder,
                &target,
                layer as u32,
                Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                },
                Some(Operations {
                    load: LoadOp::Clear(0),
                    store: StoreOp::Store,
                }),
                &[ShadowDepthDraw {
                    model_state_offset: 0,
                    model_state: binding,
                    program_resources: None,
                    vertex_buffer: &vertices,
                    index_buffer: &indices,
                    index_format: IndexFormat::Uint16,
                    indices: if layer >= 5 && (layer - 5) % 2 != 0 {
                        3..6
                    } else {
                        0..3
                    },
                    base_vertex: 0,
                    instances: 0..1,
                    scissor: ShadowScissor {
                        x: 0,
                        y: 0,
                        width: 2,
                        height: 2,
                    },
                }],
            )
            .unwrap();
    }
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("prepared native depth pixel readback"),
        source: ShaderSource::Wgsl(
            r#"
            @group(0) @binding(0) var depths: texture_depth_2d_array;
            @group(0) @binding(1) var<storage, read_write> pixels: array<f32>;
            @compute @workgroup_size(1) fn read(@builtin(global_invocation_id) id: vec3<u32>) {
                pixels[id.x] = textureLoad(depths, vec2<i32>(0), i32(id.x), 0);
            }
        "#
            .into(),
        ),
    });
    let read_pipeline =
        device
            .wgpu_device()
            .create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some("prepared native depth read pipeline"),
                layout: None,
                module: &shader,
                entry_point: Some("read"),
                compilation_options: default(),
                cache: None,
            });
    let output = device.create_buffer(&BufferDescriptor {
        label: Some("prepared native depth results"),
        size: u64::from(cases) * 4,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("prepared native depth readback"),
        size: u64::from(cases) * 4,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let read_layout = BindGroupLayout::from(read_pipeline.get_bind_group_layout(0));
    let binding = device.create_bind_group(
        "prepared native depth results",
        &read_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&target.sampled),
            },
            BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    );
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&read_pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(cases, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, u64::from(cases) * 4);
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
    let mapped = readback.slice(..).get_mapped_range();
    let pixels: Vec<_> = mapped
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect();
    assert!(
        (pixels[0] - 0.5).abs() <= f32::EPSILON,
        "LESS must write the actual triangle depth: {pixels:?}"
    );
    assert_eq!(
        &pixels[1..5],
        &[1.0; 4],
        "GREATER, writes disabled, depth disabled, and stencil NEVER must retain the clear depth"
    );
    assert_eq!(
        &pixels[5..],
        &[0.5, 0.5, 1.0, 1.0, 1.0, 1.0, 0.5, 0.5],
        "front/back selection must survive imported index reversal: {pixels:?}"
    );
    drop(mapped);
    readback.unmap();
}

#[test]
#[ignore = "requires GPU; verifies local atlas viewport/scissor mapping and preservation in D16/D24S8"]
fn local_atlas_region_draws_preserve_viewport_depth_and_neighbor_pixels() {
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
    let adapter = world.resource::<RenderAdapter>();
    let queue = world.resource::<RenderQueue>();
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[-1.0f32, -1., 0., 3., -1., 1., -1., 3., 0.]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &[0u8, 0, 1, 0, 2, 0],
        usage: BufferUsages::INDEX,
    });
    let identity = std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1. } else { 0. }));
    let model = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &crate::shadow_pass::shadow_instance_cb(&identity, 1.),
        usage: BufferUsages::UNIFORM,
    });
    let attributes = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let layout = RawVertexBufferLayout {
        array_stride: 12,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    };
    let mut state = [0; 9];
    crate::render_state::reset_render_state(&mut state);
    state[5] = (state[5] & !7) | 1;
    for format in [
        TextureFormat::Depth16Unorm,
        TextureFormat::Depth24PlusStencil8,
    ] {
        let target = ShadowDepthArray::new_with_format(
            device,
            Extent3d {
                width: 8,
                height: 6,
                depth_or_array_layers: 2,
            },
            format,
            "local atlas viewport fixture",
        )
        .unwrap();
        let pipeline = ShadowDepthPipeline::new_prepared_shadow_depth(
            device,
            adapter,
            &layout,
            &state,
            0,
            format,
            FrontFace::Ccw,
            None,
            ShadowDepthProgram::Opaque,
        )
        .unwrap();
        let binding = pipeline.bind_prepared_model_state(device, &model).unwrap();
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        for layer in 0..2 {
            pipeline
                .draw_layer(
                    &mut encoder,
                    &target,
                    layer,
                    Operations {
                        load: LoadOp::Clear(1.),
                        store: StoreOp::Store,
                    },
                    (format == TextureFormat::Depth24PlusStencil8).then_some(Operations {
                        load: LoadOp::Clear(0),
                        store: StoreOp::Store,
                    }),
                    &[],
                )
                .unwrap();
        }
        let regions = [
            ShadowScissor {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            },
            ShadowScissor {
                x: 5,
                y: 3,
                width: 2,
                height: 2,
            },
        ];
        for region in regions {
            let draw = ShadowDepthDraw {
                model_state_offset: 0,
                model_state: &binding,
                program_resources: None,
                vertex_buffer: &vertices,
                index_buffer: &indices,
                index_format: IndexFormat::Uint16,
                indices: 0..3,
                base_vertex: 0,
                instances: 0..1,
                scissor: region,
            };
            ShadowDepthPipeline::draw_layer_region_sequence(
                &mut encoder,
                &target,
                1,
                region,
                &[(&pipeline, &draw)],
            )
            .unwrap();
        }
        assert!(ShadowDepthPipeline::draw_layer_region_sequence(
            &mut encoder,
            &target,
            1,
            ShadowScissor {
                x: 7,
                y: 0,
                width: 2,
                height: 1
            },
            &[],
        )
        .is_err());
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: None,
            source: ShaderSource::Wgsl(
                r#"
            @group(0) @binding(0) var depths:texture_depth_2d_array;
            @group(0) @binding(1) var<storage,read_write> values:array<f32>;
            @compute @workgroup_size(1) fn read(@builtin(global_invocation_id) p:vec3<u32>) {
                values[(p.z*6u+p.y)*8u+p.x]=textureLoad(depths,vec2<i32>(p.xy),i32(p.z),0);
            }"#
                .into(),
            ),
        });
        let reader = device
            .wgpu_device()
            .create_compute_pipeline(&RawComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some("read"),
                compilation_options: default(),
                cache: None,
            });
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 384,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 384,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(
            "local atlas pixel results",
            &BindGroupLayout::from(reader.get_bind_group_layout(0)),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&target.sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        );
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&reader);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(8, 6, 2);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 384);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |v| {
            tx.send(v).unwrap();
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
        let mapped = readback.slice(..).get_mapped_range();
        for (index, bytes) in mapped.chunks_exact(4).enumerate() {
            let (layer, y, x) = (index / 48, (index / 8) % 6, index % 8);
            let actual = f32::from_le_bytes(bytes.try_into().unwrap());
            let expected = regions
                .iter()
                .find(|r| {
                    layer == 1
                        && x >= r.x as usize
                        && x < (r.x + r.width) as usize
                        && y >= r.y as usize
                        && y < (r.y + r.height) as usize
                })
                .map_or(1., |r| {
                    (x as f32 - r.x as f32 + 0.5) / (r.width as f32 * 2.)
                });
            assert!(
                (actual - expected).abs() <= 1. / 65535.,
                "local viewport {format:?} pixel{index}: {actual} vs{expected}"
            );
        }
        drop(mapped);
        readback.unmap();
    }
}
