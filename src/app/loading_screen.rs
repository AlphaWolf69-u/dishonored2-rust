//! bevy_backend: one device/window for engine bootstrap, map acquisition and play.
//! The early image draw follows 140369120; map acquisition is a distinct
//! phase (140062ce0), and readiness comes from the resource owners' gates.
use crate::assets::{archive::Catalog, level::LevelResources};
use bevy::{
    ecs::{
        schedule::{ConditionWithAccess, ScheduleLabel, Schedules as BevySchedules, SystemKey},
        system::IntoSystem,
    },
    prelude::*,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

static REQUESTS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static REQUEST_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Host entry for a map command/menu selection. The same application owns
/// both worlds; invalid requests leave the current world running.
pub fn request_level(name: impl Into<String>) {
    REQUESTS.lock().unwrap().push_back(name.into());
    REQUEST_PENDING.store(true, std::sync::atomic::Ordering::Release);
}

#[derive(Default)]
pub(super) struct PreparedLevel {
    resources: Vec<PreparedResource>,
    publications: Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>,
    diagnostic_exit: bool,
}
impl PreparedLevel {
    pub(super) fn diagnostic_exit() -> Self {
        Self {
            resources: Vec::new(),
            publications: Vec::new(),
            diagnostic_exit: true,
        }
    }
    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.resources.push(PreparedResource {
            kind: std::any::TypeId::of::<R>(), value: Box::new(resource),
            publish: |value, world| { world.insert_resource(*value.downcast::<R>().unwrap()); },
        });
        self
    }
    pub(super) fn take_resource<R: Resource>(&mut self) -> Option<R> {
        let index = self.resources.iter().position(|resource| resource.kind == std::any::TypeId::of::<R>())?;
        Some(*self.resources.swap_remove(index).value.downcast::<R>().unwrap())
    }
    pub(super) fn on_publish(&mut self, publish: impl FnOnce(&mut World) + Send + Sync + 'static) {
        self.publications.push(Box::new(publish));
    }
    fn publish(self, world: &mut World) {
        for resource in self.resources { (resource.publish)(resource.value, world); }
        for publish in self.publications { publish(world); }
    }
}

struct PreparedResource {
    kind: std::any::TypeId,
    value: Box<dyn std::any::Any + Send + Sync>,
    publish: fn(Box<dyn std::any::Any + Send + Sync>, &mut World),
}

/// Capture the host systems before registering map-dependent plugins. The
/// host's input, asset uploads, window and renderer keep running while those
/// plugins wait. Add conditions to system nodes, preserving all their existing
/// ordering against Bevy's transform/visibility/asset schedules.
pub(super) struct Schedules {
    systems: HashMap<Box<dyn ScheduleLabel>, HashSet<SystemKey>>,
    startup: Option<Schedule>,
    render: HashMap<Box<dyn ScheduleLabel>, HashSet<SystemKey>>,
}
impl Schedules {
    pub fn capture(app: &mut App) -> Self {
        let mut schedules = app.world_mut().resource_mut::<BevySchedules>();
        let systems = schedules
            .iter()
            .map(|(label, schedule)| {
                (
                    label.dyn_clone(),
                    schedule
                        .graph()
                        .systems
                        .iter()
                        .map(|(key, _, _)| key)
                        .collect(),
                )
            })
            .collect();
        let startup = schedules.remove(Startup);
        drop(schedules);
        let render = app
            .get_sub_app_mut(bevy::render::RenderApp)
            .map(|render| {
                render
                    .world()
                    .resource::<BevySchedules>()
                    .iter()
                    .map(|(label, schedule)| {
                        (
                            label.dyn_clone(),
                            schedule
                                .graph()
                                .systems
                                .iter()
                                .map(|(key, _, _)| key)
                                .collect(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            systems,
            startup,
            render,
        }
    }
}

#[derive(Resource)]
pub(super) struct Acquisition {
    pub(super) catalog: Arc<Catalog>,
    base: PathBuf,
    input: Option<(Arc<Catalog>, LevelResources, PathBuf)>,
    result: Option<Mutex<mpsc::Receiver<Result<PreparedLevel, String>>>>,
    frames: u32,
    startup: Option<Schedule>,
    published: bool,
    movie: Option<Mutex<mpsc::Receiver<Result<super::loading_movie::Frame, String>>>>,
    movie_visible: bool,
    map: String,
    tick: f64,
    finished: bool,
    initial_load: bool,
    render_finished: bool,
    continued: bool,
    debug_continued: bool,
    completion_visible: bool,
    gpu: GpuReadiness,
    pending: Vec<&'static str>,
    lifetime: super::level_lifetime::LevelLifetime,
    next: Option<LevelResources>,
    debug_sequence: VecDeque<String>,
    debug_sequence_active: bool,
    played_seconds: f64,
    last_loading_update: std::time::Instant,
    max_loading_gap: std::time::Duration,
    movie_max_gap: std::time::Duration,
    upload_budget: Option<usize>,
}
#[derive(Component)]
struct LoadingCamera;
#[derive(Component)]
struct LoadingBackground;

pub(super) fn install(
    app: &mut App,
    baseline: Schedules,
    catalog: Arc<Catalog>,
    level: LevelResources,
    base: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut schedules = app.world_mut().resource_mut::<BevySchedules>();
    let startup = schedules.remove(Startup);
    for (label, schedule) in schedules.iter_mut() {
        // Main and FixedMain orchestrate nested schedules; conditions belong on
        // the actual plugin systems, not on either orchestrator.
        let keys: Vec<_> = schedule
            .graph()
            .systems
            .iter()
            .map(|(key, _, _)| key)
            .filter(|key| {
                !baseline
                    .systems
                    .get(label)
                    .is_some_and(|old| old.contains(key))
            })
            .collect();
        for key in keys {
            schedule
                .graph_mut()
                .systems
                .get_conditions_mut(key)
                .expect("system condition storage")
                .push(ConditionWithAccess::new(Box::new(IntoSystem::into_system(
                    scene_published,
                ))));
        }
    }
    if let Some(startup) = baseline.startup {
        schedules.insert(startup);
    }
    drop(schedules);
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.init_resource::<ScenePublished>();
        let mut schedules = render.world_mut().resource_mut::<BevySchedules>();
        for target in [
            bevy::core_pipeline::Core3d.intern(),
            bevy::render::Render.intern(),
        ] {
            let Some(schedule) = schedules.get_mut(target) else {
                continue;
            };
            let keys: Vec<_> = schedule
                .graph()
                .systems
                .iter()
                .map(|(key, _, _)| key)
                .filter(|key| {
                    !baseline
                        .render
                        .get(&*target)
                        .is_some_and(|old| old.contains(key))
                })
                .collect();
            for key in keys {
                schedule
                    .graph_mut()
                    .systems
                    .get_conditions_mut(key)
                    .expect("system condition storage")
                    .push(ConditionWithAccess::new(Box::new(IntoSystem::into_system(
                        scene_published,
                    ))));
            }
        }
    }
    app.init_resource::<ScenePublished>().add_plugins(
        bevy::render::extract_resource::ExtractResourcePlugin::<ScenePublished>::default(),
    );
    app.init_resource::<super::scene_preparation::SceneGpuInputs>().add_plugins(
        bevy::render::extract_resource::ExtractResourcePlugin::<super::scene_preparation::SceneGpuInputs>::default(),
    );
    app.init_resource::<SceneImageRequests>().add_plugins(
        bevy::render::extract_resource::ExtractResourcePlugin::<SceneImageRequests>::default(),
    );
    crate::loading::gate(GPU_GATE);
    crate::loading::gate("initial view texture residency");
    crate::loading::gate("initial forward and shadow submissions");
    let gpu = GpuReadiness::default();
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.insert_resource(gpu.clone()).add_systems(
            bevy::render::Render,
            gpu_ready.in_set(bevy::render::RenderSystems::Cleanup),
        );
    }
    let map = level.entities.trim_end_matches(".entities").to_owned() + ".map";
    app.insert_resource(crate::gameplay::game_clock_runtime::GamePaused(true));
    app.init_resource::<super::resident_images::ResidentImages>();
    let resident = app.world().resource::<super::resident_images::ResidentImages>().clone();
    app.insert_resource(super::resident_images::UiImages::new(resident));
    app.insert_resource(super::texture_streaming::SharedImages::new(&catalog)?);
    let upload_budget = app.world().resource::<bevy::render::render_asset::RenderAssetBytesPerFrame>().max_bytes;
    limit_loading_uploads(app.world_mut());
    app.insert_resource(Acquisition {
        catalog: catalog.clone(),
        base: base.clone(),
        input: Some((catalog, level, base)),
        result: None,
        frames: 0,
        startup,
        published: false,
        movie: None,
        movie_visible: false,
        map,
        tick: 0.0,
        finished: false,
        initial_load: true,
        render_finished: false,
        continued: false,
        debug_continued: false,
        completion_visible: false,
        gpu,
        pending: Vec::new(),
        lifetime: Default::default(),
        next: None,
        debug_sequence: std::env::var("DH2_DEBUG_LEVEL_SEQUENCE")
            .unwrap_or_default()
            .split(';')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect(),
        debug_sequence_active: std::env::var_os("DH2_DEBUG_LEVEL_SEQUENCE").is_some(),
        played_seconds: 0.0,
        last_loading_update: std::time::Instant::now(),
        max_loading_gap: std::time::Duration::ZERO,
        movie_max_gap: std::time::Duration::ZERO,
        upload_budget,
    })
    .add_systems(Startup, show_bootstrap)
    .add_systems(First, acquire)
    .add_systems(
        First,
        super::resident_images::service_ui_images.after(acquire),
    )
    .add_systems(Update, loading_input)
    .add_systems(
        PostUpdate,
        super::resident_images::publish_ui_images.before(bevy::asset::AssetEventSystems),
    )
    .add_systems(Last, (initial_render_ready, finish_loading).chain())
    .add_systems(PostUpdate, (super::texture_streaming::particle_images::publish,
        super::texture_streaming::environment_images::publish_ocean)
        .before(bevy::asset::AssetEventSystems))
    .add_systems(PostUpdate, super::loading_fade::advance_fade);
    Ok(())
}

fn show_bootstrap(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    state: Res<Acquisition>,
    cache: Res<super::resident_images::UiImages>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Msaa::Off,
        bevy::camera::visibility::RenderLayers::none(),
        LoadingCamera,
    ));
    commands.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::BLACK), GlobalZIndex(i32::MAX - 1), super::loading_fade::FadeLayer));
    let catalog = &state.input.as_ref().unwrap().0;
    let image = cache.registry.load(
        catalog, "models/guis/loading_screens/loading_mainmenu_is.png", &mut images,
    )
    .unwrap_or_else(|error| panic!("startup loading background: {error}"));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            height: percent(100),
            ..default()
        },
        ImageNode::new(image),
        GlobalZIndex(i32::MAX),
        LoadingBackground,
    ));
    super::load_timing::mark("startup loading display");
}

fn acquire(world: &mut World) {
    let current = world.resource::<Acquisition>();
    if current.finished
        && !current.debug_sequence_active
        && !REQUEST_PENDING.load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }
    let mut state = world.remove_resource::<Acquisition>().unwrap();
    if state.finished {
        if state.debug_sequence_active {
            state.played_seconds += world.resource::<Time<Real>>().delta_secs_f64();
        }
        if state.debug_sequence_active && state.played_seconds >= 3.0 {
            if let Some(next) = state.debug_sequence.pop_front() {
                request_level(next);
            } else {
                state.debug_sequence_active = false;
                super::load_timing::mark("level sequence complete");
                world.write_message(AppExit::Success);
            }
            state.played_seconds = 0.0;
        }
        let request = if REQUEST_PENDING.load(std::sync::atomic::Ordering::Acquire) {
            let mut requests = REQUESTS.lock().unwrap();
            let request = requests.pop_front();
            REQUEST_PENDING.store(!requests.is_empty(), std::sync::atomic::Ordering::Release);
            request
        } else {
            None
        };
        if let Some(request) = request {
            match LevelResources::resolve(&state.catalog, &request) {
                Ok(level) => {
                    state.upload_budget = world.resource::<bevy::render::render_asset::RenderAssetBytesPerFrame>().max_bytes;
                    limit_loading_uploads(world);
                    state.last_loading_update = std::time::Instant::now();
                    state.max_loading_gap = std::time::Duration::ZERO;
                    state.movie_max_gap = std::time::Duration::ZERO;
                    state.finished = false;
                    for mut colour in world.query_filtered::<&mut BackgroundColor, With<super::loading_fade::FadeLayer>>().iter_mut(world) { colour.0 = Color::BLACK; }
                    state.next = Some(level);
                    // Stop outgoing view submission before the render-world
                    // release acknowledgement. Merely gating our publishers
                    // leaves Bevy's previous mesh phases drawable for another
                    // frame, after their resource bindings have been released.
                    for mut camera in world.query_filtered::<&mut Camera, (With<Camera3d>, Without<LoadingCamera>)>().iter_mut(world) {
                        camera.is_active = false;
                    }
                    world.spawn((
                        Camera3d::default(),
                        bevy::core_pipeline::tonemapping::Tonemapping::None,
                        Msaa::Off,
                        bevy::camera::visibility::RenderLayers::none(),
                        LoadingCamera,
                    ));
                    state.gpu.0.store(3, std::sync::atomic::Ordering::Release);
                    world.resource_mut::<ScenePublished>().0 = false;
                    world
                        .resource_mut::<crate::gameplay::game_clock_runtime::GamePaused>()
                        .0 = true;
                    // Stop map producers now. Cleanup waits for the render
                    // world to acknowledge its last outgoing publication.
                    world
                        .non_send_mut::<super::pause_menu::PauseMenu>()
                        .begin_level();
                    super::load_timing::mark("map transition requested");
                }
                Err(error) => error!("Map request {request}: {error}"),
            }
        }
        if state.finished {
            world.insert_resource(state);
            return;
        }
    }
    let now = std::time::Instant::now();
    let gap = now - state.last_loading_update;
    if gap > state.max_loading_gap {
        state.max_loading_gap = gap;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("LOAD window gap: {:.3} ms frame={} movie={} worker={} published={} gpu_phase={}",
                gap.as_secs_f64()*1000., state.frames, state.movie_visible, state.result.is_some(), state.published,
                state.gpu.0.load(std::sync::atomic::Ordering::Acquire));
            if let Some(profile) = world.get_resource::<crate::platform::frame_profile::CpuFrameProfile>() {
                let mut phases: Vec<_> = profile.latest_ns().into_iter().enumerate().collect();
                phases.sort_unstable_by_key(|(_, ns)| std::cmp::Reverse(*ns));
                eprintln!("LOAD window gap phases: {:?}", phases.into_iter().take(8).map(|(i, ns)| (crate::platform::frame_profile::LABELS[i], ns as f64 / 1_000_000.0)).collect::<Vec<_>>());
            }
        }
    }
    if state.movie_visible { state.movie_max_gap = state.movie_max_gap.max(gap); }
    state.last_loading_update = now;
    if state.next.is_some() {
        if state.gpu.0.load(std::sync::atomic::Ordering::Acquire) != 4 {
            world.insert_resource(state);
            return;
        }
        {
            let _span = super::load_timing::Span::new("outgoing world release");
            if let Err(error) = state.lifetime.clear(world) {
                error!("Outgoing level release failed: {error}");
                crate::platform::client_diagnostics::record(&format!("Outgoing level release failed: {error}"));
                world.write_message(AppExit::error());
                world.insert_resource(state);
                return;
            }
        }
        let level = state.next.take().unwrap();
        state.map = level.entities.trim_end_matches(".entities").to_owned() + ".map";
        crate::gameplay::map_info::set_current_map(&state.map);
        crate::loading::restart_level();
        state.input = Some((state.catalog.clone(), level, state.base.clone()));
        state.published = false;
        state.frames = 0;
        state.movie = None;
        state.movie_visible = false;
        state.continued = false;
        state.debug_continued = false;
        state.completion_visible = false;
        state.render_finished = false;
        state.tick = 0.0;
        state.pending.clear();
        state.gpu.0.store(0, std::sync::atomic::Ordering::Release);
        world
            .resource_mut::<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>()
            .0 = None;
        world.spawn((
            Camera3d::default(),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Msaa::Off,
            bevy::camera::visibility::RenderLayers::none(),
            LoadingCamera,
        ));
        super::load_timing::mark("outgoing world released");
    }
    match state.gpu.0.load(std::sync::atomic::Ordering::Acquire) {
        GPU_COMMON_IMAGES_READY => start_image_phase(world, &state, false),
        GPU_MAP_IMAGES_READY => {
            let cameras: Vec<_> = world
                .query_filtered::<Entity, With<LoadingCamera>>()
                .iter(world)
                .collect();
            for camera in cameras {
                world.despawn(camera);
            }
            for mut camera in world
                .query_filtered::<&mut Camera, With<Camera3d>>()
                .iter_mut(world)
            {
                camera.is_active = true;
            }
            world.resource_mut::<ScenePublished>().0 = true;
            state.gpu.0.store(1, std::sync::atomic::Ordering::Release);
            super::load_timing::mark("scene base images ready");
        }
        _ => {}
    }
    state.frames += 1;
    if state.movie.is_none() && !state.published {
        let screen = (1920, 1080);
        state.movie = Some(Mutex::new(
            world
                .non_send::<super::pause_menu::PauseMenu>()
                .start_loading(screen, state.map.clone()),
        ));
    }
    if state.published {
        let pending = crate::loading::pending();
        if pending != state.pending {
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("LOAD waiting: {pending:?}");
            }
            state.pending = pending;
        }
    }
    state.tick += world.resource::<Time<Real>>().delta_secs_f64();
    if state.movie.is_some() && state.tick >= 1.0 / 30.0 {
        world
            .non_send::<super::pause_menu::PauseMenu>()
            .tick_loading(
                state.tick,
                crate::loading::progress(),
                state.published && crate::loading::ready(),
            );
        state.tick = 0.0;
    }
    let frame = state
        .movie
        .as_ref()
        .and_then(|frames| frames.lock().unwrap().try_iter().last());
    if let Some(frame) = frame {
        match frame {
            Ok(frame) => {
                state.continued |= frame.continued;
                state.completion_visible |= frame.ready;
                world
                    .resource_mut::<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>()
                    .0 = Some(frame.frame);
                if !state.movie_visible {
                    let backgrounds: Vec<_> = world
                        .query_filtered::<Entity, With<LoadingBackground>>()
                        .iter(world)
                        .collect();
                    for entity in backgrounds {
                        world.despawn(entity);
                    }
                    state.movie_visible = true;
                    super::load_timing::mark("level loading movie");
                }
            }
            Err(error) => {
                error!("Loading presentation failed: {error}");
                crate::platform::client_diagnostics::record(&format!(
                    "Loading presentation failed: {error}"
                ));
                world.write_message(AppExit::error());
            }
        }
    }
    // Give the bootstrap a complete extraction/presentation before work starts.
    if state.frames >= 2 {
        if let Some((catalog, level, base)) = state.input.take() {
            let audio_work = match super::audio::acquire_audio_work() {
                Ok(work) => work,
                Err(error) => {
                    error!("Map audio preparation is unavailable: {error}");
                    world.insert_resource(state);
                    world.write_message(AppExit::error());
                    return;
                }
            };
            let preparation = super::scene_preparation::ScenePreparation::new(world);
            let needs_initial_inventory = !world.resource::<super::interaction_focus::DiscoveredDocuments>().is_initialized();
            let (send, receive) = mpsc::channel();
            state.result = Some(Mutex::new(receive));
            std::thread::Builder::new()
                .name("map acquisition".into())
                .stack_size(32 << 20)
                .spawn(move || {
                    let _audio_work = audio_work;
                    if _audio_work.is_closing() { return; }
                    let result = super::level_prepare::prepare_level(base, catalog, level, needs_initial_inventory, || _audio_work.is_closing())
                        .map_err(|e| e.to_string())
                        .and_then(|mut prepared| {
                            if _audio_work.is_closing() { return Err("audio session is closing".into()); }
                            if !prepared.diagnostic_exit { preparation.prepare(&mut prepared)?; }
                            Ok(prepared)
                        });
                    let _ = send.send(result);
                })
                .expect("map acquisition thread");
        }
    }
    let result =
        state
            .result
            .as_ref()
            .and_then(|receive| match receive.lock().unwrap().try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "map acquisition stopped before publishing a level".into(),
                )),
            });
    if let Some(result) = result {
        state.result = None;
        match result {
            Err(error) => {
                error!("Level load failed: {error}");
                crate::platform::client_diagnostics::record(&format!("Level load failed: {error}"));
                world.write_message(AppExit::error());
            }
            Ok(prepared) if prepared.diagnostic_exit => {
                state.finished = true;
                world.write_message(AppExit::Success);
            }
            Ok(prepared) => {
                // The setup creates the gameplay camera. Keep the loading
                // overlay until its asynchronous resource gates have completed.
                let cameras: Vec<_> = world
                    .query_filtered::<Entity, With<LoadingCamera>>()
                    .iter(world)
                    .collect();
                for camera in cameras {
                    world.despawn(camera);
                }
                let (mut lifetime, resources) = super::level_lifetime::LevelLifetime::begin(world);
                prepared.publish(world);
                if let Some(mut startup) = state.startup.take() {
                    let _span = super::load_timing::Span::new("remaining startup");
                    startup.run(world);
                    state.startup = Some(startup);
                }
                lifetime.published(world, &resources);
                state.lifetime = lifetime;
                for mut camera in world
                    .query_filtered::<&mut Camera, With<Camera3d>>()
                    .iter_mut(world)
                {
                    camera.is_active = false;
                }
                // UI-only view while resource uploads finish. It cannot
                // specialize/draw the map with placeholder image bindings.
                world.spawn((
                    Camera3d::default(),
                    Msaa::Off,
                    bevy::core_pipeline::tonemapping::Tonemapping::None,
                    bevy::camera::visibility::RenderLayers::none(),
                    LoadingCamera,
                ));
                start_image_phase(world, &state, true);
                state.published = true;
                if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                    let windows: Vec<_> = world
                        .query_filtered::<Entity, With<Window>>()
                        .iter(world)
                        .collect();
                    let outputs = world
                        .query_filtered::<Entity, With<super::level_lifetime::SessionEntity>>()
                        .iter(world)
                        .count();
                    eprintln!(
                        "LOAD session: map={} pid={} catalog={:p} device={:p} windows={windows:?} audio_outputs={outputs}",
                        state.map,
                        std::process::id(),
                        Arc::as_ptr(&state.catalog),
                        world.resource::<bevy::render::renderer::RenderDevice>()
                    );
                }
                super::load_timing::mark("level initialized");
            }
        }
    }
    world.insert_resource(state);
}

/// Reuse the production renderer's readiness signals. The benchmark's 30
/// consecutive-frame warmup is deliberately absent: one complete submission
/// is enough, without an arbitrary loading delay.
fn initial_render_ready(
    mut state: ResMut<Acquisition>, published: Res<ScenePublished>,
    forward: Res<crate::shadow_pass::ShadowCaptureReadiness>,
    directional: Res<crate::shadow_pass::ShadowSceneFrame>,
    sun: Option<Res<super::ParallelShadowOwner>>,
    local: Option<Res<super::local_shadow_updates::LocalShadowOwner>>,
    local_done: Res<crate::shadow_pass::LocalShadowCompletion>,
) {
    if state.finished || state.render_finished || !published.0 { return; }
    let ready = forward.0.load(std::sync::atomic::Ordering::Acquire) != 0
        && (sun.is_none() || matches!(directional.state, crate::shadow_pass::ShadowSceneState::Ready { .. }))
        && local.as_ref().is_none_or(|owner| local_done.0.lock().is_ok_and(|done| {
            done.as_ref().is_some_and(|(_, atlas, cache, _)| *atlas == owner.atlas_image.id() && *cache == owner.cache_image.id())
        }));
    if ready {
        state.render_finished = true;
        crate::loading::complete("initial forward and shadow submissions");
        super::load_timing::mark("initial forward and shadow submissions ready");
    }
}

/// Host: use the installed streaming budget to bound backend image acquisition
/// while displaying loading UI. Restore the host's normal policy before play.
/// Oversized single assets still progress, per Bevy's soft-limit contract.
fn limit_loading_uploads(world: &mut World) {
    let kb = crate::platform::console::shared().read().cvars.float("r_textureStreamingPerFrameBudgetKB")
        .unwrap_or(super::texture_streaming::TEXTURE_UPLOAD_BUDGET_KB);
    world.resource_mut::<bevy::render::render_asset::RenderAssetBytesPerFrame>().max_bytes =
        (kb > 0.).then_some((kb * 1024.) as usize);
}

fn finish_loading(
    mut state: ResMut<Acquisition>,
    mut frame: ResMut<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>,
    pause: NonSend<super::pause_menu::PauseMenu>,
    mut paused: ResMut<crate::gameplay::game_clock_runtime::GamePaused>,
    mut overlays: Query<&mut Visibility, With<super::state::GameplayOverlay>>,
    mut cursors: Query<&mut bevy::window::CursorOptions>,
    mut fade: Option<ResMut<super::loading_fade::LoadingFade>>,
    mut uploads: ResMut<bevy::render::render_asset::RenderAssetBytesPerFrame>,
) {
    if state.published && state.movie.is_some() && state.continued && crate::loading::ready() {
        if let Some(fade) = fade.as_deref_mut() { fade.start(state.initial_load); }
        uploads.max_bytes = state.upload_budget;
        state.initial_load = false;
        state.finished = true;
        state.played_seconds = 0.0;
        paused.0 = false;
        for mut cursor in &mut cursors {
            cursor.visible = false;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("LOAD cursor handoff: visible={} grab={:?}", cursor.visible, cursor.grab_mode);
            }
        }
        for mut overlay in &mut overlays {
            *overlay = Visibility::Inherited;
        }
        crate::loading::play_started();
        pause.end_loading();
        state.movie = None;
        frame.0 = None;
        super::load_timing::mark("loading presentation closed");
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("LOAD window update maximum gap: {:.3} ms; movie visible {:.3} ms; restored upload budget {:?}", state.max_loading_gap.as_secs_f64()*1000., state.movie_max_gap.as_secs_f64()*1000., uploads.max_bytes);
        }
    }
}

pub(super) fn is_loading(state: &Acquisition) -> bool {
    !state.finished
}

const GPU_GATE: &str = "scene upload and pipelines";
const GPU_COMMON_IMAGES: u8 = 5;
const GPU_COMMON_IMAGES_READY: u8 = 6;
const GPU_MAP_IMAGES: u8 = 7;
const GPU_MAP_IMAGES_READY: u8 = 8;
#[derive(Clone, Resource, Default)]
struct GpuReadiness(
    Arc<std::sync::atomic::AtomicU8>,
    Arc<std::sync::atomic::AtomicU64>,
);

#[derive(Clone, Resource, Default, bevy::render::extract_resource::ExtractResource)]
struct ScenePublished(bool);

#[derive(Clone, Resource, Default, bevy::render::extract_resource::ExtractResource)]
struct SceneImageRequests {
    generation: u64,
    phase: u8,
    images: Arc<Vec<bevy::asset::AssetId<Image>>>,
}

pub(super) fn image_uploads_deferred(world: &World) -> bool {
    world.contains_resource::<SceneImageRequests>()
}

fn start_image_phase(world: &mut World, state: &Acquisition, common: bool) {
    let images =
        match super::texture_streaming::initialize_scene_images(world, &state.catalog, common) {
            Ok(images) => images,
            Err(error) => {
                error!("Scene image acquisition failed: {error}");
                crate::platform::client_diagnostics::record(&format!(
                    "Scene image acquisition failed: {error}"
                ));
                world.write_message(AppExit::error());
                return;
            }
        };
    let phase = if common {
        GPU_COMMON_IMAGES
    } else {
        GPU_MAP_IMAGES
    };
    let generation = state
        .gpu
        .1
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
        + 1;
    world.insert_resource(SceneImageRequests {
        generation,
        phase,
        images: Arc::new(images),
    });
    state
        .gpu
        .0
        .store(phase, std::sync::atomic::Ordering::Release);
}

fn gpu_ready(
    published: Res<ScenePublished>,
    state: Res<GpuReadiness>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
    mailbox: Res<super::PreparedImageMailbox>,
    mut last: Local<Option<(usize, usize)>>,
    mut commands: Commands,
    requested: Res<SceneImageRequests>,
    images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    scene_inputs: (
        Res<super::scene_preparation::SceneGpuInputs>,
        Res<bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>,
        Res<bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderBuffer>>,
    ),
) {
    use std::sync::atomic::Ordering;
    let phase = state.0.load(Ordering::Acquire);
    if !published.0 && matches!(phase, GPU_COMMON_IMAGES | GPU_MAP_IMAGES) {
        if requested.phase == phase
            && requested.generation == state.1.load(Ordering::Acquire)
            && requested.images.iter().all(|id| images.get(*id).is_some())
            && (phase != GPU_MAP_IMAGES || (
                scene_inputs.0.images.iter().all(|id| images.get(*id).is_some())
                && scene_inputs.0.meshes.iter().all(|id| scene_inputs.1.get(*id).is_some())
                && scene_inputs.0.buffers.iter().all(|id| scene_inputs.2.get(*id).is_some())
            ))
        {
            state.0.store(phase + 1, Ordering::Release);
        }
        return;
    }
    if !published.0 && state.0.load(Ordering::Acquire) == 3 {
        // This render schedule has passed the outgoing frame. The main
        // world can now release map handles without racing its producers.
        let state = state.clone();
        commands.queue(move |world: &mut World| {
            crate::rendering::bevy_backend::release_level(world);
            state.0.store(4, Ordering::Release);
        });
        return;
    }
    if !published.0 || state.0.load(Ordering::Acquire) != 1 {
        return;
    }
    let counts = (
        pipelines.waiting_pipelines().count(),
        mailbox.0.lock().unwrap().pending.len(),
    );
    if *last != Some(counts) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!(
                "LOAD GPU waiting: {} pipelines, {} image uploads",
                counts.0, counts.1
            );
        }
        *last = Some(counts);
    }
    if counts != (0, 0) {
        return;
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("LOAD pipeline inventory: {}", pipelines.pipelines().count());
    }
    state.0.store(2, Ordering::Release);
    crate::loading::complete(GPU_GATE);
}

fn loading_input(
    mut state: ResMut<Acquisition>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    pause: NonSend<super::pause_menu::PauseMenu>,
    mut cursors: Query<&mut bevy::window::CursorOptions>,
) {
    if state.finished {
        return;
    }
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = bevy::window::CursorGrabMode::None;
    }
    if state.movie.is_none() {
        return;
    }
    let keyboard = crate::platform::bindings::Keyboard::default();
    for (down, codes) in [
        (true, keys.get_just_pressed().collect::<Vec<_>>()),
        (false, keys.get_just_released().collect()),
    ] {
        for code in codes {
            if let Some(engine) = super::engine_keys::engine_key(*code) {
                pause.loading_key(down, keyboard.virtual_key(engine).unwrap_or(engine));
            }
        }
    }
    for (button, index) in [
        (MouseButton::Left, 0),
        (MouseButton::Right, 1),
        (MouseButton::Middle, 2),
    ] {
        if buttons.just_pressed(button) {
            pause.loading_button(true, index);
        }
        if buttons.just_released(button) {
            pause.loading_button(false, index);
        }
    }
    // Host diagnostic: use the movie's real input route after completion,
    // allowing unattended loading checks without bypassing its state machine.
    if !state.debug_continued
        && state.completion_visible
        && (matches!(
            std::env::var("DH2_DEBUG_CONTINUE_LOADING").as_deref(),
            Ok("1" | "mouse")
        ) || std::env::var_os("DH2_BENCHMARK_SECONDS").is_some())
    {
        if std::env::var("DH2_DEBUG_CONTINUE_LOADING").as_deref() == Ok("mouse") {
            pause.loading_button(true, 0);
            pause.loading_button(false, 0);
        } else {
            pause.loading_key(true, 13);
            pause.loading_key(false, 13);
        }
        state.debug_continued = true;
    }
}

pub(super) fn is_playing(state: Res<Acquisition>) -> bool {
    state.finished
}

fn scene_published(published: Res<ScenePublished>) -> bool {
    published.0
}
