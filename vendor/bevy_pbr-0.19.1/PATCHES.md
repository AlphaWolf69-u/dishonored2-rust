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

Transparent fog adds an optional `DH2_FOG_VERTEX` output at location12 and
calls the rewrite-owned `dh2_material_fog::material_fog_vertex` after the final
world/clip transforms. Liquid-container materials use this route, following
their installed vertex shader; other transparent materials use pixel fog.
The feature is absent from depth keys and upstream materials. Existing probe
and authored basis varyings retain their locations. Shader/material validation
of this addition is pending the current integration checks.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.

Far-plane projection uses the optional presence flag `DH2_FAR_DEPTH` in the
mesh shader. The numeric `DH2_PROJECT_FAR` material key remains unchanged.
An upstream material with no rewrite definitions must compile: the shader
composer evaluates nested numeric conditions even inside an inactive `#ifdef`.
The atmosphere scene fixture exercises an ordinary StandardMaterial alongside
the rewrite's sky and fog. Validation of this guard is pending that GPU check.

- Transparent fog vertex calls carry the existing mesh tag to the shared per-object area-fog parameter table (binding167), without altering irradiance tags or mesh geometry. Numerical area-selection fixtures and material GPU validation cover the transport.

- arkVegetation wind: `forward_io.wgsl`/`prepass_io.wgsl` add the optional
  `DH2_VEGETATION_WIND` inputs at locations 10..12 (original local position,
  NORMAL.xy, COLOR.xyz). `mesh.wgsl` and `prepass.wgsl` replace only the world
  position with the rewrite-owned `dh2_vegetation_wind::vegetation_world_position`
  (material bindings 168/169) before clip projection, so probe lighting and fog
  see the bent position as in the installed forward VS. Normals, tangents and
  upstream materials are unchanged; a missing per-instance record keeps the
  rest position.
