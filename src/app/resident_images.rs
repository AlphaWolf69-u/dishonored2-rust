//! Registered image views and named UI leases, shared with scene residency.
use crate::rendering::bevy_backend::asset_preparation::{AssetBatch, AssetWrite};
use super::{archive, texture_streaming::{initial_separate_mip, read_resident_diffuse_image, DiffuseMipLevel, SharedImages}, SceneRenderQueue};
use bevy::prelude::*;
use std::{collections::HashMap, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};

/// A binding may precede off-thread batch publication (1400607f0 placeholders).
/// Dimensions belong to the identity, so UI layout never waits for that batch.
pub(super) struct ResidentImage {
    pub(super) handle: Handle<Image>,
    extent: [f64; 2],
    bytes: usize,
    pub(super) level: DiffuseMipLevel,
    pub(super) mip_count: u32,
    initial_view: bool,
    discarded: AtomicBool,
}

/// Drop reservations from the shared image registry if a scene worker fails
/// or its finished batch is dropped before reaching the main asset store.
pub(super) struct PreparedImages {
    batch: Option<AssetBatch<Image>>,
    registry: ResidentImages,
    shared: SharedImages,
    catalog: Option<u64>,
}
impl PreparedImages {
    pub(super) fn new(assets: &Assets<Image>, mut registry: ResidentImages, shared: SharedImages) -> Self {
        registry.layered_batch = Some(Arc::new(()));
        Self { batch: Some(AssetBatch::new(assets)), registry, shared, catalog: None }
    }
    pub(super) fn registry(&self) -> &ResidentImages { &self.registry }
    pub(super) fn begin_acquisition(&mut self, catalog: Option<&archive::Catalog>) {
        self.catalog = catalog.map(archive::Catalog::identity);
    }
    pub(super) fn assets(&mut self) -> &mut AssetBatch<Image> {
        self.batch.as_mut().expect("scene images already published")
    }
    pub(super) fn setup_inputs(&mut self) -> (&mut AssetBatch<Image>, &SharedImages) {
        (self.batch.as_mut().expect("scene images already published"), &self.shared)
    }
    pub(super) fn publish(mut self, assets: &mut Assets<Image>) {
        self.batch.take().expect("scene images already published").publish(assets);
        self.registry.finish_layered_batch(true);
    }
}
impl Drop for PreparedImages {
    fn drop(&mut self) {
        if let Some(batch) = &self.batch {
            self.registry.discard_batch(batch);
            self.registry.finish_layered_batch(false);
            if let Some(catalog) = self.catalog {
                self.registry.release_read_history(catalog, &self.shared, None, None);
            }
        }
    }
}

type LayeredImageSource = crate::rendering::bevy_backend::layered_images::LayeredImageSource;

struct LayeredRegistration {
    source: Arc<LayeredImageSource>,
    batch: Option<Weak<()>>,
}

#[derive(Default)]
struct Registry {
    resident: HashMap<String, Arc<ResidentImage>>,
    inline: HashMap<String, Arc<ResidentImage>>,
    layered: HashMap<String, LayeredRegistration>,
    default_colour_cube: Option<Handle<Image>>,
    noise: Arc<crate::rendering::bevy_backend::noise_textures::NoiseTextureSource>,
    leases: HashMap<String, Weak<ResidentImage>>,
    scene_leases: HashMap<String, Weak<ResidentImage>>,
    /// Editable views are never reused. Asset IDs also cover independent
    /// strong handles reconstructed by Bevy after the initial handle drops.
    editable: HashMap<String, Vec<(AssetId<Image>, Weak<bevy::asset::StrongHandle>)>>,
    editable_layered: HashMap<String, Vec<Weak<crate::rendering::bevy_backend::layered_images::LayeredImageSource>>>,
    requests: usize,
    loads: usize,
    bytes: usize,
}

impl Registry {
    fn acquire_view(&mut self, path: &str, preferred: Option<u8>, selected_mip: Option<u8>) -> Option<Arc<ResidentImage>> {
        let (requested, other) = if preferred.is_none() {
            (&self.inline, &self.resident)
        } else { (&self.resident, &self.inline) };
        if let Some(image) = requested.get(path).filter(|image| image.level.resource_mip == selected_mip) {
            return Some(image.clone());
        }
        if selected_mip.is_some() { return None; }
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
    layered_batch: Option<Arc<()>>,
}
impl Default for ResidentImages {
    fn default() -> Self {
        Self { registry: Arc::default(), separate:
            std::env::var("DH2_DEBUG_SEPARATE_RESIDENT_IMAGES").as_deref() == Ok("1"), layered_batch: None }
    }
}
impl ResidentImages {
    ///2ec020 assigns ImageManager's first pointer to the real `_default`
    ///resource. Borrow its already prepared view; model reset never performs
    ///archive IO or substitutes a new white/checker image on the frame thread.
    pub(super) fn default_texture(&self) -> Option<Handle<Image>> {
        let registry=self.registry.lock().ok()?;
        registry.resident.get("_default").or_else(||registry.inline.get("_default"))
            .map(|image|image.handle.clone())
            .or_else(||registry.leases.get("_default").or_else(||registry.scene_leases.get("_default"))
                .and_then(Weak::upgrade).map(|image|image.handle.clone()))
            .or_else(||registry.editable.get("_default").and_then(|views|views.iter().rev()
                .find_map(|(_,view)|view.upgrade().map(Handle::Strong))))
    }
    /// 1402ec020 / 1402f5920 initialize the supported builtin views during
    /// renderer bootstrap. Use the same owner as their subsequent consumers.
    pub(super) fn prepare_intrinsics(&self, catalog: &archive::Catalog,
        images: &mut dyn AssetWrite<Image>) -> Result<(), String> {
        if self.separate || std::env::var_os("DH2_DEBUG_DEFER_INTRINSIC_IMAGES").is_some_and(|v| v == "1") {
            return Ok(());
        }
        for name in ["_default", "_black", "_white"] {
            if !catalog.image_has_overlay(name) {
                self.acquire(catalog, name, images)?;
            }
        }
        if std::env::var_os("DH2_DEBUG_MAP_DEFAULT_COLOUR_CUBE").is_none_or(|v| v != "1") {
            self.registry.lock().unwrap().default_colour_cube.get_or_insert_with(|| {
                images.add(crate::rendering::bevy_backend::colour_cube::identity_image())
            });
        }
        Ok(())
    }

    pub(super) fn default_colour_cube(&self) -> Option<Handle<Image>> {
        self.registry.lock().unwrap().default_colour_cube.clone()
    }

    /// Lease a streamed scene view for other named consumers without making
    /// the registry itself keep a runtime-only image alive. UI owners that
    /// acquire it retain their own strong lease.
    pub(super) fn lease_scene_view(&self, path: &str, handle: &Handle<Image>, level: DiffuseMipLevel,
        mip_count: u32) -> Arc<ResidentImage> {
        let mut registry = self.registry.lock().unwrap();
        if let Some(existing) = registry.resident.get(path).into_iter()
            .chain(registry.inline.get(path))
            .find(|image| image.handle.id() == handle.id()) {
            return existing.clone();
        }
        if let Some(existing) = registry.scene_leases.get(path).into_iter().chain(registry.leases.get(path))
            .filter_map(Weak::upgrade).find(|image| image.handle.id() == handle.id()) {
            return existing;
        }
        let bytes = (0..mip_count).map(|mip| level.format.mip_bytes(
            (level.width >> mip).max(1), (level.height >> mip).max(1))).sum();
        let image = Arc::new(ResidentImage { handle: handle.clone(),
            extent: [level.width as f64, level.height as f64], bytes, level, mip_count, initial_view: false,
            discarded: AtomicBool::new(false) });
        registry.scene_leases.insert(path.to_owned(), Arc::downgrade(&image));
        image
    }

    pub(super) fn retire_unused_scene_lease(&self, path: &str) {
        let mut registry = self.registry.lock().unwrap();
        if registry.scene_leases.get(path).is_some_and(|lease| lease.strong_count() == 0) {
            registry.scene_leases.remove(path);
        }
    }
    /// Diagnostic restores the former per-consumer identity boundary.
    pub(super) fn consumer(&self) -> Self {
        if self.separate { Self { registry: Arc::default(), separate: true, layered_batch: None } }
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
        let editable = catalog.image_has_overlay(&path);
        // Fixed inline masks must not inherit a finer UI view. Conversely, a
        // full-image request can reuse inline data only if mip0 does not exist.
        let selected_mip = initial_separate_mip(catalog, &path, preferred);
        {
            let mut registry = self.registry.lock().unwrap();
            registry.requests += 1;
            if !editable {
                if let Some(image) = registry.acquire_view(&path, preferred, selected_mip) {
                    return Ok(image);
                }
            }
            // Named UI wrappers can outlive purge. Fixed inline sources are
            // held by their scope owner and ordinary frame/asset handles.
            if !editable && preferred.is_some() {
                let image = registry.leases.get(&path).into_iter().chain(registry.scene_leases.get(&path))
                    .filter_map(Weak::upgrade).find(|image| image.level.resource_mip == selected_mip);
                if let Some(image) = image {
                    registry.leases.insert(path.clone(), Arc::downgrade(&image));
                    registry.resident.insert(path, image.clone());
                    return Ok(image);
                }
            }
        }
        // Never hold the registry lock during archive IO/decompression. A race
        // can prepare two chains, but only one becomes an asset/GPU identity.
        let mut loaded = read_resident_diffuse_image(catalog, &path, preferred)?
            .ok_or_else(|| format!("missing resident image {path}"))?;
        loaded.pack_uploads();
        let level = loaded.level;
        let mip_count = loaded.image.texture_descriptor.mip_level_count;
        let extent = [loaded.image.width() as f64, loaded.image.height() as f64];
        let bytes = loaded.image.data.as_ref().map_or(0, Vec::len);
        let mut registry = self.registry.lock().unwrap();
        if !editable {
            if let Some(image) = registry.acquire_view(&path, preferred, selected_mip) { return Ok(image); }
        }
        let image = Arc::new(ResidentImage {handle: images.add(loaded.image), extent, bytes, level, mip_count,
            initial_view: true, discarded: AtomicBool::new(false) });
        registry.loads += 1;
        registry.bytes += bytes;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY resident image load: path={path} bytes={bytes} separate={} preferred={preferred:?}", self.separate);
        }
        if editable {
            if let Handle::Strong(handle) = &image.handle {
                let generations = registry.editable.entry(path.clone()).or_default();
                generations.push((image.handle.id(), Arc::downgrade(handle)));
            }
        } else {
            if preferred.is_none() { registry.inline.insert(path, image.clone()); }
            else {
                registry.leases.insert(path.clone(), Arc::downgrade(&image));
                registry.resident.insert(path, image.clone());
            }
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
            if catalog.image_has_overlay(path) { return None; }
            let selected = initial_separate_mip(catalog, path,
                if inline_slots.contains(&slot) { None } else { preferred });
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

    /// 1402f5530 replaces the resource's current view. A live named wrapper
    /// keeps its snapshot until its last reference releases (1402df8c0).
    /// Called at commits, never for unchanged images in the streaming hot loop.
    pub(super) fn publish_view(&self, path: &str, replaced: Option<AssetId<Image>>, handle: &Handle<Image>,
        level: DiffuseMipLevel, mip_count: u32) {
        if separate_scene_views() { return; }
        let mut registry = self.registry.lock().unwrap();
        let Some(previous) = registry.resident.get(path) else { return; };
        if Some(previous.handle.id()) != replaced { return; }
        if previous.handle == *handle { return; }
        if previous.level.resource_mip != level.resource_mip {
            // The replacement is a different view. Release the shared old
            // pin without replacing a fixed-view identity with a coarse one.
            registry.resident.remove(path);
            return;
        }
        let bytes = (0..mip_count).map(|mip| level.format.mip_bytes(
            (level.width >> mip).max(1), (level.height >> mip).max(1))).sum();
        let image = Arc::new(ResidentImage {handle: handle.clone(),
            extent: [level.width as f64, level.height as f64], bytes, level, mip_count, initial_view: false,
            discarded: AtomicBool::new(false) });
        registry.leases.insert(path.to_owned(), Arc::downgrade(&image));
        registry.resident.insert(path.to_owned(), image);
    }

    pub(super) fn noise_source(&self) -> Arc<crate::rendering::bevy_backend::noise_textures::NoiseTextureSource> {
        self.registry.lock().unwrap().noise.clone()
    }

    /// The same registry boundary also owns cube/array identities. Their
    /// backend source retires upload staging and leases one immutable GPU image.
    pub(super) fn layered(&self, catalog: &archive::Catalog, path: &str, cube: bool,
        load: impl FnOnce() -> Result<LayeredImageSource, String>,
    ) -> Result<Arc<LayeredImageSource>, String> {
        if LayeredImageSource::separate() { return load().map(Arc::new); }
        if catalog.image_has_overlay(path) {
            let source = Arc::new(load()?);
            if source.is_cube() != cube { return Err(format!("image {path} acquired with an incompatible cube/array type")); }
            let mut registry = self.registry.lock().unwrap();
            let generations = registry.editable_layered.entry(path.to_owned()).or_default();
            generations.retain(|generation| generation.strong_count() != 0);
            generations.push(Arc::downgrade(&source));
            return Ok(source);
        }
        if let Some(source) = self.registry.lock().unwrap().layered.get(path).map(|entry| entry.source.clone()) {
            if source.is_cube() != cube { return Err(format!("image {path} acquired with an incompatible cube/array type")); }
            return Ok(source);
        }
        let prepared = Arc::new(load()?);
        let mut registry = self.registry.lock().unwrap();
        let source = registry.layered.entry(path.to_owned()).or_insert_with(|| LayeredRegistration {
            source: prepared, batch: self.layered_batch.as_ref().map(Arc::downgrade),
        }).source.clone();
        if source.is_cube() != cube { return Err(format!("image {path} acquired with an incompatible cube/array type")); }
        Ok(source)
    }

    fn finish_layered_batch(&self, publish: bool) {
        let Some(batch) = &self.layered_batch else { return; };
        let mut registry = self.registry.lock().unwrap_or_else(|error| error.into_inner());
        registry.layered.retain(|_, entry| {
            if !entry.batch.as_ref().is_some_and(|owner| owner.as_ptr() == Arc::as_ptr(batch)) {
                return true;
            }
            if publish { entry.batch = None; }
            publish
        });
    }

    fn release_level(&self, shared: &super::texture_streaming::SharedImages) {
        let mut registry = self.registry.lock().unwrap();
        let before = registry.resident.len();
        let layered_before = registry.layered.len();
        let inline_before = registry.inline.len();
        // 14005fde0's builtins are class 2 even without archive manifest entries.
        let persistent = |path: &String| shared.can_persist(path) || crate::material::intrinsic_image_kind(path).is_some();
        registry.resident.retain(|path, _| persistent(path));
        registry.inline.retain(|path, _| persistent(path));
        registry.layered.retain(|path, _| persistent(path));
        registry.leases.retain(|_, image| image.strong_count() != 0);
        registry.scene_leases.retain(|_, image| image.strong_count() != 0);
        registry.editable_layered.retain(|_, generations| {
            generations.retain(|generation| generation.strong_count() != 0);
            !generations.is_empty()
        });
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

    fn discard_batch(&self, batch: &AssetBatch<Image>) {
        let mut registry = self.registry.lock().unwrap_or_else(|error| error.into_inner());
        let keep = |image: &ResidentImage| {
            if !batch.contains(image.handle.id()) { return true; }
            image.discarded.store(true, Ordering::Relaxed);
            false
        };
        registry.resident.retain(|_, image| keep(image));
        registry.inline.retain(|_, image| keep(image));
        registry.leases.retain(|_, image| image.upgrade().is_some_and(|image| keep(&image)));
        registry.scene_leases.retain(|_, image| image.upgrade().is_some_and(|image| keep(&image)));
        registry.editable.retain(|_, generations| {
            generations.retain(|(id, _)| !batch.contains(*id));
            !generations.is_empty()
        });
    }
}

/// Observe the same view admission using a real separate mip and its inline
/// chain. No gameplay view, GPU upload or movie is required for this lookup.
pub(super) fn inspect_view_replacement(catalog: &archive::Catalog, path: &str) -> Result<(), String> {
    if std::env::var_os("DH2_DEBUG_DISCARD_IMAGE_BATCH").is_some_and(|value| value == "1") {
        return observe_discarded_batch(catalog, path);
    }
    let registry = ResidentImages::default();
    let mut ui = UiImages::new(registry.clone());
    let mut images = Assets::<Image>::default();
    let full = registry.acquire(catalog, path, &mut images)?;
    let mut scene = SceneRenderQueue {
        diffuse_image_paths: vec![path.to_owned()],
        diffuse_image_handles: vec![Some(full.handle.clone())],
        diffuse_mip_states: vec![super::texture_streaming::DiffuseMipState {
            resident: Some(full.level), ..Default::default()
        }],
        ..Default::default()
    };
    fn image_id(binding: &crate::flash::iggy_functions::TextureBinding) -> Option<AssetId<Image>> {
        binding.owner::<Arc<ResidentImage>>().map(|image| image.handle.id())
    }
    let first = ui.acquire(catalog, path, &mut images, Some(&scene), None)?;
    let first_view = image_id(&first) == Some(full.handle.id());
    let mut coarse = read_resident_diffuse_image(catalog, path, None)?.ok_or("missing inline view")?;
    coarse.pack_uploads();
    let coarse_mips = coarse.image.texture_descriptor.mip_level_count;
    let coarse_level = coarse.level;
    let coarse_handle = images.add(coarse.image);
    registry.publish_view(path, Some(full.handle.id()), &coarse_handle, coarse_level, coarse_mips);
    scene.diffuse_image_handles[0] = Some(coarse_handle.clone());
    scene.diffuse_mip_states[0].resident = Some(coarse_level);
    let existing = ui.acquire(catalog, path, &mut images, Some(&scene), None)?;
    let shared_wrapper = Arc::ptr_eq(&first, &existing);
    let old_lease = Arc::downgrade(&first);
    drop(first);
    drop(existing);
    let reopened = ui.acquire(catalog, path, &mut images, Some(&scene), None)?;
    let current_view = image_id(&reopened) == Some(coarse_handle.id());
    eprintln!("UI_NAMED_VIEW path={path} initial_committed={first_view} reused_wrapper={shared_wrapper} old_wrapper_released={} reopened_committed={current_view}", old_lease.strong_count() == 0);
    let streamed = registry.lease_scene_view(path, &coarse_handle, coarse_level, coarse_mips);
    let restored = registry.acquire(catalog, path, &mut images)?;
    let shared = Arc::ptr_eq(&full, &restored);
    let checksum = |images: &Assets<Image>, image: &ResidentImage| images.get(&image.handle).and_then(|image| image.data.as_ref())
        .map(|bytes| { let mut crc = flate2::Crc::new(); crc.update(bytes); crc.sum() });
    let expected_crc = checksum(&images, &full);
    let full_mip = full.level.resource_mip;
    drop(restored);
    drop(full);
    registry.release_level(&SharedImages::default());
    let reacquired = registry.acquire(catalog, path, &mut images)?;
    eprintln!("RESOURCE_RESIDENT_VIEWS path={path} full={full_mip:?} streamed={:?} shared={shared} restored={:?} full_crc={expected_crc:?} restored_crc={:?}",
        streamed.level.resource_mip, reacquired.level.resource_mip, checksum(&images, &reacquired));
    Ok(())
}

fn observe_discarded_batch(catalog: &archive::Catalog, path: &str) -> Result<(), String> {
    const RETAINED: &str = "generated/image/models/effects/env/animals/stink_bug/bug01_n.bimage7";
    let registry = ResidentImages::default();
    let mut ui = UiImages::new(registry.clone());
    let mut assets = Assets::<Image>::default();
    let published = registry.acquire(catalog, "_white", &mut assets)?;
    let _retained = registry.acquire(catalog, RETAINED, &mut assets)?;
    let other_catalog = archive::Catalog::open(catalog.base()).map_err(|error| error.to_string())?;
    other_catalog.read(path).map_err(|error| error.to_string())?.ok_or("image absent in independent catalog")?;
    let mut batch = PreparedImages::new(&assets, registry.clone(), SharedImages::default());
    batch.begin_acquisition(Some(catalog));
    let staged = registry.acquire(catalog, path, batch.assets())?;
    let staged_bytes = batch.assets().get(&staged.handle).and_then(|image| image.data.as_ref())
        .ok_or("installed image did not create a staged payload")?;
    let expected = blake3::hash(staged_bytes);
    let pending = ui.acquire(catalog, path, &mut assets, None, None)?;
    let pending_id = pending.owner::<Arc<ResidentImage>>().ok_or("missing staged UI lease")?.handle.id();
    let recorded = crate::loading::catalog_read_recorded(catalog.identity(), "image", path);
    drop(batch);
    let read_retired = !crate::loading::catalog_read_recorded(catalog.identity(), "image", path);
    let committed_read = crate::loading::catalog_read_recorded(catalog.identity(), "image", RETAINED);
    let other_read = crate::loading::catalog_read_recorded(other_catalog.identity(), "image", path);
    let replacement = ui.acquire(catalog, path, &mut assets, None, None)?;
    let retry_read = crate::loading::catalog_read_recorded(catalog.identity(), "image", path);
    eprintln!("DISCARDED_IMAGE_READ_HISTORY recorded={recorded} retired={read_retired} committed={committed_read} other_catalog={other_read} reacquired={retry_read}");
    if !(recorded && read_retired && committed_read && other_read && retry_read) {
        return Err("discarded image did not retire only its own acquisition credit".into());
    }
    let replacement_id = replacement.owner::<Arc<ResidentImage>>().ok_or("missing replacement UI lease")?.handle.id();
    let matched = assets.get(replacement_id).and_then(|image| image.data.as_ref())
        .is_some_and(|bytes| blake3::hash(bytes) == expected);
    let preserved = registry.acquire(catalog, "_white", &mut assets)?.handle == published.handle;
    let invalidated = staged.discarded.load(Ordering::Relaxed) && !assets.contains(pending_id);
    let retried = replacement_id != pending_id && pending_id == staged.handle.id();
    eprintln!("DISCARDED_IMAGE_BATCH invalidated={invalidated} retried={retried} matched={matched} preserved={preserved} payload={expected}");
    if !(invalidated && retried && matched && preserved) {
        return Err("discarded image batch left a stale registration or changed published bytes".into());
    }
    observe_discarded_layered_batch(catalog, &registry, &mut assets)
}

fn observe_discarded_layered_batch(catalog: &archive::Catalog, registry: &ResidentImages,
    assets: &mut Assets<Image>) -> Result<(), String> {
    const DUST_CUBE: &str = "generated/image/models/campaign/dust/dust_oceancube.bimage7";
    const EOTW_CUBE: &str = "generated/image/models/campaign/eotw/eotw_oceancube.bimage7";
    const CAUSTICS: &str = "generated/image/rendering/water/caustics3.bimage7";
    let cube = |registry: &ResidentImages, path: &str| {
        registry.layered(catalog, path, true, || {
            let source = crate::material::load_cube_image(catalog, path)?;
            Ok(LayeredImageSource::cube(path.to_owned(), source.image))
        })
    };
    let array = |registry: &ResidentImages| {
        registry.layered(catalog, CAUSTICS, false, || {
            let bytes = catalog.read(CAUSTICS).map_err(|error| error.to_string())?
                .ok_or("missing installed caustics array")?;
            let chain = crate::texture::compressed_inline_array_owned(bytes)?;
            Ok(LayeredImageSource::array(CAUSTICS.to_owned(), chain))
        })
    };
    let abandoned = PreparedImages::new(assets, registry.clone(), SharedImages::default());
    let abandoned_cube = Arc::downgrade(&cube(abandoned.registry(), DUST_CUBE)?);
    let abandoned_array = Arc::downgrade(&array(abandoned.registry())?);
    let committed = PreparedImages::new(assets, registry.clone(), SharedImages::default());
    let committed_cube = cube(committed.registry(), EOTW_CUBE)?;
    committed.publish(assets);
    let staged = abandoned_cube.strong_count() == 1 && abandoned_array.strong_count() == 1;
    drop(abandoned);
    let retired = abandoned_cube.strong_count() == 0 && abandoned_array.strong_count() == 0;
    let preserved = Arc::ptr_eq(&committed_cube, &cube(registry, EOTW_CUBE)?);
    let next = PreparedImages::new(assets, registry.clone(), SharedImages::default());
    let next_cube = cube(next.registry(), DUST_CUBE)?;
    let next_array = array(next.registry())?;
    next.publish(assets);
    let retried = Arc::ptr_eq(&next_cube, &cube(registry, DUST_CUBE)?)
        && Arc::ptr_eq(&next_array, &array(registry)?);
    eprintln!("DISCARDED_LAYERED_BATCH staged={staged} retired={retired} preserved={preserved} retried={retried}");
    if !(staged && retired && preserved && retried) {
        return Err("layered image registration did not follow scene publication".into());
    }
    Ok(())
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
    fn live_binding(binding: &Weak<crate::flash::iggy_functions::TextureBinding>)
        -> Option<crate::flash::iggy_functions::BoundTexture> {
        binding.upgrade().filter(|binding| binding.owner::<Arc<ResidentImage>>()
            .is_some_and(|image| !image.discarded.load(Ordering::Relaxed)))
    }

    fn acquire(&mut self, catalog: &archive::Catalog, name: &str,
        images: &mut Assets<Image>, scene: Option<&SceneRenderQueue>, shared: Option<&SharedImages>) -> Result<crate::flash::iggy_functions::BoundTexture, String> {
        let path = crate::material::image_load_resource(catalog, name)
            .ok_or_else(|| format!("unresolved resident image source {name}"))?;
        let editable = catalog.image_has_overlay(&path);
        if editable {
            let mut registry = self.registry.registry.lock().unwrap();
            if let Some(generations) = registry.editable.get_mut(&path) {
                generations.retain(|(id, generation)| generation.strong_count() != 0 || images.contains(*id));
            }
        }
        // 14101d290 reuses the live named wrapper across Iggy contexts.
        // Only its first acquisition calls 1402df8c0 for a view snapshot.
        if !editable {
            if let Some(binding) = self.bindings.get(&path).and_then(Self::live_binding) {
                return Ok(binding);
            }
        }
        let current = if editable || separate_scene_views() { None } else {
            scene.and_then(|scene| {
                let slot = scene.diffuse_image_paths.iter().position(|source| source == &path)?;
                Some((scene.diffuse_image_handles[slot].clone()?, scene.diffuse_mip_states[slot].resident?))
            }).or_else(|| shared.and_then(|shared| shared.current_view(&path)))
        };
        let image = if let Some((handle, level)) = current {
            let source = images.get(&handle).ok_or_else(|| format!("committed UI image view unavailable: {path}"))?;
            self.registry.lease_scene_view(&path, &handle, level, source.texture_descriptor.mip_level_count)
        } else { self.registry.acquire(catalog, name, images)? };
        let binding = Arc::new(crate::flash::iggy_functions::TextureBinding::new(image.extent, image));
        self.bindings.insert(path, Arc::downgrade(&binding));
        Ok(binding)
    }
    pub(super) fn release_level(&mut self, shared: &super::texture_streaming::SharedImages) {
        self.registry.release_level(shared);
        self.bindings.retain(|_, binding| Self::live_binding(binding).is_some());
    }

    pub(super) fn release_read_history(&self, catalog: u64, shared: &SharedImages, assets: Option<&Assets<Image>>) {
        self.registry.release_read_history(catalog, shared, assets, Some(self));
    }
}

impl ResidentImages {
    /// Only identities that can actually be acquired from a surviving owner
    /// retain their loading credit; manifest membership by itself is not a
    /// resident image. Original 140060180 purges outgoing map resources.
    fn release_read_history(&self, catalog: u64, shared: &SharedImages,
        images: Option<&Assets<Image>>, named: Option<&UiImages>) {
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let mut registry = self.registry.lock().unwrap();
        if let Some(images) = images {
            registry.editable.retain(|_, generations| {
                generations.retain(|(id, generation)| generation.strong_count() != 0 || images.contains(*id));
                !generations.is_empty()
            });
        }
        registry.editable_layered.retain(|_, generations| {
            generations.retain(|generation| generation.strong_count() != 0);
            !generations.is_empty()
        });
        // Catalog read history normalizes case and separators. Most owner
        // keys are already canonical; only exceptional aliases need copies.
        let mut aliases = std::collections::HashSet::<String>::new();
        let paths = shared.resident_paths()
            .chain(registry.resident.keys().map(String::as_str))
            .chain(registry.inline.keys().map(String::as_str))
            .chain(registry.layered.keys().map(String::as_str))
            .chain(registry.editable.keys().map(String::as_str))
            .chain(registry.editable_layered.keys().map(String::as_str))
            .chain(registry.leases.iter().filter(|(_, value)| value.strong_count() != 0).map(|(path, _)| path.as_str()))
            .chain(registry.scene_leases.iter().filter(|(_, value)| value.strong_count() != 0).map(|(path, _)| path.as_str()))
            .chain(named.into_iter().flat_map(|ui| ui.bindings.iter())
                .filter(|(_, binding)| UiImages::live_binding(binding).is_some()).map(|(path, _)| path.as_str()));
        for path in paths {
            if path.bytes().any(|byte| byte == b'\\' || byte.is_ascii_uppercase()) {
                let mut key = path.replace('\\', "/");
                key.make_ascii_lowercase();
                aliases.insert(key);
            }
        }
        let (removed, bytes, slots_before, slots_after) = crate::loading::retain_catalog_kind(catalog, "image", |path| {
            (!aliases.is_empty() && aliases.contains(path))
                || shared.is_resident(path)
                || registry.resident.contains_key(path)
                || registry.inline.contains_key(path)
                || registry.layered.contains_key(path)
                || registry.editable.contains_key(path)
                || registry.editable_layered.contains_key(path)
                || registry.leases.get(path).is_some_and(|lease| lease.strong_count() != 0)
                || registry.scene_leases.get(path).is_some_and(|lease| lease.strong_count() != 0)
                || named.and_then(|ui| ui.bindings.get(path)).and_then(UiImages::live_binding).is_some()
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
    super::ui_lifecycle_diagnostics::frame(frame);
}

/// Synchronous Iggy callback, fulfilled on the main world that owns Assets.
/// The movie worker can wait without blocking the window/event loop.
pub(super) struct UiTextureRequest {
    pub name: String,
    pub reply: std::sync::mpsc::SyncSender<Result<crate::flash::iggy_functions::BoundTexture, String>>,
    pub diagnostic_id: u64,
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
            super::ui_lifecycle_diagnostics::reply(&request.name, &result);
            let sent = request.reply.send(result).is_ok();
            super::ui_lifecycle_diagnostics::reply_sent(request.diagnostic_id, sent);
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

/// The outgoing acquisition controller can already be borrowed by its
/// retirement caller. Use that same catalog lease for the ordinary UI RPC.
pub(super) fn service_ui_images_for_catalog(
    In(catalog): In<Arc<archive::Catalog>>,
    mut service: UiImageService,
    pause: NonSend<super::pause_menu::PauseMenu>,
) {
    service.service(&pause, &catalog);
}

fn separate_scene_views() -> bool {
    static SEPARATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_SCENE_IMAGE_VIEWS").as_deref() == Ok("1"))
}
