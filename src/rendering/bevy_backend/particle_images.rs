//! Backend residency of immutable particle images. The game's image registry
//! (1400607f0/14005feb0) owns one image independently of material draw states.
//! Compiled blocks are uploaded unchanged; samplers belong to material bindings.
use bevy::render::{render_resource::*, renderer::{RenderDevice, RenderQueue}};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};

struct ResidentImage {
    // Retaining the acquired identity prevents allocator address reuse from
    // aliasing another image while its GPU texture is still cached.
    _source: Arc<dyn Send + Sync>,
    _texture: Texture,
    view: TextureView,
}

pub(super) struct ParticleImagesGpu {
    resident: HashMap<(TextureDimension, usize), ResidentImage>,
    shared: bool,
    timing: bool,
    requests: usize,
    uploads: usize,
    bytes: usize,
    elapsed: Duration,
}

impl Default for ParticleImagesGpu {
    fn default() -> Self {
        Self {
            resident: Default::default(),
            shared: std::env::var("DH2_DEBUG_DUPLICATE_PARTICLE_IMAGES").as_deref() != Ok("1"),
            timing: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
            requests: 0, uploads: 0, bytes: 0, elapsed: Duration::ZERO,
        }
    }
}

impl ParticleImagesGpu {
    pub fn acquire<T: Send + Sync + 'static>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: &Arc<T>,
        descriptor: &TextureDescriptor,
        order: TextureDataOrder,
        bytes: &[u8],
    ) -> TextureView {
        let started = self.timing.then(Instant::now);
        self.requests += 1;
        let key = (descriptor.dimension, Arc::as_ptr(source) as usize);
        if self.shared {
            if let Some(image) = self.resident.get(&key) {
                if let Some(started) = started { self.elapsed += started.elapsed(); }
                return image.view.clone();
            }
        }
        let texture = device.create_texture_with_data(queue, descriptor, order, bytes);
        let view = texture.create_view(&Default::default());
        self.uploads += 1;
        self.bytes += bytes.len();
        if self.shared {
            self.resident.insert(key, ResidentImage {
                _source: source.clone(), _texture: texture, view: view.clone(),
            });
        }
        if let Some(started) = started { self.elapsed += started.elapsed(); }
        view
    }
}

impl Drop for ParticleImagesGpu {
    fn drop(&mut self) {
        if self.timing && self.requests != 0 {
            eprintln!("MEMORY particle image GPU release: shared={} requests={} uploads={} upload_bytes={} cached={} acquire_ms={:.3}",
                self.shared, self.requests, self.uploads, self.bytes, self.resident.len(), self.elapsed.as_secs_f64() * 1000.0);
        }
    }
}
