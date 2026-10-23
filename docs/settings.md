# Settings

The client keeps one command state, as the game keeps one cvar system and one
binding manager: `platform::console::shared()`, loaded once from the installed
`default.cfg` (and the configs it execs) and the rewrite's
`config/dishonored2Config.cfg`. The pause menu's Options edit it; a config
save writes it back (`Console::save_config`). Every change of a cvar value
or a binding advances `Shared::generation`.

## Readers

- **Renderer settings** (`platform::game_settings`, `camera_settings`) parse
  `name "value"` lines. Their source is now every registered cvar with its
  current value, defaults included (`Shared::cvar_text`), not the config file
  text. A value the game would refuse from a config (a cvar without the
  retail flag, such as `r_zfar` or `g_gameFixedDt`) no longer applies, as in
  the game; `DH2_DEBUG_DEVELOPER=1` sets the engine's developer flag
  (`DAT_1432cd1c8`) so such config lines are accepted.
  Most renderer systems read once at startup; their settings take effect on
  the next start.
- **Window and presentation** (`app::display_settings`), at start and on
  every change:
  - `r_fullscreen` 1: exclusive fullscreen at `r_mode`'s size and refresh;
    2: borderless at `r_mode`'s size at the monitor's corner (the windowing
    system's borderless fullscreen when that covers the monitor); 0: a window
    at `r_windowPosX/Y` of `r_windowWidth/Height` (else `r_mode`'s size,
    else 960x540) (`FUN_140347f10`).
  - An unset `r_mode` (-1, the default) takes the default mode
    (`FUN_140348bb0`) and writes `r_mode` and the window cvars.
  - `r_swapInterval` 0 presents immediately (Mailbox on Windows), 1 with
    v-sync; 2..4 also present at the full refresh rate (approximation:
    wgpu has no swap interval).
  - `r_swapChainTripleBufferEnable`: 1 + flag frames of latency.
  - Diagnostic overrides keep their effect: `DH2_BENCHMARK_SIZE`,
    `DH2_BORDERLESS` (window), `DH2_NO_VSYNC`, `DH2_FRAME_LATENCY`.
- **Frame limiter** (`FUN_1400462e0`): with `com_disableFrameSpinning` and
  `r_swapInterval` 0, each frame ends by yielding until
  `(1/30 s * 1000 / com_timeScaler) * 1000` µs passed since the last wait:
  30 x `com_timeScaler` fps (60 by default).
- **Gamma** (`FUN_140326af0`, notes-gamma.md): the tone-mapping pass gets
  `gamma_brightness = (1 / (r_gamma / 2.2), r_brightness)` every frame, so the
  Options gamma slider previews live; the tone curve also needs
  `r_skipPostProcess` 0.
- **Pause key:** the PAUSE binding is read from the shared bindings on each
  key press, so rebinding it applies at once.

## Not live yet

- Renderer systems that size resources from settings at startup (shadow
  quality and atlas sizes, view distance and LOD, water, textures, decals,
  anti-aliasing, resolution scale, field of view) apply on the next start.
- The tone-mapping colour cube's gating by `r_skipAutoExposure`,
  `r_skipPostProcess` and black-and-white mode (`r_BlackAndWhiteMode`
  selects the BWMode1 shader variant, not rendered yet).
- Audio settings (no audio system yet), subtitles.
