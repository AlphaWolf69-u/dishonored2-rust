//! Host recording chunks for direct camera mesh phases. Clear only the first
//! pass; every subsequent chunk loads the exact prior color/depth/stencil data.
use super::{BinnedPhaseItem, BinnedRenderPhase, DrawError};
use crate::{
    render_resource::{
        LoadOp, RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    },
    renderer::RenderContext,
};
use bevy_camera::Viewport;
use bevy_ecs::{entity::Entity, prelude::Resource, world::World};
use core::sync::atomic::{AtomicU64, Ordering};

/// Explicit opt-in for direct camera passes with self-contained mesh commands.
/// Multisampling, pass queries, skybox and indirect paths use ordinary passes.
#[derive(Resource)]
pub struct CameraDrawChunking {
    batch_size: usize,
    chunks: AtomicU64,
}
impl CameraDrawChunking {
    pub fn new(batch_size: usize) -> Self {
        Self {
            batch_size: batch_size.max(1),
            chunks: AtomicU64::new(0),
        }
    }
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }
    pub fn recorded_chunks(&self) -> u64 {
        self.chunks.load(Ordering::Relaxed)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one render pass's attachments, view and phase"
)]
pub fn render_binned_phase_in_chunks<BPI: BinnedPhaseItem>(
    phase: &BinnedRenderPhase<BPI>,
    ctx: &mut RenderContext,
    world: &World,
    view: Entity,
    label: &str,
    colors: &mut [Option<RenderPassColorAttachment<'_>>],
    depth: &mut Option<RenderPassDepthStencilAttachment<'_>>,
    viewport: Option<&Viewport>,
    chunk_size: usize,
    stencil_reference: u32,
) -> Result<(), DrawError> {
    let count = phase.direct_batch_count().expect("direct camera phase");
    let chunk_size = chunk_size.max(1);
    for start in (0..count.max(1)).step_by(chunk_size) {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some(label),
            color_attachments: colors,
            depth_stencil_attachment: depth.clone(),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(viewport) = viewport {
            pass.set_camera_viewport(viewport);
        }
        if stencil_reference != 0 {
            pass.set_stencil_reference(stencil_reference);
        }
        let result = if phase.is_empty() {
            Ok(())
        } else if count == 0 {
            phase.render(&mut pass, world, view)
        } else {
            phase.render_direct_batch_range(
                &mut pass,
                world,
                view,
                start..(start + chunk_size).min(count),
            )
        };
        drop(pass);
        ctx.flush_encoder();
        if let Some(settings) = world.get_resource::<CameraDrawChunking>() {
            if settings.chunks.fetch_add(1, Ordering::Relaxed) == 0 {
                bevy_log::debug!(
                    "Camera mesh pass chunking active: batch_groups={}",
                    settings.batch_size
                );
            }
        }
        for color in colors.iter_mut().flatten() {
            color.ops.load = LoadOp::Load;
        }
        if let Some(depth) = depth {
            if let Some(ops) = &mut depth.depth_ops {
                ops.load = LoadOp::Load;
            }
            if let Some(ops) = &mut depth.stencil_ops {
                ops.load = LoadOp::Load;
            }
        }
        result?;
    }
    Ok(())
}
