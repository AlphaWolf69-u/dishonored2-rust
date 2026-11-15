//! Opt-in observation through the normal application and its actual consumers.
//! Inputs are installed levels/effects and saved camera traces. This driver
//! never supplies worker results, clocks, resource contents or acknowledgements.
use super::{offscreen_runtime::Runtime, texture_streaming::TextureReadWorker};
use bevy::prelude::*;
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[derive(Resource, Default)]
struct PendingUiExit {
    requested: bool,
}

fn quit_during_pending_ui(world: &mut World) {
    if world.resource::<PendingUiExit>().requested {
        return;
    }
    let waiting = world
        .get_non_send_resource::<super::pause_menu::PauseMenu>()
        .is_some_and(|menu| menu.texture_reply_pending());
    let Some(request) = super::ui_lifecycle_diagnostics::pending_request()
        .filter(|request| !request.reply_sent && waiting)
    else {
        return;
    };
    eprintln!(
        "RUNTIME_UI_RETIREMENT phase=exit-request id={} name={:?}",
        request.id, request.name
    );
    world.resource_mut::<PendingUiExit>().requested = true;
    // Last precedes the actual ExitDrain. First remains installed to service
    // the real worker's reply until that drain emits its final AppExit.
    world.write_message(AppExit::Success);
}

fn record(world: &World, phase: &str) -> Result<(), String> {
    if let Some(view) = world.get_resource::<crate::rendering::bevy_backend::water_scene::WaterSceneFrame>()
        .and_then(|frame| frame.0.as_ref()) {
        eprintln!("RUNTIME_VIEW_LIFETIME phase={phase} camera={:?} frame={} position={:?} head_contact={:?}",
            view.camera, view.matrices.frame, view.matrices.current.position, view.head_water_surface);
    }
    if let Some(queue) = world.get_resource::<super::SceneRenderQueue>() {
        let worker = world
            .get_resource::<TextureReadWorker>()
            .map(TextureReadWorker::observe);
        eprintln!("RUNTIME_IMAGE_LIFETIME phase={phase} frame={} slots={} free_slots={} resident={} pending_uploads={} upgrades={} finer_bytes={} reserved_bytes={} worker={worker:?}",
            queue.texture_stream_frame, queue.diffuse_image_paths.len(),
            queue.diffuse_image_paths.iter().filter(|path| path.is_empty()).count(),
            queue.diffuse_image_handles.iter().filter(|handle| handle.is_some()).count(),
            queue.diffuse_image_pending.iter().filter(|handle| handle.is_some()).count(),
            queue.diffuse_mip_upgrades,
            queue.diffuse_mip_states.iter().map(|state| state.streamed_bytes).sum::<u64>(),
            queue.diffuse_mip_states.iter().map(|state| state.reserved_bytes).sum::<u64>());
    }
    if let Some(progress) = super::loading_screen::observe_exit_progress(world) {
        eprintln!("RUNTIME_EXIT_LIFETIME phase={phase} progress={progress:?}");
    }
    Ok(())
}

/// No native window is ever created. All effect requests, time advancement,
/// rendering, source reads, output consumption and shutdown are production.
pub fn inspect_runtime_lifecycles(
    base: &Path,
    args: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let level = args
        .first()
        .ok_or("runtime observation requires an installed level")?;
    let mode = args.get(1).ok_or("runtime observation requires quality, resize, streaming, fx, audio, ui-exit, exit-worker, exit-published or exit-reading")?;
    if !matches!(
        mode.as_str(),
        "quality"
            | "resize"
            | "streaming"
            | "fx"
            | "audio"
            | "music"
            | "ui-exit"
            | "ui-transition"
            | "read-exit"
            | "exit-worker"
            | "exit-published"
            | "exit-reading"
    ) {
        return Err(format!("unknown runtime lifecycle observation: {mode}").into());
    }
    if mode == "exit-reading" && !cfg!(feature = "debug_allocations") {
        return Err("active texture-read observation requires debug_allocations counters".into());
    }
    if std::env::var("DH2_DEBUG_QUALITY_RESIDENCY").as_deref() == Ok("1")
        && std::env::var("DH2_DEBUG_RESIZE_RESIDENCY").as_deref() == Ok("1") {
        return Err("quality and resize observations must run separately".into());
    }
    for (selected, control) in [
        ("quality", "DH2_DEBUG_QUALITY_RESIDENCY"),
        ("resize", "DH2_DEBUG_RESIZE_RESIDENCY"),
    ] {
        if mode == selected && std::env::var(control).as_deref() != Ok("1") {
            return Err(format!(
                "{selected} observation requires {control}=1 before process startup"
            )
            .into());
        }
    }
    if mode == "streaming"
        && std::env::var("DH2_BENCHMARK_PATH").map_or(true, |path| !path.contains(';'))
    {
        return Err(
            "streaming observation requires the existing saved-camera DH2_BENCHMARK_PATH replay"
                .into(),
        );
    }
    let extent = std::env::var("DH2_BENCHMARK_SIZE")
        .ok()
        .map(|size| -> Result<UVec2, String> {
            let (width, height) = size
                .split_once('x')
                .ok_or("viewport requires WIDTHxHEIGHT")?;
            Ok(UVec2::new(
                width.parse().map_err(|_| "invalid width")?,
                height.parse().map_err(|_| "invalid height")?,
            ))
        })
        .transpose()?
        .unwrap_or(UVec2::new(1920, 1080));
    let timeout = Duration::from_secs(
        std::env::var("DH2_DEBUG_RUNTIME_DEADLINE_SECONDS")
            .ok()
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(900),
    );
    let stream_seconds = args
        .get(2)
        .filter(|_| matches!(mode.as_str(), "streaming" | "music"))
        .map(|value| value.parse::<f64>())
        .transpose()?
        .unwrap_or(26.0);
    if !stream_seconds.is_finite() || stream_seconds <= 0.0 {
        return Err("streaming observation duration must be finite and positive".into());
    }
    super::ui_lifecycle_diagnostics::enable();
    let mut runtime = Runtime::open(base, level, extent)?;
    if mode == "ui-exit" {
        runtime
            .app
            .init_resource::<PendingUiExit>()
            .add_systems(Last, quit_during_pending_ui)
            .add_systems(
                First,
                quit_during_pending_ui.before(super::resident_images::service_ui_images),
            );
    }
    let started = Instant::now();
    let mut playing_since = None;
    let mut exit_requested = false;
    let mut draining_since = None;
    let mut fx = None;
    let mut audio: Option<super::audio::observation::Scenario> = None;
    let mut readable = None;
    let mut ui_transition = (mode == "ui-transition").then(||
        super::ui_lifecycle_diagnostics::TransitionObservation::new(level, args.get(2).map(String::as_str)));
    let mut last_frame = 0;
    let mut first_stream_position = None;
    let mut stream_moved = false;
    let mut milestone_reached = false;
    let mut failure = None;
    loop {
        match runtime.step() {
            Ok(Some(exit)) => {
                if exit.is_error() {
                    failure.get_or_insert_with(|| format!("runtime emitted {exit:?}"));
                }
                if matches!(mode.as_str(), "quality" | "resize")
                    && !super::resource_diagnostics::sequence_completed(runtime.app.world(), mode) {
                    failure.get_or_insert_with(|| format!("runtime exited before the selected {mode} sequence completed seven actual submissions"));
                }
                record(runtime.app.world(), "shutdown")?;
                if let Some(observation) = fx.as_ref() {
                    super::fx_lifecycle_diagnostics::Observation::observe_shutdown(
                        observation,
                        runtime.app.world(),
                    )?;
                }
                if mode == "audio" {
                    match audio.as_ref() {
                        Some(scenario) => {
                            if let Err(error) = scenario.validate_complete(runtime.app.world()) {
                                failure.get_or_insert(error);
                            }
                        }
                        None => { failure.get_or_insert_with(|| "audio scenario exited before it began".into()); }
                    }
                }
                match super::audio::observation::validate_shutdown(runtime.app.world_mut()) {
                    Ok(snapshot) => eprintln!("RUNTIME_AUDIO_LIFETIME phase=shutdown validated=true snapshot={snapshot:?}"),
                    Err(error) => { failure.get_or_insert(error); }
                }
                break;
            }
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
                if !exit_requested {
                    runtime.request_exit();
                    exit_requested = true;
                    draining_since = Some(Instant::now());
                }
            }
        }
        if !exit_requested
            && super::loading_screen::observe_exit_progress(runtime.app.world())
                .is_some_and(|progress| progress.requested)
        {
            exit_requested = true;
            draining_since = Some(Instant::now());
        }
        let playing = runtime.playing()?;
        if playing && playing_since.is_none() {
            playing_since = Some(Instant::now());
            record(runtime.app.world(), "playing")?;
        }
        let mut complete = false;
        if !exit_requested {
            let progress = (|| -> Result<bool, String> {
                let mut complete = false;
                match mode.as_str() {
                    "ui-transition" => {
                        complete = ui_transition.as_mut().expect("UI transition mode owner").poll(playing)?;
                    }
                    "fx" if playing => {
                        if fx.is_none() {
                            fx = Some(super::fx_lifecycle_diagnostics::Observation::begin(
                                runtime.app.world_mut(),
                                &args[2..],
                            )?);
                        }
                        let observation = fx.as_mut().unwrap();
                        if observation.needs_view_gap() {
                            runtime.set_view_enabled(false);
                            if let Some(exit) = runtime.step()? {
                                return Err(format!(
                                    "runtime exited during actual view gap: {exit:?}"
                                ));
                            }
                            observation.observe_view_gap(runtime.app.world())?;
                            runtime.set_view_enabled(true);
                        }
                        complete = observation.poll(runtime.app.world_mut())?;
                    }
                    "audio" if playing => {
                        if audio.is_none() {
                            audio = Some(super::audio::observation::begin_scenario(
                                runtime.app.world_mut(),
                                Default::default(),
                            )?);
                        }
                        complete = super::audio::observation::poll_scenario(
                            runtime.app.world_mut(),
                            audio.as_mut().unwrap(),
                        )?;
                    }
                    "music" if playing => {
                        // Observe the actual level manager and output clock;
                        // no synthetic state groups, callbacks or PCM/EOF.
                        complete = playing_since.is_some_and(|time| time.elapsed().as_secs_f64() >= stream_seconds);
                        if complete { super::audio::music::observe_live_owner(runtime.app.world())?; }
                    }
                    "read-exit" if playing => {
                        if readable.is_none() {
                            readable = super::interactions::request_admitted_readable(
                                runtime.app.world_mut(),
                            )?;
                            if readable.is_none() {
                                return Err(
                                    "this level has no applicable admitted portable readable"
                                        .into(),
                                );
                            }
                            eprintln!(
                                "RUNTIME_READABLE_LIFETIME phase=requested target={readable:?}"
                            );
                        }
                        let progress =
                            super::interactions::read_action_progress(runtime.app.world());
                        complete = runtime
                            .app
                            .world()
                            .get_non_send_resource::<super::pause_menu::PauseMenu>()
                            .is_some_and(|menu| menu.readable_ready());
                        if complete {
                            eprintln!("RUNTIME_READABLE_LIFETIME phase=ready target={readable:?} action={progress:?} pickups={:?}",
                            super::interactions::accepted_pickup_progress(runtime.app.world()));
                        }
                    }
                    "streaming" if playing => {
                        if let Some(view) = runtime.app.world()
                            .get_resource::<crate::rendering::bevy_backend::water_scene::WaterSceneFrame>()
                            .and_then(|frame| frame.0.as_ref()) {
                            let position = view.matrices.current.position;
                            let first = first_stream_position.get_or_insert(position);
                            stream_moved |= *first != position;
                        }
                        let frame = runtime
                            .app
                            .world()
                            .get_resource::<super::SceneRenderQueue>()
                            .map_or(0, |queue| queue.texture_stream_frame);
                        if frame.saturating_sub(last_frame) >= 60 {
                            record(runtime.app.world(), "moving")?;
                            last_frame = frame;
                        }
                        complete = playing_since
                            .is_some_and(|time| time.elapsed().as_secs_f64() >= stream_seconds);
                        if complete && !stream_moved {
                            return Err("saved streaming replay never moved the actual published camera".into());
                        }
                    }
                    "exit-worker" => {
                        complete = super::loading_screen::observe_exit_progress(runtime.app.world())
                            .is_some_and(|progress| progress.worker_finished == Some(false))
                    }
                    "exit-published" => {
                        complete = super::loading_screen::observe_exit_progress(runtime.app.world())
                            .is_some_and(|progress| progress.published && !playing)
                    }
                    "exit-reading" if playing => {
                        complete = runtime
                            .app
                            .world()
                            .get_resource::<TextureReadWorker>()
                            .is_some_and(|worker| worker.observe().active_slot.is_some())
                    }
                    _ => {}
                }
                Ok(complete)
            })();
            match progress {
                Ok(done) => complete = done,
                Err(error) => {
                    failure.get_or_insert(error);
                    complete = true;
                }
            }
        }
        if complete {
            milestone_reached = true;
            record(runtime.app.world(), "exit-request")?;
            runtime.request_exit();
            exit_requested = true;
            draining_since = Some(Instant::now());
        }
        if started.elapsed() > timeout && !exit_requested {
            failure = Some(format!(
                "runtime observation never completed: mode={mode} fx={:?}",
                fx.as_ref().map(|fx| fx.pending_description())
            ));
            record(runtime.app.world(), "incomplete")?;
            runtime.request_exit();
            exit_requested = true;
            draining_since = Some(Instant::now());
        }
        if draining_since.is_some_and(|time| time.elapsed() > Duration::from_secs(120)) {
            return Err(format!("actual runtime retirement exceeded diagnostic deadline: mode={mode} failure={failure:?}").into());
        }
    }
    if mode == "ui-exit" && !runtime.app.world().resource::<PendingUiExit>().requested {
        failure.get_or_insert_with(|| {
            "no real pending UI image reply was observed before exit".into()
        });
    }
    if matches!(mode.as_str(), "exit-worker" | "exit-published" | "exit-reading" | "read-exit" | "streaming")
        && !milestone_reached {
        failure.get_or_insert_with(|| format!("runtime exited before the actual {mode} milestone"));
    }
    drop(runtime);
    let readbacks = crate::rendering::bevy_backend::readback_lifecycle::snapshot();
    eprintln!("RUNTIME_READBACK_LIFETIME phase=host-dropped snapshot={readbacks:?}");
    if readbacks.enabled && (readbacks.live_owner_states != 0 || readbacks.live_retired_states != 0
        || readbacks.overflow != 0 || readbacks.coarse.created != readbacks.coarse.dropped
        || readbacks.ocean.created != readbacks.ocean.dropped) {
        failure.get_or_insert_with(|| "actual readback owners or callbacks survived App destruction, or observation overflowed".into());
    }
    if let Some(observation) = ui_transition {
        if let Err(error) = observation.observe_host_drop() { failure.get_or_insert(error); }
    } else if let Err(error) = super::ui_lifecycle_diagnostics::validate_host_drop() {
        failure.get_or_insert(error);
    }
    eprintln!(
        "RUNTIME_UI_RETIREMENT phase=host-dropped snapshot={:?}",
        super::ui_lifecycle_diagnostics::snapshot()
    );
    eprintln!(
        "RUNTIME_RESOURCE_LIFETIME level={level} mode={mode} complete={}",
        failure.is_none()
    );
    if let Some(error) = failure {
        return Err(error.into());
    }
    Ok(())
}
