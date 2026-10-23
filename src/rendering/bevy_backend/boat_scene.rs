//! Boat list publication and the two distinct world/model mask states.
use super::{
    water_depth_meshes::{DepthSurface, WaterDepthMeshes},
    water_scene::WaterWorldMeshes,
};
use bevy::{
    prelude::*,
    render::{
        extract_component::ExtractComponent,
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
        sync_component::SyncComponent,
    },
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};
#[cfg(test)]
#[path = "boat_scene/tests.rs"]
mod tests;

#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct BoatMaterials {
    pub model: Option<Result<crate::render_state::RenderState, String>>,
    pub world: Arc<HashMap<Arc<str>, Result<crate::render_state::RenderState, String>>>,
}
#[derive(Clone, Copy)]
pub enum BoatGeometry {
    World { object: usize, matrix: usize },
    RigidModel,
    Unsupported,
}
#[derive(Component, Clone)]
pub struct BoatMesh {
    pub material: Arc<str>,
    pub order: usize,
    pub geometry: BoatGeometry,
}
#[derive(Component)]
pub struct BoatInstance {
    pub material: Arc<str>,
    pub order: usize,
    pub geometry: BoatGeometry,
    pub mesh: AssetId<Mesh>,
    pub world_from_local: Mat4,
}
impl SyncComponent for BoatMesh {
    type Target = BoatInstance;
}
impl ExtractComponent for BoatMesh {
    type QueryData = (
        &'static Mesh3d,
        &'static GlobalTransform,
        &'static ViewVisibility,
        &'static BoatMesh,
    );
    type QueryFilter = ();
    type Out = BoatInstance;
    fn extract_component(
        (mesh, transform, visible, source): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self::Out> {
        visible.get().then(|| BoatInstance {
            material: source.material.clone(),
            order: source.order,
            geometry: source.geometry,
            mesh: mesh.0.id(),
            world_from_local: transform.to_matrix(),
        })
    }
}
#[derive(Default)]
pub struct BoatDepthScene {
    pub depth: WaterDepthMeshes,
    pub stencil: WaterDepthMeshes,
    pub main_stencil: WaterDepthMeshes,
}
impl BoatDepthScene {
    pub fn prepare<'a>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        instances: impl IntoIterator<Item = &'a BoatInstance>,
        materials: &BoatMaterials,
        world: &WaterWorldMeshes,
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
        buffers: &RenderAssets<GpuShaderBuffer>,
        frame: crate::rendering::view_history::ViewMatrixFrame,
        format: TextureFormat,
        main_format: TextureFormat,
    ) -> Result<BTreeSet<String>, String> {
        let mut first = Vec::new();
        let mut second = Vec::new();
        let mut pending = BTreeSet::new();
        for instance in instances {
            let result = (|| -> Result<_, String> {
                let (mesh, matrix, base, reversed, host) = match instance.geometry {
                    BoatGeometry::World { object, matrix } => {
                        let source = world
                            .objects
                            .get(&object)
                            .ok_or("world boat mesh unavailable")?
                            .as_ref()
                            .map_err(Clone::clone)?;
                        let state = *materials
                            .world
                            .get(&instance.material)
                            .ok_or("world boat program unavailable")?
                            .as_ref()
                            .map_err(Clone::clone)?;
                        (
                            source.mesh.id(),
                            Some(matrix),
                            crate::render_state::prepared_surface_state(
                                state,
                                source.surface_flags,
                            ),
                            source.indices_reversed,
                            false,
                        )
                    }
                    BoatGeometry::RigidModel => (
                        instance.mesh,
                        None,
                        *materials
                            .model
                            .as_ref()
                            .ok_or("forced boat program unavailable")?
                            .as_ref()
                            .map_err(Clone::clone)?,
                        true,
                        true,
                    ),
                    BoatGeometry::Unsupported => {
                        return Err("non-rigid boat volume ABI is not submitted".into())
                    }
                };
                let state = |stencil| {
                    if matrix.is_some() {
                        crate::render_state::world_boat_volume_state(base, stencil)
                    } else if stencil {
                        (crate::render_state::boat_volume_stencil_state(base), 8)
                    } else {
                        (crate::render_state::boat_volume_depth_state(base), 8)
                    }
                };
                let surface = |stencil| {
                    let (state, reference) = state(stencil);
                    DepthSurface {
                        mesh,
                        matrix,
                        state,
                        reference,
                        reversed,
                        host_space: host,
                        order: instance.order,
                        world_from_local: instance.world_from_local,
                        motion: false,
                    }
                };
                Ok((surface(false), surface(true)))
            })();
            match result {
                Ok((a, b)) => {
                    first.push(a);
                    second.push(b);
                }
                Err(e) => {
                    pending.insert(e);
                }
            }
        }
        self.depth.prepare(
            device, queue, first, world, meshes, allocator, buffers, frame, format,
        )?;
        self.stencil.prepare(
            device,
            queue,
            second.iter().cloned(),
            world,
            meshes,
            allocator,
            buffers,
            frame,
            format,
        )?;
        if main_format != format {
            self.main_stencil.prepare(
                device,
                queue,
                second,
                world,
                meshes,
                allocator,
                buffers,
                frame,
                main_format,
            )?;
        }
        Ok(pending)
    }
}
