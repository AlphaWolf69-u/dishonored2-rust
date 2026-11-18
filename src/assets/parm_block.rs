//! Typed arkDeclParmBlock data (14098cdd0, 14028d0c0).
use crate::{
    archive::Catalog,
    entities::{self, Value},
    render_program::{NumericStorage, ParameterMetadata, RenderParameterKind},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

pub(crate) enum ParmValue {
    Words([u32; 4]),
    /// Preserve null distinctly; interpretation depends on the registered
    /// kind (for example, null image view resolves its registered default).
    Name(Option<String>),
}

pub(crate) struct Assignment {
    pub name: String,
    pub kind: RenderParameterKind,
    pub value: ParmValue,
}

pub(crate) struct ParmBlock {
    /// Kept in authored order; the original compaction retains the last
    /// assignment to a duplicate parameter (140343670).
    pub assignments: Vec<Assignment>,
    // A weak back-reference lets a caller-local hit rejoin a record whose
    // native marker was purged while this exact generation remained leased.
    header: Option<Weak<Mutex<Entry>>>,
}

impl Drop for ParmBlock {
    fn drop(&mut self) {
        if self.header.is_some()
            && std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("PBLOCK_RELEASE identity={:p} assignments={}", self, self.assignments.len());
        }
    }
}

impl ParmBlock {
    /// The declaration preserves authored duplicates; the last assignment is
    /// the value retained by 140343670. Return raw lanes without conversion.
    pub(crate) fn numeric_parameter(&self, name: &str) -> Result<Option<[u32; 4]>, String> {
        let Some(assignment) = self.assignments.iter().rev()
            .find(|assignment| assignment.name.eq_ignore_ascii_case(name)) else { return Ok(None) };
        match &assignment.value {
            ParmValue::Words(words) => Ok(Some(*words)),
            ParmValue::Name(_) => Err(format!("render parameter {name} has a resource payload")),
        }
    }

    /// Prepare an immutable block for the existing ordered parameter scopes.
    /// The image owner resolves authored references and the registered default
    /// for null or missing images, as in 14098cdd0. Resolving them here would
    /// create a second image owner and could keep an image alive after the FX.
    pub(crate) fn draw_parameters(
        &self,
        mut resolve_image: impl FnMut(&str, Option<&str>) -> Result<String, String>,
    ) -> Result<crate::render_program::SerializedParameterBlock, String> {
        use crate::render_program::{
            SerializedParameter, SerializedParameterBlock, SerializedParameterValue,
        };
        let mut parameters = Vec::with_capacity(self.assignments.len());
        for assignment in &self.assignments {
            let value = match (&assignment.value, assignment.kind) {
                (ParmValue::Words(words), _) => SerializedParameterValue::Words(*words),
                (
                    ParmValue::Name(name),
                    RenderParameterKind::Texture | RenderParameterKind::MultisampleTexture,
                ) => SerializedParameterValue::Name(
                    resolve_image(&assignment.name, name.as_deref())?.into_bytes(),
                ),
                // Other reference kinds need their respective resource owners;
                // a string in a draw block is not an acquired resource lease.
                (ParmValue::Name(_), kind) => {
                    return Err(format!("parameter-block {kind:?} binding is not connected"))
                }
            };
            parameters.push(SerializedParameter {
                name: assignment.name.as_bytes().to_vec(),
                kind: assignment.kind,
                value,
            });
        }
        Ok(SerializedParameterBlock {
            flags: 0,
            parameters,
        })
    }
}

type Fields = [(String, Value)];
fn field<'a>(fields: &'a Fields, name: &str) -> Option<&'a Value> {
    fields
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
}
fn scope<'a>(fields: &'a Fields, name: &str) -> Result<Option<&'a Fields>, String> {
    match field(fields, name) {
        None => Ok(None),
        Some(Value::Scope(value)) => Ok(Some(value)),
        Some(_) => Err(format!("parameter block {name} is not a scope")),
    }
}
fn atom<'a>(value: &'a Value) -> Result<&'a str, String> {
    match value {
        Value::Atom(value) => Ok(value.trim_matches('"')),
        _ => Err("parameter block expected an atom".into()),
    }
}

/// Use the same token conversion as registered numeric defaults (1401461c0)
/// rather than a second host-dependent float conversion.
pub(crate) fn numeric_word(text: &str, storage: NumericStorage) -> Result<u32, String> {
    let text = text.trim();
    let (negative, number) = text
        .strip_prefix('-')
        .map_or((false, text), |rest| (true, rest.trim_start()));
    if number.is_empty() {
        return Err("empty parameter-block number".into());
    }
    let floating = number.contains(['.', 'e', 'E']) || number.ends_with(['f', 'F', 'l', 'L']);
    let flags = if number.starts_with("0x") || number.starts_with("0X") {
        4
    } else if number.starts_with("0b") || number.starts_with("0B") {
        0x10
    } else if floating {
        0x100
    } else if number.len() > 1 && number.starts_with('0') {
        8
    } else {
        2
    };
    let (integer, float, _) = entities::numeric_token_value(number, flags)?;
    Ok(match storage {
        NumericStorage::Float | NumericStorage::Color => {
            if negative {
                float ^ 0x8000_0000
            } else {
                float
            }
        }
        NumericStorage::Signed => {
            if negative {
                (integer as u32).wrapping_neg()
            } else {
                integer as u32
            }
        }
        // 14014ec30 sends both integer types through 140149060 ->
        // 1401342f0, which negates the raw 32-bit word for a leading minus.
        NumericStorage::Unsigned => {
            if negative {
                (integer as u32).wrapping_neg()
            } else {
                integer as u32
            }
        }
    })
}

fn numeric(fields: &Fields, storage: NumericStorage, lanes: u8) -> Result<ParmValue, String> {
    if matches!(storage, NumericStorage::Color) {
        let mut words = [(-1.0f32).to_bits(); 4];
        if let Some(color) = scope(fields, "m_color")? {
            for (i, key) in ["r", "g", "b", "a"].iter().enumerate() {
                if let Some(value) = field(color, key) {
                    words[i] = numeric_word(atom(value)?, NumericStorage::Color)?;
                }
            }
        }
        return Ok(ParmValue::Words(words));
    }
    let prefix = match storage {
        NumericStorage::Float => "m_float",
        NumericStorage::Signed => "m_signedInteger",
        NumericStorage::Unsigned => "m_unsignedInteger",
        NumericStorage::Color => unreachable!(),
    };
    let mut words = [0u32; 4];
    for (i, word) in words.iter_mut().take(usize::from(lanes)).enumerate() {
        if let Some(value) = field(fields, &format!("{prefix}[{i}]")) {
            *word = numeric_word(atom(value)?, storage)?;
        }
    }
    Ok(ParmValue::Words(words))
}

fn resource(fields: &Fields, key: &str) -> Result<ParmValue, String> {
    let name = field(fields, key).map(atom).transpose()?;
    Ok(ParmValue::Name(
        name.filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
            .map(str::to_owned),
    ))
}

fn assignment(
    fields: &Fields,
    metadata: &mut impl FnMut(&str) -> Result<ParameterMetadata, String>,
) -> Result<Assignment, String> {
    let name =
        atom(field(fields, "m_parm").ok_or("parameter block assignment has no renderParm")?)?
            .replace('\\', "/")
            .to_ascii_lowercase();
    let meta = metadata(&name)?;
    let value = scope(fields, "m_value")?.unwrap_or(&[]);
    use RenderParameterKind::*;
    let value = match meta.kind {
        Uniform | Option | Semantic => numeric(
            value,
            meta.numeric
                .ok_or("numeric renderParm has no numeric type")?,
            meta.lanes,
        )?,
        Texture | MultisampleTexture => resource(value, "m_ImageView")?,
        Sampler => resource(value, "m_sampler")?,
        Program => resource(value, "m_program")?,
        Material => resource(value, "m_material")?,
        ParameterBlock => resource(value, "m_parmBlock")?,
        String | Struct => resource(value, "m_string")?,
        ReadOnlyBuffer
        | ReadWriteBuffer
        | ConstantBuffer
        | StructuredBuffer
        | StructuredConstantBuffer => {
            return Err(
                "original parameter-block deserializer does not accept GPU buffer values".into(),
            )
        }
    };
    Ok(Assignment {
        name,
        kind: meta.kind,
        value,
    })
}

fn parse(
    bytes: &[u8],
    mut metadata: impl FnMut(&str) -> Result<ParameterMetadata, String>,
) -> Result<ParmBlock, String> {
    let root = entities::parse_declaration(&entities::declaration_text(bytes))?;
    let Some(edit) = scope(&root, "edit")? else {
        return Ok(ParmBlock {
            assignments: Vec::new(),
            header: None,
        });
    };
    let Some(list) = scope(edit, "m_pblock")? else {
        return Ok(ParmBlock {
            assignments: Vec::new(),
            header: None,
        });
    };
    let count = field(list, "num")
        .map(atom)
        .transpose()?
        .ok_or("pblock list has no count")?
        .parse::<usize>()
        .map_err(|e| format!("pblock list count: {e}"))?;
    let mut entries: Vec<Option<&Fields>> = vec![None; count];
    for (name, value) in list {
        if name == "num" {
            continue;
        }
        let index = name
            .strip_prefix("item[")
            .and_then(|x| x.strip_suffix(']'))
            .ok_or_else(|| format!("unknown pblock list field {name}"))?
            .parse::<usize>()
            .map_err(|e| format!("pblock index: {e}"))?;
        let Value::Scope(fields) = value else {
            return Err(format!("pblock item {index} is not a scope"));
        };
        *entries.get_mut(index).ok_or("pblock index exceeds count")? = Some(fields);
    }
    let mut assignments = Vec::with_capacity(count);
    for entry in entries {
        assignments.push(assignment(
            entry.ok_or("pblock assignment is missing")?,
            &mut metadata,
        )?);
    }
    Ok(ParmBlock {
        assignments,
        header: None,
    })
}

struct Entry {
    current: Weak<ParmBlock>,
    previous: Vec<Weak<ParmBlock>>,
    resident: Option<Arc<ParmBlock>>,
    failure: Option<Arc<str>>,
    /// None is the pre-existing host acquisition policy, whose native heap
    /// context is not supplied by the particle/material caller.
    marker: Option<u8>,
    retired: bool,
}
impl Entry {
    fn new(marker: Option<u8>) -> Self {
        Self { current: Weak::new(), previous: Vec::new(), resident: None, failure: None, marker, retired: false }
    }
    fn retain_acquired(&mut self, block: &Arc<ParmBlock>) {
        if self.retired { self.marker = None; self.retired = false; }
        self.failure = None;
        self.resident = Some(block.clone());
    }
}

/// 0607f0/05feb0 identity is shared across all consumers in this catalog.
/// A decoded generation remains resident until its actual marker is purged;
/// consumers may retain old generations after that boundary.
#[derive(Default)]
pub(crate) struct Registry {
    entries: HashMap<String, Arc<Mutex<Entry>>>,
}
impl Registry {
    fn record(&mut self, path: &str, marker: Option<u8>) -> Arc<Mutex<Entry>> {
        if let Some(header) = self.entries.get(path) { return header.clone(); }
        let header = Arc::new(Mutex::new(Entry::new(marker)));
        self.entries.insert(path.to_owned(), header.clone());
        header
    }

    pub(crate) fn retire_unused(&mut self, catalog: u64) {
        self.entries.retain(|path, header| {
            // Acquisitions take the header before IO; its payload may not yet
            // be published. Never remove that identity/read while in flight.
            if Arc::strong_count(header) > 1 { return true; }
            let mut entry = header.lock().unwrap_or_else(|error| error.into_inner());
            entry.previous.retain(|generation| generation.strong_count() != 0);
            if !entry.retired { return true; }
            if entry.current.strong_count() != 0 || !entry.previous.is_empty() { return true; }
            crate::loading::forget(catalog, "pblock", path);
            false
        });
    }

    /// 060180 removes only one native marker. Live/in-flight generations
    /// remain checked by the existing header and weak generation owner.
    fn release_marker(&mut self, catalog: u64, marker: u8) {
        let before = self.entries.len();
        let mut matched = 0usize;
        let mut resident = 0usize;
        for header in self.entries.values() {
            let mut entry = header.lock().unwrap_or_else(|error| error.into_inner());
            if (entry.marker == Some(marker) || marker == 0 && entry.marker.is_none()) && !entry.retired {
                matched += 1;
                resident += usize::from(entry.resident.is_some());
                entry.retired = true;
                entry.resident = None;
                entry.failure = None;
            }
        }
        self.retire_unused(catalog);
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("PBLOCK_PURGE catalog={catalog} marker={marker} matched={matched} resident_released={resident} removed={} retained={}",
                before - self.entries.len(), self.entries.len());
        }
    }

    pub(crate) fn release_session(&mut self) {
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            let live = self.entries.values().map(|header| {
                let entry = header.lock().unwrap_or_else(|error| error.into_inner());
                usize::from(entry.current.strong_count() != 0)
                    + entry.previous.iter().filter(|generation| generation.strong_count() != 0).count()
            }).sum::<usize>();
            eprintln!("PBLOCK_REGISTRY_RELEASE records={} live_generations={live}", self.entries.len());
        }
        drop(std::mem::take(&mut self.entries));
    }
}

fn resource_path(name: &str) -> String {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    let path = normalized.strip_prefix("generated/decls/pblock/").unwrap_or(&normalized);
    let path = path.strip_suffix(".decl").unwrap_or(path);
    format!("generated/decls/pblock/{path}.decl")
}

/// 0661e0 registers without payload IO, and explicitly writes marker 1 for
/// selected pblocks even if the header already existed.
pub(crate) fn register_preload(catalog: &Catalog, name: &str,
    context: crate::edge_animation::CreationContext, persistent_kind: bool) -> Result<(), String> {
    let registry = catalog.parameter_blocks.get_or_init(Default::default);
    let path = resource_path(name);
    let mut registry = registry.lock().map_err(|_| "parameter-block registry is poisoned")?;
    let marker = if persistent_kind { 1 } else { context as u8 };
    let trace = std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1");
    let reused = trace && registry.entries.contains_key(&path);
    let header = registry.record(&path, Some(marker));
    let mut entry = header.lock().map_err(|_| "parameter-block record is poisoned")?;
    if entry.retired {
        entry.retired = false;
        entry.marker = Some(marker);
        entry.resident = entry.current.upgrade();
    }
    if persistent_kind { entry.marker = Some(1); }
    if trace {
        eprintln!("PBLOCK_HEADER catalog={} path={path} marker={:?} reused={reused}", catalog.identity(), entry.marker);
    }
    Ok(())
}

pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    let Some(registry) = catalog.parameter_blocks.get() else { return Ok(()) };
    let registry = registry.lock().map_err(|_| "parameter-block registry is poisoned")?;
    for header in registry.entries.values() {
        let mut entry = header.lock().map_err(|_| "parameter-block record is poisoned")?;
        entry.marker = Some(2);
        entry.retired = false;
        if entry.resident.is_none() { entry.resident = entry.current.upgrade(); }
    }
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
        eprintln!("PBLOCK_BOOTSTRAP catalog={} headers={}", catalog.identity(), registry.entries.len());
    }
    Ok(())
}

pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(registry) = catalog.parameter_blocks.get() {
        registry.lock().unwrap_or_else(|error| error.into_inner()).release_marker(catalog.identity(), marker);
    }
}

/// Caller-local weak references keep the existing fast lookup and scope
/// retirement; decoded identities come from the one catalog class owner.
pub(crate) struct ParmBlockSources {
    catalog: u64,
    registry: Arc<Mutex<Registry>>,
    entries: HashMap<String, Weak<ParmBlock>>,
}
impl ParmBlockSources {
    pub(crate) fn new(catalog: &Catalog) -> Self {
        Self {
            catalog: catalog.identity(),
            registry: catalog.parameter_blocks.get_or_init(Default::default).clone(),
            entries: HashMap::new(),
        }
    }
    pub(crate) fn acquire(
        &mut self,
        catalog: &Catalog,
        name: &str,
        metadata: impl FnMut(&str) -> Result<ParameterMetadata, String>,
    ) -> Result<Arc<ParmBlock>, String> {
        let trace = std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1");
        let started = trace.then(crate::loading::thread_cycles).flatten();
        if self.catalog != catalog.identity() {
            self.entries.clear();
            self.catalog = catalog.identity();
            self.registry = catalog.parameter_blocks.get_or_init(Default::default).clone();
        }
        let resource = resource_path(name);
        let shared = !catalog.has_overlays();
        if shared {
            if let Some(block) = self.entries.get(&resource).and_then(Weak::upgrade) {
                let header = block.header.as_ref().and_then(Weak::upgrade).ok_or("parameter-block record is absent")?;
                header.lock().map_err(|_| "parameter-block record is poisoned")?.retain_acquired(&block);
                if trace {
                    let cycles = started.zip(crate::loading::thread_cycles())
                        .map_or(0, |(start, end)| end.saturating_sub(start));
                    eprintln!("MEMORY particle parameter block: reused=1 source_bytes=0 assignments={} cycles={cycles}", block.assignments.len());
                    eprintln!("PBLOCK_LEASE catalog={} path={resource} identity={:p} reused=true shared_class=false", self.catalog, Arc::as_ptr(&block));
                }
                return Ok(block);
            }
        }
        let header = self.registry.lock().map_err(|_| "parameter-block registry is poisoned")?
            .record(&resource, None);
        // Metadata lookup may acquire renderParm declarations, but never
        // another pblock. Only this resource is locked across its read/parse.
        let mut entry = header.lock().map_err(|_| "parameter-block record is poisoned")?;
        if shared {
            // 05feb0/05ede0 retain a failed immutable load on its actual
            // admitted header. Editable generations deliberately retry.
            if let Some(error) = &entry.failure { return Err(error.to_string()); }
        }
        if shared {
            if let Some(block) = entry.current.upgrade() {
                entry.retain_acquired(&block);
                if trace {
                    let cycles = started.zip(crate::loading::thread_cycles())
                        .map_or(0, |(start, end)| end.saturating_sub(start));
                    eprintln!("MEMORY particle parameter block: reused=1 source_bytes=0 assignments={} cycles={cycles} shared_class=1", block.assignments.len());
                    eprintln!("PBLOCK_LEASE catalog={} path={resource} identity={:p} reused=true shared_class=true", self.catalog, Arc::as_ptr(&block));
                }
                self.entries.insert(resource, Arc::downgrade(&block));
                return Ok(block);
            }
        }
        let loaded = (|| {
            let bytes = catalog.read(&resource).map_err(|error| error.to_string())?
                .ok_or_else(|| format!("parameter block absent: {resource}"))?;
            Ok::<_, String>((parse(&bytes, metadata)?, bytes.len()))
        })();
        let (mut block, source_bytes) = match loaded {
            Ok(value) => value,
            Err(error) => {
                if shared { entry.failure = Some(Arc::from(error.as_str())); }
                return Err(error);
            }
        };
        if trace {
            let cycles = started.zip(crate::loading::thread_cycles())
                .map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("MEMORY particle parameter block: reused=0 source_bytes={source_bytes} assignments={} cycles={cycles}", block.assignments.len());
        }
        block.header = Some(Arc::downgrade(&header));
        let block = Arc::new(block);
        if trace { eprintln!("PBLOCK_LEASE catalog={} path={resource} identity={:p} reused=false shared_class=true", self.catalog, Arc::as_ptr(&block)); }
        let previous = std::mem::replace(&mut entry.current, Arc::downgrade(&block));
        entry.retain_acquired(&block);
        entry.previous.retain(|generation| generation.strong_count() != 0);
        if previous.strong_count() != 0 { entry.previous.push(previous); }
        if shared {
            self.entries.insert(resource, Arc::downgrade(&block));
        }
        Ok(block)
    }
    /// True when this acquisition table can itself be released. Editable
    /// blocks are never entered here; their live Arcs carry their own lease.
    pub(crate) fn retire_unused(&mut self) -> bool {
        self.entries.retain(|_, value| value.strong_count() != 0);
        self.registry.lock().unwrap_or_else(|error| error.into_inner()).retire_unused(self.catalog);
        self.entries.is_empty()
    }
}

impl Drop for ParmBlockSources {
    fn drop(&mut self) {
        self.registry.lock().unwrap_or_else(|error| error.into_inner()).retire_unused(self.catalog);
    }
}
