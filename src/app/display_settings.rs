//! The window, presentation and frame rate the cvars select, applied at
//! start and whenever the settings change (`console::shared`).
//!
//! Host integration: the engine shows its window in `FUN_140347f10` and
//! presents with `r_swapInterval`; here they become Bevy window state.
use crate::platform::console;
use crate::platform::display_modes::{self, Monitors};
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, VideoMode, VideoModeSelection, WindowMode, WindowPosition};
use std::time::{Duration, Instant};

#[derive(Resource, Default)]
pub struct DisplaySettings {
    generation: Option<u64>,
    /// `_DAT_1425b2220`: the end of the last frame-limiter wait.
    last_frame: Option<Instant>,
    warned_interval: bool,
}

/// The window `FUN_140347f10` shows for the cvars.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// `r_fullscreen 1`: exclusive fullscreen at r_mode's size.
    Fullscreen { monitor: usize, size: (u32, u32), refresh_hz: u32 },
    /// `r_fullscreen 2`: borderless at r_mode's size, at the monitor's corner.
    Borderless { origin: (i32, i32), size: (u32, u32) },
    /// `r_fullscreen 0`.
    Windowed { position: (i32, i32), size: (u32, u32) },
}

fn int(c: &console::Console, name: &str) -> i32 {
    c.cvars.int(name).unwrap_or(0)
}

/// `FUN_140347f10`'s window from the cvars. An unset `r_mode` first gets the
/// default mode (`FUN_140348bb0`), which writes `r_mode` and the window cvars.
pub fn shown(table: &Monitors) -> Option<Shown> {
    let shared = console::shared();
    if shared.read().cvars.int("r_mode").unwrap_or(-1) < 0 {
        let (r_mode, mon, w, h) = table.default_mode(display_modes::desktop_size());
        shared.write(|c| {
            let mut set = |name: &str, v: i32| {
                if let Some(i) = c.cvars.find(name) {
                    c.cvars.set_int(i, v, true);
                }
            };
            set("r_mode", r_mode);
            if let Some((w, h, x, y)) = table.window(mon, w, h) {
                set("r_windowWidth", w);
                set("r_windowHeight", h);
                set("r_windowPosX", x);
                set("r_windowPosY", y);
            }
        });
    }
    let c = shared.read();
    let (w, h, mon) = table.mode(int(&c, "r_mode")).or_else(|| table.mode(0))?;
    let monitor = table.monitors.get(mon as usize)?;
    let size = (w as u32, h as u32);
    Some(match int(&c, "r_fullscreen") {
        1 => {
            let index = monitor.modes.iter().position(|&p| p == ((w as u32) << 16 | h as u32)).unwrap_or(0);
            Shown::Fullscreen { monitor: mon as usize, size, refresh_hz: monitor.max_refresh.get(index).copied().unwrap_or(0) }
        }
        // The window is placed at the monitor's corner; ShowGameWindow's
        // placement on a non-primary monitor is not traced.
        2 => Shown::Borderless { origin: monitor.origin, size },
        _ => {
            let (ww, wh) = (int(&c, "r_windowWidth"), int(&c, "r_windowHeight"));
            let size = if ww > 0 && wh > 0 {
                (ww as u32, wh as u32)
            } else if w > 0 && h > 0 {
                size
            } else {
                (960, 540)
            };
            Shown::Windowed { position: (int(&c, "r_windowPosX"), int(&c, "r_windowPosY")), size }
        }
    })
}

/// `r_swapInterval`: 0 presents immediately (the composited DISCARD path is
/// Mailbox on Windows), otherwise with v-sync. wgpu cannot present every
/// N-th refresh, so intervals 2..4 present at the full refresh rate
/// (approximation, reported once).
fn present_mode(interval: i32) -> PresentMode {
    match interval {
        0 if cfg!(target_os = "windows") && std::env::var("DH2_DEBUG_DIRECT_PRESENT").as_deref() != Ok("1") => PresentMode::Mailbox,
        0 => PresentMode::AutoNoVsync,
        _ => PresentMode::Fifo,
    }
}

/// Put the shown window on `window`. Leaving a fullscreen mode for a
/// window first changes only the mode (winit restores the old windowed size
/// on that change); returns false then, and the rest follows next frame.
fn configure(window: &mut Window, chosen: &Shown, table: &Monitors) -> bool {
    let windowed = |window: &mut Window, decorations: bool, position: (i32, i32), size: (u32, u32)| {
        if window.mode != WindowMode::Windowed {
            window.mode = WindowMode::Windowed;
            return false;
        }
        window.decorations = decorations;
        window.position = WindowPosition::At(IVec2::new(position.0, position.1));
        window.resolution.set_physical_resolution(size.0, size.1);
        true
    };
    match *chosen {
        Shown::Fullscreen { monitor, size, refresh_hz } => {
            window.mode = WindowMode::Fullscreen(
                MonitorSelection::Index(monitor),
                VideoModeSelection::Specific(VideoMode { physical_size: UVec2::new(size.0, size.1), bit_depth: 32, refresh_rate_millihertz: refresh_hz * 1000 }),
            );
            true
        }
        Shown::Borderless { origin, size } => {
            // A borderless window covering its monitor is the windowing
            // system's borderless fullscreen on that monitor.
            match table.monitors.iter().position(|m| m.origin == origin && m.size == (size.0 as i32, size.1 as i32)) {
                Some(index) => {
                    window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Index(index));
                    true
                }
                None => windowed(window, false, origin, size),
            }
        }
        Shown::Windowed { position, size } => windowed(window, true, position, size),
    }
}

/// The diagnostic overrides `DH2_BENCHMARK_SIZE` and `DH2_BORDERLESS` keep
/// the window as launch.rs sets it.
fn window_overridden() -> bool {
    std::env::var_os("DH2_BENCHMARK_SIZE").is_some() || std::env::var_os("DH2_BORDERLESS").is_some()
}

/// `r_swapInterval` and `r_swapChainTripleBufferEnable` on the window (the
/// overrides `DH2_NO_VSYNC` and `DH2_FRAME_LATENCY` keep their effect).
fn present(window: &mut Window, warned: &mut bool) {
    let c = console::shared().read();
    if std::env::var_os("DH2_NO_VSYNC").is_none() {
        let interval = int(&c, "r_swapInterval");
        if interval > 1 && !*warned {
            warn!("approximate: r_swapInterval {interval} presents at the full refresh rate");
            *warned = true;
        }
        window.present_mode = present_mode(interval);
    }
    if std::env::var_os("DH2_FRAME_LATENCY").is_none() {
        // 368b80: 1 + r_swapChainTripleBufferEnable frames of latency.
        window.desired_maximum_frame_latency = std::num::NonZeroU32::new(1 + (int(&c, "r_swapChainTripleBufferEnable") != 0) as u32);
    }
}

/// The window the client opens with (before the first frame).
pub fn initial_window(mut window: Window) -> (Window, DisplaySettings) {
    let mut state = DisplaySettings::default();
    if !window_overridden() {
        let table = display_modes::monitors();
        if let Some(chosen) = shown(&table) {
            info!("Window settings: {chosen:?}");
            configure(&mut window, &chosen, &table);
        }
    }
    present(&mut window, &mut state.warned_interval);
    state.generation = Some(console::shared().generation());
    (window, state)
}

/// Apply the window and presentation cvars when the settings changed.
pub fn apply_display_settings(mut state: ResMut<DisplaySettings>, mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let shared = console::shared();
    if state.generation == Some(shared.generation()) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else { return };
    if !window_overridden() {
        let table = display_modes::monitors();
        match shown(&table) {
            Some(chosen) => {
                info!("Window settings: {chosen:?}");
                if !configure(&mut window, &chosen, &table) {
                    return;
                }
            }
            None => warn!("No display mode for the window settings"),
        }
    }
    let state = &mut *state;
    present(&mut window, &mut state.warned_interval);
    // Read after `shown`, which may have written the default mode.
    state.generation = Some(shared.generation());
}

/// `FUN_1400462e0(com_timeScaler, 1, 1/30)` at the end of the frame
/// (`FUN_140045...` caller, chunk_000124): with `com_disableFrameSpinning`
/// and `r_swapInterval` both 0, wait (yielding) until
/// `(1/30 s * 1000 / com_timeScaler) * 1000` µs have passed since the end of
/// the last wait: 30 x com_timeScaler frames per second.
pub fn limit_frame_rate(mut state: ResMut<DisplaySettings>) {
    let (spin, scaler) = {
        let c = console::shared().read();
        (int(&c, "com_disableFrameSpinning") == 0 && int(&c, "r_swapInterval") == 0, c.cvars.float("com_timeScaler").unwrap_or(2.0))
    };
    if !spin {
        state.last_frame = None;
        return;
    }
    let last = *state.last_frame.get_or_insert_with(Instant::now);
    let frame_ms = 0.033_333_335_f32 * 1000.0;
    let target = Duration::from_micros(((frame_ms / scaler) * 1000.0) as u64);
    let mut now = Instant::now();
    while now.duration_since(last) < target {
        std::thread::yield_now();
        now = Instant::now();
    }
    state.last_frame = Some(now);
}

/// `state` comes from [`initial_window`].
pub struct DisplaySettingsPlugin(pub std::sync::Mutex<Option<DisplaySettings>>);

impl Plugin for DisplaySettingsPlugin {
    fn build(&self, app: &mut App) {
        let state = self.0.lock().unwrap().take().unwrap_or_default();
        app.insert_resource(state).add_systems(First, apply_display_settings).add_systems(Last, limit_frame_rate);
    }
}
