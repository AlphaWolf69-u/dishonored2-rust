//! Level acquisition of particle images. Image registry 1400607f0/14005feb0
//! shares a resource independently of its material and render state.
use crate::{archive::Catalog, texture::BimageInfo};
use std::{collections::HashMap, sync::{Arc, Mutex}};

/// Stable image identity survives retirement of its one-shot upload bytes.
/// Host: queue upload copies compiled bytes; material/pass rebuilds reuse GPU
/// residency instead of retaining a second copy on the CPU.
pub struct ParticleImage {
    /// Canonical installed identity, shared with the acquisition registry key
    /// (idResource+8). Material variants do not own another path allocation.
    pub path: Arc<str>,
    pub info: BimageInfo,
    pub scene_owned: bool,
    pub sampling_priority: std::sync::atomic::AtomicU8,
    pub demand: Arc<crate::texture::ProjectedAreaDemand>,
    pub depth: u32,
    pub payload_bytes: usize,
    payload: Mutex<Option<Vec<u8>>>,
    retain_payload: bool,
}

impl ParticleImage {
    fn new(path: Arc<str>, info: BimageInfo, depth: u32, payload: Vec<u8>, retain_payload: bool) -> Self {
        Self { path, info, scene_owned: false, sampling_priority: std::sync::atomic::AtomicU8::new(1), demand: Arc::default(), depth, payload_bytes: payload.len(), payload: Mutex::new(Some(payload)), retain_payload }
    }

    fn scene(path: Arc<str>, info: BimageInfo) -> Self {
        Self { path, info, scene_owned: true, sampling_priority: std::sync::atomic::AtomicU8::new(1), demand: Arc::default(), depth: 1,
            payload_bytes: 0, payload: Mutex::new(None), retain_payload: false }
    }

    pub(crate) fn upload<T>(&self, upload: impl FnOnce(&[u8]) -> T) -> Result<(T, usize), String> {
        let mut payload = self.payload.lock().unwrap_or_else(|error| error.into_inner());
        let bytes = payload.as_ref().ok_or("particle image GPU residency lost after source retirement")?;
        let result = upload(bytes);
        let released = if self.retain_payload { 0 } else { payload.take().map_or(0, |bytes| bytes.len()) };
        Ok((result, released))
    }
}

pub(crate) struct ParticleImages {
    images: HashMap<Arc<str>, Arc<ParticleImage>>,
    volumes: HashMap<Arc<str>, Arc<ParticleImage>>,
    shared: bool,
    scene_images: bool,
    retain_payload: bool,
    requests: usize,
    reads: usize,
    bytes: usize,
}

impl Default for ParticleImages {
    fn default() -> Self {
        Self {
            images: Default::default(), volumes: Default::default(),
            scene_images: false,
            shared: std::env::var("DH2_DEBUG_DUPLICATE_PARTICLE_IMAGES").as_deref() != Ok("1"),
            retain_payload: std::env::var("DH2_DEBUG_RETAIN_PARTICLE_IMAGE_DATA").as_deref() == Ok("1")
                || std::env::var("DH2_DEBUG_DUPLICATE_PARTICLE_IMAGES").as_deref() == Ok("1"),
            requests: 0, reads: 0, bytes: 0,
        }
    }
}

impl ParticleImages {
    pub fn use_scene_images(&mut self) { self.scene_images = self.shared; }
    pub fn images(&self) -> impl Iterator<Item = &Arc<ParticleImage>> { self.images.values() }
    pub fn scene_image(&mut self, catalog: &Catalog, name: &str) -> Result<Option<Arc<ParticleImage>>, String> {
        if self.scene_images { self.image(catalog, name).map(Some) } else { Ok(None) }
    }
    pub fn report_scene_overlap(&self, paths: &[String]) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let paths: std::collections::HashSet<_> = paths.iter().map(String::as_str).collect();
        let overlap: Vec<_> = self.images.iter().filter(|(path, _)| paths.contains(path.as_ref())).collect();
        eprintln!("MEMORY particle/scene images: particle_2d={} particle_3d={} shared_paths={} shared_payload_bytes={}",
            self.images.len(), self.volumes.len(), overlap.len(), overlap.iter().map(|(_, image)| image.payload_bytes).sum::<usize>());
    }

    pub fn image(&mut self, catalog: &Catalog, source: &str) -> Result<Arc<ParticleImage>, String> {
        self.requests += 1;
        let path: Arc<str> = crate::material::image_load_resource(catalog, source)
            .ok_or_else(|| format!("particle image unresolved: {source}"))?
            .replace('\\', "/").to_ascii_lowercase().into();
        if self.shared {
            if let Some(image) = self.images.get(&path) { return Ok(image.clone()); }
        }
        let image = if self.scene_images {
            let info = if path.starts_with('_') {
                crate::material::load_2d_image(catalog, source)?.info
            } else {
                let header = catalog.read_prefix(&path, 48).map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("particle image absent: {path}"))?;
                self.bytes += header.len();
                BimageInfo::parse(&header)?
            };
            Arc::new(ParticleImage::scene(path.clone(), info))
        } else {
            let image = crate::material::load_2d_image(catalog, source)?;
            Arc::new(ParticleImage::new(path.clone(), image.info, 1, image.payload, self.retain_payload))
        };
        self.reads += 1;
        self.bytes += image.payload_bytes;
        if self.shared { self.images.insert(path, image.clone()); }
        Ok(image)
    }

    pub fn volume(&mut self, catalog: &Catalog, source: &str) -> Result<Arc<ParticleImage>, String> {
        self.requests += 1;
        let path: Arc<str> = crate::material::image_load_resource(catalog, source)
            .ok_or_else(|| format!("particle volume unresolved: {source}"))?
            .replace('\\', "/").to_ascii_lowercase().into();
        if self.shared {
            if let Some(image) = self.volumes.get(&path) { return Ok(image.clone()); }
        }
        let image = crate::material::load_3d_image(catalog, source)?;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let full = if path.starts_with('_') {
                Some([image.info.width, image.info.height])
            } else {
                catalog.read_prefix(&path, 48).map_err(|e| e.to_string())?.map(|header| [
                    u32::from_le_bytes(header[32..36].try_into().unwrap()),
                    u32::from_le_bytes(header[36..40].try_into().unwrap()),
                ])
            };
            let separate: Vec<_> = (0..=15).filter(|mip|
                catalog.contains(&format!("{path}_mip{mip}"))).collect();
            eprintln!("MEMORY particle volume descriptor: path={path} inline={}x{}x{} mips={} full_xy={full:?} separate_levels={separate:?} payload_bytes={}",
                image.info.width, image.info.height, image.depth, image.info.mip_count, image.payload.len());
        }

        let image = Arc::new(ParticleImage::new(path.clone(), image.info, image.depth, image.payload, self.retain_payload));
        self.reads += 1;
        self.bytes += image.payload_bytes;
        if self.shared { self.volumes.insert(path, image.clone()); }
        Ok(image)
    }
}

impl Drop for ParticleImages {
    fn drop(&mut self) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") && self.requests != 0 {
            eprintln!("MEMORY particle image acquisition: shared={} requests={} reads={} bytes={}",
                self.shared, self.requests, self.reads, self.bytes);
        }
    }
}
