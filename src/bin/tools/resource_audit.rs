//! Resource acquisition diagnostics over installed game assets. These commands
//! measure real owner costs and output equivalence; they do not run tests.
use dishonored2_rust::entities;
use std::{env, io, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn observe_decal_leases(
    catalog: &dishonored2_rust::archive::Catalog,
    programs: &mut dishonored2_rust::shader_cache::ProgramLibrary,
    effects: &[String],
) -> Result<()> {
    use dishonored2_rust::{assets::decal::DecalMaterials, rendering::particles::{
        definition::ParticleResources, world_sources::WorldFxSources,
    }};
    use std::{collections::{BTreeMap, HashSet}, sync::{Arc, Weak}};

    if effects.len() < 2 { return Err(invalid("decal lease observation requires two installed effects").into()); }
    for share in [false, true] {
        let mut level = ParticleResources::default();
        let mut batches = Vec::new();
        let mut declarations: BTreeMap<String, Vec<Weak<DecalMaterials<String>>>> = BTreeMap::new();
        let mut signature = blake3::Hasher::new();
        for name in effects {
            let mut separate = ParticleResources::default();
            let resources = if share { &mut level } else { &mut separate };
            let batch = WorldFxSources::load_named_shared(vec![name.clone()], catalog, programs, resources);
            let effect = batch.effects.get(name).ok_or("installed effect was not acquired")?
                .as_ref().map_err(Clone::clone)?;
            for decal in effect.groups.iter().flat_map(|group| &group.decals) {
                let decal = decal.as_ref().map_err(Clone::clone)?;
                for (path, declaration) in decal.asset.declarations.iter().zip(&decal.declarations) {
                    let (Some(path), Some(declaration)) = (path, declaration) else { continue; };
                    declarations.entry(path.clone()).or_default().push(Arc::downgrade(declaration));
                    for text in [Some(name), Some(path), declaration.opaque.as_ref(),
                        declaration.opaque_instanced.as_ref(), declaration.alpha.as_ref(), declaration.alpha_instanced.as_ref()] {
                        let text = text.map_or(&[][..], |value| value.as_bytes());
                        signature.update(&(text.len() as u64).to_le_bytes());
                        signature.update(text);
                    }
                }
            }
            batches.push(batch);
        }
        if declarations.is_empty() { return Err(invalid("no installed decal declarations were admitted").into()); }
        let distinct: usize = declarations.values().map(|references| {
            references.iter().map(Weak::as_ptr).collect::<HashSet<_>>().len()
        }).sum();
        drop(batches);
        level.retire_unused();
        let retained = declarations.values().flatten().filter(|reference| reference.strong_count() != 0).count();
        eprintln!("DECAL_LEASES shared={share} names={} references={} live_identities={distinct} retained_references={retained} signature={}",
            declarations.len(), declarations.values().map(Vec::len).sum::<usize>(), signature.finalize());
    }
    Ok(())
}

pub fn run(command: &str, base: &Path, args: &[String]) -> Result<()> {
    match command {
        "inspect-scene-gpu-resources" => {
            #[cfg(feature = "game")]
            dishonored2_rust::app::inspect_scene_gpu_resources(base, args)?;
            #[cfg(not(feature = "game"))]
            return Err("inspect-scene-gpu-resources requires the game feature".into());
        }
        "inspect-scene-publication" => {
            #[cfg(feature = "game")]
            dishonored2_rust::app::inspect_scene_publication(base, args)?;
            #[cfg(not(feature = "game"))]
            return Err("inspect-scene-publication requires the game feature".into());
        }
        "inspect-scene-preparation" => {
            #[cfg(feature = "game")]
            dishonored2_rust::app::inspect_scene_preparation(base, args)?;
            #[cfg(not(feature = "game"))]
            return Err("inspect-scene-preparation requires the game feature".into());
        }
        "inspect-cpu-preparation" => {
            #[cfg(feature = "game")]
            dishonored2_rust::app::inspect_cpu_preparation(base, args)?;
            #[cfg(not(feature = "game"))]
            return Err("inspect-cpu-preparation requires the game feature".into());
        }
        "inspect-separate-mips" => {
            use dishonored2_rust::{archive::Catalog, texture};
            if args.is_empty() { return Err("inspect-separate-mips requires installed mip paths".into()); }
            let catalog = Catalog::open(base)?;
            for path in args {
                let (stem, suffix) = path.rsplit_once("_mip").ok_or("expected a separate mip path")?;
                let level = suffix.parse::<u32>()?;
                let base = catalog.read_prefix(stem, 62)?.ok_or("missing base image")?;
                let format = texture::BimageInfo::parse(&base)?.format;
                let mip = texture::read_separate_mip(&catalog, path, level, format)?
                    .ok_or("missing separate mip")?;
                let mut crc = flate2::Crc::new();
                crc.update(&mip.payload);
                println!("BIMAGE_MIP path={path} size={}x{} format={format:?} payload_bytes={} crc={:08x}",
                    mip.width, mip.height, mip.payload.len(), crc.sum());
            }
        }
        "inspect-acoustics" => {
            use dishonored2_rust::assets::{archive::Catalog, sound_propagation::Propagation};
            let catalog = Catalog::open(base)?;
            let (measure_routes, paths) = match args.split_first() {
                Some((flag, paths)) if flag == "--routes" => (true, paths.to_vec()),
                _ => (false, catalog.paths_with_suffix(".soundpropa")),
            };
            if paths.is_empty() { return Err(invalid("no acoustic resource paths selected").into()); }
            for path in paths {
                let graph = Propagation::load(&catalog, &path)?
                    .ok_or_else(|| invalid(format!("indexed acoustic resource is absent: {path}")))?;
                println!("ACOUSTIC_RESOURCE path={path} zones={} portals={} route_dimension={}",
                    graph.zones.len(), graph.portals.len(), graph.routes.zone_count);
                if measure_routes { graph.observe_route_lookups(&path)?; }
            }
        }
        "inspect-inline-volumes" => {
            use dishonored2_rust::{archive::Catalog, texture};
            let catalog = Catalog::open(base)?;
            let mut count = 0;
            let mut payload_bytes = 0;
            let mut copy_capacity = 0;
            let mut retained_headers = 0;
            let mut digest = blake3::Hasher::new();
            for path in catalog.paths_with_suffix(".bimage7") {
                let header = catalog.read_prefix(&path, 12)?.ok_or("indexed image is absent")?;
                if header.get(8..12) != Some(1u32.to_le_bytes().as_slice()) {
                    continue;
                }
                let bytes = catalog.read(&path)?.ok_or("indexed volume is absent")?;
                let allocation = bytes.as_ptr();
                let reference = texture::compressed_inline_volume(&bytes)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
                let owned = texture::compressed_inline_volume_owned(bytes)
                    .map_err(|error| invalid(format!("{path}: {error}")))?;
                if owned.info != reference.info || owned.depth != reference.depth
                    || owned.payload != reference.payload || owned.payload.as_ptr() != allocation
                {
                    return Err(invalid(format!("inline volume staging mismatch: {path}")).into());
                }
                count += 1;
                payload_bytes += owned.payload.len();
                copy_capacity += reference.payload.capacity();
                retained_headers += owned.payload.capacity() - owned.payload.len();
                digest.update(path.as_bytes());
                digest.update(blake3::hash(&owned.payload).as_bytes());
            }
            println!("BIMAGE_VOLUME count={count} payload_bytes={payload_bytes} copy_capacity={copy_capacity} retained_slack={retained_headers} signature={}", digest.finalize());
        }
        "inspect-world-geometry" => {
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let worlds = if args.is_empty() {
                catalog.paths_with_suffix(".bwm")
            } else {
                args.to_vec()
            };
            for path in worlds {
                let started = std::time::Instant::now();
                let world = dishonored2_rust::world_mesh::load(&catalog, &path, || false)?;
                let elapsed = started.elapsed();
                let mut crc = flate2::Crc::new();
                let mut vertices = 0;
                let mut indices = 0;
                for mesh in &world.meshes {
                    vertices += mesh.positions().len();
                    indices += mesh.indices().len();
                    for value in mesh.positions().iter().flatten() {
                        crc.update(&value.to_le_bytes());
                    }
                    for (uv, channels) in mesh.uvs().iter().zip(mesh.packed_channels()) {
                        crc.update(&uv[0].to_le_bytes());
                        crc.update(&uv[1].to_le_bytes());
                        crc.update(channels);
                    }
                    for index in mesh.indices() { crc.update(&index.to_le_bytes()); }
                }
                println!(
                    "BWM_GEOMETRY path={path} meshes={} objects={} vertices={vertices} indices={indices} crc={:08x} elapsed_ms={:.3}",
                    world.meshes.len(), world.objects.len(), crc.sum(), elapsed.as_secs_f64() * 1000.0,
                );
            }
        }
        "inspect-world-queries" => {
            let path = args.first().ok_or("inspect-world-queries requires a WORLD resource")?;
            let unshared = args.get(1).is_some_and(|value| value == "unshared");
            env::set_var("DH2_DEBUG_UNSHARED_WORLD_QUERIES", if unshared { "1" } else { "0" });
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let world = dishonored2_rust::world_mesh::load(&catalog, path, || false)?;
            let before = dishonored2_rust::platform::allocation_tags::report().into_iter()
                .find(|(name, _)| *name == "WORLD spatial queries").map_or(0, |(_, bytes)| bytes);
            let mut owners = Vec::new();
            let start = std::time::Instant::now();
            for round in 0..200 {
                let query = world.spatial_query();
                if round < 3 { owners.push(query); }
                else { std::hint::black_box(query); }
            }
            let acquisition = start.elapsed();
            let unique = owners.iter().map(|query| std::sync::Arc::as_ptr(query) as usize)
                .collect::<std::collections::HashSet<_>>().len();
            let acquired_bytes = dishonored2_rust::platform::allocation_tags::report().into_iter()
                .find(|(name, _)| *name == "WORLD spatial queries").map_or(0, |(_, bytes)| bytes) - before;
            // Real WORLD bounds exercise the unchanged original query path.
            // This reports outputs/costs; it supplies no invented expected values.
            let mut hash = blake3::Hasher::new();
            let start = std::time::Instant::now();
            let mut hits = 0;
            let mut queries = 0;
            for record in world.visibility.iter().take(200) {
                let volume = dishonored2_rust::rendering::bounds::OrientedBox::from_aabb(record.bounds);
                let mask = owners[0].query_box(record.bounds, &volume);
                hits += mask.iter().filter(|&&value| value != 0).count();
                queries += 1;
                hash.update(&mask);
            }
            println!("mode={} requests=200 live_consumers=3 distinct_live_trees={unique} acquired_bytes={acquired_bytes} acquisition_us={} queries={queries} query_us={} hits={hits} output_hash={}",
                if unshared { "unshared" } else { "shared" }, acquisition.as_micros(), start.elapsed().as_micros(), hash.finalize());
        }
        "inspect-decal-materials" => {
            use dishonored2_rust::{
                assets::decal::DecalResources,
                rendering::decals::material::DecalProgram,
                render_program::{RenderParameterKind, SerializedParameter, SerializedParameterBlock, SerializedParameterValue},
                shader_cache::{ProgramLibrary, ShaderCacheBackend}, material,
            };
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let mut library = ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog)?;
            let mut declarations = DecalResources::default();
            let mut names = std::collections::BTreeSet::new();
            let mut failures = 0;
            let mut count = 0;
            for path in catalog.paths_containing("generated/decls/decal/") {
                let Some(name) = path.strip_prefix("generated/decls/decal/").and_then(|p| p.strip_suffix(".decl")) else { continue; };
                count += 1;
                match declarations.resolve(&catalog, name) {
                    Ok(Some(references)) => names.extend([
                        &references.opaque, &references.opaque_instanced,
                        &references.alpha, &references.alpha_instanced,
                    ].into_iter().flatten().cloned()),
                    Ok(None) => { println!("decal_missing={name}"); failures += 1; }
                    Err(error) => { println!("decal_error={name}: {error}"); failures += 1; }
                }
            }
            let mut kinds = [0usize; 4];
            let mut fx = 0;
            for name in &names {
                let result = (|| -> std::result::Result<DecalProgram, String> {
                    let bytes = catalog.read(&format!("generated/decls/material/{name}.decl"))
                        .map_err(|error| error.to_string())?.ok_or("decal material absent")?;
                    let main = material::authored_program_override(&bytes, "mainProgram").ok_or("decal MAIN absent")?;
                    let parameters = material::material_option_parameters(&bytes, |key| {
                        library.parameter_kinds.get(key.as_bytes()) == Some(&RenderParameterKind::Option)
                    })?;
                    let options = SerializedParameterBlock { flags: 0, parameters: parameters.into_iter()
                        .map(|(name, value)| SerializedParameter { name: name.into_bytes(), kind: RenderParameterKind::Option,
                            value: SerializedParameterValue::Words([value as u32, 0, 0, 0]) }).collect() };
                    DecalProgram::load(&catalog, &mut library, &bytes, &main, &options)
                })();
                match result {
                    Ok(program) => { kinds[program.kind as usize] += 1; fx += usize::from(program.use_fx); }
                    Err(error) => { println!("decal_material_error={name}: {error}"); failures += 1; }
                }
            }
            println!("declarations={count} distinct_materials={} projection_kinds={kinds:?} fx_materials={fx} errors={failures}", names.len());
        }
        "inspect-material-program" => {
            use dishonored2_rust::{
                material,
                render_program::{
                    RenderParameterKind, SerializedParameter, SerializedParameterBlock,
                    SerializedParameterValue, StageBytecode,
                },
                shader_cache::{ProgramLibrary, ShaderCacheBackend},
            };
            let name = args
                .first()
                .ok_or("inspect-material-program requires a material name")?;
            let output = Path::new(
                args.get(1)
                    .ok_or("inspect-material-program requires a local output directory")?,
            );
            std::fs::create_dir_all(output)?;
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let bytes = catalog
                .read(&format!("generated/decls/material/{name}.decl"))?
                .ok_or("material absent")?;
            let main = material::authored_program_override(&bytes, "mainProgram")
                .ok_or("material MAIN absent")?;
            let mut library = ProgramLibrary::open(base, ShaderCacheBackend::Pc, &catalog)?;
            let parameters = material::material_option_parameters(&bytes, |name| {
                library.parameter_kinds.get(name.as_bytes()) == Some(&RenderParameterKind::Option)
            })?;
            let mut options = SerializedParameterBlock {
                flags: 0,
                parameters: parameters
                    .into_iter()
                    .map(|(name, value)| SerializedParameter {
                        name: name.into_bytes(),
                        kind: RenderParameterKind::Option,
                        value: SerializedParameterValue::Words([value as u32, 0, 0, 0]),
                    })
                    .collect(),
            };
            let inputs = dishonored2_rust::rendering::decals::material::DecalProgram::load(
                &catalog, &mut library, &bytes, &main, &options,
            )?;
            println!("projection_inputs={inputs:?}");
            let state = library
                .definition(main.as_bytes())?
                .ok_or("program definition absent")?
                .render_state();
            let state = dishonored2_rust::render_state::material_render_state(
                &entities::declaration_text(&bytes),
                state,
            )?;
            let decal = library.option_value(main.as_bytes(), &options, b"isDecal")? != 0;
            let kind = library.registered_option_value(&options, b"decalType")? as u32;
            let sky = library.option_value(main.as_bytes(), &options, b"skyType")? as u32;
            let alpha = material::shadow_is_alpha(state[0], decal, kind, sky);
            let no_screen = [
                b"noScreenShadows".as_slice(),
                b"translucency_hq/enable",
                b"translucency/enable",
            ]
            .into_iter()
            .try_fold(alpha, |value, option| {
                library
                    .option_value(main.as_bytes(), &options, option)
                    .map(|n| value || n != 0)
            })?;
            for (name, value) in [
                ("useMultiInstancing", args.get(2).is_none_or(|bank| bank != "model")),
                ("isAlpha", alpha),
                ("noScreenShadows", no_screen),
            ] {
                options.parameters.push(SerializedParameter {
                    name: name.as_bytes().into(),
                    kind: RenderParameterKind::Option,
                    value: SerializedParameterValue::Words([u32::from(value), 0, 0, 0]),
                });
            }
            let variant = library
                .serialized_variant(main.as_bytes(), &options)?
                .ok_or("installed variant absent")?;
            println!("material={name} main={main} decal_type={kind} alpha={alpha} state={state:?}");
            for (index, stage) in variant.stages.iter().enumerate() {
                let bytes = match &stage.bytecode {
                    StageBytecode::Absent => continue,
                    StageBytecode::Inline(bytes) => bytes.clone(),
                    StageBytecode::Cached(id) => library.cache.stage(*id as usize)?.bytes,
                };
                println!(
                    "stage={index} kind={} layout={:?} bindings={:?}",
                    stage.kind, stage.layout, stage.bindings
                );
                std::fs::write(
                    output.join(format!("stage-{index}-{}.dxbc", stage.kind)),
                    bytes,
                )?;
            }
        }
        "inspect-ui-bitmap-ownership" => {
            #[cfg(feature = "game")]
            dishonored2_rust::app::inspect_ui_bitmap_ownership(base, args)?;
            #[cfg(not(feature = "game"))]
            return Err("inspect-ui-bitmap-ownership requires the game feature".into());
        }
        "inspect-ui-fonts" => {
            use dishonored2_rust::flash::{raster::Images, render, Player};
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let movie = args
                .get(0)
                .map(String::as_str)
                .unwrap_or("ui/corvo_pause_menu.iggy");
            for eager in [true, false] {
                let before = dishonored2_rust::platform::allocation_tags::report()
                    .into_iter()
                    .collect::<std::collections::HashMap<_, _>>();
                env::set_var(
                    "DH2_DEBUG_EAGER_FONT_OUTLINES",
                    if eager { "1" } else { "0" },
                );
                let (player, resources) = Player::load_installed(&catalog, "english", movie)?;
                let mut images = Images::default();
                for (library, path) in resources {
                    images.load(
                        &catalog,
                        library,
                        &path,
                        &player.state.borrow().file(library),
                    )?;
                }
                let registered = player
                    .state
                    .borrow()
                    .fonts
                    .iter()
                    .map(|font| font.glyphs.decoded_count())
                    .sum::<usize>();
                let mut cache = render::Cache::default();
                cache.set_font_caching(dishonored2_rust::gameplay::pause_state::font_caching(
                    &catalog,
                    movie,
                    player.state.borrow().stage_size,
                    (1366, 768),
                ));
                let mut digest = blake3::Hasher::new();
                let mut first = None;
                for _ in 0..200 {
                    let list = render::debug_draw_all_cached(&player.vm, &mut cache);
                    if first.is_none() {
                        let pixels = dishonored2_rust::flash::raster::rasterize(
                            &list, &images, 1366, 768, [0; 4],
                        );
                        digest.update(&pixels);
                        first = Some((
                            list.vertices.len(),
                            list.batches.len(),
                            list.glyph_pages
                                .iter()
                                .map(|page| page.coverage.len())
                                .sum::<usize>(),
                        ));
                    }
                }
                println!("eager={eager} registered_outlines={registered} decoded_outlines={} vertices_batches_page_bytes={first:?} pixel_hash={}",
                    player.state.borrow().fonts.iter().map(|font| font.glyphs.decoded_count()).sum::<usize>(), digest.finalize());
                for (tag, bytes) in dishonored2_rust::platform::allocation_tags::report()
                    .into_iter()
                    .filter(|(tag, _)| tag.starts_with("font "))
                {
                    println!(
                        "eager={eager} allocation_tag={tag:?} acquired_bytes={}",
                        bytes - before.get(tag).copied().unwrap_or(0)
                    );
                }
            }
            env::remove_var("DH2_DEBUG_EAGER_FONT_OUTLINES");
        }
        "inspect-fx-resources" | "inspect-fx-definitions" => {
            use dishonored2_rust::{
                assets::entities::effects,
                rendering::particles::{
                    definition::ParticleResources, world_sources::WorldFxSources,
                },
            };
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let named = command == "inspect-fx-definitions";
            if named && args.first().is_some_and(|argument| argument == "--decal-leases") {
                let mut programs = dishonored2_rust::shader_cache::ProgramLibrary::open(
                    base, dishonored2_rust::shader_cache::ShaderCacheBackend::Pc, &catalog,
                )?;
                return observe_decal_leases(&catalog, &mut programs, &args[1..]);
            }
            let (graphs, actors) = if named {
                if args.is_empty() { return Err("inspect-fx-definitions requires installed FX names".into()); }
                (Vec::new(), Vec::new())
            } else {
                let path = args.first().ok_or("inspect-fx-resources requires an entities resource path")?;
                let bytes = catalog.read(path)?.ok_or("entities resource absent")?;
                let document = entities::parse(&entities::declaration_text(&bytes))?;
                (effects::load(&document)?, effects::actors::load(&document, &catalog))
            };
            let mut programs = dishonored2_rust::shader_cache::ProgramLibrary::open(
                base,
                dishonored2_rust::shader_cache::ShaderCacheBackend::Pc,
                &catalog,
            )?;
            let mut resources = ParticleResources::default();
            resources.debug_set_sharing(args.get(1).is_none_or(|mode| mode != "unshared"));
            let started = std::time::Instant::now();
            let sources = if named {
                WorldFxSources::load_named_shared(args.to_vec(), &catalog, &mut programs, &mut resources)
            } else {
                WorldFxSources::load_shared(&graphs, &actors, &catalog, &mut programs, &mut resources)
            };
            let acquisition_elapsed = started.elapsed();
            if named {
                let mut definitions = sources.effects.iter().collect::<Vec<_>>();
                definitions.sort_by_key(|(name, _)| *name);
                for (name, definition) in definitions {
                    match definition {
                        Ok(definition) => println!("fx_definition name={name} groups={} particles={} sounds={} decals={}",
                            definition.groups.len(), definition.groups.iter().map(|group| group.particles.len()).sum::<usize>(),
                            definition.groups.iter().map(|group| group.sounds.len()).sum::<usize>(),
                            definition.groups.iter().map(|group| group.decals.len()).sum::<usize>()),
                        Err(error) => println!("fx_definition name={name} unavailable={error}"),
                    }
                }
            }
            let mut decal_effects = sources.effects.iter().filter_map(|(name, effect)|
                effect.as_ref().ok().filter(|effect| effect.groups.iter().any(|group| !group.decals.is_empty()))
                    .map(|effect| (name, effect))).collect::<Vec<_>>();
            decal_effects.sort_by_key(|(name, _)| *name);
            for (name, effect) in decal_effects {
                for decal in effect.groups.iter().flat_map(|group| &group.decals) {
                    match decal {
                        Ok(decal) => println!("decal effect={name} descriptor={:?} references={:?}", decal.asset.name, decal.asset.declarations),
                        Err(error) => println!("decal effect={name} unavailable={error}"),
                    }
                }
            }
            println!("acquisition_ms={} declaration_requests={} declaration_reads={} model_reads={} field_image_reads={} material_reads={} deferred_syntax_bytes={} deferred_admission_bytes={}",
                acquisition_elapsed.as_millis(), resources.declaration_requests, resources.declaration_reads,
                resources.model_reads, resources.field_image_reads, resources.material_reads,
                sources.deferred_syntax_bytes, sources.deferred_admission_bytes);
            let mut instances =
                dishonored2_rust::gameplay::effects::instances::FxGraphInstances::new(
                    &graphs,
                    &sources.declarations,
                    0,
                );
            let (instance_count, definitions) = instances.definition_counts();
            println!("graph_instances={instance_count} runtime_definitions={definitions} prepared_definitions={} effects={} failures={}",
                graphs.len() + sources.declarations.declarations.iter().filter(|graph| graph.is_ok()).count(),
                sources.effects.len(), sources.effects.values().filter(|effect| effect.is_err()).count());
            let graph_errors = instances.loading_end();
            let mut digest = blake3::Hasher::new();
            for &(instance, node, port) in &instances.events {
                digest.update(instances.owner(instance).as_bytes());
                digest.update(&node.to_le_bytes());
                digest.update(&port.to_le_bytes());
            }
            println!(
                "loading_events={} loading_errors={} event_hash={}",
                instances.events.len(),
                graph_errors.len(),
                digest.finalize()
            );
            drop(resources);
            for (tag, bytes) in dishonored2_rust::platform::allocation_tags::report()
                .into_iter()
                .filter(|(tag, _)| tag.starts_with("particle ") || tag.starts_with("FX "))
            {
                println!("allocation_tag={tag:?} live_bytes={bytes}");
            }
            let mut failures = sources
                .effects
                .iter()
                .filter_map(|(name, effect)| effect.as_ref().err().map(|error| (name, error)))
                .collect::<Vec<_>>();
            failures.sort();
            for (name, error) in failures {
                println!("unavailable {name}: {error}");
            }
        }
        "inspect-fonts" => {
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let paths = args
                .get(0)
                .map(|path| vec![path.clone()])
                .unwrap_or_else(|| {
                    catalog
                        .paths_containing("ui/lib_loc_")
                        .into_iter()
                        .filter(|path| path.ends_with("_font.iggy"))
                        .collect()
                });
            for path in paths {
                let file = std::rc::Rc::new(dishonored2_rust::assets::iggy::IggyFile::parse(
                    catalog
                        .read(&path)?
                        .ok_or_else(|| invalid(format!("missing {path}")))?,
                )?);
                let ids = (0..file.movie()?.character_count as usize)
                    .filter(|&id| file.font_metadata(id).ok().flatten().is_some())
                    .collect::<Vec<_>>();
                let rounds = 200;
                let started = std::time::Instant::now();
                for _ in 0..rounds {
                    for &id in &ids {
                        std::hint::black_box(file.font_metadata(id)?);
                    }
                }
                let metadata = started.elapsed();
                let started = std::time::Instant::now();
                for _ in 0..rounds {
                    for &id in &ids {
                        std::hint::black_box(file.font(id)?);
                    }
                }
                let eager = started.elapsed();
                let started = std::time::Instant::now();
                for _ in 0..rounds {
                    for &id in &ids {
                        std::hint::black_box(file.shared_font(id)?);
                    }
                }
                let lazy = started.elapsed();
                let mut glyphs = 0;
                let mut points = 0;
                let mut different = 0;
                for &id in &ids {
                    let eager = file.font(id)?.unwrap();
                    let lazy = file.shared_font(id)?.unwrap();
                    for index in 0..eager.glyphs.len() {
                        glyphs += 1;
                        let before = eager.glyphs.get(index);
                        let after = lazy.glyphs.get(index);
                        different += usize::from(before != after);
                        points += before
                            .into_iter()
                            .flat_map(|shape| &shape.layers)
                            .map(|layer| layer.points.capacity())
                            .sum::<usize>();
                    }
                }
                println!("{path}: fonts={} glyphs={glyphs} point_capacity_bytes={} outline_differences={different} rounds={rounds} metadata_us={} eager_us={} lazy_us={}",
                    ids.len(), points * std::mem::size_of::<dishonored2_rust::assets::iggy::shape::PathPoint>(),
                    metadata.as_micros(), eager.as_micros(), lazy.as_micros());
            }
        }
        _ => unreachable!("resource audit command"),
    }
    Ok(())
}
