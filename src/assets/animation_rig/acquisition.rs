//! Rig acquisition diagnostics. Actual sharing belongs to live BasicRig/scene
//! owners and the common Edge skeleton owner, without another global cache.
use super::Rig;
use crate::archive::Catalog;
use std::{sync::{Arc, OnceLock}, time::Instant};

static TRACE: OnceLock<bool> = OnceLock::new();
static LEGACY_DIAGNOSTIC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) struct Audit { source_crc: u32, source_bytes: usize, identity: usize, logical_bytes: usize }

// The pre-sharing Rig field layout. Constructed only as a type: the
// diagnostic reconstructs its map and path list temporarily from the same
// decoded declarations, then immediately drops those temporary values.
#[allow(dead_code)]
struct LegacyRigLayout {
    skeleton: Arc<crate::edge_skeleton::EdgeSkeleton>,
    root: crate::edge_skeleton::JointRestPose,
    aliases: std::collections::HashMap<String, String>,
    animsets: Vec<String>,
    tokens: Vec<String>,
    property_groups: Vec<super::PropertyGroup>,
    path: String,
    audit: Option<Audit>,
}
#[allow(dead_code)]
struct LegacyPropertyGroup { name: usize, tags: Vec<std::ops::Range<usize>> }
impl Audit {
    pub(super) fn new(source_crc: u32, source_bytes: usize) -> Self { Self { source_crc, source_bytes, identity: 0, logical_bytes: 0 } }
}
impl Drop for Audit {
    fn drop(&mut self) {
        if self.identity != 0 {
            let owner_heap = crate::platform::allocation_tags::live_bytes("animset-owner");
            let owner_reserved = crate::platform::allocation_tags::live_reserved_bytes("animset-owner");
            eprintln!("RIG_RELEASE identity={:#x} rig_owned_bytes={} animset_owner_heap_bytes={owner_heap:?} animset_owner_reserved_bytes={owner_reserved:?} tick={:?}", self.identity, self.logical_bytes,
                crate::platform::performance_counter::counter().ok());
        }
    }
}
fn common_owned_bytes(rig: &Rig) -> usize {
    rig.path.capacity()
        + rig.property_groups.capacity() * std::mem::size_of::<super::PropertyGroup>()
        + rig.property_groups.iter().map(|group| group.tags.capacity() * std::mem::size_of::<super::PropertyTag>()).sum::<usize>()
}
fn legacy_common_owned_bytes(rig: &Rig) -> usize {
    let tokens = rig.declarations.iter().flat_map(|declaration| declaration.tokens.iter());
    rig.path.capacity()
        + rig.property_groups.capacity() * std::mem::size_of::<LegacyPropertyGroup>()
        + rig.property_groups.iter().map(|group| group.tags.capacity() * std::mem::size_of::<std::ops::Range<usize>>()).sum::<usize>()
        + tokens.clone().count() * std::mem::size_of::<String>()
        + tokens.map(String::capacity).sum::<usize>()
}
fn owned_bytes(rig: &Rig) -> usize {
    std::mem::size_of::<Rig>() + 2 * std::mem::size_of::<usize>() + common_owned_bytes(rig)
        + rig.animsets.capacity() * std::mem::size_of::<Arc<super::animset::AnimSet>>()
        + rig.declarations.capacity() * std::mem::size_of::<Arc<super::declaration::Declaration>>()
}
fn wrap(rig: Rig) -> Arc<Rig> {
    let mut rig = Arc::new(rig);
    if rig.audit.is_some() {
        let identity = Arc::as_ptr(&rig) as usize;
        let bytes = owned_bytes(&rig);
        if let Some(audit) = Arc::get_mut(&mut rig).and_then(|rig| rig.audit.as_mut()) {
            audit.identity = identity; audit.logical_bytes = bytes;
        }
    }
    rig
}

pub(super) fn tracing() -> bool {
    *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_RIG_ACQUISITION").is_some_and(|v| v == "1"))
}
fn report(model: &str, mode: &str, start: Option<Instant>, cycles: Option<u64>, rig: &Arc<Rig>) {
    let Some(start) = start else { return };
    let elapsed = start.elapsed().as_micros();
    let cpu = cycles.zip(crate::loading::thread_cycles()).map(|(a, b)| b.saturating_sub(a));
    let mut decoded = flate2::Crc::new();
    for value in rig.root.rotation.iter().chain(&rig.root.translation).chain(&rig.root.scale) {
        decoded.update(&value.to_bits().to_le_bytes());
    }
    for joint in &rig.skeleton.rest {
        for value in joint.rotation.iter().chain(&joint.translation).chain(&joint.scale) {
            decoded.update(&value.to_bits().to_le_bytes());
        }
    }
    for pair in &rig.skeleton.hierarchy { for value in pair { decoded.update(&value.to_le_bytes()); } }
    for row in rig.skeleton.inverse_bind.iter().flatten() { for value in row { decoded.update(&value.to_bits().to_le_bytes()); } }
    for name in &rig.skeleton.joint_names { decoded.update(name.as_bytes()); }
    // Diagnostic only: reconstruct the previous flattened view without
    // retaining duplicate alias definitions in every live rig.
    let _legacy_lock = cfg!(feature = "debug_allocations").then(||
        LEGACY_DIAGNOSTIC_LOCK.lock().unwrap_or_else(|error| error.into_inner()));
    let legacy_before = crate::platform::allocation_tags::live_bytes("animset-legacy-reconstruction");
    let legacy_reserved_before = crate::platform::allocation_tags::live_reserved_bytes("animset-legacy-reconstruction");
    let legacy_tag = cfg!(feature = "debug_allocations").then(||
        crate::platform::allocation_tags::tag("animset-legacy-reconstruction"));
    let mut flattened = std::collections::HashMap::<String, String>::new();
    let mut legacy_paths = Vec::<String>::new();
    for set in rig.animsets.iter().rev() {
        let name = set.path.strip_prefix("generated/decls/animset/")
            .and_then(|path| path.strip_suffix(".decl")).unwrap_or(&set.path);
        // Match the former Rig's actual path formatting/allocation.
        legacy_paths.push(format!("generated/decls/animset/{name}.decl"));
        for (name, path) in set.entries() { flattened.insert(name.to_owned(), path.to_owned()); }
    }
    drop(legacy_tag);
    let legacy_heap = legacy_before.zip(crate::platform::allocation_tags::live_bytes("animset-legacy-reconstruction"))
        .map(|(before, after)| after.saturating_sub(before));
    let legacy_reserved = legacy_reserved_before.zip(crate::platform::allocation_tags::live_reserved_bytes("animset-legacy-reconstruction"))
        .map(|(before, after)| after.saturating_sub(before));
    let owner_heap = crate::platform::allocation_tags::live_bytes("animset-owner");
    let owner_reserved = crate::platform::allocation_tags::live_reserved_bytes("animset-owner");
    let md6_owner_heap = crate::platform::allocation_tags::live_bytes("md6-declaration-owner");
    let md6_owner_reserved = crate::platform::allocation_tags::live_reserved_bytes("md6-declaration-owner");
    let md6_registry_heap = crate::platform::allocation_tags::live_bytes("md6-declaration-registry");
    let md6_registry_reserved = crate::platform::allocation_tags::live_reserved_bytes("md6-declaration-registry");
    let legacy_bytes = std::mem::size_of::<LegacyRigLayout>() + 2 * std::mem::size_of::<usize>()
        + legacy_common_owned_bytes(rig) + legacy_paths.capacity() * std::mem::size_of::<String>()
        + legacy_paths.iter().map(String::capacity).sum::<usize>()
        + flattened.capacity() * std::mem::size_of::<(String, String)>()
        + flattened.iter().map(|(name, path)| name.capacity() + path.capacity()).sum::<usize>();
    let mut aliases: Vec<_> = flattened.iter().collect();
    aliases.sort_unstable_by_key(|(name, _)| *name);
    for (name, path) in aliases { decoded.update(name.as_bytes()); decoded.update(path.as_bytes()); }
    // Skeleton bytes are reported by its own identity; count only rig-owned
    // metadata here so totals do not double-count the shared skeleton.
    let logical_bytes = owned_bytes(rig);
    let core_bytes = std::mem::size_of::<crate::edge_skeleton::EdgeSkeleton>()
        + rig.skeleton.rest.len() * std::mem::size_of::<crate::edge_skeleton::JointRestPose>()
        + rig.skeleton.parents.len() * std::mem::size_of::<Option<usize>>()
        + rig.skeleton.hierarchy.len() * std::mem::size_of::<[u16; 2]>();
    let names_bytes = rig.skeleton.joint_names.len() * std::mem::size_of::<String>()
        + rig.skeleton.joint_names.iter().map(String::len).sum::<usize>();
    let inverse_bind_bytes = rig.skeleton.inverse_bind.len() * std::mem::size_of::<[[f32; 4]; 3]>();
    // Both are Arc-owned; include the two reference counters. The diagnostic
    // allocator's identical Arc prefix is omitted from both sides.
    let rig_inline = std::mem::size_of::<Rig>() + 2 * std::mem::size_of::<usize>();
    let legacy_inline = std::mem::size_of::<LegacyRigLayout>() + 2 * std::mem::size_of::<usize>();
    let registry_inline = std::mem::size_of::<std::sync::Mutex<super::animset::Registry>>();
    eprintln!("RIG_ACQUIRE model={model} result={mode} us={elapsed} thread_cycles={cpu:?} identity={:p} skeleton_identity={:p} source_composite_crc={:?} source_bytes={:?} decoded_crc={:08x} rig_owned_bytes={logical_bytes} legacy_rig_estimated_bytes={legacy_bytes} legacy_rig_heap_bytes={legacy_heap:?} legacy_rig_reserved_bytes={legacy_reserved:?} animset_owner_heap_bytes={owner_heap:?} animset_owner_reserved_bytes={owner_reserved:?} md6_owner_heap_bytes={md6_owner_heap:?} md6_owner_reserved_bytes={md6_owner_reserved:?} md6_registry_heap_bytes={md6_registry_heap:?} md6_registry_reserved_bytes={md6_registry_reserved:?} rig_inline_bytes={rig_inline} legacy_rig_inline_bytes={legacy_inline} animset_registry_inline_bytes={registry_inline} skeleton_core_bytes={core_bytes} skeleton_names_bytes={names_bytes} skeleton_inverse_bind_bytes={inverse_bind_bytes} tick={:?}",
        Arc::as_ptr(rig), Arc::as_ptr(&rig.skeleton), rig.audit.as_ref().map(|audit| audit.source_crc), rig.audit.as_ref().map(|audit| audit.source_bytes), decoded.sum(), crate::platform::performance_counter::counter().ok());
}

pub(super) fn acquire(catalog: &Catalog, model: &str) -> Result<Arc<Rig>, String> {
    let trace = tracing();
    let start = trace.then(Instant::now);
    let cycles = trace.then(crate::loading::thread_cycles).flatten();
    let normalized = model.replace('\\', "/").to_ascii_lowercase();
    let rig = wrap(super::load_rig(catalog, &normalized)?);
    report(model, "load", start, cycles, &rig);
    Ok(rig)
}
