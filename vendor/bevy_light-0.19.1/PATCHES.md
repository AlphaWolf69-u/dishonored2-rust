# Local patches against Bevy 0.19.1

- `src/point_light.rs`: add distance decay, falloff start and half-point axis/scale
  inputs. Defaults retain upstream behavior when custom inputs are absent.
- `src/spot_light.rs`: add distance decay and falloff-start inputs.
- `src/cluster/assign.rs`: collapse an empty perspective logarithmic depth
  interval to one slice. Single-slice factors carry finite near/far extents;
  CPU slice lookup and boundary construction handle that representation.

These are data hooks used by `bevy_pbr`'s corresponding extraction and shader
patches. The project resolves authored values and original attenuation rules;
the dependency does not identify game objects or parse game assets.

Validate original light math in the CPU comparisons and actual shader pipelines
with `authored_surface_gpu_pipelines_compile`. During upgrades, keep CPU
components, extracted lights and GPU packing synchronized.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.
