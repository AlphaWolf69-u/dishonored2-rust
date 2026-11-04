//! Level acquisition of particle images. Image registry 1400607f0/14005feb0
//! shares a resource independently of its material and render state.
use crate::{archive::Catalog, texture::{InlineCompressedChain, InlineVolumeChain}};
use std::{collections::HashMap, sync::Arc};

pub(crate) struct ParticleImages {
    images: HashMap<String, Arc<InlineCompressedChain>>,
    volumes: HashMap<String, Arc<InlineVolumeChain>>,
    shared: bool,
    requests: usize,
    reads: usize,
    bytes: usize,
}

impl Default for ParticleImages {
    fn default() -> Self {
        Self {
            images: Default::default(), volumes: Default::default(),
            shared: std::env::var("DH2_DEBUG_DUPLICATE_PARTICLE_IMAGES").as_deref() != Ok("1"),
            requests: 0, reads: 0, bytes: 0,
        }
    }
}

impl ParticleImages {
    pub fn report_scene_overlap(&self, paths: &[String]) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let paths: std::collections::HashSet<_> = paths.iter().map(String::as_str).collect();
        let overlap: Vec<_> = self.images.iter().filter(|(path, _)| paths.contains(path.as_str())).collect();
        eprintln!("MEMORY particle/scene images: particle_2d={} particle_3d={} shared_paths={} shared_payload_bytes={}",
            self.images.len(), self.volumes.len(), overlap.len(), overlap.iter().map(|(_, image)| image.payload.len()).sum::<usize>());
    }

    pub fn image(&mut self, catalog: &Catalog, source: &str) -> Result<Arc<InlineCompressedChain>, String> {
        self.requests += 1;
        let path = crate::material::image_load_resource(catalog, source)
            .ok_or_else(|| format!("particle image unresolved: {source}"))?
            .replace('\\', "/").to_ascii_lowercase();
        if self.shared {
            if let Some(image) = self.images.get(&path) { return Ok(image.clone()); }
        }
        let image = Arc::new(crate::material::load_2d_image(catalog, source)?);
        self.reads += 1;
        self.bytes += image.payload.len();
        if self.shared { self.images.insert(path, image.clone()); }
        Ok(image)
    }

    pub fn volume(&mut self, catalog: &Catalog, source: &str) -> Result<Arc<InlineVolumeChain>, String> {
        self.requests += 1;
        let path = crate::material::image_load_resource(catalog, source)
            .ok_or_else(|| format!("particle volume unresolved: {source}"))?
            .replace('\\', "/").to_ascii_lowercase();
        if self.shared {
            if let Some(image) = self.volumes.get(&path) { return Ok(image.clone()); }
        }
        let image = Arc::new(crate::material::load_3d_image(catalog, source)?);
        self.reads += 1;
        self.bytes += image.payload.len();
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
