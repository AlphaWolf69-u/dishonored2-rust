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
    if count > chunk_size && !phase.is_empty() {
        if let Some(result) = render_chunks_in_parallel(
            phase, ctx, world, view, label, colors, depth, viewport, count, chunk_size,
            stencil_reference,
        ) {
            return result;
        }
    }
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

/// DH2 patch: record the batch chunks of one pass concurrently, each into its
/// own encoder with its own draw-function copy, and add the encoders in chunk
/// order, so the GPU receives the same passes and draws in the same order as
/// the serial loop. Unbatchable meshes and non-meshes, which use the main
/// registry, follow in one more pass that loads the prior attachments.
/// Returns None (serial recording) when the draw functions have no complete
/// worker copies or DH2_SERIAL_PASS_RECORDING=1.
#[expect(clippy::too_many_arguments, reason = "one render pass's attachments, view and phase")]
fn render_chunks_in_parallel<BPI: BinnedPhaseItem>(
    phase: &BinnedRenderPhase<BPI>,
    ctx: &mut RenderContext,
    world: &World,
    view: Entity,
    label: &str,
    colors: &mut [Option<RenderPassColorAttachment<'_>>],
    depth: &mut Option<RenderPassDepthStencilAttachment<'_>>,
    viewport: Option<&Viewport>,
    count: usize,
    chunk_size: usize,
    stencil_reference: u32,
) -> Option<Result<(), DrawError>> {
    static SERIAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *SERIAL.get_or_init(|| std::env::var("DH2_SERIAL_PASS_RECORDING").as_deref() == Ok("1")) {
        return None;
    }
    let workers = world.resource::<super::DrawFunctions<BPI>>().complete_workers()?;
    let chunks: Vec<core::ops::Range<usize>> = (0..count)
        .step_by(chunk_size)
        .map(|start| start..(start + chunk_size).min(count))
        .collect();
    if chunks.len() > workers.len() {
        return None;
    }
    // Each worker set is prepared once for this pass by its task.
    let device = ctx.render_device().clone();
    let pool = bevy_tasks::ComputeTaskPool::get();
    let first_colors: Vec<Option<RenderPassColorAttachment<'_>>> = colors.to_vec();
    let first_depth = depth.clone();
    let mut loaded_colors = first_colors.clone();
    for color in loaded_colors.iter_mut().flatten() {
        color.ops.load = LoadOp::Load;
    }
    let mut loaded_depth = first_depth.clone();
    if let Some(depth) = &mut loaded_depth {
        if let Some(ops) = &mut depth.depth_ops {
            ops.load = LoadOp::Load;
        }
        if let Some(ops) = &mut depth.stencil_ops {
            ops.load = LoadOp::Load;
        }
    }
    let results: Vec<(usize, wgpu::CommandEncoder, Result<(), DrawError>)> = pool.scope(|scope| {
        for (index, (range, worker)) in chunks.iter().zip(workers).enumerate() {
            let (colors, depth) = if index == 0 {
                (&first_colors, &first_depth)
            } else {
                (&loaded_colors, &loaded_depth)
            };
            let device = &device;
            scope.spawn(async move {
                let mut functions = worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                functions.prepare(world);
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some(label),
                });
                let result = {
                    let render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                        label: Some(label),
                        color_attachments: colors,
                        depth_stencil_attachment: depth.clone(),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let mut pass = crate::render_phase::TrackedRenderPass::new(device, render_pass);
                    if let Some(viewport) = viewport {
                        pass.set_camera_viewport(viewport);
                    }
                    if stencil_reference != 0 {
                        pass.set_stencil_reference(stencil_reference);
                    }
                    phase.render_direct_batches_with(&mut pass, world, view, range.clone(), &mut functions)
                };
                (index, encoder, result)
            });
        }
    });
    let mut results = results;
    results.sort_by_key(|(index, _, _)| *index);
    let mut first_error = None;
    for (_, encoder, result) in results {
        ctx.add_command_encoder(encoder);
        if let Err(error) = result {
            first_error.get_or_insert(error);
        }
    }
    if let Some(settings) = world.get_resource::<CameraDrawChunking>() {
        settings.chunks.fetch_add(chunks.len() as u64, Ordering::Relaxed);
    }
    // Unbatchable meshes and non-meshes, after every batch, as the serial
    // loop's last chunk drew them, with the main registry prepared as the
    // serial loop's first chunk prepared it.
    world.resource::<super::DrawFunctions<BPI>>().write().prepare(world);
    for (target, loaded) in colors.iter_mut().zip(loaded_colors) {
        *target = loaded;
    }
    *depth = loaded_depth;
    let tail = {
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
        phase.render_direct_batch_range(&mut pass, world, view, count..count)
    };
    ctx.flush_encoder();
    Some(first_error.map_or(tail, Err))
}

