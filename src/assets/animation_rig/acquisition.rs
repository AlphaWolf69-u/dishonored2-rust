//! Rig acquisition diagnostics. Actual sharing belongs to live BasicRig/scene
//! owners and the common Edge skeleton owner, without another global cache.
use super::Rig;
use crate::archive::Catalog;
use std::{sync::{Arc, OnceLock}, time::Instant};

static TRACE: OnceLock<bool> = OnceLock::new();

pub(super) struct Audit { source_crc: u32, source_bytes: usize, identity: usize, logical_bytes: usize }
impl Audit {
    pub(super) fn new(source_crc: u32, source_bytes: usize) -> Self { Self { source_crc, source_bytes, identity: 0, logical_bytes: 0 } }
}
impl Drop for Audit {
    fn drop(&mut self) {
        if self.identity != 0 {
            eprintln!("RIG_RELEASE identity={:#x} rig_owned_bytes={} tick={:?}", self.identity, self.logical_bytes,
                crate::platform::performance_counter::counter().ok());
        }
    }
}
fn owned_bytes(rig: &Rig) -> usize {
    std::mem::size_of::<Rig>() + rig.path.len()
        + (rig.tokens.len() + rig.animsets.len()) * std::mem::size_of::<String>()
        + rig.tokens.iter().map(String::len).sum::<usize>() + rig.animsets.iter().map(String::len).sum::<usize>()
        + rig.aliases.len() * std::mem::size_of::<(String, String)>()
        + rig.aliases.iter().map(|(key, value)| key.len() + value.len()).sum::<usize>()
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
    let mut aliases: Vec<_> = rig.aliases.iter().collect();
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
    eprintln!("RIG_ACQUIRE model={model} result={mode} us={elapsed} thread_cycles={cpu:?} identity={:p} skeleton_identity={:p} source_composite_crc={:?} source_bytes={:?} decoded_crc={:08x} rig_owned_bytes={logical_bytes} skeleton_core_bytes={core_bytes} skeleton_names_bytes={names_bytes} skeleton_inverse_bind_bytes={inverse_bind_bytes} tick={:?}",
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
