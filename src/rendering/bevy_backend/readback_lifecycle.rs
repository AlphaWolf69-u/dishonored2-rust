//! Opt-in observation of real readback owners; no strong leases or GPU work.
use std::{
    any::Any,
    sync::{Arc, LazyLock, Mutex, Weak},
};
const LIMIT: usize = 64;
static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var("DH2_DEBUG_READBACK_LIFETIME").as_deref() == Ok("1"));
static STATE: Mutex<Trace> = Mutex::new(Trace {
    entries: Vec::new(),
    counts: [Counts::ZERO; 2],
    overflow: 0,
});
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    CoarseDepth,
    OceanHeight,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Counts {
    pub created: u64,
    pub dropped: u64,
    pub map_started: u64,
    pub map_completed: u64,
    pub map_failed: u64,
    pub callbacks_after_drop: u64,
    pub pending_on_drop: u64,
    pub ready_on_drop: u64,
}
impl Counts {
    const ZERO: Self = Self {
        created: 0,
        dropped: 0,
        map_started: 0,
        map_completed: 0,
        map_failed: 0,
        callbacks_after_drop: 0,
        pending_on_drop: 0,
        ready_on_drop: 0,
    };
}
struct Entry {
    kind: Kind,
    address: usize,
    state: Weak<dyn Any + Send + Sync>,
    dropped: bool,
}
struct Trace {
    entries: Vec<Entry>,
    counts: [Counts; 2],
    overflow: u64,
}
fn address<T>(state: &Arc<T>) -> usize {
    Arc::as_ptr(state).cast::<()>() as usize
}
fn index(kind: Kind) -> usize {
    match kind {
        Kind::CoarseDepth => 0,
        Kind::OceanHeight => 1,
    }
}
/// One bounded Weak per actual staging-state allocation. Dead records are
/// reclaimed before adding another; overflow is explicit, never false success.
pub(super) fn register<T: Any + Send + Sync>(kind: Kind, state: &Arc<T>) {
    if !*ENABLED {
        return;
    }
    let mut trace = STATE.lock().unwrap_or_else(|e| e.into_inner());
    trace.counts[index(kind)].created += 1;
    trace
        .entries
        .retain(|entry| entry.state.strong_count() != 0);
    if trace.entries.len() == LIMIT {
        trace.overflow += 1;
        return;
    }
    let weak: Weak<T> = Arc::downgrade(state);
    let erased: Weak<dyn Any + Send + Sync> = weak;
    trace.entries.push(Entry {
        kind,
        address: address(state),
        state: erased,
        dropped: false,
    });
}
pub(super) fn map_started<T>(kind: Kind, _: &Arc<T>) {
    if !*ENABLED {
        return;
    }
    STATE.lock().unwrap_or_else(|e| e.into_inner()).counts[index(kind)].map_started += 1;
}
pub(super) fn callback_completed<T>(kind: Kind, state: &Arc<T>, success: bool) {
    if !*ENABLED {
        return;
    }
    let mut trace = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let dropped = trace.entries.iter().any(|entry| {
        index(entry.kind) == index(kind) && entry.address == address(state) && entry.dropped
    });
    let counts = &mut trace.counts[index(kind)];
    counts.map_completed += 1;
    counts.map_failed += u64::from(!success);
    counts.callbacks_after_drop += u64::from(dropped);
}
pub(super) fn owner_dropped<T>(kind: Kind, state: &Arc<T>, pending: bool, ready: bool) {
    if !*ENABLED {
        return;
    }
    let mut trace = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = trace
        .entries
        .iter_mut()
        .find(|entry| index(entry.kind) == index(kind) && entry.address == address(state))
    {
        entry.dropped = true;
    }
    let counts = &mut trace.counts[index(kind)];
    counts.dropped += 1;
    counts.pending_on_drop += u64::from(pending);
    counts.ready_on_drop += u64::from(ready);
    eprintln!(
        "READBACK_OWNER_DROP kind={kind:?} pending={pending} ready={ready} strong_before_field_drop={}",
        Arc::strong_count(state)
    );
}
#[derive(Debug)]
pub(crate) struct Snapshot {
    pub enabled: bool,
    pub coarse: Counts,
    pub ocean: Counts,
    pub tracked_states: usize,
    pub live_owner_states: usize,
    pub live_retired_states: usize,
    pub retained_strong_after_owner_drop: usize,
    pub overflow: u64,
}
/// Read after real render retirement/device callback processing and after App
/// Drop. This never polls a device, completes a map or retains its source state.
pub(crate) fn snapshot() -> Snapshot {
    let mut trace = STATE.lock().unwrap_or_else(|e| e.into_inner());
    trace
        .entries
        .retain(|entry| entry.state.strong_count() != 0);
    Snapshot {
        enabled: *ENABLED,
        coarse: trace.counts[0],
        ocean: trace.counts[1],
        tracked_states: trace.entries.len(),
        live_owner_states: trace.entries.iter().filter(|entry| !entry.dropped).count(),
        live_retired_states: trace.entries.iter().filter(|entry| entry.dropped).count(),
        retained_strong_after_owner_drop: trace
            .entries
            .iter()
            .filter(|entry| entry.dropped)
            .map(|entry| entry.state.strong_count())
            .sum(),
        overflow: trace.overflow,
    }
}
