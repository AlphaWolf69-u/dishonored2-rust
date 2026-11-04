//! Backend residency of immutable particle images. The game's image registry
//! (1400607f0/14005feb0) owns one image independently of material draw states.
//! Compiled blocks are uploaded unchanged; samplers belong to material bindings.
use bevy::render::{render_resource::*, renderer::{RenderDevice, RenderQueue}};
use bevy::{prelude::{Resource, Handle, Image}, render::{extract_resource::ExtractResource, render_asset::RenderAssets, texture::GpuImage}};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};

/// Material consumers of scene-owned 2D images. Handle snapshots change only
/// at residency commits; immutable definitions/demands share their identity.
#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct SceneParticleImages {
    pub slots: Arc<Vec<(usize, Arc<crate::rendering::particles::image_resources::ParticleImage>)>>,
    pub handles: Arc<HashMap<Arc<str>, Handle<Image>>>,
    pub world_materials: Arc<HashMap<String, Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>>>,
}

struct ResidentImage {
    // Retaining the acquired identity prevents allocator address reuse from
    // aliasing another image while its GPU texture is still cached.
    _source: Arc<crate::rendering::particles::image_resources::ParticleImage>,
    _texture: Texture,
    view: TextureView,
}

pub(super) struct ParticleImagesGpu {
    resident: HashMap<(TextureDimension, usize), ResidentImage>,
    scene_handles: Option<Arc<HashMap<Arc<str>, Handle<Image>>>>,
    scene_views: HashMap<Arc<str>, TextureView>,
    pub changed_paths: std::collections::HashSet<Arc<str>>,
    shared: bool,
    timing: bool,
    requests: usize,
    uploads: usize,
    bytes: usize,
    cpu_released_bytes: usize,
    elapsed: Duration,
}

impl Default for ParticleImagesGpu {
    fn default() -> Self {
        Self {
            resident: Default::default(), scene_handles: None,
            scene_views: Default::default(), changed_paths: Default::default(),
            shared: std::env::var("DH2_DEBUG_DUPLICATE_PARTICLE_IMAGES").as_deref() != Ok("1"),
            timing: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
            requests: 0, uploads: 0, bytes: 0, cpu_released_bytes: 0, elapsed: Duration::ZERO,
        }
    }
}

impl ParticleImagesGpu {
    /// Preserve all committed views until every replacement is GPU-ready.
    /// Stable frames compare one Arc identity; no per-draw lock or map copy.
    pub fn sync_scene(&mut self, sources: &SceneParticleImages,
        images: &RenderAssets<GpuImage>, force: bool) -> Result<bool, String> {
        if !force && self.scene_handles.as_ref().is_some_and(|old| Arc::ptr_eq(old, &sources.handles)) {
            return Ok(false);
        }
        if sources.handles.len() != sources.slots.len() {
            return Err("particle image handles await scene registration".into());
        }
        let mut next = HashMap::with_capacity(sources.handles.len());
        for (path, handle) in sources.handles.iter() {
            let image = images.get(handle.id()).ok_or_else(|| format!("particle image awaits scene upload: {path}"))?;
            next.insert(path.clone(), image.texture_view.clone());
        }
        self.changed_paths.clear();
        for (path, view) in &next {
            if self.scene_views.get(path).is_none_or(|old| old.id() != view.id()) {
                self.changed_paths.insert(path.clone());
            }
        }
        self.scene_views = next;
        self.scene_handles = Some(sources.handles.clone());
        Ok(!self.changed_paths.is_empty())
    }

    pub fn acquire(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: &Arc<crate::rendering::particles::image_resources::ParticleImage>,
        descriptor: &TextureDescriptor,
        order: TextureDataOrder,
    ) -> Result<TextureView, String> {
        let started = self.timing.then(Instant::now);
        self.requests += 1;
        if source.scene_owned {
            if descriptor.dimension != TextureDimension::D2 {
                return Err("scene-owned particle image has incompatible dimension".into());
            }
            let view = self.scene_views.get(&source.path)
                .ok_or_else(|| format!("particle image is not scene-resident: {}", source.path))?.clone();
            if let Some(started) = started { self.elapsed += started.elapsed(); }
            return Ok(view);
        }
        let key = (descriptor.dimension, Arc::as_ptr(source) as usize);
        if self.shared {
            if let Some(image) = self.resident.get(&key) {
                if let Some(started) = started { self.elapsed += started.elapsed(); }
                return Ok(image.view.clone());
            }
        }
        let (texture, released) = source.upload(|bytes| device.create_texture_with_data(queue, descriptor, order, bytes))?;
        self.cpu_released_bytes += released;
        let view = texture.create_view(&Default::default());
        self.uploads += 1;
        self.bytes += source.payload_bytes;
        if self.shared {
            self.resident.insert(key, ResidentImage {
                _source: source.clone(), _texture: texture, view: view.clone(),
            });
        }
        if let Some(started) = started { self.elapsed += started.elapsed(); }
        Ok(view)
    }
}

impl Drop for ParticleImagesGpu {
    fn drop(&mut self) {
        if self.timing && self.requests != 0 {
            eprintln!("MEMORY particle image GPU release: shared={} requests={} uploads={} upload_bytes={} cached={} cpu_released_bytes={} shared_2d={} acquire_ms={:.3}",
                self.shared, self.requests, self.uploads, self.bytes, self.resident.len(), self.cpu_released_bytes, self.scene_views.len(), self.elapsed.as_secs_f64() * 1000.0);
        }
    }
}
