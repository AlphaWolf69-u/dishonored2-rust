//! Gameplay display owner. It replaces the backend tone pass only after this
//! view's real colour-cube and exposure resources are prepared.
use super::{
    auto_exposure::{ExposurePass, MaterialExposure},
    colour_cube::{ColourCubePass, CubeOverrides},
    lens_scene::{LensImages, LensView},
    luminance::{LuminancePipelines, LuminanceViews},
    tone_mapping::{ToneMappingParameters, ToneMappingPass},
};
use bevy::{
    core_pipeline::{
        tonemapping::{tonemapping, Tonemapping},
        Core3d, Core3dSystems,
    },
    prelude::*,
    render::{
        camera::ExtractedCamera,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{TextureFormat, TextureViewId},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        storage::GpuShaderBuffer,
        sync_world::MainEntity,
        texture::GpuImage,
        view::ViewTarget,
        Render, RenderApp, RenderSystems,
    },
};

#[derive(Clone)]
pub struct DisplayCubeOverrides {
    /// Resident archive tables; their contents are immutable for each view ID.
    pub images: [Handle<Image>; 4],
    pub weights: Vec4,
}
#[derive(Clone)]
pub struct DisplayFrame {
    pub camera: Entity,
    pub camera_effects: Option<crate::rendering::camera_effects::CameraEffectsFrame>,
    pub parameters: ToneMappingParameters,
    /// Resident archive table; replacement produces a new GPU texture view.
    pub cube: Handle<Image>,
    pub weight: f32,
    pub custom_ev: f32,
    pub overrides: Option<DisplayCubeOverrides>,
    /// Resident authored lens inputs. GPU producer submission remains separate
    /// until all view resources are available; preparation does not alter tone.
    pub lens: Option<super::lens_scene::LensFrame>,
}
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct DisplaySceneFrame(pub Option<DisplayFrame>);
pub struct DisplayPlugin;
impl Plugin for DisplayPlugin {
    fn build(&self, app: &mut App) {
        super::display_capture::register(app);
        app.init_resource::<DisplaySceneFrame>()
            .add_plugins(ExtractResourcePlugin::<DisplaySceneFrame>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<DisplayGpu>()
                .add_systems(
                    Render,
                    retire_display.in_set(RenderSystems::PrepareResources),
                )
                .add_systems(
                    Core3d,
                    render_display
                        .in_set(Core3dSystems::PostProcess)
                        .after(ExposurePass)
                        .before(tonemapping),
                );
        }
    }
}
#[derive(Resource, Default)]
struct DisplayGpu {
    current: Option<DisplayView>,
}
struct DisplayView {
    camera: Entity,
    format: TextureFormat,
    cubes: ColourCubePass,
    display: ToneMappingPass,
    lens: Option<LensView>,
    last_cubes: Option<CubeInputIds>,
    #[cfg(test)]
    cube_submissions: u64,
}
#[derive(PartialEq, Eq)]
struct CubeInputIds {
    source: TextureViewId,
    weight: u32,
    overrides: Option<([TextureViewId; 4], [u32; 4])>,
}

fn retire_display(
    mut gpu: ResMut<DisplayGpu>,
    frame: Res<DisplaySceneFrame>,
    cameras: Query<&MainEntity, With<ExtractedCamera>>,
) {
    if gpu.current.as_ref().is_some_and(|v| {
        frame.0.as_ref().is_none_or(|f| f.camera != v.camera)
            || !cameras.iter().any(|main| main.id() == v.camera)
    }) {
        gpu.current = None;
    }
}

fn render_display(
    frame: Res<DisplaySceneFrame>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    mut gpu: ResMut<DisplayGpu>,
    mut pyramids: ResMut<LuminanceViews>,
    pyramid_programs: Res<LuminancePipelines>,
    mut context: RenderContext,
    view: ViewQuery<(
        Entity,
        &MainEntity,
        &ViewTarget,
        &ExtractedCamera,
        &MaterialExposure,
        &mut Tonemapping,
    )>,
    mut last_error: Local<Option<String>>,
    mut last_lens_error: Local<Option<String>>,
    capture_request: Res<super::display_capture::DisplayCaptureRequest>,
    mut capture: ResMut<super::display_capture::DisplayCaptureGpu>,
) {
    let Some(frame) = &frame.0 else {
        return;
    };
    let (entity, main, target, camera, exposure, mut tonemapping_mode) = view.into_inner();
    if main.id() != frame.camera {
        return;
    }
    let result = (|| -> Result<(), String> {
        if !camera.hdr || !camera.exposure.is_finite() || camera.exposure <= 0. {
            return Err("display requires this view's valid HDR exposure".into());
        }
        if camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position != UVec2::ZERO)
            || target.main_texture().sample_count() != 1
        {
            return Err("display requires a resolved full camera target".into());
        }
        let cube = images
            .get(&frame.cube)
            .ok_or("display colour cube awaits GPU preparation")?;
        let exposure = buffers
            .get(&exposure.0)
            .ok_or("display adaptation state awaits GPU preparation")?;
        let overlay = frame
            .overrides
            .as_ref()
            .map(|overrides| {
                let views = overrides
                    .images
                    .iter()
                    .map(|image| {
                        images
                            .get(image)
                            .map(|v| &v.texture_view)
                            .ok_or("display overlay colour cube awaits GPU preparation".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, String>(CubeOverrides {
                    views: views.try_into().map_err(|_| "display overlay slot count")?,
                    weights: overrides.weights,
                })
            })
            .transpose()?;
        let format = target.main_texture_format();
        if gpu
            .current
            .as_ref()
            .is_none_or(|v| v.camera != frame.camera || v.format != format)
        {
            gpu.current = Some(DisplayView {
                camera: frame.camera,
                format,
                cubes: ColourCubePass::new(&device, &queue),
                display: ToneMappingPass::new(&device, format),
                lens: None,
                last_cubes: None,
                #[cfg(test)]
                cube_submissions: 0,
            });
        }
        let current = gpu.current.as_mut().unwrap();
        let cube_ids = CubeInputIds {
            source: cube.texture_view.id(),
            weight: frame.weight.to_bits(),
            overrides: overlay.as_ref().map(|v| {
                (
                    v.views.map(|v| v.id()),
                    v.weights.to_array().map(f32::to_bits),
                )
            }),
        };
        // These source assets are immutable. Repeating this blend would write
        // identical RGBA8 texels, so retain the prepared table until inputs
        // change. GPU adaptation remains live in the full-screen pass.
        if current.last_cubes.as_ref() != Some(&cube_ids) {
            current.cubes.record(
                &device,
                &queue,
                context.command_encoder(),
                &cube.texture_view,
                frame.weight,
                overlay,
            );
            current.last_cubes = Some(cube_ids);
            #[cfg(test)]
            {
                current.cube_submissions += 1;
            }
        }
        let mut parameters = frame.parameters;
        parameters.exposure =
            parameters.exposure * 2.0_f32.powf(-frame.custom_ev) / (139.26 * camera.exposure);
        let full = [
            target.main_texture().width(),
            target.main_texture().height(),
        ];
        let lens_inputs = frame
            .lens
            .as_ref()
            .map(|lens| -> Result<LensImages<'_>, String> {
                if lens.settings.skip_post_process {
                    return Err("post-processing bypass is not yet connected".into());
                }
                if !(139.26 * camera.exposure).is_finite() {
                    return Err("lens host radiance multiplier is invalid".into());
                }
                let inputs = LensImages::resolve(lens, &images)?;
                let pyramid = pyramids.prepare(
                    &device,
                    &queue,
                    entity,
                    UVec2::from_array(full),
                    lens.settings.downsample,
                )?;
                let owner = current
                    .lens
                    .get_or_insert_with(|| LensView::new(&device, format));
                owner.prepare(&device, lens, pyramid, full)?;
                Ok(inputs)
            })
            .transpose();
        let lens_inputs = match lens_inputs {
            Ok(inputs) => {
                if inputs.is_none() {
                    current.lens = None;
                    *last_lens_error = None;
                }
                inputs
            }
            Err(error) => {
                if last_lens_error.as_ref() != Some(&error) {
                    warn!("Lens pass pending: {error}");
                    *last_lens_error = Some(error);
                }
                None
            }
        };
        let capture_source = capture_request
            .is_pending()
            .then(|| target.main_texture().clone());
        let post = target.post_process_write();
        let lens_drawn = if let (Some(inputs), Some(lens)) = (lens_inputs, frame.lens.as_ref()) {
            match current.lens.as_mut().unwrap().record(
                &device,
                &queue,
                context.command_encoder(),
                lens,
                inputs,
                pyramids.get_mut(entity).unwrap(),
                &pyramid_programs,
                full,
                camera.exposure,
                &parameters,
                post.source,
                current.cubes.view(),
                &exposure.buffer,
                post.destination,
            ) {
                Ok(()) => {
                    *last_lens_error = None;
                    true
                }
                Err(error) => {
                    if last_lens_error.as_ref() != Some(&error) {
                        warn!("Lens pass pending: {error}");
                        *last_lens_error = Some(error);
                    }
                    false
                }
            }
        } else {
            false
        };
        if lens_drawn {
            if let Some(source) = capture_source.as_ref() {
                let lens = frame.lens.as_ref().unwrap();
                let mut planes = vec![("hdr", source)];
                if let Some(pyramid) = pyramids.get_mut(entity) {
                    planes.push(("luminance", pyramid.texture()));
                }
                if let Some(bloom) = current.lens.as_ref().and_then(|owner| owner.bloom_texture()) {
                    planes.push(("bloom", bloom));
                }
                let details = format!("camera={:?} camera_exposure={} settings={:?}\nveil_size={} veil_strength={} dirt_strength={} dirt_blend={} use_default_dirt={} white_level={} camera_axis={}\ntone_bytes={:?}\ncamera_effects={:?}\n", frame.camera,camera.exposure,lens.settings,lens.veil_size,lens.veil_strength,lens.dirt_strength,lens.dirt.weight,lens.use_default_dirt,lens.white_level,lens.camera_axis,parameters.uniform_bytes(),frame.camera_effects);
                if let Err(error) = capture.record(
                    &capture_request, &device, context.command_encoder(), &planes,
                    &exposure.buffer, details,
                ) {
                    warn!("Display numeric capture failed: {error}");
                }
            }
        }
        if !lens_drawn {
            current.display.record(
                &device,
                &queue,
                context.command_encoder(),
                &parameters,
                post.source,
                current.cubes.view(),
                &exposure.buffer,
                post.destination,
            );
        }
        // Extracted from the unchanged main-world setting again next frame.
        // The backend pass returns immediately for None; never apply both.
        *tonemapping_mode = Tonemapping::None;
        Ok(())
    })();
    match result {
        Ok(()) => *last_error = None,
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Display pass pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "display_scene/tests.rs"]
mod tests;
