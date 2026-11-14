//! Acquire the selected map through the installed resource owners.
//! Runs behind the loading screen; publishes a complete typed resource bundle.
use super::{
    BenchmarkSeconds, DirectionalLightShadowMap, Instant, NavigationWorld, PathBuf,
    PointLightShadowMap, ProgramCacheSource, SceneData, ShadowCasterWorld, SnapshotPath,
    StreamingDiagnostics, TomeWorld, ViewSettings, archive,
    entities, env, expanded_model_placements, game_affine_to_view, game_settings, game_to_view,
    navigation, physworld, placed_transform, placed_vertex, probe_lighting, render_light,
    shader_cache, shadow_target_dimensions, tome, transformed, view_to_game,
};
use bevy::prelude::*;
use std::sync::Arc;

/// Hold shape acquisition history until the published world retires.
#[derive(Resource)]
pub(super) struct ShapeHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::physworld::source::ShapeHistoryLease>);

/// Keep static model history until preparation and renderer ownership stop.
#[derive(Resource)]
pub(super) struct ModelHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::bmodel::ModelHistoryLease>);

/// Keep material history until preparation and renderer ownership stop.
#[derive(Resource)]
pub(super) struct MaterialHistoryScope(#[allow(dead_code)] std::sync::Arc<crate::material::MaterialHistoryLease>);

/// Blueprint construction credits outlive transient expansion descriptors.
#[derive(Resource)]
pub(super) struct BlueprintHistoryScope(#[allow(dead_code)] std::sync::Arc<entities::blueprint::HistoryLease>);

/// Keep skeletal acquisitions through preparation and render-world retirement.
#[derive(Resource)]
pub(super) struct SkeletalHistoryScope {
    _models: crate::bmd6::HistoryLease,
    _skeletons: crate::edge_skeleton::HistoryLease,
}
impl SkeletalHistoryScope {
    pub(super) fn new(catalog: &archive::Catalog) -> Self {
        Self {
            _models: crate::bmd6::HistoryLease::acquire(catalog.identity()),
            _skeletons: crate::edge_skeleton::HistoryLease::acquire(catalog.identity()),
        }
    }
}

#[derive(Resource)]
pub(super) struct PreparedPhysicsTexts(pub(crate) crate::assets::declaration_texts::PhysicsSeed);

/// The archived companions belong to this map, even when preparation fails.
#[derive(Resource)]
pub(super) struct LevelSourceReads {
    catalog: Arc<archive::Catalog>,
    level: crate::assets::level::LevelResources,
}

impl LevelSourceReads {
    pub(super) fn acquisition_source(&self) -> (&Arc<archive::Catalog>, &str) {
        (&self.catalog, &self.level.entities)
    }
}

impl Drop for LevelSourceReads {
    fn drop(&mut self) {
        let level = &self.level;
        let mut retired = 0;
        for path in [level.world.as_str(), level.entities.as_str(), level.tome.as_str()]
            .into_iter().chain(level.physics.as_deref()).chain(level.probe_network.as_deref())
        {
            if let Some(kind) = self.catalog.kind(path) {
                retired += usize::from(crate::loading::forget(self.catalog.identity(), kind, path).is_some());
            }
        }
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("LEVEL_SOURCE_HISTORY world={} retired={retired}", level.world);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CancellationPoint {
    General,
    CollisionShapes,
    PlacedCollision,
    CollisionIndex,
}

pub(super) fn prepare_level(
    base: PathBuf,
    catalog: std::sync::Arc<archive::Catalog>,
    level: crate::assets::level::LevelResources,
    needs_initial_inventory: bool,
    initial_load: bool,
    cancelled: impl Fn(CancellationPoint) -> bool,
) -> Result<super::loading_screen::PreparedLevel, Box<dyn std::error::Error>> {
    // Host worker shutdown: stop between acquisition phases instead of
    // loading the remaining map after its presentation owner has closed.
    // The caller owns cancellation; resource readers and gameplay are unchanged.
    let check_at = |point| -> Result<(), std::io::Error> {
        if cancelled(point) {
            Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "level acquisition cancelled"))
        } else {
            Ok(())
        }
    };
    let check_cancelled = || check_at(CancellationPoint::General);
    check_cancelled()?;
    let profile = env::var("DH2_PERF_PROFILE").as_deref() == Ok("1");
    let startup = Instant::now();
    let level_reads = LevelSourceReads { catalog: catalog.clone(), level };
    let level = &level_reads.level;
    let path = PathBuf::from(&level.world);
    let game_base = Some(base);
    let mut app = super::loading_screen::PreparedLevel::default();
    app.insert_resource(ModelHistoryScope(crate::bmodel::ModelHistoryLease::acquire(&catalog)));
    app.insert_resource(MaterialHistoryScope(crate::material::MaterialHistoryLease::acquire(&catalog)));
    app.insert_resource(SkeletalHistoryScope::new(&catalog));
    app.insert_resource(BlueprintHistoryScope(entities::blueprint::HistoryLease::acquire(catalog.identity())));
    let catalog = Some(catalog);
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
    crate::loading::begin_level(list.clone(), catalog.as_ref().unwrap());
    app.own_preload(catalog.as_ref().unwrap().clone());
    let persistent_heap = crate::platform::console::shared().read().cvars
        .int("com_usePersistentHeap").ok_or("com_usePersistentHeap is not registered")? != 0;
    list.register_preload_headers(catalog.as_ref().unwrap(), map, persistent_heap)
        .map_err(std::io::Error::other)?;
    let mut physics_texts = crate::assets::declaration_texts::PhysicsSeed::new(
        catalog.as_ref().unwrap(), language,
    );
    let world_started = profile.then(Instant::now);
    let world = level.load_world(catalog.as_ref().unwrap(), || cancelled(CancellationPoint::General))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    super::load_timing::mark("world mesh");
    check_cancelled()?;
    if let Some(started) = world_started {
        println!("PERF world mesh read and parse: {:.3}s", started.elapsed().as_secs_f64());
    }
    let (player_start, placed_models, entity_document) =
        match level.read_entities(catalog.as_ref().unwrap())? {
            Some(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let document =
                    entities::parse(&text).map_err(|error| format!("entity parse: {error}"))?;
                (
                    entities::player_start(&document),
                    entities::render_placements(&document),
                    Some(document),
                )
            }
            None => (None, Vec::new(), None),
        };
    let character = super::player::character_name(super::pause_menu::debug_character());
    let loadout = if !needs_initial_inventory { None } else { match crate::assets::inventory_item::default_loadout(catalog.as_ref().unwrap(), character) {
        Ok(loadout) => Some(loadout),
        Err(error) => { eprintln!("Initial inventory loadout unavailable: {error}"); None }
    }};
    // The existing UI worker owns these concrete game-logic state backends;
    // register only those implementations, then use the authored transition matrix.
    if let Some(document) = entity_document.as_ref() {
        let declaration = crate::gameplay::game_logic::Declaration::load(catalog.as_ref().unwrap(), document)
            .map_err(std::io::Error::other)?;
        let mut logic = crate::gameplay::game_logic::GameLogic::new(declaration);
        use crate::gameplay::game_logic::StateId;
        for state in [StateId::MAP_TRANSITION, StateId::INGAME, StateId::PAUSE, StateId::READ_NOTE, StateId::JOURNAL] {
            if logic.declaration.states[state.0 as usize].is_some() { logic.register(state).map_err(std::io::Error::other)?; }
        }
        logic.begin_map_transition().map_err(std::io::Error::other)?;
        app.insert_resource(logic);
    }
    app.insert_resource(super::readable_movie::ReadableRequests::default());
    app.insert_resource(crate::assets::inventory_item::PickableSourceLease::for_catalog(catalog.as_ref().unwrap())
        .map_err(std::io::Error::other)?);
    let mut interactions = match entity_document.as_ref() {
        Some(document) => match super::interactions::Interactions::load(
            catalog.as_ref().unwrap().clone(), document, character,
        ) {
            Ok(interactions) => Some(interactions),
            Err(error) => { eprintln!("Interaction declarations unavailable: {error}"); None }
        },
        None => None,
    };
    let limits = if let Some(interactions) = interactions.as_ref() {
        Some(interactions.limits())
    } else {
        match crate::assets::inventory_item::ReadableLimits::load(catalog.as_ref().unwrap(), character) {
            Ok(limits) => Some(std::sync::Arc::new(limits)),
            Err(error) => { eprintln!("Inventory collection unavailable: {error}"); None }
        }
    };
    let initial_inventory = loadout.as_ref().zip(limits.as_ref()).map(|(loadout, limits)|
        loadout.iter().map(|(path, quantity)| (path.clone(), *quantity, limits.maximum(path))).collect());
    let initial_inventory = initial_inventory.map(|entries: Vec<(String, u32, u32)>| entries.into_iter()
        .map(|(path, quantity, maximum)| {
            let modifiers = crate::assets::inventory_item::modifiers(catalog.as_ref().unwrap(), &path)
                .unwrap_or_else(|error| { eprintln!("Initial inventory modifiers {path}: {error}"); Default::default() });
            super::interaction_focus::InitialItem { path, quantity, maximum, modifiers }
        }).collect());
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
    check_cancelled()?;
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
            let mut settings = super::launch::load_installed_shadow_quality(catalog, preset)
                .map_err(std::io::Error::other)?;
            if let Some(quality) = settings.as_mut() {
                game_settings::shadow_quality_settings(&crate::platform::console::shared().cvar_text(), quality)
                    .map_err(std::io::Error::other)?;
            }
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
    let mut light_source_reads = 0usize;
    let mut light_source_bytes = 0usize;
    let reuse_light_sources = catalog.as_ref().is_some_and(|catalog| !catalog.has_overlays())
        && std::env::var("DH2_DEBUG_SEPARATE_LIGHT_SOURCES").as_deref() != Ok("1");
    let placed_lights = entity_document
        .as_ref()
        .map(|document| {
            entities::placed_lights_from_sources(document, |resource| {
                let Some(catalog) = catalog.as_ref() else {
                    return Ok(None);
                };
                let bytes = catalog.read(resource).map_err(|error| error.to_string())?;
                light_source_reads += 1;
                light_source_bytes += bytes.as_ref().map_or(0, Vec::len);
                Ok(bytes.map(|bytes| crate::assets::declaration_texts::adopt_utf8(bytes, "light")))
            }, reuse_light_sources)
        })
        .transpose()?
        .unwrap_or_default();
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("LIGHT_SOURCE_PREPARATION reads={light_source_reads} source_bytes={light_source_bytes} lights={} signature={}",
            placed_lights.len(), blake3::hash(format!("{placed_lights:?}").as_bytes()));
    }
    let world_environment = entity_document
        .as_ref()
        .and_then(entities::world_environment);
    let environment_data = match (world_environment.as_deref(), catalog.as_ref()) {
        (Some(environment), Some(catalog)) => entities::load_world_environment(catalog, environment)?,
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
            probe_lighting::load_for_rendered_world(catalog, &path.to_string_lossy(), world.matrices.len())
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
    check_cancelled()?;
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
    check_cancelled()?;
    if let Some(physics_bytes) = level.read_physics(catalog.as_ref().unwrap())? {
        check_cancelled()?;
        let physics_start = std::time::Instant::now();
        let stage_triangles = env::var_os("DH2_DEBUG_STAGE_COLLISION_TRIANGLES").is_some_and(|value| value == "1");
        let keep_details = env::var_os("DH2_DEBUG_KEEP_COLLISION_DETAILS").is_some_and(|value| value == "1");
        let trace_physics = env::var_os("DH2_DEBUG_PHYSICS_SIGNATURES").is_some_and(|value| value == "1");
        let physics = physworld::parse(&physics_bytes)?;
        let (world_triangles, retained_source) = {
            let shape_cycles = trace_physics.then(crate::loading::thread_cycles).flatten();
            let geometry = physics.collision_for_navigation(&physics_bytes, keep_details,
                &|| cancelled(CancellationPoint::CollisionShapes))?;
            if let Some(start) = shape_cycles {
                let cycles = crate::loading::thread_cycles().unwrap_or(start).saturating_sub(start);
                eprintln!("MEMORY collision extraction: details={keep_details} detail_capacity_bytes={} cycles={cycles}",
                    geometry.detail_capacity_bytes());
            }
            let retained_source = stage_triangles.then_some(physics_bytes);
            if profile {
                println!(
                    "PERF authored physics parse and decode: {:.3}s",
                    physics_start.elapsed().as_secs_f64()
                );
            }
            println!(
                "Authored collision: {} convex shapes, {} compressed meshes, {} triangles; {} compressed meshes remain undecoded",
                geometry.convex_count(),
                geometry.compressed_count(),
                geometry.triangles.len(),
                geometry.compressed_decode_failures.len()
            );
            (geometry.triangles, retained_source)
        };
        check_cancelled()?;
        let model_start = std::time::Instant::now();
        let model_collision = if let Some(catalog) = catalog.as_ref() {
            let bodies = entity_document
                .as_ref()
                .map(entities::rigid_body_settings)
                .unwrap_or_default();
            let collision = super::view_geometry::acquire_static_model_collision_with_texts(
                catalog, &placed_models, &bodies, &mut physics_texts,
                || check_at(CancellationPoint::PlacedCollision),
            )?;
            if trace_physics {
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
                entities::physics_material::resources::acquire_with(catalog.as_ref().unwrap(), name, read).map_err(|error| {
                    missing_materials.push(error);
                })
            })
            .collect();
        if trace_physics {
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
        let mut resolved: Vec<_> = materials.iter().flatten().map(|material| material.material().clone()).collect();
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
        check_cancelled()?;
        let index_start = std::time::Instant::now();
        let index_cycles = trace_physics.then(crate::loading::thread_cycles).flatten();
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
        let mut resolve_triangle = |triangle: &physworld::MeshTriangle| {
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
        };
        let model_material_base = resolved.len() as u32;
        resolved.extend(model_collision.materials);
        let placed_triangles = model_collision.triangles.into_iter().map(
            |(triangle, owner, material, filter)| {
                let material = if material == navigation::NO_MATERIAL {
                    material
                } else {
                    model_material_base + material
                };
                (triangle, owner, material, filter)
            },
        );
        let (mut world, staging_bytes) = if stage_triangles {
            // Retain the previous borrowed-input allocation for measurement.
            let staged: Vec<_> = world_triangles.iter().map(&mut resolve_triangle).collect();
            let bytes = staged.capacity() * std::mem::size_of::<([Vec3; 3], u32, u32, u32)>();
            (
                navigation::CollisionWorld::with_materials_until(
                    staged.into_iter().chain(placed_triangles), model_collision.owners, resolved,
                    || cancelled(CancellationPoint::CollisionIndex),
                )?,
                bytes,
            )
        } else {
            let level_triangles = world_triangles.into_iter().map(|triangle| resolve_triangle(&triangle));
            (
                navigation::CollisionWorld::with_materials_until(
                    level_triangles.chain(placed_triangles), model_collision.owners, resolved,
                    || cancelled(CancellationPoint::CollisionIndex),
                )?,
                0,
            )
        };
        let retained_source_bytes = retained_source.as_ref().map_or(0, Vec::capacity);
        drop(retained_source);
        drop(physics);
        drop(materials);
        check_cancelled()?;
        if let Some(start) = index_cycles {
            let cycles = crate::loading::thread_cycles().unwrap_or(start).saturating_sub(start);
            eprintln!("MEMORY collision staging: staged={stage_triangles} source_capacity_bytes={retained_source_bytes} tuple_capacity_bytes={staging_bytes} cycles={cycles} signature={:08x}",
                world.preparation_signature());
        }
        if undecoded_filters > 0 {
            println!(
                "Collision filters: {undecoded_filters} triangles keep a partly decoded shape-tag path"
            );
        }
        if let Some(interactions) = interactions.as_mut() {
            interactions.start_boxes(&mut world);
        }
        if profile {
            println!(
                "PERF triangle collision index: {:.3}s",
                index_start.elapsed().as_secs_f64()
            );
        }
        super::load_timing::mark("physics and collision");
        check_cancelled()?;
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
    let mut placed_models = placed_models;
    let catalog_for_machinery = catalog.as_ref().unwrap();
    let separate_placements = catalog_for_machinery.has_overlays()
        || env::var_os("DH2_DEBUG_SEPARATE_BLUEPRINT_PLACEMENTS").is_some_and(|v| v == "1")
        || env::var_os("DH2_DEBUG_RETAIN_PLACEMENT_STAGING").is_some_and(|v| v == "1");
    let expanded_placement_roots = (!separate_placements).then_some(placed_models.len());
    if !separate_placements {
        super::view_geometry::expand_model_placements(&mut placed_models, catalog_for_machinery);
    }
    let machinery_placements = (separate_placements && entity_document.is_some())
        .then(|| expanded_model_placements(&placed_models, catalog_for_machinery));
    let machinery = super::machinery::AmbientMachinery::load_with_texts_cancellable(
        catalog_for_machinery,
        entity_document.as_ref(),
        machinery_placements.as_deref().unwrap_or(&placed_models),
        &mut physics_texts,
        &mut shape_sources,
        Some(&|| cancelled(CancellationPoint::General)),
    )
    .map_err(std::io::Error::other)?;
    drop(machinery_placements);
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
        let mut live = crate::gameplay::entity_lifetime::LiveEntities::default();
        match super::physics_scene::PhysicsScene::load(catalog, document, map, &environment_volumes, &mut physics_texts, &mut shape_sources, &mut live)
        {
            Ok(physics) => {
                app.insert_resource(live);
                app.insert_resource(crate::gameplay::entity_events::EntityEvents::default());
                app.insert_resource(physics);
            }
            Err(error) => eprintln!("Level physics publication pending: {error}"),
        }
        app.insert_resource(
            super::audio::level::LevelAudio::load(
                catalog,
                &level.entities,
            )
            .map_err(std::io::Error::other)?,
        );
    }
    if let Some(interactions) = interactions {
        app.insert_resource(interactions);
    }
    app.insert_resource(super::interaction_focus::FocusedInteraction::default());
    app.insert_resource(super::interaction_focus::InitialInventory { items: initial_inventory });
    app.insert_resource(super::interaction_focus::RetiredPlacements::default());
    app.insert_resource(super::usable_animation::UsableAnimations::default());
    app.insert_resource(super::usable_effects::EffectRequests::default());
    app.insert_resource(super::usable_effects::UsableEffects::default());
    app.insert_resource(super::usable_effects::DynamicWorldMaterials::default());
    app.insert_resource(super::usable_effects::DynamicDecalMaterials::default());
    app.insert_resource(super::projected_decals::RuntimeDecalProxies::default());
    app.insert_resource(super::runtime_fx_images::RuntimeFxImages::default());
    app.insert_resource(super::runtime_materials::RuntimeMaterialCoordinator::default());
    app.insert_resource(super::vegetation_wind::LevelTriggerWinds::load(
        catalog.as_ref().unwrap(),
        entity_document.as_ref(),
        &mut shape_sources,
    ));
    drop(shape_sources);
    app.insert_resource(super::fog_scene::FogMotion::load().map_err(std::io::Error::other)?);
    super::load_timing::mark("machinery and wind");
    check_cancelled()?;
    app.insert_resource(ProgramCacheSource(std::sync::Arc::new(std::sync::Mutex::new(program_cache))))
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
        .insert_resource(super::loading_fade::LoadingFade::load(catalog.as_ref().unwrap(), entity_document.as_ref(), initial_load)
            .map_err(std::io::Error::other)?)
        .insert_resource(SceneData {
            source_path: path.clone(),
            world,
            player_start,
            placed_models,
            expanded_placement_roots,
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
            catalog,
        })
        .insert_resource(super::state::EnvironmentVolumes(environment_volumes));
    app.insert_resource(PreparedPhysicsTexts(physics_texts));
    app.insert_resource(ViewSettings(view_settings));
    app.insert_resource(StreamingDiagnostics::new(&path));
    drop(entity_document);
    app.insert_resource(level_reads);
    Ok(app)
}
