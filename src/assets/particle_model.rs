//! 232220's PMODEL binary cache. These ordered emission vertices are distinct
//! from ordinary rendered BMODEL surfaces; the cache already arranges quads.

#[derive(Clone, Copy, Debug)]
pub struct ParticleModelVertex {
    pub position: [f32; 3],
    /// Original UV0, UV1, packed normal/tangent and RGBA8 bytes, in that order.
    /// Samplers consume the signed normal at offset20 and colour at offset28.
    pub attributes: [u8; 20],
}

// Host equivalent of 140060180: draw publications can retain these models
// beyond their declaration owner.
static HISTORY_SCOPES: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
pub(crate) struct HistoryLease(());
impl HistoryLease {
    pub(crate) fn acquire() -> std::sync::Arc<Self> {
        *HISTORY_SCOPES.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        std::sync::Arc::new(Self(()))
    }
}
impl Drop for HistoryLease {
    fn drop(&mut self) {
        let mut scopes = HISTORY_SCOPES.lock().unwrap_or_else(|e| e.into_inner());
        *scopes -= 1;
        if *scopes != 0 { return; }
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let start = trace.then(crate::loading::thread_cycles).flatten();
        let (count, bytes, slots, _) = crate::loading::retain_kind("staticParticleModel", |_| false);
        if trace {
            let cycles = start.zip(crate::loading::thread_cycles()).map_or(0, |(a,b)| b.saturating_sub(a));
            eprintln!("MEMORY particle model history release: entries={count} path_capacity_bytes={bytes} slots={slots} cycles={cycles}");
        }
    }
}

pub struct ParticleModel {
    pub(crate) history: Option<std::sync::Arc<HistoryLease>>,
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

pub fn parse(bytes: &[u8]) -> Result<ParticleModel, String> {
    let header = bytes
        .get(..12)
        .ok_or("particle model header is truncated")?;
    if header[..4] != *b"PML\x08" {
        return Err("bad PMODEL version".into());
    }
    if header[4..8] != *b"BML1" {
        return Err("bad BMODEL version in PMODEL".into());
    }
    let count = i32::from_le_bytes(header[8..12].try_into().unwrap());
    let count = usize::try_from(count).map_err(|_| "negative particle model vertex count")?;
    let size = count
        .checked_mul(32)
        .ok_or("particle model vertex size overflow")?;
    let end = 12usize
        .checked_add(size)
        .ok_or("particle model vertex offset overflow")?;
    let vertices = bytes
        .get(12..end)
        .ok_or("particle model vertices are truncated")?
        .chunks_exact(32)
        .map(|vertex| ParticleModelVertex {
            position: std::array::from_fn(|axis| {
                f32::from_le_bytes(vertex[axis * 4..axis * 4 + 4].try_into().unwrap())
            }),
            attributes: vertex[12..32].try_into().unwrap(),
        })
        .collect();
    Ok(ParticleModel {
        history: None,
        vertices,
        bytes_after_vertices: bytes.len() - end,
    })
}

/// The archive's staticParticleModel class resolves an authored LWO source to
/// its dedicated cache. Never substitute the ordinary mesh resource here.
pub fn resource_path(source: &str) -> Result<String, String> {
    let stem = source
        .strip_suffix(".lwo")
        .ok_or_else(|| format!("particle model source is not LWO: {source}"))?;
    Ok(format!("generated/staticparticlemodel/{stem}.pmodel"))
}
