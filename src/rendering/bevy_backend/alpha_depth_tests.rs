//! Real copy/min-gather submission, format conversion, stencil preservation,
//! fixed half-size footprints and persistent target/binding ownership.
use super::*;
use bevy::{math::UVec2, render::renderer::RenderQueue};

pub(in crate::rendering::bevy_backend) fn check_preparation(
    device: &RenderDevice,
    queue: &RenderQueue,
    pipelines: &TlfPipelines,
) {
    let format = TextureFormat::Depth24PlusStencil8;
    let extent = FogExtents::new(UVec2::splat(32), 1).unwrap();
    assert!(AlphaDepthPass::new(device, pipelines, extent, TextureFormat::Depth32Float).is_err());
    let mut owner = AlphaDepthPass::new(device, pipelines, extent, format).unwrap();
    let water = device.create_texture(&TextureDescriptor {
        label: Some("completed water-depth fixture"),
        size: Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let water_view = water.create_view(&default());
    let seed = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("alpha depth sloping source"),
        source: ShaderSource::Wgsl(
            r#"
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    return f32(u32(p.y)*32u+u32(p.x)+1u)/2048.0;
}
"#
            .into(),
        ),
    });
    let write = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: RawVertexState {
            module: &seed,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &seed,
            entry_point: Some("fragment"),
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
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("alpha depth numeric reader"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var full:texture_depth_2d;
@group(0) @binding(1) var reduced:texture_depth_2d;
@group(0) @binding(2) var<storage,read_write> output:array<vec2<f32>>;
@compute @workgroup_size(1) fn read(@builtin(global_invocation_id) i:vec3<u32>) {
    let p=vec2<i32>(i.xy);var small=-1.0;
    if all(i.xy<textureDimensions(reduced)) {small=textureLoad(reduced,p,0);}
    output[i.y*32u+i.x]=vec2(textureLoad(full,p,0),small);
}
"#
            .into(),
        ),
    });
    let read = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("read"),
        compilation_options: default(),
        cache: None,
    });
    let output = device.create_buffer(&BufferDescriptor {
        label: Some("alpha depth numeric output"),
        size: 8192,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    // 8192 depth bytes, 32 padded full-stencil rows and 16 reduced rows.
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("alpha depth numeric readback"),
        size: 20480,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let fixed = [owner.copy.id(), owner.downscale.id()];
    let vertices = owner.vertices.id();
    let mut source_bind = None;
    for (quality, flags) in [(1, 0), (1, 8), (2, 0), (2, 8)] {
        let extent = FogExtents::new(UVec2::splat(32), quality).unwrap();
        let previous = [owner.full.id(), owner.reduced.id()];
        owner.resize(device, extent);
        assert_eq!(owner.full.id(), previous[0]);
        assert_eq!(owner.reduced.id(), previous[1]);
        assert_eq!([owner.reduced.width(), owner.reduced.height()], [16, 16]);
        assert_eq!([owner.copy.id(), owner.downscale.id()], fixed);
        assert_eq!(owner.vertices.id(), vertices);
        let bind = device.create_bind_group(
            None,
            &read.get_bind_group_layout(0).into(),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&owner.full_sample),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&owner.reduced_sample),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        );
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &water_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            pass.set_pipeline(&write);
            pass.draw(0..3, 0..1);
        }
        for (view, sentinel) in [
            (&owner.full_attachment, 0x44),
            (&owner.reduced_attachment, 0xa5),
        ] {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.9),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(sentinel),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        owner.record(device, &mut encoder, &water, flags).unwrap();
        let id = owner.source.as_ref().unwrap().1.id();
        if let Some(prior) = source_bind {
            assert_eq!(id, prior, "stable source replaced its binding");
        }
        source_bind = Some(id);
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&read);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(32, 32, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 8192);
        for (texture, offset) in [(&owner.full, 8192), (&owner.reduced, 16384)] {
            let mut source = texture.as_image_copy();
            source.aspect = TextureAspect::StencilOnly;
            encoder.copy_texture_to_buffer(
                source,
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(texture.height()),
                    },
                },
                texture.size(),
            );
        }
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
        let float =
            |offset: usize| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        for y in 0..32usize {
            for x in 0..32usize {
                let expected = (y * 32 + x + 1) as f32 / 2048.;
                assert!(
                    (float((y * 32 + x) * 8) - expected).abs() < 0.000001,
                    "full alpha-depth copy at {x}/{y}"
                );
                assert_eq!(
                    bytes[8192 + y * 256 + x],
                    if flags & 8 == 0 { 0 } else { 0x44 },
                    "full stencil policy"
                );
                let size = 16usize; // 2fbc60's literal division, at either fog quality.
                if x < size && y < size {
                    let low = |p: usize| {
                        (((p as f32 + 0.5) * 32. / size as f32 - 0.5).floor() as usize).min(31)
                    };
                    let expected = (low(y) * 32 + low(x) + 1) as f32 / 2048.;
                    assert!(
                        (float((y * 32 + x) * 8 + 4) - expected).abs() < 0.000001,
                        "quality{quality} min gather {x}/{y}: {} != {expected}",
                        float((y * 32 + x) * 8 + 4)
                    );
                    assert_eq!(
                        bytes[16384 + y * 256 + x],
                        0xa5,
                        "reduction overwrote stencil"
                    );
                }
            }
        }
        drop(bytes);
        staging.unmap();
    }
    let mut encoder = device.create_command_encoder(&default());
    assert!(owner.record(device, &mut encoder, &water, 1).is_err());
    assert!(owner
        .record(device, &mut encoder, &owner.full.clone(), 0)
        .is_err());
}
