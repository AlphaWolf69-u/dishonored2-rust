# Local patches against Bevy 0.19.1

Compared with the matching upstream release, these files differ:

- `src/cluster/mod.rs`: lower the uniform-buffer light limit to 170 to fit the
  extended GPU record; synchronize with `mesh_view_types.wgsl`.
- `src/render/light.rs`: extract decay/falloff/half-point inputs, pack their GPU
  representation and allow 128 authored rectangle lights.
- `src/render/mesh_view_types.wgsl`: corresponding GPU light records and limits.
- `src/render/forward_io.wgsl` and `src/render/mesh.wgsl`: authored tangent basis
  and per-vertex probe attributes/interpolants for project material extensions.
  The mesh vertex path also transports arkDecal's uniform homogeneous Z offset
  and authored sky/cloud far-depth projection.
- `src/render/clustered_forward.wgsl` and `src/cluster/cluster_raster.wgsl`:
  match `bevy_light`'s single-depth-slice representation in forward lookup and
  GPU cluster AABB construction, avoiding undefined logarithmic factors.
- `src/render/pbr_functions.wgsl` and `src/render/pbr_lighting.wgsl`: project
  lighting hooks and original attenuation/half-point support.
- `src/render/shadows.wgsl`: compatibility with the extended light layout.
- `src/volumetric_fog/volumetric_fog.wgsl`: corresponding light-call signature.
- `src/prepass/material_keys.rs`, `src/prepass/mod.rs`, and `src/material.rs`:
  opt-in, material-type-specific key normalization for plain depth passes.
  Registered opaque paths may also omit material reads and bindings, allowing
  compatible instances with different forward materials to share a depth draw.
  Alpha-tested, normal, motion-vector and deferred passes retain their required
  material state. Unregistered materials retain upstream behavior.
- `src/meshlet/material_pipeline_prepare.rs`: supply the unchanged material-read
  behavior to the extended prepass specializer.

The project owns material interpretation, native shadow preparation and probe
lighting. These patches expose those inputs to the existing renderer. Preserve
paired CPU/WGSL layouts and upstream defaults when upgrading.

Validate original light math with CPU comparisons and
`authored_surface_gpu_pipelines_compile` for real GPU shader compilation,
material outputs and shadow sampling. Runtime testing remains necessary after
an upstream renderer upgrade.

The same GPU fixture compares every depth pixel against the unmodified key path
for opaque, coverage, alpha-tested, vegetation, layered and liquid materials.
It requires both actual key normalization and a reduction from two compatible
material batches to one, preventing a disabled optimization from passing.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.
