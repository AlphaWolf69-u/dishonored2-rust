//! Registered image views and named UI leases, shared with scene residency.
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;
use super::{archive, texture_streaming::{read_resident_diffuse_image, DiffuseMipLevel, SharedImages}, SceneRenderQueue};
use bevy::prelude::*;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

/// A binding may precede off-thread batch publication (1400607f0 placeholders).
/// Dimensions belong to the identity, so UI layout never waits for that batch.
pub(super) struct ResidentImage {
    pub(super) handle: Handle<Image>,
    extent: [f64; 2],
    bytes: usize,
    pub(super) level: DiffuseMipLevel,
    pub(super) mip_count: u32,
    initial_view: bool,
}
#[derive(Default)]
struct Registry {
    resident: HashMap<String, Arc<ResidentImage>>,
    inline: HashMap<String, Arc<ResidentImage>>,
    layered: HashMap<String, Arc<crate::rendering::bevy_backend::layered_images::LayeredImageSource>>,
    noise: Arc<crate::rendering::bevy_backend::noise_textures::NoiseTextureSource>,
    leases: HashMap<String, Weak<ResidentImage>>,
    requests: usize,
    loads: usize,
    bytes: usize,
}

impl Registry {
    fn acquire_view(&mut self, path: &str, preferred: Option<u8>, inline_equivalent: bool) -> Option<Arc<ResidentImage>> {
        let (requested, other) = if preferred.is_none() {
            (&self.inline, &self.resident)
        } else { (&self.resident, &self.inline) };
        if let Some(image) = requested.get(path) { return Some(image.clone()); }
        if !inline_equivalent { return None; }
        let image = other.get(path).filter(|image| image.level.resource_mip.is_none())?.clone();
        if preferred.is_none() { self.inline.insert(path.to_owned(), image.clone()); }
        else {
            self.resident.insert(path.to_owned(), image.clone());
            self.leases.insert(path.to_owned(), Arc::downgrade(&image));
        }
        Some(image)
    }
}

/// Host counterpart of image lookup 14005feb0 / map purge 140060180.
/// No duplicate decoded-payload cache: Bevy owns 2D staging, while acquired
/// layered sources retire staging after upload and lease their immutable GPU image.
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
        self.acquire_at(catalog, name, images, Some(0))
    }
    fn acquire_at(&self, catalog: &archive::Catalog, name: &str,
        images: &mut dyn AssetWrite<Image>, preferred: Option<u8>) -> Result<Arc<ResidentImage>, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        if path == "_default" && !name.eq_ignore_ascii_case("_default") {
            warn!("Resident image {name}: resource missing; using the image loader's default texture");
        }
        // Fixed inline masks must not inherit a finer UI view. Conversely, a
        // full-image request can reuse inline data only if mip0 does not exist.
        let inline_equivalent = preferred.is_none() || !catalog.contains(&format!("{path}_mip0"));
        {
            let mut registry = self.registry.lock().unwrap();
            registry.requests += 1;
            if let Some(image) = registry.acquire_view(&path, preferred, inline_equivalent) {
                return Ok(image);
            }
            // Named UI wrappers can outlive purge. Fixed inline sources are
            // held by their scope owner and ordinary frame/asset handles.
            if preferred.is_some() {
                if let Some(image) = registry.leases.get(&path).and_then(Weak::upgrade) {
                    registry.resident.insert(path, image.clone());
                    return Ok(image);
                }
            }
        }
        // Never hold the registry lock during archive IO/decompression. A race
        // can prepare two chains, but only one becomes an asset/GPU identity.
        let mut loaded = read_resident_diffuse_image(catalog, &path, preferred)?
            .ok_or_else(|| format!("missing resident image {path}"))?;
        if !loaded.uploads.is_empty() {
            loaded.image.data = Some(loaded.uploads.into_iter().flat_map(|mip| mip.payload).collect());
        }
        let level = loaded.level;
        let mip_count = loaded.image.texture_descriptor.mip_level_count;
        let extent = [loaded.image.width() as f64, loaded.image.height() as f64];
        let bytes = loaded.image.data.as_ref().map_or(0, Vec::len);
        let mut registry = self.registry.lock().unwrap();
        if let Some(image) = registry.acquire_view(&path, preferred, inline_equivalent) { return Ok(image); }
        let image = Arc::new(ResidentImage {handle: images.add(loaded.image), extent, bytes, level, mip_count, initial_view: true});
        registry.loads += 1;
        registry.bytes += bytes;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY resident image load: path={path} bytes={bytes} separate={} preferred={preferred:?}", self.separate);
        }
        if preferred.is_none() { registry.inline.insert(path, image.clone()); }
        else {
            registry.leases.insert(path.clone(), Arc::downgrade(&image));
            registry.resident.insert(path, image.clone());
        }
        Ok(image)
    }
    pub(super) fn load_inline(&self, catalog: &archive::Catalog, name: &str,
        images: &mut dyn AssetWrite<Image>) -> Result<Handle<Image>, String> {
        Ok(self.acquire_at(catalog, name, images, None)?.handle.clone())
    }
    pub(super) fn load(&self, catalog: &archive::Catalog, name: &str,
        images: &mut dyn AssetWrite<Image>) -> Result<Handle<Image>, String> {
        Ok(self.acquire(catalog, name, images)?.handle.clone())
    }
    /// 14005feb0 returns the registered resource, not a new upload per caller.
    /// Only equivalent base views qualify; the scene's mip minimum stays intact.
    pub(super) fn scene_views(&self, catalog: &archive::Catalog, paths: &[String],
        preferred: Option<u8>, inline_slots: &std::collections::HashSet<usize>) -> HashMap<usize, Arc<ResidentImage>> {
        if separate_scene_views() { return HashMap::new(); }
        let registry = self.registry.lock().unwrap();
        paths.iter().enumerate().filter_map(|(slot, path)| {
            let selected = (!inline_slots.contains(&slot)).then_some(preferred).flatten().and_then(|mip| (0..=mip).rev()
                .find(|mip| catalog.contains(&format!("{path}_mip{mip}"))));
            let image = registry.resident.get(path).into_iter().chain(registry.inline.get(path))
                .find(|image| image.initial_view && image.level.resource_mip == selected)?;
            Some((slot, image.clone()))
        }).collect()
    }

    /// Drop the old fixed-view cache lease once a streamed replacement is
    /// published. Other users keep their own asset handles until finished.
    pub(super) fn retire_replaced_inline(&self, path: &str, current: &Handle<Image>) {
        let mut registry = self.registry.lock().unwrap();
        if registry.inline.get(path).is_some_and(|image| image.handle.id() != current.id()) {
            let old = registry.inline.remove(path).unwrap();
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY replaced inline image lease: path={path} old={:?} current={:?} bytes={}", old.handle.id(), current.id(), old.bytes);
            }
        }
    }

    /// 1402f5530 replaces the resource's current view. Existing named wrappers
    /// keep their old snapshot (1402df8c0); only subsequent acquisitions change.
    /// Called at commits, never for unchanged images in the streaming hot loop.
    pub(super) fn publish_view(&self, path: &str, handle: &Handle<Image>,
        level: DiffuseMipLevel, mip_count: u32) {
        if separate_scene_views() { return; }
        let mut registry = self.registry.lock().unwrap();
        let Some(previous) = registry.resident.get(path) else { return; };
        if previous.handle == *handle { return; }
        let bytes = (0..mip_count).map(|mip| level.format.mip_bytes(
            (level.width >> mip).max(1), (level.height >> mip).max(1))).sum();
        let image = Arc::new(ResidentImage {handle: handle.clone(),
            extent: [level.width as f64, level.height as f64], bytes, level, mip_count, initial_view: false});
        registry.leases.insert(path.to_owned(), Arc::downgrade(&image));
        registry.resident.insert(path.to_owned(), image);
    }

    pub(super) fn noise_source(&self) -> Arc<crate::rendering::bevy_backend::noise_textures::NoiseTextureSource> {
        self.registry.lock().unwrap().noise.clone()
    }

    /// The same registry boundary also owns cube/array identities. Their
    /// backend source retires upload staging and leases one immutable GPU image.
    pub(super) fn layered(&self, path: &str, cube: bool,
        load: impl FnOnce() -> Result<crate::rendering::bevy_backend::layered_images::LayeredImageSource, String>,
    ) -> Result<Arc<crate::rendering::bevy_backend::layered_images::LayeredImageSource>, String> {
        use crate::rendering::bevy_backend::layered_images::LayeredImageSource;
        if LayeredImageSource::separate() { return load().map(Arc::new); }
        if let Some(source) = self.registry.lock().unwrap().layered.get(path).cloned() {
            if source.is_cube() != cube { return Err(format!("image {path} acquired with an incompatible cube/array type")); }
            return Ok(source);
        }
        let prepared = Arc::new(load()?);
        let mut registry = self.registry.lock().unwrap();
        let source = registry.layered.entry(path.to_owned()).or_insert(prepared).clone();
        if source.is_cube() != cube { return Err(format!("image {path} acquired with an incompatible cube/array type")); }
        Ok(source)
    }

    fn release_level(&self, shared: &super::texture_streaming::SharedImages) {
        let mut registry = self.registry.lock().unwrap();
        let before = registry.resident.len();
        let layered_before = registry.layered.len();
        let inline_before = registry.inline.len();
        // 14005fde0's builtins are class 2 even without archive manifest entries.
        let persistent = |path: &String| shared.contains(path) || crate::material::intrinsic_image_kind(path).is_some();
        registry.resident.retain(|path, _| persistent(path));
        registry.inline.retain(|path, _| persistent(path));
        registry.layered.retain(|path, _| persistent(path));
        registry.leases.retain(|_, image| image.strong_count() != 0);
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY resident image registry purge: requests={} loads={} uploaded_bytes={} released={} retained_persistent={} retained_bytes={} live_leases={}",
                registry.requests, registry.loads, registry.bytes, before-registry.resident.len(),
                registry.resident.len(), registry.resident.values().map(|image|image.bytes).sum::<usize>(), registry.leases.len());
            eprintln!("MEMORY inline image registry purge: released={} retained_persistent={} retained_bytes={}",
                inline_before - registry.inline.len(), registry.inline.len(),
                registry.inline.values().map(|image| image.bytes).sum::<usize>());
            eprintln!("MEMORY layered image registry purge: released={} retained_persistent={}",
                layered_before - registry.layered.len(), registry.layered.len());
            eprintln!("MEMORY intrinsic image registry: retained_2d={} retained_layered={}",
                registry.resident.keys().filter(|path| crate::material::intrinsic_image_kind(path).is_some()).count(),
                registry.layered.keys().filter(|path| crate::material::intrinsic_image_kind(path).is_some()).count());
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
        images: &mut Assets<Image>, scene: Option<&SceneRenderQueue>, shared: Option<&SharedImages>) -> Result<crate::flash::iggy_functions::BoundTexture, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        if let Some(binding) = self.bindings.get(&path).and_then(Weak::upgrade) { return Ok(binding); }
        let current = if separate_scene_views() { None } else {
            scene.and_then(|scene| {
                let slot = scene.diffuse_image_paths.iter().position(|source| source == &path)?;
                Some((scene.diffuse_image_handles[slot].clone()?, scene.diffuse_mip_states[slot].resident?))
            }).or_else(|| shared.and_then(|shared| shared.current_view(&path)))
        };
        let image = if let Some((handle, level)) = current {
            let source = images.get(&handle).ok_or_else(|| format!("committed UI image view unavailable: {path}"))?;
            Arc::new(ResidentImage {handle: handle.clone(), extent: [source.width() as f64, source.height() as f64],
                bytes: source.data.as_ref().map_or(0, Vec::len), level,
                mip_count: source.texture_descriptor.mip_level_count, initial_view: false})
        } else { self.registry.acquire(catalog, name, images)? };
        let binding = Arc::new(crate::flash::iggy_functions::TextureBinding::new(image.extent, image));
        self.bindings.insert(path, Arc::downgrade(&binding));
        Ok(binding)
    }
    pub(super) fn release_level(&mut self, shared: &super::texture_streaming::SharedImages) {
        self.registry.release_level(shared);
        self.bindings.retain(|_, binding| binding.strong_count() != 0);
    }

    /// Only identities that can actually be acquired from a surviving owner
    /// retain their loading credit; manifest membership by itself is not a
    /// resident image. Original 140060180 purges outgoing map resources.
    pub(super) fn release_read_history(&self, shared: &SharedImages) {
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let registry = self.registry.registry.lock().unwrap();
        // Catalog read history normalizes case and separators. Most owner
        // keys are already canonical; only exceptional aliases need copies.
        let mut aliases = std::collections::HashSet::<String>::new();
        let paths = shared.resident_paths()
            .chain(registry.resident.keys().map(String::as_str))
            .chain(registry.inline.keys().map(String::as_str))
            .chain(registry.layered.keys().map(String::as_str))
            .chain(registry.leases.iter().filter(|(_, value)| value.strong_count() != 0).map(|(path, _)| path.as_str()))
            .chain(self.bindings.iter().filter(|(_, value)| value.strong_count() != 0).map(|(path, _)| path.as_str()));
        for path in paths {
            if path.bytes().any(|byte| byte == b'\\' || byte.is_ascii_uppercase()) {
                let mut key = path.replace('\\', "/");
                key.make_ascii_lowercase();
                aliases.insert(key);
            }
        }
        let (removed, bytes, slots_before, slots_after) = crate::loading::retain_kind("image", |path| {
            (!aliases.is_empty() && aliases.contains(path))
                || shared.is_resident(path)
                || registry.resident.contains_key(path)
                || registry.inline.contains_key(path)
                || registry.layered.contains_key(path)
                || registry.leases.get(path).is_some_and(|lease| lease.strong_count() != 0)
                || self.bindings.get(path).is_some_and(|binding| binding.strong_count() != 0)
        });
        if trace {
            let cycles = started.zip(crate::loading::thread_cycles())
                .map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("MEMORY image history release: entries={removed} path_capacity_bytes={bytes} slots_before={slots_before} slots_after={slots_after} aliases={} cycles={cycles}", aliases.len());
        }
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
    scene: Option<Res<'w, SceneRenderQueue>>,
    shared: Option<Res<'w, SharedImages>>,
}
impl UiImageService<'_> {
    pub(super) fn service(
        &mut self,
        pause: &super::pause_menu::PauseMenu,
        catalog: &archive::Catalog,
    ) {
        for request in pause.texture_requests() {
            let result = self.cache.acquire(catalog, &request.name, &mut self.images, self.scene.as_deref(), self.shared.as_deref());
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

fn separate_scene_views() -> bool {
    static SEPARATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_SCENE_IMAGE_VIEWS").as_deref() == Ok("1"))
}
