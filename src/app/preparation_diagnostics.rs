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
    for (step, name) in levels.iter().enumerate() {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let level = LevelResources::resolve(&catalog, name)?;
            if let Some(map) = level.entities.strip_suffix(".entities") {
                crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
            }
            eprintln!("RESOURCE_CPU_PREPARATION step={step} level={name} phase=begin");
            let prepared = super::level_prepare::prepare_level(
                base.to_owned(), catalog.clone(), level, true, || false,
            )?;
            // This is the same body/setup acquisition used by setup::prepare.
            // Rendering arms, GPU uploads and frame updates are not exercised.
            let mut declarations = crate::entities::player::DeclarationSource::default();
            let player = super::player::load_player_setup_with_source(
                &catalog, super::pause_menu::debug_character(), &mut declarations,
            )?;
            drop(player);
            drop(declarations);
            drop(prepared);
            Ok(())
        })();
        // No renderer owns these unpublished CPU batches. Their consumers
        // above have already dropped, so use the live client's same purge.
        catalog.release_level_resources();
        crate::edge_skeleton::release_unused();
        result?;
        eprintln!("RESOURCE_CPU_PREPARATION step={step} level={name} phase=released");
        if let Some(bytes) = crate::platform::allocation_tags::live_bytes("inventory-item-owner") {
            eprintln!("PICKABLE_MEMORY phase=level-released step={step} bytes={bytes}");
        }
    }
    drop(catalog);
    if let Some(bytes) = crate::platform::allocation_tags::live_bytes("inventory-item-owner") {
        eprintln!("PICKABLE_MEMORY phase=catalog-released bytes={bytes}");
    }
    Ok(())
}
