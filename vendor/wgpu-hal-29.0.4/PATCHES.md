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
