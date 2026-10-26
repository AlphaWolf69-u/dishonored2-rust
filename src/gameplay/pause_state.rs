//! `arkGameLogicStatePause`: the data the game writes into the pause movie
//! and the callbacks it answers (notes: `artifacts/ui-pause/notes-pause-contract.md`).
use crate::assets::lang::StringTable;
use crate::flash::avm2::{value::Value, Result};
use crate::flash::Player;
use crate::gameplay::options::{self, Effect, Options};
use crate::platform::bindings::{self, BindableAction, Keyboard};
use crate::platform::console;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The movie the decl `ui/gamelogic/state_pause` loads per player
/// character (`m_characterDataContents[PLAYER_CORVO/EMILY].m_iggyInfo`).
pub fn movie(character: Character) -> &'static str {
    match character {
        Character::Corvo => "ui/corvo_pause_menu.iggy",
        Character::Emily => "ui/emily_pause_menu.iggy",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Character {
    Corvo,
    Emily,
}

/// `FUN_140788e30` (root state OnFlashLoaded when the decl sets
/// `m_flashRequiresBindingSet`): `bindableActionsData.length` first, then
/// each element's `label` and its three new arrays.
pub fn write_bindable_actions(player: &mut Player, actions: &[BindableAction]) -> Result<()> {
    player.set_value("bindableActionsData.length", (actions.len() as f64).into())?;
    for (i, action) in actions.iter().enumerate() {
        let element = format!("bindableActionsData.data[{i}]");
        player.set_value(&format!("{element}.label"), Value::from(action.label.as_str()))?;
        let strings = |items: &[String; 3]| items.iter().map(|s| Value::from(s.as_str())).collect();
        let bindings = player.array(strings(&action.bindings));
        let localized = player.array(strings(&action.localized_bindings));
        let key_codes = player.array(action.key_codes.iter().map(|&k| (k as f64).into()).collect());
        player.set_value(&format!("{element}.bindings"), bindings)?;
        player.set_value(&format!("{element}.localizedBindings"), localized)?;
        player.set_value(&format!("{element}.keyCodes"), key_codes)?;
    }
    Ok(())
}

/// The movie size the file manager reports (`FUN_140035590`): the RGB
/// resolution scaled by 1920x1080 over the viewport, truncated.
pub fn screen_size(rgb: (u32, u32), viewport: (u32, u32)) -> (i32, i32) {
    let w = rgb.0 as f32 * 1920.0 / viewport.0 as f32;
    let h = rgb.1 as f32 * 1080.0 / viewport.1 as f32;
    (w as i32, h as i32)
}

/// File ready (`FUN_14002fcb0`), before the state's OnFlashLoaded:
/// `screenPositions._C_UpdateScreenSize(w, h)` (`FUN_140035490`).
pub fn file_ready(player: &mut Player, size: (i32, i32)) -> std::result::Result<(), String> {
    player
        .call_path_method("screenPositions", "_C_UpdateScreenSize", &[(size.0 as f64).into(), (size.1 as f64).into()])
        .map(|_| ())
        .map_err(|e| format!("_C_UpdateScreenSize: {}", player.describe(&e)))
}

/// `FUN_140034920`: 1 (keyboard and mouse) when no pad is active, 3 for an
/// active pad of kind 1 (PS4), 0 for other pads. The rewrite has no pad
/// input yet, so no pad is active.
pub fn controller_type() -> i32 {
    1
}

/// What the game does next after draining the movie's notifications.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `ResumeGame` (`FUN_14078ca40`): pop the pause state.
    Resume,
    /// A display setting changed (visuals Update applies the mode).
    DisplayChanged,
    /// `SetFlashFocus` / `KillFlashFocus` from a movie (`FUN_140035020` /
    /// `FUN_140034fe0`): push or remove its focus entry.
    Focus(Movie, bool),
    /// The front message box's global movie appeared or went away.
    GlobalChanged,
    /// `QuitGame` (`LAB_1404f2970` -> `FUN_1404f3290(0)`): the `quit`
    /// command.
    Quit,
    /// `EnableUIPostFX` (`FUN_140790ef0`) / `DisableUIPostFX`
    /// (`FUN_140791370`): start or remove the screen FX.
    ScreenFx(bool),
    /// A Flash `PostSoundEvent` callback resolved by the game's Wwise handler.
    SoundEvent(u32),
}

/// The movies of the pause state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Movie {
    /// The pause movie (the message boxes' parent file).
    Pause,
    /// The front message box's `global.iggy`.
    Global,
}

/// `arkGameLogicStatePause` for one pause: the command state, the options
/// container (created by pause Init, `FUN_14078eff0`) and the inputs the
/// writers need.
pub struct Pause {
    pub options: Options,
    pub keyboard: Keyboard,
    pub strings: std::rc::Rc<StringTable>,
    pub profile: BTreeMap<String, String>,
    pub base: PathBuf,
    pub state: GameState,
    /// Where the config save writes `dishonored2Config.cfg`; `None` keeps
    /// the user's file untouched (diagnostic runs).
    pub config_path: Option<PathBuf>,
    /// The message box manager and the loader of its global movie.
    pub boxes: crate::gameplay::message_box::Manager,
    pub load_global: Box<dyn FnMut() -> std::result::Result<(Player, Vec<(usize, String)>), String>>,
}

impl Pause {
    /// Pause Init: the options sections from their decls over the client's
    /// command state (`console::shared`), and the difficulty manager.
    pub fn new(
        player: &mut Player,
        catalog: &crate::assets::archive::Catalog,
        base: &Path,
        state: GameState,
        config_path: Option<PathBuf>,
        load_global: Box<dyn FnMut() -> std::result::Result<(Player, Vec<(usize, String)>), String>>,
    ) -> std::result::Result<Pause, String> {
        Pause::with_options(player, read_options(catalog)?, base, state, config_path, load_global)
    }

    /// [`Pause::new`] with options already read ([`read_options`]).
    pub fn with_options(
        player: &mut Player,
        options: Options,
        base: &Path,
        state: GameState,
        config_path: Option<PathBuf>,
        load_global: Box<dyn FnMut() -> std::result::Result<(Player, Vec<(usize, String)>), String>>,
    ) -> std::result::Result<Pause, String> {
        let strings = player.state.borrow().strings.clone().ok_or("no string table")?;
        let profile = crate::gameplay::save_list::Saves::locate()
            .and_then(|s| crate::gameplay::save_list::read_profile(&s.folder))
            .unwrap_or_default();
        let mut pause = Pause {
            options,
            keyboard: Keyboard::default(),
            strings,
            profile,
            base: base.to_path_buf(),
            state,
            config_path,
            boxes: Default::default(),
            load_global,
        };
        pause.with_env(player, |options, env| options.start(env));
        Ok(pause)
    }

    /// Run `f` with the options' view of the game: the shared command state,
    /// the pause state active, the main menu not, and a level as the map.
    fn with_env<R>(&mut self, player: &mut Player, f: impl FnOnce(&mut Options, &mut options::Env) -> R) -> R {
        let Pause { options, keyboard, strings, profile, base, .. } = self;
        console::shared().write(|console| {
            let mut env = options::Env {
                player,
                console,
                strings,
                keyboard,
                profile,
                pause_active: true,
                main_menu_active: false,
                main_menu_map: false,
                base,
            };
            f(options, &mut env)
        })
    }

    /// OnFlashLoaded (`FUN_14078f170` -> root `FUN_1407898c0`) for the parts
    /// the rewrite can supply: the bindable actions, then
    /// `statsEntryAvailable`.
    pub fn on_flash_loaded(&mut self, player: &mut Player) -> std::result::Result<(), String> {
        self.write_bindable_actions(player)?;
        write_stats_available(player, &self.state).map_err(|e| format!("statsEntryAvailable: {}", player.describe(&e)))
    }

    fn write_bindable_actions(&mut self, player: &mut Player) -> std::result::Result<(), String> {
        let actions = bindings::bindable_actions(&console::shared().read().bindings, &self.strings, &mut self.keyboard);
        write_bindable_actions(player, &actions).map_err(|e| format!("bindableActionsData: {}", player.describe(&e)))
    }

    /// `Show(1)` (`FUN_1404f45a0`): FillData (its last write is
    /// `optionsData.sections`), then `_C_ShowPauseMenu()` with no arguments.
    pub fn show(&mut self, player: &mut Player) -> std::result::Result<(), String> {
        write_entries(player, &self.state).map_err(|e| format!("FillData: {}", player.describe(&e)))?;
        self.with_env(player, |options, env| options.write_sections(env))?;
        player.call_root_method("_C_ShowPauseMenu", &[]).map(|_| ()).map_err(|e| format!("_C_ShowPauseMenu: {}", player.describe(&e)))
    }

    /// `FUN_14002b610`: run the deferred `ExternalInterface` handlers in
    /// call order. Names the game does not register for this file are
    /// dropped; handlers the rewrite cannot perform yet are returned as
    /// unhandled.
    pub fn handle_notifications(&mut self, player: &mut Player, unhandled: &mut Vec<String>) -> std::result::Result<Vec<Request>, String> {
        let mut requests = Vec::new();
        for (name, args) in player.take_notifications() {
            match name.as_str() {
                "PostSoundEvent" => match args.first() {
                    // FUN_14002ca20 converts this numeric callback argument
                    // to uint32 before posting it to Wwise.
                    Some(Value::Number(id)) => requests.push(Request::SoundEvent(crate::flash::avm2::value::to_uint32(*id))),
                    _ => unhandled.push(name.clone()),
                },
                // Global `SetFlashFocus` -> `FUN_140035020`.
                "SetFlashFocus" => {
                    player
                        .call_path_method("controllerTypeHandler", "_C_UpdateControllerType", &[(controller_type() as f64).into()])
                        .map_err(|e| format!("_C_UpdateControllerType: {}", player.describe(&e)))?;
                    requests.push(Request::Focus(Movie::Pause, true));
                }
                "KillFlashFocus" => requests.push(Request::Focus(Movie::Pause, false)),
                "ResumeGame" => requests.push(Request::Resume),
                "QuitGame" => requests.push(Request::Quit),
                "EnableUIPostFX" => requests.push(Request::ScreenFx(true)),
                "DisableUIPostFX" => requests.push(Request::ScreenFx(false)),
                // `FUN_1404630f0`, registered by OnFlashLoaded (`FUN_1407898c0`).
                "MessageBoxRequest" => {
                    let info = crate::gameplay::message_box::Info::from_request(player, &args);
                    self.show_box(player, info, None, &mut requests)?;
                }
                // `OnOptionsClicked` has no C++ registration (dropped by the hub).
                "OnOptionsClicked" => {}
                _ => {
                    let handled = self.with_env(player, |options, env| options.handle(env, &name, &args));
                    match handled {
                        Some(effects) => {
                            let effects = effects?;
                            self.apply_effects(player, effects, &mut requests)?;
                        }
                        None => unhandled.push(name),
                    }
                }
            }
        }
        self.save_if_requested()?;
        Ok(requests)
    }

    fn apply_effects(&mut self, player: &mut Player, effects: Vec<Effect>, requests: &mut Vec<Request>) -> std::result::Result<(), String> {
        for effect in effects {
            match effect {
                Effect::MessageBox(b) => {
                    // `FUN_140517470`: the leave-section box answers the options
                    // container; the video-card notice has no callback.
                    let confirm = self.options.pending.is_some().then_some(crate::gameplay::message_box::Confirm::Options);
                    let info = crate::gameplay::message_box::Info::with_buttons(b.text, b.buttons, 0.0);
                    self.show_box(player, info, confirm, requests)?;
                }
                // `FUN_1406509f0`: the pause state shows bindings.
                Effect::BindingsChanged => self.write_bindable_actions(player)?,
                Effect::RefillTutorials => {
                    let set = |player: &mut Player, path: &str, v: bool| player.set_value(path, v.into());
                    let st = &self.state;
                    set(player, "pauseData.hasTutorials", st.new_tutorials + st.read_tutorials != 0)
                        .and_then(|_| set(player, "pauseData.tutorialsEntry.hasNewElements", st.new_tutorials != 0))
                        .map_err(|e| format!("tutorials: {}", player.describe(&e)))?;
                }
                Effect::DisplayChanged => requests.push(Request::DisplayChanged),
            }
        }
        Ok(())
    }

    fn show_box(
        &mut self,
        player: &mut Player,
        info: crate::gameplay::message_box::Info,
        confirm: Option<crate::gameplay::message_box::Confirm>,
        requests: &mut Vec<Request>,
    ) -> std::result::Result<(), String> {
        let (_, events) = self.boxes.show(info, 0, confirm, player, &self.strings, &mut *self.load_global)?;
        if !events.is_empty() {
            requests.push(Request::GlobalChanged);
        }
        Ok(())
    }

    /// The front message box's global movie: its notifications (the
    /// global file's registrations `FUN_140462210` and the global focus
    /// callbacks). `parent` is the pause movie.
    pub fn handle_global_notifications(&mut self, parent: &mut Player, unhandled: &mut Vec<String>) -> std::result::Result<Vec<Request>, String> {
        use crate::gameplay::message_box::{Confirm, Event};
        let mut requests = Vec::new();
        let Some(global) = self.boxes.global() else { return Ok(requests) };
        let notifications = global.take_notifications();
        for (name, args) in notifications {
            match name.as_str() {
                // 14002c220 registers the sound handler for every movie,
                // including the global movie loaded by 140462210.
                "PostSoundEvent" => match args.first() {
                    Some(Value::Number(id)) => requests.push(Request::SoundEvent(crate::flash::avm2::value::to_uint32(*id))),
                    _ => unhandled.push(name.clone()),
                },
                "SetFlashFocus" => {
                    if let Some(g) = self.boxes.global() {
                        g.call_path_method("controllerTypeHandler", "_C_UpdateControllerType", &[(controller_type() as f64).into()])
                            .map_err(|e| format!("_C_UpdateControllerType: {}", g.describe(&e)))?;
                    }
                    requests.push(Request::Focus(Movie::Global, true));
                }
                "KillFlashFocus" => requests.push(Request::Focus(Movie::Global, false)),
                // `FUN_140463090`.
                "OnMessageBoxConfirmed" => {
                    if let Some(Event::Confirmed { confirm: Confirm::Options, answer }) = self.boxes.confirmed(&args) {
                        requests.extend(self.answer(parent, answer.max(0) as u32)?);
                    }
                    self.boxes.close_front(parent, &self.strings, &mut *self.load_global)?;
                    requests.push(Request::GlobalChanged);
                    // The closed movie's remaining notifications go with it.
                    break;
                }
                "MessageBoxRequest" => {
                    let Some(g) = self.boxes.global() else { continue };
                    let info = crate::gameplay::message_box::Info::from_request(g, &args);
                    self.show_box(parent, info, None, &mut requests)?;
                }
                // The `arkMessageBox::On*` names and the empty names of
                // AS3-requested boxes have no registration.
                n if n.is_empty() || n.starts_with("arkMessageBox::") => {}
                _ => unhandled.push(name),
            }
        }
        Ok(requests)
    }

    /// The answer of a message box the options asked for.
    pub fn answer(&mut self, player: &mut Player, answer: u32) -> std::result::Result<Vec<Request>, String> {
        let effects = self.with_env(player, |options, env| options.answer(env, answer))?;
        let mut requests = Vec::new();
        self.apply_effects(player, effects, &mut requests)?;
        self.save_if_requested()?;
        Ok(requests)
    }

    /// Pause Update (`FUN_14078ff90`): the options' per-frame work. `pressed`
    /// is the first engine key pressed this frame.
    pub fn update(&mut self, player: &mut Player, pressed: Option<u32>) -> std::result::Result<(), String> {
        self.boxes.update(&self.strings)?;
        self.with_env(player, |options, env| options.update(env, pressed))
    }

    /// The profile update after a config save (`FUN_1400d2f10`).
    fn save_if_requested(&mut self) -> std::result::Result<(), String> {
        if !std::mem::take(&mut self.options.save_requested) {
            return Ok(());
        }
        match &self.config_path {
            Some(path) => console::shared().write(|c| c.save_config(path)),
            None => Ok(()),
        }
    }
}

/// The game state FillData reads, as far as the rewrite has it. `default()`
/// is the rewrite's current state: no save writer or loader, no mission,
/// chapter, tutorial log or statistics components yet.
#[derive(Clone, Debug, Default)]
pub struct GameState {
    /// Save system idle (`!FUN_140465db0`) and able to save now
    /// (`FUN_140465020`). The rewrite cannot write saves yet.
    pub can_save: bool,
    /// Save system idle and not in mode 2 (`FUN_140464f90`).
    pub can_load: bool,
    /// A quick save slot exists and its map chunk is installed (`FUN_140464fb0`).
    pub quick_save_available: bool,
    /// A mission-start save of the current chapter exists (`FUN_1404f3d10`).
    /// The rewrite has no current chapter, so none matches.
    pub restart_available: bool,
    /// The player has the mission component (`DAT_1439c5ac0`) with stats.
    pub stats_available: bool,
    /// New and read tutorial counts of the tutorial log component.
    pub new_tutorials: usize,
    pub read_tutorials: usize,
    /// The save helper's entries (`FUN_140467010`), newest first.
    pub saves: Vec<crate::gameplay::save_list::Entry>,
    /// A manual save has a free slot (`sys+0x18 != -1`).
    pub can_create_save: bool,
    /// `campaignSaveType` of the profile's campaign.
    pub save_type: i32,
    /// `gameLocal.m_missionPlusMode` byte +0x661; no live game state yet.
    pub mission_plus: bool,
    /// The application is the trial (Steam AppID other than 403640).
    pub trial: bool,
    /// Text of `m_newSaveSlotLabel` (`UI_createnewsave`).
    pub new_save_label: String,
    /// The chapter header (`FUN_1404f3640`): the mission name's text and
    /// index. The rewrite has no mission component, so this is the branch for
    /// a player without a current mission: the loaded map's
    /// `m_loadingInfo` (`gameLocal.m_mapInfo + 0x210 / +0x20c`); `None`
    /// without a map info, which writes neither.
    pub chapter: Option<(String, i32)>,
    /// `PlayTime_<campaignIndex>` from the profile, formatted (`FUN_14080cf50`).
    pub play_time: String,
    /// The playthrough statistic (0x22, scope 3) + 1; no statistics yet, so 1.
    pub playthrough_index: i32,
}

/// The element fields shared by both lists, in the writers' order.
fn write_save_entry(player: &mut Player, element: &str, e: &crate::gameplay::save_list::Entry, mission_plus: bool) -> Result<()> {
    let s = |v: &str| Value::from(v);
    player.set_value(&format!("{element}.missionName"), s(&e.mission_name))?;
    player.set_value(&format!("{element}.missionIndex"), (e.mission_index as f64).into())?;
    player.set_value(&format!("{element}.levelChaos"), (e.level_chaos as f64).into())?;
    player.set_value(&format!("{element}.characterName"), s(&e.character_name))?;
    player.set_value(&format!("{element}.saveDate"), s(&e.save_date))?;
    player.set_value(&format!("{element}.type"), (e.kind as f64).into())?;
    player.set_value(&format!("{element}.slotNumber"), (e.slot as f64).into())?;
    player.set_value(&format!("{element}.difficulty"), s(&e.difficulty))?;
    player.set_value(&format!("{element}.playtime"), s(&e.playtime))?;
    player.set_value(&format!("{element}.location"), s(&e.location))?;
    player.set_value(&format!("{element}.corrupted"), e.corrupted.into())?;
    player.set_value(&format!("{element}.missionPlus"), mission_plus.into())
}

/// `FUN_1404f3840`: `savesData.saveData`: the "create" element when a slot
/// is free, then the entries the save mode allows.
pub fn write_save_data(player: &mut Player, state: &GameState) -> Result<()> {
    let kept: Vec<_> = state
        .saves
        .iter()
        .filter(|e| if state.save_type == 1 { !matches!(e.kind, 2 | 4) } else { !matches!(e.kind, 1 | 2 | 4) })
        .collect();
    let create = state.can_create_save as usize;
    player.set_value("savesData.saveData.length", ((kept.len() + create) as f64).into())?;
    if create == 1 {
        player.set_value("savesData.saveData.data[0].label", Value::from(state.new_save_label.as_str()))?;
        player.set_value("savesData.saveData.data[0].type", 3.0.into())?;
    }
    for (i, e) in kept.iter().enumerate() {
        let element = format!("savesData.saveData.data[{}]", i + create);
        write_save_entry(player, &element, e, state.mission_plus)?;
        if let Some(image) = &e.mission_image {
            player.set_value(&format!("{element}.missionImage"), Value::from(image.as_str()))?;
        }
    }
    Ok(())
}

/// `FUN_1404f3300`: `savesData.loadData`. On PC every map chunk is
/// available, so `chunkLockedCallback` is `SlotLocked` only for a trial
/// application loading a full-game save.
pub fn write_load_data(player: &mut Player, state: &GameState) -> Result<()> {
    player.set_value("savesData.loadData.length", (state.saves.len() as f64).into())?;
    for (i, e) in state.saves.iter().enumerate() {
        let element = format!("savesData.loadData.data[{i}]");
        write_save_entry(player, &element, e, state.mission_plus)?;
        let locked = state.trial && !e.trial_save;
        player.set_value(&format!("{element}.chunkLockedCallback"), Value::from(if locked { "SlotLocked" } else { "" }))?;
        if let Some(image) = &e.mission_image {
            player.set_value(&format!("{element}.missionImage"), Value::from(image.as_str()))?;
        }
    }
    Ok(())
}

/// The rewrite's game state for a player: the installed full game (not the
/// trial) and the user's saves, formatted for the text language (English
/// dates use the 12-hour US format, `sys_date_format_europa` otherwise).
pub fn current_state(player: &Player, language: &str, catalog: &crate::assets::archive::Catalog) -> GameState {
    let strings = player.state.borrow().strings.clone();
    match strings {
        Some(strings) => with_chapter(with_saves(GameState::default(), &strings, language == "english"), &strings, catalog),
        None => GameState::default(),
    }
}

/// The options sections the pause state shows (`FUN_140516d70`); the game
/// creates them with the game, so the client reads them before the open.
pub fn read_options(catalog: &crate::assets::archive::Catalog) -> std::result::Result<Options, String> {
    let mut read = |path: &str| catalog.read(path).ok().flatten();
    Ok(console::shared().write(|console| options::read(&mut read, console))?)
}

/// The chapter header's values (`FUN_1404f3640`).
pub fn with_chapter(mut state: GameState, strings: &crate::assets::lang::StringTable, catalog: &crate::assets::archive::Catalog) -> GameState {
    use crate::gameplay::map_info;
    state.chapter = map_info::current_loading_info(catalog).map(|info| (strings.lookup(&info.mission_name).to_string(), info.mission_index));
    let seconds = crate::gameplay::save_list::Saves::locate().map_or(0, |s| s.profile_int(&format!("PlayTime_{}", s.campaign)));
    state.play_time = crate::gameplay::save_list::format_playtime(strings, seconds);
    state.playthrough_index = 1;
    state
}

/// The save lists from the user's saves (`FUN_140658950`, `FUN_140467010`).
pub fn with_saves(mut state: GameState, strings: &crate::assets::lang::StringTable, english: bool) -> GameState {
    use crate::gameplay::save_list::{helper_entries, next_free_slot, read_headers, system_list, Saves};
    if let Some(saves) = Saves::locate() {
        let list = system_list(read_headers(&saves.folder), saves.campaign);
        state.can_create_save = next_free_slot(&list, saves.campaign, saves.save_type) != -1;
        state.save_type = saves.save_type;
        state.saves = helper_entries(&list, strings, english);
    }
    state.new_save_label = strings.lookup("UI_createnewsave").to_string();
    state
}

/// FillData entries (`FUN_14078e740`, `FUN_1404f3800`, `FUN_14078e6c0`,
/// `FUN_14078e9e0`, `FUN_14078e850`), in the game's write order.
pub fn write_entries(player: &mut Player, state: &GameState) -> Result<()> {
    // `FUN_1404f32c0`: the save list, then the load list.
    write_save_data(player, state)?;
    write_load_data(player, state)?;
    let set = |player: &mut Player, path: &str, v: bool| player.set_value(path, v.into());
    // `FUN_14078e740` (vtable +0xa0).
    set(player, "pauseData.saveEntry.locked", !state.can_save)?;
    set(player, "pauseData.loadEntry.locked", !state.can_load)?;
    set(player, "pauseData.quickLoadEntry.locked", !(state.can_load && state.quick_save_available))?;
    set(player, "pauseData.quickSaveEntry.locked", !state.can_save)?;
    // `FUN_14078e9e0`.
    set(player, "pauseData.hasTutorials", state.new_tutorials + state.read_tutorials != 0)?;
    set(player, "pauseData.tutorialsEntry.hasNewElements", state.new_tutorials != 0)?;
    // `FUN_1404f3640`: the chapter header.
    if let Some((name, index)) = &state.chapter {
        player.set_value("chapterData.chapterName", Value::from(name.as_str()))?;
        player.set_value("chapterData.chapterIndex", (*index as f64).into())?;
    }
    set(player, "chapterData.missionPlus", state.mission_plus)?;
    player.set_value("chapterData.playTime", Value::from(state.play_time.as_str()))?;
    player.set_value("chapterData.playthroughIndex", (state.playthrough_index as f64).into())?;
    // `FUN_14078e850`.
    set(player, "pauseData.statsEntry.locked", false)?;
    // `FUN_1404f3800`.
    set(player, "pauseData.restartEntry.locked", !state.restart_available)?;
    set(player, "pauseData.optionsEntry.locked", false)?;
    Ok(())
}

/// OnFlashLoaded's `pauseData.statsEntryAvailable` (`FUN_14078f170`).
pub fn write_stats_available(player: &mut Player, state: &GameState) -> Result<()> {
    player.set_value("pauseData.statsEntryAvailable", state.stats_available.into())
}

/// The pause state's OnEnter cvar work (`FUN_1404f2fc0`): the depth of field
/// is kept while paused, the user's `r_skipDOF` saved in `r_skipDofBck`.
pub fn enter(cvars: &mut crate::platform::cvars::CVars) {
    let (Some(skip), Some(backup)) = (cvars.find("r_skipDOF"), cvars.find("r_skipDofBck")) else { return };
    let saved = cvars.list[skip].int != 0;
    cvars.set_bool(backup, saved, true);
    cvars.set_bool(skip, false, true);
}

/// OnExit (`FUN_1404f3240`): `r_skipDOF` back from `r_skipDofBck`.
pub fn exit(cvars: &mut crate::platform::cvars::CVars) {
    let (Some(skip), Some(backup)) = (cvars.find("r_skipDOF"), cvars.find("r_skipDofBck")) else { return };
    let saved = cvars.list[backup].int != 0;
    cvars.set_bool(skip, saved, true);
}

#[cfg(test)]
mod dof_tests {
    #[test]
    fn the_pause_state_keeps_depth_of_field_and_restores_the_setting() {
        let mut cvars = crate::platform::cvars::CVars::new();
        let skip = cvars.find("r_skipDOF").unwrap();
        cvars.set_bool(skip, true, true);
        super::enter(&mut cvars);
        assert_eq!(cvars.int("r_skipDOF"), Some(0));
        assert_eq!(cvars.int("r_skipDofBck"), Some(1));
        super::exit(&mut cvars);
        assert_eq!(cvars.int("r_skipDOF"), Some(1));
    }
}

/// A movie's bitmap font caching from its `iggyfile` decl
/// (`m_vectorFontBuffer` {m_maxNumGlyph, m_maxGlyphWidth, m_maxGlyphHeight},
/// the descriptor `FUN_14002c440` reads at decl +0x64), for a display of
/// `display` pixels.
pub fn font_caching(catalog: &crate::assets::archive::Catalog, movie: &str, display: (u32, u32)) -> Option<crate::flash::glyph_cache::FontCaching> {
    use crate::gameplay::options::decl::{int, read_edit, scope};
    let mut read = |p: &str| catalog.read(p).ok().flatten();
    let name = format!("{}.iggyfile", movie.strip_suffix(".iggy").unwrap_or(movie));
    let edit = read_edit(&mut read, "iggyfile", &name).ok()?;
    let buffer = scope(&edit, "m_vectorFontBuffer")?;
    let field = |n: &str| int(buffer, n, 0).max(0) as u32;
    crate::flash::glyph_cache::FontCaching::from_buffer(field("m_maxNumGlyph"), field("m_maxGlyphWidth"), field("m_maxGlyphHeight"), display)
}

#[cfg(test)]
mod font_caching_tests {
    #[test]
    #[ignore = "requires installed game archives"]
    fn the_pause_movie_caches_glyphs_up_to_64_pixels() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
        let caching = super::font_caching(&catalog, super::movie(super::Character::Corvo), (1920, 1080)).unwrap();
        assert_eq!(caching.page, (1024, 1024));
        assert_eq!(caching.max_glyph, (64.0, 64.0));
        let large = super::font_caching(&catalog, super::movie(super::Character::Corvo), (2560, 1440)).unwrap();
        assert_eq!(large.page, (2048, 2048));
    }
}
