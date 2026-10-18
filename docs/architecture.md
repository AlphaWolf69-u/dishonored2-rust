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


## Background materials and fog-volume routing

`material_surface::Program` distinguishes `arkVista` from ordinary layered
materials. Vista images stream through the same installed-archive loader as
other textures, including the UV1 lightmap (image role 22). Its shader uses
lightmap RGB for indirect light and lightmap alpha for sun visibility; ordinary
mesh probes, local lights and cascaded shadow visibility do not enter this BRDF.
Vista sun direction and colour come from the environment globals, independently
of placed light entities or shadow-atlas readiness; Dust has no placed sun entity.
`rendering/vista.rs` packs authored/registered parameters; `dh2_vista.wgsl` owns
the independent lighting equations. Defaults come from installed renderParm
declarations, including omitted gloss-boost values.

`tests/fixtures/vista_lighting_cases.txt` contains only synthetic inputs' numeric
outputs measured by executing the installed HLSL through D3D11. The offscreen
material test compares 64 cases against the production WGSL and separately
checks the real material layout, UV1 sampling, luminance scaling and alpha clip.
The private generator is `analysis/probe_vista_lighting.py`; it reads shader
captures locally and must not publish the captured game shader source.

`atm/worldDistanceFog/arkInteriorVolume` writes signed linear depths with additive
ONE/ONE blending into the original interior-volume fog target, without depth
testing/writing. It is excluded from forward colour submission by program class,
across all levels, rather than by object/material path. The understood boundary
accumulation lives in `rendering/fog.rs`. **The distance-fog render graph and its
volume accumulation target are not implemented yet.** Exclusion corrects opaque
doorway blockers, but does not add fog. The original consumer is recorded in
`analysis/decompiled/chunk_000124.c` (the `interiorVolumes` and
`interiorWorldVolumes` passes). Next work must connect those depth targets to the
world-distance-fog integration, using the retained environment parameters; it
must not substitute colour or opacity for fog depth.

Background coverage remains incomplete for `arkSky` and `arkClouds`, and the
HDR/depth-dependent `arkWater` path. Their atmospheric integration, sky depth-mask
and time inputs need a dedicated pass implementation. Installed program captures
are retained privately in `artifacts/dust-background-programs.txt` and
`artifacts/dust-background-helpers.txt`. The one-argument star `rand` helper is in
`artifacts/shader-raw/helper.decl`; the remaining integration question is the
world environment/fog parameter publication and pass ordering, not that helper.
Do not treat the generic unsupported-material fallback as faithful coverage.

F8 and streaming diagnostics belong under the checkout's `artifacts/` directory
(or beside a packaged executable), independently of virtual archive resource
names. F8 surface rays cover the camera's far range so distant vista hits can be
identified without looking at captured images.

Environment sunlight is also transported as an ordinary backend directional
light whenever the environment publishes its sun direction. A placed
`LIGHT_PRESET_SUN` is the same sun owner and must not add a duplicate when that
world environment exists. This allows ordinary surfaces and the screen-shadow
producer to work on maps such as Dust, which have no placed parallel sun actor.

The Dust shadow audit exposed another valid `arkDefault` bank-2 pairing: the
UV-exporting VS `63aba4ead9c2c3aab263a36ae8ed362c` with **no pixel stage**.
Its projected position instructions are identical to the supported depth-only
world path; the extra UV export is unused. Its original CB2 offset is word 4,
whereas the position-only original VS uses word 0. The backend's rewritten
opaque ABI transports the logical instance offset explicitly and never binds
those original constant buffers directly. Actual installed VS stream output
matched all 128 independent world-position oracle cases, word for word
(`analysis/probe_world_uv_depth.py`, private outputs under `artifacts/`).

Rejecting that pairing used to invalidate the entire local-shadow submission
when any gathered light selected one of those casters. Camera-dependent caster
selection therefore caused otherwise valid floodlight shadows to disappear.
The pairing now resolves to the verified opaque depth path. The installed Dust
material audit checks every world material through the original compiled bank;
set `DH2_AUDIT_LEVEL=dust_p_lowchaos` when running the ignored
`native_world_shadow_sources_resolve_level_materials` test. All 1,141 materials
resolve, including 997 shadow-enabled variants. No material-name exceptions or
unverified shader aliases are used.

`DH2_CAPTURE_LOCAL_CPU` enables optional numeric snapshots of selected lights,
projections, caster IDs and cache allocations. Receiver tracing remains numeric.
The two camera-angle replays and GPU samples are retained under
`artifacts/dust-flood-a*` and `artifacts/dust-flood-b*`. Normal play leaves these
capture switches unset.
