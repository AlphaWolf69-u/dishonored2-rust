use super::*;
use bevy::render::RenderApp;

#[test]
#[ignore = "requires GPU; tests current AO/depth filtering without any sunlight atlas"]
fn filters_ao_without_a_directional_atlas() {
    compare_filter_depth(false);
}

#[test]
#[ignore = "requires GPU; tests the HBAO callback's registered depth initializer"]
fn hbao_depth_transport_uses_registered_image() {
    compare_filter_depth(true);
}

fn compare_filter_depth(use_default: bool) {
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
    let [w, h] = if use_default {
        [32u32, 24u32]
    } else {
        [4u32, 4u32]
    };
    let make = |format, usage| {
        device.create_texture(&TextureDescriptor {
            label: Some("independent screen visibility fixture"),
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
    let depth = make(
        TextureFormat::Depth32Float,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
    );
    let depth_view = depth.create_view(&default());
    let output = make(
        TextureFormat::Rgba8Unorm,
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let output_view = output.create_view(&default());
    let default_image = use_default.then(|| {
        let image = device.create_texture(&TextureDescriptor {
            label: Some("registered depth initializer"),
            size: Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            image.as_image_copy(),
            &crate::material::intrinsic_image_pixels("_default").unwrap(),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(16),
            },
            image.size(),
        );
        image
    });
    let default_view = default_image
        .as_ref()
        .map(|image| image.create_view(&default()));
    let filter = ScreenVisibilityFilter::new(
        device,
        &depth,
        &output_view,
        [w, h],
        [0; 9],
        default_view.as_ref(),
    )
    .unwrap();
    let seed=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:Some("synthetic AO channels"),source:ShaderSource::Wgsl(r#"
        @vertex fn v(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
            let p=array<vec2<f32>,3>(vec2(-1,-1),vec2(3,-1),vec2(-1,3));return vec4(p[i],0,1);
        }
        @fragment fn f(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {return vec4(SEED_VALUE);}
    "#.replace("SEED_VALUE",if use_default {"0.25"} else {"floor(p.x)/3.0"}).into())});
    let seed_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("AO channel seed"),
        layout: None,
        vertex: RawVertexState {
            module: &seed,
            entry_point: Some("v"),
            buffers: &[],
            compilation_options: default(),
        },
        fragment: Some(RawFragmentState {
            module: &seed,
            entry_point: Some("f"),
            compilation_options: default(),
            targets: &[Some(ColorTargetState {
                format: TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: ColorWrites::RED,
            })],
        }),
        primitive: default(),
        depth_stencil: None,
        multisample: default(),
        multiview_mask: None,
        cache: None,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("independent visibility readback"),
        size: u64::from(256 * h),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    {
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("view depth seed"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.5),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..default()
        });
    }
    let projection = if use_default {
        Vec4::new(1.0, 99.999855, 0.00014285713, 0.0)
    } else {
        Vec4::new(1.0, 1.0, 0.0, 0.0)
    };
    filter.pack(queue, &mut encoder, &output_view, projection, [1.0, 1.0]);
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("AO without sunlight"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&seed_pipeline);
        pass.draw(0..3, 0..1);
    }
    filter.filter(&mut encoder, &output_view);
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(h),
            },
        },
        output.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    // Installed five-tap filtering, RGBA16F horizontal and RGBA8 vertical stores.
    let expected = [45u8, 98, 157, 210];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let pixel = &bytes[y * 256 + x * 4..y * 256 + x * 4 + 4];
            if use_default {
                // Actual original NoAO draw: outside the 16x16 registered image,
                // raw depth zero packs the finite reverse-Z far plane to 255/88.
                let depth = if x >= 16 || y >= 16 {
                    [255, 88]
                } else if x == 0 || x == 15 || y == 0 || y == 15 {
                    [0, 3]
                } else {
                    [0, 20]
                };
                assert_eq!(&pixel[2..], &depth, "registered depth transport at {x},{y}");
                assert_eq!(
                    &pixel[..2],
                    &[64, 255],
                    "AO and neutral sunlight at {x},{y}"
                );
            } else {
                assert!(pixel[0].abs_diff(expected[x]) <= 1, "{x},{y}: {pixel:?}");
                assert_eq!(
                    &pixel[1..],
                    &[255, 2, 2],
                    "neutral sunlight and packed depth must survive AO filtering"
                );
            }
        }
    }
}
