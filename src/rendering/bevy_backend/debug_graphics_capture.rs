//! Diagnostic: one frame captured through the graphics debugger API.
//! RenderDoc hooks the D3D12 device but not wgpu's swapchain, so its own
//! capture trigger records an unrelated D3D11 device. Run the rewrite under
//! `renderdoccmd capture` with `DH2_DEBUG_GPU_CAPTURE_SECONDS=<s>`: the first
//! render frame starting that long after launch is bracketed by wgpu's
//! start/stop graphics debugger capture.
use bevy::{
    prelude::*,
    render::{renderer::RenderDevice, Render, RenderApp, RenderSystems},
};
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

static STATE: AtomicU8 = AtomicU8::new(0); // 0 waiting, 1 capturing, 2 done

pub(super) fn register(app: &mut App) {
    let Some(seconds) = std::env::var("DH2_DEBUG_GPU_CAPTURE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
    else {
        return;
    };
    let start = std::time::Instant::now();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            Render,
            (
                (move |device: Res<RenderDevice>| {
                    if STATE.load(Relaxed) == 0 && start.elapsed().as_secs_f64() >= seconds {
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
