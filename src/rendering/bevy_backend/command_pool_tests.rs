//! Synthetic command-pool lifetime regression; no game assets or window.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        RenderApp,
    },
};
fn private_bytes() -> usize {
    crate::platform::process_memory::sample()
        .expect("current process memory counters")
        .private_bytes
}
#[test]
#[ignore = "requires a DX12 GPU; checks heterogeneous command batches retain bounded driver memory"]
fn heterogeneous_command_batches_reuse_completed_lists_gpu() {
    assert_eq!(
        std::env::var("WGPU_BACKEND").as_deref(),
        Ok("dx12"),
        "run this driver-memory regression with WGPU_BACKEND=dx12"
    );
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
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("command pool depth oracle"),
        size: Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&default());
    let initial_private_bytes = private_bytes();
    for round in 0..80 {
        let mut encoders = Vec::new();
        for index in 0..80 {
            let mut encoder = device.create_command_encoder(&default());
            let passes = if index == round % 80 { 256 } else { 1 };
            for _ in 0..passes {
                let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("command pool depth clear"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(0.25),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..default()
                });
            }
            encoders.push(encoder);
        }
        queue.submit(encoders.into_iter().map(|encoder| encoder.finish()));
        device.poll(PollType::wait_indefinitely()).unwrap();

        let growth = private_bytes().saturating_sub(initial_private_bytes);
        // Per-encoder allocator storage exceeds this bound even with shared lists.
        // This is a regression assertion, never a production resource cap.
        assert!(
            growth < 1536 * 1024 * 1024,
            "command-pool memory grew by {growth} bytes at round {round}"
        );
    }
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 1024,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::DepthOnly,
        },
        TexelCopyBufferInfo {
            buffer: &staging,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
    device.poll(PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = staging.slice(..).get_mapped_range();
    for row in 0..4 {
        for column in 0..4 {
            let offset = row * 256 + column * 4;
            assert_eq!(
                f32::from_le_bytes(mapped[offset..offset + 4].try_into().unwrap()),
                0.25
            );
        }
    }
    drop(mapped);
    staging.unmap();
}
