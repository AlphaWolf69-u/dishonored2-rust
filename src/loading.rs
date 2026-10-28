//! Level loading: what the original loads before play, what the rewrite
//! loads of it, and when the level is ready.
//!
//! The map load (`FUN_1400661e0`, `ResourceManager.cpp`) loads every entry
//! of the `init`, `common` and map lists ([`crate::assets::map_resources`])
//! by type, with their dependencies, before play. The rewrite mirrors that:
//!
//! - Every installed resource read through `Catalog::read` is recorded with
//!   its index kind ([`read`]); [`report`] compares those reads with the
//!   lists per kind, so what the rewrite does not load yet shows as data. A
//!   read after play started ([`play_started`]) is a late load and is logged
//!   (`DH2_DEBUG_LOAD_TIMING=1`), except the texture streamer's `_mipN`
//!   files, which the original also reads while playing.
//! - Work that finishes after startup (GPU uploads, worker threads,
//!   first-frame preparation) registers a [`gate`] while its plugin is built
//!   and calls [`complete`] when done, also when it fails. The level is ready
//!   when every gate is complete; the loading screen holds until then.
//!
//! Adding a system: read its resources through the catalog during startup
//! or behind a gate, and never lazily during play.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
struct State {
    loaded: HashMap<String, HashSet<String>>,
    gates: Vec<(&'static str, bool)>,
    playing: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(STATE.lock().unwrap().get_or_insert_with(State::default))
}

/// `Catalog::read` read `path`, a resource of index kind `kind`.
pub fn read(kind: &str, path: &str) {
    let path = path.to_ascii_lowercase().replace('\\', "/");
    let late = with(|s| {
        let late = s.playing && !s.loaded.get(kind).is_some_and(|paths| paths.contains(&path));
        s.loaded.entry(kind.to_owned()).or_default().insert(path.clone());
        late
    });
    if late && !path.contains(".bimage7_mip") && std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        eprintln!("LOAD late {kind}: {path}");
    }
}

/// The level is shown: later first reads are late loads.
pub fn play_started() {
    with(|s| s.playing = true);
}

/// Register work the level must finish before play.
pub fn gate(name: &'static str) {
    with(|s| {
        if !s.gates.iter().any(|g| g.0 == name) {
            s.gates.push((name, false));
        }
    })
}

pub fn complete(name: &'static str) {
    with(|s| match s.gates.iter_mut().find(|g| g.0 == name) {
        Some(gate) => gate.1 = true,
        None => s.gates.push((name, true)),
    })
}

/// Gates still open.
pub fn pending() -> Vec<&'static str> {
    with(|s| s.gates.iter().filter(|g| !g.1).map(|g| g.0).collect())
}

pub fn ready() -> bool {
    pending().is_empty()
}

/// One kind of the load lists against what the rewrite loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    pub kind: String,
    pub listed: usize,
    pub loaded: usize,
    /// Loaded resources of this kind the lists do not name (dependencies).
    pub unlisted: usize,
}

/// Per kind of `list`: how many entries the rewrite loaded, least covered first.
pub fn report(list: &[crate::assets::map_resources::Resource]) -> Vec<Coverage> {
    let mut listed: HashMap<&str, HashSet<&str>> = HashMap::new();
    for r in list {
        listed.entry(r.kind.as_str()).or_default().insert(r.path.as_str());
    }
    with(|s| {
        let mut kinds: HashSet<&str> = listed.keys().copied().collect();
        kinds.extend(s.loaded.keys().map(String::as_str));
        let mut out: Vec<Coverage> = kinds
            .into_iter()
            .map(|kind| {
                let paths = listed.get(kind);
                let done = s.loaded.get(kind);
                let loaded = match (paths, done) {
                    (Some(p), Some(d)) => p.iter().filter(|x| d.contains(**x)).count(),
                    _ => 0,
                };
                let unlisted = done.map_or(0, |d| d.iter().filter(|x| paths.is_none_or(|p| !p.contains(x.as_str()))).count());
                Coverage { kind: kind.to_owned(), listed: paths.map_or(0, HashSet::len), loaded, unlisted }
            })
            .collect();
        out.sort_by(|a, b| (b.listed - b.loaded).cmp(&(a.listed - a.loaded)).then(a.kind.cmp(&b.kind)));
        out
    })
}
