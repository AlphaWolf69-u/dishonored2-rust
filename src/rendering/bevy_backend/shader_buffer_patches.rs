//! Byte-range updates of large shared ShaderBuffer assets. `Assets::get_mut`
//! marks the whole asset modified, so extraction clones it and the GPU
//! receives all of it again: one moving model's 144-byte probe record
//! re-uploaded the 14.9 MB probe bank every frame. A patch updates the CPU
//! copy without an asset event and writes only the changed bytes.
use bevy::{
    prelude::*,
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        renderer::RenderQueue,
        storage::{GpuShaderBuffer, ShaderBuffer},
        Render, RenderApp, RenderSystems,
    },
};

use std::collections::HashMap;

type Patch = (AssetId<ShaderBuffer>, u64, Vec<u8>);

/// This frame's changed byte ranges, in write order.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct ShaderBufferPatches(Vec<Patch>);

impl ShaderBufferPatches {
    /// Drop updates from the outgoing scene while normal producers are gated.
    pub(crate) fn release_level(&mut self) {
        self.0.clear();
    }

    /// Write `bytes` at `offset` of the asset's data. Unchanged bytes produce
    /// no GPU write. Returns false when the asset or range is absent.
    pub fn write(
        &mut self,
        buffers: &mut Assets<ShaderBuffer>,
        id: impl Into<AssetId<ShaderBuffer>>,
        offset: usize,
        bytes: &[u8],
    ) -> bool {
        let id = id.into();
        // Queue writes need 4-byte aligned offsets and sizes.
        if offset % 4 != 0 || bytes.len() % 4 != 0 {
            let Some(mut buffer) = buffers.get_mut(id) else { return false };
            let Some(target) = buffer.data.as_mut().and_then(|data| data.get_mut(offset..offset + bytes.len())) else {
                return false;
            };
            target.copy_from_slice(bytes);
            return true;
        }
        let Some(target) = buffers
            .get_mut_untracked(id)
            .and_then(|buffer| buffer.data.as_mut())
            .and_then(|data| data.get_mut(offset..offset + bytes.len()))
        else {
            return false;
        };
        if target != bytes {
            target.copy_from_slice(bytes);
            self.0.push((id, offset as u64, bytes.to_vec()));
        }
        true
    }
}

/// Patches whose GPU buffer is not prepared yet; applied once it is, after
/// the full upload that already carries older bytes.
#[derive(Resource, Default)]
struct PendingPatches(Vec<Patch>);

/// The render acknowledgement stops outgoing producers before this runs.
/// All current patch targets belong to the outgoing level.
pub(super) fn release_level(world: &mut World) {
    let mut pending = world.resource_mut::<PendingPatches>();
    pending.0.clear();
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<ShaderBufferPatches>()
        .add_plugins(ExtractResourcePlugin::<ShaderBufferPatches>::default())
        .add_systems(First, |mut patches: ResMut<ShaderBufferPatches>| patches.0.clear());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<PendingPatches>()
            .add_systems(Render, apply.in_set(RenderSystems::PrepareResources));
    }
}

fn apply(
    patches: Res<ShaderBufferPatches>,
    mut pending: ResMut<PendingPatches>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    queue: Res<RenderQueue>,
) {
    if patches.is_changed() {
        pending.0.extend(patches.0.iter().cloned());
    }
    // Adjacent or overlapping ranges of one buffer become one write; within a
    // write, later patches overwrite earlier ones as their own writes would.
    let mut ready: HashMap<AssetId<ShaderBuffer>, Vec<(usize, u64, u64)>> = HashMap::new();
    for (order, (id, offset, bytes)) in pending.0.iter().enumerate() {
        if buffers.get(*id).is_some() {
            ready.entry(*id).or_default().push((order, *offset, *offset + bytes.len() as u64));
        }
    }
    for (id, mut ranges) in ready {
        let gpu = buffers.get(id).unwrap();
        ranges.sort_unstable_by_key(|&(order, start, _)| (start, order));
        let mut group: Vec<usize> = Vec::new();
        let (mut start, mut end) = (ranges[0].1, ranges[0].1);
        for (order, range_start, range_end) in ranges.into_iter().chain([(usize::MAX, u64::MAX, 0)]) {
            if range_start > end {
                group.sort_unstable();
                let mut bytes = vec![0; (end - start) as usize];
                for &order in &group {
                    let (_, offset, patch) = &pending.0[order];
                    let at = (offset - start) as usize;
                    bytes[at..at + patch.len()].copy_from_slice(patch);
                }
                queue.write_buffer(&gpu.buffer, start, &bytes);
                group.clear();
                start = range_start;
            }
            group.push(order);
            end = end.max(range_end);
        }
    }
    pending.0.retain(|(id, ..)| buffers.get(*id).is_none());
}
