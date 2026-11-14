//! Demand-loaded bootstrap and level ownership for immutable HKX shape sources.
//! 140779530 -> 14008c840 acquires physhape identity through 1400607f0.
use super::{parse_hkx, PhysWorld, TagField, TagItem, TagType};
use crate::assets::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::{Arc, Mutex, OnceLock, Weak}};

// Host: map payloads retain the accepted temporary area sharing policy rather
// than being pinned until purge. Read credits have their own owner, independent
// of the source registry, so bootstrap payloads cannot form an ownership cycle.
#[derive(Default)]
struct HistoryState { scopes: usize, sources: HashMap<Arc<str>, usize> }
struct ReadHistory { catalog: u64, state: Mutex<HistoryState> }
impl ReadHistory {
    fn acquire(self: &Arc<Self>, path: &str) -> SourceReadLease {
        // Count the in-flight read before Catalog can publish its history.
        // Failed/missing generations participate too: their rollback cannot
        // remove a successful generation's credit while that source is live.
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let path = state.sources.get_key_value(path).map(|(key, _)| key.clone())
            .unwrap_or_else(|| Arc::from(path));
        *state.sources.entry(path.clone()).or_default() += 1;
        SourceReadLease { history: self.clone(), path }
    }
}
struct SourceReadLease { history: Arc<ReadHistory>, path: Arc<str> }
impl Drop for SourceReadLease {
    fn drop(&mut self) {
        let mut state = self.history.state.lock().unwrap_or_else(|error| error.into_inner());
        let count = state.sources.get_mut(self.path.as_ref()).expect("shape source credit missing");
        *count -= 1;
        if *count == 0 {
            state.sources.remove(self.path.as_ref());
            if state.scopes == 0 {
                // Keep the state lock through forget. A replacement source
                // cannot begin its read until this old credit has retired.
                crate::loading::forget(self.history.catalog, "physhape", &self.path);
            }
        }
    }
}
pub(crate) struct ShapeHistoryLease(Arc<ReadHistory>);
impl ShapeHistoryLease {
    fn acquire(history: &Arc<ReadHistory>) -> Arc<Self> {
        history.state.lock().unwrap_or_else(|error| error.into_inner()).scopes += 1;
        Arc::new(Self(history.clone()))
    }
}
impl Drop for ShapeHistoryLease {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        state.scopes -= 1;
        if state.scopes != 0 { return; }
        // Includes archive-observer reads made during the map scope, while
        // preserving actual common and outliving mutable-source consumers.
        let (entries, bytes, _, _) = crate::loading::retain_catalog_kind(self.0.catalog, "physhape",
            |path| state.sources.get(path).is_some_and(|count| *count != 0));
        // Drop map-sized hash capacity even if one common source remains.
        state.sources.shrink_to_fit();
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
            eprintln!("MEMORY shape history release: entries={entries} path_capacity_bytes={bytes} live={} catalog={}", state.sources.len(), self.0.catalog);
        }
    }
}

pub struct ShapeSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) physics: PhysWorld,
    alias: Option<(String, f32)>,
    session: bool,
    // Release the source's read credit after its bytes/schema. This lease never
    // locks the bootstrap registry; registry -> history -> loading is the sole
    // nested lock order for admission and retirement.
    _history: Option<SourceReadLease>,
}
impl ShapeSource {
    fn require_hkx(&self) -> Result<(), ShapeSourceError> {
        if self.alias.is_some() {
            // 14008c9a0 resolves an alias through 1400607f0 and constructs a
            // Havok scaled wrapper. Standalone component acquisition retains
            // that chain; legacy direct-root callers request neither wrapper
            // identity nor bounds. Preserve their explicit admission failure
            // instead of exposing the compact descriptor's unused schema.
            return Err(ShapeSourceError::Parse("compact HKX alias requires standalone shape acquisition".into()));
        }
        Ok(())
    }
}

/// A standalone physhape and its authored wrapper chain (14008c9a0).
/// Each wrapper retains its own resource lookup identity, outermost first;
/// the component owns this lease until its body is deinitialized.
pub(crate) struct StandaloneShape {
    pub catalog_identity: u64,
    /// Overlay-backed paths must not be deduplicated by name alone.
    pub shareable: bool,
    pub source: Arc<ShapeSource>,
    pub source_path: String,
    /// The innermost resource's first four serialized bytes.
    pub source_header: u32,
    pub wrappers: Vec<Arc<ShapeAlias>>,
}

pub(crate) struct ShapeAlias {
    /// Normalized catalog path for the resource which owns this wrapper.
    pub path: String,
    pub scale: f32,
    pub header: u32,
    pub target: String,
    // Retains the exact child generation represented by this resource.
    _child: AliasChild,
    // The compact source owns its own read credit through final wrapper retirement.
    _source: Arc<ShapeSource>,
}

enum AliasChild { Source(Arc<ShapeSource>), Alias(Arc<ShapeAlias>) }
impl AliasChild {
    fn identity(&self) -> usize {
        match self { Self::Source(source) => Arc::as_ptr(source) as usize,
            Self::Alias(alias) => Arc::as_ptr(alias) as usize }
    }
}

#[derive(Hash, PartialEq, Eq)]
struct AliasKey {
    catalog: u64,
    path: String,
    header: u32,
    scale_bits: u32,
    target: String,
    child_identity: usize,
}

#[derive(Hash, PartialEq, Eq)]
struct StandaloneSourceKey { catalog: u64, path: String }

impl ShapeAlias {
    /// The stable host resource record is the identity carried by the alias
    /// compound's userData; the Arc lease owns it through body retirement.
    pub(crate) fn identity(resource: &Arc<Self>) -> u64 { Arc::as_ptr(resource) as usize as u64 }
}

impl StandaloneShape {
    /// This is the innermost shape, not the full resource when `wrappers` is
    /// nonempty. The component must retain and apply each compound wrapper.
    pub(crate) fn base_query_shapes(&self) -> Result<super::query_geometry::QueryShapes, String> {
        self.source.query_shapes(1)
    }

    /// Decode the acquired source and retain its actual alias-wrapper chain.
    pub(crate) fn query_shapes(&self) -> Result<super::query_geometry::QueryShapes, String> {
        use super::query_geometry::alias::AliasInstance;
        let mut shapes = self.base_query_shapes()?;
        shapes.set_root_resource_identity(Arc::as_ptr(&self.source) as usize as u64)?;
        let wrappers: Vec<_> = self.wrappers.iter().map(|resource| AliasInstance {
            scale: resource.scale, resource_identity: ShapeAlias::identity(resource),
        }).collect();
        shapes.wrap_aliases(&wrappers)?;
        Ok(shapes)
    }

    /// Derive the legacy movement surface from this same acquired source.
    /// Alias scales are applied from innermost to outermost, as in 14008c9a0;
    /// the resulting triangles are not the owner of the original shape.
    pub(crate) fn movement_triangles(&self) -> Result<Vec<[[f32; 3]; 3]>, String> {
        let geometry = self.source.physics.standalone_shapes(&self.source.bytes)?;
        if !geometry.compressed_decode_failures.is_empty() || !geometry.unknown_shapes.is_empty() {
            return Err(format!("incomplete HKX geometry: {:?}, {:?}",
                geometry.compressed_decode_failures, geometry.unknown_shapes));
        }
        let mut triangles: Vec<_> = geometry.triangles.into_iter().map(|triangle| triangle.vertices).collect();
        for wrapper in self.wrappers.iter().rev() {
            for triangle in &mut triangles {
                for vertex in triangle {
                    for coordinate in vertex { *coordinate *= wrapper.scale; }
                }
            }
        }
        Ok(triangles)
    }
}

impl ShapeSource {
    pub(crate) fn query_shapes(self: &Arc<Self>, root: usize)
        -> Result<super::query_geometry::QueryShapes, String> {
        self.require_hkx().map_err(ShapeSourceError::message)?;
        self.physics.query_shapes(&self.bytes, root)
    }
}

pub(crate) enum ShapeSourceError { Read(String), Parse(String) }
impl ShapeSourceError {
    pub(crate) fn message(self) -> String { match self { Self::Read(e) | Self::Parse(e) => e } }
}

#[derive(Clone, Copy)]
pub(crate) enum ShapeUse { Area, Physics, Wind, Machinery }

#[derive(Default)]
struct Counts { requests: usize, reads: usize, bytes: usize, hits: usize }

fn source_key(path: &str) -> Cow<'_, str> {
    if path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(path.replace('\\', "/").to_ascii_lowercase())
    }
}

fn read_source(catalog: &Catalog, path: &str, counts: &mut Counts)
    -> Result<Option<ShapeSource>, ShapeSourceError>
{
    counts.reads += 1;
    let history = owner(catalog).history.acquire(path);
    let Some(bytes) = catalog.read(path).map_err(|error| ShapeSourceError::Read(error.to_string()))? else { return Ok(None); };
    counts.bytes += bytes.len();
    // 14008c9a0 reads the signed material count independently of the alias
    // byte. A compact resource may therefore have a nonzero four-byte header;
    // the model-triangle reader's narrower recognition is not this format.
    let alias = if bytes.get(4).is_some_and(|flag| *flag != 0) {
        if bytes.len() < 13 { return Err(ShapeSourceError::Parse("physics shape alias is truncated".into())); }
        let scale = f32::from_le_bytes(bytes[5..9].try_into().unwrap());
        if !scale.is_finite() { return Err(ShapeSourceError::Parse("physics shape alias scale is non-finite".into())); }
        let length = u32::from_le_bytes(bytes[9..13].try_into().unwrap()) as usize;
        let end = 13usize.checked_add(length).ok_or_else(|| ShapeSourceError::Parse("physics shape alias name length overflow".into()))?;
        let name = bytes.get(13..end).ok_or_else(|| ShapeSourceError::Parse("physics shape alias name is truncated".into()))?;
        let name = std::str::from_utf8(name).map_err(|error| ShapeSourceError::Parse(format!("physics shape alias name: {error}")))?;
        Some((super::physics_shape_asset_path(name), scale))
    } else { None };
    let physics = if alias.is_some() { empty_physics() } else { parse_hkx(&bytes).map_err(ShapeSourceError::Parse)? };
    Ok(Some(ShapeSource { bytes, physics, alias, session: false, _history: Some(history) }))
}

struct BootstrapEntry { marker: u8, source: Option<Arc<ShapeSource>> }
pub(crate) struct Owner {
    registered: Mutex<HashMap<String, BootstrapEntry>>,
    history: Arc<ReadHistory>,
}
fn owner(catalog: &Catalog) -> &Owner {
    catalog.shape_sources.get_or_init(|| Owner {
        registered: Mutex::new(HashMap::new()),
        history: Arc::new(ReadHistory { catalog: catalog.identity(), state: Mutex::new(HistoryState::default()) }),
    })
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, bootstrap: bool) -> Result<(), String> {
    let mut registered = owner(catalog).registered.lock().map_err(|_| "shape source registry is poisoned")?;
    let key = source_key(path);
    if bootstrap {
        registered.entry(key.into_owned()).or_insert(BootstrapEntry { marker: 0, source: None });
    } else if let Some(entry) = registered.get_mut(key.as_ref()) {
        // Explicit map registration replaces a matching bootstrap marker. The
        // installed lists have no such overlap, but overlays need not obey it.
        entry.marker = 0;
        entry.source = None;
    }
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.shape_sources.get() {
        for entry in owner.registered.lock().map_err(|_| "shape source registry is poisoned")?.values_mut() {
            entry.marker = 2;
        }
    }
    Ok(())
}
pub(crate) fn release_level(catalog: &Catalog) {
    if let Some(owner) = catalog.shape_sources.get() {
        owner.registered.lock().unwrap_or_else(|error| error.into_inner()).retain(|_, entry| entry.marker != 0);
    }
    report(catalog, "map-retired");
}
pub(crate) fn report(catalog: &Catalog, phase: &str) {
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_none_or(|value| value != "1") { return; }
    let Some(owner) = catalog.shape_sources.get() else { return; };
    let registered = owner.registered.lock().unwrap_or_else(|error| error.into_inner());
    let mut payloads = 0;
    let mut payload_bytes = 0;
    for source in registered.values().filter_map(|entry| entry.source.as_ref()) {
        payloads += 1;
        payload_bytes += std::mem::size_of::<ShapeSource>() + 2 * std::mem::size_of::<usize>()
            + source.bytes.capacity() + schema_capacity(&source.physics)
            + source.alias.as_ref().map_or(0, |(target, _)| target.capacity());
    }
    let history = owner.history.state.lock().unwrap_or_else(|error| error.into_inner());
    let record_storage_bytes = registered.capacity() * std::mem::size_of::<(String, BootstrapEntry)>()
        + registered.keys().map(String::capacity).sum::<usize>();
    let history_storage_bytes = history.sources.capacity() * std::mem::size_of::<(Arc<str>, usize)>()
        + history.sources.keys().map(|path| path.len() + 2 * std::mem::size_of::<usize>()).sum::<usize>();
    eprintln!("RESOURCE_STORAGE catalog={} kind=physhape phase={phase} records={} record_storage_bytes={record_storage_bytes} payloads={payloads} payload_bytes={payload_bytes} history_paths={} history_storage_bytes={history_storage_bytes} map_scopes={}",
        catalog.identity(), registered.len(), history.sources.len(), history.scopes);
}

enum AcquiredSource { Shared(Arc<ShapeSource>), OneShot(ShapeSource) }
impl std::ops::Deref for AcquiredSource {
    type Target = ShapeSource;
    fn deref(&self) -> &ShapeSource { match self { Self::Shared(source) => source, Self::OneShot(source) => source } }
}
impl AcquiredSource {
    fn into_shared(self) -> Arc<ShapeSource> {
        match self { Self::Shared(source) => source, Self::OneShot(source) => Arc::new(source) }
    }
}

fn acquire_source(catalog: &Catalog, path: &str, inherited_session: bool, counts: &mut Counts)
    -> Result<Option<AcquiredSource>, ShapeSourceError>
{
    let key = source_key(path);
    let owner = owner(catalog);
    let mut registered = owner.registered.lock().map_err(|_| ShapeSourceError::Read("shape source registry is poisoned".into()))?;
    if inherited_session {
        registered.entry(key.clone().into_owned()).or_insert(BootstrapEntry { marker: 2, source: None });
    }
    let entry = registered.get_mut(key.as_ref()).filter(|entry| entry.marker == 2);
    let session = inherited_session || entry.is_some();
    let shared = !catalog.has_overlays()
        && std::env::var_os("DH2_DEBUG_SEPARATE_SHAPE_RESOURCES").is_none_or(|value| value != "1");
    if let Some(entry) = entry.filter(|_| shared) {
        if let Some(source) = &entry.source {
            counts.hits += 1;
            return Ok(Some(AcquiredSource::Shared(source.clone())));
        }
        let Some(mut source) = read_source(catalog, key.as_ref(), counts)? else { return Ok(None); };
        source.session = true;
        let source = Arc::new(source);
        entry.source = Some(source.clone());
        return Ok(Some(AcquiredSource::Shared(source)));
    }
    // Map inputs keep one-shot ownership; do not rebuild the rejected cache of
    // all alias targets. The existing ShapeSources area cache remains separate.
    drop(registered);
    let Some(mut source) = read_source(catalog, key.as_ref(), counts)? else { return Ok(None); };
    source.session = session;
    Ok(Some(AcquiredSource::OneShot(source)))
}

/// Cache only early area inputs; physics-only and late wind shapes do not grow
/// this map. It is dropped after all level preparation consumers have run.
pub(crate) struct ShapeSources {
    enabled: bool,
    accepting: bool,
    trace: bool,
    sources: HashMap<String, Arc<ShapeSource>>,
    standalone_sources: HashMap<StandaloneSourceKey, Vec<Weak<ShapeSource>>>,
    alias_resources: HashMap<AliasKey, Weak<ShapeAlias>>,
    counts: [Counts; 4],
    source_capacity: usize,
    schema_capacity: usize,
    key_capacity: usize,
    // Retire history after the acquired source data drops.
    history: Arc<ShapeHistoryLease>,
}

impl ShapeSources {
    pub(crate) fn new(catalog: &Catalog) -> Self {
        let enabled = !catalog.has_overlays()
            && std::env::var("DH2_DEBUG_SEPARATE_TRIGGER_SOURCES").as_deref() != Ok("1");
        Self::with_sharing(catalog, enabled)
    }

    /// A standalone consumer still owns its catalog's history during acquisition.
    pub(crate) fn uncached(catalog: &Catalog) -> Self {
        Self::with_sharing(catalog, false)
    }

    fn with_sharing(catalog: &Catalog, enabled: bool) -> Self {
        let trace = std::env::var("DH2_DEBUG_TRIGGER_SOURCES").as_deref() == Ok("1");
        Self {
            history: ShapeHistoryLease::acquire(&owner(catalog).history), enabled, accepting: enabled, trace,
            sources: HashMap::new(), standalone_sources: HashMap::new(), alias_resources: HashMap::new(), counts: Default::default(), source_capacity: 0, schema_capacity: 0, key_capacity: 0,
        }
    }

    pub(crate) fn history_lease(&self) -> Arc<ShapeHistoryLease> { self.history.clone() }


    /// 14008c9a0 acquires one typed source generation, then wraps that same
    /// resource rather than flattening its authored compound into triangles.
    pub(crate) fn acquire_standalone(&mut self, catalog: &Catalog, path: &str, role: ShapeUse)
        -> Result<Option<StandaloneShape>, String> {
        let mut path = path.to_owned();
        let mut visited = std::collections::HashSet::new();
        let mut wrappers = Vec::new();
        let mut inherited_session = false;
        loop {
            let key = source_key(&path).into_owned();
            if !visited.insert(key.clone()) { return Err(format!("physics shape alias cycle at {path}")); }
            self.counts[role as usize].requests += 1;
            let identity = StandaloneSourceKey { catalog: catalog.identity(), path: key.clone() };
            let retained = (!catalog.has_overlays()).then(|| {
                self.sources.get(&key).cloned().or_else(|| self.standalone_sources.get(&identity)
                    .and_then(|values| values.iter().find_map(Weak::upgrade)))
            }).flatten();
            let source = match retained {
                Some(source) => { self.counts[role as usize].hits += 1; source },
                None => {
                    let Some(source) = acquire_source(catalog, &key, inherited_session, &mut self.counts[role as usize])
                        .map_err(ShapeSourceError::message)? else { return Ok(None); };
                    let source = source.into_shared();
                    if !catalog.has_overlays() {
                        let values = self.standalone_sources.entry(identity).or_default();
                        values.retain(|value| value.strong_count() != 0);
                        values.push(Arc::downgrade(&source));
                    }
                    source
                },
            };
            if source.bytes.len() < 5 { return Err(format!("physics shape header is truncated: {key}")); }
            let header = u32::from_le_bytes(source.bytes[..4].try_into().unwrap());
            log_source(self.trace, role, &key, &source, false);
            inherited_session |= source.session;
            if let Some((target, scale)) = &source.alias {
                if !scale.is_finite() { return Err(format!("physics shape alias scale is non-finite: {key}")); }
                let target = target.clone();
                wrappers.push((key, *scale, header, target.clone(), source));
                path = target;
                continue;
            }
            let root = source.physics.items.get(1).ok_or("standalone HKX has no root item")?;
            let kind = source.physics.types.get(root.type_index).ok_or("standalone HKX root type is absent")?;
            if !kind.name.ends_with("Shape") { return Err(format!("standalone HKX root is {}, not a shape", kind.name)); }
            let mut child = AliasChild::Source(source.clone());
            let mut leases = Vec::new();
            leases.try_reserve_exact(wrappers.len()).map_err(|error| format!("physics alias leases: {error}"))?;
            for (path, scale, header, target, alias_source) in wrappers.into_iter().rev() {
                let key = AliasKey { catalog: catalog.identity(), path: path.clone(), header,
                    scale_bits: scale.to_bits(), target: target.clone(), child_identity: child.identity() };
                let lease = self.alias_resources.get(&key).and_then(Weak::upgrade).unwrap_or_else(|| {
                    let lease = Arc::new(ShapeAlias { path, scale, header, target, _child: child,
                        _source: alias_source });
                    self.alias_resources.insert(key, Arc::downgrade(&lease));
                    lease
                });
                child = AliasChild::Alias(lease.clone());
                leases.push(lease);
            }
            leases.reverse();
            return Ok(Some(StandaloneShape { catalog_identity: catalog.identity(), shareable: !catalog.has_overlays(),
                source, source_path: key, source_header: header, wrappers: leases }));
        }
    }

    pub(crate) fn finish_areas(&mut self) { self.accepting = false; }

    pub(crate) fn acquire(&mut self, catalog: &Catalog, path: &str, role: ShapeUse)
        -> Result<Option<Arc<ShapeSource>>, ShapeSourceError>
    {
        let counts = &mut self.counts[role as usize];
        counts.requests += 1;
        let key = source_key(path);
        if self.enabled {
            if let Some(source) = self.sources.get(key.as_ref()) {
                source.require_hkx()?;
                counts.hits += 1;
                log_source(self.trace, role, path, source, true);
                return Ok(Some(source.clone()));
            }
        }
        let hits = counts.hits;
        let Some(source) = acquire_source(catalog, path, false, counts)? else { return Ok(None); };
        source.require_hkx()?;
        let source = source.into_shared();
        log_source(self.trace, role, path, &source, counts.hits != hits);
        if self.accepting && matches!(role, ShapeUse::Area) {
            let key = key.into_owned();
            if self.trace {
                self.key_capacity += key.capacity();
                self.source_capacity += source.bytes.capacity();
                self.schema_capacity += schema_capacity(&source.physics);
            }
            self.sources.insert(key, source.clone());
        }
        Ok(Some(source))
    }

    /// Derive one-shot geometry without allocating an Arc for an uncached
    /// source. Existing shared area sources are borrowed when available.
    pub(crate) fn geometry(&mut self, catalog: &Catalog, path: &str, role: ShapeUse)
        -> Result<Option<super::CollisionExtraction>, String>
    {
        let counts = &mut self.counts[role as usize];
        counts.requests += 1;
        let key = source_key(path);
        if self.enabled {
            if let Some(source) = self.sources.get(key.as_ref()) {
                source.require_hkx().map_err(ShapeSourceError::message)?;
                counts.hits += 1;
                log_source(self.trace, role, path, source, true);
                return source.physics.standalone_shapes(&source.bytes).map(Some);
            }
        }
        let hits = counts.hits;
        let Some(source) = acquire_source(catalog, path, false, counts).map_err(ShapeSourceError::message)? else { return Ok(None); };
        source.require_hkx().map_err(ShapeSourceError::message)?;
        log_source(self.trace, role, path, &source, counts.hits != hits);
        source.physics.standalone_shapes(&source.bytes).map(Some)
    }
}

pub(crate) fn load_model_havok(
    catalog: &Catalog,
    path: &str,
    depth: usize,
) -> Result<Vec<[[f32; 3]; 3]>, String> {
    load_model_source(catalog, path, depth, false)
}

fn load_model_source(catalog: &Catalog, path: &str, depth: usize, inherited_session: bool)
    -> Result<Vec<[[f32; 3]; 3]>, String>
{
    if depth > 4 {
        return Err("HKX alias chain exceeds four links".into());
    }
    let trace = std::env::var_os("DH2_DEBUG_TRIGGER_SOURCES").is_some_and(|value| value == "1");
    let start = trace.then(crate::loading::thread_cycles).flatten();
    let mut counts = Counts::default();
    let source = acquire_source(catalog, path, inherited_session, &mut counts).map_err(ShapeSourceError::message)?
        .ok_or_else(|| format!("HKX target not indexed: {path}"))?;
    if trace {
        let cycles = start.zip(crate::loading::thread_cycles()).map_or(0, |(start, end)| end.saturating_sub(start));
        let mut crc = flate2::Crc::new(); crc.update(&source.bytes);
        eprintln!("MEMORY model shape source: path={path} reads={} bytes={} hits={} session={} signature={:08x} cycles={cycles}",
            counts.reads, counts.bytes, counts.hits, source.session, crc.sum());
    }
    if let Some((target, factor)) = &source.alias {
        let mut triangles = load_model_source(catalog, target, depth + 1, source.session)?;
        for triangle in &mut triangles {
            for vertex in triangle {
                for axis in 0..3 {
                    vertex[axis] *= *factor;
                }
            }
        }
        return Ok(triangles);
    }
    let geometry = source.physics.standalone_shapes(&source.bytes)?;
    if !geometry.compressed_decode_failures.is_empty() || !geometry.unknown_shapes.is_empty() {
        return Err(format!(
            "incomplete HKX geometry: {:?}, {:?}",
            geometry.compressed_decode_failures, geometry.unknown_shapes
        ));
    }
    Ok(geometry
        .triangles
        .into_iter()
        .map(|triangle| triangle.vertices)
        .collect())
}


fn log_source(trace: bool, role: ShapeUse, path: &str, source: &ShapeSource, reused: bool) {
    if !trace { return; }
    let mut crc = flate2::Crc::new(); crc.update(&source.bytes);
    let role = match role { ShapeUse::Area => "area", ShapeUse::Physics => "physics", ShapeUse::Wind => "wind", ShapeUse::Machinery => "machinery" };
    eprintln!("MEMORY shape source request: role={role} path={path} bytes={} signature={:08x} reused={reused}", source.bytes.len(), crc.sum());
}

impl Drop for ShapeSources {
    fn drop(&mut self) {
        if !self.trace { return; }
        for (name, count) in ["area", "physics", "wind", "machinery"].into_iter().zip(&self.counts) {
            eprintln!("MEMORY shape source owner: role={name} enabled={} requests={} reads={} bytes={} hits={}",
                self.enabled, count.requests, count.reads, count.bytes, count.hits);
        }
        let sole = self.sources.values().filter(|source| Arc::strong_count(source) == 1).count();
        let sole_bytes: usize = self.sources.values().filter(|source| Arc::strong_count(source) == 1).map(|source| source.bytes.capacity()).sum();
        let sole_schema: usize = self.sources.values().filter(|source| Arc::strong_count(source) == 1).map(|source| schema_capacity(&source.physics)).sum();
        eprintln!("MEMORY shape source release: entries={} sole={} source_capacity={} schema_capacity={} key_capacity={} sole_source_capacity={sole_bytes} sole_schema_capacity={sole_schema}",
            self.sources.len(), sole, self.source_capacity, self.schema_capacity, self.key_capacity);
    }
}

pub(crate) fn schema_capacity(physics: &PhysWorld) -> usize {
    physics.sdk_version.capacity()
        + physics.materials.capacity() * std::mem::size_of::<String>()
        + physics.materials.iter().map(String::capacity).sum::<usize>()
        + physics.material_map.capacity() * std::mem::size_of::<u32>()
        + physics.items.capacity() * std::mem::size_of::<TagItem>()
        + physics.types.capacity() * std::mem::size_of::<TagType>()
        + physics.types.iter().map(|ty| ty.name.capacity()
            + ty.fields.capacity() * std::mem::size_of::<TagField>()
            + ty.fields.iter().map(|field| field.name.capacity()).sum::<usize>()).sum::<usize>()
}

pub(crate) fn empty_source() -> Arc<ShapeSource> {
    static EMPTY: OnceLock<Arc<ShapeSource>> = OnceLock::new();
    EMPTY.get_or_init(|| Arc::new(ShapeSource { bytes: Vec::new(), physics: empty_physics(),
        alias: None, session: false, _history: None })).clone()
}

fn empty_physics() -> PhysWorld {
    PhysWorld {
        materials: Vec::new(), material_map: Vec::new(), sdk_version: String::new(),
        types: Vec::new(), items: Vec::new(), data_start: 0, data_size: 0,
    }
}
