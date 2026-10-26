//! Gameplay camera effects run after transparency and before exposure/bloom.
use super::{
    auto_exposure::{ExposureParameters, ExposurePass, MaterialExposure},
    camera_view::{CameraAlphaViewInputs, CameraView, CameraViewConfig, CameraViewInputs},
    display_scene::DisplaySceneFrame,
    fog::FogSceneFrame,
    fog_scene::FogGpu,
    water_scene::WaterSceneFrame,
};
use bevy::{
    core_pipeline::{Core3d, Core3dSystems},
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_asset::RenderAssets,
        render_resource::{Texture, TextureView},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        storage::GpuShaderBuffer,
        sync_world::MainEntity,
        texture::GpuImage,
        view::{ExtractedView, ViewTarget},
        RenderApp,
    },
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CameraPass;

#[derive(Resource, Default)]
pub(super) struct CameraGpu {
    current: Option<(Entity, CameraView)>,
    completed: Option<(Entity, u32)>,
    screen_effects: super::screen_effects::ScreenEffects,
}
impl CameraGpu {
    pub(super) fn capture_planes(
        &self,
        camera: Entity,
        frame: u32,
    ) -> Vec<(&'static str, &Texture)> {
        if self.completed != Some((camera, frame)) {
            return Vec::new();
        }
        let mut planes = self
            .current
            .as_ref()
            .map_or_else(Vec::new, |(_, v)| v.capture_planes());
        planes.extend(self.screen_effects.capture_planes());
        planes
    }
    pub(super) fn half_colour(&self, camera: Entity, frame: u32) -> Option<&TextureView> {
        (self.completed == Some((camera, frame)))
            .then(|| self.current.as_ref().map(|(_, v)| v.half_colour()))
            .flatten()
    }
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<super::screen_effects::ScreenEffectMaterials>();
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::screen_effects::ScreenEffectMaterials,
    >::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<CameraGpu>().add_systems(
            Core3d,
            render_camera
                .in_set(Core3dSystems::PostProcess)
                .in_set(CameraPass)
                .before(ExposurePass),
        );
    }
}
fn render_camera(
    screen_sources: Res<super::screen_effects::ScreenEffectMaterials>,
    display: Res<DisplaySceneFrame>,
    water: Res<WaterSceneFrame>,
    fog_frame: Res<FogSceneFrame>,
    fog: Res<FogGpu>,
    alpha: Res<super::alpha_depth_scene::AlphaGpu>,
    clear: Res<ClearColor>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    mut gpu: ResMut<CameraGpu>,
    mut context: RenderContext,
    view: ViewQuery<(
        &MainEntity,
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &MaterialExposure,
        &ExposureParameters,
    )>,
    mut last_error: Local<Option<String>>,
) {
    gpu.completed = None;
    let Some(frame) = display.0.as_ref() else {
        gpu.current = None;
        return;
    };
    let Some(lens) = frame.lens.as_ref() else {
        return;
    };
    if lens.settings.skip_post_process {
        return;
    }
    let (main, view, camera, target, exposure, exposure_parameters) = view.into_inner();
    if main.id() != frame.camera {
        return;
    }
    let result = (|| -> Result<(), String> {
        if !camera.hdr
            || target.main_texture().sample_count() != 1
            || camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position != UVec2::ZERO)
        {
            return Err("camera effects require a resolved full gameplay view".into());
        }
        let full = [
            target.main_texture().width(),
            target.main_texture().height(),
        ];
        let water = water
            .0
            .as_ref()
            .filter(|w| w.camera == frame.camera)
            .ok_or("camera view time awaits preparation")?;
        let (depth, linear) = fog
            .camera_depth(frame.camera, water.matrices.frame)
            .ok_or("camera depth awaits completed atmosphere")?;
        let effects = frame
            .camera_effects
            .as_ref()
            .ok_or("camera settings await preparation")?;
        let resources = frame
            .camera_blur
            .as_deref()
            .ok_or("camera materials await preparation")?;
        let exposure = buffers
            .get(&exposure.0)
            .ok_or("camera adaptation awaits preparation")?;
        let black = images
            .get(&lens.black)
            .ok_or("camera default image awaits preparation")?;
        let config = CameraViewConfig {
            extent: full,
            reduced_extent: [depth.alpha.reduced.width(), depth.alpha.reduced.height()],
            host_format: target.main_texture_format(),
            depth_format: depth.alpha.reduced.format(),
            compressed: lens.settings.hdr_buffer_format != 1,
        };
        if !lens.settings.skip_depth_of_field
            && gpu
                .current
                .as_ref()
                .is_none_or(|(id, v)| *id != frame.camera || !v.matches(config))
        {
            gpu.current = Some((frame.camera, CameraView::new(&device, config, resources)?));
        }
        let inverse = view.clip_from_view.inverse();
        let scattering = fog_frame
            .0
            .as_ref()
            .filter(|f| f.camera == frame.camera)
            .ok_or("camera scattering parameters await preparation")?
            .parameters
            .scattering_color_intensity
            .truncate()
            .to_array();
        let write = (!lens.settings.skip_depth_of_field).then(|| target.post_process_write());
        if let Some(write) = &write {
            let result = gpu.current.as_mut().unwrap().1.record(
                &device,
                &queue,
                context.command_encoder(),
                CameraViewInputs {
                    effects,
                    resources,
                    colour: write.source,
                    depth: &depth.alpha.full_sample,
                    alpha_depth: &depth.alpha.reduced_sample,
                    alpha_attachment: &depth.alpha.reduced_attachment,
                    linear_water_depth: &linear.view,
                    linear_depth_extent: [linear.texture.width(), linear.texture.height()],
                    adaptation: &exposure.buffer,
                    black: &black.texture_view,
                    camera_exposure: camera.exposure,
                    adapt_max: exposure_parameters.range.y,
                    white_level: lens.white_level,
                    projection: view.clip_from_view.to_cols_array_2d(),
                    inverse_z: [inverse.z_axis.w, inverse.w_axis.w, inverse.w_axis.z],
                    local_time: water.local_time,
                    scattering_colour: scattering,
                    destination: write.destination,
                    alpha: alpha.opacity(frame.camera, water.matrices.frame).map(
                        |(opacity, special_alpha)| CameraAlphaViewInputs {
                            refraction: opacity,
                            full_depth_attachment: &depth.alpha.full_attachment,
                            reduction_shift: lens.settings.downsample as u32,
                            clear_colour: match camera.clear_color {
                                bevy::camera::ClearColorConfig::Custom(c) => {
                                    c.to_linear().to_f32_array()[..3].try_into().unwrap()
                                }
                                _ => clear.0.to_linear().to_f32_array()[..3].try_into().unwrap(),
                            },
                            special_alpha,
                        },
                    ),
                },
            );
            if result.is_err() {
                //Recording failed before combine. Restore the readable HDR selector;
                //partial scratch writes must never become the displayed frame.
                let _ = target.post_process_write();
            }
            result?;
        } else {
            gpu.current = None;
        }
        let colour = write
            .as_ref()
            .map_or_else(|| target.main_texture_view(), |w| w.destination);
        if let Some(sources) = screen_sources.0.as_deref() {
            let underwater = water
                .head_water_surface
                .is_some_and(|height| water.matrices.current.position[2] <= height);
            gpu.screen_effects.record(
                &device,
                &queue,
                context.command_encoder(),
                sources,
                water,
                underwater,
                colour,
                &depth.alpha.full_sample,
                full,
                lens.settings.downsample as u32,
                config.compressed,
                config.host_format,
                camera.exposure,
                lens.white_level,
                &exposure.buffer,
                [inverse.z_axis.w, inverse.w_axis.w, inverse.w_axis.z],
            )?;
        }
        gpu.completed = Some((frame.camera, water.matrices.frame));
        Ok(())
    })();
    match result {
        Ok(()) => {
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Camera effects submission pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
