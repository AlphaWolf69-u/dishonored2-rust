//! Numeric coverage of 2feef0's opaque-depth restoration, including stencil
//! combinations 0/4/8/12 and unrelated upper bits. No window or image readback.
use super::tiled_lighting_gpu::TlfPipelines;
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub(super) fn check_restoration(
    device: &RenderDevice,
    queue: &RenderQueue,
    pipelines: &TlfPipelines,
) {
    let target = |format| {
        device.create_texture(&TextureDescriptor {
            label: Some("boat restore fixture"),
            size: Extent3d {
                width: 8,
                height: 8,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let opaque = target(TextureFormat::Depth32Float);
    let mut water_depth = super::water_depth::WaterDepthPass::new(
        device,
        pipelines,
        [8, 8],
        TextureFormat::Depth24PlusStencil8,
    )
    .unwrap();
    let water = water_depth.texture.clone();
    let opaque_view = opaque.create_view(&default());
    let water_view = water_depth.attachment.clone();
    let sample = water.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    });
    let seed = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("boat stencil fixture"),
        source: ShaderSource::Wgsl(
            r#"
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*2.0-1.0,0.75,1.0);
}
"#
            .into(),
        ),
    });
    let face = StencilFaceState {
        compare: CompareFunction::Always,
        fail_op: StencilOperation::Keep,
        depth_fail_op: StencilOperation::Keep,
        pass_op: StencilOperation::Replace,
    };
    let seed = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("boat stencil fixture"),
        layout: None,
        vertex: RawVertexState {
            module: &seed,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: None,
        primitive: default(),
        depth_stencil: Some(DepthStencilState {
            format: water.format(),
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: StencilState {
                front: face,
                back: face,
                read_mask: 255,
                write_mask: 255,
            },
            bias: default(),
        }),
        multisample: default(),
        multiview_mask: None,
        cache: None,
    });
    let read = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("boat restored-depth numeric reader"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var depth:texture_depth_2d;
@group(0) @binding(1) var<storage,read_write> output:array<f32>;
@compute @workgroup_size(1) fn read(@builtin(global_invocation_id) i:vec3<u32>) {
    output[i.y*8u+i.x]=textureLoad(depth,vec2<i32>(i.xy),0);
}
"#
            .into(),
        ),
    });
    let read = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &read,
        entry_point: Some("read"),
        compilation_options: default(),
        cache: None,
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read_input = device.create_bind_group(
        None,
        &read.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&sample),
            },
            BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    );
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 2560,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    for scenario in 0..3 {
        let mut encoder = device.create_command_encoder(&default());
        if scenario == 0 {
            assert!(water_depth.restore_boat_depth(&mut encoder).is_err());
        }
        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &opaque_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.25),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
        }
        if scenario == 0 {
            water_depth
                .prepare(device, &mut encoder, &opaque, false)
                .unwrap();
        }
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &water_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.75),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(0),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
            pass.set_pipeline(&seed);
            for stripe in 0..4 {
                pass.set_viewport((stripe * 2) as f32, 0., 2., 8., 0., 1.);
                pass.set_stencil_reference(0xa0 | (stripe * 4));
                pass.draw(0..3, 0..1);
            }
        }
        if scenario == 0 {
            water_depth.restore_boat_depth(&mut encoder).unwrap();
        } else {
            water_depth
                .prepare(device, &mut encoder, &opaque, scenario == 2)
                .unwrap();
        }
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&read);
            pass.set_bind_group(0, &read_input, &[]);
            pass.dispatch_workgroups(8, 8, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 256);
        let mut source = water.as_image_copy();
        source.aspect = TextureAspect::StencilOnly;
        encoder.copy_texture_to_buffer(
            source,
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 512,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(8),
                },
            },
            water.size(),
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        for y in 0..8 {
            for x in 0..8 {
                let offset = (y * 8 + x) * 4;
                let value = f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
                let expected = if scenario != 0 || x / 2 == 2 {
                    0.25
                } else {
                    0.75
                };
                assert!(
                    (value - expected).abs() < 0.000001,
                    "water depth scenario{scenario} {x}/{y}: {value} != {expected}"
                );
                assert_eq!(
                    bytes[512 + y * 256 + x],
                    if scenario == 1 {
                        0
                    } else {
                        0xa0 | ((x / 2) * 4) as u8
                    },
                    "boat restore stencil {x}/{y}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
    }
}
