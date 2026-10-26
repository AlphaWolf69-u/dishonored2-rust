//! Launch.
use super::{
    archive, auto_exposure, benchmark_frames, camera_controls, coarse_depth, collision_report,
    cull_small_local_lights, entities, env, expanded_model_placements,
    finish_streaming_diagnostics, frame_profile, game_affine_to_view, game_clock_runtime,
    game_settings, game_to_view, initialize_parallel_shadow_owner, interact_pickup,
    local_shadow_updates, shadow_target_dimensions, navigation, physworld, placed_transform,
    placed_vertex, probe_lighting, publish_initial_whole_view_depth, render_light,
    report_prepared_diffuse_images, screen_shadow_pass, setup, shader_cache, shadow_pass,
    shadow_sampling, shadow_updates, show_frame_rate, snapshot_once, static_model_collision,
    stream_visible_models, stream_visible_textures, sync_shadow_target, tome, transformed,
    update_coordinate_overlay, view_to_game, world_mesh, BenchmarkSeconds, DecalExtension,
    DecalMaterial, DirectionalLightShadowMap, Instant, MaterialPlugin, ProgramCacheSource,
    ShadowCasterWorld, ViewSettings, NavigationWorld, LocalLightCullEnabled,
    LocalLightCullStats, PathBuf, PointLightShadowMap, PreparedImageMailbox,
    ProbeExtension, ProbeMaterial, Render, RenderApp, RenderSystems, SceneData, Shader,
    SnapshotPath, StreamingDiagnostics, TomeWorld, VisibilityElapsedTiming, VisibilitySystems,
    DECAL_SHADER_HANDLE, FILLING_SHADER_HANDLE, INDIRECT_SHADER_HANDLE,
    LOCAL_LIGHT_MIN_AREA_PIXELS, PROBE_PREPASS_SHADER_HANDLE, PROBE_SHADER_HANDLE,
    SKY_SHADER_HANDLE, SURFACE_SHADER_HANDLE, VISTA_SHADER_HANDLE,
};
use bevy::prelude::*;

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let profile = env::var("DH2_PERF_PROFILE").as_deref() == Ok("1");
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
    let startup = std::time::Instant::now();
    // A first launch without a known installation asks for the game folder.
    let Some(base) = crate::platform::game_folder::game_base_or_ask()? else {
        crate::platform::client_diagnostics::record("No game folder chosen");
        return Ok(());
    };
    crate::platform::client_diagnostics::record_setup(&base);
    let game_base = Some(base);
    let catalog = Some(archive::Catalog::open(game_base.as_deref().unwrap())?);
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
        (Some(base), Some(catalog)) => shader_cache::ProgramLibrary::open(
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
        Err(error) => eprintln!("Program cache unavailable: {error}"),
    }
    // Resolve the user's ORIGINAL quality cvar and the installed declaration
    // once. No per-frame filesystem/catalog scans and no copied capture values.
    let shadow_settings = game_settings::read_shadow_settings();
    let rewrite_shadow_settings =
        crate::platform::rewrite_settings::RewriteShadowSettings::from_environment()?;
    println!(
        "Sun shadow caster minimum: {} m (0 restores authored game ranges)",
        rewrite_shadow_settings.minimum_sun_caster_extent
    );
    let shadow_depth_bias_clamp = shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.depth_bias_clamp);
    let shadow_sort_ccw = shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.sort_shadows_ccw);
    let mut shadow_cascade_controls = shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.cascade_controls);
    let mut shadow_cascade_fars = shadow_settings
        .as_ref()
        .ok()
        .and_then(|(_, settings)| settings.cascade_fars);
    let (shadow_quality_preset, shadow_quality, shadow_target_dimensions) = match (
        shadow_settings,
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
                .map(|quality| shadow_target_dimensions(parallel_slice_size, quality))
                .transpose()
                .map_err(std::io::Error::other)?;
            println!(
                "Shadow quality from {}: preset={preset}, slice_size={parallel_slice_size}, depth_bias_clamp={depth_bias_clamp}, settings={settings:?}, target={target:?}",
                config_path.display()
            );
            (Some(preset), settings, target)
        }
        (Err(error), _) => {
            eprintln!("Shadow quality unavailable: {error}");
            (None, None, None)
        }
        _ => {
            eprintln!(
                "Shadow quality unavailable: original config setting or asset catalog missing"
            );
            (None, None, None)
        }
    };
    if let Some(quality) = &shadow_quality {
        if let (Some(controls), Some(partial)) = (
            &mut shadow_cascade_controls,
            quality.parallel_partial_dynamic_update,
        ) {
            controls.partial_dynamic_update = partial;
        }
        shadow_cascade_fars.get_or_insert(quality.cascade_fars);
    }
    let view_settings = match catalog.as_ref() {
        Some(catalog) => read_installed_visuals(catalog).and_then(|visuals| {
            game_settings::read_view_settings_with_visuals(&visuals).map(|(_, settings)| settings)
        }),
        None => game_settings::read_view_settings().map(|(_, settings)| settings),
    };
    println!("View distance settings: {view_settings:?}");
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
                    let duration = entities::environment_blend_duration(&source)
                        .map_err(std::io::Error::other)?;
                    Ok::<_, std::io::Error>((lighting, render_parameters, duration))
                })
                .transpose()?
        }
        _ => None,
    };
    let (environment_lighting, environment_render_parameters, environment_blend_duration) =
        environment_data
            .map(|(lighting, parameters, duration)| (Some(lighting), Some(parameters), duration))
            .unwrap_or((None, None, 1.0));
    let area_environments = match (entity_document.as_ref(), catalog.as_ref()) {
        (Some(document), Some(catalog)) => {
            crate::rendering::environment::load_area_environments(document, catalog)
                .map_err(std::io::Error::other)?
        }
        _ => Vec::new(),
    };
    let environment_volumes=match (entity_document.as_ref(),catalog.as_ref()) {
        (Some(document),Some(catalog))=>crate::rendering::environment::load_environment_volumes(document,catalog)
            .map_err(std::io::Error::other)?,
        _=>Vec::new(),
    };
    println!(
        "Loaded {} area environment trigger shapes",
        area_environments.len()
    );
    let probe_start = Instant::now();
    let area_fog = match (catalog.as_ref(), entity_document.as_ref()) {
        (Some(catalog), Some(document)) => super::fog_volumes::load(catalog,document)
            .map_err(std::io::Error::other)?,
        _ => Vec::new(),
    };
    let mut environment_probes = match (catalog.as_ref(), path.file_stem().and_then(|s| s.to_str()))
    {
        (Some(catalog), Some(_)) => {
            probe_lighting::load_for_world(catalog, &path.to_string_lossy()).map_err(std::io::Error::other)?
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
        println!(
            "Probe accumulation: {} probes, source-backed static/sun/occluded sky/per-probe ambient; sun_ground={:?}",
            probes.probes.len(),
            parameters.sun_ground_color
        );
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
            "World environment render parameters: {} layers, {value_count} raw entries; authored inputs prepared for exposure, sky, fog and water",
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
    let compute_threads = env::var("DH2_COMPUTE_THREADS")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?;
    let mut task_pools = bevy::app::TaskPoolOptions::default();
    if let Some(threads) = compute_threads {
        if !(1..=32).contains(&threads) {
            return Err("DH2_COMPUTE_THREADS must be between 1 and 32".into());
        }
        task_pools.compute.min_threads = threads;
        task_pools.compute.max_threads = threads;
    }
    let tome_path = PathBuf::from(&level.tome);
    if let Some(tome_bytes) = level.read_tome(catalog.as_ref().unwrap())? {
        let tome_started = Instant::now();
        let tome_tree = tome::SpatialTree::parse(&tome_bytes)?;
        let gate_states = tome::initial_gate_states(&tome_bytes)?;
        let query_data = tome::DirectTomeQueryData::parse(&tome_bytes, &tome_tree)?;
        let query_job_count = env::var("DH2_UMBRA_QUERY_JOBS")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::DEFAULT_QUERY_JOB_COUNT))?;
        let threading_mode = env::var("DH2_SCHEDULER_THREADING_MODE")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::DEFAULT_SCHEDULER_THREADING_MODE))?;
        let query_job_count =
            tome::world_query_job_count(query_job_count, threading_mode)?;
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
                    "Shadow Tome cache: {} original caster objects",
                    shadow_tome.object_count()
                );
                app.insert_resource(ShadowCasterWorld(shadow_tome));
            }
            Err(error) => eprintln!("Shadow Tome cache unavailable: {error}"),
        }
        let query_threads = env::var("DH2_TOME_QUERY_THREADS")
            .ok()
            .map(|value| value.parse::<usize>())
            .transpose()?
            .unwrap_or(0);
        if query_threads > 32 {
            return Err("Tome query worker count exceeds 32".into());
        }
        let query_pool = (query_threads > 0).then(|| {
            println!("Host Tome query workers: {query_threads} dedicated; {query_job_count} unchanged query jobs");
            bevy::tasks::TaskPoolBuilder::new().num_threads(query_threads)
                .thread_name("Tome Query".into()).build()
        });
        app.insert_resource(TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&tome_bytes),
            bytes: tome_bytes,
            tree: tome_tree,
            gate_states,
            query_data,
            query_job_count,
            query_pool,
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
        // FUN_1406798b0 loads each listed arkPhysicsMaterial declaration.
        let mut missing_materials = Vec::new();
        let materials: Vec<_> = physics
            .materials
            .iter()
            .map(|name| {
                let read = |path: &str| {
                    catalog
                        .as_ref()
                        .unwrap()
                        .read(path)
                        .map(|b| b.map(|b| entities::declaration_text(&b).into_owned()))
                        .map_err(|e| e.to_string())
                };
                entities::physics_material::physics_material(name, read).map_err(|error| {
                    missing_materials.push(error);
                })
            })
            .collect();
        if !missing_materials.is_empty() {
            println!("Physics materials unresolved: {missing_materials:?}");
        }
        let resolved: Vec<_> = materials.iter().flatten().cloned().collect();
        let slot: Vec<u32> = materials
            .iter()
            .scan(0u32, |next, material| {
                Some(match material {
                    Ok(_) => {
                        *next += 1;
                        *next - 1
                    }
                    Err(()) => navigation::NO_MATERIAL,
                })
            })
            .collect();
        let index_start = std::time::Instant::now();
        // FUN_140853b80 hands each world-table tag's material block flags to
        // the triangle's filter word; an unresolved declaration keeps the
        // arkCollisionSettings_t default.
        let block_flags = |index: u32| {
            materials
                .get(index as usize)
                .and_then(|m| m.as_ref().ok())
                .map_or(crate::gameplay::collision_settings::CollisionSettings::default().flags, |m| m.collision.flags)
        };
        let mut undecoded_filters = 0usize;
        let level_triangles: Vec<_> = geometry
            .triangles
            .iter()
            .map(|triangle| {
                let material = triangle
                    .material
                    .and_then(|index| slot.get(index as usize).copied())
                    .unwrap_or(navigation::NO_MATERIAL);
                let filter = match triangle.body_material {
                    Some(body_material) => {
                        let (filter, decoded) = physworld::shape_tag_filter(
                            body_material,
                            triangle.body_filter,
                            &triangle.shape_tags,
                            &physics.material_map,
                            block_flags,
                        );
                        undecoded_filters += usize::from(!decoded);
                        filter
                    }
                    None => triangle.body_filter,
                };
                (triangle.vertices.map(game_to_view), u32::MAX, material, filter)
            })
            .collect();
        if undecoded_filters > 0 {
            println!("Collision filters: {undecoded_filters} triangles keep a partly decoded shape-tag path");
        }
        let world = navigation::CollisionWorld::with_materials(
            level_triangles.into_iter().chain(
                model_collision
                    .into_iter()
                    .map(|(triangle, owner, filter)| (triangle, owner, navigation::NO_MATERIAL, filter)),
            ),
            collider_owners,
            resolved,
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
            let hit = world.first_hit_details(start, start + direction * 40.0, crate::gameplay::collision_filter::QueryFilter::ALL);
            println!(
                "NAV RAY hit={:?}",
                hit.map(|(distance, index, owner, vertices)| (
                    distance,
                    world.physics_material(index).map(|material| material.name.as_str()),
                    world.owner_name(owner),
                    vertices.map(view_to_game)
                ))
            );
            if let Some(owner) = hit.and_then(|(_, _, owner, _)| world.owner_name(owner)) {
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
            let feet = Vec3::from(coordinates.clone().try_into().unwrap_or([0.0; 3]));
            super::player::probe_player(catalog.as_ref().unwrap(), &world, feet, 0.0, false)?;
            return Ok(());
        }
        if env::var("DH2_ANIM_PROBE").as_deref() == Ok("1") {
            super::player::probe_animation(catalog.as_ref().unwrap())?;
            return Ok(());
        }
        if env::var("DH2_NAV_PROBE").as_deref() == Ok("1") {
            let start = player_start.ok_or("navigation probe needs a player start")?;
            let yaw = start.forward_xy[1].atan2(start.forward_xy[0]);
            super::player::probe_player(catalog.as_ref().unwrap(), &world, Vec3::from(start.position), yaw, true)?;
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
    let machinery=super::machinery::AmbientMachinery::load(catalog.as_ref().unwrap(),entity_document.as_ref(),&placed_models)
        .map_err(std::io::Error::other)?;
    app.insert_resource(machinery);
    app.insert_resource(super::model_motion::BoundModels::new(
        entity_document.as_ref().map(crate::entities::entity_binds).unwrap_or_default(),
    ));
    app.insert_resource(super::fog_scene::WorldWind::load(catalog.as_ref().unwrap(),entity_document.as_ref()).map_err(std::io::Error::other)?);
    if let (Some(catalog),Some(document))=(catalog.as_ref(),entity_document.as_ref()) {
        let map=level.entities.strip_prefix("maps/").unwrap_or(&level.entities)
            .strip_suffix(".entities").ok_or("physics entity resource has no map name")?;
        match super::physics_scene::PhysicsScene::load(catalog,document,map,&environment_volumes) {
            Ok(physics)=>{app.insert_resource(physics);},
            Err(error)=>eprintln!("Level physics publication pending: {error}"),
        }
    }
    app.insert_resource(super::vegetation_wind::LevelTriggerWinds::load(catalog.as_ref().unwrap(),entity_document.as_ref()));
    app.insert_resource(super::fog_scene::FogMotion::load().map_err(std::io::Error::other)?);
    let presentation = game_settings::read_presentation_settings().map_err(std::io::Error::other)?;
    let mut display_state = None;
    let mut graphics = bevy::render::settings::WgpuSettings::default();
    #[cfg(target_os = "windows")]
    if env::var_os("WGPU_BACKEND").is_none() {
        graphics.backends = Some(bevy::render::settings::Backends::DX12);
    }
    graphics.features |= bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL
        | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8
        | bevy::render::settings::WgpuFeatures::FLOAT32_FILTERABLE;
    #[cfg(target_os = "windows")]
    crate::platform::host_graphics::prefer_installed_dxc(&mut graphics);
    app.insert_resource(ProgramCacheSource(program_cache))
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
            environment_blend_duration,
            area_environments,
            area_fog,
            rewrite_shadow_settings,
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
        .insert_resource(super::state::EnvironmentVolumes(environment_volumes))
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    custom_layer: crate::platform::client_diagnostics::log_layer,
                    ..default()
                })
                .set(bevy::app::TaskPoolPlugin {
                    task_pool_options: task_pools,
                })
                .set(bevy::window::WindowPlugin {
                    primary_window: Some({
                        // The cvars choose the window (display_settings).
                        let (window, state) = super::display_settings::initial_window(bevy::window::Window {
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
            (cfg!(target_os = "windows") && parallel_finalization).then(|| "4096".to_owned())
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
    if let Some(threads) = env::var("DH2_RENDER_FINALIZATION_THREADS")
        .ok()
        .or_else(|| {
            if !cfg!(target_os = "windows") {
                return None;
            }
            let count = std::thread::available_parallelism().ok()?.get();
            // The render thread also finalizes commands in this batch.
            (count >= 4).then(|| (count / 4).clamp(1, 3).to_string())
        })
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
    app.add_plugins(auto_exposure::ExposurePlugin)
        .add_plugins(crate::rendering::bevy_backend::display_scene::DisplayPlugin)
        .add_plugins(super::alpha_depth_pass::AlphaDepthPassPlugin)
        .add_plugins(crate::rendering::bevy_backend::flash_ui::FlashUiPlugin)
        .add_plugins(super::display_settings::DisplaySettingsPlugin(std::sync::Mutex::new(display_state.take())))
        .add_plugins(game_clock_runtime::ClockRuntimePlugin)
        .add_plugins(shadow_pass::ShadowScenePlugin)
        .add_plugins(screen_shadow_pass::ScreenShadowScenePlugin)
        .add_plugins(coarse_depth::CoarseDepthPlugin)
        .add_plugins(crate::rendering::bevy_backend::fog::FogPlugin)
        .add_plugins(crate::rendering::bevy_backend::water_scene::WaterPlugin)
        .init_resource::<super::scene_queue::VisibleSceneInstances>()
        .init_resource::<super::world_instances::WorldMatrices>()
        .init_resource::<shadow_updates::ParallelShadowCpu>()
        .init_resource::<local_shadow_updates::LocalShadowCpu>()
        .init_resource::<local_shadow_updates::LocalShadowViewControls>()
        .init_resource::<local_shadow_updates::LocalShadowDraws>()
        .init_resource::<shadow_updates::ParallelWorldDraws>()
        .init_resource::<shadow_updates::ModelCasterFrame>()
        .init_resource::<shadow_updates::ParallelModelDraws>()
        .insert_resource(ViewSettings(view_settings))
        .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default())
        .add_plugins(super::decal_pass::DecalPassPlugin)
        .insert_resource(VisibilityElapsedTiming::default())
        .init_resource::<super::model_motion::ModelPoseUpdates>()
        .init_resource::<super::model_motion::ModelMotionBindings>()
        .insert_resource(LocalLightCullEnabled(light_area_cull))
        .insert_resource(LocalLightCullStats::default())
        .insert_resource(StreamingDiagnostics::new(&path))
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
                super::player::update_player,
                super::player_arms::update_player_arms,
                super::fog_scene::update_world_wind,
                super::vegetation_wind::advance_trigger_winds,
                super::machinery::update_machinery,
                super::machinery::pose_physics_systems,
                update_coordinate_overlay,
                interact_pickup.run_if(super::pause_menu::game_not_paused),
                snapshot_once,
                benchmark_frames,
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
        .add_systems(
            PostUpdate,
            publish_initial_whole_view_depth
                .after(game_clock_runtime::ClockSystems::PublishView)
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
            (finish_streaming_diagnostics, collision_report)
                .chain()
                .after(VisibilitySystems::CheckVisibility)
                .after(super::fog_scene::publish_fog_frame)
                .after(shadow_updates::publish_parallel_shadow_frame),
        )
        .add_systems(PostUpdate,super::fog_scene::publish_fog_frame
            .after(game_clock_runtime::ClockSystems::PublishView)
            .after(bevy::camera::CameraUpdateSystems)
            .after(bevy::transform::TransformSystems::Propagate))
        .add_systems(PostUpdate,super::fog_lights::publish
            .after(local_shadow_updates::publish_local_shadow_draws)
            .after(cull_small_local_lights))
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
        .add_systems(PostUpdate,super::water_scene::publish_water_frame
            .after(game_clock_runtime::ClockSystems::PublishView)
            .after(bevy::camera::CameraUpdateSystems)
            .after(bevy::transform::TransformSystems::Propagate))
        .add_systems(Update, stream_visible_textures)
        .insert_non_send_resource(super::pause_menu::PauseMenu::default())
        .add_systems(Update, super::pause_menu::update_pause_menu.before(camera_controls))
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
    bevy::asset::load_internal_asset!(app,super::state::REFLECTION_SHADER_HANDLE,"../../assets/shaders/dh2_reflection.wgsl",Shader::from_wgsl);
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

fn read_installed_visuals(catalog: &archive::Catalog) -> Result<String, String> {
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
