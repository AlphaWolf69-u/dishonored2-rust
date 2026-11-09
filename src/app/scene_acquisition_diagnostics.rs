//! Actual scene-worker resource acquisition without a window or game frames.
//! No application runner/update is invoked: queued entities and audio never play.
use super::*;
use bevy::prelude::*;

pub fn inspect_scene_preparation(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if levels.is_empty() {
        return Err("inspect-scene-preparation requires level names".into());
    }
    let catalog = Arc::new(archive::Catalog::open(base)?);
    let session = super::audio::AudioSession::begin()?;
    for (step, name) in levels.iter().enumerate() {
        let level = crate::assets::level::LevelResources::resolve(&catalog, name)?;
        if let Some(map) = level.entities.strip_suffix(".entities") {
            crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
        }
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=begin");
        // A fresh host per sample avoids claiming reuse of unpublished assets.
        // The real worker prepares its normal asset batches; dropping the
        // unpublished result also exercises cancellation/abandoned acquisition.
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                        bevy::render::settings::WgpuSettings {
                            backends: cfg!(target_os = "windows")
                                .then_some(bevy::render::settings::Backends::DX12),
                            features: bevy::render::settings::WgpuSettings::default().features
                                | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                            ..default()
                        },
                    )),
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.add_plugins(MaterialPlugin::<ProbeMaterial>::default())
            .add_plugins(MaterialPlugin::<DecalMaterial>::default())
            .add_plugins(crate::rendering::bevy_backend::sky_mask::SkyAssetsPlugin)
            .add_plugins(super::audio::SoundPlugin)
            .init_resource::<super::texture_streaming::SharedImages>()
            .init_resource::<super::resident_images::ResidentImages>();
        app.finish();
        app.cleanup();
        let mut prepared = super::level_prepare::prepare_level(
            base.to_owned(),
            catalog.clone(),
            level,
            true,
            || false,
        )?;
        super::scene_preparation::ScenePreparation::new(app.world_mut()).prepare(&mut prepared)?;
        let inputs = prepared
            .get_resource::<super::scene_preparation::SceneGpuInputs>()
            .ok_or("scene worker did not produce its real asset inputs")?;
        eprintln!(
            "RESOURCE_SCENE_PREPARATION step={step} level={name} phase=prepared images={} meshes={} buffers={}",
            inputs.images.len(),
            inputs.meshes.len(),
            inputs.buffers.len()
        );
        drop(prepared);
        drop(app);
        catalog.release_level_resources();
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=released");
    }
    drop(catalog);
    drop(session);
    Ok(())
}
