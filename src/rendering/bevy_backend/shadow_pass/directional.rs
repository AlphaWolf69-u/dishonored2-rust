//! Independent directional slices retain their cache/draw/copy order.
use super::*;
use bevy::render::render_resource::CommandEncoder;

enum CacheUpdate {
    Unchanged,
    Written(Option<u64>),
}

pub(super) struct Publication {
    pub complete: bool,
    emitted: bool,
    cache: CacheUpdate,
}

impl Publication {
    pub fn publish(self, target: &mut CachedTarget, physical: usize, layer: usize) {
        if self.emitted {
            target.layer_valid[physical] = true;
        }
        if let CacheUpdate::Written(generation) = self.cache {
            target.cache_valid[layer - 1] = true;
            target.cache_generation[layer - 1] = generation;
        }
    }
}

pub(super) fn encode_layer(
    encoder: &mut CommandEncoder,
    target: &CachedTarget,
    physical_layer: usize,
    layer_index: usize,
    layer: &ShadowLayer,
    records: &[PreparedShadowRecord],
    pipelines: &ShadowPipelineStore,
    receiver: Option<&ParallelReceiverVars>,
    diagnostics: Option<&bevy::render::diagnostic::DiagnosticsRecorder>,
) -> Result<Publication, String> {
    // bevy_backend: 32db10 draws each slice into a temporary and transfers
    // the whole rectangle to its atlas layer (CopySubresourceRegion or the
    // depthCopy draw with compare ALWAYS). Drawing into the atlas layer leaves
    // the same depth there without the transfer or the temporaries.
    let (atlas, layer_in_atlas) = (&target.array, physical_layer as u32);
    let mut remaining = directional_layer_draws(records, pipelines, layer_index);
    if remaining.is_empty()
        && layer.cache == ShadowCacheOp::None
        && matches!(layer.ops.depth, ShadowLoad::Load)
        && matches!(layer.ops.stencil, StencilLoad::Load)
    {
        return Ok(Publication {
            emitted: false,
            cache: CacheUpdate::Unchanged,
            complete: target.layer_valid[physical_layer]
                || receiver.is_some_and(|receiver| {
                    !crate::render_shadow::parallel_shadow_layer_can_be_selected(
                        receiver.cascade_fars,
                        physical_layer,
                    )
                }),
        });
    }
    let load = ShadowLayerOps {
        depth: ShadowLoad::Load,
        stencil: StencilLoad::Load,
    };
    let mut publication = Publication {
        complete: true,
        emitted: true,
        cache: CacheUpdate::Unchanged,
    };
    let counted = remaining.len() as u64;
    match layer.cache {
        ShadowCacheOp::None => {
            draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, remaining, layer.ops)?;
        }
        ShadowCacheOp::Restore => {
            atlas.copy_layer_from(encoder, layer_in_atlas, &target.static_cache[layer_index - 1], 0)?;
            draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, remaining, load)?;
        }
        ShadowCacheOp::Store { static_instances } => {
            let dynamic = remaining.split_off(static_instances);
            draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, remaining, layer.ops)?;
            target.static_cache[layer_index - 1].copy_layer_from(encoder, 0, atlas, layer_in_atlas)?;
            publication.cache = CacheUpdate::Written(None);
            draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, dynamic, load)?;
        }
        ShadowCacheOp::Retained {
            generation,
            static_instances,
        } => {
            let dynamic = remaining.split_off(static_instances);
            let cache = layer_index - 1;
            if target.cache_valid[cache] && target.cache_generation[cache] == Some(generation) {
                let span = diagnostics.time_span(encoder, "cache_restore");
                atlas.copy_layer_from(encoder, layer_in_atlas, &target.static_cache[cache], 0)?;
                span.end(encoder);
                LAYER_STATS[layer_index.min(4)][1].fetch_add(remaining.len() as u64, std::sync::atomic::Ordering::Relaxed);
            } else {
                LAYER_STATS[layer_index.min(4)][2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let span = diagnostics.time_span(encoder, "static_draws");
                draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, remaining, layer.ops)?;
                span.end(encoder);
                let span = diagnostics.time_span(encoder, "cache_store");
                target.static_cache[cache].copy_layer_from(encoder, 0, atlas, layer_in_atlas)?;
                span.end(encoder);
                publication.cache = CacheUpdate::Written(Some(generation));
            }
            let span = diagnostics.time_span(encoder, "dynamic_draws");
            draw_shadow_groups(encoder, atlas, layer_in_atlas, pipelines, dynamic, load)?;
            span.end(encoder);
        }
    }
    LAYER_STATS[layer_index.min(4)][0].fetch_add(counted, std::sync::atomic::Ordering::Relaxed);
    Ok(publication)
}

/// Diagnostic per directional layer: [prepared draws, static draws restored
/// from cache instead of drawn, static cache rewrites], for the benchmark report.
pub static LAYER_STATS: [[std::sync::atomic::AtomicU64; 3]; 5] =
    [const { [const { std::sync::atomic::AtomicU64::new(0) }; 3] }; 5];

type RecordedLayer = Result<(Publication, Option<CommandEncoder>), String>;

pub(super) fn record_layers(
    device: &RenderDevice,
    target: &CachedTarget,
    first_layer: usize,
    layers: &[ShadowLayer],
    records: &[PreparedShadowRecord],
    pipelines: &ShadowPipelineStore,
    receiver: Option<&ParallelReceiverVars>,
) -> Vec<(usize, RecordedLayer)> {
    // Each slice has its own static cache and draws into its own atlas
    // layer, so the encoders touch disjoint subresources.
    // Publish and submit in the owner's order after all recording finishes.
    let mut encoded = bevy::tasks::ComputeTaskPool::get().scope(|scope| {
        for (index, layer) in layers.iter().enumerate() {
            scope.spawn(async move {
                let result = (|| {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    let publication = encode_layer(
                        &mut encoder,
                        target,
                        first_layer + index,
                        index,
                        layer,
                        records,
                        pipelines,
                        receiver,
                        None,
                    )?;
                    let encoder = publication.emitted.then_some(encoder);
                    Ok((publication, encoder))
                })();
                (index, result)
            });
        }
    });
    encoded.sort_by_key(|entry| entry.0);
    encoded
}

#[cfg(test)]
mod tests;
