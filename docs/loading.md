# Loading stages

Installed fade coverage audit: all 36 indexed world entity resources contain
explicit initial/later duration fields. Their references use three installed
fade declarations. `fade_black_loop` omits curve counts and changes only the
constructor's opacity value; reset `14000f740` (called by `14000f610`) supplies
one t=0 cubic key for both RGB and opacity, initialized to zero. Initializing
empty vectors instead rejects this valid partial edit. The fade owner now
ports those constructor defaults before applying installed edits. The ordinary
start-level fade still resolves its explicit three-key opacity curve unchanged.

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


Dust transition rejection (diagnostic `loading-handoff-transition`): the worker
moved setup ahead of `initialize_screen_shadow_source`, so setup had already
transferred `SceneData.catalog` before that initializer ran. The initializer
reported `screen catalog is unavailable`; without its depth source, projected
decal variants failed to compile (`prepass_depth` absent) and forward readiness
correctly remained incomplete. The screen-shadow owner now prepares through
its same helper on the worker, before setup transfers the catalog. All loading
cameras are explicitly UI-only. Acquisition errors return to the loading
coordinator rather than silently omitting this required owner. This repairs the
initialization order; it does not bypass readiness or clear failed pipelines.

The camera readiness predicate observes the camera owner's same-frame completion,
not presence of its optional DoF half-colour target. With DoF disabled that
target correctly does not exist. Explicitly disabled post-processing similarly
does not require a camera/lens submission, while actual fog/depth and forward
submission gates remain mandatory. This follows the existing settings branches;
it does not force an effect on to make loading finish.

The repaired production-client sequence (`loading-handoff-transition-fixed`,
dev DX12) completed EotW -> Dust -> EotW with zero shader errors. All three
loads retained PID 20812, catalog 0x21438ddee60, device 0x21477270530, window
65v0 and one audio output. Every handoff logged hidden/locked cursor state and
restoration of the pre-loading upload policy. The movie-visible maximum update
gaps were 848.540 / 954.235 / 903.863 ms: the deadlock is repaired, but this is
not acceptance of fully responsive loading. Exact host publication/startup
spans are being used to locate that remaining scheduling stall.

Exact Dust host spans (`loading-publication-dust`, same dev client): asset
publication 3.549 ms, entity publication 2.605 ms, remaining Startup 6.028 ms.
Thus the 955.603 ms maximum is not scene publication or vegetation Startup.
It occurs after GPU activation; the existing per-phase frame profiler is being
sampled at loading gaps to separate backend preparation from handoff waits.

The follow-up phase trace (`loading-phases-dust`) confirms two separate host
stalls: 721.466 ms in RenderSystems::PrepareAssets and 638.575 ms in the first
transparent draw stage. The longest window gap was 948.535 ms. Worker cooking,
entity publication and remaining Startup are not the cause. The Dust load still
completed normally. Further responsiveness work must address backend asset
preparation and first-draw work rather than weaken readiness or insert a delay.

Bounded acquisition verification (`loading-budget-*`, same dev DX12 binary,
1280x720, Dust District, no concurrent compilation):

| Scope | Diagnostic unbounded | Loading budget enabled |
|---|---:|---:|
| Longest mesh allocation/upload call | 691.519 ms | 87.525 ms |
| Longest particle acquisition/draw call | 594.127 ms | 30.141 ms |
| Longest window-update gap | 933.782 ms | 352.997 ms |
| Loading movie closed, since process start | 23.957 s | 26.371 s |

Both acquired 8647 meshes, 3970 image assets and 104 shader buffers; both
reported 8 CPU-backed meshes, 3 MiB image payloads and 29 MiB shader-buffer
payloads at readiness. Both logged hidden/locked cursor handoff and completed
the installed 4-second fade with no shader errors. This establishes improved
loading scheduling, not release walking/FPS or a claim that every hitch is gone.
The bounded path trades about 2.4 seconds of total loading time in this dev run
for smaller work batches. The logged mesh spans above 10 ms sum to 905.506 ms
versus 691.519 ms before; those truncated sums are not total function costs.

With `r_skipDOF=1`, loading closed at 26.004 s and the fade completed 4.003 s
later, with no DoF constructor submissions or shader errors. The initial
`no-post` case did not exercise `r_skipPostProcess`: that unarchived cvar is
protected in retail mode. A corrected developer-mode capture
(`loading-no-post-developer`) logged the explicit lens bypass, closed loading
at 24.715 s and completed the fade at 28.712 s. Neither optional pass is forced
on to satisfy readiness.

Remaining stalls are backend work, including image creation (177.695 ms), a
large shader-buffer preparation (112.942 ms), and first-view bind groups
(106.919 ms). The worst bounded frame includes several such phases; it is not
one remaining 350 ms image operation. Further work must keep ownership and
readiness intact and avoid conflating per-function costs with whole-frame gaps.

Release comparison (`loading-release-*`, same binary, DX12 1280x720; all three
processes exit 0): longest mesh preparation 590.223 -> 110.076 ms; particle
preparation 506.849 -> 26.527 ms; longest movie-visible update gap
704.718 -> 242.154 ms. Loading closes at 19.095 / 20.055 s respectively, then
the installed initial fade completes in 4.003 s. These are loading costs;
they do not establish walking CPU or FPS changes. No shader errors occurred.

The release Dust -> EotW -> Dust sequence retains PID 19592, catalog
0x1ed708fee20, device 0x1ed25a75f80, window 65v0 and one audio output. Visible
loading gaps are 281.607 / 180.974 / 186.485 ms; both later-load fades complete
in about 2 s. Outgoing-world release still causes 1036.942 / 929.513 ms gaps
before the destination movie is visible. Diagnostic spans now separate entity
destruction, each removed resource and scene-publication reset so the responsible
owner can be corrected without moving unrelated work or weakening readiness.

The follow-up disproved destruction as the one-second cause: entity destruction
takes 1.374 / 2.746 ms and the full world-clear function 86.754 / 96.303 ms.
`audio::load_level_contacts` then reconstructed its `FromWorld` resource on the
window thread, reacquiring the destination manifest/declarations. That same
contact owner now acquires on the existing audio worker, and its baseline slot
is empty while map producers are suspended. Startup diagnostic callers use the
same constructor and skip an already acquired owner.

`loading-contact-transitions` exits 0 with contact acquisition
837.816 / 854.449 / 865.939 ms on the worker. The transition interval to
"outgoing world released" falls from 1077 / 935 ms to 117 / 130 ms, while
world clear remains 86.675 / 95.979 ms. All three loading handoffs complete.
`loading-contact-tutorial` also exits 0 and admits the installed constant-black
fade with one opacity key and three packed values. That declaration deliberately
stays black until cinematic/FX code replaces it; the loader does not fabricate
a fade-out for it.

A first tutorial run records a 3112.160 ms gap before scene publication, during
bulk sound preparation. A repeated phase capture does not reproduce the long
gap, but shows the frame waiting 116.757 ms with only 4.328 ms of render work.
Bulk sound preparation occupies the same ComputeTaskPool used by frame
scheduling. A same-binary AsyncComputeTaskPool comparison did not substantiate
the starvation hypothesis: both prepared 1437 events, but background decoding
took 333.146 vs 166.434 ms, and 537.429 vs 224.821 ms with four configured
workers. Maximum window gaps did not improve (186.055 vs 157.687 ms;
176.322 vs 176.629 ms). The pool change and switch were removed. The original
three-second observation remains unexplained; repeated tutorial captures no
longer reproduce it. No claim is made that that cold-loading stall is fixed.

Normal (non-benchmark) release input capture (`loading-input-handoff.json`):
after the movie's mouse continuation, Windows reports the owned window in the
foreground, hidden cursor (`flags=0`) and a one-pixel clip rectangle inside
the client bounds. No additional gameplay click is sent. The loading interval's
maximum gap is 210.226 ms in that run. Settings are isolated from the user's
home for this observation; no screenshots or player navigation are used.

The next acquisition-pool comparison keeps the frame pool's worker count in a
temporary Bevy TaskPool owned by the audio preparation call. This addresses
blocking media work occupying frame-executor workers without the smaller async
pool's throughput loss. Workers finish and release before further scene cooking;
playback and ordinary frame work keep their existing scheduling. The original
pool remains available through the diagnostic switch for same-binary comparison.
Expected warm decode cost stays near the original, plus worker creation; cold
decode cannot queue ahead of frame tasks in the same executor. Validation pending.

Same-binary validation (`loading-acquisition-pool-*`, dev client, tutorial):
all four runs exit 0 and prepare the same 1437 events. Existing/isolated-pool
decode costs are 183.753 / 171.071 ms; with four configured workers they are
207.478 / 209.431 ms. Thus equal-capacity isolation avoids the async pool's
throughput loss. Overall window maxima are GPU-dominated and do not improve
(169.073 / 184.436 and 186.219 / 217.816 ms); these are not evidence of faster
rendering. The cold three-second episode still does not reproduce in these
runs. The scheduling invariant is that bulk acquisition cannot fill the frame
executor's queue: only the loading call owns and waits for its temporary workers.
The final visual responsiveness assessment still requires the user because
visual inspection is disabled.

Requirement audit before user verification:

| Requirement | Current evidence | Still needed |
|---|---|---|
| Responsive loading | CPU scene/image/audio/contact acquisition runs off the window thread; backend mesh/particle batches yield; bulk audio has separate equally sized workers. Transition interval falls from about 1 s to 117–130 ms. Same-binary decoding counts/costs are preserved. | User observation of the final release; the isolated cold three-second observation has not been reproduced for a direct before/after comparison. |
| Immediate camera input | Normal non-benchmark release handoff: Windows foreground belongs to the client, cursor flags are 0 and the clip rectangle is one pixel. No second gameplay click is issued. | User confirmation of camera response after their loading continuation. |
| Hide incomplete rendering | Continuation requires actual initial image residency, prepared image/mesh/buffer identities, forward/shadow submission and same-frame atmosphere/camera/lens composition; explicit disabled effects use their completion branches. Repeated-map clients finish without shader errors. | User confirmation that no incomplete world flashes on entry. |
| Authored fade from black | Installed declarations and world durations, original constructor defaults and packed curves; initial fade completes at about 4 s and later fades at about 2 s in production clients. | User confirmation of the displayed transition. |

This audit is not a completion claim. No image inspection or test suite is used.

On 2026-11-04 the user checked the final release in Dust District and confirmed
"All four work": loading responsiveness, immediate camera control, complete
world presentation on entry and the fade from black. This resolves the pending
appearance checks in the historical audit above. The implementation is merged
into main; the queued main release build remains the final delivery check.
