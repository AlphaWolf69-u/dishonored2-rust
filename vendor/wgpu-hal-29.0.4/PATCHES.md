wgpu-hal 29.0.4, MIT OR Apache-2.0 (upstream licenses retained).

- `src/dx12/{mod,device,command}.rs`: pool completed DIRECT command lists
  together with their dedicated command allocators. Keeping lists local to
  encoders duplicates each encoder's largest list count; keeping a whole-batch
  allocator per encoder still duplicates its largest retained command storage
  even after lists are shared. Heterogeneous shadow batches exercise both cases.
- Each active native list owns one allocator. Ending a recording transfers both
  into its command buffer, which retains them through GPU submission. Only
  completed buffers or discarded recordings enter the device pool. Checkout
  transfers exclusive ownership, releases the mutex, resets the allocator, then
  resets its list. Failed resets propagate through the existing device error path.
- Discarded recordings remain local until encoder retirement. They were never
  submitted, so their exclusive pair can also be reused safely. There is no
  arbitrary pool size limit: peak concurrent native work determines its size.
  GPU submission order and rendered work are unchanged.

API contracts: [command-list Reset](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12graphicscommandlist-reset)
requires a closed list and compatible allocator;
[allocator Reset](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12commandallocator-reset)
requires completed GPU work. Allocators cannot be reset while any recording
or submitted list still uses their storage.

Validation: the ignored Windows/DX12 test
`heterogeneous_command_batches_reuse_completed_lists_gpu` rotates a 256-pass
batch among 80 live encoders, checks process memory growth and reads back all
rendered depth pixels. This catches allocator amplification that a smaller
encoder pool did not expose. Shadow and parallel submission readback fixtures
also run on DX12 and Vulkan.

A heavier local diagnostic adds 512 depth-tested draws per pass. Shared lists
with whole-encoder allocators used 2.24 GB process memory and 1.61 GB nonlocal
GPU commitment. Pairing allocators with lists reduced those figures to 1.57 GB
and 0.97 GB, with identical depth output and stable memory while batches moved.
These are controlled diagnostic measurements, not game memory budgets or FPS.

## Read-only depth/stencil attachment resource flags

`src/dx12/conv.rs` keeps depth/stencil textures that permit attachment reads
compatible with shader-resource states, including textures created without
public `TEXTURE_BINDING` usage. wgpu-core 29's render-pass setup tracks a fully
read-only depth/stencil attachment as `DEPTH_STENCIL_READ | RESOURCE`.
Previously the HAL created attachment-only textures with
`DENY_SHADER_RESOURCE`, making that valid pass fail at command-list close with
D3D12 `RESOURCE_BARRIER_MISMATCHING_MISC_FLAGS` (#524) and device loss.
Write-only HAL textures may retain the deny flag. Existing shader-readable
textures keep the same flags. Public texture-usage validation is unchanged.

The visible-sun numeric GPU regression uses an attachment-only depth texture,
clears it, then reads it for depth testing with writes disabled. It compares
both sun blend modes and foreground rejection against D3D11 render outputs.
The DX12 regression passes against all 256 reference sun pixels, alongside the
fog shader suite, 666 CPU tests and the headless gameplay-camera owner test.
No test texture usage was broadened to hide the invalid transition.

## Texture-copy alignment allocations

`src/dx12/command.rs` returns intermediate texture-copy buffers through
`DeviceAllocationContext::free_resource` on encoder reset and drop. Their
`Buffer` values do not own allocator cleanup; clearing their vector only
released COM resources and leaked each placed allocation. Preserve the existing
completion boundary: reset reclaims all command buffers from the encoder, and
no returned command buffer may outlive its encoder. No memory is freed while
its submitted copy is still using it. Internal allocations receive a useful
label and do not affect public buffer object counts.

The fog TLF GPU test copies texture data at offset 256, forcing this DX12 path.
It repeats complete view construction, three-frame submission and teardown,
checks numeric output and stable persistent buffer identities, then verifies
that settled allocator usage does not exceed the pre-view baseline.
