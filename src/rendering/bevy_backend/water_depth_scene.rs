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
    pub(super) motion: TextureView,
    _motion: Texture,
    pub(super) geometry: super::water_depth_meshes::WaterDepthMeshes,
    pub boats: super::boat_scene::BoatDepthScene,
    water_requested: bool,
    volume_stencil: Option<super::underwater_stencil::UnderwaterStencilPass>,
    volume_geometry: [super::water_depth_meshes::WaterDepthMeshes;2],
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
            volume_stencil: None,
            volume_geometry: std::array::from_fn(|_|default()),
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
        self.prepare_with_stencil(device,queue,instances,materials,world,meshes,allocator,buffers,frame,None)
    }

    pub fn prepare_with_stencil<'a>(
        &mut self,
        device: &RenderDevice, queue: &RenderQueue,
        instances: impl IntoIterator<Item = &'a WaterInstance>,
        materials: &WaterMaterials, world: &WaterWorldMeshes,
        meshes: &RenderAssets<RenderMesh>, allocator: &MeshAllocator,
        buffers: &RenderAssets<GpuShaderBuffer>,
        frame: crate::rendering::view_history::ViewMatrixFrame,
        stencil: Option<(&super::water_scene::WaterStencilMaterials,crate::render_state::RenderState,TextureFormat)>,
    ) -> Result<(), String> {
        let mut surfaces = Vec::new();
        let mut volume_surfaces=Vec::new();
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
                Some(WaterGeometry::SkinnedModel) => continue,
                None => return Err("water depth geometry association unavailable".into()),
            };
            let world_draw = matrix.is_some();
            if let Some((stencils,state,_))=stencil {
                let selection=stencils.0.get(&instance.material)
                    .ok_or("water boundary stencil selection unavailable")?;
                if let Some(source)=selection[usize::from(world_draw)].as_ref().map_err(Clone::clone)? {
                    let expected=if world_draw {crate::shader_cache::CachedWaterDepthClass::World}
                        else {crate::shader_cache::CachedWaterDepthClass::Rigid};
                    if source.class!=expected {return Err("water boundary stencil ABI differs from geometry".into());}
                    volume_surfaces.push(super::water_depth_meshes::DepthSurface {
                        mesh:mesh_id,matrix,world_from_local:instance.world_from_local,order:instance.order,
                        state:crate::render_state::underwater_stencil_state(
                            crate::render_state::prepared_surface_state(state,surface)),
                        reversed,motion:false,reference:0x42,host_space:!world_draw,
                    });
                }
            }
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
                host_space: !world_draw,
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
        )?;
        for (geometry,format) in self.volume_geometry.iter_mut().zip([
            stencil.map_or(self.alpha.full.format(),|v|v.2),self.alpha.reduced.format(),
        ]) {
            geometry.prepare(device,queue,volume_surfaces.iter().cloned(),world,meshes,allocator,buffers,frame,format)?;
        }
        Ok(())
    }
    /// 2fe850 prepares boat depth when either ocean or placed water is
    /// admitted. Ocean-only scenes must use the same boat pass.
    pub fn request_ocean_depth(&mut self, enabled: bool) {
        self.water_requested |= enabled;
    }

    /// 2fdea0's volume branch is independent of ocean enable/skip and waves.
    /// Main and reduced alpha-depth stencil receive the current water plane.
    pub fn record_volume_underwater(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &super::water_scene::WaterFrame,
        state: crate::render_state::RenderState,
        main: &bevy::render::view::ViewDepthTexture,
    ) -> Result<(), String> {
        use super::underwater_stencil::{
            UnderwaterStencilParameters, UnderwaterStencilPass, UnderwaterStencilSource,
            UnderwaterStencilTarget,
        };
        if !frame.ocean_environment.view.water_volume {
            return Ok(());
        }
        if self.volume_stencil.is_none() {
            self.volume_stencil = Some(UnderwaterStencilPass::with_target_formats(
                device, [main.texture.format(),self.alpha.reduced.format()], state,
            )?);
        }
        let world_from_clip = Mat4::from_cols_array_2d(
            &frame.matrices.current.view_projection,
        ).transpose().inverse();
        // The installed waterMatrix places a horizontal plane at the surface
        // parameter, with camera XY and a 7000-unit XY scale. This shader only
        // reads its Z row, and our fullscreen vertices all have local Z=0.
        let water_matrix = Mat4::from_translation(Vec3::new(
            0., 0., frame.ocean_environment.view.surface_water_level,
        ));
        let target = |texture: &Texture, attachment| {
            let size = Vec2::new(texture.width() as f32, texture.height() as f32);
            UnderwaterStencilTarget {
                attachment,
                parameters: UnderwaterStencilParameters {
                    world_from_clip,
                    water_matrix,
                    viewport: Vec4::new(0., 0., size.x, size.y),
                    resolution_scale: Vec4::ONE,
                },
            }
        };
        self.volume_stencil.as_mut().unwrap().record(
            device, queue, encoder, UnderwaterStencilSource::Volume,
            [target(&main.texture, main.view()),
             target(&self.alpha.reduced, &self.alpha.reduced_attachment)],
            &self.water.vertices, 0..3,
        )?;
        self.volume_geometry[0].record(encoder,main.view(),None,
            [main.texture.width(),main.texture.height()])?;
        self.volume_geometry[1].record(encoder,&self.alpha.reduced_attachment,None,
            [self.alpha.reduced.width(),self.alpha.reduced.height()])
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
        self.begin_depth(device, encoder, opaque)?;
        self.finish_depth(encoder, [opaque.width(), opaque.height()])?;
        self.record_alpha(device, encoder, opaque, main)
    }

    /// Clear/copy and boat preparation precede both ocean and placed water.
    pub fn begin_depth(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
    ) -> Result<(), String> {
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
        Ok(())
    }

    /// Ocean depth is submitted before this placed-water and restoration phase.
    pub fn finish_depth(
        &mut self,
        encoder: &mut CommandEncoder,
        extent: [u32; 2],
    ) -> Result<(), String> {
        self.geometry.record(
            encoder,
            &self.water.attachment,
            Some(&self.motion),
            extent,
        )?;
        self.water.restore_boat_depth(encoder)?;
        Ok(())
    }

    /// Prepare alpha depth once, from all completed water depth producers.
    pub fn record_alpha(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
        main: Option<&TextureView>,
    ) -> Result<(), String> {
        let extent = [opaque.width(), opaque.height()];
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
