//! Registered idDeclMD6 identities and their immutable declaration metadata.
//! A record can exist before its declaration is loaded; looking up an absent
//! record never admits it or reads the archive.
use super::animset;
use crate::assets::archive::Catalog;
use std::{borrow::Cow, cell::RefCell, collections::HashMap, sync::{Arc, Mutex, Weak, atomic::{AtomicU8, Ordering}}};

pub(crate) struct Declaration {
    pub path: Arc<str>,
    pub tokens: Arc<[String]>,
    lod_zero_mesh: LodZeroMesh,
    pub parent: Option<usize>,
    parent_record: Option<Lease>,
    pub parent_declaration: Option<Arc<Declaration>>,
    pub animsets: Vec<Arc<animset::AnimSet>>,
    pub source_bytes: usize,
    pub source_crc: Option<flate2::Crc>,
}

#[derive(Clone, Copy)]
enum LodZeroMesh {
    Local(u32),
    Inherited,
    Unresolved,
}

fn tracing() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DH2_DEBUG_RIG_ACQUISITION").is_some_and(|value| value == "1"))
}

impl Drop for Declaration {
    fn drop(&mut self) {
        if tracing() { eprintln!("MD6_DECL_RELEASE identity={:p}", self); }
    }
}

struct Record {
    scope: AtomicU8,
    value: Mutex<Snapshot>,
}

#[derive(Default)]
struct Snapshot {
    value: Weak<Declaration>,
    // Native loaded fields belong to the registered resource until 060180,
    // not merely until the last host Rig happens to release its Arc.
    resident: Option<Arc<Declaration>>,
    previous: Vec<Weak<Declaration>>,
    attempted: bool,
    absent: bool,
    failure: Option<Arc<str>>,
}

/// 05e740's creation marker; 060b60 alone promotes an existing record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadContext { Level = 0, PersistentHeap = 1 }

#[derive(Clone, Copy)]
pub(crate) enum PurgeMarker { Level = 0, PersistentHeap = 1, Permanent = 2 }

thread_local! {
    static ACTIVE_LOADS: RefCell<Vec<(u64, usize)>> = const { RefCell::new(Vec::new()) };
}

struct ActiveLoad { catalog: u64, record: usize }
impl Drop for ActiveLoad {
    fn drop(&mut self) {
        ACTIVE_LOADS.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) = active.iter().rposition(|entry| *entry == (self.catalog, self.record)) {
                active.remove(index);
            }
        });
    }
}

#[derive(Default)]
pub(crate) struct Registry {
    records: HashMap<Arc<str>, Arc<Record>>,
    order: Vec<Arc<str>>,
}

/// A retained record is an actual admitted declaration identity, including
/// a lazy or failed record; it never implies that a skeleton was acquired.
#[derive(Clone)]
pub(crate) struct Lease {
    path: Arc<str>,
    record: Arc<Record>,
}

/// A nonnull idDeclMD6 pointer resolved by 193500/198940. The record and
/// its actual loaded metadata generation have the same owning reference.
#[derive(Clone)]
pub(crate) struct ModelReference {
    record: Lease,
    declaration: Arc<Declaration>,
}

impl ModelReference {
    pub(crate) fn identity(&self) -> usize { self.record.identity() }
    pub(crate) fn declaration(&self) -> &Arc<Declaration> { &self.declaration }

    /// Use the LOD0 mesh already selected from the installed declaration.
    /// A child without its own mesh keeps walking the admitted parent layers.
    pub(crate) fn compiled_mesh(&self) -> Option<String> {
        let mut current = Some(self.declaration.as_ref());
        while let Some(layer) = current {
            match layer.lod_zero_mesh {
                LodZeroMesh::Local(index) => return crate::bmd6::compiled_mesh_resource(&layer.tokens[index as usize]),
                LodZeroMesh::Inherited => current = layer.parent_declaration.as_deref(),
                LodZeroMesh::Unresolved => return None,
            }
        }
        None
    }

    /// 1c8cc0 accepts this exact MD6 or an actual descendant declaration.
    pub(crate) fn is_ancestor_of(&self, candidate: &Self) -> bool {
        if self.identity() == candidate.identity() { return true; }
        let mut current = candidate.declaration.as_ref();
        while let (Some(parent), Some(declaration)) = (&current.parent_record, &current.parent_declaration) {
            if self.identity() == parent.identity() { return true; }
            current = declaration;
        }
        false
    }

    /// 1c8720 visits each model's authored AnimSets in reverse, then its
    /// parent, suppressing duplicate declaration identities.
    pub(crate) fn animsets(&self) -> Vec<&Arc<animset::AnimSet>> {
        let mut result: Vec<&Arc<animset::AnimSet>> = Vec::new();
        let mut current = Some(self.declaration.as_ref());
        while let Some(declaration) = current {
            for set in declaration.animsets.iter().rev() {
                if !result.iter().any(|existing| Arc::ptr_eq(*existing, set)) { result.push(set); }
            }
            current = declaration.parent_declaration.as_deref();
        }
        result
    }
}

impl Lease {
    pub(crate) fn identity(&self) -> usize { Arc::as_ptr(&self.record) as usize }
    pub(crate) fn path(&self) -> &str { &self.path }

    /// 060630 returns a registered header without loading it. A genuinely
    /// lazy header has constructor-default fields; a host-evicted parsed
    /// generation cannot be silently treated as that default state.
    pub(crate) fn peek_model(&self) -> Result<Option<ModelReference>, String> {
        let value = self.record.value.lock().map_err(|_| "MD6 declaration record is poisoned")?;
        if let Some(declaration) = value.value.upgrade() {
            return Ok(Some(ModelReference { record: self.clone(), declaration }));
        }
        if let Some(error) = &value.failure { return Err(error.to_string()); }
        if value.absent { return Err(format!("MD6 default declaration is not hosted: {}", self.path)); }
        if value.attempted { return Err(format!("registered MD6 metadata was evicted: {}", self.path)); }
        Ok(None)
    }

    pub(crate) fn model(&self, catalog: &Catalog, context: LoadContext) -> Result<ModelReference, String> {
        let declaration = self.load(catalog, false, context)?
            .ok_or_else(|| format!("MD6 default declaration is not hosted: {}", self.path))?;
        Ok(ModelReference { record: self.clone(), declaration })
    }

    pub(crate) fn load(&self, catalog: &Catalog, fresh: bool, scope: LoadContext) -> Result<Option<Arc<Declaration>>, String> {
        let catalog_id = catalog.identity();
        let identity = self.identity();
        let (nested, cycle) = ACTIVE_LOADS.with(|active| {
            let active = active.borrow();
            (active.iter().any(|(id, _)| *id == catalog_id), active.contains(&(catalog_id, identity)))
        });
        if cycle { return Err(format!("MD6 inheritance cycle at {}", self.path)); }
        if !fresh {
            let mut value = self.record.value.lock().map_err(|_| "MD6 declaration record is poisoned")?;
            if let Some(cached) = value.value.upgrade() {
                if value.resident.is_none() { value.resident = Some(cached.clone()); }
                if tracing() { eprintln!("MD6_DECL_ACQUIRE path={} identity={:p} reused=1 source_bytes={} tokens={}",
                    self.path, Arc::as_ptr(&cached), cached.source_bytes, cached.tokens.len()); }
                return Ok(Some(cached));
            }
            if let Some(error) = &value.failure { return Err(error.to_string()); }
            if value.absent { return Ok(None); }
        }
        let wait_started = tracing().then(std::time::Instant::now);
        let _loading = if nested { None } else {
            Some(catalog.md6_loading.lock().map_err(|_| "MD6 declaration loader is poisoned")?)
        };
        if let Some(started) = wait_started {
            eprintln!("MD6_DECL_LOAD_WAIT path={} nested={} wait_us={}", self.path, nested, started.elapsed().as_micros());
        }
        ACTIVE_LOADS.with(|active| active.borrow_mut().push((catalog_id, identity)));
        let _active = ActiveLoad { catalog: catalog_id, record: identity };
        let mut value = self.record.value.lock().map_err(|_| "MD6 declaration record is poisoned")?;
        if !fresh {
            if let Some(cached) = value.value.upgrade() {
                if value.resident.is_none() { value.resident = Some(cached.clone()); }
                if tracing() { eprintln!("MD6_DECL_ACQUIRE path={} identity={:p} reused=1 source_bytes={} tokens={}",
                    self.path, Arc::as_ptr(&cached), cached.source_bytes, cached.tokens.len()); }
                return Ok(Some(cached));
            }
            if let Some(error) = &value.failure { return Err(error.to_string()); }
            if value.absent { return Ok(None); }
        }
        if fresh {
            value.previous.retain(|generation| generation.strong_count() != 0);
            if value.value.strong_count() != 0 {
                let previous = value.value.clone();
                value.previous.try_reserve(1).map_err(|error| format!("MD6 declaration generations: {error}"))?;
                value.previous.push(previous);
            }
        }
        // Parent declarations can register further records. Do not hold a
        // record lock while acquiring the registry: final purge takes those
        // locks in the opposite order. The catalog loader guard serializes
        // publication, while this Lease keeps the current header alive.
        drop(value);
        let Some(bytes) = catalog.read(&self.path).map_err(|error| error.to_string())? else {
            let mut value = self.record.value.lock().map_err(|_| "MD6 declaration record is poisoned")?;
            value.value = Weak::new();
            value.resident = None;
            value.attempted = true;
            value.absent = true;
            value.failure = None;
            return Ok(None);
        };
        let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("md6-declaration-owner"));
        let source = crate::entities::declaration_text(&bytes);
        let parsed = parse(catalog, self.path.clone(), &source, scope);
        let mut value = self.record.value.lock().map_err(|_| "MD6 declaration record is poisoned")?;
        match parsed {
            Ok(parsed) => {
                let parsed = Arc::new(parsed);
                value.attempted = true;
                value.value = Arc::downgrade(&parsed);
                value.resident = Some(parsed.clone());
                value.absent = false;
                value.failure = None;
                if tracing() { eprintln!("MD6_DECL_ACQUIRE path={} identity={:p} reused=0 source_bytes={} tokens={}",
                    self.path, Arc::as_ptr(&parsed), parsed.source_bytes, parsed.tokens.len()); }
                Ok(Some(parsed))
            }
            Err(error) => {
                value.value = Weak::new();
                value.resident = None;
                value.attempted = true;
                value.absent = false;
                value.failure = Some(Arc::from(error.as_str()));
                Err(error)
            }
        }
    }
}

fn parse(catalog: &Catalog, path: Arc<str>, source: &str, scope: LoadContext) -> Result<Declaration, String> {
    let lexemes = crate::entities::declaration_lexemes(source)?;
    // Both readers use the same authored LOD0 selector. Keep only its token
    // index, so no second source buffer or mesh-name string stays resident.
    let mesh = crate::bmd6::compiled_mesh_name(source).filter(|name| name.ends_with(".md6mesh"));
    let lod_zero_token = mesh.and_then(|name| lexemes.iter().position(|&token| std::ptr::eq(token, name)))
        .and_then(|index| u32::try_from(index).ok());
    let tokens: Arc<[String]> = lexemes.into_iter().map(str::to_owned).collect();
    let mut depth = 0usize;
    let mut parent = None;
    let mut parent_record = None;
    let mut parent_declaration = None;
    let mut animsets = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "{" => depth += 1,
            "}" => depth = depth.checked_sub(1).ok_or("unbalanced MD6 declaration")?,
            "inherit" if depth == 1 => {
                let name = tokens.get(index + 1).ok_or("missing MD6 inherit name")?.clone();
                if !name.get(name.len().saturating_sub(4)..).is_some_and(|suffix| suffix.eq_ignore_ascii_case(".md6")) {
                    return Err("invalid MD6 inherit path".into());
                }
                let parent_path = format!("generated/decls/md6def/{name}.decl");
                let record = register(catalog, &parent_path, scope)?;
                parent_declaration = Some(record.load(catalog, catalog.has_overlay_file(&parent_path), scope)?
                    .ok_or_else(|| format!("{parent_path} is absent"))?);
                parent_record = Some(record);
                parent = Some(index + 1);
            }
            "animsets" if depth == 1 => {
                if tokens.get(index + 1).map(String::as_str) != Some("{") {
                    return Err("MD6 AnimSet list has no opening brace".into());
                }
                index += 2;
                while tokens.get(index).map(String::as_str) != Some("}") {
                    let name = tokens.get(index).ok_or("unterminated MD6 AnimSet list")?.trim_matches('"');
                    let path = format!("generated/decls/animset/{name}.decl");
                    let context = match scope {
                        LoadContext::Level => crate::edge_animation::CreationContext::Level,
                        LoadContext::PersistentHeap => crate::edge_animation::CreationContext::PersistentHeap,
                    };
                    animsets.push(animset::acquire_in_context(catalog, &path, context)?);
                    index += 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 { return Err("unbalanced MD6 declaration".into()); }
    let lod_zero_mesh = if let Some(index) = lod_zero_token {
        LodZeroMesh::Local(index)
    } else if mesh.is_none() && parent.is_some_and(|index| {
        crate::bmd6::compiled_mesh_parent(source) == Some(tokens[index].as_str())
    }) {
        LodZeroMesh::Inherited
    } else {
        LodZeroMesh::Unresolved
    };
    let source_crc = (tracing() || std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1")).then(|| {
        let mut crc = flate2::Crc::new();
        crc.update(source.as_bytes());
        crc
    });
    Ok(Declaration { path, tokens, lod_zero_mesh, parent, parent_record, parent_declaration, animsets, source_bytes: source.len(), source_crc })
}

fn normalized(path: &str) -> Cow<'_, str> {
    if path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
        Cow::Borrowed(path)
    } else { Cow::Owned(path.replace('\\', "/").to_ascii_lowercase()) }
}

/// Register a real declaration reference without loading its payload.
pub(crate) fn register(catalog: &Catalog, path: &str, scope: LoadContext) -> Result<Lease, String> {
    let path = normalized(path);
    let mut registry = catalog.md6_declarations.lock().map_err(|_| "MD6 declaration owner is poisoned")?;
    if let Some((stored, record)) = registry.records.get_key_value(path.as_ref()) {
        return Ok(Lease { path: stored.clone(), record: record.clone() });
    }
    let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("md6-declaration-registry"));
    registry.records.try_reserve(1).map_err(|error| format!("MD6 record registry: {error}"))?;
    registry.order.try_reserve(1).map_err(|error| format!("MD6 record order: {error}"))?;
    let path: Arc<str> = path.as_ref().into();
    let record = Arc::new(Record { scope: AtomicU8::new(scope as u8), value: Mutex::new(Snapshot::default()) });
    registry.records.insert(path.clone(), record.clone());
    registry.order.push(path.clone());
    Ok(Lease { path, record })
}

/// Lookup-only admission. A missing record stays missing, without archive IO.
pub(crate) fn find(catalog: &Catalog, path: &str) -> Result<Option<Lease>, String> {
    let path = normalized(path);
    let registry = catalog.md6_declarations.lock().map_err(|_| "MD6 declaration owner is poisoned")?;
    Ok(registry.records.get_key_value(path.as_ref()).map(|(path, record)| Lease { path: path.clone(), record: record.clone() }))
}

/// Retain the existing header together with the exact declaration generation
/// already acquired by a model. This does not parse or create a resource.
pub(crate) fn reference_generation(catalog: &Catalog, declaration: Arc<Declaration>)
    -> Result<ModelReference, String> {
    let record = find(catalog, &declaration.path)?.ok_or("MD6 declaration header is absent")?;
    Ok(ModelReference { record, declaration })
}

pub(crate) fn registered(catalog: &Catalog) -> Result<Vec<Lease>, String> {
    let registry = catalog.md6_declarations.lock().map_err(|_| "MD6 declaration owner is poisoned")?;
    Ok(registry.order.iter().filter_map(|path| registry.records.get(path).map(|record|
        Lease { path: path.clone(), record: record.clone() })).collect())
}

/// Resolve a model name at the real 7d7e40 phase. Lookup-only mode does not
/// create a record; creating mode admits one with the current load context.
pub(crate) fn resolve_model(catalog: &Catalog, name: &str, create: bool, context: LoadContext)
    -> Result<Option<ModelReference>, String> {
    if !name.get(name.len().saturating_sub(4)..).is_some_and(|suffix| suffix.eq_ignore_ascii_case(".md6")) {
        return Err(format!("invalid MD6 declaration name {name}"));
    }
    let path = format!("generated/decls/md6def/{name}.decl");
    let record = if create { Some(register(catalog, &path, context)?) } else { find(catalog, &path)? };
    let Some(record) = record else { return Ok(None) };
    record.model(catalog, context).map(Some)
}

/// Resolve the existing inherited binding reader through registered MD6
/// declarations. The returned leases keep the metadata generations used by
/// that binding alive for the model consumer.
pub(crate) fn resolve_binding(catalog: &Catalog, resource: &str, scope: LoadContext)
    -> Result<(Option<crate::bmd6::SkeletonBinding>, Vec<(String, Arc<Declaration>)>), String> {
    let mut declarations: Vec<(String, Arc<Declaration>)> = Vec::new();
    let binding = crate::bmd6::resolve_skeleton_binding_tokens(resource, |path| {
        let normalized_path = normalized(path);
        if let Some((_, source)) = declarations.iter().find(|(name, _)| name == normalized_path.as_ref()) {
            return Ok(Some(source.tokens.clone()));
        }
        match register(catalog, path, scope)?.load(catalog, catalog.has_overlay_file(path), scope)? {
            Some(source) => {
                let tokens = source.tokens.clone();
                let mut current = Some(source);
                while let Some(declaration) = current {
                    if declarations.iter().any(|(name, _)| name == declaration.path.as_ref()) { break; }
                    current = declaration.parent_declaration.clone();
                    declarations.push((declaration.path.to_string(), declaration));
                }
                Ok(Some(tokens))
            }
            None => Ok(None),
        }
    })?;
    Ok((binding, declarations))
}

/// 060b60 explicitly promotes a previously registered bootstrap record.
pub(crate) fn promote(lease: &Lease) {
    lease.record.scope.store(2, Ordering::Relaxed);
}

/// 060b60 visits every registered bootstrap record, not only loaded models.
pub(crate) fn promote_bootstrap_records(catalog: &Catalog) -> Result<(), String> {
    // Host: a cold parent-chain load may admit headers before publication.
    // Finish that existing load before visiting the bootstrap registry.
    let _loading = catalog.md6_loading.lock().map_err(|_| "MD6 declaration loader is poisoned")?;
    let registry = catalog.md6_declarations.lock().map_err(|_| "MD6 declaration owner is poisoned")?;
    for record in registry.records.values() { record.scope.store(2, Ordering::Relaxed); }
    Ok(())
}

pub(crate) fn release_level(catalog: &Catalog) {
    release_marker(catalog, PurgeMarker::Level);
}

/// 060180 selects an exact native marker; live metadata leases remain valid.
pub(crate) fn release_marker(catalog: &Catalog, marker: PurgeMarker) {
    // Host: load() releases its record lock for inherited IO/parse. Taking
    // its existing publication guard first prevents a late load from pinning
    // an otherwise unused payload after this scope has already been swept.
    let _loading = catalog.md6_loading.lock().unwrap_or_else(|error| error.into_inner());
    let mut registry = catalog.md6_declarations.lock().unwrap_or_else(|error| error.into_inner());
    let before = registry.records.len();
    let purge_level = matches!(marker, PurgeMarker::Level);
    // Release matching payloads together: a child declaration can retain an
    // ancestor's metadata/record. Checking parents before dropping children
    // would preserve dead headers merely because of hash iteration order.
    for record in registry.records.values().filter(|record| record.scope.load(Ordering::Relaxed) == marker as u8) {
        record.value.lock().unwrap_or_else(|error| error.into_inner()).resident = None;
    }
    registry.records.retain(|path, record| {
        if record.scope.load(Ordering::Relaxed) != marker as u8 { return true; }
        let mut snapshot = record.value.lock().unwrap_or_else(|error| error.into_inner());
        snapshot.previous.retain(|generation| generation.strong_count() != 0);
        let keep = Arc::strong_count(record) > 1 || snapshot.value.strong_count() != 0 || !snapshot.previous.is_empty();
        if !keep {
            if let Some(kind) = catalog.kind(path).filter(|kind| !purge_level || *kind != "md6Def") {
                crate::loading::forget(catalog.identity(), kind, path);
            }
        }
        keep
    });
    if purge_level {
        // Compatibility mesh resolution may have read the declaration
        // without loading its registered metadata. Keep credit only for
        // generations the declaration owner still retains.
        crate::loading::retain_catalog_kind(catalog.identity(), "md6Def", |path| {
            registry.records.get(path).is_some_and(|record| {
                let value = record.value.lock().unwrap_or_else(|error| error.into_inner());
                value.attempted && !value.absent
            })
        });
    }
    let Registry { records, order } = &mut *registry;
    order.retain(|path| records.contains_key(path));
    if tracing() {
        eprintln!("MD6_DECL_RELEASE_MARKER catalog={} marker={} before={} retained={} capacity={} ordered_capacity={}",
            catalog.identity(), marker as u8, before, records.len(), records.capacity(), order.capacity());
    }
}
