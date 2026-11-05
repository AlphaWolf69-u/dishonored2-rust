//! Level-preparation ownership for immutable HKX shape sources.
//! 140779530 -> 14008c840 acquires physhape identity through 1400607f0.
use super::{parse_hkx, PhysWorld, TagField, TagItem, TagType};
use crate::assets::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::{Arc, OnceLock}};

pub struct ShapeSource {
    pub(crate) bytes: Vec<u8>,
    pub(crate) physics: PhysWorld,
}

pub(crate) enum ShapeSourceError { Read(String), Parse(String) }
impl ShapeSourceError {
    pub(crate) fn message(self) -> String { match self { Self::Read(e) | Self::Parse(e) => e } }
}

#[derive(Clone, Copy)]
pub(crate) enum ShapeUse { Area, Physics, Wind }

#[derive(Default)]
struct Counts { requests: usize, reads: usize, bytes: usize, hits: usize }

/// Cache only early area inputs; physics-only and late wind shapes do not grow
/// this map. It is dropped after all level preparation consumers have run.
pub(crate) struct ShapeSources {
    enabled: bool,
    accepting: bool,
    trace: bool,
    sources: HashMap<String, Arc<ShapeSource>>,
    counts: [Counts; 3],
    source_capacity: usize,
    schema_capacity: usize,
    key_capacity: usize,
}

impl Default for ShapeSources {
    fn default() -> Self {
        Self { enabled: false, accepting: false, trace: false, sources: HashMap::new(), counts: Default::default(),
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

    pub(crate) fn finish_areas(&mut self) { self.accepting = false; }

    pub(crate) fn acquire(&mut self, catalog: &Catalog, path: &str, role: ShapeUse)
        -> Result<Option<Arc<ShapeSource>>, ShapeSourceError>
    {
        let counts = &mut self.counts[role as usize];
        counts.requests += 1;
        let key: Cow<'_, str> = if path.bytes().all(|b| b != b'\\' && !b.is_ascii_uppercase()) {
            Cow::Borrowed(path)
        } else { Cow::Owned(path.replace('\\', "/").to_ascii_lowercase()) };
        if self.enabled {
            if let Some(source) = self.sources.get(key.as_ref()) {
                counts.hits += 1;
                log_source(self.trace, role, path, source, true);
                return Ok(Some(source.clone()));
            }
        }
        counts.reads += 1;
        let Some(bytes) = catalog.read(path).map_err(|e| ShapeSourceError::Read(e.to_string()))? else { return Ok(None); };
        counts.bytes += bytes.len();
        let physics = parse_hkx(&bytes).map_err(ShapeSourceError::Parse)?;
        let source = Arc::new(ShapeSource { bytes, physics });
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
}

fn log_source(trace: bool, role: ShapeUse, path: &str, source: &ShapeSource, reused: bool) {
    if !trace { return; }
    let mut crc = flate2::Crc::new(); crc.update(&source.bytes);
    let role = match role { ShapeUse::Area => "area", ShapeUse::Physics => "physics", ShapeUse::Wind => "wind" };
    eprintln!("MEMORY shape source request: role={role} path={path} bytes={} signature={:08x} reused={reused}", source.bytes.len(), crc.sum());
}

impl Drop for ShapeSources {
    fn drop(&mut self) {
        if !self.trace { return; }
        for (name, count) in ["area", "physics", "wind"].into_iter().zip(&self.counts) {
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
