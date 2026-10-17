use super::*;
use bevy::{
    prelude::*,
    render::{renderer::RenderQueue, storage::ShaderBuffer, RenderApp},
};

fn parameters() -> NativeScreenShadowUniform {
    NativeScreenShadowUniform {
        inverse_view_projection: [Vec4::X, Vec4::Y, Vec4::Z, Vec4::W],
        unproject: Vec4::new(1.0, 0.0, 1.0, 0.0),
        camera_time: Vec4::new(0.0, 0.0, 2.0, 0.0),
        depth_scale_step_hack: Vec4::new(1.0, 1.0, 0.025, 1.0),
        target: UVec4::new(4, 4, 0, 0),
        blur: Vec4::new(-0.5, -4.0, 0.0, 0.0),
        cloud_motion: Vec4::new(5.0, 0.0, 0.0, 0.0),
        cloud_density: Vec4::new(1.0, 1.0, 1.0, 0.0),
        options: UVec4::new(0, 1, 0, 0),
    }
}

#[test]
fn screen_shadow_transport_is_explicit_and_finite() {
    let mut p = parameters();
    assert!(p.validate().is_ok());
    assert_eq!(ShaderBuffer::from(p.clone()).data.unwrap().len(), 192);
    p.target.w = 2;
    assert!(p.validate().is_err());
    p.target.w = 0;
    p.depth_scale_step_hack.x = 0.0;
    assert!(p.validate().is_err());
    p.depth_scale_step_hack.x = 1.0;
    p.cloud_motion.x = f32::NAN;
    assert!(p.validate().is_err());
}

#[test]
fn screen_producer_requires_stencil_and_preserves_other_channels() {
    assert!(screen_producer_states(
        [0; 9],
        TextureFormat::Rgba8Unorm,
        TextureFormat::Depth32Float
    )
    .is_err());
    let states = screen_producer_states(
        [0; 9],
        TextureFormat::Rgba8Unorm,
        TextureFormat::Depth24PlusStencil8,
    )
    .unwrap();
    for (index, state) in states.iter().enumerate() {
        assert_eq!(state.color.write_mask, ColorWrites::GREEN);
        assert_eq!(state.depth.depth_compare, Some(CompareFunction::Always));
        assert_eq!(state.depth.depth_write_enabled, Some(false));
        assert_eq!(state.depth.stencil.read_mask, 1);
        assert_eq!(
            state.depth.stencil.front.compare,
            if index == 0 {
                CompareFunction::Equal
            } else {
                CompareFunction::NotEqual
            }
        );
        assert_eq!(state.stencil_reference, 1);
    }
}

#[test]
fn original_dwao_offset_publication_keeps_order_and_signed_zero() {
    let p =
        NativeDwaoBlurUniform::from_owner([8.0, 8.0], [1.0, 0.0], 1.0, 0.1, 0.251379132).unwrap();
    assert_eq!(
        p.offset0.to_array().map(f32::to_bits),
        [-0.25f32, -0.0, -0.125, -0.0].map(f32::to_bits)
    );
    assert_eq!(p.offset1.to_array(), [0.125, 0.0, 0.25, 0.0]);
    assert_eq!(ShaderBuffer::from(p).data.unwrap().len(), 48);
    assert!(NativeDwaoBlurUniform::from_owner([0.0, 8.0], [1.0, 0.0], 1.0, 0.1, 0.0).is_err());
}

fn upload(device: &RenderDevice, value: impl Into<ShaderBuffer>, usage: BufferUsages) -> Buffer {
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("screen shadow original parameter fixture"),
        contents: &value.into().data.unwrap(),
        usage,
    })
}

#[test]
#[ignore = "requires graphics adapter; tests original reconstruction, cloud and bilateral shader paths"]
fn native_screen_shadow_producer_and_bilateral_gpu() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                        ..default()
                    },
                )),
                ..default()
            })
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
    world.init_resource::<crate::tiled_lighting_gpu::NativeTlfPipelines>();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let make_texture = |label, format, usage, width, height| {
        device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            dimension: TextureDimension::D2,
            mip_level_count: 1,
            sample_count: 1,
            format,
            usage,
            view_formats: &[],
        })
    };
    let depth = make_texture(
        "original scene depth fixture",
        TextureFormat::Depth32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        4,
        4,
    );
    let depth_view = depth.create_view(&TextureViewDescriptor::default());
    let atlas = crate::native_shadow_gpu::NativeShadowDepthArray::new(
        device,
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 5,
        },
        "screen shadow cascade fixture",
    )
    .unwrap();
    let output = make_texture(
        "screen visibility fixture",
        TextureFormat::Rgba32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        4,
        4,
    );
    let output_view = output.create_view(&TextureViewDescriptor::default());
    let blur_x = make_texture(
        "bilateral X fixture",
        TextureFormat::Rgba32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        4,
        4,
    );
    let blur_x_view = blur_x.create_view(&TextureViewDescriptor::default());
    let final_output = make_texture(
        "bilateral Y fixture",
        TextureFormat::Rgba32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        4,
        4,
    );
    let final_view = final_output.create_view(&TextureViewDescriptor::default());
    let cloud = make_texture(
        "original cloud uniform fixture",
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        1,
        1,
    );
    queue.write_texture(
        cloud.as_image_copy(),
        &[128, 0, 0, 255],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let cloud_view = cloud.create_view(&TextureViewDescriptor::default());
    let point = device.create_sampler(&SamplerDescriptor {
        label: Some("original pointClamp"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..default()
    });
    let cloud_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("original bilinear cloud sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let comparison = crate::native_shadow_gpu::native_shadow_comparison_sampler(device);
    let target = ColorTargetState {
        format: TextureFormat::Rgba32Float,
        blend: None,
        write_mask: ColorWrites::ALL,
    };
    let passes = NativeScreenShadowPipelines::new(device, target.clone(), None, target);
    let mut parallel = crate::native_shadow_sampling::NativeParallelSamplingUniform::default();
    let shadow_rows = [
        Vec4::new(0.5, 0.0, 0.0, 0.5),
        Vec4::new(0.0, -0.5, 0.0, 0.5),
        Vec4::Z,
        Vec4::W,
    ];
    parallel.world_to_shadow.fill(Mat4::from_cols_array_2d(
        &shadow_rows.map(|row| row.to_array()),
    ));
    parallel.view_row2 = Vec4::new(0.0, 0.0, -1.0, 0.0);
    parallel.cascade_fars = Vec4::new(4.0, 12.0, 30.0, 75.0);
    parallel.sun_direction = Vec4::Z;
    parallel.shadow_size = Vec4::new(4.0, 4.0, 0.0, 0.0);
    parallel.quality = IVec4::new(2, 1, 0, 0);
    let parallel_buffer = upload(device, parallel, BufferUsages::STORAGE);
    let fill = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("original comparison depth fixture"),
        source: ShaderSource::Wgsl(
            r#"
        @vertex fn v(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            let p=array<vec2<f32>,3>(vec2(-1,-1),vec2(3,-1),vec2(-1,3));return vec4(p[i],0,1);
        }
        @fragment fn f(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
            return select(0.25,0.75,p.x>=2.0);
        }"#
            .into(),
        ),
    });
    let fill_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("comparison split depth"),
        layout: None,
        vertex: RawVertexState {
            module: &fill,
            entry_point: Some("v"),
            compilation_options: default(),
            buffers: &[],
        },
        primitive: PrimitiveState {
            cull_mode: None,
            ..default()
        },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth24PlusStencil8,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: default(),
            bias: default(),
        }),
        multisample: default(),
        fragment: Some(RawFragmentState {
            module: &fill,
            entry_point: Some("f"),
            compilation_options: default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("original source depth fixture"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.5),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    for layer in &atlas.layers {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("original shadow split fixture"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: layer,
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
        pass.draw(0..3, 0..1);
    }
    let p = parameters();
    let uniform = upload(device, p.clone(), BufferUsages::UNIFORM);
    let bind = passes.bind(
        device,
        &parallel_buffer,
        &uniform,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &blur_x_view,
    );
    let ops = Operations {
        load: LoadOp::Clear(LinearRgba::BLACK),
        store: StoreOp::Store,
    };
    let viewport = [0.0, 0.0, 4.0, 4.0, 0.0, 1.0];
    passes.draw(
        &mut encoder,
        false,
        &bind,
        &output_view,
        None,
        0,
        ops,
        viewport,
    );
    let mut x = p.clone();
    x.target.z = 1;
    let x_buffer = upload(device, x.clone(), BufferUsages::UNIFORM);
    let x_bind = passes.bind(
        device,
        &parallel_buffer,
        &x_buffer,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &output_view,
    );
    passes.draw(
        &mut encoder,
        true,
        &x_bind,
        &blur_x_view,
        None,
        0,
        ops,
        viewport,
    );
    x.target.w = 1;
    let y_buffer = upload(device, x, BufferUsages::UNIFORM);
    let y_bind = passes.bind(
        device,
        &parallel_buffer,
        &y_buffer,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &blur_x_view,
    );
    passes.draw(
        &mut encoder,
        true,
        &y_bind,
        &final_view,
        None,
        0,
        ops,
        viewport,
    );
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("screen shadow fixture readback"),
        size: 4096,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for (offset, texture) in [(0, &output), (1024, &final_output)] {
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
    }
    // Original no-atlas-shadow branch still multiplies cloud extinction;
    // neither a white fallback nor a normal-buffer approximation is valid.
    let mut cloud_parameters = p.clone();
    cloud_parameters.options = UVec4::new(0, 0, 1, 0);
    let cloud_buffer = upload(device, cloud_parameters, BufferUsages::UNIFORM);
    let cloud_bind = passes.bind(
        device,
        &parallel_buffer,
        &cloud_buffer,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &blur_x_view,
    );
    passes.draw(
        &mut encoder,
        false,
        &cloud_bind,
        &output_view,
        None,
        0,
        ops,
        viewport,
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 2048,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
        },
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    let mut jitter_parameters = p.clone();
    jitter_parameters.options.x = 1;
    let jitter_buffer = upload(device, jitter_parameters, BufferUsages::UNIFORM);
    let jitter_bind = passes.bind(
        device,
        &parallel_buffer,
        &jitter_buffer,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &blur_x_view,
    );
    passes.draw(
        &mut encoder,
        false,
        &jitter_bind,
        &output_view,
        None,
        0,
        ops,
        viewport,
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 3072,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
        },
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        send.send(r).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let w = (-0.5f32).exp() + 0.001;
    let expected = [
        0.0,
        w / (1.001 + 2.0 * w),
        (1.001 + w) / (1.001 + 2.0 * w),
        1.0,
    ];
    // Native D3D11 SampleShadowMap at the exact synthetic B8 sample positions:
    // comparison-filter weights are hardware-quantized, not ideal CPU lerps.
    let jitter_expected: Vec<f32> = include_str!("../analysis/native_screen_jitter_pcf_cases.txt")
        .lines()
        .filter_map(|line| line.strip_prefix("CASE 0 0 "))
        .map(|line| {
            f32::from_bits(
                u32::from_str_radix(line.split_ascii_whitespace().nth(3).unwrap(), 16).unwrap(),
            )
        })
        .collect();
    assert_eq!(jitter_expected.len(), 16);
    for y in 0..4 {
        for x in 0..4 {
            let scalar = |offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            assert!((scalar(y * 256 + x * 16 + 4) - if x >= 2 { 1.0 } else { 0.0 }).abs() < 1e-6);
            assert!((scalar(1024 + y * 256 + x * 16 + 4) - expected[x]).abs() < 1e-6);
            let cloud_expected = (-128.0f32 / 255.0).exp();
            assert!((scalar(2048 + y * 256 + x * 16 + 4) - cloud_expected).abs() < 1e-6);
            let actual = scalar(3072 + y * 256 + x * 16 + 4);
            assert_eq!(
                actual.to_bits(),
                jitter_expected[y * 4 + x].to_bits(),
                "original D3D11 B8/jitter pixel {x},{y}"
            );
        }
    }
    drop(bytes);
    readback.unmap();

    // Actual callback uses PostFX_DWAOBlur, not the other exponential bilateral
    // declaration. Preserve AO+shadow and packed depth; reject discontinuities.
    let packed = make_texture(
        "packed AO shadow and depth fixture",
        TextureFormat::Rgba32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        4,
        4,
    );
    let mut pixels = Vec::new();
    for _ in 0..4 {
        for x in 0..4 {
            let z = if x < 2 { 4.0f32 } else { 10.0 };
            for v in [
                x as f32 / 3.0,
                if x >= 2 { 1.0 } else { 0.0 },
                z / 255.0,
                z / 255.0,
            ] {
                pixels.extend(v.to_le_bytes());
            }
        }
    }
    queue.write_texture(
        packed.as_image_copy(),
        &pixels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(64),
            rows_per_image: Some(4),
        },
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    let packed_view = packed.create_view(&TextureViewDescriptor::default());
    let common = passes.bind(
        device,
        &parallel_buffer,
        &uniform,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &packed_view,
    );
    let blur_parameters =
        NativeDwaoBlurUniform::from_owner([4.0, 4.0], [1.0, 0.0], 1.0, 0.1, 0.251379132).unwrap();
    let blur_buffer = upload(device, blur_parameters, BufferUsages::UNIFORM);
    let dwao = NativeDwaoBlurPipeline::new(
        device,
        &passes,
        ColorTargetState {
            format: TextureFormat::Rgba32Float,
            blend: None,
            write_mask: ColorWrites::ALL,
        },
        DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(CompareFunction::NotEqual),
            stencil: default(),
            bias: default(),
        },
    );
    let dwao_bind = dwao.bind(device, &blur_buffer);
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    for (offset, scene_depth) in [(0, 0.5), (1024, 0.0)] {
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("original DWAO NOTEQUAL depth fixture"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(scene_depth),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        dwao.draw(
            &mut encoder,
            &common,
            &dwao_bind,
            &final_view,
            &depth_view,
            ops,
            viewport,
        );
        encoder.copy_texture_to_buffer(
            final_output.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        send.send(r).unwrap();
    });
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let weights = [0.152469158f32, 0.221841291, 0.221841291, 0.152469158];
    for y in 0..4 {
        for x in 0..4 {
            let center_depth: f32 = if x < 2 { 4.0 } else { 10.0 };
            let mut ao = x as f32 / 3.0 * 0.251379132;
            for (delta, weight) in [-2i32, -1, 1, 2].into_iter().zip(weights) {
                let sample = (x as i32 + delta).clamp(0, 3) as usize;
                let sample_depth = if sample < 2 { 4.0 } else { 10.0 };
                let selected = if (center_depth - sample_depth).abs() > 0.1 * center_depth {
                    x
                } else {
                    sample
                };
                ao += selected as f32 / 3.0 * weight;
            }
            let expected = [
                ao,
                if x < 2 { 0.0 } else { 1.0 },
                center_depth / 255.0,
                center_depth / 255.0,
            ];
            for c in 0..4 {
                let offset = y * 256 + x * 16 + c * 4;
                let actual = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                assert!(
                    (actual - expected[c]).abs() < 1e-6,
                    "DWAO source pixel {x},{y} channel {c}: {actual} != {}",
                    expected[c]
                );
                assert_eq!(
                    f32::from_le_bytes(bytes[1024 + offset..1028 + offset].try_into().unwrap()),
                    if c == 3 { 1.0 } else { 0.0 },
                    "original NOTEQUAL far-plane depth must retain the caller's clear colour"
                );
            }
        }
    }
    drop(bytes);
    readback.unmap();

    // Execute BOTH original stencil partitions and prove that AO and the two
    // packed-depth channels survive. A depth-only camera buffer is not enough.
    let partition = make_texture(
        "native main D32S8 screen partition",
        TextureFormat::Depth32FloatStencil8,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        4,
        4,
    );
    let partition_view = partition.create_view(&TextureViewDescriptor::default());
    use crate::native_shadow_gpu::{
        NativeOpaqueShadowDraw, NativeOpaqueShadowPipeline, NativeShadowScissor,
    };
    let position = [VertexAttribute {
        format: VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    }];
    let main_layout = RawVertexBufferLayout {
        array_stride: 12,
        step_mode: VertexStepMode::Vertex,
        attributes: &position,
    };
    let mut source_state = [0; 9];
    crate::render_state::reset_native_render_state(&mut source_state);
    let (world_state, world_reference) =
        crate::render_state::main_depth_surface_state(source_state, 0.0);
    let (model_state, model_reference) =
        crate::render_state::main_depth_surface_state(source_state, 0.5);
    let adapter = world.resource::<bevy::render::renderer::RenderAdapter>();
    let world_depth = NativeOpaqueShadowPipeline::new_main_opaque_depth(
        device,
        adapter,
        &main_layout,
        &world_state,
        world_reference.into(),
        FrontFace::Cw,
        None,
    )
    .unwrap();
    let model_depth = NativeOpaqueShadowPipeline::new_main_opaque_depth(
        device,
        adapter,
        &main_layout,
        &model_state,
        model_reference.into(),
        FrontFace::Cw,
        None,
    )
    .unwrap();
    // The stencil marker and shader multiplier have different native owners.
    // Supply the actual fixture CB, rather than deriving one from the marker.
    let model_cb = upload(
        device,
        [Vec4::W, Vec4::Z, Vec4::Y, Vec4::X, Vec4::X],
        BufferUsages::UNIFORM,
    );
    let world_binding = world_depth
        .bind_prepared_model_state(device, &model_cb)
        .unwrap();
    let model_binding = model_depth
        .bind_prepared_model_state(device, &model_cb)
        .unwrap();
    let triangle = [[-1.0f32, 1.0, 0.5], [3.0, 1.0, 0.5], [-1.0, -3.0, 0.5]];
    let vertex_bytes: Vec<u8> = triangle
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let model_vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("native main-Z rigid fixture vertices"),
        contents: &vertex_bytes,
        usage: BufferUsages::VERTEX,
    });
    let index_bytes: Vec<u8> = [0u16, 1, 2]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    let model_indices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("native main-Z rigid fixture indices"),
        contents: &index_bytes,
        usage: BufferUsages::INDEX,
    });
    let draw = |binding, x| NativeOpaqueShadowDraw {
        model_state: binding,
        program_resources: None,
        vertex_buffer: &model_vertices,
        index_buffer: &model_indices,
        index_format: IndexFormat::Uint16,
        indices: 0..3,
        base_vertex: 0,
        instances: 0..1,
        scissor: NativeShadowScissor {
            x,
            y: 0,
            width: 2,
            height: 4,
        },
    };
    let [first_state, second_state] = screen_producer_states(
        [0; 9],
        TextureFormat::Rgba32Float,
        TextureFormat::Depth32FloatStencil8,
    )
    .unwrap();
    let target = ColorTargetState {
        format: TextureFormat::Rgba32Float,
        blend: None,
        write_mask: ColorWrites::ALL,
    };
    let first = NativeScreenShadowPipelines::new(
        device,
        first_state.color,
        Some(first_state.depth),
        target.clone(),
    );
    let second = NativeScreenShadowPipelines::new(
        device,
        second_state.color,
        Some(second_state.depth),
        target,
    );
    let mut unshadowed = parameters();
    unshadowed.options.y = 0;
    let first_parameters = upload(device, unshadowed, BufferUsages::UNIFORM);
    let first_bind = first.bind(
        device,
        &parallel_buffer,
        &first_parameters,
        &depth_view,
        &point,
        &atlas.sampled,
        &comparison,
        &cloud_view,
        &cloud_sampler,
        &blur_x_view,
    );
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("native screen two-pass stencil fixture"),
    });
    {
        // The preceding DWAO sky case deliberately left sampled depth at zero.
        let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("restore this fixture\'s decompressed-depth input"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.5),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    world_depth
        .draw_depth_view(
            &mut encoder,
            &partition_view,
            [4, 4],
            // Native main-Z default is GreaterEqual (reversed depth), unlike
            // the Less/clear-one shadow atlas. Preserve its actual clear value.
            Operations {
                load: LoadOp::Clear(0.0),
                store: StoreOp::Store,
            },
            Some(Operations {
                load: LoadOp::Clear(0),
                store: StoreOp::Store,
            }),
            &[draw(&world_binding, 0)],
        )
        .unwrap();
    model_depth
        .draw_depth_view(
            &mut encoder,
            &partition_view,
            [4, 4],
            Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            Some(Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            }),
            &[draw(&model_binding, 2)],
        )
        .unwrap();
    let seed = Operations {
        load: LoadOp::Clear(LinearRgba::new(0.375, 0.625, 0.5, 0.75)),
        store: StoreOp::Store,
    };
    first.draw(
        &mut encoder,
        false,
        &first_bind,
        &output_view,
        Some(&partition_view),
        1,
        seed,
        viewport,
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
        },
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    second.draw(
        &mut encoder,
        false,
        &bind,
        &output_view,
        Some(&partition_view),
        1,
        Operations {
            load: LoadOp::Load,
            store: StoreOp::Store,
        },
        viewport,
    );
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 1024,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
        },
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        send.send(r).unwrap();
    });
    device
        .wgpu_device()
        .poll(bevy::render::render_resource::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    for stage in 0..2 {
        for y in 0..4 {
            for x in 0..4 {
                let green = if x >= 2 {
                    1.0
                } else if stage == 0 {
                    0.625
                } else {
                    0.0
                };
                for (c, expected) in [0.375f32, green, 0.5, 0.75].into_iter().enumerate() {
                    let offset = stage * 1024 + y * 256 + x * 16 + c * 4;
                    let actual = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "screen stencil stage {stage} pixel {x},{y} channel {c}"
                    );
                }
            }
        }
    }
    drop(bytes);
    readback.unmap();

    // The original unhackDepthBuffer draw changes only stencil-one depth.
    // Verify actual D32S8 depth bits and both stencil partitions, not D24
    // quantization or a CPU approximation of the shader.
    let native_tlf = world.resource::<crate::tiled_lighting_gpu::NativeTlfPipelines>();
    let attributes = [
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
    ];
    let layout = RawVertexBufferLayout {
        array_stride: 20,
        step_mode: VertexStepMode::Vertex,
        attributes: &attributes,
    };
    assert!(native_tlf
        .unhack_depth_pipeline(
            device,
            TextureFormat::Depth32Float,
            std::slice::from_ref(&layout)
        )
        .is_err());
    let unhack = native_tlf
        .unhack_depth_pipeline(
            device,
            TextureFormat::Depth32FloatStencil8,
            std::slice::from_ref(&layout),
        )
        .unwrap();
    let triangle = [
        [-1.0f32, 1.0, 0.0, 0.0, 0.0],
        [3.0, 1.0, 0.0, 2.0, 0.0],
        [-1.0, -3.0, 0.0, 0.0, 2.0],
    ];
    let vertices: Vec<u8> = triangle
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let triangle_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("unhack depth fixture vertices"),
        contents: &vertices,
        usage: BufferUsages::VERTEX,
    });
    let factor = upload(device, Vec4::new(0.5, 0.0, 0.0, 0.0), BufferUsages::UNIFORM);
    let unhack_bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: Some("native depth unhack inputs"),
            layout: &unhack.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&depth_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&point),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: factor.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("native D32S8 depth unhack fixture"),
    });
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("unhack only positive-model-depth-hack pixels"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &partition_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&unhack);
        pass.set_bind_group(0, &unhack_bind, &[]);
        pass.set_stencil_reference(1);
        pass.set_vertex_buffer(0, *triangle_buffer.slice(..));
        pass.draw(0..3, 0..1);
    }
    for (offset, aspect) in [
        (0, TextureAspect::DepthOnly),
        (1024, TextureAspect::StencilOnly),
    ] {
        let mut source = partition.as_image_copy();
        source.aspect = aspect;
        encoder.copy_texture_to_buffer(
            source,
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        send.send(r).unwrap();
    });
    device
        .wgpu_device()
        .poll(bevy::render::render_resource::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    for y in 0..4 {
        for x in 0..4 {
            let offset = y * 256 + x * 4;
            let actual = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let expected: f32 = if x >= 2 { 0.25 } else { 0.5 };
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "native unhack depth pixel {x},{y}"
            );
            assert_eq!(
                bytes[1024 + y * 256 + x],
                u8::from(x >= 2),
                "unhack must preserve stencil {x},{y}"
            );
        }
    }
    drop(bytes);
    readback.unmap();

    // Original coarse-depth CPU history uses four persistent nonblocking slots.
    // Test the actual R32G32 copy/map bytes, wraparound and older-ready selection.
    let coarse = make_texture(
        "owned original coarseDepth fixture",
        TextureFormat::Rg32Float,
        TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
        64,
        64,
    );
    let mut history = crate::native_coarse_depth::NativeCoarseDepthReadback::new(
        device,
        crate::native_coarse_depth::INITIAL_COARSE_DEPTH_OWNER_INDEX,
    )
    .unwrap();
    assert_eq!(history.try_read(|_| 0u32).unwrap(), None);
    assert!(crate::native_coarse_depth::NativeCoarseDepthReadback::new(device, 5).is_err());
    for generation in 1..=9u32 {
        let raw: Vec<u8> = (0..4096)
            .flat_map(|pixel| {
                [generation as f32, pixel as f32 + 0.25]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
            })
            .collect();
        queue.write_texture(
            coarse.as_image_copy(),
            &raw,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(512),
                rows_per_image: Some(64),
            },
            coarse.size(),
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("native coarseDepth persistent ring fixture"),
        });
        assert!(history.record_copy(&mut encoder, &coarse, false).unwrap());
        if generation > 1 {
            assert_eq!(
                history.try_read(|pixels| pixels[0][0].to_bits()).unwrap(),
                Some(((generation - 1) as f32).to_bits())
            );
        }
        queue.submit([encoder.finish()]);
        history.map_submitted().unwrap();
        // Only the fixture waits. Production request_maps/try_read never wait.
        device
            .wgpu_device()
            .poll(bevy::render::render_resource::PollType::wait_indefinitely())
            .unwrap();
        assert_eq!(
            history
                .try_read(|pixels| {
                    for (index, pixel) in pixels.iter().enumerate() {
                        assert_eq!(pixel[0].to_bits(), (generation as f32).to_bits());
                        assert_eq!(pixel[1].to_bits(), (index as f32 + 0.25).to_bits());
                    }
                    pixels[0][0].to_bits()
                })
                .unwrap(),
            Some((generation as f32).to_bits())
        );
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("native coarseDepth offscreen no-copy fixture"),
        });
        assert!(!history.record_copy(&mut encoder, &depth, true).unwrap());
        assert_eq!(
            history.try_read(|pixels| pixels[0][0].to_bits()).unwrap(),
            Some((generation as f32).to_bits())
        );
    }

    // Exercise the actual coarse producer feeding that same asynchronous ring,
    // including cropped extents, overlapping reductions and ignored Y/W channels.
    // These exactly representable values isolate the captured DXBC arithmetic;
    // this is a source-backed GPU fixture, not an original-executable parity claim.
    let mip_size = [117u32, 69];
    let source = make_texture(
        "last downsampled depth mip",
        TextureFormat::Rgba16Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        mip_size[0],
        mip_size[1],
    );
    let generated = make_texture(
        "native generated coarseDepth",
        TextureFormat::Rg32Float,
        TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
        64,
        64,
    );
    let palette = [
        (0.0f32, 0x0000u16),
        (0.125, 0x3000),
        (0.25, 0x3400),
        (0.5, 0x3800),
        (1.0, 0x3c00),
        (2.0, 0x4000),
    ];
    let input: Vec<[f32; 2]> = (0..mip_size[1])
        .flat_map(|y| {
            (0..mip_size[0]).map(move |x| {
                [
                    palette[((x + 2 * y) % 6) as usize].0,
                    palette[((2 * x + y + 1) % 6) as usize].0,
                ]
            })
        })
        .collect();
    let raw: Vec<u8> = (0..mip_size[1])
        .flat_map(|y| {
            (0..mip_size[0]).flat_map(move |x| {
                [
                    palette[((x + 2 * y) % 6) as usize].1,
                    0x4000,
                    palette[((2 * x + y + 1) % 6) as usize].1,
                    0x3800,
                ]
                .into_iter()
                .flat_map(u16::to_le_bytes)
            })
        })
        .collect();
    queue.write_texture(
        source.as_image_copy(),
        &raw,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(mip_size[0] * 8),
            rows_per_image: Some(mip_size[1]),
        },
        source.size(),
    );
    let mut inverse = Mat4::IDENTITY.to_cols_array_2d();
    inverse[3][2] = -1.0;
    inverse[3][3] = 0.0;
    inverse[2][3] = 2.0;
    let parameters = crate::native_coarse_depth::NativeCoarseDepthParameters::from_owner(
        mip_size,
        [0.85, 0.93],
        inverse,
    )
    .unwrap();
    assert_eq!(parameters.source_max, [98, 63]);
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("native coarseDepth constants"),
        contents: &parameters.to_bytes(),
        usage: BufferUsages::UNIFORM,
    });
    let source_view = source.create_view(&TextureViewDescriptor::default());
    let destination_view = generated.create_view(&TextureViewDescriptor::default());
    let binding = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: Some("native coarseDepth producer"),
            layout: &native_tlf.coarse_depth.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&source_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&destination_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("native coarseDepth generation and history"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("native coarseDepthGen"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&native_tlf.coarse_depth);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(64, 64, 1);
    }
    history
        .record_copy(&mut encoder, &generated, false)
        .unwrap();
    queue.submit([encoder.finish()]);
    history.map_submitted().unwrap();
    device
        .wgpu_device()
        .poll(bevy::render::render_resource::PollType::wait_indefinitely())
        .unwrap();
    assert!(history
        .try_read(|pixels| {
            let count = parameters.source_per_destination.map(|v| v.ceil() as u32);
            for dy in 0..64 {
                for dx in 0..64 {
                    let start = [dx, dy].map(|v| v as f32);
                    let start = std::array::from_fn::<_, 2, _>(|a| {
                        (start[a] * parameters.source_per_destination[a]).floor() as u32
                    });
                    let mut interval = [99999.0f32, 0.0f32];
                    for x in 0..count[0] {
                        for y in 0..count[1] {
                            let sx = (start[0] + x).min(parameters.source_max[0]);
                            let sy = (start[1] + y).min(parameters.source_max[1]);
                            let sample = input[(sy * mip_size[0] + sx) as usize];
                            interval = [sample[0].min(interval[0]), sample[1].max(interval[1])];
                        }
                    }
                    let expected = interval.map(|v| if v == 0.0 { 1.0 } else { (1.0 / v) / 2.0 });
                    assert_eq!(
                        pixels[(dy * 64 + dx) as usize].map(f32::to_bits),
                        expected.map(f32::to_bits),
                        "native coarseDepth pixel {dx},{dy}"
                    );
                }
            }
        })
        .unwrap()
        .is_some());

    // Original four-output reduction, including odd dimensions and edge loads.
    // Verify every RGBA16F mip against the installed shader's f32 shared-memory
    // arithmetic. This fixture is source-backed, not executable parity evidence.
    let input_size = [27u32, 19];
    let input_depth = make_texture(
        "odd-sized original depth input",
        TextureFormat::Depth32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        input_size[0],
        input_size[1],
    );
    let fill = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("depth reduction fixture source"),
        source: ShaderSource::Wgsl(
            r#"
        @vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            let p = array<vec2<f32>, 3>(vec2(-1,-1), vec2(3,-1), vec2(-1,3));
            return vec4(p[i], 0, 1);
        }
        @fragment fn fragment(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
            let z = array<f32, 3>(0.25, 0.5, 1.0);
            return z[(u32(p.x) + 2u * u32(p.y)) % 3u];
        }"#
            .into(),
        ),
    });
    let fill = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("depth reduction input fill"),
        layout: None,
        vertex: RawVertexState {
            module: &fill,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &fill,
            entry_point: Some("fragment"),
            compilation_options: default(),
            targets: &[],
        }),
        primitive: PrimitiveState {
            cull_mode: None,
            ..default()
        },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: default(),
            bias: default(),
        }),
        multisample: default(),
        multiview_mask: None,
        cache: None,
    });
    let pyramid =
        crate::native_coarse_depth::NativeDepthPyramid::new(device, native_tlf, &input_depth)
            .unwrap();
    assert_eq!(pyramid.last_mip_extent(), [2, 2]);
    assert!(
        crate::native_coarse_depth::NativeDepthPyramid::new(device, native_tlf, &source).is_err()
    );
    let mip_readback = device.create_buffer(&BufferDescriptor {
        label: Some("all four native depth mips"),
        size: 30 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("single-dispatch four-mip depth fixture"),
    });
    let source_view = input_depth.create_view(&TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("fixture actual depth pixels"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &source_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&fill);
        pass.draw(0..3, 0..1);
    }
    let mut inverse = Mat4::IDENTITY.to_cols_array_2d();
    inverse[2][3] = -1.0;
    inverse[3][2] = 1.0;
    inverse[3][3] = 0.0;
    assert!(pyramid
        .record(queue, &mut encoder, native_tlf, [28, 19], inverse)
        .is_err());
    let coverage = pyramid
        .record(queue, &mut encoder, native_tlf, input_size, inverse)
        .unwrap();
    assert_eq!(coverage.written_mips, [[16, 16], [8, 8], [4, 4], [2, 2]]);
    assert!(
        crate::native_coarse_depth::NativeCoarseDepthParameters::from_reduction(
            coverage,
            [1.0, 1.0],
            inverse
        )
        .is_ok()
    );
    let mut offset = 0;
    for mip in 0..4 {
        let size = 16u32 >> mip;
        let mut source = pyramid.texture().as_image_copy();
        source.mip_level = mip;
        encoder.copy_texture_to_buffer(
            source,
            TexelCopyBufferInfo {
                buffer: &mip_readback,
                layout: TexelCopyBufferLayout {
                    offset,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(size),
                },
            },
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        offset += u64::from(size) * 256;
    }
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    mip_readback
        .slice(..)
        .map_async(MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    device
        .wgpu_device()
        .poll(bevy::render::render_resource::PollType::wait_indefinitely())
        .unwrap();
    receive.recv().unwrap().unwrap();
    let bytes = mip_readback.slice(..).get_mapped_range();
    let mut offset = 0usize;
    let mut previous: Vec<[f32; 4]> = Vec::new();
    for mip in 0..4 {
        let size = 16usize >> mip;
        let mut values = Vec::with_capacity(size * size);
        for y in 0..size {
            for x in 0..size {
                let get = |sx: usize, sy: usize| -> [f32; 4] {
                    if mip == 0 {
                        let sx = sx.min(input_size[0] as usize - 1);
                        let sy = sy.min(input_size[1] as usize - 1);
                        let z = [4.0, 2.0, 1.0][(sx + 2 * sy) % 3];
                        [z, z, z, 0.0]
                    } else {
                        previous[sy * (size * 2) + sx]
                    }
                };
                let [a, b, c, d] = [
                    get(2 * x, 2 * y),
                    get(2 * x + 1, 2 * y),
                    get(2 * x, 2 * y + 1),
                    get(2 * x + 1, 2 * y + 1),
                ];
                let expected: [f32; 4] = [
                    0.25 * (((a[0] + b[0]) + c[0]) + d[0]),
                    a[1].min(b[1]).min(c[1]).min(d[1]),
                    a[2].max(b[2]).max(c[2]).max(d[2]),
                    0.0,
                ];
                for channel in 0..4 {
                    let at = offset + y * 256 + x * 8 + channel * 2;
                    let actual = u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
                    assert_eq!(
                        actual,
                        half::f16::from_f32(expected[channel]).to_bits(),
                        "native pyramid mip={mip} pixel={x},{y} channel={channel}"
                    );
                }
                // Never feed the half-quantized texture values back to the next level.
                values.push(expected);
            }
        }
        previous = values;
        offset += size * 256;
    }
    drop(bytes);
    mip_readback.unmap();
    // Reuse the same persistent image with a much smaller active viewport.
    // Old pixels can legitimately remain outside the written rectangle, but
    // the next consumer must not silently treat them as this frame's depth.
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("native cropped reduction coverage"),
    });
    let coverage = pyramid
        .record(queue, &mut encoder, native_tlf, [3, 3], inverse)
        .unwrap();
    assert_eq!(coverage.written_mips, [[8, 8], [4, 4], [2, 2], [1, 1]]);
    assert!(
        crate::native_coarse_depth::NativeCoarseDepthParameters::from_reduction(
            coverage,
            [1.0, 1.0],
            inverse
        )
        .is_err()
    );
    assert_eq!(
        crate::native_coarse_depth::NativeCoarseDepthParameters::from_reduction(
            coverage,
            [0.5, 0.5],
            inverse
        )
        .unwrap()
        .source_max,
        [0, 0]
    );
    queue.submit([encoder.finish()]);
}
