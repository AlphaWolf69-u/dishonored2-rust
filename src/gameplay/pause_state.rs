//! `arkGameLogicStatePause`: the data the game writes into the pause movie
//! and the callbacks it answers (notes: `artifacts/ui-pause/notes-pause-contract.md`).
use crate::flash::avm2::{value::Value, Result};
use crate::flash::Player;
use crate::platform::bindings::{self, BindableAction, Keyboard};
use std::path::Path;

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
}

/// `FUN_14002b610`: run the deferred `ExternalInterface` handlers in call
/// order. Names the game does not register for this file are dropped;
/// handlers the rewrite cannot perform yet are returned as unhandled.
pub fn handle_notifications(player: &mut Player, unhandled: &mut Vec<String>) -> std::result::Result<Vec<Request>, String> {
    let mut requests = Vec::new();
    for (name, _args) in player.take_notifications() {
        match name.as_str() {
            // Global `SetFlashFocus` -> `FUN_140035020`.
            "SetFlashFocus" => {
                player
                    .call_path_method("controllerTypeHandler", "_C_UpdateControllerType", &[(controller_type() as f64).into()])
                    .map_err(|e| format!("_C_UpdateControllerType: {}", player.describe(&e)))?;
            }
            "ResumeGame" => requests.push(Request::Resume),
            // `OnOptionsClicked` has no C++ registration (dropped by the hub).
            "OnOptionsClicked" => {}
            _ => unhandled.push(name),
        }
    }
    Ok(requests)
}

/// OnFlashLoaded (`FUN_14078f170` -> root `FUN_1407898c0`) for the parts
/// the rewrite can supply: the bindable actions from the installed and
/// user configs.
pub fn on_flash_loaded(player: &mut Player, base: &Path, user_config: Option<&str>, keyboard: &mut Keyboard) -> std::result::Result<(), String> {
    let bindings = bindings::game_bindings(base, user_config)?;
    let strings = player.state.borrow().strings.clone().ok_or("no string table")?;
    let actions = bindings::bindable_actions(&bindings, &strings, keyboard);
    write_bindable_actions(player, &actions).map_err(|e| format!("bindableActionsData: {}", player.describe(&e)))
}

/// `Show(1)` (`FUN_1404f45a0`): FillData, then `_C_ShowPauseMenu()` with no
/// arguments on the root. Returns whether the call succeeded (the state
/// retries every frame until it does).
pub fn show(player: &mut Player) -> std::result::Result<(), String> {
    player.call_root_method("_C_ShowPauseMenu", &[]).map(|_| ()).map_err(|e| format!("_C_ShowPauseMenu: {}", player.describe(&e)))
}
