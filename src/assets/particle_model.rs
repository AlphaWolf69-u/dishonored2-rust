//! 232220's PMODEL binary cache. These ordered emission vertices are distinct
//! from ordinary rendered BMODEL surfaces; the cache already arranges quads.

#[derive(Clone, Copy, Debug)]
pub struct ParticleModelVertex {
    pub position: [f32; 3],
    /// Original UV0, UV1, packed normal/tangent and RGBA8 bytes, in that order.
    /// Samplers consume the signed normal at offset20 and colour at offset28.
    pub attributes: [u8; 20],
}

pub struct ParticleModel {
    pub vertices: Vec<ParticleModelVertex>,
    pub bytes_after_vertices: usize,
}

impl ParticleModelVertex {
    pub fn packed_normal(&self) -> [i8; 3] {
        std::array::from_fn(|axis| self.attributes[8 + axis] as i8)
    }
    pub fn colour(&self) -> [u8; 4] {
        self.attributes[16..20].try_into().unwrap()
    }
}

const STREAM_THRESHOLD: usize = 256 * 1024;
const READ_BLOCK_SIZE: usize = 8 * 1024;

fn layout(header: &[u8]) -> Result<(usize, usize), String> {
    let header = header.get(..12).ok_or("particle model header is truncated")?;
    if header[..4] != *b"PML\x08" { return Err("bad PMODEL version".into()); }
    if header[4..8] != *b"BML1" { return Err("bad BMODEL version in PMODEL".into()); }
    let count = i32::from_le_bytes(header[8..12].try_into().unwrap());
    let count = usize::try_from(count).map_err(|_| "negative particle model vertex count")?;
    let size = count.checked_mul(32).ok_or("particle model vertex size overflow")?;
    let end = 12usize.checked_add(size).ok_or("particle model vertex offset overflow")?;
    Ok((count, end))
}
fn vertex(bytes: &[u8]) -> ParticleModelVertex {
    ParticleModelVertex {
        position: std::array::from_fn(|axis|
            f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())),
        attributes: bytes[12..32].try_into().unwrap(),
    }
}
pub fn parse(bytes: &[u8]) -> Result<ParticleModel, String> {
    let (_, end) = layout(bytes)?;
    let vertices = bytes.get(12..end).ok_or("particle model vertices are truncated")?
        .chunks_exact(32).map(vertex).collect();
    Ok(ParticleModel { vertices, bytes_after_vertices: bytes.len() - end })
}

/// 140232220 fills final vertices from the resource stream. Host: the
/// bounded archive reader inflates blocks without a whole-member staging Vec.
pub(crate) fn load(catalog: &crate::archive::Catalog, path: &str,
    cancelled: Option<&dyn Fn() -> bool>) -> Result<Option<ParticleModel>, String> {
    let buffered = std::env::var_os("DH2_DEBUG_BUFFERED_PARTICLE_MODELS").is_some_and(|value| value == "1");
    if !buffered && !catalog.has_overlays()
        && catalog.resource_size(path).is_some_and(|size| size >= STREAM_THRESHOLD) {
        return catalog.with_stream_cancellable(path, cancelled, read_stream)
            .map_err(|error| format!("{path}: {error}"));
    }
    catalog.read_cancellable(path, cancelled).map_err(|error| error.to_string())?
        .map(|bytes| parse(&bytes).map_err(|error| format!("{path}: {error}"))).transpose()
}
fn read_stream(reader: &mut dyn std::io::Read, length: usize) -> Result<ParticleModel, String> {
    use std::io::Read;
    let mut header = [0; 12];
    reader.read_exact(&mut header).map_err(|error| error.to_string())?;
    let (count, end) = layout(&header)?;
    if end > length { return Err("particle model vertices are truncated".into()); }
    let mut vertices = Vec::with_capacity(count);
    let mut block = [0; READ_BLOCK_SIZE];
    while vertices.len() < count {
        let bytes = ((count - vertices.len()) * 32).min(block.len());
        reader.read_exact(&mut block[..bytes]).map_err(|error| error.to_string())?;
        vertices.extend(block[..bytes].chunks_exact(32).map(vertex));
    }
    let tail = length - end;
    if std::io::copy(&mut reader.take(tail as u64), &mut std::io::sink())
        .map_err(|error| error.to_string())? != tail as u64 {
        return Err("particle model tail is truncated".into());
    }
    Ok(ParticleModel { vertices, bytes_after_vertices: tail })
}

/// The archive's staticParticleModel class resolves an authored LWO source to
/// its dedicated cache. Never substitute the ordinary mesh resource here.
pub fn resource_path(source: &str) -> Result<String, String> {
    let stem = source
        .strip_suffix(".lwo")
        .ok_or_else(|| format!("particle model source is not LWO: {source}"))?;
    Ok(format!("generated/staticparticlemodel/{stem}.pmodel"))
}


/// Installed read/parse comparison. This runs no game frames or renderer.
pub fn observe_streams(base: &std::path::Path) -> Result<(), String> {
    let catalog = crate::archive::Catalog::open(base).map_err(|error| error.to_string())?;
    let mut paths = Vec::new();
    let mut total_vertices = 0;
    for path in catalog.paths_with_suffix(".pmodel") {
        if catalog.kind(&path) != Some("staticParticleModel") { continue; }
        let bytes = catalog.read(&path).map_err(|error| error.to_string())?.ok_or("installed particle model missing")?;
        let (count, end) = layout(&bytes)?;
        let expected = blake3::hash(bytes.get(12..end).ok_or("installed particle vertices truncated")?);
        let signature = |model: &ParticleModel| {
            let mut hash = blake3::Hasher::new();
            for vertex in &model.vertices {
                for value in vertex.position { hash.update(&value.to_bits().to_le_bytes()); }
                hash.update(&vertex.attributes);
            }
            hash.finalize()
        };
        let buffered = parse(&bytes)?;
        let streamed = catalog.with_stream(&path, read_stream)?.ok_or("streamed particle model missing")?;
        let selected = load(&catalog, &path, None)?.ok_or("production particle model missing")?;
        if [signature(&buffered), signature(&streamed), signature(&selected)] != [expected; 3]
            || [buffered.vertices.len(), streamed.vertices.len(), selected.vertices.len()] != [count; 3]
            || [buffered.bytes_after_vertices, streamed.bytes_after_vertices, selected.bytes_after_vertices]
                != [bytes.len() - end; 3] {
            return Err(format!("particle model differs from installed packed bytes: {path}"));
        }
        total_vertices += count;
        if bytes.len() >= STREAM_THRESHOLD { paths.push(path.clone()); }
        println!("PARTICLE_MODEL_CONTENT path={path} source_bytes={} vertices={count} signature={expected}", bytes.len());
    }
    let mut buffered = std::time::Duration::ZERO;
    let mut streamed = std::time::Duration::ZERO;
    let mut largest = 0;
    for round in 0..32 {
        for (index, path) in paths.iter().enumerate() {
            let read_buffered = || -> Result<(), String> {
                let bytes = catalog.read(path).map_err(|error| error.to_string())?.ok_or("particle model missing")?;
                std::hint::black_box(parse(&bytes)?);
                Ok(())
            };
            let read_streamed = || -> Result<(), String> {
                std::hint::black_box(catalog.with_stream(path, read_stream)?.ok_or("particle model stream missing")?);
                Ok(())
            };
            let observe = |read: &dyn Fn() -> Result<(), String>| -> Result<std::time::Duration, String> {
                let start = std::time::Instant::now();
                read()?;
                Ok(start.elapsed())
            };
            if (round + index) % 2 == 0 {
                buffered += observe(&read_buffered)?;
                streamed += observe(&read_streamed)?;
            } else {
                streamed += observe(&read_streamed)?;
                buffered += observe(&read_buffered)?;
            }
            largest = largest.max(catalog.resource_size(path).unwrap_or(0));
        }
    }
    if paths.is_empty() { return Err("no installed large particle models were observed".into()); }
    println!("PARTICLE_MODEL_STREAMS large_models={} samples_per_mode={} vertices={total_vertices} largest_removed_buffer={largest} stack_block={READ_BLOCK_SIZE} buffered_us={} streamed_us={}",
        paths.len(), paths.len() * 32, buffered.as_micros(), streamed.as_micros());
    Ok(())
}
