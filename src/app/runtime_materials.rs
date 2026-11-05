//! Level-owned material identities and shared bindings for runtime effects.
//!
//! The resolver itself borrows asset writers. Keep its existing cache and
//! immutable scene handles so background preparation can reconstruct it with
//! `AssetBatch` writers without creating another material identity owner.
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::asset::{Asset, AssetId};
use super::{material_loading::{MaterialCache, MaterialGeometry}, ImageAccess, MaterialResolver};
use crate::rendering::bevy_backend::fog_material::FogMaterialBindings;
use crate::rendering::bevy_backend::asset_preparation::{AssetBatch, AssetWrite};
use crate::rendering::particles::{definition::ParticleResources, image_resources::ParticleImage};
use bevy::platform::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

/// Temporary worker view of requested existing assets plus newly reserved
/// assets. Snapshots are never published back over the live asset identity.
pub(super) struct MaterialBatch<A: Asset> {
    pub pending: AssetBatch<A>,
    pub existing: HashMap<AssetId<A>, A>,
}
impl<A: Asset> AssetWrite<A> for MaterialBatch<A> {
    fn add(&mut self, value: A) -> Handle<A> { self.pending.add(value) }
    fn get(&self, handle: &Handle<A>) -> Option<&A> {
        self.pending.get(handle).or_else(|| self.existing.get(&handle.id()))
    }
    fn get_mut(&mut self, handle: &Handle<A>) -> Option<&mut A> {
        self.pending.get_mut(handle)
    }
}

#[derive(Clone)]
pub(super) struct MaterialBindings {
    pub fog: FogMaterialBindings,
    pub reflections: Option<Handle<Image>>,
    pub reflection_brdf_lookup: Option<Handle<Image>>,
    pub reflection_planes: Handle<ShaderBuffer>,
    pub material_exposure: Handle<ShaderBuffer>,
    pub anisotropic_sampler_image: Handle<Image>,
    pub linear_clamp_image: Handle<Image>,
    pub vista_sun: [Vec4; 2],
    pub probe_sh_buffer: Handle<ShaderBuffer>,
    pub vertex_probe_buffer: Handle<ShaderBuffer>,
    pub shadow_buffer: Handle<ShaderBuffer>,
    pub local_shadow_buffer: Handle<ShaderBuffer>,
    pub shadow_map: Handle<Image>,
    pub vegetation_slots: Handle<ShaderBuffer>,
    pub vegetation_instances: Handle<ShaderBuffer>,
    pub lit: bool,
}

impl MaterialBindings {
    pub fn capture(resolver: &MaterialResolver<'_>) -> Self {
        Self {
            fog: resolver.fog_material.clone(),
            reflections: resolver.reflections.clone(),
            reflection_brdf_lookup: resolver.reflection_brdf_lookup.clone(),
            reflection_planes: resolver.reflection_planes.clone(),
            material_exposure: resolver.material_exposure.clone(),
            anisotropic_sampler_image: resolver.anisotropic_sampler_image.clone(),
            linear_clamp_image: resolver.linear_clamp_image.clone(),
            vista_sun: resolver.vista_sun,
            probe_sh_buffer: resolver.probe_sh_buffer.clone(),
            vertex_probe_buffer: resolver.vertex_probe_buffer.clone(),
            shadow_buffer: resolver.shadow_buffer.clone(),
            local_shadow_buffer: resolver.local_shadow_buffer.clone(),
            shadow_map: resolver.shadow_map.clone(),
            vegetation_slots: resolver.vegetation.slots.clone(),
            vegetation_instances: resolver.vegetation.instances.clone(),
            lit: resolver.lit,
        }
    }
}

/// This is the startup resolver's actual cache, moved rather than cloned. It
/// is level-owned and will also serve on-demand world and decal preparation.
#[derive(Resource)]
pub(super) struct RuntimeMaterialOwner {
    pub bindings: MaterialBindings,
    pub cache: MaterialCache,
}

/// Copies only requested live asset values and their referenced image paths.
/// The startup cache remains the identity owner; worker-local indices never
/// replace the scene streamer's indices.
pub(super) struct MaterialSnapshot {
    pub cache: MaterialCache,
    pub image_slots: HashMap<String, usize>,
    pub image_paths: Vec<String>,
    pub probes: HashMap<AssetId<super::ProbeMaterial>, super::ProbeMaterial>,
    pub decals: HashMap<AssetId<super::DecalMaterial>, super::DecalMaterial>,
}

impl MaterialSnapshot {
    pub fn capture(
        owner: &RuntimeMaterialOwner,
        queue: &super::SceneRenderQueue,
        requests: &[(String, MaterialGeometry)],
        probes: &Assets<super::ProbeMaterial>,
        decals: &Assets<super::DecalMaterial>,
    ) -> Result<Self, String> {
        let mut snapshot = Self {
            cache: MaterialCache::default(), image_slots: HashMap::new(), image_paths: Vec::new(),
            probes: HashMap::new(), decals: HashMap::new(),
        };
        let mut remapped = HashMap::<usize, usize>::new();
        for (name, geometry) in requests {
            let Some(cached) = owner.cache.get_for(name, *geometry) else { continue; };
            let Some(mut cached) = cached.clone() else {
                snapshot.cache.insert(name.clone(), *geometry, None);
                continue;
            };
            let mut remap = |slot: usize| -> Result<usize, String> {
                if let Some(&local) = remapped.get(&slot) { return Ok(local); }
                let path = queue.diffuse_image_paths.get(slot).ok_or("cached material image slot is missing")?;
                let local = if let Some(&local) = snapshot.image_slots.get(path) { local } else {
                    let local = snapshot.image_paths.len();
                    snapshot.image_paths.push(path.clone());
                    snapshot.image_slots.insert(path.clone(), local);
                    local
                };
                remapped.insert(slot, local);
                Ok(local)
            };
            for (slot, _) in &mut cached.images { *slot = remap(*slot)?; }
            cached.diffuse_image_slot = cached.diffuse_image_slot.map(&mut remap).transpose()?;
            match &cached.handle {
                super::MaterialHandle::Probe(handle) => {
                    let asset = probes.get(handle).ok_or("cached probe material is unavailable")?;
                    snapshot.probes.entry(handle.id()).or_insert_with(|| asset.clone());
                }
                super::MaterialHandle::Decal(handle) => {
                    let asset = decals.get(handle).ok_or("cached decal material is unavailable")?;
                    snapshot.decals.entry(handle.id()).or_insert_with(|| asset.clone());
                }
                _ => {}
            }
            snapshot.cache.insert(name.clone(), *geometry, Some(cached));
        }
        Ok(snapshot)
    }
}

/// Asset IDs come from the live world's providers. The worker fills these
/// batches; publication transfers the payloads into those same owners.
pub(super) struct MaterialAssets {
    pub meshes: AssetBatch<Mesh>,
    pub probes: MaterialBatch<super::ProbeMaterial>,
    pub decals: MaterialBatch<super::DecalMaterial>,
    pub skies: AssetBatch<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
    pub buffers: AssetBatch<ShaderBuffer>,
}
impl MaterialAssets {
    pub fn new(world: &World, snapshot: &mut MaterialSnapshot) -> Self {
        Self {
            meshes: AssetBatch::new(world.resource::<Assets<Mesh>>()),
            probes: MaterialBatch {
                pending: AssetBatch::new(world.resource::<Assets<super::ProbeMaterial>>()),
                existing: std::mem::take(&mut snapshot.probes),
            },
            decals: MaterialBatch {
                pending: AssetBatch::new(world.resource::<Assets<super::DecalMaterial>>()),
                existing: std::mem::take(&mut snapshot.decals),
            },
            skies: AssetBatch::new(world.resource::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>()),
            buffers: AssetBatch::new(world.resource::<Assets<ShaderBuffer>>()),
        }
    }

    pub fn publish(self, world: &mut World) {
        self.meshes.publish(&mut world.resource_mut::<Assets<Mesh>>());
        self.buffers.publish(&mut world.resource_mut::<Assets<ShaderBuffer>>());
        self.probes.pending.publish(&mut world.resource_mut::<Assets<super::ProbeMaterial>>());
        self.decals.pending.publish(&mut world.resource_mut::<Assets<super::DecalMaterial>>());
        self.skies.publish(&mut world.resource_mut::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>());
    }
}

/// CPU results stay private until the current level accepts the batch. The
/// scene image owner and render-world readiness gate are separate consumers.
pub(super) struct PreparedBatch {
    pub assets: MaterialAssets,
    pub surfaces: Vec<super::particle_surface::PreparedSurface>,
    pub surface_images: HashMap<String, RuntimeImages>,
    pub decals: Vec<PreparedDecal>,
    pub cache: MaterialCache,
    pub image_paths: Vec<String>,
    pub all_images: Vec<Arc<ParticleImage>>,
    dependencies: HashMap<MaterialKey, HashSet<MaterialConsumer>>,
}

pub(super) struct MaterialWork {
    pub bindings: MaterialBindings,
    pub catalog: Arc<crate::archive::Catalog>,
    pub environment: Option<crate::entities::EnvironmentRenderParameters>,
    pub probes: Option<Arc<crate::probe_lighting::ProbeLightingSet>>,
    pub programs: Arc<Mutex<Result<crate::shader_cache::ProgramLibrary, String>>>,
    pub particles: Arc<Mutex<ParticleResources>>,
    pub sources: Arc<crate::rendering::particles::world_sources::WorldFxSources>,
    pub decal_requests: Vec<(String, bool)>,
    pub mesh_requests: HashSet<(String, String)>,
    pub snapshot: MaterialSnapshot,
    pub assets: MaterialAssets,
    pub cancelled: Arc<AtomicBool>,
    pub superseded: Arc<AtomicBool>,
}

impl MaterialWork {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.superseded.load(Ordering::Acquire)
    }

    pub fn prepare(mut self) -> Result<PreparedBatch, String> {
        if self.cancelled() { return Err("material request cancelled".into()); }
        // Match the existing FX worker's lock order; no gameplay system waits
        // for either owner while archive reads and material parsing run.
        let mut programs = self.programs.lock().unwrap_or_else(|error| error.into_inner());
        let mut particles = self.particles.lock().unwrap_or_else(|error| error.into_inner());
        if self.cancelled() { return Err("material request cancelled".into()); }
        self.snapshot.cache.enable_dependency_trace();
        let bindings = &self.bindings;
        let mut resolver = MaterialResolver {
            blended_materials: HashSet::new(),
            fog_material: bindings.fog.clone(),
            reflections: bindings.reflections.clone(),
            reflection_brdf_lookup: bindings.reflection_brdf_lookup.clone(),
            reflection_planes: bindings.reflection_planes.clone(),
            material_exposure: bindings.material_exposure.clone(),
            anisotropic_sampler_image: bindings.anisotropic_sampler_image.clone(),
            linear_clamp_image: bindings.linear_clamp_image.clone(),
            vista_sun: bindings.vista_sun,
            environment: self.environment.as_ref(),
            catalog: Some(&self.catalog),
            programs: Some(&mut *programs),
            probe_materials: &mut self.assets.probes,
            sky_materials: &mut self.assets.skies,
            decal_materials: &mut self.assets.decals,
            probe_sh_buffers: &mut self.assets.buffers,
            probe_sh_buffer: bindings.probe_sh_buffer.clone(),
            vertex_probe_buffer: bindings.vertex_probe_buffer.clone(),
            shadow_buffer: bindings.shadow_buffer.clone(),
            local_shadow_buffer: bindings.local_shadow_buffer.clone(),
            shadow_map: bindings.shadow_map.clone(),
            probe_sh_records: Vec::new(),
            probes: self.probes.as_deref(),
            cache: std::mem::take(&mut self.snapshot.cache),
            shadow_materials: Default::default(),
            vegetation: super::vegetation_wind::VegetationMaterials {
                slots: bindings.vegetation_slots.clone(),
                instances: bindings.vegetation_instances.clone(),
                parameters: HashMap::new(),
            },
            water_materials: Default::default(),
            water_model_colours: Default::default(),
            water_stencil_materials: Default::default(),
            boat_materials: Default::default(),
            diffuse_image_slots: std::mem::take(&mut self.snapshot.image_slots),
            diffuse_image_paths: std::mem::take(&mut self.snapshot.image_paths),
            misses: 0, suppressed_transparent: 0, cutout: 0, blended: 0,
            probe_samples: 0, probe_sample_misses: 0, lit: bindings.lit,
        };
        let world_meshes: Vec<_> = self.sources.world_meshes().into_iter()
            .filter(|(source, material, _)| self.mesh_requests.contains(&(source.clone(), material.clone())))
            .collect();
        let expected = world_meshes.len();
        let mut surfaces = Vec::new();
        let mut world_images = std::collections::HashMap::new();
        let mut dynamic_materials = HashSet::new();
        let mut dependencies: HashMap<MaterialKey, HashSet<MaterialConsumer>> = HashMap::new();
        for (source, material, model) in world_meshes {
            let consumer = MaterialConsumer::Mesh((source.clone(), material.clone()));
            resolver.cache.take_dependencies();
            let failures = super::setup::prepare_particle_surfaces(vec![(source, material, model)], &mut resolver,
                &mut self.assets.meshes, &mut particles, &mut world_images, &mut dynamic_materials,
                |surface| surfaces.push(surface));
            if !failures.is_empty() {
                return Err(format!("runtime world particle preparation failed: {}", failures.join("; ")));
            }
            for key in resolver.cache.take_dependencies() { dependencies.entry(key).or_default().insert(consumer.clone()); }
        }
        if surfaces.len() != expected {
            return Err(format!("runtime world particle surfaces prepared {} of {expected}", surfaces.len()));
        }
        let mut surface_images = HashMap::new();
        for surface in &surfaces {
            if surface_images.contains_key(&surface.material) { continue; }
            let identities = world_images.get(&surface.material);
            let images = if surface.images.is_empty() { Vec::new() } else {
                let identities = identities.ok_or("runtime world particle images are missing")?;
                if identities.len() != surface.images.len() { return Err("runtime world particle image roles differ".into()); }
                surface.images.iter().zip(identities).map(|((_, role), image)| (*role, image.clone())).collect()
            };
            surface_images.insert(surface.material.clone(), images);
        }
        let mut decals = Vec::with_capacity(self.decal_requests.len());
        for (name, world) in &self.decal_requests {
            resolver.cache.take_dependencies();
            decals.push(prepare_decal(&mut resolver, &mut particles, name, *world)?);
            let consumer = MaterialConsumer::Decal((name.clone(), *world));
            for key in resolver.cache.take_dependencies() { dependencies.entry(key).or_default().insert(consumer.clone()); }
        }
        if self.cancelled.load(Ordering::Acquire) || self.superseded.load(Ordering::Acquire) {
            return Err("material request cancelled".into());
        }
        resolver.cache.disable_dependency_trace();
        let cache = std::mem::take(&mut resolver.cache);
        let image_paths = std::mem::take(&mut resolver.diffuse_image_paths);
        drop(resolver);
        let mut all_images = Vec::with_capacity(image_paths.len());
        for (slot, path) in image_paths.iter().enumerate() {
            let image = particles.scene_image(&self.catalog, path)?
                .ok_or("runtime material image requires the shared scene owner")?;
            image.sampling_priority.fetch_max(cache.image_priority(slot), Ordering::Relaxed);
            all_images.push(image);
        }
        drop(particles);
        drop(programs);
        Ok(PreparedBatch { assets: self.assets, surfaces, surface_images, decals, cache, image_paths, all_images, dependencies })
    }
}

/// The same ordered roles used by startup, with image identities leased by
/// the particle owner until the runtime proxy and its tails are gone.
pub(super) type RuntimeImages = Vec<(crate::surface_gpu::ImageRole, Arc<ParticleImage>)>;

pub(super) fn acquire_images(
    resolver: &MaterialResolver<'_>,
    resources: &mut ParticleResources,
    images: &[(usize, crate::surface_gpu::ImageRole)],
) -> Result<RuntimeImages, String> {
    let catalog = resolver.catalog.ok_or("runtime material has no catalog")?;
    images.iter().map(|&(slot, role)| {
        let name = resolver.diffuse_image_paths.get(slot).ok_or("runtime material image slot is missing")?;
        let image = resources.scene_image(catalog, name)?
            .ok_or("runtime material requires the shared scene image owner")?;
        Ok((role, image))
    }).collect()
}

#[derive(Clone)]
pub(super) struct PreparedDecal {
    pub name: String,
    pub world: bool,
    pub handle: Handle<super::DecalMaterial>,
    pub program: Arc<crate::rendering::decals::material::DecalProgram>,
    pub images: RuntimeImages,
}

/// Reuse the ordinary resolver's decal specialization and asset writer. The
/// caller publishes its reserved assets only after background work succeeds.
pub(super) fn prepare_decal(
    resolver: &mut MaterialResolver<'_>,
    resources: &mut ParticleResources,
    name: &str,
    world: bool,
) -> Result<PreparedDecal, String> {
    let (binding, images) = resolver.projected_decal_material(name, world)?;
    let super::MaterialHandle::Decal(handle) = binding else {
        return Err(format!("runtime projected material is not a decal: {name}"));
    };
    let program = resolver.decal_materials.get(&handle)
        .and_then(|material| material.extension.projection.clone())
        .ok_or("runtime decal projection controls are missing")?;
    let images = acquire_images(resolver, resources, &images)?;
    Ok(PreparedDecal { name: name.to_owned(), world, handle, program, images })
}


type FxSources = crate::rendering::particles::world_sources::WorldFxSources;
type SourceLease = std::sync::Weak<FxSources>;
type MeshKey = (String, String);
type DecalKey = (String, bool);
type MaterialKey = (String, MaterialGeometry);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MaterialConsumer { Mesh(MeshKey), Decal(DecalKey) }

struct SurfaceLease {
    entity: Entity,
    handle: super::MaterialHandle,
    images: RuntimeImages,
}
struct CacheLease {
    handle: super::MaterialHandle,
    images: Vec<Arc<ParticleImage>>,
    leases: Vec<SourceLease>,
    consumers: HashSet<MaterialConsumer>,
}
struct PendingMaterialWork {
    generation: u64,
    superseded: Arc<AtomicBool>,
    task: Mutex<bevy::tasks::Task<Result<PreparedBatch, String>>>,
}
struct WaitingMaterialBatch {
    generation: u64,
    batch: PreparedBatch,
}

#[derive(Resource, Default)]
pub(super) struct RuntimeMaterialCoordinator {
    revision: Option<u64>,
    seen_retired: u64,
    pending_prune: bool,
    leases: Vec<SourceLease>,
    static_meshes: HashMap<MeshKey, Vec<Entity>>,
    static_decals: HashSet<DecalKey>,
    initialized: bool,
    surfaces: HashMap<MeshKey, SurfaceLease>,
    decals: HashMap<DecalKey, PreparedDecal>,
    cache: HashMap<MaterialKey, CacheLease>,
    owned_bindings: HashSet<super::MaterialHandle>,
    task: Option<PendingMaterialWork>,
    waiting: Option<WaitingMaterialBatch>,
    published_generation: Option<u64>,
}
impl Drop for RuntimeMaterialCoordinator {
    fn drop(&mut self) {
        if let Some(task) = &self.task { task.superseded.store(true, Ordering::Release); }
    }
}

fn add_lease(leases: &mut Vec<SourceLease>, lease: &SourceLease) {
    if !leases.iter().any(|old| std::sync::Weak::ptr_eq(old, lease)) { leases.push(lease.clone()); }
}

fn material_bind(world: &mut World, state: &mut RuntimeMaterialCoordinator,
    handle: super::MaterialHandle, images: &[(usize, crate::surface_gpu::ImageRole)],
    sort: Option<Result<[u32; 2], String>>) -> Result<(), String> {
    let resident: Vec<_> = {
        let queue = world.resource::<super::SceneRenderQueue>();
        images.iter().map(|&(slot, role)| queue.diffuse_image_handles.get(slot)
            .and_then(Option::as_ref).cloned().map(|image| (slot, role, image))
            .ok_or_else(|| format!("runtime material image slot {slot} has no resident handle")))
            .collect::<Result<_, _>>()?
    };
    {
        let mut queue = world.resource_mut::<super::SceneRenderQueue>();
        if !queue.material_images.contains_key(&handle) { state.owned_bindings.insert(handle.clone()); }
        queue.material_images.entry(handle.clone()).or_insert_with(|| images.iter().map(|(slot, _)| *slot).collect());
        if let Some(sort) = sort { queue.material_stage_sorts.entry(handle.clone()).or_insert(sort); }
        for (slot, role, _) in &resident {
            let binding = (handle.clone(), *role);
            if !queue.diffuse_image_materials[*slot].contains(&binding) { queue.diffuse_image_materials[*slot].push(binding); }
        }
    }
    match &handle {
        super::MaterialHandle::Probe(handle) => {
            let mut assets = world.resource_mut::<Assets<super::ProbeMaterial>>();
            let Some(mut material) = assets.get_mut(handle) else { return Err("runtime probe material asset is absent".into()) };
            for (_, role, image) in &resident {
                if *role == crate::surface_gpu::ImageRole::occlusion() { material.base.occlusion_texture = Some(image.clone()); }
                else {
                    *material.extension.image_mut(*role) = Some(image.clone());
                    if role.0 == 0 { material.base.base_color_texture = Some(image.clone()); }
                }
            }
        }
        super::MaterialHandle::Decal(handle) => {
            let mut assets = world.resource_mut::<Assets<super::DecalMaterial>>();
            let Some(mut material) = assets.get_mut(handle) else { return Err("runtime decal material asset is absent".into()) };
            for (_, role, image) in &resident {
                if *role == crate::surface_gpu::ImageRole::occlusion() { material.base.occlusion_texture = Some(image.clone()); }
                else {
                    *material.extension.image_mut(*role) = Some(image.clone());
                    if role.0 == 0 { material.base.base_color_texture = Some(image.clone()); }
                }
            }
        }
        _ => return Err("runtime particle material has an unsupported binding type".into()),
    }
    Ok(())
}

fn refresh_images(world: &mut World, state: &RuntimeMaterialCoordinator) {
    let mut surfaces = std::collections::HashMap::new();
    let mut images = Vec::new();
    for ((_, material), surface) in &state.surfaces {
        surfaces.entry(material.clone()).or_insert_with(|| surface.images.clone());
        images.extend(surface.images.iter().map(|(_, image)| image.clone()));
    }
    for decal in state.decals.values() { images.extend(decal.images.iter().map(|(_, image)| image.clone())); }
    for cached in state.cache.values() { images.extend(cached.images.iter().cloned()); }
    if let Some(waiting) = &state.waiting { images.extend(waiting.batch.all_images.iter().cloned()); }
    let mut seen = HashSet::new();
    images.retain(|image| seen.insert(image.path.clone()));
    let mut request = world.resource_mut::<super::usable_effects::DynamicWorldMaterials>();
    request.images = surfaces;
    request.extra_images = images;
    request.image_revision = request.image_revision.wrapping_add(1);
}

fn release_bindings(world: &mut World, state: &mut RuntimeMaterialCoordinator) {
    let mut used = HashSet::new();
    used.extend(state.surfaces.values().map(|item| item.handle.clone()));
    used.extend(state.decals.values().map(|item| super::MaterialHandle::Decal(item.handle.clone())));
    used.extend(state.cache.values().map(|item| item.handle.clone()));
    let retired: Vec<_> = state.owned_bindings.iter().filter(|handle| !used.contains(*handle)).cloned().collect();
    if retired.is_empty() { return; }
    let mut queue = world.resource_mut::<super::SceneRenderQueue>();
    for handle in retired {
        if let Some(slots) = queue.material_images.remove(&handle) {
            for slot in slots { queue.diffuse_image_materials[slot].retain(|(candidate, _)| candidate != &handle); }
        }
        queue.material_stage_sorts.remove(&handle);
        state.owned_bindings.remove(&handle);
    }
}


/// Advance a single level-owned material preparation batch. All archive and
/// program decoding is on IO workers; only asset/slot publication runs here.
pub(super) fn coordinate(world: &mut World) {
    let Some(mut state) = world.remove_resource::<RuntimeMaterialCoordinator>() else { return };
    let result = coordinate_inner(world, &mut state);
    if let Err(error) = result {
        let generation = world.get_resource::<super::usable_effects::DynamicWorldMaterials>().map_or(0, |request| request.generation);
        if generation != 0 {
            bevy::log::warn!("Runtime FX material preparation failed: {error}");
            if let Some(request) = world.get_resource::<super::usable_effects::DynamicWorldMaterials>() { request.failed.store(generation, Ordering::Release); }
            if let Some(request) = world.get_resource::<super::usable_effects::DynamicDecalMaterials>() { request.failed.store(generation, Ordering::Release); }
        }
        if let Some(task) = state.task.take() { task.superseded.store(true, Ordering::Release); }
        state.waiting = None;
        refresh_images(world, &state);
        state.published_generation = Some(generation);
    }
    world.insert_resource(state);
}

fn coordinate_inner(world: &mut World, state: &mut RuntimeMaterialCoordinator) -> Result<(), String> {
    use crate::rendering::bevy_backend::{particle_meshes::ParticleMeshProxy, projected_decals::{ProjectedDecalProxy, DynamicProxyReadiness, ProxyReadinessBatch}, world_effects::DynamicMaterialPrewarm};
    let Some(request) = world.get_resource::<super::usable_effects::DynamicWorldMaterials>() else { return Ok(()) };
    let generation = request.generation;
    let Some(decal_request) = world.get_resource::<super::usable_effects::DynamicDecalMaterials>() else { return Ok(()) };
    if generation != decal_request.generation { return Ok(()); }
    let retired = world.resource::<DynamicMaterialPrewarm>().retired_signal.load(Ordering::Acquire);
    let expired = state.pending_prune && state.leases.iter().any(|lease| lease.strong_count() == 0);
    if state.initialized && state.revision == Some(generation) && state.seen_retired == retired
        && !expired && state.task.is_none() && state.waiting.is_none() && state.published_generation == Some(generation) {
        if request.images_ready == request.image_revision {
            if request.gpu_completed.load(Ordering::Acquire) == generation { request.completed.store(generation, Ordering::Release); }
            if decal_request.gpu_completed.load(Ordering::Acquire) == generation { decal_request.completed.store(generation, Ordering::Release); }
        }
        return Ok(());
    }
    let leases = request.leases.clone();
    let world_gpu = request.gpu_completed.clone();
    let world_completed = request.completed.clone();
    let world_failed = request.failed.clone();
    let decal_gpu = decal_request.gpu_completed.clone();
    let decal_completed = decal_request.completed.clone();
    if !state.initialized {
        let mut query = world.query::<(Entity, &ParticleMeshProxy)>();
        for (entity, proxy) in query.iter(world) {
            state.static_meshes.entry((proxy.source.clone(), proxy.material.clone())).or_default().push(entity);
        }
        let mut query = world.query::<&ProjectedDecalProxy>();
        state.static_decals.extend(query.iter(world).map(|proxy| (proxy.material.clone(), proxy.world)));
        state.initialized = true;
    }
    let retirement_changed = state.seen_retired != retired;
    if retirement_changed { state.pending_prune = true; }
    let expired = state.pending_prune && state.leases.iter().any(|lease| lease.strong_count() == 0);
    let changed = state.revision != Some(generation) || retirement_changed || expired;
    if changed {
        if let Some(task) = state.task.take() { task.superseded.store(true, Ordering::Release); }
        state.waiting = None;
        state.published_generation = None;
        for lease in leases.iter() { add_lease(&mut state.leases, lease); }
        state.leases.retain(|lease| lease.strong_count() != 0);
        state.pending_prune = state.leases.iter().any(|old| !leases.iter().any(|active| std::sync::Weak::ptr_eq(old, active)));
        let mut mesh_leases: HashMap<MeshKey, Vec<SourceLease>> = HashMap::new();
        let mut decal_leases: HashMap<DecalKey, Vec<SourceLease>> = HashMap::new();
        for lease in &state.leases {
            let Some(source) = lease.upgrade() else { continue };
            for (mesh, material, _) in source.world_meshes() { add_lease(mesh_leases.entry((mesh, material)).or_default(), lease); }
            for key in source.decal_materials() { add_lease(decal_leases.entry(key).or_default(), lease); }
        }
        let obsolete: Vec<_> = state.surfaces.keys().filter(|key| !mesh_leases.contains_key(*key)).cloned().collect();
        for key in obsolete {
            if let Some(surface) = state.surfaces.remove(&key) {
                if crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
                    eprintln!("FX_LIFECYCLE material_proxy_retire={} material={}", key.0, key.1);
                }
                world.despawn(surface.entity);
            }
        }
        state.decals.retain(|key, _| decal_leases.contains_key(key));
        for cached in state.cache.values_mut() {
            cached.leases.retain(|lease| lease.strong_count() != 0);
            for consumer in &cached.consumers {
                let owners = match consumer { MaterialConsumer::Mesh(key) => mesh_leases.get(key),
                    MaterialConsumer::Decal(key) => decal_leases.get(key) };
                if let Some(owners) = owners { for lease in owners { add_lease(&mut cached.leases, lease); } }
            }
        }
        let old_cache: Vec<_> = state.cache.iter().filter(|(_, cached)| cached.leases.is_empty()).map(|(key, _)| key.clone()).collect();
        for (name, geometry) in old_cache {
            world.resource_mut::<RuntimeMaterialOwner>().cache.remove_dynamic(&name, geometry);
            state.cache.remove(&(name, geometry));
        }
        release_bindings(world, state);
        refresh_images(world, state);
        state.revision = Some(generation);
        state.seen_retired = retired;
    }
    if let Some(work) = &mut state.task {
        let task = work.task.get_mut().unwrap_or_else(|error| error.into_inner());
        if let Some(result) = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)) {
            let task_generation = work.generation;
            state.task = None;
            let batch = result?;
            if task_generation == generation {
                state.waiting = Some(WaitingMaterialBatch { generation, batch });
                refresh_images(world, state);
            }
        }
    }
    if state.waiting.is_some() {
        let request = world.resource::<super::usable_effects::DynamicWorldMaterials>();
        if request.images_ready == request.image_revision {
            let mapping = {
                let images = world.resource::<super::runtime_fx_images::RuntimeFxImages>();
                state.waiting.as_ref().unwrap().batch.image_paths.iter()
                    .map(|path| images.slot(path).ok_or_else(|| format!("runtime image {path} has no registered scene slot")))
                    .collect::<Result<Vec<_>, _>>()?
            };
            let waiting = state.waiting.take().unwrap();
            if waiting.generation == generation { publish_batch(world, state, waiting.batch, &mapping)?; }
            refresh_images(world, state);
            state.published_generation = None;
        }
    }
    if state.task.is_some() || state.waiting.is_some() { return Ok(()); }
    if state.published_generation == Some(generation) && !changed {
        let images = world.resource::<super::usable_effects::DynamicWorldMaterials>();
        if images.images_ready == images.image_revision {
            if world_gpu.load(Ordering::Acquire) == generation { world_completed.store(generation, Ordering::Release); }
            if decal_gpu.load(Ordering::Acquire) == generation { decal_completed.store(generation, Ordering::Release); }
        }
        return Ok(());
    }
    let live: Vec<_> = state.leases.iter().filter_map(SourceLease::upgrade).collect();
    let sources = Arc::new(FxSources::with_additions(&FxSources::default(), live.iter().map(Arc::as_ref)));
    let meshes: HashSet<_> = sources.world_meshes().into_iter().map(|(source, material, _)| (source, material))
        .filter(|key| !state.static_meshes.contains_key(key) && !state.surfaces.contains_key(key)).collect();
    let decals: Vec<_> = sources.decal_materials().into_iter()
        .filter(|key| !state.static_decals.contains(key) && !state.decals.contains_key(key)).collect();
    if !meshes.is_empty() || !decals.is_empty() {
        let mut requests: Vec<_> = meshes.iter().map(|(_, material)| (material.clone(), MaterialGeometry::Model)).collect();
        requests.extend(decals.iter().map(|(name, world)| (name.clone(), if *world { MaterialGeometry::World } else { MaterialGeometry::Model })));
        let mut snapshot = MaterialSnapshot::capture(world.resource::<RuntimeMaterialOwner>(), world.resource::<super::SceneRenderQueue>(),
            &requests, world.resource::<Assets<super::ProbeMaterial>>(), world.resource::<Assets<super::DecalMaterial>>())?;
        let assets = MaterialAssets::new(world, &mut snapshot);
        let scene = world.resource::<super::SceneData>();
        let catalog = scene.catalog.clone().ok_or("runtime material catalog is absent")?;
        let environment = scene.environment_render_parameters.clone();
        let probes = scene.environment_probes.clone();
        let programs = world.resource::<super::ProgramCacheSource>().0.clone();
        let particles = world.resource::<super::usable_effects::ParticleResourceOwner>().0.clone();
        let cancelled = world.resource::<super::usable_effects::UsableEffects>().cancellation_token();
        let superseded = Arc::new(AtomicBool::new(false));
        let work = MaterialWork { bindings: world.resource::<RuntimeMaterialOwner>().bindings.clone(), catalog, environment,
            probes, programs, particles, sources, decal_requests: decals, mesh_requests: meshes, snapshot, assets,
            cancelled, superseded: superseded.clone() };
        let started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
        state.task = Some(PendingMaterialWork { generation, superseded, task: Mutex::new(
            bevy::tasks::IoTaskPool::get().spawn(async move {
                let result = work.prepare();
                if let Some(started) = started {
                    let (surfaces, decals, image_bytes) = result.as_ref().map_or((0, 0, 0), |batch|
                        (batch.surfaces.len(), batch.decals.len(), batch.all_images.iter().map(|image| image.payload_bytes).sum::<usize>()));
                    eprintln!("FX_LIFECYCLE material_prepare generation={generation} ok={} surfaces={surfaces} decals={decals} image_source_bytes={image_bytes} elapsed_ms={:.3}",
                        result.is_ok(), started.elapsed().as_secs_f64() * 1000.0);
                }
                result
            })) });
        return Ok(());
    }
    if state.published_generation != Some(generation) || changed {
        let desired: HashSet<_> = sources.world_meshes().into_iter().map(|(source, material, _)| (source, material)).collect();
        let mut proxies = Vec::new();
        for key in desired {
            if let Some(entities) = state.static_meshes.get(&key) { proxies.extend(entities.iter().copied()); }
            if let Some(surface) = state.surfaces.get(&key) { proxies.push(surface.entity); }
        }
        let mut ready = world.resource_mut::<DynamicProxyReadiness>();
        ready.world_meshes = ProxyReadinessBatch { generation, proxies: Arc::new(proxies), completed: world_gpu.clone(), failed: world_failed.clone() };
        drop(ready);
        let mut decal = world.resource_mut::<super::usable_effects::DynamicDecalMaterials>();
        decal.prepared = Arc::new(state.decals.values().cloned().collect());
        decal.prepared_generation = generation;
        decal.prepared_revision = decal.prepared_revision.wrapping_add(1);
        state.published_generation = Some(generation);
    }
    let image_ready = {
        let request = world.resource::<super::usable_effects::DynamicWorldMaterials>();
        request.images_ready == request.image_revision
    };
    if image_ready && world_gpu.load(Ordering::Acquire) == generation {
        world_completed.store(generation, Ordering::Release);
    }
    if image_ready && decal_gpu.load(Ordering::Acquire) == generation {
        decal_completed.store(generation, Ordering::Release);
    }
    Ok(())
}


fn publish_batch(world: &mut World, state: &mut RuntimeMaterialCoordinator,
    batch: PreparedBatch, slots: &[usize]) -> Result<(), String> {
    let old_surfaces: HashSet<_> = state.surfaces.keys().cloned().collect();
    let old_decals: HashSet<_> = state.decals.keys().cloned().collect();
    let old_cache: HashSet<_> = state.cache.keys().cloned().collect();
    let old_leases: Vec<_> = state.cache.iter().map(|(key, value)| (key.clone(), value.leases.clone(), value.consumers.clone())).collect();
    let mesh_ids = batch.assets.meshes.ids(|_| true);
    let probe_ids = batch.assets.probes.pending.ids(|_| true);
    let decal_ids = batch.assets.decals.pending.ids(|_| true);
    let sky_ids = batch.assets.skies.ids(|_| true);
    let buffer_ids = batch.assets.buffers.ids(|_| true);
    let mut inserted = Vec::new();
    let result = publish_batch_inner(world, state, batch, slots, &mut inserted);
    if result.is_err() {
        let surfaces: Vec<_> = state.surfaces.keys().filter(|key| !old_surfaces.contains(*key)).cloned().collect();
        for key in surfaces { if let Some(surface) = state.surfaces.remove(&key) { world.despawn(surface.entity); } }
        state.decals.retain(|key, _| old_decals.contains(key));
        state.cache.retain(|key, _| old_cache.contains(key));
        for (key, leases, consumers) in old_leases {
            if let Some(cached) = state.cache.get_mut(&key) { cached.leases = leases; cached.consumers = consumers; }
        }
        for (name, geometry) in inserted { world.resource_mut::<RuntimeMaterialOwner>().cache.remove_dynamic(&name, geometry); }
        release_bindings(world, state);
        for id in mesh_ids { world.resource_mut::<Assets<Mesh>>().remove(id); }
        for id in probe_ids { world.resource_mut::<Assets<super::ProbeMaterial>>().remove(id); }
        for id in decal_ids { world.resource_mut::<Assets<super::DecalMaterial>>().remove(id); }
        for id in sky_ids { world.resource_mut::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>().remove(id); }
        for id in buffer_ids { world.resource_mut::<Assets<ShaderBuffer>>().remove(id); }
    }
    result
}

fn publish_batch_inner(world: &mut World, state: &mut RuntimeMaterialCoordinator,
    batch: PreparedBatch, slots: &[usize], inserted: &mut Vec<MaterialKey>) -> Result<(), String> {
    let PreparedBatch { assets, surfaces, surface_images, decals, cache, image_paths: _, all_images, dependencies } = batch;
    if all_images.len() != slots.len() { return Err("runtime material slot mapping has the wrong length".into()); }
    let images_by_slot: HashMap<_, _> = slots.iter().copied().zip(all_images.iter().cloned()).collect();
    let lease_for = |consumer: &MaterialConsumer, source: &FxSources| match consumer {
        MaterialConsumer::Mesh(key) => source.world_meshes().iter().any(|(mesh, material, _)| mesh == &key.0 && material == &key.1),
        MaterialConsumer::Decal(key) => source.decal_materials().contains(key),
    };
    for (key, consumers) in &dependencies {
        if let Some(cached) = state.cache.get_mut(key) {
            cached.consumers.extend(consumers.iter().cloned());
            for lease in &state.leases {
                if let Some(source) = lease.upgrade() {
                    if consumers.iter().any(|consumer| lease_for(consumer, &source)) { add_lease(&mut cached.leases, lease); }
                }
            }
        }
    }
    let mapped = world.resource_mut::<RuntimeMaterialOwner>().cache.merge_dynamic(cache, slots)?;
    inserted.extend(mapped.iter().map(|(name, geometry, _)| (name.clone(), *geometry)));
    // Reserved identities become live before any proxy entity is spawned.
    assets.publish(world);
    for (name, geometry, material) in mapped {
        material_bind(world, state, material.handle.clone(), &material.images, Some(material.stage_sort.clone()))?;
        let images = material.images.iter().filter_map(|(slot, _)| images_by_slot.get(slot).cloned()).collect();
        let key = (name, geometry);
        let consumers = dependencies.get(&key).ok_or("runtime cached material has no traced request owner")?;
        let mut leases = Vec::new();
        for lease in &state.leases {
            if let Some(source) = lease.upgrade() {
                if consumers.iter().any(|consumer| lease_for(consumer, &source)) { add_lease(&mut leases, lease); }
            }
        }
        state.cache.insert(key, CacheLease { handle: material.handle, images, leases, consumers: consumers.clone() });
    }
    for surface in surfaces {
        let key = (surface.source.clone(), surface.material.clone());
        let images = surface_images.get(&surface.material).cloned().unwrap_or_default();
        let roles = surface.images.iter().map(|(slot, role)| slots.get(*slot).copied()
            .map(|slot| (slot, *role)).ok_or("runtime surface image mapping is incomplete"))
            .collect::<Result<Vec<_>, _>>()?;
        let handle = super::MaterialHandle::Probe(surface.handle.clone());
        material_bind(world, state, handle.clone(), &roles, None)?;
        let entity = surface.publish(world);
        state.surfaces.insert(key, SurfaceLease { entity, handle, images });
    }
    for decal in decals {
        let roles = {
            let image_owner = world.resource::<super::runtime_fx_images::RuntimeFxImages>();
            decal.images.iter().map(|(role, image)| image_owner.slot(&image.path)
                .map(|slot| (slot, *role)).ok_or_else(|| format!("runtime decal image {} is not registered", image.path)))
                .collect::<Result<Vec<_>, _>>()?
        };
        material_bind(world, state, super::MaterialHandle::Decal(decal.handle.clone()), &roles, None)?;
        state.decals.insert((decal.name.clone(), decal.world), decal);
    }
    Ok(())
}
