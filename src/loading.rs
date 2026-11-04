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
    required: std::sync::Arc<Vec<crate::assets::map_resources::Resource>>,
    required_heads: HashMap<u64, usize>,
    required_next: Vec<usize>,
    acquired: usize,
    requested: usize,
}

fn resource_key(kind: &str, path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    kind.hash(&mut hash);
    path.hash(&mut hash);
    hash.finish()
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
        if s.loaded.entry(kind.to_owned()).or_default().insert(path.clone()) {
            let mut index = s.required_heads.get(&resource_key(kind, &path)).copied().unwrap_or(usize::MAX);
            while index != usize::MAX {
                let required = &s.required[index];
                if required.kind == kind && required.path == path { s.acquired += 1; }
                index = s.required_next[index];
            }
        }
        late
    });
    if late && !path.contains(".bimage7_mip") && std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        eprintln!("LOAD late {kind}: {path}");
    }
}

/// The level is shown: later first reads are late loads.
pub fn play_started() {
    with(|s| {
        s.playing = true;
        // 1400661e0 frees the temporary acquisition table after loading.
        // Progress is now 1; late-load reporting only needs the read registry.
        if std::env::var("DH2_DEBUG_RETAIN_LEVEL_MANIFESTS").as_deref() != Ok("1") {
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY loading descriptor release: entries={} descriptor_bytes={} owners={} index_capacity={}",
                    s.required.len(), crate::assets::map_resources::descriptor_bytes(&s.required),
                    std::sync::Arc::strong_count(&s.required), s.required_heads.capacity() + s.required_next.capacity());
            }
            s.required = Default::default();
            s.required_heads = Default::default();
            s.required_next = Default::default();
        }
    });
}

/// Re-arm per-world readiness while retaining the session acquisition registry.
pub fn restart_level() {
    with(|s| {
        s.playing = false;
        s.required = Default::default();
        s.required_heads.clear();
        s.required_next.clear();
        s.acquired = 0;
        s.requested = 0;
        for (_, ready) in &mut s.gates { *ready = false; }
    });
}

/// Start the map resource phase while keeping registry/resident identities.
/// 1400661e0 reports resources acquired / resources requested; streaming mips
/// remain outside this progress count.
pub fn begin_level(list: std::sync::Arc<Vec<crate::assets::map_resources::Resource>>, catalog: &crate::assets::archive::Catalog) {
    with(|s| {
        s.playing = false;
        s.required = list;
        s.required_heads.clear();
        s.required_next.clear();
        s.acquired = 0;
        s.requested = 0;
        for (index, resource) in s.required.iter().enumerate() {
            if !catalog.preloads(&resource.path) {
                s.required_next.push(usize::MAX);
                continue;
            }
            s.requested += 1;
            let previous = s.required_heads.insert(resource_key(&resource.kind, &resource.path), index).unwrap_or(usize::MAX);
            s.required_next.push(previous);
            if s.loaded.get(&resource.kind).is_some_and(|paths| paths.contains(&resource.path)) { s.acquired += 1; }
        }
    });
}

pub fn progress() -> f64 {
    with(|s| {
        if s.playing { return 1.0; }
        if s.requested == 0 { return 0.0; }
        s.acquired as f64 / s.requested as f64
    })
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

#[cfg(test)]
mod tests {
    /// Kinds of `docs/resource-parity.md`'s ledger table (rows `| \`kind\` |`).
    fn ledger_kinds() -> std::collections::HashSet<String> {
        let ledger = include_str!("../docs/resource-parity.md");
        ledger
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|rest| rest.split_once('`').map(|(kind, _)| kind.to_owned()))
            .collect()
    }

    /// Every resource kind in the installed load lists of every map (init,
    /// common and each map's `.mapResources`) has a ledger entry.
    #[test]
    #[ignore = "requires installed game archives"]
    fn every_listed_kind_has_a_ledger_entry() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
        let language = crate::assets::lang::read_text_language(&base).unwrap();
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for path in catalog.paths_with_suffix(".mapresources") {
            let name = path.trim_start_matches("generated/buildgame/").trim_end_matches(".mapresources");
            for resource in crate::assets::map_resources::load_list(&base, &catalog, name, language).unwrap() {
                *kinds.entry(resource.kind).or_default() += 1;
            }
        }
        let ledger = ledger_kinds();
        let covered = |kind: &str| ledger.contains(kind) || ledger.iter().any(|l| l.strip_suffix('*').is_some_and(|prefix| kind.starts_with(prefix)));
        let missing: Vec<_> = kinds.iter().filter(|(k, _)| !covered(k)).collect();
        assert!(missing.is_empty(), "load-list kinds without a ledger entry (kind, entries in all lists): {missing:?}");
    }
}
