//! Structural reader for compiled MD6 models (`MM6B`). Retains original
//! bind positions, UINT8 joints, UNORM8 weights and per-surface matrix offsets.
//! Rest skeleton evaluation lives in edge_skeleton; animation is separate.
//! Field order follows the installed game's idMD6Model loader and BTRI v2
//! reader (Ghidra functions 0x14025aa00, 0x140259960, 0x14025a040).

// Host equivalent of the resource registry's scoped purge (140060180).
static HISTORY_SCOPES: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
pub(crate) struct HistoryLease;
impl HistoryLease {
    pub(crate) fn acquire() -> std::sync::Arc<Self> {
        *HISTORY_SCOPES.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        std::sync::Arc::new(Self)
    }
}
impl Drop for HistoryLease {
    fn drop(&mut self) {
        let mut scopes = HISTORY_SCOPES.lock().unwrap_or_else(|e| e.into_inner());
        *scopes -= 1;
        if *scopes != 0 { return; }
        for kind in ["baseModel", "md6Def"] {
            let (count, bytes, slots, _) = crate::loading::retain_kind(kind, |_| false);
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY {kind} history release: entries={count} path_capacity_bytes={bytes} slots={slots}");
            }
        }
    }
}

#[derive(Debug)]
pub struct ModelInfo {
    pub surfaces: Vec<SurfaceInfo>,
    pub bytes_after_surfaces: usize,
    pub has_skeleton: bool,
    pub skeleton_name: String,
    pub skeleton_reference: Option<u16>,
    pub has_decoded_geometry: bool,
    pub authored_bounds: [[f32; 3]; 2],
}

#[derive(Debug)]
pub struct SurfaceInfo {
    pub name: String,
    pub material_path: String,
    pub cpu_vertex_declaration: u32,
    pub gpu_vertex_declaration: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub attachment_count: u32,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// UV1 half2, normal/tangent SNORM8x4; declarations 9/10 have no COLOR0.
    pub packed_channels: Vec<[u8; 12]>,
    pub skin_influences: u8,
    /// Retain original UNORM8 weights and UINT8 indices; no skinning guess.
    pub skin_weights: Vec<[u8; 8]>,
    pub skin_indices: Vec<[u8; 8]>,
    pub matrix_offset: u32,
    pub indices: Vec<u16>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// Resolve the LOD0 mesh named by a Void `.md6` declaration. The entity's
/// declaration name and compiled mesh name are not always the same.
pub fn compiled_mesh_path(declaration: &str) -> Option<String> {
    let lod = declaration
        .split_once("lod 0")?
        .1
        .split_once('{')?
        .1
        .split_once('}')?
        .0;
    let after = lod.split_once("mesh \"")?.1;
    let path = after.split_once('"')?.0;
    let stem = path.strip_suffix(".md6mesh")?;
    Some(format!("generated/basemodel/{stem}.bmd6model"))
}

pub fn resolve_compiled_mesh(
    compiled_guess: &str,
    mut read_declaration: impl FnMut(&str) -> Option<String>,
) -> Result<String, String> {
    let mut declaration =
        declaration_path(compiled_guess).ok_or("not a compiled MD6 model path")?;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..16 {
        if !visited.insert(declaration.clone()) {
            return Err("MD6 inheritance cycle".into());
        }
        let Some(text) = read_declaration(&declaration) else {
            return Ok(compiled_guess.into());
        };
        if let Some(mesh) = compiled_mesh_path(&text) {
            return Ok(mesh);
        }
        let Some(inherited) = text
            .split_once("inherit \"")
            .and_then(|(_, rest)| rest.split_once('"').map(|(path, _)| path))
        else {
            return Ok(compiled_guess.into());
        };
        let stem = inherited
            .strip_suffix(".md6")
            .ok_or("invalid MD6 inherit path")?;
        declaration = format!("generated/decls/md6def/{stem}.md6.decl");
    }
    Err("MD6 inheritance exceeds 16 declarations".into())
}

pub fn declaration_path(compiled_guess: &str) -> Option<String> {
    let stem = compiled_guess
        .strip_prefix("generated/basemodel/")?
        .strip_suffix(".bmd6model")?;
    Some(format!("generated/decls/md6def/{stem}.md6.decl"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkeletonBinding {
    pub name: String,
    pub offset: Option<[f32; 3]>,
    /// init `ragdoll`: the physics-system resource (arkPhysicsSystemResource).
    pub ragdoll: Option<String>,
}

/// Resolve independently inherited MD6 init skeleton/offset. A child can
/// replace its LOD0 mesh while continuing to use the parent's skeleton.
pub fn resolve_skeleton_binding(
    compiled_guess: &str,
    mut read: impl FnMut(&str) -> Option<String>,
) -> Result<Option<SkeletonBinding>, String> {
    let mut path = declaration_path(compiled_guess).ok_or("not a compiled MD6 model path")?;
    let mut visited = std::collections::HashSet::new();
    let mut skeleton = None;
    let mut offset = None;
    let mut ragdoll = None;
    for _ in 0..16 {
        if !visited.insert(path.clone()) {
            return Err("MD6 inheritance cycle".into());
        }
        let Some(source) = read(&path) else {
            break;
        };
        let tokens = crate::entities::declaration_lexemes(&source)?;
        let mut depth = 0usize;
        let mut init = None;
        let mut parent = None;
        for (i, &token) in tokens.iter().enumerate() {
            if token == "{" {
                depth += 1;
                continue;
            }
            if token == "}" {
                if init == Some(depth) {
                    init = None;
                }
                depth = depth.checked_sub(1).ok_or("unbalanced MD6 declaration")?;
                continue;
            }
            if depth == 1 && token == "init" && tokens.get(i + 1) == Some(&"{") {
                init = Some(depth + 1);
            }
            if depth == 1 && token == "inherit" {
                parent = tokens.get(i + 1).copied();
            }
            if init != Some(depth) {
                continue;
            }
            if token == "skeleton" && skeleton.is_none() {
                skeleton = Some(
                    tokens
                        .get(i + 1)
                        .ok_or("missing MD6 skeleton name")?
                        .to_string(),
                );
            }
            if token == "ragdoll" && ragdoll.is_none() {
                ragdoll = tokens.get(i + 1).map(|name| name.to_string());
            }
            if token == "offset" && offset.is_none() {
                let mut values = Vec::new();
                let mut closed = false;
                for text in tokens.iter().skip(i + 1) {
                    for number in text.split(['(', ')', ',']).filter(|part| !part.is_empty()) {
                        values.push(
                            number
                                .parse::<f32>()
                                .map_err(|_| "invalid MD6 init offset")?,
                        );
                    }
                    if text.contains(')') {
                        closed = true;
                        break;
                    }
                }
                if !closed || values.len() != 3 || values.iter().any(|value| !value.is_finite()) {
                    return Err("invalid MD6 init offset".into());
                }
                offset = Some(values.try_into().unwrap());
            }
        }
        if depth != 0 {
            return Err("unbalanced MD6 declaration".into());
        }
        if skeleton.is_some() && offset.is_some() && ragdoll.is_some() {
            break;
        }
        let Some(parent) = parent else {
            break;
        };
        let stem = parent
            .strip_suffix(".md6")
            .ok_or("invalid MD6 inherit path")?;
        path = format!("generated/decls/md6def/{stem}.md6.decl");
    }
    Ok(skeleton.map(|name| SkeletonBinding { name, offset, ragdoll }))
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(count).ok_or("MD6 offset overflow")?;
        let result = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| format!("MD6 data truncated at {} (need {count} bytes)", self.at))?;
        self.at = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        if length > 4096 {
            return Err(format!("MD6 string too long: {length}"));
        }
        let raw = self.take(length)?;
        Ok(std::str::from_utf8(raw)
            .map_err(|error| error.to_string())?
            .trim_end_matches('\0')
            .to_string())
    }
}

pub fn inspect(bytes: &[u8]) -> Result<ModelInfo, String> {
    let mut input = Reader { bytes, at: 0 };
    if input.take(4)? != b"MM6B" {
        return Err("not an MM6B model".into());
    }
    let source_name = input.string()?;
    // The original loader reads a 16-bit skeleton reference when a source
    // name is present; models without one proceed directly to bounds.
    let skeleton_reference = if !source_name.is_empty() {
        Some(u16::from_le_bytes(input.take(2)?.try_into().unwrap()))
    } else {
        None
    };
    let bounds_raw = input.take(24)?;
    let authored_bounds = [
        std::array::from_fn(|axis| {
            f32::from_le_bytes(bounds_raw[axis * 4..axis * 4 + 4].try_into().unwrap())
        }),
        std::array::from_fn(|axis| {
            f32::from_le_bytes(bounds_raw[12 + axis * 4..16 + axis * 4].try_into().unwrap())
        }),
    ];
    let surface_count = input.u32()? as usize;
    if surface_count > 256 {
        return Err(format!("invalid MD6 surface count: {surface_count}"));
    }
    let mut surfaces = Vec::with_capacity(surface_count);
    for number in 0..surface_count {
        let name = input.string()?;
        let material_path = input.string()?;
        input.take(4)?; // per-surface field at +0x30
        input.u8()?; // per-surface flag at +0x34
        input.take(4)?; // per-surface field at +0x38
        if input.take(4)? != b"BTRI" {
            return Err(format!("surface {number} missing BTRI"));
        }
        let version = input.u32()?;
        if version <= 0x0001_0001 {
            return Err(format!(
                "surface {number} uses unsupported BTRI version {version:#x}"
            ));
        }
        let cpu_vertex_declaration = input.u32()?;
        let gpu_vertex_declaration = input.u32()?;
        input.take(40)?; // BTRI v2 bounds and flags
        let vertex_count = input.u32()?;
        let gpu_buffer_size = input.u64()?;
        if vertex_count > 1_000_000 {
            return Err(format!("surface {number} vertex count too large"));
        }
        if gpu_buffer_size != 0 {
            return Err(format!(
                "surface {number} GPU stream layout not yet decoded"
            ));
        }
        // Byte counts are selected by CPU vertex declaration, as in the game
        // loader. These strides are validated against the following index and
        // per-surface records across the installed asset catalog.
        let stride = match cpu_vertex_declaration {
            9 => 36usize,
            10 => 44usize,
            other => {
                let candidates: Vec<_> = (24usize..=96)
                    .step_by(4)
                    .filter_map(|stride| {
                        let at = input
                            .at
                            .checked_add((vertex_count as usize).checked_mul(stride)?)?;
                        let raw = input.bytes.get(at..at + 4)?;
                        let count = u32::from_le_bytes(raw.try_into().ok()?);
                        (count <= 3_000_000 && count % 3 == 0).then_some((stride, count))
                    })
                    .collect();
                return Err(format!("surface {number} CPU vertex declaration {other} not yet decoded; index-count candidates={candidates:?}"));
            }
        };
        let vertex_bytes = input.take(
            (vertex_count as usize)
                .checked_mul(stride)
                .ok_or("MD6 vertex size overflow")?,
        )?;
        // The first CPU stream is a tightly packed XYZ position array. The
        // remaining declaration-specific attributes follow it in this file.
        let primary_stride = 12;
        let primary_len = vertex_count as usize * primary_stride;
        let (primary, secondary) = vertex_bytes.split_at(primary_len);
        let texcoord_stream = secondary
            .get(..(vertex_count as usize * 8))
            .ok_or("MD6 texture-coordinate stream truncated")?;
        let mut positions = Vec::with_capacity(vertex_count as usize);
        let mut uvs = Vec::with_capacity(vertex_count as usize);
        let mut packed_channels = Vec::with_capacity(vertex_count as usize);
        let mut skin_weights = Vec::with_capacity(vertex_count as usize);
        let mut skin_indices = Vec::with_capacity(vertex_count as usize);
        let normal_stream = &vertex_bytes[vertex_count as usize * 20..vertex_count as usize * 24];
        let tangent_stream = &vertex_bytes[vertex_count as usize * 24..vertex_count as usize * 28];
        let skin_stream = &vertex_bytes[vertex_count as usize * 28..];
        let skin_influences = if cpu_vertex_declaration == 9 { 4 } else { 8 };
        let mut geometry_valid = true;
        for (vertex_index, vertex) in primary.chunks_exact(primary_stride).enumerate() {
            let point = std::array::from_fn(|axis| {
                f32::from_le_bytes(vertex[axis * 4..axis * 4 + 4].try_into().unwrap())
            });
            if !point
                .iter()
                .all(|value: &f32| value.is_finite() && value.abs() < 100_000.0)
            {
                geometry_valid = false;
            }
            if geometry_valid {
                positions.push(point);
            }
            // Live declarations 9/10: streams 0=position,1=UV0/UV1,
            // 2=normal,3=tangent,4=weights/indices (4 or 8 influences).
            let uv = &texcoord_stream[vertex_index * 8..vertex_index * 8 + 8];
            let u = half::f16::from_bits(u16::from_le_bytes(uv[0..2].try_into().unwrap())).to_f32();
            let v = half::f16::from_bits(u16::from_le_bytes(uv[2..4].try_into().unwrap())).to_f32();
            if !u.is_finite() || !v.is_finite() {
                return Err(format!("surface {number} has non-finite UV0"));
            }
            if geometry_valid {
                uvs.push([u, v]);
                let mut packed = [0; 12];
                packed[..4].copy_from_slice(&uv[4..8]);
                packed[4..8]
                    .copy_from_slice(&normal_stream[vertex_index * 4..vertex_index * 4 + 4]);
                packed[8..12]
                    .copy_from_slice(&tangent_stream[vertex_index * 4..vertex_index * 4 + 4]);
                packed_channels.push(packed);
                let width = skin_influences as usize;
                let skin = &skin_stream[vertex_index * width * 2..(vertex_index + 1) * width * 2];
                let mut weights = [0; 8];
                let mut joints = [0; 8];
                weights[..width].copy_from_slice(&skin[..width]);
                joints[..width].copy_from_slice(&skin[width..]);
                skin_weights.push(weights);
                skin_indices.push(joints);
            }
        }
        if !geometry_valid {
            positions.clear();
            uvs.clear();
            packed_channels.clear();
            skin_weights.clear();
            skin_indices.clear();
        }
        let index_count = input.u32()?;
        if index_count > 3_000_000 {
            return Err(format!("surface {number} index count too large"));
        }
        let indices = input.take(
            (index_count as usize)
                .checked_mul(2)
                .ok_or("MD6 index size overflow")?,
        )?;
        for pair in indices.chunks_exact(2) {
            if u16::from_le_bytes(pair.try_into().unwrap()) as u32 >= vertex_count {
                return Err(format!("surface {number} index exceeds vertex count"));
            }
        }
        let indices = indices
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes(pair.try_into().unwrap()))
            .collect();
        input.take(28)?; // six bounds floats and one extent field
        let attachment_count = input.u32()?;
        if attachment_count > 100_000 {
            return Err(format!("surface {number} attachment count too large"));
        }
        input.take(
            (attachment_count as usize)
                .checked_mul(12)
                .ok_or("MD6 attachment size overflow")?,
        )?;
        let matrix_offset = input.u32()?; // +5c, published as cb_mtxOffset by 25e4d0.
        input.take(44)?;
        surfaces.push(SurfaceInfo {
            name,
            material_path,
            cpu_vertex_declaration,
            gpu_vertex_declaration,
            vertex_count,
            index_count,
            attachment_count,
            positions,
            uvs,
            packed_channels,
            skin_influences,
            skin_weights,
            skin_indices,
            matrix_offset,
            indices,
        });
    }
    let margin = (0..3)
        .map(|axis| authored_bounds[1][axis] - authored_bounds[0][axis])
        .fold(0.0f32, f32::max)
        .max(1.0)
        * 0.02;
    let has_decoded_geometry = authored_bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        && surfaces.iter().all(|surface| {
            surface.vertex_count == 0
                || (!surface.positions.is_empty()
                    && surface.positions.iter().all(|point| {
                        (0..3).all(|axis| {
                            point[axis] >= authored_bounds[0][axis] - margin
                                && point[axis] <= authored_bounds[1][axis] + margin
                        })
                    }))
        });
    Ok(ModelInfo {
        surfaces,
        bytes_after_surfaces: bytes.len() - input.at,
        has_skeleton: !source_name.is_empty(),
        skeleton_name: source_name,
        skeleton_reference,
        has_decoded_geometry,
        authored_bounds,
    })
}

/// Shared immutable decoded models. 1400607f0 resolves one registered resource
/// identity and 14025aa00 loads its BMD6 contents; instances keep their own
/// poses. The host registry holds only weak model identities and coalesces
/// concurrent worker acquisitions of the same immutable catalog resource.
mod residency {
    use super::ModelInfo;
    use std::{collections::HashMap, sync::{Arc, Condvar, Mutex, OnceLock, Weak}, time::Instant};

    enum State {
        Empty,
        Loading { waiters: usize },
        Ready { weak: Weak<ModelInfo>, handoff: Option<Arc<ModelInfo>>, pending: usize },
        Failed { error: String, pending: usize },
    }
    struct Entry { state: Mutex<State>, ready: Condvar }
    type Key = (u64, String);
    struct Registry { entries: HashMap<Key, Arc<Entry>>, requests: usize }
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    static TRACE: OnceLock<bool> = OnceLock::new();
    static SEPARATE: OnceLock<bool> = OnceLock::new();
    pub(super) fn tracing() -> bool {
        *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_MODEL_ACQUISITION").is_some_and(|v| v == "1"))
    }
    pub(super) fn separate() -> bool {
        *SEPARATE.get_or_init(|| std::env::var_os("DH2_DEBUG_SEPARATE_MODEL_RESOURCES").is_some_and(|v| v == "1"))
    }

    fn retain_live(entry: &Arc<Entry>) -> bool {
        Arc::strong_count(entry) > 1 || match entry.state.try_lock() {
            Ok(state) => match &*state { State::Loading { .. } => true, State::Ready { weak, .. } => weak.strong_count() != 0, _ => false },
            Err(_) => true,
        }
    }

    pub(super) fn release_unused() {
        if let Some(registry) = REGISTRY.get() {
            registry.lock().unwrap_or_else(|e| e.into_inner()).entries.retain(|_, entry| retain_live(entry));
        }
    }

    pub(super) fn acquire(catalog: &crate::archive::Catalog, path: &str, normalized: &str) -> Result<Arc<ModelInfo>, String> {
        let trace = tracing();
        let started = trace.then(Instant::now);
        let entry = {
            let mut registry = REGISTRY.get_or_init(|| Mutex::new(Registry { entries: HashMap::new(), requests: 0 }))
                .lock().unwrap_or_else(|e| e.into_inner());
            registry.requests = registry.requests.wrapping_add(1);
            if registry.entries.len() > 1024 && registry.requests % 256 == 0 {
                registry.entries.retain(|_, entry| retain_live(entry));
            }
            registry.entries.entry((catalog.identity(), normalized.to_owned()))
                .or_insert_with(|| Arc::new(Entry { state: Mutex::new(State::Empty), ready: Condvar::new() })).clone()
        };
        let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut waited = false;
        loop {
            match &mut *state {
                State::Ready { weak, handoff, pending } => if let Some(model) = weak.upgrade() {
                    if waited && *pending > 0 {
                        *pending -= 1;
                        if *pending == 0 { handoff.take(); }
                    }
                    if let Some(started) = started { eprintln!("MODEL_ACQUIRE path={normalized} result={} us={}", if waited { "wait" } else { "reuse" }, started.elapsed().as_micros()); }
                    return Ok(model);
                },
                State::Failed { error, pending } => {
                    let result = error.clone();
                    if waited && *pending > 0 { *pending -= 1; }
                    if *pending == 0 { *state = State::Empty; }
                    return Err(result);
                }
                State::Loading { waiters } => {
                    if !waited { *waiters += 1; }
                    waited = true;
                    state = entry.ready.wait(state).unwrap_or_else(|e| e.into_inner());
                    continue;
                }
                State::Empty => {}
            }
            *state = State::Loading { waiters: 0 };
            drop(state);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let bytes = catalog.read(path).map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("{path} is absent"))?;
                super::inspect(&bytes).map(Arc::new)
            })).unwrap_or_else(|_| Err(format!("model acquisition panicked for {path}")));
            let mut state = entry.state.lock().unwrap_or_else(|e| e.into_inner());
            let waiters = match &*state { State::Loading { waiters } => *waiters, _ => 0 };
            *state = match &result {
                Ok(model) => State::Ready { weak: Arc::downgrade(model), handoff: (waiters > 0).then(|| model.clone()), pending: waiters },
                Err(error) if waiters > 0 => State::Failed { error: error.clone(), pending: waiters },
                Err(_) => State::Empty,
            };
            entry.ready.notify_all();
            if let Some(started) = started { eprintln!("MODEL_ACQUIRE path={normalized} result=decode us={}", started.elapsed().as_micros()); }
            return result;
        }
    }
}

/// Discard expired model identity metadata after level consumers are dropped.
pub(crate) fn release_unused() { residency::release_unused(); }

/// Preparation-scoped references to shared immutable MD6 models.
pub(crate) struct ModelSources {
    catalog: u64,
    share: bool,
    models: std::collections::HashMap<String, std::sync::Arc<ModelInfo>>,
    paths: std::collections::HashMap<String, String>,
}
impl ModelSources {
    pub(crate) fn new(catalog: &crate::archive::Catalog) -> Self {
        Self { catalog: catalog.identity(), share: !catalog.has_overlays(),
            models: Default::default(), paths: Default::default() }
    }

    pub(crate) fn acquire(&mut self, catalog: &crate::archive::Catalog, resource: &str)
        -> Result<std::sync::Arc<ModelInfo>, String>
    {
        let started = residency::tracing().then(std::time::Instant::now);
        if self.catalog != catalog.identity() {
            self.models.clear();
            self.paths.clear();
            self.catalog = catalog.identity();
            self.share = !catalog.has_overlays();
        }
        if self.share {
            if let Some(model) = self.paths.get(resource).and_then(|path| self.models.get(path)) {
                if let Some(started) = started { eprintln!("MODEL_SOURCE path={resource} result=local us={}", started.elapsed().as_micros()); }
                return Ok(model.clone());
            }
        }
        let read = |decl: &str| {
            catalog.read(decl).ok().flatten().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        };
        let path = resolve_compiled_mesh(resource, read)?;
        // A validated alias resolves to the same catalog identity despite
        // differences in ASCII case or slash direction (140140da0/140140de0).
        let key = if path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
            std::borrow::Cow::Borrowed(path.as_str())
        } else {
            let mut normalized = path.replace('\\', "/");
            normalized.make_ascii_lowercase();
            std::borrow::Cow::Owned(normalized)
        };
        if self.share {
            if let Some(model) = self.models.get(key.as_ref()) {
                let model = model.clone();
                self.paths.insert(resource.to_owned(), key.into_owned());
                if let Some(started) = started { eprintln!("MODEL_SOURCE path={resource} result=alias us={}", started.elapsed().as_micros()); }
                return Ok(model);
            }
        }
        let model = if self.share && !residency::separate() {
            residency::acquire(catalog, &path, &key)?
        } else {
            let bytes = catalog.read(&path).map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{path} is absent"))?;
            std::sync::Arc::new(inspect(&bytes)?)
        };
        if self.share {
            let key = key.into_owned();
            self.models.insert(key.clone(), model.clone());
            self.paths.insert(resource.to_owned(), key);
        }
        if let Some(started) = started { eprintln!("MODEL_SOURCE path={resource} result={} us={}", if !self.share { "overlay" } else if residency::separate() { "separate" } else { "shared" }, started.elapsed().as_micros()); }
        Ok(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherits_skeleton_and_offset_independently_of_lod_mesh() {
        let binding=resolve_skeleton_binding("generated/basemodel/models/child.bmd6model",|path|match path {
            "generated/decls/md6def/models/child.md6.decl"=>Some("md6 { inherit \"models/parent.md6\" init { offset (1, -2, 3) } lod 0 { mesh \"models/new.md6mesh\" } }".into()),
            "generated/decls/md6def/models/parent.md6.decl"=>Some("md6 { init { skeleton \"models/body.md6skl\" offset (4, 5, 6) } }".into()),
            _=>None,
        }).unwrap().unwrap();
        assert_eq!(
            binding,
            SkeletonBinding {
                name: "models/body.md6skl".into(),
                offset: Some([1.0, -2.0, 3.0]),
                ragdoll: None,
            }
        );
        assert!(
            resolve_skeleton_binding("generated/basemodel/models/child.bmd6model", |_| Some(
                "md6 { inherit \"models/child.md6\" }".into()
            ))
            .is_err()
        );
    }
    #[test]
    fn rejects_truncated_or_wrong_signature() {
        assert!(inspect(b"MM6B").is_err());
        assert!(inspect(b"BML1").is_err());
    }
    #[test]
    fn accepts_authored_empty_model_container() {
        let mut bytes = b"MM6B".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let result = inspect(&bytes).unwrap();
        assert!(result.surfaces.is_empty());
        assert_eq!(result.bytes_after_surfaces, 0);
    }
    #[test]
    fn resolves_md6_declaration_mesh_name() {
        assert_eq!(
            compiled_mesh_path("lod 0 { mesh \"models/a/actual.md6mesh\" }"),
            Some("generated/basemodel/models/a/actual.bmd6model".into())
        );
        assert_eq!(
            declaration_path("generated/basemodel/models/a/alias.bmd6model"),
            Some("generated/decls/md6def/models/a/alias.md6.decl".into())
        );
    }
    #[test]
    fn resolves_inherited_lod_zero_mesh() {
        let path =
            resolve_compiled_mesh(
                "generated/basemodel/models/child.bmd6model",
                |decl| match decl {
                    "generated/decls/md6def/models/child.md6.decl" => {
                        Some("lod 0 { } inherit \"models/parent.md6\"".into())
                    }
                    "generated/decls/md6def/models/parent.md6.decl" => {
                        Some("lod 0 { mesh \"models/actual.md6mesh\" }".into())
                    }
                    _ => None,
                },
            )
            .unwrap();
        assert_eq!(path, "generated/basemodel/models/actual.bmd6model");
    }
    #[test]
    fn reads_planar_cpu_streams_for_both_vertex_declarations() {
        for (declaration, stride) in [(9u32, 36usize), (10, 44)] {
            let mut bytes = b"MM6B".to_vec();
            bytes.extend_from_slice(&0u32.to_le_bytes()); // no skeleton source
            for value in [0.0f32, 0.0, 0.0, 2.0, 1.0, 0.0] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&1u32.to_le_bytes()); // one surface
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.push(b'x');
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.push(b'm');
            bytes.extend_from_slice(&[0; 9]); // surface fields
            bytes.extend_from_slice(b"BTRI");
            bytes.extend_from_slice(&0x0001_0002u32.to_le_bytes());
            bytes.extend_from_slice(&declaration.to_le_bytes());
            bytes.extend_from_slice(&declaration.to_le_bytes());
            bytes.extend_from_slice(&[0; 40]);
            bytes.extend_from_slice(&3u32.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            for point in [[0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
                for value in point {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
                for value in uv {
                    bytes.extend_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
                }
                bytes.extend_from_slice(&[0; 4]);
            }
            bytes.extend_from_slice(&vec![0; (stride - 20) * 3]);
            bytes.extend_from_slice(&3u32.to_le_bytes());
            for index in [0u16, 1, 2] {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
            bytes.extend_from_slice(&[0; 28]);
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&17u32.to_le_bytes()); // surface matrix offset, in joints
            bytes.extend_from_slice(&[0; 44]);
            let model = inspect(&bytes).unwrap();
            assert!(model.has_decoded_geometry);
            assert_eq!(model.surfaces[0].positions[1], [2.0, 0.0, 0.0]);
            assert_eq!(model.surfaces[0].uvs[2], [0.0, 1.0]);
            assert_eq!(model.surfaces[0].indices, [0, 1, 2]);
            assert_eq!(model.surfaces[0].matrix_offset, 17);
        }
    }
    #[test]
    #[ignore = "audit; requires installed Dust District archives"]
    fn installed_dust_cable_placements_decode() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        for level in ["dust_p_lowchaos", "dust_p_highchaos"] {
            let resources = crate::assets::level::LevelResources::resolve(&catalog, level).unwrap();
            let bytes = resources.read_entities(&catalog).unwrap().unwrap();
            let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
            let mentions = document
                .definitions
                .iter()
                .filter(|d| format!("{:?}", d.fields).contains("cables_speaker"))
                .count();
            let placements = crate::entities::render_placements(&document);
            let cables: Vec<_> =
                placements.iter().filter(|p| p.resource_path.contains("cables_speaker")).collect();
            println!("{level}: {mentions} entities mention cables_speaker, {} placements", cables.len());
            if level == "dust_p_lowchaos" {
                for d in document.definitions.iter().filter(|d| d.id == "map_cpntphysicssystem_decl_6" || d.id == "map_cpntrigidbody_decl_62") {
                    println!("MAPDECL {} {:?}", d.id, d.fields);
                }
                for name in [] as [&str; 0] {
                    if let Some(d) = document.definitions.iter().find(|d| d.id == name) {
                        println!("ENTITY {name} class={} {:?}", d.class, d.fields);
                    }
                }
                for decl in catalog.paths_with_suffix(".decl") {
                    if decl.contains("physicssystem_dynamic") || decl.ends_with("/physicssystem.cpntphysicssystem.decl") || decl.contains("cpntphysicssystem/default") {
                        let text = catalog.read(&decl).unwrap().map(|b| String::from_utf8_lossy(&b).into_owned());
                        println!("DECLFILE {decl}: {}", text.unwrap_or_default().replace(char::is_whitespace, " "));
                    }
                }
                let decl = super::declaration_path(&cables[0].resource_path).unwrap();
                let text = catalog.read(&decl).unwrap().map(|b| String::from_utf8_lossy(&b).into_owned());
                println!("DECL {decl}: {text:?}");
                let hkx = "generated/physsystem/models/environment/gameplay/speaker_01/cables_speaker/cable_01_rgdl.hkx";
                let bytes = catalog.read(hkx).unwrap().unwrap();
                println!("HKX {} bytes, magic {:02x?}", bytes.len(), &bytes[..16]);
                for parse in [crate::physworld::parse, crate::physworld::parse_hkx, crate::physworld::parse_embedded_havok] {
                    match parse(&bytes) {
                        Ok(world) => {
                            println!("PARSED {:?}", world.item_inventory());
                            break;
                        }
                        Err(error) => println!("PARSE ERROR {error}"),
                    }
                }
            }
            for placement in cables.iter().take(3) {
                let mesh = super::resolve_compiled_mesh(&placement.resource_path, |decl| {
                    catalog.read(decl).ok().flatten().map(|b| String::from_utf8_lossy(&b).into_owned())
                });
                println!("  {} {} -> {mesh:?}", placement.id, placement.resource_path);
                if let Ok(path) = mesh {
                    let model = super::inspect(&catalog.read(&path).unwrap().unwrap());
                    println!("    {:?}", model.as_ref().map(|m| (m.has_decoded_geometry, m.has_skeleton, m.surfaces.len())));
                }
            }
        }
    }

}
