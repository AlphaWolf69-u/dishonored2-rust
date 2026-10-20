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
builds the canonical `target/release/dishonored2-rust.exe` directly; launchers use
that executable without copying it.

## Third-party patches

Patched dependencies live under `vendor/`, with patch notes recording purpose,
changed files and validation. Retain upstream licenses. Compare against the matching upstream
release before upgrading; apply intentional changes individually and rerun GPU
pipeline and numeric checks. Never modify the Cargo registry cache. The original
game's material and shadow logic belongs in project code, with dependency patches
limited to the hooks, GPU data layouts and resource lifecycle fixes needed to
express it reliably.

The DX12 HAL pools completed native command lists together with their dedicated
command allocators. Keeping one allocator per logical encoder spreads the
largest batch's retained storage across the entire encoder pool. Each native
list now retains its own allocator through submission; the pair enters the
shared pool only after GPU completion, and checkout resets both before use.
Discarded recordings remain exclusive and can be reused without a GPU wait.
Command order and rendered work are unchanged. The Windows command-pool GPU
regression rotates large batches among 80 encoders, checks memory growth and
reads back the rendered depth. HAL patch notes document the ownership contract.

Naga's HLSL texture-query fix selects the correct `GetDimensions` output for
array layers, mip counts and sample counts. Its SPIR-V layer query uses the
non-mip opcode for storage and multisampled images. The ignored
`texture_query_dimensions_layers_levels_and_samples_gpu` test validates these
outputs on the actual Vulkan and DirectX 12 devices; shadow GPU readbacks also
exercise depth-array layer counts.


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

`arkSky` and `arkClouds` use their authored colour, star, opacity, animated UV,
density, scattering and back/rim-lighting equations in `dh2_sky.wgsl`.
`rendering/sky.rs` packs registered parameters; `MaterialResolver` supplies the
inherited environment direction, colour and phase. The backend projects sky
vertices to far depth and honours `cloudProjOnFar`; neither program writes depth.
Their prepass discards rather than occluding the rest of the environment.

`app/sky_sprite.rs` implements the environment's `PostFX_Sun_Sprite` disc using
its actual texture, direction or independent polar angles, size, colour and
additive/alpha blending. `FUN_14030a800` projects the world-space direction scaled
by `r_zfar * 0.9`, sizes it by `SunSpriteSize * 0.01`, and applies viewport aspect
to its vertical extent. The disc draws at far depth, behind scene geometry, and
participates in HDR exposure. Dust deliberately supplies a moon texture. Ordinary
visible light-source meshes continue to use their emissive maps and masks;
emission does not substitute for their scene-light illumination. Authored mesh
surfaces using `arkSimpleSprite` also sample their `textureMap` with vertex
colour and the program's alpha blend; they do not use ordinary mesh lighting.

`particle/filing_container` mesh materials use `rendering/filling.rs` and
`dh2_filling.wgsl`: two animated distortion maps, diffuse/lookup colour, authored
fill level, colour adjustment, alpha clip, optional Fresnel normal/LUT and world
clip plane. Forward/prepass share the procedural alpha, with the original
prepass exclusion of Fresnel. Pixel-lit materials use the existing Ward/probe/
direct-lighting path. Unlit mesh liquids use the original exposure-compensation
amount, including crossbow bolt liquids. `MaterialExposure` links the gameplay
view's GPU adaptation state to the material buffer: forward drawing reads its
previous value, then postprocessing updates that same buffer. Other views keep
independent state unless explicitly linked. The host's debug-unlit view preserves
HDR units without changing the material's authored lighting option. Exposure
bindings currently follow the single gameplay view; additional world views must
receive their own matching material/exposure-state binding. Image
roles 23–28 and 29–31 supply liquid and cloud maps through archive streaming.
Mesh liquid materials also honor authored particle distance fading in forward
and depth passes. The input is DirectX pixel clip W, verified in the installed
pixel bytecode. The camera transform reconstructs that value consistently
across GPU backends. Registered fade
parameters occupy spare uniform components; materials without the option keep
their compiled path. Numeric cases cover equal and reversed thresholds, and
the GPU fixture checks matching color and depth clipping.
`rendering/particles.rs` contains the understood fade, soft-contact arithmetic,
animation crossfade and distortion helpers. Emitter simulation and the scene
depth/particle-center producers for other soft-contact modes remain separate.
`filling_cases.txt` contains numeric results from 64 installed-HLSL UV/fill/colour
cases; the GPU test also draws full/empty/Fresnel liquid and checks celestial
occlusion without looking at images.

Remaining render-graph coverage: the sky/cloud secondary fog-depth mask is not
consumed until world-distance fog is connected; physical atmosphere LUT
composition and the HDR/depth-dependent `arkWater` path are also pending.
The corresponding installed shader captures remain private under
`artifacts/dust-background-programs.txt`, `artifacts/dust-background-helpers.txt`
and `artifacts/sky-atmosphere.decl.txt`. General particle emitter simulation,
particle soft-contact inputs, vertex-lit particle producers, animated-emissive mask
production and lens-flare postprocessing are not implemented. A mesh liquid that
requests an unavailable producer is rejected explicitly rather than passed
through ordinary white material shading. Next steps are the native particle
instance/clock publishers and postFX pass dependencies. Adding debug light
markers or synthetic bloom would not provide their authored appearance.

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

Mesh decals preserve the material's signed `zbias` (renderParm default zero).
Its value uses the shared material token parser and registered Uniform defaults,
including case-insensitive names and root-scope overrides. Both `arkdecal` and
`arkDecal` declarations follow this same loader path.
The installed `renderprog/arkdecal` vertex program subtracts it from homogeneous
clip Z before division by W. It is a vertex uniform, not a depth-format-dependent
integer raster bias. GPU checks place a decal behind its receiver and exercise
zero, negative and positive offsets across four distances.

The backend's perspective light grid uses one depth slice when all visible
light spheres fit before its first-slice boundary. A zero logarithmic interval
previously produced infinite factors and inconsistent CPU/GPU slice indices,
making nearby illumination disappear. Single-slice CPU assignment, forward
lookup and GPU clustering now share finite extents; this does not alter authored
light range, original Tome visibility, attenuation or shadow selection.
`DH2_TRACE_LIGHT_VISIBILITY` optionally records authored visibility, backend
visibility and CPU cluster memberships without inspecting screenshots.

Authored texture payloads retain their storage format through catalog loading,
mip validation and GPU streaming. Single-channel half-float images use
`R16Float`, including atmosphere lookup curves; they are not decoded through
the diagnostic RGBA8 image helpers. The GPU format test checks signed and HDR
values, mip selection, linear filtering and the default sampled channels.

`rendering/fog/world.rs` owns prepared distance/height fog parameters and the
understood shader calculations. Its synthetic numerical reference comes from
executing the installed fog helpers. This is groundwork for atmosphere
rendering; the runtime fog pass still needs its interior-depth, sky/cloud mask,
scattering and turbulence producers. It must consume those authored inputs
before being enabled. The interior-volume depth contract remains separate from
opaque forward surface materials.

Tome support-plane, distance and portal-pixel kernels retain the game's packed
SSE arithmetic and comparison order. Their scalar test oracles and captured
original-executable mask fixtures check signed zero, NaNs and individual pixel
bits. Portal macroblock selection and mask propagation remain separate from
the packed pixel kernel.

The shadow backend stores pipelines in an append-only table with typed IDs.
Description hashes resolve pipeline choices during preparation; prepared draws
index the table directly. Pipeline creation finishes before draw references
are borrowed. IDs retain their meaning for the lifetime of the owning GPU
scene, including cached descriptors. Semantic keys remain available for
validation; live uniform contents and the publisher's draw order remain inputs
to every submission.
Resource-binding cache keys use the same owning table's pipeline IDs, avoiding
repeated hashing of complete pipeline descriptions for every draw.

Cached shadow descriptors retain the publisher's immutable draw-list generation
through `Arc`, keeping asset ownership without copying caster constants or handles.
Exact descriptor, uniform-layout, geometry and resource checks govern reuse.
Live shader constant bytes are uploaded independently on every frame.

Directional draw lists and complete local-shadow submissions are published
through `Arc`. Render extraction shares an immutable generation rather than
cloning all caster constants and asset handles. A new publication replaces the
main-world reference; the rendering world retains the preceding generation
until it finishes using it.
The directional publisher consumes the world/model builders' completed lists
after receiver and frame validation. It preserves world-before-model order;
failed validation leaves the builders' inputs available.

Each ordinary Tome worker prepares one immutable `PortalQueryProjection` from
its matrix and winding flag, as the game does in query-context setup. All
spatial records and portal links share its six face-vector sets. A one-off
projection wrapper remains available to probes. Setup errors are consumed only
by clipped portals, preserving the no-portal sentinel path. Existing portal
masks are updated in place when their rectangles already contain the incoming
tile; growth retains transactional replacement so a failed update leaves the
previous coverage intact.

The forward texture-demand producer folds repeated image-area contributions
before publishing through the existing atomic maximum. This is a host scheduling
optimization of the area-only helper; packed streamer metadata has its own
unchanged model. The producer borrows the scene queue exclusively in PostUpdate,
while streaming consumes demands in Update. No demand clear can interleave with
that fold. Logical submission counts and validation order are retained, and a
later mapping error still publishes the valid prefix's contributions.

Scene construction resolves Tome/BWM/material associations into a compact image
slot table after all material bindings are available. Visibility submits demand
through those slots, avoiding per-frame material hashes and scattered render
instance reads. Texture uploads retain the same logical image slots; replacing
the scene or its material associations requires rebuilding the table. The
reference association walk remains available, and visibility verification
compares per-image area bits, logical counts, and deferred errors against it.
Invalid associations retain their valid prefix and fail only if selected by a
query with an area metric.

Shadow mesh allocation metadata stays in one table across unchanged allocator
and render-mesh epochs. Entries mark the submission that used them; completion
removes unused entries, and failure or epoch changes invalidate the cache.
Repeated draws resolve one allocation per submission. GPU tests compare against
fresh lookups across geometry growth, index-format changes, asset replacement,
and removal.

Shadow material bindings follow the same submission lifetime: fresh preparation
marks the image/UV bindings it uses and releases unused cache entries. Streamed
mip replacements must not leave old texture views alive in pipeline caches.
Prepared draws and submitted commands retain their own GPU handles. Periodic
`dh2::shadow_resources` diagnostics report the live and retired binding counts.

Prepared shadow records share immutable mesh-allocation metadata owned by the
allocation cache. Retained records keep those GPU buffers alive through `Arc`;
asset epoch changes still invalidate reuse. Consecutive identical world-buffer
and resource-binding lookups are reused only within one immutable render
submission. Each draw still validates its own buffer ranges and constants.

Directional slices record independently, including retained static caches. Each
slice owns its temporary surface and static-cache texture. Its cache restore or
refresh, dynamic pass and atlas copy remain ordered within one encoder. The
backend finalizes those encoders concurrently, then submits them in owner layer
order and publishes completed cache generations. Serial recording remains
available for numeric comparisons. GPU checks compare depth and stencil across
cache reuse, generation changes, empty draws and atlas replacement.

Independent shadow encoders join the common deferred-finalization batch with
camera commands; recording does not wait for their finalization. The render
context flushes its current encoder before appending an independent encoder,
so mixed ready/recorded buffers retain their exact GPU submission positions.

Prepared shadow records keep the owning pipeline table's ID rather than copying
its full semantic description per draw. The append-only table retains that
description; exact source-descriptor comparisons still validate cached reuse.
Cache resource snapshots skip repeated hashing of consecutive identical IDs
while retaining the complete resource set and live GPU identity checks.

Visibility output lists reserve their bounded native capacity up front. Shadow cache
misses recover the preceding draw vector only when `Arc::try_unwrap` proves
exclusive ownership. Retained readers prevent recycling, and verification hits
keep the preceding snapshot intact. Every reused slot is prepared afresh before its preceding references are released;
truncation removes records beyond the current submission count.

Gameplay performance comparisons use the ordinary borderless launch path and
verify the foreground window's physical rendering viewport. A forced windowed
benchmark is a separate presentation-path diagnostic, even when its pixel
size matches. Moving-camera runs determine acceptance; stationary views only
help isolate costs. Shader/GPU oracles and profiling runs remain separate from
clean frame-rate measurements.

Changed shader-buffer assets use a shared staging belt in the render world.
Their preparation batch submits all copies before dependent asset systems run;
wgpu returns staging chunks for reuse only after GPU completion. Destination
buffers, uploaded bytes, resize rules and render order remain unchanged. This
reduces temporary upload resource churn without changing game resource budgets.
