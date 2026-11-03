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

## Rewrite implications

The current desktop launcher starts a new game process (`launcher::ops::start`).
`app::launch` acquires substantial level data before constructing the Bevy app.
Neither behavior provides the original's continuous menu-to-level lifecycle.
The existing `loading` module already records resource reads and readiness
gates, but the resource ledger still marks manifest-driven acquisition and
many unload policies partial.

A faithful loading flow needs an early visible bootstrap, a persistent menu/
game runtime with reusable resource owners, and a separate map transition
that constructs/restores the destination world behind the loading UI. Its
completion must cover initialization/GPU readiness as well as file reads.
This research does not claim those screens are implemented yet.
