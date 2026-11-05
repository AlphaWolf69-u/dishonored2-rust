//! Acquire the selected map through the installed resource owners.
//! Runs behind the loading screen; publishes a complete typed resource bundle.
use super::{
    BenchmarkSeconds, DirectionalLightShadowMap, Instant, NavigationWorld, PathBuf,
    PointLightShadowMap, ProgramCacheSource, SceneData, ShadowCasterWorld, SnapshotPath,
    StreamingDiagnostics, TomeWorld, ViewSettings, archive,
    entities, env, expanded_model_placements, game_affine_to_view, game_settings, game_to_view,
    navigation, physworld, placed_transform, placed_vertex, probe_lighting, render_light,
    shader_cache, shadow_target_dimensions, tome, transformed, view_to_game, world_mesh,
};
use bevy::prelude::*;

/// Hold shape acquisition history until the published world retires.
#[derive(Resource)]
struct ShapeHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::physworld::source::ShapeHistoryLease>);

/// Keep static model history until preparation and renderer ownership stop.
#[derive(Resource)]
pub(super) struct ModelHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::bmodel::ModelHistoryLease>);

/// Keep material history until preparation and renderer ownership stop.
#[derive(Resource)]
pub(super) struct MaterialHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::material::MaterialHistoryLease>);

#[derive(Resource)]
pub(super) struct PreparedPhysicsTexts(pub(crate) crate::assets::declaration_texts::PhysicsSeed);

pub(super) fn prepare_level(
    base: PathBuf,
    catalog: std::sync::Arc<archive::Catalog>,
    level: crate::assets::level::LevelResources,
) -> Result<super::loading_screen::PreparedLevel, Box<dyn std::error::Error>> {
    let profile = env::var("DH2_PERF_PROFILE").as_deref() == Ok("1");
    let startup = Instant::now();
    let path = PathBuf::from(&level.world);
    let game_base = Some(base);
    let catalog = Some(catalog);
    let mut app = super::loading_screen::PreparedLevel::default();
    app.insert_resource(ModelHistoryScope(crate::bmodel::ModelHistoryLease::acquire()));
    app.insert_resource(MaterialHistoryScope(crate::material::MaterialHistoryLease::acquire()));
    let map = level
        .entities
        .trim_start_matches("maps/")
        .trim_end_matches(".entities");
    let language = crate::assets::lang::read_text_language(game_base.as_deref().unwrap())
        .map_err(std::io::Error::other)?;
    let list = crate::assets::map_resources::acquire_level_list(
        game_base.as_deref().unwrap(),
        catalog.as_ref().unwrap(),
        map,
        language,
    )
    .map_err(std::io::Error::other)?;
    crate::loading::begin_level(list, catalog.as_ref().unwrap());
    let mut physics_texts = crate::assets::declaration_texts::PhysicsSeed::new(
        catalog.as_ref().unwrap(), language,
    );
    let bytes = level.read_world(catalog.as_ref().unwrap())?;
    let world =
        world_mesh::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    // `run` lasts the whole session; the installed file is parsed.
    drop(bytes);
    super::load_timing::mark("world mesh");
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
        return Ok(super::loading_screen::PreparedLevel::diagnostic_exit());
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
        (Some(base), Some(catalog)) => {
            shader_cache::ProgramLibrary::open(base, shader_cache::ShaderCacheBackend::Pc, catalog)
        }
        _ => Err("original game base/catalog missing".to_owned()),
    };
    match &program_cache {
        Ok(library) => println!(
            "Installed program cache: {} programs, {} stage records, {} shared names, {} parameter declarations (data streams/definitions lazy)",
            library.cache.programs.len(),
            library.cache.stages.len(),
            library.cache.names.len(),
            library.parameter_kinds.len()
        ),
        Err(error) => eprintln!("Program cache unavailable: {error}"),
    }
    super::load_timing::mark("program cache");
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
            let settings = super::launch::load_installed_shadow_quality(catalog, preset)
                .map_err(std::io::Error::other)?;
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
        Some(catalog) => super::launch::read_installed_visuals(catalog).and_then(|visuals| {
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
    let mut shape_sources = crate::physworld::source::ShapeSources::new(catalog.as_ref().unwrap());
    app.insert_resource(ShapeHistoryScope(shape_sources.history_lease()));
    let trace_environment_cpu = env::var("DH2_DEBUG_ENVIRONMENT_CPU").as_deref() == Ok("1");
    let environment_cpu_start = if trace_environment_cpu { crate::loading::thread_cycles() } else { None };
    let (area_environments, environment_volumes) = match (entity_document.as_ref(), catalog.as_ref()) {
        (Some(document), Some(catalog)) => {
            let separate = catalog.has_overlays()
                || env::var("DH2_DEBUG_SEPARATE_ENVIRONMENT_PROFILES").as_deref() == Ok("1");
            if separate {
                let areas = crate::rendering::environment::load_area_environments_with_sources(document, catalog, &mut shape_sources)
                    .map_err(std::io::Error::other)?;
                let volumes = crate::rendering::environment::load_environment_volumes(document, catalog)
                    .map_err(std::io::Error::other)?;
                crate::rendering::environment::prepare_volume_owners(volumes, catalog, Some(areas), &mut shape_sources)
                    .map_err(std::io::Error::other)?
            } else {
                let volumes = crate::rendering::environment::load_environment_volumes(document, catalog)
                    .map_err(std::io::Error::other)?;
                crate::rendering::environment::prepare_volume_owners(volumes, catalog, None, &mut shape_sources)
                    .map_err(std::io::Error::other)?
            }
        }
        _ => (Vec::new(), Vec::new()),
    };
    shape_sources.finish_areas();
    if let (Some(start), Some(end)) = (environment_cpu_start, if trace_environment_cpu { crate::loading::thread_cycles() } else { None }) {
        eprintln!("MEMORY environment preparation CPU: areas={} volumes={} cycles={}", area_environments.len(), environment_volumes.len(), end.saturating_sub(start));
    }
    println!(
        "Loaded {} area environment trigger shapes",
        area_environments.len()
    );
    let probe_start = Instant::now();
    let area_fog = match (catalog.as_ref(), entity_document.as_ref()) {
        (Some(catalog), Some(document)) => {
            super::fog_volumes::load(catalog, document).map_err(std::io::Error::other)?
        }
        _ => Vec::new(),
    };
    let mut environment_probes = match (catalog.as_ref(), path.file_stem().and_then(|s| s.to_str()))
    {
        (Some(catalog), Some(_)) => {
            probe_lighting::load_for_world(catalog, &path.to_string_lossy())
                .map_err(std::io::Error::other)?
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
    if let Some(probes) = &mut environment_probes {
        probes.release_preparation_data();
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
    super::load_timing::mark("probes");
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
    let tome_path = PathBuf::from(&level.tome);
    if let Some(tome_bytes) = level.read_tome(catalog.as_ref().unwrap())? {
        let tome_started = Instant::now();
        let tome_tree = {
            let _tag = crate::platform::allocation_tags::tag("tome spatial tree");
            tome::SpatialTree::parse(&tome_bytes)?
        };
        let gate_states = tome::initial_gate_states(&tome_bytes)?;
        let query_data = {
            let _tag = crate::platform::allocation_tags::tag("tome query data");
            tome::DirectTomeQueryData::parse(&tome_bytes, &tome_tree)?
        };
        if env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
            for (field, bytes) in query_data.decoded_bytes() {
                eprintln!("LOAD tome decoded {field}: {} KB", bytes >> 10);
            }
        }
        let query_job_count = env::var("DH2_UMBRA_QUERY_JOBS")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::DEFAULT_QUERY_JOB_COUNT))?;
        let threading_mode = env::var("DH2_SCHEDULER_THREADING_MODE")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::DEFAULT_SCHEDULER_THREADING_MODE))?;
        let query_job_count = tome::world_query_job_count(query_job_count, threading_mode)?;
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
        let shadow_tome = {
            let _tag = crate::platform::allocation_tags::tag("tome shadow casters");
            tome::shadow_caster::ShadowCasterTome::parse(&tome_bytes)
        };
        match shadow_tome {
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
            sphere_graph: {
                let _tag = crate::platform::allocation_tags::tag("tome sphere graph");
                tome::SphereCellGraph::parse(&tome_bytes)
            },
            bytes: tome_bytes.into(),
            tree: tome_tree.into(),
            gate_states,
            query_data: query_data.into(),
            query_job_count,
            query_pool: query_pool.map(std::sync::Arc::new),
        });
    } else {
        println!(
            "No installed Tome at {}; F8 Tome diagnostics unavailable",
            tome_path.display()
        );
    }
    super::load_timing::mark("tome");
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
        let model_collision = if let Some(catalog) = catalog.as_ref() {
            let bodies = entity_document
                .as_ref()
                .map(entities::rigid_body_settings)
                .unwrap_or_default();
            let collision = super::view_geometry::acquire_static_model_collision_with_texts(
                catalog, &placed_models, &bodies, &mut physics_texts,
            );
            if env::var("DH2_DEBUG_PHYSICS_SIGNATURES").as_deref() == Ok("1") {
                let mut crc = flate2::Crc::new();
                crc.update(format!("{:?}", collision.materials).as_bytes());
                eprintln!("MEMORY model collision materials: count={} signature={:08x}", collision.materials.len(), crc.sum());
            }
            println!(
                "Placed static-model collision: {} matched authored root HKX bodies, {} triangles, {} failures/unsupported recipes; ExtraHD: {} acquired, {} unresolved",
                collision.matched,
                collision.triangles.len(),
                collision.failed,
                collision.extra_hd,
                collision.extra_hd_failed,
            );
            collision
        } else {
            super::view_geometry::ModelCollision::default()
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
                let read = |path: &str| physics_texts.read(
                    catalog.as_ref().unwrap(), path,
                    crate::assets::declaration_texts::PhysicsUse::World,
                );
                entities::physics_material::physics_material(name, read).map_err(|error| {
                    missing_materials.push(error);
                })
            })
            .collect();
        if env::var("DH2_DEBUG_PHYSICS_SIGNATURES").as_deref() == Ok("1") {
            let mut crc = flate2::Crc::new();
            for (name, material) in physics.materials.iter().zip(&materials) {
                crc.update(name.as_bytes());
                crc.update(format!("{material:?}").as_bytes());
                crc.update(&[0]);
            }
            eprintln!("MEMORY physics material output: count={} signature={:08x}", materials.len(), crc.sum());
        }
        if !missing_materials.is_empty() {
            println!("Physics materials unresolved: {missing_materials:?}");
        }
        let mut resolved: Vec<_> = materials.iter().flatten().cloned().collect();
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
                .map_or(
                    crate::gameplay::collision_settings::CollisionSettings::default().flags,
                    |m| m.collision.flags,
                )
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
                (
                    triangle.vertices.map(game_to_view),
                    u32::MAX,
                    material,
                    filter,
                )
            })
            .collect();
        if undecoded_filters > 0 {
            println!(
                "Collision filters: {undecoded_filters} triangles keep a partly decoded shape-tag path"
            );
        }
        let model_material_base = resolved.len() as u32;
        resolved.extend(model_collision.materials);
        let world = navigation::CollisionWorld::with_materials(
            level_triangles
                .into_iter()
                .chain(model_collision.triangles.into_iter().map(
                    |(triangle, owner, material, filter)| {
                        let material = if material == navigation::NO_MATERIAL {
                            material
                        } else {
                            model_material_base + material
                        };
                        (triangle, owner, material, filter)
                    },
                )),
            model_collision.owners,
            resolved,
        );
        if profile {
            println!(
                "PERF triangle collision index: {:.3}s",
                index_start.elapsed().as_secs_f64()
            );
        }
        super::load_timing::mark("physics and collision");
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
            {
                use crate::gameplay::collision_filter::{QueryFilter, filter_info, layer};
                for (label, filter) in [
                    (
                        "player proxy",
                        QueryFilter::World(filter_info(layer::PLAYER) & 0xfbffffff),
                    ),
                    ("mantle 0x2b", QueryFilter::layers(0x2b, 1 << layer::PLAYER)),
                ] {
                    let hit = world.first_hit_details(start, start + direction * 40.0, filter);
                    println!(
                        "NAV RAY {label} hit={:?}",
                        hit.map(|(distance, index, _, vertices)| (
                            distance,
                            world.collision_filter(index).map(|f| format!("{f:#x}")),
                            vertices.map(view_to_game)
                        ))
                    );
                }
            }
            let hit = world.first_hit_details(
                start,
                start + direction * 40.0,
                crate::gameplay::collision_filter::QueryFilter::ALL,
            );
            if let Some((_, index, _, _)) = hit {
                println!(
                    "NAV RAY all-layers filter word={:?}",
                    world.collision_filter(index).map(|f| format!("{f:#x}"))
                );
            }
            println!(
                "NAV RAY hit={:?}",
                hit.map(|(distance, index, owner, vertices)| (
                    distance,
                    world
                        .physics_material(index)
                        .map(|material| material.name.as_str()),
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
            return Ok(super::loading_screen::PreparedLevel::diagnostic_exit());
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
            return Ok(super::loading_screen::PreparedLevel::diagnostic_exit());
        }
        if env::var("DH2_ANIM_PROBE").as_deref() == Ok("1") {
            super::player::probe_animation(catalog.as_ref().unwrap())?;
            return Ok(super::loading_screen::PreparedLevel::diagnostic_exit());
        }
        if env::var("DH2_NAV_PROBE").as_deref() == Ok("1") {
            let start = player_start.ok_or("navigation probe needs a player start")?;
            let yaw = start.forward_xy[1].atan2(start.forward_xy[0]);
            super::player::probe_player(
                catalog.as_ref().unwrap(),
                &world,
                Vec3::from(start.position),
                yaw,
                true,
            )?;
            return Ok(super::loading_screen::PreparedLevel::diagnostic_exit());
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
        // Bevy's default caps an unfocused window at 60 Hz; a measurement
        // must not depend on which window has focus.
        app.insert_resource(bevy::winit::WinitSettings::continuous());
    }
    let machinery = super::machinery::AmbientMachinery::load_with_texts(
        catalog.as_ref().unwrap(),
        entity_document.as_ref(),
        &placed_models,
        &mut physics_texts,
        &mut shape_sources,
    )
    .map_err(std::io::Error::other)?;
    app.insert_resource(machinery);
    app.insert_resource(super::model_motion::BoundModels::new(
        entity_document
            .as_ref()
            .map(crate::entities::entity_binds)
            .unwrap_or_default(),
    ));
    app.insert_resource(
        super::fog_scene::WorldWind::load(catalog.as_ref().unwrap(), entity_document.as_ref())
            .map_err(std::io::Error::other)?,
    );
    if let (Some(catalog), Some(document)) = (catalog.as_ref(), entity_document.as_ref()) {
        let map = level
            .entities
            .strip_prefix("maps/")
            .unwrap_or(&level.entities)
            .strip_suffix(".entities")
            .ok_or("physics entity resource has no map name")?;
        match super::physics_scene::PhysicsScene::load(catalog, document, map, &environment_volumes, &mut physics_texts, &mut shape_sources)
        {
            Ok(physics) => {
                app.insert_resource(physics);
            }
            Err(error) => eprintln!("Level physics publication pending: {error}"),
        }
        app.insert_resource(
            super::audio::level::LevelAudio::load(
                catalog,
                &level.entities,
                game_base.as_deref().unwrap(),
            )
            .map_err(std::io::Error::other)?,
        );
    }
    app.insert_resource(super::vegetation_wind::LevelTriggerWinds::load(
        catalog.as_ref().unwrap(),
        entity_document.as_ref(),
        &mut shape_sources,
    ));
    drop(shape_sources);
    app.insert_resource(super::fog_scene::FogMotion::load().map_err(std::io::Error::other)?);
    super::load_timing::mark("machinery and wind");
    app.insert_resource(ProgramCacheSource(program_cache))
        .insert_resource(ClearColor(Color::srgb(0.055, 0.045, 0.06)))
        // Bevy allocates six depth layers for every shadow-casting point
        // light. Dense missions contain hundreds of authored lights, so keep
        // the shadow maps small enough to leave room for the mission assets.
        // Authored shadow flags remain intact.
        .insert_resource(PointLightShadowMap { size: 128 })
        .insert_resource(DirectionalLightShadowMap { size: 512 })
        .insert_resource(
            crate::rendering::bevy_backend::world_effects::WorldEffectFrames::load(
                entity_document.as_ref(),
            )
            .map_err(std::io::Error::other)?,
        )
        .insert_resource(
            crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions {
                graphs: std::sync::Arc::new(
                    entity_document
                        .as_ref()
                        .map(crate::assets::entities::effects::load)
                        .transpose()
                        .map_err(std::io::Error::other)?
                        .unwrap_or_default(),
                ),
                actors: std::sync::Arc::new(
                    entity_document
                        .as_ref()
                        .map(|document| {
                            crate::assets::entities::effects::actors::load(
                                document,
                                catalog.as_ref().unwrap(),
                            )
                        })
                        .unwrap_or_default(),
                ),
                seed: crate::platform::performance_counter::counter()
                    .map_err(std::io::Error::other)? as u64,
            },
        )
        .insert_resource(super::loading_fade::LoadingFade::load(catalog.as_ref().unwrap(), entity_document.as_ref())
            .map_err(std::io::Error::other)?)
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
            environment_probes: environment_probes.map(std::sync::Arc::new),
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
        .insert_resource(super::state::EnvironmentVolumes(environment_volumes));
    app.insert_resource(PreparedPhysicsTexts(physics_texts));
    app.insert_resource(ViewSettings(view_settings));
    app.insert_resource(StreamingDiagnostics::new(&path));
    drop(entity_document);
    Ok(app)
}
