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
    layer: &NativeShadowLayer,
    records: &[PreparedShadowRecord],
    pipelines: &ShadowPipelineStore,
    receiver: Option<&NativeParallelReceiverVars>,
) -> Result<Publication, String> {
    let temporary = &target.temporary[physical_layer];
    let mut remaining = directional_layer_draws(records, pipelines, layer_index);
    if remaining.is_empty()
        && layer.cache == NativeShadowCacheOp::None
        && matches!(layer.ops.depth, NativeShadowLoad::Load)
        && matches!(layer.ops.stencil, NativeStencilLoad::Load)
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
    let load = NativeShadowLayerOps {
        depth: NativeShadowLoad::Load,
        stencil: NativeStencilLoad::Load,
    };
    let mut publication = Publication {
        complete: true,
        emitted: true,
        cache: CacheUpdate::Unchanged,
    };
    match layer.cache {
        NativeShadowCacheOp::None => {
            draw_native_shadow_groups(encoder, temporary, pipelines, remaining, layer.ops)?;
        }
        NativeShadowCacheOp::Restore => {
            temporary.copy_resource_from(encoder, &target.static_cache[layer_index - 1])?;
            draw_native_shadow_groups(encoder, temporary, pipelines, remaining, load)?;
        }
        NativeShadowCacheOp::Store { static_instances } => {
            let dynamic = remaining.split_off(static_instances);
            draw_native_shadow_groups(encoder, temporary, pipelines, remaining, layer.ops)?;
            target.static_cache[layer_index - 1].copy_resource_from(encoder, temporary)?;
            publication.cache = CacheUpdate::Written(None);
            draw_native_shadow_groups(encoder, temporary, pipelines, dynamic, load)?;
        }
        NativeShadowCacheOp::Retained {
            generation,
            static_instances,
        } => {
            let dynamic = remaining.split_off(static_instances);
            let cache = layer_index - 1;
            if target.cache_valid[cache] && target.cache_generation[cache] == Some(generation) {
                temporary.copy_resource_from(encoder, &target.static_cache[cache])?;
            } else {
                draw_native_shadow_groups(encoder, temporary, pipelines, remaining, layer.ops)?;
                target.static_cache[cache].copy_resource_from(encoder, temporary)?;
                publication.cache = CacheUpdate::Written(Some(generation));
            }
            draw_native_shadow_groups(encoder, temporary, pipelines, dynamic, load)?;
        }
    }
    // 32db10 full-rectangle copy fast path, AFTER the dynamic pass.
    target
        .array
        .copy_layer_from(encoder, physical_layer as u32, temporary, 0)?;
    Ok(publication)
}

type RecordedLayer = Result<(Publication, Option<CommandEncoder>), String>;

pub(super) fn record_layers(
    device: &RenderDevice,
    target: &CachedTarget,
    first_layer: usize,
    layers: &[NativeShadowLayer],
    records: &[PreparedShadowRecord],
    pipelines: &ShadowPipelineStore,
    receiver: Option<&NativeParallelReceiverVars>,
) -> Vec<(usize, RecordedLayer)> {
    // Each slice has its own temporary and static cache. Only the atlas is
    // shared, and each encoder copies to a distinct physical array layer.
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
