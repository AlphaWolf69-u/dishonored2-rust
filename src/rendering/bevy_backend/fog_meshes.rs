//! Interior-volume program adapter over the shared position/instance transport.
use super::{
    fog::InteriorInstance,
    fog_pass::InteriorDraw,
    fog_pipeline::FogPipelines,
    position_meshes::{PositionInstance, PositionMeshes},
};
use bevy::render::{
    mesh::{allocator::MeshAllocator, RenderMesh},
    render_asset::RenderAssets,
    render_resource::FrontFace,
    renderer::{RenderDevice, RenderQueue},
};

#[derive(Default)]
pub struct InteriorMeshes(PositionMeshes);

impl InteriorMeshes {
    pub fn prepare<'a>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        fog: &FogPipelines,
        instances: impl IntoIterator<Item = &'a InteriorInstance>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
    ) -> Result<(), String> {
        self.0.prepare(
            device,
            queue,
            instances.into_iter().map(|instance| PositionInstance {
                mesh: instance.mesh,
                world_from_local: instance.world_from_local,
            }),
            allocator,
            meshes,
            &fog.interior_model_layout,
            // Scene meshes retain the forward renderer's CCW face convention.
            |layout| fog.interior_mesh(device, layout, FrontFace::Ccw),
        )
    }

    pub fn draws(&self) -> Vec<InteriorDraw<'_>> {
        self.0.draws()
    }
}
