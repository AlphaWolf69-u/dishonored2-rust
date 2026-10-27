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
`app/machinery.rs` advances that owner from the two physics timespaces (clock
groups 3 and 4, exactly the two entries supplied by `8af460`) and queues rigid pose
deltas before transform propagation, visibility and shadow collection. Meshes
and per-body solver storage are reused. Missing entity endpoints retain the
world anchor, following the constraint body's initialization rule.
`gameplay/deactivation.rs` is the hknp per-motion stillness check; the world
groups constraint-linked candidates into deactivated islands, honours
`m_startAsleep`, and wakes bodies through motors and wind. `gameplay/entity_lod.rs`
computes owner LOD levels from the view and last-frame visibility; at level 6+
constraints freeze their bodies (activation mode 2) until the level drops.
Physics systems (`assets/physworld/physics_system.rs`) load an MD6's init
`ragdoll` hknpPhysicsSystemData and its hashed `.ragdoll` body-to-joint map;
their ragdoll constraints and arkCpntConstraint cones are atom stacks
(`constraint_solver/atom_stack.rs`, bit-exact to 140c45400) solved with the
machinery. All constraints solve in creation order (1412a3480 + stable radix
sort by island), interleaving cones and hinges. System bodies start active
(the component is unpaused at spawn) and carry their `m_layer` filter info;
`gameplay/collision_filter.rs` ports the layer masks and arkCollisionFilter
pair rule, and only systems on a layer that collides with nothing (HIDDEN,
every placed system in the game) are simulated, since machinery bodies have
no contacts. `app/physics_skinning.rs` poses those MD6 placements from the
bodies each frame (per-placement meshes and shadow palettes).
Entities with `bindInfo` follow their bind parent (140752900 attach,
1407506d0 per frame): `app/model_motion.rs` gives bound children, transitively,
the world delta of their nearest moved ancestor (machinery bodies, scene
motion). Unrendered FX children have no consumer yet. Pause UI, jump, lean,
and player Midnight SFX tracks use the installed language-neutral Wwise
banks and Bevy audio. Midnight cues retain their body/arm owner, authored
key times and interruption-stop flag. Barks and timed-event tracks remain
unsupported. Wwise node and bus properties retain the authored mix and
volume RTPC curves; global settings come from the initialization bank.
Random/sequence selection retains each container's repetition limit and
global or per-emitter scope. A bound dynamic rigid body
also joins its parent's machinery motion (14080ac90 -> attachBodies, then the
c90a90 mass rebuild), so e.g. a speaker's 10 kg hangs on its turnbuckle.

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

Winit's Windows fullscreen style retains `WS_POPUP` after removing overlapped
decorations. DXGI rejects a topmost fullscreen HWND with neither a popup style
nor any overlapped styles. Display settings resolve the requested monitor to its
Bevy entity after monitor registration, rather than selecting an unavailable
monitor during initial window creation. Both fixes preserve the requested
resolution, refresh rate and presentation policy.

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


## Player locomotion

`gameplay/player/` ports the first-person player in the game frame's stage
order (FUN_1408ad260): input dispatch on simulated frames only, QUERY_PHYSICS
ground/ceiling queries, DEFAULT controller → state machine → camera pitch and
speed FOV, PRE_PHYSICS physics step, PRE_RENDER view. The step uses the
player clock group's delta (whole 1/120 s ticks); mouse look scales by clock
group 0.

- `controller.rs` (arkCpntPlayerController): bound action events from the
  IN_GAME binding set become held/pressed/released/toggled masks, the
  normalised keyboard move and mouse look (`g_mouseClamp`,
  `g_mouseSensitivity`, `g_fixMouseSensibility`, `g_invertMouseYAxis`), the
  support-state Falling/landing requests and the decl `m_statesMapping`.
- `states.rs` (arkCpntPlayerStateMachine): command processing with the
  body-part rule table and the mode's `m_rules`; Running, Sneaking (crouch
  toggle), Sprinting (`g_sprintToggle`), Jumping (vertical move to the apex
  with `m_maximumVelocity`/`m_steepness`) and Falling (InAir steering, air
  friction, extended ground probe, landing). Velocity requests follow the
  movement object's one-frame, newest-first lists.
- `physics.rs` (arkPhysicsPlayer FUN_1401b4760 and arkCpntPlayerPhysics
  queries): posture shapes, linear and yaw limiters, gravity strength ×
  (0, 0, −9.81), downhill projection, vertical move, speed scale.
- `character_proxy.rs` / `simplex_solver.rs` (hknpCharacterProxy integrate
  FUN_140085a40, checkSupport FUN_140c82a50, hkSimplexSolver FUN_140b82be0):
  manifold, surface constraints with max-slope walls and penetration
  recovery, cast iterations. Steps are climbed by the hull's foot cone; the
  proxy has no step-up code.
- `shape_cast.rs` answers the hknp castShape/closest-point contract on the
  level triangles: GJK distance, conservative-advancement casts and EPA
  penetration depth for overlaps. Havok's own kernels and mesh normal
  welding are not ported.
- States also cover Sliding (distance-based 7 → 2.5 m/s curve, uphill
  shortening, side steering, blend-out, resume from short falls), Slipping
  (controller slope thresholds every frame, slip-fall) and AutoCrouch with
  its sensor (`sensors.rs`). The WallCrouch sensor starts disabled and no
  enabling caller was found, so WALLCROUCH stays unreachable as in the game.
- `camera.rs` / `body.rs` / `animation.rs`: the view is `camera_jnt` of the
  body pose, pitched through the spine03/neck/head split and kept inside the
  capsule. The body anim tree subset runs the loco nodes for the body and
  the camera bob (speed sets, direction pairs, one shared progress, stance
  manager with transition clips, phase sync, `ark_playerBobAmount`), folded
  with the Edge lerp/additive blends; inputs come from the body DEFAULT
  stage (start delay, max-speed clamp, `ark_playerSmoothedLoco` smoothing).
  Clips are read by `assets/edge_animation.rs`; an alias resolves to the
  last md6 animset defining it (Corvo's clips over the base/Emily ones).
- Leaning (state 18) runs beside the navigation state: LEAN/LEAN_LEFT/
  LEAN_RIGHT requests, `g_leanToggle` hold or toggle mode, latched input,
  recentering, the 1.5 m/s and 60° exits and the 0.3 s fade. The body scrubs
  `ply_body_common_lean_leftright`/`_up` as root additives, offsets the
  capsule clamp base (pp+0x110), and the lean sensor's two head-box casts
  from the gameplay eye limit the lateral lean. The very-hard
  `m_limitedLean` halves both ranges.
- The body's jump stage (FUN_140774f10 / FUN_140775270 / FUN_140776a40)
  plays `ply_camera_generic_jump_in` / `_loop` / `_out` through the animBasic
  node (up to four linearly fading clips, play-once/stay/loop modes, queued
  play-on-end, the identity leaf under a partial weight) for jumps, walk-off
  falls (`min(groundRayDistanceMax / m_fullFallAnimHeight, 1)`) and small
  falls (`m_smallFallAnimWeight`, `ark_playerSmallFallCooldown`, armed on
  every navigation activate). It also zeroes the body speed while non-zero.
- `assets/entities/player.rs` resolves the character's body def components,
  startup mode, states, locomotion decl and md6 animsets.

Measured in the running game (sample_player_motion.py, tlf_view capture):
run 4.14 m/s unequipped, jump apex 1.500 m with the 5.1 m/s / 0.74 vertical
move, fall acceleration 29.43 m/s², eye 1.678 m standing and 1.010 m sneaking
at 11.5° pitch, matching the port within the camera idle bob (a few mm).
A full lean puts the eye 0.740 m to the side, the decoded `camera_jnt` value.

- `midnight.rs` plays midnight scenes on the player body: integer-ms scene
  time with a carry, the CONDMD6 choice by body md6, the clip schedule and
  Bezier weight curves, BREAK_PLAYER_STATE events flushed before the
  states, the clips folded over the loco pose (BLEND, or ADD for additive
  clips) with the interrupt snapshot fade, and the origin delta pushed as
  priority-1 root-motion velocity (`assets/entities/midnight.rs` parses the
  decls). Only the PlayerBody/PlayerCamera groups play: arms, weapons and
  the mantle dummy have no trees.
- Catfall (falls above `m_catFallHeight`, light or strong by fall damage
  against the full player health), Mantle (`MantleSensor`'s ledge search,
  the scene, and the dummy's arkCpntRetargeting as priority-3 velocities that
  warp the clip's anchor_jnt onto the ledge) and Vault (`VaultSensor`'s
  slope-aligned obstacle casts, a constant run-up velocity over the raised
  VAULTING hull, the scene for the view) are states with their camera
  modes.
- `camera.rs` runs the deferred target modes, the cosine mode blend, the
  free and constrained look integrators and the joint controls of AND_ARMS,
  ANIMATED, CONSTRAINED, CONSTRAINED_WITH_ARMS and CATFALL; leaving a
  yaw-offset mode hands the yaw to the body. The body's FollowSlope turns
  the skeleton to the ground normal while sliding or catfalling.

Not ported: hit-reaction camera additives, footsteps, hand
equipment (the player moves unequipped), health, damage and death (falls
never kill), arm and weapon midnight groups, breakable and NPC vault damage,
window vaults, blink/grapple mantles, moving platforms, HUD labels, swimming,
powers, impulses and moving or dynamic bodies.

Open questions and blockers (evidence in the local `artifacts/locomotion/`
research notes):

1. Midnight root motion and retargeting are traced statically. Next: log the
   entity origin per frame during `cat_fall_strong` (2.0 m roll) and a high
   mantle in the running game and compare with the probe (`DH2_ANIM_PROBE`).
   During a high mantle the sneaking hull pins against the wall face until
   it clears the edge; whether the original shows the same lag is open.
2. The player's non-midnight velocity term `(1 - w) * loco` in
   arkCpntAnimEvolved (FUN_14054f500) is taken as absent for the player.
3. The vault scene choice uses the game's LCG (gameLocal+0x600f8) whose seed
   is not traced; the port starts it at 0.
4. The animBasic commands are taken to reach the tree update of the same
   frame (the body DEFAULT stage before FUN_140540f20). Next: breakpoints on
   FUN_14053eef0 and FUN_1406e3de0 during a jump.
5. Lean: the post-step `SetPosition(owner + pp+0x110)` (FUN_1407ffab0 →
   FUN_1401b30c0 → FUN_140085160 command 6) moves either the hitbox or the
   proxy. Next: check the body id it uses (pp+0x210 or the proxy body).
6. CONDMD6's base-compatibility fallback (FUN_1401c8cc0) and midnight
   markers are not decoded; no installed player scene needs them.

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
`gameplay/particles/` owns particle parameter sampling, cycle timing, coordinate
spaces and stable simulation slots. `particles/emitter.rs` implements continuous
and burst emission with independent spawn/cycle seeds, rate-derived time slices
and explicit shape/field bindings. `particles/pose.rs` retains emitter position
and orientation history, using the shared reference-frame quaternion conversion
and normalized interpolation. `assets/fx/particles.rs` reads the movement
portion of particle stage declarations with shared FX curve readers. These CPU
pieces feed `particles/simulation.rs`, which owns stage slots and advances
loaded declarations. `assets/fx/particle_system.rs` reads the parent FX
descriptor and `gameplay/effects/particle.rs` implements its phase, stop, loop
and camera/owner placement rules. Player screen-effect sources retain these
descriptors alongside their quads, with independent readiness errors.
`bevy_backend/screen_effects/state.rs` owns the player transition lifecycle;
`bevy_backend/particle_system.rs` owns reusable particle instances, simulation,
and same-frame sprite/material publication. Its inputs separate the owning
entity frame from the camera. Enter/leave-water membership stays in the screen
owner, outside the shared particle system. `bevy_backend/particle_pass.rs` submits their unlit colour draws with
shared material fog, depth, water clipping and the gameplay stencil attachment.
`rendering/particles/transparency.rs` resolves the console-controlled direct
or weighted accumulation mode; `particle_pass/transparency.rs` owns retained
accumulation/revealage targets and resolve. This connection covers the loaded
player underwater dirt/bubble declarations; it does not supply every world FX
producer or animated/lit particle material path. Runtime and human appearance
acceptance remain separate from compilation.

Particle water clipping consumes an immutable publication of the canonical
physics world through `world_point_query/snapshot.rs`. It preserves actual
body/tree order, shared geometry, packed body-top heights and both stored
quantization directions. Closest-point selection precedes the authored downward
ray; only a completed miss permits fallback to the active ocean height.
`world_ray_query.rs` dispatches filtered rigid bodies through the shared hybrid
world tree; `shape_ray_query.rs` handles convex plane clipping and compound
child traversal/transforms. Query failures propagate instead of becoming dry
water results. The snapshot owns no second physics simulation.

Serialized event nodes and links live in `assets/entities/event_graph.rs`;
motion scenes and world FX share this parser. `gameplay/event_graph.rs` supplies
the common variable/entity input readers used by motion playback.
`assets/entities/effects.rs` resolves the world FX node schema and preserves
the surrounding graph instead of treating every FX reference as active.
`gameplay/effects/graph.rs` executes understood FX/gate actions against a world
owner. Gate condition outputs precede the matching regular output and `Out`,
following the installed dispatcher. The graph owner supplies random samples,
named-variable bindings, actual entity clocks and effect allocation; missing
bindings remain explicit errors. `effects/world_node.rs` handles instance-ID
observers, start/stop ordering, timed expiry and saved remaining duration.
`world_particle_state.rs` connects these graphs to deferred component creation,
FX instance ownership, group startup, completion observers and the common
particle renderer. Ordinary events append in authored link order and execute
newest-first, matching the installed event queue.

Level startup parses world FX graphs into `WorldEffectGraphs` and prepares
their installed declarations through `particles::world_sources`, sharing
particle definitions and material storage across references. Preparation does
not activate effects. `particle/dist_blur` is a separate vector-writing
program using the same sprite geometry, clip planes, soft contact and Fresnel
helpers as colour particles. It supports authored 2D and flipbook normal maps,
constant-offset reconstruction and separate blur/distortion strengths. Its
vectors accumulate in a depth-tested target before the shared motion-distortion
resolve; they do not enter colour or weighted-transparency accumulation. Group location, timing, other effect families and local
descriptor failures remain available to the playback owner. The renderer
receives shared declaration resources through ordinary resource extraction.
FX graph entity inputs retain typed runtime identities: map names and the active
player are distinct references. `arkKisculeNodePlayer` resolves the actual player
owner, matching `7a8500`'s invalid-handle fallback to player slot zero. Player FX
attachments resolve the installed model's tags first, then bare joint names
(`4b19b0`). Their offsets and orientations compose with the same controlled body
pose and entity transform used by attached arms (`53dac0/53d3f0`); render-camera
collision adjustments do not replace that skeleton pose. The publication includes
all loaded player tags and joints, independently of leg visibility. Ordinary map
entities still require their animated tag publisher before tagged FX can attach.

`WorldFxComponent` owns the source's deferred creation and removal state;
actual allocation, target pose, velocity and completion callbacks remain host
operations. `WorldEffectFrames` reuses the common entity spawn-frame reader
and moving-model pose publication, including non-rendered bound targets.
Unresolved tags and parents stay explicit errors. Main-world publication retains
the actual group3 double time, group2 delta and group3 scale: the FX manager
advances age by delta times scale rather than Bevy elapsed time.

`ParticleSystems` owns the shared 1,024-slot particle-family pool for player and
world instances, its random stream and canonical world query executors. Parents
retain their linked-list heads; declaration selection precedes allocation and
active chains update newest-first. Inactive systems retire before their next
update. Both domains submit through the same `ParticlePass` and transparency
targets. Prepared level graphs and authored entity frames share immutable
storage across extraction; only actual moved-frame overrides are copied.
Emission meshes resolve directly from installed `staticparticlemodel` PMODEL
resources, retaining their ordered 32-byte vertices. `particles::mesh` implements
the seven authored sampling modes, per-emitter sequential cursors, vertex colour
and normal velocity. Geometry selection consumes the spawn generator while
scale/rotation/normal-velocity parameters use a copy of the retained cycle seed.
These samples join ordinary location/velocity before initial orientation and
spray; player and world particles share this same simulation path.
`app::particle_view` publishes the current ordinary Tome descriptor after world
visibility and water-view publication. Extraction shares that immutable frame;
the renderer builds its AABB query and angular-size threshold once per view.
`particles::visibility` schedules draws, full simulation, clock-only updates and
flush-on-occlusion from authored visibility flags and distance curves. Finite
non-looping stages and dark-vision materials derive the declaration overrides;
completed fade-out forces simulation until the owner can retire. Culled systems
discard unused delta, and admitted updates retain the game's 125 ms clamp.
Clock-only updates still commit the emitter pose. Flushing returns live slots in
ascending order to the existing free list without resetting its history.
The same frame publishes immutable snapshots of the existing world and trigger
wind owners. Particle wind queries use layer16 and canonical collision geometry;
the physics snapshot retains trigger-to-wind component identities. Selection
replaces only a strictly lower priority, preserving traversal order on ties,
then falls back to the registered world wind on a completed absence. Wind is
acceleration, with independent cycle and lifetime curves. It stays separate
from inherited source velocity and consumes the original particle RNG sequence.
Local stages transform wind with the emitter-axis dot products. Spawn-time
inherited velocity retains the source's linear/angular point-sampling kernel;
nondefault inheritance still requires its actual producer before admission.
The connected world path currently admits particle-only groups without an
unavailable target/tag, spawn-occlusion or physics-velocity dependency. Other
enabled FX families, parameter blocks, animated/lit particle material inputs and
unsupported graph producers remain explicit errors. These are outstanding
feature coverage, not replaced with another effect. Runtime and human appearance
validation are still required; compilation alone does not establish parity.
Sprite spawn/lifetime inputs live in `gameplay/particles/sprite.rs`, with their
partial-edit readers in `assets/fx/particle_sprite.rs`. Initial size retains all
three components, independent pivot/rotation parameters and spawn-time RGBA8
precision. The shared colour packer is `colour.rs`. Sprite orientation covers
all seven emitter/camera/velocity/custom-axis modes in `particles/orientation.rs`.
Authored sprite LOD entry allocation defaults still need their reflected
constructors; the reader reports this explicitly instead of inventing values.
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
and `artifacts/sky-atmosphere.decl.txt`. General world FX producer integration,
additional particle soft-contact inputs, vertex-lit particle producers,
animated-emissive mask production and lens-flare postprocessing remain separate
coverage. A mesh liquid that
requests an unavailable producer is rejected explicitly rather than passed
through ordinary white material shading. Player particle instances now have
clock and draw publishers; other producers and postFX dependencies must use
their actual game inputs. Adding debug light markers or synthetic bloom would
not provide their authored appearance.

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

Placed BML/MD6 models retain their accepted bounds-query area and submit it
for every resolved surface material image, including custom material overrides.
Visibility and demand share the same query and the current motion-updated bounds;
the ordinary archive streamer owns mip eligibility, uploads and eviction.

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
The tree owner is supplied by `gameplay::broadphase`. Runtime body buffers,
shape bounds and exact shape dispatch still need wiring before this supplies
gameplay environment contacts.

`point_query::insertion` implements insertion-leaf selection and branch-bound
refresh. Its fixture compares 1,024 selections and 64 complete tree refreshes
with executed game routines, including unsigned distance wrap, strict limits,
tie order and unchanged leaf metadata. The broadphase owner uses these helpers
for slot allocation, leaf-block insertion and splitting.
Leaf bodies use the standard library's stable sort by lower X endpoint,
equivalent to the game's insertion sort and checked against 128 executions
with ties and complete body IDs.

`broadphase::HybridTree` retains nodes, parents, branch counts, free-slot stacks
and leaf blocks. Single and batch insertion preserve the factory's 29-entry
limit, node growth and block reuse. Splitting retains midpoint partitioning,
deferred leaf publication and the rebuilt-leaf quicksort's exact equal-key
swaps. A 53-scenario fixture compares complete graphs, body locations and free
lists with executed insertion and rebuild routines, including 2,048-body cases.

`HybridBroadphase` owns the selected default five-layer configuration and
classifies actual runtime flags and status. Its layer settings and classifier
match 4,096 executions. Six complete manager comparisons cover empty, single,
bulk, repeated and chunked additions, including each layer's dirty state. These
are insertion operations; removal, moving-body updates, collision-pair
generation and lifecycle signals remain with the physics world.

`BroadphaseQuantization` derives both the point-query grid and packed coordinate
layout from world bounds. It retains the two Y/Z packing layouts, byte shuffles,
clamps and zero-extent controls. All 69 defined output words match 512 executions
of the setup routine, including degenerate and nonfinite input cases. Producing
shape bounds and applying the packed conversion to runtime bodies remain the
next connection to this owner.

`PackedQuantization::pack_bounds` now implements static-body padding, clamping,
integer conversion and packed coordinates. `query_bounds` decodes the tree's
coarse bounds. A 1,024-case fixture executes the game's static preparation and
its actual tree insertion, comparing both representations. The signed integer
conversion is shared through `float`; rendering retains its existing wrapper.

`PackedQuantization::unpack_bounds` separately decodes the body's world-space
bounds, including the source integer padding and per-axis quantization rules.
These are the bounds read by the water contact path, rather than the raw
authored shape box. All four minimum/maximum lanes match 1,536 executions of
the game's decoder, including degenerate world configurations and empty boxes.

Compound assets retain their authored local AABB and common `ShapeHeader`.
`shape_transform::transform_bounds` implements the compound bounds virtual
operation, including unchanged empty boxes and all four output lanes, verified
against 2,048 executions. The installed trigger audit checks that every one of
5,576 authored bounds encloses its compound tree root.

Serialized shape key widths and instance material tags use the file's inherited
field schema. Their offsets differ from expanded runtime objects; the readers
no longer use runtime offsets for these fields. The installed compound audit
covers 152,215 shapes and 1,488,370 reachable instances across all 36 worlds,
including the invariant that every instance fits its compound key width.

Placed static-body acquisition (`app/view_geometry.rs`) loads the primary
HKX using its declared shape kind. When `m_materialForExtraHDcollision` is set,
it also acquires the scaled `_hd` HKT resource, independently of the primary
kind, following 140808ea0. The two children share one body transform and entity
owner. The HD child's material fills every HD slot; 140807c80 supplies the
primary override. The flattened navigation triangles retain separate material
indices and material-filtered collision words, so simple movement geometry
cannot obscure the detailed weapon/visibility geometry. Geometry and material
declarations are cached during acquisition, including aliases and inheritance.
Missing optional HD resources retain the primary and report acquisition failures.
This startup index covers static bodies; dynamic/keyframed simulation geometry
belongs to its moving-body owner and is not frozen into this index.

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

`gameplay::collision_queries` owns the 4,096-slot generation pool, submission
FIFO, cancellation, completion and result transfer. Its exclusive owner
linearizes the game's worker operations; it does not introduce a second thread
pool. Pending cancellation keeps a slot occupied until the worker drains it.
The all-hit collector preserves traversal order and duplicates. A synthetic
fixture checks 12,222 transitions through the installed query manager.

`gameplay::compound_query` traverses the compound shape's float BVH with a
retained stack, left-first equal-distance ordering and the optional node mask.
This mask indexes tree nodes across 32-bit words, independently of instance
enabled flags and nested shape masks. Executed fixtures cover 640 unmasked and
2,560 masked queries, including nonfinite distances and deferred nodes after
collector-distance changes. The instance visitor still owns tags, collision
filters, transforms and exact geometry; traversal alone never publishes a hit.

The opt-in `physworld::compound_shape` reader supplies the authored tree and
instance records directly to this traversal without duplicating movement data.
The installed-level audit covers 152,215 compounds and 1,488,370 instances across
36 levels. Only reachable nodes participate in topology validation; free-list
storage remains intact.

Compound query preparation shares the verified `shape_transform` arithmetic and
`convex_support` bounds/radius rules. Executed comparisons cover 1,024 complete
query-preparation cases and 1,536 admitted child-instance preparations, including
key prefixes, parent scaling and translation before inverse rotation. These
owners prepare real dispatch inputs; tag/filter callbacks, geometry dispatch and
world body publication still need connection before they produce view contacts.

The convex support module also preserves support-vertex tie ordering and
post-solver distance publication. Its 1,024 support-pair and 1,536 publication
cases execute the corresponding installed routines. Publication tests control
the solver-result boundary. The cold point-distance path now runs the game's
point, segment, triangle and tetrahedron reductions and its penetration path;
4,096 complete executed point queries cover interior, boundary and exterior
positions, rotated targets and vertex counts1..64. Another 1,024 publication
comparisons cover radii and strict distance thresholds. General convex/convex
distance queries still require their distinct feature branches.

Penetration support packets retain the target vertex's index bits separately
from its transformed coordinates. Face and horizon owners preserve traversal
order, opposite-edge links and retired-face reuse; hull expansion keeps the
game's 90-face bound and capacity rollback. Its comparisons cover single
updates and repeated growth through that limit. The seed state machine handles
point, segment, triangle and tetrahedron inputs, including degenerate contacts.
The penetration search composes these owners with face selection, witness
publication and the game's deterministic retry rotations. Comparisons execute
3,739 complete installed penetration searches, including rotated convex pairs
and retry exhaustion. The penetration feature wrapper pads input simplexes,
compacts repeated vertex IDs and retains the game's directional fallback after
retry exhaustion. Another 2,176 executed wrapper cases cover its returned
features, contacts and normals. Point distance and radius-adjusted publication
are connected through `distance::query_point`. Runtime body publication remains
unconnected; these helpers alone do not produce gameplay water contacts.

`gameplay::collision_hit` transforms accepted local closest points into world
coordinates and publishes their body, material, filter, key and user-data
fields. Its 1,024 executed comparisons cover missing body IDs and rejected
geometry as well as successful publication. The typed packet retains defined
fields without reproducing unspecified padding from the source's112-byte
record. Dispatch selection and ordered collection remain with the query owner.

`closest_point_query::prepare_transforms` reproduces the query-to-target frame
and collector-distance clipping before dispatch. Its 1,024 comparisons execute
the installed preparation routine, including nonfinite distance inputs.

`assets::physworld::body` reads inherited body creation fields from the installed
schema. The audit covers all530 bodies in36 physics worlds, preserving item
references, filters, transforms and attachment indices. These serialized
positions and reserved IDs do not establish runtime body identity. The trigger
body recipe separately applies `arkCpntTrigger` collision settings, water/wind
component rules, material, user-data handle and keyframed motion;256 executed
component cases verify its complete defined body parameters.

`assets::entities::triggers` resolves map component declarations, direct inherited
declarations and inline overrides, preserving independent water/wind component
presence. Its installed audit covers 5,576 triggers across all 36 worlds,
including every placed environment volume. `trigger_shape_request` reproduces
the factory's case-sensitive `.lwo` selection, generated map/entity paths and
choice of entity versus identity rotation. Another 256 executions of the
installed factory verify resource names and the resulting body parameters.
Missing entity positions use the constructor's zero position; missing basis
columns retain its identity basis. Trigger shape scale remains one.

`TriggerAssets` owns the resolved HKX bytes and schema, sharing resource files
between placements while keeping every file's item references local. It retains
compound trees, instance transforms and shape tags for query dispatch. Resource
indices are not runtime body IDs. Runtime allocation, broadphase insertion and
entity ownership remain with the world owner; these asset readers alone do not
establish gameplay water contacts.

`TriggerShapeAsset::query_shapes` retains decoded compound trees and convex
polytopes once per resource. Child references remain file-local item indices;
enabled instances retain their transforms and tags, and polytope connectivity
is preserved. Unsupported shape classes return an explicit error. This is
query input storage, not a replacement for world admission or hit collection.

`rendering::environment::load_environment_volumes` retains resolved ordinary
and water profiles together with their authored placement. The existing
ordinary-area loader uses the same profile resolver and still excludes water
volumes. The installed audit covers all 36 worlds: 2,812 ordinary profiles and
four water profiles, each with an actual trigger owner. Classification does
not establish containment, activation or a water surface height.

`gameplay::collision_body` owns the runtime query-body record and its selected
shape handles, motion-kind flags, cached frame and physics LOD flags. Its 512
executed comparisons preserve source W lanes, status bits and multi-slot shapes.
`body_storage` owns the permanent static anchor, generation-tagged reservations
and registered property-bit clearing. Three constructors and 80 reservations
cover generation wrap, record clearing and retained auxiliary arrays.
The per-body change-index array maps into owned pending change records.
Addition queues preserve generation-tagged IDs, duplicate requests and the
last recorded queue position; static bounds changes set the shared dirty bit.
Another 512 executed operations cover immediate static and deferred additions.
`deactivation::queue_attached_bodies` follows the existing attachment ring into
the pending deactivation queue, matching 512 complete game helper executions.
Queue consumption, world activation and actor callbacks remain separate work.
Another 1,024 executed comparisons cover motion-to-body rotation and center,
the COM-centered bounding radius and speed LOD updates. `body_storage::link_motion`
owns the circular attachment list and raw zero-inverse-mass classification;
40 original attachment batches cover 248 bodies, including keyframed states.

Body creation uses resolved asset references rather than serialized indices as
pointers. The manager owns copied name bytes, the shared optional mass/motion
property array and mass-distribution overrides. The creation method connects
these to the runtime record in the game's call order. The numeric checks cover
768 sidecar operations and 256 complete initialization calls, including null
names, exceptional mass values and retained/replaced distributions. Distribution
fields follow the installed `hknpMassDistribution` schema.

The manager's `mass_frame` resolves its channel-one shape and retained sidecars
before the shared mass selector. Another 1,024 complete executed manager calls
cover slot selection, missing overrides, distributions and generation masking.
Principal mass data can also produce the full rotated inertia tensor, preserving
all twelve tensor lanes; 1,024 executed conversions cover that boundary.

Mass tensors support the shipped rotation and signed parallel-axis operations.
Compound combination retains input order, four center lanes and twelve tensor
lanes; invalid total mass preserves the supplied output. Its 256 executed batch
comparisons include empty inputs and groups of up to 64 elements. Principal-frame
conversion uses the original bounded Jacobi iteration, eigenvalue clamp and
normalized cross-product axis reconstruction, with no eigenvalue sorting or
replacement axes. Another 1,024 comparisons cover iteration limits, diagonal,
degenerate and asymmetric inputs through the complete conversion. Stored NaN
words require the shared ordered arithmetic helpers and x86 division.

`assets::physworld::properties` retains schema-resolved property entries and
their last-matching-key precedence. The mass selector preserves the original's
cached, calculated and authored-distribution branches, including unordered
overrides. The installed body-shape audit covers all 530 records in 36 worlds;
those serialized roots contain no embedded property entries. Numerical fixtures
separately cover 768 property searches and 768 complete mass selections.

`motion_info` connects creation data to the actual mass frame and reuses
`rigid_body::BodyPose` for world composition. `motion` packs inverse mass and
limits and transforms angular velocity to the principal frame. `space_cells`
preserves the splitter's four-lane assignment, tie rules and pair indices.
Executed comparisons cover 512 creation records, 512 runtime motions, 1,024 cell
queries and all 1,089 pair-table entries. Existing machinery pose comparisons
still pass after sharing its composition helper.
Motion can read and restore world angular velocity across a principal-frame
change without discarding W. The pose comparison aligns equivalent quaternion
signs and tests center XYZ separately. Each has 1,024 executed comparison cases.
`motion_mass` connects those operations to the attached-body ring, mass resource
resolution and shape-bound dispatch. Single bodies retain their principal frame;
multiple bodies combine full tensors before conversion. The world angular
velocity and cached body frames follow the original pose-change decision.
Its 256 complete executed comparisons cover shuffled rings of up to 17 bodies,
fixed bodies, equal/sign-equivalent poses and changed frames. Resource ownership
and world lifecycle events remain with the query-world integration.
Executable reflection identifies the motion velocity cage, integration factor,
space-splitter weight and previous-step velocities explicitly. The cage is three
bfloat16 lanes beside the separate integration-factor half-word, not solver
links. Body LOD flags and preset quality IDs remain distinct fields.

`collision_body` also produces moving-body bounds from the world's solver-step
inputs. `motion_expansion` preserves the unrefined speed estimates, packed
velocity cage and angular padding. `predicted_frame` composes the stepped motion
with its attached body frame, reusing the verified quaternion normalization and
basis conversion. `swept_bounds` selects ordinary, spherical-displacement or
predicted-rotation expansion from the requested physics-quality flags, updates
the contact-distance cache, and marks changed packed bounds. Shape bounds come
from the caller's actual channel-three shape dispatch. Three independent sets
of 1,024 executed comparisons cover these routines; the complete bounds set
also checks clamping and repeated unchanged updates. These primitives are not
yet a connected simulation world: activation, lifecycle signals, filtering and
ordered water-contact publication still need their owners.

`body_attachment` connects the circular motion list, keyframed flags, shape-based
motion limits, local motion frames, swept bounds, radius cache, TIM distance and
physics LOD in the game's order. Its required world interface owns actual shape
dispatch and deactivation registration; it supplies no substitute implementations.
An additional 1,024 complete executed attachment cases cover all 32 presets and
eight quality configurations, active bounds, limit changes and world call order.
`deactivation` initializes the meaningful per-motion reference fields and appends
body IDs to assigned islands without sorting or deduplication. Its tests compare
1,024 partial-record initializations and 512 cumulative island registrations.
These operations use the same `MotionDeactivation` record as the per-motion
stillness checks. Island lifecycle and the complete world owner remain open.

`signal` owns the linked observer slots needed by world and mutable-shape
notifications. Subscriptions prepend without deduplicating; removal matches the
first subscriber/callback pair and defers destruction during emission. Traversal
captures the next slot before invoking a callback, preserving subscriptions and
removals performed by that callback. Stable handles replace process pointers,
and removed slots are reused. A 512-operation comparison executes the shipped
subscription/removal routines and the world body's actual observer traversal,
checking ordering, flags and callback edits. Payload dispatch remains the
responsibility of the query-world/resource owner.

`shape_registry` groups body IDs by mutable root shape and owns the two observer
subscriptions for each registration. Stable world and entry handles preserve
sharing between worlds. Mutation flags accumulate, destruction detaches both
subscriptions, and explicit world release removes callbacks before the shape
resources are released. Removing a body's first matching ID swaps the last ID
into its slot; an empty registration detaches and returns to the recycle pool.
Executable comparisons cover 534 lifecycle operations and a further 540
operations including removal, duplicate IDs, pool reuse, two worlds sharing
shapes, destruction and post-release notifications. Pending-change processing
remains unimplemented. The runtime world must
connect these operations and resolve each shape's actual mutation-signal support;
the registry alone does not activate a physics or water-query world.
Retained compound assets read `isMutable` through their file schema; both
compound runtime classes use that byte to expose or withhold the signal pair.
The installed-resource audit finds zero mutable shapes among 152,215 compounds
and 5,576 trigger shapes across 36 levels. Trigger resources retain that flag
without loading duplicate query geometry.

`shape_tags` decodes game collision tags into the existing hit packet. Invalid
tags or a missing body leave the packet unchanged; direct material tags preserve
the collision filter, while mapped tags index the body's map relative to its
current material and apply the resolved physics-material flags. Required source
methods expose both lookup boundaries without fallback values. Comparisons with
1,024 full decoder executions and 1,280 material-filter edits cover these rules.
The game installs this codec after creating the Havok world; the Havok null
codec is not its gameplay decoder. Runtime material-map ownership remains an
open integration requirement.

`collision_settings` retains authored block flags, version, blocker mode and
legacy Boolean byte. Its constructors and upgrade match 769 executable cases.
`assets::entities::physics_material` uses the shared declaration overlays, the
executable's named enum values and the original post-parse upgrade. All 314
installed physics materials parse; collision masks are 12 for one declaration,
115 for two, 119 for one, and 127 for the other 310. Seven installed reference
cases also compare the complete meaningful settings against executed conversion.
Legacy fields stay distinct from current flags, including at the current version.
The same material loader resolves density, friction, restitution, traversal
flags and collision settings. The collision-settings accessor delegates to
that loader; it does not maintain a second inheritance resolver.

`body_quality` initializes all 32 physics-quality presets from welding and
contact-cache configuration, retaining priority, supported/requested flags and
contact-cache movement thresholds. Eight complete executed configurations cover
all switch combinations. `body_lod` retains the original LOD thresholds and
ordered variant-table rules; two constructors and 512 executed updates cover
mask width, reverse writes and diagonal precedence. Automatic shape-LOD creation
and its event subscriptions still belong to the world owner.

`motion_storage` owns fixed-capacity reservations, the permanent static motion
and the initial 32 cell lists. Three constructors and 80 reservations match the
game's manager. Reserved records remain unreadable until initialized; this
prevents exposing the source allocator's unspecified floats. Motion IDs have
no body-style generation byte. Resizing, release, cell rebalancing, activation
lists and deactivation ownership remain separate lifecycle work.

Trigger resources prepare motion from their cached mass or the compound's
mode-zero calculation. This follows the original bounds-based mass path through
its identity shape-bound transform; it does not replace narrow-phase contacts
with bounds tests. Another 1,024 complete compound calls verify the fallback,
along with 1,024 bounds-mass and 1,024 box-inertia cases. Other compound mass
calculation modes still require child aggregation and tensor combination.
The installed preparation audit passes all 5,576 triggers across 36 levels,
including 23 keyframed triggers and 183 water triggers. These resources have
no embedded cached mass; their unit-scale radius-zero shape keys also have no
separate mass resources in the archive index, so the verified fallback matters.

This is CPU construction support, not an activated gameplay physics world.
Body/entity links, lifecycle publication, dynamic bounds and ordered filtered
shape contacts still need their actual producers before these bodies can drive
the water renderer.

`assets::entities::physics` reads the worldspawn `m_havokBroadPhase` box used by
the original map loader, retaining constructor defaults for partially authored
vectors and zeroing Cinfo W lanes. Every installed level supplies the box
explicitly. A missing inherited box remains an explicit unresolved input;
render geometry and Tome bounds never substitute for it. The installed audit
feeds all 36 boxes into the already compared quantization owner.

`ocean::OceanUpdateState` retains water height, the nine-word FFT control cache,
spectrum invalidation and independent spectrum/surface task flags. Its initial
tile size is 50, water height is zero and maximum wave amplitude is 5. The
world-frame owner resets both task flags before preparing its views; actual
water contacts supply the height-update gate. Water frames also carry the
client's shared cvar ocean tile size and skip flag, read during level
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
`rendering::depth_of_field` supplies material variant selection. Its filter,
cat-eye and fringing selection matches 315 executable calls. The connected
DoF passes and autofocus owner are described with `camera_view` below.

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
vegetation transmittance still requires its own complete producer; its existing
host path remains in use.

Area lights use the installed Ward solid-angle evaluator, clipping and projected
texture profile through `dh2_area_light.wgsl`. `AreaLightScene` prepares one
32-slice texture pool before opaque draws and publishes the same mapped packets
to forward surfaces, fog and particles. Rect components carry the stable scene
record index through the unused GPU colour W lane; their RGB photometric fields
are retained for host materials. The view bindings carry the shared packet buffer
and pool, so materials do not own separate texture caches. Source image copies
are submitted only when pool contents change. This supplies the authored area-light profiles. The installed light updater
`140881640` explicitly clears shadow eligibility for type 3 (area), independently
of `m_NoShadows`; `140331cc0` admits only point, half-point and spot lights.
Consequently ordinary gameplay area lights do not publish paraboloid shadow maps.
The installed shader assets retain a paraboloid ESM receiver, but its presence
does not establish an active producer. The reported EotW room also explicitly
sets `m_NoShadows=true` on its area sources.

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
The gameplay draw receives blur/refraction images and water-aware depth/history
through `camera_view`. The backend `AutofocusPass` implements the single-group
focus update, previously verified against 128 shader states.
The separate depth-of-field module currently implements filter-material
selection. Full bloom appearance and camera-motion parity are not established
by these tests.

### Ordinary decal submission and atmosphere

`app::decal_pass` adapts the game's ordinary decal list (3386a0, bucket 0x23)
to the host renderer. Ordinary decal types 0/3 retain their framebuffer blending
but draw after opaque geometry and before atmosphere. Their original material
specialization excludes alpha fog, so drawing them after atmosphere would erase
some already-composited haze. Alpha decal types 1/2 retain the transparent path.

The adapter reuses prepared host draw functions, pipelines and complete instance
batches. It suppresses only their later per-frame draw ranges and restores those
ranges after submission, preserving the host's retained phase entries. It does
not add a fog shader variant or another fog calculation. The pipeline key keeps
ordinary and alpha decal routes in separate batches.

### Physics point-query composition

`gameplay/world_point_query.rs` joins the owned body table, hybrid spatial trees,
body collision filters and retained compound/polytope query geometry. It preserves
native body/leaf traversal order and duplicate hits; contacts publish the body's
current generation-tagged identity, material, filter and user-data handle. Asset
resolution errors abort execution and discard that query's partial results.
This metadata projection is intended for render-view environment contacts, not
as a replacement for full closest-point witness results.

The shape comparison covers 1,536 executed game queries, including scaling and
missing polytope connectivity. A further 512 complete world queries compare
ordered body/material/key/filter/user packets across two trees. Constraint
adjacency admission in `gameplay/constraint_links.rs` has its own 512-case original
execution corpus. These functions require actual world-owned bodies, geometry,
links and deactivation callbacks; none creates fallback environment contacts.
Gameplay water remains dependent on complete world admission and contact-to-component
resolution before the ocean and underwater render paths can be connected.

`gameplay/deactivation/islands.rs` owns island linkage and merging with one typed
island representation shared by per-motion body registration. It borrows the
existing body/motion stores, preserves attachment and pair order, and retargets
cache links when merging. The 1,296-case executable comparison also covers free
slot reuse, stale member IDs and pair-step clearing. Complete world activation
and contact publication still need their owners connected.

`gameplay/component_state.rs` computes base component admission flags and whether
the scheduler must run. `gameplay/body_properties.rs` owns keyed body values and
retained free slots, separately from body-allocation property bitsets. Executed
comparisons cover 2,048 admission decisions and 1,024 property operations. A hit's
user-data handle is already carried by `ShapeHitInfo`; resolving it to the actual
component owner and entity must not be replaced by a second body-ID lookup table.

`gameplay/body_activation.rs` sequences body and motion activation, notification,
island detachment and the broadphase handoff. Active body IDs are distinct from
allocated-body counts. Solver cell lists and pending body queues preserve native
swap-removal and moved-index repair. Activation notifications retain callback
mutations before later ring traversal; deferred notifications carry typed body
commands. Numerical comparisons cover the full orchestration and its owners.

The hybrid broadphase owns a capacity-aware bitfield of pending body slots, its
minimum/maximum slot range and configured layer pairs. The marking virtual's last
argument is an input-record stride, not a flags mask. Marking records work for
the later update producer; it does not itself refresh tree bounds or membership.
`gameplay/bit_field.rs` implements the shared resize/fill/padding rules used here.

`body_activation/context.rs` connects the controller to the shared activation
signals and the hybrid index's actual pending marks. Subscriber dispatch borrows
the same body, motion, island and spatial owners so callback changes precede
subsequent work. The integration comparison replays the existing 1,024 executed
activation cases through these owners. The hybrid broadphase's virtual +30 is
an empty function in the game; it must not be mistaken for pending-mark processing.

`gameplay/physics_world` now holds the reflected world configuration and body
publication composition. `WorldInfo` reuses the existing LOD and solver presets;
game initialization overrides capacities, gravity and bounds separately from
the SDK defaults. `BodyPublication` executes static/dynamic preparation, actual
hybrid insertion, constraint linkage, activation and added-event dispatch in
the game's order. Its shape and event callbacks are required owner boundaries.
The comparison executes 128 complete game publication calls with real bounds,
trees, overlap queries and activation; only allocation and generic signals are
observed at their ownership boundaries. The unfiltered/filtered broadphase
overlap producer preserves right-first traversal and packed arithmetic; 512
queries compare the ordered output against the game.

`gameplay/physics_actor` owns reciprocal movement-body links, inherited alternate
body flags and signed-frame update admission. The old movement wrapper retains
its actor after another body is attached, matching the game. Attachment and
scheduling decisions compare against 640 executed calls.
Collision data retains its component owner separately: trigger setup publishes
the collision object at +40, while actor attachment changes the movement object
at +20. `collision_user_data.rs` encodes that component reference as a stable
handle; contact resolution reads its canonical `ComponentOwner::entity`.
Thirty-two composed game setup/attachment sequences prove these links remain
independent. Static triggers therefore need no invented physics actor for contact
ownership. Volume bounds still come from the hit body's packed AABB. Missing
ownership or resources fail the query instead of manufacturing dry contacts.

`physics_world/update_queue.rs` implements both bounded transform-update tasks,
versioned free-index reservation, ordered concurrent ring publication, consumption
and record recycling. Its 8,192-entry buffers live on the heap. Constructors
reserve a permanent sentinel, batch boundaries consume that sentinel, and teardown
returns it. `physics_actor::consume_updates` resolves generation-tagged references,
checks ancestors and phase stamps, then calls the required pose producer. Pre-
and post-physics stamps remain independent. The task comparison executes 128 real
constructors and 512 batches, including stale references, callback changes,
sentinel boundaries and free-list reuse. Producer comparisons separately cover
128 full queues and 512 ring pushes, plus concurrent publication. Runtime entity
construction order and the moving-trigger pose producers still need connection
before these contacts can drive the ocean and underwater gameplay owners.
Existing placed water material/geometry rendering remains the single rendering
implementation.

`gameplay/motion_requests.rs` owns general actor velocity requests, signed
priority ordering, request IDs, the ten-entry limit and timed expiry. Actors
retain these actual lists instead of separate marker-only vectors. The player's
one-frame projection reuses the same precedence rule. Request resolution uses
the required body interface, preserving cache flags and changes made by body
callbacks. Comparisons cover 1,536 request operations and 512 actor resolutions.

`gameplay/reference_frame.rs` supplies compact game-frame quaternion conversion,
binding composition, world velocity derivation and independently cached position
and basis prediction. The existing shadow basis product is re-exported from this
shared implementation. Packed arcsine reuses the physics rotation module; its
machine-code polynomial replaces neither the existing sine/cosine implementation
nor the padded Havok quaternion matrix format. Comparisons cover 1,024 bound
frames, 1,024 helper sets and 512 prediction/getter cases.

`reference_frame/joints.rs` handles phase-channel joint remaps, padded pose
matrices, model scale, published/predicted parent frames, model-origin overrides
and basis normalization. Empty channels and invalid joints preserve the game's
output/admission behavior. A missing remap or transform is an ownership error.
The 768-case comparison executes all three game joint-binding entry points.
An installed-asset audit found 23 keyframed triggers among 5,576 triggers in 36
worlds, including 19 water triggers and four joint-bound water triggers. The
remaining gameplay integration must supply actual entity/component construction,
pose-channel buffers and body interfaces; these mathematical kernels alone do
not publish water or substitute static geometry for moving triggers.

Actor construction now supplies the verified empty request managers, scheduling
flags, frame markers and current/predicted transforms. The full executable
constructor was compared using four poisoned input buffers; first request IDs
start at zero after wrapping the initial unset counter.

`gameplay/component_state.rs` owns declaration/entity links, generation assignment
and ordered manager registration. The registration comparison executes 256
sequences, covering generation wrap, signed high-water counts and denied growth.
`storage.rs` now owns the packed pointer-list implementation shared by rendering
and components; rendering's existing public names re-export that implementation.
Live entity/declaration metadata and actual pose/body owners still require
connection before ocean/underwater rendering is live.

Component startup now executes the verified pending-stop cancellation, ordered
removal, start queue insertion and required type callback. A 768-case comparison
executes the complete game start path, including mutation by that callback.
Entity insertion order has a separate 768-case comparison: cached and uncached
lists intentionally differ for equal class IDs.

`component_state/roster.rs` owns slot reservation, its sorted active window,
growth blocks and cursors as offsets into owned storage. `pool.rs` owns typed
chunks using the game's class-size policy. Comparisons cover 256 roster
sequences and 42 complete pool construction/allocation sequences, including
relocation and chunk boundaries. The manager allocates through this pool before
assigning a generation. Physics actors retain the same common `ComponentOwner`
used by lifecycle operations and references, without copied entity/generation
fields. Fifteen composed comparisons run actual game pool, actor constructor,
generation and owner-linking functions together.
`PhysicsActorLinks` now uses that same `ComponentPool<PhysicsActor>` for updates,
instead of a separate owning vector. Host-sized queue handles are checked views
of the pool's real 16-bit slots. The installed factory check constructs an actor
through the entity factory and updates it through the same pool, alongside water.

`component_state/entity.rs` composes the complete entity factory: cache mutation,
typed construction, generation assignment, ordered insertion, owner linking and
immediate startup. Stable class/slot handles replace process pointers. Required
callbacks operate on the actual component and entity; 512 comparisons execute
the complete game path and compare callback-visible state and order. Declaration
scheduling defaults and mask updates have a separate 512-case comparison.

`component_state/entity_setup.rs` performs the entity's component setup in the
resolved declaration-array order. It retains the small-entity uncached path,
preallocates the class cache only above two declarations, reloads live declarations
after factory callbacks, applies the reported prefix flag and finally caches a
non-forcing reference-actor lookup. Its 512 comparisons execute the complete game
routine with the actual list/cache helpers and mutable factory/prefix callbacks.
Declaration parsing, concrete factory dispatch and later activation remain the
level owner's responsibilities; setup does not invent an asset or spawn order.

Actors retain their own current/predicted frame cache, published velocities and
scale. Publication copies pose, velocities and marker while preserving scale
and prediction state, verified against 512 game calls. Entity delta selection
reuses `GameClock::delta` through the verified four-timespace mapping. Embedded
actors and triggers share one verified basic movement interface, selected through
their actual vtable family; rigid and unrecognized bodies have no fallback.
Remaining integration must connect the level's class/declaration bootstrap,
concrete body/entity/render owners and live phase pose buffers before
ocean/underwater gameplay is live.

`class_hierarchy.rs` materializes base classes, ordered child lists, descendant
ranges, preorder type IDs and lookup tables. `name_pool.rs` owns the shared,
case-sensitive immutable names with stable identities, exact cache/bucket rules
and block growth. It reuses the existing declaration-name hash. `class_registry.rs`
combines these with metadata registration, late parent repair and the distinct
pre-/post-initialization lookup paths. Dispatch keys and raw callback descriptors
remain separate; registration does not implement unknown class methods.
Comparisons cover 512 hierarchy scenarios, 512 name-pool operations and 64
composed registration scenarios. Component declarations and pools can obtain IDs
and instance sizes from these owners. The installed-executable audit decoded
all 1,904 class definitions, found one common initializer layout and located all
of them in the 26,015-entry startup table. `assets/class_metadata.rs` reads those
declarations directly from the selected installed executable, validates its
version and compiler layout, recovers startup order and constructs the class
registry. An installed integration check compares every declaration against an
independent disassembly audit. This replays class registration, not unrelated
startup functions; connection to the level's component bootstrap is still needed.

Entity component release removes the first matching entry and corresponding
class-cache slot before clearing owner links. A 512-case comparison executes
both original routines, including unequal cache lengths and missing entries.
Queued-start notifications are named explicitly: the notification is separate
from activation, and the trigger's notification is empty. Actor body detachment
restores its embedded body and updates hierarchy flags without scheduling a
transform update; the expanded 896-operation actor corpus verifies this alongside
attachment and update scheduling.

Per-body event properties own their free chains and deferred destruction queue.
Dispatch visits the global chain and the event's selected body chains in order;
removing a body during an event immediately removes its head but defers value
destruction until dispatch finishes. Required signal callbacks receive the real
mutable owner, and the traversal reloads links after callbacks. A 256-sequence
comparison executes the game's dispatch, removal and chain recycling routines;
the existing 1,024 lookup/allocation comparisons remain unchanged.

Machine-code inspection proved the release build's 166830 diagnostic returns.
The actor motion resolver therefore retains/cache-completes rejected requests
without applying them, and a detach-pending component skips startup unchanged.
Both branches are now in their existing executed comparison corpora.

The installed metadata reader also recovers all 350 generated component factory
bindings: their class, distinct pool ordinal, in-place constructor and actual
capacity cvar registration. It reads cvar names and reset strings from their
initializers rather than treating zero-filled executable storage as defaults.
`component_state/types.rs` owns the materialized class registry and these
bindings, and creates typed pool storage using the existing client's cvar values
and verified class-size policy. An installed check covers every binding, including
a configured non-default water capacity. Constructing a pool does not implement
the component type's constructor or admission behavior.

`physics_actor/pre_update.rs` connects admitted pre-physics tasks to the full
actor traversal: phase flags, live entity clock, velocity requests, bound-motion
fallback, body callbacks, scheduling and child traversal. Entity, joint/phase,
body and queue operations remain required live-owner interfaces. The traversal
retains a bound entity independently of whether it has a cached physics actor,
and reloads child identities during iteration. Its 512 comparisons run the
complete game traversal and real motion-list resolution, with only the clock,
bound-pose, body-virtual and final queue boundaries supplied. Actor bindings now
retain their actual entity, joint, local offset, quaternion and phase. Four
poisoned-storage executions of the complete constructor verify their defaults.
The actor's entity-transform notification copies the current pose, invalidates
prediction and optionally clears published velocity/request validity while
preserving scale, completion marker and prediction storage. Its 512 comparisons
execute the complete game callback and check all other bytes remain unchanged.

`physics_actor/post_update.rs` connects post-physics task admission to the full
publication traversal. It handles request resolution, independent frame/queue
markers, stopped-time bindings, lazy prediction, alternate-body pose callbacks,
all 21 published entity words, render gates and notifications, joint-relative
binding updates, phase rescheduling and live child traversal. `PublishedFrame`
represents the copied entity state without adding prediction caches to entities.
Its 512 three-actor comparisons execute the complete game routine, including
mutable body/render callbacks and local joint-frame composition. Concrete level
owners remain required interfaces; the comparison supplies those boundaries.
Ocean/underwater gameplay remains unfinished until those owners and water
contact/render publication are connected.

`physics_actor/basic_body.rs` implements the five shared virtual methods of the
embedded and trigger movement interfaces. Explicit admission guards retain the
distinction between those bodies. A 512-case comparison constructs both game
types and executes their actual vtables, checking unchanged body/motion storage.

`gameplay/water_component.rs` owns the water component's common component state
and retained plane. The constructor leaves that plane uninitialized, represented
explicitly until publication. Transform updates use the active component's main
physics shape and its actual transformed `calcAabb`, preserving the source phase
gate. A 1,024-case comparison runs the complete game function, including exceptional
heights. This component plane is separate from render-view water selection,
whose height still comes from the hit body's packed world bounds. The installed
factory integration check constructs this actual water type through the shared
pool, generation allocator and entity factory using current client settings.

`reference_frame/basis.rs` owns the compact entity basis inverse and quaternion
conversion used by post-physics binding updates. The inverse preserves the
game's float cofactors, double reciprocal/products and determinant admission;
the quaternion conversion keeps its distinct compact-matrix sign convention.
These operations have 1,024 comparisons against the complete game functions,
including singular and exceptional inputs. They share the existing frame basis
product rather than modifying the separate Havok body conversion.
The retained local-binding helper also matches 512 executions of the complete
game post-physics branch with a non-joint parent. This comparison exposed and
corrected the shared product's per-lane floating-point operand order; all existing
binding, prediction and joint-frame comparisons still pass.

`physics_world/publication` publishes body poses and velocities through the same
body storage, motion storage, broadphase and deactivation owners used by body
creation and contact queries. Pose changes retain body-cache W lanes, update
attached motion frames/cells, accumulate movement distances and notify observers.
Velocity changes preserve activation admission, convert world angular velocity
to motion space, refresh every attached body's swept bounds and mark inactive
members together. Comparisons execute the complete corresponding game paths;
transform cache refresh and attachment/deactivation notifications still require
concrete level implementations.

`collision_user_data/velocity.rs` owns the collision payload's four velocity
vectors and the command wrapper that publishes them. It applies the selected
motion property's scale, selects the real immediate or queued setter, then
updates the live attachment ring while preserving the requested linear W lane.
The first-body reference is reloaded after callbacks. Trigger pose publication
likewise resolves its body and water references after physics callbacks rather
than holding stale snapshots. The command queue, component bootstrap and
render-view water publication still need their concrete level connection.

`physics_world/command_buffer.rs` implements the expandable deferred-command
queue on retained blocks, with atomic reservations and the game's allocation,
reuse and reset rules. Shared producers reserve disjoint byte ranges; dispatch
and reclamation require exclusive access after producers finish. The existing
packed pointer list retains the game's capacity policy. Comparisons cover 6,144
queue operations, including overflowing reservations, both reset modes and a
handler changing its packet length; a concurrent-producer check verifies payload
integrity across block growth.

`physics_world/commands.rs` supplies the installed game's 4,096-byte queue
configuration and group-2 packet routing. Body addition, pose, velocity and
activation commands call the same shared physics owners as their immediate
entry points. Other known operations report missing implementations, while the
source processor's actual no-op codes remain no-op. A comparison of 4,224 queued
packets runs the game's router and processor; existing full world-state fixtures
also exercise deferred pose and velocity publication. This does not yet connect
the level component bootstrap or complete ocean/underwater gameplay rendering.

`collision_body/motion_cache.rs` reconstructs the current body pose from its
motion and cached local mass frame, updates historical movement counters,
refreshes swept bounds and collision LOD, and preserves or expands retained
bounds according to the game's slow-motion rules. It reuses the existing bounds
expansion, packing, quaternion-basis and inverse-length code. Its 1,024 full-game
comparisons cover every body quality/configuration, missing selected-shape state,
optional LOD manager, retained bounds and exceptional floating-point branches.

`deactivation/contacts.rs` preserves the game's overlap-pair and swap-removal
order. `DeactivationWorld::queue_island` retains wake bits and queues each island
once. `physics_world/publication/deactivation.rs` composes those rules with the
existing activation, attachment-ring and constraint owners. These algorithms now
exist as concrete implementations; the level still needs to bind their settings,
geometry and observers before ocean/underwater gameplay rendering is connected.

`physics_world::PublicationContext` supplies concrete transform-cache and
attached-body updates to immediate and queued world publication. It borrows
geometry, observer dispatch, immutable quality/constraint data and current world
settings; mutable bodies, motions, deactivation and broadphase remain the same
owners passed through publication. `TriggerAssets` implements this geometry
boundary using the retained resource's `QueryShapes::bounds`, preserving root
and compound/convex dispatch. Resource index zero is valid, lookup errors remain
errors, and source NULL is a distinct explicit geometry state. Level bootstrap
and rendering publication still need to instantiate these owners.

`trigger_body::TriggerComponent` uses the shared component owner/factory and
retains only the index of its movement interface in `PhysicsActorLinks::bodies`.
That table owns the single mutable body ID and reciprocal actor link. Trigger
collision velocities and live water/wind component handles stay with the typed
trigger pool. Constructor comparisons execute7792c0 on16 poisoned records;
the installed factory test constructs water, reference actors and triggers
through their actual class metadata and common entity registration path.

`WorldPointQuery::resolve_pending` executes queued point requests through the
same broadphase/body/filter/shape path as direct queries. Host geometry failures
propagate without publishing empty or partial completed results. Successful
completion still uses `CollisionQueries`' verified ordering, generations and
owned-result transfer. `EnvironmentContactSelection::consume_query` composes
88d060's clear/readiness gate, result transfer, collision-component resolution,
packed world bounds and ordered selection. Live contact overrides still require
the level's physics/component admission and frame scheduling. Ocean surface
submission does not wait for completion of that wider lifecycle.

`physics_world/storage.rs` composes the canonical body, motion, deactivation,
broadphase, quantization and shape-observer owners for body creation and queued
point queries. Geometry and component signals are borrowed from the level.
The installed Edge of the World integration check creates 332 triggers through
this path and resolves their real compound geometry; startup phase processing
and renderer publication remain separate unfinished connections. Body lookup
masks the generation byte from broadphase IDs, matching `1288af0`; the native
multi-body query comparisons include generation-tagged leaf entries.

`bevy_backend/ocean_scene.rs` connects the existing ocean implementations to the
gameplay atmosphere owner: installed program states and authored mask images,
view-dependent patches, retained FFT simulation, displaced water depth, foam,
and HDR colour. It uses the same camera history, opaque-depth reduction, shadow
atlas, fog and GPU exposure as placed water. The rewrite's shared console owns
ocean settings. CPU patch culling consumes the game matrix rows; GPU vertex
uniforms consume the separate column-ordered transport. Both ocean vertex entry
points declare invariant positions: the colour pass requires exact equality with
the depth prepass, so backend shader optimization must preserve their shared
displacement and projection arithmetic. Otherwise the colour pass can reject
water pixels and expose the pre-water scene through waves.

`assets/fx/screen_quad.rs` reads authored fullscreen-quad timing and stage
descriptors. `rendering/screen_distortion.rs` resolves the selected materials,
registered defaults and shipped compiled noise images. The gameplay camera's
`bevy_backend/screen_effects` owner runs these after camera effects and before
exposure/bloom, independently of the camera motion-blur setting. It reuses the
scalar FX envelope and the HDR storage bridge, with separate immutable uniforms
for the two distortion resolves. No distortion passes run without active quads.
The configured character supplies submerged and leave-water FX declarations.
Water contact is queried at the player's body independently of the render-view
query. The debug camera reuses its established feet probe. Ocean contacts admit
the swimming controller's five-point wave query, using the real displacement,
collision-height, foam and authored masks. Four persistent readback slots map
after queue submission; the controller consumes its retained handles in order.
The head sample controls submersion; the five-point maximum is a distinct body
surface value. `ark_player_follow_waves` gates submission. F8 includes both
surfaces, the five samples and the actual distortion vector field.

F8 numerical display captures include same-frame ocean displacement, gradients,
persistent foam, shoreline foam depth, immutable refraction source and water
colour, together with the resolved ocean environment and clocks. These copies
run only on request; they do not add a continuous rendering pass. Comparing
water colour with its immutable source distinguishes rejected water draws from
foam or reflection contributions without inspecting screenshot images.
A captured nearby-ocean view checks that the generated leaves cover visible
surface intersections.
Pending contact queries use the existing invalid-query rule;
live water-trigger overrides and underwater composition remain unfinished.

`gameplay/motion_properties.rs` retains the world's64-byte property records,
source profile presets, bitwise sharing identity, free-slot chain, growth and
added observers. Profile configuration reuses `DeactivationProfile`; its generic
coherence update retains the source's integer-indefinite conversion, exponent
controls and byte wrapping. `body_velocity_scale` resolves BodyStorage ->
MotionStorage -> property ID -> actual+0c value, with errors for missing owners.
No default unit scale substitutes for an absent table. The existing solver
receives its named properties through a projection of the same retained record.

`collision_user_data::velocity::VelocityPublication` binds18d020 to the real
BodyPublication, MotionPropertiesLibrary and PhysicsCommands. References and
velocity payloads are resolved through the same observer/component owner;
PublicationContext forwards these lookups to its event owner, so activation
callbacks can change the referenced body before the next read. Ring traversal
reads canonical BodyStorage links and user data. Immediate publication uses the
actual world setter; deferred publication emits the existing type9 packet.

`trigger_body::TriggerComponentStore` borrows the same typed component pools and
movement-body table used by entity construction. `ComponentPublicationEvents`
emits world signals with access to those live components and physics owners;
`TriggerPublication` composes pose publication, deferred packets, motion reset
and water-plane updates. Its source comparisons run the real queue/router and
physics helpers, including observer changes to references and deferred state.
`DeferredPhysicsState` preserves the actual three-byte OR decision from8a7820;
it does not assign defaults to the owning level's state.

`WorldStorage` owns body event properties with typed `Signal` values alongside
the canonical body and motion buffers. Trigger startup uses the same publication
context as creation, then registers the trigger on its live body's event chain.
Component update scheduling partitions the existing factory roster rather than
creating another object list. The installed-level composition check covers
static and moving trigger startup, reference-actor attachment and collision
queries. Trigger observers borrow the whole canonical actor/body owner, so a
binding never requires a copied movement table. `EnvironmentViewQuery` retains
the render view's query ID, cancels unconsumed work and submits the actual
camera point against trigger and water-trigger layers. App-level contact
publication remains unfinished.

`assets/entities/water.rs` resolves water component inheritance and inline
overrides, including spawn/context admission, density, current, material
references and wave amplitude. Constructor and enum defaults come from the
installed executable. `WaterComponent` retains the plane and current/density
vector only after publication; material IDs belong to the level's shared
physics material owner.

`gameplay/materials.rs` owns declaration handles, material records, free slots
and registration signals for the shared physics world. Material conversion,
duplicate registration and name collisions use the game's rules. Packing
constants come directly from the audited installed executable through the
shared checked PE reader in `assets/executable.rs`; no extracted table is needed.

`EntityComponents::find_component` implements877720's cached/uncached search
and phase preference, including the last eligible fallback. Reference policy
controls state admission and never forces component creation. Common base
initialization uses `ComponentOwner::mark_initialized`. Full1024 source
comparisons cover lookup order and admission across initialized owner states.
The level still needs body/motion creation and component lifecycle scheduling
before these owners can feed actual water rendering.

The trigger publication comparison now covers1,024 complete static and
keyframed-motion cases. It executes the game's body-cache, motion-frame,
queue/router/processor, bounds and water-plane functions, comparing body history,
motion state, pending broadphase updates and live observer changes. Level
creation remains a separate unfinished connection; these checks do not imply
that ocean or underwater rendering is active in gameplay.

`DeactivationWorld` represents allocated but uncreated motion states explicitly
as `None`; level construction initializes only the permanent anchor. Indexed
access requires an initialized state. Reinitialization preserves the exact
bytes left untouched by the game's partial constructor. The provided-record
constructor remains available for owners that already have initialized data.

`BodyPublication::create_motion` composes the real free-list reservation,
runtime motion initialization, deactivation-state growth and creation observers.
`PublicationContext` forwards capacity/creation signals through the same live
component owner. Full512 source comparisons cover table growth, all initialized
motion/state bytes and callback mutations. Buffer-resize subscribers and the
surrounding body/level bootstrap remain separate unfinished connections.


`water_component::startup::start_water` publishes through a component handle in
its owning pool. Bounds/material callbacks and contact notifications resolve the
same retained component; no temporary removal or duplicate owner is required.
The trigger handle is resolved once and its movement body is read again before
material registration, matching callback-sensitive startup ordering.

`gameplay/water_contacts.rs` owns per-body water membership and density updates.
The contact table preserves bucket and body-index order, duplicate water entries,
generation-sensitive body removal, and start/stop notifications. Its default
bucket count comes from the game constructor. Density uses the custom signed
shape-volume property at key1, independently of cached mass/inertia keyf100.
The separately owned body callbacks remain required. Executed-code comparisons
cover entry/exit, start/stop reordering, density and body destruction. This is the
membership owner, not the water-force simulation or its four deferred query queues;
those queues and live camera/contact publication remain unfinished.


`EnvironmentViewContacts` retains a camera query and its ordered selection.
It resolves requests through `WorldStorage` and then consumes their actual body,
component and entity results; no separate query-only collision world is created.
`WaterViewPublication` implements the water override after ordinary environment
tracking: a non-null `water/SurfaceMaterial` selects the surface and publishes
its physical upper height, while a missing material disables that surface.
Water-trigger admission controls ocean-height updates independently.
`ParameterReader::material_source` reads Material resources separately from
images; environment volumes retain these references at load time. The installed
composition test connects geometry, RTTI admission, selection and publication.
Gameplay level bootstrap and render-frame/material binding remain unfinished.


`ComponentRegistry::initialize_and_start` processes the game's sorted
initialization and startup batches, including registrations made by callbacks,
startup deferred until initialization, failed-initialization stop requests and
queue prefix cleanup. Active cursors stay with the registry. The component owner
supplies required callbacks and its factory initialization rank; LOD scheduling
priority is a separate value. The shared idLib sorter lives in `sorting.rs`,
with the menu retaining its existing alias. Registry queue constructors preserve
the source owned-list header flags. Level bootstrap remains unfinished; the
dispatcher is not yet invoked by gameplay.


`component_state::dependencies::InitializationPlan` expands exact and inherited
class dependencies in factory order, preserves duplicate edges, partitions weakly
connected groups, then applies the game's dependency-first traversal and cycle
paths. Source comparisons cover complete350-type graphs. Factory ranks are unique
across types; equal-rank component instances still use the shared source sorter.

The checked PE reader also loads350 `ComponentTypeDeclaration` inputs from the
generated descriptor constructor. Class references are cross-checked against the
same installed factories; setup addresses and static definition identities remain
explicit. `ComponentTypes` retains them alongside its class registry and pools.
The reader does not execute game code or mistake zeroed PE storage for constructed
dependency declarations. `component_definitions` reconstructs all350 static
definitions from their literal constructors using iced-x86. Its bounded reader
accepts the audited constant operations and typed declaration helpers; it never
executes game code. All938 ordered records, reasons, exact-class flags and eight
stage values match independent execution of the installed constructors.
`ComponentTypes` resolves initialization ranks through the shared class hierarchy
and dependency graph. Its98 groups and all350 ranks match execution of the game's
class registration, hierarchy and graph routines. Update-stage scheduling and the
level-owned lifecycle adapter remain separate unfinished integration work.

`trigger_body::initialization::initialize_trigger` owns779530's initialization
sequence: copied owner bit, shape admission, base initialization, live water/wind
lookups, body recipe/publication and update request or entity keyframing. Its
required world interface resolves the same pools and movement-body table used by
publication. The declaration identity survives body creation and its keyframed
field is read again afterward, including observer changes. Missing shapes follow
the game's reporting/release continuation, rather than trusting the decompiler's
noreturn annotation. These operations still need the gameplay level adapter.
`ComponentRegistry::request_release` implements7e72c0's preceding stop request,
initialization cancellation and immediate shutdown/release callbacks or deferred
release queue. Queue draining remains a separate lifecycle operation.

The trigger initialization and water startup `physics` adapters borrow one
`WorldStorage`, its geometry resources and `ComponentPublicationEvents`. Trigger
body creation uses Disabled/Activate and publishes directly into the shared
movement interface. Water admission schedules the real water pool; bounds use
the main shape's calcAabb and live transform, and material resolution/assignment
uses the world's existing material library with mode zero. Entity lookup,
reporting/release, material observers and water-contact notification remain
required level operations. Installed composition checks create real water
instances, run these adapters and consume their live flags through camera queries
on four levels. These checks still supply explicit component lifecycle inputs;
they do not replace the unfinished gameplay bootstrap or render-frame binding.

Placed rigid horizontal water now submits colour through the gameplay fog/water
owner. `WaterModelColours` retains bank0 selections independently of world bank2;
exact installed shader pairs admit the supported position-only horizontal forms,
with either no probes or one reflection probe per model.
Both use the existing water shading and HDR composition. Models supply current
host mesh positions and a game-space affine matrix, so baked shear and CPU mesh
updates match their depth pass; world geometry retains its raw source positions.
`WaterModelLighting` shares the existing object lighting buffer and its draw
indices. Unused record lanes retain the query's tetrahedron and barycentrics;
the water vertex stage selects the strongest probe in the installed accumulation
order and maps its ID through the real cube-array lookup. Model motion updates
these inputs together with opaque lighting, without a second query or GPU pass.
World water keeps its per-vertex probe transport. Skinned water, nonhorizontal
forms and complete underwater integration remain unfinished. Appearance awaits
human confirmation.

Underwater classification now has independent ocean and water-volume dispatch.
The volume branch uses the prepared surface height and installed volume-stencil
program, even with ocean rendering disabled. Its fullscreen classification and
visible rigid/world water boundaries mark main camera stencil and reduced alpha
stencil; boundary
admission uses material bank0/bank2 slot6 (`zPrePassAlphaProgram`), independently
of the normal depth pass in slot4. Empty PS640 and supported position-only vertex
stages reuse the shared mesh submission without a colour attachment.
Water depth submission is split at the ocean insertion point: clear/copy and
boats, ocean, placed water and boat restoration, one alpha-depth preparation,
then underwater marking. The gameplay publisher now resolves camera contacts
through the level-owned `PhysicsScene` and applies the selected water material
and physical surface height after ordinary environment blending. Runtime
admission and appearance of this integration remain unconfirmed.

`UnderwaterScatteringPass` loads `water/underWaterPostFX.material` and blends the
installed scattering/caustics formula into the fog radiance buffer after local
light scattering and before bilateral filtering. It reconstructs world positions
from current opaque and water depth, uses actual exposure and low-quality sun
visibility, and preserves the source's radiance/alpha blend. The host reads the
reduced stencil's 0x40 bit through a stencil-only integer view, so fog quality
does not require copying stencil or matching framebuffer attachment dimensions.
The full-resolution mark targets the main camera's D24S8 attachment; reduced
marking targets depthAlphaDiv2's D32S8 attachment, with separate pipeline formats.
Ocean admission follows the prepared water height plus maximum wave amplitude;
the volume branch requires the selected surface material publication. That live
contact publication is connected through the gameplay publisher. CPU/shader
parameter preparation and render submission are connected; runtime and visual
confirmation remain pending. `ComputeGodRays` now follows underwater scattering,
using the installed eight-step caustic accumulation, optional fog-shadow input,
and independent skip controls. `CopyWaterStencil` supplies its reduced filter
visibility mask; authored enable/intensity values use ordinary environment
tracks. Moving trigger propagation and complete reference actor binding remain
unfinished.

Gameplay submits the authored celestial disc through the retained atmosphere
sun pass after water depth preparation and before water colour. It tests against
`depthWater`, as the game's fog-task context specifies, so a water surface can
occlude the direct disc before underwater refraction supplies its sun highlight.
The former transparent celestial material is no
longer registered by gameplay: submitting both rendered a second undistorted
disc after water and supplied that duplicate to bloom.

`SunRays` submits the installed projected colour mask and radial sun scattering
before area/local fog. It reads the real water-depth pyramid, shared fog
shadow/canvas/interior sources and camera stencil. Its small visibility target
also receives underwater classification before the common fog filter. Shader
depth coordinates account for pyramid padding independently of projection.

The lens owner now supplies `FlareChain` with a real pre-DOF half HDR image from
`HdrDownscale`, the authored profile pair and current adaptation clamp. It no
longer bypasses bloom and lens composition whenever flares are enabled. The
flare target is half the half-depth dimensions, independently of luminance mips.
Water and lens adapters both follow the installed HDR selection: format 0 is
compressed packed colour; format 1 is uncompressed half-float colour. Appearance
of these newly connected paths remains pending human confirmation.

The shared sun/underwater visibility mask now passes through the installed
horizontal and vertical `renderBilateralBlurNoDepth` expansion before the
common fog filter. The retained owner uses one small R8 scratch allocation;
the source mask receives the second pass. It preserves the shader's saturated
sum, mirror sampling and width-based step in both directions. Fog-shadow
strength, marching taps and full-size filtering remain unchanged.
Underwater rays likewise sample `depthWater` hardware depth for inverse clip
projection, rather than interpreting the fog filter's linear min/max distances
as clip depth. This shares the existing water-depth allocation.

DirectX stencil-only shader views remap DXGI's green stencil component to the
single red component expected by WGSL, with absent channels set to (0,0,1).
This applies to both D24S8 and D32S8 in the vendored backend; shader consumers
retain their ordinary portable `.x` reads without extra conversion passes.

Water appearance confirmation (2026-10-25): the user confirmed that underwater
visibility now fades with distance and that moving wave-generated light bands
are visible away from the sun. The DX12 stencil sampling correction is confirmed
by both same-view numerical captures and this human check. The user also sees
sun-centred streaks above water. That radial streak pass excludes underwater
pixels using stencil bit 0x40, matching game function309b50; it is distinct from
underwater `ComputeGodRays`. The earlier notes that all underwater appearance
was pending are superseded for attenuation and wave beams. Underwater solar
disc/glow alignment was also confirmed by the user. DoF is the next requested
rendering feature after this water handoff.

`bevy_backend::camera_view` now composes the existing autofocus, masks, opaque
colour filters and final camera combine. `camera_scene` submits refraction
preparation/composition before particles and transparent colour, then final DoF
after transparent colour and before exposure/bloom. It borrows same-frame water and
alpha depth from the atmosphere owner, and retains focus and scratch images.
The final combine samples the hardware depth aspect directly. The flare owner
borrows the pre-DoF colour image instead of reducing the already blurred frame.
Camera controls follow the shared console generation, including settings-menu
changes, and `r_skipDOF` continues to bypass camera effects.

Transparent DoF preparation (`camera_alpha_colours`) and composition
(`camera_alpha_composite`) follow 317890/313730, with the shared packed glass
decoder and explicit material/stencil selection. `CameraView` composes them
with 318390's alpha mask when supplied completed opacity geometry. Gameplay
supplies that producer through `alpha_depth_scene`: actual transparent mesh
instances reuse their forward vertex deformation and material bindings for the
full/reduced alpha-depth draws. Refraction writes packed glass distortion into
the RG16 opacity target and supplies the special-alpha flag to the camera task.
Material-slot admission comes from installed declarations and shared program
rules. Unsupported custom programs report their material identity. Materials that
enable effect support retain their ordinary bank while no effect is active,
matching the model constructor's selector of -1; active effect overrides still
require their actual forward-renderer producer. The ordinary
pause-menu blur is human-confirmed; transparent appearance remains unconfirmed.
Implementation continues independently of that appearance check.

Reduced alpha depth uses 36f8a0's shared downsample allocation, independently
of 2fbc60's half-size viewport. It is resized when quality changes as well as
when the window changes. DoF and flare colour allocations use the same reduced
dimensions. The ordinary alpha-depth copy preserves its source viewport and
clips the scissor to the attachment as required by the host API.

The alpha clip helper is shared by the prepass and transparent-depth shader.
Glass IOR/normal-mask values honor MAIN defaults and authored overrides; blur
size retains its uint representation until its shader conversion. The packed
angle uses the installed shader's acos lowering and half-float packing. Alpha
pipeline preparation waits for every admitted pipeline before publishing that
view, and DoF consumes only same-camera, same-frame outputs.

Explicit F8 display captures include the completed same-frame RG16 DoF opacity
image when transparent geometry was submitted. This adds no per-frame readback.

Atmosphere submission has separate depth preparation and composition phases.
Opaque decals precede depth preparation; the alpha geometry phase then updates
its real depth/stencil targets before atmosphere composition. Transparent colour
follows atmosphere and refraction-background composition, then final camera DoF
and exposure follow transparent colour. This preserves the compiled dependency
DOF_ALPHA -> ALPHA_LIT -> DOF; glass surfaces are not part of their own blurred
background. The preparation
phase publishes its own camera/frame marker, independently of completed fog;
lookup generation, sky, snapshots and water depth are recorded only once.
Alpha-depth vertices retain the mesh's deformation but omit colour-only fog
interpolators. Alpha depth remains available when DoF is disabled because fog
also consumes it.

Linear water-depth reduction has its own binding and recording method, shared
by normal atmosphere submission and views with fog disabled. Those views still
prepare opaque/water/alpha depth for DoF, while skipping fog radiance, shafts and
fog-shadow work. Material uint vectors retain integer token words, scalar splats
and zero tail lanes; they share material scoping with floating vectors without
converting integer components through float storage.

Runtime capture confirms the glass depth/opacity producer writes nonzero packed
refraction values before camera composition. Its final geometry draw uses the
actual main depth format; full/reduced alpha targets retain their own format.
Clear colour follows the same host-to-game radiance conversion as HDR pixels.
This numeric confirmation does not replace the pending human glass appearance
check.

DoF alpha-depth program coverage follows installed program branches. Default and
vegetation materials share their Z clip; vistas use their authored alpha-test
switch and fixed 0.5 cutoff; simple sprite meshes have no depth-stage discard;
mesh filling containers reuse their existing fill/plane clipping. Default and
vegetation glass supply packed distortion. The prepass and alpha-depth wrapper
share this clipping code, including anisotropic Z sampling and the shadow path's
fixed cutoff. Ordinary mesh decals also resolve their slot-six alpha-depth
programs. Their colour and alpha-depth stages share the installed decal diffuse
rule: anisotropic diffuse sampling, layer-zero LQ mask/tint, cloth mask handling
and vertex colour, with the depth cutoff applied before HQ/gloss conversion.
Projected, FX and editor-unlit variants require their corresponding producers.
The ordinary decal-list partition precedes alpha-depth batch preparation, so
opaque-interval decals cannot enter the later transparent-depth list depending
on host system scheduling. Program overrides with different parameter/geometry requirements,
late projected decals, active material-effect banks and special model depth
variants still need their actual source producers; they are not generic glass.

The simple DoF branch (`r_dofOpaqueEnabled=0`) also writes the shared half-size
colour before combining and publishing it to flares. It reuses `HdrDownscale`;
the enhanced branch continues writing that same target through `CameraColours`.
Switching modes cannot leave the simple combine reading an unwritten or stale
half-colour image. This adds no draw to the enhanced path.

Late projected decals depend on the runtime projection system's affected-instance
lists, three World2Decal rows and per-instance colour. These are separate from
static BWM decal meshes and cannot be inferred from the BWM's fifth buffer.
Their absent scene producer remains an explicit renderer integration limitation.

Alpha-depth geometry follows 321860's quality-scaled viewport, independently of
the reduced texture's minimum allocation dimensions. Its scissor is bounded by
the attachment. This differs from the preceding 2fbc60 depth-copy pass, which
uses its own half-size viewport; the two source rules remain separate.

Projected-decal geometry now has a reusable producer in `rendering::decals`.
It reuses the existing authored affine, inverse and transformed-bounds rules,
then queries the retained WORLD tree with the asymmetric projection box
`[-1,-1,-1]..[1,1,0]`. `rendering::bounds::OrientedBox` implements the source's
15-axis overlap rule without padding. `LocalWorldQuery::query_box` retains
node broadphase and individual oriented-box tests. These functions supply
geometry intersections; runtime decal creation, material-dependent receiver
filtering, matrix/colour upload and shader submission remain unconnected.

The installed-resource DoF census covers all 36 WORLD files and 6,191 material
declarations. WORLD MAIN names use the existing mesh owners; the additional
`particle/add_default` records are screen-gate impostors excluded during material
admission. Of 1,599 materials declaring depth-program overrides, cross-program
names are ArkDecal or ArkSSSRender using ArkDefault for full Z. None are current
Probe alpha-depth MAIN families, and no explicit reduced-alpha overrides were
found. This verifies source identities, rather than runtime appearance or the
unimplemented SSS/runtime-decal primary renderers. Glass appearance remains
unconfirmed; implementation does not wait for that appearance check.

Dynamic-decal lifetime now lives in `rendering::decals::DecalPool`. Its admitted
limit is 4..128; free allocation, oldest eviction and duplicate-owner lookup
follow the native lists/hash. Reusing a full slot preserves its committed render
object. Removal checks both owner and slot, unregisters before destruction, and
cannot remove a replacement through a stale FX binding. The pool reuses the
existing hash owner, including its new unlink operation.

`prepare_world` connects retained descriptions to committed WORLD projection
geometry in physical slot order. Position, axes or scale changes invalidate
intersection data; colour/material changes copy independently. The initial
projection cannot publish an undefined inverse. FX event creation and GPU draw
submission remain separate unfinished integration work.

`assets::decal::DecalMaterials` reads the four material references from installed
decal declarations and preserves their receiver/instancing identity. Suffixed
and suffixless material names resolve only against resources actually present
in the catalog. The app uses the existing material cache, probe and texture
streaming owner for these variants. No extracted data or material-name heuristic
selects an opaque/alpha substitute.

`decals::gather_world_draws` prepares the projected WORLD draw packets from
committed projections, receiver ranges and actual instance visibility. It reuses
one 64-byte matrix/colour record per projection, retains WORLD IDs in the low
24 bits of each remap, and preserves the source's 256-record limit and target
cursor order. Depth-box receivers retain their separate admission path.
Projection-bounds visibility does not add a new WORLD geometry cull. The pool's
`register_world` supplies transformed bounds and registration flags through the
render-world service, publishing an ID only on success. These CPU producers are
implemented; the gameplay FX spawn owner and projected GPU draws remain to be
connected before projected decals are a visible gameplay feature.

`decals::DecalEffectBinding` connects sampled FX placement to that pool. It
resolves the declaration only when creating a record, follows position and
orientation independently, and updates scale/colour every tick. The attachment
computes the source's asymmetric-box pivot offset in its local and parent frames;
orientation reuses the shared reference-frame basis product. Eviction retires
the old FX binding without removing the new pool owner. Descriptor loading,
scheduler activation and parent selection remain with the gameplay FX owner.

Decal phase sampling now lives in `gameplay::effects::decal`, sharing declaration
curves through an `Arc`. Its timing update preserves delay, single loop restart,
independent phase bases and retained fade-out values. The scalar depth/opacity
curves reuse the existing sampler; size/colour use `effects::vector_curve`'s
distinct two/three-channel cubic rules and per-channel minimum/maximum mixing.
The render binding consumes the resulting sample, including expiry before pool
updates. These helpers do not spawn an effect merely because its declaration exists.

`DecalEffect::new` connects source initialization to animation, attachment and
pool binding. Timing, size, depth, rotation, position, colour and opacity consume
the shared RNG in source order. Shared vector randomness consumes two draws;
independent randomness consumes all range draws followed by all curve-mix draws.
Attachment orientation reuses the shared basis product and axis rotation, with
exact normal-axis permutations and the source's degree conversion constant.
Descriptor defaults come from its constructor/reset chain: independent size and
colour randomness, unit ranges, size fade-in/out ramps and constant unit colour,
depth and opacity curves. Following is enabled; the attachment defaults to Z
projection, centred XY, front-face Z and zero local rotation/translation.

FX group traversal now reuses `EffectPoolLinks::update_chain` for the shared
decal/environment lifetime rule. Inactive entries detach before being returned
to the free list; effects expiring during the current update remain linked until
the next tick. `DecalEffectBinding::detach` checks cached slot and owner without
resetting the retired record's fields; eviction notification remains a separate
operation. Installed FX descriptor decoding, gameplay activation and projected
GPU submission are still required for visible dynamic decals.


Particle mesh descriptors live in `assets::fx::particle_mesh`; emission-position
and velocity meshes use the separate `particle_emission_mesh` parser. All three
resolve PMODEL resources through the installed archive catalog and share model
ownership. `gameplay::particles::mesh_appearance` retains initial mesh colour,
three-axis rotation, size, pivot and flipbook state in stable particle slots.
Sprite initialization precedes mesh initialization on the shared stage RNG;
per-slot lifetime updates start from the particle seed.

`rendering::particles::mesh_publication` publishes full mesh transforms using
the same stage simulation, owner pose, visibility curves and clipping inputs as
sprites. The backend caches authored PMODEL geometry and uses the existing
particle material, lighting, fog, transparency and distortion passes. It does
not create a separate mesh-effects renderer. Installed quad indices, vertex
colour, tangent handedness and texture coordinates are retained.

Two-sided particle lighting reverses the geometric tangent basis on back faces,
retains the vertex basis for packed indirect-light decoding, and applies the
installed vertex-normal orientation rule before vertex lighting. The flag is
resolved with the other material specialization options.


Particle declarations retain authored `m_lod` distance thresholds and the
sprite/mesh spawn and emission-rate alternatives. The current selected camera's
distance from the emitter selects the first inclusive threshold, preserving
list order; each parameter clamps that level to its own alternatives. New
entries retain their reflected defaults, including emission rate20 and sprite
size .25. This uses the ordinary simulation and rendering owners.
