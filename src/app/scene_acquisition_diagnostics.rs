//! Actual scene-worker resource acquisition without a window or game frames.
//! No application runner/update is invoked: queued entities and audio never play.
use super::*;
use bevy::{ecs::system::RunSystemOnce, prelude::*};

pub fn inspect_scene_preparation(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, false)
}

pub fn inspect_scene_publication(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, true)
}

fn track_assets<A: bevy::asset::Asset>(world: &mut World) -> Result<(), std::io::Error> {
    world
        .run_system_once(Assets::<A>::track_assets)
        .map_err(|error| {
            std::io::Error::other(format!(
                "{} handle retirement: {error:?}",
                std::any::type_name::<A>()
            ))
        })
}

fn resource_snapshot(world: &World, step: usize, phase: &str) {
    eprintln!(
        "RESOURCE_SCENE_ASSETS step={step} phase={phase} meshes={} images={} probes={} decals={} skies={} buffers={}",
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<Image>>().len(),
        world.resource::<Assets<ProbeMaterial>>().len(),
        world.resource::<Assets<DecalMaterial>>().len(),
        world
            .resource::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>()
            .len(),
        world.resource::<Assets<ShaderBuffer>>().len()
    );
    for (name, bytes) in crate::platform::allocation_tags::report() {
        eprintln!("RESOURCE_SCENE_HEAP step={step} phase={phase} tag={name:?} bytes={bytes}");
    }
    super::resource_diagnostics::allocator_snapshot(world.resource::<RenderDevice>(), phase, step);
}

fn inspect_scene_resources(
    base: &std::path::Path,
    levels: &[String],
    publish: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if levels.is_empty() {
        return Err("inspect-scene-preparation requires level names".into());
    }
    let catalog = archive::Catalog::shared(base)?;
    let catalog_session = super::launch::CatalogSession(catalog.identity());
    let session = super::audio::AudioSession::begin()?;
    for (step, name) in levels.iter().enumerate() {
        let level = crate::assets::level::LevelResources::resolve(&catalog, name)?;
        if let Some(map) = level.entities.strip_suffix(".entities") {
            crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
        }
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=begin");
        // A fresh host per sample avoids claiming reuse of unpublished assets.
        // The real worker prepares its normal asset batches; dropping the
        // unpublished result observes completed-but-unpublished acquisition
        // retirement; it does not simulate mid-read cancellation or game frames.
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
        let resident = app
            .world()
            .resource::<super::resident_images::ResidentImages>()
            .clone();
        app.insert_resource(super::resident_images::UiImages::new(resident));
        resource_snapshot(app.world(), step, "before-preparation");
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
        resource_snapshot(app.world(), step, "prepared");
        if publish {
            let (mut lifetime, before) =
                super::level_lifetime::LevelLifetime::begin(app.world_mut(), catalog.clone());
            prepared.publish(app.world_mut());
            lifetime.published(app.world(), &before);
            resource_snapshot(app.world(), step, "published");
            // No render/gameplay update has run, so no outgoing GPU frame
            // exists to acknowledge. Use the actual main-world teardown.
            lifetime.clear(app.world_mut())?;
            resource_snapshot(app.world(), step, "level-cleared");
            // These are the registered asset owner's real handle-drop
            // consumers, in dependency order; no application schedule runs.
            track_assets::<ProbeMaterial>(app.world_mut())?;
            track_assets::<DecalMaterial>(app.world_mut())?;
            track_assets::<crate::rendering::bevy_backend::sky_mask::SkyMaterial>(app.world_mut())?;
            track_assets::<Mesh>(app.world_mut())?;
            track_assets::<ShaderBuffer>(app.world_mut())?;
            track_assets::<Image>(app.world_mut())?;
            resource_snapshot(app.world(), step, "asset-handles-tracked");
        } else {
            drop(prepared);
        }
        drop(app);
        for (tag, bytes) in crate::platform::allocation_tags::report() {
            eprintln!(
                "RESOURCE_SCENE_HEAP step={step} phase=host-released tag={tag:?} bytes={bytes}"
            );
        }
        catalog.release_level_resources();
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=released");
    }
    drop(session);
    drop(catalog);
    drop(catalog_session);
    for (tag, bytes) in crate::platform::allocation_tags::report() {
        eprintln!("RESOURCE_SCENE_HEAP phase=session-released tag={tag:?} bytes={bytes}");
    }
    Ok(())
}
