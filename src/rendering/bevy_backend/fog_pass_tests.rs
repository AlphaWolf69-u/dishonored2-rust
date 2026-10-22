//! Full GPU pass composition: mask, premultiplication, HDR units and alpha.
use super::{
    fog::{FogCanvas, FogParameters, FogView},
    fog_pass::{FogInputs, FogPass},
    fog_targets::FogExtents,
};
use bevy::{
    camera::CameraProjection,
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
    },
};

pub(super) fn check_upscales(device: &RenderDevice, queue: &RenderQueue) {
    check_upscale(device, queue, false);
    check_upscale(device, queue, true);
}

fn check_upscale(device: &RenderDevice, queue: &RenderQueue, nearest: bool) {
    let pipelines = super::fog_pipeline::FogPipelines::new(device, TextureFormat::Rgba16Float);
    let pipeline = if nearest {
        &pipelines.upscale
    } else {
        &pipelines.upscale_slope
    };
    let make = |size: UVec2, format, usage, data: Option<&[u8]>| {
        let desc = TextureDescriptor {
            label: Some("depth-aware fog upscale fixture"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        };
        match data {
            Some(data) => {
                device.create_texture_with_data(queue, &desc, TextureDataOrder::LayerMajor, data)
            }
            None => device.create_texture(&desc),
        }
    };
    let full = UVec2::splat(8);
    let depth = make(
        full,
        TextureFormat::Depth32Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        None,
    );
    let depth_view = depth.create_view(&default());
    let output = make(
        full,
        TextureFormat::Rgba16Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        None,
    );
    let output_view = output.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    let linear = device.create_sampler(&SamplerDescriptor {
        min_filter: FilterMode::Linear,
        mag_filter: FilterMode::Linear,
        ..default()
    });
    // Upload through R32F and write the exact raw words to a sampleable depth
    // attachment; no depth-copy or projection rounding contaminates the oracle.
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("fog upscale raw depth input"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var source:texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    return textureLoad(source,vec2<i32>(p.xy),0).r;
}
"#
            .into(),
        ),
    });
    let write_depth = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: RawVertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &module,
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
    let fixture = if nearest {
        // Unchanged captured PS20350, including its HDR divide and actual blend.
        include_str!("../../../tests/fixtures/fog_upscale_nearest.txt")
    } else {
        include_str!("../../../tests/fixtures/fog_upscale_slope.txt")
    };
    let rows = fixture
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), if nearest { 384 } else { 256 });
    for group in rows.chunks_exact(64) {
        let width = group[0][0].parse::<u32>().unwrap();
        let height = group[0][1].parse::<u32>().unwrap();
        let pattern = group[0][2].parse::<u32>().unwrap();
        let mut colors = Vec::new();
        for y in 0..height {
            for x in 0..width {
                for value in [
                    x as f32 * 0.125 + 0.03125,
                    y as f32 * 0.1875 + 0.0625,
                    (x + y) as f32 * 0.09375,
                    (x as f32 - y as f32) * 0.5,
                ] {
                    colors.extend_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
                }
            }
        }
        let color = make(
            UVec2::new(width, height),
            TextureFormat::Rgba16Float,
            TextureUsages::TEXTURE_BINDING,
            Some(&colors),
        );
        let color_view = color.create_view(&default());
        let mut raw_depth = Vec::new();
        let mut distances = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                // Reference input arithmetic is f64, then rounded once to R32F.
                let distance = if pattern == 2 {
                    2.
                } else {
                    1. + x as f64 * 0.125
                        + y as f64 * 0.2
                        + if pattern != 0 && x >= 4 { 8. } else { 0. }
                        + if pattern != 0 && y == 3 { 4. } else { 0. }
                };
                distances.push(distance);
                raw_depth.extend_from_slice(&((1. / distance) as f32).to_le_bytes());
            }
        }
        let source = make(
            full,
            TextureFormat::R32Float,
            TextureUsages::TEXTURE_BINDING,
            Some(&raw_depth),
        );
        let source_view = source.create_view(&default());
        let source_bind = device.create_bind_group(
            None,
            &BindGroupLayout::from(write_depth.get_bind_group_layout(0)),
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&source_view),
            }],
        );
        let view = FogView {
            clip_from_world: Mat4::IDENTITY,
            world_from_view: Mat4::IDENTITY,
            world_from_clip: Mat4::IDENTITY,
            ray_scale: Vec4::new(0., 0., 7000., 0.0002),
            unproject: Vec4::new(1., 1., 0., 0.75),
            extent: Vec4::new(8., 8., 4., 4.),
            reduced_extent: Vec4::new(4., 4., width as f32, height as f32),
            eye: Vec4::ZERO,
            jitter: Vec4::ZERO,
        };
        let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &ShaderBuffer::from(view).data.unwrap(),
            usage: BufferUsages::UNIFORM,
        });
        let reduced = if nearest {
            let mut bytes = Vec::new();
            for y in 0..4 {
                for x in 0..4 {
                    let samples = [
                        distances[(y * 2) * 8 + x * 2],
                        distances[(y * 2) * 8 + x * 2 + 1],
                        distances[(y * 2 + 1) * 8 + x * 2],
                        distances[(y * 2 + 1) * 8 + x * 2 + 1],
                    ];
                    for value in [
                        samples.into_iter().fold(f64::INFINITY, f64::min),
                        samples.into_iter().fold(f64::NEG_INFINITY, f64::max),
                    ] {
                        bytes.extend(half::f16::from_f64(value).to_bits().to_le_bytes());
                    }
                }
            }
            Some(make(
                UVec2::splat(4),
                TextureFormat::Rg16Float,
                TextureUsages::TEXTURE_BINDING,
                Some(&bytes),
            ))
        } else {
            None
        };
        let reduced_view = reduced.as_ref().map(|image| image.create_view(&default()));
        let mut entries = vec![
            BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&depth_view),
            },
            BindGroupEntry {
                binding: 12,
                resource: BindingResource::TextureView(&color_view),
            },
            BindGroupEntry {
                binding: 13,
                resource: BindingResource::Sampler(&sampler),
            },
        ];
        if let Some(view) = &reduced_view {
            entries.extend([
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(view),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(&linear),
                },
            ]);
        }
        let bind = device.create_bind_group(
            None,
            &BindGroupLayout::from(pipeline.get_bind_group_layout(0)),
            &entries,
        );
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
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
            pass.set_pipeline(&write_depth);
            pass.set_bind_group(0, &source_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::new(0.1, 0.2, 0.3, 0.625).into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        super::fog_tests::check_rgba16_pixels(
            device,
            queue,
            encoder,
            &output,
            &staging,
            group,
            4,
            if nearest {
                "captured nearest fog upscale"
            } else {
                "slope-aware fog upscale"
            },
        );
    }
}

pub(super) fn check_composition(device: &RenderDevice, queue: &RenderQueue) {
    let extents = FogExtents::new(UVec2::splat(8), 1).unwrap();
    let projection =
        super::camera_projection::GameProjection::new(90., 0.01, 7000.).get_clip_from_view();
    let view = FogView {
        clip_from_world: projection,
        world_from_view: Mat4::IDENTITY,
        world_from_clip: projection.inverse(),
        ray_scale: Vec4::new(
            1. / projection.x_axis.x,
            1. / projection.y_axis.y,
            7000.,
            0.0002,
        ),
        unproject: FogView::depth_coefficients(projection.inverse(), 0.75),
        extent: Vec4::new(8., 8., 4., 4.),
        reduced_extent: Vec4::splat(4.),
        eye: Vec4::ZERO,
        jitter: Vec4::ZERO,
    };
    let fog = crate::rendering::fog::world::WorldFog {
        flags: 8,
        distance: Vec4::new(0., 0.01, 3.5, 0.),
        distance_start: Vec4::new(0.02, 0.03, 0.04, 1.),
        distance_end: Vec4::new(0.02, 0.03, 0.04, 1.),
        ..default()
    };
    let parameters = FogParameters::new(
        fog,
        crate::rendering::fog::world::WorldFogEffects {
            far_clip: 7000.,
            ..default()
        },
    );
    let canvas = FogCanvas {
        s: Vec4::X,
        t: Vec4::Y,
        octaves: [Vec4::ZERO; 10],
    };
    let mut pass = FogPass::new(
        device,
        queue,
        extents,
        TextureFormat::Rgba16Float,
        view,
        parameters,
        canvas,
    );
    let make = |size, format, usage| {
        device.create_texture(&TextureDescriptor {
            label: Some("fog composition fixture"),
            size: Extent3d {
                width: size,
                height: size,
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
    let upload = |size: u32, format, pixel: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("fog composition fixture input"),
                size: Extent3d {
                    width: size,
                    height: size,
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
            &pixel.repeat((size * size) as usize),
        )
    };
    let depth = make(
        8,
        TextureFormat::Depth32Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    );
    let depth_view = depth.create_view(&default());
    let atlas = make(
        1,
        TextureFormat::Depth32Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    );
    let atlas_view = atlas.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let curve = upload(
        1,
        TextureFormat::R16Float,
        &half::f16::from_f32(0.5).to_bits().to_le_bytes(),
    );
    let noise = upload(1, TextureFormat::R8Unorm, &[128]);
    let curve_view = curve.create_view(&default());
    let noise_view = noise.create_view(&default());
    let parallel = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &ShaderBuffer::from(super::shadow_sampling::ParallelSamplingUniform::default())
            .data
            .unwrap(),
        usage: BufferUsages::STORAGE,
    });
    let hdr = make(
        8,
        TextureFormat::Rgba16Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let hdr_view = hdr.create_view(&default());
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 8 * 256 + 16,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let original = [0.1, 0.2, 0.3, 0.625];
    let original: [f32; 4] = original.map(|v| half::f16::from_f32(v).to_f32());
    let allocations = [
        pass.targets.depth.texture.id(),
        pass.targets.interior.texture.id(),
        pass.targets.canvas.texture.id(),
        pass.targets.shadow.texture.id(),
        pass.targets.world.texture.id(),
    ];
    // D3D11 output with the inspected fog blend states and RGBA16F targets.
    // The device rounds source values as well as attachment values; an ideal
    // f32 blend followed by one f16 conversion is not the original result.
    let expected = include_str!("../../../tests/fixtures/fog_blending.txt")
        .split_whitespace()
        .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 12);
    let mut second = canvas;
    second.octaves[0].z = 1.;
    pass.update_light_canvas(device, queue, Some(second));
    let second_id = pass.light_canvas_view().unwrap().id();
    assert_ne!(second_id, pass.targets.canvas.view.id());
    let canvas_values = device.create_buffer(&BufferDescriptor {
        label: Some("independent canvas readback"),
        size: 16,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("read both turbulence canvases"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var first:texture_2d<f32>;
@group(0) @binding(1) var second:texture_2d<f32>;
@group(0) @binding(2) var<storage,read_write> values:vec4<f32>;
@compute @workgroup_size(1) fn read_canvas() {
    values=vec4(textureLoad(first,vec2(0),0).r,textureLoad(second,vec2(0),0).r,0.0,0.0);
}
"#
            .into(),
        ),
    });
    let read_canvas = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: Some("read both turbulence canvases"),
        layout: None,
        module: &module,
        entry_point: Some("read_canvas"),
        compilation_options: default(),
        cache: None,
    });
    let canvas_binding = device.create_bind_group(
        None,
        &read_canvas.get_bind_group_layout(0).into(),
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(&pass.targets.canvas.view),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(pass.light_canvas_view().unwrap()),
            },
            BindGroupEntry {
                binding: 2,
                resource: canvas_values.as_entire_binding(),
            },
        ],
    );
    for (nearest, case, mask) in [true, false].into_iter().flat_map(|nearest| {
        [0u8, 128, 255]
            .into_iter()
            .enumerate()
            .map(move |(case, mask)| (nearest, case, mask))
    }) {
        // A changed input view rebuilds bindings, while unchanged viewport
        // dimensions retain all intermediate textures and uniform buffers.
        pass.update(device, queue, extents, view, parameters, canvas);
        second.octaves[0].z = if nearest { 1. } else { 0.5 };
        pass.update_light_canvas(device, queue, Some(second));
        assert_eq!(pass.light_canvas_view().unwrap().id(), second_id);
        assert_eq!(
            allocations,
            [
                pass.targets.depth.texture.id(),
                pass.targets.interior.texture.id(),
                pass.targets.canvas.texture.id(),
                pass.targets.shadow.texture.id(),
                pass.targets.world.texture.id()
            ]
        );
        let mask_texture = upload(4, TextureFormat::R8Unorm, &[mask]);
        let mask_view = mask_texture.create_view(&default());
        let mut settings =
            crate::platform::game_settings::fog_render_settings("\"r_skipFogBlur\" \"1\"").unwrap();
        settings.nearest_upscale = nearest;
        let inputs = FogInputs {
            shadow_parameters: &super::fog_shadow::FogShadowParameters {
                viewport: [
                    0.,
                    0.,
                    pass.targets.extents.shadow.x as f32,
                    pass.targets.extents.shadow.y as f32,
                ],
                resolution_projected: [1., 1., 15000., 0.],
                jitter: [0.; 4],
                options: [0, 0, 1, 0],
            },
            projected_height: &noise_view,
            areas: None,
            scattering: None,
            settings,
            ray_mask: None,
            shadowed_volumes: false,
            depth: &depth_view,
            sky_mask: &mask_view,
            curve: &curve_view,
            noise: &noise_view,
            atlas: &atlas_view,
            parallel: &parallel,
        };
        let mut encoder = device.create_command_encoder(&default());
        let clip = projection * Vec4::new(0., 0., -4., 1.);
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(clip.z / clip.w),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
        }
        let source = pass.record_sources(device, queue, &mut encoder, &inputs, &[]);
        // Water colour can write HDR after fog production. Clearing here also
        // catches accidental early composition: the native oracle still needs
        // the completed fog blended on top of this later HDR content.
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &hdr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(
                            LinearRgba::new(original[0], original[1], original[2], original[3])
                                .into(),
                        ),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
        }
        source.record_composite(&mut encoder, &hdr_view);
        {
            let mut read = encoder.begin_compute_pass(&default());
            read.set_pipeline(&read_canvas);
            read.set_bind_group(0, &canvas_binding, &[]);
            read.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&canvas_values, 0, &staging, 8 * 256, 16);
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &hdr,
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
            hdr.size(),
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
        let first = f32::from_le_bytes(bytes[2048..2052].try_into().unwrap());
        let second = f32::from_le_bytes(bytes[2052..2056].try_into().unwrap());
        assert_eq!(first, 0., "light canvas changed the world-fog canvas");
        let expected_second = if nearest { 128. } else { 64. } / 255.;
        assert!(
            (second - expected_second).abs() < 0.000001,
            "second canvas uniform was stale or aliased: {second} != {expected_second}"
        );
        for y in 0..8 {
            for x in 0..8 {
                for channel in 0..4 {
                    let offset = y * 256 + x * 8 + channel * 2;
                    let value = half::f16::from_bits(u16::from_le_bytes(
                        bytes[offset..offset + 2].try_into().unwrap(),
                    ))
                    .to_f32();
                    let reference = expected[case * 4 + channel];
                    assert!(
                        (value - reference).abs() <= (reference.abs() * 0.001).max(0.0001),
                        "fog mask={mask} pixel={x}/{y} channel={channel}: {value} != {reference}"
                    );
                }
            }
        }
        drop(bytes);
        staging.unmap();
    }
    pass.update_light_canvas(device, queue, None);
    assert!(pass.light_canvas_view().is_none());
}

/// Compare the production reduction with waterDepthReduction's single 2x2
/// gather, including quarter resolution and odd dimensions. A sloped depth
/// field distinguishes this operation from a recursive four-by-four pyramid.
pub(super) fn check_depth_reduction(device: &RenderDevice, queue: &RenderQueue) {
    use bevy::render::storage::ShaderBuffer;
    let projection =
        super::camera_projection::GameProjection::new(90., 0.01, 7000.).get_clip_from_view();
    let mut view = FogView {
        clip_from_world: projection,
        world_from_view: Mat4::IDENTITY,
        world_from_clip: projection.inverse(),
        ray_scale: Vec4::ONE,
        unproject: FogView::depth_coefficients(projection.inverse(), 1.),
        extent: Vec4::ONE,
        reduced_extent: Vec4::ONE,
        eye: Vec4::ZERO,
        jitter: Vec4::ZERO,
    };
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("sloped depth reduction fixture"),
        source: ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var<uniform> projection:mat4x4<f32>;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    let distance=1.0+floor(p.x)+3.0*floor(p.y);
    let clip=projection*vec4(0.0,0.0,-distance,1.0);
    return clip.z/clip.w;
}
"#
            .into(),
        ),
    });
    let source_pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
        label: Some("sloped depth fixture"),
        layout: None,
        vertex: RawVertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: default(),
            buffers: &[],
        },
        fragment: Some(RawFragmentState {
            module: &module,
            entry_point: Some("fragment"),
            compilation_options: default(),
            targets: &[],
        }),
        primitive: PrimitiveState::default(),
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
    let source_uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &ShaderBuffer::from(projection).data.unwrap(),
        usage: BufferUsages::UNIFORM,
    });
    let source_bind = device.create_bind_group(
        None,
        &BindGroupLayout::from(source_pipeline.get_bind_group_layout(0)),
        &[BindGroupEntry {
            binding: 0,
            resource: source_uniform.as_entire_binding(),
        }],
    );
    let pipelines = super::fog_pipeline::FogPipelines::new(device, TextureFormat::Rgba16Float);
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    for (full, quality) in [
        (UVec2::new(16, 12), 1),
        (UVec2::new(40, 36), 2),
        (UVec2::new(41, 37), 1),
        (UVec2::new(41, 37), 2),
    ] {
        let extents = FogExtents::new(full, quality).unwrap();
        let reduced = extents.depth;
        view.extent = full
            .as_vec2()
            .extend(reduced.x as f32)
            .extend(reduced.y as f32);
        view.reduced_extent = reduced
            .as_vec2()
            .extend(extents.world.x as f32)
            .extend(extents.world.y as f32);
        let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &ShaderBuffer::from(view).data.unwrap(),
            usage: BufferUsages::UNIFORM,
        });
        let make = |size: UVec2, format, usage| {
            device.create_texture(&TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: size.x,
                    height: size.y,
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
            full,
            TextureFormat::Depth32Float,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        );
        let output = make(
            reduced,
            TextureFormat::Rg16Float,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        );
        let depth_view = depth.create_view(&default());
        let output_view = output.create_view(&default());
        let bind = device.create_bind_group(
            None,
            &BindGroupLayout::from(pipelines.depth.get_bind_group_layout(0)),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&depth_view),
                },
                BindGroupEntry {
                    binding: 13,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
        );
        let staging = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 256 * reduced.y as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
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
            pass.set_pipeline(&source_pipeline);
            pass.set_bind_group(0, &source_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&pipelines.depth);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &output,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(reduced.y),
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
        for y in 0..reduced.y {
            for x in 0..reduced.x {
                let base = ((UVec2::new(x, y).as_vec2() + Vec2::splat(0.5)) * full.as_vec2()
                    / reduced.as_vec2()
                    - Vec2::splat(0.5))
                .floor()
                .as_ivec2();
                let mut minimum = f32::INFINITY;
                let mut maximum = f32::NEG_INFINITY;
                for offset in [IVec2::ZERO, IVec2::X, IVec2::Y, IVec2::ONE] {
                    let pixel = (base + offset).clamp(IVec2::ZERO, full.as_ivec2() - IVec2::ONE);
                    let distance = 1. + pixel.x as f32 + 3. * pixel.y as f32;
                    minimum = minimum.min(distance);
                    maximum = maximum.max(distance);
                }
                for (channel, expected) in [minimum, maximum].into_iter().enumerate() {
                    let offset = y as usize * 256 + x as usize * 4 + channel * 2;
                    let actual = half::f16::from_bits(u16::from_le_bytes(
                        bytes[offset..offset + 2].try_into().unwrap(),
                    ))
                    .to_f32();
                    assert!((actual-expected).abs()<=expected*0.001,"depth reduction {full:?} quality={quality} pixel={x}/{y} channel={channel}: {actual} != {expected}");
                }
            }
        }
        drop(bytes);
        staging.unmap();
    }
}
