# Local patches to Bevy 0.19.1 render macros

- `AsBindGroup` sampler validation uses the image's texture-view aspect instead
  of unconditionally asking for an unspecified aspect. Combined depth/stencil
  formats return no sample type without an aspect; their `DepthOnly` views are
  valid comparison-sampler inputs.
- An owned sampler binding retains its declared filtering/nonfiltering/comparison
  type instead of always recording `Filtering`.

No texture copies, replacement depth formats, or registry-cache modifications.
The client's `authored_surface_gpu_pipelines_compile` test exercises a real
D24S8 depth-only array view and linear/Less comparison sampler.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.
