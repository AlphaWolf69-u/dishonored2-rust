//! Publish placed world water's source geometry without host transform baking.
use super::{world_instances::WorldMatrices, world_mesh, MaterialHandle, WorldShadowMeshSources};
use crate::rendering::bevy_backend::{water_scene::WaterWorldMeshes, water_world_colour};
use bevy::{prelude::*, render::storage::ShaderBuffer};

pub(super) fn prepare_world_geometry(
    world: &world_mesh::WorldMesh,
    sources: &WorldShadowMeshSources,
    meshes: &mut Assets<Mesh>,
    matrices: &mut WorldMatrices,
    buffers: &mut Assets<ShaderBuffer>,
) -> WaterWorldMeshes {
    let objects: std::collections::HashMap<_, _> = sources
        .objects
        .iter()
        .filter(|source| matches!(source.material, Some(MaterialHandle::Water(_))))
        .map(|source| {
            let result = (|| {
                let channels = &world
                    .meshes
                    .get(source.mesh_index)
                    .ok_or("world water source mesh is unavailable")?
                    .packed_channels;
                let mut mesh = meshes
                    .get_mut(&source.mesh)
                    .ok_or("world water shared mesh asset is unavailable")?;
                source.raw_vertex_count.as_ref().map_err(Clone::clone)?;
                water_world_colour::retain_normal_w(&mut mesh, channels)?;
                Ok(source.mesh.clone())
            })();
            (source.object_id, result)
        })
        .collect();
    let has_geometry = objects.values().any(Result::is_ok);
    WaterWorldMeshes {
        objects: std::sync::Arc::new(objects),
        matrices: has_geometry
            .then(|| matrices.get_or_upload(&world.matrices, buffers))
            .flatten(),
    }
}
