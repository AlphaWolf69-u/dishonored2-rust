//! Installed loading movie contract: 1400473b0 writes loadingData;
//! 1400479a0 invokes _C_Show with m_layoutType. The movie owns its layout,
//! tips and animations; the host supplies actual acquisition progress.
use crate::rendering::bevy_backend::flash_ui::FlashFrame;
use crate::{
    assets::{archive::Catalog, lang},
    flash::{Player, avm2::value::Value, render},
    gameplay::options::decl::{atom, boolean, int, list_items, scope},
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct Frame {
    pub frame: FlashFrame,
    pub continued: bool,
    pub ready: bool,
}

pub(super) struct Movie {
    player: Player,
    images: Arc<HashMap<crate::flash::player::CharRef, Arc<crate::flash::bitmap_resources::Image>>>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    wait: bool,
    completed: bool,
    continued: bool,
    reported: bool,
}
impl Movie {
    pub fn load(screen: (i32, i32), map: &str, generation: u64) -> Result<Self, String> {
        let base = crate::assets::installation::game_base().map_err(|e| e.to_string())?;
        let catalog = Catalog::shared(&base).map_err(|e| e.to_string())?;
        let edit = crate::gameplay::map_info::find(&catalog, map)
            .ok_or_else(|| format!("no mapinfo for {map}"))?;
        let info = scope(&edit, "m_loadingInfo").ok_or("mapinfo has no loading presentation")?;
        let resource = atom(info, "m_iggyFile")
            .ok_or("loading presentation has no movie")?
            .replace(".iggyfile", ".iggy");
        let language = lang::read_text_language(&base)?;
        let (mut player, resources) = Player::load_installed(&catalog, language, &resource)?;
        crate::gameplay::pause_state::file_ready(&mut player, screen)?;
        let strings = player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("loading movie has no string table")?;
        let translate = |fields: &[(String, crate::entities::Value)], key| {
            strings.lookup(atom(fields, key).unwrap_or("")).to_owned()
        };
        let title = format!(
            "{} - {}",
            int(info, "m_missionIndex", -1),
            translate(info, "m_missionName")
        );
        let layout = match atom(info, "m_layoutType").unwrap_or("ARK_UI_LOADSCREEN_TYPE_DEFAULT") {
            "ARK_UI_LOADSCREEN_TYPE_DEFAULT" | "0" => 0,
            "ARK_UI_LOADSCREEN_TYPE_NO_TIPS" | "1" => 1,
            "ARK_UI_LOADSCREEN_TYPE_TRAINING" | "2" => 2,
            other => return Err(format!("unrecognized installed loading layout: {other}")),
        };
        // idDeclMapInfo constructor 140021e60 initializes +0x250 to true.
        let wait = boolean(info, "m_waitInteractionAtEnd", true);
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
            player.set_value("loadingData.missionName", title.as_str().into())?;
            player.set_value(
                "loadingData.mapName",
                translate(&edit, "prettyMapName").as_str().into(),
            )?;
            player.set_value(
                "loadingData.missionSummary",
                translate(info, "m_missionSummary").as_str().into(),
            )?;
            player.set_value(
                "loadingData.mapImage",
                atom(info, "m_image").unwrap_or("").into(),
            )?;
            let mut tips = scope(info, "m_tips").map(list_items).unwrap_or_default();
            // 1400473b0: Fisher-Yates with the engine's LCG. Host clock
            // supplies the timing-dependent seed; no fixed tip selection.
            let mut seed = crate::platform::performance_counter::counter()
                .map_err(crate::flash::avm2::Error::Internal)? as u32;
            for remaining in (2..=tips.len()).rev() {
                seed = seed.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
                let selected = ((seed >> 10) & 0x7fff) as usize % remaining;
                tips.swap(remaining - 1, selected);
            }
            player.set_value("loadingData.tips.length", (tips.len() as u32).into())?;
            for (index, tip) in tips.iter().enumerate() {
                let crate::entities::Value::Scope(fields) = tip else {
                    continue;
                };
                let path = format!("loadingData.tips.data[{index}]");
                player.set_value(
                    &format!("{path}.label"),
                    translate(fields, "m_title").as_str().into(),
                )?;
                player.set_value(
                    &format!("{path}.content"),
                    translate(fields, "m_description").as_str().into(),
                )?;
            }
            player.call_root_method("_C_Show", &[Value::from(layout)])?;
            Ok(())
        })();
        configure.map_err(|e| player.describe(&e))?;
        let images = Arc::new(super::pause_menu::library_images(
            &catalog, &player, &resources,
        )?);
        let mut cache = render::Cache::default();
        let stage = player.state.borrow().stage_size;
        cache.set_font_caching(crate::gameplay::pause_state::font_caching(
            &catalog,
            &resource,
            stage,
            (screen.0 as u32, screen.1 as u32),
        ));
        Ok(Self {
            player,
            images,
            cache,
            generation,
            serial: 0,
            wait,
            completed: false,
            continued: false,
            reported: false,
        })
    }
    pub fn frame(&mut self, dt: f64, progress: f64, ready: bool) -> Result<Frame, String> {
        if ready && !self.completed {
            if self.wait {
                self.player
                    .call_root_method("_C_LoadingCompleted", &[])
                    .map_err(|e| self.player.describe(&e))?;
            } else {
                self.continued = true;
            }
            self.completed = true;
        }
        self.player
            .call_root_method(
                "_C_UpdateLoadingCompletion",
                &[if ready { 1.0.into() } else { progress.into() }],
            )
            .map_err(|e| self.player.describe(&e))?;
        self.player.tick(dt).map_err(|e| self.player.describe(&e))?;
        for (notification, _) in std::mem::take(&mut self.player.state.borrow_mut().notifications) {
            if notification == "OnLoadingDisabled" {
                self.continued = true;
            }
        }
        let list = render::draw_list_cached(&self.player.vm, &mut self.cache);
        if !self.reported && !list.batches.is_empty() {
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                let game_images: Vec<_> = list
                    .batches
                    .iter()
                    .filter_map(|batch| match &batch.texture {
                        render::Texture::Game(name) => Some(name.as_str()),
                        _ => None,
                    })
                    .collect();
                eprintln!(
                    "LOAD movie frame: stage={:?} batches={} vertices={} game_images={game_images:?}",
                    list.stage,
                    list.batches.len(),
                    list.vertices.len()
                );
                for error in &self.player.state.borrow().uncaught {
                    eprintln!("LOAD movie AS3 exception: {error}");
                }
            }
            self.reported = true;
        }
        self.serial += 1;
        Ok(Frame {
            ready: self.completed,
            continued: self.continued,
            frame: FlashFrame {
                list: Arc::new(list),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            },
        })
    }
}

impl Movie {
    pub fn key(&mut self, down: bool, vk: u32) -> Result<(), String> {
        self.player
            .key_event(down, vk, 0)
            .map_err(|e| self.player.describe(&e))
    }
    pub fn mouse_button(&mut self, down: bool, button: u8) -> Result<(), String> {
        self.player
            .mouse_button(button, down)
            .map_err(|e| self.player.describe(&e))
    }
}
