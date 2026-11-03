//! Resource acquisition diagnostics over installed game assets. These commands
//! measure real owner costs and output equivalence; they do not run tests.
use dishonored2_rust::entities;
use std::{env, io, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn run(command: &str, base: &Path, args: &[String]) -> Result<()> {
    match command {
        "inspect-world-queries" => {
            let path = args.first().ok_or("inspect-world-queries requires a WORLD resource")?;
            let unshared = args.get(1).is_some_and(|value| value == "unshared");
            env::set_var("DH2_DEBUG_UNSHARED_WORLD_QUERIES", if unshared { "1" } else { "0" });
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let bytes = catalog.read(path)?.ok_or("WORLD resource absent")?;
            let world = dishonored2_rust::world_mesh::parse(&bytes)?;
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
        "inspect-fx-resources" => {
            use dishonored2_rust::{
                assets::entities::effects,
                rendering::particles::{
                    definition::ParticleResources, world_sources::WorldFxSources,
                },
            };
            let catalog = dishonored2_rust::archive::Catalog::open(base)?;
            let path = args
                .get(0)
                .ok_or("inspect-fx-resources requires an entities resource path")?;
            let bytes = catalog.read(path)?.ok_or("entities resource absent")?;
            let document = entities::parse(&entities::declaration_text(&bytes))?;
            let graphs = effects::load(&document)?;
            let actors = effects::actors::load(&document, &catalog);
            let mut programs = dishonored2_rust::shader_cache::ProgramLibrary::open(
                base,
                dishonored2_rust::shader_cache::ShaderCacheBackend::Pc,
                &catalog,
            )?;
            let mut resources = ParticleResources::default();
            resources.debug_set_sharing(args.get(1).is_none_or(|mode| mode != "unshared"));
            let started = std::time::Instant::now();
            let sources = WorldFxSources::load_shared(
                &graphs,
                &actors,
                &catalog,
                &mut programs,
                &mut resources,
            );
            println!("acquisition_ms={} declaration_requests={} declaration_reads={} model_reads={} field_image_reads={} material_reads={} deferred_syntax_bytes={} deferred_admission_bytes={}",
                started.elapsed().as_millis(), resources.declaration_requests, resources.declaration_reads,
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
                .filter(|(tag, _)| tag.starts_with("particle ") || tag.starts_with("FX graph "))
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
