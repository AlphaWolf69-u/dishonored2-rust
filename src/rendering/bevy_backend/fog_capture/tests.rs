use super::*;
use bevy::render::renderer::RenderQueue;

#[test]
#[ignore = "requires graphics adapter; exact numeric capture of live GPU input layouts"]
fn fog_capture_preserves_all_depth_layers_and_parameter_bits() {
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
    let world = app.sub_app(RenderApp).world();
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    let uploader = super::super::test_depth::DepthUploader::new(device);
    // Cross the compute dispatch's 16384-word row, and retain all five layers.
    let full = UVec2::new(139, 127);
    let shape = Extent3d {
        width: 37,
        height: 29,
        depth_or_array_layers: 5,
    };
    let shadow_shape = UVec2::new(31, 23);
    let depth: Vec<f32> = (0..full.element_product())
        .map(|i| (i % 1024) as f32 / 1024.)
        .collect();
    let atlas: Vec<f32> = (0..shape.width * shape.height * shape.depth_or_array_layers)
        .map(|i| ((i * 17) % 8192) as f32 / 8192.)
        .collect();
    let depths = uploader.upload(device, queue, full.x, full.y, 1, depth.clone());
    let atlas_texture = uploader.upload(
        device,
        queue,
        shape.width,
        shape.height,
        shape.depth_or_array_layers,
        atlas.clone(),
    );
    let shadow: Vec<f32> = (0..shadow_shape.element_product())
        .map(|i| (i % 257) as f32 / 256.)
        .collect();
    let shadow_texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: None,
            size: Extent3d {
                width: shadow_shape.x,
                height: shadow_shape.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R32Float,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &shadow
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let parameters = [
        0, 1, 0x80000000, 0x7fc01234, 0xffffffff, 0x3f800000, 2048, 5,
    ];
    let parallel = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        usage: BufferUsages::STORAGE,
        contents: &parameters
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    });
    let mut encoder = device.create_command_encoder(&default());
    let (buffer, counts) = copy_sources(
        device,
        &mut encoder,
        full,
        shadow_shape,
        shape,
        &depths.create_view(&default()),
        &atlas_texture.create_view(&default()),
        &shadow_texture.create_view(&default()),
        &parallel,
    )
    .unwrap();
    assert_eq!(counts, vec![17653, 5365, 713, 8]);
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let read = buffer.slice(..).get_mapped_range();
    let actual: Vec<u32> = read
        .chunks_exact(4)
        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    let expected: Vec<u32> = depth
        .into_iter()
        .chain(atlas)
        .chain(shadow)
        .map(f32::to_bits)
        .chain(parameters)
        .collect();
    assert_eq!(actual, expected);
    drop(read);
    buffer.unmap();
}
