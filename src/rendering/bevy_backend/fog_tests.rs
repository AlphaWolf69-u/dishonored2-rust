//! Compare production WGSL fog composition against the installed HLSL oracle.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};

#[test]
#[ignore = "requires installed shader definitions; inspect sky/cloud fog-mask state"]
fn installed_sky_cloud_fog_mask_states() {
    use crate::assets::{
        archive::Catalog,
        installation,
        shader_cache::{ProgramLibrary, ShaderCacheBackend},
    };
    let base = installation::game_base().unwrap();
    let catalog = Catalog::open(&base).unwrap();
    let mut library = ProgramLibrary::open(&base, ShaderCacheBackend::Pc, &catalog).unwrap();
    for name in [b"arkSky".as_slice(), b"arkClouds".as_slice()] {
        let definition = library.definition(name).unwrap().unwrap();
        let state = definition.render_state();
        let prepared = crate::render_state::sky_cloud_draw_state(state);
        println!(
            "{} state={state:08x?} blend={:?}",
            String::from_utf8_lossy(name),
            crate::render_state::blend_state(&state)
        );
        println!(
            "{} prepared fog MRT={:?}",
            String::from_utf8_lossy(name),
            crate::render_state::blend_state(&prepared)
        );
        let variant = library
            .serialized_variant(
                name,
                &crate::render_program::SerializedParameterBlock {
                    flags: 0,
                    parameters: Vec::new(),
                },
            )
            .unwrap()
            .unwrap();
        println!(
            "{} stages={:?}",
            String::from_utf8_lossy(name),
            variant
                .stages
                .iter()
                .map(|stage| (stage.kind, &stage.bytecode))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn fog_depth_transport_preserves_finite_reverse_projection() {
    use super::{
        camera_projection::GameProjection,
        fog::{FogCanvas, FogView},
    };
    use bevy::{camera::CameraProjection, render::storage::ShaderBuffer};

    let projection = GameProjection::new(90., 0.01, 7000.).get_clip_from_view();
    let coefficients = FogView::depth_coefficients(projection.inverse(), 0.75);
    for distance in [0.01, 0.05, 1., 150., 3000., 7000.] {
        let clip = projection * Vec4::new(0., 0., -distance, 1.);
        let raw = clip.z / clip.w;
        let restored = coefficients.x / (coefficients.y * raw + coefficients.z);
        assert!((restored - distance).abs() <= distance * 0.00001);
    }
    assert_eq!(coefficients.w, 0.75);
    let view = FogView {
        clip_from_world: projection,
        world_from_view: Mat4::IDENTITY,
        world_from_clip: projection.inverse(),
        ray_scale: Vec4::ONE,
        unproject: coefficients,
        extent: Vec4::ONE,
        reduced_extent: Vec4::ONE,
        eye: Vec4::ZERO,
        jitter: Vec4::ZERO,
    };
    assert_eq!(ShaderBuffer::from(view).data.unwrap().len(), 288);
    let canvas = FogCanvas {
        s: Vec4::X,
        t: Vec4::Y,
        octaves: [Vec4::ZERO; 10],
    };
    assert_eq!(ShaderBuffer::from(canvas).data.unwrap().len(), 192);
}

#[test]
#[ignore = "requires graphics adapter; executes the production fog shader math"]
fn world_fog_shader_matches_installed_composition() {
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
    app.sub_app_mut(RenderApp).world_mut().init_resource::<super::tiled_lighting_gpu::TlfPipelines>();
    let render = app.sub_app(RenderApp).world();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = errors.clone();
    device
        .wgpu_device()
        .on_uncaptured_error(std::sync::Arc::new(move |error| {
            eprintln!("Fog GPU validation: {error}");
            captured.lock().unwrap().push(format!("{error}"));
        }));
    let pipelines = super::fog_pipeline::FogPipelines::new(device, TextureFormat::Rgba16Float);
    check_interior_depth(device, queue, &pipelines);
    super::fog_pass_tests::check_composition(device, queue);
    super::fog_pass_tests::check_depth_reduction(device, queue);
    super::fog_pass_tests::check_slope_upscale(device, queue);
    super::fog_alpha_tests::check_alpha(device, queue);
    super::fog_alpha_tests::check_volume(device, queue);
    super::fog_alpha_tests::check_scattering(device, queue);
    super::fog_light_scattering_tests::check_pass(device, queue);
    super::alpha_depth::tests::check_preparation(device, queue, render.resource::<super::tiled_lighting_gpu::TlfPipelines>());
    super::depth_restore_tests::check_restoration(device, queue, render.resource::<super::tiled_lighting_gpu::TlfPipelines>());
    super::tiled_lighting_pass::tests::check_submission(device,queue,render.resource::<super::tiled_lighting_gpu::TlfPipelines>());
    super::fog_filter_tests::check_filter(device, queue);
    super::fog_rays_tests::check_rays(device, queue);
    super::sky_mask_tests::check_mask_accumulation(device, queue);
    let source = format!(
        "{}\n{}",
        include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
        r#"
@group(0) @binding(0) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) tid:vec3<u32>) {
let id=tid.x;
let p=WorldFogParameters(
    vec4(10.0,1.0/90.0,3.5,0.0),vec4(.1,.3,.8,.2),vec4(1.1,.8,.2,.7),
    vec4(2.0,1.0/70.0,.4,0.0),vec4(.2,.7,1.1,.3),vec4(1.8,.4,.1,.8),
    vec4(-2.0,14.0,.03,0.0),vec4(.6,0.0,.8,.75),vec4(10.0,9.8,7.2,.3),
    vec4(300.0,30.0,.25,0.0),vec4(5.0,100.0,0.0,0.0),vec4(.4,1.0,.7,0.0),vec4<u32>((id%32u)<<3u,0u,0u,0u));
let s=WorldFogSample(vec3((f32(id%5u)-2.0)*.3,(f32(id%7u)-3.0)*.2,1.0),f32(id%11u)-5.0,
    f32(id%13u)*17.0,f32(id%4u)*9.0,f32(id%9u)/8.0,f32(id%7u)/6.0,
    vec4(0.0,.25,.5,select(.75,1.0,(id&32u)!=0u)),f32(id%5u)/4.0);
output[id]=fog_world(p,s);
}

"#
    );
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
        label: Some("world fog composition oracle"),
        source: ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: default(),
            cache: None,
        });
    let size = 64 * 16;
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
    device
        .poll(PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    assert!(
        errors.lock().unwrap().is_empty(),
        "fog GPU validation: {:?}",
        errors.lock().unwrap()
    );
    let bytes = readback.slice(..).get_mapped_range();
    let expected = include_str!("../../../tests/fixtures/fog_composition.txt")
        .split_whitespace()
        .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(bytes.len(), expected.len() * 4);
    for (index, (raw, reference)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let value = f32::from_le_bytes(raw.try_into().unwrap());
        assert!(
            (value - reference).abs() < 0.0003,
            "fog case{} field{}: {value} != {reference}",
            index / 4,
            index % 4
        );
    }
    drop(bytes);
    readback.unmap();
}

/// Exercise the actual additive volume rasterization, including winding and
/// clipping to the receiver. Two opposite faces enclose the camera ray from
/// Z=2 to Z=6; receivers before, inside and behind must accumulate 0, 2 and 4.
fn check_interior_depth(
    device: &RenderDevice,
    queue: &RenderQueue,
    pipelines: &super::fog_pipeline::FogPipelines,
) {
    use super::{camera_projection::GameProjection, fog::FogView};
    use bevy::{camera::CameraProjection, render::storage::ShaderBuffer};
    let projection = GameProjection::new(90., 0.01, 7000.).get_clip_from_view();
    let view = FogView {
        clip_from_world: projection,
        world_from_view: Mat4::IDENTITY,
        world_from_clip: projection.inverse(),
        ray_scale: Vec4::ONE,
        unproject: FogView::depth_coefficients(projection.inverse(), 1.),
        extent: Vec4::new(16., 16., 8., 8.),
        reduced_extent: Vec4::new(8., 8., 8., 8.),
        eye: Vec4::ZERO,
        jitter: Vec4::ZERO,
    };
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("fog volume camera fixture"),
        contents: &ShaderBuffer::from(view).data.unwrap(),
        usage: BufferUsages::UNIFORM,
    });
    let positions: [[f32; 3]; 12] = [
        [-20., 2., -20.],
        [-20., 2., 20.],
        [20., 2., -20.],
        [20., 2., -20.],
        [-20., 2., 20.],
        [20., 2., 20.],
        [-20., 6., -20.],
        [20., 6., -20.],
        [-20., 6., 20.],
        [20., 6., -20.],
        [20., 6., 20.],
        [-20., 6., 20.],
    ];
    let vertices = positions
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect::<Vec<_>>();
    let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("fog volume opposing faces"),
        contents: &vertices,
        usage: BufferUsages::VERTEX,
    });
    let make_texture = |size, format, usage| {
        device.create_texture(&TextureDescriptor {
            label: Some("fog volume fixture"),
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
    let depth = make_texture(
        16,
        TextureFormat::Depth32Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    );
    let target = make_texture(
        8,
        TextureFormat::R16Float,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let depth_view = depth.create_view(&default());
    let target_view = target.create_view(&default());
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    let bind = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipelines.interior.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(&depth_view),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
        });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 256 * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mesh_pipeline = pipelines.interior_mesh(
        device,
        &bevy::mesh::VertexBufferLayout {
            array_stride: 12,
            step_mode: VertexStepMode::Vertex,
            attributes: vec![VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
        },
        FrontFace::Cw,
    );
    let mesh_view = device.create_bind_group(
        None,
        &pipelines.interior_view_layout,
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
                binding: 7,
                resource: BindingResource::Sampler(&sampler),
            },
        ],
    );
    let transform = Mat4::from_cols(Vec4::X, Vec4::new(0., 0., -1., 0.), Vec4::Y, Vec4::W);
    let models = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("interior model transform fixture"),
        contents: &ShaderBuffer::from(transform).data.unwrap(),
        usage: BufferUsages::STORAGE,
    });
    let mesh_models = device.create_bind_group(
        None,
        &pipelines.interior_model_layout,
        &[BindGroupEntry {
            binding: 0,
            resource: models.as_entire_binding(),
        }],
    );
    for shared_mesh in [false, true] {
        for (receiver, first_vertex, expected) in
            [(1., 0, 0.), (4., 0, 2.), (8., 0, 4.), (4., 6, 4.)]
        {
            let clip = projection * Vec4::new(0., 0., -receiver, 1.);
            let mut encoder = device.create_command_encoder(&default());
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
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::NONE.into()),
                            store: StoreOp::Store,
                        },
                    })],
                    ..default()
                });
                if shared_mesh {
                    pass.set_pipeline(&mesh_pipeline);
                    pass.set_bind_group(0, &mesh_view, &[]);
                    pass.set_bind_group(1, &mesh_models, &[]);
                } else {
                    pass.set_pipeline(&pipelines.interior);
                    pass.set_bind_group(0, &bind, &[]);
                }
                pass.set_vertex_buffer(0, *vertices.slice(..));
                pass.draw(first_vertex..12, 0..1);
            }
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &target,
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
                target.size(),
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
            for y in 0..8 {
                for x in 0..8 {
                    let offset = y * 256 + x * 2;
                    let value = half::f16::from_bits(u16::from_le_bytes(
                        bytes[offset..offset + 2].try_into().unwrap(),
                    ))
                    .to_f32();
                    assert!((value - expected).abs() < 0.01, "volume receiver={receiver} first={first_vertex} pixel={x}/{y}: {value} != {expected}");
                }
            }
            drop(bytes);
            staging.unmap();
        }
    }
}

pub(super) fn check_rgba16_pixels(
    device: &RenderDevice,
    queue: &RenderQueue,
    mut encoder: CommandEncoder,
    texture: &Texture,
    staging: &Buffer,
    group: &[Vec<&str>],
    first_channel: usize,
    description: &str,
) {
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture,
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
        texture.size(),
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
        for channel in 0..4 {
            let offset = (pixel / 8) * 256 + (pixel % 8) * 8 + channel * 2;
            let actual = half::f16::from_bits(u16::from_le_bytes(
                bytes[offset..offset + 2].try_into().unwrap(),
            ))
            .to_f32();
            let expected =
                f32::from_bits(u32::from_str_radix(row[first_channel + channel], 16).unwrap());
            let expected = half::f16::from_f32(expected).to_f32();
            assert!(
                (actual - expected).abs() <= expected.abs() * 0.0015 + 0.00005,
                "atmosphere {description} pixel={pixel} field={channel}: {actual} != {expected}"
            );
        }
    }
    drop(bytes);
    staging.unmap();
}
