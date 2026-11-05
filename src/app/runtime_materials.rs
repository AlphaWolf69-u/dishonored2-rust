//! Level-owned material identities and shared bindings for runtime effects.
//!
//! The resolver itself borrows asset writers. Keep its existing cache and
//! immutable scene handles so background preparation can reconstruct it with
//! `AssetBatch` writers without creating another material identity owner.
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::asset::{Asset, AssetId};
use super::{material_loading::{MaterialCache, MaterialGeometry}, MaterialResolver};
use crate::rendering::bevy_backend::fog_material::FogMaterialBindings;
use crate::rendering::bevy_backend::asset_preparation::{AssetBatch, AssetWrite};
use crate::rendering::particles::{definition::ParticleResources, image_resources::ParticleImage};
use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};

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
        let world_meshes = self.sources.world_meshes();
        let expected = world_meshes.len();
        let mut surfaces = Vec::new();
        let mut world_images = HashMap::new();
        let mut dynamic_materials = HashSet::new();
        let failures = super::setup::prepare_particle_surfaces(world_meshes, &mut resolver, &mut self.assets.meshes,
            &mut particles, &mut world_images, &mut dynamic_materials, |surface| surfaces.push(surface));
        if !failures.is_empty() {
            return Err(format!("runtime world particle preparation failed: {}", failures.join("; ")));
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
            decals.push(prepare_decal(&mut resolver, &mut particles, name, *world)?);
        }
        if self.cancelled.load(Ordering::Acquire) || self.superseded.load(Ordering::Acquire) {
            return Err("material request cancelled".into());
        }
        let cache = std::mem::take(&mut resolver.cache);
        let image_paths = std::mem::take(&mut resolver.diffuse_image_paths);
        drop(resolver);
        drop(particles);
        drop(programs);
        Ok(PreparedBatch { assets: self.assets, surfaces, surface_images, decals, cache, image_paths })
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
