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

### GPU heap occupancy attribution (2026-11-09)

Prediction before extending the existing opt-in allocator snapshot: prior
quality replacement reports show stable live allocation bytes but1GiB more
reserved heap capacity. The current totals cannot distinguish empty warm
blocks from free space stranded among live allocations. Report each existing
allocator block's capacity, allocated bytes and allocation count, plus label
groups within that block and total empty-block capacity. These are direct
reads of the same AllocatorReport; no resources, graphics passes or normal
frame work are added. Do not infer that all reserved-minus-live bytes are
empty heaps. Pinned gpu-allocator0.28.0's D3D12 MemoryType::free destroys an
empty general block when another general block of that type remains, retaining
the last general block for reuse. Attribution will determine how much of the
observed capacity is that explicit warm-pool policy versus fragmentation.
No new memory or CPU improvement is claimed by this diagnostic addition.

### MD6 cold-load/purge serialization (2026-11-09)

Prediction before the owner change: cold declaration loads release their
record mutex while reading/parsing inherited declarations, then republish a
resident payload under the existing Catalog loader guard. A concurrent marker
purge currently takes only the registry/record mutexes, so it can clear a
resident pin before an older in-flight load installs that pin again. Reuse the
same existing loader guard at marker purge and bootstrap promotion, before
the registry lock. Cold publication then finishes before the sweep releases
matching pins, and bootstrap promotion sees its admitted parent headers. Fast
resident lookup and every frame path remain unchanged; there is one extra
lock at each actual purge/promotion boundary, with a wait only if a cold load
is genuinely in flight. Native060180 supplies the exact marker sweep and
060b60 the bootstrap visit; this synchronization is explicitly a host worker
mechanism, not a claim those functions contain a native mutex. Source capture:
main artifacts/ghidra-tool-output/f30f285a83aa769ac4a62c81.json. Valid acquired
declarations remain leased across purge; no bytes or CPU saving is claimed.

### Inventory owner allocation attribution (2026-11-09)

Prediction before adding debug attribution: item header/payload/read-generation
allocations currently inherit a loading-thread tag, so their retained bytes
cannot be separated from unrelated loading data. Use the existing diagnostic
allocator's inventory-item-owner tag at the existing acquisition boundaries;
no new allocator, counters or payload representation is added. Normal release
code has these guards compiled out. The CPU preparation observer reports the
tag after each real level purge and after its Catalog drops. Expect retained
header capacity after each map (900 actual bootstrap items on the observed
route), then zero owner-tag bytes after session destruction when no item
leases remain. This is attribution, not an optimization or a claimed saving.
The existing original EotW RESOURCE baseline is51,830,920 bytes for the whole
tag, not a directly comparable per-item budget. Actual values remain to be
collected with the next cohesive source build.

### Main window-close and UI-readiness acceptance (2026-11-09)

Main sourceb12e6bfc built cleanly with game/release, unchanged source before
and after compilation; binary SHA256850a0842a7a392bc980dff90442e2d0293decb8453f29891178229fd776346aa.
The normal production client loaded EotW, completed its actual forward/shadow
loading gates and closed its loading movie at29.325s. A WM_CLOSE sent to its
real gameplay window then completed with exit0. This supersedes the earlier
development observer result, which left two visible handles ambiguous and did
not retain an exit code after manual close. No second client was running.
The new release covers the original DLL lifetime failure through the actual
window-close path; it does not claim arbitrary GPU-error recovery.

Opt-in session diagnostics recorded52 admitted READ_NOTE declarations,
HUD request/worker ready/accepted for epoch0 at30fps, then READ_NOTE
availability=true with actual GameLogic state1. No Journal or INTERACT input
was simulated, so movie entry, readable presentation and swimming appearance
are still not human-verified by this run. The exact session log is
target/release/logs/session-1794192970888-23144.log; production artifacts are
in main artifacts/runtime-main-window-close-20261109/ and its completed build
provenance resource-runtime-journal-20261109-build-provenance.json.

Pblock acquisition evidence from that same client:35 header events resolve
to34 unique resource paths and the final registry contains34 records, so the
observed duplicate did not create another header. Bootstrap promotion covered
28 records. Session teardown reports0 live pblock generations,0 remaining
Catalog consumers and0 read-history namespaces. No pblock payload was acquired
on this route; these counts verify registration/deduplication and session
release only, not retained payloads, different-map purge or failure caching.
The later model-depth, pblock-failure and inventory-item changes are separate
integration commits and are not included in this release acceptance.

### Backend runtime and loading-publication retirement (2026-11-09)

`host:` the original renderer uses D3D11; the rewrite's vendored wgpu D3D12
backend owns its own dynamically loaded runtime and COM objects. A normal main
release session at source4e7683a2 crashed with an access violation in
D3D12Core.dll+0x150fe reading0x7ffd6aab7038. The saved dump's unloaded-module
stream places that address inside d3d12.dll (base0x7ffd6aaa0000,size151552),
while D3D12Core.dll remains loaded. Device declared its runtime lease before
its allocator and shader owners, so Rust could unload the library before
those owners released their COM objects. The previous Instance factory-order
fix alone did not cover Device destruction or independently retained owners.

Prediction: moving Device's runtime leases after every native owner and
retaining a last-field runtime lease in the existing shared command-list,
allocator, queue and swap-chain owners prevents this unload-before-Release
failure. CommandEncoder releases its shared owner last, after its own lists,
descriptors and intermediate buffers. DirectComposition's surface state also
retains its library after its COM objects. All leases release with their real
owners; no process pin, new cache or permanent retention is introduced. There
is no per-frame work or rendered-output change. Device/queue/swap-chain and
allocator/encoder creation gains a bounded Arc ownership operation; no CPU or
RAM improvement is claimed. Compilation and normal window-close acceptance
are still required; the earlier automatic AppExit run did not cover this
observed shutdown failure. Evidence: artifacts/resource-main-normal-20261109/
{crash-raw.json,crash-modules.json} in the main checkout and the Windows dump
dishonored2-rust.exe.23080.dmp.

The same owner sweep found cached IDxcBlob/ID3DBlob values declared after
their compiler-container lease. Device now drops the shader cache before
the compiler container; that container already releases its IDxcCompiler3
before unloading dxcompiler.dll. Temporary compiled blobs remain confined
to a live Device borrow. This adjacent order correction adds no allocations,
reference-count operations or runtime work.

At the acknowledged outgoing-world boundary, LevelLifetime also resets
SceneGpuInputs and SceneImageRequests through their existing owners. These
plugin resources predate the scene snapshot and otherwise retain old
AssetId-vector capacity until the next scene setup. AssetIds do not retain
assets, so this is metadata retirement, not a texture/mesh leak claim. The
SceneImageRequests resource stays registered to preserve deferred-upload
behavior; phase/generation reset cannot admit the destination world early.
Existing load-timing diagnostics report retired counts, capacities and Arc
owners. No per-frame scan or GPU work is added. Transition acceptance remains
required with the coherent backend batch.

### Resource manager (all classes)

| Original | Behavior | Status |
|---|---|---|
| `FUN_1400661e0` (`ResourceManager.cpp`) | loads every entry of the `init`, `common` and map `.mapResources` lists by (kind, name) before play; `+0xfe24` = loaded/total | partial: `assets::map_resources` reads the lists, `crate::loading` derives approximate progress from a session read history, and readiness uses independent gates; `app::loading_screen` presents bootstrap/map stages in one renderer. Loads are not driven by the lists, and some historical successful reads can still be credited as acquisitions; failed reads no longer advance progress. Owner-driven retirement covers animation clips, probe sets, images and scoped physics sources; see the loading read-history investigation below |
| `FUN_140064720`, `FUN_140064820` | per-container registration of a map's list | host: lookup is `140064590` (vtable `141acc328` + `0xb0`), which normalizes the manifest path and searches registered container name tables; `assets::map_resources::load_list` resolves the manifest's owning archive, then interprets ids against that archive's index. Installed input audit: 38 manifest paths, no cross-container duplicates. Redundant index rescans are separate acquisition work, not an unresolved virtual call |
| "%i streamed" entries (`140066ac0`-`140066b8c`) | the map preloader skips index flag `0x20` and `.sbcm`/`.tbcm`/`.bwm` records | partial: classification traced in installed instructions; `140068800` reads the index flags into record+`0x4a0`. In installed `game1_001.index`, 6,952 flagged records are image mips (6,476), Iggy (350) and generic files (126). This does not imply that every ordinary placed mesh is spatially streamed. Typed rewrite owners request these payloads themselves; unified registry-driven loading remains partial |

### Parameter-block sharing across consumers

Prediction before implementation (2026-11-09): native0607f0/05feb0 returns one resource identity,98cdd0 deserializes its typed assignments and060180 retires unused resources. Existing ParmBlockSources only shares within one caller table; material/equipment acquisition must not decode a second copy of a live particle block. Put weak/coalescing record identities on the actual Catalog, preserve caller-local weak lookup tables and typed parse/metadata APIs, and let real FX/material Arc leases own the decoded data. Two simultaneous immutable acquisitions through different caller tables should return one Arc identity and perform one read/parse; each existing caller fast hit remains local. Editable generations stay independent and alive until their actual users release. Registry metadata is lazy and drops with Catalog; dead records retire their exact read-history identity after in-flight acquisitions and old generations release. No decoded image acquisition, rendering, sampling precision or per-frame resource lookup is added. No resident-byte or CPU-time saving is claimed before the real caller workload is measured; additional weak record metadata is intentional resource identity/ownership overhead.

### Item declaration identity across world transitions

Prediction before implementation (2026-11-09): the weak process lookup currently allows the item source registry to die with map resources while admitted inventory definitions survive. A later map can therefore create another registry and decode a duplicate live item. Move the one weak identity index onto Catalog (0607f0/05feb0); actual inventory/history/movie leases continue to own immutable item definitions, as63c630/6144e0/6170b0 retain and use each inventory record's declaration pointer. The same Catalog must return the same registry across map transitions; an already-live declaration must retain Arc identity and need no second parse. Retire dead weak entries after outgoing consumers release, preserving admitted inventory leases. Catalog destruction releases its registry; external leases may finish safely without any Catalog reference cycle. This adds no parser, resource cache, per-frame lookup or payload retention for unowned items. No byte or timing saving is claimed before production transition validation.

### Primary header containers and payload archives

Prediction before implementation (2026-11-09): registration must use the first primary header container in mounted master order (140064590), while payload acquisition keeps its existing selected data archive. Retain each path's header-group membership across payload overrides and lazily read the small master primary table once per Catalog. This adds session metadata, not a second resource owner or per-frame lookup. No CPU-time or memory-saving claim is made. Native 140068c30 reads the primary pairs and 1400658b0 mounts them before additional data archives. The installed master maps native container0 to game1_patch.index/game1.resources; init and common manifest payloads are in archive slot6 but still belong to header container0. All71 installed preload/reuse manifest paths have one primary membership, so this distinction applies across every installed map. This owner API is source-written; registration activation and production acceptance remain to be completed with the animation-resource batch. Evidence: artifacts/installed-container-layout.json and container-registration-scope-source.json. No tests were written or run.

### Image (`idImage`)

| Original | Behavior | Status |
|---|---|---|
| `1402ea140` LoadResourceInternal | reads the `.bimage` and uploads its base mip chain through `1402e9ff0` at load | ported: setup initializes every scene image's base chain |
| `1402e9ff0` / `1402f5500` | uploads each authored image record | host: `assets::texture` also supplies contiguous Bevy upload payloads. The shared 2D material reader and Ward reflection lookup reuse the decoded archive member in place. Particle field sampling keeps only the first authored RG8 mip after validating the complete chain. |
| `1400607f0` / `14005feb0` acquired image lookup | returns the resident resource identity | host: `texture_streaming::SharedImages` captures committed init/common images on map teardown and transfers matching handles/states into destination setup without a base read/upload. Transfer removes the old session-held handle, preventing it from pinning a superseded active texture. Inactive common images remain charged and eligible for mailbox-acknowledged pressure reduction. Scene and particle 2D identities share the streamer. UI, fog, lens, area-light and cloud/depth-default full-mip consumers share the session ResidentImages owner; original 14005feb0/140060180 lookup/purge boundaries are implemented with Bevy handles and explicit movie leases. Equivalent initial base views now transfer from the registered owner; named UI bindings acquire committed scene/common views and retain snapshot leases. Different source-mip views remain distinct to preserve the scene minimum policy |
| `1402e9ff0` | the base-chain GPU upload | host: existing Bevy image owner uploads reserved image identities; loading waits for generation-tagged GPU readiness, with common/init before map images. Custom material/sky/audio asset tracking remains active while map systems are suspended; dependent materials retry until images are ready. Original upload dispatch is traced: 1402f5500 selects 1402f50c0 (2D/cube UpdateSubresource1 or Map/copy/Unmap) or 1402f52f0 (3D row/slice pitches); 1402f4060 finalizes descriptors and releases replaced views/resources. Host Bevy/wgpu preparation and compressed queue uploads replace those D3D11 calls |
| `FUN_14030e840` arkTextureStreamer | streams `<image>.bimage%d_mip%d` during play | host: scene images use the original pressure gate, expired/minimum and density/one-level reductions (`14030f3d0`, `14030d7a0`, `14030d8d0`), sampling-role priorities (`1402d4e20`), static/dynamic mode and path/age keys. Pending reservations count against finer-mip residency; completed reads/uploads stay until pressure requires eviction. Common images transfer between active/map-cache ownership, with inactive pressure using the same GPU mailbox. Stable host tie-breaking and padded-block byte accounting differ explicitly. Default/constrained-budget walking and three-map execution are recorded below. Particle 2D demand uses this same streamer; named UI acquisition now reuses committed scene/common views without forcing full mips; compatible registered base views avoid scene reacquisition |
| `r_textureStreamingPerFrameBudgetKB` | upload budget per frame (4096 KB) | ported |
| `listImages` / readonly resource registry | the original's named image residency (sizes, classes, views) | verified snapshots: bootstrap has 643 resource registries / 23,534 entries including 1,548 named images; full-Steam Institute and EotW snapshots capture named image scopes and current views, including the EotW ocean streamer record. The user-driven waterfront observation sampled that record over movement; precise coverage and limits are recorded below. Allocator totals alone do not supply named image residency |
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

### Parameter-block failed acquisition (2026-11-09)

Prediction before changing the owner:05feb0 clears the lazy-load flag before
05ede0 runs;05ede0 records a failure on the admitted resource, and0607f0
consults that same record instead of retrying immutable failed acquisition
every time. Retain one failure string on the existing pblock header, reuse it
for unchanged installed acquisition, and clear it at that header's actual
purge or successful editable generation. No new table or read is introduced;
successful payloads and per-instance assignments stay unchanged. The host
continues propagating its existing explicit Result error rather than inventing
the original class's default-object callback. No valid-asset performance gain
is predicted; repeated identical failures should perform one read/parse per
admitted scope rather than one per request. Additional header storage is one
optional Arc. Native captures: main artifacts/ghidra-tool-output/
39eaa055c05163e9d9b9ccaa.json and1e49766b3d647c779ec2cad6.json. This code
follow-up is not in the already-frozen main build; runtime failure acceptance
is not supplied by successful acquisitions.

### MD6 parent-chain admission follow-up (2026-11-09)

Prediction before changing the resolvers: Rig acquisition and compiled-mesh
lookup still reject an acyclic MD6 parent chain after16 declarations even
though their existing visited-identity sets already reject actual cycles.
Original1c7c80 calls1c7bf0, which follows parent+0xd8 until a match or null;
the1c9630 inherit parser acquires the actual parent resource without imposing
this host depth count. Remove only the arbitrary count in both existing
resolvers. Installed short chains keep identical visits/reads/results; valid
long chains stop being rejected solely for depth. Keep actual cycle errors,
missing-resource behavior and acquisition owners unchanged. No per-frame work,
new allocation or performance improvement is claimed. Source captures:
main artifacts/ghidra-tool-output/a21bf560a642b5f40c56ce31.json and
8a070a8ca224520da6d8c383.json; existing integration artifacts/
md6-original-parser.c. This follow-up is not part of the already-frozen main
release build and has no production long-chain acceptance yet.

### Swimming hand observation (2026-11-09)

The existing bounded hand history now retains one swimming action alongside
mantle/catfall. Detailed clip-weight/phase snapshots and geometry remain
opt-in under DH2_DEBUG_HAND_GEOMETRY; ordinary frames add only bounded swim
visibility counters, with no formatting or alias-string allocation. F8 resolves
aliases on demand through the existing selected body Rig/Catalog; no duplicate
alias cache or animation resource owner is introduced. Histories release with
the level's existing PlayerArms owner. This is diagnostic coverage, not a
claimed swimming visual fix or CPU improvement.

Native846450 calls846330(state,0), and846270 Exit calls it with1. The helper
uses underwater/body state and active arm-ownership masks (802f90): surface
swimming is not universally hand-hidden across all possible states. The
currently supported living swim roster, however, has no held-object or0x31/
0x32 producer and no coexisting left-arm ownership bit, so its surface branch
hides both hands and its submerged branch shows both. Actual submerged clip
progress/pose publication still requires production evidence; it must not be
declared working from source inclusion alone.

### Journal input availability (2026-11-09)

The existing Journal/READ_NOTE movie and resource owners (78c830/78d290) now
receive configured standard controller Journal entry through the same engine
key/binding owner as keyboard input; player INTERACT uses the existing action
queue. A controller binding is inspected directly without allocating a
temporary key list. No movie, font, image, or item resource cache was added.
Opt-in DH2_TRACE_READABLE_UI records actual request/readiness/admission
boundaries; disabled recording performs no message formatting or file IO.
Premature Journal entry and disconnected worker requests now report their
actual failure instead of silently succeeding. Existing loading/epoch/HUDReady
semantics remain unchanged; a proposed readiness race was not supported by
the actual scheduling and was not implemented. Compilation and production
input observation remain required; source presence alone does not prove the
reported keyboard/reading availability issue resolved. Controller movie
navigation is outside this input-entry correction.

### Live quality replacement observation (2026-11-09)

Main release4e7683a2 completed the existing production quality observation
A/B/C/A/B/C/A with exit0. All seven samples retained564,133,888 finer-mip
bytes and zero stream reservations. Changing the four real CVars replaced the
directional target between2048x2048x5 depth32 and512x512x4 depth16 and completed
the local-owner replacement. Actual total live GPU allocations returned from
3,280,535,552 initially to3,280,338,944 bytes at finalA (196,608 fewer). These
replacements did not accumulate live GPU allocations in this observation.
Reserved heap capacity grew from3,795,845,120 to4,869,586,944 bytes. That1GiB
growth is a separate device-allocator accounting question, not measured live
texture retention. Most shadow textures lack diagnostic labels, so these
totals do not independently certify every atlas. Automatic AppExit did not
certify window shutdown: the subsequent normal session produced the D3D12
library-lifetime crash described above. Evidence: main artifacts/
resource-quality-main-20261109/{result.json,allocator-summary.json,stderr.log}.
No images, gameplay input, tests or frame-time performance claim were used.

## Kinds

Every kind in the installed load lists has a row (`loading::tests`). A
trailing `*` covers a family of kinds.

| Kind | Owner | Status |
|---|---|---|
| `Achievement` | none | none: no owner yet |
| `achievementManager` | none | none: no owner yet |
| `activeRagdoll` | none | none: no owner yet |
| `anim` | `assets::edge_animation::{EdgeAnimation,residency}` | host: immutable compressed resource identity; requested init/common clips persist, map-only entries are weak and prune after actor release. Original lookup/purge `1400607f0`/`140060180`, blob loader `14024adf0`; sampling reads the same compressed bytes. Three-map production acquisition 600 -> 276, with identical fingerprints for all 600 requests and zero outgoing map leases. Per-character playback state stays separate; no GPU upload belongs to the clip itself |
| `animBasic` | `assets::animation_rig`, `gameplay::player::animation`, `app::usable_animation` | host: declaration syntax converts to a model-specific immutable item slice; a BasicRig template and its instances share that slice, with independent commands, clocks and blend/playback state. Last template/node release retires item metadata; animation clips retain their existing scoped owner. Native1407e4540/1407e5190/1406e4af0/1406e51d0. Source scopes and generic native registry phases are not reproduced by this model-resolved representation. Four-load actual prop acquisition/instance-data and retirement comparisons passed; see scope below |
| `animSet` | `assets::animation_rig::animset` | host: Catalog-owned per-declaration records coalesce cold acquisition and share immutable aliases/generated movement while consumers overlap. Rigs retain ordered leases; editable generations and in-flight loads survive purge. Dead records retire their read history after consumers release, and Catalog shutdown drops registry allocations. Working-main three-map execution created/released44 matching identities and ended with zero owner-tag bytes. Original247a40/2462b0/060180 boundaries are traced; original heap/registration-phase admission remains in the active resource-manager port, and later Midnight consumers require their authored events. No sharing saving is claimed for main's current three-rig route |
| `animTree` | none | none: no owner yet |
| `areaTagCollection` | none | none: no owner yet |
| `arkCloth` | none | none: no owner yet |
| `arkTree` | none | none: no owner yet |
| `baseModel` | `assets::bmd6::ModelSources`, `assets::entities` | host: preparation scopes own immutable decoded BMD6 identities; setup explicitly borrows live physics model sources and runtime skin/usable consumers retain their required Arcs. Final consumer drop frees model payload, with optional acquisition/release identity diagnostics. The extra process-wide weak/coalescing cache was removed: a four-map debug production comparison completed with 210 requests, 203 reads/decodes and 76 matching source/decoded path signatures in BOTH modes, with no client errors. It supplied no measured reuse. First-person render models now use the same loader instead of direct read/decode; the routed loader and removed-cache batch compiled in the release game and completed both four-map runs, with matching model content signatures. Original identity/load provenance: `1400607f0` / `14025aa00`; read-history teardown: `140060180`. No CPU/RSS gain is claimed from debug duration. GPU mesh residency remains with its existing owner; complete class lifecycle parity is still partial. |
| `blueprint` | `assets::entities::blueprint` | host: batch-owned typed child paths/transforms coalesce immutable acquisition/failure, mutable overlays bypass sharing, and all definition payloads retire after expansion. Final instances retain independent transforms. Native1413517a0/1401c10d0/1401c1270/1401bc1f0; generic registry scopes remain separate. Four-load source/placement identity and retirement observations passed; see scope below |
| `boneCharmCollection` | none | none: no owner yet |
| `boneCharmLoadout` | none | none: no owner yet |
| `climb` | none | none: no owner yet |
| `contactSystem` | none | none: no owner yet |
| `ContactSystemDB` | `assets::entities::contacts`, `app::audio::contacts` | host: the common parsed database is shared by catalog identity; level-specific contact chains and sound state stay independent. Four-map clients reduced reads/parses 4 -> 1 with matching 7,407-entry fingerprints; a three-load release acquisition comparison measured 26,196,065 -> 9,042,530 thread cycles. Editable overlays retain separate reads. Original common acquisition is 1400661e0 / 1400607f0; whole-class forced purge remains outside this owner |
| `curvefloat` | `app::fog_scene`, `app::resident_images` | host: registered fog curves use installed compiled R16F images, shared through the image owner and purged by image scope; `1402edb60`/`1402ade20`/`1402add00`/`1402adf20` generate and upload the original lookup. All 16 indexed curve declarations have matching 4096x1 R16F compiled image names. An original EotW GPU capture matches its installed image in all 4,096 samples; other curves have descriptor/index coverage, not original-output comparisons. Scripted curve producers remain separate |
| `damage` | `app::player`, `gameplay::player::decls` | host: the supported drowning path resolves inherited source only during player preparation, consumes the water damage/health filters into typed player settings and drops the generic fields; no runtime source reads or retained generic damage tree. Original dispatch/filter consumers are 1404c0cf0 / 140717800 / 140717dc0; other damage consumers are not claimed implemented |
| `decal` | `assets::decal` | partial: typed `arkFXDecalDesc` acquisition (`143ae3730`/`217d50`), shared declaration references and ordinary FX chain playback (`222f80`/`2191f0`/`2191d0`/`218980`); installed projection-program defaults/overrides and the FX opacity image are resolved by the existing material owner; host: shared-geometry receiver submission, borrowed skin palettes and depth-box reconstruction are connected; typed acquisition covers all 25 installed decal declarations and 99 material variants; a D3D12 capture verifies the animated depth-box draw changes scene RGB pixels. WORLD receiver admission reuses the `298ee0` cursor/material rules. Clip-model bounds use fragment clipping in place of D3D clip-distance exports. Skin-decal FX remains a separate unsupported family. Renderer residency/unload lifecycle remains partial. |
| `DevGui_Menu` | none | none: no owner yet |
| `difficultyManager` | `assets::difficulty`, `gameplay::options::gameplay` | host: scoped immutable definition identities coalesce player/options acquisition; runtime managers retain a generation lease, shared direct preset/slot arrays and independent custom/current values. Selected headers/bootstrap promotion/map purge/Catalog release follow the common registry; no heap-1 promotion is invented for this class. Native141387290/1406472a0/140648590. Four-map CPU acquisition comparison verified (015e50e8); see acceptance below |
| `difficultyPreset` | `assets::difficulty` | host: scoped typed presets with immutable failure reuse and overlay bypass; manager definitions retain source-generation leases while contiguous copied preset tables preserve direct runtime lookups. Native1413872f0/140647550 and common registry phases. No shader/GPU/streaming objects. Four-map CPU acquisition comparison verified (015e50e8); see acceptance below |
| `drop` | none | none: no owner yet |
| `entityDef` | `assets::entities`, `assets::entities::light_sources`, `assets::entities::player::DeclarationSource` | host: placed models cache inherited model references per preparation; lights borrow ordered decoded source layers from a temporary owner, with editable-overlay/pure-reader bypass and complete retirement after typed light construction. Player/arm source bundles release after setup consumers. Native registration141352590 and identity1400607f0; light-source four-map production acquisition comparison verified (557dbc1a). Remaining entity consumers and full generic registry lifecycle still require review |
| `env` | `assets::entities`, `assets::environment`, `rendering::environment` | host: area and physics consumers reuse one resolved volume preparation; runtime physics/water records retain only needed fields and ordinary area parameters move to the area owner. Editable overlays preserve separate acquisitions. EotW / Dust / EotW resolutions fell 1,000 -> 500 and inherited declaration reads 2,636 -> 1,318. Four-map comparisons, including Dust return water, match ordered area and compact-volume fingerprints. A level-scoped parsed-parent cache avoids 1,351 inherited reads on the four-map route, with 516 matching profile signatures; it drops after preparation. Full declaration registry lifecycle remains partial |
| `EnvProbe` | `assets::bprobe`, `assets::pnetwork`, `rendering::probe_lighting` | host: original `1402b15e0` consumes the network sequentially; bounded archive streaming retains parsed query tables and retires source staging. Scene/particle leases share a probe set and release its read-history names on last owner drop. All 36 installed network records matched the previous reader; EotW / Dust / EotW released 3,312 / 2,825 / 3,312 history entries. GPU resource ownership remains host-specific |
| `equipmentUpgrade` | none | none: no owner yet |
| `explosion` | none | none: no owner yet |
| `fade` | `assets::fx::fade`, `app::loading_fade` | host: select the initial or later authored declaration at `1408aac09` / `1408aac1f`, acquire only that curve for the direct-map load, sample with `140166f60` / `140166e40` / `140167170`, and retire it at fade completion. Priority/listener/script arbitration remains outside the direct-map loader. The selected-source change still requires installed load verification |
| `File` | `assets::level`, `assets::world_mesh`, `visibility::tome` | partial: world (BWM) vertex data and the shared shadow source meshes are released after setup's uploads, as the original keeps no CPU copy of uploaded world geometry (F8 re-reads the world on demand); host: one lazily prepared immutable `295fa0` WORLD bounds tree is shared by shadows and projected decals (three consumers: 13,864,920 -> 4,622,536 requested bytes, identical installed EotW query output); the Tome blob stays resident and its cell tree is descended in place (axis codes, planes, implicit children and leaf ranks from a 64-node rank directory, bounds split from the root), as the original reads it; the decoded traversal/object tables intentionally retain flat runtime query data. Historical source a34a947 measured the associated query package at 826 -> 682 CPU ms/s and 3.05 -> 2.30 ms median query span; this is a host performance representation, not abandoned parse staging. The source blob and typed query data share map ownership and release together |
| `Font` | `assets::iggy::font`, `flash::text`, `flash::glyph_cache` | host: metadata reads do not decode glyphs; live movie consumers share metrics, kerning and lazy outlines over retained Iggy source bytes through a weak provider index (original14102bd60/14102ba20). Source/provider/movie ownership has no strong cycle. Existing R8 glyph pages, dirty updates, installed atlas caps and inactive GPU eviction remain unchanged. Working-main same-binary production acquisition created35->7 providers, peak record bytes400964->200634, and released every created identity in both modes. This validates provider lifetime/reuse, not CPU speed or total IGGY parity |
| `footStepPhysMat` | none | none: no owner yet |
| `fx` | `assets::fx::declaration`, `rendering::particles::world_sources`, `app::audio::contacts` | host: one inherited source-acquisition path for world, screen, pause and contact FX (140022fd0 / 1400607f0); scratch syntax retires after conversion to existing typed level/instance owners. Immutable common source texts use the existing session owner; overlays bypass it. Typed catalog resource-list residency remains partial; no generic syntax cache is retained. Source/build acquisition and normal-client observations passed; see scope below |
| `fxactor` | none | none: FaceFX arkFaceActor resources (registration1413529f0), not idEntityFx actors. The92 indexed .arkfaceactor files have no production reader; supported visual-effects actors acquire entityDef/fx resources through their own owners. Facial animation remains unimplemented |
| `fxanimset` | none | none: no owner yet |
| `gameflowManager` | none | none: no owner yet |
| `gameLogicManager` | `gameplay::game_logic::Declaration`, `app::level_prepare` | ported: worldspawn-selected inherited declaration, initial state and ordered transition matrix (140787770); host: temporary syntax is dropped after conversion to the level-owned typed declaration, which releases with GameLogic after UI-state retirement. Registered backends are listed under gameLogicState*; unsupported states remain unregistered |
| `gamesoundproperties` | none | none: no owner yet |
| `gameVideoClip` | none | none: no owner yet |
| `greatestMomentsManager` | none | none: no owner yet |
| `iggyFile` | `flash`, `gameplay::pause_state` | partial: reads its resources; installed imports now acquire their complete authored dependency closure once per player VM through the existing shared source owner, before entry scripts/movie construction; `flash::bitmap_resources` owns immutable library bitmap payloads, shared across menu instances. `1402e2c00` uploads format 13 as BC7_UNORM directly; gameplay now retains/uploads those blocks rather than expanding to RGBA. GPU bitmap/glyph textures release on movie close (`m_cacheResources = 0`); old unique glyph IDs no longer accumulate across opens. Host fallback: non-block-aligned base extents retain the existing cropped decode because wgpu requires block-aligned BC extents. Original `1402e2c00` mip-chain acquisition/upload is connected; its sampler filters follow `1402dfd40`. Host: immutable CPU library bitmaps remain a shared session cache to avoid repeated reads/decoding; movie GPU images release separately. Three-map acquisition used 34,556,544 CPU bitmap bytes against the retained 86,057,717-byte IGGY tag baseline (which includes additional UI owners) |
| `image` | `app::texture_streaming`, `app::resident_images`, `assets::texture`, `rendering::bevy_backend` | host: indexed compiled images are acquired through shared scene and resident-view owners; scene mips stream on demand, GPU upload staging is retired, and map images release while reusable common/UI views remain leased. Original acquisition `1402e9ff0` and streamer `14030dbf0` are mapped to the host backend. Scoped loading-history cleanup now removes stale names after level release while preserving reusable image and UI bindings. Full backend fence and device-allocation parity remain partial; see the image sections below |
| `inventoryitem` | `assets/inventory_item.rs`, `assets/inventory_item/sources.rs` | host: Catalog identity, live readable/history leases and inherited read-generation retirement; ported readable defaults63c510; blocked: full general item/equipment variants |
| `inventoryItemCollection` | `assets/inventory_item/configuration.rs` | ported:613ae0 construction, inherited item list, first-occurrence6136e0,613930 destruction; host: scoped Catalog resource records; verified: EotW/Dust/EotW CPU preparation shares bootstrap definitions and releases the item owner at Catalog destruction; gameplay quantities/eligibility are separate |
| `inventoryLoadout` | `assets/inventory_item/configuration.rs` | ported:613b60 construction, signed ordered assignments/flags,613930 destruction; host: scoped Catalog resource records; verified: EotW/Dust/EotW and same-map CPU preparation acquisition/purge scopes; gameplay equipment variants remain separate |
| `jointInfo` | none | none: no owner yet |
| `jumpProfile` | none | none: no owner yet |
| `kiscule` | `assets::entities::effects::declarations`, `assets::resource_registry` | host: catalog-scoped immutable declaration identity, cached immutable failures, selected preload headers, bootstrap/map phases and scoped purge follow 14138ca50 / 1400607f0 / 14005feb0 / 1400661e0 / 140060b60 / 140060180. Deferred bootstrap dependencies preserve later explicit map markers and recursive references keep first-reference order. Entity-embedded roots remain map-owned; runtime variables, gates, owner overrides and queues stay instance-local (`7bc1d0`/`7bb9d0`). Payload decoding remains demand driven. Same-build acquisition and normal-client observations passed; see measured tradeoff below |
| `layer` | none | none: no owner yet |
| `loco` | none | none: no owner yet |
| `mapInfo` | `gameplay::map_info` | host: session declaration index follows `1400607f0` / `14005feb0`; only requested parsed declarations are retained and aliases share the same immutable identity. A five-level production comparison reduced reads 50 -> 19 and decoded bytes 200,076 -> 77,776 with matching declaration fingerprints. Overlay and read-failure paths retain the existing scan; whole resource-class purge remains partial |
| `massprops` | `assets::physworld::mass::{MassSources,resources}` | host: demand-decoded principal frames replace original Havok resource payloads; Catalog identity, selected headers, bootstrap/map scopes, immutable absence/failure, purge and final release follow 141356de0 / 140089390 / 140089690 / 140089320 and the common registry rules. Body overrides still apply to a copy. Four-load production CPU acquisition/purge observations passed; live cross-map appearance is not claimed |
| `material` | `app::material_loading` (material class), `app::runtime_materials` (host preparation) | partial: startup cache ownership is transferred to the level for runtime reuse. Workers use the same resolver and asset providers, validate image readiness, and discard failed or superseded batches. Windowless Dunwall Escape observed installed decal admission, cancellation of a replacement batch, and zero outgoing decal materials and images after release. Windowless runtime mesh and slot observations are recorded below. Active draws, live tails, and an installed image-read failure remain outside that coverage |
| `md6Def` | `assets::animation_rig::declaration`, `assets::bmd6` | host: Catalog-owned ordered resource headers, shared immutable declaration generations and retained parent/model references. Creating lookup and existing-only lookup remain distinct; failed/absent loads stay attached to their actual header. Preload registers lazy identities from the actual primary/reuse manifests; bootstrap promotion and map-marker retirement follow060b60/0661e0/0410f0, with dead read history removed after all generations release. Unused declaration payloads remain demand-loaded. The working-host owner compiled in release4e7683a2 and completed the recorded EotW/Dust/EotW CPU-preparation acquisition and purge route; all 51 common MD6 headers remained unloaded while the three requested Corvo declarations loaded per batch. The earlier ad376ca0 three-map GPU acceptance covers AnimSet ownership and must not be used as MD6 acceptance. Full MD6 render-transition and nonloading-query coverage remains separate from this CPU-worker scope evidence. Joint-helper/pose consumers still have explicit admission limits; no unimplemented consumer is reported as active |
| `midnightScene` | `assets::entities::scene_motion`, `gameplay::scene_motion`, `app::player` | host: supported autonomous machinery clips share immutable map-owned definitions across graphs/playback, with separate clocks and bindings (1405329c0 / 140533460 / 1407d6040); temporary source syntax and lookup cache retire after preparation, final leases retire with machinery. Full cinematic runtime and registry preload scopes remain partial. Four-load source identity/initial pose and retirement observations passed; see scope below |
| `model` | `assets::bmodel`, `app::setup` (model class) | partial: shared world/model surfaces upload normal and tangent as SNORM8x4 and colour as UNORM8x4, as installed (bitangent derived in the vertex shader); packed meshes retain both authored half-float UV sets; rigid and skinned model shadow programs keep the game-space position they consume, and vegetation uses additional wind inputs; sheared baked instances keep float vertices. Map acquisition and release use temporary model preparation data and Bevy mesh handles. Windowless EotW and Dust uploads followed by map clear return to the four preexisting host mesh IDs. Pre-level host slabs are sealed before map allocation so map buffers retire after their last level lease. Demand-decoded immutable init/common BML1 sources now share Catalog ownership across levels; GPU mesh identity reuse remains open. Original static-model loader and retirement are `14026a5a0`, `140259d50`, `140278df0` and `14026a240`, with lookup/purge `1400607f0`/`140060180` |
| `musicManager` | `gameplay::music` | ported: compact inherited definition (140662be0, 141804e20, 1418052b0, 1418054d0), inline ten-request owner (140662fe0, 140663fe0, 1406645e0), stage/threat/stinger update (1406647d0, 1406649b0, 1406634c0), startup/shutdown including request reset (1406637e0, 1406641e0, 14054ced0); blocked: connect this owner to the single entity dispatcher being implemented in the HUD branch and the actual worldspawn audio component, and resolve the alloc-game feature 0x20 gate before activation. No banks, media or GPU resources are duplicated by this owner; sound acquisition/playback stays with the sound class. Source compiled in f25c019; runtime activation remains blocked. Wwise music actors (HIRC 10/12) also require their actual scheduler, so wiring the manager alone will not produce music. |
| `navigation` | none | none: no owner yet |
| `Navmesh` | none | none: no owner yet |
| `navmeshLayerSettings` | none | none: no owner yet |
| `navmeshMapSettings` | none | none: no owner yet |
| `NPCVariationDef` | none | none: no owner yet |
| `particledt` | `assets::fx::particle_resources`, `rendering::particles::definition` | host: catalog identity, coalesced immutable acquisition, cached immutable failures, bootstrap/map markers and scoped purge follow 1400607f0/14005feb0/140060b60/1400661e0/140060180; payload decoding is demand driven. Live generations outlast purge through consumer leases; overlays bypass immutable sharing. Caller-local tables retain the existing fast path. Same-build four-load CPU comparison and normal-client GPU-validation route passed; runtime simulation and GPU publication remain with their existing owners |
| `pauseMenuSectionOptions` | `gameplay::options`, `gameplay::pause_state` | host: requested options declarations initialize owned typed sections/state; root syntax retires after initialization, subclass syntax after its last constructor reader, and the actual pause/preparation owner releases typed state. No movie or GPU resources are owned by this CPU declaration layer. Native14137dcc0/140516d70/140516e40; four-load source-retirement and typed-state comparison passed; see scope below |
| `pblock` | `assets::parm_block`, `rendering::render_program`, `rendering::particles::definition` | partial: the installed registry has 96 declarations; typed acquisition and weak declaration/history ownership are connected to particle definitions and retained through runtime tails, and compile in the integrated game build. Metadata is acquired lazily through the shared program library; mutable overlays bypass sharing. FX descriptors preserve `m_parmBlock` but the world FX owner still rejects those systems because per-instance binding and actual masking-pass activation remain unconnected. Original registration is `141351ed0`, value deserialization `14098cdd0`, and particle application `140227cd0` / `140228110` / `1403421e0`. The masking executor is `1403222f0`, registered by `14034fe00`: it binds maskBuffer with scene depth, selects slots 15/16, and preserves material state except depth writes; `render_state::masking_surface_draw_state`, `material_lists::model_masking_admitted`, and `render_model::update_particle_parameters` port the traced rules but await renderer integration. EotW / Dust / Dust return / EotW measured coverage remains 0/33, 0/37, 0/31, 0/33 listed declarations. Complete per-instance binding, target execution and color-grading consumer before lifting rejection. |
| `physhape` | `assets::physworld`, `assets::entities::triggers` | host: trigger physics retains owned query geometry, bounds and cached mass results; immutable trigger HKX source/schema retire after scene initialization, while mutable shapes keep their source. EotW / Dust / EotW releases 332 / 259 / 332 sources and matching geometry/mass fingerprints were recorded. Area, physics and wind preparation share matching immutable source/schema identities through the last consumer; 1,013 reads were avoided across four loads and the temporary cache released afterward. Model collision and machinery requests now route through the physics source owner. The candidate cache of alias targets was removed after the Dust return comparison showed extra temporary storage and slower acquisition with no saved reads. Full resource registry lifetime remains partial. Original 14006a910 pushes the map heap for named maps; physhape is absent from 1400661e0's conditional persistent-type promotion table at 142280ee0. Init/common inherit their caller's scope |
| `physicsMaterial` | `assets::declaration_texts`, `assets::entities::physics_material`, `app::audio::contacts` | host: world, model collision, machinery and startup share temporary source texts, then transfer listed common texts to the existing session owner; unlisted texts drop after preparation. EotW / Dust / EotW source reads 749 -> 62, plus 45 contact reads avoided; all recorded collision/material and contact signatures matched. Typed parsed definitions and full registry lifecycle remain partial |
| `physSystem` | `assets::physworld::physics_system` | host: machinery acquires immutable decoded systems through the physics owner; repeated paths share during level preparation and each instance constructs independent bodies and constraints. Four sampled loads required 5 reads instead of 12 with matching per-instance definition fingerprints. The original MD6 definition parser `1401c9630` acquires the ragdoll's physics-system identity through `1400607f0` on registry `142bf86f0`; `14076abc0` constructs its live bodies from that resource, and `14008d7b0` releases the resource's owned Havok object at destruction. Session/common residency and full registry purge remain partial |
| `physworld` | `assets::level`, `assets::physworld`, `gameplay::navigation`, `app::level_lifetime` | host: the selected map companion is acquired once by level preparation; source bytes and Havok schema/decoded construction tables retire at the end of the physics phase. Only owned full-precision collision triangles, packed cell indices, material/filter and owner state survive in NavigationWorld; it releases with the outgoing level. Failed/cancelled preparation drops the same owned values. Native registration141388640 and compressed-mesh tree141262020; no per-query source decoding, GPU upload or session pin. This covers the supported static-query representation, not a port of the whole Havok world runtime or generic native resource registry |
| `playerLocomotion` | `assets::entities::player::DeclarationSource`, `app::player`, `gameplay::player::animation` | host: inherited declaration syntax is acquired during player setup and shared with arm/body consumers through the temporary setup owner. Runtime body/bob/arm LocoData owns model-specific clip indices, speed sets and transitions; mutable playback remains per node. Generic syntax is dropped after setup consumers, with no frame-time archive reads or permanent source cache. Original registration14138cdb0, common identity1400607f0 and animation update1406f4c70; selected native registry header/purge phases remain a host deviation, not ported |
| `playerMode` | `assets::entities::player::DeclarationSource`, `gameplay::player::decls`, `gameplay::player::states` | host: startup and swimming modes resolve inherited sources during setup; typed rules, default IDs and supported state records move to level-owned runtime StateMachine data. Generic mode syntax drops after the player/arms consumers; no mode-declaration source trees survive in the player runtime. Original registration14138f7d0 and common identity1400607f0. Unsupported player modes are not acquired for runtime use. Selected native registry header/purge phases remain a host deviation, not ported |
| `playerStatisticsManager` | none | none: no owner yet |
| `proceduralLookAt` | none | none: no owner yet |
| `ragdoll` | `assets::physworld::physics_system` | host: machinery resolves a skeleton/system pair and shares the immutable decoded ragdoll mapping, with independent runtime bodies and skinning state. Four sampled loads required 5 reads instead of 12 and matched the per-instance definitions. Original `14009a3c0` acquires the associated physics-system and skeleton identities before constructing the generated ragdoll path. Session/common residency and full registry purge remain partial |
| `ratLocomotion` | none | none: no owner yet |
| `renderParm` | `assets::render_parameter` | host: Catalog-owned static code-default bytes, coalesced immutable reads/failures and lazy numeric metadata; editable paths create fresh leases. Catalog retirement drops session ownership; renderer/environment/sampler readers share this owner. Original1402d82c0/1402d8310/1402d8820/1400606f0; no native purge marker inferred. Four-load acquisition/storage and installed decal-material observations passed; see scope below |
| `renderProg` | `assets::shader_cache::ProgramLibrary`, `app::scene_preparation`, `app::runtime_materials` | host: immutable compiled indices share Catalog ownership; definitions decode on demand into the level program library, temporary compressed/unpacked RPD/RPVD/UCD buffers retire after conversion/admission, and prepared variants retain their required binding metadata. Validated UCD stage checksums share the level library so material variants avoid re-inflating the same stage. Runtime preparation leases the same library; final level/worker release closes its cursors and drops definitions/variant caches. Original141352730/1402ca9c0/1402c9f00/1402ca440; native GPU-bank publication, source/include validation and optional preload are not implemented by this serialized-source owner. Backend GPU objects retain their existing renderer owners |
| `ruleHandler` | none | none: no owner yet |
| `skeleton` | `assets::edge_skeleton`, `assets::animation_rig` | host: a catalog/path-keyed weak/coalescing Edge skeleton owner serves player body/arms, first-person rendering, rest-skin preparation and physics. Live consumers retain one immutable full skeleton; mutable poses and derived physics rest matrices stay separate. Final-drop tokens retire identity metadata; failed loads and level teardown prune empty entries. Editable overlays bypass sharing. Original lookup/load/purge: `1400607f0` / `14024f2a0` / `140060180`. Parent arrays are no longer copied into every body-camera pose. The full skeleton retains names/inverse binds until the last consumer. The release four-map comparison recorded 225 -> 208 decodes for 225 requests, 17 live reuse hits, 79 matching resource signatures and zero final live identities in both modes. Peak full-skeleton logical payload was 60,006 -> 34,265 bytes. This compares full unshared/shared representations, not historical subset-only player RAM. Walking parent-source medians were 2,905 -> 1,540 thread cycles, with 320 -> 0 copied bytes and one allocation removed per pose. Actual original tag budgets cover additional unimplemented consumers; complete class parity is not inferred. |
| `soundauxbus` | `assets::wwise::auxiliary_resources` | host: demand-decoded immutable bus IDs share Catalog identities, native selected headers/bootstrap/map scopes and purge (141351fa0 / 1400b4460 / 1400b44a0, common resource registry rules). Acoustic zones retain their local ID lookup; bank DSP ownership remains separate. Four-load production CPU acquisition/purge observations passed |
| `soundengineinternals` | `assets::wwise::engine_controls`, `app::audio::AudioMix` | host: supported initialization inputs resolve through one resource reader (141352010 / 1400c84d0 / 1400c88c0); source syntax retires after conversion, copied IDs/ranges stay with the audio session across maps. Unused engine-declaration fields are not retained or implemented. Windowless production-path initialization observations passed |
| `soundevent` | `assets::wwise::event_resources` | ported: lazy selected headers, immutable shared acquisition/failure, bootstrap/map scopes, purge and Catalog release; production acquisition observations passed; see scope below |
| `soundrtpc` | `assets::wwise::rtpc_resources` | host: typed Catalog identity, selected headers/bootstrap/map phases, immutable failure cache, overlay bypass, purge and final release (1413520e0 / 1400b4510 / 1400b4560 and common registry). AudioMix retains copied IDs/ranges. Original clamp1400b45c0 unchanged; production initialization and four-load scope observations passed |
| `speechBarks` | none | none: no owner yet |
| `speechCharacter` | none | none: no owner yet |
| `speechScene` | none | none: no owner yet |
| `speechVoice` | none | none: no owner yet |
| `staticParticleModel` | `assets::fx::particle_resources`, `assets::particle_model`, `assets::fx::particle_mesh` | host: same catalog header/purge owner as particledt; demand-loaded mesh dependencies inherit the declaration acquisition scope only when creating a new identity, preserving explicit preloaded markers. One immutable payload is shared by emission/render consumers; GPU upload remains the particle renderer responsibility. Same-build four-load CPU comparison and normal-client GPU-validation route passed |
| `strIdFormating` | none | none: no owner yet |
| `swarmGlobalConfig` | none | none: no owner yet |
| `tutorial` | none | none: no owner yet |
| `UIPauseStatisticEntry` | none | none: no owner yet |
| `vault` | none | none: arkJumpDef_Vault resources for NPC navigation (2 installed navmesh/npc_movement declarations), with no production reader. Player vaulting uses its separate player-state/sensor declarations; it does not acquire this resource class |
| `vaultClimb` | none | none: no owner yet |
| `vaultDrop` | none | none: no owner yet |
| `cpntAudio` | `assets::entities::audio`, `app::audio`, `app::player` | host: inherited fields resolve during scene/player preparation. World emitters retain typed settings, event leases and a reference frame. Player arms retain typed sound settings. The generic component text and resolved field caches retire after their preparation consumers. Sound banks and playback retain their separate audio scopes. |
| `cpntAudioListener` | `assets::entities::player`, `app::player`, `app::audio` | host: the selected character's inherited listener fields convert to typed listener settings before the declaration bundle is returned. The active listener is a world/audio runtime object. |
| `cpntInventory` | `assets::inventory_item`, `app::interaction_focus` | host: the inherited component contributes initial item references. Runtime inventory owns item leases and quantities, with no component text cache retained by those entries. |
| `cpntPickable_Simple` | `assets::entities::usable`, `assets::inventory_item::sources` | partial: the READ_NOTE route resolves inherited pickable fields into typed item/interaction bindings and drops the per-batch parse cache. The scoped item registry owns acquired readable identities separately. General pickup component behavior remains outside this coverage. |
| `cpntPlayerSensorInteraction` | `assets::entities::usable`, `app::interactions` | host: the selected character's inherited definition supplies the interaction distance and five cast radii (1403b7a40, 1403b8d60). Only the typed sensor settings survive acquisition. |
| `cpntPlayer*` | `assets::entities::player`, `app::player`, `gameplay::player` | partial: observed physics, camera, controller, body, state machine, auto-crouch, lean, mantle, breath and FX-manager fields feed supported runtime owners. Shared player/arm preparation retains field leases until their final setup and screen-effect consumers. This row covers the observed inputs, with unimplemented component behavior excluded. |
| `cpntTrigger` | `assets::entities::triggers`, `gameplay::trigger_volume` | partial: the map loader resolves typed trigger settings and shape leases. Generic inherited fields retire after preparation. The map owns supported overlap and event state; shape lifetimes are tracked separately under `physhape`. |
| `cpntWind` | `assets::entities::wind`, `gameplay::machinery_world` | host: typed wind settings feed supported world and trigger wind consumers. The inheritance cache is local to component preparation and drops with its reader. |
| `cpnt*` | `assets::entities` (components of placed entities) | partial: the named rows cover independently traced consumers. The observed health, mana, adrenaline and attribute inputs use the player declaration bundle and typed runtime settings. Other indexed component types may have no consumer. |
| `UI_HUDElement*` | `app::hud_movie` | host: in-game HUD configuration resolves installed state/HUD declarations on its existing movie worker; typed thresholds, bindings and gauge state live with HudMovie, temporary declaration fields drop after loading, and movie/image/font leases use the shared Flash owners. Supported interaction, breath, health and mana consumers are present; this does not claim all HUD families. Source consumer 14078b550 / 14038f7d0 / 140674be0 |
| `playerState*` | `assets::entities::player::DeclarationSource`, `gameplay::player::decls`, `gameplay::player::states` | host: ordered inherited mode/state declarations are resolved during player preparation; only supported state trees survive to typed StateDecl/Midnight construction. Setup subtrees release after their last player/arm consumer; mutable state, clocks and clip leases live with the level player. Unimplemented state mechanics are excluded. Generic native registry header/preload phases remain 1400607f0 / 1400661e0; this host does not pin unused state syntax to a session registry |
| `gameLogicState*` | `gameplay::game_logic`, `app::pause_menu`, `app::loading_movie`, `app::hud_movie` | ported: world-selected transition matrix and stack (140787770 / 1407886c0 / 1407885b0 / 140787510); host: registered in-game, pause, read-note, Journal and map-transition backends retain their actual movie/declaration leases and retire through the existing UI-worker acknowledgement before level support resources drop. This does not register unimplemented states or claim their lifecycles |
| `mainMenuSection*` | none | none: no main menu |
| `menuSectionOption*` | `gameplay::options` | host: supported Controls/Visuals/Audio/Gameplay fields convert to owned widgets, categories, presets and quality tables. Parser edit trees retire after subclass initialization (14137d600/14050a480/14050df80), rather than duplicating typed data through the open menu. Pure format readers retain staging until their caller finishes. Four-load typed-state/source-retirement comparison passed; unimplemented subclasses are not claimed |

### Active implementation: original lifecycle completion

- Camera depth owner retirement (`14036d0c0`, `1402fbe40`, `1402fc6a0`, `140351780`). The coarse reduction retains a binding to the outgoing camera depth along with its pyramid and staging ring, and alpha depth keeps an opacity image even without an active view. Release those view allocations at the existing render acknowledgement, while keeping device shader pipeline caches. Cancel outstanding readback mappings by retiring their generations before dropping the staging buffers. Predict zero outgoing pyramid, opacity, and ring identities at the map boundary. At 1920×1080 their own logical texture and readback payloads exceed 13 MB before allocator padding and the old bound depth texture. Allocation starts again at the next valid view; ordinary in-map rendering and resize policy stay the same.

- Packed acoustic route lookup (`1400b7790`, `1400b7ce0`). The original resolves a zone pair to an offset and traverses the installed packed records in place. Both host audio admission and detailed path scoring currently allocate an owned route list and a portal vector for each record before applying distance and direction. Borrow validated portal words from the existing map-owned table. Predict zero route and portal allocations per gameplay lookup, with matching traversal order, cost and malformed-list rejection. Measure only lookup and traversal over actual installed zone pairs, including the added bounds-validation pass. The graph's retained table size and audio playback state stay the same.

  Installed lookup comparison `2e7959e1` matched the ordered routes and portal signatures for Edge of the World, Dust District and the Clockwork Mansion boat. Each of the two owned passes built 104,832 / 239,319 / 1,681,717 nonempty route-list and portal buffers. Each borrowed pass built zero. On the boat, lookup plus traversal used 338.8–345.8 million thread cycles with owned lists and 59.3–60.1 million with borrowed lists. This measures the changed traversal over installed pairs. Full mixer timing, in-game listening and malformed archive inputs were outside the observation. Evidence: `DH2-scene-shutdown/artifacts/packed-acoustic-route-20261110-comparison.json`.

- Shared particle physics queries (`140677ec0`, `140678070`, `1401fc710`, `140c85490`). The original queries the published physics world. The host cloned every reachable trigger body, broadphase tree and water height for every published camera frame, even after particle simulation moved to the main world. Keep one map-owned collision geometry owner, resolve wind identities after component startup, and let the frame stamp lease the same geometry. Current trigger and water bodies are published only during startup; moving machinery and player physics use separate owners. Predict one validation pass per map, zero per-frame copies or allocations for publishing query inputs, and the same tree order, filters, positions and water heights. The final lease detaches shape observers before freeing geometry.

- Physics shape-observer teardown (`140cdfcb0`, `140ce03d0`, `1408a4830`). Trigger and acoustic worlds own Havok shape-observer registrations. Their Rust drop paths currently free both objects without running the existing detach routine. Drain active and pooled registrations in the original order while each shape's signals are still owned, including failed preparation and the final acoustic-world lease. Expect zero remaining subscriptions after a successful drain, with no gameplay work or change to published shapes and queries. Detached records are retired once even when a caller explicitly releases its world before Rust drop.

  Windowless Edge of the World, Dust District and Clockwork Mansion boat publication and clear (`8ad64e43`) reached both teardown hooks in all three maps with no errors. All six registries had zero mutable observers. This verifies the owner ordering for those installed maps. A nonempty observer drain was outside the observation. Evidence: `DH2-fx-level-lifetime/artifacts/shape-observer-retirement-20261110-comparison.json`.

- Default colour grading image (`1402f5cc0`, `1402ecb20`, `1402f5420`). Renderer bootstrap registers the 32³ identity LUT once. The display owner currently recreates it in every map. Use the existing session intrinsic image owner and pass the same immutable handle to display preparation. Keep the separate mutable colour-history images (`14036d0c0`, `140326af0`) under the view owner. Expect one 131,072-byte source upload per session and zero duplicate source uploads across maps. The default handle remains resident between maps, while the authored world and override cubes retain their current map ownership. Pixel encoding, sampling and blending stay the same.

  Windowless D3D12 comparison `baf3183e` reused one 32³ RGBA8 texture across Edge of the World, Dust District and a return to Edge of the World. The map-local path created three image identities, and the shared path used the same GPU identity before preparation and after teardown at every step. Two 131,072-byte source uploads were avoided. Active image, mesh, buffer and sky material counts matched at each load. This comparison covers asset upload and residency, with view draws outside its scope. Evidence: `DH2-collision-staging/artifacts/default-colour-cube-20261110-comparison.json`.

- Acoustic propagation acquisition (`1400b7fe0`, `1400b9eb0`, `1400b7ef0`). The original consumes the installed v13 resource sequentially and reads the packed route table into its final `TAG_AUDIO_PROPA` allocation. The host currently stages the whole decoded member before copying the routes. Stream large members through the existing archive reader, reuse one zone-shape scratch allocation, and fill the final route vector directly. Installed members include 3,214,915 bytes in Edge of the World, 7,992,417 in Dust District, and 31,181,079 on the Clockwork Mansion boat. Predict removing one whole-member allocation, offset by the bounded archive buffer and largest scratch. Keep the small files on the borrowed parser to avoid inflating staging for short resources. The typed zone/portal geometry and packed runtime route bytes must match. This changes map preparation only.

  Installed comparison `7fe12e35` matched typed geometry, portals and packed route fingerprints across all 36 files, with 31 large files streamed and five small files buffered. Tracked source staging fell from 3,214,916 to 73,440 bytes in Edge of the World, from 7,992,418 to 73,248 in Dust District, and from 31,181,080 to 73,248 on the Clockwork Mansion boat. All three also reached scene publication and release. These figures cover the source buffer and retained scratch capacity, excluding the decompressor workspace and unchanged runtime route storage. No playback timing was measured. Evidence: `DH2-scene-shutdown/artifacts/acoustic-staging-20261110-comparison.json`.

- Superseded runtime material work (`1400607f0`, `140060180`, `1408a4830`). The map gate drains only the task still owned by the coordinator. Supersession or image failure previously dropped the handle while synchronous material work could still hold and populate shared owners. Keep the existing task slot until the cancelled work returns. Discard its output even when the numerical generation did not change. Both jobs already require the same program and particle locks, so serialize admission as well and avoid a second snapshot while the first job retires. Predict at most one in-flight material batch, no superseded publication or decoder writes after map purge, and no additional work on settled frames. The replacement can be submitted as soon as the coordinator observes the old task return.

  The windowless Dunwall Tower stink-bug mesh and Edge of the World blood-splatter decal acquisitions (`48327082`) each discarded generation 1 and published generation 2. The mesh route released two images and one mesh, and the decal route released seven images and four decal materials. Both returned to zero owned GPU assets with zero retired-slot texture requests. This exercises actual material supersession and GPU asset retirement. It does not time a map change while the decoder is blocked inside archive I/O. Evidence: `DH2-fx-resources/artifacts/runtime-material-supersession-20261110-comparison.json`.

- Inline volume upload staging (`1402e9ff0`, `1402f4060`, `1402f5500`, `1402f52f0`). The original creates the typed image and passes authored slice payloads to its uploader. The host needs a contiguous mip-major buffer for Bevy's 3D image upload. Reuse the archive member allocation by compacting the validated slice ranges in place, including particle volumes and display colour LUTs. Predict no second payload allocation during preparation, with identical dimensions, format, mips and payload bytes. The compacted vector retains up to the original member's header and allocation slack until upload, so this change targets peak overlapping staging and allocation traffic. No gameplay or GPU timing gain is predicted.

  All 102 installed BIM volumes matched the borrowed reader byte for byte, including descriptors and 183,073,792 payload bytes. Every owned result used the original allocation. The census removes 187,776,000 cumulative bytes of separate payload capacity and retains 80,886 cumulative bytes of original allocation slack. These totals span the entire installed set and do not measure concurrent residency. Edge of the World and Dust District also completed windowless scene publication and release. Active particle rendering and GPU timing were outside the observation. Evidence: `DH2-fx-level-lifetime/artifacts/inline-volume-staging-20261110-comparison.json` (`235741f0`).

- Blueprint instance handoff (`1401bc1f0`, `1401bc3c0`, `1401c1270`). Machinery and render setup currently expand the same installed placements in separate batches. Pass one ordered placement list through both owners after root-only collision preparation. The original uses a shared assembly definition with instance-owned transforms. Predict one fewer root-list clone, one fewer transform expansion per authored occurrence and one fewer declaration read per unique path on the duplicate batch. Existing overlay sessions still acquire independently, so mutable inputs retain their acquisition boundaries. The placement order, render geometry and machinery indices should match, with no additional gameplay residency or work.

  The same-binary Edge of the World / Dust District / Edge of the World comparison (`9281383b`) reduced expansion calls from 312 to 156 and installed declaration reads from 72 to 36. All ordered blueprint expansions, final placement signatures, placed-model mesh signatures and published/retired asset counts matched. The first comparison expected a world-mesh signature from a separate disabled diagnostic. The accepted result uses the emitted placed-model signatures and makes no world-mesh or gameplay timing claim. Evidence: `DH2-collision-staging/artifacts/blueprint-placement-handoff-20261110-accepted.json`.

- Texture worker map retirement (`14030d1e0`, `140060180`, `1408a4830`). The scene streamer remains scheduled during the outgoing render handoff. Previously it could still admit archive reads and upload work before the old worker dropped. Close admissions when the map request arrives, discard queued/completed reads, and wait through the loading screen for the detached reader to finish its current member. The worker acknowledges after dropping any unused payload, including its cancellation path. Expect zero old read results applied after closing and no old decoded read staging when the next map begins. The initial texture upload gates and ordinary mip demand remain unchanged. A read already inside archive I/O may finish without blocking the update thread.

  The windowless Dunwall Tower and Edge of the World runs (`77d7776c`) submitted an installed scene image request in each map alongside readable, rig and FX requests. Both runs completed the production retirement gate before clearing the map, with zero outgoing FX requests and texture retirement acknowledged. The observer does not distinguish a read cancelled before I/O from one already in progress. Evidence: `DH2-fx-resources/artifacts/texture-map-retirement-20261110-comparison.json`.

- Compatible named image reuse (`14005feb0`, `1402df8c0`, `1402f5530`). A streamed view may share the same path as a fixed authored view and still start at a different mip. Require the installed initial mip selection for fixed-view cache hits, including weak leases and cross-cache reuse. Named Iggy wrappers continue leasing the current committed view, and now use the existing shared scene-view lease instead of constructing another identity wrapper. Expect zero extra uploads for compatible views and a separate authored acquisition only when the active view does not cover the requested mip chain. Ordinary streaming and its per-frame pressure decisions are unchanged. Installed-view admission and exact payload evidence are separate from live UI appearance.

  The installed ocean amplitude mask and building diffuse image (`f8e7e9d5`) reused compatible leases and recovered mip 0 after a coarse scene publication and map purge. Both recovered payload CRCs matched the original full-mip acquisitions. Evidence: `DH2-scene-shutdown/artifacts/resident-view-admission-20261110-comparison.json`.

- World-particle mesh buffer lifetime (`140319900`, `1402bb550`, `1402bace0`, `1408a4830`). The direct-instancing host prepared two GPU buffers and a binding even on a view with zero particle instances. It also held their largest capacities across maps after releasing the bindings. Allocate only for an actual nonempty submission and release the buffers, staging and proxy index at outgoing render acknowledgement. Zero-count proxies keep the same no-draw admission. Expect zero particle instance buffer allocations on maps with no submitted world-mesh particles, and two fewer retained buffers when leaving a map that used them. Active submissions keep their layouts, records and upload path. Actual GPU allocation sizes and active rendering remain to be observed.
- Outgoing interaction acquisitions (`1408a4830`, `140060180`, `140844bf0`, `1407e4540`). Readable item and usable rig workers can outlive their ECS handles while synchronous decoding runs. They can still insert inventory, model, skeleton or animation entries into the shared catalog. Share one cancellation signal for these map-owned requests, stop a queued read before it takes the source lock, and observe both workers returning before map purge. A read or rig decode already running may finish. The expected change is at the map boundary, with no new work per walking frame and no change to admitted item data or animation playback. Keep observation of installed acquisition and outgoing retirement separate from gameplay presentation.

  The installed Dunwall Tower and Edge of the World observation (`b817fc2b`) submitted one authored readable and one usable rig alongside an outgoing FX request in each map. The production gate settled before both map clears. One FX decode had already returned before cancellation completed; its output was discarded. This covers task admission and map retirement, with no live animation, UI presentation or blocked I/O timing claim. Evidence: `DH2-scene-shutdown/artifacts/map-declaration-retirement-20261110-comparison.json`.

- Late Wwise preparation retirement (`1410f1910`, `141122400`, `141160970`). The existing bank generation rejects stale media publication, but a preparation can rebuild Vorbis headers after the map's header sweep. Bind unclaimed preparation results to their submission generation. If an outgoing job finishes or is discarded after that generation has retired, drop its source leases before pruning unused decoder headers. Release completed-but-unclaimed world jobs before the normal generation change, so they use the existing map sweep. Expect no late job to retain decoder headers after its final result drops. Existing prepared sources and common voices retain their leases, with no per-sample work and one generation read per submitted preparation. Validate normal source output and distinguish late-result retirement from live playback.

  The same installed EotW / Dust / EotW acquisition route (`b817fc2b`) matched 27,177 media records, 5,837 stream records, encoded residency and read volume at all three publications. This covers the ordinary acquired output after changing retirement order. It does not exercise a stream decoder completing after the generation change or measure voice timing. Evidence: `DH2-scene-shutdown/artifacts/sound-publication-after-map-retirement-20261110-comparison.json`.
- Scene transform scratch (`1402974c0`, `1401bc1f0`). The host evaluates the authored world and placed-model transforms to prepare probe samples, bounds and baked-shear meshes. It allocated a transformed position vector for every surface or matrix, and transformed each world object's first matrix twice. Reuse one temporary vector per preparation phase and reuse the first world matrix's points between material resolution and its placement. Expect capacity growth only when a larger input arrives, one fewer transform per first-matrix source vertex, and zero retained scratch after each phase. Preserve the vertex arithmetic, reduction order and baked mesh output. Compare scoped transform cycles and installed scene/mesh outputs with fresh scratch in the same executable. No frame-time or persistent memory gain is predicted.

  The optimized same-executable comparison (`19b5b7f9`) matched world and placed-mesh fingerprints, instance counts, and the prepared/published/retired asset counts on EotW and Dust District. The two-map route reduced scratch growths from 272,324 to 43 and avoided 16,235,887 first-matrix vertex transforms. Total cycles measured inside the affected fills fell from 1,041–1,048 million to 819–834 million across two pairs. Largest allocation in each phase was unchanged. This measures preparation only. The same build covered compilation and ordinary map publication/removal of the new visibility owner, with populated live visibility scratch and query timing outside scope. Evidence: `DH2-fx-level-lifetime/artifacts/scene-transform-scratch-20261110-comparison.json`.
- Visibility scratch ownership (`140a82710`, `14037ef90`, `1408a4830`). Cached Tome indices, cell/depth output and the previous visibility bitset are meaningful only within the current map. Move them from scheduler locals to the existing map visibility mapping. A renderer extraction with no scene also drops the alpha-depth candidate index from the outgoing map. Expect those allocations to end at map retirement, with the same key, visibility, shadow and texture-demand logic and no new gameplay allocations or locks. The windowless map route can verify acquisition and removal of the owner. Populated-cache bytes and live query timing are separate observations.

Visibility query scratch prediction (2026-11-10). The original Umbra consumer 14037e1d0 marks per-member visibility in its existing output storage, with the query owner at 14037ef90 and map destruction at 1408a4830. The host creates one all-false fallback Vec<bool> and a second instance-sized bitmap for every admitted result. Keep one reusable bitmap on the existing map visibility owner. For a successful result at N nonzero instances, expect two allocations and two N-byte clears to become zero allocations after warm-up and one N-byte clear. A frame without a usable result needs neither allocation nor clear. The previous visible mirror remains separate. Expect unchanged visible instance indices, hidden/retired behavior, texture demand and shadow query publication. A map replacement retires the bitmap with the map owner. Verify allocation identity and mapped outputs before integrating; any timings apply only to scratch preparation.
- Runtime FX source leases (`1400607f0`, `140060180`, `140199df0`). Reuse the live per-path source set before extracting another dependency map from the published level. Both worker acquisitions and subsets of already prepared sources register a weak identity. A request replacing its own playback can hold that source through the change, while simulation and bindings remain per actor. Prune dead identities with the existing acquisition cleanup. Expect one live dependency map per overlapping path, no additional archive reads, and no source kept alive by this index. Verify the actual request path separately from material publication.

  Installed request observation (`62563e57`): in Dunwall Tower and Edge of the World, each same-update start/stop admitted one cancelled task and settled with zero ready sources, retiring tasks, or cached identities. The later request acquired one definition through the real worker. An overlapping request reused the same live source, with two requesters and one cached identity. Stopping both returned the identity and live source count to zero. This observes command admission, the cancelled task's acknowledgement, and final release. It does not establish interruption partway through an archive read, live emitter tails, or a performance delta. Evidence: `DH2-fx-resources/artifacts/runtime-fx-source-requests-20261110-comparison.json`.
- Installed Wwise codec census. All 82 packages in this installation contain 132,518 indexed Vorbis and 114 indexed IMA ADPCM media records. Bank indexes reference a further 643,577 Vorbis and 8,269 IMA ADPCM ranges, including prefetched prefixes and repeated ranges. The existing installed packet readers cover both codecs. The PCM WAV conversion is a fallback for other inputs and this inventory found no installed PCM records. The Vorbis decoder still rebuilds its setup header from Wwise codebooks. Evidence: `DH2-scene-shutdown/artifacts/installed-wwise-media-format-census-20261110.json`.
- Decal declaration leases (`143ae3730`, `140217d50`, `140060180`). The installed FX scan found 85 references to 25 decal declarations, with 15 referenced by multiple effects. The previous per-call resolver reparsed the same declaration when separate live FX acquisitions requested it. Keep a weak decal lookup in the existing level FX acquisition owner. Two overlapping requests should read and resolve it once, with unchanged variant names. Editable overlays still resolve on demand, and a later request after the last owner drops reacquires the declaration. Expect fewer declaration reads only where live requests overlap, with no change to the frame path or permanent decoded decal residency. Evidence: `DH2-interactions/artifacts/decal-cross-effect-declaration-reuse-20261110.json`.

  Installed declaration observation (`e2922b78`) resolved nine references to four declarations across five FX. Sharing reduced simultaneous parsed identities from nine to four, with identical resolved variant signatures and zero retained references after release in both runs. This covers decoded identity and final release. Timing and authored decal animation were outside the observation. Evidence: `DH2-scene-shutdown/artifacts/installed-decal-leases-20261110-comparison.json`.
- Cancelled FX acquisition (`140022fd0`, `140060180`, `140199df0`). The host drops an unused task handle and schedules cache pruning immediately. A synchronous decoder already running can acquire resources after that prune. Signal a per-path cancellation, keep the cancelled task until it acknowledges completion, then schedule another prune after its result is dropped. Stop at declaration/group boundaries and before acquiring the shared resource locks. A separate live requester still shares a pending load, and a revived path can start a fresh load. Expect no cancelled definition publication or cache prune race, with unchanged successful output. An in-progress group or archive read may finish. Installed cancellation and retirement observations remain pending.
- Outgoing map FX worker retirement (`1408a4830`, `140199df0`, `140060180`). The shared catalog registry locks exclude purge while one declaration is being decoded. They cannot exclude another acquisition by the same outgoing job after that lock is released. At map transition stop issuing requests, signal cancellation, and retain both source and material task handles until they finish. Discard unpublished outputs before the ordinary render acknowledgement and level purge authorize the next map. The loading thread checks once per update and does not join or wait for an archive read. Expect zero outgoing worker publications after purge, with no additional gameplay frame work. Existing active effects retire through the normal FX shutdown. A transition with an installed FX acquisition in progress remains unobserved.

  The installed windowless route (`02d177f2`) recorded one admitted outgoing FX source request in each of Dunwall Tower and Edge of the World. It ran the production retirement gate and cleared both maps with zero loading, retiring, ready, or cached FX sources. The simulation owner was absent before preparation, present after publication, and absent after clear. Both runs also repeated ordinary acquisition and overlapping reuse successfully. This observes source worker and level-owner boundaries. Active emitter teardown, in-flight material decoding, shadow-owner late admission, and pipelined texture mailbox overlap were outside the run. Evidence: `DH2-fx-level-lifetime/artifacts/fx-map-retirement-20261110-comparison.json`.
- Late FX catalog ownership (`1400607f0`, `140060180`). Setup keeps the scene's reference to the same Catalog that the texture worker uses. Previously both EotW and Dust reached runtime material preparation and failed because setup had moved the scene reference away. The windowless rerun (`f136cef7`) published four installed decal materials per map and released every FX-only image and decal asset after the requests ended. EotW published seven new GPU images and Dust six, with one reused scene image. Existing world-mesh proxies were reused in both. The observation covers successful publication and asset retirement, without camera specialization or active effect tails. Evidence: `DH2-fx-resources/artifacts/runtime-fx-publication-v2-20261110-comparison.json`.
- Close admission before host teardown. Winit receives `AppExit` after the final update and clears its main world before `App::run` returns. Waiting until the external `AudioSession` drops leaves its cancellation flag unset while the world's acquisition owners begin to retire. Signal the existing session after the final loading and window-exit systems observe a real exit. This is host scheduling for the shutdown boundary (`1408a4830`, `1400bc8f0`); final cache retirement still waits for admitted work. Expect one close signal per exit, earlier cancellation at existing worker checkpoints, and zero new steady-state allocations or locks. A file read already in progress still has to return. The saved windowless cancellation samples prove the checkpoints only.
- World and player FX ownership (`1408a4830`, `140199df0`, `140821350`, `14021f820`). The map owns simulation state and its particle pools. The player-water/view update supplies frame inputs and does not own those pools. End player particle chains at map teardown before the world manager clears its chains. Prediction: no pool recreation on first camera publication or a temporary missing frame, one fewer initialization of both 1,024-slot pools per normal map entry, and no live player particle slots at the world shutdown boundary. Missing view inputs still withdraw the draw publication. This concerns host ownership and level acquisition, with no lower frame-time claim. Live camera-gap and active player-particle teardown observations remain pending.

  Allocate the simulation with the map's screen FX sources in common scene setup. Previously the host also allocated empty pools before the first map and recreated them immediately after clearing each map. Expect no simulation storage in the empty host or between map teardown and the next scene preparation. Failed preparation discards the unpublished simulation with its setup commands. The windowless scene lifecycle observer records owner presence before preparation, after publication, and after release. It does not run emitters or create a gameplay view.
- Edge skeleton registry (`1400607f0`, `14024f2a0`, `140060180`). The Catalog owns the weak coalescing table, and live skeletons hold weak retirement links. Normalized-path hits borrow the path and reuse the existing key. In the EotW and Dust publication comparison (`e33ec311`), all 131 acquisitions and 79 installed source signatures matched. Both runs decoded 122 skeletons, reused nine and released all live payloads. The catalog-owned table retired with zero entries and seven reserved slots. This observes acquisition, reuse and retirement, without a timing inference. Evidence: `DH2-interactions/artifacts/catalog-skeleton-registry-20261110-comparison.json`.
- Wwise stream location ownership (`1400bc8f0`, `1410bf740`, `1410f1910` / `141122400`). The event resolver already looks up a Sound's package using its owning bank and language. Retain that bank identity in the streamed reference until graph decoding and keep the locator table local to that acquisition. Expect zero level-retained lookup entries, no cross-event lock or overwrite during parallel preload, and unchanged media extents for unambiguous IDs. Only the graph's actual stream sources retain their locators. The existing resident package-entry reader can also fill fallback resident media directly, avoiding one temporary buffer and payload copy per full package read. Header-only and prefetched stream acquisition retain their current reads. The lifetime prediction concerns acquisition and its scratch storage, with no steady-state voice cost or lower frame-time prediction. Installed publication and scoped timing observations remain pending.

  Installed publication (`ff26580f`) matched the previously accepted EotW / Dust / EotW run across 27,177 prepared-media records and 5,837 stream-format records. Encoded residency and stream acquisition counts and read byte totals matched at all three map checkpoints. An independent installed package comparison found 76 cross-package IDs in the EotW selection and 81 in Dust, with byte-identical duplicates in both. The observer used actual installed source acquisition and publication. Voice playback, the full-media fallback and scoped performance were outside that observation. Evidence: `DH2-scene-shutdown/artifacts/wwise-event-stream-locators-20261110-comparison.json` and `wwise-stream-bank-identities-20261110.json`.
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

Final owner observations recorded 15 cached readers after 55,629 acquisitions
in the EotW / Dust / EotW route and 16 after 29,113 acquisitions in the EotW
quality route. Both counts were emitted by the catalog reader pool's destructor.
Keeping the readers for that session preserves map-to-map reuse while the
catalog drop closes the physical files. Evidence:
`DH2-scene-shutdown/artifacts/wwise-event-stream-locators-20261110-publication/stderr.log`
and `artifacts/resource-quality-blocks-20261109/stderr.log`.

Shadow retirement acceptance: the same production client exited 0 without
shadow-target retirement events. Source-absent entries are now pruned before
revision reuse, but this route did not replace an atlas within one map. No
measured GPU memory saving or live quality/resolution coverage is claimed;
that broader coverage remains open. Existing map-boundary cleanup is unchanged.

Texture mailbox retirement (`140060180`, `14030d1e0`). The pipelined host can
dispatch another render schedule after the outgoing acknowledgement and before
the main world's next update resets the mailbox. An upload batch already taken
by that render schedule could append old waiting jobs or completion records to
the new generation. Recheck the generation at the existing publication lock.
Expect zero old records or deferred payloads republished after a reset, and one
comparison per upload batch with no additional lock or GPU operation. This is a
host handoff correction. Exact overlap timing remains unobserved.

The `14030cce0` request order operates on registered images. On the host an
FX slot can outlive its old image as an empty reusable index. `r_addAllMips`
previously admitted those empty indices and could queue a read that completed
after another image reused the same index. Skip unassigned slots when building
the request order. Expect zero reads for retired slots and unchanged ordering
for live images, including in all-mips mode. The installed DX12 observer
(`5ac96a60`) published and released two Tower FX image slots and seven
Edge of the World slots. The production all-mips ordering returned 1,974
and 4,143 registered requests respectively and zero retired slots. All
observed FX assets released while their level stayed alive. This exercises
request ordering on real retired slots. A subsequent slot reassignment,
live rendering, and map-generation overlap were outside this observation.
Evidence: `DH2-fx-level-lifetime/artifacts/runtime-fx-retired-slots-20261110-comparison.json`.

A later seven-snapshot EotW quality route (`6b15daea`) observed four real
parallel/local target replacements at settings generations 3, 4, 6, and 7.
Each replacement retired one old parallel target and kept the current one.
All four local atlas/cache identity pairs changed, and the two settings-only
snapshots retained their existing pairs. This covers ordinary replacement
while both owners are present. It does not cover an owner first created after
startup. Evidence: `artifacts/resource-quality-blocks-20261109/stderr.log`.

Late shadow owner retirement (`1400410f0`). A quality change can create a
parallel or local shadow owner after startup, including a level that started
without that owner. The startup snapshot cannot record this later insertion.
Release both owners explicitly after the outgoing render acknowledgement and
the ordinary level resource removals. Expect any late-created image leases to
retire at this boundary, with two resource checks per level unload and no
frame work. This edge still needs an actual late-admission observation.


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
The first live named wrapper uses the committed scene/common view.
`14101d290` first searches the global Iggy wrapper table by name and adds the
new context reference through `14100d6f0`. It calls the texture resolver only
when no wrapper is live. `1402df8c0` likewise acquires the SRV on its first
reference and reuses the saved handle for further references. A streamed
replacement leaves that named snapshot available to all holders of its live
wrapper. After the last wrapper reference releases, the next wrapper uses
the current committed view. The host's weak named-binding lookup represents
this sharing without extending the wrapper lifetime. Registered current
views change only on successful streaming commits, including inactive
common-image reductions. This scope is established by executable code.

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
images to `14031a410`; its observed callers cover surface/material paths. At this point in the investigation, the rewrite used an inline view outside
the shared streamer. The subsequent implementation and its measured handoff
are recorded below; it does not force full resolution or use map-name exceptions.
Local evidence: `artifacts/resource-residency-eotw-verified.json` and
`artifacts/resource-eotw-water-stream-records.json` in the main checkout.


A 30-second, 300-sample read-only observation of the loaded Edge of the World
streamer (`artifacts/resource-eotw-mask-observation.json`) found the amplitude
mask registered in every sample, with zero area and a 256x256 view throughout.
This covers one unattended camera position only; it does not establish behavior
while moving or across other environment consumers. The native registration
routine `14030dbf0` inserts the image identity with zero initial area and
priority field 1 before any producer updates it. Shader binding records confirm
the mask is consumed directly as an ocean environment input; those bindings
alone do not show a call to the material demand producer `14031a510`.


During the user-driven waterfront walk, the original mask's streamer record
was sampled 900 times over 91.2 seconds, starting before movement; the separate
post-walk image capture occurred 31.4 seconds after sampling started. Every
sample found the record, area zero and 256x256 view. Sampling at 100 ms cannot
exclude shorter demand spikes. The image loader `1402e9ff0` registers any image
whose current mip count is below its complete count through `14030dbf0`;
registration is independent of whether the image has a material consumer.
This confirms that retaining the inline view matches the observed default path,
and informed the later general streamer registration and view handoff below.
Evidence: `artifacts/resource-eotw-mask-moving.json` and
`artifacts/resource-residency-eotw-after-movement.json` in the main checkout.


Environment image handoff prediction (before implementation): registering
streamable environment images in the existing scene image owner should reuse
one initial GPU image identity, preserve the inline chain where it is the only
consumer, and allow the ordinary budget/eviction machinery to publish later
committed views. Ocean rendering must replace its cached view after a new asset
is GPU-ready, keeping the old view until then; the existing head-query binding
already compares texture view IDs. The default captured walk has no demand,
so it should produce no added finer upload and the same 256x256 mask/format.
Measure image identity, descriptors, upload counts and release with actual
levels; do not claim a frame-time gain. Demand must come from evidenced native
producers, not a fabricated per-frame ocean visibility value.


Installed world-environment audit: the existing `material-audit` reader
resolved world environment declarations for all 36 indexed worlds. Across
those world-level declarations it found 28 distinct ocean image names, all
resolved; seven have an indexed `_mip0`: five boat variants, Dunwall escape
streets, and Edge of the World (the latter referenced by both chaos variants).
Each of those seven has an inline 256x256 seven-mip chain; six use BC7 sRGB and
one uses BC4. Area-specific environment overrides were not part of this
world-declaration scan. The implementation discovers sources from prepared
world and area environments and checks the installed index, rather than using
this list as an allowlist. Evidence: local `ocean-environment-worlds.json` and
`ocean-environment-images.json` in the feature checkout's artifacts directory.


The follow-up installed-declaration scan read all 121 indexed environment
files directly, covering declarations available to both world and area
selection. Nineteen contain explicit ocean image values; their 29 distinct
names differ from the world-only set by one additional source,
`models/rendering/water/vc_ocean_m.tga`. Its installed compiled image is a
complete 256x256 seven-mip BC4 chain with no indexed separate mip. Therefore
the seven streamed image names above cover these explicit declaration values;
this static scan does not claim every declaration is reachable or cover images
introduced by systems outside ocean environment parameters. Evidence:
`ocean-environment-all-decls.json` and the installed index lookup.


A live Edge of the World cross-check of all matching ocean image names in the
121 declarations confirms that the indexed-finer-mip distinction matters:
`vc_ocean_m.tga` has seven resident mips but no streamer entry, while the
amplitude mask has seven resident mips and a finer indexed mip and is
registered. One-mip shore, foam and intrinsic textures have no streamer entry.
This supports registering by available finer data rather than mip count alone.
Evidence: `artifacts/resource-eotw-environment-streamer.json`.


Before retiring replaced inline leases: once a scene-owned stream commits a
new handle for an environment image, its old inline registry entry otherwise
keeps an extra GPU asset alive for the rest of the level. Remove only that
registry lease when the published scene handle differs. Existing snapshots and
in-flight commands keep their own handles; the current scene handle remains
owned by the streamer. Prediction: a forced finer-mip transition releases one
obsolete inline registry lease per changed image, without changing payload,
frame work when no image changes, or rendering order. The release does not
claim immediate physical heap reduction; renderer and in-flight users can hold
the old view until they finish.

Layered image staging prediction (before implementation): installed BIM cube
and array readers validate records, then copy every payload into a second
allocation while the full decoded archive member remains live. For owned
archive reads, validate the exact same records and compact their payloads into
the archive allocation in place; borrowed callers still receive an owned
payload. Prediction: remove one payload allocation and a simultaneous full
payload-sized backing per acquired cube/array, with unchanged bytes, dimensions,
formats, layer and mip ordering. A small metadata range list replaces no large
buffer. Verify actual production descriptors, payload fingerprints and
allocation counts with the old copy path selected by a diagnostic control; do
not infer RSS or CPU timing gains from payload sizes.


Reflection-array preparation prediction (before implementation): its owner
reads a cube into a source buffer, creates a second full cube payload, then
copies that payload again into the required layer-major combined image. Reuse
the validated record ranges to append directly from the source into the final
combined array in the same face/level order. This removes one full cube staging
allocation per probe; the final combined GPU upload payload and its ordering
remain necessary. Compare the real copy and direct paths by per-cube output
checksums, descriptors and allocation counts, not whole-frame CPU timing.


Reflection final-buffer prediction (before implementation): each cube has an
identical validated descriptor, so the probe count and first cube's validated
record ranges determine the exact final array size. Reserve it once before
appending the first cube; the legacy diagnostic keeps geometric growth.
Prediction: final-array capacity changes at most once rather than growing with
the number of probes, with identical layer-major bytes and no extra full-sized
buffer. Report production capacity-growth counts and payload sizes; allocation
counts are not whole-process memory or CPU-time measurements.


The global `r_addAllMips` production diagnostic was stopped after it saturated
the 4 GiB streamer budget and repeatedly evicted unrelated textures before the
loading gate completed. Its output does not verify the handoff. A diagnostic-
only control now submits visible-area demand to the registered environment
slots, exercising their ordinary read/upload/commit path without forcing every
other image. The control is absent from normal play and does not represent a
claim that the original normally submits this demand.

Streamed-media diagnostic table prediction (before implementation):
`media_from_sound` scans bank DIDX and locks a process-global map for every
streamed source, but the table is read only inside load-timing diagnostics.
The native sound source owns media through its bank/package (`141122400` /
`1410f1910`), not this host-only aggregate. Populate the aggregate only when
load timing is enabled; leave the separate prefetch-prefix lookup and source
ownership unchanged. Normal mode should retain zero table entries and perform
zero table inserts or diagnostic DIDX scans, versus one insert per streamed
source before; debug timing keeps the existing reporting behavior. Measure
source and table counters in the production client, without a whole-frame CPU
benchmark or a claim about physical RAM saved from entry counts alone.


Layered staging first production comparison (source `3ff5959`): both copy and
in-place clients completed the same four-level sequence with exit 0. Six water
cube/array acquisitions carried 44,913,760 cumulative payload bytes and used
6 -> 0 separate full-payload allocations. Reflection preparation processed 343
cube requests / 179,819,808 cumulative payload bytes and used 343 -> 0
intermediate full-cube allocations. All recorded preparation identities,
reflection descriptors and per-cube fingerprints, and layered GPU-upload
fingerprints match. The final reflection-array requested capacities changed
from 67,108,864 / 16,777,216 / 67,108,864 / 67,108,864 bytes to 54,522,624 /
12,582,144 / 58,192,416 / 54,522,624 respectively. These are buffer-capacity
and cumulative allocation measurements, not RSS or CPU-time measurements.
The initial instrumentation counted growth per probe, not every reallocation;
the later source corrects the counter and still needs its own production run.
Evidence: `artifacts/layered-inplace-first-comparison.json` and paired logs.


Environment image handoff production acceptance (source `3ff5959`): all three
clients completed four levels with exit 0. In default mode, Edge of the World
reused the registered 256x256 BC4 inline image on both visits (43,688-byte
payload, CRC `30063ac6`); no finer request was injected. A separate targeted
diagnostic submitted demand only to environment slots. In that run the
streamer requested, uploaded and committed the 512x512 mip for Edge of the
World BC4 (131,072 new bytes) and the Curator boat BC7 (262,144 new bytes).
The ocean owner recorded the committed asset IDs and new GPU views. Its old
inline registry leases were retired (43,688 and 87,376 logical bytes,
respectively); these are released references, not physical-heap measurements.
The EotW return repeated the same transition. World and area sources use the
same indexed-resource rule; only these two formats/maps were exercised in this
runtime comparison. The diagnostic demand is not present in ordinary play.
Evidence: `artifacts/ocean-stream-targeted-comparison.json`, paired logs and
client results. No tests or visual inspection were performed.

UI bitmap shared-backing prediction (before implementation): `1402e2c00`
consumes installed Iggy texture records directly. The rewrite reads each raw
member and separately copies every eligible aligned BC7 image into its own
buffer. Installed index sizes and the measured loading/pause run show that the
12 used raw members total exactly 34,556,544 bytes, equal to the 430 selected
image payloads. Where validated direct-image ranges cover a complete member
without gaps or overlap, keep that one allocation behind shared immutable range
views; fall back to the existing per-image copy/decode otherwise. Prediction:
for those eligible members remove 430 full-image buffers and 34,556,544 copied
bytes while preserving every payload, format, dimension and upload; resident
payload should remain the same 34,556,544 bytes rather than grow from retained
padding. Compare production per-image fingerprints, source/retained bytes and
allocations against a same-client copy control. No CPU-time or RSS gain is
assumed from copy counts.


Follow-up production comparison (source `cad105e`): copy and compact modes
both completed four levels with exit 0, and all layered upload descriptors and
fingerprints and all 343 reflection output fingerprints match again. Corrected
per-append instrumentation recorded reflection-array capacity growths of
11 / 9 / 11 / 11 in copy mode and 1 / 1 / 1 / 1 with reservation. Across the
same eight sound-preparation checkpoints, both modes recorded matching counts
and an order-independent fingerprint of media IDs, stream types and sizes.
The former diagnostic table performed 7,355 inserts and retained 2,441 entries;
the opt-in implementation performed zero inserts and retained zero entries in
normal mode. The audio fingerprint describes source identities and sizes, not
PCM equality or measured CPU time. These diagnostics do not establish process
RSS savings. Evidence: `audio-owned-comparison.json` and
`audio-owned-layered-comparison.json` in the feature checkout artifacts.


Installed-index scope for UI bitmap backing: 54 distinct `.raw.iggytex`
members total 157,096,320 uncompressed indexed bytes across the installation;
this is not a claim that all are loaded together. The loading/pause production
sequence above uses 12 members. The complete-partition check is applied per
member and falls back for gaps, overlaps, partial mip selection or decoded
formats, so other libraries are not assumed to have the same layout. Local
index evidence: `artifacts/ui-bitmap-all-index.json`.

Production comparison for the shared UI backing: client source `2ab52d5`,
loading and pause across Edge of the World, Curator boat, Dust District and
return to Edge of the World. All 12 acquired libraries met the complete
partition rule. The copy path allocated 430 image payloads and copied
34,556,544 bytes; shared backing made zero per-image payload copies, retaining
34,556,556 bytes of source capacity (12 bytes above payload size). All 430
image identities, dimensions, mip counts, formats and CRCs matched. Both
clients exited successfully. These are allocation/copy counts and payload
signatures, not a process RSS or CPU timing measurement. Evidence:
`artifacts/full-owned-ui-comparison.json`.

### Common placed-model residency scope

The installed common manifest contains 131 static `model` entries and 88
`baseModel` entries. A production-client startup trace on Edge of the World,
Dust District and return to Edge of the World found 13, 11 and 13 distinct
placed static resource paths from the common manifest respectively; nine of
the paths occur in both levels. Their indexed uncompressed source sizes total
776,914 and 516,540 bytes for the first two levels. The trace also identifies
one common skeletal base path in Edge of the World. These counts concern paths
observed before setup's filtering, and source byte sizes are not decoded mesh
size or GPU residency. The per-map model scratch is currently discarded after
mesh preparation, whereas the generated mesh and its shadow, skin and vegetation
attributes have consumers with different mutation and lifetime requirements.
Keeping all parsed common model geometry would add resident CPU memory; a safe
session mesh owner must first establish immutable variants and cover those
consumers. This evidence does not demonstrate the cost or safety of that change.
Local evidence: `artifacts/model-common-trace-summary.json`,
`artifacts/lifecycles-manifest-models.json`, and the successful three-level
`model-common-trace` client.

### Texture-event diagnostic allocation prediction

`StreamingDiagnostics::texture_event` discards events immediately unless
`DH2_TRACE_TEXTURE_STREAMING=1`, but each of its 15 streamer call sites first
constructs a formatted `String`. With tracing off, those allocations and all
formatting have no consumer and do not affect streaming or F8 history. Pass
format arguments to the method and format after the existing trace decision.
Prediction: during the same production traversal, the number of formatted
messages with tracing off falls from the number of attempted events to zero;
with tracing on the event text and ordering remain unchanged. No changes to
texture choices, uploads, history or scheduling are expected. Count only event
attempts and formatting, not whole-frame CPU time or process RSS.
The earlier four-level targeted-demand production trace retained 17,385
texture-event lines; three of four logs also report dropped diagnostic events,
so that is a lower bound for that run rather than its exact total. The event
classes include demand entry/exit and read/upload transitions. This trace was
collected with logging enabled and does not by itself measure the disabled case.



Texture-event production comparison: three EotW clients of the same build
completed an identical moving camera route. At the third 600-frame checkpoint,
the prior eager-format control formatted 31,498 of 31,498 attempted events
with detailed tracing off; the default formatted zero of 31,433 attempted
events. Different attempt counts reflect runtime scheduling; the measured
formatting ratio is 100% versus 0%, not a CPU-time or RSS measurement. With
tracing enabled, 31,154 of 31,154 attempts were formatted, and all eight event
types observed in both the historical trace and this run had matching field
layouts. This compares diagnostic text structure, not a frame-for-frame trace
or pixel output. All three clients exited 0. Local evidence:
`artifacts/texture-event-comparison.json` and
`artifacts/texture-event-trace-layout.json`.

### Texture completion and upload scratch prediction

The read-completion consumer collects a temporary vector and immediately
moves its items into its existing `ready` queue. Draining the receiver directly
into that queue preserves channel order and avoids at least one temporary
vector allocation per nonempty completion batch. The channel sender does not
take the receiver mutex; the queue remains owned by the current map.

The GPU mailbox takes its pending vector each render frame and builds separate
waiting, completed and failed vectors. Reusing bounded render-system scratch
should eliminate capacity growth after warmup, preserving deferred jobs before
new arrivals, upload budget/order, all completion/failure acknowledgements and
map teardown. Drained scratch must hold no payloads at frame end, and its capacity must be released when the mailbox is reset for a new map. Compare
capacity-growth and batch counters with a same-build reference path; compare
image descriptors and request/upload/commit event identities. These are
transient-allocation counts, not resident-RSS or whole-frame timing predictions.


Texture completion/upload production comparison: same-build reference and
reuse clients completed Edge of the World -> Curator boat -> Dust District ->
Edge of the World. Reference completion batches used 572,320 cumulative bytes
of temporary vector capacity across 265 nonempty batches; direct transfer used
zero temporary batch capacity for the same 1,437 received items. Across the
first three map retirements, waiting/completion scratch grew on 167/172 frames
in the reference and 6/3 in the reuse path. Reuse scratch had capacities
32/32/16/0 entries and lengths 0/0/0/0 at each mailbox generation change,
then released those buffers. All 13,457 logged image packing descriptors and
checksums matched, as did the 1,828 observed request/upload event identity
pairs. Three event logs per run reported dropped diagnostic events, so these
sets are not a complete event-order comparison. These are temporary-capacity
and ownership counts, not process RSS or CPU-time measurements. Both clients
exited 0. Evidence: `artifacts/texture-scratch-v2-comparison.json`.


### Completed-read path ownership prediction

Each received image read clones its indexed resource path before handling the
result, but uses the clone only for immediate diagnostic or error messages.
Borrowing the indexed path at each message site avoids one temporary path
String per completion; the separate path owned by each asynchronous request
remains necessary. The previously captured four-level trace recorded at least
1,437 completion events and 130,498 path bytes, excluding dropped events.
Compare actual completion and cloned-byte counters with a same-build reference
over map transitions; retain the exact trace/error text and image descriptors.
This predicts temporary allocation reduction, not an RSS or CPU-time change.


Final combined production comparison: the four-level reference and reuse
clients both completed. The reference made 1,437 temporary completed-read
path copies containing 130,635 cumulative path bytes; the default made zero.
The reference also used 568,960 cumulative bytes of temporary completion
vector capacity, while direct transfer used zero. At the first three map
retirements, waiting/completion upload scratch grew on 169/172 frames in the
reference and 6/3 in the reuse mode. All reused buffers were empty at each
generation reset and their capacity was released. The 13,457 logged image
packing signatures and 1,828 observed request/upload identity pairs matched;
three logs per run reported dropped events, so this is not a complete event
ordering comparison. Counts describe cumulative temporary allocations and
buffer ownership, not process RSS or CPU-time changes. Evidence:
`artifacts/texture-scratch-v3-comparison.json`.

Streaming priority ownership prediction: original 14030cdb0 derives the
priority class from the current image record's path. The host cache currently
rebuilds classifications only when the slot array changes length. Reusing a
retired FX slot can keep its previous path class and therefore change the new
image's request and reduction order. Classify each path on admission and keep
its class beside the projected demand in the existing record's alignment
padding. Inactive shared images retain that class with the mip state. This
removes the separate two-byte-per-slot cache, its full
path rescan on append, and the second classification on shared-image capture.
Existing slots should keep the same priority keys. Reused slots should get
the same key as a fresh slot for the same path and demand, apart from the
existing slot-index tie break. No per-frame path processing is added.

Texture request scratch lifetime prediction: the streaming system currently
retains its request flags, priority keys and instance-to-image index in Bevy
system locals. They survive `LevelLifetime::clear` and are rebuilt when the
next scene queue is first observed. Giving those buffers to the per-map
texture worker should release their outgoing capacities with the worker,
including on close. The same instance and request order should reach the
streamer, with no new allocations on ordinary gameplay frames. This is a host
ownership change at the map retirement boundary (1408a4830), separate from
the earlier mailbox-scratch measurements.

The game-feature check on db1cb92a compiled the exact texture sources now
integrated. LevelLifetime removes the texture worker with the outgoing map,
which owns its request state and cached instance slots. Projected demand
carries the original name-based priority initialized for each newly admitted
identity, including reused FX slots. Existing priority-key construction is
unchanged. Live slot reuse, affected CPU cost and mip publication still need
runtime observations. Evidence: DH2-scene-shutdown/artifacts/resource-shutdown-batch-20261110-check-provenance.json.


Initial texture worker prediction. Original 1402e9ff0 hands validated
authored inline records to 1402f5500. The host worker currently allocates one
new byte buffer per inline mip even when the upload already owns the decoded
archive member. For initial inline-only worker jobs, keep that member once
and hand checked mip ranges to the upload queue. Expect no second payload
allocation or copy, with only record headers and the archive capacity slack
retained until upload. Keep separately streamed mip jobs on their existing
direct acquisition path. Compare matching mip order, dimensions, payload
signatures and GPU publication before claiming a gameplay memory or CPU gain.

Installed worker upload acceptance (0b3f82fa). Four same-executable
observations loaded 32 archived images across five formats and one to seven
inline levels. All inputs matched the previously observed installed CRC,
dimensions, format, mip count and next finer suffix. The real GPU upload
mailbox accepted every job. After upload all decoded archive owners were
released, and after the diagnostic released its image handles all 32 GPU
identities retired. Per run the old path copied 25,838,832 payload bytes
into new mip buffers. The new path reused one decoded member per image
(25,843,168 bytes of total member capacity, including 4,336 bytes of
headers and slack). These figures describe overlapping copies avoided
during acquisition, not steady-state memory saved. Scoped reader cycles
were 209.5–216.4 million copied and 190.0–191.1 million archive-backed.
Live gameplay demand and cancellation were outside this windowless
observation. Evidence: DH2-fx-resources/artifacts/worker-inline-images-20261110-comparison.json.

Worker initial-chain acquisition prediction. When an explicit initial mip
selects separate files, the worker reads each full member and copies its
payload even though the upload can own the buffer returned by
`read_separate_mip`. Use the same direct reader already measured for finer
upgrades (14030e840 and 14030f030) and transfer its payload unchanged to the
initial chain. Keep the existing resident-image packing path and dimension
checks. Expect one fewer overlapping whole-member allocation for each
selected separate worker mip, with unchanged upload bytes and suffix order.

Installed acceptance (`1f85fde2`). The same game-feature executable read 30
installed mixed chains in each mode. All 35 separate-member reads selected the
same members, totaling 13,042,364 decoded member bytes. The upload payload CRCs,
formats, dimensions, and mip selection matched. Both runs published all 30
images through the render mailbox and retired all observed image and archive
backing leases. This verifies acquisition and upload ownership. Live demand and
release CPU timing were outside the observation. Evidence:
`DH2-interactions/artifacts/initial-separate-worker-mips-20261110-comparison.json`.

### Fog curve resource ownership

The original `1402edb60` constructs a temporary curve-image builder on the
stack; `1402ade20` acquires the `.curvefloat` declaration and calls
`1402add00`, which fills 4,096 float samples, and `1402adf20` creates and
uploads the image. The existing original EotW RenderDoc capture records the
result as a 4096x1 `R16_FLOAT` texture. Its complete 8,192-byte payload is
identical to the installed EotW `colorlut.bimage7` payload used by the rewrite.
The captured image descriptor's original format word also maps to DXGI 54
through `1402c2f50`. All three existing captures contain that same payload;
they are not independent map coverage.

The installed index contains 16 `curvefloat` declarations; every one has a
same-stem compiled image, and all 16 images specify the same 4096x1 R16F
descriptor. Their payloads were not individually compared with the original
runtime. Using the installed compiled image avoids running the original's
curve-sampling/conversion step and lets the shared image owner handle reuse and
map purge; this is a host representation change, not a claim about CPU time or
all scripted curve behavior. Evidence: original decompilation and local
`artifacts/fog-curve-capture-comparison.json`,
`artifacts/fog-curve-index-mapping.json` in the feature checkout and
`analysis/fog-runtime-inputs.json` in the main checkout.


### UI library session scope

The production loading/pause route acquired 12 bitmap libraries. Matching
their actual `.iggy` paths against the installed `init` and `common`
`.mapResources` records finds four in `init` and all 12 in `common`. This
supports keeping those observed immutable libraries for the session instead
of repeatedly reading them on a map transition. The EotW and Dust map lists
each contain the separate `interactive_map_legend` Iggy declaration/import,
which this route did not acquire; this evidence does not establish eviction
behavior for future map-only movie libraries. Evidence:
`artifacts/ui-library-manifest-scope.json` and
`artifacts/full-owned-ui-comparison.json` in the feature checkout.

A scan of all 38 installed `init`, `common` and map manifests found no
direct map-exclusive `.iggy` path, but that does not settle indirect movie
imports: `interactive_map_legend.iggyfile` names `map_legend.swf`, and its
map-scoped `.imp` lists `lib_map_legend.swf`. The indexed
`ui/map_legend.iggy` is 173,585 bytes and is outside the common direct-path
set; the imported `lib_map_legend.iggy` and its bitmap data are common. The
current loading/pause route never opens this movie, so the measured session
cache results do not establish its lifecycle or savings from evicting it.
Evidence: `artifacts/ui-map-only-libraries.json` and the installed declaration
and import record.

All 13 parsed Iggy files observed in the loading/pause production route occur
in the common list, including the font library. A direct scan of all 39
indexed `mapInfo` declarations found one loading-movie path,
`ui/loading_screen.iggy`, also in common; the two pause movies and global
message-box movie named by the current client are common as well. Keeping
these currently used files for the UI worker therefore matches their session
scope. This does not extend the claim to the currently unimplemented
interactive-map movie.


### Map-info lookup prediction

The original init/common manifests register 28/39 mapInfo resources and
`1400607f0` / `14005feb0` resolve their resident identities. The rewrite
currently scans and parses mapInfo declarations in archive order for every
previously unseen map, discarding all nonmatching parsed declarations. An
incremental index of only the discovered low/high map paths and declaration
indices can skip already-scanned records, while caching full declarations only
when a requested map actually uses them. Low/high aliases of the same declaration
should share one immutable `Arc`; per-map callers retain their own reference.
Two installed map names occur in multiple declarations; the index keeps the
first declaration in the same sorted order as the existing scan. Modded
mapInfo overlays keep the existing scan behavior so in-session overlay changes
do not invalidate a speculative index. A parse/read failure likewise
falls back to the existing scan. Independently opened catalogues for the same
installation can contain different overlay snapshots; the index and last-map
cache must also invalidate on catalogue identity, rather than only on path.

Installed data contains 39 declarations; in sorted order the EotW, Curator
boat and Dust targets are positions 17, 1 and 15. For the EotW -> Curator boat
-> Dust -> EotW route, predict 33 declaration reads with the existing cache
versus 19 with incremental indexing. Appending EotW high chaos to the route
is expected to raise the reference to 50 reads while the indexed path shares
the already requested declaration and stays at 19. The indexed uncompressed
resource sizes predict 200,076 -> 77,776 decoded bytes for that five-load
route. The 39 indexed resource paths contain 2,818 UTF-8 bytes, and the 59 quoted map-path
entries found in those declarations contain 2,817 UTF-8 bytes before deduplication; vectors, hash
tables and allocator overhead are additional. This session metadata replaces
repeated scans without retaining full parsed declarations for unrequested maps. Count
reads, decoded bytes and index storage with same-build controls and compare
resolved declaration fingerprints. These are I/O and logical metadata counts,
not a CPU-time or process-RSS prediction.


### Map-info lookup result

The same-binary EotW low / Curator boat / Dust / EotW low / EotW high route
completed in both modes. Reference and indexed reads were 50 -> 19, with
200,076 -> 77,776 decoded bytes, exactly as predicted. All four distinct
map/declaration fingerprints match; the high-chaos alias reused the already
parsed EotW identity with zero reads. The indexed route retained 3,822 UTF-8
string bytes for resource paths and discovered map keys at its final lookup;
vector, hash-table and allocator overhead are additional. These are read and
logical-string counts, not process RAM or CPU timings. Mod overlay and read-error
fallbacks were checked in code; these successful clients do not exercise them.
Evidence: local `artifacts/mapinfo-index-v2-comparison.json` and paired client
logs. No tests or visual inspection performed.

### Loading read-history ownership investigation

The original load loop at `1400661e0` advances its request cursor after
`1400607f0` acquires the indexed resource identity. The rewrite records a
`Catalog::read` after a successful archive or overlay acquisition, and
keeps the record across world teardown unless its owner retires it.
`begin_level` treats every remaining historical read as already acquired,
including map-scoped data that its owner released without retiring the record.
Consequently this is a read-history approximation, not an accurate resident
registry or original request cursor; readiness gates remain independent.

Before changing it: successful reads should still be counted once in the
current load, failed reads should contribute zero, and a released map resource
should not be counted at the next load until reacquired. Cache hits must be
recorded by the actual owner or derived from its live identities, including
init/common and shared image, animation and UI resources. Merely preserving
every path named in a common manifest is insufficient evidence of residency.
Measure per-kind retained entries/bytes and current-load acquisitions on a
multi-map route; do not claim a CPU timing or physical RAM result from path
counts.

Existing `artifacts/full-owned-compact.err` reports permit an exact history
entry count without a new run: for each kind, listed-and-read plus unlisted
read paths equals the session set size. On its four-load EotW / boat / Dust /
EotW route, the totals are 22,857 / 24,643 / 29,922 / 29,961 distinct paths.
The history grows by 7,104 entries by the return to EotW; this is metadata
count, not measured bytes or proof that all of those entries are stale. The
return's listed-and-read count is 20,357 versus 20,138 on the first visit;
that difference alone cannot distinguish actual shared owners from stale
history. The animation-owner comparison recorded 38 map-only clips released
at each transition with zero outgoing map leases; their read-history entries
nevertheless remain after the owner releases them. A gated diagnostic will
measure path capacities and precredited entries; actual owner acquisition/release
events are needed to classify the full set.


Animation history release prediction: the compressed-clip owner already knows
which weak entries have no remaining leases at the map boundary. Removing only
those exact paths from the loading read history should remove 38 entries at
each measured transition in the EotW / boat / Dust / EotW route, while leaving
the 162 pinned common clips and any live map leases untouched. On return, the
38 released clips should count only after successful new acquisition; common
clip reuse should require no new reads. Those 38 reacquired paths in the existing
production log total 3,844 UTF-8 bytes; allocated string capacities may differ.
Measure retained path capacity and removed entries at the owner boundary, plus
the existing animation acquisition fingerprints. This changes transition bookkeeping, not gameplay-frame work;
this is not a CPU time or process-RAM prediction. Other resource owners need
their own release integration.


### Probe-network staging prediction

The original network loader `1402b15e0` reads from its input stream and
acquires probe identities while loading; it does not need the rewrite's whole
source file to remain live after decoding. In `load_for_world`, names, spatial
tables and vertex-reference buffers are owned values: once all three have been
parsed, the `network` Vec has no remaining reader, but its destructor currently
runs only when the function returns after every individual probe has loaded.
Release that source before the probe loops. Prediction: at the start and end of
those loops, source staging capacity falls from the network's measured capacity
to zero (installed EotW is 247,791,156 decoded bytes and Dust District is
321,048,211), with the same probe identities, spatial and vertex-reference counts, and probe data. This
reduces overlapping live allocation, not necessarily process RSS or gameplay
CPU time. Compare retained-source and early-release modes in production clients
on multiple maps; no per-frame work or output precision changes. An installed
index scan found 36 distinct probe-network paths and all 36 are compressed;
decoded sizes range from 2,333,900 to 321,277,840 bytes. The early release
still allocates the decoded input during parsing. Incremental decompression
and parsing are the next lifecycle step, using the original's stream order;
it must preserve every parser validation and output while avoiding a second
whole decoded file and without imposing a gameplay-frame cost. All 36 indexed
networks are PNW v7 with Voronoi v9 and feature byte zero; their primary probe
counts range from 63 to 5,085. The existing parser accepts older supported
PNW versions as well, so a replacement must preserve those rather than specializing the game
files. Prediction: stream each compressed member once, keeping bounded input
buffering and the same required output arrays; remove the full 247,791,156-byte
EotW and 321,048,211-byte Dust staging allocations even during parsing. Compare
all parsed records and the affected parsing/decompression time, not whole-frame
time; record actual buffer capacity and investigate a result beyond the stated
prediction before accepting it.


Probe history release prediction: the production probe loader owns the parsed
`.bprobe` data through `ProbeLightingSet`, shared by scene and particle
consumers using `Arc`. All 71,594 distinct installed `EnvProbe` index paths
end in `.bprobe`; source search found the game loads these through this owner.
A shared lifetime lease should remove that kind's read-history set only after
the last probe set is gone, while a new/in-flight set keeps it alive. Existing
EotW / boat / Dust / EotW snapshots report historical EnvProbe totals of
3,312 / 3,915 / 6,740 / 6,740. Prediction after owner-driven retirement is
3,312 / 603 / 2,825 / 3,312 for the respective active sets, absent unrelated
reads; that avoids retaining outgoing path strings without touching their
parsed probe data or gameplay-frame work. Measure removed entries and string
capacity at the actual last-owner drop and compare probe fingerprints;
allocator buckets and process RSS are separate quantities.


### Animation history and probe staging results

The same-build four-load EotW / Curator boat / Dust / EotW comparison
completed in both probe-staging modes. The animation owner removed exactly
38 stale history paths and 3,844 bytes of string capacity at each of three
transitions, while retaining 162 common clips and reporting zero outgoing map
leases. Its 800 clip acquisition fingerprints match the first four loads of
the pre-change production route; each route reused 486 clips. This is path
metadata and acquisition evidence, not an RSS or CPU-time measurement.

The retained vs early-release comparison reports the same parsed-network
fingerprint for every load and the same probe names/counts. The source Vec
capacity alive during individual probe acquisition was 247,791,156 -> 0
bytes in EotW, 43,103,144 -> 0 on Curator boat and 321,048,211 -> 0 in Dust.
This removes overlapping live source allocations; it does not establish a
process-RSS change. Both production clients exited successfully. Evidence:
local `artifacts/lifecycle-batch-comparison.json` and paired logs. No tests or
visual inspection were performed.


### Contact declaration reuse prediction

`app::audio::contacts::texts_at_load` rereads its complete init/common/map
contact, physics-material and sound-event list for every world, then stores
per-level decoded text. Original `1400661e0` requests init/common separately,
while `1400607f0` / `14005feb0` reuse their registered identities. Installed
manifest/index inspection found 9,009 distinct matching common declarations
(1,408,166 indexed decoded bytes), no matching init entries, and 382 / 389
additional map entries for EotW / Dust. The latter sets have no path overlap
with common in those maps. Assuming all selected reads succeed, EotW -> Dust ->
EotW currently needs 28,180 distinct-per-load reads and 4,428,182 indexed
decoded source bytes; retaining one immutable common text table should reduce
that to 10,162 reads and 1,611,850 bytes. Per-map entries must still release
with their owner. Do not keep a second common copy in each map, and do not
cache mutable mod overlays. Compare actual successful read counts, bytes and
declaration-text fingerprints, along with session and map ownership; this is
an IO prediction, not a CPU-time or process-RAM claim.


Listed sound-event overlap prediction: the installed common list contains
1,296 sound-event entries and the EotW / Dust map lists contain 382 / 388.
`ContactSounds` reads their declaration text before `prepare_listed_events`
reads and resolves the same listed events again. A single shared text snapshot
should satisfy at least 5,040 direct listed-event source requests across
EotW -> Dust -> EotW (1,678 / 1,684 / 1,678); inheritance may add cache hits
or genuine fallback reads. Keep the existing declaration parser and typed
outputs, support the independent fallback initializer, and report actual hit,
fallback and source-byte counts plus resolved-event fingerprints. A common
owner must expose the snapshot to both users without retaining duplicate text.


The candidate sequential parser now shares its implementation between slice
readers and the buffered archive stream. A production diagnostic read all 36
installed networks through each implementation; all 36 full parsed-record
fingerprints match and every acquisition succeeded. The affected input/parse
thread CPU summed 8,125.0 ms for the existing path and 8,218.75 ms for the
streamed path; that aggregate alone is not acceptance because the single-sample
Dunwall Tower result differed by more than 30%. Repeated targeted measurements
are underway to distinguish that discrepancy from the 15.625 ms timer
quantization and run-to-run variation. Evidence: local
`artifacts/probe-stream-world-comparison.json` and the per-mode world records.


Manifest cache identity correction: `Catalog::open` can create another catalog
for the same installation with a different enabled-overlay snapshot. The old
init/common and weak combined-manifest cache keys included only installation
path, map and language, so they could return descriptors from a different
catalog. Key both caches by catalog identity and release the cache owner's
references for other catalogs; live callers retain their own Arc. A loose
init/common manifest is read on each new assembly because the overlay file
can change in place. The ordinary one-catalog route should keep the same
manifest identities, reads, descriptors and loading gates; this is a
correctness/lifetime fix, not a predicted gameplay CPU improvement.

Repeated affected-thread measurements (six acquisitions per mode) investigated
the tower outlier: baseline median 390.625 ms, streaming 367.1875 ms. The
single 218.75 ms baseline sample was not representative of those repeats.
Interior medians were 132.8125 -> 117.1875 ms, EotW 406.25 -> 421.875 ms,
and Dust 468.75 -> 476.5625 ms. The Windows thread CPU counter advanced in
15.625 ms steps, so the small EotW/Dust differences do not establish an
exact percentage change. These timings cover only network input and parsing,
not whole-frame or GPU time. Further repeats for short networks address other
single-sample deviations affected by the same quantization. Evidence:
`artifacts/probe-stream-targeted-comparison.json`.


Catalog identity sweep prediction: the animation, Iggy file, Iggy bitmap and
render-parameter registries keyed cached installed content only by base path;
the Flash string-table registry used language alone. Independently opened catalogs can have different overlay snapshots even
for that same path. Scope these registries to the catalog identity, discarding
the registry's old-catalog references while allowing live Arc/Rc users to keep
their own data. For the normal one-catalog multi-level route, requested
identities, IO counts and output fingerprints should remain unchanged; this
fixes stale cross-catalog reuse and unneeded old-catalog pinning rather than
predicting a gameplay-frame speedup. Shader-cache indices opened directly
from `base/shaderCache` use their own file-backed key and already prune other
installations; they are not catalog overlay entries.


### Streamed probe and shared declaration results

All 36 installed probe networks were parsed by both the buffered stream and
the prior whole-input path; all parsed-record fingerprints match. Repeated
affected-thread CPU measurements on selected networks were timer-quantized at
15.625 ms; no whole-frame performance claim follows. The EotW / Dust / EotW
production clients released 3,312 / 2,825 / 3,312 probe-history names with
their last owners. These counts do not represent physical RSS.

The same production route read contact declarations 28,180 -> 10,162 times
and 4,428,182 -> 1,611,850 decoded bytes. The sound resolver satisfied 5,040
requests from that shared snapshot instead of rereading them; both contact
and sound fingerprints matched. Common texts remain session-owned and map
texts release at transition. Mutable overlays preserve separate reads.
Evidence: local `artifacts/contact-shared-comparison.json` and
`artifacts/probe-stream-world-comparison.json`.

Map-info fallback now drops the unused resolved-result cache when the
registry switches permanently to reference scans. Existing Arc holders keep
their own data. The normal indexed route is unchanged; this error-path
cleanup follows the ownership review and was not exercised by the successful
four-level clients.


### Stream completion and mutable manifest correction

The former whole-member archive reader reads one byte past the indexed
decoded size, which both detects oversized output and forces compressed
trailer validation. The bounded probe stream must perform the same final
read after consuming the indexed length. Prediction: one end-of-stream read
per network, with unchanged parsed records for valid installed resources;
no extra whole-member allocation.

A combined load-list lease can outlive a mutable loose manifest. A new
acquisition must bypass the combined cache if any of the init, common or
map manifests is overlaid, while existing holders keep their own list.
Map-info scans likewise must not reuse a resolved result or loading-info
result when its declaration can change. Prediction: unmodified installed
catalogs keep their existing read/reuse counts; affected overlay acquisitions
observe current file contents rather than a previously cached descriptor.


Catalog-scope transition comparison: the EotW / Dust / EotW production
client exited successfully after catalog identity was added to animation,
Iggy, library bitmap, localization and shader-parameter cache owners.
Contact, sound and 600 animation request fingerprints matched the earlier
client; shader parameter counts and all 430 observed UI bitmap fingerprints
also matched.
Iggy acquisition/reuse counts stayed 13 / 14. On an installation with no
overlays, the common declaration owner skipped exactly two bootstrap and
9,009 declaration overlay checks at initial acquisition. These are
acquisition and content measurements, not frame-time or RSS measurements.
Evidence: local `artifacts/catalog-sweep-comparison.json`.


Probe input CPU follow-up: the six short-network measurements used six
affected-thread runs per mode with 15.625 ms timer steps. The Tutorial
medians were 93.75 -> 62.5 ms, a two-tick, greater-than-30% change, while
the sums were 515.625 -> 437.5 ms (15.2% lower); individual baseline
samples ranged from 46.875 to 109.375 ms. This noisy result does not
establish the size of a CPU gain. The six-run sums of the other five
short networks ranged from 27.3% lower to 11.1% higher. Larger EotW and
Dust median changes were approximately 4% and 2%, respectively, also close
to this timer's resolution. All measurements concern only the input/parse
thread, not gameplay frames. Evidence: local
`artifacts/probe-stream-small-comparison.json` and
`artifacts/probe-stream-targeted-comparison.json`.


Material source handoff production comparison: EotW / Dust / EotW used
1,308 / 1,327 / 1,308 handoffs, avoiding 3,943 declaration reads and
3,464,990 cumulative decoded source bytes. The largest temporary handoff
capacity was 3,073 bytes. Source, shadow-admission and colour-resolution
input fingerprints match the separate-read control. One temporary source
is held at a time, and mutable overlays use the former separate reads.
This is neither an RSS nor a frame-time measurement. Evidence: local
`artifacts/material-handoff-comparison.json`; both production clients exited
successfully.


### Archive, player rig and loading-history comparison

The two production clients completed EotW / Dust / EotW without errors.
On the no-overlay installation, the archive fast path skipped path
normalization for at least 68,608 underlying reader acquisitions at the
last recorded checkpoint. Other overlay lookups add to that count. Nonempty
overlay catalogs use their existing lookup path.

Rig acquisition fell from 15 to 9 across the route, as predicted. Counted
skeleton bytes plus decoded MD6 and animset text were 6,736,494 -> 3,984,234
bytes (2,752,260 cumulative bytes avoided); binding lookups are outside
that byte total. Unique rig source fingerprints matched. Only aliases and
rest poses survive arm construction for scene preparation; they release
afterward. Overlay catalogs retain separate reads and their former order.

At the last pre-play checkpoint, the read-history control performed 68,608
path-normalization operations and allocated 68,608 kind keys; the borrowed
path implementation performed 282 normalizations and allocated 138 kind
keys across 68,572 reads. The two counts include 36 fewer source reads from
the rig change. Both recorded 31,735 insertions cumulatively, and
the three progress snapshots matched (34/40,335, 17,496/40,510 and
19,666/40,335 precredited/requested). The late-read log recorded zero in each client; streamed mips are excluded
from that log. The control's `canonical`
metric is zero because it deliberately bypasses the canonical-path scan;
it does not describe its inputs.

`QueryThreadCycleTime` around only `loading::read` recorded 7,714 -> 5,541
cycles per read, approximately 28.2% lower, in one instrumented run per
mode. Both modes include the same counter overhead; thread cycles are not
elapsed time and this does not establish a whole-frame improvement or a
result on every machine. Contact, sound, 600 animation, material input, 430
UI bitmap and probe-network fingerprints matched. These are acquisition
counts, logical input bytes and targeted cycles, not a process-RSS result.
Evidence: local `artifacts/archive-overlay-comparison.json` and paired logs.


### Physics-material sharing prediction

Installed world-physics acquisition recorded 134 reads of 45 distinct
physics-material paths in EotW (89 repeated reads, 3,261 repeated decoded
bytes), 115 of 41 in Dust (74, 2,548 bytes), and 114 of 40 in Institute
(74, 2,387 bytes). The resolved-output fingerprints were respectively
`d5e0bead8c732708`, `672e9c7850dc6d6b`, and `bc32baabad7b780c`; no
resolution errors occurred. EotW / Dust / Institute have 42 / 39 / 38
world paths also present in the contact list, all in the 306 common
physics-material entries. The sampled files did not change size between reads.
Evidence: local `artifacts/physics-material-overlap.json` and
`artifacts/physics-material-membership.json`.

Before implementation: one level-scoped source owner should serve model
collision, world physics, machinery and physics-world startup, then transfer
its already acquired listed texts to the existing contact declaration owner.
The existing session common table should serve later levels without a second
read. For EotW / Dust / EotW, world physics should fall from 383 reads to
at most 50 and the initial contact acquisition should avoid at least 42 more,
a combined minimum of 375 reads. The world-only source bytes are predicted
16,858 -> at most 2,963 and initial contact should avoid at least 2,446
additional bytes. Model-collision or machinery overlap can add savings; it
must not be counted twice. Retain only the unique texts needed for that handoff
and move them into the common table rather than keeping two copies. Unlisted
texts drop after acquisition, live overlays keep separate reads, and no
gameplay-frame or RSS improvement is assumed. Compare actual per-consumer
reads/bytes, resolved collision properties, contact and sound fingerprints,
owner release and loading progress in real clients; investigate a prediction
difference over 30%.

### Physics-material sharing result

The separate/shared EotW / Dust / EotW production clients both exited 0.
World reads fell 383 -> 46 and decoded source bytes 16,858 -> 2,666,
within the prediction. Including model collision, machinery and startup,
reads fell 749 -> 62 and decoded bytes 40,060 -> 5,111; initial contact
acquisition avoided 45 additional reads and 2,686 decoded bytes. Machinery
made 306 source requests and needed no extra reads in shared mode because
the earlier consumers or session owner had those texts. The maximum
temporary handoff string capacity was 6,748 bytes; after transfer, the
unlisted strings drop with the handoff. Existing common and map tables have
the same measured capacities in both modes. World and model material values,
contact and sound signatures, 600 animation and 430 UI bitmap signatures,
probe fingerprints, and loading progress snapshots matched. These counts
are not process RSS or gameplay-frame timings; a targeted acquisition CPU
comparison is still required. Evidence: local
`artifacts/physics-sharing-comparison.json` and paired client logs.

### Area-environment preparation result

EotW / Dust / EotW separate and shared production clients exited 0. Resolved
volume acquisitions fell from 1,000 to 500, inherited source reads from
2,636 to 1,318, and cumulative decoded source bytes from 19,793,854 to
9,896,927. That matches the predicted 500 removed resolutions. All ordered
area geometry/profile signatures and all individual resolved-volume
signatures matched, as did the recorded collision, contact, sound, animation,
UI, material and probe signatures. Both consumers retain their previous
independent runtime data; these cumulative input bytes are not an RSS saving.
Live overlays retain the old separate acquisition behavior. Evidence: local
`artifacts/environment-sharing-comparison.json`. Targeted release CPU
measurement remains pending.

For the physics-source change, the development comparison also instrumented
only the changed reader: 749 calls used 23,570,179 -> 5,843,521 thread
cycles. This is not an elapsed-time or release-performance measurement; the
release comparison is still required.

### Immutable trigger source retirement

The retained/released EotW / Dust / EotW clients exited 0 with identical
trigger geometry and root-mass fingerprints and matching area, collision,
contact and recorded rendering-resource fingerprints. The physics scene
retired 332 / 259 / 332 immutable source files after initialization. Their
source-buffer capacities were 1,516,972 / 1,658,543 / 1,516,972 bytes and
the parsed schema capacities were 5,573,212 / 4,594,951 / 5,573,212 bytes.
The typed query geometry, shape headers, bounds and mass results remain;
mutable shapes retain their source. Cached mass fields add per-shape storage,
and allocator overhead and process RSS are not measured by these counters.
Release-profile startup CPU measurement is pending. Local evidence:
`artifacts/trigger-sharing-comparison.json`.

### Environment-volume ownership and trigger overlap

The EotW / Dust / Dust return / EotW retained-versus-compact production
clients both exited 0. All four area, volume, trigger, physics, contact,
sound, material and probe signatures matched, as did 800 animation and 430 UI
bitmap signatures and loading progress snapshots. Dust return included one
water volume; its compact record's identity, position, classification and
surface material were included in the matching fingerprint. This does not
claim a moving camera entered that volume.

The source profile/placement allocations and vector-capacity differences
account for 11,403,356 / 5,441,286 / 936,718 / 11,403,356 bytes removed
from these live owners; the retained area-parameter capacities matched in
both modes. These are counted allocation capacities, not process RSS.
Physics and water retain compact records in authored order; ordinary area
profiles keep their parameters, and unused water profile parameters drop.
Local evidence: `artifacts/compact-volume-comparison.json`. The subsequent
release comparison measured the affected environment-preparation and trigger
acquisition/retirement spans; details are in `docs/resource-lifetimes.md`.

Successful source records additionally show 400 / 184 / 31 / 400 redundant
trigger-shape reads across those loads, with 5,983,276 cumulative decoded
bytes, assuming one acquisition per identical path per level. The EotW
area/wind consumers requested 400 files representing 204 identities; physics
requested all 204 identities again. Matching paths carried matching source
CRCs. These are I/O counts, not resident savings. The implemented temporary owner shares matching source/schema data while
retaining the separate derived geometries; four-map source, geometry and
physics fingerprints matched. It avoided 1,013 reads; two wind-only inputs
were not area-cache candidates. Evidence: `artifacts/trigger-overlap-comparison.json`
and `artifacts/shape-parent-comparison.json`.


### Localization value ownership

`1401653e0` and `140165420` resolve numeric IDs and text keys to the same
original language-dictionary entry. `assets::lang::StringTable` retains one
immutable value shared by both lookup indexes; the session UI worker owns the
table and catalog changes release its cache lease. The parser moves each
original key into its temporary collision map and reuses the computed ID,
while retaining the previous collision message. This is a host ownership
optimization; language contents, key case rules and fallbacks remain unchanged.

On the installed Russian table, the instrumented client recorded 6,226 entries
and 2,530,346 bytes of prior value-string capacities. The shared values contain
1,026,738 UTF-8 bytes; their current-toolchain Arc layouts total 1,148,208 bytes
including headers/alignment, before allocator bookkeeping. Hash-map value
slots also shrink, but their physical allocations are not included in this
value-storage comparison. The original EotW fixture reports LANGDICT 547,888
bytes without a language identifier and is not a same-language comparison.

A same-binary diagnostic over installed keys found no lookup regression in
three runs; numeric-ID timing varied enough that a fixed speedup is not
claimed. Two key-preparation comparisons removed 6,226 temporary key clones
and measured 3,726,310 -> 2,902,970 and 5,781,860 -> 3,577,770 thread
cycles. Both measurements cover changed code only, not frame time. Four-map
prior and three-map later clients recorded the same 430 UI bitmap fingerprints;
this does not independently verify displayed translated text. The diagnostic
code was removed from normal source after measurement. Evidence: local
`artifacts/language-value-inventory.json`, `language-key-perf-shared.err` and
`language-repeat-shared.err`.


### Physics-shape scope evidence

The saved original bootstrap registry snapshot contains 128 `physhape`
and 118 `massprops` resources before a map is loaded; the saved Edge of
the World snapshot contains 1,723 and 1,309 respectively. These are
resource counts across the registry, not named scope lists, memory bytes, or
proof that every bootstrap item survives a map transition. The installed
common manifest separately names 116 physhape and 108 massprops entries.
Across the 36 installed map manifests, common-list names of those kinds do
not recur in the map lists. `14006a910` pushes the map heap for named maps,
while init/common inherit their caller's scope. `1400661e0`'s conditional
promotion table `142280ee0` includes massprops but not physhape. Map
teardown and shared/common retirement therefore need separate owners and
cannot be implemented by clearing all recorded names of either kind.
Original snapshots: local `artifacts/resource-residency-initial.json` and
`artifacts/resource-residency-eotw-verified.json` in the main checkout;
installed-manifest count inventory: local
`artifacts/shape-manifest-scopes.json`.


The conditional promotion list at `142280ee0` contains 36 type strings:
`anim`, `animBasic`, `animSet`, `animTree`, `curvefloat`, `damage`,
`decal`, `entityDef`, `env`, `EnvProbe`, `explosion`, `fade`, `File`,
`fx`, `fxanimset`, `gamesoundproperties`, `image`, `kiscule`, `loco`,
`massprops`, `material`, `midnightScene`, `musicManager`, `Navmesh`,
`particledt`, `pblock`, `physicsMaterial`, `skeleton`, `soundauxbus`,
`soundevent`, `soundrtpc`, `speechBarks`, `speechCharacter`,
`speechScene`, `speechVoice`, and `staticParticleModel`. The list is
consulted only when the corresponding original loading condition is enabled
and the request is neither init nor common. It is not a statement that every
resource of those types is always persistent.


### Immutable world and reflection buffer ownership

Host: `app::world_instances::WorldMatrices` owns one immutable GPU matrix
buffer per level, shared by world shadow and water consumers; CPU world-query
matrices remain in the scene. Original metadata `14150e060` names the
world-matrix buffer view and `14155b100` identifies a 48-byte
`arkMat3x4`. The host encodes the existing rows directly from a borrowed
slice and transfers the buffer bytes to Bevy's render world; it releases the
serialized CPU upload copy after extraction. EotW / Dust / Dust return /
EotW encoded 4,507,776 / 6,887,376 / 2,301,360 / 4,507,776 bytes with
matching CRCs and matching GPU upload counts in paired production clients.
Four targeted release serialization spans totaled 34,816,810 -> 11,531,730
thread cycles; this is loading work, not gameplay frame time or RSS.


Host: immutable reflection-plane, cube-index and water-probe buffers follow
the same transfer, while the separately owned CPU probe data remains available.
Their EotW lengths were 6,672 / 416 / 26,496 bytes; paired four-level
production fingerprints and upload counts matched. This host ownership rule
does not claim an original per-buffer allocation match.


The per-instance lighting buffer retains its full CPU backing because its
moving-object updates need exact previous values. A sparse alternative was
rejected after its affected update function measured slower during walking.
Its initial, overwritten staging buffer was removed independently; the final
bytes and upload counts matched across the same levels. The renderer and
main world clear level-owned pending buffer patches at the existing render
teardown acknowledgement so stale outgoing updates cannot survive into the
next level; normal per-frame patch behavior is unchanged. Both queues were
empty at the three observed release points, so the cleanup's handling of a
nonempty pending queue is source-reviewed rather than claimed as exercised.

A rewrite history snapshot contained one common-list physhape: a coin
collider read by the static-model path three times across the level sequence.
The rewrite currently has no reusable session owner for that decoded shape, so
its last level lease retires the read-history identity even when the name is
listed as common. The next request already rereads it; clearing the stale
history does not introduce that read. This is a host bookkeeping boundary, not
a claim that the original discards its common shape.

### Model, particle and camera-metadata scopes

The host keeps static model read-history credits through preparation, render
release and the end of level teardown. Particle declaration and PMODEL credits
follow independent shared leases on the actual immutable objects, including
render-world simulations and queued draws. These are bookkeeping scopes; they
do not retain source payloads or skip an archive read. Concurrent preparations
keep their own leases. The original lookup/purge boundary is
`1400607f0` / `140060180`; the original snapshots count 163 / 965 models,
339 / 605 particle declarations and 150 / 327 particle models at bootstrap /
EotW respectively, without identifying their per-name scope or bytes.

In the six-load same-client comparison (Dust four times, EotW, Dust return),
body, collision, environment, contact, sound, material, probe, animation and
UI fingerprints matched. On each Dust transition, the host released 429 model
names (38,362 String-capacity bytes), 103 particle declarations (10,586 bytes)
and 12 particle models (999 bytes), after their last scopes. The subsequent
load re-records those identities: this is measurable loading CPU work, not a
claimed frame-time gain. The common-history ownership comparison below includes
that cost. String capacities are not process RSS.

The shared skeletal-input implementation was also compared across Edge of the
World, Dust District, its return variant, and a return to Edge of the World.
Per-instance skin fingerprints matched the separate-input implementation in
all four loads. Unique rig payloads changed from 6,272/4,704/1,568/6,272 to
1,568/3,136/1,568/1,568 bytes, and model payloads from
52,032/33,744/13,008/52,032 to 13,008/20,736/13,008/13,008 bytes. These
are payload counts, not process-memory measurements; singleton inputs also
incur sharing overhead. History leases were released at each transition.
The generalized pose-publication helper still needs an affected-path timing
comparison alongside the animated-object implementation.

Camera blur metadata is an immutable catalog-scoped session resource. The
original registry returns an acquired declaration identity at `1400607f0`;
this host cache shares the six parsed kernels and 63 material variants while
keeping view state and render passes per level. Editable overlays retain fresh
reads. Six production loads made six -> one constructions; their 63 material
and two program reads per construction imply 390 -> 65 source reads. All six
kernel/variant signatures were `f2bb4981`. Counted retained
metadata is at least 6,536 bytes, excluding allocator bookkeeping and the Arc
header. The targeted acquisition spans totaled 21,300,056 -> 4,434,045 thread
cycles, not whole-frame timing. Local evidence: `artifacts/image-particle-cost-*`.

Indexed image resource identities now use the same ASCII case and slash
normalization as the catalog (`140140da0` / `140140de0`) before shared cache
acquisition, including loose overlay lookup. Named intrinsics keep their own
identity. The sampled routes already used canonical indexed image names, so
this fixes equivalent-name aliasing without a measured duplicate-image saving;
all 568 compared reflection-image fingerprints and 430 UI bitmap fingerprints
matched. No visual inspection was performed.

Particle-model source identity audit: the selected installed indices contain
1,390 `particledt` declarations. Reading their payloads produced 3,140 LWO
references, all lowercase with forward slashes; none had multiple spellings
for the same normalized name and none failed to read. This is an installed-data
inventory, not coverage of arbitrary mods or a claim that every reference is
activated in a level. No extra PMODEL alias cache or normalization work was
added without an observed duplicate. Local analysis:
`artifacts/audit-particle-source-aliases.py`.

The selected installed index also contains 6,191 material declarations, all
readable in the inventory. A text scan found 18,537 TGA/DDS/PNG path references;
436 use uppercase or backslashes and seven normalized names have multiple
spellings. `Catalog::image_resource` normalizes these authored source names
before selecting an indexed compiled image, so this route already converges on
one indexed identity. The scan is not a parser for every image program and does
not prove each reference is used at runtime. No image-count saving is assigned
to the new direct-resource normalization from this inventory. Local analysis:
`artifacts/audit-material-source-image-aliases.py`.

### Common declarations and level material history

Host loading history now borrows the already retained common declaration text
owner rather than allocating a second copy of each name. On EotW the one-time
handoff removed 9,009 duplicate names and 656,573 bytes of String capacity;
effective history-table slots fell from 16,576 to 119. The acquired count stayed
17,900 across the handoff. Across EotW / Dust / Dust return / EotW, FX,
sound-event and physics-material coverage and precredits matched the control;
model-collision, probe, contact and material fingerprints matched as well.
The actual common owner is catalog-scoped, and a changed catalog or disabled
shared table removes its borrowed credits. Recorded catalog kinds handle
nonstandard path names; common-manifest membership alone does not grant credit.
This is host bookkeeping around the original `1400607f0` / `140060180`
registry boundary, not a count of original memory or process RSS.

Full declaration cleanup used 407,365 / 425,915 / 252,385 thread cycles on
those transitions with shared common names, versus 5,701,325 / 5,474,945 /
5,713,155 in the otherwise equivalent no-sharing control. The one-time common
handoff cost 7,369,320 cycles in this run. Manifest-precredit spans varied
between runs and are recorded in `artifacts/common-material-cost-*.err`; no
whole-load or frame-time speedup is inferred.

Across the four-map comparison, the affected read-history and transition
spans totaled 81.77 million cycles in the legacy mode and 83.91 million in
the shared mode, approximately 2.6% higher in this run. Re-recording retired
map names contributes work, while common-name cleanup costs less. Unrelated
startup spans also varied, so these numbers do not establish a whole-load
regression or improvement.

Material history has a separate level scope. It releases only after renderer
and scene teardown, preserving the 63 names represented by the live session
camera-blur metadata owner. EotW / Dust / Dust return released 1,444 / 1,475 /
685 names and 152,046 / 155,186 / 71,836 bytes of String capacity; retained
camera names account for 3,951 String-capacity bytes in the recorded history.
Returning levels re-record their material names, so this reduces stale history
at a measured loading-CPU cost. The associated material and scene fingerprints
matched. A moving-player run made zero calls to the affected `loading::read`
path during its measured interval; it therefore establishes no per-call
performance result for streaming reads. The original purge is `140060180`;
this explicit owner and its camera cache are host mechanisms.

### Player declaration setup ownership

The host shares a successful immutable player declaration preparation among
arms, player setup and underwater effects during one level setup. After the
first two consumers, only FX and controller fields remain until screen-effect
preparation completes; no declaration tree is kept for gameplay. Original
declaration identity lookup is `1400607f0`; this temporary borrowed owner is a
host optimization. Editable overlays and read failures use fresh acquisitions.
In both Edge of the World and Dust District, recorded consumer fingerprints
matched for Corvo and Emily. Each level resolved the tree three times before
and once afterward; Corvo's source reads fell 357 to 119 and Emily's 354 to
118 per level. Targeted cycle and capacity figures, including their limits,
are in `optimization-opportunities.md` under player declaration preparation.

The optional body AudioListener now resolves in that same temporary entity
pass, with its typed settings or deferred error retained until player setup.
This removes the separate setup traversal of the entity inheritance chain and
listener declarations. Installed Corvo and Emily each inherit `base/body.def`
and the base slot points to `audio_listener.cpntaudiolistener`, which inherits
`audiolistener.cpntaudiolistener`. For either character, the predicted normal
setup change is six reads to four across the shared bundle and listener lookup:
the second reads of the character and base body entity definitions disappear,
while the two listener declarations are still each read once. This is a
prediction from the installed declarations, not a measurement. For a matched
same-binary setup measurement, `DH2_PLAYER_DECL_READ_TRACE=1` prints those paths;
adding `DH2_PLAYER_DECL_READ_BASELINE=1` uses the previous separate listener
lookup. Count the `PLAYER_DECL_READ` lines per character and compare the
resulting player/arm/screen behavior before reporting an observed reduction.
The bundle still drops everything except screen inputs before screen setup and
does not retain these fields for gameplay. The preparation resolver keys its
temporary component cache by normalized declaration kind and name, consistent
with `1400607f0` looking up a name in a declaration-type manager. Player entity,
component and player MD6 inheritance now detect repeated paths instead of
rejecting an arbitrary 16-declaration chain; this is host cycle handling, not a
claim that all native declaration lifecycle or diagnostics have been ported.
Mutable overlays retain their existing fresh-acquisition behavior. Cache hits
still clone fields for callers that apply their own overlays.
The private setup owner also drops default-mode state trees that the existing
player state consumer cannot use, only after resolving every authored entry.
The public `player_declarations` result remains complete, the setup classifier
is the same one used by the state consumer, and source reads, parsing and errors
remain unchanged. The installed Corvo and Emily modes predict 80 -> 14 retained
records; no read or parsing reduction is claimed. The temporary resolver cache
has already been released before this compaction. A same-binary setup run with
`DH2_PLAYER_STATE_RETAIN_TRACE=1` reports record counts and owned capacities
before/after; adding `DH2_PLAYER_STATE_RETAIN_BASELINE=1` retains the full list.
Those capacities include the surviving vector backing, not allocator overhead,
process RSS or peak memory; actual measurements are pending.
For comparison, the original state constructor `140853ce0` stores its supplied
declaration pointer at state +0x18, and `1408644e0` constructs and initializes
states from the mode's declaration list. Mode teardown `1408645c0` calls each
state's Deinitialize and destructor, while `140803f50` retires its modes. Those
observations describe the native owners; the temporary setup bundle here does
not claim to implement that full managed state lifecycle.

### Editable declaration, animation and UI resources

The host catalog's editable layers can change their file contents without a
new catalog identity. Shader-parameter metadata bypasses session reuse when a
relevant declaration is editable. Editable animation clips are acquired anew;
weak generation references keep their history accurate while older actors own
their clips. Editable bootstrap manifests do not pin clips according to a
stale snapshot, while weak per-clip sharing remains available. Iggy files and
their library bitmaps bypass reuse when either relevant file is editable;
open movies retain their existing leases. The original registry lookup/purge
boundary is `1400607f0` / `140060180`; editable-layer handling is host behavior.

An installed no-overlay Edge of the World / Dust District production comparison
matched 430 UI bitmap and two collision-material fingerprints. Acquisition
measurements and their limitations are recorded in `optimization-opportunities.md`.
Editable branches and overlapping generations were source-reviewed; that
production comparison had no active editable layers.

Editable image base files and separately indexed mip files now participate in
the same re-acquisition decision. Scene images keep their authored common/map
upload classification, while editable views are not carried forward as shared
session images. Resident 2D views, water cube/array sources and UI bindings
likewise avoid reusing an older editable view. Existing GPU handles remain
valid; read-history bookkeeping tracks editable 2D generations by asset ID
and weak handle and layered generations by weak source reference. The host
retains an image history entry while the relevant handle or Bevy asset remains
live. The original registry and view-publication boundaries are `14005feb0`,
`140060180` and `1402f5530`.

A same-executable no-overlay Edge of the World / Dust / Edge of the World
comparison matched all 11,849 recorded image-packing descriptors and checksums,
430 UI bitmap fingerprints and three collision-material fingerprints. The
1,065 resident-image acquisition spans had a 4,200-cycle median in each mode.
Longer preparation timings include diagnostic checksum work and do not show a
clean CPU result. Editable-image branches and their overlap handling were
source-reviewed; no active editable layer was present in that comparison.

### Combined manifest lifetime

The combined level request list now leases the existing init and common lists
and the active map list as three immutable sections in their authored order.
The map section releases with its final consumer; init and common remain in
their existing session caches. Editable manifests still bypass the combined
cache. This is a host storage optimization around the original request-table
acquisition/release (`1400661e0`), not a change to which resources are loaded.

Edge of the World / Dust / Edge of the World comparisons matched ordered-list
fingerprints, loading coverage, contact and collision-material signatures in
flat and segmented modes. The old combined copy duplicated 26,193 init/common
entries totaling 3,492,748 bytes of descriptor and String capacity. Source
vectors have different spare capacity; the derived Edge of the World saving
after that difference is approximately 3,177,148 capacity bytes during loading,
excluding allocator and Arc overhead. Targeted read-history spans were nearly
identical in the recorded run. Details and timing limits are in
`optimization-opportunities.md`.

### Particle parameter-block publication under integration

`assets::parm_block` now exposes the last authored numeric assignment without
copying its declaration. `bevy_backend::particle_system` initializes a
per-instance bend-time factor with the original zero fallback and retains the
shared declaration through particle tails. `particle_pass` transports the
factor in existing instance padding; the mask shader specialization performs
opacity clipping before alpha*factor output. Status: unbuilt and inactive;
the target, pass pipeline selection, dynamic owner overrides, world-material
mesh handling and grading activation are still required before FX admission
can be enabled. Original sources: 140227cd0, 140228110, 140226930, 14033b350,
1403222f0 and the installed particle/add_default and postfx/colorgrading
programs. This is no claim of verified rendering or memory/CPU improvement.

Mask draw submission is now implemented as an explicit `ParticlePass` mode,
sharing existing buffers, image leases and samplers. Its cache keys distinguish
colour from mask variants; mask specialization writes R8_UNORM, disables only
depth writes in the authored material state, and uses stencil reference zero.
Bounded mask-material prewarming is available. Distinct authored masking
programs and distortion masking still return explicit preparation errors.
Status remains inactive after successful release compilation: no view owner invokes the mode until target,
consumer, dynamic overrides and world-material mesh handling are connected.

The mask consumer now owns its lazily prepared per-view target as well:
R8_UNORM, one mip/sample, full dimensions clamped to eight per axis, as in
14036d0c0. Its explicit clear preserves scene depth and clears even an empty
surface list (1403222f0). Resize and view release drop cached sampled input
bindings together with the target; compiled pipeline/uniform capacity may
remain with the host consumer. COPY_SRC is a host diagnostic permission only;
normal rendering adds no readback. Status: compiled, but not activated by a view.

Further grading trace identifies the actual consumer as 1403134e0, called by
140315450 before its DoF operations. It reads greyAmount and invert from the
view parameter block, selects the grey/invertedGrey material, and swaps HDR
source/destination after drawing. It bypasses grading when amount is not
positive or r_BlackAndWhiteMode is active (registration 1413754e0); this is
not the similarly named r_skipColorGrading CVar. The host consumer now enforces
that predicate and releases view bindings/target on deactivation. View
parameter publication and render-stage activation remain unconnected.

The live producer is now identified: `14081e9d0` constructs
`arkPlayerFXBendtime`, and its update `14081ef10` publishes `greyAmount` through
the active view's parameter-block interface. During an active state it uses
one minus the time-scale query (`14080d060`); after leaving that state it
fades the saved amount using the authored duration at declaration offset
`0x178`. State selection is `14081ec80`, with matching FX start/stop in
`14081eeb0` / `14081ee00`. This is not an ordinary environment-fog setting:
all 121 unique installed `env` declaration texts were scanned and none names
`greyAmount` or `invert`. The executable references, rather than that negative
inventory alone, establish the producer. The rewrite does not yet implement
that player-power owner, so merely adding environment defaults would not
activate the missing effect correctly. Its future view publication must use
this owner and retain the existing per-view target release rule; no perpetual
mask pass or speculative map-specific activation has been added.

### BMD6 owner comparison: remove a disproven cache

The integrated debug client at `f6c186c` completed Edge of the World -> Dust
District -> Dust Return -> Edge of the World in both separate and shared
modes. Both made 210 model requests, including seven local preparation-cache
hits, and 203 read/decode calls. All 76 requested paths had identical source
CRC/byte counts, decoded CRC and logical byte signatures across the modes.
Both clients exited successfully; all four level-ready gates and outgoing
model history release scopes ran. The shared registry was empty after each
of its three observed map transitions. This proves those exercised output
and release paths, not a CPU or peak-memory saving.

The expected cross-owner decode reduction did not occur because setup already
borrows physics models and runtime usable assets have their own real lifetime.
The redundant global BMD6 registry, wait handoff, retirement tokens and switch
were therefore removed instead of retaining an optimization without benefit.
The preparation-scoped common loader and explicit live-consumer sharing stay.
First-person rendering now requests through that loader too. Optional identity,
content and final-release diagnostics remain for future measured overlaps;
normal rendering performs no diagnostic checksum scan. Original class-wide
streaming/admission and all unimplemented entity consumers are not certified
by this four-map comparison.

The same consumer review removed the new aggregate Rig registry before
compilation. The integrated callers acquire distinct player body/left/right
model rigs; usable instances already share their `BasicRig` through the live
usable asset owner. Baseline four-map diagnostics show four acquisitions of
each player rig, one per level, and map-scoped usable rig acquisitions. No
additional concurrent model-rig consumer was established. `animation_rig`
therefore keeps one typed loader and optional acquisition/release diagnostics,
without another weak map, wait queue or cache-purge hook. The Edge skeleton
identity owner is distinct: that actual installed resource is requested by
player, arm rendering, physics and rest-skin consumers, so its sharing and
full-payload lifetime tradeoff remain the next measured comparison.

### Particle simulation versus ocean readback scheduling

The shared gameplay FX-owner migration must preserve this original ordering:
`1403619d0` dispatches render-world vtable `+0xe0` (`14023ad00` -> `1402031e0`
-> `140202740`) to submit `arkParticleSystemTask` work; `+0xe8`
(`14023aea0` -> `1402037c0`) waits for that task group and `+0xf0` finalizes it.
Only afterward does `140361bc0` -> `140361d60` -> `140304c40` map the ocean
staging buffer and copy results into the CPU query ring. Swimming consumes
its prior handle via `14088b360` -> `140302840`; registration `1403032b0`
selects the four-slot ring with `(render counter + 1) & 3`. These are separate
query/result and per-frame simulation lifetimes, not a reason for a second
GPU readback or a mutex spanning simulation and rendering.

The integrated host owner is now `camera_scene::FxSimulation`, in the gameplay
world. It consumes one atomic snapshot of the prior ocean query results and
publishes an immutable `Arc<FxFrame>` for rendering. The existing render readback
publishes the next result without releasing the swimming query handle early;
the ordinary ring consumer still releases it. Level retirement replaces the
mailbox identity, so outgoing render work cannot publish into a new level.
This is source integration, not yet production verification. No GPU wait or
second particle simulator was added to the gameplay thread.

Prediction before measurement: normal extraction removes one deep
`WorldEffectFrames` pose-map copy per frame. It adds one immutable frame
publication allocation/reference handoff and at most 80 bytes of ocean query
data copying. GPU pass/readback counts should remain unchanged. Same-build
`DH2_DEBUG_EFFECT_FRAME_EXTRACTION=1` restores the former pose-map extraction;
`DH2_DEBUG_FX_LIFECYCLE=1` records changed-block cycles, copied payload and
publication fingerprints. Source integration alone does not verify output or
establish CPU/RAM savings.

The first combined compilation caught an oversized Bevy system parameter list
and a double-borrowed decal iterator. Integration now groups the publication
with the existing particle input bundle. Material prewarm also accepts an
empty draw list when its admission is what the first effect is waiting for;
ordinary rendering still requires a matching camera/frame publication.
These are integration corrections, pending the next complete build.

### First-use sound preparation scheduling

Host owner: `app::audio::acquisition::SourceJobs`. Original `1400bc8f0` /
`1410bf740` configure a dedicated sound streaming device. Encoded block reads
remain in `assets::wwise::stream_io`, with the same 2 MiB budget and 16 KiB
granularity. Previously event/source preparation used the generic IO pool,
also used by the texture worker's drain loop. Source preparation now has one
session-owned worker, weak back-links from source locators/snapshots,
cancelled task leases, and shutdown/join through the same Bevy task-pool owner
already used by loading-time sound preparation. The worker starts during
session setup so source activation cannot create a thread on the audio callback. Level resets keep
the session worker and release outgoing requests; no full streamed media is
preloaded to conceal cold-start delay.

The reported fly event `0xc64fd6cf` has no action delay and selects two type-2
Vorbis streams (`0x1003ce06`, `0x12165710`). Installed HIRC and package data
therefore do not justify adding a delay or treating them as prefetched media.
Prediction: sound preparation should no longer queue behind texture-drain
jobs; preparation work and decoded samples should remain unchanged. No FPS or
codec-speed gain is predicted. `DH2_DEBUG_SOUND_ACQUISITION=1` records queue
and work time separately; `DH2_DEBUG_SHARED_SOUND_JOBS=1` selects the former
generic-pool scheduling for a same-build comparison. Intermediate cfbbba2
compiled in dev and release; dev EotW/Dust clients completed with no runtime
errors. They reproduced first-use fly preparation queued for roughly 0.54 s /
1.52 s on the shared IO pool; the dedicated path's corresponding jobs queued
for about 0.5–1.1 ms. Work itself was below 0.2 ms in both modes. These dev
runs identify scheduling contention; release measurements and the final
standard-task-pool source batch must establish the performance result before
claiming an audible fix.

Release cfbbba2 comparison: four 45-second walking clients completed with no
runtime errors. Shared-pool queue medians were 3.485 / 5.849 ms; dedicated
single-worker medians were 0.733 / 0.540 ms. The dedicated-copy run also had a
20.833 ms header-read wall-time outlier (only 1,062,704 thread cycles) that
delayed later jobs; thus this is not a universal latency reduction. The final
private task pool preserves the old IO pool's preparation concurrency instead
of reducing it to one worker. Prediction: another slow header read no longer
serializes all preparation when the old pool has multiple workers; the codec
work, encoded-data budget and GPU work stay unchanged. Extra session worker
stacks are host overhead, not a claimed RAM saving. Those cfbbba2 numbers are
not assigned to its newer implementation; the final comparison follows below.

Final implementation 7b6801a: four optimized 45-second walking clients
(shared / dedicated / dedicated / shared) all exited 0 without runtime errors.
The unchanged route produced 34/35/35/33 stream preparations. Shared queue
medians were 2,898/2,959 us; dedicated medians were 493/565 us. Maximum waits
were 7,272/5,868 us shared versus 2,383/1,945 us dedicated. Random event choices
change the job mix, so the same (kind, source ID) pairs present in every run
were also compared: 21 identities, median per-identity queue 4,653 -> 554 us.
Each identity uses its two per-run medians before that cross-identity summary.

For those matching identities, preparation work medians were 187.5 -> 156 us
and 653,065 -> 534,992.5 thread cycles. This supports no measured preparation
regression in this route; it is not a codec optimization claim. Source
arithmetic/media stayed unchanged. Both fly sources were exercised in all
four runs. No isolated slow-header event occurred here, so the comparison does
not prove an arbitrary worst-case latency bound. These are sparse source-job
measurements during walking, not whole-frame CPU, FPS or speaker latency.
User audibility remains unconfirmed. Production evidence:
`artifacts/sound-final-pool-comparison.json` and the four corresponding
`sound-final-pool-*` logs. No tests or visual inspection were performed.

The copy controls recorded 301,028 / 256,620 pose entries and 33,136,236 /
28,247,940 cumulative payload/string bytes copied across 3,514 / 3,033 frames.
Publication controls made zero extraction copies. Instrumented clone-block
medians were 61,775 / 60,445 cycles, including timer overhead; these are not
an isolated before/after CPU subtraction. Stationary loading frames had empty
pose maps, while the walking route exercised nonempty ones. These are copying
and allocation-churn results, not process-RAM/FPS savings. Evidence: local
`artifacts/sound-fx-release-comparison.json` and its source/raw logs.

Bank-generation admission now travels with each immutable registry snapshot.
Checks under the prepared-media cache locks reject a retired snapshot before
lookup/publication. Retirement advances the token before purging the cache,
so a background decode finishing later cannot restore an outgoing map's cache
entries (`1410f1910` / `141122400` release bank-owned sources). Session shutdown
also retires publication before queue join. Active source/voice leases still
finish using their own data; retained common sound identities are unchanged.
Prediction: two atomic generation comparisons per cold cache preparation and
one on cache hits, with no per-sample or warm walking-frame work. Normal cache
counts should stay unchanged; cancelled work must not create new retained
entries after retirement. This final source batch is pending compilation and
production verification; the race is source-identified, not claimed exercised
by the completed two-map runs.

### Runtime initial-image failure retirement

Original `1402ea140` reports an invalid binary base image through
`14005f790` with code `0x0d`; that path is fatal (`140166830`), rather than a
successful acquisition or an invented white/default image. The host runtime
material path previously left an initial missing/read failure with
`attempted = true`, no handle and no outstanding IO. The waiting effect could
therefore retain its source/batch indefinitely without a readiness or failure
acknowledgement.

`TextureReadWorker` now records sparse, slot-owned initial read/upload failures.
`RuntimeFxImages` attributes them to the currently published request generation;
the existing failure signals and material-coordinator error path cancel/drop
that waiting batch, and the effect owner removes failed pending requests so
normal revision/retirement releases their sources. Retiring/reusing a dynamic
slot clears its failure entry. Finer-mip failure retains the resident chain.
The host's existing startup GPU retry behavior is unchanged; this runtime
recovery is explicitly a host error mechanism, not a claim to reproduce the
original fatal-error UI. Successful resident slots allocate no failure records.
Source integration is complete; production failure/cancellation execution and
changed-path costs remain to be verified before full lifecycle acceptance.

### Shared skeleton and body-camera pose: release acceptance

Release game source `a8fc323` compiled successfully. Two production runs each
completed Edge of the World -> Dust District -> Dust Return -> Edge of the
World and exited 0. Separate/shared modes each requested 225 skeletons across
79 paths; source bytes/CRCs, parsed size fields and model content signatures
matched. The owner reduced read/parse calls from 225 to 208, with 17 live reuse
hits. Acquired/released identities reconstruct zero final live skeletons and
zero unmatched release records in both modes. Peak full-skeleton logical
payload was 60,006 versus 34,265 bytes (11 versus eight live identities).
These are logical payloads, not allocator capacity or process RAM; the separate
mode does not recreate the older subset-only player skeleton representation.

Two further production clients walked the same two-way waterfront route for
75 seconds each and exited 0. The isolated parent-source block recorded
3,529 copy-mode and 3,357 shared-mode samples: medians 2,905 -> 1,540 thread
cycles; means 3,054.293 -> 1,582.058 cycles. Copied bytes were exactly 320 per
sample versus zero, eliminating one parent-vector allocation per pose.
Both controls retain the shared skeleton reference, so these numbers isolate
the removed allocation/copy; they are not complete Body::pose, frame-time,
FPS or historical process-memory comparisons. The pose's diagnostic comparison
field adds eight header bytes compared with the former parent Vec header.

Raw skeleton read/parse sums were 15,211,338 versus 10,983,410 cycles, but path
mix/cache warmth and diagnostic work prevent treating that aggregate difference
as the isolated gain from 17 avoided parses. No such CPU percentage is claimed.
Production evidence: `artifacts/skeleton-resource-comparison.json` and its
`resource-skeleton-*` / `resource-parents-*` logs. This verifies the exercised
resource and walking paths, not unfinished music, Kiscule ownership, general
scene playback or player-state admission.

Camera joint-control correction (source integration, 2026-11-05): the numeric
hand capture `collision-1793889929793.txt` rules out missing renderer admission
and skinning failure for the selected catfall right arm. Original joint mods
14053f5f0 enqueue stage4/type2/additive1 controls; 14053bcd0 materializes each
joint's updated hierarchy before applying the next modification. Descendant
pivots therefore follow preceding ancestor rotations. The previous host pose
helper incorrectly used unmodified pivots. Controls now prepare those pivots
once and share the result among body/tag/arm/camera consumers. Sorting reuses
the existing idLib quick sort: 140540630 compares joint, stage and type; its
equal-key ordering is not stable. Camera controls share stage/type, leaving
joint as the varying key. This changes output only where an earlier ancestor
turn moves a later pivot or equal-key ordering matters. It is a correctness
correction, not a claimed performance improvement or proof that both missing
hand symptoms are resolved. Prediction: one prepared list replaces duplicate
view/arm control preparation; each entry retains one extra Vec3, with no new
per-joint allocation or GPU resource. Compilation and production verification
of this source batch are pending.


### Catalog-owned read history

Host owner: `loading::ReadHistory`, selected by immutable `archive::Catalog`
identity. The original has one registered resource manager (`1400607f0`,
`140060180`, `1400661e0`); independently opened host catalogs must not share
its acquisition credits by path alone. Previously only the common-text owner
checked catalog identity. Ordinary successful `read`/`with_stream` records,
release, progress and reports could inherit another catalog's read history.

Each catalog now owns a separate history namespace. The active catalog keeps
the direct history fast path; other live catalogs retain independent records.
Selecting a level moves its existing bootstrap history into that active slot.
Common-text publication updates only its own namespace, animation release
names its actual catalog, and catalog destruction drops its history without
initializing tracking for an unread catalog. Aggregate family release still
uses the existing family owner's retained-identity predicate. This changes
host accounting and its lifetime, not archive payload decoding or GPU work.

Prediction: zero cross-catalog precredits and zero retained history namespaces
when their catalog owners are gone. Normal single-catalog reads add identity
comparisons but no new hashing/allocation; repeated publication of the same
common text Arc does no acquisition-index walk. No FPS or decoded-data RAM
saving is claimed. Final compilation and production transitions remain to be
checked; the two-catalog interference is source-identified, not runtime-claimed.

### Action-only arm geometry capture

`app::player_arm_capture` records at most 64 camera/palette snapshots per
model/action only with `DH2_DEBUG_HAND_GEOMETRY=1`. Normal play performs no new
palette retention or camera inversion. A ring wrap reuses its old palette
allocation; capture-time CPU skinning/frustum classification happens only at
F8. Next matching action or level retirement drops the old history. It checks
conservative pre-jitter frustum intersection, not rasterization, alpha or
occlusion. Camera-control f25c019 compiled, but the user's check still found
intermittently missing hands; this diagnostic does not constitute a hand fix.

### HUD ownership and sensor scheduling integration (2026-11-05)

`app::hud_movie` uses the installed HUD movie and the existing Flash player,
font, bitmap and GPU texture owners. Player-owned health, mana, oxygen,
interaction focus and crosshair state feed the authored presentation. Ammo
remains hidden until its real equipment owner supplies it. The supported
initial possession state is constructor value 14 (1407f7860 -> 1408216d0);
this does not implement possession abilities or saved-state restoration.

Host: a HUD tick has one acknowledged worker handoff. The main thread services
texture acquisition and failure/disconnection events while waiting, preserving
ordered focus transitions rather than accumulating unbounded updates. The
original host 14002cc90 permits at most one authored movie frame per engine
update, retaining phase debt. Elapsed timer time is retained separately;
141021b40's exact distinct internal clock consumers remain a Flash-runtime
integration task. HUD sounds carry the level epoch, so retired movies cannot
post into a later level. Pause and loading sounds retain their existing owner.

GPU bitmap entries use shared image allocation identity, with weak image/movie
leases preventing address reuse and retaining still-live hidden movies. Glyph
entries use weak page leases and dirty versions. Incoming leases are registered
before retirement; lookups use the batch character reference rather than a
full image-map scan. Native bounded bitmap cache limits remain in force
(140034ba0 -> 1402df160 / 1402e3460). This is host lifetime integration, not
a claim that 1410150f0 describes bitmap cache ownership. Prediction: repeated
use of a still-live HUD across pause reuses eligible uploads, outgoing movies
release their leases, and one in-flight tick bounds queued HUD work. Existing
opt-in HUD/cache diagnostics count affected work. Compilation and production
verification of this integrated HUD batch are pending; no CPU gain is claimed.

The latest hand report collision-1793899814727 records both contact flags false,
not a renderer admission failure. Detailed geometry recording was disabled in
that manually launched process; the configured process had been hidden by our
launch helper. Startup now records the actual diagnostic flag in the session
log. The native frame routine 1408af8a0 issues QUERY_PHYSICS (stage 1), resolves
140096240 and waits through 14007a9e0, then runs DEFAULT (stage 2). Catfall
1403a5e30/1403a5a00 and mantle 1403bcb20/1403bcb00 consume results from that
same frame. The rewrite's previous-frame result consumption was corrected;
mantle still issues from its existing prior-stage destination/forward state.
This removes stale contact results without adding queries or GPU work; it is
not proof that the remaining collision geometry gaps or missing hands are fixed.

Broadphase tree-filter evidence was reused from the existing point-query audit
and independently confirmed: arkCollisionQueryFilter vtable 141ad4b68 and
arkCollisionFilter vtable 141ad4c08 both dispatch slot +0x38 to 14011d080,
which returns true. Visiting all trees is exact for both; body and leaf filters
still apply. No redundant always-true filter implementation was added.

The e1cc75b sound/catalog source batch compiled successfully. A production
EotW -> Dust -> Dust Return -> EotW client completed with exit 0 and retained
the same catalog, GPU device, window and single audio output across transitions.
No retired-generation error was logged. This verifies the exercised transition
path, not an injected cancellation race or final sound latency comparison.
The process-wide Catalog::shared intentionally retains its catalog until exit;
static destruction is not expected to emit a final catalog Drop record.
### Reusable penetration scratch (source integration, 2026-11-05)

`convex_support::solver::PenetrationScratch` is a lazy query-owned host
workspace for 140c231b0's bounded penetration search. Original solver exit at
vertex count greater than 0x31 and hull expansion 140c246a0's 90-face bound
provide the reserve sizes: 50 vertices, 90 faces, 90 retired-face slots and
50 boundary links. Geometry arithmetic, traversal order, statuses and retries
are unchanged. The existing point-query and new convex-cast owners share their
own scratch across serial leaf queries; neither a process cache nor retained
shape source is added. Dropping the query owner releases all four vectors.

Prediction: after the first ready penetration seed, resetting the hull performs
zero heap allocations or capacity growth. Four positive-capacity vector
allocations occur only at initial scratch construction rather than for each
ready hull. This trades a bounded retained workspace per active query owner
for allocation churn; it is not a process-RAM saving. Stateless convenience
functions retain their API and use a temporary workspace. The successful
feature-result path also stopped eagerly allocating its unused error String.

DH2_DEBUG_PENETRATION_SCRATCH=1 records reset count, initialization count,
isolated reset thread cycles and retained capacity bytes on owner release.
DH2_DEBUG_SEPARATE_PENETRATION_SCRATCH=1 recreates the same bounded hull each
reset for a same-binary comparison; both modes use identical arithmetic and
bounds. This control measures reuse, not the historical vectors' growth policy.
Production measurements and the integrated collision build remain pending;
no timing, memory or output acceptance is claimed from source review alone.

### Scaled compressed cast preparation (source integration, 2026-11-05)

`gameplay::compressed_cast_scaled` ports 14124faa0's scaled-target setup:
query calcAabb uses the original world frame, with columns multiplied by query
scale and shape-info offset added directly to translation. Nonnegative query
tolerance expands that AABB in world space. Target BVH bounds use the target
rigid columns multiplied by target scale and translation plus its shape-info
offset. The target-local cast displacement is rotated by the target rigid
world frame; signed zeros are cleared, W is the original constant one, and
zero XYZ reciprocal lanes use 0x7f7fffee (141ad2740). Existing reciprocal and
query calcAabb owners supply their original arithmetic. No geometry copies,
new heap storage or GPU work are added by this helper. It does not implement
leaf scale/radius adjustment or publish a second collision owner. Source
integration, cohesive compilation and affected-path production validation
remain necessary before enabling the new cast owner.

### Component slot destruction and failed acquisition (2026-11-05)

`component_state::pool::ComponentPool::release` and
`component_state::roster::ComponentRoster::release` port 14048ff20's slot
release through 14048ff80 before the virtual destructor. Only an inactive
allocation can be released: the lifecycle owner must first stop its scheduling,
detach callbacks/bodies, and unlink its entity. The allocated suffix and free
suffix preserve the source's ordering; used/cursor decrease while chunk
storage and the high-water mark remain for reuse. There is no second free list.

Host deviation: a fallible Rust constructor or chunk acquisition returns its
reserved roster slot before propagating the error. Previously these early
returns permanently consumed a reservation until whole-pool teardown.

Prediction: releasing a component drops its owned resource leases immediately
and returns exactly one reusable slot, without allocating or freeing chunk
storage. Failed acquisition leaves the live slot count unchanged. No ordinary
walking CPU improvement or process-RAM reduction is predicted; inactive chunks
remain resident by design. Source implementation is present; integration with
typed release callbacks and production-client unload evidence are still
required. No tests or compiler were run for this isolated source change.

### Music-track clip timing (source implementation, 2026-11-05)

`gameplay::sound::music_clip` ports 141159840 / 141159fd0 / 1411588a0's
40-byte serialized clip to six-word sample timing conversion. It uses the
actual engine rate, preserves double arithmetic and operand order, skips
nonpositive trimmed durations, applies signed half rounding and CVTTSD2SI's
invalid-input result, and wraps negative source offsets as the original does.
Constants .001 and +/- .5 were read from the installed image. The host reports
an invalid integer divisor rather than substituting a duration.

This record retains no media/decoder and allocates nothing on success.
Prediction: new music scheduling support, no walking CPU or resident-memory
gain. Full track/segment/container playback remains unimplemented; this is
not a claim of audible music. The acquisition batch compiled in a85088d;
clip sample conversion has not yet been exercised by scheduled playback.
Existing bank/media owners stay authoritative. No tests were run.

The bank owner now exposes `music_track_prelude`: borrowed source descriptors,
40-byte clip records and automation-point ranges over its existing bank bytes.
The ordinary HIRC property reader uses this same prefix reader instead of a
second skip-only implementation. Source plugin parameter blocks and curve
points are neither cloned nor eagerly decoded into resident arrays. Bounds
checks cover variable source parameters and counted clip/point records.
Track mode/switch settings are covered by the full reader below; playback
still requires implementation;
the prefix is explicitly not treated as a complete music track.

`music_track` additionally reads the common node properties, playback byte,
type-3 selector (14115a0a0), eight serialized transition words and lookahead
input passed to 141159280. Selector arrays stay borrowed. Transition words
and lookahead remain serialized inputs, not guessed scheduling rules; the
engine's additional lookahead factor and transition behavior still need their
actual scheduler implementation. This does not enable a generic Sound actor
fallback for music. The ordinary property-only path does not decode or retain
these extra fields when they are not requested.

`PlaybackBanks::music_track(id)` requests that view through the existing bank
namespace, using its actual feedback flag and retained actor record. No track
decoder/index/media cache is created beside it. Borrowing prevents the view
from surviving bank unload. The wrong actor kind is an explicit error rather
than an ordinary sound fallback. A future prepared track must own its runtime
clip records and use the existing bank/source lease for scheduled media.

Segment metadata is now acquired through the same bank namespace:
141154620's common fields/child IDs/meter/stinger records and 141155700's
duration plus variable-length cue records. Cue labels, children and stinger
arrays borrow bank storage; no temporary label String or secondary encoded
buffer is retained. Bounds checks cover counted arrays and variable labels.
Music-node meter/stinger interpretation, marker sample conversion, transitions
and scheduled playback remain separate work; serialized fields are not used
as guessed runtime settings. Compiled acquisition coverage is recorded below.

Music switch-container acquisition now follows 141151320's transition rules
and 1411583d0's group/type arrays, continuation flag, mode and decision-tree
bytes (1411582e0 / 1410ee910). Source/destination selectors and tree storage
borrow the bank. Fade/rule/bridge fields follow installed reads and masks;
field names are cross-referenced against wwiser's version-113 format reader.
No extra tree copy, selector Vec or transition-array cache is introduced.
Decision-tree evaluation, transition scheduling and playback remain pending;
ordinary sound-switch behavior is not substituted for music switching.

Music random/sequence-container acquisition follows 1411576a0's 30-byte
serialized preorder playlist and the same transition-rule reader. Entry
loops/modifiers/weights/repeat and selection flags retain their original
widths; no runtime cursor or random state enters the shared definition.
Playlist entries borrow the bank and decode on demand. This completes source
metadata access for HIRC music kinds 10–13, not their playback implementation.
The compiled acquisition diagnostic is recorded below. Runtime streaming/voice
scheduling remains required; no audible output/timing result is claimed.

`wwise-audit --music-resources [package-or-directory]` exercises these actual
bank-owner APIs over all installed bank/language entries without opening
media/voices. It reports per-kind objects, clips, cues, rules and selectors
plus contextual parse failures. Each bank drops before the next acquisition;
it does not turn format diagnostics into an all-banks resident cache. This
command compiled alongside the game in a85088d and exited 0 over 176 installed
banks across all installed language entries. It acquired 6,271 segment, 7,767
track, 34 switch-container and 300 playlist definitions with zero per-object
parse failures. These are bank-definition counts, not globally unique actor
IDs or simultaneously resident runtime objects. The command exercises each
owner's complete metadata reader and its variable-length record iterators;
it does not exercise sample conversion, RNG selection, scheduling or sound.
Evidence: `artifacts/music-resource-bank-acquisition.log` and its exit/source
records. No tests or visual inspection were performed. This is acquisition
coverage, not an audible-music check or original RAM-budget comparison.

### Retired catalog accounting (2026-11-05)

Host read accounting now clears the retired active catalog's play flag and
releases both manifest lookup buffers, alongside its existing history and
manifest release. A subsequently opened independent namespace no longer
inherits late-load classification from the retired world. Inactive-catalog
retirement leaves the current world's phase intact. This is diagnostic-owner
cleanup corresponding to 140060180; the process-shared gameplay catalog stays
resident as before. Prediction: no ordinary walking CPU/GPU change and no
claimed gameplay RAM saving. The source compiled in a85088d. The process-shared
catalog does not retire during ordinary map changes; that route cannot prove
the independent-catalog retirement branch was executed.

### Acquisition-batch production transitions (2026-11-05)

The a85088d game build completed EotW -> Dust District -> Dust Return -> EotW
in one production client, exit 0, with no panic/ERROR/client failure. Catalog,
device, window and single audio-output identities stayed unchanged across all
four acquisitions. Evidence: `artifacts/music-resource-lifecycle.{out,err,exit}`.
This exercises the refactored common sound-node prefix reader during ordinary
level acquisition/reuse, not music playback or the new component pool release
callbacks (those callers remain on the interactions branch). No appearance,
frame-time or new UI-cache-residency claim is made from these diagnostics.
Host fallible factory admission now also undoes the pre-allocation class-cache
insertion when construction fails. If handle conversion, entity-list insertion
or registry insertion fails after construction, it removes the exact inserted
cache/list entries and returns the inactive typed slot. Registration preserves
the original link-before-append order but restores the prior owner on failed
queue insertion, including denied appends that return an invalid index instead
of Err. The normal successful admission order and generation allocation remain
unchanged (875d50 / 7e7210). Immediate-start callback failures remain attached
to their real entity/registry for lifecycle cleanup; they are not blindly
dropped after callbacks may have registered listeners. Prediction: no walking
cost or new success-path allocation; failure no longer strands a slot or leaves
a false cached class. Production error-path exercise remains outstanding;
no tests or compilation were run for this source fix.


### Host archive-session teardown

The desktop launch now holds an explicit catalog-session root around the
application. Its teardown releases the static shared catalog reference on
both success and setup/runtime failure. Outstanding world or worker Arcs
still keep the catalog alive; destruction is deferred until their final
release. Ordinary level changes keep the same root and retain init/common
resources, following the separation established by 1400661e0/140060180.
This is host ownership of an archive namespace, not an additional game
resource manager or a forced purge of live payloads.

Catalog destruction now removes only matching-identity cache references in
the declaration-text, contact-database, camera-blur, compressed-animation,
render-parameter, CPU movie-bitmap and request-manifest owners. Consumer Arcs
remain valid. Installation-keyed shader-cache indices remain with their
independent process/device metadata owner; skeletons already retire through
weak identities and final-consumer tokens. Cache retirement happens outside
the loading-state mutex, and payloads taken from singleton owners drop after
their owner mutex is released. No extra checks run on the gameplay path.

Status: host implementation; production shutdown/error-path acceptance is
pending the cohesive build. Prediction and exact scope are recorded in
optimization-opportunities.md. No RSS, timing or behavior benefit is claimed
without that acceptance, and no tests were written or run.


Session teardown acceptance found an additional lease: the three-level
EotW/Dust/EotW production client built from dfdcc0b exited successfully, but
emitted no final catalog-destruction record. The movie-thread catalog cache
still held a strong reference outside the application. Session teardown now
releases that exact-identity cache after the application drops and before
releasing the shared root. This changes no menu acquisition hot path. Root
release diagnostics now report outstanding consumer references so a missing
final drop cannot be mistaken for success. The map-info registry and selected
loading-info cache likewise retire only their matching catalog identity.
These follow-ups require a new build and actual retirement evidence; the
successful transition alone did not verify the first retirement prediction.


The 6c8d409 follow-up still exited zero with one catalog consumer remaining.
Source tracing identifies the dedicated `pause movie` thread: its JoinHandle
was discarded, and its thread-local catalog outlived the App while main
returned. Releasing the same TLS name from the main thread cannot release
that worker's cache, so that ineffective helper has been removed.

The PauseMenu owner now sends shutdown and joins its actual worker on Drop.
The worker exits before session-root retirement, releasing its movies, VM,
thread-local catalog and stack. To avoid a destructor-order deadlock, the
owner can cancel its single outstanding synchronous named-image reply before
joining, even when a pending reply is held by the main image owner. The
normal image payload/channel and movie update protocol remain the same; no
polling timer or additional render pass is added. Normal named-image binding
adds two short mutex operations and a reply-handle clone, with at most one
cancellation record. This is host thread ownership around Iggy's existing
movie close (1410298b0); it is not a second movie resource cache.

Prediction: a completed production session now logs zero remaining catalog
consumers and the actual cache-destruction record. Movie cancellation and
shutdown acceptance remain required; an exit-zero process is insufficient.
No CPU improvement or RSS saving is claimed. No tests were written or run.


Host session-teardown acceptance is verified for ce831ef: the production
client completed EotW -> Dust District -> EotW and exited 0. After joining the
movie worker, session-root release reported `remaining_consumers=0` and the
actual Catalog destructor retired 209 animation entries, the blur/contact/
declaration/render-parameter owners, 15 bitmap libraries, four manifest
entries and both map-info cache owners. Read-history teardown released 7,852
owned path records (662,228 string-content bytes), leaving zero catalog
histories. These are owner/entry counts and string contents, not RSS or
physical GPU-byte savings. No gameplay CPU or frame-time claim is made.
This supersedes the earlier pending shutdown acceptance: both earlier
exit-zero runs lacked actual final retirement, and only the joined-worker
run proves that requirement. Evidence: local movie-retirement-build.source
and movie-retirement-final-client.{exit,out,err}. No tests or visual
inspection were performed.


### Audio session producer drain and final cache retirement

Host integration for the existing Wwise source lifetime (`141122400` /
`1410f1910`) and dedicated streaming/preparation owners (`1400bc8f0` /
`1410bf740`): `app::audio::session` now closes admission, drains work leases
acquired before map preparation, event-decode and stream-read submission,
then retires the prepared MEDIA/STREAMED and Vorbis header/setup cache roots.
The guard drops after App and its queue owners, before CatalogSession.
The existing dedicated PreparationQueue, source generations and cancellation
behavior remain intact; this does not move jobs back onto the graphics IO
pool or substitute an earlier audio implementation.

An outgoing decoder keeps its own immutable Arc valid. Final retirement
clears cache ownership even if such a decoder temporarily outlives the app;
it does not rely on a last unused-header scan happening at exactly the right
moment. Already running jobs finish before retirement; queued work that sees
closure exits, and canceled jobs release their captured lease. Streamed
sources retain a session handle so output-thread submissions use atomics
without the global session mutex. Cached/ordinary per-sample decoding adds
no new guard operations. The map-worker lease also covers scoped startup
sound preparation and cleanup when the receiver has already disappeared.

Prediction: final cache entry counts reach zero after admitted producers
retire, with no late repopulation. There is no predicted playback CPU/GPU
improvement. Submission adds bounded atomic/reference-count bookkeeping;
`DH2_DEBUG_AUDIO_RESOURCES=1` measures admitted jobs, peak concurrency,
acquisition bookkeeping time and shutdown drain time. This source batch has
not yet been compiled or production-accepted; no byte/time saving is claimed.
No tests were written or run.


Audio session normal-shutdown acceptance: dev game build of `be4d6c8`
succeeded, then the production EotW -> Dust -> EotW client exited 0. The
existing dedicated source queue ran actual stream reads. The session tracked
62 admitted jobs with peak concurrency 12; instrumented handle acquisition
latency totalled 0.003 ms and shutdown wait rounded to 0.000 ms. These are
narrow guard diagnostics, not whole-CPU or walking-performance measurements.
Final shutdown removed four Vorbis headers and two setup entries, with both
maps empty afterward. MEDIA and STREAMED were already empty after the
existing SourceJobs retirement. Catalog release then reported zero remaining
consumers and retired its matching metadata/history owners. This confirms
normal shutdown and preserves the earlier catalog cleanup; close during an
active map worker is being verified separately. No tests or visual checks.
Evidence: `artifacts/audio-session-build.source`, `audio-session-build.exit`,
`audio-session-client.{out,err,exit}` in the same worktree.


Close-during-loading evidence and cancellation prediction: the same built
production client received its normal window close request after `LOAD world
mesh`. It exited 0 with zero final catalog consumers and empty audio/header
roots, proving that the admitted map-worker lease drains safely. However,
the worker still completed program/probe/Tome/collision/machinery acquisition;
the audio guard measured 2,528.676 ms waiting for that unnecessary work.

The map preparation owner now checks its caller's existing cancellation state
at entry and the six existing acquisition phase boundaries. This adds no
per-read hook, thread, allocation, GPU pass or gameplay-frame work. A phase
already running can finish; subsequent phases do not acquire resources for
an abandoned map. Based on the measured phase sequence, the same close point
should stop by the program-cache boundary, skip probes/Tome/collision/machinery
and eliminate their acquisition counts and most of the measured worker wait.
The expected result is resource-work removal, not a walking-CPU improvement;
wall-clock process exit is not the affected-code metric. This follow-up is
not yet built or accepted. No tests were written or run.


Cancellation acceptance: the `ba7280d` dev game build succeeded. The same
production close request after `LOAD world mesh` exited 0 and stopped after
the program-cache boundary, matching the predicted phase/work removal:
there are no probe, Tome, collision or machinery acquisition phases after it.
The affected session's worker wait fell from 2,528.676 to 301.019 ms (88.1%);
that includes finishing the current phase and dropping its prepared data,
not merely the cancellation predicate. Process exit after close was 0.429 s,
reported separately from the worker wait. The retired catalog's acquired-path
count fell from 2,001 to 1,755; these are request-history counts, not a RAM
saving claim. Both runs released all catalog consumers and cleared audio and
header roots. The result supports the work-count prediction; it does not
claim a walking CPU/GPU improvement or original-game exit-time comparison.
Evidence: `artifacts/audio-cancel-build.{source,exit}` and
`artifacts/audio-close-cancel-client.{out,err,exit}`, compared with
`artifacts/audio-close-client.{out,err,exit}` from the guard-only build.
The normal three-level lifecycle acceptance remains recorded above. No
tests, screenshots, visual inspection or gameplay navigation were used.


### Empty declaration acquisition

The installed `generated/decls/fx/models/effects/empty.fx.decl` is a valid
six-byte empty object. The shared `gameplay::options::decl::read_edit` reader
previously rejected it solely because it lacks an explicit `edit` field.
Original `1401c5b20` resets FX data, calls common parser `140022fd0`, then runs
its group postprocessing. The common parser tries its editable blocks and
marks the resource defined even when neither exists. The FX reset
`1401c5ad0` clears the group count; `1401c5c10`/`1401c5b70` accept that empty
list without creating particle/sound/decal instances.

The shared reader now returns an empty field set for a syntactically valid
empty root object, matching an explicit empty edit. No effect-name/type
exception, fabricated group, fallback asset, cache or GPU allocation is added.
Missing files, malformed objects, non-scope edit values and unsupported
non-empty roots retain their existing errors. Existing acquisition owners
and downstream class defaults are unchanged.

Prediction: the current EotW FX inventory's sole acquisition failure becomes
zero, with the same 99 effect identities and zero groups/resources for the
empty declaration. This is format/lifecycle correctness, not a CPU or RAM
speedup. The existing read-only tools binary supplied the installed-data
baseline (91 declaration requests, 99 effects, one failure); it predates this
change and is not runtime acceptance of the new reader. Source references are
saved locally under `artifacts/fx-declaration-*.c`; installed data remains
uncommitted. No tests were written or run. Build/production acceptance pending.

The existing `DH2_TRACE_WORLD_FX` acquisition diagnostic now records each
prepared definition's group count or actual error. This runs only at resource
acquisition, with one opt-in check per batch, and will distinguish a valid
zero-group definition from a rejected declaration in the production client.


Empty-declaration acceptance: `a1a972a` built both the actual game and the
existing read-only tools successfully. The updated installed EotW acquisition
has 99 effect identities and zero effect acquisition failures; declaration
requests/reads remain 91/87. A production EotW run acquired the empty effect
with `groups=0`, then the ordinary authored-actor preview reported one
attempt, one acquisition and zero rejections, and the client exited 0 with
zero final catalog consumers. No default groups or fallback resource were
created. The pre-existing named-variable graph binding error remains a
separate graph-owner integration issue; zero FX declaration failures does
not claim all gameplay FX are implemented. The older tools inventory has a
different material-read count (44 versus the rebuilt tool's 45), so that
cross-version count/timing is not attributed to this parser change.
Evidence: `artifacts/empty-fx-build.{source,exit}`,
`empty-fx-inventory.{out,err,exit}` and `empty-fx-client.{out,err,exit}`.
No tests or visual checks were used.
### FX release error ownership

`WorldParticleState::retire` retains its live instance slot while particle, decal, sub-effect and sound chain releases can still fail. Completed family heads remain advanced in that same record, so an error no longer drops the declaration lease and remaining chain heads while leaving the handle and pool allocation registered. The slot is removed only after every family clear succeeds. This is `host:` Rust fallible-release ownership around the existing native family cleanup ordering (`140223c20`, `1401a31f0`), not an invented native exception behavior. Prediction: no new allocation or per-update work, no CPU gain claimed, and no orphaned instance record on a family-clear error. Source inspection confirms all fallible family clears precede `take`; production integration is still pending with the shared lifecycle batch. No tests or synthetic failure runs were performed.

### World FX release at the host level boundary

`LevelLifetime::clear` now invokes the existing `FxSimulation` owner's explicit world shutdown after the outgoing GPU publication acknowledgement and before removing its supporting resources. The manager clears particle, decal, sub-effect and sound families through their existing owners; callback routes are disarmed before a fallible clear. A failure propagates to the loader's normal fatal diagnostic instead of allowing the next map to replace still-live resources. This is `host:` integration of currently hosted effects, not a claim that arbitrary Bevy resource destruction reproduces every native entity/component callback. Native shutdown `1408a4830` requests components over the spawned list, drains the shared component release queues (the caller's literal second argument is not consumed by `1407ea870`), clears managers, destroys managed entities in ascending slot order via `1407b5f50`, then clears graph/event/FX owners; `140199df0` is called before resource purge. The full typed ordering remains under integration. The old comment attributing GameLocal destruction directly to conditional purge `1400410f0` was corrected. No per-frame work or allocation was added; production shutdown counts and the cohesive integration build remain to be collected.

The isolated production build caught a missing dependency from the shared draft: FX shutdown uses a receiver-detached owner state. That state and its retirement branch are now included explicitly, so a detached instance retains and clears its own tails without sending completion to a removed graph. This adds no allocation and does not remove the instance before its family cleanup. The initial build failed with three missing-variant errors; runtime acceptance was not started from that failed build.

World FX boundary acceptance: source `4585d3f` built the production game successfully. The existing production client loaded EotW → Dust → EotW with the same catalog, device, window and single audio output, then exited 0. The two outgoing-world calls reported live instances/handles `166→0` and `257→0`, shared particle slots `161→0` and `257→0`, and sound-family slots `69→0` and `54→0`. Decal-family slots were already zero in both calls, so this run does not prove active decal clearing. Final catalog consumers and catalog read-history namespaces were zero. The existing actor-admission rejections remained visible; this is release acceptance for supported live effects, not a claim that every FX family is implemented. Evidence: `artifacts/fx-shutdown-build.{source,exit}` and `artifacts/fx-shutdown-client.{out,err,exit}`. The failed first isolation build is preserved as `fx-shutdown-build-attempt1.*`; no client was launched from it. Counts match the predicted release result. Dev elapsed times are diagnostics, not CPU-performance or RSS measurements; no tests, visual inspection or self-navigation were used.


Live font-provider reuse integration: 14102bd60 attaches the original font provider to its serialized record. The existing IggyFile now holds weak provider identities keyed by record ID; movies share the live provider's metrics, kerning and lazily decoded outlines, while glyph pages remain on their existing movie owners. Provider -> source-file leases remain strong and source-file -> provider links remain weak, so final movie/provider release does not form a cycle. Cloning a file constructs an independent empty provider index. No font pixel format, glyph precision, rasterization, GPU upload, UI timing or gameplay path changes. DH2_DEBUG_SEPARATE_FONT_RECORDS retains the former distinct-provider route in the same binary; eager-outline diagnostic behavior stays unchanged.

Prediction before this main-based integration: for the previously measured14 acquisitions across7 live providers, created provider count14 ->7, reuse0 ->7, record-tag peak about400,964 ->200,634 bytes (200,330 saved after the weak index cost); request count, source glyph/metric values, final release count and zero final live providers must match their respective owners. This repeats the already completed resource-branch result on main's working loading path. No CPU/frame-time or whole-process RAM saving is inferred. Native original tag budget is the existing tests/fixtures/original_eotw_budget.txt TAG_IGGY capture; no new game assets are converted or copied. Resource-only integration source is isolated from incomplete gameplay component work; production repeated-level and provider lifetime evidence are required before merging main. No tests were written or run.


Font integration dependency/sweep: the main-based build identified that the allocation-tag live_bytes helper was absent from main. The existing allocation owner now exposes its nonallocating single-tag read without changing allocator accounting or adding per-allocation counters. shared_font also checks the immutable character kind before environment controls/tag bookkeeping: load_library asks for every character, so non-font records now make zero debug-mode environment queries instead of three. Actual font requests retain the same modes and same provider result; read_font revalidates only those few font records. Prediction is3*N ->3*F cold environment reads per library scan (N character requests,F actual fonts), plus one extra immutable kind check for each actual font; no walking-frame work changes. Source-level operation count is the intended gain, with no timing claim before targeted measurement. This completes the font batch dependency/known repeated-work cleanup; rebuild and production owner capture remain required.


Main-based font validation exposed a window-resource lifecycle failure before gameplay: both provider modes exited with DX12 ResizeBuffers 0x887a0001/window-in-use, after only7 acquisitions and no reuse. These incomplete captures are not the claimed200,330-byte saving. A graphics/window trace identified the actual input: physical size1920x1080 ->160x28 with nativeIsIconic=true and window position(-32000,-32000), matching Windows' minimized dimensions, followed by restore. Winit forwarded that nonzero iconic rectangle as drawable; the backend's existing zero-extent guard therefore attempted swapchain resize while minimized. This is independent of provider reuse.

Host window fix: the Win32 WM_SIZE bridge now emits zero drawable extent for SIZE_MINIMIZED or actual IsIconic, rather than using any small-size heuristic. Bevy's existing zero-extent path preserves the last surface size, and its view-target cleanup now also removes targets for nondrawable windows, releasing old output views before restore/reconfiguration. Restore still publishes the real client extent. Prediction: zero configurations/acquisitions at the iconic extent, outgoing window views released on the minimized frame, successful restore at the real size, no added drawing/pass/allocation in gameplay; one IsIconic query per WM_SIZE event only. Evidence files are DH2-interactions/artifacts/font-iconic-window-state.jsonl and font-surface-debug-20261108. Compilation and repeated owner/window production capture are pending; no tests or visual inspection were performed.

Main-based font acceptance (2026-11-08): clean source2c6a7994, binary SHA256bfb626dd929311e6906f43ab984e278b77e9539d19ec61b9455e6e816ba50535, game+debug_allocations dev build. Both same-binary provider modes completed the normal EotW loading/level-sequence route and exited0 without client/render errors. Separate:35 requests,35 providers created/released,0 reused, peak record-tag400,964 bytes. Shared:35 requests,7 providers created/released,28 reused, peak200,634 bytes. Created/released identity multisets match in both runs; catalog session remaining_consumers=0. The measured200,330-byte peak reduction exactly matches the prediction. The greater reuse count reflects the complete loading-screen/HUD movie sequence; it is not a new resident owner. Evidence: DH2-interactions/artifacts/font-window-owner-20261108/{separate.json,shared.json,separate.stderr.log,shared.stderr.log,font-owner-report.json}. This closes provider duplication and release acceptance on the working main path, and confirms the former minimized-window startup failure is absent on these runs. It does not claim whole-process RAM, CPU/FPS improvement, arbitrary window-transition coverage, or complete all-map resource acceptance. No tests or image inspection were performed.

Animation resource integration onto the working main host (2026-11-08): bring the completed Catalog-owned AnimSet and compressed-clip identity owners from resource-lifetimes without the unfinished managed gameplay component batch. Original references: MD6 alias search1c7c80, AnimSet alias resolution247a40/generated movement2462b0/243980, creating acquisition0607f0, existing lookup05feb0, conditional purge060180 and blob owner24adf0. Rigs retain shared immutable AnimSets rather than flattened duplicate alias maps; movement records remain declaration-ordered with compact alias indices. Per-declaration cold reads coalesce, overlay generations preserve existing leases, and dead read-history entries retire only after consumers and in-flight acquisitions release. Clip records separate stable header identity from immutable playback blobs and retained load failures. Sampling still borrows the existing immutable EdgeAnimation, without new locks or resource lookups per frame. Level retirement now holds the exact preparing Catalog and purges its clip/declaration records after scene consumers drop; Catalog destruction retires its own registry. Bootstrap list reuse remains explicitly host policy, not proof of original header registration; the actual list/heap/phase registration port is a separate active task.

Prediction for this isolated batch: the previously observed five-rig route should retain204 identical clip fingerprints, reduce18 AnimSet declaration requests to12 source reads/6 reuses, and release the AnimSet-owner tag to0 at session shutdown. Stable headers add56 bytes per record (672 bytes for12 records) compared with the earlier weak-only map; this is intentional identity/concurrent-acquisition correctness, not a memory-saving claim. The compact shared aliases previously measured541,701 versus915,005 estimated owned bytes on that resource route; main must record its actual counts rather than inherit them as acceptance. No rendered animation, movement speed, draw pass, GPU upload policy or walking-frame work should change. The port keeps main's existing arm/body setup and lifecycle composition. The main diagnostic uses existing live-byte accounting only; it does not add the resource branch's global per-allocation reserved-byte counter. Build and normal-client repeated-map acceptance are the next checks; no tests are written or run.

Main animation comparison corrected the prediction's scope: main uses three player rigs, while the prior resource-branch measurement included five with additional live overlapping consumers. Main's repeated EotW route therefore records32 requests/32 reads/0 reuse in both modes, and32 matching created/released AnimSet identities. There is no declaration-sharing memory or CPU saving demonstrated on this main route; the earlier18->12 and estimated-byte reduction must not be advertised as main results. The batch ports missing declaration/generated-movement ownership and stable record lifetimes; it does not constitute an accepted optimization. Its208 unique clip signatures match across modes, and all206 unique clips from the prior main baseline match exactly; the two additional Emily grapple-look clips come from authored forceLoad acquisition. Neither render output nor CPU speed is inferred from source fingerprints.

The separate trace's240-second observation deadline expired while its actual window was minimized (IsIconic=true,160x28); the client was still running. Restoring that same window let the existing level sequence complete and its catalog release; no replacement process was launched for that run, and its unavailable exit code is not rewritten as0. The shared run was then captured separately and exited0. Local evidence: artifacts/animset-main-owner-20261108/separate.stderr.log, artifacts/animset-main-shared-normal-20261108, and artifacts/animation-main-owner-report.json, binary SHA256a0e03abc304dfb71e7ad34992330dfa774578eca82bdba14147a9271fb2ab7c1.

The final AnimSet-owner tag retained1024 bytes despite all32 provider identities releasing. Source investigation identifies a diagnostic attribution defect: allocation_tags::NAMES is a process-global Vec of16-byte string slices, bounded to64 names, but its growth inherits whichever resource tag first triggers a larger capacity. Its final64-slot allocation can therefore be charged to an unrelated owner until process exit. Before changing that mechanism, predict reserving all64 slots during first tag registration charges the same1024-byte diagnostic registry to untagged, eliminates later registry growth, and makes the final AnimSet-owner tag0 without changing game-resource ownership. The tracking-disabled build remains unchanged. This is a host diagnostic correction, not1024 bytes of resource memory savings; runtime confirmation follows in the completed batch.

Three-map acceptance after diagnostic attribution repair: clean sourcead376ca0, binary SHA2563580ab8930065d3c9246f7d1be58b664435cdd1f5c90867bc7d89fd1965ebc2a, normal client EotW -> Dust -> EotW, exit0 with no client/render errors. All44 created AnimSet identities released;44 actual source reads and0 reuse reflect the current nonoverlapping rigs. The two map purges removed48/42 map clip records, retained160 common clip records (1,114,454 logical payload bytes), and reported0 live map leases. AnimSet purge retired16/12 dead records and their matching read-history entries; final session heap was0 bytes, exactly as predicted after assigning the process-global diagnostic name registry correctly. The1072 bytes remaining at map boundaries are the session registry's retained allocation capacity, released at Catalog destruction. Existing original EotW baselines are ANIMSET6,591,156 bytes, ANIMATION195,174,788 bytes and RESOURCE51,830,920 bytes; they include unimplemented consumers and are not claimed as directly equivalent owner totals. Evidence: artifacts/animation-owner-three-map-20261108/{launch.json,metadata.json,stderr.log,stdout.log}, resource-animation-owner-accounting-complete-build-provenance.json. No tests, screenshots, performance timings, or game navigation were used.

Swimming arm visibility/idle integration (2026-11-09): the working scalar player host previously hid both arms unconditionally in Swimming. Source846330/846450 shows the empty-hand underwater case enables them; the supported host has no hold-object state or layered-action producer. Resolve Swimming/m_idle arm aliases through the same installed locomotion declaration and Clips acquisition owner, retaining the real compressed clips in each existing arm animation array. The idle leaf follows6f9b90/2406b0 blend timings, source/destination stance phase and move-state qualification. The selected player-arm speed records constructed by7021d0/7013a0 leave their leading turn aliases empty before249400/243cd0, so6f4a50 cannot activate a turn for that supported path. Main's scalar owner retains an unbound optional move-state input; it does not invent the future managed validation or claim the whole13-mode lifecycle Start is complete. That separate component integration remains on its actual gate.

Prediction: underwater empty hands become visible, moving uses the already acquired directional clips, and resting selects each authored idle clip with its declared fades. Surface/exit behavior and existing mantle/catfall remain unchanged. New clip leases use existing session/map release and reuse; no extra scene graph, texture, geometry, GPU pass, per-frame source read or extra idle lookup map is introduced. At full idle weight, skip sampling lower poses that the first blend child would completely overwrite, retaining every update/clock operation; this predicts fewer pose samples, with no CPU timing claim. F8-only diagnostics record actual water contact/eye flag, arm visibility, source/target stance, idle clip/weight/clock, published joint CRC and first matrix. Those diagnostics allocate/format only during capture. Source-complete batch now needs compilation and production-client validation; no tests or visual inspection are used.


Journal and READ_NOTE integration on the working player host (2026-11-09): `app/journal_movie.rs` owns each open Journal movie; the existing `app/pause_menu` worker now hosts Journal and readable movies, input, paced frames, notifications and unload. `state_movie` resolves the world-selected GameLogicState's inherited `m_iggyInfo` through the existing declaration/Catalog owner. Iggy, bitmap, font and submitted-frame leases keep their existing owners. Journal retains its movie while its note/map subscreen is open and resumes that same instance through the source subscreen callback (78c830/78d290). Close, failure and level change release those movies; epoch checks prevent stale presentations and menu sounds from crossing levels. Status: source port plus `host:` the existing Bevy worker and scalar player are the actual implementations for this feature slice, without the separate unfinished managed class-32/equipment startup migration.

The sole existing `app/interactions::focus_and_interact` producer now acquires an inventory-item lease through `inventory_item::PickableSources` (0607f0 identity sharing), requests the current existing player mode's real READ_NOTE state, and waits for its activation result before inventory admission, portable-pickup retirement or UI presentation. READ_NOTE's secondary/empty-mask constructor is 844d60 ->853c90; accepted activation is 844bf0 ->8450b0; descriptor release is 844e80. Its two request records use 681f00 constructor order and the single process DAT143994a30/14398e390 request/interaction UID owners shared with the canonical implementation. Journal helpers 8451b0/845160 replace the retained descriptor without another pickup or inventory admission. Navigation and Swimming retain separate actual mode instances. Nonempty modified body-part masks remain unsupported rather than bypassing code-lock admission. The existing authored GameLogic matrix/stack (787770/7886c0/7885b0/787510) coordinates INGAME, PAUSE, READ_NOTE, JOURNAL and MAP_TRANSITION with these same working owners.

General selection fixes in this slice: mapComponentDecl chooses its map declaration and discards sibling inherited/inline alternatives as normalization 00c710 does; inheritedDecl-only entries retain their real inline overlays. Sensor hits retain their actual material key so 3b8430 can skip transparent breakables before candidate admission, and the existing occlusion classifier accepts only the immediate attachment parent. READ_NOTE candidates use the current mode's state rules. The existing triangle collision/focus host remains the one producer; this slice does not claim the canonical managed sensor/body migration, Swarm queries, front-only/height constraints or general action states are ported onto that legacy host.

Coverage is the installed Journal notes/books/maps views and world READ_NOTE presentations, including portable versus read-in-place behavior and reopening admitted items. No active mission/progression, power eligibility or audiograph playing state is synthesized: no GameFlow resource is inserted without a real producer, Journal leaves that section's authored movie defaults when its provider is absent, and purchase/play callbacks explicitly report their missing owners. Those remaining Journal sections are `blocked: connect actual GameFlow event, power-purchase and audiograph-completion producers`; their declaration/presentation readers do not establish live state. The selected worldspawn must supply a resolvable GameLogic manager; inheritance-only worldspawn selection remains an explicit acquisition error.

Expected resource behavior: item declarations are resolved on acquisition, active Journal items retain one shared lease, cursor work is submitted only when its position changes, elapsed movie time survives worker handoffs, and unchanged paced draw lists reuse the existing cache. These are source-level operation/lifetime statements, not measured CPU/GPU/RAM improvements. All changed Rust sources and staged snapshots are parser-checked and the diff is whitespace-checked; no tests, separate compilation, visual inspection, launch or game input were performed. Coordinated compilation and production-client document/Journal validation remain required before calling this slice accepted.


Journal integration review repairs (2026-11-09): the first coordinated check found two feature-owned compilation defects: the power metadata reader's existing entity-components helper dependency was missing, and the modal tick attempted to take elapsed time while borrowing its sender. The actual inherited entity-slot/prefix resolver is now reused as a small helper, and elapsed time is taken before dispatch. No powers runtime eligibility is inferred from that declaration reader. The third check error belongs to the separately coordinated archive header initializer repair.

The review also corrected attachment depth: 3b7b90 accepts the hit entity or one 877530 result, where 877530 reads entity+358's retained +3c8 parent; it neither accepts a grandparent nor walks all ancestors. Shared movie handling now drains the same stored-AS3-exception reporter as Pause on Journal/notes construction, frames and dispatched operations, including before movie closure. Unknown ExternalInterface names produce diagnostics rather than disappearing from either notes or Journal notification queues. Direct missing-native/configuration/input/tick errors still pass through the worker's existing failure event and UI/state cleanup; stored AS3 exceptions retain Iggy's report-and-continue behavior (14102a680), without claiming a successfully rendered first open before production verification.

Item retention is now owned by the existing admitted inventory/history entries. Source63c630 writes the declaration pointer at the inventory record's first word;6144e0 visits that same stable +38 record buffer, and6170b0 uses its retained declaration across quantity changes. Journal520af0/7240d0 resolve history through the same item manager. DocumentEntry and JournalItem therefore retain their acquired Arc<PickableItem>, including while the history entry remains at zero quantity. Each Journal snapshot clones those same leases; the movie reads them directly and releases its clones on close. Its additional per-open item map/acquisition path was removed, while PickableSources remains the sole weak identity index and retires dead keys during off-thread acquisition. Prediction: each repeated Journal open performs zero inventory-item declaration reads/parses for retained entries; movie/resource reads retain their existing owners and policies. This is a source-level prediction, not a measured performance result. Parser and staged whitespace checks pass; coordinated compilation and actual production measurement are still pending, with no tests, separate builds, visual inspection or game input performed.


Animation header/declaration integration on the working host (2026-11-09): port the checked Catalog-owned MD6 declarations, shared immutable token layers, actual manifest header selection and bootstrap/map retirement from the resource branch, without its unfinished managed gameplay initialization. Original sources: 0607f0/05feb0/05fc00 lookup/registration,0661e0 preload,060b60 bootstrap promotion, 049f30/04a6c0/062bd0/0410f0 restart/purge,1c7c80 declaration inheritance. Expected outcome: live consumers of one MD6 declaration share one parsed generation; existing model/clip fingerprints and sampling stay identical; bootstrap headers persist, same-map persistent headers reuse, and different-map headers/read history retire after outgoing consumers. No byte or CPU saving is asserted before production measurement. Host: unused payloads remain demand-loaded, and deferred forced aliases retain their real preload phase. All71 installed manifests and562,647 global-layer rows were audited from installed data. The coherent integration batch requires compilation and production transition validation. Existing test bodies are preserved without editing or running tests; no visual inspection is used.

The working-host adapters preserve the real optional skeleton binding: shadow/arm callers use the existing compiled-model fallback when no authored/inherited name exists, while machinery needing an actual skeleton rejects its absence. Debug allocation reports now distinguish requested payload bytes from bytes requested from System including the existing tag prefix; they do not claim System heap commitment. Tracking remains feature-gated and adds no counters to the normal release. The native parent chain is no longer silently truncated at a host-selected depth16; actual cycle detection remains (1c8cc0).


Inventory loadout/collection resource ownership (2026-11-09): the working client read one default loadout and two inherited collection files, then discarded declaration identity and retained separate projected values. The existing Catalog-owned PickableSources now hosts both configuration classes, with lazy headers, immutable typed generations, parent leases, load reuse, scoped purge and session release. Constructors613ae0/613b60 allocate0x80-byte declarations; collection type defaults0, loadout m_clear=false/m_allowAutoEquip=true. Common item construction6135c0 supplies nullable reference/count1; collection enumeration6136e0 uses the first occurrence, including nonpositive exclusions. The owner preserves authored slot order, inheritance/overrides, signed counts and source flags; ReadableLimits retains that actual collection Arc instead of an unordered filtered copy. The existing initial-inventory host still consumes the selected loadout once. This does not replace its gameplay component or create another inventory/UI path.

Lifecycle evidence: primary vtables141b9a2b8 (collection) and141b9a418 (loadout) share declaration load011a20 and list destructor613930 ->022c10; construction joins the existing resource record through05f6a0. Registration uses the existing0661e0 filtered primary/reuse header pass, then040490/060b60 bootstrap promotion. Neither inventory class is in the36 heap-1 promoted kinds at142280ee0. Map headers therefore retain Level0; init/common headers become Permanent2 only at the actual bootstrap promotion, and implicit inherited definitions use the acquiring declaration's real record scope. Level purge060180 releases matching payloads together, then retires headers/read paths only after live current/previous generations and parent leases release; Catalog destruction releases the remaining class residency. Host: typed payloads remain demand-loaded instead of eagerly parsing every unused header; no GPU/VRAM/upload owner is needed for these declaration-only classes. General cross-class creation contexts not supplied by a working caller are not invented.

Inventoryitem read-history retirement is part of that same owner. Each successful generic/typed item parse records its exact normalized root and inherited paths with a weak read-generation lease. Typed PickableItem, admitted inventory/history/movie Arcs, and temporary generic ItemFields projections retain that lease. Editable requests still bypass reuse, but every old/new live generation remains counted. At level/session purge the owner forgets only dead paths absent from all surviving generations; the class mutex also prevents retirement during its in-flight reads. Map/power metadata readers now acquire generic fields through that same owner. Generic declaration trees are not retained in every typed item, and no second payload cache is introduced. Catalog namespace retirement remains authoritative when the session itself ends.

Prediction for coordinated validation: noninventory header requests make zero inventory path allocations/lookups because dispatch gates on resource.kind first. Repeated immutable collection/loadout acquisitions reuse one declaration identity and perform zero source reads/parses while resident; repeated Journal opens continue to use retained item Arcs directly. Initial loading still needs the actual selected1+2 files when cold. Setup acquisitions perform no scan of all live item generations; read-history cleanup is linear in tracked generations/paths at actual purge boundaries, with no per-frame work. New resource headers/full typed collection storage are explicit ownership overhead, so no RAM/CPU/GPU saving is claimed without measurements. Existing DH2_DEBUG_PICKABLE_RESOURCES reports configuration acquisition/reuse/release/purge and item generation/path retirement; debug allocation attribution uses inventory-configuration-owner. Evidence: artifacts/inventory-loadout-collection-installed-audit.json (83 loadouts/34 collections, no read failures), inventory-resource-preload-source.c and inventory-configuration-lifecycle-evidence.json. Unsupported malformed-declaration/default-file fallback remains an explicit error rather than a fabricated default. Coordinated compilation and production scope/read-count validation are pending; no tests, independent builds, visuals or game input were used.

Header admission completion (2026-11-09): the first combined client compilation
at d211b08b passed, but source review found that five upstream manifest stages
still selected only animation kinds. Both primary HeaderSelection construction
and reuse-only index selection now use one preload-declaration predicate,
including inventory collections/loadouts. This is necessary for the actual
common bootstrap promotion; a full primary resource list alone did not publish
their headers. The installed-data audit found 112 collection and 453 loadout
header occurrences in 30 primary manifests (117 unique configuration paths),
plus 187 collection and 286 loadout occurrences in 33 active reuse passes,
with zero read/selection errors. Common contributes three headers of each
class; Edge of the World low-chaos primary contributes 4/19 and its active
reuse pass 7/12. Original 0661e0 applies the same class dispatch in both passes.
This adds required header metadata, not speculative payload loading or a
claimed memory saving. Evidence: artifacts/inventory-header-selection-fix/
installed-audit.json. Runtime scope acceptance remains a distinct requirement.

CPU preparation observation (2026-11-09): the game-feature tools command
`inspect-cpu-preparation` invokes the client's actual level preparation worker
and player-body setup, drops their unpublished CPU bundles, then invokes the
same Catalog purge as LevelLifetime after its render acknowledgement. The
shared purge preserves the existing call order; it adds no per-frame work or
new ownership/cache policy. No window, renderer, audio playback or settings
writes are started. Each requested batch includes initial inventory rather
than manufacturing a restored session. Existing acquisition identities,
source-read and temporary-state capacity diagnostics can therefore observe
these CPU owners without replacing the user's open game. This is not GPU,
render-publication, inventory-carryover or gameplay acceptance. The command
reports actual production errors and counts; it supplies no invented expected
values and is not a test suite. No CPU/GPU timing or memory gain is claimed by
this observation entry point.

MD6 parsed-payload residency (2026-11-09): the registered record previously
held only a Weak generation. Dropping the last host Rig could therefore
discard successfully loaded metadata before 060180, while non-loading
193500/198940/060630 lookup still found the admitted header; peek_model then
reported an evicted loaded declaration instead of returning its real fields.
The existing Snapshot now retains its current loaded generation until the
record's exact purge marker is selected. Matching payloads release together
before dead-header checks, so inherited parent leases do not depend on map
iteration order. External generations remain safe and unused lazy headers
still acquire no payload. The added Option<Arc> is one pointer per record;
there is no second parsed copy or per-frame lookup. This is lifecycle parity,
not a claimed memory saving. The observed working CPU route loads three
Level0 Corvo model declarations per batch, so its cold-read count is expected
to remain unchanged. Permanent metadata, once genuinely loaded, must remain
available until session release. Production confirmation of the new release
ordering follows the coordinated build.

Decal host reference retirement (2026-11-09): GeometryLayouts is a
plugin-created resource replaced during scene preparation, so its sample
Mesh handles were outside the level-created resource list. The acknowledged
world reset now clears it alongside other scene publications. Dynamic
depth-box cube ownership now ends after the last actual dynamic depth-box
proxy, independently of a static receiver tree or surviving planar decals.
Static geometry and live/fading proxy leases remain with their existing
owners. The existing decal skin bind-group cache also drops old buffer
references when SkinUniforms replaces the pair, including frames with no
decal draws; ordinary current/previous swaps do not trigger a retain scan.
The extra pair comparison is bounded CPU work and is not a measured timing
gain. Prediction: no additional GPU work/uploads, outgoing layout samples
retain zero mesh handles after reset, and obsolete cube/skin references
retire at those actual owner boundaries. DH2_TRACE_DECALS records the
boundaries. These are bevy_backend lifetime repairs, not changes to decal
projection or original effect/tail rules. Evidence:
DH2-pause/artifacts/decal-lifetime-evidence.md; runtime bytes and affected
CPU cost remain to be observed after the complete batch builds.

Working-client CPU acquisition observations (2026-11-09): build33cfa392,
SHA256 f1f36a1d2dd05dd88fc54e8c80d4577e6b66074a94b95b05fefbf193de89652a,
completed Edge of the World -> Dust -> Edge of the World in both baseline and
optimized modes of the same binary, exit0. Every batch matched the prior
prediction: relevant body/listener reads6 ->4 and retained default-state
records80 ->14. Their recursively counted owned capacities were85,134 ->35,220
bytes, including the retained outer Vec allocation:49,914 fewer temporary
setup bytes per batch, not a gameplay-RSS or allocator-commitment claim.
All nine body/arm rig source and decoded fingerprints matched. No whole-CPU,
walking, GPU or appearance improvement is inferred from these counts.

Buildd74cb237 (SHA256 b278539ae09c38ef9d23392229decdf91d923a11d5344607d0e0adb8e2538a3c)
then completed the same three CPU loading/release batches, exit0, after the
metadata-residency, decal and quality source repairs. Each level purge kept
exactly51 common MD6 headers and six common inventory configuration headers;
the actual base collection reused its loaded identity while map-scoped Corvo
loadout/collection generations released and were reacquired. The three
Level0 Corvo model declarations still acquired cold in each batch, as
predicted. This observes CPU acquisition/purge only: unpublished bundles do
not exercise the renderer, quality transitions, active decal tails or live
inventory carryover. Those acceptance requirements remain separate.

Completed baseline evidence is preserved in the main checkout independently
of worktree cleanup: artifacts/resource-lifecycle-scope-20261109/, including
the comparison JSON, both complete logs, build provenance and installed
header-selection/source packets. Newer CPU run:
artifacts/cpu-resource-loading-scoped-20261109/ in the integration checkout.
No tests, visual inspection, game input or second game window were used.

Parameter-block lifecycle prediction before implementation (2026-11-09): the installed registry has 96 pblock declarations; previous EotW, Dust, Dust-return and EotW observations decoded 0/33, 0/37, 0/31 and 0/33 listed declarations, respectively. 0661e0's audited 36-kind promotion list contains pblock. For each selected unique pblock header, expect one Catalog record and zero payload reads at header registration; repeated rows and reuse/primary duplicates must retain one identity. Bootstrap records promote to marker 2 exactly once. On a persistent same-map reload, marker-1 records and any loaded payload survive; on a different map or disabled persistence marker 1 is purged, while marker 0 retires at level release. A live or in-flight lease keeps its generation valid across purge, and editable generations remain distinct. No per-frame work or new parser is expected. Exact selected-header counts, retained payload bytes, read reuse and production retirement remain unmeasured; no time or memory saving is claimed.

Parameter-block source follow-up: the existing particle/material acquisition API supplies no allocator-heap context, so those directly created headers retain a separate host-unspecified scope and are swept at level retirement; they are not labelled as native marker 0. Actual preload registration supplies the known creation context, explicit map preload promotion writes marker 1, and the one bootstrap promotion writes marker 2. No production acquisition/purge validation has been performed.

Inventory item lifecycle prediction before implementation (2026-11-09): 63c690 allocates the 0x1e0-byte arkDeclInventoryItem, calls 63c510 and joins the existing declaration record through 05f6a0; its vtable 141b9ea48 +8 reaches 63c660 -> 457e10, which retires its item-owned fields and arrays before 022c10. The common resource list acquires through 0607f0 and 060180 purges its matching +24 marker. The audited 36-kind heap-1 preload promotion list does not contain inventoryitem. Expect one existing-owner header per distinct selected item identity, zero payload reads from header registration, and bootstrap promotion to marker 2 once; newly created map headers retain their actual level context rather than an invented persistent promotion. Live item/Journal/inheritance read-generation leases must remain valid across a matching purge; editable generations remain distinct. Dead unleased map payload/header/read-history entries should retire at the level sweep. Selected-header counts, read reuse and retained bytes have not been measured; no memory or loading-time gain is claimed. Failed loads remain errors and retry until a source-backed failed-snapshot owner is implemented.

Inventory item owner follow-up: source-written registration now enters the existing selected primary/reuse manifest stages and the Catalog-owned item registry; nonpreloaded acquisitions retain a separate host-unspecified scope because the caller does not supply its native heap. The level purge drops only level/host-scope resident item generations, preserving live typed and generic read-generation leases and their inherited paths. Init/common headers are promoted at the existing once-per-Catalog bootstrap boundary. Failed loads and parses do not retain a native failed snapshot; later requests retry. A live generation surviving the final level sweep can retain its retired header/read history until a later owner sweep or Catalog destruction. This scoped patch still requires compilation and observed production acquisition and purge counts.

Inventory item failed-acquisition prediction before follow-up (2026-11-09): 05feb0 clears a lazy load before 05ede0 records its failure, and subsequent immutable 0607f0 requests reuse that admitted resource rather than repeating the same failed source load. Retain one shared error on the existing item header only when declaration reading, inheritance resolution, or syntax parsing fails. Two identical immutable requests in one scope should therefore cause one source attempt and one reused failure; a matching purge clears it and allows one new attempt. Editable sources must retry every request, and errors in host-only typed projections must never populate this shared source-failure slot. Preserve successful older item generations and their inherited read leases. No valid-asset read, memory, or timing gain is predicted; actual failure and purge counts require observation.
Inventory item failed-acquisition follow-up: the Catalog-owned item header now retains immutable declaration read/inheritance/syntax failures and returns the same failure on a later request; editable catalogs continue to reread. A successful source parse clears that slot before any typed presentation projection, so an unsupported host-only item/map/modifier projection cannot poison generic metadata. The matching level or session purge clears the cached failure while preserving older live successful generations and their read leases. This is source-written only; no failure injection, build, or production acquisition/purge observation has been performed.


DX12 placed-resource failure prediction (2026-11-09): successful buffer, texture, and acceleration-structure creation, accounting, and resource ownership remain unchanged. On a failed placed creation or failed naming operation, the already reserved gpu-allocator range should be returned exactly once, with any returned COM resource released before its backing range; counters are not subtracted because they have not yet been incremented. A committed resource whose naming fails should only release its COM object. No recurring work or retained allocation is added. Failed-allocation observations and driver behavior have not been measured.

DX12 source follow-up: the pinned wgpu-hal DX12 suballocation owner reserved a gpu-allocator 0.28.0 allocation before CreatePlacedResource, and three resource naming paths could fail before their counter increment. The gpu-allocator D3D12 Allocation has no Drop implementation that returns its reserved range; Allocator::free is the explicit release. The isolated patch rolls back creation errors and missing output pointers for buffer, texture, and acceleration structure; it also rolls back SetName errors for placed and committed resources. Returned COM pointers are dropped before freeing placed ranges. A rollback-free error is logged and the original creation/naming error is returned. Existing successful-path counters and normal resource destruction are unchanged. This is host-backend source repair, not evidence about original-game allocator ownership; no failure injection, build, or production observation was performed.

Accounted DX12 caller rollback: bind-group construction can fail after its internal sampler-index buffer has already been created and its buffer-memory counter charged, first at Map and then at descriptor upload. The isolated patch drops that buffer and returns its placed range on either failure, subtracting the already-added memory counter while leaving the not-yet-added bind-group counter untouched. The descriptor upload only returns an error from its initial slice allocation, before obtaining a slice. The original Map or upload error remains the returned error even if allocator rollback itself reports a failure. These source paths have not been failure-injected or observed in production.


Swimming shore-exit prediction (2026-11-09): for the supported living player leaving swimming normally, 88a8c0 -> 889f10 -> 88a7a0(4) -> 88a2e0 checks the existing player-physics ground-distance result (+2b0) against the negated standing posture's authored step height (+b0). If the distance is more negative, it sets the player position to the current entity position with Z reduced by ground distance plus that step height, before restoring the previous state-machine mode. 7ff0d0 -> 1b6b10 writes the existing actor/proxy position and does not replace its owner. The existing host switched modes without this correction. The isolated change ports that conditional correction on ordinary supported swim exit, using the already completed ground query and existing standing settings; the separate mantle transition is unchanged. Prediction: on an exit where that exact condition occurs, the position rises by the source amount before the next posture update; other exits have no position change. No claim is made that every reported shore collision has this cause. Native suppresses this branch in additional supported-gameplay cases whose owners the scalar host does not admit. No build, test, game input, or production observation has been performed.

Swimming exit qualification follow-up: 889f10 also adds the negation of a negative completed ground distance to the player's eye-height comparison when Controller+281 says the head is above water and Controller action 14 is held and allowed. The host already tracks this as ACTION_JUMP through the same held-and-allowed masks; the patch uses that input and the same ground result for the existing above-surface exit comparison. The shallow-water and no-contact tests are unchanged. This is a source-based prediction, not a demonstrated fix for a particular shoreline.


### Journal construction and readable focus repair (2026-11-09)

Release acceptance: main85690fa6 built both game and tools successfully with
unchanged committed source. Game SHA256cb2d17eb26bc02f17134122fe5de24341b1250652a59bd8268bab16b2156ffcf.
The existing installed-movie resource inspection admitted and drew
ui/journal.iggy with exit0, removing the reported lib_gamepadButton construction
failure. Closed VM diagnostics report tracked62245/live53472/remaining0, and
Catalog read-history namespaces end at0. Build and admission provenance are
main artifacts/reported-ui-swim-fixes-20261109-*. No gameplay input, image
inspection or new tests were used. Cursor restoration and shore-exit changes
are compiled; their human gameplay confirmation remains outstanding. The user
requested stopping after this batch; progression and ambient-animation WIP is
preserved in the canonical checkout/artifacts and is not claimed finished.

The production Journal request reached `JournalScene()` but failed with
`unknown class lib_gamepadButton` (sessions 1794196598384-6244 and
1794196445508-28856). Installed `ui/journal.iggy` has 11 direct imports;
`lib_common.iggy` imports `lib_controller_buttons.swf`, which supplies the
missing SymbolClass (character 138). The existing loader acquired only direct
imports. The shared loader now visits each authored import list in order,
acquires dependencies before executing dependent libraries, and registers each
resource once in the same player VM. Shared Iggy source leases, VM/library
retirement and bitmap acquisition remain their existing owners. All 55 installed
Iggy resources were inspected: no missing dependency paths or import cycles;
Journal needs 12 unique libraries plus its movie. Cycles report an explicit
admission error without recursion/depth limits. `141023b00` resolves the loaded
SymbolClass table; `14002e7a0` provides localized library paths.

Readable callbacks now use the existing worker: common `SetFlashFocus` /
`KillFlashFocus` (`140035020` / `140034fe0`) retain actual movie focus and update
controller type on acquisition. `140790a00` registers `NotesClosing` to
`140791370`, removing the existing UI FX during the authored closing animation;
`OnNotesClosed` still finishes the mode. FX messages retain world-epoch checks.
The host modal handoff saves each actual window's cursor capture and visibility,
then restores that exact state on close or failure; it previously released
capture on entry and only made the cursor visible on exit. Nested Journal/note
transitions retain the same handoff until the last modal closes.

Prediction: Journal construction resolves the authored button class, without a
special class stub or additional cache. A readable close returns mouse capture
to its pre-open value, and these three supported callbacks no longer report as
unknown. Rust source parsing and patch applicability are checked; no build,
test, client input or visual inspection was performed. Production admission
must be checked with the existing `inspect-ui-fonts ui/journal.iggy` tool route
(real `Player::load_installed`) after the coordinated build, followed by actual
human interaction before claiming the report resolved.

### Journal focus and mantle corrections (2026-11-09)

Journal keeps its existing per-open Player/library/image owner. The installed
JournalScene._onOpened sets UIScene.focused only after the opening transition;
UIScene.focused creates the cursor and sends SetFlashFocus/ KillFlashFocus
(035020 / 034fe0). Journal now consumes those callbacks and gates keyboard,
cursor, button and wheel delivery like READ_NOTE. Prediction: cursor input
during the opening transition no longer dereferences a null cursor handler or
drops the movie; normal input begins when the authored movie takes focus.
No resource cache, extra tick, image decoding or allocation is introduced.
Evidence: installed ui/journal.iggy JournalScene and UIScene bytecode retained
in artifacts/journal-open-script.txt and journal-scene-input.txt.

Mantle target height restores the correction accepted on the earlier branch
(fdc800c9): 3bcb00 adds the hit entity's positive vertical displacement, not
the player's jump displacement. The current GameCollision player host models
its collision bodies as static, hence this displacement is zero; moving-body
velocity ownership remains outside that host. Prediction: the hand boxes stay
on the ledge during upward player motion and select the authored hand tracks.
No collision or animation owner is added.

The missing surface-swim branch of 3bb790 now supplies water-plane Z minus
standing step height as the mantle query origin; 3bb860 also starts its 13-ray
ladder at zero instead of the ordinary minimum height. Submerged and ordinary
queries retain the physics origin. Controller+2a0 is swimming-subobject+48
(88b2f0 writes the water plane); physics+b0 is standing step height. Both query
issuance/results and mantle height classification use this same origin. In
the reported pier capture the physical origin is about -1.406 m and the top
is 1.250 m, beyond the 2 m search. A 0.122 m water plane and 0.3 m standing
step put the search origin at -0.178 m and the ledge at 1.428 m above it.
Prediction: the ledge enters the existing search and SwimMantle admission,
without increasing search distance, adding casts, changing surface-swim arm
visibility or changing level collision. Existing fixed-size sensor ownership
and level teardown remain unchanged. Compilation is recorded with the build;
these source-backed predictions are not a new human appearance acceptance.

### Shared skeleton retirement race (2026-11-09)

Prediction before the correction: EdgeSkeleton's final release currently
checks the header Arc count and name-table entry, but not the header's
payload state. An old payload destructor can pause while another thread
reuses that header, finishes decoding its replacement, and returns it. The
old destructor then removes the live replacement's lookup, allowing duplicate
reads/decodes and breaking the one-live-identity guarantee of 0607f0/05feb0.
060180 already serializes source registry removal with lookup. The Rust host
will additionally inspect the existing entry state under the registry lock:
only an unleased Ready generation may be removed; Loading and live Ready
generations remain. A busy state mutex leaves cleanup to the existing level
sweep rather than blocking/reentering a destructor. Expected result: a final
release cannot unregister a concurrently acquired replacement; ordinary
acquisition, decoded bytes, GPU upload, query/animation paths and registry
storage sizes are unchanged. This is a lifetime correctness correction, with
no claimed CPU or memory saving and no additional per-frame work. Original
registry evidence is the existing 0607f0/24f2a0 ownership port and 060180 source
capture artifacts/ghidra-tool-output/f30f285a83aa769ac4a62c81.json in main.

### D3D12 immutable sampler-index ownership: prediction (2026-11-09)

Completed main6b15daea EotW quality A/B/C/A/B/C/A capture exits0. Live
allocator payload remains around3.28GB at full settings, but reservation grows
4,064,280,576 ->5,138,022,400 bytes. All heaps contain live allocations: zero
empty warm-block bytes at every snapshot. Sample0 has1,698 internal sampler
index buffers /111,280,128 allocation bytes; sample6 has1,664 /109,051,904.
These immutable StructuredBuffer<uint> values are wgpu's D3D12 sampler-heap
indirection, absent from the game's direct D3D11 binding model; this is an
explicit backend ownership improvement, not a game resource format change.

Before implementation: share identical index sequences in a device-owned
weak lookup. Each binding group retains the actual buffer lease, so only
the last GPU-retired group frees it; error rollback frees only an unshared
new buffer. Pending/in-flight groups retain their leases. No sampler,
descriptor, shader, image, mip, pass or draw is changed. Expected unique live
sequences are a small fraction of the current1,698 group allocations (about
10–64,97–99% fewer buffers); exact unique sequence counts will replace that
estimate. All other allocations should remain comparable. Isolated
sampler-buffer acquisition CPU cost should fall because repeat groups skip
resource allocation, mapping and copy. The weak lookup is pruned on misses
and dies with its device. Predict no resource payload held after its last
group retires. This does not predict eliminating the separate device-local
texture-heap fragmentation visible in the same capture.

Use DH2_DEBUG_SEPARATE_SAMPLER_INDEX_BUFFERS=1 for the previous behavior in
the same release executable, and DH2_DEBUG_SAMPLER_INDEX_BUFFERS=1 for
acquisition-only timing/count records. Retain measurements before claiming
a CPU/VRAM reduction. Baseline is main artifacts/resource-quality-blocks-
20261109/{result.json,stderr.log,blocks.json}; original whole-device EotW
baseline remains tests/fixtures/original_eotw_budget.txt, not a matching
per-buffer budget.

### Retained declaration-family audit and allocator result (2026-11-09)

The saved147-consumed-kind audit was compared with current consumers, not
with the old ledger's unimplemented labels. Its81 missing-owner rows were
77 playerState variants plus damage and three inventory declaration classes.
Inventory now has its real scoped class owner. Player state and drowning
configuration reads are temporary setup data, followed by typed Player/
StateMachine/scene ownership; they are not absent implementations merely
because their generic source is discarded. Correct the stale damage, player
state, HUD and game-logic rows to their actual owner and supported scope. This
is a source ownership audit, not a claim that unsupported mechanics exist or
that all native resource classes are complete.

CPU preparation through sourcea7cbcf74, with debug allocation attribution,
completed EotW/Dust/EotW and exited0. Inventory-item-owner payload bytes after
each level purge are243,941 /243,941 /243,941, then0 after Catalog destruction;
900 permanent bootstrap item headers are removed at that destruction. Each
level purge reports skeleton registry entries0 (retained table capacity7, no
payload leases). The final parameter-block release reports34 records and0
live generations. Earlier configuration identities/scopes are preserved in
artifacts/inventory-lifetime-observations-20261109.json; current allocation
observations are artifacts/cpu-owner-heap-20261109/{result.json,stderr.log}.
The original EotW RESOURCE tag is51,830,920 bytes across22,543 allocations;
it is not a matching item-only byte budget. These figures establish bounded
observed ownership and session release, without claiming RSS or CPU gains.

### Particle declaration failure lifetime (2026-11-09)

Prediction before the correction: ParticleResources retains Result entries,
but retire_unused currently deletes every Err, causing the same immutable
failed declaration to be read/parsed again after cache retirement. Its catalog
reader also turns an actual IO failure into an absent-file error. Preserve
the existing error generation until this level-scoped owner drops, and pass
through the real path-qualified IO error using the same adapter already used
by WorldSources. Native0607f0/05feb0 return the existing registered resource
identity instead of reacquiring its source; capture8774b94849d1af0a137e06d5.json
in main retains this path. Expected result: repeated requests for a failed
identity reuse its original error across retirement, with no successful-path
read/parse/upload changes, no new cache, and no per-frame work. Error metadata
is bounded by actually requested declarations and drops with the existing
ParticleResources level owner. No CPU/byte saving is claimed on healthy content.

Particle owner mutable-generation qualification: unlike immutable installed
files, configured overlay sources can change while older effects retain their
leases. Before the change, the particle declaration/model/field/material
lookup reused those old decoded generations unconditionally. Follow the
existing editable-source policy used by MD6/items/resident images: bypass
these acquisition caches for overlay-enabled catalogs, without changing live
emitters; image lookup bypasses only an actually overlaid image path. Expected
result: subsequent acquisitions see fresh overlay source and a corrected
previous error, while existing consumers retain their exact old generation.
Normal installed-only acquisition is unchanged. This is a host overlay policy,
not an assertion about the original editor's reload scheduling or a complete
scene-renderer hot-reload feature. It adds no work to existing frame updates;
overlay acquisitions intentionally perform fresh IO/parsing.

### Sampler-buffer comparison and consumed-class coverage (2026-11-09)

Releasea348ba7e, same executable with separate/shared controls, completed both
seven-step quality routes with exit0 and no validation/panic/allocation-leak
errors. Initial live sampler buffers fall1,693 ->30, with
110,952,448 ->1,966,080 allocation bytes:108,986,368 bytes (103.9375MiB)
removed. All other initial allocator bytes match exactly at3,178,823,680.
Later snapshots retain28–30 sampler buffers, matching the predicted10–64
range; sizes, indices, descriptor lengths and draw/shader code are unchanged.
Acquisition-only means over138,300 /140,776 actual calls fall5,250.144 ->
115.883ns (45.31x); medians3,000 ->100ns. Shared requests reuse140,270 times
and create506 buffers cumulatively; dead buffers are not pinned for reuse.
Instrumentation/log output is outside the measured acquisition interval.
This is not an FPS, render-pass GPU-time or process-RSS result.

Reserved heaps do not show a reliable saving: initial reservation is
4,064,280,576 bytes in both modes; final reservation is4,869,586,944 /
5,003,804,672, while live sampler allocations remain about104MiB lower.
Mixed live allocation fragmentation remains an independent optimization
opportunity; no heap-compaction improvement is inferred from this change.
Source and exact measurements: main artifacts/sampler-owner-comparison-
20261109.json and sampler-owner-{separate,shared}-20261109/{result.json,stderr.log}.

The fresh production coverage report contains148 actually read resource kinds,
including the game-logic manager absent from the old metadata audit. Each
now maps to its actual named owner in the ledger; the old missing-HUD,
missing-player-states and pause-only labels were stale. Unread indexed kinds
are not presented as implemented gameplay. Owner coverage is recorded in
artifacts/consumed-resource-owner-audit-20261109.json; it distinguishes this
coverage result from complete native parity of unimplemented systems.

### Animation preload override and deferred initialization (2026-11-09)

Prediction before implementation: source0661e0's selected-kind pass calls
0607f0, then explicitly writes resource+24=1 even if lookup returned an
existing record (resource-preload-return-flow-fixed.c). Current clip/AnimSet
registration supplies heap1 only to creation, leaving existing bootstrap
marker2 records permanent. Apply the separate explicit write to these two
classes, as the parameter-block owner already does. Expect matching marker1
records to release at a different-map/disabled-persistence sweep instead of
remaining permanent; same-map reuse remains intact. This is a purge-policy
correction, not a claimed byte/CPU saving before observation.

Host deferred payload construction must retain its original bootstrap phase
independently of the record's later marker. Keep that phase on AnimSet headers
and record an explicit map-preload override on clip snapshots. Deferred
bootstrap force-load promotion may mark a newly discovered child permanent,
but must not overwrite a later explicit map-preload marker1. Serialize that
check/write with the existing clip snapshot mutex. This adds no frame work,
no extra source reads or new cache, and preserves clip/rig content.

Final prior-batch validation: main304121ad compiled the normal game and tools
in release with unchanged clean source, then its CPU preparation route
EotW/Dust/EotW exited0. The three body/left-arm/right-arm rig signatures
remain9090571c /efe8e864 /f341fde2 at all three loads, and item/pblock
retirement completes. Exact build/observation evidence is in main artifacts/
lifecycle-audit-final-20261109-build-provenance.json and lifecycle-audit-final-
cpu-20261109/result.json. This closes compilation and ordinary load/unload
validation for the particle-failure/overlay-acquisition batch; it does not
claim fault-injection or a complete editor hot-reload appearance check.

Animation override validation: source30c2b19d compiled cleanly (game + debug
allocation tools) with unchanged source. Both EotW/Dust/EotW and EotW/EotW
CPU loading routes exited0. Different-map sweeps match3,969 /4,060 marker1
clip headers and remove38 resident pins each, with0 retained live records;
same-map reload performs no marker1 sweep. All three rig fingerprints remain
9090571c /efe8e864 /f341fde2. Actual decode/reuse counts match the prior
304121ad route: first load204 /2, subsequent maps38 /166. The sampled map
headers do not overlap existing marker2 clip headers, so the explicit
existing-header override itself is source-proven rather than exercised by
these two routes. No new RAM/CPU improvement is claimed. Detailed observations
are artifacts/animation-preload-override-observations-20261109.json.

### Particle catalog identities: prediction before acquisition observations (2026-11-09)

Original 1400607f0/14005feb0 reuse one class/path identity; 140060b60 promotes existing bootstrap headers, 1400661e0 explicitly writes marker 1 for its selected classes even on an existing identity, and 140060180 purges the selected marker. Both particledt and staticParticleModel belong to the captured 36-kind selection. The existing level table already deduplicates one setup batch, but independent tables and later level loads cannot reuse live decoded payloads. The new catalog owner carries these actual scopes, serializes cold read/publication against purge, preserves failed immutable loads until retirement, and leaves mutable overlays independent. Derived two-channel CPU field pixels are weakly shared while declarations own them; this adds no GPU image owner. Existing material/program/image and simulation ownership is unchanged.

Prediction: requests overlapping a retained identity perform no additional declaration/model read or decode, and immutable failures do not retry before purge. No per-frame lookup, GPU pass, draw or upload is added. Common/bootstrap payloads intentionally remain resident until catalog close; map marker 1 payloads are released on the next different-map preload and marker 0 payloads on outgoing level retirement. Live consumers retain their exact generation after either boundary. This may trade retained common bytes for avoided reacquisition; no RAM or CPU-time gain is claimed before the same-build acquisition comparison. The original recorded EotW TAG_PARTICLE total is 46,524,457 bytes; it includes more than these two CPU payload classes and is not a per-class budget.

The existing CPU preparation observation optionally feeds its already-prepared world FX graphs and program library into the real WorldFxSources loader, then drops them before the normal catalog purge. It records real request/read counts, graph activation-event fingerprints and tagged particle-definition residency. It does not instantiate simulation, GPU publication, audio playback, or inspect visuals. Build and production acquisition acceptance remain pending for this source batch. No tests were written or run.

Deferred bootstrap particle dependencies retain their producer phase independently of a later map marker, matching the already ported AnimSet child rule. A common declaration whose own marker was explicitly changed to 1 still produces its original bootstrap mesh acquisition phase; an explicitly map-promoted mesh is never changed back to marker 2 by deferred decoding. Descriptor suffixless names are resolved to the same full archive path as preload headers and read-history identities. These are ownership corrections, not additional asset conversions.

First acquisition comparison (6acdde3e) compiled both game/tools and exited 0 in both modes, but correctly showed **no reduction** in repeated reads (87/101/87/87 declarations, 26/12/26/26 meshes). This contradicted the reuse prediction. Root causes: the shared manifest reader still admitted only its previous classes to HeaderSelection, and suffixless descriptor names did not match full preload identities. Neither result is accepted as a performance gain or complete lifecycle. The admission predicate now includes both particle kinds, canonical identities are shared, and delayed bootstrap dependencies preserve their actual producer phase. A fresh same-build comparison is required for this corrected batch.

Particle scope acceptance (7630875a): coherent dev game/tools build exited 0 with unchanged source. Same-executable separate/shared acquisition observations over EotW -> Dust -> EotW -> EotW both exited 0. Declaration reads were 87/101/87/87 -> 87/101/87/0 (362 -> 275 total), meshes 26/12/26/26 -> 26/11/24/0 (90 -> 61), and fields 7/4/7/7 -> 7/4/7/0 (25 -> 18). Recorded request counts, effect/failure counts, activation-event hashes and existing graph errors matched at every step. Bootstrap selected 324 declaration and 144 mesh headers, not every unfiltered manifest resource. Changing maps released 53/65 outgoing decoded declaration pins and 24/10 mesh pins, with 281/276 declaration and 183/178 mesh headers removed; a same-map reload retained their identities without additional IO. Native-selected map headers override the common marker, so no cross-map declaration hit was expected for these actual requested paths; two reusable mesh identities survive in the common scope.

The measured cost is deliberate decoded-resource retention: tagged particle-definition bytes after outgoing CPU consumers drop are 1,383,405 / 1,449,674 / 1,388,895 / 1,388,895 versus 3,385 / 3,462 / 3,462 / 3,462 in separate mode. Both return to **0 at catalog close**. These are attributed live requested bytes, not RSS, complete TAG_PARTICLE accounting or a RAM-saving claim. About 1.3 MiB is retained to implement the actual map/common reuse policy. No per-function CPU-time gain is inferred from read/decode counts.

The compiled normal client also completed EotW -> Dust -> EotW -> EotW and exited 0 with graphics validation enabled, one persistent device/window, and no gameplay input or visual inspection. This exercises real CPU/GPU setup, FX release and outgoing publication acknowledgement. It does not prove a screenshot match. Evidence: artifacts/particle-owner-scopes-comparison-20261109.json and artifacts/particle-owner-scopes-client-20261109/metadata.json. No tests were written or run.

### Shader-index session retirement: prediction before implementation (2026-11-09)

Original cache DAT_142be6ee0 is constructed once by 140064e00 -> 1402c8d50, initialized by 1402ca800/1402ca9c0 and destroyed at engine exit (including its 1402c8eb0/1402c8ee0/1402c8f30 backing containers). The rewrite's static INDICES instead holds a strong Arc after the catalog session closes. Replace that process-wide retention with weak identity lookup plus a catalog-owned index lease. Program libraries retain their existing index Arc and independent file cursors. Closing the catalog must release its lease while any still-live program library remains valid.

Prediction: same-session map acquisitions still read the 733,409 serialized index bytes once and reuse one metadata identity; no new index read/parse, shader conversion, GPU upload or per-frame work. Tagged decoded index storage must fall to zero after catalog and its libraries close, versus remaining resident with the previous process-wide strong owner. This is a session-final release correction, not a frame-time or reduced in-game memory claim. The original recorded TAG_SHADERCACHE is 43,566,840 bytes for its broader cache, not this index subset. An isolated debug switch will retain the old strong-owner behavior for the same-build residency comparison.

Shader-index retirement acceptance (9f40a722): coherent dev game/tools build and both four-load CPU routes exited 0. Old-retention and session-release modes each parsed 270 program / 14,711 stage records from 733,409 serialized bytes once, reused one index identity for all four levels, and read zero index bytes on every later acquisition. Tagged decoded index storage was exactly 1,245,718 bytes through level retirement in both modes. After catalog and library shutdown it remained 1,245,718 bytes in the old-retention control and became **0** with the session owner. Particle acquisition counts and event fingerprints were identical. The result matches the prediction: explicit final release with unchanged in-session reuse. No in-game RAM, RSS, frame-time or CPU-speed gain is claimed. Evidence: artifacts/shader-retirement-comparison-20261109.json. No tests or visual checks were run.

### Kiscule declaration scope: prediction before implementation (2026-11-09)

The current FxDeclarations table shares graphs between instances within a level, but each new table reads/decodes the same declaration again. Kiscule is in 1400661e0's captured 36-class persistent selection and registered by 14138ca50; its identities use the same 1400607f0/14005feb0, 140060b60 and 140060180 phases now implemented for particle resources. Reuse that registry algorithm for typed immutable graphs, leaving instance variables, gates, queues and owner overrides separate. Root graphs embedded in map entities remain map-owned. Recursive declaration references must reserve local identities before following dependencies; deferred bootstrap dependencies must not overwrite explicit later map markers.

Prediction: a same-map reload reuses already acquired declaration identities without additional declaration reads/parses, including admitted immutable failures. Different-map purge releases marker 1 graph payloads, ordinary outgoing retirement releases marker 0, and catalog close releases the remaining graph cache. Editable overlays bypass sharing. The graph/event results and first-acquisition source count stay unchanged. Cost is only the decoded definitions retained for the real common/map scopes plus sparse registry headers; no GPU upload, simulation work, per-frame lock or latency is added. No in-game RAM, CPU-time or frame-rate improvement is claimed before measurement; retained bytes and reads must be reported separately.

Kiscule and particle owners now share the already exercised Registry<T> implementation in assets::resource_registry rather than copying its phase/purge algorithm. The particle algorithms are unchanged; only their purge diagnostic prefix becomes RESOURCE_PURGE. Kiscule dependency traversal completes outside the class registry lock, preserving recursive-declaration admission without holding a lock across recursive acquisition. A load-local visited set propagates a deferred bootstrap phase once per declaration, including identities reached earlier in another phase, without adding persistent runtime bookkeeping. Owner/session fingerprint and retained-byte observations are pending.

Kiscule scope acceptance (0b6e6c03): dev game/tools build and both EotW -> Dust -> EotW -> EotW CPU routes exited 0. Graph decode counts were 26/31/26/26 -> 26/30/25/0 (109 -> 81); request counts stayed 26/31/26/26. Particle reads, effect/error counts and activation-event fingerprints remained identical, covering the shared-registry refactor. Outgoing different-map purge released 25/30 decoded graph pins and removed 30/33 selected graph headers. Tagged graph definitions intentionally retained 1,575,062 / 2,060,510 / 1,575,606 / 1,575,606 bytes between levels versus 2,737 / 3,571 / 3,281 / 3,281 in separate mode; **both reached zero after catalog shutdown**. This is reuse with a measured retention cost, not a RAM-saving or CPU-speed claim.

Normal-client default sharing also completed all four loads and exited 0 with graphics validation. An earlier unattended run timed out because its window was minimized and render readiness had no usable view extent; the same executable in separate mode completed after Windows restoration, and default mode then completed in 30.425 seconds with the launcher requesting a normal window state. No gameplay input or visual inspection was used. The local launcher now propagates actual client failure/timeout instead of returning success for those outcomes. Evidence: artifacts/graph-scopes-comparison-20261109.json; graph-scopes-separate-client-window-20261109.json; graph-scopes-separate-client-20261109/metadata.json; graph-scopes-restored-client-20261109/metadata.json. No tests were written or run.

### FX parent acquisition and temporary declaration lifetime (2026-11-09)

Original FX registration is 1413630d0; its base parser 140022fd0 acquires the named parent through 1400607f0, copies its reflected typed state, then applies the child's edits. FX backing releases through 1401c5990. The world, screen and pause FX readers previously inspected only the local edit block; contact FX already used the inherited declaration resolver. The installed inventory contains 8,909 FX resources / 3,824,720 decoded bytes (all ASCII), with 7,095 line-start inheritance candidates: 7,090 contact-system entries and five other FX declarations. The inventory is lexical evidence of resources needing ancestry handling, not an execution oracle.

Prediction before implementation: all four consumers request FX fields through assets::fx::declaration. Inherited definitions acquire their ancestor sources and preserve inherited groups before child edits. Non-inherited definitions and recorded map graph/rig fingerprints should remain unchanged. Parent/source syntax lives only during acquisition; callers retain their existing typed definitions and Arc resources. No new resident generic field cache, GPU pass, per-frame lookup or unimplemented effect activation is introduced. Ordinary source reads may use the existing immutable common-text owner; editable overlays continue to bypass it. This is missing resource ancestry acquisition, not a CPU-performance or RAM-saving claim.

The shared partial-edit merger now removes inherited item[n] slots beyond an explicitly reduced num before applying child fields, matching reflected list resize 14098c270 (the existing executed-list fixture records 512 original resize cases). No tests or original probe were run for this change. This addresses serialized item-array shrink, without changing other field merging. Original acquisition/build observations are pending for this coherent batch.

The existing resource observation command now accepts explicitly named installed FX via inspect-fx-definitions, feeding the normal WorldFxSources acquisition owner without a scene or fabricated effect. It reports prepared group/family counts and exact acquisition failure causes. The old command's acquisition timer is captured before printing decal metadata, excluding diagnostic output IO from its reported load interval. This is an observation entry, not a test or gameplay override.

FX source-owner review: the generic one-shot component resolver clones cached trees even when no caller can reuse them. The FX owner instead moves the local edit block directly for ordinary declarations, and holds only the actual ancestry layers when inheritance exists. It merges those owned layers by moving field values into the result, reusing the shared list-resize rule; no generic field cache or second complete syntax tree is introduced. Native 140143bb0 is explicitly a length-zero check, so an empty inheritance name terminates the chain. This replaces the contact reader's one-shot scratch cache as well as fixing the missing ancestry in the other FX consumers. Build acceptance is still pending.

FX source staging is separately tagged in allocation-diagnostic builds. The normal CPU loading and explicitly named FX observations report remaining FX-source allocations after their real consumers have converted the fields, so retained declaration trees can be distinguished from typed FX data and deferred errors. This adds no tag tracking in the ordinary release build.

FX source acceptance (5f69b7fb): coherent game/tools dev build, four-load CPU acquisition and normal-client graphics-validation sequence all exited 0. The named installed acquisition observed all five inherited non-contact definitions without FX-definition failures: alarm bell, howling bolt, explosive bullet and upgraded spring razor each prepared one group; the inherited NPC grenade prepared two. These were actual installed declarations passed through the normal WorldFxSources owner, without activating effects or fabricating a scene. EotW / Dust / EotW / EotW particle read counts, effect/failure counts, graph activation fingerprints and body/arm rig fingerprints matched the preceding accepted graph batch. The normal client completed in 32.691 seconds; this dev elapsed time is not a performance claim.

The FX source allocation context retains 10,138 / 20,175 / 20,175 / 20,175 bytes in the CPU-only observation and 1,184 bytes at the named acquisition report, which still has its catalog alive. This context includes archive/read-history metadata charged during source reads; it is not a direct count of retained field trees. The implemented source path owns ancestry layers locally, moves their fields into the result, and returns no cache of generic trees; typed consumers release their source fields after conversion. No zero-context-byte or in-game memory-saving claim is made. Full typed FX catalog residency remains separately partial. Evidence: artifacts/fx-parent-acquisition-acceptance-20261109.json. No tests, gameplay input or visual inspection were performed.

### Owned declaration-text handoff: prediction before implementation (2026-11-09)

The existing declaration_text reader already preserves the signed-byte lexer behavior of 140135f70/140135ff0/140136028. Its caller in declaration_texts then allocates a separate String while still owning the archive Vec. Transfer that Vec into String for ordinary ASCII input; preserve the exact existing normalization path for non-ASCII, special control bytes and NUL termination. Shrink the archive's one-probe-byte reservation to the former retained String capacity. This changes host staging ownership, not declaration parsing, source formats or resource caching.

Prediction: ordinary ASCII handoffs remove their explicit second text allocation/copy, with the archive buffer usually staying at the same address even when shrunk. Retained string capacities and ordered declaration fingerprints must match the copy control. Shrink/reallocation cost may offset the saving, so compare only this handoff function's thread cycles over actual loading requests in the same release executable. No whole-frame, RSS, in-game resident-byte or zero-total-allocation claim is made. Preserve the change only if affected-function observations support the predicted benefit.

First handoff observation correction: source 0ca5c2ad built and both release CPU observations exited 0, but the route only reached 198 physics text handoffs / 13,745 bytes over four loads; the renderer-side contact acquisition was absent. The archive-to-string shrink moved the buffer in all 198 owned cases, contradicting the predicted usual in-place handoff. The measured 311,181 -> 281,502 total thread cycles from that small subset is not accepted as a class-wide performance result.

The owner now retains the existing archive allocation instead of forcing a shrink/reallocation. Prediction: ordinary ASCII handoffs reuse their buffer address, remove the second allocation/copy, preserve text length and fingerprints, and retain only the archive's existing one-byte spare capacity per retained source. This small measured-capacity tradeoff is explicit; no resident-memory saving is claimed. The CPU observer now optionally calls the exact contact declaration source owner with its real prepared physics-text seed, exercising the missing class volume. A fresh same-executable release comparison is required before acceptance.

Full handoff acceptance (60be7038): the frozen release Tools build and both actual contact/physics acquisition routes exited 0. Each mode performed 10,567 handoffs / 1,682,046 source bytes over EotW -> Dust -> EotW -> EotW. Owned mode reused the original buffer address for all 10,567 inputs. Handoff-only thread cycles fell 16,756,902 -> 10,700,296 (36.14%); body/arm fingerprints and ordered contact-text signatures matched (EotW 2940748952, Dust 762767404). This is one full same-executable comparison over thousands of real calls, not a frame-time/FPS claim.

Retained common string capacity changed 2,064,739 -> 2,073,703 bytes; active EotW strings 107,492 -> 107,874 and Dust strings 112,271 -> 112,660. The intentional retained-table cost is 9,346 bytes in EotW / 9,353 bytes in Dust, about 9.1 KiB; cumulative handoff capacity is not a resident total because earlier map sources retire. Normalization remains the previous function for non-ASCII/control/NUL inputs; the sampled contact/physics inputs all used ASCII. Evidence: artifacts/text-handoff-full-comparison-20261109.json. No tests, visuals or gameplay input were used.


### Sound-event declaration ownership (2026-11-09)

Prediction before measurement: one immutable typed declaration per Catalog/path should remove repeated reads/parses across placed components, listed-event preload, world FX and contact FX. All consumers retain Arc leases rather than cloning game-properties strings. No per-frame registry access is added. Map/header residency can retain metadata between loads: this is a reuse/startup change, not a predicted RAM reduction or playback-speed improvement. Compare only declaration acquisitions, parse counts/cycles and typed fingerprints, using the same executable with DH2_DEBUG_SEPARATE_SOUND_EVENTS=1 for the old separate-acquisition control.

Original registration 141352070, arkDeclSoundEvent constructor/destructor 1400b3750/1400b37b0 and metadata 1414fe270 use the common declaration identity path 1400607f0/14005feb0. Selected non-streamed headers, bootstrap promotion 140060b60, explicit map marker writes 1400661e0 and scoped retirement 140060180 now use the existing shared Registry implementation. Payload acquisition remains demand-driven (host scheduling), and mutable overlay sources bypass immutable reuse. Failures remain recorded until their owning scope retires. Typed data stays alive while actual consumers hold leases; Catalog teardown drops its final registry ownership. No Wwise voice, bank, media or streaming ownership changes.

Installed inventory: 3,444 soundevent resources, 665,746 decoded source bytes, zero inherited declarations (artifacts/sound-declaration-inventory-20261109.json). Existing inherited-edit parser remains available for changed sources; no speculative parent-resource cache was added. Native SOUND-tag fixture totals are not treated as this class's complete declaration budget. Production acquisition observations and tagged residency are pending.


### Mass-property scoped ownership (2026-11-09)

Prediction before measurement: keep the existing preparation-local fast lookup, with Catalog ownership underneath, to avoid decoding surviving common/same-map resources on repeated loads. Map-only principal frames release on normal level purge; a changed map retires marker1 while common/init marker2 survives. Payloads are tiny and header identities cost memory: this is lifecycle completion and expected repeated-loading CPU work reduction, not a RAM-saving claim. Compare acquisition-only cycles/counts and principal-frame fingerprints in the same executable using DH2_DEBUG_SEPARATE_MASS_RESOURCES=1.

Original resource registration141356de0, construction140089390, compiled loading140089690, lookup14008c7f0 and destructor140089320 use shared resource-list rules1400607f0/14005feb0. The installed common manifest names108 massprops; promotion table142280ee0 explicitly includes this class. Host demand decoding stores the existing principal-frame representation instead of a retained Havok source/schema; body overrides continue on copied values (140caf5c0). Missing sources remain distinguishable from real I/O/format errors. Mutable overlays bypass sharing. The old process-wide MassHistoryLease/count and blanket kind-history erasure are removed: exact Catalog/path identities now retire with their actual registry scope. Runtime bodies hold their copied mass/inertia, with no extra per-frame lookup or Arc traffic. Measurement pending.

Sound/mass owner diagnostics separately report requested registry record storage (reserved entries plus path/failure capacities) and live typed payload bytes (Arc header, inline type and owned nested strings), excluding hash-control bytes and allocator rounding. This exposes the cost of unused pre-registered headers rather than confusing allocation-tag context bytes with the complete class footprint. Reporting runs only at traced scope retirement; gameplay has no added work. The first dev sound observation covered placed-component acquisition only (214 requests, 214 versus151 decodes, identical ordered typed fingerprints); it did not exercise renderer-side listed/world/contact paths and is not a performance result. Broader coherent-source observations remain pending.

MassSources local hits copy the already decoded principal frame directly, preserving the old no-atomic fast path; cold acquisitions move their resource lease into the local table. Arc cloning on each local hit is unnecessary because consumers need a body-specific value copy. This final adjustment is queued for the next coherent build rather than modifying source underneath an active compilation.


Validation correction (2026-11-09): the resource-client-once helper supplied a comma-separated DH2_DEBUG_LEVEL_SEQUENCE, but loading_screen parses semicolons. Thus the c58653d6/c9d76c00 client observations, and f22fde56 integrated release observation using that helper's comma route, demonstrated initial EotW acquisition/rendering/shutdown only; exit0 did not prove cross-map transitions. The independently invoked CPU preparation routes did load all four requested levels and remain valid. The helper now normalizes route separators, records every actual LOAD session map, rejects an incomplete route even on exit0, and restores its own late-created/minimized window using Win32 events. A corrected c9d76c00 production-client route is in progress using the already compiled binary; no rebuild was needed for this validation-tool defect. Historical numerical CPU/resource observations are unchanged; no appearance comparison is claimed.

Installed-name scope: artifacts/sound-mass-manifest-inventory-20261109.json covers38 mapresources files (init/common plus36 levels) and33 reuse chunks, using their actual primary-header index namespaces. All3444 sound-event and6485 mass-property paths match their class owners' resource-path rules. Common selects1296 nonstreamed global sound headers and108 mass headers; init selects neither class. This verifies installed naming/admission data coverage, not that every level was played.


Sound/mass acceptance: source c9d76c00 compiled both the dev game/tools and optimized tools with clean, unchanged source. Release tool SHA256 ac13db9610180103ef6b05f9c31107d20e8665bd492ecf02a455a748d592a9e4. Four separate processes each loaded EotW/Dust/EotW/EotW through the real CPU preparation, world-FX and contact-text acquisition paths: separate owners, shared sound only, shared mass only, both shared. All exited0; every per-map path/typed-signature/success multiset matched in all modes. Counts match the ownership model: sound326 requests,326->233 decodes (90/56/90/90 ->89/55/89/0); mass57 requests,57->44 decodes (13/18/13/13 ->13/18/13/0). Same-map reload performs no additional decoding; changed-map purge releases the previous map's payloads.

Affected owner acquisition spans only, not whole CPU/frame time: sound21,394,288 thread cycles separate ->17,444,791 sound-only shared (18.46% fewer); mass4,201,155 separate ->3,149,930 mass-only shared (25.02% fewer). Both-shared run gave16,255,071 and2,834,965 respectively; those lower values are run variation, not an additional cross-class optimization claim. Caller-local mass hits are further kept copy-only (73e7c623); that does not change the measured owner functions. No walking/FPS improvement or total-load-time claim is made.

Storage tradeoff: these owners add preload identity records instead of dropping them after preparation. The shared CPU route's final EotW state requests473,687 record-storage bytes plus5,696 typed sound payload bytes, and277,448 record-storage bytes plus988 typed mass payload bytes. HashMap spare capacity is included; hash-control bytes and allocator rounding are excluded. Only89 sound/13 mass payloads are live in this CPU route. The initial real client prepared1708 sound payloads (130,583 bytes) with474,073 record-storage bytes, and13 mass payloads (988 bytes) with279,680 record-storage bytes. This is deliberate lifecycle reuse, not a RAM-reduction claim or a direct comparison with the original RESOURCE tag's51,830,920-byte whole-tag budget. Decode-context tags in the first dev CPU route return to0 after Catalog destruction; record storage is independently owned by the Catalog registry.

Evidence: artifacts/sound-mass-scopes-20261109-comparison.json and corresponding build provenance/client metadata. The first client passed initial acquisition/rendering/shutdown only. The corrected semicolon route was stopped after the user reported desktop disruption; it is explicitly not accepted as a completed cross-map client check. Its route verifier reports false even though the process exited0. Automatic visible diagnostic launches are now disabled; the forced window-restore/focus hook has been removed from every copy of this audit helper. The remaining audit uses windowless acquisition observations. No tests, image inspection or gameplay input were used.


### Auxiliary sound bus lifecycle (2026-11-09)

Prediction before measurement: each requested immutable soundauxbus should decode once per surviving resource scope instead of once per map. Production requests originate from acoustic-zone setup; send projection continues using its existing local copied-ID table without a Catalog lookup, lock or Arc increment during playback. Expected improvement is fewer repeated reads/parses on reload; the added header registry may retain more memory. No FPS or RAM-saving claim.

Original class registration141351fa0, constructor1400b4460 and destructor1400b44a0 use declaration lookup1400607f0/14005feb0. Its family is in the already audited142280ee0 promotion table. Selected headers, bootstrap promotion140060b60, map writes1400661e0, scoped purge140060180 and Catalog release use the existing Registry<T>. Installed inventory contains24 declarations/2642 source bytes with no inherited declarations; the shared inherited-field resolver handles changed sources without adding a syntax cache. Engine ID parsing remains the existing parser factored over resolved fields. Immutable failures are cached to scope retirement; editable overlays bypass sharing. Bank/mixer bus objects and voice/send state keep their existing separate ownership.

Diagnostics correction: Registry payload_bytes now includes trailing padding of the current host ArcInner block, not just16 header bytes plus size_of<T>. Previous mass payload figures76 bytes/entry omitted4 bytes of layout padding; corrected value is80 bytes/entry (1040 for13,1440 for18). They were structural estimates, not allocator-tag observations. This changes diagnostic accounting only, not allocation or runtime layout. New acquisition/storage observations are pending.


### Sound RTPC lifecycle (2026-11-09)

Prediction before measurement: route currently supported engine-initialization RTPC declarations through their class owner rather than direct Catalog reads. Native class registration1413520e0, constructor1400b4510/destructor1400b4560 and the common registry establish header/payload lifetime;142280ee0 includes soundrtpc in conditional map promotion. Consumers keep existing copied IDs and the physical-scale range, so playback gets no registry lookup, lock, reference-count traffic or changed clamp. The immutable owner records failures until scoped purge; overlays bypass reuse.

RtpcDeclaration::parse now parses syntax once and delegates to from_fields; the previous physical-scale parser parsed the same source twice just to obtain its ID. Numeric defaults, finite/reversed-range validation and clamp1400b45c0 remain unchanged. Initialization already runs only once per AudioMix lifetime, so no repeated-map CPU gain or RAM saving is predicted for the current caller. The purpose is whole-class lifecycle admission/release and one canonical acquisition path. All installed RTPC source ranges and actual initialization inputs still need to be checked before integration; no unverified feature is merged into main. This source is in the separate working checkout while the auxiliary owner build remains frozen.

Installed RTPC inventory:14 declarations/1952 source bytes, no inheritance, finite ordered ranges throughout; all7 actual initialization references exist. Data inventory is artifacts/sound-rtpc-inventory-20261109.json. EngineControls now owns the supported default soundengineinternals acquisition and conversion to copied control IDs/ranges; AudioMix retains the result across maps and source syntax drops after acquisition. There is no added source cache or second initialization. Backend cvar bindings stay in app::audio. The CPU preparation observer can call the exact same helper once per session after real preload (DH2_DEBUG_CPU_AUDIO_CONTROLS), preserving the caller's lifecycle without creating a window or audio output. This is not a fabricated per-map repeated-initialization benchmark. Source checking, native-range/data fingerprints and ownership observations remain pending before integration.

Auxiliary acceptance:7285e29d built the dev game/tools with unchanged clean source; optimized performance is not inferred from this build. Actual EotW/Dust/EotW/EotW CPU preparation requested59 bus declarations in each mode, with59->44 decodes and no failures. Every path/ID-signature multiset matched. This matches removal of the final same-map reload's15 decodes. Existing per-zone name deduplication and runtime copied-ID lookup stay unchanged. New registry storage is intentional reuse overhead, not a RAM reduction; no CPU-time/FPS or GPU claim is made from dev timing. Evidence: artifacts/sound-auxiliary-scopes-20261109-comparison.json and build provenance. No window, audio output, image inspection, gameplay input or tests were used.

RESOURCE_STORAGE catalog=1 kind=soundauxbus phase=scope-released records=15 record_storage_bytes=3101 payloads=15 payload_bytes=360

RESOURCE_STORAGE catalog=1 kind=soundauxbus phase=catalog-retiring records=15 record_storage_bytes=3101 payloads=15 payload_bytes=360


RTPC/control acceptance: cc2462e9 compiled the dev game/tools with clean unchanged source (tools SHA256 cda557ffd9e25fc55e875e3cb46ff1e8c5b7195a719654083ee85beb7c7c53e6). Both real four-level CPU acquisition routes exited0. Each initialized sound controls once, requested/decoded exactly7 RTPC declarations, and produced identical per-path typed fingerprints and copied control values. All7 IDs and the physical-scale default/min/max matched the installed default soundengineinternals and RTPC declarations, independently inventoried in artifacts/sound-rtpc-inventory-20261109.json. Physical-scale defaults remain1000/0/1000; no range/clamp behavior changed.

As predicted, existing AudioMix session reuse means no decode-count or repeated-map CPU gain:7 remains7. This is completion of resource identity/header/purge ownership, not an asserted optimization. All7 typed payloads remain with bootstrap records across map switches. The final CPU route requests2276 registry-storage bytes plus224 aligned live payload bytes; Catalog teardown releases its sole remaining Arc ownership, with no mutable playback state or reference cycle inside those Copy values. Evidence: artifacts/sound-rtpc-scopes-20261109-comparison.json and build provenance. This verifies supported control acquisition and CPU lifetimes, not rendered/audio-device behavior; no visible clients, gameplay input, appearance checks or tests were used.


### Floating render defaults: namespace/edit-generation correction (2026-11-09)

Prediction before correction: ProgramLibrary::parameter_value_metadata already bypasses its cache when the supplied Catalog differs or its declaration is editable, but registered_float_vector does not. Its name-only cache can return a previous namespace's value or a stale editable value/error. Apply the same identity/overlay admission to floating defaults, preserving immutable memoization and its original source parser. Expect no new reads/parses for ordinary immutable hits, and a fresh value/error for an editable source or another Catalog. No per-frame speed/RAM saving is predicted. Ordinary hits build no resource path or file query, and use one existing HashMap lookup; unrelated overlays retain reuse for the unchanged path.

Original render-parameter constructor/destructor1402d82c0/1402d8310 and parser1402d8820 establish declaration-owned data; ResolveCodeResources1400606f0 rebinds dirty code references through1400607f0. Shader startup14036f800 explicitly resolves the renderParm family. Existing native initial and EotW snapshots both contain1688 renderParm headers (artifacts/resource-residency-initial.json and resource-residency-eotw-verified.json in main); this is a count, not evidence of every header's purge marker. Installed inventory:1688 source resources/62570 bytes;1686 are ASCII-only. No speculative global parsed-tree cache or guessed persistent marker was added. The broader class owner remains unfinished; this change fixes the demonstrated local cache-ownership inconsistency. Source compilation and ordinary production acquisition observations remain pending; editable hot reload is not claimed as exercised.

Render-parameter owner prediction before implementation: the installed1688 declarations total only62570 source bytes, and both native boot and EotW snapshots contain1688 headers. Native14036f800/1400606f0 binds engine code references; these are defaults shared across worlds, not image mip payloads. Centralize actual source acquisition on the Catalog, coalesce immutable reads/errors, retain original bytes plus only requested lazy numeric metadata, and drop the namespace's ownership with Catalog. Host: explicit session sharing replaces engine code-reference ownership; no guessed native purge marker is assigned from header counts alone. Editable declarations bypass this source owner and the corrected local float cache; already held immutable leases remain valid. Expect fewer source reads across renderer/environment/sampler/default consumers; source and record storage increase deliberately. Keep existing format parsers/encodings and local fast lookup tables; add no per-frame source requests or GPU work. Measure real source-request/read counts and content fingerprints before accepting; no RAM/FPS claim is predicted.

Implementation: raw source bytes retain their installed representation; only requested numeric metadata/floating defaults are cached, with existing parsers and each caller's prior byte-to-text conversion preserved. No generic Value syntax tree or source/decoded image copy is retained. Path identity storage is shared between key and source lease through Arc<str>. No source node references its Catalog, so teardown has no strong cycle; callers may finish an already acquired lease independently. All production environment, type-schema, subtype-metadata, float-default, anisotropic-sampler and blur-default source reads route through the same owner. Existing per-program fast tables and map-specific override values remain separate. Acquisition trace reports source CRCs/read counts; storage trace distinguishes original source length/capacity and the actually initialized lazy metadata fields. Observations pending before accepting this source.

Lazy subtype/default storage is itself allocated only when a consumer asks for it. Source-only schema scans do not reserve two Result/String-sized parse cells in each of1688 nodes. The per-node OnceLock<Box<Parsed>> owns a small optional parse block; no extra runtime resource lookups or per-frame work are introduced. The owner reports its requested node/key/source/initialized-parse storage separately from allocator rounding and hash-control overhead, and the diagnostic allocation tag is checked after Catalog destruction.

Render-parameter observation accounting: the coherent e2888054 dev game/tools build passed. Its actual four-level CPU routes both exited0 and requested3404 sources with identical name/source-CRC/success multisets; reads fell3404->1688 with no failures. Shared source bytes/capacity stayed62570/64258 on every map, with1688 records and no numeric parse blocks initialized in this CPU route. The diagnostic owner context returned to0 after Catalog release. The original and new inspect-decal-materials production paths both exited0 and returned25 declarations,99 distinct materials, projection counts[0,74,0,25],52 FX materials and0 errors. These are source/data acquisition observations, not GPU/appearance or CPU-time claims.

The reported383506 storage bytes are a logical-capacity estimate, not an exact allocator measurement: HashMap::capacity reports usable entries and omits reserve buckets beyond its load factor. The final diagnostic label therefore becomes logical_storage_bytes, explicitly excluding hash spare/control buckets and allocator rounding; tag_context_bytes additionally reports the actual diagnostic allocation context, which can include archive read-history metadata allocated under the owner. No RAM saving is claimed: source reuse deliberately retains roughly this amount across levels. Source/parse data drop with Catalog; actual standalone decoder output equivalence beyond the listed aggregates is not claimed. The final diagnostic-label change will be compiled by the normal integration build; it does not change resource behavior.


### Autonomous motion scene ownership (2026-11-09)

Prediction before implementation: scene_motion::load currently caches one parsed MotionClip per path but deep-clones its groups/keys into each entity Graph; every Play action clones the entire clip again and then once per bound group. Expect one immutable clip allocation per successfully parsed path per map preparation, shared by graph and playback leases; unchanged curves, bindings, initial poses and playback state. No new session pin, read, conversion, frame lookup, GPU upload or streaming policy. Transient parse cache releases at load return; graph/playback leases release with AmbientMachinery on map retirement. Measure unique payloads and logical retained curve storage during actual CPU machinery initialization, and tag retirement after prepared owners drop. No CPU-speed claim from dev measurements.

Original provenance: arkDeclMidnightScene ctor140193a10 and declaration registry1400607f0 establish the source definition; keyframe parameter copy1405329c0 copies its declaration pointer at+8, rather than cloning the scene. Action140533460 compares that identity with the Midnight parameters and passes it to1407d6040 while storing a separate playback handle at+0x30. Reference-frame capture817060 and curve composition816710 stay unchanged. Host Arc leases replace the engine declaration pointer for supported autonomous motion-only clips. This does not claim the full cinematic runtime or original preload-marker parity.


### Player Midnight sound-event ownership follow-up (2026-11-09)

Audit correction: the earlier sound-event owner integration missed player Midnight prepare_sounds, which directly read declarations and parsed IDs once per scene. The music Definition parser also has a direct ID reader, but no production caller currently constructs that definition; it is not claimed as an integrated music lifecycle. Original SFX constructor140188470 and copy140188490 retain the declaration pointer at+8 (reflection1415c4ff0); sound-event registration141352070 and common lookup1400607f0 establish its shared identity.

Prediction before implementation: route production player scene sound resolution through assets::wwise::event_resources, preserving its existing per-scene ID table, cast filtering, key schedule and copied playback IDs. Repeated paths already present in that owner should require no additional source reads or declaration decodes; new paths acquire one immutable scoped generation. Original resources have no inherited sound-event sources in the installed inventory. No per-frame work, voice lifetime or bank/media change, and no CPU speed claim without release measurement. Compare actual requested reference/ID sequences in the windowless player preparation, with the former direct resolver selected in the same executable. Mutable overlays retain the owner's existing bypass. Existing pure format-reader callers remain available for offline inspection.


Motion owner first observation: dev game/tools5bc5c81d compiled from clean unchanged source; both actual four-load CPU routes exited0. EotW retained8 clip allocations/2040 logical payload bytes in separate mode versus1/599 shared; Dust6/1530 versus1/599. The prediction of one source allocation is met. This is a small1.4KiB/0.9KiB logical payload reduction, not a large RAM improvement. Both modes retain324/439/439/439 tag-context bytes after each map because Catalog read-history allocations are also attributed to that context. No leaked curves are inferred from those contextual totals. Initial combined fingerprints differed, including repeated unchanged EotW loads: the diagnostic included live performance-counter-seeded random playback rates. Correction separates immutable definition and initial-pose fingerprints from those rates, preserving actual RNG behavior instead of freezing gameplay inputs. Exact content/pose comparison and final Catalog retirement remain pending in the next coherent batch.


Motion and player-SFX acceptance (80d16b8f, 2026-11-09): coherent dev game/tools build completed from clean unchanged source; Tools SHA2561c349e5a51288ae568308d445d6fe96e50e1861d1817f991bc772e7a29ca3191. Both actual EotW/Dust/EotW/EotW CPU-loading routes exited0. All four immutable motion-definition and initial-pose fingerprints match between separate/shared modes; live randomized rates remain independent and are not frozen. Motion payload counts/storage remain8/2040 ->1/599 logical bytes in EotW and6/1530 ->1/599 in Dust. Source-tag contextual324/439 bytes remaining after map release also retire to0 at Catalog release in both modes. No GPU data, curve precision, sample scheduling or streaming policy changes.

Player scene acquisition requested the same120 ordered sound references/IDs in both modes. The former direct path parsed120 sources; canonical-owner acquisition decoded13 unique declarations and reused107, with zero failures. All13 source identities are present, and the sound-event allocation context returns to0 after Catalog release. This fixes a missing consumer route through the already measured scoped owner; it does not add a second sound cache or per-frame reference counting. Full inventory inspection found1026 distinct SFX-event references/7589 occurrences across all2137 installed Midnight resources, with zero missing event paths; that inventory does not claim support for every cinematic. Evidence: artifacts/motion-and-player-sound-20261109-comparison.json and midnight-sound-inventory-20261109.json. These dev observations prove content/count/retirement for this route, not CPU speed, FPS, GPU behavior or a gameplay appearance check. No tests, windows or game input were used.


### Blueprint preparation ownership (2026-11-09)

Prediction before implementation: expanded_model_placements currently reads/parses a blueprint for every placed instance, independently during machinery and render setup. Introduce one temporary source owner per expansion batch: demand-load each requested immutable declaration once, retain only typed child paths/local transforms for the batch, and drop those definitions after producing instance placements. Repeated immutable instances should perform zero additional archive reads/parses; mutable overlays bypass reuse, and missing/failed immutable results retain their existing per-instance failure outcome. No new session pin, gameplay lookup, GPU upload, reduced precision or changed transform algorithm. Acquisitions may still repeat between independent preparation batches; no cross-batch benefit is predicted. If actual production loads show no reuse, remove the cache instead of retaining unmeasured overhead. Compare exact expanded-placement signatures with separate acquisition in one executable; dev counts do not establish CPU speed.

Original provenance: blueprint registration1413517a0 names arkDeclAssemblyModel; constructor1401c10d0 and destructor1401c1270 own its submodel array. Render assembly1401bc1f0 reads that existing definition's array at+0x58/count+0x60, resolves child model identities and builds per-instance transforms/runtime objects;1401bc3c0 retains the definition pointer separately from instance state. Host temporary typed ownership deliberately releases definitions after static instance cooking rather than retaining unused declaration syntax through gameplay. The existing affine expansion, validation and nesting admission remain unchanged. Full generic preload-header parity is not claimed.

Installed blueprint inventory:49 declarations/161070 source bytes, zero inherited declarations and zero read failures (artifacts/blueprint-inventory-20261109.json). Both model-audit acquisition routes now use the same owner; the pure expand_blueprint source-reader remains a thin parser/expansion adapter for existing format callers. No tests were written or run.


### Audit allocator label lifetime correction (2026-11-09)

Prediction before implementation: load_timing::mark currently Box::leak's a freshly formatted label on every call, including repeated labels and sound/streaming labels containing variable counts/durations. The fixed64-slot registry silently sends excess labels to untagged while live_bytes reports an unregistered name as0. This is host diagnostic bookkeeping, not an original game mechanic. Intern one label per stable phase category, keep detailed variable counts only in the existing log line, and use the full u8 index range already stored in the allocation prefix. Repeated map loads should create no new phase-name allocations after those categories are first registered. Unregistered measurements should returnNone, preventing absent tracking from being mistaken for verified zero bytes. Runtime resource acquisition, allocator System behavior and non-diagnostic rendering remain unchanged. No CPU timing improvement is claimed.


Blueprint ownership acceptance (20de65fe, 2026-11-09): coherent dev game, Tools and model-audit build completed from clean unchanged source; Tools SHA2562706973c00b42cbe127736c07a6d7ce32cd1a4b7a7f9c28964d917439f6cb36a. Both actual EotW/Dust/EotW/EotW CPU preparation routes exited0. All227 ordered expanded-placement signatures match between separate/shared acquisition. Batch reads71/14/71/71 ->13/10/13/13 (227 ->49), meeting the predicted per-batch reuse; no cross-batch saving is claimed. Typed definitions temporarily occupy15992 logical payload bytes in EotW and12136 in Dust, then retire at expansion return. Source-tag contextual1813/2561/2561/2561 bytes match both modes after map release and return to0 at Catalog release. Those residual contextual allocations are read history, not retained Blueprint definitions. The shared/separate comparison uses the same refactored parser/affine implementation; the original expressions and field validation were moved unchanged, but no independent visual/GPU or original-executable transform comparison is claimed here. Existing pure source-reader fixtures were preserved without writing or running tests. No CPU-speed, FPS or resident-RAM gain is inferred from dev timings. Evidence: artifacts/blueprint-owner-20261109-comparison.json.


Audit-label acceptance (403835d0, 2026-11-09): coherent dev game/Tools compilation completed from clean unchanged source; Tools SHA25657226298d1346eb1156012c06e69795f27da79ff262debdc3c8d7051c3cab409. The actual EotW/Dust/EotW/EotW windowless CPU route exited0 and retained exactly6 interned phase labels/108 payload bytes after each load. This confirms bounded reuse for the categories exercised by that route; variable sound/streaming detail remains text-only by the static-category API and was not exercised with audio/GPU playback. Original source-byte prefixes support the full256 u8 indices without an allocator-layout change. All227 blueprint expansion signatures,4 motion definition/initial-pose pairs and120 ordered player-SFX references/IDs match the prior accepted acquisitions. Registered blueprint, motion, render-parameter, mass, sound-event, FX-graph, shader-index and item memory contexts return to0 at Catalog release. Unregistered contexts now remain unobserved(None), so absence no longer supplies false zero evidence. No tests, visual checks or visible clients were used. Evidence: artifacts/audit-phase-labels-20261109-observation.json and audit-phase-labels-20261109-content-comparison.json. No CPU-speed or resident-RAM claim.


Root physics lifecycle source audit (2026-11-09): level::LevelResources resolves the current map's compiled .bphysworld and read_physics reads it through Catalog. level_prepare's if-let physics phase owns the byte Vec, PhysWorld schema and CollisionGeometry. CollisionWorld::with_materials consumes independent triangle/filter/owner records; its fields contain no source/schema reference or source-byte lease. NavigationWorld is a published map resource tracked and removed by LevelLifetime after outgoing render acknowledgement; unpublished/failing/cancelled batches drop through PreparedLevel ownership. Original141388640 registers arkPhysicsWorldResource; the supported host replaces Havok's141262020 compressed query tree with the already measured full-precision packed navigation broad phase. Root world payloads are intentionally not session-cached, so mutable map owner/body state cannot be reused accidentally. No source change was needed here: the previous generic ledger placeholder concealed the existing acquisition/release path. This source audit does not establish whole-Havok behavior, every native registry phase or new performance measurements. Existing four-map CPU preparation observations exercise this construction/drop path; no gameplay input, windows or tests were used.


Render-program source lifetime audit (2026-11-09): registration141352730 names idDeclRenderProg. ProgramLibrary opens the installed compiled shader-cache files through the existing v14 readers (1402ca9c0 shared index,1402cc070 names,1402c9f00 variants,1402ca440 stages). Index sharing/retirement already has accepted measurements above; the global lookup is weak, Catalog keeps the session index, and libraries have independent file cursors. Program definitions decode only on definition(), with their temporary raw buffer released before return. serialized_variant() converts its compressed/unpacked source into typed D3dVariantData, then drops source staging; source/admission caches retain typed variants/required binding records. Shadow/water classification drops unpacked DXBC immediately after its checks, except explicit diagnostic export. Inline payloads remain where the typed variant genuinely owns them; no whole UCD/RPVD file is retained. ScenePreparation transfers ProgramCacheSource to the published level, and dynamic material jobs clone that same Arc rather than opening independent libraries. Level retirement drops its owner; an active job retains its actual lease until cancellation/completion. There is no library-to-Catalog strong-reference cycle. Failed construction drops opened cursors/temporary data through Rust ownership. These are source ownership boundaries, not new GPU-fence or full native bank validation.


### Options declaration staging retirement (2026-11-09)

Prediction before implementation: options::read builds typed SectionDecl settings/categories, Controls gamepads/presets and Visuals quality tables, but retains the complete generic edit tree in every live SectionDecl. The only production readers of that tree are controls::State::new and visuals::State::new; later callbacks use owned typed records. Release edit immediately after all subclass initialization. Expect generic syntax capacity retained by opened options to fall to0, with identical typed section/state and local console results. No source acquisition, widget, binding, setting or callback policy changes. Total destructor work moves from menu close to its background initialization; no CPU-speed or FPS improvement is predicted, and no frame work is added. The pure decl::section reader keeps its source tree for callers that still need initialization.

Original provenance: pause options registration14137dcc0, base declaration registration14137d600, section acquisition516d70 and option host516e40; subclasses Controls and Visuals construct owned fields (Visuals50a480/50df80). Their native declarations are typed settings/arrays rather than a generic parser AST retained alongside those fields. Host drops parser staging after converting the last supported reader; this does not implement unsupported menu subclasses. Observe the actual options::read path with installed resources and a local Console clone, without opening a UI, applying settings or writing config files; compare retained/control modes in one executable.


Resource-kind mapping correction (2026-11-09): source registration1413529f0 instantiates idTypedResourceList<arkFaceActor> for fxactor. Installed inventory contains92 .arkfaceactor resources/4339411 source bytes; searching production acquisition paths finds no reader (the FaceFX debug CVar is not one). entities::effects::actors loads idEntityFx entity definitions, not these FaceFX files. The vault resource registration names arkJumpDef_Vault; both indexed resources (607 total bytes) are navmesh/npc_movement/simplevault and simplevault_mech. Player state/sensor vault code has no acquisition of those resources. Previous generic owner mappings were wrong and have been corrected to unsupported; this does not remove any implemented reader or change runtime behavior, nor does it count those resource lifecycles as implemented. Evidence: artifacts/face-actor-vault-kind-inventory-20261109.json. The active audit objective remains all currently supported resources, with facial animation/NPC traversal classes explicitly outside current support rather than falsely credited to unrelated code.


Options staging acceptance (660cbbbc, 2026-11-09): coherent dev game/Tools build completed from clean unchanged source; Tools SHA2561a03d2be560215a34b53e85a33db23310fc05b66e4162058754a5a8bd56a60fa. Retained/retired modes each completed the actual EotW/Dust/EotW/EotW CPU preparation and options::read route with exit0. All16 section IDs/types/typed-state fingerprints match; all4 local-console results match. Per acquisition, parser storage retained by Controls/Visuals/Audio/Gameplay falls53840/59931/9879/20980 bytes ->0 (144630 total logical owned syntax bytes, about141.24KiB). This is live declaration-storage retirement, not a measured process-RSS decrease or CPU-speed gain. Options resource context returns to0 at Catalog release in both modes. No movie, callbacks, window, config writes, visual inspection or tests were used. Display initialization only queries monitors/modes; all settings initialization acts on a local Console clone. The live options path uses the same constructors and releases staging after their last reader. Evidence: artifacts/options-source-retirement-20261109-comparison.json. Existing player parser-tree accounting now calls the same capacity helper without changing its arithmetic or retention behavior.


### AnimBasic immutable item ownership (2026-11-09)

Prediction before implementation: BasicRig is shared by live usable instances, but AnimBasicNode::clone deep-copies its immutable alias/clip-index/blend item Vec and alias strings into each placement. Share that model-resolved item slice while keeping clips, time, deferred commands and blend state per node. Expect one immutable item allocation per acquired model template, no duplicate aliases for its instances, no new read/clip decode/GPU upload and no reference-count work in update/evaluate. The copied list uses model-specific clip indices, so never share it merely by animBasic declaration name across different rigs. Actual data storage and node signatures will be observed for animation-bearing placed usable models; a separate-item control uses the same representation, not an exact historical process-RAM baseline. No CPU-speed/FPS gain is predicted.

Original provenance: arkDeclAnimBasic construction1407e4540/1407e5190 owns its declaration list and joins the resource registry14005f6a0; node update1406e3de0 delegates command/playback mutation to1406e4af0 and1406e51d0. Host Arc slices share only the converted immutable model-resolved metadata, retaining independent playback. The installed declaration/tree node's exact C++ allocation layout is not reproduced. Actual acquire/release remains with BasicRig and usable instance owners; no session pin is added.


AnimBasic representation scope: Arc<[AnimBasicItem]> keeps direct slice indexing in update/evaluate, avoiding an extra Vec-header indirection. Building one shared slice moves the item records/owned alias strings once; only explicit separate-item diagnostics deep-copy them on node cloning. Existing mutable clips/deferred fields retain their former clone semantics. Default empty nodes may have a small Arc header; measurements count the chosen representation's list headers/payloads and do not equate that control with the historical Vec layout. The production asynchronous Asset acquisition and instance constructor are factored into shared helpers, and the windowless observer uses those helpers for authored animation-bearing usable root placements. It creates no visibility, frame, command or gameplay simulation and does not claim appearance/whole-frame timing. Existing cold-visibility release drops idle offscreen asset/node leases; no session pin or new per-frame allocation/upload is added. Physics-pose scratch/publication allocations are already tracked separately in optimization-opportunities and are not altered by this ownership change.


AnimBasic item acceptance (ccf955d9, 2026-11-09): coherent dev game/Tools compilation completed from clean unchanged source; Tools SHA25636f77edc7b35a561525aa5cf0fdd7b73fd14d67c0339c3cb930c58648358acdf. Both actual EotW/Dust/EotW/EotW CPU acquisition routes exited0. All1106 authored animated-root instance data fingerprints match between separate/shared items; neither route had an acquisition failure. Source assets/instances were44/266,52/308,44/266,44/266. Distinct item arrays310/360/310/310 ->44/52/44/44, matching one immutable list per model template. Logical list/header/alias payload33193/39055/33193/33193 bytes ->4776/6129/4776/4776; the same-representation control is not an exact historical Vec/RSS baseline. The item allocation context returns to0 at Catalog release in both modes. The observer uses the same usable Asset::load, model filter and instance constructor as production; it does not synthesize frames, visibility or commands and does not establish a new GPU/gameplay appearance or CPU-speed result. Mutable playback copying and frame evaluation expressions remain unchanged. Evidence: artifacts/animbasic-items-20261109-comparison.json. No tests or visible clients were used.


### Difficulty declaration ownership (2026-11-09)

Prediction before implementation: player and options create independent mutable Manager state by reading the same immutable difficulty manager/preset sources. Introduce scoped typed resource identities for both classes, with selected headers, bootstrap promotion and purge through the existing Registry. Each consumer still constructs fresh custom values/current index and keeps its direct contiguous preset/slot tables; share their immutable array storage instead of sharing mutable managers or adding a per-frame lookup. Native registrations141387290/1413872f0, declaration constructors6472a0/647550 and runtime profile initialization648590 establish the separation. Neither class occurs in142280ee0's conditional heap-1 promotion list: map headers keep their actual creation context, common/init promote at bootstrap, and dependencies inherit the producer phase only through existing Registry rules.

Installed inventory is1 manager/11193 source bytes and4 presets/2359 bytes, with no inherited declarations; options-source inventory also found no inherited options declarations. Keep the current parser/default/error behavior, immutable failure reuse and editable-overlay bypass. Expect fewer source reads/decodes across player/options acquisitions and map returns, equal typed menu/settings/profile values, and complete scoped/Catalog release. Registry/header/template storage is an explicit small residency tradeoff, not a RAM-reduction claim. No extra per-frame lock, reference counting, GPU work or lookup indirection is intended. Pure offline readers remain uncached. Parsing uses the existing field helpers; no menu/gameplay mechanics are changed.


Difficulty representation notes: parsing factories reuse the existing options field/default conversion; assets::difficulty is the sole typed identity/admission/retirement owner. The public pure format reader remains available without sharing for offline callers. Runtime Manager fields keep direct Arc slices and copied scalars, so getters do not dereference a new definition wrapper or lock a registry. A private definition lease preserves the header generation through consumer retirement; the definition also holds its preset resource leases. Small copied contiguous preset values intentionally replace per-value pointer chasing, with that duplicate immutable metadata counted as an explicit host layout tradeoff. UI/profile/custom mutation stays outside shared definitions. Semantic Debug output excludes lease handles so existing typed-setting fingerprints remain comparable. Validation will acquire through the actual player load_difficulty and options initialization paths on four real level preparations, including saved-profile inputs, without UI/gameplay simulation.


Difficulty acceptance (015e50e8, 2026-11-09): coherent dev game/Tools build completed from clean unchanged source; Tools SHA256496aaa7cc0241264a3ee69e5d7c1561416abc71ba89ab7658b17c60970d90d8d. Both actual EotW/Dust/EotW/EotW routes exited0. Eight player/options manager acquisitions decoded8 ->1 definitions; nested preset requests/decodes32 ->4. All four player profile-setting fingerprints,16 typed option sections and four local Console results match; option semantic signatures also match the previous accepted660cbbbc observation. No acquisition failures. Existing common/bootstrap registration kept the one manager/four preset generations across map returns; marker0 purge did not remove them. Their allocation context returns to0 after Catalog release in both modes. Final logical decoded payload1432 manager bytes +480 preset bytes; registry record estimate302+819 bytes excludes hash control bytes/allocator rounding. This small retained immutable metadata is a reuse/parity tradeoff, not a RAM-saving claim. Runtime custom/current fields remain independent by construction, with no new per-frame registry operations. Evidence: artifacts/difficulty-resources-20261109-comparison.json. No tests, windows, UI callbacks or config writes were used; no new GPU/gameplay appearance or CPU-speed result is claimed.


### All-level CPU acquisition and retirement coverage (2026-11-09)

The clean557dbc1a game/Tools build completed actual CPU preparations for all36 installed level manifests in one Catalog: boat maps, both chaos variants, interiors, return maps, menu/tutorial and Void maps. Every level reached its released phase and the process exited0. The route acquired1964 unique inherited light sources/1488977 source bytes and constructed12751 lights; all36 temporary light-source owners reported0 live generations after preparation. At Catalog shutdown all12 reported resource contexts were0 bytes: light sources, difficulty, AnimBasic items, options, blueprints, motion scenes, render parameters, mass, sound events, FX graphs, shader indices and pickables. Acquisition diagnostics contained no success=false/preparation-failed/acquisition-failed/unavailable records. Difficulty and option initialization used their actual production consumers; no gameplay inputs, application runner, windows or tests were used. This broadens acquisition, typed conversion and prepared-owner retirement coverage. It does not establish all-map visual parity, GPU upload/driver residency, runtime streaming during movement, or unimplemented consumers. Evidence: artifacts/all-level-resource-acquisition-20261109-summary.json and the matching per-process result/logs.

### Player mode and locomotion source-lifetime review (2026-11-09)

Installed inventory:3 playerLocomotion resources/150207 source bytes, including2 inherited character declarations;15 playerMode resources/50044 bytes, including2 inherited default modes. Source acquisition already uses components::component_fields, keyed by canonical type/path, with inherited layers resolved in parent-to-child order. Its cache is local to player_declarations_with_listener and is dropped before the result returns. DeclarationSource shares that resolved bundle for the actual player and arm setup consumers, disables sharing for editable overlays, and keys it by Catalog identity and character. setup::prepare calls keep_screen_inputs after those consumers; only FX/controller syntax survives for the later screen-effect reader, and the local setup owner drops on return or unwinding. Thus it does not keep mode/locomotion source trees through gameplay or across level unload.

app::player::load_body compiles the locomotion fields into separate body/bob/arm LocoData using each model's resolved clip indices, preserving their distinct model bindings. Runtime LocoNode owns those tables alongside independent playback state; animation clips remain in the existing clip/rig owners. PlayerSetup converts mode fields into RuleDecl/default IDs and supported StateDecl values; StateMachine owns those values with the level player. Source review found no additional mode/locomotion source read in frame updates and no source-syntax retention requiring a new permanent cache. Adding such a cache would retain the generic parse tree after its last consumer without demonstrated reuse, so none is added. Native registrations14138cdb0/14138f7d0 identify these classes; common identity and animation behavior addresses are cited in their rows. This is a source-qualified acquisition/retirement review, not new native-header parity, runtime memory measurement, or an appearance/timing claim. Inventory evidence: artifacts/player-mode-locomotion-inventory-20261109.json. Other entityDef consumers still need their separate complete review.


EntityDef light-source prediction (before implementation,2026-11-09): inherited_light_edits currently reads/parses each ancestor again for every placed light and deep-copies source edits. Registration141352590 and common lookup1400607f0 identify one immutable entityDef source identity; parent-before-child overlay is140022fd0. Introduce a preparation-only owner of decoded source layers with shared parent leases and borrowed edit views. Expect source read/parse counts to fall from total ancestor visits to unique canonical declarations in each light preparation, identical ordered PlacedLight data, and zero retained source payload after preparation. Keep pure readers and editable overlays separate; do not change constructor defaults, validation, layered channel overrides, rendered lighting, GPU work or frame-time lookup. This is temporary setup reuse, not a session-wide generic syntax cache or a RAM/CPU-speed claim. Compare both acquisition modes through real map preparations before accepting.


Light-source acceptance (557dbc1a,2026-11-09): coherent dev game/Tools build completed from clean unchanged source; Tools SHA25695bdd440b25c9a3a243ba0e0f58e79a8c095572dcbf14415fe3f095487f5871c. Both actual EotW/Dust/EotW/EotW preparation routes exited0. Source reads1431/1394/1431/1431 ->74/90/74/74 (5687 ->312 total), matching distinct acquired declarations per map. Source bytes870648/817070/870648/870648 ->56215/67341/56215/56215 (3429014 ->235986 total). Ordered PlacedLight counts611/570/611/611 and all four complete data signatures match, covering2403 light records. All74/90/74/74 shared source generations have zero strong references after light preparation; both resource allocation contexts return to0 after Catalog release. The private owner only moves needed edit layers and borrows them in unchanged conversion expressions; it retains no generic source payload during gameplay and adds no frame-time work. The old reader remains the separate-acquisition control and pure/overlay route. This is an I/O/parse-count and scoped-retirement result, not a measured CPU-speed/RSS/GPU/appearance claim. Evidence: artifacts/light-sources-20261109-comparison.json. No tests or game windows were used.

EntityDef consumer review: installed sources contain5653 entityDef records/7987730 source bytes. All production literal entityDef acquisition sites were traced: compact inherited-model references; temporary light layers; player/body/arm setup bundle and arm render-model resolution; idEntityFx actors; HUD outsider-mark defaults; journal power-system/component descriptions; inventory initialization/collection selection; interaction-sensor settings. Each discards parser syntax at the end of its resolution/preparation scope and retains its own typed output, never the complete entity source. The HUD retains scalar thresholds, initial power names and inactivity; the journal retains Power/Upgrade descriptions and rune path per presentation; inventory retains assignments or a configuration-generation Arc; the interaction owner retains a Copy Sensor. RuntimeDecalProxies is inserted by prepare_level (not plugin initialization), so LevelLifetime captures/removes it with map-created resources; its material/mesh references do not escape through the persistent GeometryLayouts owner, which is explicitly reset. Render-world decal bindings clear after outgoing-render acknowledgement while reusable device upload capacity stays session-owned. These are source-qualified ownership findings, not a new render-transition or all-map appearance measurement. Deferred unsupported world-FX families already store compact Result<bool,String> admission data, not generic parser trees; that previously identified retention issue is already fixed and was not reimplemented. Entity actor ancestry inventory finds30 func/fx descendants; its555-byte root contains none of ActorParameters' overrides, so no installed default mismatch was established from that separate reader's current limitation. Local evidence: artifacts/entity-definition-consumers-20261109.json and fx-actor-entity-defaults-20261109.json. Full generic declaration-header parity and unsupported consumers remain outside these host-scoped representations.


Material handoff scope prediction (before implementation,2026-11-09): MaterialCache::pending_source is a one-use transfer from caster admission to material conversion (common identity1400607f0), but the whole cache moves into RuntimeMaterialOwner and PreparedBatch without explicitly ending that transfer scope. Retire any unconsumed staging at those two preparation boundaries, after the last possible take_source call. Expected saved residency is exactly the pending path/String and Vec capacity when nonempty, otherwise zero; no read, upload, material identity, parameter value or frame path changes are expected. Use the existing real ScenePreparation worker through a windowless graphics-device diagnostic, without calling application run/update, creating a window, starting gameplay or playing audio. Acquire fresh host assets per independent preparation and compare source handoff CRCs, material cache counts and pending byte capacities with retention switched on/off; do not infer a nonzero saving before observing it. This is scoped resource retirement, not a CPU-speed claim.


Material handoff scope acceptance (fe42153d,2026-11-09): coherent dev game/Tools compilation completed from clean unchanged source; Tools SHA25600fae841cf76f6e6374490a764ffe80d34de4bf0887aec9cd9a816b3daf06680. Both actual ScenePreparation worker routes (EotW/Dust, fresh graphics host per independent preparation) exited0 with retention enabled/disabled in the same binary. EotW retained573 logical String/Vec capacity bytes at cache transfer before ->0 after; Dust had0 in both modes. Actual source handoff signatures/admission/resolved CRCs match, as do1384/1409 material cache entries and scene asset counts:28/29 images,9783/8985 meshes and68/74 shader buffers. This closes a small one-use staging lifetime defect; it is not a significant RAM/RSS or CPU-speed improvement. Both startup and runtime batch transfers end the staging scope after their final potential take_source; only the startup transfer is exercised by this observation. The helper executes no application run/update, creates no OS window and plays no audio. Dropping the real completed-but-unpublished batches observes that cleanup boundary; it is not mid-read cancellation, asset publication/render upload, runtime-tail or GPU draw coverage. Evidence: artifacts/material-staging-scope-20261109-comparison.json.

Layered image streaming inventory (2026-11-09): header-only reads over all14885 selected installed BIM resources classify11885 2D images,2897 cubes,102 volumes and1 array, with0 read/header errors. None of the3000 layered resources has an indexed separate mip resource (_mip0..15). Their owner must retain the complete authored inline chain and retire its CPU staging after upload; inventing a finer-mip scheduler for these installed classes would have no source data to request. This extends the previous102-volume inventory to all typed cube/array inputs. It is not activation coverage, a full payload validation or a GPU residency measurement. Evidence: artifacts/layered-image-streaming-inventory-20261109.json. MD6 metadata inventory separately found133 ragdoll references/125 normalized names across461 model declarations, with no spelling aliases; no extra physics-system alias cache was added from that inventory.


Scene publication observation prediction (before implementation,2026-11-09): successful completed-but-unpublished worker acquisition does not prove the next lifetime boundary. Extend the same windowless observer to call actual PreparedLevel::publish, LevelLifetime::begin/published/clear and Bevy Assets::track_assets. Source boundaries1408a4830/1400410f0 remain the existing host implementation; no synthetic gameplay or replacement cleanup algorithm is added. Record asset counts, allocation-tag snapshots and the existing device allocator report before preparation, after preparation/publication and after teardown/asset tracking. Common session leases can legitimately survive; no zero budget or GPU-draw proof is inferred from an empty rendered view. Do not call App.run/update or playback. A fresh host per sample isolates this observation from unresolved inter-map admission/renderer scheduling.


Publication observer namespace correction (2026-11-09):30cf0d92 built cleanly and both EotW/Dust publication/teardown paths exited0. Published mesh/material/buffer assets returned to baseline after actual handle tracking (meshes9786/8988 ->3; probes1301/1305 ->0; decals78/104 ->0; sky materials2 ->0; buffers68/74 ->0). Images4177/4006 ->11 against8 initial images; surviving session images require identity qualification rather than a guessed zero budget. The final heap snapshot still held about36MB under a preparation phase plus sound/header data. Source tracing identified a diagnostic setup error: it used Catalog::open while actual contact preparation requests Catalog::shared, creating a second archive namespace retained by its session root. The normal client already uses shared plus launch::CatalogSession. Reuse that exact root/guard in the observer, dropping the real host/audio consumers and local root before the guard. Do not interpret the old two-namespace residual as a production leak. Existing14302ca9 main release completed successfully. No production ownership algorithm is changed by this observer correction; actual corrected shutdown evidence is still required.


Archive alias identity prediction (before implementation,2026-11-09): the corrected shared-root observer7c38e117 still exited0 with unchanged map asset retirement, but the session guard found no matching root and about36MB of catalog-tag allocations remained. The installed Steam registry returns d:/programs/steam while the acquisition command names D:/Programs/Steam; Windows directory identity and final-path lookup confirm both reach exactly the same base directory. Catalog::shared compares PathBuf spelling, so contact/audio acquisition replaces the root with a second Catalog identity. Reuse standard filesystem canonical identity on a spelling miss, retaining the initial request and one most-recent alias for allocation/IO-free repeat hits. This bounded alias memo avoids adding filesystem work to normal equal-path hits or every subsequent sound acquisition. Keep installed resource keys, payload paths, overlays and every typed parser unchanged. Expect one shared catalog instead of two for this actual two-provider route, correct guard matching and retirement of catalog-tag allocations. The ordinary matching-path route is not predicted to gain RAM; do not call this a general game RAM reduction. A diagnostic switch preserves spelling-specific identity for a same-binary comparison. Actual comparison must preserve scene assets/source checksums and distinguish remaining independent session caches from catalog retention.


### Archive identity and sound diagnostic shutdown (2026-11-09)

Host catalogue identity acceptance: coherent dev game/tools06ad18dc completed from clean unchanged source. Both actual windowless EotW/Dust prepare/publish/LevelLifetime-clear/Assets::track_assets routes exited0. Equivalent installed-directory spellings formerly created catalogues1 and2 and prevented catalogue1 shutdown from matching the shared root. Canonical physical-directory identity now reuses catalogue1; final guard reports matching_root=true and remaining_consumers=0. Primary exact-path hits remain free of filesystem work; one bounded cached alias avoids repeated canonicalization. Material admission/resolved fingerprints and every recorded asset count match both modes. After handle tracking meshes return to3, probes/decals/skies/buffers to0, and images to11 (8 initial plus3 persistent class2 intrinsic images,3072 bytes). Original1400607f0/14005feb0 resource identity semantics are preserved; physical installation-path normalization is host integration. Evidence: artifacts/catalog-alias-lifetimes-20261109-comparison.json. These observations run no application/render updates and do not prove GPU upload/retirement.

Prediction before sound-diagnostic change: static property-chain diagnostic Weak<[NodeProperties]> entries keep dead Arc slice allocations reserved after the last sound owner drops. They also retain cumulative request fingerprints beyond audio session shutdown. Retire this host-only diagnostic metadata after AudioSession has closed producers and retired media, leaving actual playback/property ownership unchanged (original1410f2370/1410de430/1410deb00). Expect all diagnostic request entries and dead slice allocations to release at that boundary; compare actual tagged heap before/after using one binary with DH2_DEBUG_RETAIN_SOUND_PROPERTY_DIAGNOSTICS=1 as the former behavior. No frame work, sound latency, GPU or production non-diagnostic memory improvement is predicted. The prior11.86MB sound-bank-lookup residual is a lead, not yet attributed in full.

Sound-diagnostic acceptance40bd7b9d: coherent clean unchanged dev game/tools build exited0, toolsSHA2568bda98f865d7391cc015119f7c12ceb3d5b7b6078a7f7a233fc558dd5c393088. Both actual windowless EotW/Dust publication/clear observations exited0. Final diagnostic retirement released11794 dead Arc slices (10627104 bytes of slice/header storage), the weak-reference vector and14118 cumulative request identities. Sound-bank-lookup tagged residue fell11867336 ->167952 bytes (11699384 bytes released), consistent with predicted dead slices plus diagnostic container metadata. All live property counts/bytes/nodes, request counts/content fingerprints, scene asset counts and material fingerprints match. Concurrent cold candidate construction varies slightly before publication; created totals are not an output-parity assertion. Evidence: artifacts/sound-diagnostic-retirement-20261109-comparison.json. This fixes host diagnostic retention; no normal gameplay RAM/CPU/FPS improvement is claimed. Remaining contexts include sound media1460800 and sound graph364816 bytes; their owners are still under audit, not certified free from retention. No tests, windows, gameplay input or application/render updates were used.


### Sound cache container retirement (2026-11-09)

Prediction before implementation: existing final-session MEDIA/STREAMED/header/setup retirement clears entries but leaves process-static HashMap backing allocations resident. Keep existing map transitions and shared-source reuse unchanged, but move these four entire tables out of their mutexes at final audio shutdown, then drop their storage. The diagnostic streamed-media identity table has no session retirement at all; retire it at the same boundary and reset its counters. Original source release141122400/1410f1910 and streaming-device shutdown1400bc8f0/1410bf740 establish the resource boundary; freeing Rust table backing is host integration. Expect all five tables to have capacity0 after shutdown, versus former retained capacities under DH2_DEBUG_RETAIN_SOUND_CACHE_CAPACITY=1. No per-frame work, source-format/conversion, decoder precision, audio latency or inter-level cache change is predicted. Compare actual publication/retirement route content fingerprints and final tagged heap in the same binary. Logical capacity-times-slot sizes exclude hash load-factor/control bytes and allocator rounding; do not treat them as exact heap savings. No RSS/FPS/CPU-speed claim is made.

The first sound-cache batch failed compilation before any observation: media maps use Bevy hashbrown while header/diagnostic maps use std HashMap. The shared retirement helper now accepts each existing container through usage/clear closures; hashers and lookup representations remain unchanged. No source conversion or gameplay table replacement was introduced.

Sound cache storage acceptance (da2be524, 2026-11-09): clean unchanged dev game/tools build completed0; toolsSHA256368adaeda1ac73cf7085e4e7d22ca8025d7ecc6d4b1b7c5540988bdd501cb9c5. Both actual windowless EotW/Dust publication/clear routes exited0. All five final table capacities now become0; former clear-only media/streamed tables retained capacity13848/3475 in the control, header/setup tables7/7 and stream diagnostics3584 entries of capacity. Capacity reports may vary with concurrent insertion/removal and are not exact bucket-byte counts. Actual tagged shutdown storage fell sound media1460800 ->0, sound graph364816 ->256 and sound bank lookup167952 ->0:1993312 bytes released across those contexts. All asset counts, material fingerprints and live property storage/content fingerprints match. The256-byte graph-context residue is not attributed to a surviving audio resource. This change affects final session cleanup only; map transitions retain their existing caches, and no walking/frame-time or normal resident-playing RAM improvement is claimed. Evidence: artifacts/sound-cache-storage-v2-20261109-comparison.json. No tests or visible game launches were used.

### Windowless render-asset lifecycle observation (2026-11-09)

The previous windowless publication observer intentionally ran no render extraction/preparation, so its unchanged device allocator totals cannot establish GPU asset retirement. Extend the same actual map preparation/publication path with an explicit inspect-scene-gpu-resources mode: use the installed asset owner's First/PreUpdate/PostUpdate maintenance and real RenderApp extraction/preparation, wait for actual image/mesh/buffer identities to become resident, acknowledge GPU completion, then consume real handle/removal events through the existing material/image owners. Cameras are inactive, no window/winit plugin or audio output device is created, no App.run/App.update is called, and no gameplay or sound playback schedules are registered. Audio acquisition initialization is factored from SoundPlugin into one shared function used by the client and observer; its source queues/banks/media owners remain the same. Existing non-render observer modes now also avoid creating an unused host audio output. This is host observation infrastructure, not a second upload path or a change to gameplay rendering.

No speed or memory gain is predicted from the observation itself. The evidence sought is actual GPU asset counts/allocation groups through upload and map retirement, which the former CPU/main-world route could not supply. Original image upload/replacement1402e9ff0/1402f4060 and resource purge140060180 remain represented by the existing Bevy/wgpu owners. A120-second diagnostic safety deadline fails the observation if inputs never become resident; it is not a game readiness rule. Three maintenance/removal epochs expose GPU-material-held image leases after main-world handle retirement; recorded survivors require attribution and are not automatically declared leaks. Build and actual-route evidence remain pending.
The first sound-cache batch failed compilation before any observation: media maps use Bevy hashbrown while header/diagnostic maps use std HashMap. The shared retirement helper now accepts each existing container through usage/clear closures; hashers and lookup representations remain unchanged. No source conversion or gameplay table replacement was introduced.

Windowless upload follow-up:7a4bd8b4 compiled cleanly but the first actual RenderApp asset preparation failed on missing SkyProgram. SkyAssetsPlugin registered GpuSkyMaterial upload without its required program, relying on unrelated FogPlugin registration. Move the unchanged init_gpu_resource<SkyProgram> registration into SkyAssetsPlugin and remove it from FogPlugin. Expect the normal client to construct exactly the same single program, bind-group layouts and shader module in RenderStartup, with no extra per-frame work or GPU allocation; sky-only asset acquisition now has its own complete dependency. Original sky state293510/2a8140/2a09f0 and image upload1402e9ff0 remain unchanged. The observer is explicitly limited to asset upload/removal: it does not install camera/view pass owners and must not call their global release routine. GPU queue completion precedes main-world teardown; real material/image handle events perform asset retirement. This is not acceptance of the uninstantiated view-pass owners or a claim of a screenshot match.

Sky-owner integration acceptance4ec66886: clean unchanged dev game/tools build exited0 (toolsSHA256b5cc9e24c2eadcfc138177e0de6fb72c25ed243496fc0a2bdf0d15bc1d373d39), and actual windowless EotW/Dust GPU acquisition/removal exited0. Material admission/resolved fingerprints exactly match the earlier non-render acquisition route. GPU images4180/4009, meshes9787/8989, buffers68/74 and sky materials2 return to14/4/0/0 respectively after map clear and removal epochs. Live device allocations fall2368602112/2339962880 ->49938432 bytes. Remaining host assets/slab occupancy still require attribution against an initialized host-only baseline; this does not certify viewport pass resources or dynamic FX cancellation. The source has one SkyProgram GPU-resource registration, in SkyAssetsPlugin; the normal FogPlugin uses that same owner. Evidence: DH2-fx-resources/artifacts/render-asset-lifetimes-v4-20261109-summary.json. The second fresh host logs an already-installed global logger error, with no GPU validation error; the observer follow-up prevents that redundant logger registration. No tests, windows or gameplay input were used.

The corrected4ec66886 windowless GPU route completed EotW/Dust with exit0. Uploaded GPU asset counts were4180/4009 images,9787/8989 meshes,68/74 buffers and2 sky materials; after actual map clear and removal epochs they became14 images,4 meshes,0 buffers and0 sky materials in both independent hosts. Device live allocations fell2368602112/2339962880 ->49938432 bytes. This is actual asset release, not a complete render-view acceptance. The remaining live allocation is dominated by three general mesh slabs (40370176,1114112,1048576 bytes), while20 texture allocations total5177344 bytes; reserved heap blocks are separate from those live allocations. Do not treat the14 images/four meshes or49.9MB as map leaks without identifying their owners. The next observation initializes host-only GPU assets before preparing the map and records the retained IDs, geometry ranges/buffer sizes and image descriptors; this distinguishes persistent builtins and oversized pooled slabs from outgoing map resources. Its added maintenance work is diagnostic-only. A second-host global logger reinitialization error was unrelated to graphics validation; subsequent hosts now reuse the first subscriber. No gameplay visual/CPU/FPS result is inferred. Evidence: artifacts/render-asset-lifetimes-v4-20261109-summary.json.


### Host mesh allocation failure rollback (2026-11-09)

Source audit found that SlabAllocator::allocate_general reserves an offset before requesting backing-buffer growth, but skips a slab on CantGrow without freeing that reservation. grow_if_necessary can also increment current_slot_capacity before reaching its growth limit and returning CantGrow, although no replacement buffer is then scheduled. Correct failed acquisition to preserve both prior ownership and capacity: free the uncommitted offset on rejection and restore the entry capacity before reporting failed growth. Prediction: successful allocations, buffer data, indices and upload/draw scheduling remain unchanged; a rejected acquisition leaves zero new slot ownership and no advertised capacity beyond its prior buffer/staged growth. This is a host allocator correctness repair beneath the existing model resource boundaries (native lookup/purge1400607f0/140060180), not a port of the original allocator or a claimed measured performance gain. The installed two-map route has not reproduced this rare limit failure; do not invent a RAM or FPS saving for it. No new checks run on the successful no-growth path.

Mesh rollback acceptance. Clean tools build 9aedd0b7 and actual windowless EotW and Dust District upload/removal both exited zero. The initialized-host baseline and post-removal inventories match the preceding observation, including four host meshes, fourteen images and zero level buffers or sky materials after removal. Live device allocations return to 49,938,432 bytes in each map, and material fingerprints match. This exercises normal allocation and retirement with the fix. The rare growth rejection was established by source inspection. Evidence: artifacts/render-slab-rollback-20261110-comparison.json.

### Runtime material acquisition cancellation

Host worker audit found that a running material batch could continue resolving further meshes, decals and images after level cancellation or request supersession. The existing generation gate rejects obsolete publication. Check the same cancellation tokens between acquisitions and before returning a completed batch, so the worker releases its unpublished assets and owner locks once it notices cancellation. An archive call already underway can finish. The expected gain is the work and staging avoided for the remaining items of a cancelled batch. Successful requests retain the same resource ordering and output. The checks run on the acquisition worker. Original material identity and purge boundaries remain 1400607f0 and 140060180. Runtime cancellation observations remain outstanding.


Effect source acquisition uses one task per path shared by current requesters. The previous task owner could keep queued acquisitions alive after their last requester stopped or moved to a different effect. Retire the task after applying the command batch or rejecting a dependency generation when no requester still needs its path. A stop followed by a start for the same path in one batch keeps the existing task. Already running synchronous archive work can finish, and dropping the task discards its result. The expected saving is queued source acquisition avoided for orphaned requests, with cache cleanup scheduled through the existing owner. There is no additional scan on frames without a request change. These are host task ownership rules under the same 1400607f0 and 140060180 resource boundaries. The live cancellation route still needs an observation.


### Layered water image acquisition on the GPU

The installed windowless EotW and Dust District routes from 56a90471 explicitly acquired their prepared water sources through LayeredImageSource and completed with exit zero. EotW requested four cube aliases sharing two uploads and two caustics aliases sharing one upload. Dust requested six cube aliases sharing three uploads and two caustics aliases sharing one upload. Every duplicate alias returned the same GPU image. All seven layer sources reported one upload each and zero retained CPU staging. Uploaded payloads retained their authored format and mip counts, including the seven-level BC4 array and Dust's 10-level and 9-level float cubes. Both maps finished removal with fourteen ordinary GPU images, four host meshes, zero buffers and zero sky materials. Device live allocation was 50,331,648 bytes, 393,216 bytes above the preceding route. The observer also acquired the class-2 default cube and both procedural noise textures through the persistent host owners. GPU view sampling and demand admission require gameplay evidence. Original lifecycle boundaries are 1402e9ff0, 1402f4060 and 140060180. Evidence is DH2-interactions/artifacts/layered-gpu-lifetimes-20261110-summary.json. The duplicate logger diagnostic from the older fresh-host harness is addressed by the already verified host logger registration in main.


### Common static model source reuse

Original lookup 1400607f0 reuses resource identities and 140060b60 promotes bootstrap residents. The installed init and common manifests contain 131 static models. None overlaps the 36 inspected primary map manifests (reuse chunks are a separate admission path), and the captured placement route requests 13, 11 and 13 common models on EotW, Dust and EotW. The rewrite decoded these source models once per map and discarded the result after setup. The host owner now acquires immutable common geometry on demand and leases it from the installation catalog. Per-map source models retain their existing ownership. Editable catalogs keep independent acquisitions. GPU mesh construction, transforms, placement-dependent attributes and frame paths retain their current owners. This completes source reuse for the common case and leaves GPU mesh identity reuse open.

Prediction before acceptance. Common source reads and decodes occur once per distinct requested path while the catalog lives. Measure actual production setup in shared and separate modes, including retained decoded geometry bytes and final owner release. This deliberately trades resident CPU geometry for avoided archive reads and decoding, and requires evidence of that tradeoff. The diagnostic control is DH2_DEBUG_SEPARATE_COMMON_MODELS. No timing or gameplay result is claimed yet.

Common static source acceptance. The clean ad2e5623 tools build ran actual EotW, Dust District and EotW preparation with independent and shared sources. Reads/decodes fell from 13/11/13 to 13/2/0. The 37 matched source signatures and all three prepared image/mesh/buffer counts and resolved material fingerprints were identical. Shared sources retained 894,148 tagged bytes after the last map preparation and released the tag to zero at catalog shutdown. This is a startup and transition optimization with a bounded session CPU cost, with no per-frame work. Evidence: artifacts/common-static-models-v2-20261110-{1,0}.

Area-fog model source review: the existing installed inventory in analysis/fog-volume-audit.log identifies 157 enabled authored fog volumes across 12 of 36 inspected maps. The 157 model paths are distinct within each map. Their BML1 positions and indices prepare a dedicated fog mesh, while the separately acquired collision hull serves containment. No duplicated per-map fog render-model acquisition was demonstrated. The render model is temporary preparation data and the map owns its GPU mesh handles. The installed inventory does not establish dynamic volume behavior or GPU draw timing.

### Isolate pre-level GPU mesh slabs

Prediction before change: once the first scene publishes its mesh IDs, stop assigning new allocations to the already initialized bootstrap slabs. The real asset upload/removal route should retain its four host mesh identities and reclaim about 39.3 MB of live GPU buffers after either level unloads. There may be additional active-level backing capacity for the separately packed bootstrap vertices and indices. New geometry and layouts still upload through the existing allocator, with no movement of resident allocations, new draw-path lookups, or added copies. This is a host allocation boundary under the original map purge 140060180, not a port of the original GPU allocator.

Bootstrap mesh slab acceptance (ee96434d). The same clean executable uploaded and retired actual installed EotW and Dust District assets in both modes. GPU asset counts and resolved material fingerprints match. Each map returns to the same four host mesh IDs and vertex/index ranges. With mixed slabs the four-vertex host mesh pins a 40,312,752-byte buffer. With separated slabs its buffer remains 1,048,608 bytes. The device reports 50,331,648 -> 11,075,584 live bytes after either map unloads, a 39,256,064-byte reduction. Uploaded totals were 2,313,027,584 -> 2,312,765,440 in EotW and 2,320,302,080 -> 2,329,280,512 in Dust. Active slab packing and reserved driver heap totals can vary; this change targets live post-unload buffers. The two dev upload spans do not establish a speed or frame-time result. Cameras were disabled, and no gameplay, test suite or draw-output acceptance ran. Evidence: DH2-fx-resources/artifacts/bootstrap-mesh-slabs-20261110-{1,0}.

### Installed shader stage admission metadata

Prediction before change: 1402ca440 has a stage-indexed resident path and reads and inflates the UCD span only when no resident payload exists. The host shadow and water admission paths only need the validated DXBC identity, but separate program variants can repeatedly inflate the same span. Within one ShaderCache, keep only the validated 16-byte checksum per demanded stage ID. Expect one read/inflate per distinct valid cached stage instead of one per admission request. Unpacked bytecode still drops immediately; cache entries release with their file owner. No GPU, frame submission or per-draw changes. Explicit bytecode export retains direct access to the full payload. The same-binary admission diagnostic records stage reads, decoded bytes and affected CPU cycles; keep the cache only if actual production preparation shows reuse.

Stage identity acceptance (79688ae3): the same clean dev tools build completed both windowless EotW/Dust preparation modes. All 1,070 ordered stage ID/checksum records, material fingerprints and prepared asset counts match. Cached stage reads and inflations fell 548 -> 38 in EotW and 522 -> 40 in Dust. Cumulative decoded stage bytes fell 2,956,344 -> 485,396. Total thread cycles recorded inside the affected stage-admission calls fell 130,807,137 -> 13,818,203 in the two dev samples. The timing covers those calls only. The new level-scoped payload is at most 40 checksums (640 logical bytes plus keys and table overhead in these samples); the actual unpacked stages still retire after use. No gameplay/FPS or steady CPU timing claim. Evidence: DH2-interactions/artifacts/shader-stage-admission-20261110-comparison.json.

### Render placement staging transfer

Prediction before change: the original render assembly 1401bc1f0 resolves child models into ordered instance records. In the host setup path, SceneData keeps the complete authored placement vector after the expanded copy becomes RenderSourcePlacements. The completed EotW/Dust preparation reports 4,727/4,654 authored records before expansion. Transfer that vector to the renderer in place, preserving all root and child indices, transforms and material overrides. Expect zero source placement capacity after setup and one fewer clone of every root ID, path and override array. The renderer, machinery, interactions, visibility and shadow consumers keep their current indices and inputs. CPU-only preparation diagnostics and collision still use the original root records before scene setup. Compare the exact ordered placement signatures and retained capacities with the previous storage mode in the same executable.

Render placement acceptance (30bf3f0c). Both same-binary dev observations completed cleanly. EotW retains 2,198,868 bytes of combined source/live placement capacity, down from 4,239,811. Dust retains 2,062,355 bytes, down from 4,082,167. The source Vec now has zero capacity after setup. The ordered 5,638 and 4,743 placements, their geometry/material input signatures, prepared GPU asset counts and resolved material fingerprints match the separate-storage route. The live vector capacity varies slightly because it reuses the original allocation. These figures cover the measured owner capacities, excluding allocator overhead. No gameplay or frame timing claim. Evidence: DH2-fx-resources/artifacts/placement-storage-20261110-{1,0}.

Static model acquisition boundaries. 14026a5a0 opens the BML1 resource, verifies the endian-aware BML magic and installed version byte, delegates triangle data to 140259d50 and closes the input stream. The static path computes bounds in 140269ff0, conditionally releases the CPU triangle arrays in 140278df0 and releases surface ownership in 14026a240. This is distinct from the BMD6 loader 14025aa00. Both placed models and fog volumes now request installed BML1 data through assets::bmodel. Fog retains its own map-scoped GPU position/index representation and no new cache. Expect unchanged acquisition count and geometry. The host common-source cache deliberately retains decoded data where repeated placement preparation benefits from it; it does not reproduce native GPU mesh residency.

The windowless GPU observer gains an optional same-host route for EotW/Dust/EotW. It reuses the actual Bevy asset storage and device, runs level publication/clear and real GPU upload/removal maintenance for each map, and records remaining IDs before the next map. This observes repeated asset retirement. Cameras stay inactive and view-pass, gameplay and audio scheduling remain outside this route.

Same-device acceptance (1c02a23d). The clean dev tools build loaded, uploaded and cleared EotW, Dust District and EotW in one device and asset store. All three removals returned to exactly 14 images, four original host mesh IDs with the same ranges and slab sizes, zero buffers and zero sky materials. Each finished with 11,075,584 live device bytes. The EotW visits matched uploaded asset counts (4,180 images / 9,787 meshes / 68 buffers / 2 skies) and resolved material fingerprints; Dust matched its prior independent-host result. The 1,006,632,960 bytes of reserved D3D12 blocks stayed constant at the three retirement checkpoints. None of the five blocks was empty. Three 256 MiB blocks still contain small live textures, with total live allocations of 65,536, 1,114,112 and 65,536 bytes respectively. This is a separate placement/heap reservation problem. No render-view draw, window, gameplay input or test suite ran. Evidence: DH2-interactions/artifacts/same-host-resource-retirement-20261110.

Main release 305da3dd built cleanly from unchanged source. It includes common model source reuse, bootstrap mesh slab separation, shader identity reuse and placement transfer. Executable SHA256 d81e4b5563c3f5f278d5c15ef9b43e193793b81d85420ba04507da8885d839de. Build provenance: artifacts/resource-owner-batch-20261110-build-provenance.json. Compilation is separate from live gameplay acceptance.

### Bootstrap intrinsic image placement

Prediction before change: original 14036f800/14005fde0 registers intrinsic resource identities with class 2 and the renderer's image initialization 1402ec020/1402f5920 invokes the builtin generators before map acquisition. The same-device observation shows the rewrite's three supported 16x16 intrinsic 2D images first allocated during map preparation, then held across every map while multiple large D3D12 heaps retain small unlabeled texture allocations. Prepare those three builtins through the existing ResidentImages owner during bootstrap so their first upload precedes bulk map allocation. Expect the same content and final live bytes, three fewer scene-worker payloads and no per-frame overhead. If those allocations pin the three late heaps, reserved memory after map teardown should fall; the font atlas and other persistent objects are independent, so the magnitude requires an actual allocator observation. Editable views and the separate-residency diagnostic keep their existing behavior. The same-device diagnostic compares early versus demand acquisition using installed EotW/Dust/EotW data.

A separate windowless device profile compares the backend's existing smaller-heap policy (`MemoryHints::MemoryUsage`, 8–64 MiB GPU blocks and 4–32 MiB host blocks in the installed wgpu-hal source) against its default (128–256 and 64–128 MiB). Expect lower reserved bytes if surviving allocations still pin partly empty heaps, unchanged live image and mesh counts and bytes, and potentially more heap allocations during upload. Inspect both the active and retired heap reports and the affected upload spans. This comparison adds no production setting or policy change.

Bootstrap intrinsic acceptance (2ea77993). All three same-device EotW/Dust/EotW runs succeeded with the same three intrinsic texture checksums, resolved material fingerprints and uploaded/retired asset counts. The default heap policy retained 1,006,632,960 reserved bytes after each unload with deferred intrinsics and 469,762,048 with bootstrap allocation, a 512 MiB reduction. All three intrinsic allocations now share the host's first block; before, each occupied a different 256 MiB block, one of which still holds another texture. Live allocations remained 11,075,584 bytes after each unload. The diagnostic smaller-heap profile further reduced reserved bytes to 100,663,296, but increased active heap blocks from 11 to 29–30. That profile remains diagnostic-only pending a gameplay performance comparison. These are allocator reservations, distinct from measured dedicated VRAM use. No gameplay draw or frame timing is inferred from the windowless upload spans. Evidence: DH2-interactions/artifacts/bootstrap-intrinsic-heaps-v2-20261110-comparison.json.

The GPU setup resolves authored per-placement material override strings before publication. Every live placement consumer uses IDs, transforms, bounds or the already resolved material state. Drop the setup-only override arrays immediately before transferring RenderSourcePlacements. Expect unchanged material/shadow decisions and no runtime copy of those arrays. The exact capacity removed is recorded by the existing load-timing diagnostic. Original model reader 14026a5a0 resolves surface material names to resource references at 140269670/1402d49b0; the host keeps the typed material handles in its render instances and shadow inputs.

Material override retirement acceptance (93aa6db5). Installed EotW and Dust District setup released 105,938 and 116,040 bytes of override vector and string capacity respectively. Both routes matched the previous accepted ordered placement signatures, material source/admission/resolution fingerprints and prepared asset counts exactly. The comparison covers actual CPU preparation, with no gameplay draw or whole-frame timing claim. Evidence: DH2-fx-resources/artifacts/bitmap-handoff-and-placement-retirement-20261110-placement.


### Loading movie bitmap lifetime

Prediction before change. The installed loader 1402ddc20 frees its upload staging, and 1402de790/1402deda0 remove the closing movie's pack reference. Give the host level-loading movie live ownership of its exclusive packs. Preserve the warm session scope for HUD, pause, Journal and readables. A warm consumer that acquires an existing live pack promotes that identity, with no duplicate read or decode. Independent cold packs can load on the existing I/O pool; warm lookups and character table publication stay on the calling thread.

Installed acquisition acceptance (6d450535). Four same-build windowless comparisons matched every ordered bitmap signature, all 290 loading-to-HUD bitmap identities and all 669 overlapping Journal identities. Closing the loading map reduced retained bitmap payload from 88,247,440 to 85,004,704 bytes. Reopening the loading screen read only its one exclusive pack again, taking 3.703–3.748 ms versus 0.034–0.037 ms warm. Journal bitmap reopen remained below 0.123 ms in all four samples. Independent cold-pack scheduling reduced first Journal bitmap acquisition from 72.269–83.283 ms to 30.715–30.742 ms in this dev observation. These measurements cover the bitmap acquisition calls and their CPU backing, not total menu latency or a gameplay frame. Submitted render frames own their bitmap maps until release. GPU texture cache limits and retirement remain unchanged. Evidence: DH2-interactions/artifacts/loading-ui-pack-lifetime-20261110-comparison.json. No movie was rendered and no visible game was launched.


### Preserve authored static model UVs

Original 14026a5a0 / 140259d50 reads the BML1 triangle records with half-float UVs. Keep UV0 in that encoding in CPU staging and copy the same bits into the packed vertex attribute. The baked and float-layout paths still widen to f32 where their output requires it. Expect four fewer staging bytes per vertex and removal of the widen/narrow pair for packed meshes, with the same GPU vertex layout and no draw-path change.

Installed acquisition acceptance (200e81fd). The EotW/Dust/EotW production setup comparison matched all 1,750,331 checked packed UV pairs to their installed source bits. Across the two maps compared with 93aa6db5, aggregate map model vertex/index staging fell 29,293,776 -> 27,130,188 bytes and 25,679,562 -> 23,501,666 bytes. The 15 acquired common static model signatures matched after normalizing both representations to f32 for comparison, with geometry capacity 883,962 -> 800,242 bytes. Prepared asset counts and resolved material signatures matched. These are staging and source-owner capacities, with no gameplay frame timing claim.

### Host performance overlay ownership

Prediction before change. The host FontAtlasSet outlives level preparation. The previous windowless sample held one late 1 MiB texture in a 256 MiB heap after map teardown. Create the hidden host performance overlay at startup instead of for each map. Keep the same fields and display settings and hide/reveal it at the loading boundary. Bevy's GpuImage updates matching atlas descriptors in place. A different display scale may require a separate atlas.

Windowless device acceptance (200e81fd). The 18 px overlay atlas is a 512x512 RGBA8 image. The map-scoped route first creates it after map preparation and retains a 256 MiB block for that allocation. The session route uploads it at startup and keeps the same asset and device texture IDs across all three EotW/Dust/EotW unloads. Reserved heap bytes after every unload fall 469,762,048 -> 201,326,592. Both modes retain exactly 11,075,584 live bytes, 14 images, four meshes and zero shader buffers and skies. Uploaded asset counts, material fingerprints and active reserved bytes match per map. This is a 256 MiB reduction in post-unload allocator reservation at the observed display scale. Active live bytes vary with slab placement. No game window, draw output or frame timings were evaluated. Evidence for both changes: DH2-fx-resources/artifacts/static-uvs-and-session-overlay-20261110-comparison.json.


### Wwise bank read backing

Original 1400bd510 selects the common, init and map packages and 1400c97e0 / 1400c0360 releases map banks independently. Sound sources refer to resident media at 1410f1910 / 141122400. The original EotW WWISE tag records 240,890,571 bytes. The host EotW observation reports 209,705,529 encoded bank bytes; embedded media ranges already share that allocation. Previously the package reader filled a Vec and converting it to Arc<[u8]> copied the entire bank. Read directly into the final shared allocation without changing bank selection, media ranges, playback or streaming. Expected benefit: one fewer allocation and copy per cold bank, with identical steady-state encoded residency.

Installed acquisition acceptance (c98c8ec6). Four clean same-build windowless EotW/Dust/EotW observations match all ordered bank IDs, byte lengths and CRCs, all 33,014 media and stream records per run, encoded residency, prepared asset counts and material fingerprints. The direct route eliminates 636,524,854 cumulative copied bytes on that route, including a 159,688,819-byte temporary allocation for the largest cold bank. Read-and-adopt thread cycles fall from 950–1,011 million to 572–615 million in the two dev samples per mode. The observation covers acquisition and asset retirement, with no audio playback, walking benchmark or steady-state RAM reduction. Evidence: DH2-interactions/artifacts/wwise-bank-backing-20261110-comparison.json.


### Compiled world UV staging

The original dispatcher 140295fa0 routes the installed BWM version word 0x00010015 to 1402974c0. Its earlier-version path is 140296240. The v21 reader uploads the section buffers through 1402d3550/1402d3aa0 using a stream view or reusable TAG_TEMP scratch, and uses vertex declaration 8 for the mesh descriptors. The installed 20-byte vertex attribute stream contains authored half2 UV0. The forward mesh already uses half2. Keeping the source representation in staging removes the widen/narrow pair and four CPU bytes per staged vertex, while the baked shear and diagnostic float output still widen when required.

Installed acceptance (380bade5). EotW/Dust/EotW prepared and uploaded 18,848,218 world mesh UV pairs that match their installed half bits, with no mismatch. EotW UV0 staging uses 20,585,928 instead of 41,171,856 bytes; Dust uses 20,869,224 instead of 41,738,448. Uploaded and retired GPU asset counts, static-model UV observations and material fingerprints match the previous installed route. Both routes return to 11,075,584 live device bytes and 201,326,592 reserved bytes on every unload. These are CPU staging capacities, with no permanent world-geometry CPU copy or gameplay timing claim. Evidence: DH2-fx-resources/artifacts/packed-world-uvs-20261110-comparison.json. No game window or test suite ran.


### Scene worker cancellation

Prediction before change. The host's existing audio-session close gate stops map parsing between phases. After the last parsing checkpoint, the real worker enters ScenePreparation and continues until scene materials, meshes, reflections and base images have all been prepared. The latest installed windowless sample spent 5.934 seconds in the EotW setup span and 7.308 seconds in Dust in the dev build. Extend the existing close signal into scene preparation, including the world and placed model loops, cube acquisition and base image reads. The signal belongs to the terminal host session, so abandoned entity reservations belong only to the retiring world. No partially prepared assets or commands may publish; drop them using the existing asset and command queue destructors. An archive read already running may finish. The expected result is fewer acquisitions and a shorter wait when the window closes during these stages. Successful sessions retain the same load and render behavior. Original world and resource retirement are 1408a4830 and 1400410f0; cancellation checkpoints are a host worker mechanism. Do not infer live close timings from a windowless successful preparation.


### Stream compiled world geometry into shared sections

Prediction before change. The installed BWM v21 dispatcher 140295fa0 calls 1402974c0, which uploads position, attribute and index sections through 1402d3550 / 1402d3aa0 before reading the ranged mesh descriptors. The EotW archive member is 122,270,692 stored bytes and expands to 203,394,146 bytes. The previous rewrite kept the whole expanded member while also copying each mesh's geometry. All 36 installed index records are compressed. Existing EotW, menu and tutorial mesh tables show contiguous stride-aligned ranges covering the geometry sections. Stream the installed member through the existing Catalog reader into one owned typed backing per geometry section, preserving the two half-float UVs and other packed attributes. Each mesh then holds ranges into that backing. Read the small record tail separately and discard it after parsing. Expect no whole decoded member allocation alongside the geometry, no repeated per-mesh geometry allocations and no geometry backing after GPU preparation. Preserve material, visibility, indices and vertex data. Confirm read/decode and geometry signatures, actual setup cycles, asset counts and retirement before acceptance. This is a host staging layout for the installed BWM v21 data.


### Packed mesh position staging

Prediction before change. Installed world/model geometry remains game-space data; the Bevy mesh has the existing view-space axis conversion. The packed builder currently allocates a Vec<Vec3> of converted vertices, then copies those same values into the position attribute's Vec<[f32; 3]>. Feed the converted values directly into the final attribute allocation. The existing EotW shared world meshes submit 6,112,973 packed position/UV pairs, so expect to avoid one short-lived position array per packed mesh, accounting for 73,355,676 cumulative temporary bytes over those world meshes, plus applicable placed models. Peak saving is bounded by the largest individual mesh. Preserve the exact coordinate conversion, index winding, baked shear and float-layout diagnostic. No live CPU/GPU memory or frame-time change is predicted. Compare installed uploaded counts and position signatures before acceptance.

### Vertex probe upload staging

The installed PNW reader 1402b15e0 supplies typed probe records. The host combines their typed SRV equivalents into one WGSL storage buffer. The previous host route grew a Vec<u32>, serialized those words, and copied the resulting bytes into the ShaderBuffer. Write the same record sequence straight into the final upload backing with the same offsets and GPU layout. ShaderBuffer still owns the normal Bevy transfer and retirement.

Installed acceptance (14b6a153). Four same-build windowless Edge of the World and Dust District observations matched the full encoded buffer size and CRC in both modes (227,179,736 bytes / bbc9941d and 302,742,124 bytes / 4fa1e3b9). All material signatures, uploaded asset counts and device retirement records match. The direct path removes a 435,159,184-byte or 589,459,960-byte word capacity in the two maps, and removes the separate serialized-to-asset copy. Final upload capacity equals payload size. Affected preparation cycles were 727–851 million -> 273–295 million in EotW, and 1,198–1,375 million -> 349–376 million in Dust. These are dev acquisition spans and allocation capacities, with no peak RSS, steady GPU memory or gameplay FPS claim. Evidence: DH2-fx-resources/artifacts/vertex-probe-upload-backing-v2-20261110-comparison.json. No visible game or test suite ran.

### World geometry release before base textures

Prediction before change. After water publishes the required game-space attributes, all world geometry consumers either own their prepared mesh data or use object, visibility and matrix metadata. The installed world upload path 1402974c0 uses a direct stream view or temporary scratch for each source section. The host retained them until the end of setup, overlapping the entire base texture acquisition phase. Drop the shared source section backing at the last CPU consumer. The installed Edge of the World and Dust District geometry sections total 194,098,770 and 195,731,094 bytes respectively. Expect this backing to be absent before base image reads, without changing prepared mesh or image data, reservations or gameplay lifetime. Peak process memory reduction requires measurement because unrelated allocations contribute to the peak.

### Probe vertex source retirement

Prediction before change. The installed PNW reader at 1402b15e0 supplies separate runtime tetrahedron positions and shader vertex positions. The host CPU query selects a runtime tetrahedron, uses its own positions for barycentrics and reads four packed weights from each referenced shader vertex. Only GPU packing and standalone audit output read the shader vertex positions. Split the parsed shader vertices into position and weight arrays and drop the former after successful upload backing construction, along with the unconsumed cell-to-probe source table. The decoded weights and runtime query algorithm stay intact. The EotW/Dust GPU descriptors imply 205,467/167,413 shader vertices, predicting 2,465,604/2,008,956 position bytes released. The table saving depends on the installed row count. The temporary standalone tooling uses both original arrays before release. Expect identical GPU bytes and material/query results, no added per-query branch and no new source reads. Compare complete installed PNW fingerprints, encoded GPU CRCs and post-retirement material signatures. Retain control is DH2_DEBUG_RETAIN_VERTEX_PROBE_SOURCES.

Installed acceptance (eef2b201). Same-build windowless retain/release acquisitions matched the full PNW source fingerprints, all 227,179,736 / 302,742,124 upload bytes by CRC, material handoff signatures, GPU asset counts and retirement in Edge of the World and Dust District. The released source capacities were 2,465,604 position bytes + 1,502,592 table bytes and 2,008,956 position bytes + 2,295,792 table bytes, respectively. The setup trace reported those capacities before drop. CPU queries still read the runtime tetrahedron positions and packed weights. No gameplay frame timing or process peak is inferred. Evidence: DH2-fx-resources/artifacts/probe-source-retirement-20261110-{retain,release}/stderr.log.

Installed acceptance (fdd41322). The streamed reader opened all 36 indexed worlds, matched their indexed decoded lengths, the independently decoded raw geometry signatures for Edge of the World, the menu and tutorial, and the separate Dust District section and record counts. Two same-executable buffered/streamed pairs per map matched geometry counts and CRCs. The streamed path avoids a whole-member allocation of 203,394,146 bytes in Edge of the World and 209,366,676 in Dust District. Read and parse spans were 656–674 to 616–624 ms and 675–696 to 647–650 ms, respectively, in the dev observer. These are archive read and parse durations, with no frame-time or peak process RAM claim. The real windowless D3D12 asset preparation and removal matched material signatures, upload counts, and the post-removal device allocator record. All 12,735,245 observed position and UV pairs matched their source-derived bits. The geometry release checkpoint precedes base image acquisition, and the CPU backing is no longer required by any later setup consumer. This verifies normal preparation and GPU asset retirement. Live close timing and draw output were outside this observation. Evidence: DH2-scene-shutdown/artifacts/world-section-streaming-v3-20261110-comparison.json.

Installed BWM capture and audit consumers now use the same bounded archive loader. The F8 report retains only the nearest-hit data after the ray query, releasing the temporary geometry before collecting the other diagnostics. The predicted capture-only saving is the same avoided whole-member allocation as level loading, with the existing ray query and report fields preserved. Tools that explicitly read a local BWM or export source bytes retain their file-input paths.

### Retire collision acquisition staging at its last consumer

Prediction before change. The installed compiled physics decoder owns independent triangles and material mappings after reading .bphysworld. The host currently retains the decoded archive member and geometry metadata while copying all world triangles into an intermediate position/filter tuple vector, then constructing the final query index. The installed Edge of the World and Dust District files decode to 37,930,026 and 46,325,954 bytes. Their 1,479,026 and 1,590,070 world triangles create 70,993,248 and 76,323,360 bytes of 48-byte intermediate tuples. Retire archive bytes and unused extraction metadata immediately after shape decoding and feed the same transformed triangles and filters directly into the existing index constructor. The final decoded geometry, original shape-tag handling (140853b80), query order, material indices, owner IDs and runtime query implementation are unchanged. Expect the intermediate tuple allocation to be absent and the source allocations to have shorter lifetimes. Compare the existing installed-map acquisition and query observations before integration. No gameplay CPU or peak process memory claim is predicted.
The installed-data observer can run the prior tuple allocation with DH2_DEBUG_STAGE_COLLISION_TRIANGLES=1. DH2_DEBUG_PHYSICS_SIGNATURES=1 reports the retained archive and tuple capacities, CPU thread cycles around construction, and a checksum of every final triangle's vertices, bounds, owner, material and filter. The checksum runs after the cycle counter and before any mutable interaction boxes are installed. Both modes use the same collision index and actual installed source files.
### Direct acquisition of separately streamed mips

Prediction before change. All 16,393 selected installed .bimage7_mip members exceed the existing archive buffer size, with 10,059 compressed and 6,334 stored directly. Decoded member extents run from 65,556 to 16,777,236 bytes. Original 14030e840 reads the five header words and hands the still-open stream to the upload worker via 14030f030. The host worker currently allocates and reads the whole member, then copies its payload into the completed queue entry. Preserve the authored header validation and read directly into the existing queue-owned payload allocation. Expect one fewer temporary decoded member per requested mip, with identical bytes, dimensions, next-suffix selection and queue handoff. No change to demand, GPU submission, or resident byte budgets is predicted. Compare scoped acquisition cycles and exact output on a matched installed inventory across all four suffixes and both archive storage modes before integration.

Installed acceptance (b482d770). The optimized game-feature tools build completed unchanged. The source census independently read all 16,393 indexed separate headers and 8,126 base images, finding contiguous suffixes and dimensions and matching all indexed extents. Four same-build acquisitions covered 70 independent raw payload samples across all four suffixes and all five packed formats, including all six members of the rarest format. Each run matched every payload CRC, extent and descriptor. The 70 requests cover 81,593,720 decoded member bytes, up to 16,777,236 for one member. The exact acquisition path uses a bounded archive stream and the final upload allocation. Total affected CPU cycles were 312.6–315.6 million buffered and 262.9–264.4 million streamed, with improvements for both compressed and stored members. The observation exercises the same payload owner called by the gameplay worker. Demand selection, asynchronous completion and GPU publication are unchanged and were outside this observation. Evidence: DH2-scene-shutdown/artifacts/separate-mip-streaming-20261110-comparison.json and separate-mip-header-census-20261110.json.
### Retire replaced scene images after the last view owner

Prediction before change. Original 1402df8c0 leases the current SRV through 1402bd380 and stores it in the named wrapper. Publication at 1402f5530 can replace the current view while that wrapper still holds its snapshot. The previous host scene commit removed the CPU descriptor after updating material slots even when a snapshot still leased the asset. The vendored render extractor retains the GPU texture until AssetEvent::Unused. Drop only the scene handle at the existing commit point so the descriptor follows the same last-handle lifetime as the GPU view. The shared registry and particle consumers already update current views independently. Unleased descriptors and textures retire through Assets::track_assets and the existing render removal events. No new registry, pin, GPU copy or per-frame scan is needed. The game-feature compilation check passed on 61cf1407. This is source and compile verification of handle ownership. Live UI replacement was outside the observation. Evidence: DH2-interactions/artifacts/scene-image-leases-20261110-check-provenance.json.

### Keep collision decoder detail on its audit callers

Prediction before change. Original compiled physics registration is 141388640, with shape-key and material lookup at 1400922d0 and 140853b80. The host retains the current decoded runtime triangle index for query cost. During level preparation the existing decoder also saves every convex vertex vector, transformed compressed-shape bounds and reverse shape-to-body list, although the client only uses triangles and summary counts. The existing installed loads generate 82,846 and 107,123 convex records and 1,132 and 1,457 compressed records in Edge of the World and Dust District. The convex records alone occupy at least 2,651,072 and 3,427,936 bytes of 32-byte record storage, excluding vertices and reserve capacity. Count successful shapes in the navigation acquisition and release per-shape scratch after triangulation. Keep full audit output for explicit shape inspection. Decode, failure rollback, triangle order, authored filter/material assignment and runtime queries must match. Compare the same installed levels with and without the detail tables, measure capacities and decode-thread cycles, and compare all final triangle data. No frame-time or process-peak claim is predicted.

The optimized same-executable comparison (`d69007b6`) matched every final
triangle signature, material table and navigation count in both orders on
Edge of the World and Dust District. The direct path has no retained source
member at index construction (formerly 37,930,027 and 46,325,955 bytes of
capacity) and no transformed tuple staging (70,993,248 and 76,323,360 bytes).
It also omits 15,479,532 and 17,700,024 bytes of extraction-detail capacity.
These counters measure different stages and are not additive peak RAM.
Decode plus index cycles averaged 2,885.3 to 2,915.4 million in Edge of the
World (+1.0%) and 2,179.4 to 2,003.1 million in Dust District (-8.1%). Paired
samples varied in both directions, with index-only means 3.1% and 2.5%
higher, respectively. Runtime queries and frame timings were outside this
acquisition comparison. Evidence: DH2-collision-staging/artifacts/collision-lifetimes-20261110-comparison.json.

Palace convex extraction prediction. The buffered high-chaos Palace run
spent 1,223,622,028,106 thread cycles in shape extraction. A sample of the
same executable's direct run located 9,209 instruction-pointer samples in
the five instructions of the convex horizon's reversed-edge linear search
(RVA 0x34f2704–0x34f2714). The large horizon path will index edge positions,
preserving the first reversed match, swap-removal order and emitted triangles.
Small horizons keep the linear scan. Expect the large-horizon search to
cease dominating Palace decode cycles, with unchanged triangle identity and
no extra allocations on the small-horizon path. Original 14127f590 dispatches
the authored primitive. Hull triangulation here supplies the host query
representation. Compare same-build extraction cycles and exact collision
signatures before integration.

Installed collision acceptance. The menu and high-chaos Palace comparison
(`d69007b6`) matched all geometry signatures, material tables and navigation
counts. The Palace path no longer constructs the 190,361,808-byte tuple
vector or retains the 23,525,626-byte decoded member at index construction.
It also omits 13,547,616 bytes of audit detail capacity. The old linear hull
search dominated Palace in both ownership modes. In the unchanged optimized
executable `c9ada5e4`, the indexed and linear searches produced the same
triangle signatures as the preceding builds, with matching authored counts,
placed collision, materials and navigation counts. Palace extraction cycles
fell from 1,345.58 billion to 6.82 billion (about 197 times lower). Edge of
the World measured 1.28 billion to 0.98 billion, with one run per setting.
Palace index construction stayed at 1.55–1.56 billion cycles. These are
windowless acquisition measurements, without gameplay CPU or peak RAM
claims. Evidence: DH2-collision-staging/artifacts/collision-lifetime-extremes-20261110-comparison.json
and DH2-collision-staging/artifacts/convex-horizon-20261110-comparison.json.

Placed model mesh preparation prediction. The host shares a nondeforming
BMD6 mesh by model, surface, mirrored winding and skin influence count.
Its rest palette is shared by model, and `apply_rest_pose` uses no placement
transform for model-local meshes. The current setup still builds the four
skin input arrays and applies the same rest pose again for each handle reuse.
Move those writes to creation of the mesh. Baked and deforming meshes keep
per-placement preparation, and vegetation still admits its channels by the
actual material. Static BML1 shadow positions likewise attach at mesh creation.
Expect one fewer initialization per shared handle hit, with identical final
vertex and index bytes. This is a host consequence of preparing the shared
geometry from the BMD6 source (14025aa00); it does not move runtime skinning
or change GPU residency. Measure the affected setup work before integration.

Installed acceptance (`a813a5bf`). Two pairs of optimized runs from the same
executable matched final vertex/index fingerprints, instances and mesh counts.
Edge of the World dropped from 6,259 to 908 initializations and from
136.8–138.7 million to 131.8–133.4 million affected cycles. Dust District
dropped from 5,313 to 961 initializations and from 137.5–139.4 million to
121.9–123.2 million affected cycles. These are mesh initialization counts and
scoped setup cycles. Frame timing and live GPU memory were outside this
observation. Evidence:
`DH2-collision-staging/artifacts/placed-model-mesh-v2-20261110-comparison.json`.

### Direct acquisition of whole inline image payloads

Prediction before change. In the recorded Edge of the World / Dust District / Edge of the World acquisition, 11,838 whole-image reads used only indexed inline records; 7,421 of those members exceeded the archive stream's 65,536-byte buffer. The largest indexed member was 16,777,298 bytes. The current 2D resident path reads each complete member and copies its records into a second Bevy Image allocation. Original 1402e9ff0 sends authored mip records to 1402f5500. For large inline-only resident images, validate the same headers and read the authored bytes into their final upload allocation. Expect no full decoded member allocation on that route, lower or unchanged affected CPU cycles, and matching upload CRCs, descriptors and GPU retirement. The stream scratch bounds temporary member staging; the required Image allocation remains. Compare scoped acquisition cycles and output in the same executable before integrating. No gameplay or process peak claim is predicted.

Optimized acceptance rejected the read-direct variant (0ae65b15). Four same-executable observations matched 8,129 whole-image acquisitions and all mip descriptors, payload CRCs, material handoffs and GPU asset counts. The streamed reader avoided full-member temporaries but consumed 2,482–2,764 million versus 2,000–2,057 million affected cycles in Edge of the World, and 2,099–2,137 million versus 1,804–1,824 million in Dust District. Retain the faster archive acquisition call. Prediction for the replacement: validate the same indexed mip payload ranges, then compact the owned member in place using the same range compaction as layered images. This removes a second full-sized allocation, preserves one payload move, and avoids the per-image buffered decoder and output initialization of the rejected variant. Header slack is bounded by 62 + 20 times the mip count (plus archive read capacity). Keep the same large inline-only scope and baseline for a matched repeat. Evidence: DH2-fx-resources/artifacts/resident-inline-image-streaming-20261110-comparison.json.

Installed compaction acceptance (99ea2165). Four same-executable optimized acquisitions matched all 8,129 prepared image payloads, format/extent/mip selection and CRC, material handoffs, and GPU upload/retirement counts. The 5,039 eligible inline reads per two-map route reuse the decoded member allocation as the upload allocation, removing a second overlapping buffer. The largest eligible member was 16,777,298 bytes; the old separate payload was 16,777,216 bytes. Affected Edge of the World cycles were 1,894–1,899 million buffered and 1,818–1,839 million compacted. Dust District measured 1,707–1,720 million buffered and 1,670–1,739 million compacted. The two-map acquisition total fell 1.7% and 2.5% in the two paired observations. Individual Dust samples vary around parity. These are scoped acquisition cycles and temporary allocation capacities, with no frame-time or peak RAM claim. Evidence: DH2-fx-resources/artifacts/resident-inline-image-compaction-20261110-comparison.json.

### Runtime material batch publication

Prediction before change. The host resolves material identity and purge through
1400607f0 and 140060180. Its exclusive publication currently snapshots all live
leases for rollback, yet a failure after an earlier binding cannot undo edits
to a shared material asset. Resolve the complete slot, image, material and
dependency mapping before any live asset or cache changes. Then publish the
same ordered batch. Failed preparation should leave the preceding bindings
and owners intact, and successful preparation should stop cloning their
rollback snapshots. This is host transaction ownership. The installed
dynamic FX route is needed to measure the affected publication work.

Installed usable-family acquisition (`ff26580f`): all 12 declarations under
`models/effects/usable/` resolved. Only the toilet flush acquired a particle
model. Its authored programs are `particle/add_default` and
`particle/dist_blur`, which keep it on the particle renderer, and none of the
12 declarations has a decal group. Edge of the World and Dust District both
reference the flush and sink definitions. These are candidates for observing
particle image, model and sound release. A world-material or decal batch needs
an installed definition with the corresponding dependency. No live usable
activation, proxy publication or image retirement is established by this
acquisition. Evidence:
`DH2-interactions/artifacts/runtime-fx-installed-usable-sources-20261110.json`.

Named acquisition (`a813a5bf`) found two installed inputs for the remaining
publication observation. `dirt_flies_few.fx` acquired two model resources and
one particle material. Its mesh declaration pairs `dirt_flies_body.pmodel`
with an `arkDefault` material. `blood_splat_01.fx` resolved a projected decal
definition. Sword impacts and the other sampled model candidates are weaker
inputs because they acquired no model records. These are acquisition results.
Publication, pending-image failure, and final release are still separate
observations. Evidence: `DH2-interactions/artifacts/runtime-fx-model-stage-source-acquisition-20261110.json`
and `runtime-fx-material-route-source-acquisition-20261110.json`.

The windowless runtime FX observer requests installed sources in a prepared
level and drives the existing material coordinator, texture read worker,
upload mailbox and asset retirement systems. The input is a resource request,
with no interaction or simulation. It identifies FX-only GPU images and
meshes, and newly published decal material assets. Expect those identities to
retire after both request leases end, while the loaded level stays alive. A
camera-specific decal pipeline, active effect tails, and failure recovery are
outside this observation.

The first run (`cce7d92d`) resolved one world mesh and four decal material
requests in both maps, then the production coordinator reported that the
scene catalog was absent. Setup had moved the only `SceneData` reference into
the texture worker, although late FX requests still use `SceneData`.
Keep the scene lease and give the worker a clone of the same Catalog Arc.
This should restore late acquisition with no second index, archive cache,
or payload read. Both leases end at their existing level-owner boundaries.

Installed publication and retirement (`f136cef7`): both EotW and Dust
published four blood-splatter decal materials through the runtime coordinator.
EotW published and retired seven FX-only GPU images. Dust published and
retired six and reused one existing scene image. Both reused the dirt-fly mesh
already owned by their scene. A Dunwall Tower request for `stink_bug_running_sub.fx`
created and retired one new world mesh and two GPU images. The trace observed
the proxy and its world material binding retire before those image slots. All
runs exited successfully after release. They exercise actual successful
publication, sharing, and disposal of the observed assets. They do not cover
failed input, in-flight read cancellation, or camera-specific material readiness.
Evidence: `DH2-fx-resources/artifacts/runtime-fx-publication-v2-20261110-comparison.json`
and `runtime-fx-publication-v2-20261110-tower-stinkbug/result.json`.

### Runtime FX image binding retirement

Prediction before change. A dynamic material proxy retires after its final effect or tail lease, under the map material identity/release boundaries 1400607f0 and 140060180. RuntimeMaterialCoordinator publishes only the surviving surfaces to DynamicWorldMaterials. The host image synchronization previously removed absent bindings only inside its replacement-image readiness gate. While an unrelated image read is pending or fails, the stale world_materials entry can hold the old image's Arc and prevent its streaming slot and GPU lease from retiring. Drop absent dynamic bindings on the new surface revision, independently of incoming-image readiness. Existing tail and render-frame leases still defer the actual slot release, and startup keys are excluded from the dynamic entry set. Incoming binding publication retains its readiness gate. Expect the expired binding to release before a new pending/failed read completes, with the same ready materials and no new work on unchanged revisions. Live cancellation and tail evidence remain required; the runtime trace reports the binding release and the existing image-slot retirement.

### Abandoned runtime texture reads

Prediction before change. The original image owner limits residency by live resource demand and releases it through 140060180; its mip budgeting path is 14030f3d0. The host tracks FX-only slot leases separately from the shared scene. Before this change an orphan slot waited for its read, but the completed read still created an image and enqueued GPU upload. Cancel queued reads for FX-only slots once their final effect or tail lease ends. For a blocking read that already started, keep the slot assigned until its completion is acknowledged, discard the payload outside the upload budget and release the outstanding read reservation. A revived consumer of the same slot before completion may still use the result. Ordinary scene slots and already submitted GPU work keep their current ownership. Expect no new upload for a read abandoned before submission, with unchanged live texture output and no read cancellation based only on camera motion. Live FX tracing is still required for this ownership edge.

Collision cancellation prediction. The closing audio session waits for the
map worker's lease, and the former checks bracket the whole collision phase.
An interrupted map should stop visiting authored shapes and placed bodies,
or building the host cell index, at their next work boundary. No partial
world may publish. Ordinary loads must preserve the recorded triangles and
materials, with no checks on gameplay queries. An archive read already in
progress can finish. This bounds host worker retirement at the map lifetime
boundary (1408a4830). Measure the cancellation span separately from ordinary
load costs.

The optimized installed-data observer at c40dcdbe requested cancellation
during shape extraction, placed-body acquisition and index construction in
high-chaos Palace. Each used the same preparation cancellation callback at
its 64th checkpoint. Time from the stop signal to a returned cancellation
was 93.894, 110.464 and 118.288 ms respectively, including disposal of the
unpublished partial data. All three observed the subsequent catalog purge,
with no prepared world returned. Ordinary EotW/Dust preparations retained
the recorded triangle signatures. The observer did not close a live window
or measure a blocked archive read. Evidence: DH2-scene-shutdown/artifacts/collision-close-and-motion-20261110-comparison.json.

Autonomous motion event ownership. The host now borrows the selected event
node, its parameters and its ports from the loaded graph while mutating
only the playback, motor and variable owners. This removes one temporary
definition copy per dispatched event. The original keyframe action
140533460 reads its authored declaration identity and stores a separate
playback handle at +0x30. The existing node lookup order and dispatch rules
are preserved. The same-executable comparison observed 36 dispatch steps
in eight graphs across EotW and Dust in every mode. Definition and initial
pose signatures, per-graph event and error counts, and collision signatures
matched. Scoped dispatch totals were 740,880–746,550 cycles with copies and
547,680–555,905 with borrowed nodes. This measures those startup dispatches,
without a gameplay frame-time claim. The source was c40dcdbe and the same
comparison artifact records both batches.

Static model acquisition prediction. The original BML1 loader 14026a5a0
feeds the model stream to 140259d50 and closes it after constructing the
surfaces. The host currently inflates the entire member while also building
the typed position, packed attribute and index arrays. Use the Catalog
stream for installed members of at least 256 KiB, with 64 KiB of archive
buffering and an 8 KiB stack block. Expect to remove one member-sized heap
allocation per qualifying cold acquisition. Keep small and editable sources
on the buffered route and keep packed channels, geometry, bounds, bindings
and reuse policy unchanged. Compare decoded fields and acquisition costs on
installed models before integrating the change.

Installed static model comparison (`fb704c39`). The independent index and
layout census found 102 qualifying BML1 members, all compressed, with 258
surfaces and 1,315,706 vertices. The optimized tool compared buffered and
streamed decoding twice and matched all model fields in all 102 resources,
with no unsupported members. The largest decoded member was 2,226,130
bytes. It no longer overlaps the typed mesh arrays as a whole-file buffer.
Scoped elapsed read/parse totals were 138.686–142.428 ms buffered and
129.899–131.034 ms streamed. Both routes use the same model parser. The
comparison does not measure peak process memory, whole-map load time or
rendered output. Evidence: `DH2/artifacts/static-model-stream-index-20261110.json`
and `DH2-fx-level-lifetime/artifacts/static-model-stream-20261110-comparison.json`.

Shared point-query owner acceptance (91584611). A windowless production
scene observation prepared Edge of the World, Dust District and Edge of the
World again. Gameplay and the particle query publication acquired the same
level owner in all three preparations. The geometry held 332/259/332 bodies,
256 tree nodes and 640 leaf blocks per map, with 332/259/332 shape records.
Frame publication now passes a lease instead of copying these inputs. Body
centre wind/water/ray queries produced identical signatures on both visits
to Edge of the World. All three scenes retired through the trigger and
acoustic shape teardown paths. This observes the shared path and repeat
acquisition. A direct comparison with the removed snapshot, active observer
detach, frame timing and rendered particle output were outside the observation.
Evidence: artifacts/shared-level-point-queries-20261110-comparison.json.

Ocean height readback prediction. The selected-slot consumer 140304c40
waits, maps, copies straight into the persistent CPU ring, then unmaps.
The host's asynchronous callback currently allocates a temporary vector and
the consumer copies it into the persistent ring. Keep the existing queued
mapping and exact slot selection, but let the callback publish readiness
only. Read the mapped prefix into the existing CPU allocation and unmap it
on consumption, ring reuse or owner teardown. Expect zero transient sample
allocations and one CPU copy per consumed sample, with no new GPU work or
wait. A dropped owner must cancel pending maps without publishing results.

Level cleanup after an FX shutdown error. The host's 1408a4830 transition
returned immediately if a particle chain reported an invalid handle. That
skipped entity and asset release, the publication reset and 1400410f0's
resource purge. Preserve the first FX error and the existing abort of the
destination load, but run both player and world shutdown and always release
the outgoing owners. Expect the same order and no additional work on a
successful transition. This covers fallible teardown, without trying to
restore gameplay after a corrupted chain.

Packed skeletal UV prediction. The installed BMD6 loader 14025aa00 calls
140259960 for the authored vertex streams. Declarations 9 and 10 use two
half-float UV sets. Host decode kept UV1 packed but widened UV0, which the
normal mesh builder immediately packed again. Keep UV0 packed in ModelInfo
and pass it through the existing packed mesh builder for placed and arm
models. Predict four fewer bytes per decoded vertex and no UV0 widen/pack
round trip for ordinary meshes. The sheared and diagnostic float layouts
still widen at their consumers. Preserve the established decoded signatures
and compare the installed GPU attribute bits and acquisition costs.

Packed skeletal UV acceptance (`97dc2212`). Windowless scene preparation
matched the previous complete vertex/index fingerprints for 908 placed
meshes in Edge of the World and 961 in Dust District, including their packed
UV bytes. The parser now retains four fewer bytes per decoded vertex.
Normal arm preparation calls the same packed mesh builder. Arm draw output,
scoped UV decode timing and peak RAM were outside this observation.
Evidence: `DH2-collision-staging/artifacts/view-and-model-resources-20261110-model-comparison.json`.

Screen-shadow publication prediction. The host level teardown corresponds
to 1408a4830 and the 1400410f0 resource purge. The standard Bevy extractor
copies the optional ScreenShadowFrame when present but leaves the render
copy in place when the main-world source disappears. That copy holds the
outgoing visibility output, cloud and optional AO depth image handles even
after ScreenGpu releases its cached bindings. Withdraw the main publication
when its camera stops and remove the extracted frame when its source is
absent. Expect one fewer outgoing frame lease and retirement of its private
output before the next map's acquisition. Shared resident images remain
with their own owners. The ordinary changed-frame extraction stays the same.

Screen-shadow publication acceptance (`84c3fbe7`). On one windowless DX12
host, Edge of the World, Dust District and Edge of the World each extracted
the authored screen-shadow frame and uploaded its private output. All
three releases removed the render-world frame and the exact CPU and GPU
image identities. No GPU validation errors were reported. This observes
the real setup and removal owners using the prepared 1×1 output. An active
full-size draw and live handoff timing were outside this observation.
Evidence: `DH2-screen-shadow-retirement/artifacts/screen-shadow-retirement-20261110-comparison.json`.

Owned declaration text extension. The 140135f70 lexer treats signed bytes
as unquoted whitespace and preserves quoted/commented bytes. The measured
contact-only handoff already avoids copying ASCII source. Extend ownership
transfer to installed declaration consumers that already own their archive
read. Normalize in that allocation and transfer it into the String when
UTF-8 validates. Malformed quoted bytes keep the existing replacement
behavior. Expect one fewer whole-text allocation for valid owned reads
and matching parsed definitions, with no new retained source owner.

Installed decoder comparison (`a7043617`). The previous decoder and both
new entry points produced identical text for 1,593 installed declarations
sampled across 511 index kind names. The sample includes 24 non-ASCII
members and three whose decoded bytes changed from the source. Owned
decoding kept the input allocation for 1,590 members covering 7,056,074
cumulative source bytes. This isolates the decoder and does not measure
full loading time or retained process memory. Evidence:
`DH2-fx-resources/artifacts/owned-declaration-layout-20261110.json`.

Camera depth owner acceptance (`e40c02cf`). Two windowless DX12 observations
released the 1920×1080 opacity image, reduction pyramid, coarse output,
constants and four staging slots with a readback map outstanding. The callback
had zero leases after cancellation. The fence left one 65,536-byte coarse
output and one 65,536-byte staging allocation in submitted device work.
The next empty submission and drain retired both. Live allocations then
returned to the same 655,360 bytes of labeled wgpu device resources on both
samples, with no validation errors. The allocator reserved 268,435,456 bytes
after warmup. This observes ownership and queue retirement, without measuring
active rendering cost. Evidence:
`DH2-collision-staging/artifacts/camera-depth-allocation-labels-20261110-comparison.json`.

Projected decal replacement prediction. The effect chain releases its final
tail through 140199df0 and the resource purge follows 140060180. The host
coordinator already removes expired decals, but the prior publication and
proxy set still hold the same materials and images while a replacement is
waiting for its reads. Withdraw expired entries from that publication on
the retirement revision and let the proxy owner prune without admitting
new decals. Keep all surviving effects and tails. Expect the retired image
and geometry leases to end even if an unrelated replacement has not reached
readiness. No change to decal program, sampling, geometry or draw ordering.

Projected decal replacement acceptance (`e8697d4f`). The installed
Dunwall Escape blood effect published four material variants, ten receiver
proxies and seven new image slots. The material worker had a different
installed effect pending when all ten outgoing proxies and all four
prepared materials released. Superseding that pending generation discarded
its result. Final release left zero of the tracked assets and all seven
slots were free. The windowless observer used the actual publication and
removal systems with cameras inactive. It did not measure active decal draw
appearance or the duration of rendered tails. Evidence:
`DH2-fx-resources/artifacts/decal-replacement-retirement-20261110-comparison.json`.

Loading fade prediction. The installed executable tests the initial-load
flag at `1408aabff`, reads one declaration pointer and its duration from
`+0x3b8/+0x3c0` or `+0x3c8/+0x3d0`, and submits only that selection to
`140166f60`. The host worker decoded both curves before making this choice,
then retained them after playback. Prepare only the phase selected by the
existing loader state and drop it after its last sample. The archived Edge
of the World load decoded the same declaration twice, for 4- and 2-second
durations. Expect one declaration read and decode per load, the same
phase-specific duration and samples, and no retained curve during play.
This changes loading ownership, with no gameplay timing gain predicted.

Map source history prediction. The map owns its BWM, entity, Tome and
physics companions from acquisition until teardown (`1408a4830` /
`1400410f0`). The host releases their payloads but the generic archive read
history still retains the old paths. Retire the exact map identities, using
their indexed kinds, after the prepared or published level is dropped.
Keep the dedicated probe declaration lease and all shared histories with
their current owners. Predict one retired read entry per acquired companion,
no additional file I/O and unchanged acquisition on return to a map.
This corrects loading coverage and has no measurable gameplay memory target.

Placed static model staging prediction. The BML1 loader retires its input at
14026a5a0 and releases dispensable triangle arrays through 140278df0. The host
needs decoded level triangles while resolving placements in authored order.
The last placement that uses each source is known before setup. Record it in
the same source table and drop level triangles after that placement, leaving
session sources with the catalog. The archived Edge of the World load kept
30,145,962 bytes of combined static and skeletal geometry until the end of
setup. Expect lower peak live static geometry within this scope, one source
acquisition per path, identical mesh bytes and placement order, and no change
to gameplay CPU. Compare the placement loop including the new table pass.

Placed static model staging acceptance (`c285c906`). Two optimized
retain/early-release pairs matched all prepared vertex and index bytes
and their ordered mesh references. Each map decoded the same source count
and byte total. Peak static CPU staging fell from 20,588,490 to 4,298,220
bytes in Edge of the World and from 20,741,684 to 6,323,080 in Dust
District. Both released paths had zero static staging left at loop exit.
The affected placement span averaged 3.292B cycles with retention and
3.286B with early release across both maps. Individual Dust runs varied
in both directions. No frame timing was measured. Evidence:
`DH2-collision-staging/artifacts/static-model-last-use-20261110-comparison.json`.

Common model scope follow-up. The installed Edge of the World / Dust
District / Edge of the World publication requested 13 / 11 / 13 common
model sources. Adjacent maps shared nine identities and 483,146 bytes of
decoded source geometry. Their session union was 15 sources and 800,242
bytes. These are CPU source sizes, not GPU buffer sizes. The host mutates
mesh vertex layouts for vegetation during preparation and packs world
meshes into shared GPU slabs. Extending these mesh handles across maps
would need an immutable layout key and separately scoped slab allocation.
The current host keeps CPU source reuse and per-map GPU retirement until
that separate layout and allocation path is measured. Evidence:
`DH2-fx-level-lifetime/artifacts/loading-manifest-publication-20261110.log`.

Runtime material snapshot prediction. Material lookup 1402d49b0 and release
140060180 keep the registered material separate from the task that resolves
it. The host worker clones existing material values to resolve a request.
Its completed batch only publishes newly reserved values and reads existing
values from the live world. Drop the worker's cloned values after resolution,
before image admission or a wait for publication. This releases their image
and buffer references while preserving live material IDs and their resolved
roles. The empty projected-decal scene also clears the previous projection
lighting rows while keeping reusable device buffers. No gameplay sampling,
archive re-read, additional GPU allocation or frame timing gain is predicted.

Probe index upload backing prediction. The installed PNW loader 1402b15e0
passes packed shader reference words to the GPU buffer constructor. The
host now streams the archive, but it still builds a typed index array and
copies it into a separate WGSL byte buffer. Installed acquisition reports
54,300,976 index words in Edge of the World and 73,538,998 in Dust District.
Read their little-endian bytes into the eventual upload allocation. Reserve
the prefix from the already loaded world instance count and the suffix
from the PNW tables. Use the actual parsed offset count for the header and
relocate in place if it differs. Expect removal of 217,203,904 and
294,155,992 overlapping index bytes, with matching full upload CRCs and
unchanged runtime query data. Compare the affected read/pack cost and
asset removal. This changes host staging, not the WGSL layout.

Probe index upload backing acceptance (`eb285fd3`). Two windowless DX12
copy/direct pairs matched PNW fingerprints and full upload CRCs `bbc9941d`
and `4fa1e3b9`. Direct transfer reused the full backing on both maps, avoiding
217,203,904 B of overlapping index storage in Edge of the World and
294,155,992 B in Dust District. The measured upload packing step fell from
142.5–152.7M to 6.6–7.1M cycles and from 182.4–187.8M to 6.1–6.4M cycles,
respectively. PNW input CPU samples overlapped or differed by one scheduler
tick. Both maps returned to 15 GPU images, four meshes and zero buffers after
retirement. No frame timings were measured. Evidence:
`DH2-scene-shutdown/artifacts/probe-index-upload-backing-20261110-comparison.json`.

Screen-shadow binding replacement prediction. The original visibility and
bilateral tasks bind the active view resources and reuse compiled programs
(`14032c7f0`/`1402fcff0`, installed screenshadows and PostFX_DWAOBlur).
The host visibility bind groups also hold the parallel-sampling buffer and
AO scratch view. Include both identities in invalidation so replacements
release the previous bindings and cannot keep sampling an old buffer. Keep
one compiled pair per device owner for the last producer state and depth
format. A resize, level change or input swap with identical formats and
state should recreate only the dependent bindings. The AO and depth-unhack
owners also compile when replaced. Their independent reuse needs separate
format and program ownership evidence.

Renderer owner acceptance (`b1309447`). The windowless DX12 readback used
the gameplay staging owner. Five selected-slot results matched the direct
map result, the four staging buffers totalled 65,536 bytes, and the queued
callback held no lease after cancellation. Installed Edge of the World,
Dust District and Edge of the World retained both visibility pipeline IDs
across the owner release call. All three scenes completed ordinary FX
shutdown and released the extracted screen-shadow image again. Active
particle shutdown errors, live resize and replacement binding draws were
outside the observation. Evidence: `DH2-fx-resources/artifacts/render-owner-batch-20261110-comparison.json`.

Map request table retirement follow-up. The original 1400661e0 frees its
assembled request table at the end of loading. The host drops its loading
lease there, but an unused inventory snapshot kept a second LevelList Arc
in the session. The archived EotW release line reports 46,546 entries,
8,628,721 logical bytes across the three sections and two strong owners.
Remove the unused snapshot and its candidate scan. Initial inventory and
readables still use the selected collection declaration, and discovered
journal items retain their own identities. Expect one loading owner before
play, zero surviving request-table owners after the gate, and release of
the map-only descriptor section. Init/common still have their own owners.

Windowless publication exposed an additional observer lease. The timing
coverage reporter had stored a separate map-list reference for its PostUpdate
report, while this acquisition observation calls the play gate directly. All
three publications dropped from two owners to one. Route the same coverage
calculation through the request list retired at play start and remove the
independent timing acquisition. Expect one to zero owners with timing enabled
and the same report content for the reads recorded before handoff. The isolated
map sections measured 4,562,689 bytes in Edge of the World and 4,508,068 in
Dust District. This counts descriptor ownership, rather than process memory.

Instance probe record residency prediction. The original indirect-lighting
owner retires buffer backing in 14035bca0. The host currently preserves
the complete serialized probe buffer for byte comparisons on moving
models and arms. Those writers use only records appended after static
world geometry. Keep that contiguous writable tail with exact initial
values and let extraction take the complete upload allocation. Keep the
same asset ID, GPU bytes and patch order. Expect to release the static
prefix after upload, with the same per-record compare and writes as the
previous path. The existing EotW and Dust full CPU copies are 14,902,560
and 21,835,296 bytes. Measure the actual retained tail and output CRC.

Instance probe record residency acceptance (`acca51c7`). Installed Edge of
the World and Dust District produced the same full upload CRCs `089f05a5`
and `d496779e`, with unchanged GPU buffer lengths. CPU backing went from
14,902,560 to 902,592 bytes and from 21,835,296 to 766,368 bytes. The
main-world asset retained no serialized upload payload after extraction;
the separate mutable tails remain on the model lighting owner. Both maps
returned to zero GPU buffers after release. Active motion and arm patches
were traced in source and were outside the windowless observations.
Evidence: `DH2-screen-shadow-retirement/artifacts/probe-record-residency-20261110-comparison.json`.

Named UI image release prediction. The installed named wrapper keeps its
SRV through 1402df8c0 and binds it through 1410150f0. In the host, the
extracted frame already holds that snapshot's scene asset handle. Flash
also cloned the GPU texture and view into its bitmap cache, where a frame
with no draw batches or no camera could leave a retired named view pinned.
Bind named images from the existing GPU asset, so only the wrapper, frame
and submitted GPU work own that snapshot. Keep the bitmap, glyph and ramp
cache for textures Flash creates. Expect unchanged view/sampler selection,
no extra upload and no named-view cache entry or per-batch name allocation.

Named UI image acceptance (`a6db27a8`). The game-feature Tools build
completed, and the real UI image owner used the committed view for both
installed resources observed. A second wrapper request reused the first
snapshot after replacement. The old wrapper had zero owners after release,
and a later request received the new view. The fixed authored mip view and
its payload CRC also matched the previous acquisitions on both resources.
The renderer binds from the extracted handle and retains no named cache
entry. These observations cover wrapper identity and CPU view selection.
Active movie drawing was outside the windowless observation. Evidence:
`DH2-screen-shadow-retirement/artifacts/ui-registered-image-leases-20261110-comparison.json`.

Fog and shadow extraction prediction. Level destruction at 1408a4830 and
resource purge at 1400410f0 separate world views from device programs. The
host removes FogMaterialResources and both shadow sampling resources from
the gameplay world, but Bevy's standard extraction keeps the last render
copy when a source disappears. These copies hold exact map-owned image and
shader-buffer handles, including the fog attachments. Extend the existing
screen-shadow absence handoff to these three sources. Expect their extracted
copies and private handles to retire after map clear while still forwarding
changed bindings during play. No texture upload or draw-path change.

Verified combined fog, fade and map-source retirement at 05e8fbad. The windowless DX12 route acquired Edge of the World, Dust District and Edge of the World in one host and exited zero without validation errors. Each publication acquired five fog/sampling images and four buffers. Each withdrawal removed all nine identities from CPU and GPU asset stores and cleared the three extracted owners. The screen-shadow image also released on all three transitions. Exactly five map-source history entries retired per transition. The installed fade acquisition returned one declaration per load, using the four-second initial fade and the two-second subsequent fade on both transitions. The route observes resource preparation and removal. On-screen fade playback and fog draw output were outside its scope. Evidence: DH2-screen-shadow-retirement/artifacts/fog-fade-map-source-ownership-20261110-comparison.json.

Particle scene-image retirement prediction. Original image publication
1402f5530 and residency 14030f3d0 own the active texture independently of
particle bindings. The host GPU particle cache retained a strong reference
to the previous map of scene image handles and its view objects. The final
effect can end with no draws, bypassing image synchronization until a later
effect starts. Track publication identity weakly and remove absent views
when the particle pass is idle or preparing a changed publication. Active
materials and submitted work keep their own leases. Retained path view
changes still use the existing GPU-ready synchronization. Expect no uploads
or view rebuilds for unchanged publications and no retired handles pinned
by an idle particle pass.

Probe sample staging prediction. PRB result reader 1402b8b30 reads 128
samples per record. The installed Voronoi query 140382d90/140383d80 uses
samples attached to its primary cell. The host builds equivalent query
quads, then discards the samples. Resolve cells by their installed probe
names before loading and build each cell while its primary PRB is in scope.
Preserve cell order and first occurrence matching for duplicate probe names.
Skip sample decode for secondary and alternate results where no query reads
those arrays, and skip materializing strings in already ignored record lists.
The prior route held 20,348,928 and 17,356,800 primary sample bytes in Edge
of the World and Dust District. Expect at most one primary sample vector
live in this part of setup, identical query geometry bits and lighting
uploads, and lower scoped CPU cost from removing the per-cell linear scan.
The standalone complete PRB/PNW readers still expose their full records.

Integrated ownership acceptance (`116a0d59`). Windowless Edge of the World,
Dust District and Edge of the World each transferred the final manifest
lease from one owner to zero while retaining the coverage report. Their
map-only descriptor capacities were 4,562,689, 4,508,068 and 4,562,689 bytes.
An installed FX publication supplied seven committed scene image views to
the idle particle cache. It retained zero after withdrawal and had no
strong owner of the old handle table. The actual FX release also removed
all seven GPU image assets and returned their slots. No active particle
draw was observed. The decal replacement repeated the ten-proxy release
result. Both FX preparations had zero existing material values in their
worker snapshots, so they exercise batch handoff but do not quantify the
snapshot fix for a cache hit.

Two paired optimized acquisitions matched full probe cell geometry CRCs
`714c76bc` / `d39ec98e`, vertex upload CRCs `bbc9941d` / `4fa1e3b9`, and
instance records `089f05a5` / `d496779e`. The primary sample vectors held
20,348,928 / 17,356,800 bytes at the old loop exit and zero after the
change. Mean affected CPU cycles fell from 511.1M to 454.6M and 484.9M
to 448.6M respectively. No frame timing was measured. All seven scoped
observations exited successfully without reported graphics validation errors.
Evidence: `DH2-collision-staging/artifacts/resource-ownership-integration-20261110-comparison.json`.

All-level follow-up (`116a0d59`). The same optimized Tools binary completed
all 36 installed CPU preparation and release phases in a single catalog.
The 36 ordered probe acquisition records match the earlier full route,
including primary, secondary and rejected counts. Geometry and GPU upload
byte parity remain measured on Edge of the World and Dust District.
Evidence: `DH2-collision-staging/artifacts/integrated-all-level-acquisition-20261110.json`.

World environment declaration acquisition prediction (2026-11-10). The game owns an idDeclEnv with a default blend duration at 140017b30, and its environment stack at 14088e220 consumes the authored world and area assignments. The host reads each ancestor twice while preparing the world environment and normalizes the same source separately for lighting, render parameters, inheritance and duration. Build one ordered profile per installed world load from a single pass over the inheritance chain. Expect one read rather than two per ancestor, one normalization per visited source, unchanged order and exact typed lighting/parameter/duration values. Release source text at each iteration. Area profiles should also share one normalization for their parameter and parent readers. Preserve the lossy UTF-8 behavior of the current text path, depth 16, missing parents, cycle stopping, and fresh overlay reads on each acquisition. No new retained source cache or frame work is expected.

Tiled-light publication prediction. Original `140371750` owns the
1,152,000 four-byte light-node links and the lighting records. The host
releases its view-scoped TLF pass at the render handoff, but the forward
area-light publisher still holds the outgoing records, list heads and node
pool. Save the forward renderer's initial empty bindings and restore them
at the existing release boundary with a disabled grid. At 1920×1080 and
16-pixel tiles the three outgoing buffers total 4,788,736 logical bytes.
Expect their allocations to retire after submitted GPU work drains. The
shared texture array and device pipelines remain with their own owners.
No additional upload, draw work or per-frame reference clone is expected.

Tiled-light retirement observation (964f7f2a). Windowless DX12 created the production 1920x1080 tiled-light pass, published the buffers through the forward publisher, and dropped the pass. Its forward bindings alone kept the light-node pool (4,653,056 allocated bytes), lighting records (65,536) and per-type list heads (131,072) live. The normal area-light level release restored the exact empty binding identities and removed all three allocator entries after queue drain. Their logical buffer size was 4,788,736 bytes and their reported allocations totaled 4,849,664 bytes. The observer exited zero with no validation errors. It did not submit gameplay draws or measure frame time. Evidence: DH2-fx-level-lifetime/artifacts/area-light-tile-retirement-20261110-comparison.json.

Runtime FX image identity prediction (2026-11-10). Original image lookup 1400607f0 / 14005feb0 shares one registered path identity; release follows final resource demand at 140060180. The host can acquire distinct ParticleImage identities for an editable source while still addressing the same streaming slot by path. The slot records one weak identity and counts its registered consumer lease. When that identity loses its external consumer, another live identity cannot keep the slot or its pending read alive. Keep weak aliases for distinct identities only, leaving the shared installed case allocation-free. Expect one streaming slot and unchanged read/upload counts while any of the two owners is live, followed by release after the last owner. If retirement occurs before read publication, preserve the existing queued cancellation and running-read acknowledgement. No change to image bytes or active rendering.

Installed 2D image staging prediction (2026-11-10). The original hands each installed record to the upload routine at 1402e9ff0 / 1402f5500. The Ward lookup and shared 2D material reader still allocate and fill a second full chain after reading the member, although the scene reader already supports in-place header removal. Reuse the scene reader's compaction. Each installed read should lose one full-payload staging allocation and retain exactly the same descriptor and payload bytes, with unused capacity bounded by the member's headers and original allocation slack. The particle field reader validates every record but samples only the base RG8 mip (particle field sampling 214160). It should lose the temporary packed chain while keeping the same base pixel allocation. No changed uploads, decode format, texture residency policy or per-frame work are expected.

Visibility workspace acceptance (0c9345b2). Five changing camera angles in each installed Edge of the World and Dust District scene produced identical selected instance indices with fresh and reused scratch. One allocation remained stable throughout each reuse sweep (100,169 and 148,800 bytes). Withdrawing the camera viewport left zero visible instances and retained the same scratch allocation until map release. Both maps completed their normal release. The former two buffers requested 200,338 and 297,600 temporary bytes respectively per successful query. The new route retains one map-owned allocation and performs no further scratch allocation in the sweep. No gameplay frame timing or rendering comparison was performed. Evidence: DH2-fx-resources/artifacts/visibility-scratch-20261110-comparison.json.

Combined installed resource acceptance (341badc5, 2026-11-10). The compiled windowless resource observer completed all three routes without validation errors. The Ward lookup retained exactly 32,768 original payload bytes, reusing the 32,851-byte member allocation in place of a separate 32,768-byte chain allocation. Eight indexed RG8 vector-field assets produced identical base pixels and material chain bytes. Their reader eliminated 1,146,880 cumulative bytes of temporary packed-chain capacity across the samples, with unchanged base capacities. The sampled assets each have one mip. The reader still validates every authored mip.

All 36 indexed maps yielded 26 distinct world environment names. The combined acquisition matched lighting values, ordered parameter layers and blend duration bits for all 121 indexed environment declarations. It resolved 300 layers. Two separate walkers issued 358 ancestor requests across those declarations, including 70 across the world subset. The one-chain reader needs 179 and 35 respectively. No per-frame timing was measured.

Windowless DX12 Dunwall Tower acquired two distinct descriptions of the same installed FX image. The existing streaming slot and committed image survived the first owner's removal and the second owner's tail, then the slot released after the final owner dropped. The ordinary effect then published two images and one mesh, removed all three, returned both slots, and submitted zero requests for freed slots. This observes identity and withdrawal, with active gameplay draws and cancellation during the archive read outside the route. Evidence: DH2-fx-resources/artifacts/installed-resource-payloads-integrated-20261110-comparison.json.
