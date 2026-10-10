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
- Dishonored 2 installed locally. Set `DH2_GAME_BASE` to its `base` directory.
- Disk space for Cargo dependencies and locally extracted map files.

## Build and test

From the repository root:

```powershell
cargo test --locked
cargo test --locked --features game --bin dishonored2-rust
.\build-game.ps1
```

The build script creates `bin/` and copies the release client there. Dependencies
are downloaded on the first build; `--offline` is only appropriate once cached.
Tests marked ignored need a local game installation, extracted assets, runtime
captures, or a GPU. They are not part of the asset-free test suite.

Patched Bevy crates are included under `vendor/`, with their original MIT and
Apache-2.0 licenses. Their licensed auxiliary assets are not Dishonored assets.

## Prepare your own map

Set the installation path, adjusting it for your Steam library:

```powershell
$env:DH2_GAME_BASE = "C:\Program Files (x86)\Steam\steamapps\common\Dishonored2\base"
cargo build --locked --release --bin dishonored2-tools
.\target\release\dishonored2-tools.exe verify $env:DH2_GAME_BASE
.\target\release\dishonored2-tools.exe find $env:DH2_GAME_BASE "eotw_p_lowchaos"
New-Item -ItemType Directory -Path artifacts -Force
```

The search prints the exact resource paths in your installed archives. Extract
one resource at a time using the paths it reports:

```powershell
.\target\release\dishonored2-tools.exe extract $env:DH2_GAME_BASE "<exact .bwm resource path>" artifacts\eotw_p_lowchaos.bwm
```

Also extract the matching `.entities`, `.bphysworld`, and `.tome` resources into
`artifacts/`, using the same filename stem. Collision needs `.bphysworld`;
visibility needs `.tome`. Material textures and placed models are read directly
from the installed archives. The tools do not modify the installation.

Launch from the repository root so the rewrite's shaders are available:

```powershell
.\bin\dishonored2-rust.exe artifacts\eotw_p_lowchaos.bwm
```

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

Reverse-engineering findings and function coverage live in a **local** SQLite
workspace, not in Git. The database and historical findings snapshot are not
shipped. The project-owned tooling can initialize a new workspace:

```powershell
python knowledge\db.py refresh
python knowledge\db.py search streaming
python -m unittest discover -s knowledge -p "test_*.py"
```

Implementations must be supported by installed data, executable inspection, or
measured behavior; unsupported branches should remain explicitly unsupported.
See `AGENTS.md` for contributor/agent guidance.

This project is not affiliated with Arkane Studios or Bethesda. Owning this
source repository does not grant a license to Dishonored 2 or its assets.
Third-party license notices remain in `vendor/`; no new reuse license is granted
for project-authored code by this publication.
