//! The pause menu: the game's Iggy pause movie run by the rewrite's player
//! and drawn over the display output.
use crate::assets::{archive::Catalog, iggy::IggyFile, installation, lang};
use crate::flash::{raster, render, Player};
use crate::rendering::bevy_backend::flash_ui::{FlashFrame, FlashUiFrame, Image};
use bevy::prelude::*;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// The movie the game loads for Corvo (`ui/gamelogic/state_pause` decl).
const MOVIE: &str = "ui/corvo_pause_menu.iggy";

struct LoadedMenu {
    player: Player,
    images: Arc<HashMap<crate::flash::player::CharRef, Arc<Image>>>,
    generation: u64,
    /// Seconds not yet consumed by player ticks.
    pending: f64,
    frame_interval: f64,
}

/// Main-thread owner of the pause movie (the player is single-threaded).
#[derive(Default)]
pub struct PauseMenu {
    menu: Option<LoadedMenu>,
    open: bool,
    failed: Option<String>,
    generation: u64,
}

fn load(generation: u64) -> Result<LoadedMenu, String> {
    let started = std::time::Instant::now();
    let base = installation::game_base().map_err(|e| e.to_string())?;
    let catalog = Catalog::open(&base).map_err(|e| e.to_string())?;
    let language = installation::game_language(&base).map_err(|e| e.to_string())?;
    let read = |p: &str| -> Result<IggyFile, String> {
        let bytes = catalog.read(p).map_err(|e| e.to_string())?.ok_or_else(|| format!("{p} not indexed"))?;
        IggyFile::parse(bytes).map_err(|e| format!("{p}: {e}"))
    };
    let movie = read(MOVIE)?;
    let info = movie.movie().map_err(|e| e.to_string())?;
    let size = ((info.stage[2] - info.stage[0]) as f64, (info.stage[3] - info.stage[1]) as f64);
    let mut player = Player::new(size, info.frame_rate as f64).map_err(|e| format!("{e:?}"))?;
    player.state.borrow_mut().strings = Some(Rc::new(lang::StringTable::load(&catalog, &language)?));
    let mut resources = Vec::new();
    for import in &info.imports {
        let path = lang::library_resource(&import.library, &language);
        let index = player.load_library(read(&path)?).map_err(|e| format!("{path}: {}", player.describe(&e)))?;
        resources.push((index, path));
    }
    player.load_movie(movie).map_err(|e| format!("{MOVIE}: {}", player.describe(&e)))?;
    resources.push((player.state.borrow().libraries.len() - 1, MOVIE.to_string()));
    let mut decoded = raster::Images::default();
    for (index, path) in &resources {
        let file = player.state.borrow().file(*index);
        decoded.load(&catalog, *index, path, &file)?;
    }
    let images = decoded
        .images
        .into_iter()
        .map(|(c, (width, height, rgba))| (c, Arc::new(Image { width, height, rgba })))
        .collect();
    info!("Pause movie loaded in {:.2} s", started.elapsed().as_secs_f64());
    Ok(LoadedMenu { player, images: Arc::new(images), generation, pending: 0.0, frame_interval: 1.0 / info.frame_rate as f64 })
}

fn report_uncaught(player: &mut Player) {
    let uncaught = std::mem::take(&mut player.state.borrow_mut().uncaught);
    for e in uncaught {
        warn!("Pause movie: {e}");
    }
}

/// Escape toggles the pause menu; the menu is ticked at its frame rate and
/// its draw list published for the UI pass.
pub fn update_pause_menu(
    mut menu: NonSendMut<PauseMenu>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut frame: ResMut<FlashUiFrame>,
) {
    if keys.just_pressed(KeyCode::Escape) && menu.failed.is_none() {
        menu.open = !menu.open;
        if menu.open && menu.menu.is_none() {
            menu.generation += 1;
            match load(menu.generation) {
                Ok(loaded) => menu.menu = Some(loaded),
                Err(e) => {
                    error!("Pause menu unavailable: {e}");
                    menu.failed = Some(e);
                    menu.open = false;
                }
            }
        }
        if menu.open {
            if let Some(loaded) = menu.menu.as_mut() {
                // `FUN_1404f45a0`: the show call on the document instance.
                if let Err(e) = loaded.player.call_root_method("_C_ShowPauseMenu", &[]) {
                    warn!("Pause movie: _C_ShowPauseMenu: {}", loaded.player.describe(&e));
                }
                report_uncaught(&mut loaded.player);
            }
        }
    }
    if !menu.open {
        frame.0 = None;
        return;
    }
    let Some(loaded) = menu.menu.as_mut() else { return };
    // `IggyPlayerReadyToTick`/`FUN_141021b40`: one tick per frame interval.
    loaded.pending += time.delta_secs_f64();
    if loaded.pending >= loaded.frame_interval {
        let dt = loaded.pending.min(loaded.frame_interval);
        loaded.pending = (loaded.pending - loaded.frame_interval).max(0.0);
        if let Err(e) = loaded.player.tick(dt) {
            warn!("Pause movie tick: {}", loaded.player.describe(&e));
        }
        report_uncaught(&mut loaded.player);
    }
    // Diagnostic: DH2_DEBUG_FLASH_ALL=1 also draws hidden objects.
    let debug_all = std::env::var_os("DH2_DEBUG_FLASH_ALL").is_some_and(|v| v == "1");
    let list = if debug_all { render::debug_draw_all(&loaded.player.vm) } else { render::draw_list(&loaded.player.vm) };
    frame.0 = Some(FlashFrame {
        list: Arc::new(list),
        images: loaded.images.clone(),
        generation: loaded.generation,
    });
}
