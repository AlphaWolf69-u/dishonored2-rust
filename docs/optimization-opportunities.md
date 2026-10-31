# Optimization opportunities

Findings of the module sweeps in `AGENTS.md` ("Performance work") that were
not fixed while the module was open. Each entry: what, where, measured or
estimated gain, what it must keep equal to the original, and why it waits.
Remove an entry when it is implemented and measured.

| Opportunity | Where | Gain | Keeps | Status |
|---|---|---|---|---|
| Prepare the level's listed sound events on all cores | `app::audio::prepare_all` | load: 31 s single-threaded estimate to the measured parallel time (release) | the same prepared events and media | implemented 2026-10-28 (the original prepares them on its load thread) |
| Upload a texture's subresources from one staging buffer | `vendor/bevy_render` `RenderDevice::create_texture_with_data` | peak memory: 4.4 GB of D3D12 staging at level ready (64 KiB minimum per write) | identical texels (`texture_upload_tests`) | implemented 2026-10-31 (9f02587): staging at level ready 158 -> 17 MB |
| Build the world geometry and materials on several threads | `app::setup` world object loop | load: 55-113 s debug, 8 s release single-threaded | the same meshes, materials and instance order | recorded |
| Load the level while the loading screen plays | `app::launch`, `app::setup` | load screen instead of a frozen window for the whole load (about 116 s release) | the same load order and gates | recorded; the original also loads behind its loading screen |
