//! Host-only resource-lifetime observation through the real window/renderer.
//! Enabled explicitly; it does not write settings or collect performance timings.
use bevy::{prelude::*, render::renderer::RenderDevice, window::PrimaryWindow};

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
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    let started = *sequence.started.get_or_insert(now);
    if now - started > 300.0 {
        eprintln!("MEMORY resize observation incomplete: step={} frames={}", sequence.step, sequence.frames);
        exit.write(AppExit::error());
        return;
    }
    let Ok(mut window) = windows.single_mut() else { return };
    let origin = *sequence.origin.get_or_insert(window.resolution.physical_size());
    let sizes = [origin, UVec2::new(1920, 1080), UVec2::new(960, 540), origin,
        UVec2::new(1920, 1080), UVec2::new(960, 540), origin];
    if sequence.step >= sizes.len() { return; }
    let wanted = sizes[sequence.step];
    if window.resolution.physical_size() != wanted
        || !cameras.iter().any(|camera| camera.physical_viewport_size() == Some(wanted)) {
        sequence.frames = 0;
        return;
    }
    sequence.frames += 1;
    if sequence.frames < 120 { return; }
    let finer_bytes: u64 = scene.diffuse_mip_states.iter().map(|state| state.streamed_bytes).sum();
    let reserved_bytes: u64 = scene.diffuse_mip_states.iter().map(|state| state.reserved_bytes).sum();
    eprintln!("MEMORY resize streaming: sample={} finer_bytes={finer_bytes} reserved_bytes={reserved_bytes} upgrades={}",
        sequence.step, scene.diffuse_mip_upgrades);
    eprintln!("MEMORY resize snapshot: sample={} width={} height={} frames={} resident_images={} pending_images={}",
        sequence.step, wanted.x, wanted.y, sequence.frames,
        scene.diffuse_image_handles.iter().filter(|image| image.is_some()).count(),
        scene.diffuse_image_pending.iter().filter(|image| image.is_some()).count());
    if let Some(report) = device.wgpu_device().generate_allocator_report() {
        eprintln!("MEMORY resize allocator: sample={} allocated_bytes={} reserved_bytes={} blocks={} allocations={}",
            sequence.step, report.total_allocated_bytes, report.total_reserved_bytes,
            report.blocks.len(), report.allocations.len());
        let mut groups = std::collections::BTreeMap::<&str, (u64, usize)>::new();
        for allocation in &report.allocations {
            let group = groups.entry(allocation.name.as_str()).or_default();
            group.0 += allocation.size;
            group.1 += 1;
        }
        for (name, (bytes, count)) in groups {
            eprintln!("MEMORY resize allocation group: sample={} name={name:?} bytes={bytes} count={count}", sequence.step);
        }
    } else {
        eprintln!("MEMORY resize allocator unavailable: sample={}", sequence.step);
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
