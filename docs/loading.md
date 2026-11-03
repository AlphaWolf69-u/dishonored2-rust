# Loading stages

Evidence from the installed assets and Dishonored2.exe, 2026-11-03.
This describes the stage split and verified reuse mechanisms, not a complete
port of every resource class's residency policy.

## Before the main menu

Startup establishes the engine/resource/rendering/audio infrastructure and
prepares the shared resources and the menu scene/UI. The menu is itself a
map: `ark_mainMenuMap` defaults to `campaign/menu/menu_p` (`141353790`).
`14004a7e0` / `14004a850` issue `map <ark_mainMenuMap>`, and `1408a9680`
compares the current map against that cvar to recognize the menu map.

The installed menu has its own entities, navigation data and lighting probes.
`menu_p.entities` contains 221 entity records and 34 component definitions.
The `ui/gamelogic/state_mainmenu` declaration selects `ui/mainmenu.iggyfile`
and `menucam_1`. This is a scene load plus a UI state, not just displaying
the menu's bitmap or loading every campaign level in advance.

There is also a separate renderer-side loading background path:
`140369120` can draw `models/guis/loading_screens/loading_mainmenu_is.png`
directly. Its existence establishes an early loading display that does not
require the fully running main-menu movie. The exact handoff between this
path and the loading movie during every boot variant has not been traced.

## Before a gameplay level

The destination changes to the selected gameplay map, using the existing
engine and resource manager. Resource acquisition and game/world construction
are distinct work: `140062ce0` has explicit progress phases for the resource
file and game creation; `1408a96e0` is the map-load entry.

The resource system understands `init`, `common` and map-specific load lists
(`generated/buildGame/<name>.mapResources`). `1400661e0` processes the list,
acquires resources through the type-specific registry and reports resource
progress as loaded/total at resource-manager offset `0xfe24`. The map also
has world/entity/visibility/physics/navigation/lighting companion resources;
these are not all enumerated by the map-resources list itself. Dependent
models, images, materials, animations, sounds and declarations are acquired
through their owners. Loading a save additionally restores the level state.

The installed loading presentation includes `ui/loading_screen.iggy` and
its `.iggyfile` declaration, plus authored per-map loading backgrounds.
The presentation's movie state is separate from the resource loader's work.
The existing image owner uploads base mip chains at load (`1402ea140` ->
`1402e9ff0`); texture streaming (`14030e840`) continues during gameplay.
Finishing a loading screen does not imply every image's highest mip is resident.

## What can survive a load

- `1400607f0` first queries `14005feb0` and returns an already acquired
  resource rather than constructing/loading it again. Registry entries can
  also be placeholders awaiting their first actual load: entry existence is
  not proof that all payloads are already resident.
- Resource lifetimes are classified. `1400600d0` / `140060180` purge a
  specified class; `1400410f0` can purge class 0 and optionally class 1.
  Class 2 is explicitly identified as `staticResource` by the destructor
  checks at `14005e7b0` / `14005e810` and survives those ordinary purge calls.
- `sys_FreeResourcesOnMapChange` (`141353c70`) has compiled default `0`.
  Its description is "If true, it will free all resources loaded by map on
  map change". Other transition flags can also trigger the purge path;
  default zero is not a guarantee that every previous map resource survives.
- Wwise package selection (`1400bd510`) combines `common`, `init` and the
  current map, deduplicating selected package/bank entries. Shared source
  objects and encoded media have their own lifetime rules, described in
  `resource-parity.md`.

Thus resource reuse within the same running process is real, and shared
assets need not be read/decoded/uploaded again when still resident. This
does not retain the old level's live entity/game state, nor establish which
assets survive every forced purge. Persistent disk/driver cache policy is a
separate question and is not established by this evidence.

## Rewrite loading flow

`app::launch` creates the window, renderer and shared application services first.
`app::loading_screen` presents the installed startup background using the existing
resident-image owner. Once that window can render, the UI worker loads the selected
map's authored loading movie. The main-menu map and menu movie are deliberately
skipped until the save/menu flow exists.

`app::level_prepare` acquires the map on a worker and publishes typed resources to
the existing application. Map-dependent systems wait without changing their
ordering against Bevy transforms, visibility or uploads. The map Startup schedule
runs after publication; its scene construction still contains synchronous work.
The loading presentation remains up through this work and the asynchronous gates.
Scene/Bevy asset construction can still hold the last loading frame; its
incremental publication work is recorded in `optimization-opportunities.md`.
The render-side gate covers pending image uploads and pipeline preparation before
play; ordinary high-mip streaming continues afterward. The active loading state
blocks player input and pauses the gameplay clock; its movie advances on real
time. Pause preparation happens on that same UI worker behind its existing gate.

`app::loading_movie` implements `1400473b0` and `1400479a0`: it reads the installed
mapinfo's Iggy movie, layout, mission index/name/summary, image and tips. It calls
`_C_Show(layout)` and `_C_UpdateLoadingCompletion(acquired / requested)`. The movie
provides its authored layout and animations. `140021e60` initializes the
loading layout to 0, mission index to -1 and `m_waitInteractionAtEnd` to true.
Once resources are ready, the host calls `_C_LoadingCompleted` and forwards
keyboard/mouse events; the movie's `OnLoadingDisabled` ends the screen after
its authored close. An explicit false declaration skips that interaction. No simulated progress or timed end
is used. The manifest coverage report remains important: unsupported resource
classes are still partial and are not claimed to have acquired payloads.

Resource owners shared by the two stages retain their identities: the archive
catalog, parsed Iggy files and immutable ABC definitions on the UI worker,
compressed library images, game images, map manifests and parsed mapinfo declarations. GPU pipelines/device remain those of the
same renderer. Game-texture bindings from `IggyFunctions.setTextureForBitmap`
now reach the compositor rather than silently drawing the placeholder bitmap.
Live loading-movie state is released when play starts; it is separate from those
immutable library resources. Resource residency on forced purges and arbitrary
map-to-map transitions remains governed by the partial lifecycles in the ledger.

Production-client validation (dev profile, DirectX 12, 2026-11-03):

- Edge of the World and Dust District both pass bootstrap -> installed loading
  movie -> all resource/GPU gates -> movie continue/close -> gameplay -> clean
  scheduled exit. Captures were saved; their images were not inspected.
- Dust District's first nonempty loading draw list has stage 1920x1080,
  57 batches and 26,892 vertices, and binds the authored `loading_dust_is.png`.
  The movie reports no AS3 exception in that first publication.
- Four shared libraries (`lib_common`, controller buttons, particles and the
  Russian font library) return exactly the same file identities during loading
  and pause preparation. The map manifest is acquired once and reused three
  times by other consumers. Its immutable source descriptors remain shared.
- All 39 installed mapinfo declarations name the loading movie. The authored
  non-default layout values are precisely the no-tips and training values in
  the executable's `141a1bb60` enum table; explicit false wait flags are supported.
- Loading progress uses a cached count. No gameplay GPU pass was added. These
  are startup/ownership checks, not a claim of improved gameplay CPU timings.
- No synthetic or unit tests were written or run. Human appearance verification
  remains pending because image inspection is disabled by the user.

Local artifacts: `artifacts/loading-eotw-3.*`, `artifacts/loading-dust.*`,
`artifacts/loading-mapinfo-summary.json`, `artifacts/loading-build-10.*`.


Initial paused scene publication: `1408ad580` pauses simulation-clock stepping,
but a newly constructed world still needs initialized render inputs. Fog, water
and display publishers now create their first publication with delta zero even
when the clock is paused; later unchanged paused frames retain that publication.
This lets the existing particle-material gate prepare before play rather than
waiting for simulation to start. Simulation time and authored animations stay
paused until the loading movie's close callback.

Loading presentation correction (2026-11-03): merely binding a game image did
not replace the embedded placeholder bitmap's dimensions. `14101d290` stores
the resolved texture extent at wrapper +0x28/+0x2c, and `14103bb40` uses that
extent for bitmap bounds. The Flash runtime now obtains that extent from the
shared image owner before the movie lays out its background; drawing uses the
same dimensions and full UVs. The EotW image contains 2,073,600 opaque BC3 alpha
samples, so its missing background was not authored transparency. This adds no
gameplay pass or duplicate image upload.

Loading mouse input also needs the game's `IG_OnMouseEvent` route, as in the
existing pause integration: ordinary Flash mouse events alone do not reach the
loading movie's controller-event continuation listener. Both routes are now
forwarded. The host cursor is hidden while the loading movie is active.

The corrected production dev client acquired the 1920x1080 EotW background
through that callback. Its six draw vertices cover (0,0)..(1920,1080), with
UVs (0,0)..(1,1) and alpha 1. The diagnostic injected only a mouse down/up
through the regular loading input route after readiness: the authored close
callback fired, gameplay started, and the client exited normally. No AS3
exception was reported. Artifacts: `artifacts/loading-mouse.*`. This verifies
geometry and input behavior without viewing screenshots; human appearance
verification is still required.

## In-process world transitions

The host map-command entry is `app::request_level`; a `map` command sent through
the existing command state uses it. Resolution happens before disturbing the
current world, so an invalid level leaves gameplay running. Map acquisition
uses the existing catalog, window, renderer, settings, clock, UI worker and
audio output. The main menu and save restoration remain outside this entry.

`app::level_lifetime` separates world lifetime from resource lifetime, following
`1400410f0`: stop map producers, wait for the render world's acknowledgement of
the outgoing publication, despawn map entities, remove resources published by
map preparation/startup, and reset the plugins' mutable scene publications.
The same map startup schedule constructs the destination world. Readiness
gates are re-armed; a new loading movie uses the destination mapinfo and the
same completion/interaction path as the first load. Visibility scratch is
invalidated explicitly on world replacement, including an allocator reusing
the same buffer address on a same-map reload.

Audio has a narrower lifetime than a blanket session cache: `1400c93c0`
acquires startup banks into separate lists from `1400c93f0`'s map acquisition;
`1400c97e0` calls `1400c0360` / `1400c0620` to unload the latter banks/packages.
The host retains the common/init bank prefix and source media belonging to its
Sound IDs. Map banks, map-only source caches and old world playback/actions are
released. Package indices are shared by Arc rather than copied per bank. The
stereo output/bus clock remains alive. General resource-class forced purge and
unsupported resource families remain partial as recorded in the ledger.

The image owner retains the currently committed handles/mip states of images
explicitly listed by `init`/`common`. It captures them only at a world boundary;
the next setup binds those handles instead of creating another base chain.
Pending uploads are not transferred. Ordinary streaming then continues from
the committed mip state, including cold reductions. No per-frame cache lock,
lookup or new GPU pass is introduced. Map-specific image residency remains the
partial lifecycle in the ledger. Individual manifest owners also share the
immutable `init`/`common` lists between destination manifests.

Expected acquisition counts: a repeated map performs zero reads/parses of
already resident common/init banks; map-owned banks are reacquired after
unload. No second window/device/audio output should be created. The diagnostic
`DH2_DEBUG_LEVEL_SEQUENCE` sends semicolon-separated map requests through this
same production entry after three seconds of play per map, and exits after the
last map. `DH2_DEBUG_CONTINUE_LOADING=mouse` exercises movie continuation rather
than bypassing readiness. Completed-client transition measurements are recorded below.

The first completed three-map run (`artifacts/level-sequence-2.*`) exposed a
render-target lifetime bug: directional static-cache textures accumulated
3 -> 6 -> 9 (60 -> 121 -> 182 MiB) across EotW -> Dust -> EotW. These are
world-owned targets, not reusable shader programs. Render-side teardown now
releases shadow targets, map descriptors, mesh-allocation references and
prepared caster records before acknowledging the outgoing world; it retains
compiled pipelines, samplers and reusable upload capacity. The expected result
is three live directional static-cache textures for each loaded world, rather
than another three retained per load. This adds no gameplay GPU work.


Asset cleanup belongs to the persistent host, not the suspended world. Probe,
decal, sky and scheduled-audio asset plugins are registered before the map
system boundary, so their tracking/removal systems continue during acquisition.
The map boundary releases outgoing per-camera fog/display/exposure/particle
publications before destination construction. Shadow material association
caches store weak image IDs; the actual owners retain the handles. Shader
pipelines and reusable session buffers are retained.

Scene image identities are reserved before the chained startup consumers bind
them, corresponding to the executable resource registry's placeholder entries
(`1400607f0` / `14005feb0`). The existing image owner fulfils those identities.
Dependent materials retry preparation until their actual images are uploaded.
Common/init uploads complete before map uploads, following `1400661e0`'s
acquisition order. A generation-tagged render acknowledgement prevents a
previous phase or world satisfying the destination gate. The gameplay camera
and map producers activate only after both image phases finish. This is a
host upload mechanism; it adds no gameplay render pass.


Final same-map client validation (`artifacts/level-owner-run.*`, normal dev
profile, DirectX 12, 1280x720): four consecutive EotW loads complete their real
mouse continuation and close callback, then the client exits normally. All
four retain catalog/device/window identity and exactly one audio output. Each
reload reuses 194 common/init scene images and three common sound banks. The
live directional static cache is three textures / 60 MiB at every readiness
and settled publication, correcting the earlier 3/6/9 accumulation.

At the settled publications, GPU allocated MiB are 2753 / 2761 / 2757 / 2763;
reserved MiB are 3492 / 3620 / 3748 / 3620. The pre-correction same-map run
reserved 3364 / 4516 / 4644 / 5156 MiB with essentially the same live payload.
Thus the fourth-load reservation is 1536 MiB lower (29.8%), without changing
texture precision or the allocator's performance configuration. Each load
publishes the same 4343 active scene instances; screenshots were not compared.
The rough 1 GiB prediction underestimated allocator block granularity:
the old fourth load holds 23 blocks, the corrected load 18, while both hold
2763 MiB live. The measured gain is therefore five released backing blocks
(1536 MiB), not fewer live scene resources. The corrected fourth reservation
is also below its third load, rather than growing monotonically. Private process
memory at final sequence completion is 7048 MiB; this is not a claim that all
CPU heaps or general resource-class retention now match the original.
No tests were written/run and no screenshots were inspected.


Different-map validation (`artifacts/level-owner-transition.*`): EotW -> Dust
District -> EotW completes all three movies/gates and exits normally at 71.697s.
The same PID 20708, window 65v0, catalog/device identities and single audio
output persist throughout. Active scene counts are 4343 / 382 / 4343, matching
the earlier pre-staging transition run. Settled GPU allocation/reservation MiB
are 2756/3492, 2778/3524 and 2786/3492; destination-specific resources replace
the previous world without reservation growth. Shared scene-image captures are
194 then 184 (202 distinct common/init images retained across these maps).
No panic, error or AS3 exception was reported. A separate diagnostic request
using the invalid alias `dust_p` was rejected without disturbing the running
world; the following valid request still loaded normally.

The final cache review also made instance-to-image slot scratch reset on a
newly added scene resource, alongside visibility/decal/alpha-depth scratch.
This prevents an allocator reusing the same instance-vector address from
mistaking a replacement world for appended instances.


Loading handoff follow-up (user report): the current synchronous setup and
base-image reads block the window thread, and pipeline idleness can precede
a successful final gameplay composition. Expected change: move scene payload
cooking and image reads to their existing acquisition worker, reserve identities
through the live asset/entity allocators, and publish the completed payloads on
the window thread. This removes the measured multi-second construction/read
stalls from window updates without changing assets or adding gameplay passes.
The readiness gate must observe a successful final scene composition, and the
loading close must capture the cursor rather than show it. Fade timing still
requires the executable/installed world parameters, not an arbitrary delay.


Follow-up implementation: scene cooking and base-image reads now run inside the
existing acquisition worker. Prepared assets reserve handles through the live
owner's `AssetHandleProvider`; commands reserve entities through its remote
allocator. Both publish into the same world/stores, with the same setup body
also used by synchronous diagnostic callers. The window thread keeps its asset stores throughout: the acquisition worker
holds payloads plus reserved handles, not borrowed main-world assets or a second
application. The payload-writing interface is shared with existing resource
readers; GPU uploads still happen through the canonical owners.

The final display owner reports readiness after a successful same-view
atmosphere/camera/lens chain; the image owner waits for the first view's actual
read/upload/residency work. These replace the incorrect assumption that an
idle pipeline queue means the gameplay image is complete. Camera input is
captured and the cursor remains hidden when the authored close callback fires.

Disassembly at `1408aac09` / `1408aac1f` selects world +0x3b8/+0x3c8 using
initial-load flag +0x337134 bit 0; `1408aac49` starts `140166f60` at priority 3.
`140166e40` advances seconds and `140167170` evaluates the packed curves and
clamps RGBA. EotW's installed definitions specify 4s initially and 2s later,
with opacity staying at 1 until normalized time 0.2969281673. The black cover
exists before gameplay can render and stays beneath the loading movie, so its
removal cannot expose an uninitialized world. It then follows the installed
fade rather than a fabricated linear animation. Other fade priorities and save
restoration remain outside the direct-map loader.


The previous client trace (`level-owner-run.err`) measured roughly 6s of scene
construction plus 4.6–5.7s to reach base-image readiness on the window thread.
Listed sound preparation alone contributed about 0.5s. Its existing registry,
mix and shared engine now transfer to the acquisition worker while their
suspended ECS slots retain identity, then return during publication. The live
output callback keeps the same shared engine backing. No bank/media clone or
new output is needed on transitions. Runtime transport helpers use generic
asset writers so ordinary per-frame paths can still dispatch statically.


Host GPU acquisition uses Bevy's existing soft image upload limiter while the
loading presentation is active, with the configured
`r_textureStreamingPerFrameBudgetKB` value. A single oversized asset still
progresses. The previous host policy is restored before play: this does not
change gameplay texture budgets or add a render pass. Preparing all images on
the worker is insufficient if their backend creation monopolizes one render
frame; the limiter keeps that work progressing across loading frames.


The first bounded-upload client run exposed an additional readiness hole:
shadow mesh uploads were still pending after the image-only upload gate.
`SceneGpuInputs` now carries the actual renderable mesh/buffer identities from
the prepared payload batch; map activation waits for those GPU owners too.
The existing forward pipeline readiness signal and directional/local submission
completion are also loading gates. The benchmark's 30-frame warmup is not a
loading rule and is not copied: a completed publication is sufficient.
This run completed the installed fade, but its 798ms maximum loading update gap
and transient missing-RenderMesh errors are intermediate results, not acceptance.


The local-light caster precomputation and directional target constructors now
run on that same worker through their existing constructors, before the payload
batch is published. Startup wrappers retain the synchronous diagnostic entry
and skip already prepared owners. The GPU input gate includes those prepared
attachment images as well as renderable meshes and shader buffers. This removes
the remaining measured shadow-construction wait from the window thread, without
changing queries, light selection, target sizes or image precision.
