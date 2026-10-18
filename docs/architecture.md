# Architecture and maintenance

The desktop client and diagnostic executables use one library. A format parser
or original engine rule is compiled once and has one test suite. The `game`
feature adds Bevy integration; asset inspection and CPU rules work without it.
`Cargo.toml` explicitly lists executables, so helper modules cannot accidentally
become programs.

| Location | Responsibility |
| --- | --- |
| `src/assets/` | Read installed indices, archives, shader caches and original file formats; locate the installation and resolve native level companions. |
| `src/rendering/` | Original material, model, light, image demand and render-state rules expressed as CPU data. |
| `src/rendering/shadows/` | Local shadow cache ownership and spatial queries. |
| `src/rendering/bevy_backend/` | Bevy resources, GPU buffers, pipelines, depth, exposure and shadow passes. |
| `src/visibility/` | Tome spatial, cell, portal, coverage and caster algorithms. |
| `src/gameplay/` | Game clocks, navigation, collision queries and entity runtime. |
| `src/platform/` | Read original user settings, task scheduling and profiling. |
| `src/app/` | Compose systems, own scene state, load a level, prepare materials, stream assets, accept input and produce diagnostics. |
| `src/bin/` | Executable entry points and command-line inspection reports. |
| `assets/shaders/` | Rewrite-authored WGSL embedded in the executable. |
| `tests/fixtures/` | Numerical executable-comparison vectors and test camera metadata, without original game assets. |

Root module re-exports expose the same shared implementations to diagnostic
tools. New APIs should follow the owning subsystem rather than add
executable-local copies. `FUN_...` addresses belong in documentation for evidence,
not in ordinary API names.

## Naming

Game behavior is the default. Name files, modules and fixtures by what they do;
do not prefix them with `native_` or `original_`. For example, `game_settings`
reads installed settings, `local_caster_query` selects local-light casters, and
`shadow_pipeline` builds the GPU shadow pipelines. Keep executable-comparison
provenance in test descriptions and comments.

Mark integration and deviations explicitly: `bevy_backend/` owns host-renderer
integration; use descriptive `debug_`, `approximate_` or similarly explicit names
for a deliberate departure from game behavior. A renamed file is not evidence
of parity; the project's evidence and verification requirements still apply.

## Dependency rules

Application modules may compose all subsystems. Shared parsers and CPU rules must
not depend on application state or the executable. Bevy integration belongs in
`bevy_backend/` or feature-gated adapters. Keep archive IO out of per-frame queries:
resolve immutable resources at startup and stream through the existing workers.
Do not create separate Cargo crates merely to mirror folders; original format
and renderer metadata have legitimate cross-references within this library.

Keep module dependencies explicit. Shared application state is private to
`app/`; sibling modules use only parent-visible types and functions. Tests can
exercise private integration through child modules. Put new coverage beside the
owning algorithm rather than growing the client integration test collection.
Preserve source-backed numeric behavior when moving code. A structural change
must not silently introduce different material defaults or renderer algorithms.

## Installed resources

`assets::installation::game_base` uses an authoritative `DH2_GAME_BASE` override
or Steam library manifests for AppID 403640. The catalog retains native archive
override ordering. `assets::level::LevelResources` accepts an exact world resource
or an unambiguous stem and resolves its entities, Tome and compiled physics.
Physics names come from the indexed level directory; they need not match the
world stem. Ambiguous companion resources produce an error.

There is no production fallback to `artifacts/` or an extracted map. Optional
private captures are diagnostic/test inputs only. Never distribute archive
contents, original shaders, decompiled code or executable bytes. The client's
own shader sources are embedded, so launch is independent of the working folder.

## Verification

```powershell
cargo test --locked
cargo test --locked --features game --lib
cargo check --locked --all-targets --features game
.\build-game.ps1
```

The CPU suite includes numerical cases measured against original functions.
Ignored tests explicitly identify installation, private capture or GPU needs.
On a development machine with the game installed, run the direct archive loader
test and `authored_surface_gpu_pipelines_compile` separately with `--ignored`.
The latter checks actual shader pipelines and numeric GPU outputs. Final visual
and play confirmation belongs to the user.

Build and runtime output, performance measurements and private byte-for-byte
archive comparisons remain under ignored `artifacts/`. The release build script
checks that the executable copied into `bin/` matches the built executable.

## Third-party patches

Each patched Bevy crate includes `PATCHES.md`, listing purpose, changed files and
validation. Retain upstream licenses. Compare against the matching upstream
release before upgrading; apply intentional changes individually and rerun GPU
pipeline and numeric checks. Never modify the Cargo registry cache. The original
game's material and shadow logic belongs in project code, with dependency patches
limited to the hooks and GPU data layouts needed to express it.
