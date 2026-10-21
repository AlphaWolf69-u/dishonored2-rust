//! Installed radial shaders against their actual D3D11 numerical outputs.
use super::{
    fog::FogParameters,
    fog_rays::{FogRayPipelines, FogRayUniform},
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub(super) fn check_rays(device: &RenderDevice, queue: &RenderQueue) {
    let state = crate::render_state::parse_render_state(
        "{ blend ONE ONE alphaBlend ONE ONE depthEnable 0 depthWrite 0 depthfunc ALWAYS cull NONE }", None).unwrap();
    let pipelines =
        FogRayPipelines::new(device, state, TextureFormat::Depth24PlusStencil8).unwrap();
    let upload = |format, data: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("radial shader comparison input"),
                size: Extent3d {
                    width: 8,
                    height: 8,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            data,
        )
    };
    let mut source = Vec::new();
    let mut depth = Vec::new();
    let mut canvas = Vec::new();
    let mut interior = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            for v in [x as f32 * 0.12, y as f32 * 0.09, (x + y) as f32 * 0.06, 0.] {
                source.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
            }
            let z = 1. + x as f32 * 0.125 + y as f32 * 0.05 + if x >= 4 { 20. } else { 0. };
            for v in [z, z + 0.5] {
                depth.extend(half::f16::from_f32(v).to_bits().to_le_bytes());
            }
            canvas.push(((x * 37 + y * 19) % 256) as u8);
            interior.extend(
                half::f16::from_f32((x + y) as f32 * 0.25)
                    .to_bits()
                    .to_le_bytes(),
            );
        }
    }
    let textures = [
        upload(TextureFormat::Rgba16Float, &source),
        upload(TextureFormat::Rg16Float, &depth),
        upload(TextureFormat::R8Unorm, &canvas),
        upload(TextureFormat::R16Float, &interior),
    ];
    let views = textures.each_ref().map(|t| t.create_view(&default()));
    let sampler = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    let target = |format| {
        device.create_texture(&TextureDescriptor {
            label: Some("radial shader comparison output"),
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
                | if format.has_depth_aspect() {
                    TextureUsages::empty()
                } else {
                    TextureUsages::COPY_SRC
                },
            view_formats: &[],
        })
    };
    let color = target(TextureFormat::Rgba16Float);
    let visibility = target(TextureFormat::R8Unorm);
    let depth = target(TextureFormat::Depth24PlusStencil8);
    let color_view = color.create_view(&default());
    let visibility_view = visibility.create_view(&default());
    let depth_view = depth.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8 * 256,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut ray = UniformBuffer::from(FogRayUniform::default());
    let mut parameters = UniformBuffer::from(FogParameters {
        distance: Vec4::ZERO,
        distance_start: Vec4::ZERO,
        distance_end: Vec4::ZERO,
        vertical: Vec4::ZERO,
        vertical_start: Vec4::ZERO,
        vertical_end: Vec4::ZERO,
        height: Vec4::ZERO,
        scattering_direction_phase: Vec4::ZERO,
        scattering_color_intensity: Vec4::ZERO,
        effects: Vec4::ZERO,
        turbulence: Vec4::new(5., 30., 0., 0.),
        turbulence_color: Vec4::new(0.4, 1., 0.7, 0.),
        flags: UVec4::ZERO,
    });
    ray.write_buffer(device, queue);
    parameters.write_buffer(device, queue);
    let mut entries = vec![
        BindGroupEntry {
            binding: 0,
            resource: ray.buffer().unwrap().as_entire_binding(),
        },
        BindGroupEntry {
            binding: 1,
            resource: parameters.buffer().unwrap().as_entire_binding(),
        },
    ];
    entries.extend(views.iter().enumerate().map(|(i, v)| BindGroupEntry {
        binding: i as u32 + 2,
        resource: BindingResource::TextureView(v),
    }));
    entries.push(BindGroupEntry {
        binding: 6,
        resource: BindingResource::Sampler(&sampler),
    });
    let bind = device.create_bind_group(None, &pipelines.layout, &entries);
    let cases = include_str!("../../../tests/fixtures/fog_rays.txt")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 1152);
    for (case, group) in cases.chunks_exact(64).enumerate() {
        let kind: u32 = group[0][0].parse().unwrap();
        ray.set(FogRayUniform {
            center: Vec4::new(
                group[0][1].parse().unwrap(),
                group[0][2].parse().unwrap(),
                0.,
                1.,
            ),
            controls: Vec4::new(group[0][3].parse().unwrap(), 0.96815, 0.125, 0.),
            extent: Vec4::new(8., 8., 0.125, 0.125),
        });
        ray.write_buffer(device, queue);
        // Repeat scattering with bit 0x40 set: its installed override must
        // reject these pixels independently of the shader's radial calculation.
        for stencil in if kind == 0 { vec![0] } else { vec![0, 0x40] } {
            let visibility_pass = kind == 0;
            let (output, view) = if visibility_pass {
                (&visibility, &visibility_view)
            } else {
                (&color, &color_view)
            };
            let mut encoder = device.create_command_encoder(&default());
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("radial shader comparison"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::NONE.into()),
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: (!visibility_pass).then_some(
                        RenderPassDepthStencilAttachment {
                            view: &depth_view,
                            depth_ops: None,
                            stencil_ops: Some(Operations {
                                load: LoadOp::Clear(stencil),
                                store: StoreOp::Store,
                            }),
                        },
                    ),
                    ..default()
                });
                pass.set_pipeline(if visibility_pass {
                    &pipelines.visibility
                } else {
                    pipelines.scattering(kind == 2)
                });
                if !visibility_pass {
                    pass.set_stencil_reference(0x40);
                }
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: output,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                },
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(8),
                    },
                },
                output.size(),
            );
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
            device
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = staging.slice(..).get_mapped_range();
            for (pixel, row) in group.iter().enumerate() {
                for field in 0..if visibility_pass { 1 } else { 4 } {
                    let expected = f32::from_bits(u32::from_str_radix(row[5 + field], 16).unwrap());
                    let (actual, expected, tolerance) = if visibility_pass {
                        (
                            bytes[(pixel / 8) * 256 + pixel % 8] as f32 / 255.,
                            (expected * 255.).round().clamp(0., 255.) / 255.,
                            1. / 255.,
                        )
                    } else {
                        let offset = (pixel / 8) * 256 + (pixel % 8) * 8 + field * 2;
                        let actual = half::f16::from_bits(u16::from_le_bytes(
                            bytes[offset..offset + 2].try_into().unwrap(),
                        ))
                        .to_f32();
                        let expected = if stencil == 0x40 {
                            0.
                        } else {
                            half::f16::from_f32(expected).to_f32()
                        };
                        (actual, expected, expected.abs() * 0.0015 + 0.00005)
                    };
                    assert!((actual-expected).abs()<=tolerance,"radial case={case} kind={kind} stencil={stencil} pixel={pixel} field={field}: {actual} != {expected}");
                }
            }
            drop(bytes);
            staging.unmap();
        }
    }
    // The projected colour mask consumes scene radiance before radial sampling.
    let sun_program = super::fog_sun_mask::FogSunProgram::new(device);
    let pipelines = super::fog_sun_mask::FogSunMaskPipelines::with_program(device, &sun_program);
    parameters.get_mut().scattering_direction_phase = Vec4::new(0.6, 0., 0.8, 0.75);
    parameters.get_mut().scattering_color_intensity = Vec4::new(10., 9.8, 7.2, 0.3);
    parameters.write_buffer(device, queue);
    let mut sun = UniformBuffer::from(super::fog_sun_mask::FogSunMaskUniform {
        transform1: Vec4::new(1., 0., 0., 1.),
        transform2: Vec4::new(0., 0., 0., 1.),
        threshold_bias: Vec4::new(0.02, 0.03, 0.04, 0.),
        threshold_scale: Vec4::new(0.1, 0.2, 0.3, 0.),
        color: Vec4::new(0.8, 0.6, 0.4, 0.7),
        projection: Vec4::new(1., 1., 30., 1.),
        viewport: Vec4::new(8., 8., 10., 0.),
        game_world_from_view: Mat4::IDENTITY,
    });
    sun.write_buffer(device, queue);
    let mut entries = vec![
        BindGroupEntry {
            binding: 0,
            resource: sun.buffer().unwrap().as_entire_binding(),
        },
        BindGroupEntry {
            binding: 1,
            resource: parameters.buffer().unwrap().as_entire_binding(),
        },
    ];
    entries.extend(
        views[..3]
            .iter()
            .enumerate()
            .map(|(i, view)| BindGroupEntry {
                binding: i as u32 + 2,
                resource: BindingResource::TextureView(view),
            }),
    );
    entries.push(BindGroupEntry {
        binding: 5,
        resource: BindingResource::Sampler(&sampler),
    });
    let bind = device.create_bind_group(None, &pipelines.layout, &entries);
    let cases = include_str!("../../../tests/fixtures/fog_sun_mask.txt")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 192);
    for (variant, group) in cases.chunks_exact(64).enumerate() {
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("sun colour mask comparison"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(pipelines.pipeline(variant != 0, variant == 2));
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..6, 0..1);
        }
        super::fog_tests::check_rgba16_pixels(
            device,
            queue,
            encoder,
            &color,
            &staging,
            group,
            2,
            &format!("sun mask variant={variant}"),
        );
    }
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
    let mut programs = crate::assets::shader_cache::ProgramLibrary::open(
        &base,
        crate::assets::shader_cache::ShaderCacheBackend::Pc,
        &catalog,
    )
    .unwrap();
    let state = programs
        .definition(b"PostFX_Sun_Sprite")
        .unwrap()
        .unwrap()
        .render_state();
    let sprite_image = upload(TextureFormat::Rgba8Unorm, &[64, 32, 16, 128].repeat(64));
    let sprite_view = sprite_image.create_view(&default());
    let mut uniform = *sun.get();
    uniform.color = Vec4::new(0.5, 0.75, 1., 0.25);
    uniform.projection.w = 1. / (139.26 * 0.75);
    let mut sprite = super::fog_sun_sprite::SunSpritePass::new(
        device,
        &sun_program,
        state,
        TextureFormat::Rgba16Float,
        TextureFormat::Depth24PlusStencil8,
        uniform,
    )
    .unwrap();
    sprite.prepare(device, queue, uniform, &sprite_view);
    let cases = include_str!("../../../tests/fixtures/fog_sun_sprite.txt")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 256);
    for group in cases.chunks_exact(64) {
        let additive = group[0][0] == "1";
        let raw_depth: f32 = group[0][1].parse().unwrap();
        let mut encoder = device.create_command_encoder(&default());
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("sun sprite reference background"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::new(0.1, 0.2, 0.3, 0.625).into()),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(raw_depth),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(0),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        sprite.record(&mut encoder, &color_view, &depth_view, additive);
        super::fog_tests::check_rgba16_pixels(
            device,
            queue,
            encoder,
            &color,
            &staging,
            group,
            3,
            &format!("visible sun additive={additive} depth={raw_depth}"),
        );
    }
}
