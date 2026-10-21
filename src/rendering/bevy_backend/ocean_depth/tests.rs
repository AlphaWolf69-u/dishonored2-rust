//! Actual indexed raster submission against cached native depth/alpha stages.
use super::*;
use bevy::{
    prelude::*,
    render::{renderer::RenderQueue, RenderApp},
};

#[test]
#[ignore = "requires graphics adapter; numeric ocean depth/stencil/motion readback"]
fn ocean_depth_matches_cached_stages() {
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
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let noise = super::super::noise_textures::NoiseTextures::new(device, queue);
    let upload = |format, bytes: &[u8]| {
        device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: Some("ocean depth fixture image"),
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
            bytes,
        )
    };
    let data = include_bytes!("../../../../tests/fixtures/ocean_surface_inputs.bin");
    let images = [
        upload(TextureFormat::R32Float, &data[..256]),
        upload(TextureFormat::Rgba16Float, &data[256..768]),
        upload(TextureFormat::Rgba16Float, &data[768..1280]),
        upload(TextureFormat::R16Float, &data[1280..1408]),
        upload(TextureFormat::R32Float, &data[1408..]),
        upload(
            TextureFormat::R32Float,
            include_bytes!("../../../../tests/fixtures/ocean_height_query_mask.bin"),
        ),
    ];
    let views = images.each_ref().map(|v| v.create_view(&default()));
    let sampler = |address| {
        device.create_sampler(&SamplerDescriptor {
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_max_clamp: 13.,
            ..default()
        })
    };
    let wrap = sampler(AddressMode::Repeat);
    let clamp = sampler(AddressMode::ClampToEdge);
    let mesh = OceanMeshBuffers::new(
        device,
        crate::rendering::ocean::OceanGeometry {
            vertices: vec![
                [-1., -1.],
                [-1., 1.],
                [1., -1.],
                [1., 1.],
                [-0.6, -0.3],
                [0.5, -0.2],
                [0.1, 0.8],
            ],
            indices: vec![0, 1, 2, 3, 4, 5, 6],
            records: vec![crate::rendering::ocean::OceanGeometryRecord {
                vertex_count: 7,
                strip_primitive_count: 2,
                strip_index_offset: 0,
                edge_vertex_count: 7,
                edge_triangle_count: 1,
                edge_index_offset: 4,
            }],
        },
    );
    assert_eq!(mesh.buffer_bytes(), 7 * 8 + 16);
    let base = [
        0x01e008a9, 0x01e00802, 0x01e00802, 0x01e00802, 0xe00e0000, 0x30ffff1e, 0, 0, 0,
    ];
    let (state, reference) = crate::rendering::ocean::ocean_draw_state(
        base,
        crate::rendering::ocean::OceanDrawInputs {
            depth_pass: true,
            eye_height: 0.,
            surface_height: 0.,
            height_extent: 1.,
            wireframe: false,
            mirrored: false,
        },
    );
    let programs = [false, true].map(|alpha| {
        OceanDepthProgram::new(
            device,
            TextureFormat::Depth32FloatStencil8,
            state,
            reference,
            alpha,
        )
        .unwrap()
    });
    let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment);
    let offset = 288u64.next_multiple_of(stride);
    let uniform = device.create_buffer(&BufferDescriptor {
        label: None,
        size: offset + 288,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let parameters = programs
        .each_ref()
        .map(|v| v.parameter_binding(device, &uniform));
    let textures = programs.each_ref().map(|v| {
        v.texture_binding(
            device,
            [
                noise.permutation(),
                noise.gradients(),
                &views[0],
                &views[1],
                &views[2],
                &views[3],
                &views[4],
                &views[5],
            ],
            [noise.point_wrap(), &wrap, &clamp],
        )
    });
    let target = |format| {
        device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 32,
                height: 24,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let depth = target(TextureFormat::Depth32FloatStencil8);
    let motion = target(TextureFormat::Rg16Float);
    let depth_view = depth.create_view(&default());
    let motion_view = motion.create_view(&default());
    let plane = 256 * 24u64;
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: plane * 3,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut lines = include_str!("../../../../tests/fixtures/ocean_depth.txt").lines();
    let mut cases = 0;
    while let Some(header) = lines.next() {
        let header = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(header.len(), 75);
        let mode = header[1].parse::<usize>().unwrap();
        let words = header[3..]
            .iter()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let floats = words.iter().map(|v| f32::from_bits(*v)).collect::<Vec<_>>();
        let vec4 = |start| floats[start..start + 4].try_into().unwrap();
        let surface = super::super::ocean_surface::OceanSurfaceParameters {
            spatial: vec4(0),
            clock_view: vec4(4),
            camera: vec4(8),
            perlin: vec4(12),
            perlin_foam: vec4(16),
            shore: vec4(20),
            foam: vec4(24),
        };
        let vertex = super::super::ocean_vertex::OceanVertexParameters {
            view: super::super::ocean_vertex::OceanVertexView {
                clip_from_world: floats[28..44].try_into().unwrap(),
                previous_clip_from_world: floats[44..60].try_into().unwrap(),
                view_row: vec4(60),
                jitter_relative_to_previous: [floats[69], floats[70]],
            },
            patch_transform: vec4(64),
            perlin_blend_end: floats[68],
        };
        let bytes = uniform_bytes(&surface, &vertex);
        assert_eq!(
            bytes.as_slice(),
            words
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        queue.write_buffer(&uniform, offset, &bytes);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &motion_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(
                            bevy::color::LinearRgba::new(0.03125, -0.0625, 0., 0.).into(),
                        ),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(0x80),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        programs[mode]
            .record(
                &mut encoder,
                &depth_view,
                (mode == 0).then_some(&motion_view),
                [1., 2., 30., 20.],
                [3, 4, 26, 17],
                &mesh,
                &textures[mode],
                &[OceanPatchDraw {
                    record: 0,
                    parameters: &parameters[mode],
                    parameter_offset: offset as u32,
                }],
            )
            .unwrap();
        for (i, image, aspect) in [
            (0, &depth, TextureAspect::DepthOnly),
            (1, &depth, TextureAspect::StencilOnly),
            (2, &motion, TextureAspect::All),
        ] {
            let mut source = image.as_image_copy();
            source.aspect = aspect;
            encoder.copy_texture_to_buffer(
                source,
                TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: TexelCopyBufferLayout {
                        offset: plane * i,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(24),
                    },
                },
                Extent3d {
                    width: 32,
                    height: 24,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let mut max_depth = 0f32;
        let mut max_motion = 0f32;
        let mut covered = 0;
        for pixel in 0..768 {
            let expected = lines
                .next()
                .unwrap()
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let row = pixel / 32;
            let x = pixel % 32;
            let z = f32::from_le_bytes(
                bytes[row * 256 + x * 4..row * 256 + x * 4 + 4]
                    .try_into()
                    .unwrap(),
            );
            let stencil = bytes[plane as usize + row * 256 + x];
            assert_eq!(
                u32::from(stencil),
                expected[1],
                "case{cases} pixel{pixel} coverage/stencil"
            );
            covered += usize::from(stencil != 0x80);
            let error = (z - f32::from_bits(expected[0])).abs();
            max_depth = max_depth.max(error);
            assert!(
                error < 3e-6,
                "case{cases} pixel{pixel} depth {z} != {}",
                f32::from_bits(expected[0])
            );
            for c in 0..2 {
                let address = plane as usize * 2 + row * 256 + x * 4 + c * 2;
                let actual_bits =
                    u16::from_le_bytes(bytes[address..address + 2].try_into().unwrap());
                let value = half::f16::from_bits(actual_bits).to_f32();
                let reference = half::f16::from_bits(expected[2 + c] as u16).to_f32();
                let error = (value - reference).abs();
                max_motion = max_motion.max(error);
                assert!(
                    value == reference || actual_bits.abs_diff(expected[2 + c] as u16) <= 1,
                    "case{cases} pixel{pixel} motion{c} {value} != {reference}"
                );
            }
        }
        eprintln!("ocean raster mode{mode} case{cases}: covered={covered}, max depth={max_depth}, motion={max_motion}");
        assert!(covered > 50);
        drop(bytes);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 6);
}
