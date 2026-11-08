//! Catalog-owned AnimSet declaration leases. 140247a40 resolves aliases on
//! the actual AnimSet; the model or Midnight group retains that declaration.
use crate::assets::archive::Catalog;
use std::{collections::HashMap, sync::{Arc, Mutex, OnceLock, Weak}};

// Immutable declaration values do not need String's spare-capacity word.
// Keep the parser's transient type separate so forceLoad still runs before
// the corresponding alias is inserted.
struct StoredAlias {
    animation: Box<str>,
    _force_load: bool,
    _force_zero_speed: bool,
    movement: Option<std::num::NonZeroU32>,
}

pub(crate) struct AnimSet {
    pub(crate) path: String,
    aliases: HashMap<Box<str>, StoredAlias>,
    // 2462b0 retains generated movement in declaration order, separate from
    // the alias table. The alias holds only the index of its own record.
    movements: Box<[crate::edge_animation::AnimMovement]>,
    source_bytes: usize,
    // A composable digest is retained only for the existing rig diagnostic;
    // ordinary marker lookup reads its selected declaration lazily.
    source_crc: Option<flate2::Crc>,
}

impl AnimSet {
    pub(crate) fn alias(&self, name: &str) -> Option<&str> { self.aliases.get(name).map(|alias| alias.animation.as_ref()) }
    pub(crate) fn movement(&self, name: &str) -> Option<crate::edge_animation::AnimMovement> {
        let index = self.aliases.get(name)?.movement?.get() as usize - 1;
        self.movements.get(index).copied()
    }
    pub(super) fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.aliases.iter().map(|(name, animation)| (name.as_ref(), animation.animation.as_ref()))
    }
    pub(super) fn source_bytes(&self) -> usize { self.source_bytes }
    pub(super) fn source_crc(&self) -> Option<&flate2::Crc> { self.source_crc.as_ref() }
    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>() + self.path.capacity()
            + self.aliases.capacity() * std::mem::size_of::<(Box<str>, StoredAlias)>()
            + self.movements.len() * std::mem::size_of::<crate::edge_animation::AnimMovement>()
            + self.aliases.iter().map(|(name, path)| name.len() + path.animation.len()).sum::<usize>()
    }
}

impl Drop for AnimSet {
    fn drop(&mut self) {
        if tracing() {
            eprintln!("ANIMSET_RELEASE path={} identity={:p} estimated_retained_bytes={}", self.path, self, self.retained_bytes());
        }
    }
}

#[derive(Default)]
struct Entry {
    weak: Weak<AnimSet>,
    // Editable and diagnostic acquisitions can coexist with older actors.
    previous: Vec<Weak<AnimSet>>,
}
#[derive(Default)]
pub(crate) struct Registry {
    entries: HashMap<String, Arc<Mutex<Entry>>>,
}

impl Registry {
    fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<std::sync::Mutex<Self>>()
            + self.entries.capacity() * std::mem::size_of::<(String, Arc<Mutex<Entry>>)>()
            + self.entries.len() * (2 * std::mem::size_of::<usize>() + std::mem::size_of::<Mutex<Entry>>())
            + self.entries.keys().map(String::capacity).sum::<usize>()
            + self.entries.values().map(|entry| entry.lock().unwrap_or_else(|error| error.into_inner())
                .previous.capacity() * std::mem::size_of::<Weak<AnimSet>>()).sum::<usize>()
    }
    fn report_bytes(&self) {
        if tracing() {
            eprintln!("ANIMSET_REGISTRY estimated_retained_bytes={} entries={} pinned={} record_bytes={} map_entry_bytes={} map_capacity={}", self.estimated_bytes(),
                self.entries.len(), 0, 2 * std::mem::size_of::<usize>() + std::mem::size_of::<Mutex<Entry>>(),
                std::mem::size_of::<(String, Arc<Mutex<Entry>>)>(), self.entries.capacity());
        }
    }
    /// Level owners have dropped; keep identities still held by live
    /// consumers, and forget only dead weak entries.
    pub(crate) fn release_level(&mut self, catalog: &Catalog) {
        let before = self.entries.len();
        let mut history_entries = 0;
        let mut history_bytes = 0;
        self.entries.retain(|path, entry| {
            // An acquisition owns the header before archive IO starts. Do not
            // retire its just-recorded read while parsing is still in flight.
            if Arc::strong_count(entry) > 1 { return true; }
            let mut entry = entry.lock().unwrap_or_else(|error| error.into_inner());
            entry.previous.retain(|generation| generation.strong_count() != 0);
            if entry.weak.strong_count() != 0 || !entry.previous.is_empty() { return true; }
            if let Some(bytes) = catalog.kind(path).and_then(|kind|
                crate::loading::forget(catalog.identity(), kind, path)) {
                history_entries += 1;
                history_bytes += bytes;
            }
            false
        });
        if tracing() {
            eprintln!("ANIMSET_LEVEL_RELEASE forgotten={} retained={} pinned={} history_entries={history_entries} history_bytes={history_bytes}",
                before - self.entries.len(), self.entries.len(), 0);
        }
        self.report_bytes();
        if tracing() {
            eprintln!("ANIMSET_OWNER_HEAP stage=level bytes={:?}", crate::platform::allocation_tags::live_bytes("animset-owner"));
        }
    }

    /// Catalog is retiring; drop lookup allocations before reporting.
    pub(crate) fn release_session(&mut self) {
        drop(std::mem::take(&mut self.entries));
        if tracing() {
            eprintln!("ANIMSET_OWNER_HEAP stage=session bytes={:?}", crate::platform::allocation_tags::live_bytes("animset-owner"));
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        if tracing() { eprintln!("ANIMSET_REGISTRY_RELEASE entries={} pinned={}",
            self.entries.len(), 0); }
    }
}

pub(super) fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_ANIMSET_LIFETIMES").is_some_and(|value| value == "1"))
}

fn report(set: &Arc<AnimSet>, reused: bool, pinned: bool) {
    if tracing() {
        eprintln!("ANIMSET_ACQUIRE path={} identity={:p} reused={reused} pinned={pinned} source_bytes={} estimated_retained_bytes={} aliases={} alias_capacity={} alias_entry_bytes={} movement_records={} movement_record_bytes={}",
            set.path, Arc::as_ptr(set), set.source_bytes, set.retained_bytes(), set.aliases.len(), set.aliases.capacity(),
            std::mem::size_of::<(Box<str>, StoredAlias)>(), set.movements.len(),
            std::mem::size_of::<crate::edge_animation::AnimMovement>());
    }
}

/// A mutable overlay is acquired afresh; existing owners keep their exact
/// generation. The immutable catalog shares one live parsed declaration.
pub(crate) fn acquire(catalog: &Catalog, path: &str) -> Result<Arc<AnimSet>, String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let mutable = catalog.has_overlay_file(&path);
    let separate = std::env::var("DH2_DEBUG_SEPARATE_ANIMSET_DECLARATIONS").as_deref() == Ok("1");
    let header = {
        let mut registry = catalog.animset_declarations.lock().map_err(|_| "AnimSet declaration owner is poisoned")?;
        let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animset-owner"));
        let header = if let Some(header) = registry.entries.get(&path) { header.clone() }
            else {
                let header = Arc::new(Mutex::new(Entry::default()));
                registry.entries.insert(path.clone(), header.clone());
                header
            };
        registry.report_bytes();
        header
    };
    // Parsing can force-load Edge clips, but does not request another AnimSet.
    // Serialize one declaration, never the whole class, across that work.
    let mut entry = header.lock().map_err(|_| "AnimSet resource record is poisoned")?;
    if !mutable && !separate {
        if let Some(set) = entry.weak.upgrade() {
            report(&set, true, false);
            return Ok(set);
        }
    }
    if tracing() {
        eprintln!("ANIMSET_READ_BEGIN path={path} catalog={} separate={separate} mutable={mutable}", catalog.identity());
    }
    let read_start = tracing().then(crate::loading::thread_cycles).flatten();
    let bytes = catalog.read(&path).map_err(|error| error.to_string())?.ok_or_else(|| format!("{path} is absent"))?;
    let source = crate::entities::declaration_text(&bytes).into_owned();
    let read_end = read_start.and_then(|_| crate::loading::thread_cycles());
    let parse_start = tracing().then(crate::loading::thread_cycles).flatten();
    let mut aliases = HashMap::new();
    let mut movements = Vec::new();
    let parsed = {
        let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animset-owner"));
        let tokens = crate::entities::declaration_lexemes(&source)?;
        crate::edge_animation::visit_animset_alias_tokens(&tokens, |name, alias| {
            if alias.force_load {
                let path = crate::edge_animation::resource_path(&alias.animation)
                    .ok_or_else(|| format!("invalid forced animation {}", alias.animation))?;
                let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animation-force-load"));
                crate::edge_animation::force_load(catalog, &path, crate::edge_animation::ResourceScope::Level)?;
            }
            aliases.insert(name.into_boxed_str(), StoredAlias { animation: alias.animation.into_boxed_str(),
                _force_load: alias.force_load, _force_zero_speed: alias.force_zero_speed, movement: alias.movement });
            Ok(())
        })?;
        movements.reserve_exact(tokens.iter().filter(|token| **token == "aliasRef").count());
        crate::edge_animation::visit_animset_movement(&tokens, |name, movement| {
            let alias = aliases.get_mut(name).ok_or_else(|| format!("generated movement references absent alias {name}"))?;
            if alias.movement.is_some() { return Err(format!("duplicate generated movement for alias {name}")); }
            let index = u32::try_from(movements.len()).ok().and_then(|value| value.checked_add(1))
                .and_then(std::num::NonZeroU32::new).ok_or("too many generated movement records")?;
            alias.movement = Some(index);
            movements.push(movement);
            Ok(())
        })
    };
    let parse_end = parse_start.and_then(|_| crate::loading::thread_cycles());
    if tracing() {
        let read_cycles = read_start.zip(read_end).map(|(start, end)| end.saturating_sub(start));
        let parse_cycles = parse_start.zip(parse_end).map(|(start, end)| end.saturating_sub(start));
        eprintln!("ANIMSET_READ path={path} catalog={} separate={separate} mutable={mutable} source_bytes={} read_cycles={read_cycles:?} parse_cycles={parse_cycles:?}",
            catalog.identity(), source.len());
    }
    parsed.map_err(|error| format!("{path}: {error}"))?;
    let trace_signature = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") || super::acquisition::tracing();
    let source_crc = trace_signature.then(|| { let mut crc = flate2::Crc::new(); crc.update(source.as_bytes()); crc });
    let set = {
        let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animset-owner"));
        Arc::new(AnimSet { path: path.clone(), aliases, movements: movements.into_boxed_slice(), source_bytes: source.len(), source_crc })
    };
    // Current callers are actual model/Rig acquisitions in level scope. A
    // future common preloader must pass its actual allocator scope here.
    let pinned = false;
    let previous = std::mem::replace(&mut entry.weak, Arc::downgrade(&set));
    let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animset-owner"));
    entry.previous.retain(|generation| generation.strong_count() != 0);
    if previous.strong_count() != 0 { entry.previous.push(previous); }
    report(&set, false, pinned);
    Ok(set)
}
