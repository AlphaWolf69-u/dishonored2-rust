//! The option-section decls (`arkDeclMenuSectionOption` and its four
//! subclasses, layouts in notes-options.md 0.2) read from the installed
//! `generated/decls` resources.
use super::tables;
use crate::assets::entities::{parse_declaration, Value};
use crate::assets::lang::key_id;

pub type Fields = Vec<(String, Value)>;

/// `arkOptionPlatform_t`.
pub mod platform {
    pub const NONE: i32 = 0;
    pub const ALL: i32 = 1;
    pub const PC: i32 = 2;
    pub const BNET: i32 = 6;
}

/// `arkOptionAvailability_t`.
pub mod availability {
    pub const NONE: i32 = 0;
    pub const MAIN_MENU: i32 = 1;
    pub const PAUSE_MENU: i32 = 2;
    pub const ALL: i32 = 3;
}

/// `arkOptionWidgetTypes_t`.
pub mod widget {
    pub const STEPPER: i32 = 0;
    pub const STEPPER_BOOLEAN: i32 = 1;
    pub const SLIDER: i32 = 2;
    pub const DROPBOX: i32 = 3;
    pub const KEYBOARD_BINDING: i32 = 4;
    pub const GAMMA: i32 = 5;
    pub const CONTROL_SCHEME: i32 = 6;
    pub const DIFFICULTY: i32 = 7;
}

/// `arkMenuOptionWidget` (ctor `FUN_140503d10`: every field zero).
#[derive(Clone, Debug, Default)]
pub struct Widget {
    pub kind: i32,
    pub description: u32,
    /// `m_selectedValue`, the value the screens edit.
    pub selected: i32,
    pub key_code_0: u32,
    pub key_code_1: u32,
    pub minimum: i32,
    pub maximum: i32,
    pub interval: i32,
    pub display_value: bool,
    pub string_values: Vec<u32>,
    pub binding_type: i32,
    pub bindable_game_action: i32,
    pub bindable_console_command: String,
    pub locked: bool,
    /// `m_runtimeStringValues`, built by PreFill.
    pub runtime_string_values: Vec<String>,
}

/// `arkMenuOptionSetting` (ctor `FUN_140503b70`).
#[derive(Clone, Debug)]
pub struct Setting {
    pub label: u32,
    /// `m_conditionalLabel {m_strId, m_cvarName}`.
    pub conditional_label: (u32, String),
    pub runtime_label: String,
    pub description: u32,
    pub category_index: u32,
    pub widget: Widget,
    pub runtime_setting: bool,
    pub platform: i32,
    pub availability: i32,
    pub cpu_gpu_dependency_ratio: f32,
}

impl Default for Setting {
    fn default() -> Self {
        Setting {
            label: 0,
            conditional_label: (0, String::new()),
            runtime_label: String::new(),
            description: 0,
            category_index: 0,
            widget: Widget::default(),
            runtime_setting: false,
            platform: platform::NONE,
            availability: availability::ALL,
            cpu_gpu_dependency_ratio: 0.5,
        }
    }
}

/// `arkDeclMenuSectionOption`: the common part; `edit` keeps the
/// subclass fields for the section that reads them.
#[derive(Clone, Debug, Default)]
pub struct SectionDecl {
    pub label: u32,
    pub screen_class_name: String,
    /// Per category, the label of every `arkOptionPlatform_t`.
    pub categories: Vec<[u32; 7]>,
    /// Indexed by the section's setting enum.
    pub settings: Vec<Setting>,
    pub edit: Fields,
}

pub fn field<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a Value> {
    fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

pub fn scope<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a [(String, Value)]> {
    match field(fields, name)? {
        Value::Scope(s) => Some(s),
        _ => None,
    }
}

/// An atom without its quotes.
pub fn atom<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    match field(fields, name)? {
        Value::Atom(a) => Some(unquote(a)),
        _ => None,
    }
}

pub fn unquote(a: &str) -> &str {
    a.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(a)
}

/// `enumItem[ NAME ]` -> `NAME`.
pub fn enum_key(key: &str) -> Option<&str> {
    Some(key.strip_prefix("enumItem")?.trim().strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// The value of an enum item name in a reflected table.
pub fn enum_value(table: &[(&str, i32)], name: &str) -> Option<i32> {
    table.iter().find(|(n, _)| *n == name).map(|&(_, v)| v)
}

/// The scopes of an `enumItem[...]`-indexed field, by enum value.
pub fn enum_items<'a>(fields: &'a [(String, Value)], table: &[(&str, i32)]) -> Vec<(i32, &'a Value)> {
    fields.iter().filter_map(|(k, v)| Some((enum_value(table, enum_key(k)?)?, v))).collect()
}

/// An `idList`: `num`, then `item[i]`.
pub fn list_items<'a>(fields: &'a [(String, Value)]) -> Vec<&'a Value> {
    let count = atom(fields, "num").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    (0..count).filter_map(|i| field(fields, &format!("item[{i}]"))).collect()
}

pub fn string_id(fields: &[(String, Value)], name: &str) -> u32 {
    atom(fields, name).map_or(0, key_id)
}

pub fn int(fields: &[(String, Value)], name: &str, default: i32) -> i32 {
    atom(fields, name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

pub fn float(fields: &[(String, Value)], name: &str, default: f32) -> f32 {
    atom(fields, name).and_then(|v| v.trim_end_matches('f').parse().ok()).unwrap_or(default)
}

pub fn boolean(fields: &[(String, Value)], name: &str, default: bool) -> bool {
    match atom(fields, name) {
        Some(v) if v.eq_ignore_ascii_case("true") || v == "1" => true,
        Some(v) if v.eq_ignore_ascii_case("false") || v == "0" => false,
        _ => default,
    }
}

fn enum_field(fields: &[(String, Value)], name: &str, table: &[(&str, i32)], default: i32) -> i32 {
    atom(fields, name).and_then(|v| enum_value(table, v)).unwrap_or(default)
}

fn widget(fields: &[(String, Value)]) -> Widget {
    let strings = |name: &str| -> Vec<u32> {
        scope(fields, name)
            .map(|s| list_items(s).into_iter().filter_map(|v| match v {
                Value::Atom(a) => Some(key_id(unquote(a))),
                _ => None,
            }).collect())
            .unwrap_or_default()
    };
    Widget {
        kind: enum_field(fields, "m_type", tables::WIDGET_TYPES, widget::STEPPER),
        description: string_id(fields, "m_description"),
        selected: int(fields, "m_selectedValue", 0),
        minimum: int(fields, "m_minimum", 0),
        maximum: int(fields, "m_maximum", 0),
        interval: int(fields, "m_interval", 0),
        display_value: boolean(fields, "m_displayValue", false),
        string_values: strings("m_stringValues"),
        binding_type: enum_field(fields, "m_bindingType", tables::BINDING_TYPES, 0),
        bindable_game_action: enum_field(fields, "m_bindableGameAction", tables::GAME_ACTIONS, 0),
        bindable_console_command: atom(fields, "m_bindableConsoleCommand").unwrap_or("").to_string(),
        locked: boolean(fields, "m_locked", false),
        ..Widget::default()
    }
}

fn setting(fields: &[(String, Value)]) -> Setting {
    let conditional = scope(fields, "m_conditionalLabel").unwrap_or(&[]);
    Setting {
        label: string_id(fields, "m_label"),
        conditional_label: (string_id(conditional, "m_strId"), atom(conditional, "m_cvarName").unwrap_or("").to_string()),
        runtime_label: atom(fields, "m_runtimeLabel").unwrap_or("").to_string(),
        description: string_id(fields, "m_description"),
        category_index: int(fields, "m_categoryIndex", 0) as u32,
        widget: scope(fields, "m_widget").map(widget).unwrap_or_default(),
        runtime_setting: boolean(fields, "m_runtimeSetting", false),
        platform: enum_field(fields, "m_platform", tables::PLATFORMS, platform::NONE),
        availability: enum_field(fields, "m_availability", tables::AVAILABILITY, availability::ALL),
        cpu_gpu_dependency_ratio: float(fields, "m_CPUGPUDependencyRatio", 0.5),
    }
}

/// The root `edit` scope of an installed decl.
pub fn read_edit(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>, kind: &str, name: &str) -> Result<Fields, String> {
    let path = format!("generated/decls/{kind}/{name}.decl");
    let bytes = read(&path).ok_or_else(|| format!("decl missing: {path}"))?;
    let text = crate::assets::entities::declaration_text(&bytes);
    let mut fields = parse_declaration(&text).map_err(|e| format!("{path}: {e}"))?;
    let at = fields.iter().position(|(k, _)| k == "edit").ok_or_else(|| format!("{path}: no edit scope"))?;
    match fields.swap_remove(at).1 {
        Value::Scope(edit) => Ok(edit),
        Value::Atom(_) => Err(format!("{path}: edit is not a scope")),
    }
}

/// A section decl; `settings_enum` sizes and indexes `m_settings`.
pub fn section(edit: Fields, settings_enum: &[(&str, i32)]) -> SectionDecl {
    let categories = scope(&edit, "m_categories")
        .map(|c| {
            list_items(c)
                .into_iter()
                .map(|item| {
                    let mut labels = [0u32; 7];
                    if let Value::Scope(s) = item {
                        for (p, v) in enum_items(s, tables::PLATFORMS) {
                            if let (Some(slot), Value::Atom(a)) = (labels.get_mut(p as usize), v) {
                                *slot = key_id(unquote(a));
                            }
                        }
                    }
                    labels
                })
                .collect()
        })
        .unwrap_or_default();
    let mut settings = vec![Setting::default(); settings_enum.len()];
    if let Some(s) = scope(&edit, "m_settings") {
        for (i, v) in enum_items(s, settings_enum) {
            if let (Some(slot), Value::Scope(f)) = (settings.get_mut(i as usize), v) {
                *slot = setting(f);
            }
        }
    }
    SectionDecl {
        label: string_id(&edit, "m_label"),
        screen_class_name: atom(&edit, "m_screenClassNameAS3").unwrap_or("").to_string(),
        categories,
        settings,
        edit,
    }
}
