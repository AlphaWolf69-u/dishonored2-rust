//! Level-preparation ownership for immutable HKX shape sources.
//! 140779530 -> 14008c840 acquires physhape identity through 1400607f0.
use super::{parse_hkx, PhysWorld, TagField, TagItem, TagType};
use crate::assets::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::{Arc, OnceLock}};

// 140060180 retires map resource identities separately from the request
// list. The rewrite has no session-owned physhape cache: preparation and the
// published level hold the scope until no owner can reuse its identities.
static SHAPE_SCOPES: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
pub(crate) struct ShapeHistoryLease;
impl ShapeHistoryLease {
    fn acquire() -> Arc<Self> {
        *SHAPE_SCOPES.lock().unwrap_or_else(|error| error.into_inner()) += 1;
        Arc::new(Self)
    }
}
impl Drop for ShapeHistoryLease {
    fn drop(&mut self) {
        let mut scopes = SHAPE_SCOPES.lock().unwrap_or_else(|error| error.into_inner());
        *scopes -= 1;
        if *scopes != 0 { return; }
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let start = trace.then(crate::loading::thread_cycles).flatten();
        let (entries, bytes, slots, _) = crate::loading::retain_kind("physhape", |_| false);
        if trace {
            let cycles = start.zip(crate::loading::thread_cycles())
                .map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("MEMORY shape history release: entries={entries} path_capacity_bytes={bytes} slots={slots} cycles={cycles}");
        }
    }
}

pub struct ShapeSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) physics: PhysWorld,
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
    let Some(bytes) = catalog.read(path).map_err(|error| ShapeSourceError::Read(error.to_string()))? else { return Ok(None); };
    counts.bytes += bytes.len();
    let physics = parse_hkx(&bytes).map_err(ShapeSourceError::Parse)?;
    Ok(Some(ShapeSource { bytes, physics }))
}

/// Cache only early area inputs; physics-only and late wind shapes do not grow
/// this map. It is dropped after all level preparation consumers have run.
pub(crate) struct ShapeSources {
    history: Arc<ShapeHistoryLease>,
    enabled: bool,
    accepting: bool,
    trace: bool,
    sources: HashMap<String, Arc<ShapeSource>>,
    counts: [Counts; 4],
    source_capacity: usize,
    schema_capacity: usize,
    key_capacity: usize,
}

impl Default for ShapeSources {
    fn default() -> Self {
        Self { history: ShapeHistoryLease::acquire(), enabled: false, accepting: false, trace: false, sources: HashMap::new(), counts: Default::default(),
            source_capacity: 0, schema_capacity: 0, key_capacity: 0 }
    }
}

impl ShapeSources {
    pub(crate) fn new(catalog: &Catalog) -> Self {
        let enabled = !catalog.has_overlays()
            && std::env::var("DH2_DEBUG_SEPARATE_TRIGGER_SOURCES").as_deref() != Ok("1");
        let trace = std::env::var("DH2_DEBUG_TRIGGER_SOURCES").as_deref() == Ok("1");
        let mut sources = Self::default();
        sources.enabled = enabled;
        sources.accepting = enabled;
        sources.trace = trace;
        sources
    }

    pub(crate) fn history_lease(&self) -> Arc<ShapeHistoryLease> { self.history.clone() }

    pub(crate) fn finish_areas(&mut self) { self.accepting = false; }

    pub(crate) fn acquire(&mut self, catalog: &Catalog, path: &str, role: ShapeUse)
        -> Result<Option<Arc<ShapeSource>>, ShapeSourceError>
    {
        let counts = &mut self.counts[role as usize];
        counts.requests += 1;
        let key = source_key(path);
        if self.enabled {
            if let Some(source) = self.sources.get(key.as_ref()) {
                counts.hits += 1;
                log_source(self.trace, role, path, source, true);
                return Ok(Some(source.clone()));
            }
        }
        let Some(source) = read_source(catalog, path, counts)? else { return Ok(None); };
        let source = Arc::new(source);
        log_source(self.trace, role, path, &source, false);
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
                counts.hits += 1;
                log_source(self.trace, role, path, source, true);
                return source.physics.standalone_shapes(&source.bytes).map(Some);
            }
        }
        let Some(source) = read_source(catalog, path, counts).map_err(ShapeSourceError::message)? else { return Ok(None); };
        log_source(self.trace, role, path, &source, false);
        source.physics.standalone_shapes(&source.bytes).map(Some)
    }
}

pub(crate) fn load_model_havok(
    catalog: &Catalog,
    path: &str,
    depth: usize,
) -> Result<Vec<[[f32; 3]; 3]>, String> {
    if depth > 4 {
        return Err("HKX alias chain exceeds four links".into());
    }
    let bytes = catalog
        .read(path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("HKX target not indexed: {path}"))?;
    if let Some((target, factor)) = super::standalone_hkx_alias(&bytes) {
        let mut triangles = load_model_havok(catalog, &target, depth + 1)?;
        for triangle in &mut triangles {
            for vertex in triangle {
                for axis in 0..3 {
                    vertex[axis] *= factor;
                }
            }
        }
        return Ok(triangles);
    }
    let physics = super::parse_hkx(&bytes)?;
    let geometry = physics.standalone_shapes(&bytes)?;
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
    EMPTY.get_or_init(|| Arc::new(ShapeSource { bytes: Vec::new(), physics: PhysWorld {
        materials: Vec::new(), material_map: Vec::new(), sdk_version: String::new(),
        types: Vec::new(), items: Vec::new(), data_start: 0, data_size: 0,
    } })).clone()
}
