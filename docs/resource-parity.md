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
| `FUN_140064720`, `FUN_140064820` | per-container registration of a map's list | host: lookup is `140064590` (vtable `141acc328` + `0xb0`), which normalizes the manifest path and searches registered container name tables; `assets::map_resources::load_list` resolves the manifest's owning archive, then interprets ids against that archive's index. Installed input audit: 38 manifest paths, no cross-container duplicates. Redundant index rescans are separate acquisition work, not an unresolved virtual call |
| "%i streamed" entries (`140066ac0`-`140066b8c`) | the map preloader skips index flag `0x20` and `.sbcm`/`.tbcm`/`.bwm` records | partial: classification traced in installed instructions; `140068800` reads the index flags into record+`0x4a0`. In installed `game1_001.index`, 6,952 flagged records are image mips (6,476), Iggy (350) and generic files (126). This does not imply that every ordinary placed mesh is spatially streamed. Typed rewrite owners request these payloads themselves; unified registry-driven loading remains partial |

### Image (`idImage`)

| Original | Behavior | Status |
|---|---|---|
| `1402ea140` LoadResourceInternal | reads the `.bimage` and uploads its base mip chain through `1402e9ff0` at load | ported: setup initializes every scene image's base chain |
| `1400607f0` / `14005feb0` acquired image lookup | returns the resident resource identity | host: `texture_streaming::SharedImages` captures committed init/common images on map teardown and transfers matching handles/states into destination setup without a base read/upload. Transfer removes the old session-held handle, preventing it from pinning a superseded active texture. Inactive common images remain charged and eligible for mailbox-acknowledged pressure reduction. Scene and particle 2D identities share the streamer. UI, fog, lens, area-light and cloud/depth-default full-mip consumers share the session ResidentImages owner; original 14005feb0/140060180 lookup/purge boundaries are implemented with Bevy handles and explicit movie leases. Equivalent initial base views now transfer from the registered owner; named UI bindings acquire committed scene/common views and retain snapshot leases. Different source-mip views remain distinct to preserve the scene minimum policy |
| `1402e9ff0` | the base-chain GPU upload | host: existing Bevy image owner uploads reserved image identities; loading waits for generation-tagged GPU readiness, with common/init before map images. Custom material/sky/audio asset tracking remains active while map systems are suspended; dependent materials retry until images are ready. Original upload dispatch is traced: 1402f5500 selects 1402f50c0 (2D/cube UpdateSubresource1 or Map/copy/Unmap) or 1402f52f0 (3D row/slice pitches); 1402f4060 finalizes descriptors and releases replaced views/resources. Host Bevy/wgpu preparation and compressed queue uploads replace those D3D11 calls |
| `FUN_14030e840` arkTextureStreamer | streams `<image>.bimage%d_mip%d` during play | host: scene images use the original pressure gate, expired/minimum and density/one-level reductions (`14030f3d0`, `14030d7a0`, `14030d8d0`), sampling-role priorities (`1402d4e20`), static/dynamic mode and path/age keys. Pending reservations count against finer-mip residency; completed reads/uploads stay until pressure requires eviction. Common images transfer between active/map-cache ownership, with inactive pressure using the same GPU mailbox. Stable host tie-breaking and padded-block byte accounting differ explicitly. Default/constrained-budget walking and three-map execution are recorded below. Particle 2D demand uses this same streamer; named UI acquisition now reuses committed scene/common views without forcing full mips; compatible registered base views avoid scene reacquisition |
| `r_textureStreamingPerFrameBudgetKB` | upload budget per frame (4096 KB) | ported |
| `listImages` / readonly resource registry | the original's named image residency (sizes, classes, views) | bootstrap captured: 643 resource registries / 23,534 registered entries, including 1,548 named images. Readonly full-Steam capture validates image list `1432e9ff0`, registry `+38/+40`, scope `+24`, and current image/view descriptors. Map capture requires a loaded EotW view; allocator totals alone do not supply this data. The earlier loading request became stale when that process was closed. A fresh full-Steam process was verified running on 2026-11-04 before requesting the level again |
| `1400607f0` / `14005feb0`, `1402ea140` | particle image acquisition independent of materials/draw state | host: `rendering::particles::image_resources::ParticleImages` shares compiled 2D/volume chains by resolved path and dimension during level acquisition. `bevy_backend::particle_images::ParticleImagesGpu` owns one GPU texture per acquired identity; sampler/state variants only bind it. Both release with level/pass ownership. Production release A/B: 102 -> 62 CPU image acquisitions, 22,200,416 -> 16,134,008 acquired bytes; 190 -> 59 GPU uploads, 43,785,808 -> 16,046,296 uploaded bytes. Image acquisition/upload CPU 37.240 -> 11.323 ms (repeat 12.003 ms). EotW -> Dust -> EotW releases each outgoing pass and reacquires 59/54/59 unique textures, with zero client errors. Particle 2D identities/demand now use the scene owner (3dd8a11); remaining volumes retain complete single-mip authored data. Equivalent registered scene-base views and current named-UI snapshots now share handles; intentionally different mip views remain independent |

### Sound (Wwise)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400bd480` arkWwiseBankLoader::LoadBanksAndPackages | loads the `common`, `init` and map packages (`FUN_1400bd510`; loose banks `FUN_1400bca10`) | ported: `app::audio::level_banks` reads `wwisedescription.txt` (`FUN_1400c0cb0`, path `FUN_1400c1480`) and loads the `common`, `init` and current-map entries' packages (prefix + language / `sfx.pck`, plus `main-init.pck`) and banks (FNV-1 ids, verified on `main-common-sfx.pck`). Loose-bank mode (`FUN_1400bca10`) not ported |
| Wwise codecs | media stay encoded; Vorbis and IMA ADPCM decode per voice; Vorbis packets are decoded as installed | ported: `assets::wwise::VorbisCursor`, `AdpcmCursor`; installed Wwise Vorbis packets decoded in place through vendored `lewton` (window flags from the neighbouring packets' modes), with the decoder setup rebuilt once per distinct Wwise setup. Verified: all 132,518 installed Vorbis media give the samples of their ww2ogg conversion |
| `141122400`, `1411284e0`, `1410f1910` | a source references its loaded Sound/bank media rather than copying it | ported: prepared media shared by Wwise sound id; owned bank DATA ranges share their encoded backing through ADPCM/Vorbis preparation and playback. Borrowed diagnostic banks copy because they do not own their source lifetime. Parallel preparation publishes one cached source identity. `host:` Arc-backed byte ranges retain their bank until its last user releases it; complete map unload remains partial |
| stream manager (stream type != 0 sources) | streamed sources (type 2, no bank prefix) are read from their package while they play; prefetched sources (type 1) start at once from a bank prefix while the rest streams | host: required format metadata is prepared for graph construction; selected voices stream installed Vorbis/IMA packets through a shared 2 MiB, 16 KiB-granularity reader. Type 1 borrows its authored bank prefix and streams continuation. Packed seek tables stay packed; metadata/setup and decoder workspaces are separate from that device budget. Three-map default playback, the final walking PCM/CPU comparison and installed byte-prefix comparisons are recorded below. The legacy full-reader path is diagnostic-only |
| listed `soundevent` decls | loaded with the map | ported: prepared at startup |
| `1400c93c0`, `1400c93f0`, `1400c97e0` -> `1400c0360` / `1400c0620` | startup and map bank/package acquisitions have separate unload lists | partial: common/init banks and their media stay resident; map banks, map-only cached media and world playback/actions release on transition. Host: one Arc package index per path, a stable startup-bank prefix and one persistent audio output; verified in repeated-map client acquisition (three retained startup bank identities). Prepared event graphs are rebuilt against the current bank registry rather than retaining references to unloaded map actors |

### Material, model, animation, physics, FX, Iggy, entities

The per-kind rows and subsequent acceptance records distinguish traced owners
from remaining gaps. In particular, model loading (`14025aa00`/`140259960`),
animation blob ownership (`14024adf0`), Iggy image upload (`1402e2c00`) and the
resource lookup/purge boundary (`1400607f0`/`140060180`) have source evidence;
this does not certify every declaration subtype or unconsumed system. Generic
rows below remain incomplete until their own lifecycle has been traced.

## Kinds

Every kind in the installed load lists has a row (`loading::tests`). A
trailing `*` covers a family of kinds.

| Kind | Owner | Status |
|---|---|---|
| `Achievement` | none | none: no owner yet |
| `achievementManager` | none | none: no owner yet |
| `activeRagdoll` | none | none: no owner yet |
| `anim` | `assets::edge_animation::{EdgeAnimation,residency}` | host: immutable compressed resource identity; requested init/common clips persist, map-only entries are weak and prune after actor release. Original lookup/purge `1400607f0`/`140060180`, blob loader `14024adf0`; sampling reads the same compressed bytes. Three-map production acquisition 600 -> 276, with identical fingerprints for all 600 requests and zero outgoing map leases. Per-character playback state stays separate; no GPU upload belongs to the clip itself |
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
| `File` | `assets::level`, `assets::world_mesh`, `visibility::tome` | partial: world (BWM) vertex data and the shared shadow source meshes are released after setup's uploads, as the original keeps no CPU copy of uploaded world geometry (F8 re-reads the world on demand); host: one lazily prepared immutable `295fa0` WORLD bounds tree is shared by shadows and projected decals (three consumers: 13,864,920 -> 4,622,536 requested bytes, identical installed EotW query output); the Tome blob stays resident and its cell tree is descended in place (axis codes, planes, implicit children and leaf ranks from a 64-node rank directory, bounds split from the root), as the original reads it; the decoded traversal/object tables intentionally retain flat runtime query data. Historical source a34a947 measured the associated query package at 826 -> 682 CPU ms/s and 3.05 -> 2.30 ms median query span; this is a host performance representation, not abandoned parse staging. The source blob and typed query data share map ownership and release together |
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
| `iggyFile` | `flash`, `gameplay::pause_state` | partial: reads its resources; `flash::bitmap_resources` owns immutable library bitmap payloads, shared across menu instances. `1402e2c00` uploads format 13 as BC7_UNORM directly; gameplay now retains/uploads those blocks rather than expanding to RGBA. GPU bitmap/glyph textures release on movie close (`m_cacheResources = 0`); old unique glyph IDs no longer accumulate across opens. Host fallback: non-block-aligned base extents retain the existing cropped decode because wgpu requires block-aligned BC extents. Original `1402e2c00` mip-chain acquisition/upload is connected; its sampler filters follow `1402dfd40`. Host: immutable CPU library bitmaps remain a shared session cache to avoid repeated reads/decoding; movie GPU images release separately. Three-map acquisition used 34,556,544 CPU bitmap bytes against the retained 86,057,717-byte IGGY tag baseline (which includes additional UI owners) |
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

### Active implementation: original lifecycle completion

- Sound source release: `141122400` releases an old bank-backed source before
  assigning new media; `1410f1910` reuses the registered Sound object. The host
  now prunes unused Vorbis header/codebook cache entries after map voices and
  prepared media are released, preserving identities still held by live/common
  sources. Same-build retention control: `DH2_DEBUG_RETAIN_SOUND_HEADERS`.
- Map request descriptors: `1400661e0` releases its assembled request table
  after acquisition. Combined rewrite manifests now use weak cache entries;
  active consumers retain one identity. Init/common source manifests remain
  available without rescanning archives; map request tables and progress indices
  release after their acquisition/report consumers finish. Control: `DH2_DEBUG_RETAIN_LEVEL_MANIFESTS`.
- World teardown: `1400410f0` ends world ownership independently of resource
  class purging. The backend now also drops outgoing screen-shadow bindings,
  area-light image slot handles, projected-decal skin/resource bindings and
  direct-draw bindings. Fixed viewport/upload capacity stays reusable.
- Live Iggy collection: installed `1410b0040` jumps to `141062a90`; the rewrite
  previously returned without collecting. The original marks roots through
  `1410642f0`/`141064430`, traces in `141063ed0`, and sweeps through `141063530`.
  `flash::avm2::heap` now traces object references and shared capture scopes,
  preserves VM/host/stack roots, and breaks unreachable cycles during a live
  movie. `System.gc` invokes it; movie safepoints use host growth-relative
  scheduling instead of Iggy's allocator-specific incremental budget. No
  collector runs on the ordinary gameplay update path when no movie ticks.
  Control: `DH2_DEBUG_RETAIN_UI_GARBAGE`.

Production-client validation and measured effects for these changes are
recorded below. Unused setup eviction yielded no saving in the sampled maps
because those setups remained live. Combined descriptors release after use;
closed VMs release all tracked runtime objects. No steady-state frame-time
claim is assigned to unload operations.

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
| `1410150f0`, `14101d290`, `14103bb40` | `flash::iggy_functions` / `flash::render` / `bevy_backend::flash_ui`: named game image bound to Bitmap | host: binding obtains the resident image's dimensions before layout; bounds/drawing use those dimensions and full UVs. Display objects and submitted draw lists own shared opaque binding leases (14101d160); publishing forwards those handles instead of reacquiring names. Map purge drops non-common UI registry entries while actual consumers retain their leases; init/common identities survive. Worker-to-main request/reply transports the synchronous callback without blocking the window thread. Cross-domain image identity remains incomplete |

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

### Acquisition descriptors: completion and measurement

Expected gain: release each map's parsed source request list after assembly,
then release the combined list and its progress hash/chains after the loading
presentation closes. Keep init/common source metadata for session reuse, and
keep registered resource payloads in their actual owners. This ports the
request-table release in `1400661e0`; it does not purge the resource registry.
Diagnostic coverage must relinquish its temporary strong reference after its
single report. EotW's combined table alone accounts for 6,878,036 bytes; retained
copies and source lists should disappear during gameplay, with no per-frame
work. Measure descriptor identities/bytes, not process-wide FPS.

Production dev-client validation of the preceding unload changes completed
EotW -> Dust District -> EotW with both retention controls and corrections,
exit code 0 in both runs. Disabling outgoing world cameras before render
cleanup is required: standard Bevy mesh phases otherwise outlive publication
and access released bindings. The replacement loading camera keeps the loading
presentation active while the outgoing renderer retires.

Weak event listeners now follow `141083aa0`'s useWeakReference flag and
`141065f20`'s broadcast root rule. `141064cd0` clears dead weak handles and
`141086220` prunes their records. The host uses weak object references and a
tracing collector rather than Iggy's allocator. A three-map production dev run
completed with exit code 0; each closed UI VM reported zero remaining runtime
objects. Live collections visited 3,527/3,528/3,527 objects and freed zero:
these loading movies' live heaps are not evidence of a live-garbage memory
saving. System.gc correctness and unreachable-cycle handling are distinct
from the already measured movie-close leak correction. Full collector pauses
were 1.652/1.649/1.649 ms in dev, once per movie in that sequence; release cost
and long-running menu allocation behavior remain to be measured.

Dictionary weak-key lifetime is also implemented. Original `141044820`
recognizes Dictionary's class tag 0x21 and exactly one true argument
(`DAT_142424049`), creates its weak dynamic table, and sets its weak-table
field. `141063ed0` marks the values of that table while deferring object keys;
`141064cd0` removes entries whose keys were not marked. The host collector
counts the physical key references without making them roots, preserves the
original strong value references (including value-to-key cycles), and removes
dead-key entries at collection. Primitive keys remain ordinary keys. Expected
gain is release of unreachable object keys/values from long-running UI caches;
loading movies need not contain such garbage, so their zero collected count
is not a savings claim.

### Stream format acquisition

Original `1411606d0` takes source format/frame count from RIFF metadata and
`141149320` handles the pre-data loop record. Source playback uses the Wwise
stream device, whose game initialization `1400bc8f0` overrides the defaults in
`1410bf740`: 2 MiB IO memory, 16 KiB granularity, blocking scheduler, cached
reads, and 100 ms target automatic buffering (installed DAT_141ac8ffc).

Expected acquisition correction: read the RIFF header/chunks through the data
chunk header, retain no audio bytes at map acquisition, and derive the exact
same channel/rate/frame/loop fields without constructing a full Vorbis packet
index. This removes whole-file reads and transient buffers from the type-2
format-acquisition path. Unsupported format layouts still acquire their full
payload through the existing decoder; no guessed frame counts. Resident and
prefetched sources preserve their current decoding. Same-build control:
`DH2_DEBUG_EAGER_STREAM_ACQUISITION=1`. Record actual acquired header bytes vs
referenced source bytes and acquisition CPU/read cost. Earlier EotW logs list
2,070 type-2 source files totaling 422 MiB; repeated source references can make
actual eliminated IO larger. This does not yet change whole-file playback IO
or implement the original bounded playback buffering; those remain required.

### Glyph page residency and updates

Original `14107b7e0` creates one R8 page when player+0x468 is empty;
`14107b2d0` uploads only a bordered glyph rectangle through GDraw+0xa8.
`14107b980` stamps the current draw counter on a cache hit. At capacity,
`14107a980` scans fitting shelves and reuses entries not used this draw whose
reserved width is in [requestedWidth, requestedWidth+6); it keeps slots used
by the current draw and falls back to vector drawing if no slot qualifies.
The rewrite instead cleared the entire page at capacity and recreated/uploaded
the whole GPU texture for every page version.

The host now keeps draw ages and physical slots, applies that reuse rule, and
keeps the GPU page identity while updating dirty rectangles. Dirty bounds are
unioned only within a draw; a renderer that skipped a version uploads the full
page into the same identity. There is no unbounded update journal. Direct movie
replacement also releases the outgoing player's bitmap/glyph textures, even
when no empty UI frame separates the two players.

Expected gain: GPU page creations fall from one per page revision to one per
page; updated bytes follow changed glyph rectangles. Capacity pressure no
longer invalidates every cached glyph or overwrites glyphs used by the same
draw. Normal glyph hits add the original draw-age stamp; no extra GPU pass or
shader/output change. Same-build page recreation control:
`DH2_DEBUG_RECREATE_GLYPH_PAGES=1`. Production diagnostics record uploaded bytes,
page creation/update counts and CPU time of that path; targeted release
measurement is required before assigning a CPU improvement.

### Animation resource ownership

Original `14024adf0` replaces/releases the previous resource buffer at +0x48,
then allocates only the authored Edge blob (+0x54 size rule) under
TAG_ANIMATION. Registry lookup `1400607f0`/`14005feb0` gives consumers that
resource identity; actor playback clocks/poses are independent. The retained
original EotW tag baseline is 195,174,788 bytes, including more animation
systems than the rewrite currently implements.

The player loader deduplicated by rig alias, so multiple aliases resolving to
one resource and different body/arm rigs could still independently acquire,
parse and own identical clip bytes. The resource owner now shares immutable
Edge definitions by installation/resource path through Arc, with weak registry
entries. Pose sampling takes the same references and does not perform registry
lookups, locks or reference-count operations. Outgoing world teardown prunes
expired weak identities; startup/player clocks remain separate.

Expected gain: clip reads, retained blobs/tables and acquisition CPU follow
unique resources rather than aliases. If installed aliases do not duplicate a
resource, this change has no demonstrated residency gain and must not be kept
as an optimization on assumption alone. Same-build control:
`DH2_DEBUG_DUPLICATE_ANIMATIONS=1`. Production diagnostics report requests,
reads, live unique payloads, payload bytes and acquisition time.

### Stream classification in acquisition progress

`140068800` reads the resource-index flags into record+0x4a0. The preloader
`140066ac0`/`140066b8c` classifies flag 0x20 and paths containing .sbcm, .tbcm or
.bwm as streamed instead of ordinary preload requests. The archive owner now
preserves that flags word; loading-progress indexing applies the same rule.
Typed resource owners and complete coverage reports still see the full list.
This changes classification/progress bookkeeping, not resource decoding or
payload readiness gates. It does not claim a completed registry-driven loader.

Stream acquisition release measurement (`lifecycles-stream-release-*`): both
modes completed with exit 0 and no fatal client/GPU errors. At 2,492 requests,
logical package bytes read fell from 530,033,360 to 232,514; cumulative read/
acquisition CPU time fell from 383.678 to 189.805 ms. The separately timed
initial event preparation fell from 285.880 to 157.283 ms; the later 33-event
batch fell from 12.108 to 2.679 ms. These are acquisition measurements, not
walking CPU or whole-frame improvements.

Comparing all 2,381 prepared source formats found 118 rate differences and no
channel/frame/loop differences. Investigation identified an existing baseline
cache bug: WWISE_HEADERS keyed identification headers only by codebook bytes,
channels and block sizes, incorrectly reusing a 48 kHz identity for 44.1 kHz
files. `1411606d0` reads each source's sample rate from fmt+4. Header keys now
include that rate; decoded setup/codebook sharing remains independent of rate.
A fresh comparison after that correction is required; the old baseline's
wrong rate must not be reproduced as supposed parity.

A further world-release defect was found in ScreenEffects: its cached passes
owned material textures and depth/HDR bindings, but the cache was reused only
by viewport configuration and survived `clear_frame`. A same-size destination
world could therefore reuse outgoing material images. World unload now drops
that material/pass owner, not merely its completed-frame flag. The original
world ownership boundary is `1400410f0`; device-level shader services remain
separate. This is correctness as well as residency, with no ordinary-gameplay
update work added.

### Authored Iggy bitmap mip chains

Original `1402e2c00` takes the raw entry's mip count from byte+5, builds one
subresource record for each authored mip, advances by ceil(width/blockEdge) *
ceil(height/blockEdge) * blockBytes, and creates the texture with that count.
The bitmap owner currently copied/uploaded mip 0 only despite retaining the
entry's authored count. Correct acquisition must preserve the whole chain;
block-aligned BC7 entries stay compressed, while the existing wgpu-only
nonaligned fallback decodes each actual level with its proper extent.

Expected fidelity change: minified UI bitmaps use their authored mip chain.
Expected cost is exactly the extra authored levels, not an RGBA expansion of
BC7 assets. Counts/bytes are reported against the retained IGGY tag budget
(86,057,717 CPU bytes); no lower-memory claim is made for restoring missing
levels. `DH2_DEBUG_UI_SINGLE_MIP=1` reproduces the previous incomplete upload
for owner accounting. No new shader or GPU pass is needed.
Native bitmap sampling was traced with the mip-chain fix: `1402dfd40` creates
D3D sampler filter 0x15 for smooth fills and 0x10 otherwise. The host now uses
linear minification for both, with linear magnification/mips for smooth fills
and point magnification/mips for nonsmooth fills. Previously mip filtering was
always point and nonsmooth minification was also point. Addresses/repeat modes
and glyph rasterization remain separate from this authored sampling rule.

Animation result: the production release loaded 200 clips from 200 distinct
resources (1,133,582 payload/table bytes); the proposed shared-definition owner
removed no acquisition in that case. It was reverted, including the Arc wrapper
and extra registry, rather than keeping unmeasured complexity. Existing body/
arm owners retain compressed clips and release them with their map; CPU skin
sampling and GPU-only mesh upload remain separate required representations.

Sustained pause collection revealed a performance defect in the first host
collector: tracing about 13,694 live objects took roughly 4.8-5.2 ms, repeatedly
freeing only eight cyclic objects. Its per-object adjacency Vec allocated once
per traced object and used the standard string-oriented hasher for pointer
indices. The collector now stores edges contiguously with per-node offsets,
and uses the project's existing hashbrown hasher for pointer identities. Root,
weak-key/listener, collection scheduling and sweep rules are unchanged. Expected
gain is lower collection CPU/transient allocation, without delayed weak cleanup.
`DH2_DEBUG_UI_GC_NESTED_EDGES=1` restores both the previous adjacency layout and
standard hasher in the same build for targeted comparison.

### Pending sound acquisition ownership

Source inspection found a pending-event lifetime defect: both ordinary and FX
queues discarded completed StreamHolds whenever any other source was still
loading. Since StreamedSource keeps only a Weak resident reference, a completed
file could be freed and requested again on the next retry, potentially
preventing a multi-source event from ever becoming ready. Original file-source
state retains its pending/ready media through source ownership (bank source
`1411606d0` / release `141160970`; file source `1411614e0` / release
`141160cc0`); a partial completion must not lose that ownership.

Expected correction: each queued request/ticket retains its completed holds
across retries, hands them to the actual voice on admission, and releases them
on cancellation/world teardown. Scope is the pending event, not a process-wide
strong source cache. This is a correctness fix established by the ownership
path; any reduction in repeated reads needs actual production observation.
It does not solve all-child preloading or whole-file active playback, which
remain required streaming work.

Latest production dev validation (`lifecycles-owner-dev2-*`) completed
EotW -> Dust District -> EotW in both modes, exit 0, no fatal client/GPU
errors. All 2,699 captured source formats now match between eager preparation
and metadata-only preparation after the identification-header key correction.
Every closed VM reports zero remaining runtime objects. The 430 acquired UI
bitmaps in these loading-library sets all have one authored mip: full-chain
support changes neither their 34,556,544 retained bytes nor mip count (430).
The screen-distortion material cache was not instantiated at these saved views
(material_owner=false), so that run verifies safe teardown integration rather
than demonstrating savings from that cache.

Release glyph-page comparison (`lifecycles-owner-release-*`, 9 actual page
revisions per run): full recreation used 9 textures / 7,864,320 uploaded bytes /
0.984 ms cumulative acquisition CPU; page reuse used 2 textures + 7 rectangle
updates / 2,313,100 uploaded bytes / 0.535 ms. Both clients exited 0 without
GPU validation errors. These are nine acquisition operations, not hundreds of
independent timing samples or a whole-frame FPS improvement.

### Session shader metadata reuse

UI named-image binding correction: `1410150f0` binds the image resource,
whose current GPU view can change independently of a movie's draw list. The
host compositor cached that view by name forever. Expected correction is zero
stale named-image views after replacement, and zero compositor-owned named
views after movie close; the image owner's resource lifetime is unchanged.
Only a changed view needs a replacement cache entry, with no texture upload.

Original bootstrap `140064e00` constructs global cache DAT_142be6ee0 once
through `1402c8d50`, registers its destructor at exit, and initializes indices
through `1402ca800` -> `1402ca9c0`; shared names use `1402cc070`. The rewrite
instead reconstructs those immutable indices and render-parameter schema for
every ProgramLibrary at map acquisition. Its stage/definition payload streams
already remain on disk, and per-world prepared shadow/program definitions still
have live consumers.

Expected correction: share only immutable index/schema/default metadata for
the session, keeping separate seekable data-stream handles and per-owner
prepared definitions. No mutable file cursor may be shared through File cloning.
Expected gain is zero repeated index reads/schema parsing after the first
acquisition of an installation/backend. Steady-state sampling/draw logic must
not change. Compare same-build `DH2_DEBUG_UNSHARED_SHADER_METADATA=1` against
normal reuse with targeted acquisition time, read bytes and retained metadata
counts. Existing original SHADERCACHE budget is 43,566,840 CPU bytes; do not
confuse serialized index bytes or descriptor-capacity estimates with that tag's
entire live allocation.

Production release collector measurement (`lifecycles-owner-performance-*`):
29 sustained pause-menu collections per mode, visiting 13,686–13,695 objects.
Nested vectors/standard hash averaged 4.862 ms (median 4.982); contiguous
edges/pointer hasher averaged 3.238 ms (median 3.221), a 33.4% reduction in the
changed collection function. Both modes reclaimed 172 objects over the run
and exited 0. Collection scheduling and weak-reference rules were identical.
This sample supports the data-layout improvement, not a whole-frame claim or
hundreds of independent collection samples.

Production dev shader acquisition (`lifecycles-shader-metadata-*`) completed
EotW -> Dust District -> EotW in both modes, exit 0. Unshared mode read
733,409 index bytes on each acquisition and rebuilt 1,688 parameter kinds /
239 option defaults each time. Shared mode read the indices once, preserved
index/schema identities across all maps, and read zero index bytes on both
subsequent acquisitions; parameter acquisition took 0.007 / 0.005 ms instead
of 159.217 / 155.381 ms in that dev run. Release timing is measured separately;
file-handle opening remains required and is not counted as eliminated work.

Mapinfo loading-header lookup now keys by installation and map, matching its
underlying declaration owner. Switching installations also releases registry
references to outgoing installation declarations; existing callers keep their
Arc until finished. This prevents stale mission metadata for equal map paths
from different installations; it does not change authored loading text.

Pending read cancellation correction: StreamedSource previously retained its
IO Task strongly for the cached source's entire lifetime. If the requesting
event was cancelled before its next retry, a completed Task retained a whole
encoded file with no active playback. Expected correction is zero strong
read-result owners in the source registry: pending ordinary/FX event holds own
a shared StreamRead; the source keeps only a Weak. The last cancellation drops
the task/result, while completed media stays shared through ready voice holds
or other pending read leases. File-source release follows `141160cc0`; this is a
host asynchronous ownership mechanism, not yet bounded incremental streaming.

Bounded file-reader preparation evidence: original file Vorbis constructor /
initialization / packet assembly / decode / destruction are `141160e10`,
`1411614e0`, `141161240`, `1411610b0`, `141160cc0` respectively. The bank
source functions are distinct. Packet assembly retains only an unfinished
packet across buffer boundaries, and pending IO (`0x2e`) is distinct from
exhaustion (`0x11`). Sparse seeking delegates `141160ba0`, whose u16 frame/byte
deltas are relative to the end of the table. Production-only diagnostic
`DH2_DEBUG_SOUND_SEEK_METADATA=1` records how those authored offsets align
with the existing decoder's packet frame positions before replacing its
whole-file index. It changes no acquisition or playback decisions.

Texture streaming settings connection: original `14030d1e0` reads the live
per-frame upload budget, `14030cce0` reads the maximum candidate texture size
and add-all-mips flag, and `14030f3d0` reads freeze/unload controls. The host
streamer instead used constant 4096 KB / 131072 KB / 60 frames and never
honoured those registered settings. Expected correction: unchanged decisions
at defaults; changed configured limits reach their existing eligibility and
upload helpers. Cache the parsed settings by console generation so unchanged
frames add one generation read, not repeated console locks/string lookups.
This does not replace the outstanding VRAM pressure/priority policy.

Release shader acquisition (`lifecycles-shader-release-*`), same-build control,
EotW -> Dust -> EotW, both exit 0: repeated index acquisition 4.822 / 4.539 ms
and 733,409 logical bytes per map -> 0.314 / 0.276 ms and zero index reads;
parameter acquisition 139.144 / 139.974 ms -> 0.005 / 0.004 ms. First acquired
indices/schema remain resident and keep identical identities on reuse. These
are targeted loading costs, not a frame-rate measurement.

Particle/scene live-path overlap in the same production runs: EotW 53 2D / 9
3D particle image paths, Dust 44 2D / 10 3D; each has two paths in common with
the surface registry, totalling 88,400 compiled payload bytes. This bounds the
observed duplicated particle/scene source payload, not GPU allocation or
UI/postprocess overlap. The remaining distinct images cannot be counted as
cross-domain duplicates. Canonical ownership and particle mip demand remain
separate work.

Original automatic image-budget initialization is in the disassembled tail
`140369eed`–`140369f8f` (the prior decompilation stopped at logging calls).
For a negative configured budget it uses inspected dedicated video memory in
MiB minus `r_vramMiscSizeEstimationMB`, limits the result to 4096 MiB and falls
back to 256 MiB if memory detection fails or leaves less than 256 MiB. The
hardware inspector `1400ddf60` selects vendor inspection (`1400dd580`,
`1400dd330`) then fallback `1400dd950`; NVIDIA memory is reported in KiB and
converted to bytes. Expected backend bridge: query the actual rendering
adapter once, not a second arbitrary system adapter; expose read-only
capacity to the image owner without adding per-frame driver calls. Vulkan
unified/CPU heaps cannot be claimed as dedicated VRAM and use the source's
fallback when no equivalent dedicated capacity is available.

Production ownership validation (`lifecycles-image-bindings-client`):
EotW -> Dust -> EotW exited 0, no fatal client/GPU errors. All three VMs closed
with zero retained objects. The pending/ready read-owner path released 130
completed reads; no unclaimed read cancellation occurred in this sequence,
so cancellation reclamation is established by the weak ownership path and
still needs an actual cancelled in-flight event for a measured byte claim.
UI named-image binding followed EotW view28 -> Dust view12948 -> EotW view28,
reusing the existing image-owner texture without a second upload. This run
validates handoff/reuse; it does not exercise a live same-name texture resize.

Sound residency diagnostics correction: the old CODECS totals counted every
acquisition and never subtracted map-unloaded sources, so EotW -> Dust -> EotW
reported monotonically increasing codec residency despite MEDIA pruning.
Replace the mutable cumulative counter with a snapshot of the actual current
MEDIA owner at the loading report. Label representation and logical view bytes
explicitly; overlapping bank-backed ranges are not physical allocations.
Existing physical bank/source allocation deduplication remains the authority
for retained encoded bytes. This fixes accounting, not an additional measured
memory saving, and removes the diagnostic tally lock from source preparation.

Bounded sound IO owner implementation in progress: `assets/wwise/stream_io`
uses the original device's 2 MiB encoded-byte capacity and 16 KiB blocks.
Host integration is a lazy blocking reader thread with shared package-range
requests and least-recently-used idle blocks. Active buffer leases count
against the same capacity; only the reader waits for space. Cancellation is
checked under the registry lock before starting an unrequested queued read;
failed reads are not retained in the cache. Device teardown signals both
wait conditions, joins the reader and releases cached buffers/package handles.
The old whole-file playback path is not yet replaced; do not count this
in-progress owner as completed streaming or claim memory savings from it.

Selected-voice source admission: the current event preparer acquires every
streamed child before container selection, so one selected random/switch child
can pin unselected files for the event's entire lifetime. Original file-source
construction/destruction (`141160e10` / `141160cc0`) belongs to a voice; deferred
buffer readiness is separate from EOF (`141161770`, `141161240`). Expected
correction: selected audible source owns its pending/readied media; unrelated
children are not read. The audio callback only submits/polls background reads,
with readiness checked at the existing 1024-frame audio quantum. No source
clock advancement or completion is fabricated while its first buffer is
pending. Keep an opt-in event-preload comparison until production acquisition
counts and output-clock behaviour have been measured; active whole-file
reading remains until the bounded decoder is connected.

Production stream-metadata validation (`lifecycles-stream-metadata-*`): both
DirectX 12 and Vulkan clients exited 0, no fatal client/GPU errors. Each
observed 1,674 Vorbis sources / 130,109 seek-table entries, with zero offsets
missing a packet boundary. The dominant (current packet start minus authored
seek frame, packet end minus authored seek frame) pairs were (-1024, 0),
(-128, 0), (-576, 0); long/short transitions also produced an end difference
of +448 frames. Sparse seeking must preserve priming/output-window alignment,
not use the table's frame as the existing decoder packet's output start.
Both adapters resolved the source-derived automatic image budget to 4096 MiB;
the physical capacity reports were 17,130,815,488 / 17,163,091,968 bytes,
respectively. This verifies the read-only backend bridge and configured mip
limit propagation (131072 -> -1 in the client), not the remaining eviction
scheduler or sound decoder connection.

Bounded decoder connection (currently opt-in while validating):
`DH2_DEBUG_BOUNDED_SOUND_STREAMS=1` acquires only RIFF/setup/seek metadata,
then uses shared encoded windows for Vorbis and IMA ADPCM playback. The packed
seek table stays packed; no whole-file packet index or PCM expansion is kept.
A source's initial/ongoing IO wait preserves its clock and is distinct from
EOF. Lewton sparse priming aligns the authored point with its output tail;
an earlier authored point supplies any required long/short tail samples.
`DH2_DEBUG_COMPARE_STREAM_PCM=1` retains the existing encoded decoder only as
a production diagnostic and compares actual played frame/channel samples;
this diagnostic's duplicate encoded backing is not production residency.
Default playback remains the existing file reader until those results and
changed-function costs have been reviewed. Contiguous packets now borrow their
IO block; only cross-block/prefix packets spill into owned assembly storage.

### Prediction correction: selected-source admission

The pre-change statement specified selected-only reads and voice-owned
residency, but did not specify a quantitative CPU prediction. That omission
cannot be repaired by calling the observed timing an expected result after
the change. Keep this implementation unfinished until affected-function CPU
cost and behaviour are compared; it is not included in main's 0b39187.

Completed dev runs (`lifecycles-selected-stream-*`) exited 0, no fatal errors:
event-wide preload 53 file reads / 12,315,654 bytes / 13.123 ms combined
read-and-prepare wall time; selected-only 47 reads / 7,437,056 bytes / 13.271 ms.
The sampled source choices differ (135 vs 82 source-ready notifications) and
IO task wall time includes scheduling, so these are neither identical CPU
workloads nor evidence of a CPU improvement/regression. They show the observed
IO volume in those runs; they do not measure resident-memory peak.

For subsequent validation, separate these requirements before further changes:
- Admission must issue zero reads solely for unselected children. Count the
  actual selected source identities and physical reads, rather than predict a
  universal percentage from the total event graph.
- Existing per-voice decode/mix work must not regress. Compare the affected
  decoder calls on the exact same played source frames, and measure pending
  readiness overhead separately from successful sample work. Do not use frame
  rate or asynchronous task wall time as CPU time.
- The bounded device must retain at most 2,097,152 encoded bytes including
  active block leases; metadata/seek tables and codec workspaces are separate
  and must be accounted for rather than hidden in that figure.
- The contiguous-packet path must copy zero encoded payload bytes; only
  packets crossing IO block boundaries require a spill copy. Its precise
  changed-function CPU cost remains to be measured, not asserted as unchanged.

Any CPU regression or discrepancy from these work/count predictions requires
investigation of the implementation or cost model before acceptance. The
source already written before this record is implementation under review,
not a newly documented pre-implementation prediction.

Bounded production playback result (`lifecycles-bounded-sound-client`):
EotW -> Dust -> EotW exited 0. Fifty-one actual voice decoders compared
8,331,264 played frame/channel samples with the existing encoded decoder:
zero differences, maximum absolute error 0. The shared encoded IO cache peaked
at 1,021,584 bytes (limit 2,097,152). No decoder failures or fatal client/GPU
errors were recorded. Comparison mode also retains the reference full files;
that duplication is diagnostic-only and is not a production memory figure.
The latest contiguous-packet zero-copy path and affected-function CPU cost
still need their final build/measurement before enabling bounded playback by
default. Upcoming traces use the actual audio callback/decoder/IO functions;
no file IO, allocator work or seek-table scan is expected on a cached sample.
Any such stack or a material per-function regression invalidates acceptance.

Prefetch continuation prediction (before implementation): type-1 sources
must retain their authored bank prefix by borrowed bank range and read only
continuation bytes when selected. For prefixes covering setup and the initial
audio packets, source admission must perform zero file reads and must not
wait for IO before decoding those bytes. Incomplete/absent prefixes must defer
until required bytes arrive, as file-source initialization already does.
Expected loading work eliminated: full-file acquisition and full packet-index
construction for these sources; do not predict a CPU percentage without their
packet counts. Cached PCM sampling must gain no IO/prefix parsing work. The
prefix shares its bank backing, so zero extra physical prefix allocation is
expected; the continuation still shares the 2 MiB encoded device budget.

Prefetch implementation (awaiting production validation): extent-aware bank
resolution borrows the DIDX/DATA prefix and registers its package extent
without reading the full file. MediaBytes uses a resident/header/prefetch
extent in place of the previous optional source size; the bytes/range remain
shared. Legacy whole-resource tooling keeps its existing provider contract.
Complete prefixes prepare metadata without file IO and are immediately
available to selected voices. Windows consume the bank bytes first and request
only continuation ranges starting after that prefix. Packets crossing either
the prefix boundary or an IO block boundary use the same spill assembler.
Comparison mode deliberately prepares the reference whole file in background;
its initial wait/bytes are diagnostic and cannot prove normal prefetch latency.

Package-retirement prediction (before implementation): the new IO device
currently retains package handles until session end. At the existing map-bank
unload boundary (`1400c97e0` -> `1400c0360`), retire cached ranges/queued reads
for packages no longer owned by retained banks, and close those reader handles
on the reader thread. Expected result after acknowledgement: zero cache entries,
queued reads or reader handles for unloaded packages; an already executing
16 KiB read and active buffer leases can finish/release normally. Cached audio
sampling gains no work. New IO requests gain one package-membership check;
map teardown visits cached entries/queued requests once, with no per-frame
sweeps or extra reads within a loaded map. This is host handle/cache retirement
following the verified bank lifetime, not a claim that the game's internal
cache invalidation implementation has been copied.

Map-package retirement is now connected to audio::begin_level after the
engine's outgoing-voice release. Retained banks define the allowed packages;
new package registration admits their paths. The reader retires queued/cache
entries immediately, closes obsolete handles on its own control wake (even
while idle), and cannot publish a completed outgoing read into a replacement
request with the same key. This path still needs the queued three-map run;
source-level ownership is not substituted for that execution evidence.

Budget-wakeup correction prediction (before implementation): a released
ReadLease can make a cached block evictable without decreasing used bytes.
If its notification lands between the worker's eviction scan and wait, the
worker can sleep indefinitely. Retiring an in-flight package request has the
same race. Record a wake generation under the budget mutex and wait only if
it has not changed since the scan. Expected result: neither release can be
lost; cached sample lookup/decode gains zero work, one generation increment
per lease release/package retirement, and one snapshot/check per full-budget
retry. This is a host concurrency correction, not a throughput improvement.

Prefetch/contiguous-packet production comparison (`lifecycles-prefetch-*`):
library and game-bin checks, dev build and both three-map clients exited 0.
Comparison mode matched 8,730,624 played samples across 92 voice decoders,
zero differences and maximum absolute error 0. Encoded cache peak was
1,037,728 bytes in comparison mode and 526,645 bytes in normal mode, both
below 2,097,152. Normal mode prepared 26 bank-prefetch metadata records, but
none of these sources was selected during those saved-view runs. This proves
prefix preparation executes, not immediate type-1 playback latency. The newer
package-retirement/wakeup changes are excluded from those runs and have their
own queued production validation. No CPU acceptance is inferred from them.

Installed prefetch extent audit (2026-11-04,
`artifacts/lifecycles-prefetch-inputs.json`): the actual common/init and
EotW/Dust bank IDs from wwisedescription.txt select seven available packages
and seven banks in the installed Russian/SFX set. All 33,259 type-1 Sound
references (22,015 unique media/language pairs) have a package continuation;
every DIDX/DATA prefix byte matches that continuation's beginning. Zero
missing continuations or prefix mismatches. References sum to 32,594,418
prefix bytes and 1,059,610,258 full-source bytes, with repeated references;
these totals are not allocations or predicted RAM savings. This validates
borrowed-prefix continuation offsets across the installed sources, without
claiming all those dialogue sources were played by the saved-view benchmark.

Changed-function CPU baseline (`lifecycles-stream-cpu-whole`, same profiling
binary as the queued bounded control): 47.811 seconds, 2,110 samples in
VorbisCursor::sample (44.132 sampled CPU ms/s, inclusive), 1,534 in its decode
(32.085 ms/s), 2,247 in EncodedCursor::sample (46.998 ms/s), and 32 in
Stream::ready (0.669 ms/s). These nested costs must not be summed. This is an
unpaired baseline, not acceptance or a regression result. Different randomized
sound choices can change cost. A fresh production comparison capture runs the
bounded and reference cursor on each same played sample; per-cursor function
stacks distinguish their costs in that capture. It also records pending/read
costs separately. CPU-only ETW capture omits scheduler/wakeup stacks to avoid
the unrelated 24 GB dump from the full scheduling trace; production code is
unchanged by that trace configuration. The profiling build/run is serialized
after the package-retirement dev validation, preserving the PDB for each trace.

Trace setup correction: the old bounded-control client completed its full
benchmark, but no ETW capture was started: the helper awaited only the
waiting->ready transition message. When loading has completed render gates
before the benchmark runs, that transition does not occur. The same production
ready path always emits BENCHMARK_INPUT; the helper now accepts that marker
as well. The failed capture contributes no CPU comparison. The fresh paired
capture uses the corrected helper and CPU samples only. The first retirement runner stopped before writing its initial command result;
its cause is not established by the empty log. The retry uses explicit native
command exit codes with Continue preference, and an outer runner captures
PowerShell failures separately. No successful build/run is inferred from queueing.

Particle upload-source retirement prediction (before implementation): image
registry 1400607f0/14005feb0 owns identity independently of material/pass, and
1402ea140 uploads the installed compiled blocks. Current particle materials
and the GPU identity cache both pin the compressed Vec after upload. Host
correction: retain stable descriptor/identity but retire those bytes only
after create_texture_with_data has copied them into backend staging. Keep
GPU image residency across colour/depth target rebuilds; samplers and target
pipelines may rebuild without rereading/reuploading images. Prediction for
the measured EotW workload: same 59 uploads / 16,046,296 uploaded bytes,
16,046,296 compressed CPU bytes released after those uploads (unrequested
images remain staged), zero additional image uploads on target rebuild.
One uncontended mutex acquisition per initial upload; cached acquisitions
and gameplay draws gain zero payload locking/allocation or IO. Retain-data
and duplicate-image diagnostic modes preserve their baseline bytes for a
same-build comparison. Validate through production counters and map/rebuild
runs, not a whole-process RAM delta or synthetic tests.

Particle source retirement implementation: typed image identities retain only
the descriptor, original payload byte count and a take-on-upload staging Vec.
The backend retires that Vec after the queue upload call; GPU residency owns
the stable identity without pinning payload bytes. CameraGpu owns the image
cache separately from ParticlePass, because transient missing view/source
inputs and target-format changes can discard a pass without ending the level.
Only release_level retires this level image cache. Material preparation and
lazy first-use acquisition borrow the same cache; there is no mutex/cache
lookup added to an already prepared material's draw. Production release logs
now include cpu_released_bytes; retained-data control leaves upload identity
and rendered data unchanged. Compilation and runtime comparison are queued,
not inferred from this ownership inspection. Canonical cross-domain images
and finer-mip demand remain separate unfinished requirements.

Resource-class purge source findings (2026-11-04): 1400410f0 always performs
outgoing world/audio teardown before its conditional resource purge. That
purge calls 1400600d0(0), and optionally (1). 1400600d0 serializes all resource
lists; 140060180 traverses a list backwards, removes matching +0x24 lifetime
classes from the ID hash/list, clears their registered ID and dispatches the
+0x08 destructor. Nonmatching resources receive their +0x48 callback instead.
140060b60 promotes all currently registered resources to class 2; builtin
acquisition 14005fde0 also creates class-2 resources. Constructor 14005e740
initializes class 0/1 from the active allocation scope: 1401548e0 returns the
idMemLocal singleton (constructor 1400fa870; vtable 141ae5098). Its +0x98
slot is 1400fa990, which tests virtual +0xa8 (1400fa900) == 1. The latter
reads the TLS heap stack at +0x130/+0x140; 1400fa930 names heap 0 system,
1 persistent, 2 map. Therefore constructor class 1 means persistent allocation
scope; other active scopes produce class 0. Static resource class 2 is the
separate promotion above, not heap enum 2. Push/pop are 1400faa20/1400fa9b0.
Manifest loading 1400661e0 can also mark selected resource types class 1. Therefore a map teardown,
a class purge and destruction of every session cache are different operations;
a host Arc owner is not evidence that an optional force-purge was ported.
These functions give the common lifecycle rule for the per-class work;
the host must preserve session identities while ending map payload ownership.

Texture pressure scheduling source resolution: 14030f3d0 ages records, builds
its packed sort key, and reduces residency only when its +0x3310 byte counter
plus headroom exceeds its configured budget. The headroom is the registered
per-frame upload KB plus maximum texture KB (both shifted by 10), not a
fixed arbitrary reserve. It drops an aged candidate to minimum through
14030d7a0; otherwise an oversized projected-area candidate drops one level
through 14030d8d0. It considers an upgrade only while current bytes are within
budget. Priority multiplier defaults to 1 when r_skipAnisoTextureStreaming
is enabled; otherwise bits 1..5 scale projected area. Read-only Ghidra bytes
confirm float constants 1.0 (141f697b0), 4.0 (141ac8fdc) and 0.5
(141ab3764). Existing area-only demand and unconditional idle reductions
cannot establish parity. This identifies the required accounting/order/gate
change; enabling budget-only eviction without those inputs would be incomplete.

CPU trace readiness correction prediction (before implementation): the prior
helper fallback was wrong: BENCHMARK_INPUT is emitted inside elapsed >= target,
not on initial readiness. The paired production client matched 69,000,183
samples across 27 voices with zero differences, but its trace recorded after
exit and contains zero client samples. It provides no CPU comparison. Emit
the existing starting-warmup message once when the ready benchmark path is
first entered, even if no waiting frame preceded it. Expected result: one
initial readiness marker before measurement, no gameplay rendering/audio
change; the helper waits only for that marker and rejects early exit. Reject
zero-client-sample parses instead of publishing a successful empty trace.

Texture request scratch prediction (before implementation): the host adapter
currently allocates request flags and a sorted request list every active
streaming frame. Reuse those two vectors per level; reset on SceneRenderQueue
replacement. Expected allocation count after first capacity growth: zero for
these vectors, versus at least two allocation/free pairs per nonempty baseline
frame. Reserve the full request domain once so first fill does not grow
repeatedly; report capacity-growth frames, not allocator call counts.
Do not predict a CPU percentage from allocation count. Keep exactly the same
area/tie comparator and use in-place unstable sorting (the slot tie-breaker
makes the ordering total). No request, mip eligibility, IO budget, output or
GPU work change. Diagnostic baseline clears storage before the same algorithm,
and production counters record capacity growth per 600 frames. Original
14030f3d0 also reuses its job/priority lists; this corrects host scratch lifetime,
not the outstanding original priority/pressure policy.

Final-owner production comparison (`lifecycles-sound-retire-compare`): current
library/game-bin checks and dev build exited 0; EotW -> Dust -> EotW client
exited 0. Thirty-six decoders matched 8,064,999 actual played samples, zero
differences, maximum error 0; no decoder/residency/fatal errors. Both reader
map-retirement acknowledgements reported open_handles=0 with three startup
packages retained. Particle upload count/bytes remained 59/16,046,296,
54/30,148,760, 59/16,046,296; cpu_released_bytes exactly matched those byte
counts. EotW's 16,046,296-byte prediction therefore matched exactly. GPU image
storage was not reduced, recompressed or uploaded again per state variant.
The normal non-comparison run and retained/released particle control remain
separate queued results; no CPU acceptance is claimed from these dev upload
wall spans. Runtime target-format/pass replacement remains distinct from the
verified map transitions.

Render submission budget prediction (before implementation):
report_prepared_diffuse_images currently drains every queued GPU mip job,
although main-world staging alone uses MipUploadBudget. Main/render scheduling
can accumulate several staging frames before one render frame. Apply the
already ported 14030d1e0 budget at actual compressed upload submission, using
the same registered setting. Expected bytes per render frame: check the KB
threshold before each job, then charge successful upload bytes / 1000; one
oversized final job may exceed it. Preserve FIFO among deferred and newly
arrived jobs. No extra GPU pass/copy/upload bytes, format changes or premature
readiness/handle commit; old committed images remain until GPU completion.
Only an accumulated over-budget backlog gains deferred frames, matching the
original budget rather than an invented latency policy. Empty-frame work is
one cached settings read and budget initialization, with no new heap allocation.
Record actual render upload totals, peak per-frame/job bytes and deferrals;
do not infer GPU cost from main-world staging counts.

Particle retained/released A/B (`lifecycles-particle-retire-*`): both three-map
clients exited 0 with no residency/decoder/fatal errors. Both modes retained
59/54/59 unique GPU images and uploaded 16,046,296 / 30,148,760 / 16,046,296
bytes. Retained-data mode released zero CPU payload bytes after upload;
retirement mode released exactly those totals. Dev acquire/upload wall spans
were 20.281/28.933/22.470 ms versus 21.653/30.268/32.513 ms. These are not
per-function CPU time, so neither hide the higher spans nor declare a CPU
regression/improvement from them. The change adds no payload lock to an
already prepared draw; loading/unload CPU cost remains a separate measurement.
Normal sound streaming also completed the three-map run without errors; its
observed encoded-device peak was 559,653 bytes. Zero-read device-release logs
come from temporary default WwiseRegistry values used by AudioPreparation::take,
not the actual active reader: active reader reports contain 83 reads / 1,332,298
bytes. Do not interpret placeholder-owner release as device accounting.

Submission-budget current-source validation (`lifecycles-texture-submit-*`):
library check, game-bin check, dev build and both clients exited 0. Raw source
patch snapshots before/after build are byte-identical, SHA-256
393d1d2e5b7fabb0ceda64cb388e38df253f48ab40e383ca98e0bc4c24cc2b4d.
The runner's hash variable was empty, so that guard alone is not acceptance;
the retained raw snapshots independently establish unchanged build inputs.
Default render submission processed 367 uploads / 169,213,952 bytes, deferred
166 queued-job visits, peak frame 5,767,168 bytes, largest job 2,097,152 bytes.
The native 4,096,000-byte pre-job threshold plus one final job allows this peak.
Across 1,200 frames, request flag/order storage each grew on one frame only;
no additional capacity growth occurred after initial fill. The 32 KB config
trial remained 4096 because non-retail cvars reject unforced config sets;
repeat both values using existing DH2_DEBUG_DEVELOPER, without changing retail
configuration policy or the user's config. No CPU gain is inferred from counts.

Priority source refinement: 1402d4e20 raises image +0x118 through 1402eab10;
14031a510 reads it through 140416b50. The specularMap role requests 16, reduced
to 4 for material +0x4b alpha-test bit 4; the explicit role whitelist requests
4 and other image roles 1. Registration 14030dbf0 obtains nine packed bits
through 14030cdb0, which classifies installed image paths (skiff_engine_01,
skiff_, vista plus suffix rules). These are original authored path rules,
not permission to invent exceptions. Instruction capture at 14030f471/479
shows the sort key uses the scaled area's raw float bits shifted by 32;
it is not numeric float-to-integer conversion despite the old decompilation's
cast. Preserve this distinction when connecting the full packed scheduler.

Controlled submission-budget runs (`lifecycles-texture-submit-developer-*`)
completed at the actual 4096 KB and 32 KB settings with the same current dev
binary. Both processed exactly 367 jobs / 169,213,952 upload bytes. At 4096 KB,
peak frame upload was 5,767,168 bytes and peak job 2,097,152; at 32 KB, both
peaks were 2,097,152 bytes. Thus the smaller threshold schedules at most the
single oversized job in the observed worst frame, and total submitted data is
unchanged. Queue deferral visits were 193 and 43 respectively; differing
producer timing means these are not an independent performance metric.
Both clients exited 0. Source/game-bin compilation and default-mode 1,200-frame
scratch counters are verified; baseline scratch control and valid paired CPU
trace still await the corrected profiling build.

Pressure-counter scope correction from original data references: streamer
base 1432f0210 + 0x3310 is global 1432f3520. It is increased inside 14030f3d0
when finer-mip work is queued and decreased by 14030d7a0/14030d8d0 when dropping
mips; it does not simply sum every base image's descriptor size. Charging all
base chains against this counter would port the wrong budget model. Track
streaming increments/reservations and corresponding releases, including reused
shared images, when connecting pressure eviction. Native registration's
nine-bit path priority comes from 14030cdb0; it is distinct from per-image
sampling-role priority read at +0x118.

Valid paired CPU result (`lifecycles-paired-sound-v3`): 47.219 traced seconds,
175,561 client CPU samples; 48 voice comparisons matched 69,024,812 actual
samples, zero differences / maximum error 0. Thin LTO inlines FileCursor into
EncodedCursor::sample. Resolve the outermost caller PC against the matching
PDB with source lines (return PCs minus one; leaf PCs unchanged), rather than
report its absent symbol as zero. Graph line 169 (bounded cursor work):
794 samples / 16.815 CPU ms/s; line 172 (reference on those same played inputs):
812 / 17.197 ms/s. Other resident voices (line 165) are excluded; comparison
bookkeeping and shared enum entry/return instructions remain separate. The
2.2% difference is within sampling uncertainty: no detected regression, not a
claim of a 2.2% speedup. Windows symbol lookup must eagerly load line data;
deferred symbol loading returned unresolved lines. Evidence: audio-lines.json
plus affected.json beside that trace.

Scratch A/B (`lifecycles-texture-scratch-v3-*`), same profiling binary: both
clients exited 0. At 1,800 streaming frames the allocation control recorded
1,800 flag-growth frames and 1,800 order-growth frames; reuse recorded 1 and 1.
Both capacities were 4,094 slots. This matches the two-vector allocation
prediction; do not turn growth-frame counts into an unmeasured CPU percentage.

Final stream-owner sweep prediction (before implementation/default change):
141161240 releases its consumed input through stream +0x80 before obtaining
the next buffer. A host Window must not pin historical blocks or unrelated
read-ahead while its required block waits: 128 one-block cursors can otherwise
pin the entire 2 MiB device and prevent their next reads. On a pending miss,
release old local blocks and non-target pending leases, keeping only the needed
request. Preserve a packet whose required next-mode lookahead crosses a block
in spill storage before releasing that block (lewton host requirement).
Expected invariant: waiting windows pin no historical/non-target buffers;
maximum encoded-device bytes stay 2,097,152. Cached sample and cached-block
hit paths gain no ownership work. Ordinary packet payload remains zero-copy
when payload and required lookahead fit its block; only a boundary packet or
lookahead spills. Borrow immutable headers for packet decoding instead of two
Arc clone/drop pairs per packet; expected header reference-count traffic there
falls to zero with identical PCM. Bounded streaming becomes the normal owner;
DH2_DEBUG_BOUNDED_SOUND_STREAMS=0 retains the diagnostic legacy-reader control.
Default must produce the already compared opt-in samples and bounded residency;
verify the final sweep on the existing production walking route before main.

Walking validation uses the already established ten-metre EotW player route
(-30.81336,192.69846,2.94306 to -30.81336,182.69846,2.94306), with
DH2_BENCHMARK_WALK=1, not a stationary frame or camera-only substitute.
record_cpu_trace now accepts structured EnvironmentValues, preserving the
semicolon in waypoint lists through elevated CLIXML; the older flat delimiter
cannot represent that value. No new synthetic/oracle executable or tests are
used. Compilation is one cohesive final audio-owner batch, followed by actual
paired playback and targeted CPU capture; default bounded mode has no opt-in.


Final default-owner acceptance (`lifecycles-walking-sound`,
`lifecycles-default-streams`): library check, game-bin check, profiling build,
walking client and normal three-map client all exited 0. The recorded player
route reached both waypoints repeatedly; 47.199 traced seconds contain 188,645
client CPU samples. Eighty-one voice decoders compared 82,699,258 actual played
frame/channel samples with the existing reader: zero differences, maximum
error 0. Matching-PDB source attribution puts 920 samples / 19.492 CPU ms/s in
the bounded decoder call and 932 / 19.746 ms/s in the reference call on those
same samples. The 1.3% difference is within sampling uncertainty, not a speedup
claim. The comparison run's encoded device peaked at 2,095,324 bytes, below the
2,097,152-byte bound. It includes the final pending-buffer release and borrowed
header changes, with bounded streaming enabled by default.

Normal playback (comparison disabled, no bounded-stream opt-in) completed
EotW -> Dust -> EotW with one catalog/device/window/audio output, no decoder,
GPU upload or fatal errors, and an encoded-device peak of 552,547 bytes.
Outgoing map reads release their cached bytes before destination playback.
The map particle owners still report 59/54/59 uploads and release exactly
16,046,296 / 30,148,760 / 16,046,296 CPU payload bytes. Counts establish
retirement without extra image uploads; no additional loading-CPU improvement
is claimed. The earlier dev selected-source wall spans remain inconclusive
for CPU and cannot be retrospectively called a fulfilled prediction.


Texture pressure/reservation prediction (before implementation): replace the
unconditional 60-frame invisible-image reduction with 14030f3d0's pressure
gate and 14030d7a0/14030d8d0's minimum/one-level reductions. Count finer mips
only, including accepted read reservations; base-chain allocations are excluded
from the native +0x3310 counter. Keep that charge with init/common images on
map reuse, clear failed/cancelled reservations, and release map-only accounting
with its owner. Headroom is (per-frame KB + maximum-texture KB) << 10. With
charge + headroom <= budget, expected pressure reductions are exactly zero,
regardless of invisible age. Above headroom, choose one eligible low-priority
resident image per scheduling frame: expired images go to minimum, otherwise
oversized projected density loses one level. Do not reduce an image with an
outstanding read/upload. Upgrades stop once charged bytes exceed the budget;
one accepted job can cross the threshold, as in the original.

A completed read/upload stays owned by its current map even if it became
invisible meanwhile; the original finishes requested mip work and later
pressure scheduling decides residency. Expected invisible-result discards:
zero, avoiding repeated reads/uploads when turning back. GPU completion still
precedes handle replacement. There is no new pixel conversion or GPU pass;
reductions copy only retained mip levels as before. Main-thread accounting
adds one integer charge sum to the existing image scan, with no per-frame
allocation; mip-byte arithmetic runs only at transitions. No CPU speedup is
predicted. GPU residency may increase within budget, trading previously
premature eviction for reuse. Priority/producer metadata must be connected
before claiming full scheduler parity; pressure validation alone cannot prove
that ordering.


Priority input resolution: shared Ghidra instruction captures establish that
14031ad30 submits mode 1 at 14031b9ef, while static world calls 140339742 and
14033d2ad submit mode 0. Sampling priority comes from the source parameter
identity, not the backend binding number: arkSimpleSprite's backend slot zero
is textureMap (4), whereas diffuseMap is 1. Supported material bindings now
raise their shared image priority with 1402d4e20's actual whitelist, including
specularMap 16/alpha-test 4; dynamic and static producers publish their actual
mode with the area through the existing packed CAS. This is fidelity work;
it is not a prediction of lower CPU cost. Inactive/unimplemented source banks
remain outside current material rendering, so those are not claimed covered
by the implemented binding inventory.

Shared-image handoff prediction (before implementation): the session cache
currently keeps a strong handle to the old texture after a reused common
image upgrades in the active map. Transfer cached ownership into the new map
instead of retaining a stale parallel handle; capture the current committed
identity back on teardown. Expected extra session-held superseded textures:
zero. Base/read/GPU upload counts on reuse remain zero, and old handles still
live until their admitted GPU copies/consumers complete. Preserve the maximum
sampling priority across those transfers. No per-frame cache update/lookup is
needed. Retained common images absent from the active map remain charged,
rather than disappearing from global budget accounting; their pressure
reduction needs the same canonical owner as active images.


Inactive shared-image pressure prediction (before implementation): include
retained common images in candidate selection, with their age advanced by the
same owner scheduling tick. Otherwise an inactive image could keep the budget
full while the active map has nothing left to evict. Queue its minimum/one-mip
copy through the existing upload mailbox; old cache handles remain committed
until acknowledgement. Count pending reduction at its target size, restore
old charge on failure, and cancel uncommitted cache replacements at map
publication reset. Choose the single lowest key across active and inactive
images; no second reduction or new GPU pass per frame. All inactive cache
replacement bookkeeping is empty in frames with no such job.


Texture implementation batch now connects scoped finer-mip/reservation
accounting, pressure-only minimum/one-level reductions across active and
inactive shared images, path/mode/age ordering and sampling-priority density
inputs for implemented material bindings. Static producer mode 0 and dynamic
producer mode 1 are read from installed call instructions. Pending mip results
stay with their admitting map until completion; source/GPU failure releases
reservations. Reused common image handles transfer to active ownership, so an
old session-cache handle cannot pin a superseded GPU allocation. Cache-only
reductions use the existing completion mailbox and restore committed ownership
on failure/cancellation. Host reduction selection scans for the lowest key
instead of sorting all residents, and uses stable slot/path tie-breaking.
This batch is on the feature worktree awaiting the queued production residency
runs; it is not the main release built from 67da10c. No tests or visual checks
were run. Whole-registry image identity and finer particle mip participation
remain required work; this batch is not full image-lifecycle completion.


First pressure batch result (`lifecycles-texture-pressure-4096`): library and
game-bin checks, dev build and the production walking client exited 0. The
registered budget was actually 4096 MiB; gameplay quality changed the maximum
texture setting from 131072 to -1 KB, which the pressure headroom reads live.
Across streaming frames 600/1200/1800/2400/3000, finer-mip charge remained
522,584,064 bytes with exactly zero reductions/released bytes. This matches
the predicted absence of eviction while comfortably within budget, even over
many 60-frame invisible intervals. The same 628 mip jobs / 522,584,064 upload
bytes remain stable over those samples, rather than accumulating new uploads
on each walk. Request flag/order storage each grows once. Both ten-metre route
waypoints are reached repeatedly; no decoder, GPU upload, device-loss or fatal
errors occur. This is residency/work-count validation, not a dev CPU/FPS claim.
The higher detail total includes the newly restored original sampling-role
priorities; do not present it as a byte reduction against the old area-only
admission. The 256 MiB pressure and repeated-map handoff runs remain required
before this texture batch can enter main.


Low-budget result (`lifecycles-texture-pressure-256`): the same dev client
exited 0. At the actual 256 MiB budget, owner samples from frames 600 through
4200 stayed at 271,056,896 charged bytes, 64 reductions and 35,258,368 bytes
released. Total upload work stabilized at 383 jobs / 306,315,264 bytes, with
an 8,388,608-byte largest job. The 2,621,440-byte budget overshoot fits one
accepted final job; no continuing admissions or eviction/read cycle appears
in these samples. No decoder/GPU/fatal errors. This matches the native
pre-job budget gate and pressure-only reduction prediction. Normal-budget
and constrained-budget outputs intentionally have different mip residency;
they are not image-equivalence or CPU comparisons.

Final ownership correction prediction (before implementation): a one-level
reduction that lands on the inline minimum must retain that minimum's exact
resource identity, rather than inventing a separate-file mip index for it.
Use the same reduction descriptor builder for active and cached images.
Expected copy extent, dropped level count and byte release stay unchanged;
only the incorrect minimum resource_mip/next-suffix metadata is corrected.
Cache the native path priority when a shared image is captured, removing
repeated string classification during inactive pressure scans. No draw/pass,
read or per-frame allocation is added. Host accounting uses actual compressed
block extents; native +0x3310 uses the width*height*format-bpp estimate. Their
scope is finer mips/reservations in both cases; padded sub-block byte differences
are an explicit backend accounting difference, not a byte-parity claim.


Final texture-owner acceptance (`lifecycles-texture-handoff`): current library
and game-bin checks, dev build and EotW -> Dust -> EotW client all exit 0.
One catalog, render device, window and audio output span all three maps.
Outgoing common-image capture counts are 194 and 184, with the union bounded
at 202 identities; the destination GPU readiness gates complete. No missing
resident source, mip-copy/upload error, decoder failure, device loss or fatal
error occurs. This includes the corrected minimum-mip identity and cache path
priority. Compile-only checks and actual clients were used; no tests or visual
inspection. The map run proves successful lifecycle execution; it does not
claim pixel equivalence or measure a new CPU improvement. The standalone
budget runs provide the direct pressure/work-count evidence above.


Manifest acquisition prediction (before implementation): 140064590 searches
registered container metadata; it does not reopen every archive index to find
a manifest. Use the existing catalogue's selected-entry location instead of
load_list's whole patch-index discovery scans. Discovery index scans/read bytes
become zero per cold manifest, rather than scanning one or more complete patch
indices; the selected container's id table still needs one parse because the
compact catalogue intentionally does not retain a second full id namespace.
All 38 installed manifest paths have a single indexed container, so selecting
the catalogue entry preserves their container selection.

Read manifest ids before that selected index. Keep only records for requested
ids, including all language variants needed by the existing locale selection.
Expected retained IndexEntry count: exactly matching-id records, zero unrelated
records, versus the entire container before. Skip unrelated name payloads
without allocating Strings; retain header/length/extent validation and request
ordering. This removes redundant startup parsing/temporary allocations in
map_resources::load_list and archive::index; no gameplay-frame work changes.
Do not claim an absolute CPU improvement from this structural prediction or
from IO wall spans. Record actual selected/total metadata counts and run the
production loading path, with no tests or visual inspection.


Manifest acquisition result (`lifecycles-manifest`): library/game-bin checks,
dev build and actual EotW loading/playback exit 0. Reported discovery scans
are zero for init/common/EotW. Requested/retained ids and retained records:
1739/1739/1774, 21170/21169/27040, 15774/15774/21713. Common's missing id 2789
is the same known input omission as before. Localization variants remain,
unrelated ids do not enter the retained table, and request order is unchanged.
No loading/decoder/GPU/fatal errors. This establishes removed discovery reads
and metadata retention, not a CPU-time or FPS improvement. No tests run.


Backend shadow-target retirement prediction (before implementation):
ShadowSceneGpu::targets keeps each source Image's wrapped atlas and auxiliary
static-cache arrays until map teardown. It never removes an entry when that
Image leaves RenderAssets within the map. Follow the source owner's release,
as the generic 140060180 destructor path does, rather than add an age/size
heuristic: retain only ids still in RenderAssets<GpuImage>, before any repeated
revision early return. Expected stale-cache invariant: zero cached target ids
absent from the live image store after that pass. Preserve every live target's
texels, valid bits, cache generations and draw records. An unchanged revision
may skip submission only if its target cache still exists. This adds one live
id lookup per cached target, no allocation, copy, GPU pass or wait. No CPU
speedup or predetermined byte saving is claimed; retirement counters show
whether the actual client exercises dead target replacement. Resource bindings
already retain only the current submission, mesh allocation caches invalidate
on frees/slab epochs, and alpha bindings prune unused entries; those do not
need a second cleanup mechanism.

Catalogue lookup sweep prediction (before implementation): Catalog::read
currently resolves the same installed entry once for kind reporting and again
for payload access. Reuse the first immutable entry reference. Expected entry
lookups there: two to one per read, with identical overlay precedence, loading
reporting, bounds/decompression and returned bytes. No payload or IO scheduling
change, no new cache/lock, and no absolute CPU timing claim.


Archive container owner evidence/prediction (before implementation): 140068720
retains the index file at container+0x68; 14006a680 opens an active payload
container only when +0x70 is null and stores that reader; 14006a310 closes both
at container release. Current archive::read_location/read_prefix instead open
and query a container on every acquisition. Keep a catalogue-owned pool of
exclusive file readers per physical container, opened lazily. Host: independent
file cursors preserve the rewrite's concurrent readers; a shared synchronous
cursor or mutex held through IO/decompression would serialize them.

Expected work: file opens and extent queries approach peak simultaneous readers
per used container instead of resource request count. After reaching that peak,
repeated requests open no additional handles. Canonicalize game2/game3's shared
8192 container to the same physical pool key. No payload cache, format change,
extra compressed copy or per-frame work. Pool locks cover only checkout/return;
IO and inflation hold no pool lock. Reader leases borrow their catalogue owner,
and cached readers close when that owner drops. Loose mod files retain the
current direct-read path. Verify actual request/open/reuse counts on production
loading; do not assign CPU milliseconds or MB savings from handle counts.


Archive reader acceptance (production client, no tests): library/game-bin checks,
dev build and EotW -> Dust District -> EotW client all exited 0. At the final
reported checkpoint, 95,232 acquisitions used 16 opens/extent queries and
95,216 reused readers; the open count remained 16 throughout the late requests.
The same catalogue served all three maps. IO/decompression does not hold the
pool mutex, loose overlays keep their direct-read behavior, and no decoded
payload cache was introduced. These counts establish eliminated filesystem
operations, not measured CPU milliseconds or resident MB savings.

Shadow retirement acceptance: the same production client exited 0 without
shadow-target retirement events. Source-absent entries are now pruned before
revision reuse, but this route did not replace an atlas within one map. No
measured GPU memory saving or live quality/resolution coverage is claimed;
that broader coverage remains open. Existing map-boundary cleanup is unchanged.


Named UI image lifetime evidence/prediction (before implementation):
1410150f0 replaces a bitmap's texture wrapper; 14101d290/14100d6f0 share
wrappers and track context references. 14101d160 releases the last context
through 1402dfb00 -> 1402df680; 1402df6f0 drains those releases and destroys the
substitution handle via 1402e39d0. Acquisition 1402df8c0 separately loads the
underlying image from the resource manager through 14005feb0: movie-binding
release is not an instruction to purge every image in the resource registry.

Current UiImages holds every visited map's image/alias/dimensions forever, while
the movie's Display holds only its name and extent. Implement a host opaque
image lease carried by each bound Display and submitted draw list, with weak
wrapper lookup. At map teardown purge UI registry entries outside the existing
init/common image classification; retain common handles to avoid reopen latency.
An outgoing/prepared movie or submitted frame keeps its own lease even when
registry membership changes. Same-name binding is a no-op as in 1410150f0.
Publish existing leased handles instead of resolving/reloading names while
publishing every frame. No format, mip quality, sampler, GPU pass, or image
payload copy changes. Expected invariant: after map purge, strong registry
entries contain only common/init images, with old non-common images retained
only by actual consumers; weak wrapper metadata cannot accumulate dead entries.
Quantify registry/consumer counts on the existing three-map production route.
No CPU milliseconds or MB savings are predicted from ownership/count changes;
no additional CPU benchmark is justified for them.

Movie-close sweep prediction: pause_menu::run retains last_frame after Close,
which retains that closed movie's draw-list/library-image references until the
next Open. Clear that private reuse slot on Close, alongside the loaded movie;
submitted main/render frames keep independent references until they retire.
Expected retained worker reuse frames after Close: one to zero. No extra IO,
GPU work, per-frame processing, or CPU benchmark is needed for this ownership
correction (movie destruction 1410298b0).


Named UI image production acceptance: the first implementation's library/game
checks, dev build and EotW -> Dust -> EotW route exited 0. The first purge
released two registry entries (bootstrap plus outgoing map background); the
second released one, with zero common entries or live wrappers at both purge
checkpoints. Each movie resolved its 1920x1080 background, and all three maps
used one catalogue/device/window/audio output without reported errors. These
are registry ownership counts, not GPU heap/RSS measurements. The subsequent
Close reuse-slot correction is included in the cohesive retirement run; its
final result is recorded separately, not inferred from this earlier binary.


Model staging lifetime prediction (before implementation): setup's model_cache
and md6_cache have no consumer after the placement summary but currently survive
through environment/water/FX/particle acquisition until setup returns. Release
these scratch owners at their last use. Published meshes, shadow/query geometry
and skinning definitions retain their independently owned runtime backing.
Original compiled-input ownership ends at 14025aa00's exit; host decoding
scratch is separate from resident model geometry. Expected benefit: no overlap
between these scratch surface vectors and later acquisition, not lower gameplay
CPU or an asserted RSS delta. Report released vertex/index vector capacities
(excluding strings, structs, hash tables and allocator overhead) on the existing
production loading route. No extra conversion, copy, cache or benchmark.


Cohesive UI retirement acceptance: lifecycles-ui-image-retirement library check,
game-bin check, dev build and EotW -> Dust -> EotW production client all exited
0. This binary includes Close clearing last_frame. Registry purges again
released 2 then 1 entries, retained zero common entries in this route, and had
zero live wrapper entries at purge. Background bindings and all three loading
gates completed; one device/window/catalogue/audio output was retained. No
reported runtime errors, CPU benchmark, test or visual inspection. UI source
commit c1c8760 is integrated into main; its release build was started separately.


Particle binding/streaming prerequisite prediction (before implementation):
ParticlePass::material creates a new render pipeline for every material/state
binding, including sampler-driven recreation. The backend pipeline depends on
this pass's fixed shader/layout/formats and the tuple (draw state, accumulation,
distortion, effective lighting mode); material uniforms, names, textures and
samplers do not specialize it. Reuse pipelines by that exact tuple within the
pass, leaving image/material bindings independent. This is a host wgpu pipeline
cache, not an original API-level port: original prepared-program identities are
retained separately from material bindings (1402c9f00/1402ca440 stage/variant
acquisition). Expected creation calls equal distinct specialization tuples,
not material binding count; sampler changes create zero additional pipelines
when those tuples already exist. No extra GPU pass, altered shader constants,
format, state, draw or descriptor binding. Report request/unique/reuse counts at
pass release; no CPU-time claim from counts. This prerequisite prevents future
streamed-image binding refresh from recompiling shaders per texture change.


Particle binding refresh prediction (before implementation): isolate material
image-view/sampler binding creation from its uniform upload and pipeline lookup.
When the canonical surface sampler changes, rebind existing material variants
instead of clearing them and constructing new uniforms/pipelines. Expected
per sampler change: one binding recreation per resident material variant,
zero new uniform buffers/uploads and zero pipeline lookup/creation requests.
Reuse the same map allocation while refreshing; restore ownership even if a
binding reports an error. This also supplies the required binding-only entry
point for future image residency changes. All binding indices, fallback views,
shader parameters and draw states stay the same. This host descriptor refresh
stands in for the original resource/material separation (14005feb0, 1402c9f00).
Counts distinguish exercised sampler changes from the normal loading route;
no CPU milliseconds or sampler-switch coverage is inferred from compilation.


Particle image identity preparation (before implementation): idResource owns its
resolved resource name (+8); image/material variants refer to that identity
rather than an allocation address. Preserve the already resolved particle image
path in the acquired resource and share its immutable text with the acquisition
map key. This adds no image read or decoded copy and permits source-to-scene
identity matching without a second name-resolution pass. Backend address-based
GPU caching is not claimed to be replaced until the shared-image binding route
is wired. No memory/CPU saving is predicted from path metadata alone.


Particle shared-image route prediction (before implementation): enable the scene
image owner for 2D particle resources in the gameplay acquisition context, with
standalone acquisition remaining explicit. Keep only the 48-byte BIM metadata
needed by BimageInfo::parse at particle-definition acquisition; the scene owner
reads/uploads the compiled base chain once, streams finer mips and drops staging.
Intrinsics use the existing generator/format metadata. Scene-owned definitions
must never invoke the custom particle GPU upload or fall back to empty payloads.
3D volumes remain on their dimension-specific route until separately integrated.

Register canonical 2D paths in the scene's existing image slots, publish committed
handles to the render world with shared immutable snapshots, and refresh only
material bindings when those views change. Preserve pending/old bindings until
new GPU resources are prepared. Particle visibility already queries live system
bounds; retain its flag-zero projected area (14031ad30 / 14031a510) and publish it
to each accepted material image, rather than submitting whole-screen demand.
Reuse the existing projected-area atomic publication and upload/pressure budgets.
Expected custom 2D GPU uploads: zero for the shared route; CPU definition payload
residency: metadata only. GPU residency moves to the scene owner, not an assumed
16 MB total saving. Finer requested mips can increase detail/residency legitimately.
The existing world-material particle proxies remain separately identified until
their material-slot demand is connected; no claim of complete particle parity.


Particle model bounds source confirmation: 1401fb4d0 constructs one
arkRenderModelParticleSystem (a render-model subclass). 140201480 writes its
whole-system authored/live union to +0x1840/+0x1850; 1401c3890 supplies authored
bounds and +0x1e58/+0x1e64 supplies the live renderer union unless fixed-box is
set. The existing particles::bounds::system_bounds implements that union and
1efbe0 descriptor-radius handling. Thus texture demand uses the system model's
bounds, not a guessed screen radius per sprite. RenderAabbQuery::visible already
calls visibility_and_area on the no-grid branch; retaining that area avoids a
second query. Render-to-main feedback atomically takes a separate entire packed
publication word (including explicit zero demand) and submits to the persistent
scene record; native conditional area clearing stays with the existing streamer.

Completed preceding batch: the EotW -> Dust -> EotW production run exited 0.
Temporary model vertex/index capacities retired before later acquisition were
30,158,970 / 26,266,368 / 30,158,970 bytes, excluding metadata/allocator overhead;
no RSS delta is assigned. Particle pipeline requests 72/94/72 used 18/16/18
unique pipelines, reusing 54/78/54. That run reported zero binding refreshes, so
it does not prove sampler-switch refresh coverage. The new shared-image route
and metadata-only particle acquisition are not certified by that earlier run.
The c1c8760 main release build completed with exit 0.


Prepared-renderer demand source correction: 1401fbad0 publishes all prepared
renderer surfaces into arkRenderModelParticleSystem; 14031ad30 walks that list
at chunk_000119.c:6300 without an instance-count test before 14031a510. The first
per-draw feedback omitted inactive renderer prefetch. Prepare a deduplicated
image list for the selected declaration's enabled FX renderers and publish once
per visible system. World-material mesh renderers need the same treatment:
bind their already-resolved scene image slots to declaration material names,
cache those image references on the system's first publication, then submit the
same system area. Remove their permanent whole-screen fallback while retaining
the projected-decal fallback that shared the old list. Expected work: one atomic
publication per unique image of a visible system, no per-particle image lookup,
no root-graph clone, no image-payload duplicate, and no world-material image
held at whole-screen demand just because its definition exists. Preserve the
rendered geometry/materials. No CPU-time or RSS saving is assigned.

The special +0x1bf0 exclusion in 14031ad30 is auxiliary render data populated by
volumetric overrides (14028bfa0/14028c100), not the particle constructor
1401fb4d0, whose base starts the pointer null. Thus it does not invalidate this
particle surface-material traversal. First shared-route game compilation found
two integration defects (system arity and an omitted state field), corrected
before any build was delivered; that failed check is not acceptance evidence.


FX dispatch borrowing prediction (before implementation): 1407c0e30 obtains
existing node/input pointers via 1407bbec0/1407bbf80 and invokes callbacks;
1407bb9d0 inserts declaration entries through existing definitions. Runtime::step
and Instances::insert instead deep-clone immutable Node parameters/ports/links;
emit clones its output links, and insertion clones matching entry nodes into a
Vec. Hold a local definition Arc and borrow those nodes/links while mutating only
runtime state. Replay borrowed entry iteration for the existing two passes.
Expected copies: one Node per step -> zero; one Node plus entry copies per
insertion -> zero; one link Vec per emit -> zero. Preserve lookup errors,
authored order, owner overrides, queues, callbacks and mutable variables/gates.
No CPU-time or resident-RAM saving is inferred from these transient-copy counts.
The prepared change waits until the current image build snapshot is finished.

Adjacent queue sweep: take_pending transfers its Vec so recursive insertion can
run without holding a runtime borrow, potentially losing capacity at flush
boundaries. A recursive scratch strategy needs active allocation/depth evidence;
do not add a shared locked buffer or reorder insertion. Dynamic parameter/entity
snapshots remain owned because runtime callbacks may retain their values.


Shared particle image acceptance (source 3dd8a11): game-bin check, dev build and
EotW -> Dust -> EotW production execution exited 0, with one catalogue/device/
window/audio output. Scene bindings covered 57/48/57 distinct 2D identities;
custom particle uploads were 9/10/9, all volumes, versus 59/54/59 previously.
Their uploaded/retired staging bytes were 13,729,792 / 28,311,552 / 13,729,792.
Those bytes move only for the remaining 3D owner; 2D storage moved to the scene
owner, so this is not a claim of equivalent total GPU memory savings. Acquired
particle-definition bytes were 13,732,480 / 28,313,808 / 13,732,480, including
metadata for scene-owned images. Two binding-only refreshes occurred in the
return EotW pass; the current combined counter does not identify sampler versus
image-view causes. No reported runtime errors. No CPU benchmark, tests or visual
inspection. Source integrated into main; release delivery build started.

Active volume evidence: all captured 3D flipbooks had one authored mip, full XY
equal to inline XY, and zero indexed separate levels. Dimensions ranged from
128x128x4 to 512x512x64 (also 256x256x128). They retain all authored data and
retire CPU upload staging; forcing these into a 2D mip scheduler would invent
absent levels. This verifies these consumed volumes in two levels, not every
volume asset in the installation. Other image families and cache policies remain
within the broader goal, including complete installed-volume classification.


Acquisition-cache admission prediction (before implementation): scene image
registration currently iterates every image acquired while preparing particle
materials, including any temporary/overridden image that only its cache retains.
After all definitions/proxies are prepared, retire cache-only declarations and
materials first, then cache-only model/field/image backing. Live definitions and
world-material binding lists already own their Arcs. Register only the remaining
2D image identities. This implements the owner-release boundary of 140060180
using actual consumer ownership rather than guessing resource age or paths.
Expected invariant: no cache-only image becomes a scene GPU asset. Count retired
entries and their acquisition payload bytes; do not presume a nonzero count or
infer RSS savings. Runtime definitions, future stages and active bindings keep
their references, so prefetch and acquisition latency are preserved.


Installed-volume classification (header-only acquisition audit, 2026-11-04):
scanned the final patched PC bimage7 selection across all four installed index
groups: 14,885 image headers, 102 volume images, zero read/header errors. Every
volume has exactly one authored mip, matching full/inline XY, and no indexed
separate mip levels. Thus the remaining volume owner retains the complete
authored chain; there is no finer installed volume data to stream. This extends
the earlier two-map observation without rendering assets or running CPU tests.
Original 1402e9ff0 / 1402f4060 loads/uploads the base; 14030dbf0 registration
requires the current count to be below the full count. Descriptor finalization
1402e98a0 preserves an explicit one-mip count. This does not certify unrelated
image-family ownership. The local report is artifacts/lifecycles-installed-volumes.json.


Resource-consumer acceptance (compiled source d372bf8): library/game-bin checks,
dev build and EotW -> Dust -> EotW production execution exited 0. Cache-only
retirement removed 42/41/42 declarations, 2/0/2 materials, 5/0/5 models and
3/0/3 images; no field images retired. Acquisition image payload bytes were zero
for these metadata-only entries. Scene particle 2D identities became 54/48/54
(previously 57/48/57); custom volume uploads remained 9/10/9 and staging bytes
13,729,792 / 28,311,552 / 13,729,792. Thus the observed admission reduction is
three unused EotW images, none in Dust; no total GPU byte or RSS delta assigned.
Particle pipeline specialization counts remained 18/16/18, with 54/78/54 reuse.
FX step/insertion/emission now borrow immutable definitions instead of cloning
Node/link data; mutable instance storage remains separate. This run verifies
compilation and map lifetimes, not a targeted CPU-time improvement or a visual
comparison. No tests or CPU benchmarks were run.


Full-mip image owner consolidation prediction (before implementation):
1400607f0 first returns the existing 14005feb0 image identity, and 140060180
purges map-class resources independently of consumers. UI, fog, lens and
area-light preparation currently have separate full-mip path maps despite using
the same reader. Route these through one session registry, retaining init/common
entries across map purge and explicit movie/frame leases. Keep image IO outside
the registry mutex and publish one canonical Bevy handle even if acquisitions
race. Store dimensions with the identity so an off-thread prepared image can be
bound before batch publication without blocking the window thread. Cloud
sampling already uses the backend's explicit bilinear-wrap sampler; its mutable
per-image sampler override is redundant and must not mutate a shared resource.
Expected result: same-path full-mip acquisitions publish/upload one image rather
than one per consumer cache; redundant loads and logical uploaded bytes fall by
the observed overlap, whose size is not yet measured. Stable gameplay adds no
registry lookup, lock or GPU pass. Count production acquisitions, reuse and
map-purge identities using the separate-cache diagnostic in the same build.
No walking CPU or total RSS improvement is predicted. Scene-streamed images
remain separate until full-resolution consumer pinning is connected; full-mip
residency must never be silently replaced by a reduced scene mip.


Delivery: main source 63f5d1d completed its release build with exit 0. The
subsequent full-mip registry change 3be3ac1 has passed library and game-bin
compilation checks; its production acquisition/purge comparison remains pending.


Full-mip registry acceptance (compiled source 3be3ac1, same dev executable):
library/game-bin checks and build exited 0; separate/shared production clients
both completed EotW -> Dust -> EotW with exit 0 and no fatal/error lines. Full-mip
image asset publications/uploads fell 39 -> 23, logical payload bytes
11,761,316 -> 9,716,732 (2,044,584 bytes removed), with the same 14 distinct
requested paths. Removed repeated publications were _black (5), _default (3),
_white (2), chroma_noise_128 (2), default_foglut (2) and lens_dirt_1920_1080 (2).
These are image-owner asset/upload counts, not an independent archive IO timing,
whole-process RAM delta or physical GPU heap measurement. Shared map purge
released 7 then 6 registry entries, retained 3 then 4 common identities
(1,017,172 then 1,191,932 logical image bytes), and retained no stale map-only
registry entry. Movie bindings reported the authored 1920x1080 backgrounds;
particle geometry/material lifetime counts are recorded in the same logs.
No visual inspection, new test cases or CPU benchmarks. Existing diagnostic
constructor calls were mechanically adjusted to pass the new owner argument.
Local reports: artifacts/lifecycles-resident-images-{separate,shared}.* and
lifecycles-resident-images-comparison.json. Scene-streamed/full-mip identity
remains the next separate policy boundary: a full-resolution consumer must
pin that image rather than receive a reduced scene chain.


Inline image staging prediction (before implementation): 1402e9ff0 walks the
compiled mip records and passes their payload pointers to 1402f5500; it does not
need a second packed CPU chain between the file and per-mip upload. The host
read_streamed_diffuse_at_target currently constructs compressed_inline_chain's
owned payload, then copies every mip to a separate CompressedMipUpload Vec.
Expose the same validator through borrowed record visitation; keep at most 16
borrowed descriptors on the stack and copy directly into the required cross-
thread upload buffers. Keep the owned-chain API for consumers that need one
packed chain, using the same validator. Expected removed copy/live temporary
bytes per acquisition: exactly the sum of inline mip payload sizes. GPU upload
count/bytes, mip shape, selection and validation errors remain unchanged. Use
the same production executable's copy control and acquisition checksums/counts;
no walking CPU timing or total RAM delta is predicted.


Particle device-program lifetime prediction (before implementation): global
serialized program cache construction 140064e00 -> 1402c8d50 is session-owned;
1402c9f00/1402ca440 reads cached serialized shader data independently of image
residency. These functions do not themselves prove GPU object persistence.
Retaining compiled backend objects is an explicit host reuse improvement. The
current ParticlePass caches pipeline specializations within a level, but map
release drops its shader module, layouts and pipelines. Keep only those immutable
objects in CameraGpu's device-scoped cache, keyed by colour/depth formats, and
transfer their handles into the next pass. Existing specialization keys remain
unchanged. Material/image bindings, geometry, buffers and mutable simulation
continue to retire with the outgoing map. No new per-draw lookup or mutex.
Expected same-format return EotW: zero new shader modules/pipelines for already
seen states instead of 18 pipelines; Dust only compiles states absent from the
existing cache. Record actual creations and hits under a same-binary recreate
control. Counts establish removed compilation operations, not total driver RAM
or CPU timings. This explicitly increases residency of useful compiled GPU
objects to avoid recompilation, separate from the original's proven serialized-cache lifetime.
Prepared source is local-only until the current inline staging snapshot completes.


Inline staging acceptance (compiled source 60c92eb): library/game-bin checks,
dev build and copy/borrowed production clients exited 0. Both EotW -> Dust ->
EotW runs recorded exactly 11,840 image acquisitions across 4,959 image/target
identities, 1,102,163,164 prepared upload bytes. Every identity's byte count,
mip count and payload/descriptor checksum matched; no missing/extra identity
and no fatal/error line. Redundant packed-chain copy bytes were
1,101,966,556 -> 0; the largest single removed packed payload was 16,777,216
bytes. This measures cumulative copying and the removed per-acquisition
temporary, not a 1.1 GB resident-memory or measured CPU-time saving. Borrowed
descriptors occupy at most 16 stack entries; required cross-thread upload
buffers still own their payload. All reader validation remains shared with
compressed_inline_chain. Local evidence: artifacts/lifecycles-inline-staging-
comparison.json and the two production logs. No tests or visuals inspected.

Particle program batch first game compilation found a missing TextureFormat
qualification in CameraGpu's new cache field. Fixed before any delivery build;
the failed check is not runtime acceptance evidence.


Particle program acceptance (compiled source 547006e): corrected game-bin check
and dev build exited 0; recreate/reuse production clients both completed EotW ->
Dust -> EotW, exit 0, no fatal/error lines. Recreate compiled 18/16/18 pipelines
(52 total) and three shader/layout sets. Reuse compiled 18/0/0 (18 total), with
one shader/layout set; Dust's 16 states were already in EotW's 18-state set.
The return EotW reused all 18. Per-map pipeline requests remained 72/94/72;
volume image uploads remained 9/10/9 and 13,729,792 / 28,311,552 / 13,729,792
bytes in both runs. Material bindings and CPU upload staging still retire with
the map. Cache keys include colour/depth formats; format replacement preserves
separate specialization sets. This is host compiled-object reuse, not evidence
that 1402c9f00 (serialized cache lookup) retains GPU objects. No CPU-time or
physical GPU/driver memory reduction is inferred. Local evidence:
artifacts/lifecycles-particle-programs-comparison.json. No tests/visuals run.

Delivery: main source 596c051 release completed with exit 0 before integrating
the accepted inline staging and particle program changes.


Whole-image buffer prediction (before implementation): full-mip and base-image
preparation still copies each validated record into CompressedMipUpload buffers,
then joins those buffers into the final contiguous Image data. Streamed jobs
need per-mip backing across threads, but whole-image consumers do not. Use the
same reader/validator and select direct packed output for those consumers,
reserving the final authored chain size at its first mip. Expected temporary
per-mip payload buffers for whole-image preparation: N -> 0, with one final
image buffer in both modes. Initial/separate mip selection, descriptors and
final bytes must remain identical. A split-output diagnostic restores the old
assembly in the same binary; record final payload CRC/descriptor equality and
intermediate buffer counts/bytes. No whole-frame CPU or RSS gain is predicted.


Whole-image packing acceptance (compiled source b37d03b): library/game-bin
checks, dev build and split/packed production clients exited 0. Both EotW ->
Dust -> EotW clients prepared 11,852 whole images across 4,963 image/target
identities, with 1,102,175,452 final payload bytes. All final CRCs, dimensions,
mip counts and GPU formats matched; no missing/extra identity or fatal/error
line. Intermediate per-mip buffers fell 80,400 -> 0 and their cumulative
payload bytes 1,102,175,452 -> 0. One final Image payload remains in both modes,
with the largest 16,777,216 bytes. These are eliminated intermediate ownership/
copying counts, not allocator RSS or a claimed 1.1 GB permanent saving.
Streaming exact-mip jobs still use their required independent payloads. Local
evidence: artifacts/lifecycles-image-packing-comparison.json. No tests/visuals.

FX descriptor borrowing prediction (before implementation): 2263f0 and the
existing typed FX constructors establish defaults before serialized partial
edits; the shared descriptor_list preserves count, ordering and omitted slots.
world_sources::descriptor_fields unnecessarily deep-clones each selected Fields
subtree before its typed parser consumes it. Add the input lifetime to the
existing decoder and use borrowed Cow slices at that boundary; empty/default
slots remain empty borrowed fields so typed constructors still supply defaults.
All four consumers (groups, particles, decals, deferred family admission) read
these fields immutably. Expected cloned syntax bytes/array: the recursively
cloned field/string/vector payload -> zero, with descriptor metadata storage
unchanged. Typed runtime definitions stay owned. Use the same production
binary's explicit copy control and record selected-array/field bytes plus
prepared particle/renderer admission counts. No CPU-time/RSS saving predicted.


Adjacent UI audit reconciliation: the optimization table's raw-byte Iggy cache
and whole-page GPU glyph upload findings were stale. flash::Player::load_installed
now calls assets::iggy::acquire and receives shared Rc<IggyFile>; the three-map
logs preserve library identities across movies. GPU glyph pages update dirty
rectangles using dirty_base/version (14107b7e0 / 14107b2d0), falling back only
when an older version was skipped. CPU page snapshots preserve immutable
in-flight draw-list versions and copy only when Arc ownership overlaps; that
is required backing, not redundant per-frame conversion. FX dispatch borrowing
was likewise already implemented/accepted; its stale opportunity is closed.
The separate gradient-cache question is not resolved by those changes:
1410947c0 generates a 64x1 premultiplied ramp and 141095740 routes it through
GDraw callbacks; the current production pause run retained one content-cached
ramp, while loading-only runs retained none. No animated-ramp growth is proven.
Do not infer texture ownership from 14108a580/14108a6b0: those are audio slots.


FX descriptor acceptance (compiled source 440fb6e): library/game-bin checks,
dev build and copy/borrowed three-map production clients all exited 0. Both
modes selected 941 descriptor arrays / 616 entries, 631,105 logical field-tree
bytes and 58,368 bytes of descriptor-vector capacity. Deep-cloned syntax bytes
fell 631,105 -> 0; metadata capacity and prepared particle identity counts
54/48/54 (2D), 9/10/9 (volumes) matched, as did cache-retirement counts. There
were no fatal/error lines. This is transient syntax-copy removal, not a 631 KB
resident saving or measured CPU-time improvement. Omitted/default slots and
error/order handling still run through the same shared descriptor decoder.
Local evidence: artifacts/lifecycles-fx-descriptors-comparison.json.


Probe preparation retirement prediction (before implementation): installed
PRB loading/sample decoding is 1402b99a0 / 1402b9720; native Voronoi query paths
140382d90/140383d80/140383750/140382f90 read their source representation. The host
already precomputes CellGeometry quads for fast runtime queries, but keeps the
primary sample vectors and cell occluder index lists used only by that build.
Secondary reflection samples (including alternate-result sample vectors) have
no runtime consumer; their reflection planes/names/positions/SH remain needed.
After gameplay probe preparation, release only these superseded sample/index
vectors. Standalone installed-resource readers retain the complete format.
Expected capacity bytes retired: sum of primary/reflection/alternate sample
Vec capacities times sizeof(Sample), plus cell occluder-index capacities times
sizeof(usize). Prepared quads, spatial planes/neighbors/tetrahedra, SH and GPU
reflection data remain unchanged; no query recomputation or per-frame work.
A retain control in the same production client records exact before/after
capacities and prepared geometry counts. Do not assign MB before measurement.
Original RESOURCE budget 51,830,920 bytes includes multiple resource families;
it is not a separate measured probe tag. This is a host representation-lifetime
improvement, not an assertion that the original discards its live query samples.


Probe preparation acceptance (compiled source ae19ab7): library/game-bin checks,
dev build and retain/retire EotW -> Dust -> EotW production clients all exited 0,
no fatal/error lines. Retired capacities per map were:
- EotW: primary samples 20,348,928 + reflection samples 638,976 + cell occluder
  indices 2,437,760 = 23,425,664 bytes; each capacity became zero.
- Dust: primary 17,356,800 + reflection 681,984 + indices 1,931,232 =
  19,970,016 bytes; each became zero.
- Return EotW: same 23,425,664 bytes. Neither map had alternate sample capacity.
Retain mode kept exactly those capacities. Prepared geometry remained
3,312 cells / 213,317 quads / 3,312 primary and 104 reflection probes in EotW;
2,825 / 171,859 / 2,825 / 111 in Dust. Particle identities matched, and no
runtime query/SH/reflection upload path was changed. This is actual retired
Vec capacity, excluding allocator overhead and not claiming an equal RSS drop.
No walking CPU benchmark is needed for freeing vectors no gameplay reader uses;
no per-frame work or recomputation was added. Standalone PRB readers keep full
source data. Local report: artifacts/lifecycles-probe-preparation-comparison.json.


Named-image source clarification for the next registry integration: 1402df8c0
looks up the existing image through 14005feb0 and acquires its current SRV via
1402bd380; it does not itself request full mips or disable streaming. It stores
the wrapper's view and dimensions while that named wrapper has references.
1402bd790 includes first mip/count, layer range and format in the view key;
1402e9cb0 derives its dimensions from the selected mip. Therefore old leased
views and current source residency are distinct ownership scopes. A universal
full-resolution pin inferred from UI binding would be unsupported. Integration
must share equal source/view identities while preserving explicit snapshots
and streamer replacement/eviction, rather than silently changing image quality.


### Scene and named-UI image views: implementation prediction

1402df8c0 resolves the registered image through 14005feb0 and acquires its
current SRV with 1402bd380. Its named wrapper retains that snapshot until the
last binding reference is released; acquiring a name does not force full mips.
The host image owner will reuse an already acquired scene-base view only when
its initial complete-chain view selects the scene loader's requested source mip.
Already replaced/trimmed views are not treated as complete base chains. This
preserves the scene minimum/residency policy rather than pinning full chains.
Later UI acquisitions use the committed scene/common view; existing wrappers
retain their previous view. Registered current views change only on successful
streaming commits, including inactive common-image reductions.

Prediction: each eligible base slot removes exactly one independent payload
preparation and upload, equal to that registered view's payload bytes. Mip and
payload output must stay identical. No whole-frame or CPU timing improvement
is predicted or required; validation uses production acquisition/upload counts
and image descriptors/checksums. DH2_DEBUG_SEPARATE_SCENE_IMAGE_VIEWS restores
the former independent acquisition for the same-build comparison.


Scene/named-UI integration acceptance (c038705 source snapshot): library/game
bin compilation checks, dev executable and both production EotW -> Dust ->
EotW clients exited 0. Independent/shared scene preparation acquired 11,852 ->
11,849 images, with 1,102,175,452 -> 1,102,172,380 cumulative payload bytes.
Exactly one `_default` 16x16 RGBA8 single-mip image (1,024 bytes, CRC 75c34c05)
was reused on each map, matching the one-upload-per-eligible-slot prediction.
The fingerprint multiset lost only those three identical duplicates; it gained
no changed/new descriptor or payload fingerprint. This is a small identity
correction, not a substantial process-memory or CPU-performance result.
Registry purges still release 7 then 6 map entries and retain 3 then 4 common
entries. New UI requests select current committed scene/common handles; cached
wrappers keep their leased view. No extra stable-frame registry scan was added.
Source-mip mismatches and previously trimmed registered views do not override
the scene's minimum chain. Local evidence:
artifacts/lifecycles-scene-image-views-comparison.json. No tests or visuals run.

Delivery: the preceding main 90bbcf2 release completed with exit 0.


### Sound property-chain ownership: prediction before implementation

Original first-owner actor registration (`1410f2370`/`1410f2270`) and node
initialization (`1410de430`, `1410deb00`) retain authored properties on actors.
The rewrite repeatedly decodes the same actor/parent hierarchy in
`PlaybackBanks::node_hierarchy`; `Definition::decode` clones the root hierarchy
again while converting the tree into runtime definitions. Immutable flat
property chains can share an Arc allocation, acquired through weak per-bank
lookup, without changing iteration order/layout of the NodeProperties array.
Weak lookup must not pin unused definitions; map-bank truncation must reject
chains referring to removed actors. Runtime mixer parameters, random values,
voice filters, selections and cursors remain independent.

Prediction: repeated live acquisitions of a root use one chain allocation;
root clones copy no property vectors in shared mode. The exact saved retained
bytes are the difference between actual live chain allocations/capacities,
including nested values/ranges/curves/effect slots. No MB or CPU-time gain is
guessed from the coarse sound-preparation allocation tag. Production diagnostic
weak tracking will count those allocations and fingerprint requested property
contents in both modes; it owns no chain payload. No steady-state property
evaluation algorithm, PCM decoder or output routing changes.


Sound-chain sharing acceptance (32c3909 source): library/game-bin compilation
checks, dev build and copied/shared production clients all exited 0. Six
EotW -> Dust -> EotW preparation checkpoints matched all requested-chain
content fingerprints, request counts and identity counts. Final prepared chain
allocations/owned bytes were:

| Level | Independent chains | Shared chains | Removed chain bytes |
|---|---:|---:|---:|
| EotW | 30,302 / 43,882,288 | 12,822 / 17,602,144 | 26,280,144 |
| Dust | 29,521 / 42,336,448 | 11,881 / 15,857,296 | 26,479,152 |
| EotW return | 30,302 / 43,882,288 | 12,822 / 17,602,144 | 26,280,144 |

These are actual flat Arc slice/header and nested Vec capacity bytes from live
weakly observed chain allocations; the weak lookup table/diagnostic metadata
and allocator reservation are excluded. No whole-process delta is claimed.
Across three visits, created chain allocations fell 102,820 -> 41,802 and
cumulative allocated chain bytes 145,011,880 -> 56,543,144. The prediction is
confirmed: live duplicates share their existing allocation, and root clones no
longer deep-copy the chain. Weak lookup does not retain otherwise unused data;
map truncation removes keys/chains depending on removed actors. EotW return
retained exactly its first-visit counts/bytes. Encoded bank/source residency
was unchanged at every checkpoint (209,705,529 EotW and 217,113,796 Dust unique
bank-backed bytes). Runtime property evaluation still walks the same flat
NodeProperties slices; no new locks/refcount operations enter those loops.
Local evidence: artifacts/lifecycles-sound-properties-comparison.json.
No tests, visual inspection or CPU benchmarks were run.

Delivery: main 4861bb1 image-view release completed with exit 0.


### Init/common animation reuse: prediction before implementation

The previous one-map animation-cache candidate was correctly reverted: 200
requests were 200 distinct clips. That does not cover the session boundary.
Installed manifests contain 2,641 anim entries in common and none in init;
`1400661e0` acquires init/common before maps and `1400607f0` returns existing
resource identity. `14024adf0` owns the compressed animation blob at +0x48;
`1412e2c30`/`1412e6a70` sample that backing.

Expected change: immutable Arc clip data; acquire by resource path, pin only
requested init/common clips, weak references for map-only clips, and prune weak
lookup entries on map release. Per-character clocks, poses, blending and
validation against each skeleton remain independent. Classification shares the
already acquired manifests and keeps sorted integer indices, not another set
of copied resource-name strings. No cache lookup/lock belongs in sampling.

Prediction: first-visit distinct acquisition is unchanged; each later request
for a still-live/shared clip removes one archive read and parse. If the same 200
common player clips are requested over all three maps, reads fall 600 -> 200.
Record actual path/identity reuse and clip-content fingerprints; adjust that
prediction from the measured common/map classification rather than retaining
unrequested animations. No steady-state CPU or RAM saving is predicted: this
keeps the already required compressed allocation across the load boundary.
Production acquisition can also transfer its owned archive buffer into the
clip instead of making a second blob copy; the borrowed standalone parser
remains available and must produce identical clip contents.


Tome retained-table finding resolved from existing performance evidence:
commit a34a947 (2026-10-31) introduced load-time traversal/object tables
together with fixed query scratch and hashing changes. Its same-view moving
camera traces measured query CPU 826 -> 724 -> 682 ms/s, with 3.05 -> 2.30 ms
median query span; the accepted visible count remained 4,927/100,169. The
combined gain is not attributed exclusively to any one decoded table. Current
source d05a6f5 already descends packed cell trees in place; the remaining
decoded tables have actual query consumers and preserve that accepted runtime
representation. They are not data awaiting retirement after parsing. A new
packed-query redesign would require its own affected-query performance proof;
this resource audit does not undo the measured implementation. Source blob,
prepared tables and worker Arcs all release at the map boundary. No new CPU
measurement, test or visual inspection was used for this finding.


Animation session-reuse acceptance (0c38214 compiled source): library/game-bin
checks, dev build and copied/shared EotW -> Dust -> EotW clients exited 0. Both
modes requested the same 200 identities 600 times; every requested source-byte
CRC, frame count/rate, flags and properties matched. Actual classification is
162 common clips and 38 map-only clips, so the conditional all-common 600 ->
200 prediction becomes exactly 200 + 38 + 38 = 276 reads/parses. Measured
acquisitions were 600 -> 276, 324 reuses, and archive bytes 3,360,864 ->
1,266,848. Only requested clips are retained; 2,479 other common-manifest clips
are not acquired by this owner.

Both map purges removed 38 map lookup entries, reported zero live map leases,
and retained the same 162 common clips / 1,121,838 owned allocation bytes. The
compressed allocation survives the load gap instead of being rebuilt; this is
a load/reuse result, not a resident-memory reduction. The owner shares manifest
metadata and uses integer indices for common membership. Production parsing
transfers the archive buffer after reading its metadata; the borrowed parser
uses the same header decoder and copies its blob. Keeping the owned buffer's
trailer capacity can add a few bytes per clip; no whole-process or steady-state
CPU gain is claimed. Per-rig pose validation still runs on acquisition.
Source sampling/pose algorithms and per-character time remain unchanged.
Local evidence: artifacts/lifecycles-animation-residency-comparison.json.
No tests, visuals or CPU benchmarks were run.

Delivery: main 4efdc12 sound-property release completed with exit 0.


Original registry capture: `analysis/capture_resource_residency.py` reuses the
existing verified full-Steam process reader and reads memory only. Native
`140060180` verifies registry array/count offsets and resource-class +0x24;
`140060aa0` verifies lookup through that same array. Initial process snapshot
contains 643 type registries / 23,534 registrations. Image registry contains
1,548 entries: 1,400 class-2 (1,388 GPU-backed, 12 without GPU resources), and
148 class-1 GPU-backed entries; every GPU-backed image has valid dimensions.
This snapshot predates the requested loaded-level observation and must not be
presented as EotW residency. It establishes actual name/scope/view descriptors
where the previous per-tag budget had totals only. Asset names and process
pointers remain local in artifacts/resource-residency-initial.json. No input,
visual capture, breakpoint, in-process command or GPU pass is used.


### Tiled-light upload staging: prediction before implementation

The ported `1402f7260` producer already prepares 112-byte lighting records and
48-byte cull packets; shadow records are 144 bytes. The host uploader then
allocates byte vectors for lighting/shadow records, and the packet uploader
allocates both flattened words and serialized bytes. None of those intermediate
buffers has a consumer after `RenderQueue::write_buffer` copies its input.

On little-endian hosts, borrow the contiguous u32 arrays as bytes; preserve
explicit little-endian serialization for other targets. Borrow flattened packet
words directly from the existing fixed stack array. Expected removal per
nonempty preparation: one temporary allocation per lighting/shadow upload and
two per cull upload, with copied bytes 112*L + 144*S + 96*C -> 0. Required GPU
staging and submitted record bytes remain unchanged. Diagnostic copy mode
restores both former allocations. Counters will report per-upload values in
the production client; no CPU-time claim is predicted.


Initial image descriptor comparison (before the requested loaded-level capture):
1,425 native image names resolve through installed PC bimage7 source aliases.
Header-only reads reported zero errors. No resident image was below its
authored inline extent; 125 were above it (67 class-2, 58 class-1), confirming
that persistent/common scope does not mean a fixed resident mip. The remaining
non-intrinsic name is the runtime-generated save screenshot placeholder, not
an absent compiled texture. This supports the existing base-chain minimum and
independent common-image pressure policy, without substituting this startup
snapshot for map residency. Native first-view mip zero refers to the current
GPU allocation and does not imply the highest source-file resolution (for
example, an authored 512x512 source can have a 256x256 allocation). Local
evidence: artifacts/lifecycles-initial-image-descriptors.json.


### Cube/array image identity and staging: prediction before implementation

WaterImageSources already canonicalizes authored aliases to one Arc chain per
resource. Its three consumers nevertheless upload separate immutable textures
via LayeredImage::cube/array (placed water, ocean, underwater scattering). That
violates the existing-image lookup boundary `1400607f0`/`14005feb0`; typed SRVs
are cached by `1402bd380`, not separate pixel allocations per consumer.

Give each acquired backend source one immutable GPU result and transfer its
compressed staging into that upload. Keep only descriptor/identity afterward;
consumer leases hold the same GPU image. Cube and CubeArray views share its
texture; samplers and material bindings stay consumer-owned. Source acquisition
and all parser/format/mip/layer checks remain unchanged. Consumers only sample/copy from these images; they do not issue texture updates
or use them as storage/render attachments.

Prediction: N consumers of one source create one texture/upload instead of N,
removing (N-1)*payload bytes of duplicate uploads/storage. Used source staging
capacity retires once after upload rather than persisting for the whole map;
unused prefetched source staging still belongs to map acquisition. GPU images
and source identity release with the map and outstanding frame leases. Resize
or quality changes must not upload an unchanged source again. The diagnostic
separate mode retains the former uploads and source staging in the same client.
Validate production source fingerprints, dimensions/mips/layers, upload counts
and source retirement; no GPU-time or physical-driver-memory claim is inferred
from payload bytes.

The same ResidentImages acquisition registry owns these typed sources. It
retains init/common source identities and purges map entries at the existing
class boundary, so per-pass sharing also survives map changes for explicitly
common images. The source's cached GPU result survives view resizing; its CPU
payload does not. Standalone borrowed-chain upload helpers remain available,
but production consumers request the acquired source.


Intrinsic image scope finding: native `14005fde0` registers builtins with
resource class 2. The readonly original snapshot confirms `_default`,
`_defaultcube`, `_white`, `_black` all have that class. They have no installed
index entries and cannot appear in the archive-backed common-path set. The
current purge therefore incorrectly treats acquired builtins as map images.
Use the existing generator's exact supported-kind classifier for both resource
resolution and persistent ownership. Prediction: acquired builtin identities
survive map purge, removing one recreation per affected builtin on later maps;
compiled-file IO is unchanged. Classification must allocate no pixels just to
answer whether a name is an intrinsic; pixel generation itself stays identical.


Procedural image scope prediction: `1402f6450` creates the renderer's
`_perlinNoisePerm2D` (256x256 RGBA8_UNORM) and `_perlinNoiseGradients`
(256x1 RGBA8_SNORM). The readonly original snapshot shows class 2 for both.
WaterColourScene and OceanScene each call NoiseTextures::new independently.
Acquire these through the session image owner, lazily once on the renderer.
Each removed factory call saves two GPU uploads and 263,168 payload bytes;
all calls still use the unchanged permutation/gradient generator and samplers.
No per-frame lookup or refcount operation is needed. Separate diagnostic mode
restores per-consumer generation; record factory counts and payload CRCs.


### Verified typed-image and lighting-staging ownership (2026-11-04)

Source c9b2206 completed the production EotW -> Dust -> EotW route in both
copied/separate and shared modes, exit 0 each. No tests or visual inspection
were run. All three acquired cube/array resource identities matched format,
extent, mip/layer counts and exact compressed-payload CRC. This is resource
content/ownership evidence, not a pixel comparison or total process-RAM claim.

| Affected operation | Copied/separate | Shared | Scope |
| --- | ---: | ---: | --- |
| Cube/array uploads | 8 | 4 | Three-map route |
| Cube/array upload payload | 41,592,992 B | 36,000,928 B | Cumulative logical bytes |
| Caustics uploads | 5 | 1 | Same common source across consumers/maps |
| Procedural image uploads | 12 | 2 | One session factory instead of six |
| Procedural upload payload | 1,579,008 B | 263,168 B | Same generator CRC ad10a461 |
| TLF intermediate allocations | 1,016 | 0 | First map: exactly 254 preparations |
| TLF intermediate copy bytes | 3,710,432 B | 0 | Same first-map upload count/bytes |
| TLF required queue uploads | 762 / 2,930,144 B | 762 / 2,930,144 B | Unchanged first-map submitted record volume |

Shared sources retired 36,700,160 B of used CPU staging capacity during the
route. This includes vector capacity, whereas upload sizes above count payload
length. Unused prefetched Dust reflection-cube staging (8,388,608 B) and the
48-byte unused builtin cube remained until source release; they were not
uploaded just to make their CPU-buffer counter disappear. Per-map ocean cubes
were released and reacquired on return. Purges retained two persistent typed
sources (common caustics and builtin cube), and three intrinsic 2D identities.
No per-frame owner lookup, extra render pass, image precision change or sampler
change was introduced.

The TLF prediction is met: three borrowed record uploads replace four temporary
allocations per nonempty preparation. Later-map frame counts differ, so their
raw cumulative timing or copy totals are not compared as identical work. No CPU
or GPU time gain is claimed. The lifecycle launcher now removes benchmark and
profiling timers explicitly; its existing level-sequence driver already supplies
completion. The completed run had no CPU/render profiling enabled; benchmark
configuration was redundant and is not acceptance evidence.

Native scope/identity evidence is `14005fde0`, `140060180`, `1400607f0`,
`14005feb0`, `1402bd380`, and procedural creation `1402f6450`. Queue/Arc source
ownership is a host implementation of those boundaries. Local numeric evidence:
artifacts/lifecycles-image-ownership-comparison.json and the two completed client
outputs. Remaining 2D ocean masks still have a separate upload/CPU-staging path;
that is not claimed fixed by the cube/array result. A loaded-map native named
image capture remains needed before changing their mip/streaming policy.


### Resolution replacement: investigation before changes

Source inspection finds replacement Options for ocean colour/underwater targets,
one current camera configuration, and a host texture-cache retirement age of
three frames (`vendor/bevy_render-0.19.1/src/texture/texture_cache.rs`). This
predicts bounded live targets when revisiting the same window sizes. Reserved
allocator blocks can retain a high-water mark; they are not live textures.

Use a diagnostic-only production resize sequence, with no config writes, CPU
profiling or benchmark timer. Resize the ordinary Window resolution, then record
exact live GPU allocation counts/bytes by label after the actual camera viewport
matches and 120 gameplay frames have run at each size. Return to the starting
size twice. This is observation, not a pass/fail test or an optimization claim.
A rising live count for a repeated size requires tracing retained references;
a rising reserved heap alone does not establish a leak. Record image streaming
counts alongside target snapshots so texture-demand changes are not mislabeled
as target retention. No production resource policy changes are justified yet.


Resolution observation 64b2924 completed both EotW and Dust clients, exit 0,
seven snapshots each. EotW named viewport targets return to exactly the same
allocation counts/bytes at each repeated size; its increasing unlabeled image
bytes require separate streaming accounting and are not assigned to targets.
Dust exposes a concrete dormant-pass defect: after the 960x540 stage, the ocean
colour target remains alive at larger sizes when no surface patches are drawn.
Its cached foam/HDR bindings also retain the old depthWater and main a/b views.
At 1280x720 the extra old-size allocations include depthWater 3,538,944 B,
main a/b 4,784,128 B each, pre-water source and water HDR 2,686,976 B each,
foam targets 262,144 B, plus their uniforms, descriptors and opaque input.

Prediction before correction: invalidate incompatible ocean colour and
underwater viewport targets on viewport changes, even if the draw path returns
early. Existing colour/depth pipelines, authored images, growing patch buffers,
wave simulation and history remain owned. No render-pass/input changes are
needed: an active pass already rejects and replaces a mismatched target before
using it. Removing its dormant old-size targets should also remove their stale
main/depth references after normal queued work completes. Track actual target
retirements and repeated-size allocation differences in a same-binary
retain/invalidate comparison. No CPU-time gain is predicted; no image-resolution
policy changes are included.


### Water material preparation: prediction before ownership cleanup

The typed water owner still retains three preparation-only fields: a copied
MAIN name, the pre-specialization option dictionary, and copied declaration
bytes. Searches of all consumers find no runtime reads of those fields. Runtime
state, depth selections, colour selections and numeric/image parameter scopes
have separate typed owners and remain necessary. Native `1402d5210` constructs
material parameter scopes/program banks; `14029f890` selects those prepared
banks. This cleanup changes host retention, not that selection algorithm.

The MAIN, world-colour and model-colour constructors also independently call
the same colour-parameter parser for the same material/program pair. Share one
immutable Arc of those two parameter blocks per selected program during one
material construction. A different selected alpha program must retain its own
defaults; never share merely because the material name matches. The temporary
lookup itself drops before publication. Prediction: one parse/backing instead
of up to three for an equal selected program, unchanged block fingerprints,
no runtime lookup/lock and no change to per-view evaluated water settings.
Count actual constructed vs reused scopes and their owned capacity, and the
removed unused source bytes. No CPU-time or large-RAM saving is presumed.


### Fixed inline ocean images: prediction before consolidation

OceanScene currently uploads four authored 2D masks itself while the main-world
WaterImageSources retains their compressed chains. Move those acquisitions to
ResidentImages and its existing Bevy asset/upload path. Preserve Initial(None):
the inline base chain remains the ocean's requested view, rather than silently
using the UI loader's Initial(Some(0)) finer levels. Registry reuse requires an
equivalent view; different selected source mips retain distinct snapshot handles.
The existing image owner must retain/purge those inline views with the same
common/builtin/map scope, and equivalent base-chain scene views may reuse them.

Prediction: the ocean source table contains handles, not compressed pixel
vectors; preparation bytes retire through the already established render-only
asset extraction path. OceanScene only borrows prepared GPU views and performs
no image IO, parsing or uploads. Equal fixed-view requests reuse identity;
common/builtin masks survive map changes. No samplers, formats, source mip
selection, FFT inputs or draw passes change. Measure acquired image identities,
inline image descriptors/content CRC and map purge counts. No CPU timing gain
is predicted, and retained view snapshots must not force finer texture residency.


### UI image-cache bounds: prediction before implementation

Native fill submission `141089760` validates static gradient handles through
`14107cf80` (+0x78 callback), but invalidates morph-gradient associations through
`14107cea0` (+0x90) before generation. `1410947c0`/`141095740` regenerate the
64x1 ramp. Game initialization `140034ba0` configures texture-cache kind 1 with
2,048 slots and 0x1428000 nominal bytes through `1402df160`/`1402e3460`.
`1402e5350` charges image bytes to renderer+0x1008; `1402e38f0` uses a zero-byte
slot for an external image. `1402e6540` calls `1402e67e0` to retire old entries
when bytes or slots run out. Callback +0x50 advances upload row blocks; it must
not be mistaken for GPU texture destruction.

The rewrite already retires outgoing movie bitmaps/glyphs/named bindings, but
its content-keyed gradients have no bound. Add an owner for cached texture
entries using those installed slot/byte limits and least-recently-used inactive
entries. Current draw-list textures remain pinned. Host difference: wgpu command
buffers hold in-flight references, so eviction drops the cache lease without
forcing a GPU wait; a simultaneously active working set may exceed the cache
budget until it becomes inactive. White stays a persistent builtin. External
game-image views consume a slot but no duplicated image-payload charge.

Prediction: stable authored UI below the installed budgets uploads/evicts
nothing extra, and ordinary gameplay without a movie does no cache maintenance.
Inactive cache residency is bounded under dynamic content. No saving is
predicted for the already observed one-ramp pause view. Use real loading/pause
movies and existing diagnostics for acquisition/eviction counts and unchanged
content/draw submission; no synthetic test, invented lower limit or broad CPU
benchmark is required.


UI owner sweep also finds that the 16-byte stage transform is allocated and
uploaded on every UI submission although it depends only on the movie's logical
stage dimensions. Preserve its four exact float words, borrow a stack array,
and upload only when the dimension bit patterns change. Prediction: one
16-byte temporary allocation per UI submission -> zero, and uploads reduce from
one per submission to one per stage-size change. The binding/passes stay the
same. The diagnostic copy mode restores allocation and unconditional upload;
record submitted stage words and operation counts, not whole-frame CPU time.
Cache-pressure selection reuses a scratch vector of numeric age/ordinal/charge
records and removes entries in one pass; it does not clone every gradient key or
allocate one temporary vector per entry while trimming.


The UI sweep also finds that movie image retirement was gated by the first
successful vertex upload. A movie closed while an image was still awaiting GPU
preparation could already own cache entries without having that upload marker.
Track movie generation independently of vertex-upload identity, so close and
direct replacement retire movie-owned entries even after partial preparation.
Prediction: successful existing movies keep the same acquisition/submission;
partial preparation cannot leave bitmap/glyph/named-image entries behind after
close. Shared bounded ramps/white and reusable viewport/buffer capacity remain.


UI owner production acceptance (2026-11-04, source `9df316a`): game-bin
compilation and both actual EotW -> Dust -> EotW client sequences exit 0. The
sequence opens the real pause movie after loading; no synthetic workload or
CPU benchmark was run. Legacy mode records 2,992 stage submissions, 2,992
uploads / 47,872 bytes / 2,992 temporary allocations. Owned mode records 2,998
submissions, one upload / 16 bytes / zero temporary allocations. Both submit
identical logical stage-size bits and identical four float words. Submission
counts differ with asynchronous loading duration; the accepted metric is work
per submission and per stage change, not a frame-time comparison.

Both modes peak at 44 entries / 8,495,572 nominal payload bytes, below the
installed budget, and every movie close and final release leaves only the
4-byte white builtin. Pressure evictions are zero in both modes, as predicted
for this real content; no measured pressure saving is claimed. Source inspection
establishes that partial preparation now has its own generation owner; these
successful runs do not reproduce cancellation during partial preparation.
The backend lease/epoch eviction remains `host`, not an exact native fence
implementation. Evidence: local `lifecycles-ui-texture-owner-comparison.json`
and the paired production logs. No tests or visual inspection performed.


### Original Institute image residency capture (2026-11-04)

The user confirmed the loaded level was Institute. The read-only registry
capture `artifacts/resource-residency-eotw-loaded.json` (in the main checkout;
the historical filename does not identify this level) records 5,414 named
images. `rendering/water/foam_d.tga` is class 2, with a resident GPU resource
and a 2048x2048 single-mip view. `models/campaign/institute/institute_shore_df.dds`
is class 1, with a 512x512 resident view; the Institute ocean mask has a
256x256 resident view. These are single observations after loading, not proof
of their full-mip demand or behavior as the camera moves. The capture does not
contain the Edge of the World amplitude mask; its finer-mip demand remains
unresolved. The previous startup and loaded-level class captures also show
session and map model identities separately; no claim is made about native
allocation sizes or about each model's CPU/GPU backing from class alone.


Institute installed-header comparison: the existing `material-audit --image-info`
reader resolves all three captured resources through the installed archive.
The original's live width, height, mip count and low 21 format bits match the
compiled inline headers exactly: ocean mask 256x256 / 7 / 582990 (BC1), shore
512x512 / 1 / 460098 (R16_UNORM), foam 2048x2048 / 1 / 812436 (BC7 sRGB).
An installed index lookup finds no separate `institute_ocean_m.bimage7_mip`
resources. Thus the rewrite's inline chain provides the complete available
Institute ocean mask; this evidence does not settle the distinct EotW mask,
which has an indexed finer mip. Capture and source paths are as above.


### Edge of the World environment-image streamer evidence

The user loaded Edge of the World in the full Steam process. The read-only
`resource-residency-eotw-verified.json` capture records the authored ocean
amplitude mask as a class-1 resident 256x256 seven-mip image and the shoreline
as a class-1 512x512 one-mip image; foam is class 2 at 2048x2048. A separate
read-only snapshot of the actual streamer table (`1432f0210 + 1b8`, 24-byte
entries as indexed by `14030d6b0`) finds the amplitude image identity among
4,337 entries, while the one-mip shore and foam have no table entry. The
amplitude record's area was zero at that instant. This proves eligibility for
streamer management, not that the environment renderer submits demand or that
the full 512x512 mip is requested during play. `14031a510` forwards material
images to `14031a410`; its observed callers cover surface/material paths. The
rewrite's environment image currently uses an inline view outside the shared
streamer, so its eligibility, demand and view handoff need a general solution
if the original requests finer data. Trace environment use and demand before
selecting a producer; do not force full resolution or add a map-name exception.
Local evidence: `artifacts/resource-residency-eotw-verified.json` and
`artifacts/resource-eotw-water-stream-records.json` in the main checkout.
