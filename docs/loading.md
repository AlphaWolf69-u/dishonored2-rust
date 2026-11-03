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
