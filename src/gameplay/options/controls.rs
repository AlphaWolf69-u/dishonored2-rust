//! `arkMenuSectionOptionControls` (vtable 0x141b70778), notes-options.md 5.
use super::decl::{self, widget, SectionDecl};
use super::{apply_cvar, load_cvar, read_back, Effect, Env, Kind, Result, Section, CURRENT};
use crate::assets::entities::Value as DeclValue;
use crate::assets::lang::key_id;
use crate::flash::avm2::value::Value;
use crate::platform::bindings::{Binding, IN_GAME};

/// `ARK_CONTROLSETTING_*` used by name.
const CONTROL_SCHEME: usize = 0;
const KEY_INTERACT: usize = 18;
const KEY_QUICK_LOAD: usize = 30;
const LEAN_TOGGLE: usize = 47;
/// `INTERLUDES`, the second set INTERACT and QUICK_LOAD write.
const INTERLUDES: usize = 1;
/// `arkOptionGamepads_t` XBOX / PS4.
const XBOX: usize = 0;
const PS4: usize = 1;

/// The cvar of each value setting (Load `FUN_140507f20` / Apply
/// `FUN_140507a90`); none inverts.
fn cvar(index: usize) -> Option<&'static str> {
    Some(match index {
        CONTROL_SCHEME => "ark_input_set_gamepad_preset",
        1 => "ark_rumble_enable",
        2 => "g_lookStickSensitivityX",
        3 => "g_lookStickSensitivityY",
        4 => "g_invertLookYAxis",
        5 => "ark_player_autoaim_model_pad",
        6 => "ark_player_autoaim_strength_pad",
        41 => "g_mouseSensitivity",
        42 => "g_mouseSmooth",
        43 => "g_invertMouseYAxis",
        44 => "ark_player_autoaim_model_mouse",
        45 => "ark_player_autoaim_strength_mouse",
        46 => "g_sprintToggle",
        LEAN_TOGGLE => "g_leanToggle",
        _ => return None,
    })
}

/// `RestoreDefaults` reset order (`FUN_140506d10`).
const RESTORED: [&str; 14] = [
    "ark_input_set_gamepad_preset",
    "ark_rumble_enable",
    "g_lookStickSensitivityX",
    "g_lookStickSensitivityY",
    "g_invertLookYAxis",
    "ark_player_autoaim_model_pad",
    "ark_player_autoaim_strength_pad",
    "g_mouseSensitivity",
    "g_invertMouseYAxis",
    "ark_player_autoaim_model_mouse",
    "ark_player_autoaim_strength_mouse",
    "g_mouseSmooth",
    "g_leanToggle",
    "g_sprintToggle",
];

/// One `m_presets` entry (0x18): `m_name` and `m_layoutDesc` (the fill
/// does not read `m_id`).
#[derive(Clone, Debug, Default)]
pub struct Preset {
    pub name: u32,
    pub layout: Vec<u32>,
}

/// One `m_gamepads` entry (0x38, decl+0x36a0).
#[derive(Clone, Debug, Default)]
pub struct Gamepad {
    pub layout_image: String,
    pub presets: Vec<Preset>,
}

#[derive(Debug, Default)]
pub struct State {
    pub gamepads: Vec<Gamepad>,
    /// `+0x230`: the confirmed gamepad preset.
    pub confirmed_preset: i32,
    /// `+0x238`, `+0x23c`, `+0x240`: key capture.
    pub capturing: bool,
    pub capture_element: usize,
    pub captured: u32,
}

impl State {
    pub fn new(decl: &SectionDecl) -> State {
        let mut gamepads = vec![Gamepad::default(); super::tables::GAMEPADS.len()];
        if let Some(pads) = decl::scope(&decl.edit, "m_gamepads") {
            for (pad, value) in decl::enum_items(pads, super::tables::GAMEPADS) {
                let DeclValue::Scope(f) = value else { continue };
                let presets = decl::scope(f, "m_presets")
                    .map(|p| {
                        decl::list_items(p)
                            .into_iter()
                            .filter_map(|v| match v {
                                DeclValue::Scope(p) => Some(Preset {
                                    name: decl::string_id(p, "m_name"),
                                    layout: decl::scope(p, "m_layoutDesc")
                                        .map(|l| {
                                            decl::list_items(l)
                                                .into_iter()
                                                .filter_map(|v| match v {
                                                    DeclValue::Atom(a) => Some(key_id(decl::unquote(a))),
                                                    _ => None,
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default(),
                                }),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                gamepads[pad as usize] = Gamepad { layout_image: decl::atom(f, "m_layoutImage").unwrap_or("").to_string(), presets };
            }
        }
        State { gamepads, ..State::default() }
    }
}

fn state(section: &mut Section) -> &mut State {
    match &mut section.kind {
        Kind::Controls(s) => s,
        _ => unreachable!("controls state"),
    }
}

/// `FUN_140039d30` / `FUN_140039d40`: keyboard and mouse key codes.
fn keyboard_or_mouse(key: u32) -> bool {
    (1..=0xdf).contains(&key) || (0xe0..=0xea).contains(&key)
}

/// The keys of a binding setting in `set` (`FUN_140039ad0` / `FUN_140039b90`).
fn bound_keys(env: &Env, w: &decl::Widget, set: usize) -> Vec<u32> {
    let b = &env.console.bindings;
    if w.binding_type == 1 {
        b.keys_for_command(set, &w.bindable_console_command)
    } else {
        b.keys_for_action(set, w.bindable_game_action)
    }
}

pub fn load(env: &mut Env, section: &mut Section, index: usize) {
    let s = &mut section.decl.settings[index];
    if s.widget.kind == widget::KEYBOARD_BINDING {
        // `FUN_140502810`: IN_GAME keys, ascending; the secondary flag picks
        // the slot and the last match wins. Pad keys are ignored.
        let keys = bound_keys(env, &s.widget, IN_GAME);
        let (mut k0, mut k1) = (0, 0);
        for key in keys.into_iter().filter(|&k| keyboard_or_mouse(k)) {
            if env.console.bindings.sets[IN_GAME][key as usize].secondary {
                k1 = key;
            } else {
                k0 = key;
            }
        }
        s.widget.key_code_0 = k0;
        s.widget.key_code_1 = k1;
    } else if let Some(name) = cvar(index) {
        load_cvar(env, s, name, false);
    }
}

/// `FUN_140501930(widget, set)`: clear the action's keyboard and mouse keys
/// in `set`, then bind both slots (a zero code is written too).
fn apply_binding(env: &mut Env, w: &decl::Widget, set: usize) {
    let old: Vec<u32> = bound_keys(env, w, set).into_iter().filter(|&k| keyboard_or_mouse(k)).collect();
    let (action, command) = if w.binding_type == 1 { (0, w.bindable_console_command.clone()) } else { (w.bindable_game_action, String::new()) };
    let entries = &mut env.console.bindings.sets[set];
    for key in old {
        entries[key as usize] = Binding::default();
    }
    entries[w.key_code_0 as usize] = Binding { secondary: false, action, command: command.clone() };
    entries[w.key_code_1 as usize] = Binding { secondary: true, action, command };
}

/// `FUN_140507a90`.
fn apply_setting(env: &mut Env, section: &Section, index: usize) {
    let s = &section.decl.settings[index];
    if s.widget.kind == widget::KEYBOARD_BINDING {
        apply_binding(env, &s.widget, IN_GAME);
        if matches!(index, KEY_INTERACT | KEY_QUICK_LOAD) {
            apply_binding(env, &s.widget, INTERLUDES);
        }
    } else if let Some(name) = cvar(index) {
        apply_cvar(env, s, name, false);
    }
}

/// `FUN_140507d90`.
pub fn apply(env: &mut Env, section: &mut Section) -> Result<Vec<Effect>> {
    let mut effects = Vec::new();
    for (out, index) in section.elements(env) {
        if read_back(env, section, out, index, true)? {
            apply_setting(env, section, index);
            if index == LEAN_TOGGLE && env.pause_active {
                // `FUN_14078e9e0`: the pause tutorials entry is rewritten.
                effects.push(Effect::RefillTutorials);
            }
        }
    }
    effects.push(Effect::BindingsChanged);
    Ok(effects)
}

/// `FUN_140506d10`.
pub fn restore(env: &mut Env) -> Result<Vec<Effect>> {
    // `exec default_keybinds.cfg -s`, executed at once.
    let base = env.base.to_path_buf();
    env.console.execute(&base, "exec default_keybinds.cfg -s\n")?;
    for name in RESTORED {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    Ok(vec![Effect::BindingsChanged])
}

fn link_path(out: usize) -> String {
    format!("{CURRENT}.settings.data[{out}].link")
}

/// `FUN_140506df0`: the control-scheme data for preset `selected`.
fn fill_scheme(env: &mut Env, section: &mut Section, out: usize, index: usize, selected: i32) -> Result<()> {
    let st = state(section);
    let xbox = st.gamepads.get(XBOX).cloned().unwrap_or_default();
    let ps4 = st.gamepads.get(PS4).cloned().unwrap_or_default();
    let last = xbox.presets.len() as i32 - 1;
    let selected = if selected < 0 { 0 } else { selected.min(last) };
    env.set("optionsData.controlSchemeData.controllers.length", 2.0.into())?;
    // `FUN_140034990`: 3 for an active pad of controller type 1, else 0;
    // the rewrite has no pad input, so no pad is active.
    let active_type = 0;
    for (c, pad) in [(XBOX, &xbox), (PS4, &ps4)] {
        let element = format!("optionsData.controlSchemeData.controllers.data[{c}]");
        env.set(&format!("{element}.isActive"), ([0, 3][c] == active_type).into())?;
        env.set(&format!("{element}.type"), (c as f64).into())?;
        let actions = pad.presets.get(selected.max(0) as usize).map(|p| p.layout.iter().map(|&id| env.text(id)).collect::<Vec<_>>()).unwrap_or_default();
        let actions = env.strings_array(actions);
        env.set(&format!("{element}.actions"), actions)?;
        // `image` is the resolved image's name (`FUN_1400607f0`, field +8);
        // how the image manager names `m_layoutImage` is not traced, so the
        // field is not written (blocker in docs/flash.md).
        let _ = &pad.layout_image;
    }
    let link = link_path(out);
    env.set(&format!("{link}.as3callback"), Value::from("_C_DisplayGamepadScheme"))?;
    env.set(&format!("{link}.gameCallbackConfirmation"), Value::from("OnControlSchemeModifiedConfirmation"))?;
    env.set(&format!("{link}.gameCallbackCancel"), Value::from("OnControlSchemeCancel"))?;
    env.set(&format!("{link}.gameDataProvider.length"), 1.0.into())?;
    let description = env.text(section.decl.settings[index].widget.description);
    env.set(&format!("{link}.gameDataProvider.data[0].label"), Value::from(description.as_str()))?;
    let stepper = format!("{link}.gameDataProvider.data[0].optionStepper");
    env.set(&format!("{stepper}.updateGameCallback"), Value::from("OnControlSchemeUpdated"))?;
    let names: Vec<String> = xbox.presets.iter().map(|p| env.text(p.name)).collect();
    let current = names.get(selected.max(0) as usize).cloned().unwrap_or_default();
    let values = env.strings_array(names);
    env.set(&format!("{stepper}.values"), values)?;
    env.set(&format!("{stepper}.selectedIndex"), (selected as f64).into())?;
    env.set(&format!("{link}.currentValue"), Value::from(current.as_str()))
}

/// Vfunc +0x90 (`FUN_140505e30`).
pub fn link_fill(env: &mut Env, section: &mut Section, out: usize, index: usize) -> Result<()> {
    let selected = section.decl.settings[index].widget.selected;
    state(section).confirmed_preset = selected;
    fill_scheme(env, section, out, index, selected)
}

/// The scheme stepper's index; the link element is always `data[0]`
/// (vfunc +0xa8 returns 0).
fn scheme_stepper_index(env: &mut Env) -> Result<i32> {
    env.get_int(&format!("{}.gameDataProvider.data[0].optionStepper.selectedIndex", link_path(0)))
}

/// Vfunc +0x98 / +0xa0.
pub fn link_read(env: &mut Env, section: &mut Section, index: usize, store: bool) -> Result<bool> {
    let v = scheme_stepper_index(env)?;
    let w = &mut section.decl.settings[index].widget;
    let changed = w.selected != v;
    if store {
        w.selected = v;
    }
    Ok(changed)
}

/// The capture's target element (`FUN_140507420`).
fn finish_capture(env: &mut Env, section: &mut Section, key: u32) -> Result<()> {
    let target = state(section).capture_element;
    let path = format!("{CURRENT}.settings.data[{target}].keyboardBinding");
    let slot = env.get_int(&format!("{path}.selectedBindingIndex"))?;
    let cancelled = key == 1;
    let key = if cancelled { env.get_int(&format!("{path}.keyCode_{slot}"))? as u32 } else { key };
    let name = env.keyboard.localized_name(key, env.strings);
    env.set(&format!("{path}.key_{slot}"), Value::from(name.as_str()))?;
    env.set(&format!("{path}.keyCode_{slot}"), (key as f64).into())?;
    if !cancelled {
        for (out, index) in section.elements(env) {
            if section.decl.settings[index].widget.kind != widget::KEYBOARD_BINDING {
                continue;
            }
            let other = format!("{CURRENT}.settings.data[{out}].keyboardBinding");
            let mut cleared = false;
            if env.get_int(&format!("{other}.keyCode_0"))? as u32 == key && (out != target || slot == 1) {
                env.set(&format!("{other}.keyCode_0"), 0.0.into())?;
                env.set(&format!("{other}.key_0"), Value::from(""))?;
                cleared = true;
            }
            if env.get_int(&format!("{other}.keyCode_1"))? as u32 == key && (out != target || slot == 0) {
                env.set(&format!("{other}.keyCode_1"), 0.0.into())?;
                env.set(&format!("{other}.key_1"), Value::from(""))?;
                cleared = true;
            }
            if cleared {
                env.call(&other, "refresh", &[])?;
            }
        }
    }
    state(section).capturing = false;
    Ok(())
}

/// Vfunc +0x28 (`FUN_140507920`): while capturing, the first key pressed
/// this frame (keyboard before mouse, mouse move excluded) ends the capture.
pub fn update(env: &mut Env, section: &mut Section, pressed: Option<u32>) -> Result<()> {
    if !state(section).capturing {
        return Ok(());
    }
    let Some(key) = pressed.filter(|&k| k != 0 && k != 0xea && crate::platform::bindings::KEY_NAMES.iter().any(|n| n.0 == k)) else {
        return Ok(());
    };
    state(section).captured = key;
    finish_capture(env, section, key)
}

pub fn handle(env: &mut Env, section: &mut Section, name: &str, args: &[Value]) -> Option<Result<Vec<Effect>>> {
    if !matches!(
        name,
        "EnterControlSchemeScreen"
            | "ExitControlSchemeScreen"
            | "OnControlSchemeUpdated"
            | "OnControlSchemeModifiedConfirmation"
            | "OnControlSchemeCancel"
            | "StartKeyboardBindingAssignation"
            | "OnKeyboardBindingUnassigned"
    ) {
        return None;
    }
    let arg = args.first().cloned().unwrap_or(Value::Undefined);
    let scheme = section.elements(env).into_iter().find(|e| e.1 == CONTROL_SCHEME);
    Some((|| -> Result<Vec<Effect>> {
        match name {
            // The input-device-change refill callback is removed while the
            // scheme screen shows; the rewrite has no device changes.
            "EnterControlSchemeScreen" | "ExitControlSchemeScreen" => {}
            "OnControlSchemeUpdated" => {
                if !args.is_empty() {
                    let v = env.player.member(&arg, "selectedIndex").map_err(|e| env.player.describe(&e))?.primitive_to_number() as i32;
                    if let Some((out, index)) = scheme {
                        fill_scheme(env, section, out, index, v)?;
                    }
                }
            }
            "OnControlSchemeModifiedConfirmation" => {
                state(section).confirmed_preset = scheme_stepper_index(env)?;
                env.call_root("_C_BackFromOptionSection")?;
            }
            "OnControlSchemeCancel" => {
                let confirmed = state(section).confirmed_preset;
                if let Some((out, index)) = scheme {
                    fill_scheme(env, section, out, index, confirmed)?;
                }
                env.call_root("_C_BackFromOptionSection")?;
            }
            "StartKeyboardBindingAssignation" => {
                let element = env.player.member(&arg, "actionID").map_err(|e| env.player.describe(&e))?.primitive_to_number() as usize;
                let st = state(section);
                if !st.capturing {
                    st.captured = 0;
                    st.capture_element = element;
                    st.capturing = true;
                }
            }
            "OnKeyboardBindingUnassigned" => {
                let slot = env.player.member(&arg, "selectedBindingIndex").map_err(|e| env.player.describe(&e))?.primitive_to_number() as i32;
                let target = arg.clone();
                env.player.set_member(&target, &format!("key_{slot}"), Value::from("")).map_err(|e| env.player.describe(&e))?;
                env.player.set_member(&target, &format!("keyCode_{slot}"), 0.0.into()).map_err(|e| env.player.describe(&e))?;
            }
            _ => return Err(format!("unregistered controls callback {name}")),
        }
        Ok(Vec::new())
    })())
}
