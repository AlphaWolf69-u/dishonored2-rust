use super::*;
#[test]
#[ignore = "requires GPU; checks exact F8 display input bytes and row alignment"]
fn display_capture_preserves_texture_rows_and_exposure() {
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
    let queue = world.resource::<bevy::render::renderer::RenderQueue>();
    let mut textures = Vec::new();
    let mut payloads = Vec::new();
    for (index, [width, height]) in [[17, 3], [4, 2], [1, 1]].into_iter().enumerate() {
        let payload: Vec<u8> = (0..width * height * 4)
            .flat_map(|i| ((i + index as u32 * 41) as u16).to_le_bytes())
            .collect();
        textures.push(device.create_texture_with_data(
            queue,
            &TextureDescriptor {
                label: None,
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba16Float,
                usage: TextureUsages::COPY_SRC,
                view_formats: &[],
            },
            TextureDataOrder::MipMajor,
            &payload,
        ));
        payloads.push(payload);
    }
    let exposure_bytes: Vec<u8> = [0.25f32, 0.5, 1., 2., 3., 4., 5., 6.]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    let exposure = device.create_buffer_with_data(&BufferInitDescriptor {
        label: None,
        contents: &exposure_bytes,
        usage: BufferUsages::COPY_SRC,
    });
    let mut encoder = device.create_command_encoder(&default());
    let (staging, metadata) = copy_sources(
        device,
        &mut encoder,
        &[
            ("hdr", &textures[0]),
            ("luminance", &textures[1]),
            ("bloom", &textures[2]),
        ],
        &exposure,
    )
    .unwrap();
    assert!(metadata.contains("plane=hdr offset=0 row_bytes=256"));
    assert!(metadata.contains("plane=luminance offset=768 row_bytes=256"));
    assert!(metadata.contains("plane=bloom offset=1280 row_bytes=256"));
    assert!(metadata.contains("exposure_offset=1536 total_bytes=1568"));
    let submission = queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging.slice(..).map_async(MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = staging.slice(..).get_mapped_range();
    for (i, offset) in [0usize, 768, 1280].into_iter().enumerate() {
        let row = textures[i].width() as usize * 8;
        for y in 0..textures[i].height() as usize {
            assert_eq!(
                &bytes[offset + y * 256..offset + y * 256 + row],
                &payloads[i][y * row..(y + 1) * row]
            );
        }
    }
    assert_eq!(&bytes[1536..1568], exposure_bytes.as_slice());
    drop(bytes);
    staging.unmap();
}
