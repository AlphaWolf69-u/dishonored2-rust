use super::sky_mask::{SkyMaskPipeline, SkySurface, SkyView};
use bevy::{
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
    },
};

pub(super) fn check_mask_accumulation(device: &RenderDevice, queue: &RenderQueue) {
    let vertex = VertexBufferLayout {
        array_stride: 48,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,
            },
            VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: 24,
                shader_location: 2,
            },
            VertexAttribute {
                format: VertexFormat::Float32x4,
                offset: 32,
                shader_location: 3,
            },
        ],
    };
    let mut state = [0; 9];
    crate::render_state::reset_render_state(&mut state);
    state[5] &= !0x03000000; // synthetic fixture uses uncullable screen triangle
    let pipeline = SkyMaskPipeline::new(
        device,
        TextureFormat::Rgba16Float,
        TextureFormat::Depth32Float,
        &vertex,
        state,
        false,
    )
    .unwrap();
    let upload = |value: ShaderBuffer| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &value.data.unwrap(),
            usage: BufferUsages::UNIFORM,
        })
    };
    let globals = upload(
        SkyView {
            clip_from_world: Mat4::IDENTITY,
            time: 0.,
            exposure: 1.,
            padding: Vec2::ZERO,
        }
        .into(),
    );
    let mut sky = [Vec4::ZERO; 7];
    sky[0].x = 1.;
    sky[1] = Vec4::new(32., 64., 1., 0.);
    sky[2] = Vec4::new(1., 0., 0.5, 1.);
    sky[3] = Vec4::new(1., 0., 0., 1.);
    let make_surface = |cloud| {
        upload(
            SkySurface {
                sky,
                world_from_local: Mat4::IDENTITY,
                fog: Vec4::new(2., 0.8, 0., 0.),
                depth_uv: Vec4::new(1., 1., 0., 0.),
                flags: UVec4::new(u32::from(cloud), 1, 0, 0),
            }
            .into(),
        )
    };
    let surfaces = [make_surface(false), make_surface(true)];
    let groups = surfaces.each_ref().map(|surface| {
        device.create_bind_group(
            None,
            &pipeline.view_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: surface.as_entire_binding(),
                },
            ],
        )
    });
    let make = |format, usage| {
        device.create_texture(&TextureDescriptor {
            label: Some("sky mask MRT fixture"),
            size: Extent3d {
                width: 4,
                height: 4,
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
    let hdr = make(TextureFormat::Rgba16Float, TextureUsages::RENDER_ATTACHMENT);
    let mask = make(
        TextureFormat::R8Unorm,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let z = make(
        TextureFormat::Depth32Float,
        TextureUsages::RENDER_ATTACHMENT,
    );
    let hdr_view = hdr.create_view(&default());
    let mask_view = mask.create_view(&default());
    let z_view = z.create_view(&default());
    let image = |pixel: [u8; 4]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("sky mask source sample"),
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
            },
            TextureDataOrder::LayerMajor,
            &pixel,
        )
    };
    let diffuse = image([255; 4]);
    let depth = image([128; 4]);
    let diffuse_view = diffuse.create_view(&default());
    let depth_view = depth.create_view(&default());
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    let materials = device.create_bind_group(
        None,
        &pipeline.material_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&diffuse_view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&depth_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(&diffuse_view),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&diffuse_view),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::TextureView(&depth_view),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::Sampler(&sampler),
            },
        ],
    );
    let vertices = [[-1., -1., 0.], [3., -1., 0.], [-1., 3., 0.]]
        .into_iter()
        .flat_map(|p| {
            [
                p[0], p[1], p[2], 0., -0.5, 0.8660254, 0.5, 0.5, 1., 1., 1., 0.8,
            ]
        })
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &vertices,
        usage: BufferUsages::VERTEX,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 4 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let source_depth = 128. / 255.;
    let sky_mask = (1. - (1. - source_depth) * (1. - source_depth)) * 0.8;
    let cloud_mask = sky_mask * source_depth * 0.8;
    for (clouds, expected) in [(0, sky_mask), (1, sky_mask + cloud_mask), (2, 1.)] {
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut draw = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[
                    Some(RenderPassColorAttachment {
                        view: &hdr_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::NONE.into()),
                            store: StoreOp::Store,
                        },
                    }),
                    Some(RenderPassColorAttachment {
                        view: &mask_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::NONE.into()),
                            store: StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &z_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
            draw.set_pipeline(&pipeline.pipeline);
            draw.set_bind_group(1, &materials, &[]);
            draw.set_vertex_buffer(0, *vertices.slice(..));
            draw.set_bind_group(0, &groups[0], &[0]);
            draw.draw(0..3, 0..1);
            draw.set_bind_group(0, &groups[1], &[0]);
            for _ in 0..clouds {
                draw.draw(0..3, 0..1);
            }
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &mask,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(4),
                },
            },
            mask.size(),
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
        for y in 0..4 {
            for x in 0..4 {
                let value = bytes[y * 256 + x] as f32 / 255.;
                assert!(
                    (value - expected).abs() <= 1.5 / 255.,
                    "sky +{clouds} clouds pixel{x}/{y}: {value} != {expected}"
                );
            }
        }
        drop(bytes);
        staging.unmap();
    }
}
