use super::*;
use bevy::prelude::*;
use bevy::render::{
    render_resource::*,
    renderer::{RenderAdapter, RenderQueue},
    RenderApp,
};

#[test]
fn native_gpu_depth_stencil_preserves_all_operations_and_disabled_tests() {
    let expected = [
        StencilOperation::Keep,
        StencilOperation::Zero,
        StencilOperation::Replace,
        StencilOperation::IncrementClamp,
        StencilOperation::DecrementClamp,
        StencilOperation::Invert,
        StencilOperation::IncrementWrap,
        StencilOperation::DecrementWrap,
    ];
    for code in 0..8u32 {
        let mut state = [0; 9];
        crate::render_state::reset_native_render_state(&mut state);
        state[4] = (code << 8)
            | (code << 11)
            | (code << 14)
            | (7 << 17)
            | (code << 20)
            | (code << 23)
            | (code << 26)
            | (7 << 29);
        state[5] |= 32;
        let gpu = native_depth_stencil_gpu(&state, TextureFormat::Depth32FloatStencil8).unwrap();
        for face in [gpu.stencil.front, gpu.stencil.back] {
            assert_eq!(face.compare, CompareFunction::Always);
            assert_eq!(face.fail_op, expected[code as usize]);
            assert_eq!(face.depth_fail_op, expected[code as usize]);
            assert_eq!(face.pass_op, expected[code as usize]);
        }
        assert!(native_depth_stencil_gpu(&state, TextureFormat::Depth32Float).is_err());
        state[5] &= !(8 | 32);
        let disabled = native_depth_stencil_gpu(&state, TextureFormat::Depth32Float).unwrap();
        assert_eq!(disabled.depth_compare, Some(CompareFunction::Always));
        assert_eq!(disabled.depth_write_enabled, Some(false));
        assert_eq!(disabled.stencil, StencilState::default());
    }
    assert!(native_depth_stencil_gpu(&[0; 9], TextureFormat::Rgba8Unorm).is_err());
}

#[test]
fn native_shadow_opaque_rejects_unsupported_depth_bias_clamp() {
    let captured_bias = DepthBiasState {
        constant: 80,
        slope_scale: 5.0,
        clamp: 1000.0,
    };
    let error = validate_opaque_depth_bias(captured_bias, DownlevelFlags::empty()).unwrap_err();
    assert_eq!(
        error,
        "native opaque shadow depth-bias clamp 1000 is unsupported: adapter lacks DownlevelFlags::DEPTH_BIAS_CLAMP"
    );
    assert!(validate_opaque_depth_bias(
        DepthBiasState {
            clamp: 0.0,
            ..captured_bias
        },
        DownlevelFlags::empty(),
    )
    .is_ok());
}

#[test]
#[ignore = "requires a graphics adapter; verifies native D16/D24S8 layer views and linear/Less/clamp sampling"]
fn native_shadow_depth_array_and_sampler_gpu() {
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
    let size = Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 2,
    };
    for format in [
        TextureFormat::Depth16Unorm,
        TextureFormat::Depth24PlusStencil8,
    ] {
        let depths = NativeShadowDepthArray::new_with_format(
            device,
            size,
            format,
            "native shadow storage fixture",
        )
        .unwrap();
        assert_eq!(depths.texture.format(), format);
        let copied_cache = NativeShadowDepthArray::new_with_format(
            device,
            size,
            format,
            "whole-resource native static cache fixture",
        )
        .unwrap();
        let image_alias = NativeShadowDepthArray::from_owned_texture(
            depths.texture.clone(),
            "shadow attachment aliases sampled image",
        )
        .unwrap();
        assert_eq!(image_alias.texture.id(), depths.texture.id());
        let parallel = NativeShadowDepthArray::parallel(
            device,
            256,
            0,
            1,
            format == TextureFormat::Depth24PlusStencil8,
            "parallel quality format fixture",
        )
        .unwrap();
        assert_eq!(parallel.texture.format(), format);
        assert_eq!(depths.layers.len(), 2);
        assert!(
            NativeShadowDepthArray::new(device, Extent3d { width: 0, ..size }, "invalid").is_err()
        );
        let fill = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("shadow depth storage fixture fill"),
            source: ShaderSource::Wgsl(
                r#"
            @group(0) @binding(0) var<uniform> layer: vec4<u32>;
            @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let p = array<vec2<f32>,3>(vec2<f32>(-1,-1),vec2<f32>(3,-1),vec2<f32>(-1,3));
                return vec4<f32>(p[i],0,1);
            }
            @fragment fn fragment(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
                return select(0.25,0.75,(p.x >= 2.0) != (layer.x == 1u));
            }
        "#
                .into(),
            ),
        });
        let fill_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("native D24S8 fixture fill"),
            layout: None,
            vertex: RawVertexState {
                module: &fill,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: default(),
            },
            fragment: Some(RawFragmentState {
                module: &fill,
                entry_point: Some("fragment"),
                targets: &[],
                compilation_options: default(),
            }),
            primitive: default(),
            multisample: default(),
            multiview_mask: None,
            cache: None,
            depth_stencil: Some(DepthStencilState {
                format,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Always),
                stencil: default(),
                bias: default(),
            }),
        });
        let probe = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("native comparison sampler fixture"),
            source: ShaderSource::Wgsl(
                r#"
            @group(0) @binding(0) var depth: texture_depth_2d_array;
            @group(0) @binding(1) var comparison: sampler_comparison;
            @group(0) @binding(2) var<storage,read_write> result: array<f32>;
            @compute @workgroup_size(1) fn main() {
                let xs = array<f32,6>(0.25,0.75,1.25,-0.25,2.25,0.5);
                for (var layer=0; layer<2; layer++) {
                    for (var i=0u; i<6u; i++) {
                        result[u32(layer)*6u+i] = textureSampleCompareLevel(
                            depth,comparison,vec2<f32>(xs[i],0.25),layer,0.5);
                    }
                }
            }
        "#
                .into(),
            ),
        });
        let pipeline =
            device
                .wgpu_device()
                .create_compute_pipeline(&RawComputePipelineDescriptor {
                    label: Some("native shadow sampler probe"),
                    layout: None,
                    module: &probe,
                    entry_point: Some("main"),
                    compilation_options: default(),
                    cache: None,
                });
        let sampler = native_shadow_comparison_sampler(device);
        let result = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 48,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: if format.has_stencil_aspect() {
                256 + 2 * 4 * 256
            } else {
                48
            },
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bindings = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&copied_cache.sampled),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: result.as_entire_binding(),
                    },
                ],
            });
        let mut encoder = device.create_command_encoder(&default());
        assert!(
            depths
                .copy_resource_from(&mut encoder, &image_alias)
                .is_err(),
            "aliased resources cannot copy onto themselves"
        );
        let wrong_size = NativeShadowDepthArray::new_with_format(
            device,
            Extent3d {
                depth_or_array_layers: 1,
                ..size
            },
            format,
            "invalid whole-cache dimensions",
        )
        .unwrap();
        assert!(
            wrong_size
                .copy_resource_from(&mut encoder, &depths)
                .is_err(),
            "whole-copy must not silently select layer zero"
        );
        let other_format = if format == TextureFormat::Depth16Unorm {
            TextureFormat::Depth24PlusStencil8
        } else {
            TextureFormat::Depth16Unorm
        };
        let wrong_format = NativeShadowDepthArray::new_with_format(
            device,
            size,
            other_format,
            "invalid cache format",
        )
        .unwrap();
        assert!(
            wrong_format
                .copy_resource_from(&mut encoder, &depths)
                .is_err(),
            "whole-copy must not convert depth formats"
        );
        for layer in 0..2_u32 {
            let parameter = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &[layer.to_le_bytes(), [0; 4], [0; 4], [0; 4]].concat(),
                usage: BufferUsages::UNIFORM,
            });
            let bind = device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &fill_pipeline.get_bind_group_layout(0),
                    entries: &[BindGroupEntry {
                        binding: 0,
                        resource: parameter.as_entire_binding(),
                    }],
                });
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &image_alias.layers[layer as usize],
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: (format == TextureFormat::Depth24PlusStencil8).then_some(
                        Operations {
                            load: LoadOp::Clear(13 + layer),
                            store: StoreOp::Store,
                        },
                    ),
                }),
            });
            pass.set_pipeline(&fill_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // Native topology: slice -> one-layer temporary -> whole static cache;
        // dynamic changes only the temporary. Next frame restores the static
        // resource, then transfers its final subresource to the sampled atlas.
        let static_snapshot = NativeShadowDepthArray::new_with_format(
            device,
            Extent3d {
                depth_or_array_layers: 1,
                ..size
            },
            format,
            "native separate static snapshot fixture",
        )
        .unwrap();
        assert!(depths
            .copy_layer_from(&mut encoder, 2, &wrong_size, 0)
            .is_err());
        assert!(depths
            .copy_layer_from(&mut encoder, 0, &wrong_format, 0)
            .is_err());
        assert!(depths
            .copy_layer_from(&mut encoder, 0, &image_alias, 1)
            .is_err());
        wrong_size
            .copy_layer_from(&mut encoder, 0, &depths, 1)
            .unwrap();
        static_snapshot
            .copy_resource_from(&mut encoder, &wrong_size)
            .unwrap();
        {
            // Deliberately different dynamic-frame contents; never save these.
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("synthetic dynamic-frame cache contamination guard"),
                color_attachments: &[],
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &wrong_size.layers[0],
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.9),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: format.has_stencil_aspect().then_some(Operations {
                        load: LoadOp::Clear(42),
                        store: StoreOp::Store,
                    }),
                }),
            });
        }
        depths
            .copy_layer_from(&mut encoder, 1, &wrong_size, 0)
            .unwrap();
        wrong_size
            .copy_resource_from(&mut encoder, &static_snapshot)
            .unwrap();
        depths
            .copy_layer_from(&mut encoder, 1, &wrong_size, 0)
            .unwrap();
        copied_cache
            .copy_resource_from(&mut encoder, &depths)
            .unwrap();
        if format.has_stencil_aspect() {
            let mut stencil_source = copied_cache.texture.as_image_copy();
            stencil_source.aspect = TextureAspect::StencilOnly;
            encoder.copy_texture_to_buffer(
                stencil_source,
                TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: TexelCopyBufferLayout {
                        offset: 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(4),
                    },
                },
                size,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, 48);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |r| {
            tx.send(r).unwrap();
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
        if format.has_stencil_aspect() {
            for layer in 0..2 {
                for y in 0..4 {
                    let offset = 256 + (layer * 4 + y) * 256;
                    assert_eq!(
                        &bytes[offset..offset + 4],
                        &[13 + layer as u8; 4],
                        "whole cache copy preserves stencil, not just depth"
                    );
                }
            }
        }
        let actual: Vec<f32> = bytes[..48]
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(
            actual,
            [0.0, 1.0, 1.0, 0.0, 1.0, 0.5, 1.0, 0.0, 0.0, 1.0, 0.0, 0.5],
            "linear/Less/clamp addressing, fractional hardware PCF and independent array layers"
        );
        drop(bytes);
        readback.unmap();
    }
}

#[test]
#[ignore = "requires a graphics adapter; verifies the original rigid-opaque indexed D24S8 shadow draw and clip-depth transform"]
fn native_shadow_indexed_draws_and_alpha_match_original_depth_paths() {
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
    let size = Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 1,
    };
    let depths =
        NativeShadowDepthArray::new(device, size, "opaque shadow draw depth target").unwrap();
    let attributes = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let vertex_layout = RawVertexBufferLayout {
        array_stride: NATIVE_SHADOW_OPAQUE_VERTEX_STRIDE,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    };
    let bias = DepthBiasState {
        constant: 80,
        slope_scale: 5.0,
        clamp: 1000.0,
    };
    let pipeline = match NativeOpaqueShadowPipeline::new(device, adapter, &vertex_layout, bias) {
        Ok(pipeline) => pipeline,
        Err(error)
            if !adapter
                .get_downlevel_capabilities()
                .flags
                .contains(DownlevelFlags::DEPTH_BIAS_CLAMP) =>
        {
            assert!(
                error.contains("DownlevelFlags::DEPTH_BIAS_CLAMP"),
                "unexpected unsupported-clamp diagnostic: {error}"
            );
            return;
        }
        Err(error) => panic!("native opaque shadow pipeline creation failed: {error}"),
    };

    // A 32-byte native vertex record; the other five floats are deliberately
    // present but not consumed by vertex program 17296.
    let vertex_words = [
        -1.0_f32, -1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, -1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0,
        -1.0, 3.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0,
    ];
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("opaque native shadow 32-byte vertex fixture"),
        contents: &vertex_words
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    // Cw front-face state is evaluated after the viewport's y inversion.
    let index_words = [0_u16, 2, 1];
    let indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("opaque native shadow UInt16 index fixture"),
        contents: &index_words
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::INDEX,
    });
    // Native CB order w,z,y,x,depthhack,padding. The captured shader applies
    // depthhack after its z-row dot: (0.25*0.5 + 0.25)*2 = 0.75.
    let state_record = crate::shadow_pass::native_shadow_instance_cb(
        &[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.25, 0.25],
            [0.0, 0.0, 0.0, 1.0],
        ],
        2.0,
    );
    let state_offset = u64::from(device.limits().min_uniform_buffer_offset_alignment) * 2;
    let mut state_bytes = vec![0; state_offset as usize];
    state_bytes.extend_from_slice(&state_record);
    let state = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("pooled native opaque shadow PerInstanceCB fixture"),
        contents: &state_bytes,
        usage: BufferUsages::UNIFORM,
    });
    // The zero prefix would produce no triangle: successful comparison below
    // therefore verifies that the correct pooled record, not offset zero, ran.
    assert!(pipeline.bind_prepared_model_state(device, &state).is_ok());
    assert!(pipeline
        .bind_prepared_model_state_at(device, &state, 1)
        .is_err());
    assert!(pipeline
        .bind_prepared_model_state_at(device, &state, state_offset + 4)
        .is_err());
    assert!(pipeline
        .bind_prepared_model_state_at(device, &state, u64::MAX)
        .is_err());
    // The shared descriptor must select the second record through its dynamic offset.
    let model_state = pipeline.bind_prepared_model_state(device, &state).unwrap();
    // Existing rewrite meshes are not the original captured 32-byte stream.
    // Use an unchanged 60-byte owner buffer, with the same float3 positions.
    let owner_layout = RawVertexBufferLayout {
        array_stride: 60,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    };
    // game.rs::native_render_indices reverses imported native triangles once;
    // test that the same owner allocation renders with the corresponding CCW
    // raster state instead of accidentally culling every native front face.
    let owner_pipeline = NativeOpaqueShadowPipeline::new_with_raster_state(
        device,
        adapter,
        &owner_layout,
        bias,
        TextureFormat::Depth24PlusStencil8,
        FrontFace::Ccw,
        Some(Face::Back),
    )
    .unwrap();
    let owner_indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("unchanged CCW-converted mesh-owner index fixture"),
        contents: &[0_u16, 1, 2]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::INDEX,
    });
    let mut owner_words = vec![123.0_f32; 3 * 15];
    for vertex in 0..3 {
        owner_words[vertex * 15..vertex * 15 + 3]
            .copy_from_slice(&vertex_words[vertex * 8..vertex * 8 + 3]);
    }
    let owner_vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("unchanged 60-byte mesh-owner shadow fixture"),
        contents: &owner_words
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let owner_model_state = owner_pipeline
        .bind_prepared_model_state_at(device, &state, state_offset)
        .unwrap();
    let draw = NativeOpaqueShadowDraw {
        model_state_offset: state_offset as u32,
        model_state: &model_state,
        program_resources: None,
        vertex_buffer: &vertices,
        index_buffer: &indices,
        index_format: IndexFormat::Uint16,
        indices: 0..3,
        base_vertex: 0,
        instances: 0..1,
        scissor: NativeShadowScissor {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        },
    };

    let probe = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("opaque shadow depth result probe"),
        source: ShaderSource::Wgsl(
            r#"
            @group(0) @binding(0) var depth: texture_depth_2d_array;
            @group(0) @binding(1) var comparison: sampler_comparison;
            @group(0) @binding(2) var<storage, read_write> result: array<f32>;
            @compute @workgroup_size(1) fn main() {
                result[0] = textureSampleCompareLevel(depth, comparison, vec2(0.125, 0.125), 0, 0.70);
                result[1] = textureSampleCompareLevel(depth, comparison, vec2(0.125, 0.125), 0, 0.80);
            }
        "#
            .into(),
        ),
    });
    let probe_pipeline =
        device
            .wgpu_device()
            .create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some("opaque shadow depth comparison probe"),
                layout: None,
                module: &probe,
                entry_point: Some("main"),
                compilation_options: default(),
                cache: None,
            });
    let sampler = native_shadow_comparison_sampler(device);
    let result = device.create_buffer(&BufferDescriptor {
        label: Some("opaque shadow depth comparison results"),
        size: 8,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("opaque and alpha shadow depth comparison readback"),
        size: 64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let probe_bindings = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: Some("opaque shadow depth comparison bindings"),
            layout: &probe_pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&depths.sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: result.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&default());
    pipeline
        .draw_layer(
            &mut encoder,
            &depths,
            0,
            Operations {
                load: LoadOp::Clear(1.0),
                store: StoreOp::Store,
            },
            Some(Operations {
                load: LoadOp::Clear(0),
                store: StoreOp::Store,
            }),
            &[draw],
        )
        .unwrap();
    let probe_depth = |encoder: &mut CommandEncoder, offset: u64| {
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&probe_pipeline);
            pass.set_bind_group(0, &probe_bindings, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, offset, 8);
    };
    probe_depth(&mut encoder, 0);
    // No dirty owner: an empty cached reuse must retain the previous triangle.
    pipeline
        .draw_layer(
            &mut encoder,
            &depths,
            0,
            Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            None,
            &[],
        )
        .unwrap();
    probe_depth(&mut encoder, 8);
    // A dirty owner with no casters still explicitly clears the layer. It must
    // not keep an occluder left by the previous cached draw.
    pipeline
        .draw_layer(
            &mut encoder,
            &depths,
            0,
            Operations {
                load: LoadOp::Clear(1.0),
                store: StoreOp::Store,
            },
            None,
            &[],
        )
        .unwrap();
    probe_depth(&mut encoder, 16);
    let owner_draw = NativeOpaqueShadowDraw {
        model_state_offset: 0,
        model_state: &owner_model_state,
        program_resources: None,
        vertex_buffer: &owner_vertices,
        index_buffer: &owner_indices,
        index_format: IndexFormat::Uint16,
        indices: 0..3,
        base_vertex: 0,
        instances: 0..1,
        scissor: NativeShadowScissor {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        },
    };
    owner_pipeline
        .draw_layer(
            &mut encoder,
            &depths,
            0,
            Operations {
                load: LoadOp::Clear(1.0),
                store: StoreOp::Store,
            },
            None,
            &[owner_draw],
        )
        .unwrap();
    probe_depth(&mut encoder, 24);

    // arkDefaultHelpers::ClipZorShadowPass uses raw diffuse alpha with the
    // bilinear sampler and a FIXED 0.5 cutoff, independent of forward cutoff.
    // A two-texel FP16 map distinguishes nearest/linear and tests equality:
    // u=.25 -> .25 (discard), u=.5 -> .5 (keep), u=.75 -> .75 (keep).
    let alpha_attributes = [
        attributes[0],
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
    ];
    let alpha_layout = RawVertexBufferLayout {
        array_stride: 60,
        step_mode: VertexStepMode::Vertex,
        attributes: &alpha_attributes,
    };
    let alpha_pipeline = NativeOpaqueShadowPipeline::new_alpha_with_raster_state(
        device,
        adapter,
        &alpha_layout,
        bias,
        TextureFormat::Depth24PlusStencil8,
        FrontFace::Ccw,
        Some(Face::Back),
    )
    .unwrap();
    let alpha_model = alpha_pipeline
        .bind_prepared_model_state_at(device, &state, state_offset)
        .unwrap();
    let diffuse = device.create_texture(&TextureDescriptor {
        label: Some("native raw shadow-alpha fixture"),
        size: Extent3d {
            width: 2,
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
    let texels = [0_u16, 0, 0, 0x3400, 0, 0, 0, 0x3a00]; // alpha .25/.75
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &diffuse,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &texels
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(1),
        },
        diffuse.size(),
    );
    let diffuse_view = diffuse.create_view(&TextureViewDescriptor::default());
    let bilinear = device.create_sampler(&SamplerDescriptor {
        label: Some("explicit shadow-alpha fixture bilinear sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    for (case, u) in [0.25_f32, 0.5, 0.75].into_iter().enumerate() {
        let uv = [0.0_f32, 0.0, u, 0.5];
        let uv_state = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("native selected layer-zero transform fixture"),
            contents: &uv.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>(),
            usage: BufferUsages::UNIFORM,
        });
        let material = alpha_pipeline
            .bind_alpha_material(device, &diffuse_view, &bilinear, &uv_state)
            .unwrap();
        let draw = NativeOpaqueShadowDraw {
            model_state_offset: 0,
            model_state: &alpha_model,
            program_resources: Some(&material),
            vertex_buffer: &owner_vertices,
            index_buffer: &owner_indices,
            index_format: IndexFormat::Uint16,
            indices: 0..3,
            base_vertex: 0,
            instances: 0..1,
            scissor: NativeShadowScissor {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
        };
        alpha_pipeline
            .draw_layer(
                &mut encoder,
                &depths,
                0,
                Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                },
                None,
                &[draw],
            )
            .unwrap();
        probe_depth(&mut encoder, 32 + case as u64 * 8);
    }
    // Native cached alpha VS passes raw UV; its PS transforms AFTER raster
    // interpolation. At this pixel raw U rounds 2^24+0.5 down to 2^24, so
    // the PS obtains zero and discards. Transforming each vertex first would
    // interpolate 0.5 instead and incorrectly keep the fragment.
    let mut large_uv_words = owner_words.clone();
    for vertex in 0..3 {
        large_uv_words[vertex * 15 + 3] = if vertex == 1 { 16777224.0 } else { 16777216.0 };
        large_uv_words[vertex * 15 + 4] = 0.5;
    }
    let large_uv_vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("raw-UV interpolation order regression"),
        contents: &large_uv_words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::VERTEX,
    });
    let large_uv = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("pixel-stage UV cancellation"),
        contents: &[1.0_f32, 1.0, -16777216.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
        usage: BufferUsages::UNIFORM,
    });
    let large_uv_material = alpha_pipeline
        .bind_alpha_material(device, &diffuse_view, &bilinear, &large_uv)
        .unwrap();
    let large_uv_draw = NativeOpaqueShadowDraw {
        model_state_offset: 0,
        model_state: &alpha_model,
        program_resources: Some(&large_uv_material),
        vertex_buffer: &large_uv_vertices,
        index_buffer: &owner_indices,
        index_format: IndexFormat::Uint16,
        indices: 0..3,
        base_vertex: 0,
        instances: 0..1,
        scissor: NativeShadowScissor {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        },
    };
    alpha_pipeline
        .draw_layer(
            &mut encoder,
            &depths,
            0,
            Operations {
                load: LoadOp::Clear(1.0),
                store: StoreOp::Store,
            },
            None,
            &[large_uv_draw],
        )
        .unwrap();
    probe_depth(&mut encoder, 56);
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
    let actual: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    assert_eq!(
        actual,
        [1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0],
        "opaque cache/CCW, bilinear alpha<0.5 and pixel-stage UV transform must match native semantics"
    );
    drop(bytes);
    readback.unmap();
}

#[test]
#[ignore = "requires a graphics adapter; compares original D3D11 HLSL weighted-PCF results against WGSL on synthetic D24 checker/ramp layers"]
fn native_shadow_pcf_original_hlsl_gpu_parity() {
    let mut depth_codes = vec![0_u32; 2 * 8 * 8];
    let mut depth_layers = 0;
    let mut positions = Vec::<f32>::new();
    let mut settings = Vec::<u32>::new();
    let mut expected = Vec::<f32>::new();
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/shadow_pcf_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        match fields.first().copied() {
            Some("MAP") => assert_eq!(fields, ["MAP", "8", "8", "2", "16777215"]),
            Some("DEPTH") => {
                let layer: usize = fields[1].parse().unwrap();
                assert_eq!(fields.len(), 66, "one 8x8 D24 code row per layer");
                for (pixel, code) in fields[2..].iter().enumerate() {
                    depth_codes[layer * 64 + pixel] = u32::from_str_radix(code, 16).unwrap();
                }
                depth_layers += 1;
            }
            Some("CASE") => {
                let filtering_quality: u32 = fields[1].parse().unwrap();
                let layer: u32 = fields[2].parse().unwrap();
                let bits = |index| u32::from_str_radix(fields[index], 16).unwrap();
                positions.extend([
                    f32::from_bits(bits(3)),
                    f32::from_bits(bits(4)),
                    f32::from_bits(bits(5)),
                    layer as f32,
                ]);
                settings.extend([layer, filtering_quality, 0, 0]);
                expected.push(f32::from_bits(bits(6)));
            }
            Some("#") | None => {}
            Some(other) => panic!("unexpected original-PCF fixture record {other:?}"),
        }
    }
    assert_eq!(depth_layers, 2);
    assert_eq!(
        expected.len(),
        147,
        "3 optimized-PCF kernels x (40 finite and9 non-finite positions)"
    );

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
    let depths = NativeShadowDepthArray::new(
        device,
        Extent3d {
            width: 8,
            height: 8,
            depth_or_array_layers: 2,
        },
        "original shadow PCF parity D24 array",
    )
    .unwrap();

    // Render the fixture's exact 24-bit codes into D24S8; the fragment shader
    // converts each integer code to its normalized D24 depth, not a CPU oracle.
    let depth_bytes: Vec<u8> = depth_codes
        .iter()
        .flat_map(|code| code.to_le_bytes())
        .collect();
    let depth_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("synthetic D24 checker/ramp codes"),
        contents: &depth_bytes,
        usage: BufferUsages::STORAGE,
    });
    let fill_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("synthetic D24 code-array raster fill"),
        source: ShaderSource::Wgsl(
            r#"
            @group(0) @binding(0) var<storage, read> codes: array<u32>;
            @group(0) @binding(1) var<uniform> layer: u32;
            @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                let p = array<vec2<f32>,3>(vec2<f32>(-1,-1),vec2<f32>(3,-1),vec2<f32>(-1,3));
                return vec4<f32>(p[i],0,1);
            }
            @fragment fn fragment(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
                let code = codes[layer * 64u + u32(p.y) * 8u + u32(p.x)];
                return f32(code) / 16777215.0;
            }
        "#
            .into(),
        ),
    });
    let fill_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("synthetic D24 code-array raster pipeline"),
        layout: None,
        vertex: RawVertexState {
            module: &fill_shader,
            entry_point: Some("vertex"),
            buffers: &[],
            compilation_options: default(),
        },
        fragment: Some(RawFragmentState {
            module: &fill_shader,
            entry_point: Some("fragment"),
            targets: &[],
            compilation_options: default(),
        }),
        primitive: default(),
        multisample: default(),
        multiview_mask: None,
        cache: None,
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth24PlusStencil8,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: default(),
            bias: default(),
        }),
    });

    let pcf_source = format!(
        "{}\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow.wgsl"
        )),
        r#"
        @group(0) @binding(0) var map: texture_depth_2d_array;
        @group(0) @binding(1) var comparison: sampler_comparison;
        @group(0) @binding(2) var<storage, read_write> results: array<f32>;
        @group(0) @binding(3) var<storage, read> positions: array<vec4<f32>>;
        @group(0) @binding(4) var<storage, read> settings: array<vec4<u32>>;
        @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
            let p = positions[id.x];
            let s = settings[id.x];
            results[id.x] = native_shadow_pcf(
                map, comparison, p.xyz, i32(s.x), vec2<f32>(8.0), i32(s.y));
        }
    "#
    );
    let pcf_shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("original HLSL weighted shadow-PCF parity"),
        source: ShaderSource::Wgsl(pcf_source.into()),
    });
    let pcf_pipeline =
        device
            .wgpu_device()
            .create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some("original HLSL weighted shadow-PCF parity"),
                layout: None,
                module: &pcf_shader,
                entry_point: Some("main"),
                compilation_options: default(),
                cache: None,
            });

    let positions_bytes: Vec<u8> = positions
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let settings_bytes: Vec<u8> = settings
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let positions_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("original-PCF input positions"),
        contents: &positions_bytes,
        usage: BufferUsages::STORAGE,
    });
    let settings_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("original-PCF quality/layer inputs"),
        contents: &settings_bytes,
        usage: BufferUsages::STORAGE,
    });
    let result_size = (expected.len() * 4) as u64;
    let result = device.create_buffer(&BufferDescriptor {
        label: Some("original-PCF WGSL results"),
        size: result_size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("original-PCF WGSL result readback"),
        size: result_size,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = native_shadow_comparison_sampler(device);
    let fill_parameter = |layer: u32| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("synthetic D24 layer selector"),
            contents: &layer.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        })
    };
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("original-PCF parity encoder"),
    });
    for layer in 0..2_u32 {
        let parameter = fill_parameter(layer);
        let bindings = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: Some("synthetic D24 raster-fill bindings"),
                layout: &fill_pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: depth_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: parameter.as_entire_binding(),
                    },
                ],
            });
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("write synthetic D24 checker/ramp layer"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depths.layers[layer as usize],
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: LoadOp::Clear(0),
                    store: StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&fill_pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.draw(0..3, 0..1);
    }
    let pcf_bindings = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: Some("original-PCF parity bindings"),
            layout: &pcf_pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&depths.sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: result.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: positions_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: settings_buffer.as_entire_binding(),
                },
            ],
        });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("original optimized PCF comparison"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pcf_pipeline);
        pass.set_bind_group(0, &pcf_bindings, &[]);
        pass.dispatch_workgroups(expected.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, result_size);
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
    let actual: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|value| f32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    let max_error = actual
        .iter()
        .zip(&expected)
        .map(|(actual, expected)| (actual - expected).abs())
        .fold(0.0_f32, f32::max);
    let mismatches = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .filter(|(_, (actual, expected))| actual.to_bits() != expected.to_bits())
        .map(|(index, (actual, expected))| {
            format!(
                "{index}: WGSL={actual:.9} (0x{:08x}), original={expected:.9} (0x{:08x})",
                actual.to_bits(),
                expected.to_bits()
            )
        })
        .collect::<Vec<_>>();
    eprintln!("original weighted PCF parity: {} cases; max abs error {max_error:.9}; {} non-bitwise matches", expected.len(), mismatches.len());
    drop(bytes);
    readback.unmap();
    assert!(
        mismatches.is_empty(),
        "original HLSL/WGSL weighted-PCF results differ ({} of {}): {}",
        mismatches.len(),
        expected.len(),
        mismatches
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
}
#[test]
#[ignore = "requires a graphics adapter; verifies original light-quality/filter dispatch and distinct fallbacks"]
fn native_shadow_quality_dispatch_gpu() {
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
    let depths = NativeShadowDepthArray::new(
        device,
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        "quality dispatch fixture",
    )
    .unwrap();
    let source = format!(
        "{}\n{}",
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/shaders/dh2_shadow.wgsl"
        )),
        r#"
        @group(0) @binding(0) var map: texture_depth_2d_array;
        @group(0) @binding(1) var comparison: sampler_comparison;
        @group(0) @binding(2) var<storage, read_write> result: array<f32>;
        @compute @workgroup_size(1) fn main() {
            for (var q=0; q<3; q++) {
                result[u32(q)] = f32(native_shadow_selected_filter(2, q));
                result[u32(3+q)] = f32(native_shadow_selected_filter(1, q));
                result[u32(6+q)] = f32(native_shadow_selected_filter(0, q));
            }
            let p=vec4<f32>(0.5,0.5,0.5,1.0); let size=vec2<f32>(4.0);
            result[9] = native_sample_directional_shadow(map, comparison, p, 0, size, -1, 2);
            result[10] = native_sample_local_shadow(map, comparison, p, 0, size, -1, 2);
            result[11] = native_sample_directional_shadow(map, comparison, p, 0, size, 2, 2);
            result[12] = native_sample_local_shadow(map, comparison, p, 0, size, 1, 2);
            result[13] = native_sample_directional_shadow(map, comparison, p, 0, size, 3, 2);
            result[14] = native_sample_local_shadow(map, comparison, p, 0, size, 3, 2);
            result[15] = f32(native_shadow_selected_filter(2, -1));
            result[16] = native_sample_directional_shadow(map, comparison, p, 0, size, 2, -1);
        }
    "#
    );
    let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("original quality dispatch"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("original quality dispatch"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let result = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 17 * 4,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 17 * 4,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = native_shadow_comparison_sampler(device);
    let bindings = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&depths.sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: result.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("explicit unoccluded fixture depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depths.layers[0],
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, 17 * 4);
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
    let expected = [
        0.0, 1.0, 2.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, 1.0,
    ];
    for (index, raw) in bytes.chunks_exact(4).enumerate() {
        let actual = f32::from_le_bytes(raw.try_into().unwrap());
        assert!(
            (actual - expected[index]).abs() <= 0.000001,
            "dispatch {index}: {actual} != {}",
            expected[index]
        );
    }
    drop(bytes);
    readback.unmap();
}
