# Optimization opportunities

Findings of the module sweeps in `AGENTS.md` ("Performance work") that were
not fixed while the module was open. Each entry: what, where, measured or
estimated gain, what it must keep equal to the original, and why it waits.
Remove an entry when it is implemented and measured.

Resource lifetime audit follow-ups (full owner inventory and evidence in
`resource-lifetimes.md`):

- Live-movie cycle collection and original weak listeners/Dictionary keys are
  implemented. Sustained menu collection exposed excessive temporary adjacency
  allocations; contiguous edges and the existing pointer-key hasher remove that
  work without changing weak-reference collection timing. Same-build release
  comparison measured 4.862 -> 3.238 ms mean over 29 pause collections per mode
  (172 objects reclaimed in both); further reduction must preserve those
  semantics. No live-heap savings were observed in loading movies, whereas
  pause movies did produce cyclic garbage.
- Some backend caches retain the last view's bindings/area-light handles until
  destination preparation replaces them, while viewport capacity intentionally
  persists. Trace the outgoing-world boundary separately from settled memory
  before tightening release: expected gain is a lower transition peak, not a
  steady-state saving. Keep compiled pipelines and reusable viewport capacity;
  existing shadow static-cache accumulation was already fixed and measured.

- Resolved: Wwise inherited property chains now use immutable flat Arc slices
  with weak actor lookup. Same-binary three-map runs removed 26,280,144 EotW /
  26,479,152 Dust live chain allocation bytes (excluding weak lookup metadata),
  with identical content/request fingerprints and EotW return counts. The flat
  runtime iteration layout is retained; this is not an audio-thread CPU claim.
- Navigation candidates still allocate a temporary Vec per query, as before.
  At roughly 1.4 microseconds/query in the measured release walk, reusable
  per-worker scratch is a low priority. Shared mutable scratch would serialize
  concurrent queries; measure actual allocator CPU before adding ownership
  machinery. The immutable triangle/cell backing is now packed, with no
  per-query compressed-mesh decoding.
- UI gradient ramps remain cached by immutable content across movie opens.
  Constant authored ramps reuse their texture, but an animated sequence of
  distinct ramps could grow that cache. No growth was measured in this audit;
  inspect a real animated-ramp workload and the GDraw ramp lifetime before
  imposing eviction that could add re-uploads. Obsolete bitmap/glyph textures
  now release on movie close; their IDs/generations cannot be reused.

| Opportunity | Where | Gain | Keeps | Status |
|---|---|---|---|---|
| Share parsed immutable Iggy libraries between players | `assets::iggy::acquire`, called by `flash::Player::load_installed` | repeated acquisition returns the same parsed file/relocation identity | separate AVM2/timeline/host state and requesting-player context | resolved: the cache now owns `Rc<IggyFile>`, not raw bytes reparsed per player. Three-map logs retain the same shared font/common/controller/particle library identities. No further raw-byte cache replacement is needed |
| Publish glyph cache changes without copying an entire page | `flash::glyph_cache`, `bevy_backend::flash_ui` | GPU updates only the changed bordered rectangle; one GPU texture per page | immutable CPU page version referenced by each in-flight draw list | GPU finding resolved: dirty-base/version tracking updates rectangles and uses a full update only after skipped history; stale versions retire. CPU `Arc::make_mut` snapshots are intentional when an older extracted frame still owns that version, and incur no copy when exclusive. Removing them without equivalent version storage would corrupt in-flight draws; no CPU copy reduction is claimed |
| Borrow descriptor arrays during FX acquisition | `assets::fx::descriptor_list`, `particles::world_sources` | removes transient generic field/string copies per expanded descriptor; no claimed resident saving | partial-edit constructor defaults for omitted array slots, order and deferred errors | resolved: the same descriptor decoder now returns borrowed Cow field slices through its input lifetime. Both three-map modes selected 941 arrays / 616 entries with identical metadata capacity and particle identities; cloned syntax bytes fell 631,105 -> 0. Empty/default slots and authored ordering remain unchanged |
| Borrow event nodes and link lists from shared graph definitions | `gameplay::effects::{graph,instances}` | eliminates a node/subtree clone per dispatched action and a link-vector clone per emitted output | event ordering, per-instance variable/gate writes and declaration ownership | resolved: step/emit/insertion borrow nodes and output links through a local definition Arc; entry iteration is borrowed across both passes. Source d372bf8 completed the three-map client route. Mutable variables, queues and owner overrides remain instance-local; no CPU-time claim |
| Static collision kept as installed hknp compressed meshes, decoded per query | `gameplay::navigation`, `assets::physworld` | memory: about 216 MB resident at eotw level ready (the 1.55M-triangle copy and its 4 m cell hash) | identical triangles, materials and filters per query (checked on 400 eotw probes) | decided 2026-11-03: keep the decoded triangles. Per-query decoding cost 391 ms/s of CPU on the walk against about 10 ms/s for the copy; a later version walking the mesh's hkcdSimdTree (FUN_141262020: 112-byte nodes, leaves `section << 9 \| primitive << 1 \| 1`) was never measured in game. Revisit only with a cost close to the copy's |
| Prepare the level's listed sound events on all cores | `app::audio::prepare_all` | load: 31 s single-threaded estimate to the measured parallel time (release) | the same prepared events and media | implemented 2026-10-28 (the original prepares them on its load thread) |
| Upload a texture's subresources from one staging buffer | `vendor/bevy_render` `RenderDevice::create_texture_with_data` | peak memory: 4.4 GB of D3D12 staging at level ready (64 KiB minimum per write) | identical texels (`texture_upload_tests`) | implemented 2026-10-31 (9f02587): staging at level ready 158 -> 17 MB |
| Build the world geometry and materials on several threads | `app::setup` world object loop | load: 55-113 s debug, 8 s release single-threaded | the same meshes, materials and instance order | recorded |
| Load the level while the loading screen plays | `app::launch`, `app::setup` | load screen instead of a frozen window for the whole load (about 116 s release) | the same load order and gates | recorded; the original also loads behind its loading screen |
| Visibility diagnostics text only when it is logged | `app::visibility_queries` `diagnostics.pending`, `app::performance::finish_streaming_diagnostics` | about 0.05 ms per frame (two long `format!` strings, a scan of every mesh entity and of 4094 image slots) | the F8 report's 120-frame history | recorded: the F8 history keeps every frame's text; it needs a structured per-frame record formatted only on F8 |
| Keep Bevy scope waiters from running Tome query jobs | `vendor/bevy_render` `write_batched_instance_buffers` and other render-world systems that wait on a `ComputeTaskPool::scope` | 0.30 ms per frame on the render critical path in the eotw sweep (0.89 ms with 6 compute workers): a waiting scope ticks the compute pool and runs a whole Tome query job (about 0.75 ms) inline | the same query jobs and outputs | recorded: needs finer jobs or a scope wait that runs only its own tasks; Bevy's scopes are not vendored |
| Placed-model visibility in parallel or behind a coarse cull | `app::view_geometry::query_placed_models` | 0.28 ms per frame serial on the visibility critical path (static and moving camera) | the same accepted groups and areas, in group order | recorded: per-group tests are independent; a scope on the shared pool risks running unrelated long tasks on this critical thread, as the early Tome query found |
| Texture demand and instance update without full-list scans | `PreparedTextureDemands::submit`, the `stream_visible_models` update loop | 0.30 + 0.33 ms per frame (merged query list with duplicates times images; a 100k-instance scan) | the same per-slot maximum areas and visibility changes | recorded |
| Phase draws carry their mesh ranges instead of looking them up | `vendor/bevy_render` `RenderCommandState::draw`, `bevy_pbr` `DrawMesh`/`SetMeshBindGroup` | the main opaque pass and early prepass spend ~145 ms/s (about 1.6 ms per frame) on Bevy-side per-draw work for ~3200 draws: parameter fetches, entity and mesh hash lookups, slab lookups; wgpu encoding is only ~5 ms/s | identical draw commands in the same order | tried 2026-10-31: a dense table of resident mesh allocations replaced three hash probes per lookup (verified exact over a level run) but measured no total gain (148.9 -> 140.5 -> 148.6 ms/s, within noise): the lookups cost cache misses, which a dense array also takes; reverted. Also tried: draw commands taking the mesh ID and slabs from the phase item keys (verified exact): SetMeshBindGroup 30.6 -> 11.2 ms/s but DrawMesh 45.8 -> 61.4, total unchanged (152.5) - the first touch of each of ~3200 meshes' render data per draw misses cache wherever it happens; reverted. Next: keep each mesh's draw data (vertex/index buffer, ranges, index format and count) in the retained bins, which are only rebuilt for changed entities, so a draw reads it from the bin instead of from per-mesh tables. 2026-11-01 (dbe0ab7): the chunks of each pass now record concurrently with per-worker draw-function copies, which took ~0.8-0.9 ms off the render critical path (static and moving); the per-draw cost itself is unchanged and the retained-bin idea still applies to total CPU |
| Decode looping or repeated sound media once | `assets::wwise::VorbisCursor` (lewton) on the audio callback | 70-80 ms/s of audio-thread CPU while sounds play (sweeping-camera trace) | identical PCM | recorded: needs a memory budget for decoded PCM; compare with how the original streams and caches decoded media. 2026-11-01 sweep traces: decoding comes in bursts of a full core per 20 ms window (median 2 ms), and the 2% slowest frames coincide with them (audio thread 16 ms/frame in slow frames vs 1.7). Not loop wraps: seeking by Ogg page on a wrap (exact once page granules are mapped to recorded true frame indices, since the converted Ogg's granules are not frame indices across block-size changes) changed nothing (86.5 vs 87.4 ms/s) because looping media loop the whole file, so a wrap only reopens at frame 0; reverted. Cause found and fixed (51752cc): at a whole-file loop's seam the resampler's reads alternated between the last and first frames, and each alternation reopened the decoder and re-decoded the whole stream (blocks of 110-157 ms, 5800-8600 packets for 11 voices); the cursor now keeps the first 8192 frames. Sweep: Vorbis 106 -> 57 ms/s, worst 20 ms window 20 -> 4 ms, slow frames 17 -> 8 |
| Ocean per-patch bind groups | `bevy_backend::ocean_scene` | ocean recording ~23 ms/s (sweeping camera) was measured with 148 frustum-culled patches x2 passes; the patch list now uses the original's query path (1403007e0: ARK_RENDERPASS_CULL_OCEAN flag renderer+5fa360, read live as set at eotw) and matches the original's live 134-node tree there (`eotw_ocean_query_culling_matches_running_original`). The shared uniform is serialized once per frame; the bind groups are still recreated each frame | the original's per-patch constant updates | re-measure the ocean recording cost with the query-culled list, then retain bind groups across frames when the patch count is unchanged |
| Indirect mesh drawing (`DH2_DIRECT_MESH_DRAWS=0`) | `vendor/bevy_render` `write_binned_instance_buffers`, `bevy_pbr` `prepare_preprocess_bind_groups` | measured 2026-11-01 at the eotw static view: 58 fps vs direct 98. Two `queue.write_buffer` per multidrawable batch set every frame (each a new D3D12 staging resource, 305 ms/s) and one `unpack_bins` bind group per batch set every frame (149 ms/s). Uploading only changed batch sets brought indirect to 85 fps; caching the bind groups measured no frame gain; both reverted because direct stays ahead: indirect GPU busy 7.8 ms vs 6.5 (`unpack_bins` ~1.1 ms) with the same ~3.5 ms CPU wait | identical images (GPU culling must keep every visible mesh) | recorded: indirect only wins if the CPU wait falls below ~2 ms; the upload-only-when-changed fix also does nothing for a moving camera, where bins change every frame |
| Shadow record preparation while the camera moves | `bevy_backend::shadow_pass::prepare_shadow_records` | 0.18 ms/frame static, 0.91 sweeping (render prepare, ahead of the graph): mesh allocation lookups 20 ms/s, alpha/vegetation bind groups 17 ms/s, the rest per-record work. Measured 2026-11-01: the mesh allocation memo never hits (hits=0 over a whole run) - its epoch includes allocator and RenderAssets change ticks, and 4 meshes are freed and reallocated every frame (below); keying it on a free/slab-reallocation counter and keeping entries 300 frames still gave hits=0. Retaining bindings for 120 submissions gave no gain: the world visible-index buffer in every binding key is a new GPU buffer ~1.2 times per frame (890 new assets from the cascade path's immutable cached-slice generations, 859 from `local_shadow_updates` `build_world_layer` without upload slots, per sweep run) | identical records and bindings | mesh allocation memo implemented 2026-11-02: entries drop per freed mesh (MeshAllocator::freed_meshes) or on a slab buffer replacement instead of on any allocator change; walk: prepare_shadow_records 94.8 -> 58.8 ms/s, fresh 29.5 -> <1.5 ms/s. Cascade static generations written in place 2026-11-02 (one live generation per cascade, so its visible-ID buffer and bind groups survive): walk 71.8 -> 46.6 ms/s (0.71 -> 0.44 ms/frame), world alpha/vegetation bind group creation and drops ~24 ms/s -> below 1.5. Remaining: the image-keyed binding lookups (~10 ms/s) and per-record work |
| Directional static shadow caches that last longer than one frame | `render::render_shadow::parallel_shadow_slice_plan`, `bevy_backend::shadow_pass::directional::encode_layer` | directional shadows are the largest GPU pass while moving (2.06 ms mean on the eotw walk, 4.1 ms in the busiest frame). The original's partial slice update (r_shadowParallelPartialSliceUpdate) redraws each cached cascade's static casters every other frame and reuses the cache only for the frame between: 2129-2130 cache rewrites per cascade in a 30 s walk, 672 in a 10 s static view | the original's shadow pixels | recorded 2026-11-02: an atlas layer can never be left as it is (tried: skip the restore and transfer when the layer already holds an unchanged cache with no dynamic casters; 0 skips, reverted), because the cache generation changes every second frame. The static pass also takes the current time (animated vegetation casters) and the cascade projection follows the camera, so a longer-lived cache (texel-snapped cascades redrawing only newly exposed strips, or static vegetation split out) would change pixels: a design decision, not a same-output optimization. Reusing the cache when an update would draw exactly what it holds (same cascade clip and caster mask; vegetation drawn per frame with the current clock) was measured first on the walk (2026-11-02): the clip was unchanged in 89/93/94% of cascade 1/2/3 rewrites, but the caster mask only in 16/6/2% (it follows the camera's receiver volume, object distance and coarse depth), so it would save ~40 of ~2400 shadow draws per frame and was not implemented. Fixed-size scrolling cascades wait until the rewrite is feature-complete. Open: the share of rewrites whose new mask is a subset of the cached one, and whether each culling term (receiver planes, distance, coarse depth) leaves extra cached casters invisible on screen |
| First-person arms skinned on the GPU | `app::player_arms::update_player_arms`, vendored `bevy_pbr` mesh/prepass vertex shaders | walking: arms skinning 1.3-1.4 ms per frame of main-thread waits (after e6901f4's parallel skinning) plus 0.43 ms of mesh upload on the render thread (after df8da1d); the body model alone is 10391 vertices | the installed skin VS blend (eight byte weights, weightY then MAD weightX, Z/W, second quartet; edge_skeleton::vertex_skin_matrix) | implemented 2026-11-02: bind-pose streams with joints 4..7 in DH2_JointIndexB/WeightB, the DH2_SKIN8 blend in the vendored mesh and prepass shaders (installed order; bitangent = cross(skinned N, T) x sign), the palette published as joint transforms with identity inverse binds. Rest pose matches the CPU-skinned screenshot (arms identical in an 8x diff). Deterministic walk: arm skinning 1.08 -> 0 ms/frame, allocate_and_free_meshes 0.41 -> 0.04, mean frame 10.74 -> 10.18 ms, fps 84.8 -> 88.3; opaque pass 1.874 -> 1.898 ms and prepass 0.391 -> 0.396 ms (within walk variance) |
| CPU-skinned physics placements re-uploaded every frame | `app::physics_skinning::pose_physics_systems` | 4 meshes `modified` every frame (Assets<Mesh>::get_mut + apply_rest_pose), so Bevy re-extracts, frees and reallocates them each frame (`allocate_and_free_meshes` ~0.23 ms/frame) and invalidates any slab-based memo | identical deformed vertices | identical poses skipped 2026-11-02 (the placement keeps its last palette; resting bodies no longer re-skin or re-upload): eotw camera path, skinning 0.13 -> 0.03 ms/frame. Open: GPU skinning from the published palette for bodies that do move |
| Wwise virtual voices (below-threshold behaviour) | `app::audio::graph` `Runtime::frame_inner` | eotw sweep with benchmark audio: audio thread 175 -> 163 ms/s (Sound::sample 75.8 -> 71.4, voice filters 14.1 -> 10.9); 72 one-shot fly/mosquito voices start beyond their attenuation range and stay virtual, while the cost is dominated by audible Vorbis loops | the original's audible output (connections at or below the -96.3 dB threshold) and voice lifetimes | implemented 2026-11-01 (1410fcf50 audibility, 1410fdda0 kill/virtual/return, 1411609c0/141161c80 return modes, 1410dd720 resolution); not covered: voices above two channels (unrouted). The Wwise instance limiter (141115880) is not ported; when it is, threshold-virtual voices (PBI+0x1f8 bit 0, set by 141106180) must stay out of its count |

- `rendering/bevy_backend/particle_meshes.rs`: `by_key` and its per-key lists
  are allocated during each publication; retaining the grouping scratch would
  remove O(active material/source pairs) small allocations per frame. No byte
  or CPU saving has been measured. Waits for a targeted publication A/B; this
  pass does not alter grouping or draw order while adding another direct-surface
  owner. GPU instance/lighting buffers already grow only when required.

- Projected decal receiver publication: shares geometry/palettes but currently
  rebuilds visible receiver transforms and temporary packet lists each frame.
  Cache immutable classification/mesh-layout metadata and reuse list capacity
  after the connected path is verified. Do not gate acquisition on previous-frame
  effect activity: that would delay a newly started effect by one frame.
  Gain is proportional to visible receiver count; not measured yet.
- Direct projected receivers need Bevy's skinned mesh bind-group layout even
  though projection programs borrow the current palette through their own group.
  The host proxy has no independent skeleton. Fixed: `DirectMeshSkin` selects
  the shared skinned binding; palette ownership stays with the source skin and
  no duplicate proxy joint records are allocated.

- Projected decal receiver GPU upload: the connected blood-splat diagnostic
  publishes 4,928 receiver records although only one depth-box proxy is drawn.
  At 64 bytes each this is 315,392 bytes/frame; that depth-box draw needs only
  one 64-byte receiver. Build receiver records lazily when an admitted pair
  actually references them, reusing the per-frame remap scratch. Prediction for
  this view: 4,928 -> 1 records and 315,392 -> 64 uploaded receiver bytes, with
  the same projection, pair count and draw. Implemented with a reused receiver
  index scratch. Measured in the same executable: 315,392 -> 64 receiver
  buffer bytes, one 36-index/one-instance depth-box draw in both modes. The
  predicted byte saving was exact; admission, record calculation and draw
  order are unchanged. Retained.

- Projected decal CPU packet preparation still checks every visible receiver
  per material/layout proxy before rejecting non-overlapping projections. Cache
  immutable classification and use the prepared projection target lists to
  visit only potential receivers after measuring this loop; expected work is
  proportional to admitted targets rather than total visible meshes. This is
  recorded separately from sparse GPU publication, whose byte saving is known.
  Host phase proxies also retain zero-count submissions while inactive to
  admit effects immediately without a one-frame visibility handoff delay.


## Loading scene publication

World-lifetime audit: shadow target caches were keyed by image identity and
survived every world. An actual three-map client run retained 60/121/182 MiB
of directional static-cache textures (3/6/9 owners). Map teardown now releases
world targets and descriptors while retaining compiled pipelines. This is a
lifetime correction required by the persistent runtime; expected live cache
count is three after each destination load. Quality-driven target replacement
inside one world can also leave obsolete target IDs until the world unloads;
pruning those against live render assets is a separate opportunity requiring a
quality-change capture, rather than changing the ordinary shadow hot path here.

`app::level_prepare` now moves archive parsing and map CPU acquisition off the
window/event thread. `app::setup` and audio startup still build their Bevy assets
synchronously after publication. Existing release load traces (`residency-expanded`
and `residency-profile-baseline`, local artifacts) put scene construction at
roughly 7–9 seconds including initial image reads; this can hold the last loading
frame during that phase. Incremental publication would improve event/animation
responsiveness rather than reduce the underlying parser/mesh work. It requires
separating CPU cooking from main-world Assets/Commands publication while keeping
one handle allocator and image owner; a second application/device or private
copy of its asset stores would break ownership. This remains a host scheduling
limitation, not a claim of complete asynchronous scene construction.


The repeated-map production client now retains three directional cache textures
at every load. After correcting suspended custom asset cleanup, weak atlas
associations and outgoing view ownership, the fourth EotW load reserves
3620 MiB across 18 GPU blocks versus 5156 MiB / 23 blocks before; both have
2763 MiB live allocations. Common-before-map upload staging preserves resource
acquisition order without changing allocator block policy or adding gameplay
passes. This measures transition residency, not walking CPU time. Per-world
visibility, decal, alpha-depth and instance-to-image scratch is invalidated
explicitly when its scene owner is added; raw allocator addresses alone cannot
identify a replacement world. Detailed client evidence is in docs/loading.md.


Loading responsiveness follow-up supersedes the synchronous scene-cooking
limitation above: the setup body now prepares payloads/commands on the map
acquisition worker through the live allocators, then publishes once. Initial
base-mip reads also move off the window thread while GPU upload ownership stays
unchanged. The separate audio preparation and shadow-target/static-caster constructors
also move through their same helpers on that worker. Publication and backend
uploads still require measurements: moving meshes alone was not evidence that
every loading stall had gone away. GPU image uploads use the configured soft
budget during loading, restored to the prior host policy before gameplay.

Targeted dev Dust loading trace (`loading-phases-dust`) measures asset publication
3.543 ms, entity publication 2.310 ms and remaining Startup 5.519 ms. Remaining
loading hitches are in backend PrepareAssets (721.466 ms) and the first
transparent draw stage (638.575 ms), not synchronous scene setup. Investigate
which asset types participate in the soft upload budget and first-use draw
initialization before changing them. These are dev diagnostic spans, not
release gameplay CPU or FPS measurements; no walking-performance gain is claimed.

The precise backend trace (`loading-backend-dust`) attributes 691.558 ms to
mesh allocation/upload and 581.237 ms to particle material acquisition, not
ordinary transparent drawing. Expected host change: limit extraction before
the mesh allocator consumes payloads, using the existing loading byte budget;
shader buffers must report their byte size to participate too. Preserve pending
Modified events until actual extraction. Ordinary gameplay has no byte budget.
Particle loading acquires one new material/state variant per frame (cached
variants do not consume the quota), rather than constructing the whole list
inside one draw-stage call. Expected maximum work scales with one admitted
batch or one oversized asset/variant, rather than the whole map. The loading
gate stays closed until the unchanged resource set is complete. A diagnostic
switch restores unbounded acquisition in the same binary for comparison.

Particle image duplication was fixed by the resource lifetime audit: level-scoped
compiled chains and one backend texture per acquired identity, independently of
material/state/sampler. Release EotW: 190 -> 59 uploads, 43,785,808 -> 16,046,296
uploaded bytes, image acquisition/upload CPU 37.240 -> 11.323 ms. The common
image-registry baseline and per-tag original snapshot are recorded in
`resource-lifetimes.md`; cross-family image identity/streaming remains partial.

The transition trace (`loading-teardown`) disproves the hypothesis that deep
world-resource destruction causes the remaining one-second blank interval:
entities take 1.374 / 2.746 ms, publication reset 39.257 / 48.000 ms, total
`LevelLifetime::clear` 86.754 / 96.303 ms. After that, contact-sound `FromWorld`
construction reacquired the destination manifest and contact/sound declarations
on the window thread. This finding is resolved: `AudioPreparation` acquires
that same map-scoped owner on its existing worker, publication supplies it before
map systems run, and unload resets mutable state without reacquisition. The
recorded production run placed 837–866 ms of acquisition on the worker and
measured 117–130 ms transition intervals, with initial/return handoffs succeeding.
See resource-parity.md, Responsive loading and load fade. No new work is queued
for the old one-second main-thread acquisition hypothesis.

Texture streamer module sweep: request flag/order storage now belongs to the
level's FrameRequests; production diagnostics reported growth only on its initial
fill across 1,200 frames. Completion/mailbox vectors still deserve allocation
profiling before any scratch refactor; no bytes or CPU gain is assumed. Unused
diagnostic result formatting is guarded by its enabled history branch. The
actual render submission budget is implemented in report_prepared_diffuse_images
from 14030d1e0, not just at main-world staging. Same-binary 4096/32 KB runs both
submitted 367 jobs / 169,213,952 bytes; the largest observed render frame fell
5,767,168 -> 2,097,152 bytes at the lower limit, with the permitted oversized
final job. This closes the missing upload-budget finding without changing total
submitted data. Settings use the existing generation cache.



## Archive acquisition owner sweep

Manifest lookup now has an implementation batch that uses the retained
catalogue container location and filters selected-index metadata by requested
ids before allocating record names. Production loading/count validation completed with exit 0: discovery scans
are zero, and retained bootstrap/common/EotW records are 1774/27040/21713.
Removed work is whole patch-index discovery parsing and retention/decoding
of unrelated IndexEntry records; no walking-frame gain is claimed.

The adjacent reader changes are implemented: Catalog::read reuses the selected
entry for kind reporting and payload access; catalogue-owned exclusive readers
reuse physical container handles and cached immutable extents. Native container
ownership is 140068720 / 14006a680 / 14006a310. Locks cover checkout/return only,
with independent cursors for concurrent IO/decode. The production three-map
route reported 95,232 acquisitions, 16 opens and 95,216 reuses; no extra payload
cache. This establishes removed operations, not CPU-time or RAM savings.

Shadow target sweep: source-absent atlas wrappers and auxiliary arrays now
retire before revision reuse. Actual three-map loading retired no target within
one map; no GPU-memory reduction is assigned. Resource bindings, mesh allocator
entries and alpha materials already have invalidation/pruning. A live quality
or resolution replacement still needs residency coverage; do not infer it from
map teardown or add a second cleanup for caches already pruned.


UI owner sweep: per-display/per-submission image leases and map-class registry
purge now replace perpetual strong map-image caching. Shared bootstrap/common
identities stay resident. Worker Close also releases its private last_frame,
which otherwise kept a closed movie's library-image/draw references alive until
next Open. Cross-domain scene/UI/particle canonical identity is still separate
work; this change does not claim that unification. Cohesive production retirement completed with exit 0: purges released 2 then 1
registry entries over three maps, with zero live wrapper entries at purge.
No CPU timing or RSS claim is assigned.

Setup model temporary ownership: model_cache/md6_cache are only referenced
through the placement summary, but remain allocated until setup returns after
water/FX/particle acquisition. Their decoded surface vectors are staging, while
published mesh/query/skinning owners hold runtime backing independently.
Implemented retirement before later acquisition: the three-map production run
released vertex/index capacities 30,158,970 / 26,266,368 / 30,158,970 bytes,
excluding metadata and allocator overhead. This removes overlapping live staging;
it is not an observed RSS or gameplay CPU saving. Native model input closes
at 14025aa00's exit; runtime geometry lifetime is a separate owner.


Particle owner sweep for shared image streaming: material image-view changes must
refresh bindings while preserving uniforms and pipelines. ParticlePass now keys
pipelines by draw state, accumulation, distortion and effective lighting mode;
this pass's formats/layout/shader stay fixed. Sampler-driven material rebuilding
reuses those pipeline identities. Production requests 72/94/72 used 18/16/18 unique pipelines (54/78/54 reuses);
no CPU-time claim follows from counts. The subsequent shared 2D route includes
metadata acquisition, scene slots/uploads, demand and binding refresh, with its
production execution completed with exit 0 over EotW/Dust/EotW. The cache-only
retirement follow-up reduced registered 2D identities 57/48/57 -> 54/48/54;
volume uploads remained 9/10/9. Combined binding-refresh counts do not isolate
sampler refresh, which remains unmeasured. All 102 installed bimage7 volumes
have one complete authored mip and no separate levels; there is no missing
finer volume payload to request. This establishes ownership/count changes,
not a targeted CPU-time or total VRAM saving.


Inline image packing sweep: the temporary packed inline chain between BIM
validation and per-mip upload buffers is removed. The common borrowed visitor
retains the existing validation; production A/B matched 4,959 image/target
identities and removed 1,101,966,556 cumulative copying bytes (16 MiB largest
removed temporary). Required owned buffers remain across the IO/render boundary.
Full-mip Image publication still assembles the final contiguous payload from
those owned mip buffers; changing that ownership needs a coherent whole-chain
transfer rather than another duplicate parser or per-mip shared-reference work.
Expected remaining opportunity is one packing copy for whole-chain consumers,
not an assumed resident-memory or walking-CPU reduction. This remaining packing copy is now resolved: direct whole-image preparation
removed 80,400 intermediate mip buffers / 1,102,175,452 cumulative payload bytes
in the three-map comparison, with all 4,963 output identities matching CRC,
descriptors and final bytes. Streaming jobs retain their per-mip payloads.


Particle program lifetime sweep: CameraGpu now retains an Arc of immutable
shader/layout/pipeline objects across maps, keyed by target formats. Active
ParticlePass owns mutable buffers and image/material bindings separately. No
per-frame mutex, added pipeline lookup or pipeline-map copy is introduced.
Same-build EotW/Dust/EotW recreate/reuse production comparison created 52 -> 18
pipelines and 3 -> 1 shader/layout sets, with unchanged image upload counts/
bytes. Dust and the return map created no new states. This closes repeated
map shader compilation for this owner; other pass shader/device residency and
physical allocation accounting remain distinct work.


Probe owner sweep: raw primary/reflection sample vectors and cell occluder
indices survived after CellGeometry construction, with no gameplay readers.
Explicit gameplay preparation retirement now releases 23,425,664 / 19,970,016 /
23,425,664 capacity bytes across EotW/Dust/EotW. Prepared query cells/quads and
probe counts match the retain control; lighting coefficients, spatial planes/
neighbors/tetrahedra and reflection metadata remain live. No per-query decoding
or CPU timing change was introduced. The underlying PRB parser stays complete
for standalone acquisition/inspection. This closes that retained-staging defect.


Typed-image owner sweep (c9b2206, accepted 2026-11-04): source ownership, common
scope and Cube/CubeArray view reuse remove four redundant caustics uploads over
EotW/Dust/EotW (5,592,064 cumulative payload bytes). Used cube/array staging
retires 36,700,160 capacity bytes; one persistent procedural factory replaces
six (ten texture uploads / 1,315,840 bytes avoided). TLF borrow-upload staging
removes exactly four allocations per preparation, with equal first-map required
upload volume. No CPU/GPU time claim. Source descriptors/payload CRCs match.

Former ocean-mask finding (resolved below): earlier versions had a separate
2D upload map and retained compressed CPU chains. The current shared image
owner preserves inline view selection, registers separately indexed mips with
the streamer and hands committed views to ocean consumers. The original EotW
waterfront observation found no finer demand in 900 samples; that observation
is not a claim about every camera or map. Unused prefetched typed sky staging
remains outside the 36,700,160-byte staging-retirement result.


Resolution-retirement finding resolved (14909a0): a culled ocean colour pass
returned before validating viewport targets, so old foam/HDR bindings pinned
960x540 main colour, water depth and opaque inputs. Pre-admission invalidation
removes exactly 21,954,560 B of live GPU allocations at three larger-size
snapshots in a same-binary Dust comparison. Both clients exit 0; finer image
bytes and mip upgrades match at every snapshot. Unchanged-size frames only
compare the cached extent; wave simulation/history, images and size-independent
program/buffer capacity remain. No extra passes or CPU-time claim. This covers
resolution replacement for the observed owners, not unmeasured quality changes.


Water material sweep (d8c086f): removed unread declaration/MAIN-name/options
storage after bank preparation. Root, world and model consumers share parameter
scopes only for the same material/selected program; differing alpha-program
defaults remain separate. Three-map same-binary construction count 12 -> 4,
24,237 -> 8,079 cumulative owned structure/vector bytes, all 12 fingerprints
match. The temporary acquisition map drops before play; no runtime hashing,
locks or reference-count operation was added to view evaluation. This is a
small correctness/ownership cleanup, with no CPU-time or process-RAM claim.


Fixed ocean-mask ownership resolved (0c9e97b): removed OceanScene's private image
uploader and the retained compressed-chain table. ResidentImages supplies fixed
inline handles through normal render-only asset extraction. Three-map shared
acquisitions 17 -> 6 / 14,252,368 -> 5,854,544 cumulative bytes, all seven source
descriptors/content fingerprints match; ocean CPU payload backing is zero at
all three ready snapshots. The later environment-image audit registered separately indexed finer mips
with the shared streamer while preserving inline views by default; the sampled
original EotW waterfront showed zero demand throughout. A diagnostic demand
confirmed view handoff on EotW and Curator boat, as recorded in the parity ledger.


Streaming-owner sweep resolved: direct receiver-to-ready transfer and
map-scoped reusable upload scratch removed the measured temporary batch
capacity and most bookkeeping growth; completed-read paths are borrowed. The
same-build four-level comparison, generation releases and matching image
signatures are recorded in `resource-parity.md`.

`upload_compressed_mips` also creates and submits a command encoder for every
replacement that copies an existing GPU mip chain. The maximum avoidable
submission count is one per such replacement; the actual time and potential
batching benefit are unmeasured. A batching change must preserve the ordering
between each copy, `write_texture` and completion acknowledgement, and must
first be compared with the installed upload path. It is not part of the
current allocation changes.


Material acquisition overlap resolved: the caster passes a single temporary
source buffer to the next matching colour-material resolution. The measured
EotW / Dust / EotW route removed 3,943 reads and 3,464,990 decoded source
bytes, retaining at most 3,073 bytes in that handoff. Admission, source and
colour-resolution input fingerprints match the separate-read control. Loose
overlays retain separate reads. This is acquisition IO and temporary buffer
capacity, not a frame-time or process-RSS measurement.


Sound declaration acquisition overlap resolved: one shared common/level text
owner supplies contact and sound-event preparation. EotW / Dust / EotW
contact reads 28,180 -> 10,162 and decoded bytes 4,428,182 -> 1,611,850;
sound-event reads 5,040 -> 0. Contact and resolved-event fingerprints match.
Mutable overlays retain the separate read path. These are acquisition counts,
not a claim about frame time or process RSS.

Declaration overlay discovery resolved: the production EotW / Dust / EotW
client skipped exactly 9,009 common declaration checks and two bootstrap
manifest checks when the overlay set was empty. Contact and sound signatures
matched the earlier client. Existing per-path checks remain when overlays are
present; there is no claimed gameplay-frame or CPU-time gain.


Material-source handoff prediction (before implementation): world setup often
requests caster admission immediately before resolving the same material for
colour. Keep only the most recent successful caster source until the next
admission or colour request, and transfer it only when its full path matches.
This is a single temporary byte buffer, not a session or map-resident raw
material cache. Bypass the handoff if overlays are enabled. On EotW / Dust /
EotW, each matched transfer should eliminate exactly one archive read and its
source bytes; unmatched sources follow the old path. Measure transfers,
source bytes and source/admission fingerprints in the same production client
with handoff enabled and disabled, and track the largest held source capacity.
No frame-time or process-RSS gain is predicted.


Ocean environment view ownership comparison: a prior production Institute
camera path crossed from the world environment into the exterior area.
Retiring names outside the active selection removed one `_white` view lease;
that builtin remains shared and resident, so this does not establish a GPU
allocation saving. The selected views and source fingerprints matched. Do not
add four path comparisons per frame and retain another set of names for this
observed result. A stale view after an independently streamed inactive image
replacement remains a conditional possibility, but no normal producer was
observed in the waterfront capture. Evidence: local
`artifacts/ocean-area-final-comparison.json` and its paired client logs.


Archive overlay lookup prediction (before implementation): every
`Catalog::overlay_file` call currently normalizes a path even when the
catalog contains no overlay files. The three-map production client recorded
at least 72,704 archive reader acquisitions, each preceded by an overlay
lookup, with an empty overlay set. The material handoff in the next client
removes up to 3,943 of those reads, and the arm-rig handoff removes more;
expect roughly 68,000–73,000 remaining acquisitions at a comparable checkpoint,
with additional overlay lookups outside those acquisitions. Returning
immediately for an empty set removes their path-string allocations without
changing archive reads, lookup results, or the nonempty-overlay path. This
affects resource acquisition, including late reads; count calls and compare
resource fingerprints rather than infer whole-frame speed or RSS savings.


Player arm rig handoff prediction (before implementation): `load_arm`
loads and parses each arm rig, then `load_body` loads both rigs a second
time for Midnight scene aliases and skeleton validation. The original
resource lookup `1400607f0` reuses acquired definitions; `1407021d0`
constructs per-instance animation state separately. Return only each first rig's aliases and rest pose with its arm and borrow
them during scene preparation. For the ordinary two-arm
player this removes exactly two of five rig loads per level (body plus two
loads for each arm become body plus one per arm), including each repeated
skeleton, MD6 and animset source. Both rig values were already needed during
setup; these inputs release after scene preparation. The first arm's scene
inputs are kept slightly earlier while the second arm is constructed, so a
lower peak RSS is not assumed. If overlays are present,
keep separate reads so a live modified file is not silently cached. Compare
actual source reads, rig content and scene/clip fingerprints in a matched
production sequence. No gameplay-frame or process-RSS gain is predicted.


Player arm scene aliases: the EotW / Dust / EotW Corvo production client
requested exactly 200 animation paths per load, with 200 distinct paths on each
load. Thus the proposed alias-map handoff cannot save any of those measured
clip requests and has not been added. Other characters and scene graphs were
not exercised. Evidence: `artifacts/material-handoff-shared.err` animation
acquisition records.


Loading read-history allocation prediction (before implementation): the
host progress approximation normalizes every acquired path into two new
strings and allocates a kind string before attempting a HashSet insertion,
even for paths already present. Normalized lowercase paths can be looked up
by reference; allocate a stored path only on first insertion, and allocate
a kind only on its first use. Paths needing normalization retain the same
ASCII case and separator rules. The three-map client already observed tens
of thousands of archive acquisitions and far fewer distinct history names,
so repeated canonical reads should need zero path/kind allocations instead
of at least three each. Count borrowed paths, new identities and kind
insertions in a production comparison with the former allocation behavior
selectable; both modes derive late-read decisions from insertion, so the
control is not a byte-for-byte copy of the former implementation. Verify
progress, late-read identities and resource fingerprints. A diagnostic
thread-cycle counter around only `loading::read` will compare average work
per read; expect it not to increase, and investigate a change over 30% in
either direction. The counter adds the same instrumentation to both modes
and cycles are not elapsed time. No process-RSS or gameplay-frame gain is
assumed from allocation counts.


Physics-material source overlap is resolved: the three-map production route
reduced physics source reads 749 -> 62 and initial contact reads by 45, with
matching resolved material and contact signatures. World, collision, machinery
and startup still resolve typed values separately; a shared parsed-value owner
would need its own memory and affected-CPU measurements before it is justified.

Animation, Iggy, UI bitmap, localization and render-parameter caches return
resident resources for the same catalog. The original registry lookup
`1400607f0` / `14005feb0` likewise returns an existing identity. In-place mod
edits alone do not establish a reload boundary; do not bypass those caches
without evidence for one. Mutable load-list and map-info acquisition have
separate handling because they are descriptors rather than resident payloads.

Trigger-shape source ownership: immutable HKX source and parsed schema now
release after physics scene initialization, once the root mass result is
retained. Query geometry, bounds and headers remain; mutable shapes retain
their source. The EotW / Dust clients released 7,090,184 / 6,253,494 bytes of
source and schema capacities, with matching geometry and mass fingerprints.
The cached mass field has its own per-shape cost; these are not net RSS
measurements. The release comparison subsequently measured 205,954,140 ->
229,400,255 thread cycles (11.4% more) for trigger acquisition plus source
retirement over EotW / Dust return / EotW. This preparation cost is separate
from gameplay-frame CPU. The shape-sharing work below addresses repeated reads
and parsing; measure its net cost before claiming that it offsets this increase.

Trigger source-retirement prediction: the prior installed-data audit found
5,576 trigger shape assets across 36 worlds and no mutable ones. After scene
initialization, capture each immutable root's mass-property result, including
any deferred error, and release the source bytes and schema while keeping the
query geometry, header and bounds. Mutable sources must remain live. Predict
one released source/schema per immutable shape and unchanged collision/query
and mass values; one additional mass-property lookup per immutable shape is
startup work. Compare actual released capacities and affected acquisition CPU
in same-build retained/released clients. Counts across all installed worlds do
not represent simultaneous residency or a process-RSS saving.

Before the release CPU measurement, the expected cost of those extra lookups
is less than 30% of trigger shape acquisition, which already parses each HKX
and constructs query geometry. Measure acquisition and retirement on the
loading thread separately, then investigate if their combined cost differs by
more than that bound. This is a startup prediction, not a gameplay-FPS claim.

Trigger acquisition overlap prediction: area/wind preparation reads shape
files through `TriggerVolume::load`, while the physics scene separately reads
trigger shape files into `TriggerAssets`. The original trigger initializer
`140779530` calls `14008c840`, which acquires the shape through the shared
resource registry `1400607f0`. At least one path is expected to
overlap in EotW because both are derived from authored trigger placements.
Record successful paths and source-byte fingerprints in a production route
before proposing a shared owner; different trigger recipes or geometry needs
may mean the apparent overlap does not exist. No saving is assumed until
the path overlap and output requirements are measured.

Trigger acquisition overlap result: the four-map production route found
400 / 184 / 31 / 400 repeated reads after matching path, size and CRC;
the potential avoided decoded bytes total 5,983,276. EotW's 204 distinct
area/wind shapes all reappear in physics, and 196 of them are also reread by
the other area/wind consumer. The original `140779530 -> 14008c840 ->
1400607f0` resolves a shared shape identity. A shared preparation owner must
keep the different world-space volume and physics query representations,
retain sources until all consumers have finished, and avoid adding unbounded
or cross-map source residency. Measure peak source/schema capacity and the
affected acquisition CPU as well as matching shape output fingerprints.
Evidence: local `artifacts/trigger-overlap-comparison.json`.

Trigger shared-source prediction before implementation: retain one immutable
HKX source and parsed schema for each shape first requested by area loading,
share it with matching physics and wind preparation, and drop the preparation
owner after wind preparation. Do not retain new physics-only or late-wind
shapes in that owner; mutable overlays retain separate acquisitions. The
measured route should avoid exactly 1,015 reads and 5,983,276 cumulative
decoded bytes, with unchanged area, wind and physics shape data. In the four
sampled maps every volume source also appears in physics, so the shared source
and physics leases should refer to the same allocation rather than duplicate
it. Count cache capacity, final sole-owner releases, per-consumer source
fingerprints and changed acquisition CPU; other maps can have unmatched
sources, so a universal peak-memory reduction is not assumed.

The production comparison avoided 1,013 reads and 5,973,956 decoded bytes.
The difference from the prediction is two wind-only inputs: water-volume
shapes are excluded from ordinary area preparation, so those inputs were not
available in the early-area cache. Source CRCs and the ordered area, wind and
physics results matched. The final cache releases were 204 / 92 / 15 / 204
sole-owned sources for EotW / Dust / Dust return / EotW. A same-binary release
comparison for EotW / Dust return / EotW, including source retirement, measured
trigger acquisition plus retirement at 193,581,570 -> 147,945,420 thread
cycles. The adjacent area-preparation totals were 171,343,830 -> 177,449,860
cycles. Together these affected preparation spans fell about 10.8%; neither
measurement is gameplay-frame time. Parent-layer sharing was enabled in both
modes and is measured separately. Evidence: local
`artifacts/shape-parent-perf-comparison.json`.

Area-environment acquisition prediction: `level_prepare` resolves every
authored environment volume once for the area selection and again for the
physics/environment list. The previous production route created 204 / 92 /
204 ordinary area shapes; each required a resolved environment in both passes.
Reusing the first resolved volume list for area preparation should eliminate
at least 500 repeated resolutions over that route, while retaining the same
area order, trigger geometry, profiles and separate live consumers. Water
volumes may add to that count. Mutable overlays require the previous separate
reads. Compare resolution/read counts and ordered profile/area signatures in
the same production sequence; this is loading work, not a frame-time or RSS
prediction.

Area-environment acquisition result: the same-binary three-map clients matched
all ordered geometry and profile signatures; resolutions fell 1,000 -> 500,
inherited reads 2,636 -> 1,318 and cumulative decoded bytes 19,793,854 ->
9,896,927. This is load I/O, not an RSS or frame-time measurement. A targeted
release CPU comparison remains pending.

Environment-volume residency prediction: after area preparation, the live
`EnvironmentVolumes` consumers read only placement identity/position, water
classification and surface material. Full placement strings and resolved
parameter layers remain duplicated alongside `AreaEnvironment` without a
runtime reader in those consumers. Move the parameters into the area owner,
publish only the fields used by physics, setup and water, and release the
remaining preparation data. Ordinary areas should avoid one parameter clone;
water profiles' unconsumed parameter layers should release entirely. Compare
the retained field capacities, resolved area/profile fingerprints and water
selection in real map transitions. This predicts lower live owner storage,
not an exact RSS or frame-time change; count actual data before reporting it.

Environment-volume residency result: retained/compact four-map clients
matched area geometry, compact volume values including Dust return's water
profile, trigger/physics and other recorded resource fingerprints. The
counted source-profile/placement capacities plus source-to-runtime vector
capacity change were 11,403,356 / 5,441,286 / 936,718 / 11,403,356 bytes
for EotW / Dust / Dust return / EotW. The area parameter capacities matched;
these are owner allocation capacities, not process RSS or a camera-in-water
selection measurement. Local evidence: `artifacts/compact-volume-comparison.json`.

Water selection allocation: `app::water_scene` converts the selected
volume's `surface_material` from `&str` to a new `Arc<str>` on each query
publication. A volume-owned Arc could replace that allocation with a reference
clone, preserving the material string and selection order. The gain is one
allocation per completed publication with a selected material; the actual
publication frequency and affected CPU have not yet been measured. Verify
selection and measure the affected function over a moving underwater workload
before changing this per-frame path.

Localization values: `assets::lang::StringTable` stores a separate owned copy
of each translated value in both the key and numeric-ID maps. Sharing one
immutable value between the two indexes could remove approximately one value
allocation per entry, offset by the shared-owner header and representation
cost. The original's ID lookup is `1401653e0`. Count actual translated value
bytes and profile both lookup paths before choosing a representation; keeping
borrowed `&str` results and lookup behavior is required, and no memory or CPU
saving is yet measured.
An installed-data scan counted 6,226 uncomplicated Russian entries containing
1,026,738 UTF-8 value bytes; escaped or otherwise complex entries are excluded.
Those values are duplicated by the current two-map parser. This is a lower
bound on logical duplicated text, not the net saving after shared-owner and
allocator overhead. Evidence: local `artifacts/language-value-inventory.json`.
The existing original EotW capture reports 547,888 bytes under LANGDICT; its
fixture does not identify the active language, so it is not a comparable
Russian-table budget.
Before the ownership change, predict that sharing each immutable value will
remove roughly one full translated-value allocation per entry, offset by the
shared allocation header and different hash-map slot widths. Lookup remains
one hash-map search followed by a borrowed string view, with no runtime lock or
reference-count change; compare the two lookup paths on installed values and
measure table capacity before claiming net savings.
The first same-binary release diagnostic on the Russian table measured 6,226
entries, 2,530,346 bytes of former value-string capacity, and 1,026,738
shared UTF-8 payload bytes. The current toolchain lays out the corresponding
Arc allocations at 1,148,208 bytes including headers and alignment; the
value allocations therefore request 1,382,138 fewer bytes, excluding
allocator bookkeeping and process RSS. Both map value slots also shrink
from String to Arc<str>; their physical bucket sizes are not included here.
In one 16-sweep diagnostic over installed keys, key lookups measured
29,449,350 -> 22,949,640 thread cycles and ID lookups 6,306,265 ->
5,395,635. A repeat measured 31,738,731 -> 23,562,945 for keys and
5,440,890 -> 3,539,725 for IDs; another run was 30,447,374 -> 23,042,880
and 9,642,675 -> 6,033,265. Numeric-ID differences vary, and smaller map
slots and different hash-table seeds can affect cache behavior; the runs
show no detected lookup regression but do not establish a fixed speedup.
These are affected lookup spans, not frame time. Original
LANGDICT capture language is not identified and cannot be used as a
same-language memory comparison. Evidence: local language-perf logs.
The same parser also clones every original key into a temporary collision map
and computes its ID twice. Moving the key into the map through its entry API
should remove one key allocation and one key-hash computation per parsed
non-colliding entry, while retaining the original key in collision errors.
That changes startup parsing only, not lookup. Count entries and verify the
collision behavior and output against installed data before accepting it.
The follow-up diagnostic on the 6,226 installed Russian keys removed 6,226
temporary key clones; isolated index preparation measured 3,726,310 ->
2,902,970 thread cycles, and a repeat 5,781,860 -> 3,577,770. These are
short startup spans rather than frame timings. The same error message uses
the retained original key for collisions; that error path was reviewed in
source, not exercised by the successful installed-table run.
Localization key lookup also allocates a lowercase String even for keys
that are already lowercase. Borrowing such keys could avoid one allocation
per qualifying call, but would add a byte scan; the actual key mix, call
frequency and affected CPU are not measured. Profile those before changing
this hot lookup, preserving ASCII-only case semantics and missing-key behavior.

Area containment work: environment tracks independently traverse the same
area list and call `contains` for their current point. Reusing one ordered
selection per view/update could remove repeated geometry queries, but the
affected CPU time and the number of genuinely shared points are unmeasured.
Track timing and positions first; preserve query timing, area precedence and
independent camera/body views before changing this gameplay-frame path.

Environment declaration layers: each profile currently owns its parsed
inheritance layers as separate strings and vectors. A shared parent declaration
is expected to occur in more than one EotW profile. Count distinct layer
content, duplicate capacities and actual owner lifetimes before considering
sharing; preserve per-profile layer order and hot-path parameter lookup cost.
No resident saving is claimed from inheritance depth alone.

Environment parent preparation prediction: direct installed entity-text
inspection found 204 Edge of the World area environment strings using two
immediate parent names (119 and 85 entries), plus one separate shrine entry.
Dust has 92 area entries using eight immediate parents plus one shrine entry;
Dust return has 16 entries using four parents. A level-scoped cache of parsed
immutable parent layers should avoid at least 202 + 84 + 12 + 202 = 500
immediate-parent reads on the four-level route, with additional savings for
shared ancestors. Root inline parameters remain per volume. Clone the parsed
layers into the existing live representation so layer precedence, consumers
and gameplay lookup do not change; release the temporary cache at the end of
preparation. Bypass sharing for mutable overlays. Compare full profile/area
signatures and changed loading CPU; temporary cache capacity and persistent
profile memory are separate measurements. Evidence: local
`artifacts/environment-parent-scan.json`.

A subsequent installed-data scan found 2,853 environment strings across all
36 map entity files, with at least 2,738 repeated immediate-parent occurrences
within their respective maps (the report retains at most 20 parent names per
map). This establishes a broad candidate, not a measured saving or proof of
runtime equivalence for all maps. Evidence: local
`artifacts/environment-parent-all-maps.json`.

Map-info fallback ownership prediction: when an indexed declaration cannot
be read, `disable_index` permanently switches this registry to reference scans
and clears its index and declaration table. Its `resolved` table is then
never read again but still pins prior declaration Arcs. Clearing that table at
the same transition should release only cache-owned references; existing
callers keep their leases. The normal indexed path gains no work. This is an
error/fallback lifetime correction, not a predicted gameplay CPU improvement.
Replacing the unused resolved map with an empty map, instead of only clearing
its entries, also releases its bucket allocation at that transition. The size
depends on the number of earlier lookups and has not been measured.

Physics-material targeted CPU prediction: the shared path eliminates 687 of
749 observed source reads over the three-map route, while adding source-map
lookups and temporary string copies. CPU cycles inside the changed reader
should fall overall; its archive I/O wait and the separate declaration parser
are not whole-frame measurements. Measure the reader on both modes in the same
client executable and investigate a result more than 30% away from that model.


Archive / rig / read-history production result: the no-overlay route
skipped normalization for at least 68,608 archive acquisitions. Player rig
loads fell 15 -> 9, with counted skeleton and decoded declaration inputs
6,736,494 -> 3,984,234 bytes and matching unique rig fingerprints. At the
last pre-play checkpoint, read-history normalization operations fell
68,608 -> 282 and kind-key allocations 68,608 -> 138; the new run also
made 36 fewer total reads due to the rig handoff. The changed function's
instrumented thread cycles per read were 7,714 -> 5,541 in one run per
mode; cycles are not elapsed time. Progress snapshots and all recorded
resource fingerprints matched. Full values and limits are in the parity
ledger and local `artifacts/archive-overlay-comparison.json`.

Contact declaration text handoff prediction: all 11,165 installed resources
in the physics-material, sound-event and contact-FX declaration classes are
ASCII with no bytes requiring the 140135f70 signed-byte normalization. The
current reader borrows the decoded archive bytes, then allocates and copies a
String. Reusing the owned archive buffer for that fast case can avoid that
copy; non-ASCII/control input must use the existing normalization path.
Archive buffers reserve an extra probe byte, so shrink the retained text to
its previous capacity and measure the realloc/copy cost rather than claim all
allocations disappear. On the recorded first EotW load, contact acquisition
read 9,346 declarations / 1,472,416 decoded bytes; expect most payloads
to use the direct path. Compare exact
ordered declaration fingerprints, text capacities and affected-function CPU
with the former copy path in the same production executable. No gameplay-frame
or resident-memory saving is predicted. Installed-data inventory:
`artifacts/contact-text-inventory.json`.

Contact database reuse prediction: the installed common manifest lists one
ContactSystemDB and the installed catalog contains one such declaration,
`default_db.decl`, with 7,407 authored records and 433,879 decoded bytes.
`ContactSounds::Definitions::load` currently reads and parses it anew per map;
1400661e0 preloads common resources and 1400607f0 resolves their existing
identities. One immutable catalog-scoped Arc should reduce EotW / Dust / EotW
from three reads/parses to one (867,758 cumulative decoded bytes avoided),
while keeping per-map contact chains/effect state independent. Replacing the
catalog must release the prior cache lease; an editable database overlay keeps
the old per-acquisition behavior. Compare database fingerprints and actual
contact outputs, acquisition CPU and identity/lifetime over real transitions.
The parsed common database stays resident between maps, so this is a reuse
and startup-cost prediction, not a claim of lower between-map live bytes.

Contact declaration buffer result: four-map production diagnostics matched
contact text, physics, sound, material, animation, UI and probe signatures;
retained text capacities also matched. In the release EotW / Dust return /
EotW run, the allocator moved 9,826 of 9,827 supposedly reused buffers during
capacity trimming. Conversion CPU cycles were 15,582,221 -> 14,430,922, but
this still performs nearly all copies and the small single-run difference does
not justify the extra path. The change was reverted. Evidence: local
`artifacts/contact-copy-comparison.json` and `artifacts/contact-copy-perf-*.err`.

Contact lookup temporary allocation prediction: `ContactDatabase::effect`
constructs a material-pair path with `format!` and then allocates a second
String to lowercase it for each pair considered. Lowercasing the first String
in place preserves the exact ASCII case rule and should remove one temporary
allocation per examined pair. The original uses 140003cd0 for pair-name
construction. Its gameplay call frequency and affected CPU are unmeasured;
measure the changed lookup during moving gameplay before claiming a speedup.

Contact database reuse result: on EotW / Dust / Dust return / EotW, the
baseline read and parsed four times; the shared owner read and parsed once,
avoiding 1,301,637 cumulative decoded bytes. All four source fingerprints
were 3c118c31 and parsed 7,407-entry fingerprints were 9c8476f2; the shared
owner identity remained the same, and recorded physics, contact, sound,
material, probe, 800 animation and 430 UI bitmap fingerprints matched. The
308,145 bytes of retained value String capacities exclude hash buckets and
allocator overhead. A three-load same-binary release comparison measured
26,196,065 -> 9,042,530 acquisition thread cycles. This is loading-thread
work, not frame time; the database remains resident between maps. Mutable
loose database overlays retain separate reads; that branch was source-reviewed,
not exercised by these installed-data runs. Evidence: local
`artifacts/contact-db-comparison.json` and `artifacts/contact-db-perf-*.err`.

Contact lookup allocation result: the established moving-player route recorded
186 baseline and 179 candidate lookups, each examining one material pair; all
recorded input/effect fingerprints matched. The affected lookup averaged about
6,847 -> 6,592 thread cycles per call with per-call sampling overhead included.
The roughly 3.7% difference is too small to establish a useful gain in these
unequal runs, so the in-place lowercase candidate was discarded. No frame-time
claim is made. Evidence: local `artifacts/contact-walk-*.err`.

Initial probe-buffer staging prediction: `app::setup` creates a ShaderBuffer
from `records.clone()`, but `MaterialResolver::upload_probe_records` replaces
that data after all per-instance records are appended and before the setup
asset batch is published. No production code reads that initial buffer in
between. Bevy's `ShaderBuffer::from` serializes into a temporary buffer and
copies it into the asset; the EotW initial 3,312 records are 476,928 bytes.
Reserve the same handle with an empty descriptor and fill it once at the
existing final handoff. Predict removal of the initial record clone and two
serialized 476,928-byte buffers, while final buffer bytes, descriptor and GPU
upload stay unchanged. Validate both direct and worker setup publication,
final buffer fingerprints and counts, and affected setup CPU; this is a
transient allocation result, not the separate 14.9 MB live-buffer question.
The existing vertex-probe asset uses render-world ownership as a host mapping
of original probe GPU preparation (1402b15e0).

Probe-buffer residency investigation: EotW reports 103,489 per-instance SH
records; including the sentinel, its CPU ShaderBuffer contains 14,902,560
bytes. Dust's 151,633 records correspond to 21,835,296 bytes. The existing
`ShaderBufferPatches` needs those bytes to compare and apply 144-byte updates
for moving models and arms; simply marking this buffer render-only would make
those updates fail. The original `14035bca0` destroys both backing pointers of
its indirect-lighting structured buffer; the EotW INDIRECT_LIGHTING tag is
61,751,560 bytes for the wider subsystem, not a direct budget for this buffer.
A possible host representation would upload the full initial buffer once, then
retain exact previous values only for offsets that actually receive dynamic
updates. First use of an uncached offset may require one redundant GPU write;
capacity checks, writes before GPU preparation, overlapping updates, map teardown
and eventual GPU asset loss must remain correct. Measure actual moving offsets,
unchanged/changed writes and affected CPU/GPU work before deciding whether this
saves memory without a performance or fidelity regression. Do not discard the
full backing until those lifecycle paths are handled and measured.
The existing render-side pending-patch queue retains an entry while its GPU
asset is absent, with no outgoing-level cancellation. The existing render-world level-release acknowledgement runs after outgoing
producers stop, and can clear this level-owned queue there. The main-world
queue must also be cleared at teardown: its usual First-schedule reset is
gated during loading. Predict zero retained pending or extracted patches for
an abandoned old buffer and no change to current-level writes. Verify the
transition ordering and exercise the production route; do not rely on a
one-frame cancellation message while rendering is gated.

World matrix upload prediction: `WorldMatrices::get_or_upload` creates one
immutable 48-byte-per-instance GPU stream shared by world shadow and water
consumers; those consumers only retain its handle. The level's independently
owned source matrices remain available to CPU queries. Bevy currently retains
a second full serialized CPU ShaderBuffer after upload. Moving this one-shot
buffer to render-world ownership should release precisely 48 times the uploaded
matrix count in CPU buffer length after extraction, with the same uploaded bytes,
one GPU buffer and no gameplay-frame work. This is a host resource-lifetime
optimization, not a claim about the original's internal matrix layout.
Original metadata 14150e060 names a world-matrix buffer view, 14155b100
identifies arkMat3x4 as 48 bytes, and world teardown is 1400410f0; compare buffer identities and upload
fingerprints and measure actual retained capacities before keeping the change.

Initial probe-buffer staging result: on EotW / Dust / Dust return / EotW,
final probe buffer byte lengths and CRCs matched (14,902,560 / 089f05a5,
21,835,296 / d496779e, 7,066,368 / 62ff7f7f, 14,902,560 / 089f05a5);
upload counts for these byte lengths and the recorded resource fingerprints
matched. In a three-load release comparison, initialization of the discarded
staging buffer used 1,713,495 -> 4,935 loading-thread cycles. The ordinary
worker-based production setup was exercised; the direct setup handoff was
source-reviewed. These numbers describe staging work, not gameplay frame
time or process RSS. Local evidence: `artifacts/probe-staging-comparison.json`
and `artifacts/probe-staging-perf-*.err`.

Reflection buffer sweep: `app::environment_reflection` also creates
immutable plane, cube-index and water-probe-info ShaderBuffers. Production
consumers appear to retain GPU handles only; the probe set keeps independent
CPU query data. If confirmed through acquisition and transition, the same
render-world handoff can retire their serialized CPU payloads without changing
GPU bytes or frame work. From the existing EotW trace (104 reflection cubes and 3,312 primary probes),
the encoded plane, cube-index and water-probe-info buffers are expected to be
6,672, 416 and 26,496 bytes respectively if the water consumer is present.
Verify those counts and output fingerprints in the production acquisition
before keeping the change; these are much smaller than the world-matrix and
per-instance lighting buffers.

Physics-shape loading-history prediction: the three-map production trace
retains 685 physhape path identities (75,998 String-capacity bytes) before
the first transition and 1,152 (122,949 bytes) by the third. The temporary
HKX source owner, scene collision data, environment volumes, machinery and
physics world are all level-owned; decoded collision geometry can outlive its
source, so retire the path identities only after all outgoing level resources
and publications are released. Original resource class purge is 140060180.
The installed common list contains 116 physics-shape names; preserve that
scope while checking whether those shapes have a session owner. Prediction:
the first outgoing level releases its map-only shape names, returning to a
map no longer precredits those released names, and later acquisitions
re-register them. Common resources stay untouched. Compare count/capacity, acquisition progress,
shape/physics fingerprints and live owner scope across the four-map route.
This changes loading progress/history memory, not physics geometry or per-frame
CPU. Verify no session-scoped shape owner survives before keeping the change.

Loading-history path storage investigation: the four-level trace reaches
22,515 recorded paths with 1,889,380 bytes of String capacities, excluding
hash tables. The catalog already owns a normalized archive path table and
resolves an index during each successful read. A history representation keyed
by catalog identity and existing archive entry index may avoid retaining a
second String per indexed path; loose overlay-only entries still need stable
names. Predict no change to actual resource ownership or GPU work, and lower
history allocation only for paths proven present in that catalog. Before any
change, measure indexed versus overlay-only reads, preserve kind and catalog
isolation, and account for the index lookup cost in manifest progress and
owner-driven forget. Loading read-history CPU must be measured at the changed
functions; do not claim that all 1.89 MB can be removed.

The archive index contains about one million names in packed storage, so a
one-bit-per-entry residency map would have a roughly 125 KB bit payload at
that scale, versus the 1.89 MB path capacities observed in history alone.
This is only a design bound: counts by kind, coverage reports, the bootstrap
reads before level selection, catalog replacement and owner-driven single-path
and whole-kind retirement all need exact handling. `Catalog::read` currently
records overlay reads only when an indexed entry also exists, and
`read_prefix` follows a different reporting path; preserve or explicitly
resolve those semantics. No implementation or CPU saving is claimed yet.

Archive lookup normalization prediction: `Catalog::entry` currently calls
`replace` and `to_ascii_lowercase` for every lookup, even when the input
already contains no backslashes or ASCII uppercase. A borrowed fast path for
those canonical paths should remove both temporary String operations while
keeping the existing normalization for other inputs and the same binary
search and resource identity. Prior loading-history diagnostics counted
68,326 canonical path reads out of 68,608 in one three-level run, though
Catalog::entry has additional callers and its actual mix must be measured.
Measure only the affected lookup's thread cycles and canonical/noncanonical
counts with both paths in one production build, compare acquisition
fingerprints and overlay behavior, and check no gameplay-frame regression on
streaming call sites. This predicts transient allocation and lookup CPU
reduction, not a resident-memory or GPU saving.

Installed-manifest scope check: `common.mapResources` names 116 physhape
and 108 massprops records. EotW's and Dust's map-specific lists name 1,431
and 1,293 physhape records respectively; those lists share 384 names with
each other and none with the common list. The 622 unique paths observed in
the four-level area/physics/wind ShapeSources trace include no common-list
physhape. Other HKX consumers read through different code, so this does not
prove that common shapes are unused by those consumers or other maps. Do not
discard all shape history at a map transition; measure complete shape paths
and implement ownership according to scope.

World matrix staging prediction: `ShaderBuffer::from(matrices.to_vec())`
clones all matrix rows, encodes them into a temporary Vec, and copies that Vec
again into the asset. The dependency's `ShaderBuffer::set_data` can encode a
borrowed slice into the asset's final Vec directly. Predict removal of two
temporary full-buffer allocations and copies while preserving every encoded
byte, final GPU upload and source matrix ownership. Compare per-map byte
fingerprints, upload counts and affected loading-thread cycles. This is
transient memory/loading CPU, distinct from the retained CPU-copy reduction.

Reflection staging sweep: each of the three small immutable reflection
buffers also uses `ShaderBuffer::from`, which copies the encoded Vec once
before upload. The predicted EotW temporary copy is 33,584 bytes in total
when the water consumer is present. This is much smaller than the matrix
stream; keep the straightforward code until production byte counts and
affected loading cost establish whether an additional shared serializer
helper is worthwhile.

Original scope qualification: the conditional 36-type promotion table at
`142280ee0`, used by `1400661e0` for eligible map loads, includes
`massprops` but not `physhape`. Common/init loads take a different branch;
constructor `14005e740` also derives lifetime from the active allocation
scope. Therefore list membership alone does not establish a resource's
lifetime. Use the actual owner/scope evidence when implementing retirement,
and do not apply the same rule to shapes and mass properties.

`14006a910` confirms the distinction: it pushes heap 2 around
`1400661e0` for a named map, but does not push that heap for init or common.
A map-listed physhape is absent from the conditional promotion table; the
common/init entries inherit their caller's active scope. This supports
separating map and common shape identities, while leaving the inherited
common scope to be traced instead of assuming it.

All installed manifest scopes: 36 map lists contain 12,457 distinct
physhape paths (24,660 distinct per-map name occurrences; 4,316 paths
occur in at least two maps) and 6,377 distinct massprops paths (18,106
per-map name occurrences; 4,127 recur).
No map list repeats a common-list path of either kind; init names neither
kind. This is installed-list coverage, not evidence that every listed
resource is actually acquired or held simultaneously. Local inventory:
`artifacts/shape-manifest-scopes.json`.

Actual four-level ShapeSources requests were narrower than the manifests:
EotW / Dust / Dust return / EotW used 332 / 259 / 33 / 332 distinct paths.
Dust and Dust return each requested just one path seen in a previous map
(18,612 decoded source bytes); returning to EotW reread its 332 paths,
1,516,640 decoded bytes, after its previous map owner was released. Keeping
all former EotW parsed sources just for return would retain their source and
schema between maps; the prior EotW trace counted approximately 1.5 MB source
capacity and 5.6 MB schema capacity. This is a workload-specific tradeoff,
not evidence that the original holds these map-class shapes across teardown.

Particle-image name normalization sweep: both `ParticleImages::image` and
`volume` own the String returned by `image_load_resource`, then call
`replace` and `to_ascii_lowercase` before storing the shared key. In-place
ASCII lowercasing and conditional slash replacement could avoid temporary
Strings while preserving intrinsic image names and exact normalization.
The four-level client recorded only 108 / 148 / 67 / 108 image requests;
the likely impact is loading-only and small. Measure the affected path and
all resolved image identities before adding a separate normalization helper;
the higher-volume archive lookup is being addressed first.

Image resolver duplicate lookup: when `image_load_resource` is passed an
already indexed name, it calls `Catalog::contains(name)` and then checks the
same name again before returning it. On an immutable catalog, returning the
already confirmed name can remove the second lookup without changing the
fallback decision. Mutable overlays may change between checks and need their
existing behavior unless a coherent snapshot is established. Request frequency
and affected lookup CPU are unmeasured; compare resolved identities and
fallbacks on installed data before retaining any change.

Physics-shape cross-owner audit: `ShapeSources` shares immutable area,
trigger and wind HKX sources during level preparation, but static model
collision in `app::view_geometry::load_model_havok` separately reads and
parses its own paths, including aliases. Original trigger acquisition `140779530` and model-body acquisition
`140808ea0` both call `14008c840 -> 1400607f0` on the same physhape
registry at `142bf8670`. Compare
actual per-level model-source and machinery wind-source names, bytes and
fingerprints against the existing ShapeSources trace before considering a
common acquisition owner;
preserve alias transforms and the derived triangle lifetime. No overlap or
saving is assumed before that inventory.

Sparse probe-buffer result: the four-level development comparison produced
matching initial buffer CRCs and no missed changed records in the instrumented
control. At the EotW checkpoint the sparse pages plus directory used 141,180
counted bytes instead of the 14,902,560-byte full CPU data buffer; Dust used
199,332 instead of 21,835,296 bytes. These are allocation payload capacities,
not RSS. In the same-binary 75-second walking comparison, the backed control
recorded 341,342 updates and 279,582,567 targeted thread cycles (819 per call);
the sparse version recorded 345,565 updates and 346,604,744 cycles (1,003 per
call). This timer excluded part of the sparse wrapper and included sampling
overhead, so it does not establish the complete per-call cost. The two runs had 103 first-record writes and no observed
redundant first write; the control audit found no missed changes. The savings
do not meet the no-performance-regression requirement, so the sparse-buffer
change is discarded. Initial staging elimination and outgoing queue cleanup
are independent. Local evidence: `artifacts/probe-sparse-walk-*.err` and
`artifacts/probe-sparse-comparison.json`.

Machinery wind-shape reuse prediction: in the real Dust level the machinery
recipe loader reads ten wind-shape sources but only two distinct paths; all ten
have identical length and checksum for their respective paths. The same
per-body loop also reads mass properties using that same shape key, so those
ten requests necessarily include at least eight repeated mass-property reads.
The original `14008be50` checks for an existing property on a physics shape
and constructs `14008b830` only when absent. A level-preparation cache
keyed by the authored shape path should reduce those ten wind reads/parses
to two and avoid 17,056 cumulative decoded source bytes; cache the immutable
principal mass value before per-body overrides. Share the immutable aero-face
and hull arrays between recipes and runtime bodies, while keeping mass,
speed, motion, activation and trigger state per body. Do not cache mutable
overlay data. Predict no per-frame refcount operations and identical per-body
geometry, masses and wind outputs. Measure per-map requests/read counts,
affected acquisition CPU, retained capacities and output fingerprints in
paired production clients before keeping this change.

Archive lookup first release result: four-level clients made 336,410
lookups, 335,946 already canonical, in each mode. Of sampled canonical
lookups, the control used 11,164,472 cycles / 5,253 samples and the borrowed
path used 10,249,009 / 5,254 (about 8% lower per sample). The 464 sampled
noncanonical lookups used 1,380,153 -> 1,555,992 cycles, about 13% higher.
Those are affected-function thread cycles including instrumentation, not
frame time; this first candidate failed the no-regression requirement. The
fallback now replaces backslashes into one owned String and lowercases it
in place, rather than allocating a second String after the initial scan.
Predict identical normalized bytes and no fallback slowdown; repeat both
canonical and noncanonical measurements before keeping it. Local evidence:
`artifacts/immutable-perf-separate.err` and `artifacts/immutable-perf-shared.err`.

World-matrix production result: EotW / Dust / Dust return / EotW encoded
4,507,776 / 6,887,376 / 2,301,360 / 4,507,776 bytes, respectively. CRCs
matched across old and new encoders (02e24cfb / 7c1f69fd / 16df2b5d /
02e24cfb), and the matching buffer-size GPU upload counts were unchanged.
The retained CPU data payload is removed after extraction, while the
independent source matrices remain for queries. Four loading-thread
serialization samples totaled 34,816,810 -> 11,531,730 cycles, with two
full-size staging allocations/copies eliminated. This is not a gameplay
frame-time or process-RSS result. Reflection buffer CRCs and exact expected
sizes also matched; their combined EotW CPU data length was 33,584 bytes.
Production output signatures for physics, contact, material, animation,
UI bitmap and probes matched in the development comparison. Local evidence:
`artifacts/immutable-buffer-results.json` and `artifacts/immutable-perf-*.err`.

Image resolver inventory: on EotW / Dust / Dust return / EotW the real
client made 25,310 image-resolution requests, of which 216 (0.85%) were
already direct resource names. Both runs recorded the same counts. Removing
the second existence check would affect only those 216 calls on an immutable
catalog, so it is not being retained without evidence of a meaningful gain.
The fallback and mutable-overlay behavior remain unchanged. Evidence: local
`artifacts/immutable-buffers-*.err`.

Cross-owner shape inventory: the four-level production trace recorded
370 / 309 / 49 / 370 model-shape reads; none of their paths overlapped that
level's area/physics/wind ShapeSources requests. Dust additionally made ten
machinery wind-shape reads for two distinct paths, also with no overlap
against the area cache. One common-list coin shape appeared in the complete
history and was read by the model path three times across the sequence
(2,132 decoded bytes each). Do not add a broad cross-owner cache solely on
the area/model hypothesis; the repeated machinery inputs have their own
measured same-owner reuse, and common-shape retention needs its own CPU and
memory accounting. Evidence: local `artifacts/immutable-buffers-shared.err`.

Model shape alias prediction: production tracing found 17 repeated
static-model shape-source reads in Edge of the World (146,596 decoded bytes),
17 in Dust (85,188 bytes), and none in Dust return. Repeated path/byte CRCs
matched within each level. The per-level model cache stores root shapes but
the recursive alias loader bypasses it. `140808ea0 -> 14008c840 ->
1400607f0` requests shape identities from the shared resource registry.
Routing alias targets through one level-owned shape cache should eliminate
those repeated reads and parses, while retaining independent scaled
triangle results and the existing maximum alias depth. Mutable overlays
must preserve their current read behavior. Measure source identities,
transformed collision output fingerprints, peak cache capacities, and the
affected preparation CPU before retaining the change.

Model alias development comparison: EotW / Dust / Dust return / EotW
read 370 / 309 / 49 / 370 sources before sharing and 353 / 292 / 49 / 353
afterward, matching the predicted 17 / 17 / 0 / 17 reduction. Source identity
and CRC sets, transformed collision geometry CRCs and body/triangle counts
matched; the other recorded physics, material, animation, image and probe
fingerprints matched as well. Extra cached triangle capacity was 8,208 /
31,896 / 25,056 / 8,208 bytes respectively, plus keys and hash buckets,
and is temporary. Even a level with no avoided source reads can require
extra alias entries. An installed-index inventory identified 1,138 alias
headers, all one link to indexed targets with no observed cycles or missing
targets; this is not evidence about modded resources. Release measurements
of the affected loading path have since been taken. The measured Dust return
span rose from 6,367,863 to 8,216,457 thread cycles while it saved no reads;
the other three spans fell. These are single-run loading measurements and do
not establish a gameplay-frame change. The speculative alias-target cache
and its extra temporary triangle storage were removed; the existing root cache
and the central physics-shape ownership remain. No area/model source overlap
was observed, so that speculative sharing was also removed.
Local evidence: `artifacts/model-alias-*.err` and
`artifacts/model-alias-comparison.json`.

Archive lookup follow-up: with every affected lookup sampled during the same
75-second player walk, 111 canonical calls used 959,525 -> 782,565 thread
cycles. No noncanonical calls occurred during the walk. A separate loading
comparison of 61 noncanonical calls fluctuated between a small gain and a
small loss across runs, so it does not establish a fallback speedup. This is
the changed lookup's thread time with diagnostics, not whole-frame CPU.
Local evidence: `artifacts/archive-entry-walk-*.err`.

Machinery cache development result: EotW / Dust / Dust return / EotW
requested 87 / 93 / 20 / 87 mass-property resources. The control performed
287 reads (163,016 decoded bytes); the shared preparation owner performed 53
(30,104 bytes), avoiding 234 reads and 132,912 cumulative decoded bytes.
This is much more reuse than the initial lower bound of eight: the trace
shows that many additional bodies share the same mass-property paths, not
just the ten wind bodies. Dust's wind shape requests fell from ten reads
(21,320 bytes) to two (4,264 bytes), as predicted. Per-body mass, position,
orientation, wind-face/hull and speed fingerprints matched in each level:
c90d7b68 / e3773255 / 198e2cb7 / c90d7b68. Recorded physics, contact,
material, animation, bitmap and probe signatures also matched. These are
loading input bytes, not resident-memory or gameplay-frame savings; the
temporary cache and affected loading CPU are being measured in release.
Local evidence: `artifacts/machinery-cache-*.err`.

Release measurements of the same affected preparation spans totaled
14,579,244 -> 3,367,609 thread cycles for mass acquisition over four loads;
the Dust wind acquisition span was 853,020 -> 236,983 cycles. The Dust
temporary caches had 18 mass entries and two wind entries; key capacities
were 3,852 and 430 bytes, plus 1,008 bytes of wind-vector payload. These
exclude hash-table and allocator overhead. Neither result measures gameplay
frame time or process RSS. Local evidence: `artifacts/machinery-perf-*.err`.

Machinery resource ownership prediction: moving the existing preparation-only
mass cache to the physics mass owner and acquiring wind HKX through the shared
shape owner should preserve the measured 53 mass reads and two Dust wind reads
on the four-map route. A wind shape already held by an area owner can avoid one
additional read; no such overlap was observed on the sampled route. Cached mass
values remain copied before body-specific overrides, and all preparation caches
retire after their consumers. This changes no per-frame code, draw or GPU work.
Compare body fingerprints and the affected loading spans; do not infer a frame
time improvement or a physical-memory saving from avoided decoded bytes.

Physics-system acquisition investigation: the MD6 parser in the executable
(`1401c9630`) requests its ragdoll system via the `physSystem` registry and
`14076abc0` builds independent runtime bodies from it. Machinery currently
reads and parses one system for each eligible placement, then consumes those
recipes in preparation. Before adding a cache, count distinct paths and
matching source fingerprints in production levels. If requests are all
unique there is no demonstrated duplicate acquisition to remove; a cache
would add retained preparation memory. This is loading-only, not an assumed
per-frame improvement.
The saved original EotW allocation snapshot reports HAVOK 103,479,168 bytes
and PHYSICS 38,820,534 bytes; these tags include many physics structures and
do not isolate these systems or justify attributing their totals to a cache.
Moving the existing system and ragdoll reads plus parsing into their physics
resource owner is expected to keep the same read count, error behavior and
decoded data. It changes preparation only and should have no gameplay-frame
cost. The caller will still construct independent bodies; any actual sharing
must be justified by the per-level duplicate-source inventory first.
The production inventory has now found 4 / 3 / 1 / 4 system requests over
EotW / Dust / Dust return / EotW, but only 1 / 2 / 1 / 1 distinct paths per
level; ragdoll maps show the same counts. Repeated inputs match by size and
CRC. Prediction: a preparation-scoped owner sharing immutable decoded system
and ragdoll definitions reduces both classes from 12 to 5 reads, avoiding
147,840 and 4,256 cumulative decoded bytes respectively. Independent body,
constraint and animation state must remain per instance. The ragdoll mapping
can remain shared and immutable in runtime skinning; only setup clones its Arc,
and iteration must not acquire locks or refcounts per frame. Compare source,
body/constraint/map fingerprints, retained capacities and affected loading
CPU, including maps with no repeated inputs. Mutable overlays stay uncached.

Sound identity audit to perform: prepared media caches use `(sound ID, source
length)` across banks. The original sound registry owns a source by sound
identity, but the key alone does not prove that same-ID, same-length sources
from different installed banks have identical bytes. Inventory actual reused
keys and their encoded fingerprints before changing the key or its lifetime.
Prediction: if the installed bank registry guarantees one payload per sound
identity, there is no affected-content change to make; if it does not, a
collision needs an owner/source identity rather than a speculative cache flush.
Four-map acquisition tracing recorded 100,343 resident requests across 9,281
keys without differing same-size payload CRCs. Streamed requests used 2,798
keys; 401 keys pointed into more than one installed package location. Comparing
the complete contents at those locations read 197,348,728 bytes and found no
different payloads. This covers requested resources on the sampled route, not
every installed sound or mod. No cache-key change is justified by this result.
Local evidence: `artifacts/audio-package-identity-comparison.json`.

Image read-history investigation: before the fourth load on the sampled
route, the progress registry retained 6,185 image names with 567,748 bytes
of String capacity. This is bookkeeping, not image payload or RSS. Scene
images live in `SceneRenderQueue`; common images can move into `SharedImages`,
and named UI bindings can retain `ResidentImages` weak leases across a purge.
Do not clear the entire image kind or treat a path merely present in the common
manifest as a live image. Predict that retiring only identities whose last
reusable image owner has released them will remove stale precredits at the
next map, with no per-frame work. Measure removed/retained names and compare
actual scene/loaded UI identities and acquisition on repeated map transitions;
loading-only pruning work and temporary path storage must also be counted.


The four-map development machinery-owner comparison matched all 292 body
records and their mass/pose/wind fingerprints, plus the other recorded resource
signatures. Its Dust wind acquisition span is not a valid timing comparison:
the new owner logged and checksummed each source while the control did not.
Disable that source tracing in the release comparison before deciding its cost.

Shape read-history prediction: the rewrite retains no session physhape source
registry; its temporary preparation owner and the published world are the
current scopes, while scene geometry and mutable trigger sources are level
owned. Give preparation and its published level one shared scope lease and
forget physhape read identities only after the last such scope is gone. This
is a host bookkeeping correction; original 140060180 purges resource classes
according to scope and the original may retain common identities that this
rewrite does not currently cache. On the measured route, 685 shape names were
present before the second map and 1,152 before the fourth; predict those stale
names are removed at scope release, not an RSS or GPU-memory reduction. Measure
actual name/string/table counts, cleanup cycles, loading precredits and physics
geometry signatures across map transitions. The guard changes no gameplay
frame work. Massprops is not included in this change: its original conditional
promotion and owner lifetime must be handled separately.

Ragdoll mapping representation follow-up: the first sharing candidate uses
`Arc<Vec<RagdollEntry>>`, adding an extra pointer dereference in skinning. Use
`Arc<[RagdollEntry]>` instead so the per-instance view carries its data pointer
and length directly; this also avoids a separate Vec allocation per definition.
Constructing the shared slice copies once per distinct map during preparation.
Predict identical mapping entries and no additional per-frame reference-count
operations; compare the affected skinning function during a moving-player run
before claiming that gameplay CPU is unchanged.

Contact declaration history prediction: the session `COMMON_TEXTS` retains
common contact FX, physics-material and sound-event texts; `DeclarationTexts`
contains a separate level map (382 / 389 / 99 entries in the sampled levels).
When `audio::begin_level` resets ContactSounds after world FX publications are
removed, forget only the level map's indexed identities, leaving all common
entries and unrelated FX owners alone. Predict at most one removed history
name per released level entry, unchanged declaration contents and future
acquisition, no gameplay-frame work, and one short batch of indexed lookups
at transition. Count removals, String/table capacity and affected transition
cycles; do not infer that the entries are all distinct sound events or that
this measures payload RSS.

Image-history development comparison: the three transitions removed 4,603 /
4,157 / 2,324 stale image names and 424,658 / 380,021 / 213,370 bytes of
String capacity. The hash-set capacity changed 7,168 -> 448, 7,168 -> 448,
and 3,584 -> 448 slots. These are bookkeeping capacities, not image payload,
physical memory or GPU savings. Named resident-image load records and scene
registered-view reuse matched; all recorded physics, contact, material,
animation, UI bitmap and probe signatures matched. Precredits decreased as
expected. Transition CPU still needs release-profile measurement; the existing
UI and shared image leases remained in their owners. Local evidence:
`artifacts/image-history-*.err` and `artifacts/image-history-comparison.json`.

Machinery definition development comparison: the system and ragdoll source
reads both fell 12 -> 5 over four loads, with all twelve per-instance definition
fingerprints matching and independent runtime body creation unchanged. The
first version used Arc<Vec>; the follow-up uses Arc slices and its moving-player
skinning timing must be measured separately. Local evidence:
`artifacts/image-history-*.err`.

Mass-property history prediction: no rewrite session owner retains massprops
resources after machinery setup; body instances hold copied mass values. Keep a
separate mass-history lease in the mass source and published machinery owner,
then retire read identities after the last scope closes. The original can
conditionally promote massprops, so this is explicitly the host's current
ownership and does not infer that its native scope equals physhape. The route
has 13 / 18 / 9 / 13 unique machinery mass paths; predict those names can be
released after the corresponding map without changing body values or adding
per-frame work. Measure actual history removals, same-view output fingerprints
and transition-only cost; do not present discarded path strings as a reduction
of the native PHYSICS tag.

Installed common manifests list seven physSystem and twelve ragdoll names.
They cover characters, animals and the music box; the speaker cable sources
actually requested on the four-map route are not in those lists. The current
preparation-scoped sharing therefore does not demonstrate a missed reusable
common cable identity. A future session cache for those nineteen names should
be admitted lazily only after an active consumer and measured across maps;
preloading them or claiming a cross-map saving from this route would be
unsupported. Original per-resource registry behavior is still the baseline.

FX acquisition inventory prediction: contact declarations have a shared
common text table, while world and screen FX still resolve declarations through
`Catalog::read`. The original resource registry can return the same FX identity
for repeated requests. Count path, length and source CRC in the real four-map
client, grouped by load, before introducing another shared cache. Duplicate
input reads alone predict acquisition work; they do not prove equal parsed
semantics, current residency savings or a gameplay-frame CPU gain. Mutable
overlays must preserve their existing live read behavior.

Image history table-capacity correction: the measured candidate shrank the
hash table at every map change, but the next map immediately adds thousands of
image identities again. Retain the table's bounded peak capacity for reuse and
still free removed path strings. This deliberately forgoes the measured slot
reduction to avoid repeated table growth and rehashing; do not report those
slots as freed in the final implementation. The history now represents live
identities, and no per-frame work is added.
The same capacity-reuse rule applies to shape and mass-property history:
clear stale names under their own last-owner scopes but keep the hash-table
allocation for the next level. This retains bounded bookkeeping space rather
than forcing a fresh table allocation on every transition; scope cleanup is
still separate for the two original resource classes.
At normal transition `play_started` has already released the loading request
index. Skip recomputing request hashes while pruning when that index is empty;
this avoids one pointless hash per retired name. The fallback still updates
credits if cleanup occurs while an acquisition index exists. Predict fewer
transition-thread cycles, identical removed identities and no frame-path work.

Physics-system skinning scratch sweep: `SystemSkin::current` allocates a
joint-option vector, a copy of the rest-pose vector and a completion bitmap
on each invocation before it can decide that a pose is unchanged. At most
three vector allocations per active skin call are candidates for reuse; the
actual frame-time and allocation-size impact has not been measured. Preserve
joint order and full pose precision. This is separate from shared ragdoll
ownership, whose moving-player measurement is currently running; changing the
scratch in the same comparison would conceal its effect. Measure and address
it after the mapping comparison rather than claiming an FPS gain now.

Image-history normalization correction: `Catalog::read` records lowercase,
forward-slash identities, but a successful direct `image_load_resource`
request may retain the caller's case and slashes in a live image-owner key.
An exact-key-only pruning predicate could therefore discard a live image's
credit on another map or with an overlay. At level teardown, gather normalized
aliases only for noncanonical live owner keys; preserve the allocation-free
fast path when all keys are canonical. Predict identical behavior for the
sampled canonical route and correct retention for noncanonical aliases. Count
noncanonical owners and transition-only cost; this adds no per-frame work.
Direct indexed image names can also enter `ResidentImages` or scene slots in
noncanonical form, so equivalent spellings might miss shared-owner lookups.
Inventory such resolved names on the four-map route before changing acquisition.
If they occur, canonicalize at the common image resolver without introducing a
new per-frame operation; verify descriptors, GPU identities, overlays and the
affected loading cost. An alias used only by an untested overlay remains a
possible correctness case even when this installed-data sample contains none.

Release-profile results before the capacity-reuse follow-up: the scoped shape
owner released 685 / 553 / 82 names at the first three transitions, with
75,998 / 56,119 / 8,466 bytes of String capacity, while the matching collision
geometry signatures remained unchanged. The scoped system owner again reduced
both system and ragdoll reads 12 -> 5 with all twelve definition signatures
matching. In the 75-second moving-player comparison, the affected ragdoll
mapping loop recorded 40,010,561 cycles / 13,832 calls for the independent
vector and 37,908,848 / 13,796 for the shared slice. These are diagnostic
thread cycles for that loop, not total frame time; the sample found no CPU
regression there. Local evidence: `artifacts/image-history-perf-*.err` and
`artifacts/ragdoll-walk-*.err`.
The earlier release-profile image-history candidate used 4,469,850 /
3,988,075 / 2,492,350 thread cycles at the three transitions and removed
4,520 / 4,140 / 2,346 names. That candidate still shrank its table and hashed
every removed name against an already-empty request index. Those costs are not
claimed for the revised capacity-reusing implementation; measure it directly.

Machinery wind-source cost investigation: unlike the prior direct reader,
`ShapeSources::acquire` wraps every uncached HKX in an Arc even though the
machinery caller immediately derives faces/hulls and releases that source.
If the repeated release measurement confirms a regression, keep the single
physics-shape owner and its cached-hit behavior, but offer a scoped borrowed
consumer on cache misses so the temporary source needs no Arc allocation.
Predict identical source bytes and per-body wind geometry, lower affected
loading CPU, and no additional frame-time work. Compare the same-binary Dust
sequence before retaining the change; the original game shares shape resources
via 14008c840/1400607f0.

The repeated Dust comparison (four loads per mode, same release binary)
measured mass acquisition spans of 4,923,790 -> 5,385,335 thread cycles and
wind source/geometry spans of 872,375 -> 1,051,820 cycles; all body signatures
matched. The proposed owner thus added about 9% and 21% respectively on these
short loading spans, despite the earlier overall cache still avoiding repeated
reads. Keep the centralized owner, remove temporary Arc construction from
one-shot wind geometry, and remeasure the exact spans. Per-frame CPU is not
involved. Local evidence: `artifacts/machinery-owner-repeat-*.err`.
The mass owner call has also been marked for inlining because the earlier
caller performed the same lookup directly in its body-construction loop.
The hypothesis is that avoiding a call boundary recovers the small observed
loading overhead; the compiler may already inline it, so no gain is assumed.
Recompare the same affected Dust spans and remove the hint if it does not help.

Mass cache ownership correction to investigate next: the original direct
recipe cache moved its newly formatted path into the cache, whereas the
centralized owner accepts `&str` and allocates a second path on every miss.
The first diagnostic control also cloned the key, so it did not measure this
extra allocation against the true old implementation. Transfer the owned path
to the owner on a successful miss, preserving the complete path for missing
resource errors, and remeasure the exact loading span against a control that
also moves its key. Expected change: one fewer path allocation per unique mass
resource, with unchanged body data and no gameplay-frame work.
Because the inlining hint was not isolated from the key-allocation difference,
its effect is unknown; the final key-transfer candidate removes that hint and
compares the actual old and new ownership paths directly.

The one-shot wind change factors the common HKX read/parse into `read_source`,
which also affects area and physics preparation. Predict identical inputs and
outputs, no gameplay-frame work and no meaningful change to those existing
acquisition spans. The final same-binary Dust comparison will time those
specific source-owner calls as well as the mass and wind spans; it must use the
true old key-moving control, not the earlier cloning control.

FX acquisition inventory result: the four-map production route read
7,509 / 120 / 45 / 102 distinct FX source records, with zero repeated
path/length/CRC records within a map. The large first-map count is common
contact data; the later maps reuse it. No additional FX source cache is
justified by this route. The image resolver reported seven noncanonical names,
all the intrinsic `_defaultCube`; no noncanonical indexed image name was
observed, so the acquisition path remains unchanged. The owner-alias cleanup
still handles indexed names supplied with different case/slashes. Evidence:
`artifacts/remaining-history-shared.err`.

Declaration-history scope correction: the owner audit found, before the first
transition, 7,407 FX names and 1,296 sound-event names in the common cache,
382 map-listed sound events, and 102 FX plus 33 sound-event names outside
those tables; six physics-material names were likewise outside the common
cache. World FX, screen/pause FX, map physics and contact owners have stopped
before audio teardown, while COMMON_TEXTS is the reusable session source.
Therefore retain only actual common-cache names in these three classes at that
boundary, including unlisted map declarations, rather than clearing just the
listed contact table. Predict all common identities survive, map-only history
is retired, actual source/scene outputs are unchanged, and the cost is confined
to three bounded transition scans. Measure removals and transition cycles; do
not treat path-string capacity as native allocation or RSS.

The revised capacity-reusing release comparison removed 4,520 / 4,140 /
2,346 stale image names (416,800 / 378,152 / 215,524 String-capacity bytes)
at transitions; no live owner had a noncanonical key in that route. Cleanup
used 3,670,940 / 3,216,570 / 1,981,315 thread cycles, compared with the
previous shrinking candidate's 4,469,850 / 3,988,075 / 2,492,350 in separate
runs. These are indicative transition-only spans, not a controlled claim about
frame time or an exact allocator saving. Hash-table allocations now stay
available for later maps. The scope also preserved matching image loads and
shared-view records. Independent mass scopes retired 13 / 18 / 9 names and
the selective contact owner retired 382 / 389 / 99 names. The latter cleanup
is being extended to all three declaration classes after the audit found map
FX and other sound/material names outside the listed contact table. Evidence:
`artifacts/remaining-history-*.err`.

Read-history reinsertion prediction: after a map-owned identity is retired, a
later actual read inserts its name again instead of finding the old entry.
This is necessary for accurate progress but adds loading-only bookkeeping;
keep hash-table capacity to avoid repeated growth. Profile `loading::read`
separately by affected resource class in the paired real-level sequence,
including calls, new insertions and thread cycles. Do not use total CPU or a
stationary-frame benchmark to infer the cost, and do not claim the extra work
is free if the measurement detects it.

Image identity acquisition correction: `Catalog::contains` resolves case and
slash variants to the same normalized archive/overlay entry, while direct
`image_load_resource` returns the caller spelling. Two equivalent direct names
can therefore create two owner keys and duplicate image resources. Return the
catalog's canonical indexed/overlay key from the same lookup, preserving the
actual overlay file selection and intrinsic names. The sampled four-map route
had no noncanonical indexed image requests, so predict unchanged game-data
image identities and uploads there; equivalent case/slash aliases should now
share one identity. Reuse the existing lookup rather than adding another index
search, measure the affected resolver cost, and keep this as a resource identity
fix rather than claiming a sampled VRAM reduction.

### Particle declaration read-history scope: prediction before implementation

The installed resource manager's lookup/purge boundary is 1400607f0 /
140060180. The host already keeps decoded particledt declarations in Arc
objects shared by screen/world effects; CameraGpu particle simulations can hold
those declarations after the main-world scene begins teardown. The temporary
ParticleResources acquisition cache alone is therefore not the last owner.

Attach one shared host history lease to each successfully acquired declaration,
with the acquisition owner holding it while it works. Clear stale particledt
read names when the last such scope is dropped, including render-world users;
concurrent acquisitions keep independent scopes and cannot clear each other.
No common particle declaration is currently held by a session owner in the
rewrite, so this retires bookkeeping without introducing a resource reread.
The sampled map sequence left 89, then 166, then 169 names in history; the
prediction is that map-owned names cease accumulating after the last owner
releases. One Arc reference per acquired declaration adds setup/teardown work,
with no new per-particle or per-frame operations. Measure the affected loading
read/teardown spans and compare particle identity/output signatures; include the
extra small lease references when describing memory and do not claim RSS gains.

### Particle model read-history scope: prediction before implementation

The original's resource manager lookup/purge boundary is 1400607f0 /
140060180, and 232220 supplies PMODEL geometry. ParticleResources acquires
PMODEL sources only during level preparation, but declarations and queued
particle draws retain independent Arc<ParticleModel> references. Track the
host read-history scope through those model objects, not just their acquisition
cache or their parent particle declarations. As with declaration history,
overlapping scopes must keep the shared read identities until the last owner
releases. There is no reusable session PMODEL owner in the current rewrite.

Expected effect: transition-retired map names stop accumulating; sampled
production coverage lists 14, then 9, then 5 listed identities and additional
unlisted identities. One Arc reference per acquired model adds setup/teardown
work and a small resident pointer, with no per-frame operation. Measure the
model-class history release and affected loading::read work, and compare
particle fingerprints. Original bootstrap/EotW snapshots count 150/327
staticParticleModel identities, without naming scopes or measuring bytes; they
are not evidence that the original discards all of those models at transition.

### Static model read-history scope: prediction before implementation

Original model acquisition and purge use 14025aa00 / 140259960 and
1400607f0 / 140060180. In the rewrite, placed models and area-fog models are
acquired for a level; their CPU source caches end after preparation, while scene
meshes and fog geometry live until renderer teardown and main-world release.
There is no session owner from which the next level can reacquire those prepared
model meshes. A host level scope beginning before preparation and ending after
the render-world release acknowledgement should retire stale model read
identities without adding reads, per-frame work, or suppressing live objects.
Concurrent/prepared scopes must protect each other, including cancellation.

The sampled history grew from 417 to 535 to 539 model names during transitions,
with 37,586 to 48,198 to 48,532 bytes of String capacity. These totals include
active names and are not a prediction of exactly how many entries will drop.
Measure per-transition release and later model read insertion cost, comparing
model geometry/material/visibility fingerprints; do not claim process-RSS
savings. Original bootstrap/EotW registry counts 163/965 do not identify their
lifetimes or allocation bytes.

### Camera blur metadata reuse: prediction before implementation

The original resource lookup 1400607f0 returns an already acquired declaration
identity. CameraScene currently rebuilds BlurResources on every level even
though its six blur kernels and the 63 material variants depend only on the
installed catalog, not the map. Their runtime consumer already receives an
immutable Arc. Share one catalog-scoped Arc for the session; a different
catalog or any editable overlay takes a fresh acquisition. Per-view parameters
and GPU passes stay level-owned. This is host reuse, not a claim about the
original's exact parsed-object layout.

Prediction for four level loads: 4 -> 1 BlurResources constructions and
260 -> 65 catalog reads (63 materials and two render-program declarations),
with matching kernel/variant fingerprints. The retained metadata adds one small
session allocation between levels; measure its owned capacities and the changed
acquisition function in a same-executable comparison. No frame-time or RSS
saving is predicted, and no tests or visual inspection are needed.

### Declaration-history membership lookup: prediction before implementation

The first complete scope scan retired 536 map names / 55,498 String-capacity
bytes per Dust transition while preserving the common 9,009-text owner. Its
scan used 3,232,355 to 4,489,520 thread cycles; the older contact-only scan
was faster but left real FX and sound names stale. Most complete-scan lookups
probe the same common text map. The project already uses hashbrown's foldhash
for other immutable name maps. Using it for DeclarationTexts should reduce the
membership cost without an additional index or duplicated strings; map values,
common/level scope and paths stay unchanged.

Prediction: fewer CPU cycles in the three declaration release scans and no
regression in PhysicsSeed source lookup/acquisition; no per-frame work or
change in retained text bytes. Compare a standard-hasher control and foldhash
in the same executable, including the affected physics source lookups, and
compare contact, sound, FX and scene fingerprints. If the result does not
justify the change, retain the previous implementation and report its measured
transition cost rather than claiming a speedup.

Six-load declaration and machinery comparison: both clients completed, and
all six machinery-body, model-collision, probe, contact and material signatures
matched. The four Dust loads gave machinery mass medians 1,188,562 ->
1,337,522 cycles and wind medians 227,710 -> 217,490; one shared run rose to
2,352,385 mass and 515,865 wind cycles. The paired spike affects both measured
spans, so the source of that outlier is not established by these measurements;
a further same-binary repeat is in progress before accepting a performance
claim. Full declaration cleanup removed 536 names / 55,498 String-capacity
bytes at each repeated Dust transition, and the full scan took 3,232,355 to
4,489,520 cycles. The added reinsertions consumed cycles on the following load;
these are changed loading/bookkeeping spans, not gameplay frame timings or RSS.
Evidence: artifacts/final-mass-cost-{separate,shared}.err.

### Material read-history scope: prediction before implementation

Original material lookup/purge is 1400607f0 / 140060180. The host's scene,
world and particle material data are retired with the level after render-world
release. The prepared camera-blur metadata cache is a session owner of its
63 concrete material variants; compiled reusable pipelines alone do not
retain a material declaration identity. At the last level material scope,
retain names represented by the actual camera-blur session cache and retire
other material read identities. Editable-overlay mode has no blur session cache.
The retained names must be generated from the stored variants, not from common
manifest membership. Concurrent preparations keep independent scopes.

Prediction: the 1,507 -> 1,960 -> 1,964 accumulating names in the sampled
sequence stop accumulating, while the 63 cached camera names remain credited
if that cache is active. This is progress-history bookkeeping; it does not
change material loads, GPU resources or any frame work. The temporary name set
adds 63 short-lived paths at teardown. Measure the release and following-load
read cost and compare material-source and scene signatures. Original registry
counts 1,174 at bootstrap and 2,993 in EotW do not identify lifetimes or bytes.

### Common declaration history sharing: prediction before implementation

Original 1400607f0 precredits an acquired identity and 140060180 releases its
scope. The host already retains one common DeclarationTexts map containing
7,407 FX, 1,296 sound-event and 306 physics-material source texts, but loading
history separately owns the same 9,009 path strings. Every transition then
scans those names and hashes them back into the common table to find a few
hundred map-only entries. This duplicates live keys and caused the measured
3.23-4.49 million-cycle Dust transition scans above.

Expose the already-owned immutable common map to loading history by Arc; after
it is published, remove its duplicate history keys once. Check that shared map
when a resource is precredited, read or reported, while keeping map-only names
in the existing history sets. Dropping a level can then retire just those map
sets, and the common map remains owned for precisely the cache lifetime.
Handle the no-common/overlay path and catalog changes without keeping stale
credits; preserve required counts, late-read detection and coverage reports.

Prediction: the duplicate 9,009 common path strings and their excess history
hash-table capacity retire once per session; transition scans process only
map-owned entries rather than all common entries. Actual String capacity,
index capacity, scoped teardown cycles, per-kind read and manifest precredit
costs must be measured. Ordered progress/coverage and source/scene signatures
must match a same-executable control. A walking trace must check any affected
late-read path if it is exercised. No GPU work or player-visible difference is
predicted. If accounting or acquisition cost regresses, find and correct the
cause before retaining the change.

### Machinery mass cache miss: prediction before implementation

The centralized MassSources owner currently looks up a path and then hashes it
again when inserting a newly parsed value. The former local cache did the same,
but moving ownership measured a 12.5% higher median in the affected short Dust
loading span, with one much larger outlier; the repeat is still pending. Use a
single occupied/vacant entry lookup for the shared cache, moving the same owned
path into a miss and preserving missing/read/parse errors and the overlay path.

Prediction: each successful cache miss hashes its path once rather than twice;
hits, overlay reads, outputs, scope and gameplay updates stay unchanged. The
sampled four-map route had 53 reads and 234 repeated requests. Measure the mass
acquisition span against the real former key-moving control and compare body
fingerprints; reject or investigate if it does not improve the targeted span.

Six-load follow-up: the original key-moving machinery control and shared owner
matched all six body fingerprints. Dust medians in this run were 1,751,504 ->
1,061,929 mass cycles and 282,713 -> 207,007 wind cycles; pooled with the prior
four-load repeat, medians were 1,273,208 -> 1,087,973 and 255,378 -> 209,405.
The separate client was also slower on unrelated image and shape acquisition
spans in the follow-up, so these changes are not evidence of an equivalent
steady-state speedup. The earlier isolated shared-run spike likewise cannot be
attributed to a specific code path from these totals. No frame-time claim is
made; the repeated sources and body output remain identical.

Each repeated Dust transition retired 429 model names / 38,362 String-capacity
bytes, 103 particle names / 10,586 bytes, and 12 PMODEL names / 999 bytes. The
next Dust load then reinserted those names. For the second Dust load, the model
read-history span was 1,213,170 -> 2,180,814 cycles, including this expected
bookkeeping cost; scope releases cost 143,325 / 39,235 / 6,615 cycles in the
first transition. These are loading spans, not per-frame work. The following
common-history comparison isolates whether eliminating duplicate common names
and scans offsets that work, and includes the moving-player read path.

The session camera-blur owner constructed six -> one immutable metadata sets;
the six inputs had matching `f2bb4981` fingerprints. Its acquisition spans
totaled 21,300,056 -> 4,434,045 cycles; the retained counted lower bound was
6,536 bytes plus Arc/allocator overhead. Both modes rendered from the same
variant data; no visual inspection or test was performed. Source-path aliasing
was not observed in those six maps, so no saved texture allocations are claimed.

The later repeat did not establish a stable regression in the existing mass
owner: pooled medians were lower for the shared mode and several unrelated
acquisition spans moved in the same direction as the outlying run. The
single-entry cache proposal therefore remains unapplied; it is not claimed as
an improvement. The current owner keeps its already verified error behavior.

### Player declaration preparation: prediction before implementation

Original declaration identity lookup uses 1400607f0. During a normal rewrite
level setup, first-person arms, the player, and underwater screen effects each
call player_declarations for the same character and catalog. Each call rereads
and reparses the full inherited entity/component/mode/state tree even though
each consumer only borrows it. Reuse a single successful immutable preparation
within that setup, keyed by catalog and character; active overlays and failed
reads retain their current fresh-acquisition behavior. Once the player and arms
are prepared, keep only the FX and controller fields needed by the later screen-effect setup,
then drop that temporary data before gameplay. Other player runtime state stays
independent; no session-wide generic tree is retained.

Prediction for a normal successful setup: three full player declaration
resolutions -> one (two -> one when replay mode omits player setup), with the
same consumer values and no per-frame work. The shared tree lives longer than
one individual resolution but is compacted to those screen-effect inputs before later rendering
preparation; measure its temporary owned capacities and affected acquisition
cycles as well as actual archive reads. Compare all player/arm/screen-effect
input fingerprints in a same-executable production run. Character changes,
overlays, missing components and failed reads must keep their existing behavior;
no unsourced memory or frame-time saving is assumed.

Production comparison: for each of Corvo and Emily in Edge of the World and
Dust District, the three consumers' recorded inputs matched. Resolutions fell
from three to one per level, and reads from 357 to 119 for Corvo and 354 to 118
for Emily. The shared tree's counted capacity was 673,502 / 672,874 bytes and
fell to 42,947 / 42,491 after compaction; these are capacity counts, not process
RSS or a peak-memory comparison. Measured acquisition spans for the player and
screen consumers fell from roughly 18–22 million cycles each to 17–23 thousand
cycles each; compaction cost 1.26–1.49 million cycles. The first consumer still
performs the full acquisition. These are instrumented setup spans, not frame
timings. Editable overlays and failed acquisitions retain fresh reads.

The reordered common-history comparison recorded nine image reads during one
moving-camera interval and five in the other, so the intervals do not establish
a comparable per-read gameplay cost. The earlier moving-player interval had no
calls to that read path. The two modes recorded the same 3,717 image reads
before play in the Institute run, with 21.43 million versus 17.15 million
instrumented cycles; this single startup comparison does not establish a
steady-state speedup or regression.

### Editable render-parameter declarations: prediction before correction

The session shader metadata currently keys its shared declaration snapshot by
catalog identity. A loose render-parameter declaration can change while that
catalog remains open, so a later program library can receive stale parameter
types or option defaults. Limit reuse when an active overlay contains a
`generated/decls/renderparm/` declaration; unrelated overlays must retain the
existing shared path. Expect the normal no-overlay acquisition and gameplay
paths to be unchanged. A relevant overlay intentionally repeats declaration
acquisition when a program library opens; the cost is confined to that setup,
and errors must remain visible instead of returning a previous snapshot.

### Editable animation clips: prediction before correction

The animation registry currently also reuses a clip when its own loose overlay
can change during the catalog's lifetime. Bypass identity reuse for that exact
overlaid path, while retaining the existing shared registry for all other clips.
Expect an inexpensive empty-overlay check on clip acquisition, no per-frame
work, and a fresh read/decode only for the editable clip. Existing actors keep
their acquired data until released; a subsequent acquisition should see the
updated bytes or its read/decode error. Do not invalidate unrelated clips.
If either editable bootstrap manifest changes, the set of clips eligible for
session retention can change too; bypass the old classification in that case.

### Editable UI libraries: prediction before correction

The session Iggy file and library-bitmap registries currently key by catalog
and resource name. A loose `.iggy` file can change independently of the
catalog; its companion `.raw.iggytex` can change too. Bypass reuse only when
the relevant source or companion is editable, keeping already-open movies'
leases intact and allowing later openings to observe current data and errors.
The normal installed-archive path should retain its existing reads, sharing,
and GPU uploads; the extra checks occur on movie/library acquisition only.

The installed no-overlay production comparison loaded Edge of the World then
Dust District in both modes. All 430 recorded UI bitmap fingerprints and both
recorded collision-material fingerprints matched. The animation acquisition
span had 400 calls in each mode: medians were 33,232 versus 34,230 thread
cycles. Cached Iggy calls below 100,000 cycles had medians 10,045 versus 9,625
cycles (nine each). These spans include the temporary timing instrumentation.
The one cached shader-parameter call per run measured 16,555 versus 29,295
cycles, too few samples to establish its overhead. Cold acquisition times also
varied across owners, and bitmap signature logging occurs within bitmap spans;
no general loading or frame-time claim is made. The edited-file and concurrent
generation branches were source-reviewed; this comparison had no active
editable layers.

### Editable streamed images: investigation and prediction

Scene, fixed-view and UI image owners can currently retain a path across
levels even when an editable base image or one of its separate mip files can
change. Before changing that, trace all owners and GPU-view handoffs together.
The expected correction is to keep the existing upload-phase classification
and active view leases, while re-acquiring affected identities at the next
appropriate boundary. Unedited resources should keep their existing reuse,
uploaded bytes and frame-loop behavior. Any additional checks belong to
acquisition or level transitions, not every streaming frame. Measure actual
acquisition counts, uploads, input fingerprints and only the affected CPU
spans before keeping a change.

The no-overlay EotW / Dust / EotW production comparison matched 11,849
image-packing descriptors and payload checksums, 430 UI bitmap fingerprints,
and three collision-material fingerprints. The affected resident-image
acquisition had 1,065 calls in each mode and a 4,200-cycle median in both.
Image preparation included checksum work in the measured span, so its totals
cannot establish an uninstrumented speed difference. No editable layer was
active; the editable branches and overlapping-handle behavior were reviewed
from the ownership code rather than observed in that run. Asset-ID checks at
level cleanup also account for independently reconstructed Bevy handles; they
do not keep an asset alive.

### Combined level manifests: investigation and prediction

`acquire_level_list` currently combines init, common and map manifests by
cloning every resource descriptor, including its two owned strings. The init
and common lists remain cached independently while the combined copy is live.
The measured Edge of the World combined list contains 46,546 descriptors and
counts 6,878,036 bytes of vector and String capacity; that is not all waste,
because map-only descriptors are still needed and the count is not RSS.

Investigate keeping three borrowed/shared manifest segments in load order,
with map data released when its final consumer finishes. The predicted saving
is the duplicate init/common descriptor and string storage plus the temporary
clone work; one copy of every active map descriptor remains necessary. Before
implementing, account for indexed request matching, report traversal, overlap
between preparations and editable-manifest invalidation. Compare exact request
order and progress, retained capacities, and only the affected assembly,
precredit and read-history CPU spans. Per-entry lookup must not turn into an
unmeasured gameplay-frame regression.

The same manifest reader currently lowercases each path into a new String and
then calls `replace` even when it contains no backslashes. Copy the path once,
lowercase it in place and allocate a replacement only when a backslash exists.
Predict one fewer temporary path allocation for each forward-slash-only entry,
with unchanged ASCII normalization, error handling and request order. Measure
the actual separator mix and the affected manifest-assembly span independently
of segmented-list reuse before keeping this change.

Three production modes compared the old flat list, the flat list with the
normalization change, and the segmented list. Edge of the World / Dust / Edge
of the World had matching ordered-list fingerprints, coverage totals, contact
signatures and collision-material fingerprints. Each combined list included
26,193 init/common entries whose duplicate descriptor and String capacities
totaled 3,492,748 bytes in the old flat list. The segmented lists instead
borrow those entries from the cached source sections. The sections' vectors
have more spare capacity than the flat vector; using the recorded capacities,
the derived difference for the Edge of the World live lists is approximately
3,177,148 bytes, excluding Arc headers and allocator overhead. This describes
loading-time capacities, not process RSS or gameplay residency.

Across the three loads, the recorded manifest-assembly spans totaled
480,339,335 cycles with the normalized flat list and 412,565,405 with the
segmented list. The affected read-history instrumentation counted 62,972
reads in both modes, totaling 200,394,648 versus 200,477,467 cycles. Those
are instrumented startup spans from one run, not a frame-time result. The
diagnostic uses a Flat/Segmented variant to compare both paths in one binary;
the retained implementation has only the segmented layout.

The five source-manifest reads prepared 87,529 entries and recorded no
backslashes. The new normalization therefore avoids one temporary path
allocation per entry in this sample. Preparation spans varied between runs,
including a slower Dust sample, so no general CPU speedup is assigned to
normalization. Loose manifests still use fresh reads; overlap and nonstandard
paths retain the same handling.

### Physics-system skin sources: prediction before implementation

`physics_skinning::load_skins` currently reads and parses a skeleton and an
MD6 mesh for every simulated system, then keeps a separate copy of immutable
rest data and decoded model geometry in each `SystemSkin`. The original MD6
loader (`14025aa00`) and resource registry (`1400607f0`) acquire shared
resource identities; `14076abc0` constructs each system's own runtime bodies.
Reuse successful immutable rig and model preparations by their resource paths
within the level, with separate per-instance transforms, palettes and physics
state. Editable overlays retain fresh acquisitions and errors stay visible.

Predict one source read/decode per distinct repeated skeleton or model path,
lower retained immutable data where systems share a source, and no added
per-frame reference-count operations. Measure repeated paths, retained data
capacities, skin inputs and the affected skinning loop; no map-specific names,
RSS savings or frame-time improvement are assumed. If a source has only one
consumer, the shared representation's overhead must also be accounted for.

Scene setup independently resolves and decodes the same MD6 sources after
physics-system preparation, despite the physics owners already retaining them.
Predict that borrowing those successful model identities for scene setup will
avoid one resolve/read/decode for each distinct shared placement path and
avoid the transient second decoded model. Non-physics models and editable
overlays keep their existing acquisition path. No change to frame work is
expected; count actual reused paths and compare prepared mesh inputs.
For scene models without a physics owner, keep the decoded value directly in
the temporary cache: allocating an `Arc` for every unrelated model would add
one allocation per resource without sharing or improving runtime ownership.

The history scopes for `skeleton`, `baseModel`, and `md6Def` are predicted to
release the corresponding level-only path strings after the last overlapping
level retires. The existing EotW/Dust/EotW trace retained 79 names for each
class (7,644, 7,880, and 7,988 bytes of string capacity respectively), but that
is not a prediction of process RSS. Acquisition and release happen at level
transitions, not in the walking loop.

The existing `SystemSkin::current` also allocates pose and bookkeeping vectors
on every update, including when its resulting palette is unchanged. Its cost
and the potential savings from reusable per-instance scratch space are not yet
measured; that independent hot-loop change must be predicted and measured
before altering the pose path. Moving animated physics meshes to GPU skinning
is already tracked in the table above.

The generic animated-object path and physics skinning need the same mesh,
palette and shadow publication. Extract the existing publication without
changing its allocations, lookup order or unchanged-palette early return;
predict no change to physics per-frame work, and compare the affected pose
and publication spans rather than whole-frame CPU.

In a same-build EotW moving-player comparison, four physics instances shared
one rig and one model; all four input fingerprints matched the separate-source
mode. Rig payload fell from 6,272 to 1,568 bytes and model payload from 52,032
to 13,008 bytes, excluding allocation headers and hash-table overhead. One
scene source reused the physics model; its measured acquisition was 218,820
versus 2,905 thread cycles in this single pair. Physics acquisition totals
were 2,061,185 versus 1,174,810 cycles across four calls. Per-call pose costs
were 6,326 versus 6,028 cycles and palette costs 1,898 versus 1,568 cycles;
the diagnostic retained both input representations, and these are not
whole-frame timings. At exit each run released 58 entries in each history
class. A separate same-build EotW -> Dust -> Dust Return -> EotW comparison
matched every skin input fingerprint. Its unique rig payloads were
6,272 -> 1,568, 4,704 -> 3,136, 1,568 -> 1,568, and 6,272 -> 1,568 bytes;
model payloads were 52,032 -> 13,008, 33,744 -> 20,736, 13,008 -> 13,008,
and 52,032 -> 13,008 bytes. Scene setup reused 1, 2, 1 and 1 model paths.
The singleton case still has shared-allocation overhead; the scene source
also avoids a second decode. Each transition released its level history.
These payload counts exclude allocator overhead and are not process RSS;
editable overlays were source-reviewed but not exercised.
The publication helper itself was moved without changing its operations or
lookup order, but its targeted CPU cost has not yet been measured; the two
walking spans above cover pose and palette calculation only. Measure the
publication path alongside the next already-required animated-object build.

### Particle parameter blocks: prediction before implementation

FX particle descriptors can reference an `arkDeclParmBlock`; the existing
world FX admission rejects those effects because the original per-instance
parameter and masking path is absent. The original deserializer `14098cdd0`
uses each registered renderParm's declared value type; `140227cd0` and
`140228110` apply static and changing blocks to the individual particle model.

Acquire and share a block only when an active owner needs it, keep existing
image and shader owners for its dependencies, and release the decoded block
and GPU bindings with their last users. Predict no new resource acquisition or
frame work for FX without a block; when a block is used, repeated live
references should share one read and decode. The original masking/color-grade
path requires its own conditional GPU work; it must not add a pass when that
path is inactive. Compare only the affected acquisition, publication and pass
costs, and verify values and bindings against installed data/original code.

### Dynamic particle retirement: prediction before integration

The acquisition and render caches can both retain the same material, so each
sees another strong reference after the last effect has released it. The same
issue affects image acquisitions and GPU residency. Change acquisition entries
to weak references, and retire render bindings only when a dynamic effect's
last instance releases them. A live definition, draw, image lease or in-flight
publication must keep the corresponding resource resident; after the final
image owner retires, a later request must reread its upload payload.

Predict lower idle CPU/GPU residency after dynamic effects stop, with no
additional per-frame scan. Weak upgrades and later reacquisition can add work
to effect start and stop, so measure those affected paths and resource counts
alongside the completed interactions feature. No saving or performance result
is claimed before that comparison.

### Compact loading-history paths: prediction before implementation

The successful-read history owns each path as a `String` inside a hash table.
Those paths never mutate after insertion. Store their immutable text as
`Box<str>` instead: the UTF-8 allocation and hash/equality semantics stay the
same, while each table value has a two-word header rather than a three-word
header. Prediction on 64-bit builds: table storage falls by eight bytes per
allocated bucket; path payload stays the same size because it is inserted
from a borrowed string at exact length. Hash-table control bytes, allocator
rounding and process RSS are excluded. This does not remove paths or change
owner-driven history retirement and progress accounting.

Predict similar affected read/lookup time, with less table data touched; the
boxing conversion must not introduce a second allocation. Compare the existing
opt-in read-history thread-cycle totals, path/entry counts, per-kind table
capacity and acquisition/progress results on the same moving route and level
transitions before attributing any speed or physical-memory result.

### Scrolling-water particle program: prediction before verification

The installed `particle/scrolling_water` shader always samples two scrolling
distortion maps, opacity, normals and diffuse, then uses its authored lighting,
soft-contact, alpha-mask and fog options. Prepare those maps through the existing
particle image owner and select the program as a pipeline specialization. Predict
no extra texture reads or per-pixel branch for other particle programs once the
specialization is compiled; scrolling-water draws incur the five original samples.
Dynamic-effect lifetimes and shader inputs must be checked together when the
complete interaction path is built. These predictions are not measured results.

### Runtime world-material reuse: prediction before implementation

The worker and startup surface preparer now report image-acquisition failures
as errors before reserving mesh/material assets. Expected effect: unchanged
successful-path GPU work; failed runtime requests terminate with their source
error rather than panicking and leaving readiness pending. This is a failure
handling correction, with no CPU or memory improvement claim. Runtime proxy
publication uses the same bundle as startup and returns its actual entity ID
for the existing render-pipeline readiness owner. These changes remain unbuilt
until the complete dynamic material coordinator is connected.

Startup currently drops its material-name cache after publishing scene assets.
Runtime world-mesh and decal effects need the same material/program and image
owners. Transfer the existing cache to a level-scoped runtime owner, rather than
creating duplicate GPU materials or eagerly decoding additional declarations.
The scene already retains its published material handles; the transferred map
will additionally retain path keys, lookup-table capacity and cached metadata
until level teardown. Predict no extra GPU assets or per-frame lookup work when
no runtime effect requests one, but a measurable increase in retained CPU cache
metadata compared with dropping this map. Record its entry count, capacities and
path bytes, and measure first-use/reuse and retirement paths; do not call it a
memory saving. If the cost is material, replace the map with a compact shared
identity index before completing the lifecycle audit.

### Masking surface state: implementation prediction

The original distortion task also draws masking surfaces (`14034fe00`
registers `1403222f0` at task offset `0x540`). The masking loop preserves the
selected material's nine-word state and clears only word 5 bit `0x10`, which
is depth-write enable. The shared state helper ports this operation with no
runtime cost until the mask renderer is connected. Expected active cost is
one bit clear per prepared state; it introduces no allocations or GPU work.
The task clears its mask output even for an empty surface list, so the earlier
assumption that original execution itself must be absent when grading is
inactive was unsupported. A host omission may still be equivalent when no
consumer needs the mask, but requires establishing every consumer first.

Mask admission follow-up: `14033b350` instructions `14033ba23..14033bab8`
require masking-program availability, an ordered positive per-object bend-time
factor (140342640 with zero fallback), and stageSort+stageSortAdjust unequal
to runtime threshold 0 (boat volume). `model_masking_admitted` ports these
comparisons for the pending renderer; no per-frame work is introduced before
integration. Expected integration cost is these scalar comparisons per
already-admitted model surface, with no declaration reads or allocations.

### Particle automatic parameter updates: prediction

`render_model::update_particle_parameters` ports 140228110 using the existing
paired-slot owner and removal helper. 140226930 proves that its protected
bend-time bit means the authored pblock contains that registered parameter;
140236990 proves scalar writes splat four lanes. It removes/replaces at most
two keys, retains Vec capacity, and never grows from repeated updates after
space for those keys exists. The helper remains unconnected and unbuilt;
expected integration cost is two bounded key scans and at most two appends,
with no resource reads or extra GPU work. Do not apply it before merging the
incoming instance block or replace omitted keys with per-frame defaults.

### Mask grayscale consumer implementation

The pending mask consumer now has a complete host pass and installed-program
WGSL port in `bevy_backend::mask_grading`. It consumes caller-owned HDR/mask
views and the existing adaptation buffer, reuses unchanged bind groups and
uniforms, and owns no target textures. The prediction remains zero runtime
work until the actual effect owner activates it; an active invocation issues
one fullscreen draw. Pipeline construction must stay lazy at the effect/view
owner. No runtime owner or producer calls the pass yet, so this is unbuilt
integration work and has no verified output or timing claim. The formula uses
max(R,G,B), unsaturated mask*greyAmount, original invert semantics, and the
existing HDR range conversion functions; it preserves source alpha.

### Per-instance particle masking transport: prediction before implementation

Carry the authored bend-time scalar in the existing instance clip_fade.z
padding and extend the existing flat fade/radius varying within its location.
This adds no instance buffer bytes, allocation or texture acquisition. The
normal shader specialization keeps masking disabled, allowing its unused
factor/branch to be eliminated. The mask specialization must return after
original opacity/ClipAlpha and before lighting. Expected additional CPU work
is one declaration lookup at system construction and one scalar copy into
existing padding per published particle. This is incomplete integration;
shader output and affected upload/vertex costs require later production GPU
capture, and world FX rejection must remain until target/pass/consumer and
dynamic per-instance overrides are all connected.

Particle mask submission prediction: add an explicit record_mask mode to the
existing ParticlePass rather than a second buffer/image owner. Pipeline and
material cache keys distinguish masking from colour. Active mask work uses
the original material state with depth writes disabled, stencil reference0,
R8_UNORM output and the opacity-only shader specialization. Ordinary record
uses colour mode. No additional pass, target allocation or upload occurs until
record_mask is invoked; the caller must gate it on actual mask demand. Shader
and affected record/upload costs remain to be measured after integration.

Mask target ownership implementation: predicted active logical payload is one
R8 texel per full-resolution pixel (minimum8x8), with no mip chain or duplicate
CPU image. Allocation occurs only on first prepare/extent change, clear occurs
once per requested mask frame, and release_view drops both target and cached
input references. The pipeline and uniform buffer can remain reusable without
retaining scene textures. This is still inactive integration work; no memory
or pass timing is claimed until its actual consuming view is connected.
# Shared skeleton acquisition and runtime ownership — prediction

Before integrating the shared owner: player body/arms, arm rendering, rest-skin
preparation and physics should use one immutable Edge skeleton identity per
catalog/path, coalescing overlapping reads and parses. A repeated acquisition
while that identity is live should perform no skeleton read/parse. Mutable
poses and derived physics matrices remain independent. The body-camera pose
should stop copying its parent array each frame, saving one allocation and
`parent_count * size_of::<Option<usize>>()` copied bytes per body pose; compare
only that block over a moving route with the existing diagnostic switch.
The cost is retaining names and inverse-bind data with the skeleton until its
last consumer releases it. Acquisition/release identity timestamps and separate
core/name/inverse-bind logical-byte counts must demonstrate the actual peak
and retained-byte tradeoff. Do not infer an RSS or total CPU improvement from
the removed copy or from debug load duration. This is a host weak/coalesced
registry for the original `1400607f0` identity and `14024f2a0` skeleton loader;
GPU output and pose behavior must remain unchanged.

The same owner sweep found first-person arm/body render models still reading
and decoding BMD6 directly. Route them through the existing `ModelSources`
owner and retain its Arc instead of a separate `ModelInfo`. Prediction: source
and decoded-content signatures stay identical; repeated aliases inside the arm preparation scope avoid another read/decode,
while a unique arm still has one read/decode and no claimed payload saving. The normal arm mesh upload and
per-frame skinning do not change. Include this acquisition in the next
production identity/retirement comparison before claiming a measured gain.


The four-map BMD6 comparison invalidated the added global cache prediction:
separate/shared both performed 203 reads/decodes for 210 requests. Existing
setup-to-physics sharing already covered the exercised overlap. Remove that
additional registry rather than pinning extra resources to create cache hits.
Any future cross-owner cache requires an actual concurrent installed use case
and an affected-function/residency measurement; it is not a current gain.

The aggregate Rig global cache has also been removed after reviewing every
integrated caller: body/arms use distinct model paths and usable instances
already share the containing BasicRig asset. It had no established additional
overlap to optimize. The common typed loader remains; skeleton resource
sharing is measured separately because those consumers do overlap.

Initial runtime image failure prediction: preserve successful-path IO,
uploads and output, with no failure-map allocation for successful resident
images. A failed base read/upload should stop the currently published effect
generation once, drop its waiting batch and release only that generation's
source leases through existing retirement. Failure storage is sparse and
bounded by actual scene slots, clearing on dynamic slot retirement/reuse.
Finer-mip failures keep the existing resident texture. This fixes unbounded
waiting/retention on the demonstrated source error path; it is not a claimed
steady-state CPU improvement. Measure the affected coordinator/image-owner
paths and exercise the actual failure route after compilation.

Shared-skeleton release result (`a8fc323`): 17 live reuse hits remove 17 of
225 decodes across four loads; all 79 skeleton signatures and model content
signatures match. Full-representation peak logical payload falls from 60,006
to 34,265 bytes and final live identities are zero. This is not a claim about
the old subset-only representation or total RAM. Walking confirms the exact
predicted one-allocation/320-byte parent-copy removal per pose; median isolated
block cycles fall 2,905 -> 1,540 over 3,529/3,357 samples. The allocation-count
prediction matches. Aggregate read/parse timing drift is not attributed to
sharing; it contains cache/path effects. No further optimization is justified
by those aggregate cycles alone. Full source/measurement scope is recorded in
the parity ledger.
