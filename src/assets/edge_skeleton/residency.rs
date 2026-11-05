//! Shared Edge skeleton resource identity (1400607f0, 14024f2a0).
use super::EdgeSkeleton;
use crate::archive::Catalog;
use std::{collections::HashMap, sync::{Arc, Condvar, Mutex, OnceLock, Weak}};

type Key = (u64, Arc<str>);
enum State { Empty, Loading { waiters: usize }, Ready { weak: Weak<EdgeSkeleton>, handoff: Option<Arc<EdgeSkeleton>>, pending: usize }, Failed { error: String, pending: usize } }
struct Entry { state: Mutex<State>, ready: Condvar }
static REGISTRY: OnceLock<Mutex<HashMap<Key, Arc<Entry>>>> = OnceLock::new();
static TRACE: OnceLock<bool> = OnceLock::new();
static SEPARATE: OnceLock<bool> = OnceLock::new();
fn tracing() -> bool { *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_SKELETON_ACQUISITION").is_some_and(|v| v == "1") || std::env::var_os("DH2_DEBUG_RIG_ACQUISITION").is_some_and(|v| v == "1") || std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1")) }
fn separate() -> bool { *SEPARATE.get_or_init(|| std::env::var_os("DH2_DEBUG_SEPARATE_SKELETON_RESOURCES").is_some_and(|v| v == "1")) }

#[derive(Debug)]
pub(super) struct Audit { pub source_crc: u32, pub source_bytes: usize, identity: usize, logical_bytes: usize }
impl Drop for Audit {
    fn drop(&mut self) {
        if self.identity != 0 {
            eprintln!("SKELETON_RELEASE identity={:#x} logical_bytes={} tick={:?}", self.identity, self.logical_bytes,
                crate::platform::performance_counter::counter().ok());
        }
    }
}
fn sizes(skeleton: &EdgeSkeleton) -> (usize, usize, usize, usize) {
    let core = skeleton.rest.len() * std::mem::size_of::<super::JointRestPose>()
        + skeleton.parents.len() * std::mem::size_of::<Option<usize>>() + skeleton.hierarchy.len() * std::mem::size_of::<[u16; 2]>();
    let names = skeleton.joint_names.len() * std::mem::size_of::<String>() + skeleton.joint_names.iter().map(String::len).sum::<usize>();
    let inverse = skeleton.inverse_bind.len() * std::mem::size_of::<[[f32; 4]; 3]>();
    (core, names, inverse, std::mem::size_of::<EdgeSkeleton>() + core + names + inverse + skeleton.parent_skeleton.as_ref().map_or(0, String::len))
}
fn wrap(skeleton: EdgeSkeleton) -> Arc<EdgeSkeleton> {
    let mut skeleton = Arc::new(skeleton);
    if skeleton.audit.is_some() {
        let identity = Arc::as_ptr(&skeleton) as usize;
        let logical_bytes = sizes(&skeleton).3;
        if let Some(audit) = Arc::get_mut(&mut skeleton).and_then(|s| s.audit.as_mut()) { audit.identity = identity; audit.logical_bytes = logical_bytes; }
    }
    skeleton
}

pub(super) struct Retire { key: Key, entry: Weak<Entry> }
impl std::fmt::Debug for Retire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_tuple("SkeletonRetire").field(&self.key).finish() }
}
impl Drop for Retire {
    fn drop(&mut self) {
        let (Some(registry), Some(entry)) = (REGISTRY.get(), self.entry.upgrade()) else { return };
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        if Arc::strong_count(&entry) == 2 && registry.get(&self.key).is_some_and(|e| Arc::ptr_eq(e, &entry)) { registry.remove(&self.key); }
    }
}
fn discard_empty(key: &Key, entry: &Arc<Entry>) {
    let Some(registry) = REGISTRY.get() else { return };
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    if Arc::strong_count(entry) == 2 && registry.get(key).is_some_and(|e| Arc::ptr_eq(e, entry))
        && entry.state.try_lock().is_ok_and(|state| matches!(*state, State::Empty)) { registry.remove(key); }
}
pub(crate) fn release_unused() {
    let Some(registry) = REGISTRY.get() else { return };
    let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
    registry.retain(|_, entry| Arc::strong_count(entry) > 1 || match entry.state.try_lock() {
        Ok(state) => match &*state { State::Loading { .. } => true, State::Ready { weak, .. } => weak.strong_count() != 0, _ => false },
        Err(_) => true,
    });
    if tracing() {
        let path_bytes: usize = registry.keys().map(|(_, path)| path.len()).sum();
        eprintln!("SKELETON_REGISTRY entries={} slots={} path_bytes={path_bytes} entry_struct_bytes={}", registry.len(), registry.capacity(), registry.len() * std::mem::size_of::<Entry>());
    }
}
fn load(catalog: &Catalog, path: &str) -> Result<EdgeSkeleton, String> {
    let trace = tracing();
    let start = trace.then(crate::loading::thread_cycles).flatten();
    let bytes = catalog.read(path).map_err(|error| error.to_string())?.ok_or_else(|| format!("skeleton {path} is absent"))?;
    let mut skeleton = super::parse(&bytes)?;
    let cpu = start.zip(if trace { crate::loading::thread_cycles() } else { None }).map(|(a,b)| b.saturating_sub(a));
    if trace {
        let mut crc = flate2::Crc::new(); crc.update(&bytes);
        skeleton.audit = Some(Audit { source_crc: crc.sum(), source_bytes: bytes.len(), identity: 0, logical_bytes: 0 });
        eprintln!("SKELETON_DECODE path={path} read_parse_cycles={cpu:?}");
    }
    Ok(skeleton)
}
fn report(path: &str, mode: &str, cycles: Option<u64>, skeleton: &Arc<EdgeSkeleton>) {
    let Some(start) = cycles else { return };
    let cpu = crate::loading::thread_cycles().map(|end| end.saturating_sub(start));
    let (core, names, inverse, logical_bytes) = sizes(skeleton);
    eprintln!("SKELETON_ACQUIRE path={path} result={mode} thread_cycles={cpu:?} identity={:p} source_crc={:?} source_bytes={:?} core_bytes={core} names_bytes={names} inverse_bind_bytes={inverse} logical_bytes={logical_bytes} tick={:?}",
        Arc::as_ptr(skeleton), skeleton.source_audit().map(|x| x.0), skeleton.source_audit().map(|x| x.1), crate::platform::performance_counter::counter().ok());
}
pub(crate) fn acquire(catalog: &Catalog, path: &str) -> Result<Arc<EdgeSkeleton>, String> {
    let trace = tracing();
    let cycles = trace.then(crate::loading::thread_cycles).flatten();
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    if catalog.has_overlays() || separate() {
        let skeleton = wrap(load(catalog, &normalized)?);
        report(&normalized, if catalog.has_overlays() { "overlay" } else { "separate" }, cycles, &skeleton);
        return Ok(skeleton);
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
            State::Ready { weak, handoff, pending } => if let Some(skeleton) = weak.upgrade() {
                if waited && *pending > 0 { *pending -= 1; if *pending == 0 { handoff.take(); } }
                drop(state); report(&normalized, if waited { "wait" } else { "reuse" }, cycles, &skeleton); return Ok(skeleton);
            },
            State::Loading { waiters } => { if !waited { *waiters += 1; } waited = true; state = entry.ready.wait(state).unwrap_or_else(|e| e.into_inner()); continue; }
            State::Failed { error, pending } => {
                let error = error.clone(); if waited && *pending > 0 { *pending -= 1; } if *pending == 0 { *state = State::Empty; }
                drop(state); discard_empty(&key, &entry); return Err(error);
            }
            State::Empty => {}
        }
        *state = State::Loading { waiters: 0 }; drop(state);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut skeleton = load(catalog, &normalized)?;
            skeleton.residency = Some(Retire { key: key.clone(), entry: Arc::downgrade(&entry) });
            Ok(wrap(skeleton))
        })).unwrap_or_else(|_| Err(format!("skeleton acquisition panicked for {normalized}")));
        let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
        let waiters = match &*state { State::Loading { waiters } => *waiters, _ => 0 };
        *state = match &result {
            Ok(skeleton) => State::Ready { weak: Arc::downgrade(skeleton), handoff: (waiters > 0).then(|| skeleton.clone()), pending: waiters },
            Err(error) if waiters > 0 => State::Failed { error: error.clone(), pending: waiters }, Err(_) => State::Empty,
        };
        entry.ready.notify_all(); drop(state);
        if let Ok(skeleton) = &result { report(&normalized, "decode", cycles, skeleton); } else { discard_empty(&key, &entry); }
        return result;
    }
}
