//! Actual indexed raster submission against cached native depth/alpha stages.
use super::*;
use bevy::{
    prelude::*,
    render::{renderer::RenderQueue, RenderApp},
};

#[test]
#[ignore = "requires graphics adapter; numeric placed-water depth/stencil/motion readback"]
fn placed_water_depth_matches_cached_stages() {
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
    let positions = [
        [-1.0f32, -1., 0.2],
        [-1., 1., -0.1],
        [1., -1., 0.1],
        [1., 1., 0.3],
        [-0.6, -0.3, 0.5],
        [0.5, -0.2, 0.1],
        [0.1, 0.8, -0.2],
    ];
    let vertex_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        usage: BufferUsages::VERTEX,
        contents: &positions
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
    });
    let indices = [0u16, 1, 2, 2, 1, 3, 4, 5, 6];
    let index_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        usage: BufferUsages::INDEX,
        contents: &indices
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    });
    let vertex_layout = bevy::mesh::VertexBufferLayout {
        array_stride: 12,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    };
    let base = [
        0x01e008a9, 0x01e00802, 0x01e00802, 0x01e00802, 0xe00e0000, 0x30ffff1e, 0, 0, 0,
    ];
    let programs = [false, true].map(|model| {
        let state = if model {
            crate::render_state::water_depth_state()
        } else {
            crate::render_state::world_depth_state(base, 0)
        };
        [false, true].map(|alpha| {
            WaterDepthProgram::new(
                device,
                TextureFormat::Depth32FloatStencil8,
                state,
                alpha,
                false,
            )
            .unwrap()
        })
    });
    let pipelines = programs
        .each_ref()
        .map(|v| v.each_ref().map(|p| p.pipeline(device, &vertex_layout)));
    let stride = u64::from(device.limits().min_uniform_buffer_offset_alignment);
    let offset = 144u64.next_multiple_of(stride);
    let uniform = device.create_buffer(&BufferDescriptor {
        label: None,
        size: offset + 144,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let parameters_binding = programs
        .each_ref()
        .map(|v| v.each_ref().map(|p| p.parameter_binding(device, &uniform)));
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
    let mut lines = include_str!("../../../../tests/fixtures/water_depth_raster.txt").lines();
    let mut cases = 0;
    while let Some(header) = lines.next() {
        let header = header.split_whitespace().collect::<Vec<_>>();
        assert_eq!(header.len(), 40);
        let model = header[1].parse::<usize>().unwrap();
        let mode = header[2].parse::<usize>().unwrap();
        let words = header[4..]
            .iter()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let floats = words.iter().map(|v| f32::from_bits(*v)).collect::<Vec<_>>();
        let parameters = WaterDepthParameters {
            clip_from_local: std::array::from_fn(|i| floats[i * 4..i * 4 + 4].try_into().unwrap()),
            previous_clip_from_local: std::array::from_fn(|i| {
                floats[16 + i * 4..20 + i * 4].try_into().unwrap()
            }),
            model_depth_hack: floats[32],
            jitter_relative_to_previous: [floats[33], floats[34]],
        };
        let bytes = parameters.uniform_bytes();
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
        programs[model][mode]
            .record(
                &mut encoder,
                &depth_view,
                (mode == 0).then_some(&motion_view),
                [1., 2., 30., 20.],
                [3, 4, 26, 17],
                if model == 1 { 4 } else { 0 },
                None,
                &[WaterDepthDraw {
                    pipeline: &pipelines[model][mode],
                    parameters: &parameters_binding[model][mode],
                    parameter_offset: offset as u32,
                    positions: &vertex_buffer,
                    vertices: 0..7,
                    instances: 0..1,
                    indices: Some((&index_buffer, IndexFormat::Uint16, 0..9, 0)),
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
        let mut depth_pixels = 0;
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
            depth_pixels += usize::from(z > 0.);
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
        eprintln!("placed water model{model} raster mode{mode} case{cases}: stencil marked={covered}, depth pixels={depth_pixels}, max depth={max_depth}, motion={max_motion}");
        assert!(depth_pixels > 50);
        if model == 1 {
            assert!(covered > 50);
        }
        drop(bytes);
        staging.unmap();
        cases += 1;
    }
    assert_eq!(cases, 12);
}
