//! Opt-in observation of the real UI worker and its submitted resource leases.
//! No synthetic movies, request pauses, replacement views or strong asset roots.
//! The source boundaries are Iggy's named-view callback 1402df8c0 and movie
//! teardown 1410298b0. The host worker/reply transport is a backend mechanism.
use crate::flash::{
    bitmap_resources::MovieBitmaps,
    iggy_functions::{BoundTexture, TextureBinding},
    render::DrawList,
};
use crate::rendering::bevy_backend::flash_ui::FlashFrame;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock, Weak},
};

const HISTORY: usize = 128;
static OBSERVER: OnceLock<Mutex<State>> = OnceLock::new();

#[derive(Clone, Debug)]
pub(super) struct Pending {
    pub id: u64,
    pub name: String,
    /// A real reply has been placed in the worker's channel, but may not yet
    /// have been consumed. This differs from an outstanding acquisition.
    pub reply_sent: bool,
}

#[derive(Clone, Default, Debug)]
pub(super) struct Snapshot {
    pub requests: u64,
    pub replies: u64,
    pub reply_failures: u64,
    pub received: u64,
    pub acquisition_errors: u64,
    pub pending: Option<Pending>,
    pub cancellation_wakes: u64,
    pub worker_starts: u64,
    pub worker_exits: u64,
    pub worker_joins: u64,
    pub worker_panics: u64,
    pub level_epochs: u64,
    /// Commands acknowledged by the actual worker, including already empty
    /// owners. Movie releases and weak leases are reported independently.
    pub pause_closes: u64,
    pub loading_closes: u64,
    pub modal_clears: u64,
    pub movie_opens: u64,
    pub movie_releases: u64,
    pub published_frames: u64,
    pub published_named_frames: u64,
    pub live_bindings: usize,
    pub live_lists: usize,
    pub live_bitmap_maps: usize,
    /// Weak observations are bounded; overflow makes closure inconclusive.
    pub observation_window: usize,
    pub weak_overflow: u64,
    pub events: Vec<String>,
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    events: VecDeque<String>,
    bindings: VecDeque<Weak<TextureBinding>>,
    lists: VecDeque<Weak<DrawList>>,
    bitmaps: VecDeque<Weak<MovieBitmaps>>,
}

/// Enable before constructing the ordinary runtime. Never changes scheduling.
pub(super) fn enable() {
    OBSERVER.get_or_init(Default::default);
}
pub(super) fn enabled() -> bool {
    OBSERVER.get().is_some()
}

/// Cheap milestone selector for the real Last-system observer. Unlike a full
/// snapshot, this does not walk weak histories or clone the event log.
pub(super) fn pending_request() -> Option<Pending> {
    let observer = OBSERVER.get()?;
    observer
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .snapshot
        .pending
        .clone()
}

fn observe(action: impl FnOnce(&mut State)) {
    if let Some(observer) = OBSERVER.get() {
        action(&mut observer.lock().unwrap_or_else(|error| error.into_inner()));
    }
}
fn retain<T>(entries: &mut VecDeque<Weak<T>>, value: &Arc<T>) -> bool {
    entries.retain(|entry| entry.strong_count() != 0);
    if entries
        .iter()
        .any(|entry| entry.as_ptr() == Arc::as_ptr(value))
    {
        return true;
    }
    if entries.len() == HISTORY {
        return false;
    }
    entries.push_back(Arc::downgrade(value));
    true
}
fn event(state: &mut State, value: String) {
    if state.events.len() == HISTORY {
        state.events.pop_front();
    }
    state.events.push_back(value);
}

pub(super) fn snapshot() -> Option<Snapshot> {
    let observer = OBSERVER.get()?;
    let state = observer.lock().unwrap_or_else(|error| error.into_inner());
    let mut result = state.snapshot.clone();
    result.live_bindings = state
        .bindings
        .iter()
        .filter(|entry| entry.strong_count() != 0)
        .count();
    result.live_lists = state
        .lists
        .iter()
        .filter(|entry| entry.strong_count() != 0)
        .count();
    result.live_bitmap_maps = state
        .bitmaps
        .iter()
        .filter(|entry| entry.strong_count() != 0)
        .count();
    result.observation_window = HISTORY;
    result.events = state.events.iter().cloned().collect();
    Some(result)
}

pub(super) fn request(name: &str) -> u64 {
    let Some(observer) = OBSERVER.get() else {
        return 0;
    };
    let mut state = observer.lock().unwrap_or_else(|error| error.into_inner());
    state.snapshot.requests += 1;
    let id = state.snapshot.requests;
    state.snapshot.pending = Some(Pending {
        id,
        name: name.into(),
        reply_sent: false,
    });
    event(&mut state, format!("request {id} {name}"));
    id
}
pub(super) fn reply(name: &str, result: &Result<BoundTexture, String>) {
    observe(|state| {
        if let Ok(binding) = result {
            retain(&mut state.bindings, binding);
        }
        event(
            state,
            format!("reply-prepared {name} ok={}", result.is_ok()),
        );
    });
}
pub(super) fn reply_sent(id: u64, sent: bool) {
    observe(|state| {
        if sent {
            state.snapshot.replies += 1;
        } else {
            state.snapshot.reply_failures += 1;
        }
        if let Some(pending) = state
            .snapshot
            .pending
            .as_mut()
            .filter(|pending| pending.id == id)
        {
            pending.reply_sent = sent;
        }
    });
}
pub(super) fn received(id: u64, ok: bool) {
    observe(|state| {
        state.snapshot.received += 1;
        state.snapshot.acquisition_errors += u64::from(!ok);
        if state
            .snapshot
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            state.snapshot.pending = None;
        }
        event(state, format!("received {id} ok={ok}"));
    });
}
pub(super) fn cancellation_wake(sent: bool) {
    observe(|state| {
        state.snapshot.cancellation_wakes += u64::from(sent);
        event(state, format!("session-cancel-wake sent={sent}"));
    });
}
pub(super) struct WorkerLifetime;
pub(super) fn worker_started() -> WorkerLifetime {
    observe(|state| {
        state.snapshot.worker_starts += 1;
        event(state, "worker-start".into());
    });
    WorkerLifetime
}
impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        observe(|state| {
            state.snapshot.worker_exits += 1;
            event(state, "worker-exit".into());
        });
    }
}
pub(super) fn joined(panicked: bool) {
    observe(|state| {
        state.snapshot.worker_joins += 1;
        state.snapshot.worker_panics += u64::from(panicked);
        event(state, format!("worker-joined panicked={panicked}"));
    });
}
pub(super) fn boundary(kind: &'static str, epoch: u64) {
    observe(|state| {
        match kind {
            "level-epoch" => state.snapshot.level_epochs += 1,
            "pause-closed" => state.snapshot.pause_closes += 1,
            "loading-closed" => state.snapshot.loading_closes += 1,
            "modal-cleared" => state.snapshot.modal_clears += 1,
            _ => {}
        }
        event(state, format!("{kind} epoch={epoch}"));
    });
}
pub(super) fn movie(label: &'static str, open: bool) {
    observe(|state| {
        if open {
            state.snapshot.movie_opens += 1;
        } else {
            state.snapshot.movie_releases += 1;
        }
        event(
            state,
            format!("movie-{} {label}", if open { "open" } else { "released" }),
        );
    });
}
pub(super) fn frame(frame: &FlashFrame) {
    observe(|state| {
        state.snapshot.published_frames += 1;
        state.snapshot.published_named_frames += u64::from(!frame.list.named_images.is_empty());
        state.snapshot.weak_overflow += u64::from(!retain(&mut state.lists, &frame.list));
        state.snapshot.weak_overflow += u64::from(!retain(&mut state.bitmaps, &frame.images));
        for binding in frame.list.named_images.values() {
            state.snapshot.weak_overflow += u64::from(!retain(&mut state.bindings, binding));
        }
    });
}


/// A bounded weak checkpoint of real live movie/frame ownership before a map
/// command. These weak references survive registry history pruning without
/// pinning the outgoing movie, image wrapper or submitted draw list.
struct LeaseCheckpoint {
    bindings: Vec<Weak<TextureBinding>>,
    lists: Vec<Weak<DrawList>>,
    bitmaps: Vec<Weak<MovieBitmaps>>,
}
impl LeaseCheckpoint {
    fn live(&self) -> (usize, usize, usize) {
        (self.bindings.iter().filter(|lease| lease.strong_count() != 0).count(),
         self.lists.iter().filter(|lease| lease.strong_count() != 0).count(),
         self.bitmaps.iter().filter(|lease| lease.strong_count() != 0).count())
    }
}
struct Transition {
    target: String,
    before: Snapshot,
    leases: LeaseCheckpoint,
    saw_loading: bool,
}
/// Diagnostic orchestration only: the actual loading owner accepts every map
/// command, and the ordinary full application owns clocks, UI, IO and rendering.
/// No movie reply, loading acknowledgement or scene image reference is supplied.
pub(super) struct TransitionObservation {
    next: VecDeque<String>,
    pending: Option<Transition>,
    completed: usize,
    worker_starts: Option<u64>,
}
impl TransitionObservation {
    pub(super) fn new(initial: &str, middle: Option<&str>) -> Self {
        Self {
            next: [middle.unwrap_or("dust_p_lowchaos").to_owned(), initial.to_owned()].into(),
            pending: None, completed: 0, worker_starts: None,
        }
    }
    pub(super) fn poll(&mut self, playing: bool) -> Result<bool, String> {
        let observer = OBSERVER.get().ok_or("UI lifecycle observation is not enabled")?;
        // The stored counters contain no event log or strong resources. Read
        // scalars here; clone bounded events only at the actual milestones below.
        let now = observer.lock().unwrap_or_else(|error| error.into_inner()).snapshot.clone();
        if let Some(expected) = self.worker_starts {
            if now.worker_starts != expected || now.worker_exits != expected - 1 || now.worker_joins != expected - 1 {
                return Err(format!("UI worker changed during live map transition: starts={} exits={} joins={} expected_live_starts={expected}", now.worker_starts, now.worker_exits, now.worker_joins));
            }
        }
        if let Some(pending) = &mut self.pending {
            if !playing && now.level_epochs > pending.before.level_epochs { pending.saw_loading = true; }
            let acknowledged = now.level_epochs > pending.before.level_epochs
                && now.modal_clears > pending.before.modal_clears
                && now.pause_closes > pending.before.pause_closes
                && now.loading_closes > pending.before.loading_closes;
            if !playing || !pending.saw_loading || !acknowledged
                || now.published_named_frames <= pending.before.published_named_frames { return Ok(false); }
            let live = pending.leases.live();
            if live != (0, 0, 0) { return Ok(false); }
            self.completed += 1;
            eprintln!("RUNTIME_UI_TRANSITION phase=replaced target={} completed={} old_live_bindings_lists_bitmaps={live:?} snapshot={:?}", pending.target, self.completed, snapshot());
            self.pending = None;
        }
        if !playing { return Ok(false); }
        let Some(target) = self.next.front() else { return Ok(self.completed == 2); };
        let (before, leases) = {
            let state = observer.lock().unwrap_or_else(|error| error.into_inner());
            let leases = LeaseCheckpoint {
                bindings: state.bindings.iter().filter(|lease| lease.strong_count() != 0).cloned().collect(),
                lists: state.lists.iter().filter(|lease| lease.strong_count() != 0).cloned().collect(),
                bitmaps: state.bitmaps.iter().filter(|lease| lease.strong_count() != 0).cloned().collect(),
            };
            (state.snapshot.clone(), leases)
        };
        // Observe an actual movie frame before withdrawing it; merely loading
        // a level or constructing a movie is insufficient for this scenario.
        if leases.live().1 == 0 || before.published_named_frames == 0 || before.pending.is_some() { return Ok(false); }
        if before.worker_starts == 0 || before.worker_starts != before.worker_exits + 1 || before.worker_starts != before.worker_joins + 1 {
            return Err("UI transition requires the one actual live worker".into());
        }
        self.worker_starts.get_or_insert(before.worker_starts);
        eprintln!("RUNTIME_UI_TRANSITION phase=map-command target={target} old_live_bindings_lists_bitmaps={:?} snapshot={:?}", leases.live(), snapshot());
        super::loading_screen::request_level(target.clone());
        let target = self.next.pop_front().expect("front was present");
        self.pending = Some(Transition { target, before, leases, saw_loading: false });
        Ok(false)
    }
    pub(super) fn observe_host_drop(&self) -> Result<(), String> {
        let final_state = snapshot().ok_or("UI lifecycle observation is absent after host drop")?;
        eprintln!("RUNTIME_UI_TRANSITION phase=host-dropped completed={} snapshot={final_state:?}", self.completed);
        if self.completed != 2 || self.pending.is_some() || !self.next.is_empty() {
            return Err(format!("UI map transition sequence incomplete: completed={}", self.completed));
        }
        validate_host_drop()
    }
}

/// Check actual App destruction, not merely an acknowledged AppExit request.
/// A destructor cancellation may supply the final reply instead of the image
/// service, so acquisition-error/reply counts are reported but not fabricated
/// into a required successful image read during shutdown.
pub(super) fn validate_host_drop() -> Result<(), String> {
    let state = snapshot().ok_or("UI lifecycle observation is absent after host drop")?;
    if state.pending.is_some() || state.requests != state.received
        || state.worker_starts == 0
        || state.worker_starts != state.worker_exits
        || state.worker_starts != state.worker_joins
        || state.worker_panics != 0 || state.movie_opens != state.movie_releases || state.weak_overflow != 0
        || state.live_bindings != 0 || state.live_lists != 0 || state.live_bitmap_maps != 0 {
        return Err(format!("actual UI host destruction left worker or observed movie leases: {state:?}"));
    }
    Ok(())
}
