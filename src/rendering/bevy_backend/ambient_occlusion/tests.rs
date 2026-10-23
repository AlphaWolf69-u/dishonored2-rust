use super::*;
use bevy::{color::LinearRgba, render::RenderApp};

#[test]
#[ignore = "requires graphics adapter; compares the whole producer to the installed game SDK"]
fn producer_matches_game_sdk_pixels() {
    compare_producer_fixture(false);
}

#[test]
#[ignore = "requires graphics adapter; compares the producer to the installed DWAO shader"]
fn dwao_matches_game_shader_pixels() {
    compare_producer_fixture(true);
}

fn compare_producer_fixture(legacy: bool) {
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
    world.init_resource::<crate::tiled_lighting_gpu::TlfPipelines>();
    let pipelines = world.resource::<crate::tiled_lighting_gpu::TlfPipelines>();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:Some("AO test depth transfer"),source:ShaderSource::Wgsl(r#"
        @group(0) @binding(0) var source:texture_2d<f32>;
        @vertex fn v(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
            let uv=vec2(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
        }
        @fragment fn f(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {return textureLoad(source,vec2<i32>(p.xy),0).r;}
    "#.into())});
    let layout = device.create_bind_group_layout(
        Some("AO test depth transfer"),
        &[BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
        }],
    );
    let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let transfer = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("AO test depth transfer"),
        layout: Some(&pipeline_layout),
        vertex: RawVertexState {
            module: &shader,
            entry_point: Some("v"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &shader,
            entry_point: Some("f"),
            compilation_options: default(),
            targets: &[],
        }),
        primitive: default(),
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
    let fixture = if legacy {
        include_str!("../../../../tests/fixtures/ambient_occlusion_dwao_pixels.txt")
    } else {
        include_str!("../../../../tests/fixtures/ambient_occlusion_pixels.txt")
    };
    let frames: Vec<_> = fixture.lines().collect();
    let mut maximum = 0;
    let mut compared = 0;
    for (frame, pair) in frames.chunks_exact(2).enumerate() {
        let words: Vec<_> = pair[0].split_whitespace().collect();
        let w = words[1].parse::<u32>().unwrap();
        let h = words[2].parse::<u32>().unwrap();
        let pattern = words[3].parse::<u32>().unwrap();
        let f = |i| f32::from_bits(u32::from_str_radix(words[i], 16).unwrap());
        let scale = f(4);
        let parameters = if legacy {
            crate::rendering::ambient_occlusion::AmbientOcclusionParameters::Dwao {
                extent: [w, h],
                render_scale: [scale; 2],
                inverse_projection: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 0.0, -1.0],
                    [0.0, 0.0, 99.99986, 0.00014285714],
                ],
            }
        } else {
            let mut projection = [0.; 16];
            projection[0] = f(5);
            projection[5] = f(6);
            projection[10] = 1.4285733413998969e-6;
            projection[11] = -1.;
            projection[14] = 0.010000013746321201;
            crate::rendering::ambient_occlusion::AmbientOcclusionParameters::Hbao(
                HbaoParameters::new([w, h], projection, [scale; 2]).unwrap(),
            )
        };
        let make = |format, usage| {
            device.create_texture(&TextureDescriptor {
                label: Some("AO fixture"),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let source = make(
            TextureFormat::R32Float,
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        );
        let depth = make(
            TextureFormat::Depth32Float,
            TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        );
        let output = make(
            TextureFormat::Rgba8Unorm,
            TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT,
        );
        let source_view = source.create_view(&default());
        let depth_view = depth.create_view(&default());
        let output_view = output.create_view(&default());
        let mut raw = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let z = match pattern {
                    1 => 2.,
                    2 => 1. + 0.03 * x as f32 + 0.015 * y as f32 + f32::from(u8::from(y > h / 2)),
                    3 => 8. + 0.1 * x as f32 + 2. * ((x / 3 + y / 3) % 2) as f32,
                    _ => 2. + 0.005 * x as f32 + f32::from(u8::from(x > w / 2)),
                };
                raw.extend(((1. / z - 0.00014285713) / 99.999855f32).to_le_bytes());
            }
        }
        queue.write_texture(
            source.as_image_copy(),
            &raw,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            source.size(),
        );
        let inputs = device.create_bind_group(
            Some("AO fixture depth"),
            &layout,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&source_view),
            }],
        );
        let mut producer =
            AmbientOcclusionPass::new(device, pipelines, &depth, &parameters).unwrap();
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("AO fixture depth fill"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            pass.set_pipeline(&transfer);
            pass.set_bind_group(0, &inputs, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("AO fixture channel seed"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::new(0., 0.25, 0.5, 0.75).into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
        }
        producer
            .record(
                device,
                queue,
                &mut encoder,
                pipelines,
                &depth_view,
                &output_view,
                &parameters,
            )
            .unwrap();
        let row = (w * 4).next_multiple_of(256);
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("AO fixture readback"),
            size: u64::from(row * h),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            output.size(),
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging.slice(..).map_async(MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for pixel in 0..w * h {
            let at = (pixel / w * row + pixel % w * 4) as usize;
            let expected =
                u8::from_str_radix(&pair[1][pixel as usize * 2..pixel as usize * 2 + 2], 16)
                    .unwrap();
            let steps = bytes[at].abs_diff(expected);
            maximum = maximum.max(steps);
            compared += 1;
            assert!(
                steps <= 1,
                "frame{frame} pixel{pixel} actual={} game={expected} steps={steps}",
                bytes[at]
            );
            assert_eq!(
                &bytes[at + 1..at + 4],
                &[64, 128, 191],
                "AO must preserve shared shadow and depth channels"
            );
        }
    }
    println!("{compared} game SDK AO pixels, maximum UNORM8 difference {maximum}");
    assert_eq!(frames.len() / 2, 32);
}
