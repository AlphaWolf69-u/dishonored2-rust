//! Windowless GPU check of the occlusion query pass:
//! `cargo run --features game --example occlusion_query_check`.
use dishonored2_rust::rendering::bevy_backend::occlusion_query_pass::OcclusionQueryPass;
use dishonored2_rust::rendering::occlusion_query::OcclusionQueries;
use bevy::{prelude::*, render::{render_resource::*, renderer::{RenderDevice, RenderQueue}, RenderApp}};

/// The view, projection and query are identities with an NDC radius of
/// 0.25 at depth 0.5, so the quad covers pixels 24..40 on both axes of the
/// 64 x 64 target: 256 samples where the stored depth is at most 0.5
/// (reversed-Z GEQUAL), none where it is larger.
fn counted(device: &RenderDevice, queue: &RenderQueue, stored: f32) -> u64 {
    let format = TextureFormat::Depth24PlusStencil8;
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("occlusion query depth fixture"),
        size: Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth = texture.create_view(&TextureViewDescriptor::default());
    let identity: dishonored2_rust::rendering::render_shadow::ShadowMatrix = std::array::from_fn(|r| std::array::from_fn(|c| if r == c { 1. } else { 0. }));
    let matrices = dishonored2_rust::rendering::view_history::ViewMatrices {
        position: [0.; 3],
        projection: identity,
        view: identity,
        view_projection: identity,
    };
    let mut manager = OcclusionQueries::default();
    let slot = manager.allocate().unwrap();
    manager.request(slot, [0., 0., 0.5], 0.25, 0., false, false);
    let mut pass = OcclusionQueryPass::new(device, format);
    // Frame 1 issues the query; frame 2 maps its copy; frame 3 reads it.
    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        if frame == 0 {
            encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(Operations { load: LoadOp::Clear(stored), store: StoreOp::Store }),
                    stencil_ops: Some(Operations { load: LoadOp::Clear(0), store: StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        pass.record(device, queue, &mut encoder, &depth, &matrices, [64, 64], &mut manager);
        queue.submit([encoder.finish()]);
        device
            .poll(PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(30)) })
            .unwrap();
    }
    manager.count(slot)
}

fn main() {
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
    assert_eq!(counted(device, queue, 0.25), 256);
    assert_eq!(counted(device, queue, 0.75), 0);
    println!("occlusion query pass: 256 visible / 0 occluded samples as expected");
}
