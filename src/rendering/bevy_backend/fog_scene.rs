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
        RenderApp,
    },
};

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<FogGpu>().add_systems(
            Core3d,
            render_atmosphere
                .in_set(AtmospherePass)
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );
    }
}
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AtmospherePass;
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
    water_materials: Option<Res<'w, super::water_scene::WaterMaterials>>,
    water_meshes: Option<Res<'w, super::water_scene::WaterWorldMeshes>>,
    depth_programs: Option<Res<'w, super::tiled_lighting_gpu::TlfPipelines>>,
    local_fog: Res<'w, super::fog_light_scattering::LocalFogFrame>,
    local_shadows: Option<Res<'w, super::shadow_pass::LocalShadowSceneFrame>>,
    local_completion: Option<Res<'w, super::shadow_pass::LocalShadowCompletion>>,
}
#[derive(Resource, Default)]
struct FogGpu {
    current: Option<ViewFog>,
    reported: bool,
}
struct ViewFog {
    tiled: Option<super::tiled_lighting_pass::TiledLightingPass>,
    scattering: Option<super::fog_light_scattering::LightScatteringPass>,
    camera: Entity,
    water_depth: Option<super::water_depth_scene::WaterDepthScene>,
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

fn render_atmosphere(
    frame: Res<FogSceneFrame>,
    resources: Resources,
    shared: Option<Res<FogMaterialResources>>,
    views: Query<
        (
            &MainEntity,
            &ViewTarget,
            &ViewDepthTexture,
            &ExtractedView,
            &ExtractedCamera,
        ),
        With<super::fog::FogCamera>,
    >,
    skies: Query<&SkyInstance>,
    interiors: Query<&InteriorInstance>,
    waters: Query<&super::water_scene::WaterInstance>,
    clock: Res<crate::gameplay::game_clock_runtime::GameplayViewClock>,
    sampling: Option<Res<super::shadow_sampling::ParallelSamplingBuffer>>,
    atlas_frame: Res<super::shadow_pass::ShadowSceneFrame>,
    atlas_gpu: Res<super::shadow_pass::ShadowSceneGpu>,
    mut capture: ResMut<super::fog_capture::FogCaptureGpu>,
    mut gpu: ResMut<FogGpu>,
    mut context: RenderContext,
    mut last_error: Local<Option<String>>,
) {
    let material_parameters = shared
        .as_ref()
        .and_then(|shared| resources.buffers.get(&shared.parameters));
    if let Some(parameters) = material_parameters {
        // Queue readiness only after all same-view texture passes are recorded.
        // A pending/removed atmosphere must not leave yesterday's fog enabled.
        resources
            .queue
            .write_buffer(&parameters.buffer, 0, &[0; 16]);
    }
    let Some(frame) = &frame.0 else {
        return;
    };
    let result = (|| -> Result<(), String> {
        let (main, target, depth, view, camera) =
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
        if let Some(sun) = &frame.sun_sprite {
            if let Some(projection) = sun.parameters.project(
                fog_view.clip_from_game_world(),
                frame.settings.far_clip,
                full,
            )? {
                let image = resources
                    .images
                    .get(&sun.image)
                    .ok_or("sun sprite image awaits GPU preparation")?;
                let uniform = FogSunMaskUniform::sprite(sun.parameters, projection, fog_view, full);
                if current
                    .sun_sprite
                    .as_ref()
                    .is_none_or(|pass| pass.state != sun.state)
                {
                    let program = current
                        .sun_program
                        .get_or_insert_with(|| FogSunProgram::new(&resources.device));
                    current.sun_sprite = Some(SunSpritePass::new(
                        &resources.device,
                        program,
                        sun.state,
                        target.main_texture_format(),
                        depth.texture.format(),
                        uniform,
                    )?);
                }
                let pass = current.sun_sprite.as_mut().unwrap();
                pass.prepare(
                    &resources.device,
                    &resources.queue,
                    uniform,
                    &image.texture_view,
                );
                pass.record(
                    context.command_encoder(),
                    target.main_texture_view(),
                    depth.view(),
                    sun.parameters.additive,
                );
            }
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
        if frame.parameters.flags.x & 24 == 0 && frame.areas.is_empty() && !scattering_enabled {
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
        current.interiors.prepare(
            &resources.device,
            &resources.queue,
            current.fog.pipelines(),
            interiors.iter(),
            &resources.allocator,
            &resources.meshes,
        )?;
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
            owner.prepare(
                &resources.device,
                &resources.queue,
                waters.iter(),
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
            )?;
            owner.record(&resources.device, context.command_encoder(), &depth.texture)?;
        } else {
            current.water_depth = None;
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
        current.fog.record(
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
                ray_mask: None,
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
            target.main_texture_view(),
        );
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
                warn!("Atmosphere submission pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
