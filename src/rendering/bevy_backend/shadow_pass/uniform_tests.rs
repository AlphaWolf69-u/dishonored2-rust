use super::*;

pub(super) fn world_instance(offset: u32, count: u32) -> ShadowInstance {
    ShadowInstance {
        mesh: Handle::default(),
        per_instance_cb: ShadowInstanceConstants::World {
            constants: world_shadow_view_cb(
                &[
                    [1., 0., 0., 0.],
                    [0., 1., 0., 0.],
                    [0., 0., 1., 0.],
                    [0., 0., 0., 1.],
                ],
                offset,
            ),
            visible_indices: Handle::default(),
            matrices: Handle::default(),
            instance_count: count,
        },
        program: ShadowProgram::WorldOpaque,
        alpha_material: None,
        front_face: FrontFace::Ccw,
        cull_mode: None,
        draw_state: [0; 9],
        stencil_reference: 0,
        scissor: ShadowScissor {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        },
    }
}

#[test]
fn compact_world_uniforms_preserve_packet_ranges_and_raw_constants() {
    let mut bytes = Vec::new();
    let mut slots = HashMap::new();
    let first = world_instance(5, 3);
    let second = world_instance(8, 2);
    let a = pack_shadow_uniform(&first, 256, true, &mut bytes, &mut slots);
    let b = pack_shadow_uniform(&second, 256, true, &mut bytes, &mut slots);
    assert_eq!((a.slot, b.slot, bytes.len()), (0, 0, 256));
    assert_eq!((a.instances, b.instances), (5..8, 8..10));
    let mut expected = first.per_instance_cb.as_bytes().to_vec();
    expected[64..68].fill(0);
    assert_eq!(&bytes[..96], &expected);
    let mut other = second.clone();
    if let ShadowInstanceConstants::World { constants, .. } = &mut other.per_instance_cb {
        constants[80..84].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        constants[84..88].copy_from_slice(&0x7fc0_0123u32.to_le_bytes());
    }
    let c = pack_shadow_uniform(&other, 256, true, &mut bytes, &mut slots);
    assert_eq!(c.slot, 1, "signed zero and NaN payloads remain distinct");
    assert_eq!(
        &bytes[256 + 80..256 + 88],
        &other.per_instance_cb.as_bytes()[80..88]
    );
    let overflow = world_instance(u32::MAX - 1, 3);
    let d = pack_shadow_uniform(&overflow, 256, true, &mut bytes, &mut slots);
    assert_eq!(d.instances, 0..3);
    assert_eq!(
        &bytes[d.slot * 256..d.slot * 256 + 96],
        overflow.per_instance_cb.as_bytes()
    );
    let e = pack_shadow_uniform(&first, 256, false, &mut bytes, &mut slots);
    assert_eq!(e.instances, 0..3);
    assert_eq!(
        &bytes[e.slot * 256..e.slot * 256 + 96],
        first.per_instance_cb.as_bytes()
    );
    // Wind has a second independent offset; this optimization leaves its ABI alone.
    let mut wind = first.clone();
    wind.program = ShadowProgram::WorldVegetation;
    wind.per_instance_cb = ShadowInstanceConstants::WorldVegetation {
        constants: [0x55; 208],
        visible_indices: Handle::default(),
        matrices: Handle::default(),
        winds: None,
        instance_count: 2,
    };
    let f = pack_shadow_uniform(&wind, 256, true, &mut bytes, &mut slots);
    assert_eq!(f.instances, 0..2);
    assert_eq!(&bytes[f.slot * 256..f.slot * 256 + 208], &[0x55; 208]);
}

#[test]
#[ignore = "requires GPU; exact depth comparison for packet offset versus first_instance"]
fn compact_world_uniforms_preserve_instanced_shadow_depth_gpu() {
    use bevy::prelude::*;
    use bevy::render::{render_resource::*, RenderApp};
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
    let queue = world.resource::<RenderQueue>();
    let adapter = world.resource::<RenderAdapter>();
    let make_buffer = |label, bytes: &[u8], usage| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage,
        })
    };
    let words = |values: &[u32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let floats = |values: &[f32]| {
        values
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>()
    };
    let vertices = make_buffer(
        "shadow parity triangle",
        &floats(&[-0.14, -0.3, 0., 0.14, -0.3, 0., 0., 0.3, 0.]),
        BufferUsages::VERTEX,
    );
    let indices = make_buffer("triangle indices", &words(&[0, 1, 2]), BufferUsages::INDEX);
    let remap = make_buffer(
        "nonzero offset and packed flags",
        &words(&[0, 0, 0, 0, 0, 0xff00_0002, 0xa500_0000, 0x9300_0001]),
        BufferUsages::STORAGE,
    );
    let matrices = make_buffer(
        "three distinct matrix instances",
        &floats(&[
            1., 0., 0., -0.5, 0., 1., 0., 0., 0., 0., 1., 0.2, 1., 0., 0., 0.0, 0., 1., 0., 0., 0.,
            0., 1., 0.4, 1., 0., 0., 0.5, 0., 1., 0., 0., 0., 0., 1., 0.6,
        ]),
        BufferUsages::STORAGE,
    );
    let owner = world_instance(5, 3);
    let mut packed = Vec::new();
    let metadata = pack_shadow_uniform(&owner, 256, true, &mut packed, &mut HashMap::new());
    assert_eq!(metadata.instances, 5..8);
    let normal = make_buffer(
        "original CB offset",
        owner.per_instance_cb.as_bytes(),
        BufferUsages::UNIFORM,
    );
    let compact = make_buffer("shared zero-offset CB", &packed, BufferUsages::UNIFORM);
    let pipeline = ShadowDepthPipeline::new_world_with_raster_state(
        device,
        adapter,
        &RawVertexBufferLayout {
            array_stride: 12,
            step_mode: VertexStepMode::Vertex,
            attributes: &[VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            }],
        },
        DepthBiasState::default(),
        TextureFormat::Depth16Unorm,
        FrontFace::Ccw,
        None,
    )
    .unwrap();
    let resources = pipeline
        .bind_world_instances(device, &remap, &matrices)
        .unwrap();
    let second_owner = world_instance(6, 2);
    let second_normal = make_buffer(
        "second original CB",
        second_owner.per_instance_cb.as_bytes(),
        BufferUsages::UNIFORM,
    );
    let states = [
        pipeline.bind_prepared_model_state(device, &normal).unwrap(),
        pipeline
            .bind_prepared_model_state(device, &second_normal)
            .unwrap(),
        pipeline
            .bind_prepared_model_state(device, &compact)
            .unwrap(),
    ];
    let depth = ShadowDepthArray::new_with_format(
        device,
        Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 2,
        },
        TextureFormat::Depth16Unorm,
        "offset transport parity layers",
    )
    .unwrap();
    let readback = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 16384,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    for layer in 0..2 {
        let draw = ShadowDepthDraw {
            model_state_offset: 0,
            model_state: &states[if layer == 0 { 0 } else { 2 }],
            program_resources: Some(&resources),
            vertex_buffer: &vertices,
            index_buffer: &indices,
            index_format: IndexFormat::Uint32,
            indices: 0..3,
            base_vertex: 0,
            instances: if layer == 0 { 0..1 } else { 5..6 },
            scissor: owner.scissor,
        };
        let second_draw = ShadowDepthDraw {
            model_state: &states[if layer == 0 { 1 } else { 2 }],
            instances: if layer == 0 { 0..2 } else { 6..8 },
            indices: draw.indices.clone(),
            ..draw
        };
        ShadowDepthPipeline::draw_layer_sequence(
            &mut encoder,
            &depth,
            layer,
            Operations {
                load: LoadOp::Clear(1.),
                store: StoreOp::Store,
            },
            None,
            &[(&pipeline, &draw), (&pipeline, &second_draw)],
        )
        .unwrap();
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &depth.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: TextureAspect::DepthOnly,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: u64::from(layer) * 8192,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(32),
                },
            },
            Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let data = readback.slice(..).get_mapped_range();
    let mut covered = 0;
    for y in 0..32 {
        let row = &data[y * 256..y * 256 + 64];
        assert_eq!(row, &data[8192 + y * 256..8192 + y * 256 + 64]);
        covered += row.chunks_exact(2).filter(|v| *v != [255, 255]).count();
    }
    assert!(
        covered > 24,
        "test must draw all three nonzero-offset instances"
    );
    drop(data);
    readback.unmap();
}
