//! Installed in-game HUD movie and its draw-list ownership.
//!
//! `arkGameLogicStateInGame::OnFlashLoaded` (14078b550) binds the HUD
//! elements before the root state supplies the input bindings.  The movie is
//! kept alive for the in-game state, unlike the per-open journal movie.
use crate::assets::archive::Catalog;
use crate::flash::{Player, player::CharRef, render};
use crate::gameplay::pause_state;
use crate::rendering::bevy_backend::flash_ui::{FlashFrame, Image};
use std::{collections::HashMap, sync::Arc};

pub(super) struct HudMovie {
    player: Player,
    images: Arc<HashMap<CharRef, Arc<Image>>>,
    cache: render::Cache,
    generation: u64,
    serial: u64,
    accumulated: f64,
    timer_elapsed: f64,
    interval: f64,
    last: Option<FlashFrame>,
    settings_generation: u64,
    interaction_type: Option<i32>,
    current_interaction: Option<Interaction>,
    oxygen_input: Option<Oxygen>,
    oxygen_displayed: Option<f32>,
    mana_low_level: Option<f32>,
    starting_powers: Option<Vec<String>>,
    inactivity_delay: Option<f64>,
    inactivity_elapsed: f64,
    inactivity_admitted: bool,
    hud_activity: bool,
    master_visible: Option<bool>,
    timing_since: Option<std::time::Instant>,
    timing_ticks: u64,
    timing_lists: u64,
    timing_tick_cost: std::time::Duration,
    timing_list_cost: std::time::Duration,
    _residency: Option<super::pause_menu::MovieResidency>,
}

/// The data assembled by arkHUDElementInteractives (14038f7d0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Interaction {
    pub entity: String,
    pub kind: i32,
    pub action_label: String,
    pub game_action: String,
    pub name: String,
    pub name_postfix: String,
    pub hold: bool,
    pub locked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Oxygen {
    pub underwater: bool,
    pub breath: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Health {
    pub current: f32,
    pub maximum: f32,
    pub projected: f32,
    pub low: bool,
    pub extensions: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mana {
    pub current: f32,
    pub maximum: f32,
    pub regen_target: f32,
    pub extension: bool,
    pub suppressed: bool,
    pub lock_flags: u8,
    pub has_mark: bool,
}

fn mana_config(catalog: &Catalog) -> Result<(f32, Vec<String>, f64), String> {
    use crate::gameplay::options::decl::{atom, list_items, scope, unquote};
    use crate::assets::entities::Value;
    let state = crate::assets::entities::player::resolved_decl("gamelogicstateingame", "ui/gamelogic/state_ingame.gamelogicstateingame", |path| {
        Ok(catalog.read(path).map_err(|error| error.to_string())?
            .map(|bytes| crate::assets::entities::declaration_text(&bytes).into_owned()))
    })?;
    // arkDeclGameLogicStateInGame constructor 14078abd0 initializes the
    // duration at +0x130 to zero before the declaration is applied.
    let inactivity = match scope(&state, "m_masterHUDInactivityDelay")
        .and_then(|duration| atom(duration, "m_seconds")) {
        Some(value) => value.parse::<f64>().map_err(|e| format!("invalid HUD inactivity delay: {e}"))?,
        None => 0.0,
    };
    let hud_elements = scope(&state, "m_hudElements").ok_or("in-game state has no HUD elements")?;
    let mana_name = atom(hud_elements, "enumItem[ARK_HUD_ELEMENT_MANA]").ok_or("in-game state has no mana HUD")?;
    let mana = crate::assets::entities::player::resolved_decl("ui_hudelementmana", mana_name, |path| {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::assets::entities::declaration_text(&bytes).into_owned()))
    })?;
    // 140674be0 initializes this to 0.5; the live declaration agrees.
    let low = match atom(&mana, "m_lowLevel") {
        Some(value) => value.trim_end_matches('f').parse::<f32>().map_err(|e| format!("invalid mana HUD threshold: {e}"))?,
        None => 0.5,
    };
    let entity = format!("models/characters/player/{}/body.def", super::player::character_name(super::pause_menu::debug_character()));
    let component = crate::assets::entities::player::entity_component(&entity, "cpntplayeroutsidermark", |path| {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| crate::assets::entities::declaration_text(&bytes).into_owned()))
    })?.ok_or("player has no outsider-mark component")?;
    let powers = scope(&component, "m_startingPowers").map(|list| list_items(list).into_iter().filter_map(|v| match v {
        Value::Atom(path) => Some(unquote(path).to_owned()), _ => None,
    }).collect()).unwrap_or_default();
    Ok((low, powers, inactivity))
}

impl HudMovie {
    fn settings(player: &mut Player) -> Result<(), String> {
        let strings = player
            .state
            .borrow()
            .strings
            .clone()
            .ok_or("HUD has no string table")?;
        let console = crate::platform::console::shared().read();
        let actions = crate::platform::bindings::bindable_actions(
            &console.bindings,
            &strings,
            &mut crate::platform::bindings::Keyboard::default(),
        );
        let style = console
            .cvars
            .find("ark_ui_crosshair_style")
            .map_or(2, |i| console.cvars.list[i].int);
        let opacity = console
            .cvars
            .find("ark_ui_crosshair_opacity")
            .map_or(1.0, |i| console.cvars.list[i].float);
        let master = console
            .cvars
            .find("ark_ui_show_master_hud")
            .map_or(2, |i| console.cvars.list[i].int);
        drop(console);
        pause_state::write_bindable_actions(player, &actions).map_err(|e| player.describe(&e))?;
        // 140672f70, with no equipped item or ability components. Style 0
        // suppresses both shapes, 1 retains the dot, 2 uses the normal HUD.
        let dot = if style == 0 { -1 } else { 0 };
        let crosshair = if style < 2 { -1 } else { 0 };
        player
            .call_path_method("_crosshair", "_C_SetDot", &[dot.into()])
            .map_err(|e| player.describe(&e))?;
        player
            .call_path_method(
                "_crosshair",
                "_C_SetCrosshair",
                &[
                    crosshair.into(),
                    (-1i32).into(),
                    (-1.0).into(),
                    0.0.into(),
                    0.0.into(),
                ],
            )
            .map_err(|e| player.describe(&e))?;
        player
            .set_value("_crosshair.alpha", (opacity as f64).into())
            .map_err(|e| player.describe(&e))?;
        let method = if master == 0 { "_C_Hide" } else { "_C_Show" };
        player
            .call_path_method("_masterHUD", method, &[])
            .map_err(|e| player.describe(&e))?;
        Ok(())
    }

    pub(super) fn load(
        catalog: &Catalog,
        language: &str,
        screen: (i32, i32),
        generation: u64,
    ) -> Result<Self, String> {
        let (mana_low_level, starting_powers, inactivity_delay) = match mana_config(catalog) {
            Ok((low, powers, inactivity)) => (Some(low), Some(powers), Some(inactivity)),
            Err(e) => { bevy::log::warn!("HUD mana configuration unavailable: {e}"); (None, None, None) }
        };
        let path = super::pause_menu::state_movie(
            catalog,
            "gamelogicstateingame",
            "ui/gamelogic/state_ingame.gamelogicstateingame",
        )?;
        let (mut player, resources) = Player::load_installed(catalog, language, &path)?;
        pause_state::file_ready(&mut player, screen)?;
        // FlashActivated (14078b260) shows the crosshair after the HUD is
        // attached. Other elements are driven by their gameplay components.
        player
            .call_path_method("_crosshair", "_C_Show", &[])
            .map_err(|e| player.describe(&e))?;
        // The interactions owner publishes the current input device before
        // its first prompt (14038f7d0).
        player
            .call_path_method(
                "controllerTypeHandler",
                "_C_UpdateControllerType",
                &[pause_state::controller_type().into()],
            )
            .map_err(|e| player.describe(&e))?;
        Self::settings(&mut player)?;
        let settings_generation = crate::platform::console::shared().generation();
        // The first gauge values come from the live player/component owner.
        // The movie retains its authored initial state until that data exists.
        let image_map = super::pause_menu::library_images(catalog, &player, &resources)?;
        let residency =
            super::pause_menu::MovieResidency::new("hud", catalog, &resources, &image_map);
        let images = Arc::new(image_map);
        let stage = player.state.borrow().stage_size;
        let interval = 1.0 / player.state.borrow().frame_rate;
        let mut cache = render::Cache::default();
        cache.set_font_caching(pause_state::font_caching(
            catalog,
            &path,
            stage,
            (screen.0 as u32, screen.1 as u32),
        ));
        Ok(Self {
            player,
            images,
            cache,
            generation,
            serial: 0,
            accumulated: 0.0,
            timer_elapsed: 0.0,
            interval,
            last: None,
            settings_generation,
            interaction_type: None,
            current_interaction: None,
            oxygen_input: None,
            oxygen_displayed: None,
            mana_low_level,
            starting_powers,
            inactivity_delay,
            inactivity_elapsed: 0.0,
            inactivity_admitted: false,
            hud_activity: true,
            master_visible: None,
            timing_since: std::env::var_os("DH2_DEBUG_HUD_TIMING")
                .filter(|v| v == "1")
                .map(|_| std::time::Instant::now()),
            timing_ticks: 0,
            timing_lists: 0,
            timing_tick_cost: std::time::Duration::ZERO,
            timing_list_cost: std::time::Duration::ZERO,
            _residency: residency,
        })
    }

    /// Remove the previous type before adding its replacement, as in
    /// 1403903b0 and 14038f7d0. The producer only sends changed focus.
    pub(super) fn interaction(&mut self, interaction: Option<Interaction>) -> Result<(), String> {
        self.current_interaction = interaction.clone();
        if let Some(kind) = self.interaction_type.take() {
            let types = self.player.array(vec![kind.into()]);
            self.player
                .call_path_method("_interactionsManager", "_C_RemoveInteraction", &[types])
                .map_err(|e| self.player.describe(&e))?;
        }
        if let Some(interaction) = interaction {
            // 1403913f0 gates types 8 and 9 with contextual icons; all
            // other interaction types use the separate window setting.
            let enabled = {
                let console = crate::platform::console::shared().read();
                let name = if matches!(interaction.kind, 8 | 9) {
                    "ark_ui_show_contextual_icons"
                } else {
                    "ark_ui_show_interactions"
                };
                console
                    .cvars
                    .find(name)
                    .is_some_and(|index| console.cvars.list[index].int != 0)
            };
            if !enabled {
                self.last = None;
                return Ok(());
            }
            let strings = self
                .player
                .state
                .borrow()
                .strings
                .clone()
                .ok_or("HUD has no string table")?;
            let label = strings.lookup(&interaction.action_label);
            let name = format!(
                "{}{}",
                strings.lookup(&interaction.name),
                strings.lookup(&interaction.name_postfix)
            );
            self.player
                .call_path_method(
                    "_interactionsManager",
                    "_C_AddInteraction",
                    &[
                        interaction.kind.into(),
                        label.into(),
                        interaction.game_action.as_str().into(),
                        name.as_str().into(),
                        interaction.hold.into(),
                        interaction.locked.into(),
                    ],
                )
                .map_err(|e| self.player.describe(&e))?;
            self.interaction_type = Some(interaction.kind);
        }
        self.last = None;
        Ok(())
    }

    /// 14045de20: keep the breath value while submerged, update the gauge
    /// when it changes, and hide it after leaving water or hiding the HUD.
    pub(super) fn oxygen(&mut self, input: Option<Oxygen>) -> Result<(), String> {
        self.oxygen_input = input;
        let master_enabled = {
            let console = crate::platform::console::shared().read();
            console
                .cvars
                .find("ark_ui_show_master_hud")
                .is_some_and(|index| console.cvars.list[index].int != 0)
        };
        let current = input
            .filter(|input| input.underwater && master_enabled)
            .map(|input| input.breath);
        if let Some(breath) = current {
            if self
                .oxygen_displayed
                .is_some_and(|previous| (breath - previous).abs() >= f32::EPSILON)
            {
                self.player
                    .call_path_method(
                        "_oxygenGauge",
                        "_C_UpdateGauge",
                        &[((breath * 0.01).clamp(0.0, 1.0) as f64).into()],
                    )
                    .map_err(|e| self.player.describe(&e))?;
                self.last = None;
            }
        } else if self.oxygen_displayed.is_some() {
            self.player
                .call_path_method("_oxygenGauge", "_C_Hide", &[])
                .map_err(|e| self.player.describe(&e))?;
            self.last = None;
        }
        self.oxygen_displayed = current;
        Ok(())
    }

    /// 140674230/140674870 publish health and projected regeneration as
    /// fractions of the actual evolved-health maximum.
    pub(super) fn health(&mut self, health: Health) -> Result<(), String> {
        self.hud_activity = true;
        let (current, projected, low) = if health.maximum > 0.0 {
            (health.current / health.maximum, health.projected / health.maximum, health.low)
        } else { (0.0, 0.0, false) };
        self.player.call_path_method("_masterHUD", "_C_FillHealthGauge", &[
            (current as f64).into(), health.extensions.into(), low.into(), (projected as f64).into(),
        ]).map_err(|e| self.player.describe(&e))?;
        self.last = None;
        Ok(())
    }

    pub(super) fn inactivity_admission(&mut self, admitted: bool) {
        self.inactivity_admitted = admitted;
    }

    pub(super) fn starting_powers(&self) -> Option<Vec<String>> { self.starting_powers.clone() }

    pub(super) fn mana(&mut self, mana: Mana) -> Result<(), String> {
        self.hud_activity = true;
        let Some(low_level) = self.mana_low_level else { return Ok(()) };
        let current = mana.current / mana.maximum;
        let target = mana.regen_target / mana.maximum;
        let status = if mana.suppressed || ((mana.lock_flags & 3) != 0 && mana.current == 0.0) {
            1
        } else if mana.has_mark { 0 } else { 2 };
        self.player.call_path_method("_masterHUD", "_C_FillManaGauge", &[
            (current as f64).into(), (mana.extension as i32).into(), (current < low_level).into(),
            (target as f64).into(), (-1.0).into(), status.into(),
        ]).map_err(|e| self.player.describe(&e))?;
        self.last = None;
        Ok(())
    }

    /// The HUD retains its last rendered frame between movie ticks.
    pub(super) fn interval(&self) -> f64 {
        self.interval
    }

    pub(super) fn frame(
        &mut self,
        dt: f64,
        force: bool,
    ) -> Result<(Option<FlashFrame>, Vec<u32>, bool), String> {
        let settings_generation = crate::platform::console::shared().generation();
        if self.settings_generation != settings_generation {
            Self::settings(&mut self.player)?;
            self.settings_generation = settings_generation;
            self.master_visible = None;
            self.interaction(self.current_interaction.clone())?;
            self.oxygen(self.oxygen_input)?;
            self.last = None;
        }
        // 14078bb20 only advances this timer in its admitted player state.
        // Missing possession state is not inferred from locomotion.
        // Oxygen's native activity flag remains set while the head is submerged.
        let oxygen_active = self.oxygen_input.is_some_and(|input| input.underwater);
        if !self.inactivity_admitted || self.hud_activity || oxygen_active {
            self.inactivity_elapsed = 0.0;
        } else {
            self.inactivity_elapsed += dt;
        }
        self.hud_activity = false;
        let mode = {
            let console = crate::platform::console::shared().read();
            console.cvars.find("ark_ui_show_master_hud").map_or(2, |i| console.cvars.list[i].int)
        };
        let visible = mode != 0 && (mode != 1 || self.inactivity_delay.is_none_or(|delay| self.inactivity_elapsed <= delay));
        if self.master_visible != Some(visible) {
            self.player.call_path_method("_masterHUD", if visible { "_C_Show" } else { "_C_Hide" }, &[])
                .map_err(|e| self.player.describe(&e))?;
            self.master_visible = Some(visible);
            self.last = None;
        }
        self.accumulated += dt;
        self.timer_elapsed += dt;
        let mut sounds = Vec::new();
        let tick = self.accumulated >= self.interval;
        let changed = self.last.is_none() || tick;
        if tick {
            // The host schedules at most one movie frame per engine update
            // (14002cc90). Preserve elapsed clock time for timers separately
            // from the frame-phase debt, including delayed worker commands.
            let step = std::mem::take(&mut self.timer_elapsed);
            self.accumulated = (self.accumulated - self.interval).max(0.0);
            let started = self.timing_since.map(|_| std::time::Instant::now());
            self.player
                .tick(step)
                .map_err(|e| self.player.describe(&e))?;
            if let Some(started) = started {
                self.timing_ticks += 1;
                self.timing_tick_cost += started.elapsed();
            }
        }
        if changed {
            sounds = super::pause_menu::movie_notifications(&mut self.player, "").1;
            self.serial += 1;
            let started = self.timing_since.map(|_| std::time::Instant::now());
            self.last = Some(FlashFrame {
                list: Arc::new(render::draw_list_cached(&self.player.vm, &mut self.cache)),
                images: self.images.clone(),
                game_images: Default::default(),
                generation: self.generation,
                serial: self.serial,
            });
            if let Some(started) = started {
                self.timing_lists += 1;
                self.timing_list_cost += started.elapsed();
            }
        }
        if let Some(since) = self.timing_since {
            if since.elapsed().as_secs_f64() >= 5.0 {
                bevy::log::info!(
                    "HUD movie over {:.1}s: {} ticks ({:.1} us each), {} draw lists ({:.1} us each)",
                    since.elapsed().as_secs_f64(),
                    self.timing_ticks,
                    self.timing_tick_cost.as_secs_f64() * 1e6 / self.timing_ticks.max(1) as f64,
                    self.timing_lists,
                    self.timing_list_cost.as_secs_f64() * 1e6 / self.timing_lists.max(1) as f64
                );
                self.timing_ticks = 0;
                self.timing_lists = 0;
                self.timing_tick_cost = std::time::Duration::ZERO;
                self.timing_list_cost = std::time::Duration::ZERO;
                self.timing_since = Some(std::time::Instant::now());
            }
        }
        Ok((
            if changed || force {
                self.last.clone()
            } else {
                None
            },
            sounds,
            tick,
        ))
    }
}
