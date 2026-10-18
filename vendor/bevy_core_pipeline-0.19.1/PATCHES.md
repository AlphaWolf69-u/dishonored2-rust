# Local patches against Bevy 0.19.1

`src/core_3d/mod.rs` selects `Depth24PlusStencil8` for the 3D depth attachment,
matching the original game's depth precision and stencil-capable format.

Validate with the client's `authored_surface_gpu_pipelines_compile` ignored test,
which creates real depth attachments, shadow views and shader pipelines. Keep
this format synchronized with project depth-copy and shadow sampling code when
upgrading Bevy. No upstream algorithms were otherwise changed in this crate.

Upstream source is licensed MIT OR Apache-2.0; licenses are included.
