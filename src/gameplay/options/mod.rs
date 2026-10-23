//! `arkPauseMenuSectionOptions` and the four option sections
//! (`arkMenuSectionOption*`): the data written under `optionsData`, the
//! movie callbacks, and the cvars and bindings they edit
//! (notes: `artifacts/ui-pause/notes-options.md`).
pub mod decl;
mod tables;

mod audio;
mod controls;
mod gameplay;
mod visuals;

use crate::assets::lang::{key_id, StringTable};
use crate::flash::avm2::value::Value;
use crate::flash::Player;
use crate::platform::bindings::Keyboard;
use crate::platform::console::Console;
use decl::{availability, platform, widget, SectionDecl, Setting};
use std::collections::BTreeMap;

pub use tables::MENU_ITEMS;

/// `arkOptionMenuItem_t`.
pub const CONTROLS: i32 = 0;
pub const VISUALS: i32 = 1;
pub const AUDIO: i32 = 2;
pub const GAMEPLAY: i32 = 3;

const CURRENT: &str = "optionsData.currentSection";

type Result<T> = std::result::Result<T, String>;

/// What the option code reads from the rest of the game.
pub struct Env<'a> {
    pub player: &'a mut Player,
    pub console: &'a mut Console,
    pub strings: &'a StringTable,
    pub keyboard: &'a mut Keyboard,
    /// The platform profile dictionary (`savegame/profile.bin`).
    pub profile: &'a BTreeMap<String, String>,
    /// `gameLocal+0x1f6ef0` / `+0x1f6f10`: the pause and main-menu states.
    pub pause_active: bool,
    pub main_menu_active: bool,
    /// The current map is the main-menu map (gameLocal vfunc +0xf8).
    pub main_menu_map: bool,
    /// The installed `base` folder (`exec` reads configs from it).
    pub base: &'a std::path::Path,
}

impl Env<'_> {
    /// The localized text of a string id (`FUN_14002a530` writes text).
    pub fn text(&self, id: u32) -> String {
        self.strings.text(id).to_string()
    }

    /// A profile dictionary integer; a missing key reads 0.
    pub fn profile_int(&self, key: &str) -> i32 {
        self.profile.get(key).map_or(0, |v| crate::platform::cvars::atoi(v))
    }

    fn set(&mut self, path: &str, value: Value) -> Result<()> {
        self.player.set_value(path, value).map_err(|e| format!("{path}: {}", self.player.describe(&e)))
    }

    fn get(&mut self, path: &str) -> Result<Value> {
        self.player.get_value(path).map_err(|e| format!("{path}: {}", self.player.describe(&e)))
    }

    /// `IggyValueGetS32RS`.
    fn get_int(&mut self, path: &str) -> Result<i32> {
        Ok(self.get(path)?.primitive_to_number() as i32)
    }

    fn call(&mut self, path: &str, method: &str, args: &[Value]) -> Result<()> {
        self.player
            .call_path_method(path, method, args)
            .map(|_| ())
            .map_err(|e| format!("{path}.{method}: {}", self.player.describe(&e)))
    }

    fn call_root(&mut self, method: &str) -> Result<()> {
        self.player.call_root_method(method, &[]).map(|_| ()).map_err(|e| format!("{method}: {}", self.player.describe(&e)))
    }

    fn strings_array(&self, items: impl IntoIterator<Item = String>) -> Value {
        self.player.array(items.into_iter().map(|s| Value::from(s.as_str())).collect())
    }

    fn cvar(&self, name: &str) -> usize {
        self.console.cvars.find(name).unwrap_or_else(|| panic!("cvar {name} is registered by the executable"))
    }
}

/// The section-specific state (each subclass's fields past the base).
#[derive(Debug)]
enum Kind {
    Controls(controls::State),
    Visuals(visuals::State),
    Audio,
    Gameplay(gameplay::State),
}

#[derive(Debug)]
pub struct Section {
    /// `+0x08`: the `arkOptionMenuItem_t`.
    pub id: i32,
    pub decl: SectionDecl,
    kind: Kind,
    /// `+0x90`: the widget objects exist (created by Fill, deleted by Close).
    filled: bool,
}

/// A confirmation box waiting for its answer (`container+4`).
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    /// `FUN_140517470`: `ui_main_settings_confirmation`, buttons [no, yes];
    /// the answer is `Close(answer != 0)`.
    LeaveSection,
}

/// A message box the options code asks the host to show.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBox {
    pub text: &'static str,
    /// The `arkMessageBox` button set (2 = [`ui_common_no`, `ui_common_yes`]).
    pub buttons: u32,
}

/// `arkPauseMenuSectionOptions` (`FUN_140516e40`).
#[derive(Debug)]
pub struct Options {
    pub sections: Vec<Section>,
    /// `+0x00`: the current section's index in `sections`.
    pub current: Option<usize>,
    pub pending: Option<Pending>,
    /// Set by the config save (`FUN_140650aa0`): the profile update writes
    /// `dishonored2Config.cfg`.
    pub save_requested: bool,
}

/// What a handled callback asks of the host.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    MessageBox(MessageBox),
    /// The bindings changed (`FUN_1406509f0`): every state that shows
    /// bindings rewrites `bindableActionsData`.
    BindingsChanged,
    /// `FUN_14078e9e0`: the pause tutorials entry is rewritten.
    RefillTutorials,
    /// A display setting changed: the renderer applies the mode
    /// (`renderer vfunc +0x200`, `FUN_1400d1a90`).
    DisplayChanged,
}

/// Reads the decls `ui/gamelogic/pause/sections/options` names, in
/// `arkOptionMenuItem_t` order (`FUN_140516d70`).
pub fn read(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>, console: &mut Console) -> Result<Options> {
    let pause = decl::read_edit(read, "pausemenusectionoptions", "ui/gamelogic/pause/sections/options.pausemenusectionoptions")?;
    let defs = decl::scope(&pause, "m_optionsDefs").ok_or("options decl without m_optionsDefs")?;
    let mut sections = Vec::new();
    for (id, value) in decl::enum_items(defs, MENU_ITEMS) {
        let crate::assets::entities::Value::Atom(name) = value else { continue };
        let name = decl::unquote(name);
        let (kind_name, settings) = match id {
            CONTROLS => ("menusectionoptioncontrols", tables::CONTROL_SETTINGS),
            VISUALS => ("menusectionoptionvisuals", tables::VISUAL_SETTINGS),
            AUDIO => ("menusectionoptionaudio", tables::AUDIO_SETTINGS),
            GAMEPLAY => ("menusectionoptiongameplay", tables::GAMEPLAY_SETTINGS),
            _ => continue,
        };
        let decl = decl::section(decl::read_edit(read, kind_name, name)?, settings);
        let kind = match id {
            CONTROLS => Kind::Controls(controls::State::new(&decl)),
            VISUALS => Kind::Visuals(visuals::State::new(&decl, console)),
            AUDIO => Kind::Audio,
            _ => Kind::Gameplay(gameplay::State::new(read)?),
        };
        sections.push((id, Section { id, decl, kind, filled: false }));
    }
    sections.sort_by_key(|s| s.0);
    Ok(Options { sections: sections.into_iter().map(|s| s.1).collect(), current: None, pending: None, save_requested: false })
}

impl Section {
    /// `FUN_1405034e0` (vfunc +0x68) and the subclass additions.
    fn visible(&self, env: &Env, index: usize) -> bool {
        let s = &self.decl.settings[index];
        let platform = match s.platform {
            platform::ALL | platform::PC => true,
            platform::BNET => env.profile_int("HasLinkedBnet") != 0,
            _ => false,
        };
        let available = match s.availability {
            availability::MAIN_MENU => env.main_menu_active,
            availability::PAUSE_MENU => env.pause_active,
            availability::ALL => true,
            _ => false,
        };
        let base = platform && available && (s.category_index as usize) < self.decl.categories.len();
        base && match &self.kind {
            Kind::Audio => audio::visible(env, index),
            Kind::Visuals(_) => visuals::visible(s, index),
            _ => true,
        }
    }

    /// The visible settings in enum order: (element index, enum index).
    fn elements(&self, env: &Env) -> Vec<(usize, usize)> {
        (0..self.decl.settings.len()).filter(|&i| self.visible(env, i)).enumerate().collect()
    }

    fn element_of(&self, env: &Env, index: usize) -> Option<usize> {
        self.elements(env).into_iter().find(|e| e.1 == index).map(|e| e.0)
    }
}

fn element_path(out: usize) -> String {
    format!("{CURRENT}.settings.data[{out}]")
}

/// The widget object member of an element, by widget type.
fn widget_member(kind: i32) -> &'static str {
    match kind {
        widget::STEPPER_BOOLEAN => "optionStepperBoolean",
        widget::SLIDER => "slider",
        widget::DROPBOX => "dropBox",
        widget::KEYBOARD_BINDING => "keyboardBinding",
        widget::GAMMA | widget::CONTROL_SCHEME | widget::DIFFICULTY => "link",
        _ => "optionStepper",
    }
}

fn widget_path(s: &Setting, out: usize) -> String {
    format!("{}.{}", element_path(out), widget_member(s.widget.kind))
}

/// The stepper/dropbox `values`: runtime strings when the decl has none,
/// otherwise the localized decl strings, otherwise nothing.
fn values(env: &Env, s: &Setting) -> Option<Value> {
    let w = &s.widget;
    if w.string_values.is_empty() && !w.runtime_string_values.is_empty() {
        Some(env.strings_array(w.runtime_string_values.iter().cloned()))
    } else if !w.string_values.is_empty() {
        Some(env.strings_array(w.string_values.iter().map(|&id| env.text(id))))
    } else {
        None
    }
}

/// `FUN_140502720` (stepper, boolean stepper, dropbox) and `FUN_1405025f0`
/// (slider): the widget object's fields.
fn write_value_widget(env: &mut Env, s: &Setting, path: &str, refresh_enabled: bool, callback: Option<&str>) -> Result<()> {
    let w = &s.widget;
    env.set(&format!("{path}.refreshEnabled"), refresh_enabled.into())?;
    env.set(&format!("{path}.locked"), w.locked.into())?;
    if w.kind == widget::SLIDER {
        env.set(&format!("{path}.minimum"), (w.minimum as f64).into())?;
        env.set(&format!("{path}.maximum"), (w.maximum as f64).into())?;
        env.set(&format!("{path}.interval"), (w.interval as f64).into())?;
        env.set(&format!("{path}.displayValue"), w.display_value.into())?;
        env.set(&format!("{path}.value"), (w.selected as f64).into())?;
        if let Some(c) = callback {
            env.set(&format!("{path}.updateGameCallback"), Value::from(c))?;
        }
    } else {
        if let Some(v) = values(env, s) {
            env.set(&format!("{path}.values"), v)?;
        }
        if let Some(c) = callback {
            env.set(&format!("{path}.updateGameCallback"), Value::from(c))?;
        }
        env.set(&format!("{path}.selectedIndex"), (w.selected as f64).into())?;
    }
    Ok(())
}

/// `FUN_140502030`: one `currentSection.settings.data[out]` element.
fn write_element(
    env: &mut Env,
    section: &mut Section,
    out: usize,
    index: usize,
    refresh_enabled: bool,
    callback: Option<&str>,
    locked: bool,
) -> Result<()> {
    let element = element_path(out);
    let s = &section.decl.settings[index];
    let label = if !s.runtime_label.is_empty() {
        s.runtime_label.clone()
    } else {
        let (id, cvar) = &s.conditional_label;
        let conditional = !cvar.is_empty() && env.console.cvars.int(cvar).is_some_and(|v| v != 0);
        env.text(if conditional { *id } else { s.label })
    };
    env.set(&format!("{element}.label"), Value::from(label.as_str()))?;
    section.decl.settings[index].widget.locked = locked;
    let s = section.decl.settings[index].clone();
    if s.description != 0 {
        let text = env.text(s.description);
        env.set(&format!("{element}.description"), Value::from(text.as_str()))?;
    }
    env.set(&format!("{element}.categoryIndex"), (s.category_index as f64).into())?;
    let path = widget_path(&s, out);
    match s.widget.kind {
        widget::KEYBOARD_BINDING => {
            let w = &s.widget;
            env.set(&format!("{path}.refreshEnabled"), refresh_enabled.into())?;
            env.set(&format!("{path}.locked"), locked.into())?;
            env.set(&format!("{path}.actionID"), (out as f64).into())?;
            let name = env.keyboard.localized_name(w.key_code_1, env.strings);
            env.set(&format!("{path}.key_1"), Value::from(name.as_str()))?;
            env.set(&format!("{path}.keyCode_1"), (w.key_code_1 as f64).into())?;
            let name = env.keyboard.localized_name(w.key_code_0, env.strings);
            env.set(&format!("{path}.key_0"), Value::from(name.as_str()))?;
            env.set(&format!("{path}.keyCode_0"), (w.key_code_0 as f64).into())?;
        }
        widget::GAMMA | widget::CONTROL_SCHEME | widget::DIFFICULTY => {
            let text = env.text(s.widget.description);
            env.set(&format!("{path}.description"), Value::from(text.as_str()))?;
            link_fill(env, section, out, index)?;
        }
        _ => write_value_widget(env, &s, &path, refresh_enabled, callback)?,
    }
    Ok(())
}

/// `FUN_140503740`: rewrite a value widget with `refreshEnabled` and no
/// callback, then call its `refresh()`. Bindings and links are skipped.
fn refresh(env: &mut Env, section: &Section, index: usize) -> Result<()> {
    let Some(out) = section.element_of(env, index) else { return Ok(()) };
    let s = &section.decl.settings[index];
    if matches!(s.widget.kind, widget::KEYBOARD_BINDING | widget::GAMMA | widget::CONTROL_SCHEME | widget::DIFFICULTY) {
        return Ok(());
    }
    let path = widget_path(s, out);
    write_value_widget(env, s, &path, true, None)?;
    env.call(&path, "refresh", &[])
}

/// `FUN_140502e40`: the movie's value of a value widget.
fn movie_value(env: &mut Env, section: &Section, out: usize, index: usize) -> Result<i32> {
    let s = &section.decl.settings[index];
    let path = widget_path(s, out);
    match s.widget.kind {
        widget::SLIDER => env.get_int(&format!("{path}.value")),
        _ => env.get_int(&format!("{path}.selectedIndex")),
    }
}

/// `FUN_140502a60` (store) and `FUN_140503100` (compare only): the movie
/// value against the widget's; returns whether they differ.
fn read_back(env: &mut Env, section: &mut Section, out: usize, index: usize, store: bool) -> Result<bool> {
    let kind = section.decl.settings[index].widget.kind;
    match kind {
        widget::KEYBOARD_BINDING => {
            let path = widget_path(&section.decl.settings[index], out);
            let k0 = env.get_int(&format!("{path}.keyCode_0"))? as u32;
            let k1 = env.get_int(&format!("{path}.keyCode_1"))? as u32;
            let w = &mut section.decl.settings[index].widget;
            let changed = w.key_code_0 != k0 || w.key_code_1 != k1;
            if store {
                w.key_code_0 = k0;
                w.key_code_1 = k1;
            }
            Ok(changed)
        }
        widget::GAMMA | widget::CONTROL_SCHEME | widget::DIFFICULTY => link_read(env, section, index, store),
        _ => {
            let v = movie_value(env, section, out, index)?;
            let w = &mut section.decl.settings[index].widget;
            let changed = w.selected != v;
            if store {
                w.selected = v;
            }
            Ok(changed)
        }
    }
}

/// `FUN_140502970`: a cvar into the widget value.
fn load_cvar(env: &Env, s: &mut Setting, cvar: &str, invert: bool) {
    let c = &env.console.cvars.list[env.cvar(cvar)];
    let w = &mut s.widget;
    let numeric = c.flags & (crate::platform::cvars::flag::INT | crate::platform::cvars::flag::FLOAT) != 0;
    w.selected = if matches!(w.kind, widget::SLIDER | widget::GAMMA) && numeric {
        let range = c.max - c.min;
        let span = (w.maximum - w.minimum) as f32;
        let q = if range.abs() < f32::EPSILON { 0.0 } else { span * (c.float - c.min) / range };
        (q + 0.5).floor() as i32 + w.minimum
    } else if c.flags & crate::platform::cvars::flag::BOOL != 0 {
        ((c.int != 0) ^ invert) as i32
    } else if c.flags & crate::platform::cvars::flag::FLOAT != 0 {
        (c.float + 0.5).floor() as i32
    } else {
        c.int
    };
}

/// `FUN_140501b00`: the widget value into a cvar.
fn apply_cvar(env: &mut Env, s: &Setting, cvar: &str, invert: bool) {
    let index = env.cvar(cvar);
    let c = env.console.cvars.list[index].clone();
    let w = &s.widget;
    let numeric = c.flags & (crate::platform::cvars::flag::INT | crate::platform::cvars::flag::FLOAT) != 0;
    let cvars = &mut env.console.cvars;
    if matches!(w.kind, widget::SLIDER | widget::GAMMA) && numeric {
        let span = w.maximum - w.minimum;
        let v = if span == 0 { 0.0 } else { (c.max - c.min) * (w.selected - w.minimum) as f32 / span as f32 } + c.min;
        cvars.set_float(index, v, true);
    } else if c.flags & crate::platform::cvars::flag::BOOL != 0 {
        cvars.set_bool(index, (w.selected == 1) ^ invert, true);
    } else if c.flags & crate::platform::cvars::flag::FLOAT != 0 {
        cvars.set_float(index, w.selected as f32, true);
    } else {
        cvars.set_int(index, w.selected, true);
    }
}

/// Section vfunc +0x90: the link widget's data.
fn link_fill(env: &mut Env, section: &mut Section, out: usize, index: usize) -> Result<()> {
    match section.kind {
        Kind::Controls(_) => controls::link_fill(env, section, out, index),
        Kind::Visuals(_) => visuals::link_fill(env, section, out, index),
        Kind::Gameplay(_) => gameplay::link_fill(env, section, out),
        Kind::Audio => Ok(()),
    }
}

/// Section vfunc +0x98 (store) / +0xa0 (compare).
fn link_read(env: &mut Env, section: &mut Section, index: usize, store: bool) -> Result<bool> {
    match &section.kind {
        Kind::Controls(_) => controls::link_read(env, section, index, store),
        Kind::Visuals(_) => visuals::link_read(env, section, index, store),
        // `FUN_140508b00`: the helper's modified byte.
        Kind::Gameplay(g) => Ok(g.difficulty.modified),
        Kind::Audio => Ok(false),
    }
}

/// `FUN_140501bb0` (vfunc +0x20): `optionsData.currentSection`.
fn fill(env: &mut Env, section: &mut Section) -> Result<()> {
    match section.kind {
        Kind::Visuals(_) => visuals::pre_fill(env, section)?,
        Kind::Audio | Kind::Controls(_) | Kind::Gameplay(_) => {}
    }
    let label = env.text(section.decl.label);
    env.set(&format!("{CURRENT}.label"), Value::from(label.as_str()))?;
    env.set(&format!("{CURRENT}.screenClassName"), Value::from(section.decl.screen_class_name.as_str()))?;
    if !section.decl.categories.is_empty() {
        let names: Vec<String> = section
            .decl
            .categories
            .iter()
            .map(|c| env.text(if c[platform::ALL as usize] != 0 { c[platform::ALL as usize] } else { c[platform::PC as usize] }))
            .collect();
        let array = env.strings_array(names);
        env.set(&format!("{CURRENT}.categories"), array)?;
    }
    // FillSettings (vfunc +0x70).
    let elements = section.elements(env);
    env.set(&format!("{CURRENT}.settings.length"), (elements.len() as f64).into())?;
    for (out, index) in elements {
        let (refresh_enabled, callback, locked) = match &section.kind {
            Kind::Audio | Kind::Gameplay(_) => (false, None, false),
            Kind::Controls(_) => (true, None, false),
            Kind::Visuals(_) => visuals::element_arguments(env, index),
        };
        write_element(env, section, out, index, refresh_enabled, callback, locked)?;
    }
    if let Kind::Visuals(_) = section.kind {
        visuals::after_fill(env, section)?;
    }
    section.filled = true;
    Ok(())
}

/// Vfunc +0x40: cvars and bindings into the widgets.
fn load(env: &mut Env, section: &mut Section) {
    let visible: Vec<usize> = section.elements(env).into_iter().map(|e| e.1).collect();
    for index in visible {
        match section.kind {
            Kind::Audio => audio::load(env, section, index),
            Kind::Controls(_) => controls::load(env, section, index),
            Kind::Visuals(_) => visuals::load(env, section, index),
            Kind::Gameplay(_) => gameplay::load(env, section, index),
        }
    }
}

/// Vfunc +0x38.
fn has_changes(env: &mut Env, section: &mut Section) -> Result<bool> {
    if let Kind::Visuals(_) = section.kind {
        return visuals::has_changes(env, section);
    }
    for (out, index) in section.elements(env) {
        if read_back(env, section, out, index, false)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Vfunc +0x48: the movie values into the widgets and on to the game.
fn apply(env: &mut Env, section: &mut Section) -> Result<Vec<Effect>> {
    match section.kind {
        Kind::Audio => audio::apply(env, section).map(|_| Vec::new()),
        Kind::Controls(_) => controls::apply(env, section),
        Kind::Visuals(_) => visuals::apply(env, section).map(|_| Vec::new()),
        Kind::Gameplay(_) => gameplay::apply(env, section).map(|_| Vec::new()),
    }
}

/// Vfunc +0x50.
fn restore(env: &mut Env, section: &mut Section) -> Result<Vec<Effect>> {
    match section.kind {
        Kind::Audio => audio::restore(env).map(|_| Vec::new()),
        Kind::Controls(_) => controls::restore(env),
        Kind::Visuals(_) => visuals::restore(env, section).map(|_| Vec::new()),
        Kind::Gameplay(_) => gameplay::restore(env, section).map(|_| Vec::new()),
    }
}

/// Vfunc +0x60: `_C_BackFromOptionSection()` (`FUN_1405018b0`).
fn back(env: &mut Env, section: &Section) -> Result<()> {
    if let Kind::Audio = section.kind {
        env.call("gameService", "_C_ClearLocalization", &[])?;
    }
    env.call_root("_C_BackFromOptionSection")
}

impl Options {
    /// State created with the game: the difficulty manager's
    /// initialisation (`FUN_140648460` -> `FUN_140648590`).
    pub fn start(&mut self, env: &mut Env) {
        for s in &mut self.sections {
            if let Kind::Gameplay(g) = &mut s.kind {
                if let Some(m) = g.manager.as_mut() {
                    m.init(env);
                }
            }
        }
    }

    /// `FUN_140516ed0`, the last write of every pause FillData:
    /// `optionsData.sections`.
    pub fn write_sections(&self, env: &mut Env) -> Result<()> {
        env.set("optionsData.sections.length", (self.sections.len() as f64).into())?;
        for (i, s) in self.sections.iter().enumerate() {
            let element = format!("optionsData.sections.data[{i}]");
            let label = env.text(s.decl.label);
            env.set(&format!("{element}.label"), Value::from(label.as_str()))?;
            env.set(&format!("{element}.screenClassName"), Value::from(s.decl.screen_class_name.as_str()))?;
            env.set(&format!("{element}.gameCallbackOnClick"), Value::from("OnOptionSectionSelected"))?;
        }
        Ok(())
    }

    /// Base Close (`FUN_140503650`, visuals `FUN_14050e2a0`).
    fn close(&mut self, env: &mut Env, apply_changes: bool) -> Result<Vec<Effect>> {
        let Some(at) = self.current.take() else { return Ok(Vec::new()) };
        let section = &mut self.sections[at];
        let mut effects = Vec::new();
        if let Kind::Visuals(_) = section.kind {
            if visuals::close(env, section, apply_changes)? {
                self.save_requested = true;
            }
        }
        if apply_changes {
            effects = apply(env, section)?;
            self.save_requested = true;
        } else {
            load(env, section);
        }
        section.filled = false;
        back(env, section)?;
        Ok(effects)
    }

    /// The container's and the sections' movie callbacks. Returns `None`
    /// for a name no options code registers.
    pub fn handle(&mut self, env: &mut Env, name: &str, args: &[Value]) -> Option<Result<Vec<Effect>>> {
        let index = |args: &[Value]| args.first().map(|v| v.primitive_to_number() as i64);
        Some(match name {
            // `FUN_1405171f0`.
            "OnOptionSectionSelected" => (|| {
                let i = index(args).filter(|&i| (0..=3).contains(&i)).ok_or("OnOptionSectionSelected: bad index")? as usize;
                let section = self.sections.get_mut(i).ok_or("OnOptionSectionSelected: no section")?;
                self.current = Some(i);
                load(env, section);
                fill(env, section)?;
                show(env, section)?;
                Ok(Vec::new())
            })(),
            // `FUN_140517110`.
            "OnBackFromSettingsList" => (|| {
                let Some(at) = self.current else { return Ok(Vec::new()) };
                if !has_changes(env, &mut self.sections[at])? {
                    return self.close(env, false);
                }
                self.pending = Some(Pending::LeaveSection);
                Ok(vec![Effect::MessageBox(MessageBox { text: "ui_main_settings_confirmation", buttons: 2 })])
            })(),
            // `FUN_1405172d0`.
            "OnRestoreSettings" => (|| {
                let Some(at) = self.current else { return Ok(Vec::new()) };
                let section = &mut self.sections[at];
                let effects = restore(env, section)?;
                load(env, section);
                fill(env, section)?;
                self.save_requested = true;
                show(env, section)?;
                Ok(effects)
            })(),
            _ => {
                let at = self.current?;
                let section = &mut self.sections[at];
                match section.kind {
                    Kind::Controls(_) => controls::handle(env, section, name, args)?,
                    Kind::Visuals(_) => visuals::handle(env, section, name, args)?,
                    Kind::Gameplay(_) => gameplay::handle(env, section, name, args)?,
                    Kind::Audio => return None,
                }
            }
        })
    }

    /// `FUN_1405171b0`: the confirmation box's answer (button index).
    pub fn answer(&mut self, env: &mut Env, answer: u32) -> Result<Vec<Effect>> {
        match self.pending.take() {
            Some(Pending::LeaveSection) => self.close(env, answer != 0),
            None => Ok(Vec::new()),
        }
    }

    /// `FUN_1405175f0` (pause Update): the current section's per-frame work.
    /// `pressed` is the engine key pressed this frame, if any.
    pub fn update(&mut self, env: &mut Env, pressed: Option<u32>) -> Result<()> {
        let Some(at) = self.current else { return Ok(()) };
        let section = &mut self.sections[at];
        match section.kind {
            Kind::Controls(_) => controls::update(env, section, pressed),
            _ => Ok(()),
        }
    }
}

/// Vfunc +0x30 (`FUN_140503920`): `_C_ShowOptionSection()`.
fn show(env: &mut Env, _section: &Section) -> Result<()> {
    env.call_root("_C_ShowOptionSection")
}

/// The localized text of a key (`FUN_14002a530` on a string id name).
fn text_of(env: &Env, key: &str) -> String {
    env.text(key_id(key))
}
