# Local patches against Bevy 0.19.1

Compared with the matching upstream release, these files differ:

- `src/cluster/mod.rs`: lower the uniform-buffer light limit to 170 to fit the
  extended GPU record; synchronize with `mesh_view_types.wgsl`.
- `src/render/light.rs`: extract decay/falloff/half-point inputs, pack their GPU
  representation and allow 128 authored rectangle lights.
- `src/render/mesh_view_types.wgsl`: corresponding GPU light records and limits.
- `src/render/forward_io.wgsl` and `src/render/mesh.wgsl`: authored tangent basis
  and per-vertex probe attributes/interpolants for project material extensions.
- `src/render/pbr_functions.wgsl` and `src/render/pbr_lighting.wgsl`: project
  lighting hooks and original attenuation/half-point support.
- `src/render/shadows.wgsl`: compatibility with the extended light layout.
- `src/volumetric_fog/volumetric_fog.wgsl`: corresponding light-call signature.

The project owns material interpretation, native shadow preparation and probe
lighting. These patches expose those inputs to the existing renderer. Preserve
paired CPU/WGSL layouts and upstream defaults when upgrading.

Validate original light math with CPU comparisons and
`authored_surface_gpu_pipelines_compile` for real GPU shader compilation,
material outputs and shadow sampling. Runtime testing remains necessary after
an upstream renderer upgrade.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.
