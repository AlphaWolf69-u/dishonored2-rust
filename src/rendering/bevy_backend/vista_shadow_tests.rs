use super::*;
use bevy::prelude::*;
use bevy::render::{render_resource::*, renderer::RenderQueue, RenderApp};

#[test]
#[ignore = "requires GPU; checks actual Vista raw-UV/wrap cutout raster and persistent depth"]
fn world_vista_cutout_raster_uses_raw_uv_sampler() {
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
    let adapter = world.resource::<RenderAdapter>();
    let queue = world.resource::<RenderQueue>();
    let layout = RawVertexBufferLayout {
        array_stride: 20,
        step_mode: VertexStepMode::Vertex,
        attributes: &[
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
        ],
    };
    let pipeline = ShadowDepthPipeline::new_world_vista_with_raster_state(
        device,
        adapter,
        &layout,
        DepthBiasState::default(),
        TextureFormat::Depth16Unorm,
        FrontFace::Ccw,
        None,
    )
    .unwrap();
    let floats = |values: &[f32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let buffer = |label, bytes: &[u8], usage| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
    };
    // Original local POSITION at conventional depth .25, as retained by the
    // live shadow attribute; UV is one wrap beyond [0,1].
    let vertices = buffer(
        "Vista native source vertices",
        &floats(&[
            -1., -1., 0.25, 1., 2., -1., 3., 0.25, 1., 0., 3., -1., 0.25, 3., 2.,
        ]),
        BufferUsages::VERTEX,
    );
    let triangles = buffer(
        "Vista triangle",
        &[0u32, 1, 2]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        BufferUsages::INDEX,
    );
    let ids = buffer(
        "Vista real low24 remap",
        &0xff00_0000u32.to_le_bytes(),
        BufferUsages::STORAGE,
    );
    let matrices = buffer(
        "Vista original 48-byte affine rows",
        &floats(&[1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.]),
        BufferUsages::STORAGE,
    );
    let identity = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    // Deliberately zero Layer0: the ordinary cutout program would sample the
    // same texel everywhere. Vista must not perform that material MAD.
    let cb = crate::shadow_pass::world_alpha_shadow_view_cb(&identity, 0, [0.; 4]);
    let uniform = buffer("Vista bridge state", &cb, BufferUsages::UNIFORM);
    let model = pipeline
        .bind_prepared_model_state(device, &uniform)
        .unwrap();
    let diffuse = device.create_texture(&TextureDescriptor {
        label: Some("Vista raw alpha checker"),
        size: Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let texels: Vec<u8> = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| [255, 255, 255, if (x + y) % 2 == 0 { 0 } else { 255 }])
        })
        .collect();
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &diffuse,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &texels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        diffuse.size(),
    );
    let sampler = shadow_vista_sampler(device);
    let resources = pipeline
        .bind_world_alpha_instances(
            device,
            &ids,
            &matrices,
            &diffuse.create_view(&TextureViewDescriptor::default()),
            &sampler,
        )
        .unwrap();
    // From the installed anisotropicsampler declaration, NOT capture defaults.
    let canonical = [0x2a, 0, 0, 0, 0, 0, 0, 13f32.to_bits(), 4f32.to_bits()];
    for global in [1., 4., 16.] {
        let desc = crate::render_state::sampler_descriptor(&canonical, global);
        assert_eq!(&desc[..4], &[0x55, 1, 1, 1]);
        assert_eq!(desc[5], 4);
        assert_eq!(&desc[11..], &[0, 13f32.to_bits()]);
    }
    let target = ShadowDepthArray::new_with_format(
        device,
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        TextureFormat::Depth16Unorm,
        "Vista cutout depth",
    )
    .unwrap();
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("Vista D16 readback"),
        size: 1024,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    pipeline
        .draw_layer(
            &mut encoder,
            &target,
            0,
            Operations {
                load: LoadOp::Clear(1.0),
                store: StoreOp::Store,
            },
            None,
            &[ShadowDepthDraw {
                model_state_offset: 0,
                model_state: &model,
                program_resources: Some(&resources),
                vertex_buffer: &vertices,
                index_buffer: &triangles,
                index_format: IndexFormat::Uint32,
                indices: 0..3,
                base_vertex: 0,
                instances: 0..1,
                scissor: ShadowScissor {
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 4,
                },
            }],
        )
        .unwrap();
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::DepthOnly,
        },
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
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |result| sender.send(result).unwrap());
    device
        .wgpu_device()
        .poll(PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    for y in 0..4 {
        for x in 0..4 {
            let offset = y * 256 + x * 2;
            let depth = u16::from_le_bytes(mapped[offset..offset + 2].try_into().unwrap());
            assert_eq!(
                depth,
                if (x + y) % 2 == 0 { u16::MAX } else { 16384 },
                "Vista pixel {x},{y}"
            );
        }
    }
    drop(mapped);
    readback.unmap();
}
