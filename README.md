# Dishonored 2 Rust rewrite

An independent, unfinished Rust rewrite using assets from a **legally owned,
locally installed copy of Dishonored 2**. This repository contains rewrite
source and development tools, not the original game or its assets.

The Windows client currently supports selected world/model formats, visibility
and asset streaming, authored material textures, basic first-person movement,
decoded collision, and the pause menu with its options (the game's own Flash
movies, settings and key bindings). It is **not a complete playable
replacement**: NPCs, combat, missions, saves, animation, and parts of
lighting/material rendering remain incomplete. Visual and gameplay parity are
still being verified.

The rewrite exists to be modded: mods change its source, and the launcher
merges the enabled mods and builds the game from them.

## Playing: the launcher

Download `dishonored2-launcher.exe` into an empty folder and run it. That
folder becomes the rewrite folder: the launcher downloads the rewrite source,
installs nothing elsewhere, and keeps everything beside itself:

```
config/      dishonored2Config.cfg (settings and key bindings), rewrite.toml
saves/       the rewrite's saves
mods/        one folder per mod
source/      the rewrite source the launcher builds
cache/ logs/ build output, session and build logs
```

Building needs Rust and the Visual Studio C++ build tools; the launcher detects
them and offers to install them. Choose the game folder if it is not found
(any store's installation). Settings and saves are separate from the original
game's; **Import saves…** and **Import config…** copy them in on request.

Mods are folders under `mods/` with a `mod.toml` and changes to the source
(`patches/*.patch`) and/or replacement and new game files (`resources/`). The
launcher merges enabled mods top to bottom (drag to reorder), merges
`Cargo.toml` changes by key, shows conflicts and lets you resolve them (keep
one side, or edit the file), and checks relationships between mods
(`requires`, `incompatible`, `load_after`, `load_before`). See
[docs/rewrite-folder.md](docs/rewrite-folder.md) for the mod format and the
launcher's rules.

`dishonored2-launcher.exe <command>` does the same without the window and
prints JSON (`help` lists the commands: status, conflicts, resolve, enable,
disable, move, sort, download, build, launch and more).

## Requirements

- Windows and the Rust MSVC toolchain (`rustup`, Cargo, and Visual Studio C++
  build tools).
- A recent stable Rust compiler and a graphics driver supporting Bevy's renderer.
- Dishonored 2 installed (Steam is found automatically; other stores' installations
  are chosen once), or `DH2_GAME_BASE` set to its `base` directory.
- Disk space for Cargo dependencies. No map extraction is required.

## Build and test

From the repository root:

```powershell
cargo test --locked
cargo test --locked --features game --lib
.\build-game.ps1
```

The build script produces `target/release/dishonored2-rust.exe`. Dependencies
are downloaded on the first build; `--offline` is only appropriate once cached.
Tests marked ignored need a local game installation, private research captures,
or a GPU. Numeric regression fixtures ship in `tests/fixtures/`; the normal suite
does not need extracted assets or the private research directory.

Patched Bevy crates are included under `vendor/`, with their original MIT and
Apache-2.0 licenses. Their licensed auxiliary assets are not Dishonored assets.

## Run with installed resources

```powershell
.\target\release\dishonored2-rust.exe
```

The folder beside the executable is its rewrite folder (`target/release/` for
development builds; `DH2_REWRITE_HOME` selects another): settings are read from
`config/dishonored2Config.cfg` there, and nothing is imported automatically, so
a fresh folder starts with the game's defaults. Copy a config in, or run
`dishonored2-rust.exe --import-saves <folder>` to copy saves.

The client discovers Steam libraries and AppID 403640's installation. It reads
world geometry, entities, visibility, collision, models, textures, probes and
declarations directly from the installed indices and resource archives. Compiled
program definitions are read from the installation's shader cache. The rewrite's
own shaders are embedded in the executable. Launching does not require the
repository to be the current directory or any pre-extracted map files.

The default level is `eotw_p_lowchaos`. Select another installed world by its
unambiguous stem or full archive resource path:

```powershell
.\target\release\dishonored2-rust.exe menu_p
```

To override discovery, set `DH2_GAME_BASE` to the installed game's `base` directory.
An invalid override reports an error. The tools can list archive resource names;
their optional extraction command is for research, not a launch prerequisite.

Windows launches use DirectX 12 by default. `WGPU_BACKEND=vulkan` selects the
Vulkan backend explicitly. DirectX shader compilation prefers an installed DXC
compiler beside the executable, on PATH, or in the Windows SDK; an SDK is not a
launch requirement. `WGPU_DX12_COMPILER=Fxc` retains the FXC fallback explicitly.
No compiler DLLs or game executables are copied by the launcher.

Sunlight caster depth uses the game's authored area ranges by default.
Set `DH2_SUN_SHADOW_MIN_EXTENT=150` to extend short ranges to at least 150 metres;
authored longer ranges are preserved. This optional rewrite setting leaves the
game's configuration and assets untouched; longer caster depth can increase
shadow cost. A value of `0` also selects the authored ranges.

## Controls and diagnostics

- Each launch saves renderer warnings/errors, panic backtraces and the exit result
  under the rewrite folder's `logs/`, even if the console closes.
- Click the window to capture the mouse. Esc (the game's pause binding) opens the
  pause menu: Options, help and Quit.
- The player uses the key bindings of `config/dishonored2Config.cfg` (the
  game's IN_GAME binding set; defaults from the installed `default.cfg`):
  move, jump, sprint (toggle or hold per `g_sprintToggle`), crouch toggle,
  lean (Q/E, LALT; toggle or hold per `g_leanToggle`), mouse look with
  `g_mouseSensitivity`, `g_invertMouseYAxis` and `g_mouseSmooth`, and
  interact. Movement, jumping, falling, sneaking, sprinting, sliding and
  leaning follow the game's player decls and physics.
- Debug views: G toggles a debug free camera (bound move actions, MOVE_UP /
  MOVE_DOWN vertically, SPRINT for speed); Home/End/PageUp/PageDown cycle
  focus targets, returning the player to the start where it has one.
- F8 saves a screenshot and collision/render diagnostics under ignored
  `artifacts/`. Use matching captures when reporting a rendering defect.
- `DH2_GAME_BASE` selects the installed asset directory.
- `DH2_HOST_CONFIG_PATH` selects a configuration file instead of the rewrite folder's.
- `DH2_DEBUG_DEVELOPER=1` lets a configuration set developer-only cvars, as the
  game's developer flag does.
- `DH2_RENDER_PROFILE=1` enables rendering timing/streaming diagnostics.
- `DH2_BENCHMARK_SECONDS` runs a fixed-camera benchmark; it disables movement
  during that measurement.
- `DH2_BENCHMARK_SIZE=1920x1080` fixes the physical benchmark viewport independently
  of desktop size and DPI. Benchmark output records viewport and camera pose.

Exact shadow-query reuse, compact world-shadow constants, unchanged local
atlas reuse, mesh allocation metadata reuse and prepared shadow record reuse
are enabled by default. Depth passes share compatible pipelines and omit
material reads only for verified opaque shader paths. Windows also finalizes
independent command encoders in parallel, using a small dedicated pool sized
to the host, and records large direct camera phases
in consecutive chunks. These paths preserve quality settings, selected
geometry, draw order and shadow coverage. Diagnostic controls retain the
reference submission paths:

| Control | Reference path |
| --- | --- |
| `DH2_CACHE_SHADOW_QUERIES=0` | Recompute every caster query |
| `DH2_CACHE_SHADOW_MESH_ALLOCATIONS=0` | Resolve mesh allocations for every packet |
| `DH2_CACHE_PREPARED_SHADOWS=0` | Prepare shadow records every frame |
| `DH2_SHARE_DEPTH_PIPELINES=0` | Use complete material keys for depth passes |
| `DH2_MATERIAL_FREE_DEPTH=0` | Keep material bindings in otherwise shared depth passes |
| `DH2_COMPACT_SHADOW_UNIFORMS=0` | Separate constants for every world packet |
| `DH2_REUSE_LOCAL_SHADOW_ATLAS=0` | Repack the local atlas every frame |
| `DH2_PARALLEL_COMMAND_FINALIZATION=0` | Finalize commands serially |
| `DH2_RENDER_FINALIZATION_THREADS=0` | Use the shared worker pool for finalization |
| `DH2_CAMERA_DRAW_CHUNK_SIZE=0` | Record each camera phase as one pass |

## Sharing and research

**Do not commit or redistribute extracted game assets, original executable
bytes, decompiled game code, original shader sources, process dumps, or game
screenshots.** These belong to the installed game, not this source repository.
Local `artifacts/`, `bin/`, `target/`, `reference/`, `.pi/`, and research databases
are intentionally ignored. Synthetic numerical regression vectors are separate
from extracted game assets.

The implementation is shared by the desktop client and command-line tools.
See [docs/architecture.md](docs/architecture.md) for module responsibilities,
dependency rules, verification and the policy for third-party patches. Local
research evidence remains private under ignored directories.

Implementations must be supported by installed data, executable inspection, or
measured behavior; unsupported branches should remain explicitly unsupported.
See `AGENTS.md` for contributor/agent guidance.

This project is not affiliated with Arkane Studios or Bethesda. Owning this
source repository does not grant a license to Dishonored 2 or its assets.
Third-party license notices remain in `vendor/`; no new reuse license is granted
for project-authored code by this publication.
