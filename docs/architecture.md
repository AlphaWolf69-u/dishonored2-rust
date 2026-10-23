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

Initial render placements apply component startup visibility. Umbra gate
components hide their owner models when initialized, independently of model,
material or entity names. The parsed entity and `placed_models` data remain
available to visibility and gameplay consumers. Runtime gate toggling and its
script-driven connection to model visibility remain separate gameplay work.

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

Machinery asset bindings live in `assets/entities/machinery.rs`; compressed
shape mass properties live in `assets/physworld/mass.rs`. Both read the installed
archives through the shared catalog. `gameplay/machinery.rs` evaluates constraint
motor commands and measures preceding relative motion. `gameplay/rigid_body.rs`
owns mass-frame initialization and pose integration. `gameplay/constraint_solver/`
owns solver phases and coupled fixed-pivot rows; its parent module owns angular
motor and axis-lock rows. These CPU routines have executable comparison fixtures.
`app/model_motion.rs` publishes body deltas to geometry, visibility queries and
both shadow-bound sources, retaining authored scale, shear and blueprint offsets.
Its integration tests exercise that handoff without a graphics window.
`assets/entities/machinery/recipes.rs` resolves complete connected assemblies and
shape mass frames directly from archives. `gameplay/machinery_world.rs` owns their
bodies, fixed pivots, hinge alignment, angular limits, friction and motor history.
`app/machinery.rs` advances that owner from the game clock and queues rigid pose
deltas before transform propagation, visibility and shadow collection. Meshes
and per-body solver storage are reused. Missing entity endpoints retain the
world anchor, following the constraint body's initialization rule.

Scripted machinery uses `assets/entities/scene_motion.rs` to read motion-only
scene loops and their startup graph bindings from the installed archives.
`gameplay/motion_curve.rs` evaluates unwrapped Euler curves, including constant,
linear, Catmull-Rom and Bezier interpolation. `gameplay/scene_motion.rs` follows
the relevant loading events, named variables, relays and runtime entity inputs
before starting playback. A scene reference or editor preview binding alone
does not start an animation. Angular motor-control nodes apply the authored
enabled state and selected position, velocity and acceleration targets to the
existing constraint motor, including timed changes from the current values.
Loading events therefore apply the level's ordinary speed before physics runs;
the same event handler accepts authored storm notifications when weather
production is integrated. Physics and scripted motion publish through the
same `app/model_motion.rs` handoff, with stable meshes and visibility/shadow
bounds. Interpolation, angle conversion and reference-frame composition have
numerical fixtures from the installed executable. Moving models retain their
Voronoi cell/tetrahedron query and update indirect lighting from the center of
their current world bounds. All material surfaces use that model query; vertex
density does not move a surface's lighting sample. Probe records keep stable
indices and the same GPU storage allocation. This covers autonomous
single-scene motion loops; general cinematics, scene seeking, weather-event
production, skeletal tracks and saved playback state remain separate work.

This covers authored motor-driven machinery and its supporting fixed-pivot
hinges, including dynamic cages connected to blades, inherited constraint
settings and material-derived mass. Generator-owned rotors initialize from
their `cpntWindMill` declaration speed before loading scripts run; temporary
speed overrides retain the original negative-value reset rule.
All 36 installed level files resolve their
active motor assemblies. General contact-driven physics, skeletal/articulated
joint endpoints, gameplay start/stop events and save-state restoration remain
separate work. Unsupported assembly recipes produce explicit diagnostics rather
than replacement animation. Rendering is driven by the simulated pose; no
object-name rotation rules or extracted runtime files are used.

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

Ordinary placed props and world surfaces use the installed HDR reflection cubemaps as well as
diffuse probe lighting. `assets/bprobe.rs` preserves the cubemap names, parallax
planes and optional alternate capture records; `assets/texture.rs` validates
all six faces and mip records. `app/environment_reflection.rs` uploads the
authored BC6H data directly into a shared cubemap array. Plane packing and
primary-to-reflection probe mapping follow the executable; `dh2_reflection.wgsl`
evaluates the HQ Ward lobe, parallax correction and luminance factors. The
vegetation path samples its registered RG32Float Ward lookup instead of using
the ordinary material's analytical integral. The
per-object SH record also transports its selected reflection probe and gain,
and both update with model motion. World vertices select their reflection probe
and interpolate its luminance factor from the same tetrahedron weights as SH.
The retained normal-W corner channel transports three probe indices through
perspective interpolation; each cube gets its own parallax correction, with a
single sample when all three indices agree. Numeric tests cover both shader
variants and actual compressed HDR uploads. Alternate-capture weather blending
and the special 512-probe residency mode
remain separate renderer work.

`assets/level.rs` resolves probe networks in the exact indexed world directory.
Substring matching cannot distinguish the manor from its boat level and must
not determine whether a level has indirect lighting.

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

Lights marked `m_scatterOnly` retain their source entity, visibility admission,
fog packets and volumetric shadow owner. They receive no host surface-light
component: the installed TLF shader excludes this flag from its ordinary light
lists. This applies to point, half-point, spot and area sources across levels.

`atm/worldDistanceFog/arkInteriorVolume` writes signed linear depths with additive
ONE/ONE blending into the original interior-volume fog target, without depth
testing/writing. It is excluded from forward colour submission by program class,
across all levels, rather than by object/material path. The understood boundary
accumulation lives in `rendering/fog.rs`; the render-world owner in
`rendering/bevy_backend/fog_scene.rs` submits the retained geometry to the signed
depth target and consumes it in distance/height fog. The owner also supplies
current camera depth, the sky/cloud mask, authored LUT/noise, environment
parameters and directional shadow resources. GPU tests compare numeric output
and exercise the actual Core3d owner without a window. The same owner submits
placed-water depth and local-light scattering. Ocean depth and the underwater mask remain separate unfinished parts of the water sequence.

`fog::lookup` selects the two retained fog-curve history banks and the separate
override output. It preserves ordered weight decreases, raw transition weights,
the executable's NaN/negative-zero clamp behavior and the override admission
threshold. Its 120-frame fixture executes the game's actual fog owner with only
resource/command boundaries observed. `bevy_backend::fog_lookup` owns the three
4096x1 R16F images, initial values, blend and weighted-override programs. It
reuses Direct3D point-coordinate conversion for curves with different extents,
and keeps override output separate from base history. `FogLookupTrack` publishes
the target and fraction independently of ordinary images' halfway selection;
55 executable updates cover area entry, exit and interrupted transitions.
The atmosphere owner now submits the prepared curve before world fog. The three
banks share one texture allocation: world fog reads its selected 2D layer, while
transparent materials read the same array using a reserved backend parameter
lane. Curve transitions require no image copy or material rebinding. Immutable
inputs are blended only when their view identities or raw weights change.
Scripted/external curve controllers still need their producer connected; their
four-image weighted payload is supported by the frame and render owner.

`fog_shadow.rs` contains the full parallel/projected-height raymarch and temporal
sampling rules. The fog scene prepares the authored height image and selects
between the two installed fog-shadow materials by resource identity against the
registered default. Material options control projected and temporal branches;
they are resolved through the program registry, separately from environment
uniforms. The pass uses integer pixel coordinates and its actual viewport for
reconstruction, then stores the result in the source R8 target.

The sun sprite's registered `Texture2D<float>` input samples red and broadcasts
that scalar to colour and opacity; the image's stored alpha is not used.
Its raster regression executes the installed cached pixel shader, including
the exposure-buffer and white-level conversion, against the rewrite's output.

`FogPass::record_sources` records the completed, filtered fog image separately
from HDR composition. Its borrowed `FogSource` exposes that image for water
sampling and consumes itself when recording the final blend. The borrow keeps
targets and bindings stable between these phases. `FogPass::record` retains the
existing consecutive sequence for views without intervening water rendering.
The GPU composition test writes HDR between the phases and checks the result
against the installed shader's numeric reference.

`arkBoatVolume` is another geometry-only material: its installed fragment
program has no output. `MaterialHandle::BoatVolume` retains its mesh and authored
identity without a forward material or texture requests. Scene loading preserves
resolved `stageSort` and `stageSortAdjust` instead of dropping a boat-proxy
filename. The material-list selector follows runtime sort boundaries, while the
constructor separately excludes `sortBoatVolume` from ordinary shadow gathering
and forces the final `arkWater` sort after overrides. Boat/water depth-stencil
states and shared indexed/nonindexed mesh submission have numeric tests.
`water_depth_scene` submits visible placed-water depth, its motion output,
opaque restoration and the separate full/half alpha-depth copies. `boat_scene`
publishes visible world and rigid-model boat volumes through the same
`water_depth_meshes` allocator and pipeline owner. It inserts their depth and
stencil phases before water, restores opaque depth inside the resulting mask,
and marks the reduced alpha and actual camera stencil after the alpha copies.
Ocean geometry still requires submission into that sequence. The shared
position-mesh allocation owner is also used by interior fog geometry.

Placed `arkWater` uses `MaterialHandle::Water` and `WaterMesh`, independently
of forward prop materials. The loader retains its complete declaration, typed
material-only options, MAIN program and authored render state in `WaterMaterials`.
The immutable registry is shared across main/render worlds; visible instances
carry current transforms, stable scene order and world/model provenance while
reusing the scene allocator. Unresolved material input errors remain explicit.
The depth owner retains model transforms by frame and uses the shared camera
history for world geometry. The atmosphere owner now submits the admitted
horizontal world water colour paths, with or without probes, between fog-source production and fog
composition, using actual GPU exposure, the current sun atlas, retained material
inputs and shared depth reduction. Other water colour ABIs, ocean and underwater
submission remain unfinished. Mansion and Stilton pools have numeric gameplay checks;
human appearance confirmation remains pending.

`bevy_backend/water_depth.rs` owns the full-size depth/stencil water target,
copies the current view's immutable opaque depth, and restores opaque depth
inside boat volumes after water geometry. It reuses the shared copy shader and
the verified stencil predicate; the alternate task branch preserves stencil.
World depth uses its material-derived cached state and reference zero, whereas
model water uses the separate forced state and reference four. Numeric tests
exercise ordinary copy, alternate copy, and masked restoration on both graphics
backends. The gameplay owner connects placed-water depth to alpha depth, fog
and admitted water colour. Boat volumes use the actual material MAIN state for
world geometry and the forced `arkBoatVolume` state for rigid models. World
stencil reference two and model reference eight remain distinct. The camera
attachment clears stencil with its first depth clear; the reduced alpha
attachment clears stencil once per frame before preserving subsequent masks.
Ocean geometry remains unconnected.

`bevy_backend/underwater_stencil.rs` implements the ocean-mask and water-volume
marking draws over the full camera and reduced alpha-depth attachments. It consumes
the actual wave-height mask or water matrix, preserves scene depth and all
stencil bits except the underwater bit, and retains the boat-interior exclusion.
The two target sizes have independent uniform buffers and bounded bindings.
GPU comparisons use coverage generated from the installed shader definitions
and verify depth/stencil preservation on both graphics backends. Gameplay water
scheduling must still supply current wave images and depth attachments; this
module does not assume a dry view or enable an additional fog filter.
`bevy_backend/ocean_water_line.rs` supplies the wave-aware mask producer using
the shared shoreline-wave functions, actual FFT collision-height texture, shore
distance and amplitude maps. Its ocean marking entry point records mask
generation followed by both stencil targets in the same command stream. Mask
storage and image bindings persist across changing water levels and wave times.
The gameplay owner still needs to publish the current ocean selection and wave
resources and schedule this sequence with water depth and fog.

`rendering/water.rs` also separates water colour admission from visible ocean
patches. `bevy_backend/water_foam_depth.rs` produces the quarter-resolution R16F
foam mask from completed opaque and water depth attachments, then runs the four
authored point/mirror Gaussian draws. It retains its textures, uniforms and input
bindings across frames. The view owner supplies matrices, viewport/scissor and
resolution transforms; connecting this producer to gameplay still depends on
the complete water-depth and water-colour sequence above.
The shared point-sampling helper preserves level-zero Direct3D fixed-point
rounding and mirrored texel selection across graphics backends.

Ocean wave simulation, quadtree selection and grid generation live in
`rendering/ocean/`; their GPU resources live under `bevy_backend/ocean_waves/`.
`ocean_surface.rs` and `ocean_vertex.rs` carry the shared displacement equations
and explicit current/previous view parameters. `ocean_mesh.rs` uploads the shared
grid once. `ocean_depth.rs` submits indexed strips and edge triangles with the
selected program's prepared state. Normal depth writes RG16F motion; alpha depth has no fragment
stage. It preserves caller-owned depth/stencil and requires real wave history,
patch constants, viewport and scissor. Shared GPU state translation lives in
`pipeline_state.rs`, also used by the shadow pipeline. `ocean_colour.rs` provides
the colour pipelines, projected-height shadows, foam sampling and complete water
shader composition. GPU tests compare complete colour, depth and stencil
readbacks against the installed cached ocean stages, including perspective
interpolation, implicit foam mips, nonzero parameter offsets and scissor clipping.
View resource ownership and the complete gameplay water sequence remain
unfinished; these producers do not establish visible gameplay ocean support.
Ocean settings resolve the authored projected-height image, extent and thresholds
from each effective environment. These images are installed resources, with the
registered black image used where the environment specifies it.
`water/settings.rs` resolves the shared optical, caustic, sky and reflection
uniforms through the same registered-default reader. Ocean settings include
these values; shader options and runtime reflection admission remain separate.
The installed-environment audit resolves the combined inputs across all 36 levels.

`water_normals.rs` preserves placed `arkWater`'s distance-dependent Perlin detail
and finite-difference normal, including the tangent-space path for vertical
surfaces. These waves affect shading rather than displacing mesh vertices.
`water_colour.rs` prepares the fragment's inputs to the shared complete water
shading function, with the placed surface's actual height and source constants
for foam and additional visibility. Numeric comparisons execute the installed
functions and fragment preparation on both graphics backends.
`water_probes.rs` preserves PC probe transport: normal-W triangle corners encode
the vertex's cube-array index, and the fragment reconstructs three indices and
their weights after interpolation. The single-probe and disabled branches stay
separate. `water_colour_input_with_probes` feeds these values into the shared
surface input. `probe_weights.rs` supplies the same unsaturated tetrahedral
interpolation to general indirect lighting and water probe selection.
`water_world_colour.rs` implements the world-water vertex stage: instance remap,
game-space transform, PNW reference lookup, strongest interpolated probe selection,
ProbeInfo lookup, cube-array remapping and normal-W transport. It consumes the
existing eight-word PNW buffer layout, with separate ProbeInfo and cube-remap
buffers. Level setup publishes `WaterLighting` with the existing PNW, cube-array
and plane allocations. The reflection loader supplies the primary ProbeInfo
records and the source-ID-to-uploaded-slot buffer once per level. Missing or
invalid associations remain explicit results. Visible instances share these
handles across extraction. `WaterWorldMeshes` retains the raw shared world mesh
for each water object; visible instances preserve their original object and
matrix indices even where the host forward mesh has baked shear. Only those
shared meshes acquire the additional SNORM normal-W probe-corner channel.
`app/world_instances.rs` owns one immutable 48-byte matrix stream per level,
shared by whole-view depth, directional shadows, local shadows and water.
`WorldProbeVertexStage` binds that stream and the shared probe buffers with an
explicit raw-position/normal-W vertex layout. Numeric raster checks cover real
vertex fetch, nonzero allocator offsets, draw-relative instances and projection
against the cached shader's stream output. `water_colour_scene.rs` owns admitted
horizontal world colour draws in gameplay. Cached stage checks admit both the
probe and non-probe variants. The latter binds only instance and matrix buffers
in its vertex stage. The owner retains resource bindings, skips private depth
reduction when SSR is disabled and reports unsupported classes explicitly.
The shared current-depth reduction remains available while host materials
compile; only the persistent adaptive-shadow readback waits for complete depth.
Water material loading also resolves slot4 separately for rigid bank0 and world
bank2, respecting authored depth-program nulls, constructor finalization and the
direct/multi-instancing exclusion. The shader cache admits only the actual
verified rigid/world vertex and motion-pixel bytecode pairs. These sources and
their errors stay attached to the material; they do not imply a completed draw.
Its parameters explicitly compensate for the allocator vertex base and WGPU's
first-instance convention; D3D11's source instance ID is draw-relative.
`water_depth_program.rs` submits rigid water meshes with actual current/previous
MVP rows, the model depth multiplier, and jitter. Its world branch resolves
the remap buffer's low 24 bits, reads the shared 48-byte instance matrices,
and applies current/previous view-projection matrices to the current world
position. It reuses the world-shadow projection arithmetic and applies no prop
depth multiplier. The world parameters likewise remove WGPU's first-instance
base before the source remap lookup. Normal depth writes the
material-selected RG16F motion channels; alpha depth has no fragment output.
The shared motion helper also serves ocean depth. Raster comparisons use the
installed cached vertex/pixel stages and verify all depth, stencil, and motion
samples. Gameplay owns rigid/world depth and horizontal world colour; other
colour ABIs and wind/skinned variants remain unfinished.

Resident BIM texture arrays retain their authored block format and mip/layer
order through `assets/texture.rs` and `bevy_backend/layered_images.rs`. The shared
uploader also preserves cube faces and supplies a cube view. GPU format
mapping is shared with the ordinary texture streamer through `image_formats.rs`.
Registered image defaults include array and cube identities. The floating array
frame index follows the source sampler's nearest/even rounding and layer clamp.

`water_refraction.rs` provides the active water refraction equations, including
depth-validated distortion, exposure decoding, animated chromatic caustics,
parallel shadow visibility and vertical extinction. `screen_ray.rs` implements
the source's hierarchical depth traversal, used by screen-space reflection.
`water_reflection.rs` combines that traversal with the authored sky/probe
fallback, fog and HDR decoding. `water_surface.rs` supplies complete default
above-water and underwater colour composition, including Fresnel, sun reflection
and refraction, foam and geometry-edge alpha. These require explicit current-view
inputs and are tested with numeric GPU readback against installed shader
functions. Their application resource owners and final water colour submission
remain unfinished. The shared `test_depth.rs` uploader and `test_water.rs`
numeric scene are compiled only for GPU tests.

`colour_output.rs` supplies the shared black/white overlay and HDR range
compression. Local-light fog and ocean colour use the same overlay function.
The conversion uses the actual auto-exposure luminance factor and white level,
including the source's unguarded zero-exposure results.
`rendering/water.rs` also preserves the per-view refracted sunlight calculation,
verified against the executed game code; the eventual view owner supplies the
actual parallel-light direction.

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

The sky/cloud secondary depth mask now feeds world-distance fog. Remaining
render-graph coverage includes physical atmosphere LUT composition and the
HDR/depth-dependent `arkWater` path.
Water material preparation now retains colour-program defaults and actual
material numeric/image overrides as separate ordered scopes. The evaluated view
sits beneath them; instance overrides remain a further draw-owner input.
`render_program::parameters::ParameterScopes` preserves missing assignments,
last matching names, raw vector words and named image identities. Scope outputs
match 64 executions of the original slot publisher. All 21 installed arkWater
materials resolve their 367 explicit optical/image assignments. The loader also
retains the selected world-bank colour variant, effective specialization and
that program's defaults. Selection preserves bank exclusion, finalization,
explicit nulls and the SSS-driven choice of `mainAlphaProgram`. Serialized stages
remain separate from GPU ABI admission; their presence does not enable a draw.
Level setup
also prepares registered, world and area water values and retains shared CPU
cube/array image chains. `app/water_scene.rs` publishes evaluated parameters,
game-space camera matrices and gameplay view-local time through `WaterSceneFrame`.
`view_history.rs` preserves the original per-frame current/previous matrices and
camera position, including first-update seeding, same-frame updates, explicit
resets and counter wraparound. Water publication uses the host render-frame
marker and starts a fresh history when its level owner or camera changes.
All 96 executable camera updates match bit for bit; this supplies static world
water's previous view-projection without substituting a model transform history.
Area transitions preserve missing assignments and image resource identity;
ordinary images switch at the original half-transition threshold. The shared
environment track initializes its first selection immediately, including an
area selected at startup. Later source changes retain their authored duration
and restart from the current value. A 36-frame executable fixture verifies
initial selection, zero-time updates and interrupted ordinary area blends.
This rule is shared by atmosphere, display, water and shadow-range inputs.
The shared texture loader supports RGBA32F sky cubes without reducing their
precision.
The placed-water owner consumes these inputs in actual gameplay draws and
uploads the immutable image chains it needs. Ocean and underwater
dependencies remain unfinished.
The world-probe colour program has a view-exposure variant that reads the
camera's existing `MaterialExposure` GPU record for every HDR decode and encode.
It preserves the uniform white level and does not require CPU exposure readback
or another rendering pass. This binding expects HDR samples and output in the
game's storage units. `water_hdr.rs` now owns matching pre-water and writable
colour targets in R11G11B10F or RGBA16F, converts the host primary using the
live exposure record, and restores working RGB while preserving host alpha.
The pre-water source stays separate from subsequent water writes; two bounded
bindings retain the host's alternating views. This reproduces native storage
rounding at the water boundary, without claiming the preceding host opaque
passes already reproduce every native framebuffer store. These targets are
connected to the placed-water colour owner between fog production and composition.

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
`world_shadow_sources_resolve_level_materials` test. All 1,141 materials
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
executing the installed fog helpers. Runtime world fog uses signed interior
depth, the sky/cloud mask and independent world/light turbulence canvases.
`app/fog_scene.rs` publishes environment parameters and images;
`rendering/bevy_backend/fog_scene.rs` owns GPU submission and same-view resource
readiness. `app/fog_lights.rs` publishes admitted lights in scene order, using
the existing preset colours, the original authored-strength conversion and
actual local-shadow indices. The atmosphere owner submits the tiled lists from
distinct opaque/alpha depths, checks shadow-atlas completion and adds local-light
fog before composition. Removing the inputs removes the contribution. The
interior-volume depth contract remains
separate from opaque forward surface materials.

The tiled-light depth gather preserves the measured reciprocal/MAD arithmetic
through depth-bin mapping. Rounding linear Z before that mapping changes the
occupied bin for constant-depth tiles despite identical near/far endpoints.
Native shader fixtures check exact masks on both graphics backends.

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

The gameplay performance overlay samples process memory once per second with
the frame-rate display. `RAM` is the resident working set; `Private` is private
committed memory and includes pages that may be paged out. These process counters
include driver allocations charged to the process and are not GPU VRAM totals.
Unavailable counters are shown explicitly rather than reported as zero.

F8 can request a numerical fog capture through `fog_capture`. The render owner
records the exact scene-depth, directional-atlas, fog-shadow and sampling-buffer
inputs from one submitted view. Mapping is asynchronous after submission;
private binary data and matching view metadata are written by the I/O pool.
No continuous capture, image encoding or gameplay visibility decision depends
on these diagnostic resources. Adapter buffer limits are checked before copying.

On Windows, uncapped HWND output uses the DX12 backend's `composited` surface
owner. Rendering stays on the DX12 queue; a D3D11 device on the same adapter
copies the shared final image into a discard swapchain and presents with zero
interval and flags, matching the measured game window path. Shared fences
protect both the copy and subsequent reuse of the image. The owner drains the
copy context before resize or destruction. This is platform integration;
shaders and bloom composition remain independent of presentation.
`platform/game_settings` reads `r_swapChainTripleBufferEnable` to select two or
three buffers and a frame latency of one or two. Explicit latency overrides
remain available. `DH2_DEBUG_DIRECT_PRESENT=1` selects the direct DX12 flip
path for comparison; enabling VSync retains the existing FIFO path.

F8 also requests numerical display inputs through `display_capture`: the exact
pre-display HDR source, post-lens colour, first luminance mip, final bloom target and 32-byte GPU
exposure record. Copies are recorded after the display producer in the same
command stream. The text sidecar records texture formats, padded row layouts
and the settings used by that submitted frame. Mapping and file writes are
asynchronous. These resources perform no copies or allocations without a
request; the capture adds no shader pass. CPU lens publications in the ordinary
F8 report are labelled separately from this GPU evidence.

`DH2_TRACE_BLOOM=1` enables an optional bounded GPU history for intermittent
defects. Each frame retains synchronized 96-by-54 pixel-centre samples of
post-lens colour and bloom, plus exposure and lens controls. Where the window
supports texture sampling or copying, its final buffer after UI and scaling replaces the
HDR sample in plane zero; the header identifies this stage and frame serial.
Unsupported surfaces and the newest F8 frame retain HDR instead. Its 1,024 slots
occupy about 122 MiB and overwrite the oldest samples; F8 asynchronously saves
the preceding history beside the ordinary inputs. It adds two small diagnostic
compute dispatches per frame only when explicitly enabled and supported. DX12
also uses one reusable window-sized sampling copy because its swapchain cannot
be sampled directly through wgpu. Normal gameplay
allocates no history and submits no diagnostic dispatch. These reduced samples
locate affected stages; they do not establish full-resolution appearance parity.

The authored filmic curve calculation lives in `rendering::tone_mapping`.
`app::display_scene` prepares registered defaults, level and area values, and
resident 3D colour tables directly from archives. Gameplay publishes typed
camera-area transitions for the filmic curve, grading and exposure controls;
it does not parse declarations or load tables during frame updates.

Camera optics follow the same boundary. `app::camera_effects` owns environment
transitions and publishes `rendering::camera_effects::CameraSettings`.
`assets::camera_blur` reads kernel points and the selected material bank directly
from installed archives during level preparation. The render frame shares these
immutable `BlurResources` through an `Arc`; it performs no archive reads to change
the bokeh filter or select an unspecialized material. Material options come from
the selected declaration, including its fallback, rather than from filename
assumptions. The shared settings and material-selection rules also work without
the `game` feature.

The camera GPU components have explicit producer inputs. `camera_masks` owns
near-mask generation, separable filtering and the combined near/far mask.
`camera_colours` owns the ordered HDR/colour/mask downscales, stencil admission
and near/far colour draws. `camera_blur` implements the colour-filter leaf;
`camera_stencil` implements near, far, autofocus and constant-depth admission.
They require the view's real depth/stencil, autofocus, refraction and colour
resources. The colour producer retains its internal targets and bindings across
frames. Its downscale draws use the actual fullscreen geometry and compute tap
coordinates before interpolation, matching the installed vertex program. Draws
queued together retain independent uniform buffers. Existing depth and unrelated
stencil bits survive classification, and disabled branches preserve the game's
clear/retain rules.

These producers are numerically verified separately from gameplay integration.
The combined colour check uses 24 installed vertex/fragment shader sequences on
Vulkan and DirectX 12, repeated through game-HDR and two host-source adapters.
Archive preparation and render-frame publication are connected; the gameplay
render graph still needs the alpha/refraction producers and final camera-combine
ordering. A standalone producer check does not establish appearance parity for
the complete gameplay frame, or emulate the original main HDR target's packed
storage conversion.

`bevy_backend::colour_cube` owns three bounded RGBA8 volume textures for
environment colour-table history and overlays. It preserves the last base
blend when a new transition starts; overlay output has its own texture.
`bevy_backend::tone_mapping` supplies the filmic curve, 3D table lookup and
display gamma as a single full-screen pass with GPU-resident exposure input.
`bevy_backend::display_scene` runs this pass after exposure computation and
replaces Bevy tone mapping only when the view's real inputs are ready. Missing
resources preserve the existing loading fallback. The fragment reads current
GPU adaptation directly; the host removes camera pre-exposure once. Immutable
colour-table inputs are blended only when their views or weights change.
These components are numerically checked against installed shader rules, and
headless render-graph checks cover readiness, fallback and changing exposure on
Vulkan and DirectX 12. Fog-shadow sampling and filtering are unchanged. External
Midnight environment overrides and several other final-display effects remain
unfinished; ordinary camera-area inputs, bloom and lens dirt are connected.

`bevy_backend::bloom` implements the authored upsampling draw with a linear
mirror sampler and the original UV limits and repeated sample offsets. Its
actual raster pass is checked against installed HLSL at full and half precision
on both graphics backends. `rendering::bloom` provides the original inclusive
mip schedule, including the distinct floor-sized source views and ceil-sized
temporary targets. All 192 captured schedules match the executable exactly.
`BloomChain` owns persistent RGBA16F targets with one shared pipeline and separate
uniforms per draw. Chained readbacks match installed HLSL within one FP16 step;
repeated frames reuse the same target views. Gameplay submits this chain before
final tone mapping when lens flares are disabled. Exposure and bloom share a
per-view luminance pyramid: frame-generation tracking fills only missing levels
and prevents stale reads when exposure is unavailable. Resize and camera
retirement release the corresponding targets.
`rendering::viewport` supplies the executable's pixel-to-view and viewport
constants; 128 native calls match word for word. Bloom's complete-target
constructor uses these constants and is exercised by the chained GPU test.
`bevy_backend::lens_streak` supplies the authored angular streak calculation.
Its shader preserves the compiled angle polynomial and fused phase arithmetic:
substituting a backend's ordinary angle approximation changes the 4,096-slice
noise. Numerical checks cover 256 installed-HLSL cases on both graphics backends.
`bevy_backend::lens_composite` combines these inputs in a variant of the existing
tone-mapping pass, without an additional full-screen draw. The shader preserves
lens-dirt selection/blending, HDR expansion, the positive-strength bloom guard,
shifted flare sampling, black-and-white transforms and exposure ordering. The
fused pass is checked against 42 installed-HLSL frames, including poisoned bloom
data at zero strength, several host radiance scales and compressed-HDR input
adaptation. The original compressed mode expands bloom separately; a source
adapter inside the existing final draw preserves the host primary scene
brightness while retaining that bloom scale. This does not reproduce the
original main-target packed-storage quantization. Both-backend render-graph
checks cover actual submission, exposure sharing, changing images, resize,
retirement and fallback. Enabled lens flares retain the ordinary display path
until their real source producer is connected.
`rendering::lens_dirt` supplies the special image transition state, including
world/area/secondary selection, interrupted transitions, missing assignments
and ordered dominant-image overrides. Its outputs match 74 executable updates
word for word. `PreparedLensDirt` resolves immutable image values at level
preparation; updates perform no archive reads. Installed checks cover all 36
levels and 2,812 area snapshots. This image state remains distinct from the
ordinary numeric environment track, whose interpolation rules differ.
`app::lens_scene` now prepares the level's archive-backed lens-dirt and colour
profile images alongside its float and integer settings. `DisplayFrame` carries
these resident handles and evaluated values to the render world. The display
owner consumes the authored dirt pair, intrinsic black for disabled flares,
actual camera-axis term and tracked white level. No archive reads occur during
updates. Preparation checks cover all 36
levels and 2,812 area dictionaries, including the actual BC7 sRGB dirt image.
`rendering::environment_tracks` preserves presence, initialization, source
changes and stored-word comparison for these values. Its float, signed/unsigned
integer and lens-profile results match 221 executable updates; lens-dirt's
separate publication rule retains its own 74-update check. Post-processing
configuration supplies the original mip controls and skip bits without changing
the installed settings file.

`environment_images` prepares the four fog-curve or colour-cube override groups
by resource identity. Explicit parameters replace both weight factors; weighted
environments raise them independently. Zero-weight images still occupy slots,
and contributions above a total of one are normalized in the original order.
All 512 fog publications match the executed manager word for word; the same
512 image/weight results were independently checked on its colour-cube branch.
`environment_overrides` owns the manager's 16 stable environment slots, including
first-empty registration, raw weight updates and deferred removal. Its 512-operation
fixture verifies every slot and the resulting image publication against the game.
The fog frame accepts this prepared blend and retains its existing lookup renderer.
`environment_effects` implements the FX view callback: registration waits for an
eligible view mask, retains that view's identity and updates its slot with the
sampled blend and a second factor of one. Pending removals call only currently
live views and are then cleared. These state changes match 512 executed binding
updates and 64 retirement batches. The ordered registration/removal list matches
512 executed operations, retaining duplicates and queuing view cleanup only for
successfully removed entries with a view and handle.
`gameplay::effects` supplies shared-RNG scalar sampling, timing preparation and
packed Bezier float curves. Its environment blend controller preserves delayed
starts, fade phases, one loop restart per update, explicit stop rules and view
masks. Primitive checks cover 512 samples each; controller checks cover 64
initializations and 940 tick/stop calls with exact stored-word comparisons.
`assets::fx` loads environment descriptors as sparse edits of the executed
constructor defaults, preserving inherited curve entries and zero-initializing
new float entries through declared array counts. The complete constructor's
45 scalar/curve words and 512 original list-reader resizes have independent
checks. All 67 environment descriptors across 8,909 installed FX declarations
load; the 57 enabled descriptors with an environment reference also resolve
their environment resource and evaluate. Group loading preserves location
conditions and the group's independent timing defaults. Parent offset sampling
matches 512 executed group starts. The environment chain walker matches 768
updates across 64 pools, including deferred retirement, shared-RNG update order,
free-list reuse and retained inactive state. Definitions can be shared through
`Arc` across live instances, as the executable retains a definition pointer.
The index-pool transfer matches 512 executed acquisitions. Parent activation,
other FX families' shared-RNG work and actual camera ownership remain
prerequisites for starting these effects in gameplay.
`environment_effect_runtime` connects the pool, controller, registration and
per-view slots. Its default capacity and free-list layout match the executed
64-slot constructor. A composed 560-operation fixture follows initialization,
updates, publication, retirement dispatch and the manager's final four fog-curve
images/weights through the game's actual routines. Pool exhaustion consumes no
child RNG, and effects retain both their registration order and group update
order. This owner still requires the parent activation and prepared environment
resources described above; it does not start arbitrary parsed effects.
Entity/script controllers remain unconnected; the gameplay producer still
publishes no external override frame. `environment_profiles` prepares inherited
partial profiles against the captured parameter schema and caches their shared
identity. Atmosphere profile images use the same resident resources and aliases
as ordinary world and area images. Preparation preserves absent fields and
propagates image errors; loading a profile does not activate it.

`rendering::environment_parameters` shares prepared scalar, vector, boolean and
image snapshots and area evaluation between atmosphere and ocean inputs.
Numeric external overrides are evaluated after ordinary tracks. Explicit
entries precede weighted environments, preserve duplicates and retain the
ordinary baseline. Environment influence averages the target separately from
the blend weight. `environment_parameter_overrides` retains the boolean
thresholds, integer conversion/rounding and float operand order; 512 executed
manager cases cover all four numeric kinds, including non-finite float/bool
inputs. The prepared float/bool adapter matches 256 of those cases. Its frame
updates keep external results out of ordinary track history. Image overrides
continue through the separate four-image grouping rules.
`ocean::environment` captures exactly the inputs used by `OceanSettings`, keeps
absent area fields distinct from defaults, and resolves image aliases during
level preparation. The water frame publishes these evaluated ordinary ocean
inputs alongside its matrices and clock. Ordered render-view water-contact results
and preparation of the retained ocean view state remain requirements before drawing;
ordinary area containment does not select a local water volume.

`environment_contacts` implements selection from completed, ordered render-view
physics hits. Ordinary environments are inserted by descending squared AABB
diagonal, with later equal-size hits taking precedence. Water environments use
the last hit's maximum Z; active water-trigger contact is tracked independently
and controls ocean-height updates. Invalid or unfinished queries clear the
selection while retaining the unused stored height. The selector's 128-case
fixture executes the original routine, including duplicate hits and NaN sizes.
It does not produce physics hits or replace their order with document order or
camera-area containment. That physics producer remains required for gameplay.

`gameplay::point_query` implements the hybrid broadphase's quantized point
traversal, which the game's physics-world configuration selects. It preserves
upper-bound expansion, branch-distance ordering, right-first ties, collector
distance changes and early termination. A retained stack avoids per-query
allocation after warming up. Its visitor delegates collision filters and exact
shape queries; visiting a body candidate does not itself create a contact.
The 268-case fixture executes the original traversal on synthetic packed trees.
Runtime tree construction, body buffers and shape dispatch still need wiring
before this can supply gameplay environment contacts.

`gameplay::collision_filter` shares the body's five-bit layer and required-flag
predicate with shape-pair filtering. `collision_dispatch` constructs the
closest-point handler table with the original shape inheritance, feature masks,
registration priority and wrapper overrides. Filtered pair selection uses that
same predicate and preserves the flipped-query rule. Numeric execution checks
cover 2,048 filter cases, 3,750 registered entries and 3,750 pair selections.
Handler tags identify the required geometry kernel; an unimplemented kernel
must not be treated as the original empty handler. The kernels remain separate
from this completed routing layer.

`gameplay::convex_support` prepares the geometry inputs used by convex distance
handlers. It borrows unscaled four-component vertices, applies authored scale
and offset to XYZ when required, preserves W bits, and selects the original
radius override and support-kind code. Reusable workspaces keep both query
sides alive independently. Its 224-case fixture executes the original wrapper;
the distance solver and asset-to-runtime shape publication remain separate work.
`physworld::convex_polytope` reads the authored float4 vertices, planes, radius,
face records and optional connectivity through inherited Havok type metadata.
This opt-in reader leaves ordinary movement collision allocations unchanged.
`gameplay::polytope_query` implements the incident-edge distance hint used by the
point/polytope kernel. It preserves the unrefined reciprocal-square-root estimate
and source edge-ring traversal; 2,400 executed synthetic cases match every result
bit, including nonfinite inputs. The hint is a bound, never a contact result.
The finite zero-radius, zero-distance render-view point predicate preserves the
actual plane arithmetic and strict boundary exclusion; 6,086 calls through the
installed point/polytope kernel verify it, including adjacent floats at a face.
Installed audits cover 540,135 level polytopes and 11,338 environment-trigger
polytopes, all with zero radius. The two trigger polytopes without connectivity
explicitly require the general distance kernel. Neither missing
connectivity nor an unsupported query becomes a fabricated contact or miss.
The general distance solver, body transforms and ordered runtime contact owner
still need integration before ocean or underwater selection can consume these
shapes.

`ocean::OceanUpdateState` retains water height, the nine-word FFT control cache,
spectrum invalidation and independent spectrum/surface task flags. Its initial
tile size is 50, water height is zero and maximum wave amplitude is 5. The
world-frame owner resets both task flags before preparing its views; actual
water contacts supply the height-update gate. Water frames also carry the
installed configuration's ocean tile size and skip flag, read during level
preparation without modifying the settings file. `OceanWaves::for_view` allocates
resources without an unnecessary initial spectrum calculation. Its view tasks
reuse the existing FFT, displacement, collision-height and foam producers,
initialize the spectrum when invalidated, and preserve separate height-query
admission. `begin_world_frame` clears both displacement targets without changing
the ping-pong index, gradient, collision or persistent foam images. The other
displacement slot is therefore not guaranteed to contain the previous world
frame. Native callback checks cover disabled and culled views and ordered
render-target clears; connected GPU fixtures cover prepared controls, FFT,
displacement clearing and both foam modes.
These tasks still need the gameplay water/depth frame owner before submission.

HDR conversion white is distinct from the filmic curve's authored white level.
`PostProcessSettings::hdr_white_level` reproduces the shared 3586a0/325910
selection: use `1` unless `r_postProcessExposureUseToneMappingWhiteLevel` is
enabled and tone mapping is not skipped. The display owner supplies this value
to HDR conversion and the exposure window while retaining the authored white
in `ToneCurve`. The 32-case fixture executes the original view-owner branch.
Lens dirt's `UseDefault` view uniform follows equality of the resolved From/To
images, as in 3586a0; it is not an independently authored visibility switch.

`bevy_backend::lens_flare` owns the threshold, ghost/halo generation and
incremental Gaussian H/V passes from the flare producer. `FlareChain` retains
two R11G11B10 float targets, shares its pipelines and keeps a separate uniform
buffer for each queued draw. It preserves the scaled scissor, the threshold's
unscaled viewport, integer parameter clamps and the small-radius blur branch.
Both backends match installed-HLSL packed stores exactly in isolated and
four-pass tests. This producer is not yet submitted by gameplay: its actual
`hrBufferDiv2` source is the clamped colour downsample made before DOF filtering,
and must not be replaced by the luminance pyramid. No fog-shadow pass is added.
`rendering::colour_downscale` provides executable-checked material selection,
source-texel offsets and clear-colour preparation. Its backend raster pass
implements all seven installed colour reduction modes, including clear-pixel
replacement and the authored adaptation maximum. Floating-point and packed
render stores match installed shaders on both graphics backends. `HdrDownscale`
retains the pre-DOF colour target in the main HDR format (packed RGB or RGBA16F).
Its host-source variant reads current GPU adaptation and removes camera radiance
units inside the downsample draw, without a separate full-screen conversion.
The source texel offsets remain tied to the full source image. This adapter
preserves linear compression arithmetic but does not reproduce the original
main-target packed-storage quantization. Checks cover 72 shader frames in
float, half and packed outputs, including 16 host-source cases, live GPU factors
and retained target identity. Gameplay submission still awaits the enabled-flare
owner and its verified inherited threshold offset; it is not used as a
replacement for the luminance pyramid. The caller must provide the pre-DOF
source, actual target extent and full unscaled view; flare temporary/output
targets remain separately packed.
`rendering::depth_of_field` currently supplies material variant selection only.
Its filter, cat-eye and fringing selection matches 315 executable calls; this
does not provide the DOF render passes or autofocus producer.

`bevy_backend::luminance` owns per-view RGBA16F mip chains. Exposure renders the
first two levels once and reads the second level for its histogram; other
post-processing can continue generating later levels from those same images.
Allocation follows the configured downsample quality, minimum dimensions and
the original short-axis mip stop. Shared extent rules also serve fog allocation.
Buffers are replaced on resize and released when their view retires. Installed
shader comparisons cover complete chains, including odd sizes and both quality
settings; render-graph tests also cover exposure, display and view retirement.

Cutout shadow materials create their authored initial image mip chains during
level setup. Both model and static-world shadow program banks determine which
resources are needed, including vegetation and vista cutouts. They use the same
archive reader, image attachment and residency bookkeeping as the streamer;
finer mips still follow projected-area demand. A caster entering a new shadow
range therefore has a resident base image before its first draw, rather than
making the entire directional shadow frame wait for an initial texture read.

### Ambient occlusion

`rendering::ambient_occlusion` transports the per-view AO mode, projection and
HBAO kernel constants. `bevy_backend::ambient_occlusion` owns persistent GPU
resources for the configured producer. HBAO uses the game's SDK 3.0 parameters
and fixed sixteen-slice jitter sequence. Its host adapter packs the independent
coarse slices in a 4x4 R8 atlas, preserving the native sample positions and
intermediate quantization. Source-derived NVIDIA code retains its license in
`vendor/hbao/`. Numerical fixtures contain synthetic inputs and outputs only.

The legacy DWAO producer uses the existing `DepthPyramid` reduction and its
RGBA16F maximum-depth lane. Its reduction is submitted before opaque lighting;
the adaptive-shadow readback owner's later reduction remains separate. Neither
legacy reduction nor its textures are allocated in HBAO mode. Settings come from
`r_hbao` and `r_skipSSAO` in the installed game's configuration.

`screen_visibility` owns packed depth and the two shared filter passes.
`screen_shadow_pass` records AO in red independently of sunlight-atlas readiness,
then records available sunlight visibility in green and filters both channels.
The receiver uses a separate current-AO validity flag, so an atlas reset cannot
remove AO or reuse an old view. Missing AO input retains material AO.
Opaque material shaders combine screen AO with material AO using their minimum
before SH normal relaxation, ambient lighting and reflections. The actual
`isAlpha` material specialization bypasses AO. Direct lighting and emissive
colour retain their independent inputs. HBAO receives view depth directly, but its following NoAO draw resolves the
registered `SSAO/Zbuffer` image initializer. The legacy and skipped-AO callbacks
explicitly publish view depth instead. Depth packing preserves that distinction;
indexed loads outside the initializer return zero as in D3D11. The shared filter
therefore receives the same packed depth as the game, including its HBAO path.
The current full-view scale is one; dynamic-resolution production and sub-view
rendering remain separate work.

### Direct surface lighting

`dh2_direct.wgsl` implements the standard Ward specular, accurate Fresnel,
Oren–Nayar diffuse and sheen kernels from installed declarations. The surface
shader shares one gloss/metal mapping with indirect lighting and prepares its
normal, view, IOR and sheen context once per fragment. Bit 7 of the existing
material specialization selects this path in the host point, half-point, spot
and directional loops. Shadow visibility retains its existing owner.

The current integration covers default/decal/filling material variants without
sheen maps, diffuse lookups, fuzz or translucency. Those feature variants,
vegetation transmittance and area-light solid-angle integration still require
their own complete producers; their existing host paths remain in use.

The authored `m_specularAttn` survives inherited entity parameters and maps to
host light components independently of shadow readiness. The typed clustered
light record carries it in an additional vec4, with a 96-byte stride that still
fits the existing 170-light uniform-buffer limit. The directional scalar uses
existing end padding. Both CPU and WGSL layouts declare the field explicitly.

The ignored GPU comparison test uses 512 synthetic cases executed through the
shipped shader bodies, including zero and fractional specular attenuation.
The material pipeline test covers the connected standard specialization on both
Vulkan and DirectX 12. Matched runtime captures compare physical HDR and bloom
inputs; stationary measurements do not establish camera-motion stability.

Camera settings and combine material selection live in `camera_effects`.
The app prepares registered defaults and world/area transitions from archives,
then publishes typed settings to the display frame and F8 reports. Startup
console overrides retain integer kernels without converting through floats.
`r_skipDOF` is preserved: the original bypasses sharpening and vignetting along
with depth-of-field when this switch is set.

The backend camera-combine draw implements sharpening, distortion, vignetting,
optional noise and near/far composition. It reuses the shared D3D11 point
sampler rules. Numerical tests cover the actual recorded frame and 64 synthetic
frames on Vulkan and DirectX 12; the noise branch is not covered by these tests.
The gameplay draw remains disconnected pending its actual blur/refraction
images and autofocus depth/history inputs. The backend `AutofocusPass`
implements the single-group focus update, verified against 128 shader states;
its view owner must supply the real water-aware depth and focus history.
The separate depth-of-field module currently implements filter-material
selection. Full bloom appearance and camera-motion parity are not established
by these tests.
