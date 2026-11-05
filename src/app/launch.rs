//! Launch.
use super::{
    DECAL_SHADER_HANDLE, DecalExtension, DecalMaterial, FILLING_SHADER_HANDLE,
    INDIRECT_SHADER_HANDLE, LOCAL_LIGHT_MIN_AREA_PIXELS, LocalLightCullEnabled,
    LocalLightCullStats, MaterialPlugin, PROBE_PREPASS_SHADER_HANDLE, PROBE_SHADER_HANDLE, PathBuf,
    PreparedImageMailbox, ProbeExtension, ProbeMaterial, Render, RenderApp, RenderSystems,
    SKY_SHADER_HANDLE, SURFACE_SHADER_HANDLE, Shader, VISTA_SHADER_HANDLE, VisibilityElapsedTiming,
    VisibilitySystems, archive, auto_exposure, benchmark_frames, camera_controls, coarse_depth,
    collision_report, cull_small_local_lights, entities, env, finish_streaming_diagnostics,
    frame_profile, game_clock_runtime, game_settings, initialize_parallel_shadow_owner,
    local_shadow_updates, publish_initial_whole_view_depth,
    report_prepared_diffuse_images, screen_shadow_pass, setup, shadow_pass, shadow_sampling,
    shadow_updates, show_frame_rate, snapshot_once, stream_visible_models, stream_visible_textures,
    sync_shadow_target, update_coordinate_overlay,
};
use bevy::prelude::*;

/// Host root for the immutable archive namespace. Declared before App so its
/// world and workers release their leases before this session root retires.
struct CatalogSession(u64);
impl Drop for CatalogSession {
    fn drop(&mut self) {
        archive::Catalog::release_shared(self.0);
    }
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let light_area_cull = env::var("DH2_LIGHT_AREA_CULL").as_deref() != Ok("0");
    println!(
        "Local-light culling: {} (initial map Umbra candidates + non-Umbra fallback; fallback minimum {:.0} px; DH2_LIGHT_AREA_CULL=0 disables)",
        if light_area_cull {
            "enabled"
        } else {
            "disabled"
        },
        LOCAL_LIGHT_MIN_AREA_PIXELS,
    );
    super::load_timing::mark("start");
    // A first launch without a known installation asks for the game folder.
    let Some(base) = crate::platform::game_folder::game_base_or_ask()? else {
        crate::platform::client_diagnostics::record("No game folder chosen");
        return Ok(());
    };
    crate::platform::client_diagnostics::record_setup(&base);
    let base_for_loading = base.clone();
    let game_base = Some(base);
    let catalog = Some(archive::Catalog::shared(game_base.as_deref().unwrap())?);
    let _catalog_session = CatalogSession(catalog.as_ref().unwrap().identity());
    super::load_timing::mark("catalog");
    let requested = env::args()
        .nth(1)
        .unwrap_or_else(|| crate::assets::level::DEFAULT_LEVEL.to_owned());
    let level =
        crate::assets::level::LevelResources::resolve(catalog.as_ref().unwrap(), &requested)?;
    // The map beside the level's entities (`gameLocal.m_mapInfo` is the decl
    // naming it).
    if let Some(map) = level.entities.strip_suffix(".entities") {
        crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
    }
    println!(
        "Level source: installed resource archives; world={}, entities={}, tome={}, physics={:?}",
        level.world, level.entities, level.tome, level.physics
    );
    let mut app = App::new();
    // bevy_backend: the original ships compiled shaders; the D3D12 backend
    // keeps the bytecode it compiles here for later runs (vendored wgpu-hal).
    if env::var_os("DH2_SHADER_CACHE_DIR").is_none() {
        // SAFETY: set before the renderer and its threads start.
        unsafe {
            env::set_var(
                "DH2_SHADER_CACHE_DIR",
                crate::platform::home::home().cache_dir().join("shaders"),
            );
        }
    }
    let compute_threads = env::var("DH2_COMPUTE_THREADS")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?;
    // bevy_backend: Bevy sizes its pools from the hardware thread count: 25%
    // each for I/O and async compute by default, the rest for compute. Here
    // the I/O pool runs the serial texture reader (one thread at most) and
    // the async pool waits on the early Tome query and compiles pipelines in
    // bursts while new areas come into view. Frame work runs on the compute
    // pool, which gets every hardware thread: the main and render threads
    // mostly wait on it, and idle I/O and async threads park. On the eotw
    // camera path with 4 cores (DH2_DEBUG_TOTAL_THREADS=4, affinity 0x55),
    // compute 2 (the remainder) ran 65 fps, 3 ran 81 and 4 ran 89; on 12
    // threads, 9 and 12 measured the same.
    let mut task_pools = bevy::app::TaskPoolOptions::default();
    task_pools.io.percent = 0.1;
    task_pools.async_compute.percent = 0.15;
    // DH2_DEBUG_TOTAL_THREADS sizes the pools for a smaller CPU (measurement,
    // with the process affinity limited to that many cores).
    let total_override = env::var("DH2_DEBUG_TOTAL_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok());
    if let Some(total) = total_override {
        task_pools.min_total_threads = total;
        task_pools.max_total_threads = total;
    }
    task_pools.compute.min_threads = total_override
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |threads| threads.get()));
    if let Some(threads) = compute_threads {
        if !(1..=32).contains(&threads) {
            return Err("DH2_COMPUTE_THREADS must be between 1 and 32".into());
        }
        task_pools.compute.min_threads = threads;
        task_pools.compute.max_threads = threads;
    }
    // An explicit host benchmark size keeps pixel/LOD work independent of
    // desktop resolution and DPI. Ordinary launches retain borderless mode.
    let benchmark_resolution = if let Ok(size) = env::var("DH2_BENCHMARK_SIZE") {
        let (width, height) = size
            .split_once('x')
            .ok_or("DH2_BENCHMARK_SIZE must be WIDTHxHEIGHT")?;
        let (width, height) = (width.parse::<u32>()?, height.parse::<u32>()?);
        if width == 0 || height == 0 {
            return Err("benchmark dimensions must be positive".into());
        }
        Some(bevy::window::WindowResolution::new(width, height).with_scale_factor_override(1.0))
    } else {
        None
    };
    let presentation =
        game_settings::read_presentation_settings().map_err(std::io::Error::other)?;
    let mut display_state = None;
    let mut graphics = bevy::render::settings::WgpuSettings::default();
    #[cfg(target_os = "windows")]
    if env::var_os("WGPU_BACKEND").is_none() {
        graphics.backends = Some(bevy::render::settings::Backends::DX12);
    }
    graphics.features |= bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL
        | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8
        | bevy::render::settings::WgpuFeatures::FLOAT32_FILTERABLE;
    graphics.features |= bevy::render::settings::WgpuFeatures::TEXTURE_COMPRESSION_BC
        | bevy::render::settings::WgpuFeatures::TEXTURE_COMPRESSION_BC_SLICED_3D;
    // Direct-instanced particle surfaces pass {mesh_index, first} as immediates.
    graphics.features |= bevy::render::settings::WgpuFeatures::IMMEDIATES;
    let mut limits = graphics.limits.clone();
    limits.max_immediate_size = limits
        .max_immediate_size
        .max(crate::rendering::bevy_backend::particle_meshes::IMMEDIATE_SIZE);
    graphics.limits = limits;
    #[cfg(target_os = "windows")]
    crate::platform::host_graphics::prefer_installed_dxc(&mut graphics);
    app.add_plugins(
        DefaultPlugins
            .set(bevy::log::LogPlugin {
                custom_layer: crate::platform::client_diagnostics::log_layer,
                ..default()
            })
            .set(bevy::app::TaskPoolPlugin {
                task_pool_options: task_pools,
            })
            // Host diagnostic: benchmarks and DH2_DEBUG_MUTE=1 play silently;
            // DH2_DEBUG_BENCHMARK_AUDIO=1 keeps benchmark audio for level captures.
            .set(bevy::audio::AudioPlugin {
                global_volume: bevy::audio::GlobalVolume::new(
                    if (benchmark_resolution.is_some() || env::var("DH2_BENCHMARK_SECONDS").is_ok())
                        && env::var("DH2_DEBUG_BENCHMARK_AUDIO").as_deref() != Ok("1")
                        || env::var("DH2_DEBUG_MUTE").as_deref() == Ok("1")
                    {
                        bevy::audio::Volume::SILENT
                    } else {
                        bevy::audio::Volume::Linear(1.0)
                    },
                ),
                ..default()
            })
            .set(bevy::window::WindowPlugin {
                primary_window: Some({
                    // The cvars choose the window (display_settings).
                    let (window, state) =
                        super::display_settings::initial_window(bevy::window::Window {
                            // Match the installed game's measured r_fullscreen=2 borderless mode.
                            // DH2_BORDERLESS=0 remains available for windowed debugging.
                            resolution: benchmark_resolution.clone().unwrap_or_default(),
                            decorations: benchmark_resolution.is_none(),
                            resizable: benchmark_resolution.is_none(),
                            mode: if benchmark_resolution.is_some() {
                                bevy::window::WindowMode::Windowed
                            } else if env::var("DH2_BORDERLESS").as_deref() != Ok("0") {
                                bevy::window::WindowMode::BorderlessFullscreen(
                                    bevy::window::MonitorSelection::Primary,
                                )
                            } else {
                                bevy::window::WindowMode::Windowed
                            },
                            desired_maximum_frame_latency: env::var("DH2_FRAME_LATENCY")
                                .ok()
                                .and_then(|s| s.parse().ok())
                                .or(Some(presentation.frame_latency())),
                            // The uncapped game uses composited DISCARD presentation.
                            // The DX12 host bridge implements that path for Mailbox.
                            present_mode: if env::var("DH2_NO_VSYNC").as_deref() != Ok("0") {
                                if cfg!(target_os = "windows")
                                    && env::var("DH2_DEBUG_DIRECT_PRESENT").as_deref() != Ok("1")
                                {
                                    bevy::window::PresentMode::Mailbox
                                } else {
                                    bevy::window::PresentMode::AutoNoVsync
                                }
                            } else {
                                bevy::window::Window::default().present_mode
                            },
                            ..default()
                        });
                    display_state = Some(state);
                    window
                }),
                ..default()
            })
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    graphics,
                )),
                ..default()
            }),
    );
    if compute_threads.is_some() {
        println!(
            "Host compute workers: {}",
            bevy::tasks::ComputeTaskPool::get().thread_num()
        );
    }
    // LogTracer initializes `log` at TRACE even when tracing rejects TRACE.
    // wgpu constructs owned resource labels before that later rejection.
    // Match the subscriber's actual upper bound before formatting log args;
    // explicitly enabled debug/trace output remains available.
    log::set_max_level(
        bevy::log::tracing_subscriber::filter::LevelFilter::current()
            .to_string()
            .parse()
            .unwrap_or(log::LevelFilter::Trace),
    );
    if env::var("DH2_CPU_PROFILE").as_deref() == Ok("1") {
        println!("CPU_PROFILE log_max_level={}", log::max_level());
    }
    if env::var("DH2_RENDER_PROFILE").as_deref() == Ok("1") {
        println!(
            "Bevy render profiling enabled (GPU timestamps and pipeline stats require Vulkan or DX12)"
        );
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
    }
    if env::var("DH2_RENDER_PROFILE").as_deref() == Ok("1")
        || env::var("DH2_CPU_PROFILE").as_deref() == Ok("1")
    {
        frame_profile::install(&mut app);
    }
    let parallel_finalization = env::var("DH2_PARALLEL_COMMAND_FINALIZATION")
        .map_or(cfg!(target_os = "windows"), |value| value != "0");
    if let Some(chunk_size) = env::var("DH2_CAMERA_DRAW_CHUNK_SIZE")
        .ok()
        .or_else(|| {
            // Each chunk records and finishes on its own thread. At the eotw
            // view (release, PresentMon, 3-5 runs each) 4096 left the main
            // opaque encoder's 1.32 ms finish as the frame's last task:
            // p50 11.29 ms; 1024 10.89, 512 10.86, 256 10.94, 128 11.14.
            (cfg!(target_os = "windows") && parallel_finalization).then(|| "512".to_owned())
        })
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|size| *size > 0 && *size <= 16384)
    {
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.insert_resource(bevy::render::render_phase::CameraDrawChunking::new(
                chunk_size,
            ));
        }
    }
    let shared_depth = env::var("DH2_SHARE_DEPTH_PIPELINES").as_deref() != Ok("0");
    let material_free_depth = env::var("DH2_MATERIAL_FREE_DEPTH").as_deref() != Ok("0");
    if shared_depth {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.insert_resource(super::materials::depth_prepass_material_keys(
                material_free_depth,
            ));
        }
    }
    if parallel_finalization {
        if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render.insert_resource(bevy::render::renderer::ParallelCommandFinalization);
        }
    }
    // Encoders finish on Bevy's compute pool; DH2_RENDER_FINALIZATION_THREADS
    // (measurement only) gives them a private pool instead. Private pools
    // over-subscribe the hardware once their work overlaps with the frame's.
    if let Some(threads) = env::var("DH2_RENDER_FINALIZATION_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|threads| *threads > 0 && *threads <= 32)
    {
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(bevy::render::RenderStartup,
                (move |mut pending: ResMut<bevy::render::renderer::PendingCommandBuffers>| {
                    pending.set_finalization_threads(threads);
                }).after(bevy::render::init_gpu_resource::<bevy::render::renderer::PendingCommandBuffers>));
        }
    }
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<bevy::render::storage::ShaderBufferUploadBatch>();
    }
    app.init_resource::<super::interaction_focus::DiscoveredDocuments>();
    app.add_plugins(crate::rendering::bevy_backend::flash_ui::FlashUiPlugin)
        .add_plugins(super::display_settings::DisplaySettingsPlugin(
            std::sync::Mutex::new(display_state.take()),
        ));
    // Host asset owners must collect dropped assets and process dependencies
    // even while map systems are suspended by the loading screen.
    app.add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default())
        .add_plugins(super::audio::AudioAssetsPlugin)
        .add_plugins(crate::rendering::bevy_backend::sky_mask::SkyAssetsPlugin);
    let loading_schedules = super::loading_screen::Schedules::capture(&mut app);
    app.add_plugins(auto_exposure::ExposurePlugin)
        .add_plugins(super::audio::SoundPlugin)
        .add_plugins(crate::rendering::bevy_backend::display_scene::DisplayPlugin)
        .add_plugins(super::alpha_depth_pass::AlphaDepthPassPlugin)
        .add_plugins(game_clock_runtime::ClockRuntimePlugin)
        .add_plugins(shadow_pass::ShadowScenePlugin)
        .add_plugins(screen_shadow_pass::ScreenShadowScenePlugin)
        .add_plugins(coarse_depth::CoarseDepthPlugin)
        .add_plugins(crate::rendering::bevy_backend::fog::FogPlugin)
        .add_plugins(crate::rendering::bevy_backend::water_scene::WaterPlugin)
        .init_resource::<super::scene_queue::VisibleSceneInstances>()
        .init_resource::<super::visibility_queries::EarlyTomeQuery>()
        .init_resource::<super::world_instances::WorldMatrices>()
        .init_resource::<shadow_updates::ParallelShadowCpu>()
        .init_resource::<local_shadow_updates::LocalShadowCpu>()
        .init_resource::<local_shadow_updates::LocalShadowViewControls>()
        .init_resource::<local_shadow_updates::LocalShadowDraws>()
        .init_resource::<shadow_updates::ParallelWorldDraws>()
        .init_resource::<shadow_updates::ModelCasterFrame>()
        .init_resource::<shadow_updates::ParallelModelDraws>()
        .add_plugins(super::decal_pass::DecalPassPlugin)
        .add_plugins(super::projected_decals::ProjectedDecalPlugin)
        .insert_resource(VisibilityElapsedTiming::default())
        .init_resource::<super::model_motion::ModelPoseUpdates>()
        .init_resource::<super::model_motion::ModelMotionBindings>()
        .insert_resource(LocalLightCullEnabled(light_area_cull))
        .insert_resource(LocalLightCullStats::default())
        .add_systems(
            Startup,
            (
                shadow_updates::initialize_screen_shadow_source,
                setup,
                super::vegetation_wind::initialize_vegetation_wind,
                initialize_parallel_shadow_owner,
                shadow_updates::initialize_parallel_receiver_source,
                local_shadow_updates::initialize_local_shadow_owner,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                super::player::sample_player_input,
                camera_controls.run_if(super::pause_menu::game_not_paused),
                super::interaction_focus::apply_inventory_modifiers,
                super::player::update_player,
                super::player::publish_drowning_fx,
                super::player_arms::update_player_arms,
                super::fog_scene::update_world_wind,
                super::vegetation_wind::advance_trigger_winds,
                super::machinery::update_machinery,
                super::machinery::pose_physics_systems,
                update_coordinate_overlay,
                snapshot_once.run_if(super::loading_screen::is_playing),
                benchmark_frames.run_if(super::loading_screen::is_playing),
                show_frame_rate,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            super::model_motion::publish_model_poses
                .before(bevy::transform::TransformSystems::Propagate)
                .before(stream_visible_models)
                .before(shadow_updates::collect_model_casters),
        )
        .add_systems(PreUpdate, super::interaction_focus::initialize_inventory)
        .add_systems(PostUpdate, super::interactions::sync_boxes
            .after(super::model_motion::publish_model_poses))
        .add_systems(PostUpdate, super::interactions::focus_and_interact
            .after(super::interactions::sync_boxes)
            .run_if(super::pause_menu::game_not_paused)
            .run_if(super::loading_screen::is_playing))
        .add_systems(PostUpdate, super::interactions::retire_admitted_pickups
            .after(super::interactions::focus_and_interact)
            .before(stream_visible_models)
            .before(shadow_updates::collect_model_casters))
        .add_systems(PostUpdate, super::usable_animation::advance
            .after(super::interactions::focus_and_interact)
            .before(stream_visible_models)
            .before(shadow_updates::collect_model_casters)
            .run_if(super::loading_screen::is_playing))
        .add_systems(PostUpdate, (super::usable_effects::prepare_requests,
            super::runtime_fx_images::synchronize,
            super::runtime_materials::coordinate.run_if(super::runtime_materials::needs_update),
            super::projected_decals::publish_runtime,
            super::usable_effects::publish).chain()
            .after(super::usable_animation::advance)
            .run_if(super::loading_screen::is_playing))
        .add_systems(
            PostUpdate,
            publish_initial_whole_view_depth
                .after(game_clock_runtime::ClockSystems::PublishView)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            super::visibility_queries::begin_tome_query
                .before(bevy::transform::TransformSystems::Propagate)
                .before(stream_visible_models),
        )
        .add_systems(
            PostUpdate,
            stream_visible_models
                .after(VisibilitySystems::UpdateFrusta)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            cull_small_local_lights
                .after(stream_visible_models)
                .after(VisibilitySystems::UpdateFrusta)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_parallel_shadow_cpu
                .after(stream_visible_models)
                .after(shadow_updates::collect_model_casters)
                .after(game_clock_runtime::ClockSystems::PublishView),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::collect_model_casters.after(stream_visible_models),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_parallel_world_draws
                .after(shadow_updates::prepare_parallel_shadow_cpu)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_parallel_model_draws
                .after(shadow_updates::prepare_parallel_shadow_cpu),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::publish_parallel_shadow_frame
                .after(shadow_updates::prepare_parallel_world_draws)
                .after(shadow_updates::prepare_parallel_model_draws)
                .after(publish_initial_whole_view_depth),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_screen_shadow_frame
                .after(shadow_updates::publish_parallel_shadow_frame)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            local_shadow_updates::prepare_local_shadow_cpu
                .after(cull_small_local_lights)
                .after(shadow_updates::collect_model_casters)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            local_shadow_updates::publish_local_shadow_draws
                .after(local_shadow_updates::prepare_local_shadow_cpu)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            (
                finish_streaming_diagnostics,
                collision_report,
                super::load_timing::track_streaming,
                super::load_timing::level_ready,
            )
                .chain()
                .after(VisibilitySystems::CheckVisibility)
                .after(super::fog_scene::publish_fog_frame)
                .after(shadow_updates::publish_parallel_shadow_frame),
        )
        .add_systems(
            PostUpdate,
            super::fog_scene::publish_fog_frame
                .after(game_clock_runtime::ClockSystems::PublishView)
                .after(bevy::camera::CameraUpdateSystems)
                .after(bevy::transform::TransformSystems::Propagate),
        )
        // It reads the catalog setup takes for the texture worker.
        .add_systems(Startup, super::area_lights::load.before(super::setup))
        .add_systems(
            PostUpdate,
            super::fog_lights::publish
                .after(local_shadow_updates::publish_local_shadow_draws)
                .after(cull_small_local_lights),
        )
        .add_systems(
            PostUpdate,
            super::display_scene::publish_display_frame
                .after(game_clock_runtime::ClockSystems::PublishView)
                .after(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            super::vegetation_wind::update_vegetation_wind
                .after(game_clock_runtime::ClockSystems::PublishView),
        )
        .add_systems(
            PostUpdate,
            super::water_scene::publish_water_frame
                // The ocean cull consumes this frame's ordinary query descriptor.
                .after(super::visibility_queries::stream_visible_models)
                .after(game_clock_runtime::ClockSystems::PublishView)
                .after(bevy::camera::CameraUpdateSystems)
                .after(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(Update, stream_visible_textures)
        .add_systems(
            PostUpdate,
            super::particle_view::publish
                .after(super::water_scene::publish_water_frame)
                .after(super::visibility_queries::stream_visible_models),
        )
        .add_systems(
            PostUpdate,
            crate::rendering::bevy_backend::camera_scene::simulate_effects
                .after(super::particle_view::publish)
                .after(super::model_motion::publish_model_poses)
                .after(super::fog_scene::publish_fog_frame)
                .after(super::water_scene::publish_water_frame)
                .after(super::usable_effects::publish),
        )
        .insert_non_send_resource(super::pause_menu::PauseMenu::default())
        .add_systems(
            Update,
            super::pause_menu::update_pause_menu.before(camera_controls),
        )
        .add_systems(Startup, || {
            crate::loading::gate(super::pause_menu::PAUSE_GATE)
        })
        .add_systems(
            Update,
            super::pause_fx::update_pause_screen_fx
                .after(super::pause_menu::update_pause_menu)
                .run_if(resource_exists::<super::pause_fx::PauseScreenFx>),
        );
    bevy::asset::load_internal_asset!(
        app,
        PROBE_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        PROBE_PREPASS_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe_prepass.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        INDIRECT_SHADER_HANDLE,
        "../../assets/shaders/dh2_indirect.wgsl",
        crate::rendering::bevy_backend::probe_weights::indirect_shader
    );
    bevy::asset::load_internal_asset!(
        app,
        super::state::REFLECTION_SHADER_HANDLE,
        "../../assets/shaders/dh2_reflection.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        super::state::PARTICLE_SURFACE_SHADER_HANDLE,
        "../../assets/shaders/dh2_particle_surface.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SKY_SHADER_HANDLE,
        "../../assets/shaders/dh2_sky.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        FILLING_SHADER_HANDLE,
        "../../assets/shaders/dh2_filling.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        VISTA_SHADER_HANDLE,
        "../../assets/shaders/dh2_vista.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SURFACE_SHADER_HANDLE,
        "../../assets/shaders/dh2_surface.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        DECAL_SHADER_HANDLE,
        "../../assets/shaders/dh2_decal.wgsl",
        Shader::from_wgsl
    );
    shadow_sampling::register_shaders(&mut app);
    crate::rendering::bevy_backend::vegetation_wind::register_shaders(&mut app);
    app.add_systems(
        PostUpdate,
        (
            sync_shadow_target::<ProbeExtension>,
            sync_shadow_target::<DecalExtension>,
        )
            .after(bevy::asset::AssetEventSystems)
            .after(shadow_updates::publish_parallel_shadow_frame),
    );
    let prepared_images = PreparedImageMailbox::default();
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.insert_resource(prepared_images.clone());
        render_app.add_systems(
            Render,
            report_prepared_diffuse_images.after(RenderSystems::PrepareAssets),
        );
        app.insert_resource(prepared_images);
    }
    if env::var("DH2_DEBUG_RESIZE_RESIDENCY").as_deref() == Ok("1") {
        app.add_systems(PostUpdate, super::resource_diagnostics::observe_resize
            .run_if(super::loading_screen::is_playing));
    }
    super::loading_screen::install(
        &mut app,
        loading_schedules,
        catalog.unwrap(),
        level,
        base_for_loading,
    )?;
    super::load_timing::mark("app built");
    if let bevy::app::AppExit::Error(code) = app.run() {
        return Err(format!("Game runtime exited with error code {code}").into());
    }
    Ok(())
}

/// Original numeric quality -> the installed declaration's selected row.
/// Never substitutes an extracted capture or an arbitrary quality profile.
pub(super) fn load_installed_shadow_quality(
    catalog: &archive::Catalog,
    preset: u32,
) -> Result<Option<entities::ShadowQualitySettings>, String> {
    entities::shadow_quality_settings_for_preset(&read_installed_visuals(catalog)?, preset)
}

pub(super) fn read_installed_visuals(catalog: &archive::Catalog) -> Result<String, String> {
    let candidates = catalog.paths_containing("menusectionoptionvisuals");
    if candidates.len() != 1 {
        return Err(format!(
            "expected one installed visuals declaration, found {}",
            candidates.len()
        ));
    }
    let resource = &candidates[0];
    let bytes = catalog
        .read(resource)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("indexed visuals declaration vanished: {resource}"))?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

pub fn main() {
    // `--import-saves [folder]` imports saves and exits.
    let args: Vec<String> = env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--import-saves") {
        crate::platform::game_folder::import_saves(args.get(i + 1).map(PathBuf::from));
        return;
    }
    crate::platform::client_diagnostics::initialize();
    if let Err(error) = run() {
        crate::platform::client_diagnostics::record(&format!("Game client error: {error}"));
        eprintln!("Game client error: {error}");
        std::process::exit(1);
    }
    crate::platform::client_diagnostics::record("Game client exited normally");
}
