//! bevy_backend: bridge exit requests to the existing level retirement owner.
//! The native GameLocal destructor1408a4830/1408a5f60 precedes session teardown.
//! Winit otherwise clears the world immediately after observing AppExit.
use super::*;
use bevy::{app::MainScheduleOrder, window::{CursorGrabMode, CursorOptions, WindowCloseRequested}};

#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct ExitDrain;

#[derive(Resource, Default)]
pub(in crate::app) struct ExitState {
    request: Option<AppExit>,
    started: bool,
    emitted: bool,
    render_owner: bool,
    phase: &'static str,
    reported_phase: &'static str,
    window_close: bevy::ecs::message::MessageCursor<WindowCloseRequested>,
}
impl ExitState {
    pub(in crate::app) fn requested(&self) -> bool { self.request.is_some() }
}

pub(super) fn install(app: &mut App) {
    let render_owner = app.get_sub_app(bevy::render::RenderApp).is_some();
    app.insert_resource(ExitState { render_owner, ..Default::default() })
        .init_schedule(ExitDrain)
        .add_systems(ExitDrain, drain);
    // Capture every main-world exit producer, including diagnostics in Last,
    // before the runner calls should_exit() after the complete app update.
    app.world_mut().resource_mut::<MainScheduleOrder>().insert_after(Last, ExitDrain);
}

pub(super) fn pending(world: &World) -> bool {
    world.get_resource::<ExitState>().is_some_and(|exit| exit.request.is_some())
}

pub(super) fn running(exit: Res<ExitState>) -> bool { exit.request.is_none() }

/// Actual owner state for windowless phase observers; no worker, GPU or
/// retirement acknowledgement is supplied by this snapshot.
#[derive(Debug)]
pub(in crate::app) struct ExitProgress {
    pub requested: bool,
    pub started: bool,
    pub emitted: bool,
    pub phase: &'static str,
    pub worker_finished: Option<bool>,
    pub result_receiver: bool,
    pub published: bool,
    pub gpu_phase: u8,
}

pub(super) fn progress(world: &World) -> Option<ExitProgress> {
    let exit = world.get_resource::<ExitState>()?;
    let acquisition = world.get_resource::<Acquisition>()?;
    Some(ExitProgress { requested: exit.request.is_some(), started: exit.started,
        emitted: exit.emitted, phase: exit.phase,
        worker_finished: acquisition.map_worker.as_ref().map(|worker| worker.is_finished()),
        result_receiver: acquisition.result.is_some(), published: acquisition.published,
        gpu_phase: acquisition.gpu.0.load(std::sync::atomic::Ordering::Acquire) })
}

pub(super) fn observe_drain(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<ExitState>() || !world.contains_resource::<Acquisition>() {
        return Err("session exit observer requires the installed loading/session owners".into());
    }
    drain(world);
    Ok(())
}

fn request(exit: &mut ExitState, next: AppExit) {
    // Match App::should_exit: the first error takes precedence over success.
    if exit.request.is_none() || (next.is_error() && exit.request.as_ref().is_some_and(AppExit::is_success)) {
        exit.request = Some(next);
    }
}

fn begin(world: &mut World, exit: &mut ExitState) {
    super::super::audio::close_session_admission();
    super::super::level_lifetime::close_level_acquisitions(world);
    {
        let mut state = world.resource_mut::<Acquisition>();
        // Dropping the result receiver makes unpublished prepared resources
        // retire in their existing owner; the worker handle still fences exit.
        state.result = None;
        state.input = None;
        state.next = None;
        state.movie = None;
        state.finished = false;
        state.debug_sequence_active = false;
        state.gpu.0.store(if exit.render_owner { 3 } else { 4 }, std::sync::atomic::Ordering::Release);
    }
    world.resource_mut::<ScenePublished>().0 = false;
    if let Some(mut paused) = world.get_resource_mut::<crate::gameplay::game_clock_runtime::GamePaused>() { paused.0 = true; }
    if let Some(mut menu) = world.get_non_send_resource_mut::<super::super::pause_menu::PauseMenu>() { menu.begin_level(); }
    for mut camera in world.query::<&mut Camera>().iter_mut(world) { camera.is_active = false; }
    // A real quit closes the user's view immediately without destroying the
    // window/device whose outstanding render publication still has to retire.
    for (mut window, mut cursor) in world.query::<(&mut Window, &mut CursorOptions)>().iter_mut(world) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        window.visible = false;
    }
    if let Some(mut settings) = world.get_resource_mut::<bevy::winit::WinitSettings>() {
        // Reuse the host's existing unfocused game cadence. Continuous mode
        // spins on invisible Windows windows; no focus or redraw is required.
        let mode = bevy::winit::WinitSettings::game().unfocused_mode;
        settings.focused_mode = mode;
        settings.unfocused_mode = mode;
    }
    exit.started = true;
}

fn advance(world: &mut World, exit: &mut ExitState) -> Result<bool, String> {
    exit.phase = "map acquisition worker";
    let mut state = world.remove_resource::<Acquisition>().ok_or("exit has no acquisition lifetime owner")?;
    let worker = retire_map_worker(&mut state);
    world.insert_resource(state);
    if !worker? { return Ok(false); }
    exit.phase = "level acquisition owners";
    if !super::super::level_lifetime::retire_level_acquisitions(world) { return Ok(false); }
    exit.phase = "outgoing render publication";
    if world.resource::<Acquisition>().gpu.0.load(std::sync::atomic::Ordering::Acquire) != 4 { return Ok(false); }
    exit.phase = "component, FX and sound retirement";
    let mut state = world.remove_resource::<Acquisition>().ok_or("exit lost its acquisition lifetime owner")?;
    let cleared = state.lifetime.clear(world);
    exit.phase = state.lifetime.retirement_phase();
    world.insert_resource(state);
    cleared
}

fn drain(world: &mut World) {
    world.resource_scope(|world, mut exit: Mut<ExitState>| {
        if exit.emitted { return; }
        for next in world.resource_mut::<Messages<AppExit>>().drain() { request(&mut exit, next); }
        let window_close = exit.window_close.read(world.resource::<Messages<WindowCloseRequested>>()).next().is_some();
        if window_close {
            request(&mut exit, AppExit::Success);
        }
        if exit.request.is_none() { return; }
        if !exit.started { begin(world, &mut exit); }
        let result = advance(world, &mut exit);
        if exit.reported_phase != exit.phase {
            crate::platform::client_diagnostics::record(&format!("Session exit waiting: {}", exit.phase));
            exit.reported_phase = exit.phase;
        }
        match result {
            Ok(false) => {},
            Ok(true) => {
                crate::platform::client_diagnostics::record("Session exit: level retirement completed");
                exit.emitted = true;
                world.write_message(exit.request.as_ref().expect("exit request retained").clone());
            }
            Err(error) => {
                let message = format!("Session exit cleanup failed during {}: {error}; remaining owners will drop without a clean retirement acknowledgement", exit.phase);
                error!("{message}");
                crate::platform::client_diagnostics::record(&message);
                request(&mut exit, AppExit::error());
                exit.emitted = true;
                world.write_message(exit.request.as_ref().expect("error exit retained").clone());
            }
        }
    });
}
