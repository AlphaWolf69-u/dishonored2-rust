//! Shared full-rig acquisition. 1400607f0 resolves one resource identity;
//! callers retain the rig only while its player or scene instance needs it.
use super::Rig;
use crate::archive::Catalog;
use std::{collections::HashMap, sync::{Arc, Condvar, Mutex, OnceLock, Weak}, time::Instant};

type Key = (u64, Arc<str>);
enum State {
    Empty,
    Loading { waiters: usize },
    Ready { weak: Weak<Rig>, handoff: Option<Arc<Rig>>, pending: usize },
    Failed { error: String, pending: usize },
}
struct Entry { state: Mutex<State>, ready: Condvar }
static REGISTRY: OnceLock<Mutex<HashMap<Key, Arc<Entry>>>> = OnceLock::new();
static TRACE: OnceLock<bool> = OnceLock::new();
static SEPARATE: OnceLock<bool> = OnceLock::new();

pub(super) fn tracing() -> bool {
    *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_RIG_ACQUISITION").is_some_and(|v| v == "1"))
}
fn separate() -> bool {
    *SEPARATE.get_or_init(|| std::env::var_os("DH2_DEBUG_SEPARATE_RIG_RESOURCES").is_some_and(|v| v == "1"))
}

pub(super) struct Retire { key: Key, entry: Weak<Entry> }
impl Drop for Retire {
    fn drop(&mut self) {
        let (Some(registry), Some(entry)) = (REGISTRY.get(), self.entry.upgrade()) else { return };
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        if Arc::strong_count(&entry) == 2 && registry.get(&self.key).is_some_and(|e| Arc::ptr_eq(e, &entry)) {
            registry.remove(&self.key);
        }
    }
}

fn discard_empty(key: &Key, entry: &Arc<Entry>) {
    let Some(registry) = REGISTRY.get() else { return };
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    if Arc::strong_count(entry) == 2 && registry.get(key).is_some_and(|e| Arc::ptr_eq(e, entry))
        && entry.state.try_lock().is_ok_and(|state| matches!(*state, State::Empty)) { registry.remove(key); }
}

pub(super) fn release_unused() {
    let Some(registry) = REGISTRY.get() else { return };
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    registry.retain(|_, entry| Arc::strong_count(entry) > 1 || match entry.state.try_lock() {
        Ok(state) => match &*state { State::Loading { .. } => true, State::Ready { weak, .. } => weak.strong_count() != 0, _ => false },
        Err(_) => true,
    });
    if tracing() {
        let path_bytes: usize = registry.keys().map(|(_, path)| path.len()).sum();
        eprintln!("RIG_REGISTRY entries={} slots={} path_bytes={path_bytes} entry_struct_bytes={}",
            registry.len(), registry.capacity(), registry.len() * std::mem::size_of::<Entry>());
    }
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
    let logical_bytes = std::mem::size_of::<Rig>() + std::mem::size_of::<super::SkeletonCore>() + rig.path.len()
        + rig.skeleton.rest.len() * std::mem::size_of::<crate::edge_skeleton::JointRestPose>()
        + rig.skeleton.parents.len() * std::mem::size_of::<Option<usize>>()
        + rig.skeleton.inverse_bind.len() * std::mem::size_of::<[[f32; 4]; 3]>()
        + rig.skeleton.hierarchy.len() * std::mem::size_of::<[u16; 2]>()
        + rig.skeleton.parent_skeleton.as_ref().map_or(0, String::len)
        + rig.skeleton.joint_names.len() * std::mem::size_of::<String>()
        + rig.skeleton.joint_names.iter().map(String::len).sum::<usize>()
        + (rig.tokens.len() + rig.animsets.len()) * std::mem::size_of::<String>()
        + rig.tokens.iter().map(String::len).sum::<usize>() + rig.animsets.iter().map(String::len).sum::<usize>()
        + rig.aliases.len() * std::mem::size_of::<(String, String)>()
        + rig.aliases.iter().map(|(key, value)| key.len() + value.len()).sum::<usize>();
    let core_bytes = std::mem::size_of::<super::SkeletonCore>()
        + rig.skeleton.rest.len() * std::mem::size_of::<crate::edge_skeleton::JointRestPose>()
        + rig.skeleton.parents.len() * std::mem::size_of::<Option<usize>>()
        + rig.skeleton.hierarchy.len() * std::mem::size_of::<[u16; 2]>();
    let names_bytes = rig.skeleton.joint_names.len() * std::mem::size_of::<String>()
        + rig.skeleton.joint_names.iter().map(String::len).sum::<usize>();
    let inverse_bind_bytes = rig.skeleton.inverse_bind.len() * std::mem::size_of::<[[f32; 4]; 3]>();
    eprintln!("RIG_ACQUIRE model={model} result={mode} us={elapsed} thread_cycles={cpu:?} identity={:p} core_identity={:p} source_crc={:?} source_bytes={:?} decoded_crc={:08x} logical_bytes={logical_bytes} core_bytes={core_bytes} names_bytes={names_bytes} inverse_bind_bytes={inverse_bind_bytes}",
        Arc::as_ptr(rig), Arc::as_ptr(&rig.skeleton.core), rig.audit.map(|audit| audit.0), rig.audit.map(|audit| audit.1), decoded.sum());
}

pub(super) fn acquire(catalog: &Catalog, model: &str) -> Result<Arc<Rig>, String> {
    let trace = tracing();
    let start = trace.then(Instant::now);
    let cycles = trace.then(crate::loading::thread_cycles).flatten();
    let normalized = model.replace('\\', "/").to_ascii_lowercase();
    if catalog.has_overlays() || separate() {
        let rig = Arc::new(super::load_rig(catalog, &normalized)?);
        report(model, if catalog.has_overlays() { "overlay" } else { "separate" }, start, cycles, &rig);
        return Ok(rig);
    }
    let key = (catalog.identity(), Arc::<str>::from(normalized.as_str()));
    let entry = {
        let mut registry = REGISTRY.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
        registry.entry(key.clone()).or_insert_with(|| Arc::new(Entry { state: Mutex::new(State::Empty), ready: Condvar::new() })).clone()
    };
    let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut waited = false;
    loop {
        match &mut *state {
            State::Ready { weak, handoff, pending } => if let Some(rig) = weak.upgrade() {
                if waited && *pending > 0 { *pending -= 1; if *pending == 0 { handoff.take(); } }
                drop(state);
                report(model, if waited { "wait" } else { "reuse" }, start, cycles, &rig);
                return Ok(rig);
            },
            State::Loading { waiters } => {
                if !waited { *waiters += 1; }
                waited = true;
                state = entry.ready.wait(state).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            State::Failed { error, pending } => {
                let error = error.clone();
                if waited && *pending > 0 { *pending -= 1; }
                if *pending == 0 { *state = State::Empty; }
                drop(state);
                discard_empty(&key, &entry);
                return Err(error);
            }
            State::Empty => {}
        }
        *state = State::Loading { waiters: 0 };
        drop(state);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rig = super::load_rig(catalog, &normalized)?;
            rig.residency = Some(Retire { key: key.clone(), entry: Arc::downgrade(&entry) });
            Ok(Arc::new(rig))
        })).unwrap_or_else(|_| Err(format!("rig acquisition panicked for {model}")));
        let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
        let waiters = match &*state { State::Loading { waiters } => *waiters, _ => 0 };
        *state = match &result {
            Ok(rig) => State::Ready { weak: Arc::downgrade(rig), handoff: (waiters > 0).then(|| rig.clone()), pending: waiters },
            Err(error) if waiters > 0 => State::Failed { error: error.clone(), pending: waiters },
            Err(_) => State::Empty,
        };
        entry.ready.notify_all();
        drop(state);
        if let Ok(rig) = &result { report(model, "decode", start, cycles, rig); }
        else { discard_empty(&key, &entry); }
        return result;
    }
}
