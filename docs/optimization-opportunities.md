# Optimization opportunities

Findings of the module sweeps in `AGENTS.md` ("Performance work") that were
not fixed while the module was open. Each entry: what, where, measured or
estimated gain, what it must keep equal to the original, and why it waits.
Remove an entry when it is implemented and measured.

Resource lifetime audit follow-ups (full owner inventory and evidence in
`resource-lifetimes.md`):

- Surface, resident UI/postprocess and particle image registries still have
  separate identity domains. Joining them can remove duplicate compiled chains
  and GPU textures for paths requested by several domains, and connect particle
  finer-mip demand to streaming. Saving is the intersection of their live path
  sets, not the sum of their allocations. Production EotW/Dust found two
  overlapping particle/scene paths (88,400 compiled payload bytes), so
  consolidation must not claim the full particle allocation as duplicate.
  UI/postprocess overlap and GPU allocation still need separate accounting. Preserve dimensional fallbacks, per-use samplers, mip demand and
  readiness gates before consolidating.
- Wwise type-2 acquisition is now format-only: production release read
  232,514 instead of 530,033,360 logical package bytes and took 189.805 instead
  of 383.678 ms. Playback still holds a whole encoded file; type-1 prefetch
  still needs a bounded continuation after its bank prefix. The original game
  initializes 2 MiB IO storage, 16 KiB granularity and 100 ms read-ahead.
  Implement that lifecycle without synchronous audio-callback IO or PCM
  expansion. Sample-rate identity was also missing from the shared Vorbis
  identification-header key and is corrected; compare formats after that fix.
- Vorbis header/setup caches now prune unused definitions after outgoing media
  release. The measured four setups were all live/common (4 -> 4), so there is
  no demonstrated eviction saving to pursue there on assumption.
- Temporary map manifests and progress indices now release after acquisition/
  coverage; init/common metadata remains shared. Production transitions retain
  one combined table at a time rather than accumulating visited-map copies;
  EotW releases 6,878,036 descriptor bytes with one remaining owner at handoff.
- Live-movie cycle collection and original weak listeners/Dictionary keys are
  implemented. Sustained menu collection exposed excessive temporary adjacency
  allocations; contiguous edges and the existing pointer-key hasher remove that
  work without changing weak-reference collection timing. Same-build release
  comparison measured 4.862 -> 3.238 ms mean over 29 pause collections per mode
  (172 objects reclaimed in both); further reduction must preserve those
  semantics. No live-heap savings were observed in loading movies, whereas
  pause movies did produce cyclic garbage.
- `bevy_backend::particle_pass` rebuilds pipelines/uniforms as well as bindings
  on surface-sampler changes. Image uploads are now reused. Reusing compatible
  pipelines could remove one creation per affected material/state on that rare
  transition; measure those creations before splitting this cache. No new
  per-frame scanning or locking should be introduced.
- Some backend caches retain the last view's bindings/area-light handles until
  destination preparation replaces them, while viewport capacity intentionally
  persists. Trace the outgoing-world boundary separately from settled memory
  before tightening release: expected gain is a lower transition peak, not a
  steady-state saving. Keep compiled pipelines and reusable viewport capacity;
  existing shadow static-cache accumulation was already fixed and measured.

- Wwise playback preparation repeatedly parses/clones parent property chains
  into event graphs (`assets::wwise::playback::media_properties`,
  `app::audio::graph::Definition::decode`). The load attribution includes
  49 MiB in bank lookup and 25 MiB in graph construction; only an unmeasured
  fraction is these properties. Sharing immutable chains could save tens of
  MiB and startup parsing, but needs per-owner accounting before changing
  graph-local node indexing and inherited property evaluation. Encoded media
  duplication is addressed separately; do not count logical source lengths
  as physical allocations when the sources share bank DATA.
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
