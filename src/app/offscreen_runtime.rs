//! Host-only image presentation for the normal application and render graph.
//! No Winit plugin, native window handle, swapchain, synthetic clock or GPU
//! completion is created. Loading, gameplay, audio and retirement stay owned by
//! the same production schedules as the visible client.
use bevy::{
    app::MainScheduleOrder,
    ecs::{schedule::ScheduleLabel, system::RunSystemOnce},
    prelude::*,
};
use std::{collections::HashMap, path::Path};

/// Rust drops fields in declaration order: consumers before session caches.
pub(crate) struct Runtime {
    pub(crate) app: App,
    _audio: super::audio::AudioSession,
    _catalog: super::launch::CatalogSession,
    terminal: Option<AppExit>,
}
impl Runtime {
    pub(super) fn new(
        app: App,
        audio: super::audio::AudioSession,
        catalog: super::launch::CatalogSession,
    ) -> Self {
        Self {
            app,
            _audio: audio,
            _catalog: catalog,
            terminal: None,
        }
    }
    pub(crate) fn open(
        base: &Path,
        level: &str,
        extent: UVec2,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut runtime = super::launch::build(
            base.to_owned(),
            level.to_owned(),
            super::launch::HostMode::Offscreen { extent },
        )?;
        // Match the normal runner's plugin-readiness boundary before finish.
        // This services initialization tasks, without advancing a game frame.
        while runtime.app.plugins_state() == bevy::app::PluginsState::Adding {
            bevy::tasks::tick_global_task_pools_on_main_thread();
            std::thread::yield_now();
        }
        runtime.app.finish();
        runtime.app.cleanup();
        Ok(runtime)
    }
    pub(crate) fn step(&mut self) -> Result<Option<AppExit>, String> {
        if self.terminal.is_some() {
            return Ok(self.terminal.clone());
        }
        redirect(self.app.world_mut());
        assert_offscreen(self.app.world_mut());
        // Real Time<Real>, gameplay delta, rendering, async mappings and the
        // ordinary ExitDrain all advance exactly once through App::update.
        self.app.update();
        // Keep the actual terminal acknowledgement even if the observation
        // below finds an error. A subsequent update must not age its message out.
        self.terminal = self.app.should_exit();
        super::audio::observation::poll(self.app.world_mut())?;
        assert_offscreen(self.app.world_mut());
        Ok(self.terminal.clone())
    }
    pub(crate) fn playing(&mut self) -> Result<bool, String> {
        self.app
            .world_mut()
            .run_system_once(super::loading_screen::is_playing)
            .map_err(|error| error.to_string())
    }
    pub(crate) fn request_exit(&mut self) {
        self.app.world_mut().write_message(AppExit::Success);
    }
    pub(crate) fn set_view_enabled(&mut self, enabled: bool) {
        let world = self.app.world_mut();
        world.resource_scope(|world, mut target: Mut<Target>| {
            if target.enabled == enabled {
                return;
            }
            target.enabled = enabled;
            if enabled {
                for (entity, (view, active)) in target.suspended.drain() {
                    if let Ok(mut entity) = world.get_entity_mut(entity) {
                        // Restore the exact source component, not a default view.
                        entity.insert(view);
                        if let Some(mut camera) = entity.get_mut::<Camera>() {
                            camera.is_active = active;
                        }
                    }
                }
            }
        });
        redirect(world);
    }
}

#[derive(Resource)]
struct Target {
    image: Handle<Image>,
    enabled: bool,
    suspended: HashMap<Entity, (Camera3d, bool)>,
}
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
struct ImagePresentation;

pub(super) fn install(app: &mut App, extent: UVec2) -> Result<(), String> {
    if extent.min_element() == 0 {
        return Err("offscreen extent must be positive".into());
    }
    super::audio::observation::enable(app.world_mut())?;
    super::resource_diagnostics::install_offscreen(app);
    let mut image = Image::new_target_texture(
        extent.x,
        extent.y,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.texture_descriptor.label = Some("offscreen final display target");
    let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(Target {
        image,
        enabled: true,
        suspended: HashMap::new(),
    });
    app.init_schedule(ImagePresentation)
        .add_systems(ImagePresentation, redirect);
    // Loading publishes cameras in Last. Resolve their presentation AFTER
    // those commands and before extraction, including the first loading view.
    app.world_mut()
        .resource_mut::<MainScheduleOrder>()
        .insert_after(Last, ImagePresentation);
    // Keep CameraUpdateSystems' ordinary extent derivation on the image path.
    app.add_systems(
        PostStartup,
        redirect.before(bevy::camera::CameraUpdateSystems),
    );
    app.add_systems(
        PostUpdate,
        redirect.before(bevy::camera::CameraUpdateSystems),
    );
    Ok(())
}

fn redirect(world: &mut World) {
    let Some(target) = world.get_resource::<Target>() else {
        return;
    };
    let image = target.image.clone();
    // Logical viewport metadata may change in the existing resize observer.
    // It is never attached to a native handle; the target asset is replaced by
    // that same dimensions change before camera extraction.
    let extent = world
        .query_filtered::<&Window, With<bevy::window::PrimaryWindow>>()
        .iter(world)
        .next()
        .map(|window| window.resolution.physical_size());
    if let Some(extent) = extent {
        if let Some(current) = world.resource::<Assets<Image>>().get(&image) {
            if current.texture_descriptor.size.width != extent.x
                || current.texture_descriptor.size.height != extent.y
            {
                let mut replacement = Image::new_target_texture(
                    extent.x,
                    extent.y,
                    bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
                    None,
                );
                replacement.texture_descriptor.label = Some("offscreen final display target");
                world
                    .resource_mut::<Assets<Image>>()
                    .insert(image.id(), replacement)
                    .expect("offscreen target identity remains owned");
            }
        }
    }
    world.resource_scope(|world, mut target: Mut<Target>| {
        for mut destination in world.query_filtered::<&mut bevy::camera::RenderTarget, With<Camera>>().iter_mut(world) {
            if !matches!(&*destination, bevy::camera::RenderTarget::Image(current) if current.handle.id() == image.id()) {
                *destination = bevy::camera::RenderTarget::Image(image.clone().into());
            }
        }
        if !target.enabled {
            // Production view publishers query Camera3d, not Camera.is_active.
            // Removing only that exact component creates a genuine input gap;
            // identity, transforms, projection and the remaining state survive.
            let views = world.query_filtered::<Entity, With<Camera3d>>().iter(world).collect::<Vec<_>>();
            for entity in views {
                let mut entity_mut = world.entity_mut(entity);
                let active = {
                    let mut camera = entity_mut.get_mut::<Camera>().expect("Camera3d owns Camera");
                    let active = camera.is_active;
                    camera.is_active = false;
                    active
                };
                let view = entity_mut.take::<Camera3d>().expect("queried Camera3d remains present");
                target.suspended.insert(entity, (view, active));
            }
        }
    });
    assert_offscreen(world);
}
fn assert_offscreen(world: &mut World) {
    assert!(
        world
            .query::<&bevy::window::RawHandleWrapper>()
            .iter(world)
            .next()
            .is_none(),
        "offscreen runtime acquired a native window handle"
    );
    for target in world
        .query_filtered::<&bevy::camera::RenderTarget, With<Camera>>()
        .iter(world)
    {
        assert!(
            matches!(target, bevy::camera::RenderTarget::Image(_)),
            "offscreen camera must target an image before update/extraction"
        );
    }
}
