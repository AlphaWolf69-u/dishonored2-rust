# Dishonored 2 Rust rewrite

An independent, unfinished Rust rewrite using assets from a **legally owned,
locally installed copy of Dishonored 2**. This repository contains rewrite
source and development tools, not the original game or its assets.

The Windows client currently supports selected world/model formats, visibility
and asset streaming, authored material textures, basic first-person movement,
and decoded collision. It is **not a complete playable replacement**: NPCs,
combat, missions, saves, animation, and parts of lighting/material rendering
remain incomplete. Visual and gameplay parity are still being verified.

## Requirements

- Windows and the Rust MSVC toolchain (`rustup`, Cargo, and Visual Studio C++
  build tools).
- A recent stable Rust compiler and a graphics driver supporting Bevy's renderer.
- Dishonored 2 installed through Steam, or `DH2_GAME_BASE` set to its `base` directory.
- Disk space for Cargo dependencies. No map extraction is required.

## Build and test

From the repository root:

```powershell
cargo test --locked
cargo test --locked --features game --lib
.\build-game.ps1
```

The build script creates `bin/` and copies the release client there. Dependencies
are downloaded on the first build; `--offline` is only appropriate once cached.
Tests marked ignored need a local game installation, private research captures,
or a GPU. Numeric regression fixtures ship in `tests/fixtures/`; the normal suite
does not need extracted assets or the private research directory.

Patched Bevy crates are included under `vendor/`, with their original MIT and
Apache-2.0 licenses. Their licensed auxiliary assets are not Dishonored assets.

## Run with installed resources

```powershell
.\bin\dishonored2-rust.exe
```

The client discovers Steam libraries and AppID 403640's installation. It reads
world geometry, entities, visibility, collision, models, textures, probes and
declarations directly from the installed indices and resource archives. Compiled
program definitions are read from the installation's shader cache. The rewrite's
own shaders are embedded in the executable. Launching does not require the
repository to be the current directory or any pre-extracted map files.

The default level is `eotw_p_lowchaos`. Select another installed world by its
unambiguous stem or full archive resource path:

```powershell
.\bin\dishonored2-rust.exe menu_p
```

To override discovery, set `DH2_GAME_BASE` to the installed game's `base` directory.
An invalid override reports an error. The tools can list archive resource names;
their optional extraction command is for research, not a launch prerequisite.

## Controls and diagnostics

- Click the window to capture the mouse; Esc releases it.
- WASD moves, Shift moves faster, Space jumps. G toggles walking/free-fly;
  Q/E move vertically in free-fly.
- F8 saves a screenshot and collision/render diagnostics under ignored
  `artifacts/`. Use matching captures when reporting a rendering defect.
- `DH2_GAME_BASE` selects the installed asset directory.
- `DH2_RENDER_PROFILE=1` enables rendering timing/streaming diagnostics.
- `DH2_BENCHMARK_SECONDS` runs a fixed-camera benchmark; it disables movement
  during that measurement.

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
