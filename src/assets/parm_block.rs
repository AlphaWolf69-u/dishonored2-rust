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

// 140060180 purges resource identities only after their owners retire.
static HISTORY_SCOPES: Mutex<usize> = Mutex::new(0);
struct HistoryLease;
impl HistoryLease {
    fn acquire() -> Arc<Self> {
        *HISTORY_SCOPES
            .lock()
            .unwrap_or_else(|error| error.into_inner()) += 1;
        Arc::new(Self)
    }
}
impl Drop for HistoryLease {
    fn drop(&mut self) {
        let mut scopes = HISTORY_SCOPES
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *scopes -= 1;
        if *scopes == 0 {
            let (count, bytes, slots, _) = crate::loading::retain_kind("pblock", |_| false);
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY pblock history release: entries={count} path_capacity_bytes={bytes} slots={slots}");
            }
        }
    }
}

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
    _history: Option<Arc<HistoryLease>>,
}

impl Drop for ParmBlock {
    fn drop(&mut self) {
        if self._history.is_some()
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
fn numeric_word(text: &str, storage: NumericStorage) -> Result<u32, String> {
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
            _history: None,
        });
    };
    let Some(list) = scope(edit, "m_pblock")? else {
        return Ok(ParmBlock {
            assignments: Vec::new(),
            _history: None,
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
        _history: None,
    })
}

#[derive(Default)]
struct Entry {
    current: Weak<ParmBlock>,
    previous: Vec<Weak<ParmBlock>>,
}

/// 0607f0/05feb0 identity is shared across all consumers in this catalog.
/// The header never owns a decoded block; actual FX/material leases do.
#[derive(Default)]
pub(crate) struct Registry {
    entries: HashMap<String, Arc<Mutex<Entry>>>,
}
impl Registry {
    fn record(&mut self, path: &str) -> Arc<Mutex<Entry>> {
        if let Some(header) = self.entries.get(path) { return header.clone(); }
        let header = Arc::new(Mutex::new(Entry::default()));
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
            if entry.current.strong_count() != 0 || !entry.previous.is_empty() { return true; }
            crate::loading::forget(catalog, "pblock", path);
            false
        });
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

/// Caller-local weak references keep the existing fast lookup and scope
/// retirement; decoded identities come from the one catalog class owner.
pub(crate) struct ParmBlockSources {
    catalog: u64,
    registry: Arc<Mutex<Registry>>,
    entries: HashMap<String, Weak<ParmBlock>>,
    history: Option<Arc<HistoryLease>>,
}
impl ParmBlockSources {
    pub(crate) fn new(catalog: &Catalog) -> Self {
        Self {
            catalog: catalog.identity(),
            registry: catalog.parameter_blocks.get_or_init(Default::default).clone(),
            entries: HashMap::new(),
            history: None,
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
            self.history = None;
            self.catalog = catalog.identity();
            self.registry = catalog.parameter_blocks.get_or_init(Default::default).clone();
        }
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        let path = normalized
            .strip_prefix("generated/decls/pblock/")
            .unwrap_or(&normalized);
        let path = path.strip_suffix(".decl").unwrap_or(path);
        let resource = format!("generated/decls/pblock/{path}.decl");
        let shared = !catalog.has_overlays();
        if shared {
            if let Some(block) = self.entries.get(&resource).and_then(Weak::upgrade) {
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
            .record(&resource);
        // Metadata lookup may acquire renderParm declarations, but never
        // another pblock. Only this resource is locked across its read/parse.
        let mut entry = header.lock().map_err(|_| "parameter-block record is poisoned")?;
        if shared {
            if let Some(block) = entry.current.upgrade() {
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
        // Protect the read while it is in flight, but do not leave the cache
        // owning an otherwise unused scope if IO or parsing fails.
        let history = self.history.clone().unwrap_or_else(HistoryLease::acquire);
        let bytes = catalog
            .read(&resource)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("parameter block absent: {resource}"))?;
        let mut block = parse(&bytes, metadata)?;
        if trace {
            let cycles = started.zip(crate::loading::thread_cycles())
                .map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("MEMORY particle parameter block: reused=0 source_bytes={} assignments={} cycles={cycles}", bytes.len(), block.assignments.len());
        }
        self.history.get_or_insert_with(|| history.clone());
        block._history = Some(history);
        let block = Arc::new(block);
        if trace { eprintln!("PBLOCK_LEASE catalog={} path={resource} identity={:p} reused=false shared_class=true", self.catalog, Arc::as_ptr(&block)); }
        let previous = std::mem::replace(&mut entry.current, Arc::downgrade(&block));
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
        if self.entries.is_empty() {
            self.history = None;
            return true;
        }
        false
    }
}

impl Drop for ParmBlockSources {
    fn drop(&mut self) {
        self.registry.lock().unwrap_or_else(|error| error.into_inner()).retire_unused(self.catalog);
    }
}
