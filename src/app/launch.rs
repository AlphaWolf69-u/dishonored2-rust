//! Launch.
use super::{
    archive, auto_exposure, benchmark_frames, camera_controls, coarse_depth, collision_report,
    cull_original_small_local_lights, entities, env, expanded_model_placements,
    finish_streaming_diagnostics, frame_profile, game_affine_to_view, game_clock_runtime,
    game_settings, game_to_view, initialize_native_parallel_shadow_owner, interact_pickup,
    local_shadow_updates, native_shadow_target_dimensions, navigation, physworld, placed_transform,
    placed_vertex, probe_lighting, publish_initial_native_whole_view_depth, render_light,
    report_prepared_diffuse_images, screen_shadow_pass, setup, shader_cache, shadow_pass,
    shadow_sampling, shadow_updates, show_frame_rate, snapshot_once, static_model_collision,
    stream_visible_models, stream_visible_textures, sync_native_shadow_target, tome, transformed,
    update_coordinate_overlay, view_to_game, world_mesh, BenchmarkSeconds, DecalExtension,
    DecalMaterial, DirectionalLightShadowMap, Instant, MaterialPlugin, NativeProgramCacheSource,
    NativeShadowCasterWorld, NativeViewSettings, NavigationWorld, OriginalLocalLightCullEnabled,
    OriginalLocalLightCullStats, PathBuf, PointLightShadowMap, PreparedImageMailbox,
    ProbeExtension, ProbeMaterial, Render, RenderApp, RenderSystems, SceneData, Shader,
    SnapshotPath, StreamingDiagnostics, TomeWorld, VisibilityElapsedTiming, VisibilitySystems,
    DECAL_SHADER_HANDLE, INDIRECT_SHADER_HANDLE, ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
    PROBE_PREPASS_SHADER_HANDLE, PROBE_SHADER_HANDLE, SURFACE_SHADER_HANDLE,
};
use bevy::prelude::*;

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let profile = env::var("DH2_PERF_PROFILE").as_deref() == Ok("1");
    let original_light_area_cull = env::var("DH2_ORIGINAL_LIGHT_AREA_CULL").as_deref() != Ok("0");
    println!(
        "Original local-light culling: {} (initial map Umbra candidates + non-Umbra fallback; fallback minimum {:.0} px; DH2_ORIGINAL_LIGHT_AREA_CULL=0 disables)",
        if original_light_area_cull {
            "enabled"
        } else {
            "disabled"
        },
        ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
    );
    let startup = std::time::Instant::now();
    let game_base = Some(crate::assets::installation::game_base()?);
    let catalog = Some(archive::Catalog::open(game_base.as_deref().unwrap())?);
    let requested = env::args()
        .nth(1)
        .unwrap_or_else(|| crate::assets::level::DEFAULT_LEVEL.to_owned());
    let level =
        crate::assets::level::LevelResources::resolve(catalog.as_ref().unwrap(), &requested)?;
    let path = PathBuf::from(&level.world);
    println!(
        "Level source: installed resource archives; world={}, entities={}, tome={}, physics={:?}",
        level.world, level.entities, level.tome, level.physics
    );
    let bytes = level.read_world(catalog.as_ref().unwrap())?;
    let world =
        world_mesh::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if profile {
        println!(
            "PERF world mesh parse: {:.3}s",
            startup.elapsed().as_secs_f64()
        );
    }
    let (player_start, placed_models, pickups, entity_document) =
        match level.read_entities(catalog.as_ref().unwrap())? {
            Some(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let document =
                    entities::parse(&text).map_err(|error| format!("entity parse: {error}"))?;
                (
                    entities::player_start(&document),
                    entities::render_placements(&document),
                    entities::pickups(&document),
                    Some(document),
                )
            }
            None => (None, Vec::new(), Vec::new(), None),
        };
    if env::var("DH2_TRANSFORM_AUDIT").as_deref() == Ok("1") {
        let samples = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let mut worst_world = (0.0f32, 0usize);
        let mut mismatches_world = 0usize;
        for (index, matrix) in world.matrices.iter().enumerate() {
            let converted = game_affine_to_view(matrix).to_matrix();
            for sample in samples {
                let difference = converted
                    .transform_point3(game_to_view(sample))
                    .distance(transformed(matrix, sample));
                if difference > worst_world.0 {
                    worst_world = (difference, index);
                }
                if difference > 0.01 {
                    mismatches_world += 1;
                }
            }
        }
        let mut worst_placed = (0.0f32, String::new());
        let mut mismatches_placed = 0usize;
        for placement in &placed_models {
            let converted = placed_transform(placement).to_matrix();
            for sample in samples {
                let difference = converted
                    .transform_point3(game_to_view(sample))
                    .distance(placed_vertex(placement, sample));
                if difference > worst_placed.0 {
                    worst_placed = (difference, placement.id.clone());
                }
                if difference > 0.01 {
                    mismatches_placed += 1;
                }
            }
        }
        println!(
            "TRANSFORM AUDIT: world worst={worst_world:?}m, samples exceeding 1cm={mismatches_world}; placed worst={worst_placed:?}m, samples exceeding 1cm={mismatches_placed}"
        );
        return Ok(());
    }
    println!(
        "Viewing {} meshes and {} objects from {}",
        world.meshes.len(),
        world.objects.len(),
        path.display()
    );
    if let Some(start) = player_start {
        println!("Player start: {:?}", start.position);
    }
    if let (Some(base), Some(catalog)) = (game_base.as_ref(), catalog.as_ref()) {
        println!(
            "Opened {} indexed assets from {}",
            catalog.len(),
            base.display()
        );
    }
    // The original compiled programs live beside the archives in shaderCache,
    // not necessarily in generated/renderprogs2/*.vbin archive entries.
    let program_cache = match (game_base.as_deref(), catalog.as_ref()) {
        (Some(base), Some(catalog)) => shader_cache::NativeProgramLibrary::open(
            base,
            shader_cache::ShaderCacheBackend::Pc,
            catalog,
        ),
        _ => Err("original game base/catalog missing".to_owned()),
    };
    match &program_cache {
        Ok(library) => println!(
            "Installed native program cache: {} programs, {} stage records, {} shared names, {} parameter declarations (data streams/definitions lazy)",
            library.cache.programs.len(),
            library.cache.stages.len(),
            library.cache.names.len(),
            library.parameter_kinds.len()
        ),
        Err(error) => eprintln!("Native program cache unavailable: {error}"),
    }
    // Resolve the user's ORIGINAL quality cvar and the installed declaration
    // once. No per-frame filesystem/catalog scans and no copied capture values.
    let original_shadow_settings = game_settings::read_shadow_settings();
    let shadow_depth_bias_clamp = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.depth_bias_clamp);
    let shadow_sort_ccw = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.sort_shadows_ccw);
    let shadow_cascade_controls = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.cascade_controls);
    let shadow_cascade_fars = original_shadow_settings
        .as_ref()
        .ok()
        .and_then(|(_, settings)| settings.cascade_fars);
    let (shadow_quality_preset, shadow_quality, shadow_target_dimensions) = match (
        original_shadow_settings,
        catalog.as_ref(),
    ) {
        (
            Ok((
                config_path,
                game_settings::ShadowSettings {
                    quality_preset: Some(preset),
                    parallel_slice_size,
                    depth_bias_clamp,
                    ..
                },
            )),
            Some(catalog),
        ) => {
            let settings =
                load_installed_shadow_quality(catalog, preset).map_err(std::io::Error::other)?;
            let target = settings
                .as_ref()
                .map(|quality| native_shadow_target_dimensions(parallel_slice_size, quality))
                .transpose()
                .map_err(std::io::Error::other)?;
            println!(
                "Native shadow quality from {}: preset={preset}, slice_size={parallel_slice_size}, depth_bias_clamp={depth_bias_clamp}, settings={settings:?}, target={target:?}",
                config_path.display()
            );
            (Some(preset), settings, target)
        }
        (Err(error), _) => {
            eprintln!("Native shadow quality unavailable: {error}");
            (None, None, None)
        }
        _ => {
            eprintln!(
                "Native shadow quality unavailable: original config setting or asset catalog missing"
            );
            (None, None, None)
        }
    };
    let far_shadow_bounds = entity_document
        .as_ref()
        .map(entities::world_far_shadow_bounds)
        .transpose()
        .map_err(std::io::Error::other)?
        .flatten();
    let placed_lights = entity_document
        .as_ref()
        .map(|document| {
            entities::placed_lights_with_inheritance(document, |resource| {
                let Some(catalog) = catalog.as_ref() else {
                    return Ok(None);
                };
                let bytes = catalog.read(resource).map_err(|error| error.to_string())?;
                Ok(bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            })
        })
        .transpose()?
        .unwrap_or_default();
    let world_environment = entity_document
        .as_ref()
        .and_then(entities::world_environment);
    let environment_data = match (world_environment.as_deref(), catalog.as_ref()) {
        (Some(environment), Some(catalog)) => {
            let resource = format!("generated/decls/env/{environment}.decl");
            catalog
                .read(&resource)?
                .map(|bytes| {
                    let source = String::from_utf8_lossy(&bytes).into_owned();
                    let lighting =
                        entities::environment_lighting_with_inheritance(&source, |parent| {
                            let parent_resource = format!("generated/decls/env/{parent}.decl");
                            catalog
                                .read(&parent_resource)
                                .map(|bytes| {
                                    bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                                })
                                .map_err(|error| error.to_string())
                        })
                        .map_err(std::io::Error::other)?;
                    let render_parameters =
                        entities::environment_render_parameters_with_inheritance(
                            environment,
                            &source,
                            |parent| {
                                let parent_resource = format!("generated/decls/env/{parent}.decl");
                                catalog
                                    .read(&parent_resource)
                                    .map(|bytes| {
                                        bytes.map(|bytes| {
                                            String::from_utf8_lossy(&bytes).into_owned()
                                        })
                                    })
                                    .map_err(|error| error.to_string())
                            },
                        )
                        .map_err(std::io::Error::other)?;
                    Ok::<_, std::io::Error>((lighting, render_parameters))
                })
                .transpose()?
        }
        _ => None,
    };
    let (environment_lighting, environment_render_parameters) = environment_data
        .map(|(lighting, parameters)| (Some(lighting), Some(parameters)))
        .unwrap_or((None, None));
    let probe_start = Instant::now();
    let mut environment_probes = match (catalog.as_ref(), path.file_stem().and_then(|s| s.to_str()))
    {
        (Some(catalog), Some(stem)) => {
            probe_lighting::load_for_world(catalog, stem).map_err(std::io::Error::other)?
        }
        _ => None,
    };
    if let (Some(probes), Some(environment)) = (&mut environment_probes, environment_lighting) {
        let parameters = probe_lighting::ProbeLightingParameters {
            static_factor: environment.bounce_static.unwrap_or([1.0; 3]),
            sun_factor: environment.bounce_sun.unwrap_or([1.0; 3]),
            sun_ground_color: render_light::sun_ground_color(
                environment.sun_color.unwrap_or([0.215861; 3]),
            ),
            sky_zenith: environment
                .bounce_sky_zenith
                .unwrap_or([3.88954449, 8.88057613, 20.3924446]),
            sky_horizon: environment
                .bounce_sky_horizon
                .unwrap_or([33.8765373, 44.5004768, 50.7929688]),
            ambient: environment.indirect_ambient.unwrap_or([0.0; 3]),
            // Original r_ILProbeUpdate_ContributionFactorMultiBounces default3,
            // r_ILProbeUpdate_MultiBouncesDebugFlags default7 (all sources).
            multi_bounce: 3.0,
            multi_bounce_flags: 7,
        };
        probes.initialize_indirect_lighting(&parameters);
        println!("Native probe accumulation: {} probes, source-backed static/sun/occluded sky/per-probe ambient; sun_ground={:?}",probes.probes.len(),parameters.sun_ground_color);
    }
    if let Some(probes) = &environment_probes {
        println!(
            "Loaded authored probe lighting from {}: {} probes, {} secondary network entries, {} rejected PRB resources ({:.2}s)",
            probes.network_resource,
            probes.probes.len(),
            probes.secondary_resources.len(),
            probes.rejected_resources.len(),
            probe_start.elapsed().as_secs_f32(),
        );
    } else {
        println!("No unambiguous installed probe network matches the world stem.");
    }
    if let Some(parameters) = &environment_render_parameters {
        let value_count = parameters
            .layers
            .iter()
            .map(|layer| layer.parameters.len())
            .sum::<usize>();
        println!(
            "World environment render parameters retained: {} layers, {value_count} raw entries (not yet consumed by fog/water/sky/postfx rendering)",
            parameters.layers.len(),
        );
    }
    if let Some(environment) = environment_lighting {
        println!(
            "World environment lighting: ambient high={}, low={}, sun color={}, auto-exposure EV={:?}, luminance min/max={:?}/{:?}, adapt up/down={:?}/{:?}",
            environment.ambient_high.is_some() || environment.ambient_high_scalar.is_some(),
            environment.ambient_low.is_some() || environment.ambient_low_scalar.is_some(),
            environment.sun_color.is_some(),
            environment.auto_exposure_ev,
            environment.auto_exposure_min_luminance,
            environment.auto_exposure_max_luminance,
            environment.auto_exposure_adapt_speed_up,
            environment.auto_exposure_adapt_speed_down,
        );
    }
    println!("Authored light entities resolved: {}", placed_lights.len());
    let mut app = App::new();
    let tome_path = PathBuf::from(&level.tome);
    if let Some(tome_bytes) = level.read_tome(catalog.as_ref().unwrap())? {
        let tome_started = Instant::now();
        let tome_tree = tome::SpatialTree::parse(&tome_bytes)?;
        let gate_states = tome::initial_gate_states(&tome_bytes)?;
        let query_data = tome::DirectTomeQueryData::parse(&tome_bytes, &tome_tree)?;
        let query_job_count = env::var("DH2_UMBRA_QUERY_JOBS")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT))?;
        let threading_mode = env::var("DH2_SCHEDULER_THREADING_MODE")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE))?;
        let query_job_count =
            tome::original_world_query_job_count(query_job_count, threading_mode)?;
        println!(
            "Loaded original Tome from {}: {} spatial nodes, {} initially enabled gates; cell-slot rows {} Г— {} bits",
            tome_path.display(),
            tome_tree.node_count(),
            u32::from_le_bytes(tome_bytes[0x68..0x6c].try_into().unwrap()),
            query_data.slot_rows.spatial_record_count(),
            query_data.slot_rows.bit_count(),
        );
        println!(
            "Tome immutable query views: {} records decoded in {:.3}s; original query jobs={query_job_count}, scheduler mode={threading_mode}",
            query_data.decoded_record_count(),
            tome_started.elapsed().as_secs_f64(),
        );
        match tome::shadow_caster::ShadowCasterTome::parse(&tome_bytes) {
            Ok(shadow_tome) => {
                println!(
                    "Native shadow Tome cache: {} original caster objects",
                    shadow_tome.object_count()
                );
                app.insert_resource(NativeShadowCasterWorld(shadow_tome));
            }
            Err(error) => eprintln!("Native shadow Tome cache unavailable: {error}"),
        }
        app.insert_resource(TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&tome_bytes),
            bytes: tome_bytes,
            tree: tome_tree,
            gate_states,
            query_data,
            query_job_count,
        });
    } else {
        println!(
            "No installed Tome at {}; F8 Tome diagnostics unavailable",
            tome_path.display()
        );
    }
    if let Some(physics_bytes) = level.read_physics(catalog.as_ref().unwrap())? {
        let physics_start = std::time::Instant::now();
        let physics = physworld::parse(&physics_bytes)?;
        let geometry = physics.collision_shapes(&physics_bytes)?;
        if profile {
            println!(
                "PERF authored physics parse and decode: {:.3}s",
                physics_start.elapsed().as_secs_f64()
            );
        }
        println!(
            "Authored collision: {} convex shapes, {} compressed meshes, {} triangles; {} compressed meshes remain undecoded",
            geometry.convex.len(),
            geometry.compressed.len(),
            geometry.triangles.len(),
            geometry.compressed_decode_failures.len()
        );
        let model_start = std::time::Instant::now();
        let (model_collision, collider_owners) = if let Some(catalog) = catalog.as_ref() {
            let bodies = entity_document
                .as_ref()
                .map(entities::rigid_body_settings)
                .unwrap_or_default();
            let (triangles, owners, matched, failed) =
                static_model_collision(catalog, &placed_models, &bodies);
            println!(
                "Placed static-model collision: {matched} matched authored root HKX bodies, {} triangles, {failed} failures/unsupported recipes",
                triangles.len()
            );
            (triangles, owners)
        } else {
            (Vec::new(), Vec::new())
        };
        if profile {
            println!(
                "PERF placed model collision decode: {:.3}s",
                model_start.elapsed().as_secs_f64()
            );
        }
        let index_start = std::time::Instant::now();
        let world = navigation::CollisionWorld::with_owned(
            geometry
                .triangles
                .iter()
                .map(|triangle| (triangle.vertices.map(game_to_view), u32::MAX))
                .chain(model_collision),
            collider_owners,
        );
        if profile {
            println!(
                "PERF triangle collision index: {:.3}s",
                index_start.elapsed().as_secs_f64()
            );
        }
        println!(
            "Navigation collision index: {} nondegenerate triangles",
            world.triangle_count()
        );
        if let Some(start) = player_start {
            println!(
                "Navigation index: {} large triangles, {} candidates within 2m of spawn",
                world.large_triangle_count(),
                world.nearby_triangle_count(game_to_view(start.position), 2.0)
            );
        }
        if let Ok(ray) = env::var("DH2_NAV_RAY") {
            let values = ray
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() != 6 {
                return Err("DH2_NAV_RAY needs x,y,z,dx,dy,dz".into());
            }
            let start = game_to_view([values[0], values[1], values[2]]);
            let direction = game_to_view([values[3], values[4], values[5]]).normalize();
            let hit = world.first_hit_details(start, start + direction * 40.0);
            println!(
                "NAV RAY hit={:?}",
                hit.map(|(distance, owner, vertices)| (
                    distance,
                    world.owner_name(owner),
                    vertices.map(view_to_game)
                ))
            );
            if let Some(owner) = hit.and_then(|(_, owner, _)| world.owner_name(owner)) {
                if let Some(catalog) = &catalog {
                    for placement in expanded_model_placements(&placed_models, catalog)
                        .iter()
                        .filter(|placement| placement.id == owner)
                    {
                        println!("NAV RAY placement={placement:?}");
                    }
                }
                if let Some(document) = &entity_document {
                    for body in entities::rigid_body_settings(document)
                        .iter()
                        .filter(|body| body.entity_id == owner)
                    {
                        println!("NAV RAY body={body:?}");
                    }
                }
            }
            return Ok(());
        }
        if let Ok(point) = env::var("DH2_NAV_POINT") {
            let coordinates = point
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            if coordinates.len() != 3 {
                return Err("DH2_NAV_POINT needs x,y,z".into());
            }
            let point = game_to_view([coordinates[0], coordinates[1], coordinates[2]]);
            println!(
                "NAV POINT {:?}: floor={:?} floor-20cm={:?} floor-40cm={:?} nearby={}",
                point,
                world.floor_at(point, 0.05, 5.0),
                world.floor_at(point - Vec3::Y * 0.2, 0.05, 5.0),
                world.floor_at(point - Vec3::Y * 0.4, 0.05, 5.0),
                world.nearby_triangle_count(point, 2.0)
            );
            let mut walker = navigation::Walker::new(point, &world);
            let probe_start = std::time::Instant::now();
            for _ in 0..180 {
                walker.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, false);
            }
            if profile {
                println!(
                    "PERF 180 walker steps: {:.3}s",
                    probe_start.elapsed().as_secs_f64()
                );
            }
            println!(
                "NAV POINT after 3s: feet={:?}, drop={:.3}m, supported={}",
                walker.feet,
                point.y - walker.feet.y,
                walker.supported
            );
            return Ok(());
        }
        if env::var("DH2_NAV_PROBE").as_deref() == Ok("1") {
            let start = player_start.ok_or("navigation probe needs a player start")?;
            let original = game_to_view(start.position);
            let floor = world.floor_at(original, 0.5, 2.0);
            let mut walker = navigation::Walker::new(original, &world);
            for _ in 0..180 {
                walker.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, false);
            }
            println!(
                "NAV PROBE spawn feet={original:?} floor={floor:?} after 3s={:?} supported={}",
                walker.feet, walker.supported
            );
            for (label, direction) in [
                (
                    "forward",
                    game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]),
                ),
                (
                    "back",
                    -game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]),
                ),
                (
                    "right",
                    Vec3::new(start.forward_xy[1], 0.0, start.forward_xy[0]),
                ),
            ] {
                let mut walker = navigation::Walker::new(original, &world);
                for _ in 0..120 {
                    walker.step(&world, direction, 6.0, 1.0 / 60.0, false);
                }
                println!(
                    "NAV PROBE {label} after 2s: feet={:?}, displacement={:.2}m, supported={}",
                    walker.feet,
                    walker.feet.distance(original),
                    walker.supported
                );
            }
            let mut jumper = navigation::Walker::new(original, &world);
            let initial_height = jumper.feet.y;
            let mut peak = initial_height;
            for frame in 0..180 {
                jumper.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, frame == 0);
                peak = peak.max(jumper.feet.y);
            }
            println!(
                "NAV PROBE jump peak +{:.2}m, after 3s feet={:?}, supported={}",
                peak - initial_height,
                jumper.feet,
                jumper.supported
            );
            return Ok(());
        }
        app.insert_resource(NavigationWorld(world));
    } else {
        println!(
            "No authored physics resource for {}; free-fly only",
            level.world
        );
    }
    if let Some(path) = env::var_os("DH2_SNAPSHOT") {
        app.insert_resource(SnapshotPath(PathBuf::from(path)));
    }
    if let Ok(seconds) = env::var("DH2_BENCHMARK_SECONDS") {
        let total = seconds.parse::<f32>()?.max(3.0);
        let warmup = env::var("DH2_BENCHMARK_WARMUP_SECONDS")
            .unwrap_or_else(|_| "2".to_owned())
            .parse::<f32>()?;
        if !total.is_finite() || !warmup.is_finite() || warmup < 0.0 || warmup >= total {
            return Err(
                "benchmark duration must be finite and exceed its non-negative warmup".into(),
            );
        }
        app.insert_resource(BenchmarkSeconds(total, warmup));
    }
    app.insert_resource(NativeProgramCacheSource(program_cache))
        .insert_resource(ClearColor(Color::srgb(0.055, 0.045, 0.06)))
        // Bevy allocates six depth layers for every shadow-casting point
        // light. Dense missions contain hundreds of authored lights, so keep
        // the shadow maps small enough to leave room for the mission assets.
        // Authored shadow flags remain intact.
        .insert_resource(PointLightShadowMap { size: 128 })
        .insert_resource(DirectionalLightShadowMap { size: 512 })
        .insert_resource(SceneData {
            source_path: path.clone(),
            world,
            player_start,
            placed_models,
            placed_lights,
            environment_lighting,
            environment_render_parameters,
            environment_probes,
            shadow_quality_preset,
            shadow_quality,
            shadow_target_dimensions,
            shadow_depth_bias_clamp,
            shadow_sort_ccw,
            shadow_cascade_controls,
            shadow_cascade_fars,
            far_shadow_bounds,
            pickups,
            catalog,
        })
        .add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: Some(bevy::window::Window {
                        // Match the installed game's measured r_fullscreen=2 borderless mode.
                        // DH2_BORDERLESS=0 remains available for windowed debugging.
                        mode: if env::var("DH2_BORDERLESS").as_deref() != Ok("0") {
                            bevy::window::WindowMode::BorderlessFullscreen(
                                bevy::window::MonitorSelection::Primary,
                            )
                        } else {
                            bevy::window::WindowMode::Windowed
                        },
                        desired_maximum_frame_latency: env::var("DH2_FRAME_LATENCY")
                            .ok()
                            .and_then(|s| s.parse().ok()),
                        // The user's original r_swapInterval=0; retain that uncapped default.
                        present_mode: if env::var("DH2_NO_VSYNC").as_deref() != Ok("0") {
                            bevy::window::PresentMode::AutoNoVsync
                        } else {
                            bevy::window::Window::default().present_mode
                        },
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                        bevy::render::settings::WgpuSettings {
                            // Original shadow world draws disable depth clipping.
                            // The captured `decompressedDepth` input uses an
                            // R24G8 typeless depth-stencil backing (not a DXGI
                            // format number passed through unchanged).
                            features: bevy::render::settings::WgpuSettings::default().features
                                | bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL
                                | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                            ..default()
                        },
                    )),
                    ..default()
                }),
        );
    if env::var("DH2_RENDER_PROFILE").as_deref() == Ok("1") {
        println!(
            "Bevy render profiling enabled (GPU timestamps and pipeline stats require Vulkan or DX12)"
        );
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
        frame_profile::install(&mut app);
    }
    app.add_plugins(auto_exposure::NativeExposurePlugin)
        .add_plugins(game_clock_runtime::NativeClockRuntimePlugin)
        .add_plugins(shadow_pass::NativeShadowScenePlugin)
        .add_plugins(screen_shadow_pass::NativeScreenShadowScenePlugin)
        .add_plugins(coarse_depth::NativeCoarseDepthPlugin)
        .init_resource::<shadow_updates::NativeParallelShadowCpu>()
        .init_resource::<local_shadow_updates::NativeLocalShadowCpu>()
        .init_resource::<local_shadow_updates::NativeLocalShadowViewControls>()
        .init_resource::<local_shadow_updates::NativeLocalShadowDraws>()
        .init_resource::<shadow_updates::NativeParallelWorldDraws>()
        .init_resource::<shadow_updates::NativeModelCasterFrame>()
        .init_resource::<shadow_updates::NativeParallelModelDraws>()
        .insert_resource(NativeViewSettings(
            game_settings::read_view_settings().map(|(_, settings)| settings),
        ))
        .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default())
        .insert_resource(VisibilityElapsedTiming::default())
        .insert_resource(OriginalLocalLightCullEnabled(original_light_area_cull))
        .insert_resource(OriginalLocalLightCullStats::default())
        .insert_resource(StreamingDiagnostics::new(&path))
        .add_systems(
            Startup,
            (
                shadow_updates::initialize_native_screen_shadow_source,
                setup,
                initialize_native_parallel_shadow_owner,
                shadow_updates::initialize_native_parallel_receiver_source,
                local_shadow_updates::initialize_native_local_shadow_owner,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                camera_controls,
                update_coordinate_overlay,
                interact_pickup,
                snapshot_once,
                benchmark_frames,
                show_frame_rate,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            publish_initial_native_whole_view_depth
                .after(game_clock_runtime::NativeClockSystems::PublishView)
                .before(bevy::asset::AssetEventSystems),
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
            cull_original_small_local_lights
                .after(stream_visible_models)
                .after(VisibilitySystems::UpdateFrusta)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_native_parallel_shadow_cpu
                .after(stream_visible_models)
                .after(shadow_updates::collect_native_model_casters)
                .after(game_clock_runtime::NativeClockSystems::PublishView),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::collect_native_model_casters.after(stream_visible_models),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_native_parallel_world_draws
                .after(shadow_updates::prepare_native_parallel_shadow_cpu)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_native_parallel_model_draws
                .after(shadow_updates::prepare_native_parallel_shadow_cpu),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::publish_native_parallel_shadow_frame
                .after(shadow_updates::prepare_native_parallel_world_draws)
                .after(shadow_updates::prepare_native_parallel_model_draws)
                .after(publish_initial_native_whole_view_depth),
        )
        .add_systems(
            PostUpdate,
            shadow_updates::prepare_native_screen_shadow_frame
                .after(shadow_updates::publish_native_parallel_shadow_frame)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            local_shadow_updates::prepare_native_local_shadow_cpu
                .after(cull_original_small_local_lights)
                .after(shadow_updates::collect_native_model_casters)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            local_shadow_updates::publish_native_local_shadow_draws
                .after(local_shadow_updates::prepare_native_local_shadow_cpu)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            (finish_streaming_diagnostics, collision_report)
                .chain()
                .after(VisibilitySystems::CheckVisibility),
        )
        .add_systems(Update, stream_visible_textures);
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
    app.add_systems(
        PostUpdate,
        (
            sync_native_shadow_target::<ProbeExtension>,
            sync_native_shadow_target::<DecalExtension>,
        )
            .after(bevy::asset::AssetEventSystems)
            .after(shadow_updates::publish_native_parallel_shadow_frame),
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
    app.run();
    Ok(())
}

/// Original numeric quality -> the installed declaration's selected row.
/// Never substitutes an extracted capture or an arbitrary quality profile.
pub(super) fn load_installed_shadow_quality(
    catalog: &archive::Catalog,
    preset: u32,
) -> Result<Option<entities::ShadowQualitySettings>, String> {
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
    let source = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    entities::shadow_quality_settings_for_preset(source, preset)
}

pub fn main() {
    if let Err(error) = run() {
        eprintln!("Game client error: {error}");
        std::process::exit(1);
    }
}
