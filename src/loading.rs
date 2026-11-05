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
    // Paths are immutable after acquisition. A boxed slice preserves the
    // string payload while avoiding an unused capacity word in every bucket.
    loaded: HashMap<String, HashSet<Box<str>>>,
    common_texts: Option<(u64, std::sync::Arc<crate::assets::declaration_texts::TextMap>)>,
    common_exceptions: HashMap<String, Option<&'static str>>,
    gates: Vec<(&'static str, bool)>,
    playing: bool,
    required: std::sync::Arc<crate::assets::map_resources::LevelList>,
    required_heads: HashMap<u64, usize>,
    required_next: Vec<usize>,
    acquired: usize,
    requested: usize,
    history_metrics: HistoryMetrics,
}

// Generated declaration prefixes are a fast path. The recorded catalog kind
// remains authoritative for paths that do not follow that convention.
fn inferred_common_kind(path: &str) -> Option<&'static str> {
    if path.starts_with("generated/decls/fx/") { Some("fx") }
    else if path.starts_with("generated/decls/soundevent/") { Some("soundevent") }
    else if path.starts_with("generated/decls/physicsmaterial/") { Some("physicsMaterial") }
    else { None }
}
fn common_kind(exceptions: &HashMap<String, Option<&'static str>>, path: &str) -> Option<&'static str> {
    if exceptions.is_empty() { inferred_common_kind(path) }
    else { exceptions.get(path).copied().unwrap_or_else(|| inferred_common_kind(path)) }
}
impl State {
    fn common_kind(&self, path: &str) -> Option<&'static str> { common_kind(&self.common_exceptions, path) }
    fn contains(&self, kind: &str, path: &str) -> bool {
        self.common_texts.as_ref().is_some_and(|(_, texts)|
            matches!(kind, "fx" | "soundevent" | "physicsMaterial") && self.common_kind(path) == Some(kind) && texts.contains_key(path))
            || self.loaded.get(kind).is_some_and(|paths| paths.contains(path))
    }
    fn adjust_required(&mut self, kind: &str, path: &str, add: bool) {
        let mut index = self.required_heads.get(&resource_key(kind, path)).copied().unwrap_or(usize::MAX);
        while index != usize::MAX {
            let required = &self.required[index];
            if required.kind == kind && required.path == path {
                if add { self.acquired += 1; } else { self.acquired = self.acquired.saturating_sub(1); }
            }
            index = self.required_next[index];
        }
    }
    fn replace_common(&mut self, catalog: u64, common: Option<std::sync::Arc<crate::assets::declaration_texts::TextMap>>) {
        if let (Some((id, old)), Some(new)) = (&self.common_texts, &common) {
            if *id == catalog && std::sync::Arc::ptr_eq(old, new) { return; }
        }
        let acquired_before = self.acquired;
        let old_exceptions = std::mem::take(&mut self.common_exceptions);
        if let Some((_, old)) = self.common_texts.take() {
            if !self.required_heads.is_empty() {
                for path in old.keys() {
                    if let Some(kind) = common_kind(&old_exceptions, path) {
                        if !self.loaded.get(kind).is_some_and(|paths| paths.contains(path.as_str())) {
                            self.adjust_required(kind, path, false);
                        }
                    }
                }
            }
        }
        if let Some(common) = common {
            let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
            let start = trace.then(thread_cycles).flatten();
            let mut removed = 0usize;
            let mut bytes = 0usize;
            let slots_before: usize = if trace { ["fx", "soundevent", "physicsMaterial"].iter().filter_map(|kind| self.loaded.get(*kind)).map(HashSet::capacity).sum() } else { 0 };
            for path in common.keys() {
                let inferred = inferred_common_kind(path);
                let mut kind = inferred;
                let mut duplicate = inferred.and_then(|kind| self.loaded.get_mut(kind).and_then(|paths| paths.take(path.as_str())));
                if duplicate.is_none() {
                    let actual = self.loaded.iter().find_map(|(kind, paths)| paths.contains(path.as_str()).then_some(kind.as_str()));
                    match actual {
                        Some("fx") => kind = Some("fx"),
                        Some("soundevent") => kind = Some("soundevent"),
                        Some("physicsMaterial") => kind = Some("physicsMaterial"),
                        Some(_) => kind = None,
                        None => {},
                    }
                    if kind != inferred { self.common_exceptions.insert(path.clone(), kind); }
                    if let Some(kind) = kind { duplicate = self.loaded.get_mut(kind).and_then(|paths| paths.take(path.as_str())); }
                }
                let Some(kind) = kind else { continue; };
                if let Some(path) = duplicate { removed += 1; bytes += path.len(); }
                else if !self.required_heads.is_empty() { self.adjust_required(kind, path, true); }
            }
            for kind in ["fx", "soundevent", "physicsMaterial"] {
                if self.loaded.get(kind).is_some_and(HashSet::is_empty) { self.loaded.remove(kind); }
                else if let Some(paths) = self.loaded.get_mut(kind) { paths.shrink_to_fit(); }
            }
            let slots_after: usize = if trace { ["fx", "soundevent", "physicsMaterial"].iter().filter_map(|kind| self.loaded.get(*kind)).map(HashSet::capacity).sum() } else { 0 };
            self.common_texts = Some((catalog, common));
            if trace {
                let cycles = start.zip(thread_cycles()).map_or(0, |(a,b)| b.saturating_sub(a));
                eprintln!("MEMORY common declaration history handoff: duplicate_paths={removed} path_capacity_bytes={bytes} slots_before={slots_before} slots_after={slots_after} acquired_before={acquired_before} acquired_after={} cycles={cycles}", self.acquired);
            }
        }
    }
}

#[derive(Default)]
struct HistoryMetrics {
    reads: usize,
    borrowed_paths: usize,
    inserted: usize,
    new_kinds: usize,
    normalizations: usize,
    kind_allocations: usize,
    cycle_reads: usize,
    cycles: u64,
}
fn history_modes() -> (bool, bool) {
    static MODES: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    *MODES.get_or_init(|| (
        std::env::var("DH2_DEBUG_SEPARATE_READ_HISTORY").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_READ_HISTORY_METRICS").as_deref() == Ok("1"),
    ))
}

/// Diagnostic only: CPU cycles consumed by this thread in the affected code.
#[cfg(windows)]
pub(crate) fn thread_cycles() -> Option<u64> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0;
    // The pseudo-handle and output pointer remain valid for this call.
    (unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) } != 0).then_some(cycles)
}
#[cfg(not(windows))]
pub(crate) fn thread_cycles() -> Option<u64> { None }

fn resource_key(kind: &str, path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    kind.hash(&mut hash);
    path.hash(&mut hash);
    hash.finish()
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Attach the actual session text owner, never just manifest membership.
pub(crate) fn set_common_texts(catalog: u64, common: Option<std::sync::Arc<crate::assets::declaration_texts::TextMap>>) {
    with(|state| state.replace_common(catalog, common));
}
/// A different catalog cannot inherit credits from the former text owner.
pub(crate) fn check_common_catalog(catalog: u64) {
    with(|state| {
        if state.common_texts.as_ref().is_some_and(|(id, _)| *id != catalog) {
            state.replace_common(catalog, None);
        }
    });
}


fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(STATE.lock().unwrap().get_or_insert_with(State::default))
}

/// `Catalog::read` read `path`, a resource of index kind `kind`.
pub fn read(kind: &str, path: &str) {
    use std::borrow::Cow;
    let (separate, trace) = history_modes();
    let cycle_start = if trace { thread_cycles() } else { None };
    let borrowed_path = !separate && path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase());
    let path: Cow<'_, str> = if !borrowed_path {
        Cow::Owned(path.to_ascii_lowercase().replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    };
    let normalized: &str = path.as_ref();
    let late = with(|s| {
        let shared = s.common_texts.as_ref().is_some_and(|(_, texts)|
            matches!(kind, "fx" | "soundevent" | "physicsMaterial") && s.common_kind(normalized) == Some(kind) && texts.contains_key(normalized));
        let (inserted, new_kind) = if shared { (false, false) } else if separate {
            // Diagnostic control: reproduce the former per-read key allocations.
            use std::collections::hash_map::Entry;
            match s.loaded.entry(kind.to_owned()) {
                Entry::Occupied(mut entry) => (entry.get_mut().insert(Box::<str>::from(path)), false),
                Entry::Vacant(entry) => {
                    let mut paths = HashSet::new();
                    let inserted = paths.insert(Box::<str>::from(path));
                    entry.insert(paths);
                    (inserted, true)
                }
            }
        } else if let Some(paths) = s.loaded.get_mut(kind) {
            if paths.contains(normalized) { (false, false) }
            else { (paths.insert(Box::<str>::from(normalized)), false) }
        } else {
            let mut paths = HashSet::new();
            let inserted = paths.insert(Box::<str>::from(normalized));
            s.loaded.insert(kind.to_owned(), paths);
            (inserted, true)
        };
        let late = s.playing && inserted;
        if trace {
            let m = &mut s.history_metrics;
            m.reads += 1;
            m.borrowed_paths += usize::from(borrowed_path);
            m.inserted += usize::from(inserted);
            m.new_kinds += usize::from(new_kind);
            m.normalizations += usize::from(!borrowed_path);
            m.kind_allocations += usize::from(separate || new_kind);
        }
        if inserted {
            let mut index = s.required_heads.get(&resource_key(kind, normalized)).copied().unwrap_or(usize::MAX);
            while index != usize::MAX {
                let required = &s.required[index];
                if required.kind == kind && required.path == normalized { s.acquired += 1; }
                index = s.required_next[index];
            }
        }
        if let (Some(start), Some(end)) = (cycle_start, if trace { thread_cycles() } else { None }) {
            s.history_metrics.cycle_reads += 1;
            s.history_metrics.cycles = s.history_metrics.cycles.saturating_add(end.saturating_sub(start));
        }
        late
    });
    if late && !path.contains(".bimage7_mip") && std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        eprintln!("LOAD late {kind}: {path}");
    }
}

/// Forget a resource only when its owning cache releases the last lease.
/// 140060180 purges registered map resources separately from map requests.
pub(crate) fn forget(kind: &str, path: &str) -> Option<usize> {
    with(|s| {
        let removed = s.loaded.get_mut(kind)?.take(path)?;
        let mut index = s.required_heads.get(&resource_key(kind, path)).copied().unwrap_or(usize::MAX);
        while index != usize::MAX {
            let required = &s.required[index];
            if required.kind == kind && required.path == path { s.acquired = s.acquired.saturating_sub(1); }
            index = s.required_next[index];
        }
        Some(removed.len())
    })
}

/// Release one resource family's read identities after its last owner drops.
pub(crate) fn forget_kind(kind: &str) -> Option<(usize, usize, usize)> {
    with(|s| {
        let paths = s.loaded.remove(kind)?;
        let mut removed_required = 0usize;
        if !s.required_heads.is_empty() {
            for path in &paths {
                let mut index = s.required_heads.get(&resource_key(kind, path)).copied().unwrap_or(usize::MAX);
                while index != usize::MAX {
                    let required = &s.required[index];
                    if required.kind == kind && required.path.as_str() == &**path { removed_required += 1; }
                    index = s.required_next[index];
                }
            }
        }
        s.acquired = s.acquired.saturating_sub(removed_required);
        Some((paths.len(), paths.iter().map(|path| path.len()).sum(), paths.capacity()))
    })
}

/// Prune read identities only when their resource owner reports that no
/// reusable instance remains. The current acquisition index, if any, is
/// adjusted just like `forget`; this runs at level boundaries, not per frame.
pub(crate) fn retain_kind(kind: &str, mut keep: impl FnMut(&str) -> bool) -> (usize, usize, usize, usize) {
    with(|s| {
        let Some(paths) = s.loaded.get_mut(kind) else { return (0, 0, 0, 0); };
        let slots_before = paths.capacity();
        let mut removed = 0usize;
        let mut bytes = 0usize;
        let mut removed_required = 0usize;
        paths.retain(|path| {
            if keep(path) { return true; }
            removed += 1;
            bytes += path.len();
            if !s.required_heads.is_empty() {
                let mut index = s.required_heads.get(&resource_key(kind, path)).copied().unwrap_or(usize::MAX);
                while index != usize::MAX {
                    let required = &s.required[index];
                    if required.kind == kind && required.path.as_str() == &**path { removed_required += 1; }
                    index = s.required_next[index];
                }
            }
            false
        });
        let slots_after = paths.capacity();
        s.acquired = s.acquired.saturating_sub(removed_required);
        (removed, bytes, slots_before, slots_after)
    })
}

/// The level is shown: later first reads are late loads.
pub fn play_started() {
    with(|s| {
        s.playing = true;
        let (separate, trace) = history_modes();
        if trace {
            let m = &s.history_metrics;
            eprintln!("MEMORY read history allocation operations: separate={separate} reads={} borrowed_paths={} inserted={} new_kinds={} normalizations={} kind_allocations={} cycle_reads={} cycles={}",
                m.reads, m.borrowed_paths, m.inserted, m.new_kinds, m.normalizations, m.kind_allocations, m.cycle_reads, m.cycles);
        }
        // 1400661e0 frees the temporary acquisition table after loading.
        // Progress is now 1; late-load reporting only needs the read registry.
        if std::env::var("DH2_DEBUG_RETAIN_LEVEL_MANIFESTS").as_deref() != Ok("1") {
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY loading descriptor release: entries={} logical_section_bytes={} owners={} index_capacity={}",
                    s.required.len(), s.required.logical_descriptor_bytes(),
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
pub fn begin_level(list: std::sync::Arc<crate::assets::map_resources::LevelList>, catalog: &crate::assets::archive::Catalog) {
    with(|s| {
        s.playing = false;
        if s.common_texts.as_ref().is_some_and(|(id, _)| *id != catalog.identity()) {
            // Drop the borrowed credits before constructing the next request table.
            s.common_texts = None;
            s.common_exceptions = HashMap::new();
        }
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
            if s.contains(&resource.kind, &resource.path) { s.acquired += 1; }
        }
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let mut by_kind: Vec<_> = s.loaded.iter().map(|(kind, paths)| {
                let bytes = paths.iter().map(|path| path.len()).sum::<usize>();
                (kind.as_str(), paths.len(), bytes)
            }).collect();
            by_kind.sort_unstable_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(b.0)));
            let usable_slots: usize = s.loaded.values().map(HashSet::capacity).sum();
            eprintln!("MEMORY loading read history: kinds={} entries={} path_capacity_bytes={} precredited={} requested={} largest={:?}",
                by_kind.len(), by_kind.iter().map(|x| x.1).sum::<usize>(),
                by_kind.iter().map(|x| x.2).sum::<usize>(), s.acquired, s.requested,
                by_kind.iter().take(10).collect::<Vec<_>>());
            eprintln!("MEMORY loading read history table: usable_slots={usable_slots} value_bytes_per_slot={}",
                std::mem::size_of::<Box<str>>());
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
    report_iter(list.iter())
}

pub(crate) fn report_iter<'a>(list: impl IntoIterator<Item = &'a crate::assets::map_resources::Resource>) -> Vec<Coverage> {
    let mut listed: HashMap<&str, HashSet<&str>> = HashMap::new();
    for r in list {
        listed.entry(r.kind.as_str()).or_default().insert(r.path.as_str());
    }
    with(|s| {
        let mut kinds: HashSet<&str> = listed.keys().copied().collect();
        kinds.extend(s.loaded.keys().map(String::as_str));
        if let Some((_, common)) = &s.common_texts {
            kinds.extend(common.keys().filter_map(|path| s.common_kind(path)));
        }
        let mut out: Vec<Coverage> = kinds
            .into_iter()
            .map(|kind| {
                let paths = listed.get(kind);
                let done = s.loaded.get(kind);
                let loaded = paths.map_or(0, |p| p.iter().filter(|path| s.contains(kind, path)).count());
                let mut unlisted = done.map_or(0, |d| d.iter().filter(|x| {
                    let path: &str = x;
                    paths.is_none_or(|p| !p.contains(path))
                }).count());
                if let Some((_, common)) = &s.common_texts {
                    unlisted += common.keys().filter(|path| s.common_kind(path) == Some(kind)
                        && paths.is_none_or(|p| !p.contains(path.as_str()))).count();
                }
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
