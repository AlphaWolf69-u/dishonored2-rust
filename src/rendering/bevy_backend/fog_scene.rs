//! Render-world owner: sky MRT, signed interior depth and world fog share one
//! real gameplay camera, current mesh allocations and resident image assets.
use super::{
    fog::{FogFrame, FogSceneFrame, FogView, InteriorInstance},
    fog_material::{FogMaterialParameters, FogMaterialResources},
    fog_material_pass::FogDepthSnapshot,
    fog_meshes::InteriorMeshes,
    fog_pass::{FogInputs, FogPass},
    fog_sun_mask::{FogSunMaskUniform, FogSunProgram},
    fog_sun_sprite::SunSpritePass,
    fog_targets::{FogAttachment, FogExtents, FogTargets},
    sky_mask::{GpuSkyMaterial, SkyInstance, SkyProgram},
    sky_pass::SkyPass,
};
use bevy::{
    core_pipeline::{
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
        Core3d, Core3dSystems,
    },
    ecs::system::SystemParam,
    prelude::*,
    render::{
        camera::ExtractedCamera,
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderContext, RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
        sync_world::MainEntity,
        texture::GpuImage,
        view::{ExtractedView, ViewDepthTexture, ViewTarget},
        RenderApp, Render, RenderSystems,
    },
};

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<FogGpu>().add_systems(
            Core3d,
            (
                prepare_atmosphere_depth.in_set(AtmosphereDepthPass)
                    .in_set(Core3dSystems::MainPass).after(main_opaque_pass_3d).before(AtmospherePass),
                render_atmosphere.in_set(AtmospherePass)
                    .in_set(Core3dSystems::MainPass).before(main_transparent_pass_3d),
            ),
        );
        render.add_systems(Render,map_ocean_head_queries.in_set(RenderSystems::Cleanup));
    }
}
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AtmospherePass;
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct AtmosphereDepthPass;
#[derive(SystemParam)]
struct Resources<'w> {
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
    images: Res<'w, RenderAssets<GpuImage>>,
    buffers: Res<'w, RenderAssets<GpuShaderBuffer>>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    materials: Res<'w, RenderAssets<GpuSkyMaterial>>,
    allocator: Res<'w, MeshAllocator>,
    program: Res<'w, SkyProgram>,
    capture_request: Res<'w, super::fog_capture::FogCaptureRequest>,
    water_frame: Option<Res<'w, super::water_scene::WaterSceneFrame>>,
    water_selection: Option<Res<'w,super::water_scene::WaterViewSelection>>,
    ocean_sources: Option<Res<'w, super::ocean_scene::OceanSources>>,
    sun_ray_sources: Option<Res<'w, super::sun_rays::SunRaySources>>,
    water_materials: Option<Res<'w, super::water_scene::WaterMaterials>>,
    water_stencil_materials: Option<Res<'w, super::water_scene::WaterStencilMaterials>>,
    water_model_colours: Option<Res<'w,super::water_scene::WaterModelColours>>,
    water_model_lighting: Option<Res<'w,super::water_scene::WaterModelLighting>>,
    water_meshes: Option<Res<'w, super::water_scene::WaterWorldMeshes>>,
    water_lighting: Option<Res<'w, super::water_scene::WaterLighting>>,
    water_images: Option<Res<'w, super::water_scene::WaterImageSources>>,
    boat_materials: Option<Res<'w, super::boat_scene::BoatMaterials>>,
    display: Option<Res<'w, super::display_scene::DisplaySceneFrame>>,
    coarse: Option<Res<'w, super::coarse_depth::CoarseDepthGpu>>,
    depth_programs: Option<Res<'w, super::tiled_lighting_gpu::TlfPipelines>>,
    local_fog: Res<'w, super::fog_light_scattering::LocalFogFrame>,
    local_shadows: Option<Res<'w, super::shadow_pass::LocalShadowSceneFrame>>,
    local_completion: Option<Res<'w, super::shadow_pass::LocalShadowCompletion>>,
}
#[derive(Resource, Default)]
pub(super) struct FogGpu {
    current: Option<ViewFog>,
    reported: bool,
    completed: Option<(Entity, u32)>,
    depth_completed: Option<(Entity, u32)>,
    prepared_revision: Option<(Entity, u64)>,
}
impl FogGpu {
    pub(super) fn head_water_surface(&self,camera:Entity,frame:u32,baseline:Option<f32>)->Option<f32> {
        self.ocean_capture(camera,frame).and_then(|o|o.head_water_snapshot(frame)).map_or(baseline,|s|s.surface)
    }
    pub(super) fn ocean_capture(&self, camera: Entity, frame: u32) -> Option<&super::ocean_scene::OceanScene> {
        if self.completed != Some((camera, frame)) { return None; }
        self.current.as_ref()?.ocean.as_ref()
    }
    pub(super) fn alpha_depth(&self, camera: Entity, frame:u32) -> Option<&super::water_depth_scene::WaterDepthScene> {
        if self.depth_completed!=Some((camera,frame)) {return None;}
        self.current.as_ref()?.water_depth.as_ref()
    }
    /// Borrow the completed same-frame depth producers. The post-process owner
    /// cannot retain an old view after atmosphere preparation fails or resizes.
    pub(super) fn camera_depth(&self, camera: Entity, frame: u32) -> Option<(&super::water_depth_scene::WaterDepthScene, &FogAttachment)> {
        if self.completed != Some((camera, frame)) { return None; }
        let view = self.current.as_ref()?;
        Some((view.water_depth.as_ref()?, &view.fog.targets.depth))
    }
}
fn map_ocean_head_queries(mut gpu:ResMut<FogGpu>) {
    if let Some(ocean)=gpu.current.as_mut().and_then(|v|v.ocean.as_mut()) {
        if let Err(error)=ocean.map_head_queries() {warn!("Ocean head query readback failed: {error}");}
    }
}
#[derive(Default)]
struct WaterSubmission {
    error: Option<String>,
    boat_error: Option<String>,
    underwater_error: Option<String>,
    underwater_active: bool,
    reported: bool,
}
#[derive(SystemParam)]
struct WaterInstances<'w, 's> {
    water: Query<'w, 's, &'static super::water_scene::WaterInstance>,
    boats: Query<'w, 's, &'static super::boat_scene::BoatInstance>,
}
struct ViewFog {
    tiled: Option<super::tiled_lighting_pass::TiledLightingPass>,
    scattering: Option<super::fog_light_scattering::LightScatteringPass>,
    camera: Entity,
    water_depth: Option<super::water_depth_scene::WaterDepthScene>,
    water_colour: Option<super::water_colour_scene::WaterColourScene>,
    ocean: Option<super::ocean_scene::OceanScene>,
    underwater: Option<super::underwater_scattering::UnderwaterScatteringPass>,
    sun_rays: Option<super::sun_rays::SunRays>,
    areas: Option<super::fog_volumes::AreaFogPass>,
    sun_program: Option<FogSunProgram>,
    sun_sprite: Option<SunSpritePass>,
    hdr_format: TextureFormat,
    depth_format: TextureFormat,
    depth_id: TextureId,
    depth_view: TextureView,
    mask: FogAttachment,
    sky: SkyPass,
    fog: FogPass,
    lookup: Option<super::fog_lookup::FogLookupPass>,
    last_lookup: Option<(
        FogLookupInputIds,
        crate::rendering::fog::lookup::FogLookupPlan,
    )>,
    #[cfg(test)]
    lookup_submissions: u64,
    interiors: InteriorMeshes,
    snapshot: Option<FogDepthSnapshot>,
}
#[derive(PartialEq, Eq)]
struct FogLookupInputIds {
    target: TextureViewId,
    weight: u32,
    overrides: Option<([TextureViewId; 4], [u32; 4])>,
}
#[cfg(test)]
pub(super) fn lookup_submissions(world: &World) -> u64 {
    world
        .resource::<FogGpu>()
        .current
        .as_ref()
        .map_or(0, |view| view.lookup_submissions)
}
impl ViewFog {
    fn new(
        resources: &Resources,
        depth: &ViewDepthTexture,
        hdr: TextureFormat,
        extents: FogExtents,
        view: FogView,
        frame: &FogFrame,
        outputs: Option<(&GpuImage, &GpuImage)>,
    ) -> Result<Self, String> {
        let device = &resources.device;
        let targets = match outputs {
            Some((world, canvas)) => FogTargets::shared(device, extents, world, canvas)?,
            None => FogTargets::new(device, extents),
        };
        Ok(Self {
            tiled: None,
            scattering: None,
            camera: frame.camera,
            water_depth: None,
            water_colour: None,
            ocean: None,
            underwater: None,
            sun_rays: None,
            areas: None,
            sun_program: None,
            sun_sprite: None,
            hdr_format: hdr,
            depth_format: depth.texture.format(),
            depth_id: depth.texture.id(),
            depth_view: sampled_depth(depth),
            mask: FogAttachment::new(
                device,
                "sky and cloud fog mask",
                extents.full,
                TextureFormat::R8Unorm,
            ),
            sky: SkyPass::new(hdr, depth.texture.format()),
            fog: FogPass::with_targets(
                device,
                &resources.queue,
                targets,
                hdr,
                view,
                frame.parameters,
                frame.canvas,
            ),
            interiors: default(),
            lookup: None,
            last_lookup: None,
            #[cfg(test)]
            lookup_submissions: 0,
            snapshot: None,
        })
    }
}
fn sampled_depth(depth: &ViewDepthTexture) -> TextureView {
    depth.texture.create_view(&TextureViewDescriptor {
        aspect: TextureAspect::DepthOnly,
        ..default()
    })
}

#[derive(SystemParam)]
struct AtmosphereParams<'w, 's> {
    frame: Res<'w, FogSceneFrame>,
    resources: Resources<'w>,
    shared: Option<Res<'w, FogMaterialResources>>,
    views: Query<
        'w,
        's,
        (
            &'static MainEntity,
            &'static ViewTarget,
            &'static ViewDepthTexture,
            &'static ExtractedView,
            &'static ExtractedCamera,
            Option<&'static super::auto_exposure::MaterialExposure>,
        ),
        With<super::fog::FogCamera>,
    >,
    skies: Query<'w, 's, &'static SkyInstance>,
    interiors: Query<'w, 's, &'static InteriorInstance>,
    water_instances: WaterInstances<'w, 's>,
    clock: Res<'w, crate::gameplay::game_clock_runtime::GameplayViewClock>,
    sampling: Option<Res<'w, super::shadow_sampling::ParallelSamplingBuffer>>,
    atlas_frame: Res<'w, super::shadow_pass::ShadowSceneFrame>,
    atlas_gpu: Res<'w, super::shadow_pass::ShadowSceneGpu>,
    capture: ResMut<'w, super::fog_capture::FogCaptureGpu>,
    gpu: ResMut<'w, FogGpu>,
    context: RenderContext<'w, 's>,
    last_error: Local<'s, Option<String>>,
    water_submission: Local<'s, WaterSubmission>,
}
fn prepare_atmosphere_depth(params: AtmosphereParams) {
    submit_atmosphere(params, true);
}
fn render_atmosphere(params: AtmosphereParams) {
    submit_atmosphere(params, false);
}
fn submit_atmosphere(params:AtmosphereParams, preparation:bool) {
    let AtmosphereParams {frame,resources,shared,views,skies,interiors,water_instances,clock,sampling,atlas_frame,atlas_gpu,mut capture,mut gpu,mut context,mut last_error,mut water_submission}=params;
    let mut depth_prepared=false;

    let material_parameters = shared
        .as_ref()
        .and_then(|shared| resources.buffers.get(&shared.parameters));
    if let Some(parameters) = material_parameters.filter(|_|preparation) {
        // Queue readiness only after all same-view texture passes are recorded.
        // A pending/removed atmosphere must not leave yesterday's fog enabled.
        resources
            .queue
            .write_buffer(&parameters.buffer, 0, &[0; 16]);
    }
    if preparation {gpu.completed=None;gpu.depth_completed=None;gpu.prepared_revision=None;}
    let Some(frame) = &frame.0 else {
        return;
    };
    let depth_frame=resources.water_frame.as_ref().and_then(|f|f.0.as_ref()).map(|water|(frame.camera,water.matrices.frame));
    if !preparation && gpu.prepared_revision!=Some((frame.camera,frame.revision)) {return;}
    let result = (|| -> Result<(), String> {
        let (main, target, depth, view, camera, material_exposure) =
            views.single().map_err(|error| error.to_string())?;
        if main.id() != frame.camera {
            return Err("fog inputs belong to another camera".into());
        }
        if camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|viewport| viewport.physical_position != UVec2::ZERO)
        {
            return Err("fog sub-view owner unresolved".into());
        }
        if depth.texture.sample_count() != 1 || target.main_texture().sample_count() != 1 {
            return Err("fog requires the game's resolved single-sample view".into());
        }
        let full = camera
            .physical_viewport_size
            .ok_or("fog viewport unavailable")?;
        let extents = FogExtents::new(full, frame.settings.downsample)?;
        let outputs = if let Some(shared) = &shared {
            if shared.extents() != Some(extents) {
                return Err("shared fog outputs await the game-camera viewport".into());
            }
            let get = |handle: &Handle<Image>| {
                resources
                    .images
                    .get(handle)
                    .ok_or("shared fog output awaits GPU preparation")
            };
            let world = get(&shared.world)?;
            let canvas = get(&shared.canvas)?;
            let opaque = get(&shared.opaque_depth)?;
            if opaque.texture.size().width != full.x
                || opaque.texture.size().height != full.y
                || opaque.texture.format() != TextureFormat::Depth32Float
                || opaque.texture.sample_count() != 1
            {
                return Err("opaque fog-depth snapshot awaits current camera size".into());
            }
            if material_parameters.is_none() {
                return Err("fog material parameters await GPU preparation".into());
            }
            Some((world, canvas, opaque))
        } else {
            None
        };
        let world_from_view = view.world_from_view.to_matrix();
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * world_from_view.inverse());
        let inverse = view.clip_from_view.inverse();
        let fog_view = FogView {
            clip_from_world,
            world_from_view,
            world_from_clip: clip_from_world.inverse(),
            ray_scale: Vec4::new(
                1. / view.clip_from_view.x_axis.x,
                1. / view.clip_from_view.y_axis.y,
                frame.parameters.effects.x,
                frame.settings.nearest_threshold,
            ),
            unproject: FogView::depth_coefficients(inverse, camera.exposure),
            extent: Vec4::new(
                full.x as f32,
                full.y as f32,
                extents.depth.x as f32,
                extents.depth.y as f32,
            ),
            reduced_extent: Vec4::new(
                extents.depth.x as f32,
                extents.depth.y as f32,
                extents.world.x as f32,
                extents.world.y as f32,
            ),
            eye: frame.eye.extend(0.),
            // The current gameplay projection has no temporal jitter producer.
            jitter: Vec4::ZERO,
        };
        if gpu.current.as_ref().is_none_or(|current| {
            current.camera != frame.camera
                || current.hdr_format != target.main_texture_format()
                || current.depth_format != depth.texture.format()
        }) {
            gpu.current = Some(ViewFog::new(
                &resources,
                depth,
                target.main_texture_format(),
                extents,
                fog_view,
                frame,
                outputs.map(|(world, canvas, _)| (world, canvas)),
            )?);
        }
        let current = gpu.current.as_mut().unwrap();
        if current.depth_id != depth.texture.id() {
            current.depth_id = depth.texture.id();
            current.depth_view = sampled_depth(depth);
        }
        if current.fog.targets.extents.full != full {
            current.mask = FogAttachment::new(
                &resources.device,
                "sky and cloud fog mask",
                full,
                TextureFormat::R8Unorm,
            );
        }
        if preparation {
        if let Some((world, canvas, _)) = outputs {
            current.fog.update_shared(
                &resources.device,
                &resources.queue,
                extents,
                fog_view,
                frame.parameters,
                frame.canvas,
                world,
                canvas,
            )?;
        } else {
            current.fog.update(
                &resources.device,
                &resources.queue,
                extents,
                fog_view,
                frame.parameters,
                frame.canvas,
            );
        }
        current
            .fog
            .update_light_canvas(&resources.device, &resources.queue, frame.light_canvas);
        let time = clock
            .view_shader_times(crate::gameplay::game_clock::RESET_VIEW_LOCAL_TIME_FLAG)?
            .local_time[0];
        current.sky.prepare(
            &resources.device,
            &resources.queue,
            &resources.program,
            skies.iter(),
            &resources.allocator,
            &resources.meshes,
            &resources.materials,
            frame,
            clip_from_world,
            time,
            camera.exposure,
        )?;
        current.sky.record(
            context.command_encoder(),
            target.main_texture_view(),
            depth.view(),
            &current.mask.view,
        );
        }
        // The native lookup owner runs even when this environment disables
        // world fog. Keep the same history available when a later area enables it.
        let curve_target = resources
            .images
            .get(&frame.curve)
            .ok_or("fog curve target awaits GPU preparation")?;
        let overrides = frame
            .curve_overrides
            .as_ref()
            .map(|frame| {
                let images = frame
                    .images
                    .iter()
                    .map(|image| {
                        resources
                            .images
                            .get(image)
                            .map(|image| &image.texture_view)
                            .ok_or("fog override curve awaits GPU preparation")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, &str>(super::fog_lookup::FogLookupOverrides {
                    views: [images[0], images[1], images[2], images[3]],
                    weights: frame.weights,
                })
            })
            .transpose()?;
        if let Some(shared) = &shared {
            let image = resources
                .images
                .get(&shared.curve)
                .ok_or("shared fog curve layers await GPU preparation")?;
            if current
                .lookup
                .as_ref()
                .is_none_or(|pass| pass.texture_id() != image.texture.id())
            {
                current.lookup = Some(super::fog_lookup::FogLookupPass::shared(
                    &resources.device,
                    &image.texture,
                    frame.curve_weight,
                )?);
                current.last_lookup = None;
            }
        } else if current.lookup.is_none() {
            current.lookup = Some(super::fog_lookup::FogLookupPass::new(
                &resources.device,
                &resources.queue,
                frame.curve_weight,
            ));
        }
        let input_ids = FogLookupInputIds {
            target: curve_target.texture_view.id(),
            weight: frame.curve_weight.to_bits(),
            overrides: overrides.as_ref().map(|v| {
                (
                    v.views.map(|view| view.id()),
                    v.weights.to_array().map(f32::to_bits),
                )
            }),
        };
        let lookup = current.lookup.as_mut().unwrap();
        // As in the display colour-cube owner, these resident image assets are
        // immutable. Identical raw weights and view identities reproduce the
        // same curve and history roles; retain that output without GPU work.
        let lookup_plan = if let Some((_, plan)) = current
            .last_lookup
            .as_ref()
            .filter(|(ids, _)| *ids == input_ids)
        {
            *plan
        } else {
            if !preparation {return Err("fog curve awaits depth preparation".into());}
            let plan = lookup.record(
                &resources.device,
                &resources.queue,
                context.command_encoder(),
                &curve_target.texture_view,
                frame.curve_weight,
                overrides,
            );
            current.last_lookup = Some((input_ids, plan));
            #[cfg(test)]
            {
                current.lookup_submissions += 1;
            }
            plan
        };
        let curve = lookup.view().clone();
        // Sky also owns visible colour when this environment disables world fog.
        let scattering_enabled = !frame.settings.skip_light_scattering
            && resources
                .local_fog
                .lights
                .iter()
                .any(|p| f32::from_bits(p[13]) > 0.);
        let no_atmosphere=frame.parameters.flags.x & 24 == 0
            && frame.areas.is_empty()
            && !scattering_enabled
            && water_instances.water.is_empty()
            && water_instances.boats.is_empty()
            && !resources.water_frame.as_ref().and_then(|f|f.0.as_ref()).is_some_and(|f|
                (f.ocean_environment.view.enabled && !f.ocean_render_settings.skip)
                || f.ocean_environment.view.water_volume)
;
        if preparation {
        current.interiors.prepare(
            &resources.device,
            &resources.queue,
            current.fog.pipelines(),
            interiors.iter(),
            &resources.allocator,
            &resources.meshes,
        )?;
        if let Some((_, _, opaque)) = outputs {
            current
                .snapshot
                .get_or_insert_with(|| FogDepthSnapshot::new(&resources.device))
                .record(
                    &resources.device,
                    context.command_encoder(),
                    &current.depth_view,
                    &opaque.texture_view,
                );
        }
        if let Some(water_frame) = resources
            .water_frame
            .as_ref()
            .and_then(|frame| frame.0.as_ref())
        {
            if water_frame.camera != frame.camera {
                return Err("water depth inputs belong to another camera".into());
            }
            let programs = resources
                .depth_programs
                .as_ref()
                .ok_or("water depth programs unavailable")?;
            if current.water_depth.as_ref().is_none_or(|owner| {
                owner.water.texture.width() != full.x || owner.water.texture.height() != full.y
            }) {
                current.water_depth = Some(super::water_depth_scene::WaterDepthScene::new(
                    &resources.device,
                    programs,
                    extents,
                    TextureFormat::Depth32FloatStencil8,
                )?);
            }
            let owner = current.water_depth.as_mut().unwrap();
            owner.alpha.resize(&resources.device, extents);
            let volume_stencil=if water_frame.ocean_environment.view.water_volume {
                Some((resources.water_stencil_materials.as_deref()
                    .ok_or("water boundary stencil registry unavailable")?,
                    resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref())
                        .ok_or("water volume programs unavailable")?.volume_underwater_state,
                    depth.texture.format()))
            } else {None};
            owner.prepare_with_stencil(
                &resources.device,
                &resources.queue,
                water_instances.water.iter(),
                resources
                    .water_materials
                    .as_ref()
                    .ok_or("water depth material registry unavailable")?,
                resources
                    .water_meshes
                    .as_ref()
                    .ok_or("water depth mesh registry unavailable")?,
                &resources.meshes,
                &resources.allocator,
                &resources.buffers,
                water_frame.matrices,
                volume_stencil,
            )?;
            if let Some(materials) = resources.boat_materials.as_ref() {
                let pending = owner.boats.prepare(
                    &resources.device,
                    &resources.queue,
                    water_instances.boats.iter(),
                    materials,
                    resources
                        .water_meshes
                        .as_ref()
                        .ok_or("boat world mesh registry unavailable")?,
                    &resources.meshes,
                    &resources.allocator,
                    &resources.buffers,
                    water_frame.matrices,
                    owner.water.texture.format(),
                    depth.texture.format(),
                )?;
                let error = (!pending.is_empty())
                    .then(|| pending.into_iter().collect::<Vec<_>>().join("; "));
                if error != water_submission.boat_error {
                    if let Some(error) = &error {
                        warn!("Boat volume submission pending: {error}");
                    }
                    water_submission.boat_error = error;
                }
            } else {
                owner.boats = default();
            }
            owner.request_ocean_depth(water_frame.ocean_environment.view.enabled && !water_frame.ocean_render_settings.skip);
            owner.begin_depth(
                &resources.device,
                context.command_encoder(),
                &depth.texture,
            )?;
            if current.ocean.is_some() || (water_frame.ocean_environment.view.enabled && !water_frame.ocean_render_settings.skip) {
                if current.ocean.is_none() {
                    current.ocean=Some(super::ocean_scene::OceanScene::new(&resources.device,&resources.queue)?);
                }
                let ocean=current.ocean.as_mut().unwrap();
                ocean.prepare_with_selection(&resources.device,&resources.queue,context.command_encoder(),water_frame,
                    resources.water_images.as_ref().ok_or("ocean image sources unavailable")?,full.to_array(),
                    resources.water_selection.as_ref().and_then(|s|s.0.as_ref()))?;
                let programs=resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref())
                    .ok_or("ocean programs unavailable")?;
                ocean.record_depth(&resources.device,&resources.queue,context.command_encoder(),water_frame,
                    programs,&owner.water.attachment,&owner.motion,full.to_array())?;
            }
            owner.finish_depth(context.command_encoder(), full.to_array())?;
            owner.record_alpha(&resources.device,context.command_encoder(),&depth.texture,Some(depth.view()))?;
            if let Some(ocean)=current.ocean.as_mut() {
                let programs=resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref())
                    .ok_or("ocean programs unavailable")?;
                ocean.record_underwater(&resources.device,&resources.queue,context.command_encoder(),water_frame,programs,owner,depth)?;
            }
            if water_frame.ocean_environment.view.water_volume {
                let programs=resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref())
                    .ok_or("water volume programs unavailable")?;
                owner.record_volume_underwater(&resources.device,&resources.queue,
                    context.command_encoder(),water_frame,programs.volume_underwater_state,depth)?;
            }
        } else {
            current.water_depth = None;
            current.ocean = None;
        }
            depth_prepared=true;
            return Ok(());
        }
        if no_atmosphere {
            let source=current.water_depth.as_ref().map_or(&current.depth_view,|owner|&owner.water.sample);
            current.fog.record_linear_depth(&resources.device,context.command_encoder(),source);
            return Ok(());
        }
        let noise = resources
            .images
            .get(&frame.noise)
            .ok_or("fog noise awaits GPU preparation")?;
        let projected_height = resources
            .images
            .get(&frame.shadow.image)
            .ok_or("fog projected height awaits GPU preparation")?;
        let sampling = sampling
            .as_ref()
            .ok_or("fog parallel sampling owner unavailable")?;
        let parallel = resources
            .buffers
            .get(&sampling.buffer)
            .ok_or("fog shadow parameters await GPU preparation")?;
        let atlas = if frame.parameters.flags.x & 64 != 0 {
            if !atlas_gpu.completed_revision(atlas_frame.revision) {
                return Err("fog shadows await current atlas submission".into());
            }
            let super::shadow_pass::ShadowSceneState::Ready { target_image, .. } =
                &atlas_frame.state
            else {
                return Err("fog shadow atlas unavailable".into());
            };
            resources
                .images
                .get(target_image)
                .ok_or("fog atlas awaits GPU preparation")?
        } else {
            resources
                .images
                .get(&sampling.unavailable_depth)
                .ok_or("disabled shadow binding awaits GPU preparation")?
        };
        //309430 places resource hash407bd17f (depthWater) in context slot7;
        //307530 passes that attachment to30a800, rather than opaque depth.
        if let Some(sun) = &frame.sun_sprite {
            if let Some(projection) = sun.parameters.project(
                fog_view.clip_from_game_world(),frame.settings.far_clip,full,
            )? {
                let owner=current.water_depth.as_ref().ok_or("sun depthWater awaits preparation")?;
                let sun_depth=&owner.water.attachment;
                let sun_depth_format=owner.water.texture.format();
                let image=resources.images.get(&sun.image).ok_or("sun sprite image awaits GPU preparation")?;
                let uniform=FogSunMaskUniform::sprite(sun.parameters,projection,fog_view,full);
                if current.sun_sprite.as_ref().is_none_or(|pass|pass.state!=sun.state || pass.depth_format!=sun_depth_format) {
                    let program=current.sun_program.get_or_insert_with(||FogSunProgram::new(&resources.device));
                    current.sun_sprite=Some(SunSpritePass::new(&resources.device,program,sun.state,
                        target.main_texture_format(),sun_depth_format,uniform)?);
                }
                let pass=current.sun_sprite.as_mut().unwrap();
                pass.prepare(&resources.device,&resources.queue,uniform,&image.texture_view);
                pass.record(context.command_encoder(),target.main_texture_view(),sun_depth,sun.parameters.additive);
            }
        }
        if scattering_enabled {
            use super::fog_light_scattering::{
                LightScatterView, LightScatteringInputs, LightScatteringPass,
            };
            if resources.local_fog.camera != Some(frame.camera) {
                return Err("local fog lights belong to another camera".into());
            }
            let water_frame = resources
                .water_frame
                .as_ref()
                .and_then(|f| f.0.as_ref())
                .ok_or("local fog view matrices unavailable")?;
            let water = current
                .water_depth
                .as_ref()
                .ok_or("local fog alpha depth unavailable")?;
            let programs = resources
                .depth_programs
                .as_ref()
                .ok_or("local fog culling programs unavailable")?;
            let local = resources.local_shadows.as_ref().and_then(|s| s.0.as_ref());
            let shadowed = resources
                .local_fog
                .lights
                .iter()
                .any(|p| p[24] != u32::MAX && p[25] & 2 != 0);
            if shadowed {
                let local = local.ok_or("local fog shadow frame unavailable")?;
                if resources.local_fog.shadow_revision != Some(local.revision) {
                    return Err("local fog shadow indices belong to another frame".into());
                }
                let ready = resources
                    .local_completion
                    .as_ref()
                    .and_then(|c| c.0.lock().ok())
                    .is_some_and(|done| {
                        done.as_ref().is_some_and(|(revision, atlas, cache, _)| {
                            *revision == local.revision
                                && *atlas == local.atlas.id()
                                && *cache == local.cache.id()
                        })
                    });
                if !ready {
                    return Err("local fog shadows await atlas submission".into());
                }
            }
            let receiver = local
                .map(|s| s.receiver.as_ref())
                .transpose()
                .map_err(Clone::clone)?;
            let local_atlas = if shadowed {
                &resources
                    .images
                    .get(&local.unwrap().atlas)
                    .ok_or("local fog shadow atlas awaits GPU preparation")?
                    .texture_view
            } else {
                &atlas.texture_view
            };
            let grid = crate::tiled_lighting::TileGrid::new(
                full.x,
                full.y,
                frame.settings.tiled_light_dimension,
            )?;
            let tiled = current.tiled.get_or_insert_with(|| {
                super::tiled_lighting_pass::TiledLightingPass::new(
                    &resources.device,
                    programs,
                    grid,
                )
            });
            tiled.resize(&resources.device, programs, grid);
            let matrices = water_frame.matrices.current;
            tiled.prepare(
                &resources.device,
                &resources.queue,
                programs,
                super::tiled_lighting_pass::TiledLightView {
                    uv_to_ray: Vec4::new(0., 0., fog_view.ray_scale.x, fog_view.ray_scale.y),
                    view: matrices.view.map(Vec4::from_array),
                    projection: matrices.projection.map(Vec4::from_array),
                    ..default()
                },
                &current.depth_view,
                &water.alpha.full_sample,
                &resources.local_fog.lights,
                receiver.map_or(&[], |r| r.shadows.as_slice()),
            )?;
            tiled.record(context.command_encoder(), programs)?;
            let scattering_view = LightScatterView {
                world_from_clip: Mat4::from_cols_array_2d(&matrices.view_projection)
                    .transpose()
                    .inverse(),
                camera_far: frame.eye.extend(frame.settings.far_clip),
                position_to_view_texture: Vec4::new(
                    0.,
                    0.,
                    1. / extents.world.x as f32,
                    1. / extents.world.y as f32,
                ),
                canvas_scale_jitter: Vec4::new(1., 1., 0., 0.),
                grid: UVec4::new(
                    grid.lighting_shift,
                    grid.dimensions[0],
                    grid.dimensions[1],
                    1u32 << frame.settings.downsample,
                ),
                black_white: Vec4::ZERO,
            };
            let scattering = current.scattering.get_or_insert_with(|| {
                LightScatteringPass::new(
                    &resources.device,
                    current.fog.targets.world.texture.format(),
                    scattering_view,
                )
            });
            scattering.prepare(
                &resources.device,
                &resources.queue,
                scattering_view,
                LightScatteringInputs {
                    lights: &tiled.lights,
                    pool: &tiled.buffers.pool,
                    heads: &tiled.scattering_heads,
                    shadows: &tiled.shadows,
                    atlas: local_atlas,
                    depth: &current.fog.targets.depth.view,
                    first_canvas: &current.fog.targets.canvas.view,
                    second_canvas: current
                        .fog
                        .light_canvas_view()
                        .ok_or("local fog turbulence canvas unavailable")?,
                },
            );
        } else {
            current.scattering = None;
        }
        let draws = current.interiors.draws();
        if !frame.areas.is_empty() {
            let states = frame
                .area_states
                .ok_or("area fog render states unavailable")?;
            if current.areas.as_ref().is_none_or(|pass| {
                pass.states.interval != states.interval || pass.states.radiance != states.radiance
            }) {
                current.areas = Some(super::fog_volumes::AreaFogPass::new(
                    &resources.device,
                    extents,
                    states,
                )?);
            }
            current.areas.as_mut().unwrap().prepare(
                &resources.device,
                &resources.queue,
                extents,
                fog_view,
                &frame.areas,
                &resources.meshes,
                &resources.allocator,
                &current.fog.targets.depth.view,
            )?;
        }
        let underwater_result=(|| -> Result<bool,String> {
            use super::underwater_scattering::{UnderwaterInputs,UnderwaterScatteringPass};
            let Some(water_frame)=resources.water_frame.as_ref().and_then(|f|f.0.as_ref()) else {return Ok(false)};
            let volume=water_frame.ocean_environment.view.water_volume;
            let ocean_underwater=current.ocean.as_ref().is_some_and(|o|o.state.view.enabled
                && !water_frame.ocean_render_settings.skip
                && water_frame.matrices.current.position[2]<o.state.view.water_level+o.state.view.max_wave_amplitude);
            if !volume && !ocean_underwater {return Ok(false)}
            let console=crate::platform::console::shared().read();
            let scattering=!console.cvars.get("r_skipFogUnderWater").is_some_and(|v|v.int!=0);
            let program=&resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref())
                .ok_or("underwater program inputs unavailable")?.underwater_effect;
            let selected=if volume {
                let material=resources.water_selection.as_ref().and_then(|s|s.0.as_ref())
                    .and_then(|s|s.surface_material.as_ref()).ok_or("underwater volume material publication pending")?;
                Some(resources.water_materials.as_ref().ok_or("underwater material registry unavailable")?
                    .0.get(material).ok_or("selected underwater material unavailable")?.as_ref().map_err(Clone::clone)?)
            } else {None};
            let mut blocks=vec![&program.parameters[0],&program.parameters[1]];
            if let Some(material)=selected {blocks.push(&material.parameters[1]);}
            let settings=crate::rendering::water::WaterSettings::resolve(
                &crate::render_program::parameters::ParameterScopes{view:&water_frame.values,blocks:&blocks})?;
            let mut ray_blocks=vec![&program.ray_parameters[0],&program.ray_parameters[1]];
            if let Some(material)=selected {ray_blocks.push(&material.parameters[1]);}
            let mut rays=crate::rendering::water::rays::WaterRaySettings::resolve(
                &crate::render_program::parameters::ParameterScopes{view:&water_frame.values,blocks:&ray_blocks})?;
            rays.enabled &= !console.cvars.get("r_skipGodRaysUnderWater").is_some_and(|v|v.int!=0);
            drop(console);
            if !scattering && !rays.enabled {return Ok(false)}
            let display=resources.display.as_ref().and_then(|f|f.0.as_ref())
                .filter(|f|f.camera==frame.camera).and_then(|f|f.lens.as_ref())
                .ok_or("underwater HDR view parameters unavailable")?;
            let exposure=resources.buffers.get(&material_exposure.ok_or("underwater adaptation owner unavailable")?.0)
                .ok_or("underwater adaptation buffer awaits GPU preparation")?;
            let (water_atlas,shadow)=match &atlas_frame.state {
                super::shadow_pass::ShadowSceneState::Ready{target_image,receiver,..}=>{
                    if !atlas_gpu.completed_revision(atlas_frame.revision) {return Err("underwater shadows await current atlas submission".into());}
                    (&resources.images.get(target_image).ok_or("underwater shadow atlas unavailable")?.texture_view,
                        super::shadow_sampling::ParallelSamplingUniform::from_owner_receiver(receiver)?)
                },
                super::shadow_pass::ShadowSceneState::Disabled=>(&atlas.texture_view,default()),
                _=>return Err("underwater shadows are not ready".into()),
            };
            let shadow=super::water_refraction::WaterShadowParameters{
                world_to_shadow:shadow.world_to_shadow.map(|m|m.to_cols_array()),view_row:shadow.view_row2.to_array(),
                slice_fars:shadow.cascade_fars.to_array(),sun_direction:shadow.sun_direction.to_array(),
                shadow_size:shadow.shadow_size.to_array(),bias:shadow.bias.to_array(),
                options:[u32::from(shadow.config.x!=0),shadow.config.z,0,0],
            };
            let compressed=display.settings.hdr_buffer_format!=1;
            if current.underwater.as_ref().is_none_or(|u|!u.matches(full.to_array(),compressed,target.main_texture_format())) {
                current.underwater=Some(UnderwaterScatteringPass::new(&resources.device,&resources.queue,
                    full.to_array(),compressed,target.main_texture_format(),program)?);
            }
            let water_depth=current.water_depth.as_ref().ok_or("underwater alpha depth unavailable")?;
            let water_level=if volume {water_frame.ocean_environment.view.surface_water_level}
                else {current.ocean.as_ref().ok_or("underwater ocean state unavailable")?.state.view.water_level};
            current.underwater.as_mut().unwrap().prepare(&resources.device,&resources.queue,context.command_encoder(),UnderwaterInputs{
                frame:water_frame,settings:&settings,water_level,white_level:display.white_level,camera_exposure:camera.exposure,
                adaptation:&exposure.buffer,shadow,atlas:water_atlas,
                opaque:outputs.map_or(&depth.texture,|(_,_,opaque)|&opaque.texture),alpha:&water_depth.alpha,hdr:target,
                images:resources.water_images.as_deref().ok_or("underwater images unavailable")?,
                depth_programs:resources.depth_programs.as_deref().ok_or("underwater depth programs unavailable")?,
                extent:extents.world.to_array(),
                rays, ray_depth:&water_depth.water.sample, ray_shadow:&current.fog.targets.shadow.view,
                jitter:[fog_view.jitter.x,fog_view.jitter.y], temporal:frame.shadow.temporal,
                downsample:frame.settings.downsample, scattering,
            })?;
            if !water_submission.underwater_active {
                info!(volume, scattering, rays=rays.enabled, shadowed_rays=rays.shadowing,
                    ray_intensity=rays.intensity, water_level=if volume {water_frame.ocean_environment.view.surface_water_level} else {current.ocean.as_ref().unwrap().state.view.water_level},
                    eye_z=water_frame.matrices.current.position[2], extinction=settings.extinction,
                    inscattering=settings.inscattering, "Underwater effect inputs admitted");
            }
            Ok(true)
        })();
        let underwater_active=underwater_result.as_ref().copied().unwrap_or(false);
        if underwater_active != water_submission.underwater_active {
            info!(active=underwater_active, "Underwater effect submission changed");
            water_submission.underwater_active=underwater_active;
        }
        let underwater_error=underwater_result.err();
        if underwater_error!=water_submission.underwater_error {
            if let Some(error)=&underwater_error {warn!("Underwater scattering pending: {error}");}
            water_submission.underwater_error=underwater_error;
        }
        let underwater=current.underwater.as_ref().filter(|_|underwater_active);
        let ray_projection=if crate::platform::console::shared().read().cvars.get("r_skipGodRays").is_some_and(|v|v.int!=0) {
            None
        } else {frame.rays.project(fog_view.clip_from_game_world(),frame.settings.far_clip,full)?};
        if let Some(projection)=ray_projection {
            let state=resources.sun_ray_sources.as_ref().and_then(|s|s.0).ok_or("sun ray program unavailable")?;
            let alpha=&current.water_depth.as_ref().ok_or("sun ray water depth unavailable")?.alpha;
            let programs=resources.depth_programs.as_deref().ok_or("sun ray depth programs unavailable")?;
            if current.sun_rays.as_ref().is_none_or(|r|!r.matches(extents,state,alpha)) {
                current.sun_rays=Some(super::sun_rays::SunRays::new(&resources.device,programs,extents,state,alpha,frame.settings.downsample,frame,fog_view,projection)?);
            }
            current.sun_rays.as_mut().unwrap().prepare(&resources.device,&resources.queue,context.command_encoder(),
                frame,fog_view,projection,inverse.transpose().to_cols_array_2d(),target.main_texture_view(),
                &current.fog.targets.shadow.view,&current.fog.targets.canvas.view,&current.fog.targets.interior.view,programs)?;
        }
        let sun_rays=current.sun_rays.as_ref().filter(|_|ray_projection.is_some());
        let ray_visibility=sun_rays.map(|r|r.visibility()).or_else(||underwater.and_then(|u|u.visibility()));
        let fog_source = current.fog.record_sources_with_underwater(
            &resources.device,
            &resources.queue,
            context.command_encoder(),
            &FogInputs {
                shadow_parameters: &super::fog_shadow::FogShadowParameters {
                    viewport: [0., 0., extents.shadow.x as f32, extents.shadow.y as f32],
                    resolution_projected: [1., 1., frame.shadow.extent, 0.],
                    jitter: [fog_view.jitter.z, fog_view.jitter.w, 0., 0.],
                    options: [
                        u32::from(frame.shadow.projected),
                        u32::from(frame.shadow.temporal),
                        1,
                        0,
                    ],
                },
                projected_height: &projected_height.texture_view,
                scattering: current.scattering.as_ref(),
                areas: if frame.areas.is_empty() {
                    None
                } else {
                    current.areas.as_ref()
                },
                settings: frame.settings,
                ray_mask: ray_visibility,
                shadowed_volumes: false,
                // 309430 ORs base-light bit5 across all admitted lights,
                // including lights whose scattering value is zero. The TLF
                // packet writer preserves that flag in bit1.
                shadowed_local_lights: resources.local_fog.lights.iter().any(|p| p[25] & 2 != 0),
                depth: current
                    .water_depth
                    .as_ref()
                    .map_or(&current.depth_view, |owner| &owner.water.sample),
                sky_mask: &current.mask.view,
                curve: &curve,
                noise: &noise.texture_view,
                atlas: &atlas.texture_view,
                parallel: &parallel.buffer,
            },
            &draws,
            underwater.is_some_and(|u|u.shadowed_rays()),
            sun_rays,
            |encoder,target| {
                if let Some(underwater)=underwater {underwater.record_with_visibility(encoder,target,sun_rays.map(|r|r.visibility()));}
            },
        );
        if !water_instances.water.is_empty() || current.ocean.as_ref().is_some_and(|o|o.draw_count()>0) {
            let result = (|| -> Result<_, String> {
                use super::water_colour_scene::{
                    WaterColourResources, WaterColourScene, WaterColourView,
                };
                let water_frame = resources
                    .water_frame
                    .as_ref()
                    .and_then(|f| f.0.as_ref())
                    .ok_or("water colour frame unavailable")?;
                let display = resources
                    .display
                    .as_ref()
                    .and_then(|f| f.0.as_ref())
                    .filter(|f| f.camera == frame.camera)
                    .and_then(|f| f.lens.as_ref())
                    .ok_or("water HDR view parameters unavailable")?;
                let exposure = resources
                    .buffers
                    .get(
                        &material_exposure
                            .ok_or("water adaptation owner unavailable")?
                            .0,
                    )
                    .ok_or("water adaptation buffer awaits GPU preparation")?;
                let compressed = display.settings.hdr_buffer_format != 1;
                if current.water_colour.as_ref().is_none_or(|owner| {
                    !owner.matches(
                        full.to_array(),
                        compressed,
                        target.main_texture_format(),
                        TextureFormat::Depth32FloatStencil8,
                    )
                }) {
                    current.water_colour = Some(WaterColourScene::new(
                        &resources.device,
                        &resources.queue,
                        full.to_array(),
                        compressed,
                        target.main_texture_format(),
                        TextureFormat::Depth32FloatStencil8,
                    )?);
                }
                let (water_atlas, shadow) = match &atlas_frame.state {
                    super::shadow_pass::ShadowSceneState::Ready {
                        target_image,
                        receiver,
                        ..
                    } => {
                        if !atlas_gpu.completed_revision(atlas_frame.revision) {
                            return Err("water shadows await current atlas submission".into());
                        }
                        let atlas = resources
                            .images
                            .get(target_image)
                            .ok_or("water shadow atlas awaits GPU preparation")?;
                        (
                            &atlas.texture_view,
                            super::shadow_sampling::ParallelSamplingUniform::from_owner_receiver(
                                receiver,
                            )?,
                        )
                    }
                    super::shadow_pass::ShadowSceneState::Disabled => {
                        (&atlas.texture_view, default())
                    }
                    _ => return Err("water shadow owner is not ready".into()),
                };
                let shadow = super::water_refraction::WaterShadowParameters {
                    world_to_shadow: shadow.world_to_shadow.map(|m| m.to_cols_array()),
                    view_row: shadow.view_row2.to_array(),
                    slice_fars: shadow.cascade_fars.to_array(),
                    sun_direction: shadow.sun_direction.to_array(),
                    shadow_size: shadow.shadow_size.to_array(),
                    bias: shadow.bias.to_array(),
                    options: [u32::from(shadow.config.x != 0), shadow.config.z, 0, 0],
                };
                let water_resources=WaterColourResources {
                        device: &resources.device,
                        queue: &resources.queue,
                        materials: resources
                            .water_materials
                            .as_ref()
                            .ok_or("water materials unavailable")?,
                        world: resources
                            .water_meshes
                            .as_ref()
                            .ok_or("water meshes unavailable")?,
                        lighting: resources
                            .water_lighting
                            .as_ref()
                            .ok_or("water lighting unavailable")?,
                        images: resources
                            .water_images
                            .as_ref()
                            .ok_or("water images unavailable")?,
                        image_assets: &resources.images,
                        meshes: &resources.meshes,
                        buffers: &resources.buffers,
                        allocator: &resources.allocator,
                        depth_programs: resources
                            .depth_programs
                            .as_ref()
                            .ok_or("water depth programs unavailable")?,
                    };
                let water_view=WaterColourView {
                        frame: water_frame,
                        white_level: display.white_level,
                        camera_exposure: camera.exposure,
                        adaptation: &exposure.buffer,
                        shadow,
                        atlas: water_atlas,
                        fog: fog_source.view(),
                        hdr: target.main_texture_view(),
                        opaque: outputs.map_or(&depth.texture, |(_, _, opaque)| &opaque.texture),
                        water_depth: &current
                            .water_depth
                            .as_ref()
                            .ok_or("water depth owner unavailable")?
                            .water
                            .attachment,
                        pyramid: resources
                            .coarse
                            .as_ref()
                            .and_then(|c| c.pyramid_for(depth.texture.id())),
                        shared_pyramid_expected: resources.coarse.is_some(),
                    };
                if let Some(ocean)=current.ocean.as_mut() {
                    ocean.record_colour(&water_resources,&water_view,
                        resources.ocean_sources.as_ref().and_then(|s|s.0.as_deref()).ok_or("ocean programs unavailable")?,
                        full.to_array(),compressed,target.main_texture_format(),
                        &current.water_depth.as_ref().unwrap().water.sample,context.command_encoder())?;
                }
                current.water_colour.as_mut().unwrap().record_with_models(
                    water_resources,
                    water_view,
                    water_instances.water.iter(),
                    context.command_encoder(),
                    resources.water_model_colours.as_deref().ok_or("model water colour registry unavailable")?,
                    resources.water_model_lighting.as_deref(),
                )
            })();
            let error = match result {
                Ok(pending) if pending.is_empty() => None,
                Ok(pending) => Some(pending.into_iter().collect::<Vec<_>>().join("; ")),
                Err(error) => Some(error),
            };
            if error != water_submission.error {
                if let Some(error) = &error {
                    warn!("Water colour submission pending: {error}");
                }
            }
            let ocean_patches=current.ocean.as_ref().map_or(0,|owner|owner.draw_count());
            let draws = current
                .water_colour
                .as_ref()
                .map_or(0, |owner| owner.draw_count())
                +ocean_patches;
            if error.is_none() && draws > 0 && !water_submission.reported {
                info!(draws, ocean_patches, "Water colour submission active");
                water_submission.reported = true;
            }
            if error.is_some() {
                water_submission.reported = false;
            }
            water_submission.error = error;
        } else {
            current.water_colour = None;
            water_submission.error = None;
            water_submission.reported = false;
        }
        fog_source.record_composite(context.command_encoder(), target.main_texture_view());
        if let Err(error) = capture.record(
            &resources.capture_request,
            &resources.device,
            context.command_encoder(),
            fog_view,
            frame,
            current
                .water_depth
                .as_ref()
                .map_or(&current.depth_view, |owner| &owner.water.sample),
            &atlas_frame,
            atlas,
            &parallel.buffer,
            &current.fog,
            current.water_depth.as_ref(),
            resources.water_frame.as_ref().and_then(|f|f.0.as_ref()),
            underwater_active,
        ) {
            warn!("Fog numeric capture failed: {error}");
        }
        if let Some(parameters) = material_parameters {
            let mut parameters_value = FogMaterialParameters::ready(frame, fog_view);
            // Reserved backend flag lane selects the same retained curve bank
            // for material receivers without rebinding/copying an image per frame.
            parameters_value.world.flags.y = lookup_plan.final_image as u32;
            if !frame.areas.is_empty() {
                if let Some(shared) = &shared {
                    if let Some(objects) = &shared.object_frame {
                        if objects.camera != frame.camera || objects.revision != frame.revision {
                            return Err("alpha fog receivers belong to another frame".into());
                        }
                        let buffer = resources
                            .buffers
                            .get(&shared.objects)
                            .ok_or("alpha fog object buffer awaits GPU preparation")?;
                        objects.write(&resources.queue, &buffer.buffer)?;
                        parameters_value.control.w = objects.world_offset as f32;
                    }
                }
            }
            let bytes = bevy::render::storage::ShaderBuffer::from(parameters_value);
            resources
                .queue
                .write_buffer(&parameters.buffer, 0, bytes.data.as_ref().unwrap());
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            if preparation {
                gpu.depth_completed=depth_frame.filter(|_|depth_prepared);
                gpu.prepared_revision=depth_prepared.then_some((frame.camera,frame.revision));
                *last_error=None;
                return;
            }
            gpu.completed = resources.water_frame.as_ref().and_then(|f| f.0.as_ref())
                .map(|water| (frame.camera, water.matrices.frame));
            if !gpu.reported {
                info!(
                    world_fog = frame.parameters.flags.x & 24 != 0,
                    "Atmosphere command submission active"
                );
                gpu.reported = true;
            }
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!(preparation, "Atmosphere submission pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
