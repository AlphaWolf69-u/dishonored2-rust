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
    /// Weak lease observations are bounded; these are live entries in the
    /// retained window, not a claim about every resource in the session.
    pub observation_window: usize,
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
fn retain<T>(entries: &mut VecDeque<Weak<T>>, value: &Arc<T>) {
    entries.retain(|entry| entry.strong_count() != 0);
    if entries
        .iter()
        .any(|entry| entry.as_ptr() == Arc::as_ptr(value))
    {
        return;
    }
    if entries.len() == HISTORY {
        entries.pop_front();
    }
    entries.push_back(Arc::downgrade(value));
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
        retain(&mut state.lists, &frame.list);
        retain(&mut state.bitmaps, &frame.images);
        for binding in frame.list.named_images.values() {
            retain(&mut state.bindings, binding);
        }
    });
}
