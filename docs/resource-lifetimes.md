# Resource lifetime audit

Scope: every resource family consumed by the rewrite, from installed archive
acquisition to CPU/GPU ownership, streaming, reuse and release. Unimplemented
game systems are recorded separately from defects in existing owners.

The original is the behavior and measurement baseline, not a requirement to
duplicate inefficient ownership. The user explicitly authorizes improvements
over its algorithms when output is preserved, memory is reduced and performance
does not regress. Backend differences must remain labelled and measured.

## Particle image ownership: prediction before implementation

Original image registry acquisition (`1400607f0` / `14005feb0`) returns one
resident resource identity; `1402ea140` loads its compiled image chain. Current
particle materials instead read an image per binding/material and upload it
again per render-state variant. Changing a sampler also reuploads those images.

Expected change: a level acquisition owner keyed by resolved image path and
dimension shares immutable compiled chains across particle materials. A backend
owner uploads each acquired identity once per particle pass, independently of
draw state and sampler. Both owners die with the level/pass. No new GPU pass,
pixel conversion, mip change, sampler change or gameplay update work is needed.

Prediction: for the two water stencil states, texture upload count/bytes fall
by at least half; sharing across materials/bindings can save more. CPU reads and
retained compressed bytes fall to unique requested images. Acquisition CPU time
should fall with removed reads/uploads; steady-state draws are unchanged.
Measure the actual production client with sharing enabled/disabled, report
owner upload/byte counters and changed-function timings, and check release on
map transitions. Do not substitute overall FPS for those measurements.

Existing original same-level baseline: `tests/fixtures/original_eotw_budget.txt`
(2026-10-28): PARTICLE 46,524,457 CPU bytes, IMAGE 1,250,104 CPU bytes; total
dedicated GPU 3,987 MiB. These tags do not isolate particle image GPU storage;
do not present their CPU totals as a texture GPU budget.

## Movie runtime teardown: prediction before implementation

The AVM2 host uses `Rc` for objects/classes. Every function creates a
function -> prototype -> constructor -> function cycle; class objects and
bound method closures introduce more. `Vm` has no destructor, and `Unit`
retains its own methods/classes, whose definitions point back to that unit.
Dropping `Player` therefore does not establish that its runtime was released.
Immutable shared ABC/Iggy resources are a separate, intentional cache.

Expected host correction: a VM-local weak allocation registry and explicit
runtime teardown break object, class, unit and host-state cycles when the
movie closes. No live VM execution state is shared. Weak tracking must not
keep otherwise dead objects alive; compact dead tracking entries as capacity
fills. Runtime collection during a live movie remains a separate concern.

Prediction: after closing a loading/pause movie, tracked runtime objects and
unit/class back-references are released, rather than accumulating per open.
No gameplay draw/pass changes. Measure actual repeated loading movies, with
cleanup enabled/disabled in the same client, and record remaining identities
and teardown/acquisition cost. Original IGGY baseline is 86,057,717 CPU bytes;
that includes systems beyond the runtime heap and is not a per-movie budget.

## Owner inventory

### Completed measurements

Same release client, EotW, 1280x720; comparison controls restore duplicate
particle images and retain closed VM cycles in the same executable.

| Changed owner | Before | After |
|---|---:|---:|
| Particle CPU image acquisitions | 102 | 62 |
| Acquired compiled image payload bytes | 22,200,416 | 16,134,008 |
| Particle GPU image uploads | 190 | 59 |
| GPU uploaded payload bytes | 43,785,808 | 16,046,296 |
| Image acquire/upload CPU milliseconds | 37.240 | 11.323 (repeat 12.003) |
| Tracked objects alive after loading movie closure | 3,722 | 0 |

The texture reduction is 26.45 MiB of uploaded image payload per level instance;
physical GPU heap reservation is not the same measurement. CPU payload sharing
removes another 5.79 MiB of duplication. No whole-frame performance improvement
or exact UI byte saving is inferred from these owner measurements.

Normal release transition runs EotW -> Dust -> EotW both exit 0. With cleanup
disabled, each closed loading movie retains 3,722 / 3,723 / 3,722 objects. With
cleanup enabled all three leave zero; teardown costs 2.697 / 2.930 / 2.849 ms
on the existing UI worker. Particle pass destruction reports 59 / 54 / 59
unique uploads rather than carrying outgoing level textures into the next pass.
Local evidence: `artifacts/resource-lifetime-release-*` and
`artifacts/resource-lifetime-transition-*`; no screenshots inspected or tests
written/run. These runs verify production execution and owner release, not
pixel-for-pixel output equivalence.

`host` describes current Rust ownership, not a claim that the full original
lifecycle has been ported. Source paths below are relative to `src/`.

| Family | Acquisition / representation | Residency, reuse and release | Audit finding |
|---|---|---|---|
| Archive catalog and aliases | `assets/archive`: one shared index per installation, packed sorted names; archive members read by offset; zlib only for compressed members | Process owner; lazy exclusive container-reader leases, closed with catalogue; no extracted-file prerequisite or whole-archive resident payload | Intentional resident metadata; 95,232 production acquisitions reused 16 file opens across three maps (95,216 reuses). Required zlib decoding streams into final output; no payload cache or CPU-time saving claim |
| Load manifests | `assets/map_resources`: session init/common lists, temporary per-map assembly, weak combined-table lookup | `loading` releases progress tables at play; diagnostic coverage takes and drops its report reference | Original `1400661e0` frees request tables separately from registered payloads. Host cache retains only init/common metadata; map descriptors no longer accumulate across visits |
| Mapinfo/localization/config | `gameplay/map_info`, `assets/lang`, `platform/rewrite_settings` | Parsed mapinfo cached per installation/map; loading-info cache holds current map; session settings retained | Intentional small metadata, but mapinfo cache is not an eviction policy; mission text remains a localization key; translation tables are keyed by the startup language. Runtime language switching is explicitly absent (`gameplay/options/audio::after_change`), so it is not a hidden live-language invalidation path |
| Compiled world | `app/level_prepare`, `assets/world_mesh`: current BWM only, parsed once | Scene holds instance/spatial metadata; `app/setup` empties vertex/index vectors after upload; world teardown drops metadata | CPU geometry release exists; shared receiver tree survives upload because shadows/decals still query it |
| Placed models/materials | `app/setup`, `app/material_loading`, `assets/bmd6` | Per-map geometry/material construction and asset handles; canonical Bevy assets release when references disappear | Original `14025aa00` reads each compiled model through `140259960`, acquires its skeleton through `1400607f0`, and closes the source before returning. Installed streamed-index flags do not identify ordinary placed-model pages. The existing per-map prepared model/mesh owner is appropriate; no missing spatial-page streamer is inferred. Generic init/common model identity reuse remains a distinct acquisition question |
| Surface images | `app/texture_streaming`: base compiled chain, finer separate mips requested on demand | Bounded IO queue, per-frame upload budget, mip reduction; init/common committed handles retained; map owners removed on teardown | Scene pressure/reservation and static/dynamic priority rules implemented from `14030f3d0`/`1402d4e20`; default walking has zero unnecessary drops, the 256 MiB run releases 35,258,368 bytes then stabilizes. Three-map handoff exits 0. Particle 2D images now use the same slots, projected-area demand, upload and pressure budgets; compatible registered base views now avoid a second scene upload; UI snapshots reuse the committed scene/common handle |
| Resident postprocess/UI images | `app/resident_images::ResidentImages` | One full-mip path registry for UI, fog, lens, area lights and cloud/depth-default inputs; movie/frame leases survive registry purge; init/common entries survive maps. IO is outside its mutex; pending batch identities include layout dimensions | Same-build EotW/Dust/EotW separate/shared comparison: 39 -> 23 asset/uploads, 11,761,316 -> 9,716,732 logical payload bytes. Shared purge released 7 then 6 entries and retained 3 then 4 common images. Scene-base preparation reuses equivalent registered complete views; new UI acquisitions select current committed scene/common views and existing wrappers retain snapshot leases (`1402df8c0`/`1402bd380`). Three-map A/B removed three identical 1,024-byte default-image uploads; no large residency saving is claimed. Different source-mip views preserve their own resolution policy |
| Cube/array water images and renderer noise | `app/resident_images`, `bevy_backend/layered_images`, `bevy_backend/noise_textures` | One shared immutable GPU source per typed resource; acquired staging retires after upload; common/builtin identities survive map purge; map cubes release | Production EotW/Dust/EotW: cube/array uploads 8 -> 4, caustics 5 -> 1; 36,700,160 B used staging capacity retired. Procedural uploads 12 -> 2, identical content CRC. All three cube/array descriptors/payload fingerprints match. Unused prefetched staging remains outside this result. Fixed 2D ocean masks now use the same image owner: three-map acquisitions 17 -> 6, 8,397,824 cumulative payload bytes avoided; all seven descriptor/content fingerprints match, GPU-only CPU backing is zero, and map-only entries release at transitions |
| Lighting upload staging | `bevy_backend/tiled_lighting_gpu`, `bevy_backend/tiled_lighting_pass` | Borrow initialized contiguous u32 words for synchronous queue copy on little-endian hosts; retain serialization on other targets | First-map 254 preparations: 1,016 temporary allocations / 3,710,432 copied bytes -> zero; required 762 queue writes / 2,930,144 bytes unchanged. No CPU timing claim or tests |
| Probes/lighting | `assets/probe_lighting`, `app/setup`, `bevy_backend/lighting_scene` | Current-map CPU query backing plus GPU buffers/images; references released by scene teardown | Runtime geometry/SH and GPU shading have real consumers. Superseded raw probe sample/index vectors now retire after gameplay preparation: 23,425,664 bytes EotW, 19,970,016 Dust, with unchanged prepared cell/quad/probe counts in the three-map A/B. Standalone PRB readers retain full source data |
| Shader cache/programs | `assets/shader_cache`: indices resident, large stage streams stay on disk; requested stages inflated | ProgramLibrary caches prepared definitions per owner; device pipelines persist | Justified format inflation and backend shader translation. `ProgramCacheSource` is still requested by directional/local shadow owner preparation, so dropping it after scene cooking is unsafe. Shared resource lookup/purge remains the original registry boundary; per-world ProgramLibrary ownership is required until shadow owners retire. Immutable index/schema metadata now shares across maps (733,409 serialized index bytes read once instead of once per level, same index/schema identities over three acquisitions); per-world prepared definitions and independent file cursors remain. Host particle compiled programs now survive map retirement separately from materials/images: same-binary three-map creations 52 -> 18 pipelines and 3 -> 1 shader/layout sets, unchanged volume uploads. Physical driver/device-cache byte accounting remains required work |
| Tome/visibility | `app/state::TomeWorld`, `ShadowCasterWorld`: source blob plus typed query structures | Shared Arc backing for query jobs, level-scoped query pool and mutable scratch | Source blob still serves runtime queries; decoded traversal/object tables are a host performance representation, not forgotten staging. Existing a34a947 query traces measured 826 -> 682 CPU ms/s and median span 3.05 -> 2.30 ms for the combined query optimization. Packed tree descent is already implemented (d05a6f5); preserve the accepted query layout rather than discarding consumed tables without equivalent performance proof |
| Collision/physics | `app/physics_scene`, `gameplay/collision_queries`, model collision loaders | Level-owned shapes and compact decoded triangle/cell backing; dynamic bodies hold runtime state | Keep decoded query backing: previous real walking measurements showed compressed decoding regressed CPU severely; source-format purity is not a goal |
| Collision broad phase (`NavigationWorld`) | `gameplay/navigation::CollisionWorld`, `app/state::NavigationWorld` | Current-map static collision triangles and packed cell lists; removed with map resources | Despite the wrapper name, this is the player collision query index, not an AI Navmesh resource owner. The `navigation`/`Navmesh` declaration classes are unconsumed, as the parity ledger records; do not claim an implemented AI navigation lifecycle |
| Skeletons/animation/machinery | `assets/bmd6`, `assets/edge_animation`, `app/model_motion`, player animation owners | Shared definitions vs per-instance time/pose; arms retain GPU skinning streams | Production release acquisition found 200 requested clips and 200 distinct resources (1,133,582 payload/table bytes); a proposed extra sharing registry saved no load and was reverted. Session reuse is now verified separately: the three-map route requests those 200 identities 600 times, with 162 common clips retained and 38 map-only clips released each transition. Reads/parses fall 600 -> 276; 3,360,864 -> 1,266,848 archive bytes. All request fingerprints match, and map purges report zero map leases. This preserves an existing compressed allocation across loading, rather than claiming lower steady-state residency. Compressed clip ownership is already map-scoped; per-actor clocks/poses are separate. GPU-only arm meshes discard CPU vertex streams; palettes/rest transforms remain required for sampling |
| FX graphs/curves | `gameplay/effects/instances`, typed `assets/fx` readers | Arc graph definitions; per-instance variables, queues, bindings; level resources and instances drop on teardown | Deep graph copies were already removed; unsupported families retain compact admission errors rather than generic entity trees |
| Particle definitions/models/fields | `rendering/particles/definition::ParticleResources` | Level acquisition cache shares declarations, PMODEL and decoded field pixels across world/screen consumers | Both world_sources and screen_effects/sources use ParticleDefinition::load with the single setup ParticleResources owner; runtime emitters retain prepared definitions and do not parse declarations or reacquire PMODEL/field images. CPU field pixels and PMODEL simulation geometry have real consumers |
| Particle images/materials | `rendering/particles/image_resources`, scene `texture_streaming`, `bevy_backend/particle_images` | Particle 2D definitions retain metadata and request the scene image identity; visible system bounds publish projected-area demand for all prepared materials. The remaining 3D owner retains authored chains, retires upload staging and releases on level teardown | Source 3dd8a11 completed EotW/Dust/EotW: 57/48/57 shared 2D identities; custom uploads 9/10/9, all volumes, releasing 13,729,792 / 28,311,552 / 13,729,792 staging bytes. This is ownership consolidation, not equivalent total VRAM savings. Header-only inspection of all 14,885 selected installed bimage7 images found 102 volumes: every volume has one authored mip, full XY equals inline XY, and none has indexed separate levels (zero read/header errors) |
| Projected decals | `assets/decal`, `bevy_backend/projected_decals` | Typed definitions plus per-instance state; lazy receiver geometry upload; level publication reset | Residency is separate from authored animation fidelity; do not classify a missing effect implementation as memory saving |
| Sound packages/banks/events | `app/audio`, `assets/wwise` | Shared package indices/startup banks; map sources/actions/banks removed by `audio::begin_level`; DATA byte ranges share Arc backing | Existing measurements confirm encoded backing sharing; startup audio output is intentionally persistent |
| Sound media/streams | `app/audio/graph`, `assets/wwise/vorbis` | Encoded media shared per sound; per-voice bounded decoded blocks; file sources use a shared 2 MiB encoded IO device during selected playback | Incremental Vorbis/IMA file playback is now the default; type-1 sources borrow authored bank prefixes. Acquisition reads required format metadata (232,514 vs 530,033,360 logical bytes in the measured dev run), without whole-file conversion/indexing. Final default three-map encoded-device peak 552,547 bytes; walking compared 82,699,258 samples with zero differences and no detected affected-decoder CPU regression (19.492 vs 19.746 CPU ms/s) |
| Sound actor property chains | `assets::wwise::PlaybackBanks`, `wwise::property_chains` | Immutable flat Arc slices; per-bank weak actor lookup, live definitions hold the data; map truncation prunes removed actor dependencies | Same-binary three-map comparison removes 26,280,144 EotW / 26,479,152 Dust live chain bytes, excluding weak-map metadata. All six property request/content fingerprints match; EotW return has identical retained counts. Mutable voice/mixer state remains separate; no extra hot-loop locks or altered property iteration |
| Sound decoder setup | `assets/wwise/vorbis::{WWISE_HEADERS,SETUPS}` | Shared immutable definitions; unload prunes entries after outgoing prepared sources/voices release | Original `141122400` releases old media ownership. EotW/Dust transitions retained four live/common setups: 4 -> 4, no measured savings claimed |
| Iggy files/ABC | `assets/iggy::acquire`, `flash/avm2::Unit` | One immutable parsed source/ABC per UI worker; player namespaces/classes/VM are separate | File cache intentionally survives movie close; parsed runtime Unit ownership must not keep closed VM state alive |
| Fonts/glyphs/library bitmaps | `assets/iggy/font`, `flash/glyph_cache`, `flash/bitmap_resources`, `bevy_backend/flash_ui` | Metadata-only lookups; lazy outlines; R8 glyph pages; BC7 library images passed through; obsolete movie-generation GPU textures removed on close | Original page creation/update and unused-slot reuse are implemented (`14107b7e0`, `14107b2d0`, `14107a980`); versions update one GPU identity with dirty rectangles. Immutable library bitmap caching is intentional; source-backed CPU page snapshots preserve in-flight draws and copy only while an older version is leased. UI GPU texture leases now have the installed 2,048-entry / 0x1428000-byte limits, with inactive eviction; real loading/pause peak is 44 entries / 8,495,572 bytes, closing leaves only the 4-byte white builtin |
| Movie runtime | `flash::Player`, `flash/avm2::heap` | Per-movie roots, tracing of objects/shared scopes, weak listeners and Dictionary keys, explicit closed-VM teardown | Original `141062a90`/`141063ed0`/`141064cd0` collection and weak-handle behavior implemented with host reference tracing; automatic scheduling is a host difference. Three-map production run completed, zero retained objects on close; measured live loading heaps contained no garbage |
| Render targets/descriptors/history | `bevy_backend/{camera_scene,fog_scene,display_scene,shadow_pass,auto_exposure}` | Level acknowledgement releases world/view references before destination acquisition; device pipelines and reusable viewport capacity survive | Prior repeated-map measurements verified static shadow cache no longer accumulates. Repeated production resizes completed in EotW and Dust. The Dust ocean pass retained culled 960x540 targets/bindings: pre-admission invalidation now releases 21,954,560 B of obsolete live GPU allocations, with identical streamed bytes and upgrade counts in the same-binary comparison. Size-independent wave history and resources remain. Quality-setting replacement still needs separate coverage |
| Pending IO/uploads/diagnostics | `TextureReadWorker`, `PreparedImageMailbox`, render extraction, F8 histories | Receiver drop stops outgoing reader after the current read; publication mailboxes reset; pending extraction retains handles until prepared | Bounded in-flight work can briefly outlive map teardown; do not treat allocator-reserved GPU heaps or OS file cache as live resources |

Absent systems remain `none` in `resource-parity.md`; they are not silently
counted as audited implementations. The table records ownership findings; it is not completion of the requested implementation.
Rows marked partial/under measurement still require the stated runtime or
original-code evidence before claiming a complete, correct lifecycle.

## Audit result and remaining gaps

The initial two measured corrections are duplicate particle image
ownership/upload and closed movie runtime cycles. Further implementation
now releases temporary acquisition descriptors, cancels outgoing IO, retires
backend world bindings before reuse, prunes unused decoder definitions, and
collects live UI cycles with the original weak-reference rules. These are
separate changes with their evidence in `resource-parity.md`. Compressed image
blocks, encoded sound, packed vertex attributes, lazy glyph outlines and
shared FX definitions already avoid the previously identified unnecessary
expansion/deep copies. Decoded collision/query data and source-backed Tome
bytes have actual runtime consumers and are intentionally retained.

The audit does **not** certify full lifecycle parity. The main open areas are
image identity shared across renderer domains,
resident UI/fog image policy and device-cache accounting. Particle 2D mip demand is connected to the scene streamer; all 102 installed bimage7 volumes have a single complete authored mip and no finer indexed levels. Scene texture pressure/priority rules are implemented;
equivalent registered scene-base views and named-UI current-view acquisition are now connected; resident UI/fog/lens/area/cloud consumers now share one owner. Live UI collection is implemented; automatic scheduling is a host difference.
The sustained release pause-menu run measured 29 collections per mode: mean
4.862 -> 3.238 ms in the collector, with 172 objects reclaimed in both runs.
These differ from a verified memory leak: several are deliberate caches or
partial implementations whose actual cost and required behavior remain to be
measured. Concrete next measurements and expected gains are recorded in
`optimization-opportunities.md`; absent game systems remain absent, rather
than being counted as resource savings or implemented by this audit.


Inline image staging is now borrowed through the common installed-record
validator before copying into required upload buffers. Same-executable
copy/borrowed EotW/Dust/EotW production runs removed 1,101,966,556 cumulative
copy bytes, with the largest removed temporary 16,777,216 bytes. All 4,959
image/target payload/descriptor checksums, mip counts and byte totals matched.
No residency, whole-frame or CPU-time gain is inferred from cumulative bytes.

Full-image preparation now copies validated records directly into the final
Image payload; the split/packed production comparison removed 80,400 transient
per-mip buffers, with identical final CRCs/descriptors for 4,963 identities. FX
descriptor acquisition likewise borrows its source trees; the 941-array run
removed 631,105 cloned syntax bytes while preserving all selected entries.
These are allocation/copy counts, not whole-process RAM or CPU-time deltas.


Audio bank ownership review during image integration: `audio::index_banks`
checks package path, bank id and language before `PlaybackBanks::insert`; map
entry calls `audio::begin_level`, which stops world voices/actions, truncates
the map-bank suffix, prunes map packages/locators/media and retains startup
actors. Keeping bank DATA through encoded-source Arc ranges is intentional
(`1410f1910` / `141122400`), not an extra copied payload. This review does not
justify a new duplicate-bank cache. A separate demonstrated cost remains in
`PlaybackBanks::node_hierarchy`: inherited properties are decoded into each
prepared graph; quantify repeated actor/property ownership before changing
that representation.


UI bitmap backing review: `flash::bitmap_resources::load` reads the separate
`.raw.iggytex` member, copies just the selected compiled image ranges and then
drops the source member. Those bitmap blocks are not embedded duplicates in
`IggyFile`'s retained `.iggy` bytes. The session library cache prevents repeated
bitmap reads/decode; it is intentional, not a second resident raw-file copy.
Aligned BC7 data remains compressed; the documented nonaligned backend fallback
still needs cropped RGBA output. The generic entity document likewise drops at
the end of `app::level_prepare`; runtime scene ownership holds typed records,
not the whole parser tree.


2026-11-04 viewport acceptance (14909a0): both same-binary Dust retain/retire
clients completed seven resize snapshots, exit 0. At snapshots 3, 4 and 6,
retirement removes exactly 21,954,560 B in the affected old-size texture,
uniform and sampler-index allocation labels. This matches the predicted stale
960x540 colour/depth/HDR/foam/input owners. Snapshots 0..2 have equal viewport
target storage before there is a dormant colour target to invalidate. The two
runs have exactly the same streamed finer bytes and upgrade counts at all seven
snapshots (286,654,464 B / 270 initially; 383,254,528 B / 318 afterward).
Staging and unrelated animated particle-material allocations vary and are not
included in that saving. Wave data, patch buffers, authored textures and draw
admission remain unchanged; only incompatible viewport allocations retire.
No CPU benchmarks, tests or visual inspection were performed. Numeric evidence:
artifacts/lifecycles-ocean-viewport-comparison.json. The earlier main release
at f4bb382 completed successfully before this source was integrated.


2026-11-04 water material scope acceptance (d8c086f): production three-map
separate/shared acquisition exits 0 in both modes. All 12 (material, selected
program, owned bytes, content CRC) request fingerprints match. The four actual
material constructions now create four scope backings instead of twelve:
24,237 -> 8,079 cumulative owned structure/vector bytes. The construction cache
drops before publication and adds no per-frame lookup. In the same route,
7,093 bytes of unused declaration copies are no longer retained, alongside the
unused MAIN-name/options fields. These are small owner-cleanup counts, not a
large RAM or CPU-performance claim. Existing fixture initializers were adapted
to the production type; no test or expected value was added or run. Local
numeric evidence: artifacts/lifecycles-water-scopes-comparison.json.


2026-11-04 fixed ocean-image owner acceptance (0c9e97b): library/game-bin checks,
dev build, and both production three-map separate/shared clients completed
successfully. All seven requested source identities match image format,
extent, mip count and exact content CRC. The source table now contains handles;
OceanScene binds prepared views and performs no image IO/parsing/upload. Ready
snapshots report 6 / 5 / 6 ocean references with zero CPU image-payload bytes.
The class owner releases two map-specific views at EotW retirement and one at
Dust retirement, retaining common foam and three builtins. Inline acquisitions
fall 17 -> 6, 14,252,368 -> 5,854,544 cumulative payload bytes. These are
acquisition/ownership counts, not RSS or frame-time savings. Common foam uploads
once; map shore/amplitude views reacquire on return. Equivalent base-chain scene
views reuse the registered identity through the existing preparation path.

Installed headers confirm that the EotW amplitude mask has a separate 512x512
level above its 256x256 inline chain. That is why the new owner explicitly
preserves the previous inline selection instead of routing masks through the
UI loader's finer request. The shore fields and foam have no separate levels.
Native demand/priority for environment-only finer levels still requires its
own evidence; this ownership change does not claim a new mip-streaming policy.
Local evidence: artifacts/lifecycles-ocean-image-owner-comparison.json and
artifacts/lifecycles-ocean-image-headers.json. No tests or visual inspection.


2026-11-04 UI owner acceptance (`9df316a`, measurements committed in
`c057099`): real EotW/Dust/EotW loading/pause sequences exit 0 in both modes.
Stage-transform preparation removes one temporary allocation per submission:
2,992 allocations/uploads before, zero allocations and one upload after
(2,998 submissions). Stage words match exactly; no CPU-time claim. Movie
textures retire independently of whether vertex preparation succeeded.
Both runs remain below the installed texture-cache budget, so pressure
retirement is source-backed but was not exercised by this content. Main now
contains this change, with its release compilation started separately.
