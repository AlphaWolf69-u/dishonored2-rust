Bevy Render 0.19.1, MIT OR Apache-2.0 (upstream licenses retained).

- `renderer/render_context.rs`: opt-in deferred, parallel finalization of
  independently recorded encoders. Ready buffers and encoders retain their
  original GPU submission order. An optional dedicated task pool isolates
  command finalization from simulation jobs. Default upstream submission is
  retained when the marker resource is absent.
- `renderer/mod.rs`: expose the finalization marker with the render context.
- `render_phase/chunked_pass.rs`, `render_phase/mod.rs`: ordered consecutive
  direct camera batch ranges. The first pass retains its requested clears;
  following passes load the preceding color/depth/stencil contents. Restore
  viewport per pass; prepare draw functions once; keep unbatchable items last.
  Multisampling, pass diagnostics, indirect draws and background passes retain
  their ordinary path. Clear-only empty phases do not prepare draw functions.

Validation: `src/rendering/bevy_backend/submission_tests.rs` compares complete rendered pixels
with serial rendering across light/grid changes, and checks interleaved ready
buffers/recorded encoders for both shared and dedicated finalization pools.
Run the ignored GPU tests on Vulkan and DirectX 12. Camera cache and multi-draw
experiments that measured slower were removed rather than retained as modes.

- `mesh/allocator.rs`: leave the allocator's change tick untouched when the
  extracted mesh change sets are all empty. Allocation/free staging has no
  deferred work in that case. This permits the project to retain immutable
  shadow allocation metadata across unchanged mesh epochs. The GPU fixture
  `shadow_mesh_metadata_tracks_real_gpu_reallocation_and_removal` verifies
  growth, index-format changes, unrelated allocations, same-ID replacement and
  strong-handle lifetime against fresh allocator lookups.

- `storage.rs`, `render_asset.rs`: opt-in `ShaderBufferUploadBatch` recycles
  staging chunks with wgpu's `StagingBelt` for changed shader-buffer assets.
  A default no-op `RenderAsset::finish_preparation` hook submits deferred copies
  before dependent asset systems can consume the buffers. Initial uploads,
  descriptor changes, GPU-preserving resizes and destination identities retain
  their existing behavior. Chunk size follows the first observed update batch,
  bounded by the device allocation limit; recycling waits for GPU completion.
  Without the resource, ordinary queue writes remain in use.

Validation: `storage_upload_tests.rs` reads back consecutive in-flight epochs,
resizes, GPU-owned data and removals with batching both absent and present.
The prepared-shadow cache GPU fixture also enables batching. A controlled DX12
test updating 80 buffers over 120 frames reduced native placed-resource creation
from 10,227 to 630, with identical endpoints and destination identities. This is
an allocation measurement, not a gameplay FPS result.
