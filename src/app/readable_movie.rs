//! The installed notes movie, presented by the shared Flash UI worker.
//!
//! `arkGameLogicStateReadNote::OnFlashLoaded` (1407905e0) writes these
//! three fields before calling `_C_ShowNote`. The caller supplies the
//! string-table keys and the authored note type.
use std::{collections::{HashMap, VecDeque}, sync::Arc};
use bevy::prelude::Resource;

use crate::{
    assets::{archive::Catalog, lang},
    flash::{Player, render},
    rendering::bevy_backend::flash_ui::FlashFrame,
};

pub(super) struct Request {
    pub title: String,
    pub text: String,
    pub note_type: i32,
}

/// Authored map request consumed by the shared notes-movie worker.
pub(super) struct MapRequest {
    pub title: String,
    pub image: String,
    pub legends: Vec<String>,
    pub map_type: i32,
}

/// Requests are drained by the Flash UI owner. `closed` counts completed
/// presentations, including their authored closing animation.
#[derive(Resource, Default)]
pub(super) struct ReadableRequests {
    pub open: VecDeque<Request>,
    pub map_open: VecDeque<MapRequest>,
    pub closed: u64,
}

pub(super) struct Frame {
    pub frame: FlashFrame,
    pub closed: bool,
}

pub(super) struct Movie {
    player: Player,
    images: Arc<HashMap<crate::flash::player::CharRef, Arc<crate::flash::bitmap_resources::Image>>>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
}

impl Movie {
    pub fn load(screen: (i32, i32), request: &Request, generation: u64) -> Result<Self, String> {
        let base = crate::assets::installation::game_base().map_err(|e| e.to_string())?;
        let catalog = Catalog::shared(&base).map_err(|e| e.to_string())?;
        let language = lang::read_text_language(&base)?;
        let resource = "ui/notes.iggy";
        let (mut player, resources) = Player::load_installed(&catalog, language, resource)?;
        crate::gameplay::pause_state::file_ready(&mut player, screen)?;
        let strings = player.state.borrow().strings.clone().ok_or("notes movie has no string table")?;
        let title = strings.lookup(&request.title);
        let text = strings.lookup(&request.text);
        let actions = crate::platform::bindings::bindable_actions(
            &crate::platform::console::shared().read().bindings,
            &strings,
            &mut crate::platform::bindings::Keyboard::default(),
        );
        crate::gameplay::pause_state::write_bindable_actions(&mut player, &actions)
            .map_err(|e| player.describe(&e))?;
        player.call_path_method("controllerTypeHandler", "_C_UpdateControllerType", &[1.into()])
            .map_err(|e| player.describe(&e))?;
        let configure = (|| -> crate::flash::avm2::Result<()> {
            player.set_value("notesData.title", title.into())?;
            player.set_value("notesData.text", text.into())?;
            player.set_value("notesData.type", request.note_type.into())?;
            player.call_root_method("_C_ShowNote", &[])?;
            Ok(())
        })();
        configure.map_err(|e| player.describe(&e))?;
        let images = Arc::new(super::pause_menu::library_images(&catalog, &player, &resources)?);
        let mut cache = render::Cache::default();
        let stage = player.state.borrow().stage_size;
        cache.set_font_caching(crate::gameplay::pause_state::font_caching(
            &catalog, resource, stage, (screen.0 as u32, screen.1 as u32),
        ));
        Ok(Self { player, images, cache, generation, serial: 0 })
    }

    pub fn frame(&mut self, dt: f64) -> Result<Frame, String> {
        self.player.tick(dt).map_err(|e| self.player.describe(&e))?;
        let closed = std::mem::take(&mut self.player.state.borrow_mut().notifications)
            .iter().any(|(name, _)| name == "OnNotesClosed");
        let list = render::draw_list_cached(&self.player.vm, &mut self.cache);
        self.serial += 1;
        Ok(Frame {
            frame: FlashFrame {
                list: Arc::new(list),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            },
            closed,
        })
    }

    pub fn key(&mut self, down: bool, vk: u32) -> Result<(), String> {
        self.player.key_event(down, vk, 0).map_err(|e| self.player.describe(&e))
    }

    pub fn mouse_button(&mut self, down: bool, button: u8) -> Result<(), String> {
        self.player.mouse_button(button, down).and_then(|_| {
            self.player.call_path_method("inputsManager.mouseHandler", "IG_OnMouseEvent", &[
                (0xe0u32 + button as u32).into(),
                (if down { 1.0 } else { 0.0 }).into(),
            ]).map(|_| ())
        }).map_err(|e| self.player.describe(&e))
    }

    pub fn mouse_wheel(&mut self, delta: i32) -> Result<(), String> {
        self.player.mouse_wheel(delta).map_err(|e| self.player.describe(&e))
    }
}
