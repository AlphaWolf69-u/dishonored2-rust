//! Diagnostic: `DH2_DEBUG_LOAD_TIMING=1` logs the time of each load phase,
//! since the previous phase and since the first mark.
use std::sync::Mutex;
use std::time::Instant;

static MARKS: Mutex<Option<(Instant, Instant)>> = Mutex::new(None);

/// Exact host publication costs, separate from the worker's phase markers.
pub(super) struct Span(&'static str, Option<Instant>);
impl Span {
    pub(super) fn new(label: &'static str) -> Self {
        Self(label, (std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1")).then(Instant::now))
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.1 {
            eprintln!("LOAD host {}: {:.3} ms", self.0, start.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

pub(super) fn mark(phase: &str) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let now = Instant::now();
    let mut marks = MARKS.lock().unwrap();
    let (start, last) = marks.get_or_insert((now, now));
    let memory = crate::platform::process_memory::sample()
        .map_or(String::new(), |m| format!(" [private {} MB, resident {} MB]", m.private_bytes >> 20, m.resident_bytes >> 20));
    eprintln!("LOAD {phase}: +{:.3}s (at {:.3}s){memory}", (now - *last).as_secs_f64(), (now - *start).as_secs_f64());
    let tags: Vec<String> = crate::platform::allocation_tags::report().into_iter().filter(|t| t.1 > 16 << 20).map(|(n, b)| format!("{n} {} MB", b >> 20)).collect();
    if !tags.is_empty() {
        eprintln!("LOAD heap: {}", tags.join(", "));
    }
    // This thread's later allocations belong to the phase after this mark.
    if cfg!(feature = "debug_allocations") {
        std::mem::forget(crate::platform::allocation_tags::tag(Box::leak(format!("after {phase}").into_boxed_str())));
    }
    *last = now;
}

/// Marks the first frame, then "streaming settled" once no texture upload
/// is pending and the resident count has held for 60 frames.
pub(super) fn track_streaming(scene: bevy::prelude::Res<super::SceneRenderQueue>, mut state: bevy::prelude::Local<(bool, usize, u32, bool)>,
    device: Option<bevy::prelude::Res<bevy::render::renderer::RenderDevice>>) {
    use bevy::prelude::DetectChanges;
    if scene.is_added() { *state = Default::default(); }
    let (first, resident, stable, settled) = &mut *state;
    if !*first {
        *first = true;
        mark("first frame");
    }
    if *settled {
        return;
    }
    let now = scene.diffuse_image_handles.iter().filter(|h| h.is_some()).count();
    let pending = scene.diffuse_image_pending.iter().any(|p| p.is_some());
    if now == *resident && !pending {
        *stable += 1;
    } else {
        *stable = 0;
        *resident = now;
    }
    if *stable == 60 {
        *settled = true;
        mark(&format!("streaming settled ({now} of {} images resident)", scene.diffuse_image_handles.len()));
        gpu_allocations(device.as_deref());
    }
}

static LIST: Mutex<Option<std::sync::Arc<Vec<crate::assets::map_resources::Resource>>>> = Mutex::new(None);

/// Keep the current map's load lists for the coverage report.
pub(super) fn keep_load_list(catalog: &crate::assets::archive::Catalog) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let Some(map) = crate::gameplay::map_info::current_map() else { return };
    let map = map.trim_start_matches("maps/").trim_end_matches(".map").to_owned();
    let language = crate::assets::lang::read_text_language(catalog.base()).unwrap_or("english");
    match crate::assets::map_resources::acquire_level_list(catalog.base(), catalog, &map, language) {
        Ok(list) => *LIST.lock().unwrap() = Some(list),
        Err(e) => eprintln!("LOAD list {map}: {e}"),
    }
}

/// When every loading gate is complete: play starts, and the coverage of
/// the original's load lists is reported.
pub(super) fn level_ready(
    mut done: bevy::prelude::Local<bool>,
    scene: bevy::prelude::Res<super::SceneData>,
    device: Option<bevy::prelude::Res<bevy::render::renderer::RenderDevice>>,
    meshes: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Mesh>>,
    images: bevy::prelude::Res<bevy::prelude::Assets<bevy::prelude::Image>>,
    buffers: bevy::prelude::Res<bevy::prelude::Assets<bevy::render::storage::ShaderBuffer>>,
    shaders: bevy::prelude::Res<bevy::prelude::Assets<bevy::shader::Shader>>,
) {
    use bevy::prelude::DetectChanges;
    if scene.is_added() { *done = false; }
    if *done || !crate::loading::ready() {
        return;
    }
    *done = true;
    mark("level resources ready");
    asset_bytes(&meshes, &images);
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        eprintln!("LOAD shader assets: {}", shaders.len());
        let bytes: usize = buffers.iter().map(|(_, b)| b.data.as_ref().map_or(0, Vec::len)).sum();
        eprintln!("LOAD shader buffers: {} with {} MB CPU", buffers.len(), bytes >> 20);
        let mut largest: Vec<_> = buffers.iter()
            .map(|(_, b)| (b.data.as_ref().map_or(0, Vec::len), b.buffer_description.label.unwrap_or("--")))
            .collect();
        largest.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        for (bytes, label) in largest.iter().take(8) {
            eprintln!("LOAD shader buffer {label}: {} KB", bytes >> 10);
        }
    }
    gpu_allocations(device.as_deref());
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        for (frames, bytes, blocks) in crate::platform::allocation_tags::big_blocks().into_iter().take(30) {
            eprintln!("LOAD big {} MB in {blocks}: {frames}", bytes >> 20);
        }
    }
    // Coverage is emitted once. A diagnostic must not retain the map's
    // acquisition descriptors throughout gameplay or into the next load.
    if let Some(list) = LIST.lock().unwrap().take() {
        let report = crate::loading::report(&list);
        let (listed, loaded) = report.iter().fold((0, 0), |a, c| (a.0 + c.listed, a.1 + c.loaded));
        eprintln!("LOAD coverage: {loaded} of {listed} listed resources loaded");
        for c in report.iter().filter(|c| c.listed > 0 || c.unlisted > 0) {
            eprintln!("LOAD coverage {}: {} of {} listed, {} unlisted", c.kind, c.loaded, c.listed, c.unlisted);
        }
    }
}

/// Diagnostic: a resource loaded during play rather than at level load.
pub(super) fn late(kind: &str, what: &str) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let at = MARKS.lock().unwrap().map_or(0.0, |(start, _)| start.elapsed().as_secs_f64());
    eprintln!("LOAD late {kind}: {what} (at {at:.3}s)");
}

/// Diagnostic: CPU bytes held by the mesh and image assets, the largest
/// meshes, and how many distinct meshes share a vertex count.
pub(super) fn asset_bytes(meshes: &bevy::prelude::Assets<bevy::prelude::Mesh>, images: &bevy::prelude::Assets<bevy::prelude::Image>) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    // Render-world-only meshes lose their data once extracted; count only
    // meshes whose CPU copy is still held.
    let mut sizes: Vec<(usize, usize, usize)> = meshes
        .iter()
        .filter_map(|(_, m)| {
            let attributes = m.try_attributes().ok()?;
            let (mut bytes, mut vertices, mut count) = (0, 0, 0);
            for (_, values) in attributes {
                bytes += values.get_bytes().len();
                vertices = values.len();
                count += 1;
            }
            let indices = m.try_indices_option().ok().flatten()
                .map_or(0, |i| i.len() * if matches!(i, bevy::mesh::Indices::U16(_)) { 2 } else { 4 });
            Some((bytes + indices, vertices, count))
        })
        .collect();
    sizes.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let total: usize = sizes.iter().map(|s| s.0).sum();
    // Which builders keep CPU copies: held meshes grouped by attribute set.
    let mut groups: std::collections::HashMap<String, (usize, usize)> = Default::default();
    for (_, m) in meshes.iter() {
        let Ok(attributes) = m.try_attributes() else { continue };
        let mut names: Vec<&str> = Vec::new();
        let mut bytes = 0;
        for (attribute, values) in attributes {
            names.push(attribute.name);
            bytes += values.get_bytes().len();
        }
        names.sort_unstable();
        let entry = groups.entry(names.join(",")).or_default();
        entry.0 += 1;
        entry.1 += bytes;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (names, (count, bytes)) in groups.iter().take(8) {
        eprintln!("LOAD held meshes {count} ({} MB): {names}", bytes >> 20);
    }
    let image_bytes: usize = images.iter().map(|(_, i)| i.data.as_ref().map_or(0, Vec::len)).sum();
    eprintln!(
        "LOAD assets: {} of {} meshes hold {} MB CPU (largest {:?}), {} images {} MB CPU",
        sizes.len(),
        meshes.len(),
        total >> 20,
        sizes.iter().take(5).map(|s| (s.0 >> 20, s.1, s.2)).collect::<Vec<_>>(),
        images.len(),
        image_bytes >> 20
    );
}

/// Diagnostic: reports `what` when private memory grew by more than 32 MB
/// while it was alive.
pub(super) struct Growth(Option<(String, usize)>);
impl Growth {
    pub(super) fn new(what: String) -> Self {
        let enabled = std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1");
        Self(enabled.then(|| (what, crate::platform::process_memory::sample().map_or(0, |m| m.private_bytes))).filter(|(_, b)| *b > 0))
    }
}
impl Drop for Growth {
    fn drop(&mut self) {
        let Some((what, before)) = self.0.take() else { return };
        let after = crate::platform::process_memory::sample().map_or(0, |m| m.private_bytes);
        if after > before + (32 << 20) {
            eprintln!("LOAD growth {} MB: {what}", (after - before) >> 20);
        }
    }
}

/// Diagnostic: the GPU allocator's live allocations by label, largest first.
fn gpu_allocations(device: Option<&bevy::render::renderer::RenderDevice>) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|v| v != "1") {
        return;
    }
    let Some(report) = device.and_then(|d| d.wgpu_device().generate_allocator_report()) else {
        return eprintln!("LOAD gpu: no allocator report");
    };
    let mut by_name: std::collections::HashMap<&str, (u64, usize)> = Default::default();
    for a in &report.allocations {
        let e = by_name.entry(if a.name.is_empty() { "--" } else { a.name.as_str() }).or_default();
        e.0 += a.size;
        e.1 += 1;
    }
    let mut names: Vec<_> = by_name.into_iter().collect();
    names.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    eprintln!("LOAD gpu: {} MB allocated, {} MB reserved, {} blocks, {} allocations",
        report.total_allocated_bytes >> 20, report.total_reserved_bytes >> 20, report.blocks.len(), report.allocations.len());
    for (name, (bytes, count)) in names.iter().take(25) {
        eprintln!("LOAD gpu {name}: {} MB in {count}", bytes >> 20);
    }
    let mut buckets = [(0u64, 0usize); 8];
    for a in report.allocations.iter().filter(|a| a.name.ends_with("Staging")) {
        let bucket = match a.size { 0..4096 => 0, 4096..65536 => 1, 65536..262144 => 2, 262144..1048576 => 3, 1048576..4194304 => 4, 4194304..16777216 => 5, 16777216..67108864 => 6, _ => 7 };
        buckets[bucket].0 += a.size;
        buckets[bucket].1 += 1;
    }
    eprintln!("LOAD gpu staging by size (<4K, <64K, <256K, <1M, <4M, <16M, <64M, more): {:?}", buckets.map(|(b, n)| format!("{n}: {} MB", b >> 20)));
}
