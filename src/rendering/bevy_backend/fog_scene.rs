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
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );
    }
}
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
}
#[derive(Resource, Default)]
struct FogGpu {
    current: Option<ViewFog>,
    reported: bool,
}
struct ViewFog {
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
    interiors: InteriorMeshes,
    snapshot: Option<FogDepthSnapshot>,
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
            current.hdr_format != target.main_texture_format()
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
        // Sky also owns visible colour when this environment disables world fog.
        if frame.parameters.flags.x & 24 == 0 && frame.areas.is_empty() {
            return Ok(());
        }
        let curve = resources
            .images
            .get(&frame.curve)
            .ok_or("fog LUT awaits GPU preparation")?;
        let noise = resources
            .images
            .get(&frame.noise)
            .ok_or("fog noise awaits GPU preparation")?;
        let projected_height=resources.images.get(&frame.shadow.image)
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
                shadow_parameters:&super::fog_shadow::FogShadowParameters {
                    viewport:[0.,0.,extents.shadow.x as f32,extents.shadow.y as f32],
                    resolution_projected:[1.,1.,frame.shadow.extent,0.],
                    jitter:[fog_view.jitter.z,fog_view.jitter.w,0.,0.],
                    options:[u32::from(frame.shadow.projected),u32::from(frame.shadow.temporal),1,0],
                },
                projected_height:&projected_height.texture_view,
                scattering: None,
                areas: if frame.areas.is_empty() {
                    None
                } else {
                    current.areas.as_ref()
                },
                settings: frame.settings,
                ray_mask: None,
                shadowed_volumes: false,
                depth: &current.depth_view,
                sky_mask: &current.mask.view,
                curve: &curve.texture_view,
                noise: &noise.texture_view,
                atlas: &atlas.texture_view,
                parallel: &parallel.buffer,
            },
            &draws,
            target.main_texture_view(),
        );
        if let Err(error) = capture.record(
            &resources.capture_request, &resources.device, context.command_encoder(),
            fog_view, frame, &current.depth_view, &atlas_frame, atlas,
            &parallel.buffer, &current.fog,
        ) {
            warn!("Fog numeric capture failed: {error}");
        }
        if let Some(parameters) = material_parameters {
            let mut parameters_value = FogMaterialParameters::ready(frame, fog_view);
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
