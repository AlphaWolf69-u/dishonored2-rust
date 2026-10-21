//! Retained water material sources and visible scene allocations. History and
//! effective view/material parameters must be prepared before raster submission.
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[cfg(test)]
#[path = "water_scene/tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub struct WaterMaterialSource {
    pub main_program: String,
    pub state: crate::render_state::RenderState,
    /// Material-only options. Program defaults and pass specialization remain
    /// separate, as in the installed material constructor.
    pub options: crate::render_program::SerializedParameterBlock,
    pub declaration: Arc<[u8]>,
    /// Slot4 in rigid bank0 and world bank2, independently admitted by actual
    /// resident bytecode. MAIN colour readiness never conceals a depth error.
    pub depth: [Result<Option<Arc<crate::shader_cache::CachedWaterDepthSource>>, String>; 2],
}

/// The immutable level registry is shared across main/render worlds. An error
/// remains attached to its material instead of creating a fabricated ready state.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct WaterMaterials(pub Arc<HashMap<Arc<str>, Result<Arc<WaterMaterialSource>, String>>>);

/// Shared level lighting allocations. Current draw geometry and view constants
/// are separate; retaining these handles does not make a water draw ready.
#[derive(Clone)]
pub struct WaterProbeResources {
    pub vertex_references: Handle<bevy::render::storage::ShaderBuffer>,
    pub probe_info: Handle<bevy::render::storage::ShaderBuffer>,
    pub cube_indices: Handle<bevy::render::storage::ShaderBuffer>,
    pub cubes: Handle<Image>,
    pub planes: Handle<bevy::render::storage::ShaderBuffer>,
    pub primary_count: u32,
    pub cube_count: u32,
}

#[derive(Resource, Clone, bevy::render::extract_resource::ExtractResource)]
pub struct WaterLighting(pub Result<Option<WaterProbeResources>, String>);
impl Default for WaterLighting {
    fn default() -> Self {
        Self(Err("water probe inputs have not been published".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaterGeometry {
    /// Original object and affine-matrix indices, including host-baked meshes.
    World {
        object: usize,
        matrix: usize,
    },
    Model,
}

/// Raw world meshes retain local game coordinates even where the forward host
/// mesh has baked shear. These handles share the shadow mesh allocations.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct WaterWorldMeshes {
    pub objects: Arc<HashMap<usize, Result<Handle<Mesh>, String>>>,
    pub matrices: Option<Handle<bevy::render::storage::ShaderBuffer>>,
}

#[derive(Component, Clone)]
pub struct WaterMesh {
    pub material: Arc<str>,
    pub order: usize,
    pub geometry: Option<WaterGeometry>,
}

#[derive(Component)]
pub struct WaterInstance {
    pub material: Arc<str>,
    pub mesh: bevy::asset::AssetId<Mesh>,
    pub order: usize,
    pub geometry: Option<WaterGeometry>,
    pub world_from_local: Mat4,
}
impl bevy::render::sync_component::SyncComponent for WaterMesh {
    type Target = WaterInstance;
}
impl bevy::render::extract_component::ExtractComponent for WaterMesh {
    type QueryData = (
        &'static Mesh3d,
        &'static GlobalTransform,
        &'static ViewVisibility,
        &'static WaterMesh,
    );
    type QueryFilter = ();
    type Out = WaterInstance;
    fn extract_component(
        (mesh, transform, visible, source): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self::Out> {
        visible.get().then(|| WaterInstance {
            mesh: mesh.0.id(),
            material: source.material.clone(),
            order: source.order,
            geometry: source.geometry,
            world_from_local: transform.to_matrix(),
        })
    }
}

pub struct WaterPlugin;
impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaterMaterials>();
        app.init_resource::<WaterLighting>();
        app.init_resource::<WaterWorldMeshes>();
        app.add_plugins((
            bevy::render::extract_component::ExtractComponentPlugin::<WaterMesh>::default(),
            bevy::render::extract_resource::ExtractResourcePlugin::<WaterMaterials>::default(),
            bevy::render::extract_resource::ExtractResourcePlugin::<WaterLighting>::default(),
            bevy::render::extract_resource::ExtractResourcePlugin::<WaterWorldMeshes>::default(),
        ));
    }
}
