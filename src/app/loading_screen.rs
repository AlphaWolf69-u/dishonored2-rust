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
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};

#[derive(Default)]
pub(super) struct PreparedLevel {
    resources: Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>,
    diagnostic_exit: bool,
}
impl PreparedLevel {
    pub(super) fn diagnostic_exit() -> Self {
        Self {
            resources: Vec::new(),
            diagnostic_exit: true,
        }
    }
    pub fn insert_resource<R: Resource>(&mut self, resource: R) -> &mut Self {
        self.resources.push(Box::new(move |world| {
            world.insert_resource(resource);
        }));
        self
    }
    fn publish(self, world: &mut World) {
        for insert in self.resources {
            insert(world);
        }
    }
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
    continued: bool,
    debug_continued: bool,
    completion_visible: bool,
    gpu: GpuReadiness,
    pending: Vec<&'static str>,
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
                    resource_exists::<super::SceneData>,
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
    crate::loading::gate(GPU_GATE);
    let gpu = GpuReadiness::default();
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.insert_resource(gpu.clone()).add_systems(
            bevy::render::Render,
            gpu_ready.in_set(bevy::render::RenderSystems::Cleanup),
        );
    }
    let map = level.entities.trim_end_matches(".entities").to_owned() + ".map";
    app.insert_resource(crate::gameplay::game_clock_runtime::GamePaused(true));
    app.init_resource::<super::resident_images::UiImages>();
    app.insert_resource(Acquisition {
        catalog: catalog.clone(),
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
        continued: false,
        debug_continued: false,
        completion_visible: false,
        gpu,
        pending: Vec::new(),
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
    .add_systems(Last, finish_loading);
    Ok(())
}

fn show_bootstrap(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    state: Res<Acquisition>,
    mut cache: ResMut<super::resident_images::UiImages>,
) {
    commands.spawn((
        Camera3d::default(),
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Msaa::Off,
        LoadingCamera,
    ));
    let catalog = &state.input.as_ref().unwrap().0;
    let super::resident_images::UiImages {
        resident, aliases, ..
    } = &mut *cache;
    let image = super::resident_images::prepare_image(
        catalog,
        "models/guis/loading_screens/loading_mainmenu_is.png",
        &mut images,
        resident,
        aliases,
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
    let mut state = world.remove_resource::<Acquisition>().unwrap();
    if state.finished {
        world.insert_resource(state);
        return;
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
            let (send, receive) = mpsc::channel();
            state.result = Some(Mutex::new(receive));
            std::thread::Builder::new()
                .name("map acquisition".into())
                .stack_size(32 << 20)
                .spawn(move || {
                    let result = super::level_prepare::prepare_level(base, catalog, level)
                        .map_err(|e| e.to_string());
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
                prepared.publish(world);
                state.gpu.0.store(1, std::sync::atomic::Ordering::Release);
                if let Some(mut startup) = state.startup.take() {
                    startup.run(world);
                    state.startup = Some(startup);
                }
                world.resource_mut::<ScenePublished>().0 = true;
                state.published = true;
                super::load_timing::mark("level initialized");
            }
        }
    }
    world.insert_resource(state);
}

fn finish_loading(
    mut state: ResMut<Acquisition>,
    mut frame: ResMut<crate::rendering::bevy_backend::flash_ui::FlashUiFrame>,
    pause: NonSend<super::pause_menu::PauseMenu>,
    mut paused: ResMut<crate::gameplay::game_clock_runtime::GamePaused>,
    mut overlays: Query<&mut Visibility, With<super::state::GameplayOverlay>>,
    mut cursors: Query<&mut bevy::window::CursorOptions>,
) {
    if state.published && state.movie.is_some() && state.continued && crate::loading::ready() {
        state.finished = true;
        paused.0 = false;
        for mut cursor in &mut cursors {
            cursor.visible = true;
        }
        for mut overlay in &mut overlays {
            *overlay = Visibility::Inherited;
        }
        crate::loading::play_started();
        pause.end_loading();
        state.movie = None;
        frame.0 = None;
        super::load_timing::mark("loading presentation closed");
    }
}

pub(super) fn is_loading(state: &Acquisition) -> bool {
    !state.finished
}

const GPU_GATE: &str = "scene upload and pipelines";
#[derive(Clone, Resource, Default)]
struct GpuReadiness(Arc<std::sync::atomic::AtomicU8>);

#[derive(Clone, Resource, Default, bevy::render::extract_resource::ExtractResource)]
struct ScenePublished(bool);

fn gpu_ready(
    published: Res<ScenePublished>,
    state: Res<GpuReadiness>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
    mailbox: Res<super::PreparedImageMailbox>,
    mut last: Local<Option<(usize, usize)>>,
) {
    use std::sync::atomic::Ordering;
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
    if state.finished || state.movie.is_none() {
        return;
    }
    for mut cursor in &mut cursors {
        cursor.visible = false;
        cursor.grab_mode = bevy::window::CursorGrabMode::None;
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
