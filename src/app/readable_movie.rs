//! The installed notes movie, presented by the shared Flash UI worker.
//!
//! `arkGameLogicStateReadNote::OnFlashLoaded` (1407905e0) writes these
//! three fields before calling `_C_ShowNote`. The caller supplies the
//! string-table keys and the authored note type.
use bevy::prelude::Resource;
use std::{
    collections::VecDeque,
    sync::Arc,
};

use crate::{
    assets::{archive::Catalog, lang},
    flash::{Player, render},
    rendering::bevy_backend::flash_ui::FlashFrame,
};

use crate::gameplay::player::read_note::Presentation;

/// Requests are drained by the Flash UI owner. `closed` counts presentations
/// completed (including their closing animation) or rejected on load failure.
#[derive(Resource, Default)]
pub(super) struct ReadableRequests {
    pub open: VecDeque<Arc<Presentation>>,
    pub closed: u64,
    pub failed: u64,
}

pub(super) struct Frame {
    pub frame: Option<FlashFrame>,
    pub closed: bool,
    pub sounds: Vec<u32>,
    pub screen_fx: Option<bool>,
}

pub(super) struct Movie {
    player: Player,
    images: Arc<crate::flash::bitmap_resources::MovieBitmaps>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    accumulated: f64,
    interval: f64,
    last: Option<FlashFrame>,
    focused: bool,
    _residency: Option<super::pause_menu::MovieResidency>,
}

impl Movie {
    pub fn load(screen: (i32, i32), request: &Presentation, generation: u64, state_path: &str) -> Result<Self, String> {
        let base = crate::assets::installation::game_base().map_err(|e| e.to_string())?;
        let catalog = Catalog::shared(&base).map_err(|e| e.to_string())?;
        let language = lang::read_text_language(&base)?;
        let resource = super::pause_menu::state_movie(
            &catalog,
            "gamelogicstatereadnote",
            state_path,
        )?;
        let (mut player, resources) = Player::load_installed(&catalog, language, &resource)?;
        crate::gameplay::pause_state::file_ready(&mut player, screen)?;
        let strings = player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("notes movie has no string table")?;
        let actions = crate::platform::bindings::bindable_actions(
            &crate::platform::console::shared().read().bindings,
            &strings,
            &mut crate::platform::bindings::Keyboard::default(),
        );
        crate::gameplay::pause_state::write_bindable_actions(&mut player, &actions)
            .map_err(|e| player.describe(&e))?;
        player
            .call_path_method(
                "controllerTypeHandler",
                "_C_UpdateControllerType",
                &[1.into()],
            )
            .map_err(|e| player.describe(&e))?;
        let configure = (|| -> crate::flash::avm2::Result<()> {
            match request {
                Presentation::Note { title, text, note_type } => {
                    player.set_value("notesData.title", strings.lookup(title).into())?;
                    player.set_value("notesData.text", strings.lookup(text).into())?;
                    player.set_value("notesData.type", (*note_type).into())?;
                    player.call_root_method("_C_ShowNote", &[])?;
                }
                Presentation::Map { title, image, legends, map_type } => {
                    let legends = if legends.is_empty() {
                        vec!["INCORRECT LEGEND SETUP".into()]
                    } else {
                        legends.iter().map(|key| strings.lookup(key).into()).collect()
                    };
                    let legends = player.array(legends);
                    player.call_root_method("_C_ShowMap", &[
                        strings.lookup(title).into(), image.as_str().into(),
                        legends.into(), false.into(), (*map_type).into(),
                    ])?;
                }
            }
            Ok(())
        })();
        configure.map_err(|e| player.describe(&e))?;
        let image_map = super::pause_menu::library_images(&catalog, &player, &resources)?;
        let residency =
            super::pause_menu::MovieResidency::new("readable", &catalog, &resources, &image_map);
        let images = Arc::new(image_map);
        let mut cache = render::Cache::default();
        let stage = player.state.borrow().stage_size;
        cache.set_font_caching(crate::gameplay::pause_state::font_caching(
            &catalog,
            &resource,
            stage,
            (screen.0 as u32, screen.1 as u32),
        ));
        let interval = 1.0 / player.state.borrow().frame_rate;
        super::pause_menu::report_uncaught(&mut player, "Readable");
        Ok(Self {
            player,
            images,
            cache,
            generation,
            serial: 0,
            accumulated: 0.0,
            interval,
            last: None,
            focused: false,
            _residency: residency,
        })
    }

    pub(super) fn report_uncaught(&mut self) {
        super::pause_menu::report_uncaught(&mut self.player, "Readable");
    }

    pub fn interval(&self) -> f64 {
        self.interval
    }

    pub fn frame(&mut self, dt: f64, force: bool) -> Result<Frame, String> {
        self.accumulated += dt;
        let tick = self.accumulated >= self.interval;
        let changed = self.last.is_none() || tick;
        if tick {
            let step = self.accumulated.min(self.interval);
            self.accumulated = (self.accumulated - self.interval).max(0.0);
            self.player
                .tick(step)
                .map_err(|e| self.player.describe(&e))?;
        }
        let mut closed = false;
        let mut sounds = Vec::new();
        let mut screen_fx = None;
        for (name, args) in self.player.take_notifications() {
            match name.as_str() {
                "OnNotesClosed" => closed = true,
                // 790a00 binds NotesClosing to 791370: remove UI post FX
                // during the authored close animation, before OnNotesClosed.
                "NotesClosing" => screen_fx = Some(false),
                // Common arkIggyFile focus callbacks 035020 / 034fe0.
                "SetFlashFocus" => {
                    self.player.call_path_method("controllerTypeHandler", "_C_UpdateControllerType",
                        &[(crate::gameplay::pause_state::controller_type() as f64).into()])
                        .map_err(|e| self.player.describe(&e))?;
                    self.focused = true;
                }
                "KillFlashFocus" => self.focused = false,
                "PostSoundEvent" => {
                    if let Some(crate::flash::avm2::value::Value::Number(id)) = args.first() {
                        sounds.push(crate::flash::avm2::value::to_uint32(*id));
                    }
                }
                _ => super::pause_menu::report_unknown_callback("Readable", &name),
            }
        }
        if changed {
            self.serial += 1;
            self.last = Some(FlashFrame {
                list: Arc::new(render::draw_list_cached(&self.player.vm, &mut self.cache)),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            });
        }
        self.report_uncaught();
        Ok(Frame {
            frame: if changed || force {
                self.last.clone()
            } else {
                None
            },
            closed,
            sounds,
            screen_fx,
        })
    }

    pub fn key(&mut self, down: bool, vk: u32) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .key_event(down, vk, 0)
            .map_err(|e| self.player.describe(&e))
    }

    pub fn cursor(&mut self, x: f64, y: f64) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .set_value("mouseCursor.x", x.into())
            .and_then(|_| self.player.set_value("mouseCursor.y", y.into()))
            .and_then(|_| self.player.mouse_move(x, y))
            .map_err(|e| self.player.describe(&e))
    }

    pub fn mouse_button(&mut self, down: bool, button: u8) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        let result = if button < 3 {
            self.player.mouse_button(button, down)
        } else {
            Ok(())
        };
        result
            .and_then(|_| {
                self.player
                    .call_path_method(
                        "inputsManager.mouseHandler",
                        "IG_OnMouseEvent",
                        &[
                            (0xe0u32 + button as u32).into(),
                            (if down { 1.0 } else { 0.0 }).into(),
                        ],
                    )
                    .map(|_| ())
            })
            .map_err(|e| self.player.describe(&e))
    }

    pub fn mouse_wheel(&mut self, delta: i32) -> Result<(), String> {
        if !self.focused { return Ok(()); }
        self.last = None;
        self.player
            .mouse_wheel(delta)
            .map_err(|e| self.player.describe(&e))
    }
}
