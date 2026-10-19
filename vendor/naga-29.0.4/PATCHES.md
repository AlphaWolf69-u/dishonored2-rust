Naga 29.0.4, MIT OR Apache-2.0 (licenses included).

`src/back/hlsl/help.rs`: derive GetDimensions output arity from texture
dimensions, arrayness and image class. Select array layers before the optional
mip/sample count, and select the correct mip-count output for 1D textures.
The prior lowering returned mip count for sampled/depth array layer queries,
and supplied invalid output argument counts for storage arrays and 1D queries.

GPU regression: `texture_query_dimensions_layers_levels_and_samples_gpu` in
the root crate; run on Vulkan and DirectX 12. It covers 1D, sampled/depth/storage
2D arrays, cube arrays, multisample dimensions and explicit mip dimensions.

`src/back/spv/image.rs`: array-layer queries for storage and multisampled
images use `OpImageQuerySize`, without a mip operand. The prior lowering always
used `OpImageQuerySizeLod`, which is invalid for storage images. The same GPU
regression covers this case; replacing only the storage-array layer query with
its known fixture constant isolated the previous Vulkan pipeline rejection.
