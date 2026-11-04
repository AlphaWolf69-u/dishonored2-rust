//! Session ownership of full-mip images. Streamed consumers use the scene owner.
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;
use super::{archive, texture_streaming::{TextureReadTarget, read_streamed_diffuse_at_target}};
use bevy::prelude::*;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

/// A binding may precede off-thread batch publication (1400607f0 placeholders).
/// Dimensions belong to the identity, so UI layout never waits for that batch.
pub(super) struct ResidentImage {
    handle: Handle<Image>,
    extent: [f64; 2],
    bytes: usize,
}
#[derive(Default)]
struct Registry {
    resident: HashMap<String, Arc<ResidentImage>>,
    leases: HashMap<String, Weak<ResidentImage>>,
    requests: usize,
    loads: usize,
    bytes: usize,
}

/// Host counterpart of image lookup 14005feb0 / map purge 140060180.
/// No payload cache: Assets owns upload staging, consumers own handles/leases.
#[derive(Resource, Clone)]
pub(super) struct ResidentImages {
    registry: Arc<Mutex<Registry>>,
    separate: bool,
}
impl Default for ResidentImages {
    fn default() -> Self {
        Self { registry: Arc::default(), separate:
            std::env::var("DH2_DEBUG_SEPARATE_RESIDENT_IMAGES").as_deref() == Ok("1") }
    }
}
impl ResidentImages {
    /// Diagnostic restores the former per-consumer identity boundary.
    pub(super) fn consumer(&self) -> Self {
        if self.separate { Self { registry: Arc::default(), separate: true } }
        else { self.clone() }
    }
    fn acquire(&self, catalog: &archive::Catalog, name: &str,
        images: &mut dyn AssetWrite<Image>) -> Result<Arc<ResidentImage>, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        if path == "_default" && !name.eq_ignore_ascii_case("_default") {
            warn!("Resident image {name}: resource missing; using the image loader's default texture");
        }
        {
            let mut registry = self.registry.lock().unwrap();
            registry.requests += 1;
            if let Some(image) = registry.resident.get(&path) { return Ok(image.clone()); }
            // An outgoing movie/frame can keep an identity alive through purge.
            if let Some(image) = registry.leases.get(&path).and_then(Weak::upgrade) {
                registry.resident.insert(path, image.clone());
                return Ok(image);
            }
        }
        // Never hold the registry lock during archive IO/decompression. A race
        // can prepare two chains, but only one becomes an asset/GPU identity.
        let mut loaded = read_streamed_diffuse_at_target(catalog, &path,
            TextureReadTarget::Initial(Some(0)))?
            .ok_or_else(|| format!("missing resident image {path}"))?;
        loaded.image.data = Some(loaded.uploads.into_iter().flat_map(|mip| mip.payload).collect());
        let extent = [loaded.image.width() as f64, loaded.image.height() as f64];
        let bytes = loaded.image.data.as_ref().map_or(0, Vec::len);
        let mut registry = self.registry.lock().unwrap();
        if let Some(image) = registry.resident.get(&path) { return Ok(image.clone()); }
        let image = Arc::new(ResidentImage {handle: images.add(loaded.image), extent, bytes});
        registry.loads += 1;
        registry.bytes += bytes;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY resident image load: path={path} bytes={bytes} separate={}", self.separate);
        }
        registry.leases.insert(path.clone(), Arc::downgrade(&image));
        registry.resident.insert(path, image.clone());
        Ok(image)
    }
    pub(super) fn load(&self, catalog: &archive::Catalog, name: &str,
        images: &mut dyn AssetWrite<Image>) -> Result<Handle<Image>, String> {
        Ok(self.acquire(catalog, name, images)?.handle.clone())
    }
    fn release_level(&self, shared: &super::texture_streaming::SharedImages) {
        let mut registry = self.registry.lock().unwrap();
        let before = registry.resident.len();
        registry.resident.retain(|path, _| shared.contains(path));
        registry.leases.retain(|_, image| image.strong_count() != 0);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY resident image registry purge: requests={} loads={} uploaded_bytes={} released={} retained_common={} retained_bytes={} live_leases={}",
                registry.requests, registry.loads, registry.bytes, before-registry.resident.len(),
                registry.resident.len(), registry.resident.values().map(|image|image.bytes).sum::<usize>(), registry.leases.len());
        }
    }
}

pub(super) fn prepare_image(catalog: &archive::Catalog, name: &str,
    images: &mut dyn AssetWrite<Image>, resident: &ResidentImages,
    aliases: &mut HashMap<String, Handle<Image>>) -> Result<Handle<Image>, String> {
    if let Some(handle) = aliases.get(name) { return Ok(handle.clone()); }
    let handle = resident.load(catalog, name, images)?;
    aliases.insert(name.to_owned(), handle.clone());
    aliases.insert(name.trim_end_matches(".curvefloat").to_owned(), handle.clone());
    Ok(handle)
}

#[derive(Resource)]
pub(super) struct UiImages {
    pub registry: ResidentImages,
    bindings: HashMap<String, Weak<crate::flash::iggy_functions::TextureBinding>>,
}
impl UiImages {
    pub(super) fn new(registry: ResidentImages) -> Self { Self {registry, bindings: HashMap::new()} }
    fn acquire(&mut self, catalog: &archive::Catalog, name: &str,
        images: &mut Assets<Image>) -> Result<crate::flash::iggy_functions::BoundTexture, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        let image = self.registry.acquire(catalog, name, images)?;
        if let Some(binding) = self.bindings.get(&path).and_then(Weak::upgrade) { return Ok(binding); }
        let binding = Arc::new(crate::flash::iggy_functions::TextureBinding::new(image.extent, image));
        self.bindings.insert(path, Arc::downgrade(&binding));
        Ok(binding)
    }
    pub(super) fn release_level(&mut self, shared: &super::texture_streaming::SharedImages) {
        self.registry.release_level(shared);
        self.bindings.retain(|_, binding| binding.strong_count() != 0);
    }
}

pub(super) fn publish_ui_images(
    mut frame: ResMut<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>,
    mut exit: MessageWriter<AppExit>,
) {
    if !frame.is_changed() { return; }
    let Some(frame) = frame.0.as_mut() else { return; };
    let mut named = HashMap::with_capacity(frame.list.named_images.len());
    for (name, binding) in &frame.list.named_images {
        let Some(image) = binding.owner::<std::sync::Arc<ResidentImage>>() else {
            error!("UI image {name}: renderer owner unavailable");
            exit.write(AppExit::error());
            return;
        };
        named.insert(name.clone(), image.handle.clone());
    }
    frame.game_images = std::sync::Arc::new(named);
}

/// Synchronous Iggy callback, fulfilled on the main world that owns Assets.
/// The movie worker can wait without blocking the window/event loop.
pub(super) struct UiTextureRequest {
    pub name: String,
    pub reply: std::sync::mpsc::SyncSender<Result<crate::flash::iggy_functions::BoundTexture, String>>,
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct UiImageService<'w> {
    images: ResMut<'w, Assets<Image>>,
    cache: ResMut<'w, UiImages>,
}
impl UiImageService<'_> {
    pub(super) fn service(
        &mut self,
        pause: &super::pause_menu::PauseMenu,
        catalog: &archive::Catalog,
    ) {
        for request in pause.texture_requests() {
            let result = self.cache.acquire(catalog, &request.name, &mut self.images);
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("LOAD UI texture binding: {} {result:?}", request.name);
            }
            let _ = request.reply.send(result);
        }
    }
}

pub(super) fn service_ui_images(
    mut service: UiImageService,
    pause: NonSend<super::pause_menu::PauseMenu>,
    state: Res<super::loading_screen::Acquisition>,
) {
    service.service(&pause, &state.catalog);
}
