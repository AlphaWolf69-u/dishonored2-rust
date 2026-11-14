//! Host-only resource-lifetime observation through the real window/renderer.
//! Enabled explicitly; quality observation restores its temporary Console values.
//! Neither observation writes configuration files or collects performance timings.
use bevy::{prelude::*, render::renderer::RenderDevice, window::PrimaryWindow};


#[derive(Resource, Clone, Default)]
pub(super) struct TargetObservation(std::sync::Arc<std::sync::Mutex<TargetState>>);
#[derive(Default)]
struct TargetState {
    request: Option<(&'static str, usize)>,
    ready: bool,
    rows: Vec<String>,
    submitted: Option<bevy::render::render_resource::PollType>,
}

/// Install only for an offscreen observation. Normal gameplay has no target
/// snapshot system, shared mailbox, fence polling or allocation-range tracing.
pub(super) fn install_offscreen(app: &mut App) {
    let observation = TargetObservation::default();
    let submitted = bevy::render::renderer::ObservedSubmission::default();
    app.insert_resource(observation.clone());
    app.insert_resource(submitted.clone());
    app.sub_app_mut(bevy::render::RenderApp)
        .insert_resource(observation).insert_resource(submitted)
        .add_systems(bevy::render::Render, observe_submitted_targets.in_set(bevy::render::RenderSystems::Cleanup));
}

fn observe_submitted_targets(world: &mut World) {
    let observation = world.resource::<TargetObservation>().clone();
    let mut state = observation.0.lock().unwrap();
    if state.request.is_none() { return; }
    let (fog_ready, mut rows) = crate::rendering::bevy_backend::fog_scene::observe_target_resources(world);
    let (camera_ready, camera_rows) = crate::rendering::bevy_backend::camera_scene::observe_target_resources(world);
    rows.extend(camera_rows);
    state.rows = rows;
    state.ready = fog_ready && camera_ready;
    state.submitted = world.resource::<bevy::render::renderer::ObservedSubmission>().latest();
}

fn submitted_targets_ready(observation: Option<&TargetObservation>, device: &RenderDevice,
    kind: &'static str, sample: usize) -> bool {
    let Some(observation) = observation else { return true; };
    let mut state = observation.0.lock().unwrap();
    if state.request != Some((kind, sample)) {
        state.request = Some((kind, sample));
        state.ready = false;
        state.submitted = None;
        return false;
    }
    if !state.ready { return false; }
    let Some(submission) = state.submitted.take() else { return false; };
    if let Err(error) = device.poll(submission) {
        eprintln!("MEMORY {kind} submission wait failed: sample={sample} error={error}");
        state.ready = false;
        return false;
    }
    for row in state.rows.drain(..) { eprintln!("MEMORY {kind} rendered target: sample={sample} {row}"); }
    eprintln!("MEMORY {kind} actual submission completed: sample={sample}");
    state.request = None;
    true
}

#[derive(Default)]
pub(super) struct ResizeSequence {
    origin: Option<UVec2>,
    step: usize,
    frames: u32,
    started: Option<f64>,
}

/// Revisit sizes rather than inferring resize retirement from map teardown.
/// All targets are still created/replaced by their ordinary production owners.
pub(super) fn observe_resize(
    time: Res<Time<Real>>,
    mut sequence: Local<ResizeSequence>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    cameras: Query<&Camera, With<Camera3d>>,
    scene: Res<super::SceneRenderQueue>,
    device: Res<RenderDevice>,
    targets: Option<Res<TargetObservation>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    let started = *sequence.started.get_or_insert(now);
    if now - started > 300.0 {
        eprintln!(
            "MEMORY resize observation incomplete: step={} frames={}",
            sequence.step, sequence.frames
        );
        exit.write(AppExit::error());
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let origin = *sequence
        .origin
        .get_or_insert(window.resolution.physical_size());
    let sizes = [
        origin,
        UVec2::new(1920, 1080),
        UVec2::new(960, 540),
        origin,
        UVec2::new(1920, 1080),
        UVec2::new(960, 540),
        origin,
    ];
    if sequence.step >= sizes.len() {
        return;
    }
    let wanted = sizes[sequence.step];
    if window.resolution.physical_size() != wanted
        || !cameras
            .iter()
            .any(|camera| camera.physical_viewport_size() == Some(wanted))
    {
        sequence.frames = 0;
        return;
    }
    sequence.frames += 1;
    if sequence.frames < 120 {
        return;
    }
    if !submitted_targets_ready(targets.as_deref(), &device, "resize", sequence.step) { return; }
    let finer_bytes: u64 = scene
        .diffuse_mip_states
        .iter()
        .map(|state| state.streamed_bytes)
        .sum();
    let reserved_bytes: u64 = scene
        .diffuse_mip_states
        .iter()
        .map(|state| state.reserved_bytes)
        .sum();
    eprintln!("MEMORY resize streaming: sample={} finer_bytes={finer_bytes} reserved_bytes={reserved_bytes} upgrades={}",
        sequence.step, scene.diffuse_mip_upgrades);
    eprintln!("MEMORY resize snapshot: sample={} width={} height={} frames={} resident_images={} pending_images={}",
        sequence.step, wanted.x, wanted.y, sequence.frames,
        scene.diffuse_image_handles.iter().filter(|image| image.is_some()).count(),
        scene.diffuse_image_pending.iter().filter(|image| image.is_some()).count());
    if !allocator_snapshot(&device, "resize", sequence.step) {
        exit.write(AppExit::error());
        return;
    }
    sequence.step += 1;
    sequence.frames = 0;
    if let Some(next) = sizes.get(sequence.step) {
        window.resolution.set_physical_resolution(next.x, next.y);
    } else {
        eprintln!("MEMORY resize sequence complete");
        exit.write(AppExit::Success);
    }
}

pub(super) fn allocator_snapshot(device: &RenderDevice, kind: &str, sample: usize) -> bool {
    if let Some(report) = device.wgpu_device().generate_allocator_report() {
        eprintln!("MEMORY {kind} allocator: sample={} allocated_bytes={} reserved_bytes={} blocks={} allocations={}",
            sample, report.total_allocated_bytes, report.total_reserved_bytes,
            report.blocks.len(), report.allocations.len());
        let mut groups = std::collections::BTreeMap::<&str, (u64, usize)>::new();
        for allocation in &report.allocations {
            let group = groups.entry(allocation.name.as_str()).or_default();
            group.0 += allocation.size;
            group.1 += 1;
        }
        for (name, (bytes, count)) in groups {
            eprintln!("MEMORY {kind} allocation group: sample={} name={name:?} bytes={bytes} count={count}", sample);
        }
        let mut empty_bytes = 0u64;
        let mut empty_blocks = 0usize;
        for (index, block) in report.blocks.iter().enumerate() {
            let Some(allocations) = report.allocations.get(block.allocations.clone()) else {
                eprintln!("MEMORY {kind} allocation block unavailable: sample={sample} block={index}");
                return false;
            };
            for allocation in allocations {
                eprintln!("MEMORY {kind} allocation range: sample={sample} block={index} name={:?} offset={} bytes={}",
                    allocation.name, allocation.offset, allocation.size);
            }
            let bytes: u64 = allocations.iter().map(|allocation| allocation.size).sum();
            eprintln!("MEMORY {kind} allocation block: sample={sample} block={index} reserved_bytes={} allocated_bytes={bytes} allocations={}",
                block.size, allocations.len());
            if allocations.is_empty() {
                empty_bytes += block.size;
                empty_blocks += 1;
            }
            let mut groups = std::collections::BTreeMap::<&str, (u64, usize)>::new();
            for allocation in allocations {
                let group = groups.entry(allocation.name.as_str()).or_default();
                group.0 += allocation.size;
                group.1 += 1;
            }
            for (name, (bytes, count)) in groups {
                eprintln!("MEMORY {kind} allocation block group: sample={sample} block={index} name={name:?} bytes={bytes} count={count}");
            }
        }
        eprintln!("MEMORY {kind} empty allocation blocks: sample={sample} blocks={empty_blocks} reserved_bytes={empty_bytes}");
    } else {
        eprintln!("MEMORY {kind} allocator unavailable: sample={}", sample);
        return false;
    }
    true
}

const QUALITY_CVARS: [&str; 4] = [
    "r_downsampleQuality",
    "r_shadows32Bits",
    "r_shadowParallelResolutionDividerLevel",
    "r_shadowParallelSliceCnt",
];

#[derive(Default)]
pub(super) struct QualitySequence {
    original: Vec<(usize, String)>,
    states: Vec<Vec<String>>,
    step: usize,
    frames: u32,
    started: Option<f64>,
    extent: Option<UVec2>,
}

impl QualitySequence {
    fn restore(&self) {
        crate::platform::console::shared().write(|console| {
            for (index, value) in &self.original {
                console.cvars.set(*index, Some(value), true);
            }
        });
    }
}

/// Fixed-size A/B/C/A/B/C/A observation through the existing live Console and
/// owner replacement paths. Values are valid native CVar options (511a70,
/// 50ff40); this never edits a menu, gameplay state or saved configuration.
pub(super) fn observe_quality(
    time: Res<Time<Real>>,
    mut sequence: Local<QualitySequence>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<&Camera, With<Camera3d>>,
    scene: Res<super::SceneData>,
    queue: Res<super::SceneRenderQueue>,
    parallel: Res<super::shadow_preparation::ParallelShadowOwner>,
    local: Res<super::local_shadow_updates::LocalShadowOwner>,
    frame: Res<crate::rendering::bevy_backend::shadow_pass::ShadowSceneFrame>,
    done: Res<crate::rendering::bevy_backend::shadow_pass::ShadowSceneCompletion>,
    local_done: Res<crate::rendering::bevy_backend::shadow_pass::LocalShadowCompletion>,
    fog: Res<crate::rendering::bevy_backend::fog::FogSceneFrame>,
    device: Res<RenderDevice>,
    targets: Option<Res<TargetObservation>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    let started = *sequence.started.get_or_insert(now);
    if now - started > 300.0 {
        eprintln!(
            "MEMORY quality observation incomplete: step={} frames={}",
            sequence.step, sequence.frames
        );
        sequence.restore();
        exit.write(AppExit::error());
        return;
    }
    let Ok(window) = windows.single() else { return };
    let extent = *sequence
        .extent
        .get_or_insert(window.resolution.physical_size());
    if sequence.states.is_empty() {
        let shared = crate::platform::console::shared().read();
        for name in QUALITY_CVARS {
            let Some(index) = shared.cvars.find(name) else {
                eprintln!("MEMORY quality observation missing CVar: {name}");
                exit.write(AppExit::error());
                return;
            };
            sequence
                .original
                .push((index, shared.cvars.list[index].value.clone()));
        }
        let original: Vec<String> = sequence
            .original
            .iter()
            .map(|(_, value)| value.clone())
            .collect();
        let mut fog = original.clone();
        fog[0] = if shared.cvars.int(QUALITY_CVARS[0]) == Some(1) {
            "2"
        } else {
            "1"
        }
        .into();
        let mut shadow = original.clone();
        shadow[1] = if shared.cvars.int(QUALITY_CVARS[1]) == Some(0) {
            "1"
        } else {
            "0"
        }
        .into();
        shadow[2] = if shared.cvars.int(QUALITY_CVARS[2]) == Some(0) {
            "2"
        } else {
            "0"
        }
        .into();
        shadow[3] = if shared.cvars.int(QUALITY_CVARS[3]) == Some(4) {
            "3"
        } else {
            "4"
        }
        .into();
        sequence.states = vec![
            original.clone(),
            fog.clone(),
            shadow.clone(),
            original.clone(),
            fog,
            shadow,
            original,
        ];
    }
    if sequence.step >= sequence.states.len() {
        return;
    }
    let expected = sequence.states[sequence.step]
        .iter()
        .map(|value| value.parse::<i32>())
        .collect::<Result<Vec<_>, _>>();
    let Ok(expected) = expected else {
        sequence.restore();
        exit.write(AppExit::error());
        return;
    };
    let quality_matches = scene.shadow_quality.as_ref().is_some_and(|quality| {
        quality.depth_32_bits == (expected[1] != 0)
            && quality.parallel_resolution_divider == expected[2] as u32
            && quality.parallel_slice_count == expected[3] as u32
    });
    let ready = quality_matches
        && window.resolution.physical_size() == extent
        && cameras
            .iter()
            .any(|camera| camera.physical_viewport_size() == Some(extent))
        && fog
            .0
            .as_ref()
            .is_some_and(|fog| fog.settings.downsample == expected[0])
        && matches!(&frame.state, crate::rendering::bevy_backend::shadow_pass::ShadowSceneState::Ready { target_image, dimensions, .. }
            if target_image.id() == parallel.target_image.id() && *dimensions == parallel.dimensions)
        && done.whole_view_completed(&parallel.target_image, parallel.dimensions)
        && local_done.0.lock().is_ok_and(|done| {
            done.as_ref().is_some_and(|(_, atlas, cache, _)| {
                *atlas == local.atlas_image.id() && *cache == local.cache_image.id()
            })
        });
    if !ready {
        sequence.frames = 0;
        return;
    }
    sequence.frames += 1;
    if sequence.frames < 120 {
        return;
    }
    if !submitted_targets_ready(targets.as_deref(), &device, "quality", sequence.step) { return; }
    for (role, image) in [("directional", parallel.target_image.id()), ("local", local.atlas_image.id()), ("local-cache", local.cache_image.id())] {
        eprintln!("MEMORY quality target identity: sample={} role={role} image={image:?}", sequence.step);
    }
    eprintln!("MEMORY quality snapshot: sample={} extent={extent:?} values={:?} parallel={:?} local_atlas={:?} local_cache={:?} finer_bytes={} reserved_stream_bytes={}",
        sequence.step, sequence.states[sequence.step], parallel.dimensions, local.atlas_image.id(), local.cache_image.id(),
        queue.diffuse_mip_states.iter().map(|state| state.streamed_bytes).sum::<u64>(),
        queue.diffuse_mip_states.iter().map(|state| state.reserved_bytes).sum::<u64>());
    if !allocator_snapshot(&device, "quality", sequence.step) {
        sequence.restore();
        exit.write(AppExit::error());
        return;
    }
    sequence.step += 1;
    sequence.frames = 0;
    if let Some(values) = sequence.states.get(sequence.step) {
        crate::platform::console::shared().write(|console| {
            for ((index, _), value) in sequence.original.iter().zip(values) {
                console.cvars.set(*index, Some(value), true);
            }
        });
    } else {
        sequence.restore();
        eprintln!("MEMORY quality sequence complete");
        exit.write(AppExit::Success);
    }
}
