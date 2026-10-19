//! Reuse immutable GPU allocation metadata only across unchanged asset epochs.
use super::*;
use bevy::render::render_resource::{BufferId, IndexFormat};

struct EntryAllocation {
    allocation: ShadowMeshAllocation,
    used_frame: u64,
}

#[derive(Default)]
pub(super) struct MeshAllocationCache {
    epoch: Option<(u32, u32)>,
    entries: HashMap<AssetId<Mesh>, EntryAllocation>,
    frame: u64,
    verify: bool,
    pub hits: u64,
    pub misses: u64,
}

impl MeshAllocationCache {
    pub fn begin_frame(&mut self, epoch: (u32, u32), enabled: bool) {
        if !enabled || self.epoch != Some(epoch) {
            self.entries.clear();
        }
        self.frame = self.frame.wrapping_add(1);
        if self.frame == 0 {
            self.entries.clear();
            self.frame = 1;
        }
        self.epoch = enabled.then_some(epoch);
        self.verify = std::env::var("DH2_VERIFY_SHADOW_MESH_ALLOCATIONS").as_deref() == Ok("1");
    }

    pub fn finish_frame(&mut self, complete: bool) {
        if !complete || self.epoch.is_none() {
            self.entries.clear();
            self.epoch = None;
        } else {
            // Keep exactly this submission's meshes without moving every
            // unchanged allocation between two hash tables each frame.
            self.entries
                .retain(|_, entry| entry.used_frame == self.frame);
        }
    }

    pub fn get(
        &mut self,
        id: AssetId<Mesh>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
    ) -> Result<&ShadowMeshAllocation, String> {
        use bevy::platform::collections::hash_map::Entry;
        match self.entries.entry(id) {
            Entry::Occupied(mut entry) => {
                let cached = entry.get_mut();
                if cached.used_frame != self.frame {
                    self.hits += 1;
                    if self.verify {
                        let fresh = Self::fresh(id, allocator, meshes)?;
                        assert!(
                            cached.allocation.layout == fresh.layout,
                            "cached mesh layout changed"
                        );
                        assert_eq!(
                            Self::snapshot(&cached.allocation),
                            Self::snapshot(&fresh),
                            "cached GPU allocation changed"
                        );
                    }
                    cached.used_frame = self.frame;
                }
                Ok(&entry.into_mut().allocation)
            }
            Entry::Vacant(entry) => {
                self.misses += 1;
                let allocation = Self::fresh(id, allocator, meshes)?;
                Ok(&entry
                    .insert(EntryAllocation {
                        allocation,
                        used_frame: self.frame,
                    })
                    .allocation)
            }
        }
    }

    fn fresh(
        id: AssetId<Mesh>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
    ) -> Result<ShadowMeshAllocation, String> {
        let render_mesh = meshes.get(id).ok_or_else(|| {
            format!(
                "native shadow owner Mesh {id:?} has no RenderMesh; refusing an incomplete layer"
            )
        })?;
        let (Some(vertex), Some(index), Some(index_format)) = (
            allocator.mesh_vertex_slice(&id),
            allocator.mesh_index_slice(&id),
            render_mesh.index_format(),
        ) else {
            return Err(format!(
                "native shadow owner Mesh {id:?} has no indexed GPU allocation"
            ));
        };
        Ok(ShadowMeshAllocation {
            layout: render_mesh.layout.clone(),
            layout_key: hash_layout(&render_mesh.layout),
            vertex: vertex.buffer.clone(),
            base_vertex: vertex.range.start as i32,
            index: index.buffer.clone(),
            indices: index.range.clone(),
            index_format,
        })
    }

    fn snapshot(
        value: &ShadowMeshAllocation,
    ) -> (
        u64,
        BufferId,
        i32,
        BufferId,
        std::ops::Range<u32>,
        IndexFormat,
    ) {
        (
            value.layout_key,
            value.vertex.id(),
            value.base_vertex,
            value.index.id(),
            value.indices.clone(),
            value.index_format,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::PrimitiveTopology;
    use bevy::{ecs::system::RunSystemOnce, prelude::*, render::RenderApp};

    #[derive(Resource, Default)]
    struct FixtureCache(MeshAllocationCache);
    #[derive(Resource)]
    struct SelectedMesh(AssetId<Mesh>);

    fn observe(
        allocator: Res<MeshAllocator>,
        meshes: Res<RenderAssets<RenderMesh>>,
        selected: Res<SelectedMesh>,
        mut cache: ResMut<FixtureCache>,
    ) -> (bool, u64, u64, (u32, u32)) {
        let epoch = (allocator.last_changed().get(), meshes.last_changed().get());
        cache.0.begin_frame(epoch, true);
        let actual = cache
            .0
            .get(selected.0, &allocator, &meshes)
            .map(MeshAllocationCache::snapshot);
        let fresh = MeshAllocationCache::fresh(selected.0, &allocator, &meshes)
            .map(|v| MeshAllocationCache::snapshot(&v));
        assert_eq!(
            actual, fresh,
            "cached metadata must match current GPU buffers, ranges and format"
        );
        if actual.is_ok() {
            let counts = (cache.0.hits, cache.0.misses);
            let repeated = cache
                .0
                .get(selected.0, &allocator, &meshes)
                .map(MeshAllocationCache::snapshot);
            assert_eq!(repeated, actual);
            assert_eq!(
                (cache.0.hits, cache.0.misses),
                counts,
                "repeated draws of one mesh resolve only once per submission"
            );
        }
        let present = actual.is_ok();
        cache.0.finish_frame(present);
        (present, cache.0.hits, cache.0.misses, epoch)
    }

    fn mesh(vertices: usize, wide_indices: bool) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.25f32, 0.5, 0.75]; vertices],
        );
        mesh.insert_indices(if wide_indices {
            bevy::mesh::Indices::U32(vec![0, 1, 2])
        } else {
            bevy::mesh::Indices::U16(vec![0, 1, 2])
        });
        mesh
    }

    #[test]
    #[ignore = "requires GPU; validates allocation reuse across real asset growth, format and lifetime changes"]
    fn shadow_mesh_metadata_tracks_real_gpu_reallocation_and_removal() {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.finish();
        app.cleanup();
        let handle = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(mesh(3, false));
        app.sub_app_mut(RenderApp)
            .world_mut()
            .init_resource::<FixtureCache>();
        app.sub_app_mut(RenderApp)
            .world_mut()
            .insert_resource(SelectedMesh(handle.id()));
        let update = |app: &mut App| {
            for _ in 0..3 {
                app.update();
            }
            app.sub_app_mut(RenderApp)
                .world_mut()
                .run_system_once(observe)
                .unwrap()
        };
        let a = update(&mut app);
        assert!(a.0);
        let b = update(&mut app);
        assert!(
            b.0 && b.1 > a.1,
            "stable frames must actually reuse metadata"
        );
        assert_eq!(a.3, b.3, "idle allocator/assets must keep their epochs");
        {
            let mut cache = app
                .sub_app_mut(RenderApp)
                .world_mut()
                .resource_mut::<FixtureCache>();
            cache.0.begin_frame(b.3, true);
            cache.0.finish_frame(true);
            assert!(
                cache.0.entries.is_empty(),
                "an unused allocation must not survive a complete submission"
            );
        }
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(handle.id(), mesh(3000, true))
            .unwrap();
        let c = update(&mut app);
        assert!(c.0 && c.2 > b.2);
        assert_ne!(b.3, c.3);
        let other = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(mesh(5000, false));
        let d = update(&mut app);
        assert!(
            d.0 && d.2 > c.2,
            "other allocations invalidate slab metadata too"
        );
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .remove(handle.id());
        let e = update(&mut app);
        assert!(
            e.0,
            "CPU asset removal retains the GPU allocation while a strong handle is alive"
        );
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .insert(handle.id(), mesh(6, false))
            .unwrap();
        let f = update(&mut app);
        assert!(
            f.0 && f.2 > e.2,
            "reinsertion under the same ID must resolve the new allocation"
        );
        let g = update(&mut app);
        assert!(g.0 && g.1 > f.1);
        drop(handle);
        let h = update(&mut app);
        assert!(
            !h.0,
            "dropping the last handle must remove the GPU allocation"
        );
        drop(other);
    }
}
