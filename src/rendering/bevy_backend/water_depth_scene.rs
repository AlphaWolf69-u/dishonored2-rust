//! Submit the visible placed-water depth programs using their retained meshes.
use super::{
    alpha_depth::AlphaDepthPass,
    fog_targets::FogExtents,
    tiled_lighting_gpu::TlfPipelines,
    water_depth::WaterDepthPass,
    water_scene::{WaterGeometry, WaterInstance, WaterMaterials, WaterWorldMeshes},
};
use bevy::{
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
    },
};

#[cfg(test)]
#[path = "water_depth_scene/tests.rs"]
mod tests;

pub struct WaterDepthScene {
    pub water: WaterDepthPass,
    pub alpha: AlphaDepthPass,
    motion: TextureView,
    _motion: Texture,
    pub(super) geometry: super::water_depth_meshes::WaterDepthMeshes,
    pub boats: super::boat_scene::BoatDepthScene,
    water_requested: bool,
}
impl WaterDepthScene {
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        extents: FogExtents,
        format: TextureFormat,
    ) -> Result<Self, String> {
        let motion = device.create_texture(&TextureDescriptor {
            label: Some("placed water motion"),
            size: Extent3d {
                width: extents.full.x,
                height: extents.full.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg16Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Self {
            water: WaterDepthPass::new(device, pipelines, extents.full.to_array(), format)?,
            alpha: AlphaDepthPass::new(device, pipelines, extents, format)?,
            motion: motion.create_view(&default()),
            _motion: motion,
            geometry: default(),
            boats: default(),
            water_requested: false,
        })
    }

    pub fn prepare<'a>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        instances: impl IntoIterator<Item = &'a WaterInstance>,
        materials: &WaterMaterials,
        world: &WaterWorldMeshes,
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
        buffers: &RenderAssets<GpuShaderBuffer>,
        frame: crate::rendering::view_history::ViewMatrixFrame,
    ) -> Result<(), String> {
        let mut surfaces = Vec::new();
        self.water_requested = false;
        for instance in instances {
            self.water_requested = true;
            let material = materials
                .0
                .get(&instance.material)
                .ok_or("water depth material unavailable")?
                .as_ref()
                .map_err(Clone::clone)?;
            let (mesh_id, matrix, surface, reversed) = match instance.geometry {
                Some(WaterGeometry::World { object, matrix }) => {
                    let source = world
                        .objects
                        .get(&object)
                        .ok_or("world water depth mesh unavailable")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    (
                        source.mesh.id(),
                        Some(matrix),
                        source.surface_flags,
                        source.indices_reversed,
                    )
                }
                Some(WaterGeometry::Model) => (instance.mesh, None, 0, false),
                None => return Err("water depth geometry association unavailable".into()),
            };
            let world_draw = matrix.is_some();
            let Some(source) = material.depth[usize::from(world_draw)]
                .as_ref()
                .map_err(Clone::clone)?
            else {
                continue;
            };
            let expected = if world_draw {
                crate::shader_cache::CachedWaterDepthClass::World
            } else {
                crate::shader_cache::CachedWaterDepthClass::Rigid
            };
            if source.class != expected {
                return Err("water depth program and geometry class differ".into());
            }
            let state = if world_draw {
                crate::render_state::world_depth_state(material.state, surface)
            } else {
                crate::render_state::water_depth_state()
            };
            surfaces.push(super::water_depth_meshes::DepthSurface {
                mesh: mesh_id,
                matrix,
                world_from_local: instance.world_from_local,
                order: instance.order,
                state,
                reversed,
                motion: true,
                reference: if world_draw { 0 } else { 4 },
                host_space: false,
            });
        }
        self.geometry.prepare(
            device,
            queue,
            surfaces,
            world,
            meshes,
            allocator,
            buffers,
            frame,
            self.water.texture.format(),
        )
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
    ) -> Result<(), String> {
        self.record_with_main(device, encoder, opaque, None)
    }
    pub fn record_with_main(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
        main: Option<&TextureView>,
    ) -> Result<(), String> {
        if !self.boats.stencil.is_empty() && main.is_none() {
            return Err("boat stencil needs the actual main camera attachment".into());
        }
        // The frame-graph clear precedes alpha-depth preparation. That copy
        // deliberately preserves reduced stencil; it must not preserve last
        // frame's boat/underwater classification into this frame.
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("water frame stencil clear"),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &self.alpha.reduced_attachment,
                    depth_ops: None,
                    stencil_ops: Some(Operations {
                        load: LoadOp::Clear(0),
                        store: StoreOp::Store,
                    }),
                }),
                ..default()
            });
        }
        self.water.prepare(device, encoder, opaque, false)?;
        let extent = [opaque.width(), opaque.height()];
        if self.water_requested || !self.geometry.is_empty() {
            self.boats
                .depth
                .record(encoder, &self.water.attachment, None, extent)?;
            self.boats
                .stencil
                .record(encoder, &self.water.attachment, None, extent)?;
        }
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("water motion clear"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.motion,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
        }
        self.geometry.record(
            encoder,
            &self.water.attachment,
            Some(&self.motion),
            [opaque.width(), opaque.height()],
        )?;
        self.water.restore_boat_depth(encoder)?;
        self.alpha.record(device, encoder, &self.water.texture, 0)?;
        if !self.boats.stencil.is_empty() {
            self.boats.stencil.record(
                encoder,
                &self.alpha.reduced_attachment,
                None,
                [self.alpha.reduced.width(), self.alpha.reduced.height()],
            )?;
            let main = main.ok_or("boat stencil needs the actual main camera attachment")?;
            let stencil = if opaque.format() == self.water.texture.format() {
                &self.boats.stencil
            } else {
                &self.boats.main_stencil
            };
            stencil.record(encoder, main, None, extent)?;
        }
        Ok(())
    }
}
