//! bevy_backend: device-local residency for immutable common BML1 meshes.
//! Resource lookup/promotion1400607f0/140060b60 and surface release14026a240
//! retain the common identity. Host allocation domains keep map buffers separate.
use bevy::{asset::AssetId, prelude::*, render::{extract_resource::ExtractResource, mesh::allocator::{MeshAllocator, allocate_and_free_meshes}, Render, RenderApp, RenderSystems}};
use std::{collections::HashMap, sync::{Arc, Mutex}};
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key { path: String, surface: usize, flipped: bool, vegetation: bool }

pub(super) struct CachedMesh {
    pub handle: Handle<Mesh>,
    pub vegetation: Option<Result<usize, String>>,
    // Only the opt-in mesh-content observer needs a CPU copy after upload.
    debug_mesh: Option<Mesh>,
}
#[derive(Default)]
struct State {
    catalog: Option<u64>,
    builds: usize,
    meshes: HashMap<Key, Arc<CachedMesh>>,
}
#[derive(Resource, Clone, Default)]
pub(super) struct Shared(Arc<Mutex<State>>);
pub(super) struct Retirement(std::sync::Weak<Mutex<State>>);
impl Retirement {
    pub fn released(&self) -> bool { self.0.strong_count() == 0 }
}
impl Shared {
    pub fn retirement(&self) -> Retirement { Retirement(Arc::downgrade(&self.0)) }
    pub fn counts(&self) -> (usize, usize) {
        let shared = self.0.lock().unwrap_or_else(|error| error.into_inner());
        (shared.meshes.len(), shared.builds)
    }
    pub fn ids(&self) -> Vec<AssetId<Mesh>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
            .meshes.values().map(|mesh| mesh.handle.id()).collect()
    }
}
#[derive(Resource, Clone, Default, ExtractResource)]
struct MeshIds(Arc<Vec<AssetId<Mesh>>>);

pub(super) fn install(app: &mut App) {
    app.init_resource::<Shared>().init_resource::<MeshIds>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<MeshIds>::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        fn classify(ids: Res<MeshIds>, mut allocator: ResMut<MeshAllocator>) {
            allocator.set_long_lived_meshes(&ids.0);
        }
        render.add_systems(Render, classify.in_set(RenderSystems::PrepareAssets)
            .before(allocate_and_free_meshes)
            .run_if(bevy::ecs::schedule::common_conditions::resource_changed::<MeshIds>));
    }
}

pub(super) struct Prepared {
    shared: Shared,
    catalog: Option<u64>,
    enabled: bool,
    reuse: bool,
    trace: bool,
    debug_meshes: bool,
    used: HashMap<Key, Arc<CachedMesh>>,
    fresh: usize,
    reused: usize,
    cycles: u64,
}
impl Prepared {
    pub fn new(shared: Shared) -> Self {
        Self { shared, catalog: None, enabled: false,
            reuse: std::env::var_os("DH2_DEBUG_SEPARATE_COMMON_MESHES").is_none_or(|v| v != "1"),
            trace: std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1"),
            debug_meshes: std::env::var_os("DH2_DEBUG_MODEL_MESH_PREPARATION").is_some_and(|v| v == "1"),
            used: HashMap::new(), fresh: 0, reused: 0, cycles: 0 }
    }
    pub fn begin(&mut self, catalog: Option<&crate::archive::Catalog>) {
        self.catalog = catalog.map(|catalog| catalog.identity());
        self.enabled = catalog.is_some_and(|catalog| !catalog.has_overlays())
            && std::env::var_os("DH2_DEBUG_REPEAT_MODEL_MESH_PREPARATION").is_none_or(|v| v != "1");
    }
    pub fn enabled(&self) -> bool { self.enabled }
    pub fn acquire(&mut self, path: &str, surface: usize, flipped: bool, vegetation: bool,
        source: &crate::bmodel::Surface, meshes: &mut dyn AssetWrite<Mesh>,
        initialize: impl FnOnce(&mut Mesh)) -> Arc<CachedMesh> {
        let key = Key { path: path.to_owned(), surface, flipped, vegetation };
        if let Some(mesh) = self.used.get(&key) { return mesh.clone(); }
        if self.reuse {
            let shared = self.shared.0.lock().unwrap_or_else(|error| error.into_inner());
            let existing = (shared.catalog == self.catalog)
                .then(|| shared.meshes.get(&key).cloned()).flatten();
            drop(shared);
            if let Some(mesh) = existing {
                self.reused += 1;
                self.used.insert(key, mesh.clone());
                return mesh;
            }
        }
        let started = self.trace.then(crate::loading::thread_cycles).flatten();
        let mut mesh = super::view_geometry::source_mesh_from_uvs(&source.positions,
            super::view_geometry::MeshUvs::Packed(&source.uvs), &source.indices,
            &source.packed_channels, flipped);
        initialize(&mut mesh);
        let vegetation = vegetation.then(|| super::view_geometry::retain_vegetation_channels(
            &mut mesh, &source.positions, &source.packed_channels).map(|()| source.positions.len()));
        let debug_mesh = self.debug_meshes.then(|| mesh.clone());
        let mesh = Arc::new(CachedMesh { handle: meshes.add(mesh), vegetation, debug_mesh });
        self.cycles += started.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        self.fresh += 1;
        self.shared.0.lock().unwrap_or_else(|error| error.into_inner()).builds += 1;
        self.used.insert(key, mesh.clone());
        mesh
    }
    pub fn ids(&self) -> impl Iterator<Item = AssetId<Mesh>> + '_ {
        self.used.values().map(|mesh| mesh.handle.id())
    }
    pub fn debug_mesh(&self, id: AssetId<Mesh>) -> Option<&Mesh> {
        self.used.values().find(|mesh| mesh.handle.id() == id)?.debug_mesh.as_ref()
    }
    pub fn publish(self, world: &mut World) {
        let mut shared = self.shared.0.lock().unwrap_or_else(|error| error.into_inner());
        // A cancelled or superseded batch never promotes its reserved handles.
        // Its ordinary asset/command batch owns them until publication or drop.
        if self.reuse {
            if shared.catalog != self.catalog {
                shared.meshes.clear();
                shared.catalog = self.catalog;
            }
            shared.meshes.extend(self.used);
        }
        let mut ids = shared.meshes.values().map(|mesh| mesh.handle.id()).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        if world.resource::<MeshIds>().0.as_ref() != &ids {
            world.insert_resource(MeshIds(Arc::new(ids)));
        }
        if self.trace {
            eprintln!("COMMON_MODEL_MESHES catalog={:?} reuse={} fresh={} reused={} resident={} cycles={}",
                self.catalog, self.reuse, self.fresh, self.reused, shared.meshes.len(), self.cycles);
        }
    }
}


/// The real upload observer must see disjoint physical buffers, not just keys.
pub(super) fn observe_gpu(app: &App, step: usize, phase: &str) -> Result<(), String> {
    use bevy::render::{mesh::RenderMesh, render_asset::RenderAssets};
    let ids = app.world().resource::<Shared>().ids().into_iter().collect::<std::collections::HashSet<_>>();
    let world = app.sub_app(RenderApp).world();
    let allocator = world.resource::<MeshAllocator>();
    let meshes = world.resource::<RenderAssets<RenderMesh>>();
    let mut persistent = std::collections::HashMap::new();
    let mut transient = std::collections::HashSet::new();
    for (id, _) in meshes.iter() {
        for slice in [allocator.mesh_vertex_slice(&id), allocator.mesh_index_slice(&id)].into_iter().flatten() {
            if ids.contains(&id) { persistent.insert(slice.buffer.id(), slice.buffer.size()); }
            else { transient.insert(slice.buffer.id()); }
        }
    }
    let missing = ids.iter().filter(|id| meshes.get(**id).is_none()).count();
    let mixed = persistent.keys().filter(|id| transient.contains(*id)).count();
    eprintln!("COMMON_MODEL_GPU step={step} phase={phase} resident={} missing={missing} buffers={} buffer_bytes={} mixed={mixed}",
        ids.len(), persistent.len(), persistent.values().sum::<u64>());
    if missing != 0 || mixed != 0 { return Err("common mesh residency or lifetime domain is invalid".into()); }
    Ok(())
}
