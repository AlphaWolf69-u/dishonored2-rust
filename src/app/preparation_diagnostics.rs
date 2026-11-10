//! CPU acquisition observation through the actual client loading worker.
//! No window, renderer, input, audio playback or settings writes are created.
//! This observes prepared CPU resources, not GPU publication or live gameplay.
use crate::assets::{archive::Catalog, level::LevelResources};
use super::level_prepare::CancellationPoint;
use std::cell::Cell;
use std::{path::Path, sync::Arc};

/// Run complete CPU loading batches and release their real prepared owners.
/// Each batch requests initial inventory; no saved inventory/gameplay state
/// is manufactured. Existing owner diagnostics supply identities and costs.
pub fn inspect_cpu_preparation(base: &Path, levels: &[String])
    -> Result<(), Box<dyn std::error::Error>>
{
    if levels.is_empty() { return Err("inspect-cpu-preparation requires level names".into()); }
    let cancellation = match std::env::var("DH2_DEBUG_CPU_CANCEL_COLLISION").as_deref() {
        Ok("shapes") => Some(CancellationPoint::CollisionShapes),
        Ok("placed") => Some(CancellationPoint::PlacedCollision),
        Ok("index") => Some(CancellationPoint::CollisionIndex),
        Err(std::env::VarError::NotPresent) => None,
        _ => return Err("DH2_DEBUG_CPU_CANCEL_COLLISION expects shapes, placed, or index".into()),
    };
    // CPU readers use the same task pools as the client. No application
    // runner or window/render plugin is installed.
    let mut task_host = bevy::prelude::App::new();
    task_host.add_plugins(bevy::app::TaskPoolPlugin::default());
    let catalog = Arc::new(Catalog::open(base)?);
    let _audio_session = super::audio::AudioSession::begin()?;
    let mut engine_controls = None;
    for (step, name) in levels.iter().enumerate() {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let level = LevelResources::resolve(&catalog, name)?;
            if let Some(map) = level.entities.strip_suffix(".entities") {
                crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
            }
            eprintln!("RESOURCE_CPU_PREPARATION step={step} level={name} phase=begin");
            let checkpoints = Cell::new(0u32);
            let requested = Cell::new(None);
            let prepared = super::level_prepare::prepare_level(
                base.to_owned(), catalog.clone(), level, true, step == 0, |point| {
                    if requested.get().is_some() { return true; }
                    if cancellation != Some(point) { return false; }
                    checkpoints.set(checkpoints.get() + 1);
                    if checkpoints.get() < 64 { return false; }
                    requested.set(Some(std::time::Instant::now()));
                    true
                },
            );
            if let Some(point) = cancellation {
                let requested = requested.get().ok_or("collision cancellation point was not reached")?;
                let error = prepared.err().ok_or("cancelled collision preparation returned a world")?;
                let release_ms = requested.elapsed().as_secs_f64() * 1000.0;
                let error = error.to_string();
                if !error.ends_with("cancelled") { return Err(error.into()); }
                eprintln!("RESOURCE_COLLISION_CANCEL level={name} phase={point:?} checkpoints={} release_ms={release_ms:.3} result={error}",
                    checkpoints.get());
                return Ok(());
            }
            let mut prepared = prepared?;
            // AudioMix initializes these once per session after map preload.
            // Observe the same resource owner, without a device or audio output.
            if engine_controls.is_none()
                && std::env::var("DH2_DEBUG_CPU_AUDIO_CONTROLS").as_deref() == Ok("1") {
                let controls = crate::assets::wwise::engine_controls::EngineControls::acquire(&catalog)?;
                eprintln!("SOUND_ENGINE_CONTROLS values={controls:?}");
                engine_controls = Some(controls);
            }
            // Exercise the actual contact source owner too when measuring its
            // handoff, rather than just the earlier physics-material subset.
            let contact_texts = if std::env::var("DH2_DEBUG_CPU_CONTACT_TEXTS").as_deref() == Ok("1") {
                let seed = prepared.take_resource::<super::level_prepare::PreparedPhysicsTexts>()
                    .ok_or("prepared physics text seed is absent")?;
                let map = crate::gameplay::map_info::current_map().ok_or("current map is absent")?;
                let language = crate::assets::lang::read_text_language(base).map_err(std::io::Error::other)?;
                Some(crate::assets::declaration_texts::acquire(base, &catalog, &map, language, seed.0)
                    .map_err(std::io::Error::other)?)
            } else { None };
            if std::env::var("DH2_DEBUG_CPU_USABLE_ANIMATION_RESOURCES").as_deref() == Ok("1") {
                let interactions = prepared.get_resource::<super::interactions::Interactions>()
                    .ok_or("prepared interactions are absent")?;
                let scene = prepared.get_resource::<super::SceneData>().ok_or("prepared scene is absent")?;
                let root_count = scene.expanded_placement_roots.unwrap_or(scene.placed_models.len());
                super::usable_animation::observe_resources(&catalog, interactions, &scene.placed_models[..root_count]);
            }
            // This is the same body/setup acquisition used by setup::prepare.
            // Rendering arms, GPU uploads and frame updates are not exercised.
            let mut declarations = crate::entities::player::DeclarationSource::default();
            let player = super::player::load_player_setup_with_source(
                &catalog, super::pause_menu::debug_character(), &mut declarations,
            )?;
            drop(player);
            drop(declarations);
            if std::env::var("DH2_DEBUG_CPU_DIFFICULTY_RESOURCES").as_deref() == Ok("1") {
                let manager = super::player::load_difficulty(&catalog).ok_or("player difficulty acquisition failed")?;
                eprintln!("DIFFICULTY_PLAYER_INPUTS signature={}", blake3::hash(format!("{manager:?}").as_bytes()));
            }
            if std::env::var("DH2_DEBUG_CPU_OPTIONS_RESOURCES").as_deref() == Ok("1") {
                // Actual options acquisition without a movie, window, callbacks
                // or config writes. Preset initialization changes this copy only.
                let mut console = crate::platform::console::shared().read().clone();
                let options = crate::gameplay::options::read_with_resources(Some(&catalog), &mut |path| catalog.read(path).ok().flatten(), &mut console)?;
                let values: Vec<_> = console.cvars.list.iter().map(|value| (&value.name, &value.value)).collect();
                eprintln!("OPTIONS_RESOURCE_PREPARATION step={step} sections={} console_signature={}", options.sections.len(), blake3::hash(format!("{values:?}").as_bytes()));
                drop(options);
            }
            if std::env::var("DH2_DEBUG_CPU_PARTICLE_RESOURCES").as_deref() == Ok("1") {
                // The same already-prepared graphs and shader library feed
                // setup::prepare. No fabricated FX or renderer is introduced.
                let graphs = prepared.take_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectDefinitions>()
                    .ok_or("prepared effect definitions are absent")?;
                let programs = prepared.take_resource::<super::ProgramCacheSource>()
                    .ok_or("prepared program library is absent")?;
                let mut programs = programs.0.lock().map_err(|_| "program library is poisoned")?;
                let programs = programs.as_mut().map_err(|error| std::io::Error::other(error.clone()))?;
                let mut resources = crate::rendering::particles::definition::ParticleResources::default();
                let sources = crate::rendering::particles::world_sources::WorldFxSources::load_shared(
                    &graphs.graphs, &graphs.actors, &catalog, programs, &mut resources,
                );
                let mut instances = crate::gameplay::effects::instances::FxGraphInstances::new(
                    &graphs.graphs, &sources.declarations, 0,
                );
                let errors = instances.loading_end();
                let mut digest = blake3::Hasher::new();
                for &(instance, node, port) in &instances.events {
                    digest.update(instances.owner(instance).as_bytes());
                    digest.update(&node.to_le_bytes()); digest.update(&port.to_le_bytes());
                }
                eprintln!("PARTICLE_CPU_PREPARATION step={step} level={name} declaration_requests={} declaration_reads={} model_reads={} field_reads={} effects={} failures={} events={} graph_errors={} event_hash={}",
                    resources.declaration_requests, resources.declaration_reads, resources.model_reads, resources.field_image_reads,
                    sources.effects.len(), sources.effects.values().filter(|effect| effect.is_err()).count(),
                    instances.events.len(), errors.len(), digest.finalize());
                resources.retire_unused();
            }
            drop(prepared);
            if let Some(texts) = contact_texts { texts.release_level_history(); }
            Ok(())
        })();
        // No renderer owns these unpublished CPU batches. Their consumers
        // above have already dropped, so use the live client's same purge.
        catalog.release_level_resources();
        crate::assets::render_parameter::report(&catalog, "level-released");
        result?;
        eprintln!("RESOURCE_CPU_PREPARATION step={step} level={name} phase=released");
        if let Some((labels, bytes)) = crate::platform::allocation_tags::phase_label_storage() {
            eprintln!("AUDIT_PHASE_LABELS step={step} labels={labels} payload_bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Blueprint preparation") {
            eprintln!("BLUEPRINT_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Motion scene resources") {
            eprintln!("MOTION_SCENE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        crate::assets::declaration_texts::report_text_handoff("level-released");
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("inventory-item-owner") {
            eprintln!("PICKABLE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("particle definitions") {
            eprintln!("PARTICLE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("FX declaration sources") {
            eprintln!("FX_SOURCE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Mass property resources") {
            eprintln!("MASS_RESOURCE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Sound event declarations") {
            eprintln!("SOUND_EVENT_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("FX graph definitions") {
            eprintln!("FX_GRAPH_MEMORY phase=level-released step={step} bytes={bytes}");
        }
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("shader cache index") {
            eprintln!("SHADER_INDEX_MEMORY phase=level-released step={step} bytes={bytes}");
        }
    }
    drop(catalog);
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Light declaration sources") {
        eprintln!("LIGHT_SOURCE_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Difficulty declarations") {
        eprintln!("DIFFICULTY_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("AnimBasic item definitions") {
        eprintln!("ANIMBASIC_ITEM_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Option section resources") {
        eprintln!("OPTIONS_RESOURCE_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Blueprint preparation") {
        eprintln!("BLUEPRINT_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Motion scene resources") {
        eprintln!("MOTION_SCENE_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Render parameter resources") {
        eprintln!("RENDER_PARAMETER_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Mass property resources") {
        eprintln!("MASS_RESOURCE_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("Sound event declarations") {
        eprintln!("SOUND_EVENT_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("FX graph definitions") {
        eprintln!("FX_GRAPH_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("shader cache index") {
        eprintln!("SHADER_INDEX_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("particle definitions") {
        eprintln!("PARTICLE_MEMORY phase=catalog-released bytes={bytes}");
    }
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("inventory-item-owner") {
        eprintln!("PICKABLE_MEMORY phase=catalog-released bytes={bytes}");
    }
    Ok(())
}


/// Acquire installed bitmap packs through the movie worker's owner and
/// observe their leases without advancing a player or rendering a frame.
pub fn inspect_ui_bitmap_ownership(base: &Path, movies: &[String])
    -> Result<(), Box<dyn std::error::Error>>
{
    use crate::flash::{bitmap_resources::{MovieBitmaps, PackLifetime}, Player};
    let loading_first = movies.first().is_some_and(|arg| arg == "--loading-first");
    let movies = &movies[usize::from(loading_first)..];
    if movies.is_empty() { return Err("provide at least one installed Iggy movie path".into()); }
    let mut task_host = bevy::prelude::App::new();
    task_host.add_plugins(bevy::app::TaskPoolPlugin::default());
    let catalog = crate::assets::archive::Catalog::shared(base)?;
    let language = crate::assets::lang::read_text_language(base)?;
    let acquire = |movie: &str, phase: &str, lifetime| -> Result<Arc<MovieBitmaps>, String> {
        let (player, resources) = Player::load_installed(&catalog, language, movie)?;
        let started = std::time::Instant::now();
        let cycles = crate::loading::thread_cycles();
        let images = super::pause_menu::library_images(&catalog, &player, &resources, lifetime)?;
        let elapsed = started.elapsed();
        let cycles = cycles.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        let mut members: Vec<_> = images.iter().collect();
        members.sort_unstable_by_key(|(id, _)| (id.library, id.id));
        let mut crc = flate2::Crc::new();
        let mut bytes = 0;
        for (id, image) in &members {
            crc.update(&id.library.to_le_bytes());
            crc.update(&id.id.to_le_bytes());
            crc.update(&image.width.to_le_bytes());
            crc.update(&image.height.to_le_bytes());
            crc.update(&image.mip_count.to_le_bytes());
            crc.update(format!("{:?}", image.format).as_bytes());
            crc.update(&image.bytes);
            bytes += image.bytes.len();
        }
        println!("UI_BITMAP_OWNERSHIP phase={phase} movie={movie} images={} bytes={bytes} signature={:08x} acquisition_ms={:.3} acquisition_cycles={cycles}",
            members.len(), crc.sum(), elapsed.as_secs_f64() * 1000.0);
        Ok(Arc::new(images))
    };
    let mut active = Vec::new();
    for (index, movie) in movies.iter().enumerate() {
        let lifetime = if loading_first && index == 0 { PackLifetime::Movie } else { PackLifetime::Session };
        active.push(acquire(movie, "opened", lifetime)?);
    }
    if loading_first && active.len() > 1 {
        let loading_ids: std::collections::HashSet<_> = active[0].values().map(Arc::as_ptr).collect();
        let reused = active[1].values().filter(|image| loading_ids.contains(&Arc::as_ptr(image))).count();
        println!("UI_BITMAP_OWNERSHIP loading_transfer_images={reused}");
    }
    let movie = movies.last().unwrap();
    let lifetime = if loading_first && active.len() == 1 { PackLifetime::Movie } else { PackLifetime::Session };
    let mut overlapping = acquire(movie, "overlapping", lifetime)?;
    let previous = active.last().unwrap();
    let reused = overlapping.iter().filter(|(id, image)| {
        previous.get(*id).is_some_and(|other| Arc::ptr_eq(image, other))
    }).count();
    println!("UI_BITMAP_OWNERSHIP overlapping_reuses={reused} overlapping_images={}", overlapping.len());
    if active.len() > 1 {
        drop(overlapping);
        drop(active.pop());
        overlapping = acquire(movie, "reopened-with-earlier-maps", lifetime)?;
        active.push(overlapping.clone());
    }
    let mut observed = std::collections::HashMap::new();
    for images in &active {
        for image in images.values() {
            observed.entry(Arc::as_ptr(image)).or_insert_with(|| Arc::downgrade(image));
        }
    }
    let report = |phase| {
        let bytes: usize = observed.values().filter_map(std::sync::Weak::upgrade)
            .map(|image| image.bytes.len()).sum();
        println!("UI_BITMAP_OWNERSHIP phase={phase} retained_payload_bytes={bytes}");
    };
    if loading_first && active.len() > 1 {
        drop(active.remove(0));
        report("loading-map-released");
    }
    drop(active);
    report("final-map-held");
    drop(overlapping);
    report("all-maps-released");
    drop(acquire(movie, "reopened", lifetime)?);
    if loading_first && movies.len() > 1 {
        drop(acquire(&movies[0], "loading-reopened", PackLifetime::Movie)?);
    }
    Ok(())
}
