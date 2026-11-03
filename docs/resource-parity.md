# Resource parity ledger

What the rewrite loads of each resource class the original loads, and how
far each class's lifecycle is ported. Rules: `AGENTS.md`, "Resource parity".
Status words: `ported`, `verified` (against a capture or the original's
output), `blocked: <reason, next step>`, `host: <why the backend differs>`,
`partial`, `none`.

The cross-family ownership audit, including intentional performance-oriented
differences, current release scopes and unresolved streaming gaps, is in
[`resource-lifetimes.md`](resource-lifetimes.md). Original allocation budgets
remain the baseline; the user also authorizes improvements that preserve
behavior and improve performance/memory rather than copying inefficiencies.

## Classes

### Resource manager (all classes)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400661e0` (`ResourceManager.cpp`) | loads every entry of the `init`, `common` and map `.mapResources` lists by (kind, name) before play; `+0xfe24` = loaded/total | partial: `assets::map_resources` reads the lists, `crate::loading` records reads, actual manifest progress and readiness gates; `app::loading_screen` presents bootstrap/map stages in one renderer; the rewrite's loads are not driven by the lists yet |
| `FUN_140064720`, `FUN_140064820` | per-container registration of a map's list | blocked: vfunc `+0xb0` (map name to container id) not traced |
| "%i streamed" entries (`140066ac0`-`140066b8c`) | the map preloader skips index flag `0x20` and `.sbcm`/`.tbcm`/`.bwm` records | partial: classification traced in installed instructions; `140068800` reads the index flags into record+`0x4a0`. In installed `game1_001.index`, 6,952 flagged records are image mips (6,476), Iggy (350) and generic files (126). This does not imply that every ordinary placed mesh is spatially streamed. Typed rewrite owners request these payloads themselves; unified registry-driven loading remains partial |

### Image (`idImage`)

| Original | Behavior | Status |
|---|---|---|
| `1402ea140` LoadResourceInternal | reads the `.bimage` and uploads its base mip chain through `1402e9ff0` at load | ported: setup initializes every scene image's base chain |
| `1400607f0` / `14005feb0` acquired image lookup | returns the resident resource identity | partial: `texture_streaming::SharedImages` retains the current committed GPU handle/mip state for explicitly listed init/common images at a map boundary; destination setup binds that identity without reading/uploading another base chain. No gameplay cache work is added. UI handles remain with `resident_images::UiImages`. General map image retention/purge and original streamer eviction remain partial |
| `1402e9ff0` | the base-chain GPU upload | host: existing Bevy image owner uploads reserved image identities; loading waits for generation-tagged GPU readiness, with common/init before map images. Custom material/sky/audio asset tracking remains active while map systems are suspended; dependent materials retry until images are ready. Original API-level upload remains untraced |
| `FUN_14030e840` arkTextureStreamer | streams `<image>.bimage%d_mip%d` during play | partial: `app::texture_streaming` uses its own request and residency rules, not the original's |
| `r_textureStreamingPerFrameBudgetKB` | upload budget per frame (4096 KB) | ported |
| `listImages` | the original's image listing (sizes, classes) | capture pending |
| `1400607f0` / `14005feb0`, `1402ea140` | particle image acquisition independent of materials/draw state | host: `rendering::particles::image_resources::ParticleImages` shares compiled 2D/volume chains by resolved path and dimension during level acquisition. `bevy_backend::particle_images::ParticleImagesGpu` owns one GPU texture per acquired identity; sampler/state variants only bind it. Both release with level/pass ownership. Production release A/B: 102 -> 62 CPU image acquisitions, 22,200,416 -> 16,134,008 acquired bytes; 190 -> 59 GPU uploads, 43,785,808 -> 16,046,296 uploaded bytes. Image acquisition/upload CPU 37.240 -> 11.323 ms (repeat 12.003 ms). EotW -> Dust -> EotW releases each outgoing pass and reacquires 59/54/59 unique textures, with zero client errors. Cross-family scene/UI/particle identity unification and particle finer-mip streaming remain partial |

### Sound (Wwise)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400bd480` arkWwiseBankLoader::LoadBanksAndPackages | loads the `common`, `init` and map packages (`FUN_1400bd510`; loose banks `FUN_1400bca10`) | ported: `app::audio::level_banks` reads `wwisedescription.txt` (`FUN_1400c0cb0`, path `FUN_1400c1480`) and loads the `common`, `init` and current-map entries' packages (prefix + language / `sfx.pck`, plus `main-init.pck`) and banks (FNV-1 ids, verified on `main-common-sfx.pck`). Loose-bank mode (`FUN_1400bca10`) not ported |
| Wwise codecs | media stay encoded; Vorbis and IMA ADPCM decode per voice; Vorbis packets are decoded as installed | ported: `assets::wwise::VorbisCursor`, `AdpcmCursor`; installed Wwise Vorbis packets decoded in place through vendored `lewton` (window flags from the neighbouring packets' modes), with the decoder setup rebuilt once per distinct Wwise setup. Verified: all 132,518 installed Vorbis media give the samples of their ww2ogg conversion |
| `141122400`, `1411284e0`, `1410f1910` | a source references its loaded Sound/bank media rather than copying it | ported: prepared media shared by Wwise sound id; owned bank DATA ranges share their encoded backing through ADPCM/Vorbis preparation and playback. Borrowed diagnostic banks copy because they do not own their source lifetime. Parallel preparation publishes one cached source identity. `host:` Arc-backed byte ranges retain their bank until its last user releases it; complete map unload remains partial |
| stream manager (stream type != 0 sources) | streamed sources (type 2, no bank prefix) are read from their package while they play; prefetched sources (type 1) start at once from a bank prefix while the rest streams | partial: type 2 format is read at load and media on the IO pool when an event plays it, held by that playback and released after; a request waits for its read, as the original starts a non-prefetch stream once its first buffer arrives. `approximate_`: type 1 media (eotw: 13, 1 MB; ~4 KB prefixes) stay resident whole, which starts them at once as the prefix does |
| listed `soundevent` decls | loaded with the map | ported: prepared at startup |
| `1400c93c0`, `1400c93f0`, `1400c97e0` -> `1400c0360` / `1400c0620` | startup and map bank/package acquisitions have separate unload lists | partial: common/init banks and their media stay resident; map banks, map-only cached media and world playback/actions release on transition. Host: one Arc package index per path, a stable startup-bank prefix and one persistent audio output; verified in repeated-map client acquisition (three retained startup bank identities). Prepared event graphs are rebuilt against the current bank registry rather than retaining references to unloaded map actors |

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
| `decal` | `assets::decal` | partial: typed `arkFXDecalDesc` acquisition (`143ae3730`/`217d50`), shared declaration references and ordinary FX chain playback (`222f80`/`2191f0`/`2191d0`/`218980`); installed projection-program defaults/overrides and the FX opacity image are resolved by the existing material owner; host: shared-geometry receiver submission, borrowed skin palettes and depth-box reconstruction are connected; typed acquisition covers all 25 installed decal declarations and 99 material variants; a D3D12 capture verifies the animated depth-box draw changes scene RGB pixels. WORLD receiver admission reuses the `298ee0` cursor/material rules. Clip-model bounds use fragment clipping in place of D3D clip-distance exports. Skin-decal FX remains a separate unsupported family. Renderer residency/unload lifecycle remains partial. |
| `DevGui_Menu` | none | none: no owner yet |
| `difficultyManager` | `gameplay::options::gameplay` | partial: reads its resources; lifecycle not ported (see its class) |
| `difficultyPreset` | `gameplay::options::gameplay` | partial: reads its resources; lifecycle not ported (see its class) |
| `drop` | none | none: no owner yet |
| `entityDef` | `assets::entities` | partial: reads its resources; lifecycle not ported (see its class) |
| `env` | `assets::entities`, `assets::environment` | partial: reads its resources; lifecycle not ported (see its class) |
| `EnvProbe` | `assets::bprobe`, `assets::texture` (probe network) | partial: the per-vertex probe references become the GPU vertex-probe buffer at setup and are then released (no main-world copy of either); load path not compared with the original |
| `equipmentUpgrade` | none | none: no owner yet |
| `explosion` | none | none: no owner yet |
| `fade` | `assets::fx::fade`, `app::loading_fade` | partial: world-authored load colour/opacity curves and durations; runtime sampling follows `140166f60` / `140166e40` / `140167170`. Host display UI composition; priority/listener/script arbitration remains outside the direct-map loader |
| `File` | `assets::level`, `assets::world_mesh`, `visibility::tome` | partial: world (BWM) vertex data and the shared shadow source meshes are released after setup's uploads, as the original keeps no CPU copy of uploaded world geometry (F8 re-reads the world on demand); host: one lazily prepared immutable `295fa0` WORLD bounds tree is shared by shadows and projected decals (three consumers: 13,864,920 -> 4,622,536 requested bytes, identical installed EotW query output); the Tome blob stays resident and its cell tree is descended in place (axis codes, planes, implicit children and leaf ranks from a 64-node rank directory, bounds split from the root), as the original reads it; the remaining decoded Tome tables (links, cell codes, object data) are still copies |
| `Font` | `assets::iggy::font`, `flash::text` | partial: metadata reads do not decode glyphs; host outline caches decode only requested glyphs over shared library bytes (original provider `14102bd60` / metric reader `14102ba20` reads serialized tables). Glyph pages use R8_UNORM as `14107b7e0` format 6; atlas sizing follows `14002c440` movie-stage caps and the high-resolution `bnet` exception using already loaded metadata; full lifecycle not ported |
| `footStepPhysMat` | none | none: no owner yet |
| `fx` | `rendering::particles::world_sources`, `app::audio::contacts` | partial: reads its resources; lifecycle not ported (see its class) |
| `fxactor` | `assets::entities::effects::actors` | partial: reads its resources; lifecycle not ported (see its class) |
| `fxanimset` | none | none: no owner yet |
| `gameflowManager` | none | none: no owner yet |
| `gameLogicManager` | none | none: no owner yet |
| `gamesoundproperties` | none | none: no owner yet |
| `gameVideoClip` | none | none: no owner yet |
| `greatestMomentsManager` | none | none: no owner yet |
| `iggyFile` | `flash`, `gameplay::pause_state` | partial: reads its resources; `flash::bitmap_resources` owns immutable library bitmap payloads, shared across menu instances. `1402e2c00` uploads format 13 as BC7_UNORM directly; gameplay now retains/uploads those blocks rather than expanding to RGBA. GPU bitmap/glyph textures release on movie close (`m_cacheResources = 0`); old unique glyph IDs no longer accumulate across opens. Host fallback: non-block-aligned base extents retain the existing cropped decode because wgpu requires block-aligned BC extents. Only mip 0 is connected, as before; original mip-chain and CPU library-cache unload remain partial |
| `image` | `app::texture_streaming`, `assets::texture` (image class) | partial: reads its resources; lifecycle not ported (see its class) |
| `inventoryitem` | none | none: no owner yet |
| `inventoryItemCollection` | none | none: no owner yet |
| `inventoryLoadout` | none | none: no owner yet |
| `jointInfo` | none | none: no owner yet |
| `jumpProfile` | none | none: no owner yet |
| `kiscule` | `assets::entities::effects::declarations` | partial: level and declaration graph definitions shared by Arc; runtime variables, gate values, owner overrides and queues stay instance-local (`7bc1d0`/`7bb9d0`). Host ownership optimization; full lifecycle not ported |
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
| `particledt` | `rendering::particles::definition::ParticleResources` | partial: one level-scoped host acquisition cache for declarations, PMODELs, field images and material references across player/world FX; emitters retain independent mutable simulation. Full original resource lifecycle not ported |
| `pauseMenuSectionOptions` | `gameplay::options` | partial: reads its resources; lifecycle not ported (see its class) |
| `pblock` | `rendering::render_program` | partial: reads its resources; lifecycle not ported (see its class) |
| `physhape` | `assets::physworld` | partial: reads its resources; lifecycle not ported (see its class) |
| `physicsMaterial` | `assets::entities::physics_material`, `app::audio::contacts` | partial: reads its resources; lifecycle not ported (see its class) |
| `physSystem` | none | none: no owner yet |
| `physworld` | `assets::physworld`, `gameplay::body_storage`, `gameplay::navigation` | partial: reads its resources; lifecycle not ported (see its class). Host navigation broad phase keeps decoded full-precision triangles for query speed; immutable cell lists pack u32 shape keys into one allocation, retaining their order and releasing construction capacity. This replaces the Havok compressed-mesh tree (`141262020`); no per-query mesh decoding or reduced precision |
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

### Loading presentation and shared UI acquisitions

Closed movie runtime: `flash::avm2::{Vm,heap::Heap,unit::Unit}` owns per-player
objects, classes, methods and decoded bodies. Original `1410298b0` tears down
the VM through `1410629a0` / `141086460` and releases its allocator; ordinary
Rust `Rc` drops did not release function/prototype/constructor and unit cycles.
Host: weak per-VM allocation tracking, explicit object/class edge teardown and
unit cache release on movie close. Immutable Iggy/ABC/library images keep their
existing shared owners. Verified normal release client EotW -> Dust -> EotW:
cleanup disabled leaves 3,722 / 3,723 / 3,722 tracked movie objects alive;
cleanup enabled leaves zero after each close (2.697 / 2.930 / 2.849 ms on the
UI worker). No gameplay render work is added. This is movie teardown, not a
claim that live-movie cyclic garbage collection has been implemented.

| Original | Owner / behavior | Status |
|---|---|---|
| `140369120` | `app::loading_screen`: installed startup image before map publication | host: Bevy image composition in the existing gameplay window |
| `1400410f0`, `1408a96e0` | `app::level_lifetime`, `app::loading_screen`: outgoing world teardown and destination acquisition | host: suspend map producers and wait for render acknowledgement, drop map entities/resources, reset scene publications, rerun map startup in the same application. Re-arm all loading gates and movie interaction. Shared catalog/UI/image owners, renderer, clocks, settings and audio output persist. General resource-class purge remains partial; verified host integration: four same-map loads and EotW -> Dust -> EotW retain one window/device/output; exact counts and memory in docs/loading.md |
| world/render target teardown | `bevy_backend::shadow_pass`: outgoing shadow target and descriptor ownership | host: map-boundary release clears directional/local target references, prepared caster records, alpha material views and mesh/resource bindings, while preserving compiled pipelines/samplers and upload capacity. Before correction, actual GPU diagnostics showed 3/6/9 static-cache textures after three map loads; verified: three textures / 60 MiB on all four repeated-map loads |
| `1400473b0`, `1400479a0` | `app::loading_movie`: installed mapinfo data and authored Iggy layout/callbacks | partial: mission/map text, image, shuffled tips, actual resource progress, authored continue/close callback connected; cinematic and saved-transition variants remain outside this direct-map flow |
| `1400607f0` -> `14005feb0` | `flash::bitmap_resources`: shared immutable compressed library images; `app::resident_images`: shared game-image handles; `gameplay::map_info`: shared immutable parsed declarations | host: Arc registry lifetime; acquisition returns the resident identity, movie state remains separate; forced purge classes remain partial |
| `1410150f0`, `14101d290`, `14103bb40` | `flash::iggy_functions` / `flash::render` / `bevy_backend::flash_ui`: named game image bound to Bitmap | partial: binding obtains the resident image's actual dimensions before movie layout; bounds and drawing use those dimensions and full UVs, replacing the embedded placeholder rectangle. The compositor uses the image owner's existing GPU texture. Host: worker-to-main request/reply transports the synchronous texture callback without blocking the window thread. Forced image purge remains with image lifecycle above |

Expected cost of this change: `level_prepare::prepare_level` moves its existing
parsing off the event/render thread, without changing the parsers or geometry.
The common UI library payload is acquired once across loading/pause players;
reacquisition should perform zero archive reads/decodes and reuse Arc identity.
No gameplay GPU pass is added: the loading UI disappears when its gates finish.
No RAM saving or gameplay CPU timing improvement is claimed without targeted
owner measurements. Existing original per-tag budgets remain the baseline.

Manifest reuse expectation: `assets::map_resources::acquire_level_list` returns
one immutable request list to loading progress, audio contacts, listed sound
preparation and diagnostics. After the first request, archive-index scans and
manifest decoding for that same map/language should be zero; those callers
must receive the same Arc identity. This is startup work, not a walking-frame
optimization. Individual resource payload acquisition remains with its owner.

Iggy definition reuse expectation: the shared parsed Iggy file also owns one
immutable ABC syntax tree. Loading and pause VMs retain their own namespaces,
classes, globals and mutable execution state; `Vm::load_shared_abc` should not
parse that source again. This removes repeated source-definition parsing and
retained duplicate ABC trees while preserving per-player VM state. Measure
acquisition/parse counts and identities during the actual startup handoff;
this does not claim whole-frame CPU savings.

The Iggy resource owner (`assets::iggy::acquire`) retains parsed file relocations
and the immutable ABC source tree. Per-player `flash::avm2::Unit` keeps its own
namespace pools, classes, globals and execution state. `141093380` remains the
file reader; the resident identity follows `1400607f0` / `14005feb0`. Host: Rc
ownership is confined to the UI worker; forced resource-class purge remains
partial. Named game textures are acquired/uploaded by `app::resident_images`
and bound by `bevy_backend::flash_ui`; the UI does not decode/upload another copy.

Loading progress expectation: original `1400661e0` publishes a cached loaded/total
value. The host read recorder should update the acquired count when a new read
arrives; `loading::progress` must perform no manifest traversal. This replaces
46,546 membership lookups per progress sample on EotW with a cached count, while
retaining exact (kind,path) comparisons for hash collisions and duplicate list
entries. This affects loading CPU only, with no gameplay sampling cost.

2026-11-03 production-client verification: EotW and Dust District complete both
loading stages and enter gameplay. Loading/pause preparation reuses the same
four common Iggy file identities; the selected map manifest is acquired once
and reused three times. Dust District publishes the authored background in a
57-batch, 26,892-vertex loading draw list. No first-publication AS3 exception is
reported. These verify acquisition identity and loading integration, not full
map-to-map purge/residency parity or gameplay CPU timing. See `docs/loading.md`
for scope, captures and the remaining human appearance check.


### Responsive loading and load fade

Contact-sound declaration admission (`1400661e0` listed resources) stays in
`app::audio::contacts`. Its map-scoped owner now acquires through the existing
audio preparation worker on initial load and transitions. Plugin construction
creates an unacquired slot, publication supplies the resolved state before map
systems run, and unload clears it without synchronously rereading declarations.
The synchronous Startup entry remains for diagnostic callers and skips an
already acquired state. Resource paths, lookup caches and error handling are
unchanged; this is host thread scheduling. The production client verifies
837–866 ms of acquisition on the worker and 117–130 ms transition intervals,
with successful initial/return map handoffs and one persistent audio output.
Bulk event decoding retains its existing task pool: an alternative background
pool doubled decoding cost without improving observed loading gaps, so that
experiment was reverted. The independent contact-acquisition correction remains.
A subsequent host comparison isolates only acquisition in a temporary Bevy
TaskPool with the frame pool's existing worker count; synchronous diagnostics
and post-publication playback keep their previous executor. This preserves
parallelism while preventing cold media tasks from occupying frame workers.
Same-binary client validation prepares 1437 events in both paths: equal-capacity
isolation takes 171.071 vs 183.753 ms, and 209.431 vs 207.478 ms with four workers.
The pool is released after acquisition and adds no persistent resource cache
or audio output. No reduction of the remaining GPU-dominated window gaps is claimed.

Fade constructor/reset (`14000f610` -> `14000f740`, curve constructors
`14000d610` / `14000d0b0`) starts RGB and opacity with a single t=0 cubic key.
Installed partial edits retain those arrays when `num` is omitted. The loader
now applies those defaults before edits; empty arrays incorrectly rejected
`fade_black_loop`, referenced by tutorial and end maps. All 36 installed worlds
explicitly provide both load-fade durations and reference three fade declarations.
End/tutorial script-driven replacement of a constant-black fade remains part of
their cinematic/FX lifecycle, not an invented timer in the loading controller.

The composition gate uses the camera owner's same-frame completion flag;
optional DoF scratch-image residency is not a completion requirement when that
effect is disabled. Explicit `r_skipPostProcess` bypasses only the disabled
camera/lens chain, not atmosphere/depth or forward/shadow readiness. This
settings-path correction is verified in the production client with DoF disabled
and with developer-only post-processing bypass; both enter play and complete
the installed fade without shader errors (docs/loading.md).

Host responsiveness follow-up: backend extraction defers whole assets under
the existing loading byte budget before mesh allocation; Modified/Unused events
remain owned by the extractor. Shader-buffer sizes participate in that same
budget. Particle material acquisition yields after one newly compiled state
variant while loading, retaining the same material owner and completeness gate.
Gameplay upload policy, payloads, shader states and drawing are unchanged.
Original image acquisition remains `1402ea140` / `1402e9ff0`; the host's
extraction/compilation scheduling is not claimed as original engine code.
Production-client same-binary A/B validates unchanged mesh/image/buffer counts
and reduces the maximum mesh call from 691.519 to 87.525 ms and particle call
from 594.127 to 30.141 ms. Remaining backend stalls are documented in
docs/loading.md; full responsiveness is not yet claimed.

| Original | Owner / behavior | Status |
|---|---|---|
| `1400661e0`, `1400607f0` / `14005feb0` | resource acquisition with pre-registered identities | host: `bevy_backend::asset_preparation` prepares CPU payloads using the existing asset handle providers; `app::setup_commands` reserves entities with the live world's remote allocator. `app::scene_preparation` runs the existing setup implementation on the acquisition worker and transfers payloads/commands once. Canonical asset stores, shared resource caches and GPU uploads stay in their existing owners. Scene base-image bytes are read by the same texture owner on that worker; common/map GPU upload phases remain separate. Audio owner data moves to that worker while map systems are suspended, retaining its resource identities/output and shared-bank backing; the same bank/event preparation helpers serve synchronous diagnostic callers. Screen-shadow/depth preparation runs before setup transfers the catalog. GPU attachment/mesh/buffer readiness precedes camera activation; forward and shadow submissions precede continuation. EotW -> Dust -> EotW completes without shader errors; remaining subsecond loading stalls are under investigation (docs/loading.md) |
| renderer-complete handoff | `bevy_backend::display_scene`, `app::texture_streaming` | host: initial gameplay readiness requires successful atmosphere/camera/lens composition and initial visible image residency, in addition to pipeline/upload gates. No timer substitutes for missing rendering |
| `1408aac09` / `1408aac1f` / `1408aac49`, `140166f60`, `140166e40`, `140167170` | `assets::fx::fade`, `app::loading_fade` | partial: read world initial/later-load declarations and durations, parse colour/opacity curves using the existing partial-edit array readers, advance/sample the fade after loading. Host: black loading cover is a persistent UI node below the authored Flash presentation, then fades into gameplay. Native priority/listener arbitration and script-triggered fades remain separate. Direct first launch maps to the initial-load variant; later host map commands map to the later-load variant. No save-game mode is inferred |
