//! Finer-mip accounting and pressure reduction (14030f3d0, 14030d7a0,
//! 14030d8d0). Backend handle replacement still waits for GPU completion.
use super::*;

pub(super) fn level_bytes(level: DiffuseMipLevel) -> u64 {
    // Host: account actual backend block extents. The original +0x3310 uses
    // width*height*bpp/8; sub-block extents need padding in this backend.
    let format = gpu_image_format(level.format);
    let (width, height) = format.block_dimensions();
    u64::from(level.width.div_ceil(width)) * u64::from(level.height.div_ceil(height))
        * streamed_block_bytes(format).expect("streamed image format") as u64
}

pub(super) fn finer_bytes(mut level: DiffuseMipLevel, minimum: DiffuseMipLevel) -> u64 {
    let mut bytes = 0;
    while level.width > minimum.width || level.height > minimum.height {
        bytes += level_bytes(level);
        level.width = (level.width / 2).max(1);
        level.height = (level.height / 2).max(1);
    }
    bytes
}

pub(super) fn charged_bytes(queue: &SceneRenderQueue) -> u64 {
    queue.diffuse_mip_states.iter().map(|state| {
        if state.pending_reduction {
            // The original releases its accounting when it queues a reduction.
            // The backend retains the old handle until the copy is complete.
            finer_bytes(state.pending.unwrap(), state.minimum.unwrap())
        } else {
            state.streamed_bytes + state.reserved_bytes
        }
    }).sum()
}

pub(super) fn budget_bytes(settings: settings::Settings) -> u64 {
    settings.memory_budget_mb.saturating_mul(1 << 20)
}

fn pressure_bytes(settings: settings::Settings, charged: u64) -> i128 {
    // Signed: the supported -1 max-texture setting contributes -1024 here.
    let headroom = (i64::from(settings.upload_kb as i32) << 10)
        + (i64::from(settings.max_texture_kb) << 10);
    i128::from(charged) + i128::from(headroom) - i128::from(budget_bytes(settings))
}

pub(super) struct Reduction {
    pub key: u64,
    pub slot: usize,
    pub level: DiffuseMipLevel,
    pub mip_count: u32,
    pub dropped: u32,
    pub released_bytes: u64,
}

fn reduction_target(resident: DiffuseMipLevel, minimum: DiffuseMipLevel,
    old_count: u32, minimum_count: u32, expired: bool) -> (DiffuseMipLevel, u32) {
    let count = if expired { minimum_count } else { old_count - 1 };
    if count == minimum_count { return (minimum, count); }
    (DiffuseMipLevel {
        width: (resident.width / 2).max(1), height: (resident.height / 2).max(1),
        resource_mip: resident.resource_mip.map(|mip| mip + 1),
        next_separate_suffix: resident.resource_mip, format: resident.format,
    }, count)
}

/// Original packed path/area/mode/age ordering. Host: select the single
/// lowest eligible reduction by a scan instead of sorting every resident;
/// sort only potentially eligible requests, reusing both scratch vectors.
#[derive(Default)]
pub(super) struct Pressure {
    path_priority: Vec<u16>,
    keys: Vec<u64>,
    prepared_frame: u64,
    pub reductions: u64,
    pub released_bytes: u64,
}

pub(super) fn path_priority(path: &str) -> u16 {
    // 14030cdb0's authored resource-name classes, not rewrite exceptions.
    let class = if path.contains("skiff_engine_01") { 224 }
        else if path.contains("skiff_") { 192 }
        else if path.contains("vista") { 0 } else { 128 };
    class + if path.contains("_d.") { 31 } else if path.contains("_n.") { 15 } else { 0 }
}

fn area_key(area: f32, mode: u16, age: u32, priority: u16, slot: usize) -> u64 {
    let age = u64::from(0xffff - age.min(0xffff));
    let priority = u64::from(priority);
    // 14030f471 uses raw float bits, not float-to-int conversion. With the
    // mode-zero producer the area scale is one half.
    let area = if mode & 1 == 0 { area * 0.5 } else { area };
    let (age_shift, priority_shift) = if area == 0.0 { (24, 8) } else { (8, 24) };
    // Host: a stable slot breaks ties instead of native allocator pointer bits.
    (u64::from(area.to_bits()) << 32) | (age << age_shift)
        | (priority << priority_shift) | (slot as u64 & 0xff)
}

impl Pressure {
    fn prepare(&mut self, queue: &SceneRenderQueue) {
        if self.prepared_frame == queue.texture_stream_frame && self.path_priority.len() == queue.diffuse_image_paths.len() { return; }
        self.prepared_frame = queue.texture_stream_frame;
        if self.path_priority.len() != queue.diffuse_image_paths.len() {
            self.path_priority = queue.diffuse_image_paths.iter().map(|p| path_priority(p)).collect();
            self.keys.resize(self.path_priority.len(), 0);
        }
        for (slot, demand) in queue.diffuse_image_demands.iter().enumerate() {
            let age = queue.texture_stream_frame.saturating_sub(queue.diffuse_image_last_visible[slot]).min(0xffff) as u32;
            self.keys[slot] = area_key(demand.area(), demand.metadata(), age, self.path_priority[slot], slot);
        }
    }

    pub fn request_order(&mut self, queue: &SceneRenderQueue, requested: &[bool], all_mips: bool, order: &mut Vec<usize>) {
        self.prepare(queue);
        order.clear();
        order.reserve(requested.len());
        // Zero-area images cannot pass 14030cce0 except with addAllMips. Base
        // acquisition is a separate load phase, but retain its fallback path.
        order.extend(requested.iter().enumerate().filter_map(|(slot, &needed)|
            (all_mips || queue.diffuse_image_demands[slot].area() > 0.0
                || (needed && queue.diffuse_mip_states[slot].resident.is_none())).then_some(slot)));
        order.sort_unstable_by(|&left, &right| self.keys[right].cmp(&self.keys[left]).then_with(|| left.cmp(&right)));
    }

    pub fn choose(
        &mut self, queue: &SceneRenderQueue, worker: &TextureReadWorker,
        images: &Assets<Image>, settings: settings::Settings, charged: u64,
    ) -> Option<Reduction> {
        if settings.frozen || pressure_bytes(settings, charged) <= 0 { return None; }
        self.prepare(queue);
        let mut selected = None;
        let mut lowest = u64::MAX;
        for (slot, state) in queue.diffuse_mip_states.iter().enumerate() {
            let (Some(resident), Some(minimum)) = (state.resident, state.minimum) else { continue };
            if state.streamed_bytes == 0 || state.pending.is_some()
                || queue.diffuse_image_pending[slot].is_some() || worker.inflight[slot] { continue; }
            let Some(old) = queue.diffuse_image_handles[slot].as_ref().and_then(|h| images.get(h)) else { continue };
            let age = queue.texture_stream_frame.saturating_sub(queue.diffuse_image_last_visible[slot]).min(0xffff) as u32;
            let expired = age.saturating_add(1) > settings.unload_frames;
            let pixels = queue.diffuse_image_demands[slot].projected_pixels(queue.texture_viewport_pixels, settings.priority_multiplier);
            if !expired && resident.width as f32 * resident.height as f32 <= pixels * 4.0 { continue; }
            let key = self.keys[slot];
            if selected.is_some() && key >= lowest { continue; }
            let old_count = old.texture_descriptor.mip_level_count;
            let (level, mip_count) = reduction_target(resident, minimum, old_count, state.minimum_mip_count, expired);
            if mip_count == 0 || mip_count >= old_count { continue; }
            lowest = key;
            selected = Some(Reduction { key, slot, level, mip_count, dropped: old_count - mip_count,
                released_bytes: state.streamed_bytes - finer_bytes(level, minimum) });
        }
        selected
    }
}


#[derive(Clone)]
pub(super) struct SharedReplacement {
    path: String,
    handle: Handle<Image>,
    level: DiffuseMipLevel,
    mip_count: u32,
}

impl SharedImages {
    pub(super) fn age(&mut self) {
        for (_, _, state) in self.resident.values_mut() {
            state.idle_frames = state.idle_frames.saturating_add(1).min(0xffff);
        }
    }

    pub(super) fn charged_bytes(&self) -> u64 {
        self.resident.values().map(|(_, _, state)| {
            if state.pending_reduction { finer_bytes(state.pending.unwrap(), state.minimum.unwrap()) }
            else { state.streamed_bytes }
        }).sum()
    }

    pub(super) fn finish_replacement(&mut self, id: AssetId<Image>, success: bool, registry: Option<&super::super::resident_images::ResidentImages>) -> bool {
        let Some(replacement) = self.pending.remove(&id) else { return false };
        let (_, handle, state) = self.resident.get_mut(&replacement.path).expect("pending shared image owner");
        state.pending = None;
        state.pending_reduction = false;
        if success {
            if let Some(registry) = registry {
                registry.publish_view(&replacement.path, &replacement.handle, replacement.level, replacement.mip_count);
            }
            *handle = replacement.handle;
            state.resident = Some(replacement.level);
            state.streamed_bytes = finer_bytes(replacement.level, state.minimum.unwrap());
        }
        true
    }

    pub(super) fn reduce(&mut self, images: &mut Assets<Image>, mailbox: Option<&PreparedImageMailbox>,
        settings: settings::Settings, charged: u64, active_key: Option<u64>) -> Option<u64> {
        if settings.frozen || pressure_bytes(settings, charged) <= 0 { return None; }
        let mailbox = mailbox?;
        let candidate = self.resident.iter().filter_map(|(path, (_, handle, state))| {
            if state.streamed_bytes == 0 || state.pending.is_some() { return None; }
            let old = images.get(handle)?;
            let resident = state.resident?;
            let minimum = state.minimum?;
            let key = area_key(0.0, 0, state.idle_frames, state.path_priority, 0);
            if active_key.is_some_and(|active| active <= key) { return None; }
            let old_count = old.texture_descriptor.mip_level_count;
            let (level, count) = reduction_target(resident, minimum, old_count, state.minimum_mip_count, state.idle_frames > settings.unload_frames);
            if count == 0 || count >= old_count { return None; }
            Some((key, path, level, count, old_count - count, state.streamed_bytes - finer_bytes(level, minimum)))
        }).min_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        let (_, path, level, count, dropped, released) = candidate?;
        let path = path.clone();
        let (_, old, state) = self.resident.get_mut(&path).unwrap();
        let handle = images.add(empty_streamed_image(level.width, level.height, count, gpu_image_format(level.format)));
        state.pending = Some(level);
        state.pending_reduction = true;
        mailbox.0.lock().unwrap().pending.push(PendingGpuImageUpload {
            id: handle.id(), previous: Some(old.id()), copy: GpuMipCopy::Reduce { dropped }, uploads: Vec::new(),
        });
        self.pending.insert(handle.id(), SharedReplacement { path, handle, level, mip_count: count });
        Some(released)
    }
}
