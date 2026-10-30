//! Diagnostic: one frame captured through the graphics debugger API.
//! RenderDoc hooks the D3D12 device but not wgpu's swapchain, so its own
//! capture trigger records an unrelated D3D11 device. Run the rewrite under
//! `renderdoccmd capture` with `DH2_DEBUG_GPU_CAPTURE_SECONDS=<s>`: the first
//! render frame starting that long after complete rendering (a benchmark run,
//! `DH2_BENCHMARK_SECONDS`) or after launch (otherwise) is bracketed by wgpu's
//! start/stop graphics debugger capture. Counting a benchmark run from launch
//! captured a loading frame whenever loading ran long, which RenderDoc then
//! spent minutes serializing.
use bevy::{
    prelude::*,
    render::{renderer::RenderDevice, Render, RenderApp, RenderSystems},
};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

static STATE: AtomicU8 = AtomicU8::new(0); // 0 waiting, 1 capturing, 2 done
static READY: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// The benchmark's complete-rendering point, which a benchmark capture counts from.
pub fn mark_rendering_ready() {
    READY.get_or_init(std::time::Instant::now);
}

pub(super) fn register(app: &mut App) {
    let Some(seconds) = std::env::var("DH2_DEBUG_GPU_CAPTURE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
    else {
        return;
    };
    let launch = std::time::Instant::now();
    let benchmark = std::env::var_os("DH2_BENCHMARK_SECONDS").is_some();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            Render,
            (
                (move |device: Res<RenderDevice>| {
                    let start = if benchmark { READY.get().copied() } else { Some(launch) };
                    if STATE.load(Relaxed) == 0
                        && start.is_some_and(|start| start.elapsed().as_secs_f64() >= seconds)
                    {
                        // SAFETY: paired with the stop below on this device.
                        unsafe { device.wgpu_device().start_graphics_debugger_capture() };
                        STATE.store(1, Relaxed);
                        eprintln!("GPU capture started");
                    }
                })
                .before(RenderSystems::ExtractCommands),
                (|device: Res<RenderDevice>| {
                    if STATE.load(Relaxed) == 1 {
                        unsafe { device.wgpu_device().stop_graphics_debugger_capture() };
                        STATE.store(2, Relaxed);
                        eprintln!("GPU capture stopped");
                    }
                })
                .after(RenderSystems::PostCleanup),
            ),
        );
    }
}
