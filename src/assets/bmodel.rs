//! Read-only decoder for static BML1/BTRI model geometry.

use std::{collections::{HashMap, HashSet}, io::{self, Read}, mem::size_of, sync::{Arc, Mutex}};

const STREAM_THRESHOLD: usize = 256 * 1024;
const READ_BLOCK_SIZE: usize = 8 * 1024;

/// Host preparation payload for common model identities. Native lookup
/// 1400607f0 and bootstrap promotion 140060b60 retain the common resource.
/// Keep only requested decoded models and leave GPU meshes to their owner.
pub(crate) struct CommonModels {
    paths: HashSet<String>,
    models: Mutex<HashMap<String, Arc<Model>>>,
    reuse: bool,
}

impl CommonModels {
    pub(crate) fn shared(catalog: &super::archive::Catalog) -> Option<&Self> {
        if catalog.has_overlays() {
            return None;
        }
        catalog.common_models.get_or_init(|| {
            let language = super::lang::read_text_language(catalog.base()).map_err(|error| error.to_string())?;
            let mut paths = HashSet::new();
            for name in ["init", "common"] {
                let list = super::map_resources::acquire_list(catalog.base(), catalog, name, language)
                    .map_err(|error| error.to_string())?;
                paths.extend(list.iter().filter(|resource| resource.kind.eq_ignore_ascii_case("model"))
                    .map(|resource| resource.path.clone()));
            }
            let reuse = std::env::var("DH2_DEBUG_SEPARATE_COMMON_MODELS").as_deref() != Ok("1");
            Ok(Self { paths, models: Mutex::default(), reuse })
        }).as_ref().ok()
    }
}

pub(crate) enum ModelSource {
    Level(Model),
    Common(Arc<Model>),
}

impl std::ops::Deref for ModelSource {
    type Target = Model;

    fn deref(&self) -> &Model {
        match self { Self::Level(model) => model, Self::Common(model) => model }
    }
}

/// Load one model without extending its lifetime past the caller's lease.
pub fn load(catalog: &super::archive::Catalog, path: &str) -> Result<Option<Model>, String> {
    if !catalog.has_overlays()
        && catalog.resource_size(path).is_some_and(|size| size >= STREAM_THRESHOLD)
    {
        return catalog.with_stream(path, |reader, size| {
            let mut scratch = [0; READ_BLOCK_SIZE];
            let mut input = ModelInput {
                source: ModelSourceBytes::Stream(reader, &mut scratch),
                length: size,
                at: 0,
            };
            let model = read_model(&mut input).map_err(|error| format!("{path}: {error}"))?;
            // Consume the opaque tail so the archive reader can validate the member.
            let expected = model.bytes_after_bindings as u64;
            let read = io::copy(&mut reader.take(expected), &mut io::sink())
                .map_err(|error| format!("{path}: {error}"))?;
            if read != expected {
                return Err(format!("{path}: truncated model tail"));
            }
            Ok(model)
        });
    }
    catalog.read(path).map_err(|error| error.to_string())?
        .map(|bytes| parse(&bytes).map_err(|error| format!("{path}: {error}")))
        .transpose()
}

pub(crate) fn acquire(
    catalog: &super::archive::Catalog,
    path: &str,
    common: Option<&CommonModels>,
) -> Result<Option<ModelSource>, String> {
    let read = || load(catalog, path);
    let Some(common) = common.filter(|owner| owner.paths.contains(path)) else {
        return Ok(read()?.map(ModelSource::Level));
    };
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    if !common.reuse {
        let model = read()?;
        if trace {
            if let Some(model) = &model { report_model(path, model, "acquired"); }
        }
        return Ok(model.map(ModelSource::Level));
    }
    let mut models = common.models.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(model) = models.get(path) {
        crate::loading::read_catalog(catalog.identity(), "model", path);
        if trace { report_model(path, model, "reused"); }
        return Ok(Some(ModelSource::Common(model.clone())));
    }
    let _tag = crate::platform::allocation_tags::tag("common static model sources");
    let Some(model) = read()? else { return Ok(None); };
    let model = Arc::new(model);
    if trace { report_model(path, &model, "acquired"); }
    models.insert(path.to_owned(), model.clone());
    Ok(Some(ModelSource::Common(model)))
}

fn report_model(path: &str, model: &Model, action: &str) {
    let mut crc = flate2::Crc::new();
    let mut geometry_bytes = 0;
    for surface in &model.surfaces {
        crc.update(surface.material_path.as_bytes());
        for value in surface.authored_bounds.iter().flatten()
            .chain(surface.positions.iter().flatten())
        {
            crc.update(&value.to_le_bytes());
        }
        // Keep diagnostic signatures comparable with the earlier float staging.
        for value in surface.uvs.iter().flatten() { crc.update(&value.to_f32().to_le_bytes()); }
        for channels in &surface.packed_channels { crc.update(channels); }
        for index in &surface.indices { crc.update(&index.to_le_bytes()); }
        geometry_bytes += surface.positions.capacity() * size_of::<[f32; 3]>()
            + surface.uvs.capacity() * size_of::<[half::f16; 2]>()
            + surface.packed_channels.capacity() * size_of::<[u8; 16]>()
            + surface.indices.capacity() * size_of::<u16>();
    }
    eprintln!("MEMORY common model {action}: path={path} geometry_bytes={geometry_bytes} signature={:08x}", crc.sum());
}

// Host equivalent of 140060180: retain model read identities for the level
// that owns the prepared meshes, including pending preparation.
static HISTORY_SCOPES: crate::loading::CatalogHistoryScopes = crate::loading::CatalogHistoryScopes::new();
pub(crate) struct ModelHistoryLease(Arc<super::archive::Catalog>);
impl ModelHistoryLease {
    pub(crate) fn acquire(catalog: &Arc<super::archive::Catalog>) -> Arc<Self> {
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let start = trace.then(crate::loading::thread_cycles).flatten();
        HISTORY_SCOPES.acquire(catalog.identity());
        let lease = Arc::new(Self(catalog.clone()));
        if trace {
            let cycles = start.zip(crate::loading::thread_cycles()).map_or(0, |(a,b)| b.saturating_sub(a));
            eprintln!("MEMORY model history scope acquisition: cycles={cycles}");
        }
        lease
    }
}
impl Drop for ModelHistoryLease {
    fn drop(&mut self) {
        let catalog = self.0.identity();
        HISTORY_SCOPES.release(catalog, || {
            let trace = std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1");
            let start = trace.then(crate::loading::thread_cycles).flatten();
            let common = self.0.common_models.get().and_then(|models| models.as_ref().ok())
                .map(|models| models.models.lock().unwrap_or_else(|error| error.into_inner()));
            let (count, bytes, slots, _) = crate::loading::retain_catalog_kind(catalog, "model", |path| {
                common.as_ref().is_some_and(|models| models.contains_key(path))
            });
            if trace {
                let cycles = start.zip(crate::loading::thread_cycles()).map_or(0, |(a,b)| b.saturating_sub(a));
                let retained = common.as_ref().map_or(0, |models| models.len());
                eprintln!("MEMORY model history release: entries={count} path_capacity_bytes={bytes} slots={slots} cycles={cycles} catalog={catalog} retained={retained}");
            }
        });
    }
}

#[derive(Debug, PartialEq)]
pub struct Model {
    pub surfaces: Vec<Surface>,
    /// FUN_140269ff0 unions the authored BTRI surface bounds.
    pub authored_bounds: [[f32; 3]; 2],
    pub material_bindings: Vec<String>,
    /// Bytes left after the material-binding table, not silently interpreted.
    pub bytes_after_bindings: usize,
}

#[derive(Debug, PartialEq)]
pub struct Surface {
    pub material_path: String,
    /// BML1 surface separator/marker preserved from the source record.
    pub surface_marker: u32,
    /// FUN_14025a040 reads six finite floats after the index stream.
    pub authored_bounds: [[f32; 3]; 2],
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[half::f16; 2]>,
    /// Declaration 1: UV1 half2, normal/tangent SNORM8x4, color UNORM8x4.
    pub packed_channels: Vec<[u8; 16]>,
    pub indices: Vec<u16>,
}

fn region(bytes: &[u8], at: usize, size: usize) -> Result<&[u8], String> {
    bytes
        .get(at..at.checked_add(size).ok_or("offset overflow")?)
        .ok_or_else(|| format!("model section at {at} exceeds {} bytes", bytes.len()))
}

fn float_at(bytes: &[u8], at: usize) -> Result<f32, String> {
    let value = f32::from_le_bytes(region(bytes, at, 4)?.try_into().unwrap());
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("non-finite model coordinate at {at}"))
    }
}

enum ModelSourceBytes<'a> {
    Slice(&'a [u8]),
    Stream(&'a mut dyn Read, &'a mut [u8; READ_BLOCK_SIZE]),
}

struct ModelInput<'a> {
    source: ModelSourceBytes<'a>,
    length: usize,
    at: usize,
}

impl ModelInput<'_> {
    fn section_end(&self, size: usize) -> Result<usize, String> {
        let end = self.at.checked_add(size).ok_or("offset overflow")?;
        if end > self.length {
            return Err(format!("model section at {} exceeds {} bytes", self.at, self.length));
        }
        Ok(end)
    }

    fn read(&mut self, size: usize) -> Result<&[u8], String> {
        let end = self.section_end(size)?;
        let bytes = match &mut self.source {
            ModelSourceBytes::Slice(bytes) => &bytes[self.at..end],
            ModelSourceBytes::Stream(reader, scratch) => {
                let bytes = &mut scratch[..size];
                reader.read_exact(bytes).map_err(|error| error.to_string())?;
                bytes
            }
        };
        self.at = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.read(4)?.try_into().unwrap()))
    }

    fn string(&mut self, length: usize) -> Result<String, String> {
        std::str::from_utf8(self.read(length)?)
            .map(str::to_owned)
            .map_err(|error| error.to_string())
    }
}

/// FUN_140269ff0: union authored, non-null surface bounds into a resource.
/// Empty lists retain prior bounds. Null geometry/shader recovery belongs to
/// the owning loader and is not represented by this typed helper.
pub fn accumulate_resource_bounds(
    previous: [[f32; 3]; 2],
    surfaces: &[[[f32; 3]; 2]],
) -> [[f32; 3]; 2] {
    if surfaces.is_empty() {
        return previous;
    }
    let initial = f32::from_bits(0x7149_f2ca);
    let mut result = [[initial; 3], [-initial; 3]];
    for surface in surfaces {
        for axis in 0..3 {
            if surface[0][axis] <= result[0][axis] {
                result[0][axis] = surface[0][axis];
            }
            if result[1][axis] <= surface[1][axis] {
                result[1][axis] = surface[1][axis];
            }
        }
    }
    if result[1][0] <= result[0][0] && result[0][0] != result[1][0] {
        [[0.0; 3]; 2]
    } else {
        result
    }
}

pub fn parse(bytes: &[u8]) -> Result<Model, String> {
    read_model(&mut ModelInput {
        source: ModelSourceBytes::Slice(bytes),
        length: bytes.len(),
        at: 0,
    })
}

fn read_model(input: &mut ModelInput<'_>) -> Result<Model, String> {
    if input.read(4)? != b"1LMB" {
        return Err("not a BML1 model".into());
    }
    input.read(4)?;
    let source_length = input.u32()? as usize;
    if source_length > 4096 {
        return Err("unreasonable BML1 source path length".into());
    }
    input.read(source_length)?;
    let surface_count = input.u32()? as usize;
    if surface_count == 0 || surface_count > 128 {
        return Err("unreasonable BML1 surface count".into());
    }
    let mut surfaces = Vec::with_capacity(surface_count);
    for surface_number in 0..surface_count {
        let length = input.u32()? as usize;
        if length == 0 || length > 4096 {
            return Err(format!("surface {surface_number} material length invalid"));
        }
        let material_path = input.string(length)?;
        let surface_marker = input.u32()?;
        // Ordinary surfaces use 0xffffffff. Area volume proxies use zero.
        if surface_marker != u32::MAX && surface_marker != 0 {
            return Err(format!(
                "surface {surface_number} has unsupported marker {surface_marker:#x}"
            ));
        }
        if input.read(4)? != b"BTRI" {
            return Err(format!("surface {surface_number} missing BTRI"));
        }
        if input.u32()? != 0x0001_0002 {
            return Err(format!(
                "surface {surface_number} has unsupported BTRI version"
            ));
        }
        let cpu_declaration = input.u32()?;
        let gpu_declaration = input.u32()?;
        input.read(0x28)?;
        let vertex_count = input.u32()? as usize;
        if cpu_declaration != 1 || gpu_declaration != 1 || input.read(8)? != [0; 8] {
            return Err(format!("surface {surface_number} unsupported vertex declarations {cpu_declaration}/{gpu_declaration} or GPU stream"));
        }
        if vertex_count == 0 || vertex_count > u16::MAX as usize {
            return Err(format!("surface {surface_number} vertex count invalid"));
        }
        input.section_end(vertex_count * 32)?;
        let mut positions = Vec::with_capacity(vertex_count);
        let mut uvs = Vec::with_capacity(vertex_count);
        let mut packed_channels = Vec::with_capacity(vertex_count);
        for first in (0..vertex_count).step_by(READ_BLOCK_SIZE / 32) {
            let count = (vertex_count - first).min(READ_BLOCK_SIZE / 32);
            for vertex in input.read(count * 32)?.chunks_exact(32) {
                let position = [
                    float_at(vertex, 0)?,
                    float_at(vertex, 4)?,
                    float_at(vertex, 8)?,
                ];
                let u = half::f16::from_le_bytes(vertex[12..14].try_into().unwrap());
                let v = half::f16::from_le_bytes(vertex[14..16].try_into().unwrap());
                if !u.is_finite() || !v.is_finite() {
                    return Err(format!("surface {surface_number} has invalid UVs"));
                }
                positions.push(position);
                uvs.push([u, v]);
                packed_channels.push(vertex[16..32].try_into().unwrap());
            }
        }
        let index_count = input.u32()? as usize;
        if index_count == 0 || index_count % 3 != 0 || index_count > 3_000_000 {
            return Err(format!("surface {surface_number} index count invalid"));
        }
        input.section_end(index_count * 2)?;
        let mut indices = Vec::with_capacity(index_count);
        for first in (0..index_count).step_by(READ_BLOCK_SIZE / 2) {
            let count = (index_count - first).min(READ_BLOCK_SIZE / 2);
            for pair in input.read(count * 2)?.chunks_exact(2) {
                let index = u16::from_le_bytes(pair.try_into().unwrap());
                if index as usize >= vertex_count {
                    return Err(format!(
                        "surface {surface_number} index {index} exceeds {vertex_count} vertices"
                    ));
                }
                indices.push(index);
            }
        }
        // FUN_14025a040: authored minimum/maximum then the seventh field.
        let bounds = input.read(28)?;
        let authored_bounds = [
            [
                float_at(bounds, 0)?,
                float_at(bounds, 4)?,
                float_at(bounds, 8)?,
            ],
            [
                float_at(bounds, 12)?,
                float_at(bounds, 16)?,
                float_at(bounds, 20)?,
            ],
        ];
        float_at(bounds, 24)?;
        surfaces.push(Surface {
            material_path,
            surface_marker,
            authored_bounds,
            positions,
            uvs,
            packed_channels,
            indices,
        });
        if input.read(4)? != b"BML1" {
            return Err(format!(
                "surface {surface_number} BML1 section boundary missing"
            ));
        }
    }
    let binding_count = input.u32()? as usize;
    if binding_count > 128 {
        return Err("unreasonable BML1 material-binding count".into());
    }
    let mut material_bindings = Vec::with_capacity(binding_count);
    for _ in 0..binding_count {
        let length = input.u32()? as usize;
        if length > 4096 {
            return Err("unreasonable BML1 material-binding path length".into());
        }
        material_bindings.push(input.string(length)?);
        input.read(12)?;
    }
    let surface_bounds: Vec<_> = surfaces
        .iter()
        .map(|surface| surface.authored_bounds)
        .collect();
    let authored_bounds = accumulate_resource_bounds([[0.0; 3]; 2], &surface_bounds);
    Ok(Model {
        surfaces,
        authored_bounds,
        material_bindings,
        bytes_after_bindings: input.length - input.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_authored_bounds_instead_of_vertex_extrema() {
        let mut bytes = Vec::new();
        let u32_bytes = |bytes: &mut Vec<u8>, value: u32| bytes.extend(value.to_le_bytes());
        bytes.extend(b"1LMB");
        u32_bytes(&mut bytes, 0);
        u32_bytes(&mut bytes, 3);
        bytes.extend(b"x.y");
        u32_bytes(&mut bytes, 1);
        u32_bytes(&mut bytes, 1);
        bytes.extend(b"m");
        u32_bytes(&mut bytes, u32::MAX);
        let mut header = [0u8; 0x44];
        header[..4].copy_from_slice(b"BTRI");
        header[4..8].copy_from_slice(&0x0001_0002u32.to_le_bytes());
        header[8..12].copy_from_slice(&1u32.to_le_bytes());
        header[12..16].copy_from_slice(&1u32.to_le_bytes());
        header[0x38..0x3c].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend(header);
        for point in [[0.0f32; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in point {
                bytes.extend(value.to_le_bytes());
            }
            bytes.extend([0; 20]);
        }
        u32_bytes(&mut bytes, 3);
        for index in [0u16, 1, 2] {
            bytes.extend(index.to_le_bytes());
        }
        let bounds = [[-10.0f32, -20.0, -30.0], [40.0, 50.0, 60.0]];
        for side in bounds {
            for value in side {
                bytes.extend(value.to_le_bytes());
            }
        }
        bytes.extend(7.0f32.to_le_bytes());
        bytes.extend(b"BML1");
        u32_bytes(&mut bytes, 0);
        let model = parse(&bytes).unwrap();
        assert_eq!(model.surfaces[0].authored_bounds, bounds);
        assert_eq!(model.authored_bounds, bounds);
    }

    #[test]
    fn rejects_truncated_model() {
        assert!(parse(b"BML1").is_err());
    }
}
