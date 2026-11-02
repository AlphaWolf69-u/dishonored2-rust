# Resource parity ledger

What the rewrite loads of each resource class the original loads, and how
far each class's lifecycle is ported. Rules: `AGENTS.md`, "Resource parity".
Status words: `ported`, `verified` (against a capture or the original's
output), `blocked: <reason, next step>`, `host: <why the backend differs>`,
`partial`, `none`.

## Classes

### Resource manager (all classes)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400661e0` (`ResourceManager.cpp`) | loads every entry of the `init`, `common` and map `.mapResources` lists by (kind, name) before play; `+0xfe24` = loaded/total | partial: `assets::map_resources` reads the lists, `crate::loading` records reads and gates; the rewrite's loads are not driven by the lists yet |
| `FUN_140064720`, `FUN_140064820` | per-container registration of a map's list | blocked: vfunc `+0xb0` (map name to container id) not traced |
| "%i streamed" entries | some listed resources are registered for streaming rather than loaded | blocked: trace which entries count as streamed |

### Image (`idImage`)

| Original | Behavior | Status |
|---|---|---|
| `1402ea140` LoadResourceInternal | reads the `.bimage` and uploads its base mip chain through `1402e9ff0` at load | ported: setup initializes every scene image's base chain |
| `1402e9ff0` | the base-chain GPU upload | blocked: not traced |
| `FUN_14030e840` arkTextureStreamer | streams `<image>.bimage%d_mip%d` during play | partial: `app::texture_streaming` uses its own request and residency rules, not the original's |
| `r_textureStreamingPerFrameBudgetKB` | upload budget per frame (4096 KB) | ported |
| `listImages` | the original's image listing (sizes, classes) | capture pending |

### Sound (Wwise)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400bd480` arkWwiseBankLoader::LoadBanksAndPackages | loads the `common`, `init` and map packages (`FUN_1400bd510`; loose banks `FUN_1400bca10`) | blocked: the rewrite indexes every installed bank; per-map package loading not ported |
| Wwise codecs | media stay encoded; Vorbis and IMA ADPCM decode per voice; Vorbis packets are decoded as installed | ported: `assets::wwise::VorbisCursor`, `AdpcmCursor`; installed Wwise Vorbis packets decoded in place through vendored `lewton` (window flags from the neighbouring packets' modes), with the decoder setup rebuilt once per distinct Wwise setup. Verified: all 132,518 installed Vorbis media give the samples of their ww2ogg conversion |
| `141122400` | a source references its Sound's media rather than copying it | ported: prepared media shared by Wwise sound id |
| stream manager (stream type != 0 sources) | streamed sources (type 2, no bank prefix) are read from their package while they play; prefetched sources (type 1) start at once from a bank prefix while the rest streams | partial: type 2 format is read at load and media on the IO pool when an event plays it, held by that playback and released after; a request waits for its read, as the original starts a non-prefetch stream once its first buffer arrives. `approximate_`: type 1 media (eotw: 13, 1 MB; ~4 KB prefixes) stay resident whole, which starts them at once as the prefix does |
| listed `soundevent` decls | loaded with the map | ported: prepared at startup |

### Material, model, animation, physics, FX, Iggy, entities

Lifecycle not traced. Each owner below reads its resources at startup, but
its load, upload and unload path has not been compared with the original.
Next step per class: find its `LoadResourceInternal` vtable entry (as for
`idImage`) and port it.

## Kinds

Every kind in the installed load lists has a row (`loading::tests`). A
trailing `*` covers a family of kinds.

| Kind | Owner | Status |
|---|---|---|
| `Achievement` | none | none: no owner yet |
| `achievementManager` | none | none: no owner yet |
| `activeRagdoll` | none | none: no owner yet |
| `anim` | `assets::edge_animation` | partial: reads its resources; lifecycle not ported (see its class) |
| `animBasic` | `app::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `animSet` | `app::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `animTree` | none | none: no owner yet |
| `areaTagCollection` | none | none: no owner yet |
| `arkCloth` | none | none: no owner yet |
| `arkTree` | none | none: no owner yet |
| `baseModel` | `assets::bmd6`, `assets::entities` | partial: reads its resources; lifecycle not ported (see its class) |
| `blueprint` | `assets::entities`, `app::view_geometry` | partial: reads its resources; lifecycle not ported (see its class) |
| `boneCharmCollection` | none | none: no owner yet |
| `boneCharmLoadout` | none | none: no owner yet |
| `climb` | none | none: no owner yet |
| `contactSystem` | none | none: no owner yet |
| `ContactSystemDB` | `app::audio::contacts` | partial: reads its resources; lifecycle not ported (see its class) |
| `curvefloat` | `rendering::fog::settings`, `app::resident_images` | partial: reads its resources; lifecycle not ported (see its class) |
| `damage` | none | none: no owner yet |
| `decal` | `assets::decal` | partial: reads its resources; lifecycle not ported (see its class) |
| `DevGui_Menu` | none | none: no owner yet |
| `difficultyManager` | `gameplay::options::gameplay` | partial: reads its resources; lifecycle not ported (see its class) |
| `difficultyPreset` | `gameplay::options::gameplay` | partial: reads its resources; lifecycle not ported (see its class) |
| `drop` | none | none: no owner yet |
| `entityDef` | `assets::entities` | partial: reads its resources; lifecycle not ported (see its class) |
| `env` | `assets::entities`, `assets::environment` | partial: reads its resources; lifecycle not ported (see its class) |
| `EnvProbe` | `assets::bprobe`, `assets::texture` (probe network) | partial: the per-vertex probe references become the GPU vertex-probe buffer at setup and are then released (no main-world copy of either); load path not compared with the original |
| `equipmentUpgrade` | none | none: no owner yet |
| `explosion` | none | none: no owner yet |
| `fade` | none | none: no owner yet |
| `File` | `assets::level`, `assets::world_mesh`, `visibility::tome` | partial: world (BWM) vertex data and the shared shadow source meshes are released after setup's uploads, as the original keeps no CPU copy of uploaded world geometry (F8 re-reads the world on demand); Tome blob and decoded views stay resident |
| `Font` | `flash::text` | partial: reads its resources; lifecycle not ported (see its class) |
| `footStepPhysMat` | none | none: no owner yet |
| `fx` | `rendering::particles::world_sources`, `app::audio::contacts` | partial: reads its resources; lifecycle not ported (see its class) |
| `fxactor` | `assets::entities::effects::actors` | partial: reads its resources; lifecycle not ported (see its class) |
| `fxanimset` | none | none: no owner yet |
| `gameflowManager` | none | none: no owner yet |
| `gameLogicManager` | none | none: no owner yet |
| `gamesoundproperties` | none | none: no owner yet |
| `gameVideoClip` | none | none: no owner yet |
| `greatestMomentsManager` | none | none: no owner yet |
| `iggyFile` | `flash`, `gameplay::pause_state` | partial: reads its resources; lifecycle not ported (see its class) |
| `image` | `app::texture_streaming`, `assets::texture` (image class) | partial: reads its resources; lifecycle not ported (see its class) |
| `inventoryitem` | none | none: no owner yet |
| `inventoryItemCollection` | none | none: no owner yet |
| `inventoryLoadout` | none | none: no owner yet |
| `jointInfo` | none | none: no owner yet |
| `jumpProfile` | none | none: no owner yet |
| `kiscule` | `assets::entities::effects::declarations` | partial: reads its resources; lifecycle not ported (see its class) |
| `layer` | none | none: no owner yet |
| `loco` | none | none: no owner yet |
| `mapInfo` | `gameplay::map_info` | partial: reads its resources; lifecycle not ported (see its class) |
| `massprops` | `assets::physworld::mass`, `assets::entities::machinery` | partial: reads its resources; lifecycle not ported (see its class) |
| `material` | `app::material_loading` (material class) | partial: reads its resources; lifecycle not ported (see its class) |
| `md6Def` | `assets::bmd6`, `app::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `midnightScene` | `assets::entities::scene_motion`, `app::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `model` | `assets::bmodel`, `app::setup` (model class) | partial: shared world/model surfaces upload normal and tangent as SNORM8x4 and colour as UNORM8x4, as installed (bitangent derived in the vertex shader); UVs stay f32 (Bevy has no half-float vertex values); world shadow programs read the shared mesh's own position, NORMAL and COLOR (axis swap undone in the shader), so only vegetation (forward wind) and water meshes keep game-space copies; sheared baked instances keep float vertices. Lifecycle not ported (see its class) |
| `musicManager` | none | none: no owner yet |
| `navigation` | none | none: no owner yet |
| `Navmesh` | none | none: no owner yet |
| `navmeshLayerSettings` | none | none: no owner yet |
| `navmeshMapSettings` | none | none: no owner yet |
| `NPCVariationDef` | none | none: no owner yet |
| `particledt` | `rendering::particles::definition` | partial: reads its resources; lifecycle not ported (see its class) |
| `pauseMenuSectionOptions` | `gameplay::options` | partial: reads its resources; lifecycle not ported (see its class) |
| `pblock` | `rendering::render_program` | partial: reads its resources; lifecycle not ported (see its class) |
| `physhape` | `assets::physworld` | partial: reads its resources; lifecycle not ported (see its class) |
| `physicsMaterial` | `assets::entities::physics_material`, `app::audio::contacts` | partial: reads its resources; lifecycle not ported (see its class) |
| `physSystem` | none | none: no owner yet |
| `physworld` | `assets::physworld`, `gameplay::body_storage` | partial: reads its resources; lifecycle not ported (see its class) |
| `playerLocomotion` | `assets::entities::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `playerMode` | `assets::entities::player` | partial: reads its resources; lifecycle not ported (see its class) |
| `playerStatisticsManager` | none | none: no owner yet |
| `proceduralLookAt` | none | none: no owner yet |
| `ragdoll` | `assets::bmd6`, `assets::entities::machinery` | partial: reads its resources; lifecycle not ported (see its class) |
| `ratLocomotion` | none | none: no owner yet |
| `renderParm` | `assets::environment`, `app::material_loading` | partial: reads its resources; lifecycle not ported (see its class) |
| `renderProg` | `assets::shader_cache` | partial: reads its resources; lifecycle not ported (see its class) |
| `ruleHandler` | none | none: no owner yet |
| `skeleton` | `assets::bmd6` | partial: reads its resources; lifecycle not ported (see its class) |
| `soundauxbus` | `gameplay::sound::auxiliary` (sound class) | partial: reads its resources; lifecycle not ported (see its class) |
| `soundengineinternals` | `app::audio` (sound class) | partial: reads its resources; lifecycle not ported (see its class) |
| `soundevent` | `app::audio`, `assets::wwise` (sound class) | partial: reads its resources; lifecycle not ported (see its class) |
| `soundrtpc` | `app::audio` (sound class) | partial: reads its resources; lifecycle not ported (see its class) |
| `speechBarks` | none | none: no owner yet |
| `speechCharacter` | none | none: no owner yet |
| `speechScene` | none | none: no owner yet |
| `speechVoice` | none | none: no owner yet |
| `staticParticleModel` | `assets::particle_model`, `assets::fx::particle_mesh` | partial: reads its resources; lifecycle not ported (see its class) |
| `strIdFormating` | none | none: no owner yet |
| `swarmGlobalConfig` | none | none: no owner yet |
| `tutorial` | none | none: no owner yet |
| `UIPauseStatisticEntry` | none | none: no owner yet |
| `vault` | `gameplay::player::decls` | partial: reads its resources; lifecycle not ported (see its class) |
| `vaultClimb` | none | none: no owner yet |
| `vaultDrop` | none | none: no owner yet |
| `cpnt*` | `assets::entities` (components of placed entities) | partial: a few component types read (e.g. `cpntHealthEvolved`); most none |
| `UI_HUDElement*` | none | none: no HUD |
| `playerState*` | none | none: player states not ported as decls |
| `gameLogicState*` | `gameplay::pause_state` (pause only) | partial: pause state only |
| `mainMenuSection*` | none | none: no main menu |
| `menuSectionOption*` | `gameplay::options` | partial |
