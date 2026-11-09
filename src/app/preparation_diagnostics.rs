//! CPU acquisition observation through the actual client loading worker.
//! No window, renderer, input, audio playback or settings writes are created.
//! This observes prepared CPU resources, not GPU publication or live gameplay.
use crate::assets::{archive::Catalog, level::LevelResources};
use std::{path::Path, sync::Arc};

/// Run complete CPU loading batches and release their real prepared owners.
/// Each batch requests initial inventory; no saved inventory/gameplay state
/// is manufactured. Existing owner diagnostics supply identities and costs.
pub fn inspect_cpu_preparation(base: &Path, levels: &[String])
    -> Result<(), Box<dyn std::error::Error>>
{
    if levels.is_empty() { return Err("inspect-cpu-preparation requires level names".into()); }
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
            let mut prepared = super::level_prepare::prepare_level(
                base.to_owned(), catalog.clone(), level, true, || false,
            )?;
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
            // This is the same body/setup acquisition used by setup::prepare.
            // Rendering arms, GPU uploads and frame updates are not exercised.
            let mut declarations = crate::entities::player::DeclarationSource::default();
            let player = super::player::load_player_setup_with_source(
                &catalog, super::pause_menu::debug_character(), &mut declarations,
            )?;
            drop(player);
            drop(declarations);
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
        crate::edge_skeleton::release_unused();
        result?;
        eprintln!("RESOURCE_CPU_PREPARATION step={step} level={name} phase=released");
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
