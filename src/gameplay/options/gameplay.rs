//! `arkMenuSectionOptionGameplay` (vtable 0x141b70ca8) and the parts of the
//! difficulty manager (`gameLocal+0x1f7b90`) it drives, notes-options.md 7.
use super::decl;
use super::{apply_cvar, load_cvar, read_back, Effect, Env, Kind, Result, Section, CURRENT};
use crate::assets::entities::Value as DeclValue;
use crate::flash::avm2::value::Value;

/// `ARK_GPSETTING_DIFFICULTY`.
const DIFFICULTY: usize = 0;
/// `arkDifficultyManager::DIFFICULTY_CUSTOM`.
pub const CUSTOM: i32 = 4;
/// Values per preset (`+0x68`, 24 ints).
const VALUES: usize = 24;
/// The rows the custom-difficulty screen shows (`uiSlot` 0..21).
const ROWS: usize = 22;

/// The cvar and inversion of each setting (Load `FUN_1405096d0` / Apply
/// `FUN_140509180`).
fn cvar(index: usize) -> Option<(&'static str, bool)> {
    Some(match index {
        1 => ("ark_inventory_mana_potion_autouse", false),
        2 => ("ark_autosave_in_journal", false),
        3 => ("ark_killcam_chance_setting", false),
        4 => ("ark_playerBobAmount", false),
        5 => ("ark_ui_show_tutorial_messages", false),
        6 => ("ark_ui_crosshair_style", false),
        7 => ("ark_ui_crosshair_opacity", false),
        8 => ("ark_ui_show_master_hud", false),
        9 => ("ark_ui_show_interactions", false),
        10 => ("ark_ui_show_objective_notifications", false),
        11 => ("ark_ui_show_notifications", false),
        12 => ("r_skipOutlines", true),
        13 => ("ark_ui_show_pickup_log", false),
        14 => ("ark_ui_show_contextual_icons", false),
        15 => ("ark_ui_show_information_messages", false),
        16 => ("ark_ui_show_location_discovery", false),
        17 => ("ark_ui_show_target_notifications", false),
        18 => ("ark_ui_show_heart_markers", false),
        19 => ("ark_ui_show_awareness_markers", false),
        20 => ("ark_ui_show_firelock_markers", false),
        21 => ("ark_ui_show_snap_reaction_gauge", false),
        22 => ("ark_ui_show_grenade_markers", false),
        23 => ("ark_ui_show_grenade_cooking_gauge", false),
        24 => ("ark_ui_show_objective_markers", false),
        25 => ("ark_ui_show_shortcuts", false),
        _ => return None,
    })
}

/// The preset/uiSlot field of each value index (struct order; the custom
/// table loader `FUN_140648590` reads them in this order). Index 22 has no
/// field the decls or the loader name.
const SLOT_FIELDS: [&str; VALUES] = [
    "m_ennemyDamage",
    "m_ennemyPerception",
    "m_sleepDartEfficiency",
    "m_potionEffect",
    "m_potionSpeed",
    "m_searchEfficiency",
    "m_rangedAccuracy",
    "m_alarmCount",
    "m_ammoQuantity",
    "m_panicChance",
    "m_attackFequency",
    "m_attackSlots",
    "m_leanVisibility",
    "m_npcVisionHeight",
    "m_playerStepNoise",
    "m_allowManaReplenish",
    "m_limitHealthRegen",
    "m_limitLean",
    "m_lightVisibility",
    "m_saveModel",
    "m_realTimeHud",
    "m_chaseEfficiency",
    "",
    "m_mergeHealthMana",
];

/// The profile keys of the custom values (`FUN_140648590`), by value index.
const CUSTOM_KEYS: [(usize, &str); 22] = [
    (0, "difficultyEnemyDamage"),
    (1, "difficultyEnemyPerception"),
    (2, "difficultySleepDartEfficiency"),
    (3, "difficultyPotionEffect"),
    (4, "difficultyPotionSpeed"),
    (5, "difficultySearchEfficiency"),
    (6, "difficultyRangedAccuracy"),
    (7, "difficultyAlarmCount"),
    (8, "difficultyAmmoQuantity"),
    (9, "difficultyPanicChance"),
    (10, "difficultyAttackFequency"),
    (11, "difficultyAttackSlots"),
    (12, "difficultyLeanVisibility"),
    (13, "difficultyNpcVisionHeight"),
    (14, "difficultyPlayerStepNoise"),
    (15, "difficultyManaReplenish"),
    (16, "difficultyHealthRegen"),
    (17, "difficultyLimitedLean"),
    (23, "difficultyMergeHealthMana"),
    (18, "difficultyLightVisibility"),
    (20, "difficultyRealTimeHud"),
    (21, "difficultyChaseEfficiency"),
];

/// `arkDeclDifficultyPreset` ctor (`FUN_140647550`) and the static custom
/// preset (`FUN_141387350`) defaults.
const PRESET_DEFAULTS: [i32; VALUES] = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0];

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: u32,
    pub values: [i32; VALUES],
}

/// `uiSlotInformation_t` (0x24) and `m_difficultyStepper`.
#[derive(Clone, Debug, Default)]
pub struct Slot {
    pub label: u32,
    pub category: u32,
    pub description: u32,
    pub values_description: u32,
    pub levels: [u32; 3],
    pub availability: i32,
    pub priority: i32,
}

/// `arkDifficultyManager` and its decl.
#[derive(Clone, Debug)]
pub struct Manager {
    pub presets: Vec<Preset>,
    pub custom_name: u32,
    pub stepper: Slot,
    pub slots: Vec<Slot>,
    /// `DAT_1439889f0`: the custom preset.
    pub custom: [i32; VALUES],
    /// `mgr+4`.
    pub current: i32,
}

fn slot(fields: &[(String, DeclValue)]) -> Slot {
    Slot {
        label: decl::string_id(fields, "m_label"),
        category: decl::string_id(fields, "m_category"),
        description: decl::string_id(fields, "m_description"),
        values_description: decl::string_id(fields, "m_valuesDescription"),
        levels: [decl::string_id(fields, "m_level1"), decl::string_id(fields, "m_level2"), decl::string_id(fields, "m_level3")],
        availability: decl::atom(fields, "m_availability")
            .and_then(|v| decl::enum_value(super::tables::AVAILABILITY, v))
            .unwrap_or(decl::availability::ALL),
        priority: decl::int(fields, "m_priority", 0),
    }
}

/// `DIFFICULTY_LVL1..3` (the reflected level enum is 0..2).
fn level(fields: &[(String, DeclValue)], name: &str, default: i32) -> i32 {
    match decl::atom(fields, name) {
        Some("DIFFICULTY_LVL1") => 0,
        Some("DIFFICULTY_LVL2") => 1,
        Some("DIFFICULTY_LVL3") => 2,
        _ => default,
    }
}

impl Manager {
    pub fn read(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<Manager> {
        let edit = decl::read_edit(read, "difficultymanager", "difficulty/difficultysettings.difficultymanager")?;
        let mut presets = Vec::new();
        for item in decl::scope(&edit, "m_presets").map(decl::list_items).unwrap_or_default() {
            let DeclValue::Atom(name) = item else { continue };
            let name = decl::unquote(name);
            let kind = "difficultypreset";
            let p = decl::read_edit(read, kind, name)?;
            let mut values = PRESET_DEFAULTS;
            for (i, field) in SLOT_FIELDS.iter().enumerate() {
                if !field.is_empty() {
                    values[i] = level(&p, field, values[i]);
                }
            }
            presets.push(Preset { name: decl::string_id(&p, "m_presetName"), values });
        }
        let ui = decl::scope(&edit, "m_uiInformation").unwrap_or(&[]);
        let slots = SLOT_FIELDS[..ROWS + 1]
            .iter()
            .map(|f| decl::scope(ui, if f.is_empty() { "m_mergeHealthMana" } else { f }).map(slot).unwrap_or_default())
            .collect();
        Ok(Manager {
            presets,
            custom_name: decl::string_id(ui, "m_customDifficultyName"),
            stepper: decl::scope(ui, "m_difficultyStepper").map(slot).unwrap_or_default(),
            slots,
            custom: PRESET_DEFAULTS,
            current: 0,
        })
    }

    /// `FUN_140648590`: the current preset (the override cvar, else the
    /// profile's `difficultyPresetIndex_<campaign>`, 1 when missing) and the
    /// custom values, then SetDifficulty.
    pub fn init(&mut self, env: &mut Env) {
        let suffix = format!("_{:02}", env.profile_int("campaignIndex"));
        let key = |name: &str| format!("{name}{suffix}");
        let override_index = env.cvar("ark_difficulty_level_override");
        let forced = env.console.cvars.list[override_index].int;
        self.current = if forced < 0 {
            env.profile.get(&key("difficultyPresetIndex")).map_or(1, |v| crate::platform::cvars::atoi(v))
        } else {
            env.console.cvars.set_int(override_index, -1, true);
            forced
        };
        if self.current == CUSTOM {
            for (i, name) in CUSTOM_KEYS {
                self.custom[i] = env.profile.get(&key(name)).map_or(0, |v| crate::platform::cvars::atoi(v));
            }
        }
        self.custom[19] = env.profile.get(&key("difficultySaveModel")).map_or(0, |v| crate::platform::cvars::atoi(v));
        self.set_difficulty(env, self.current);
    }

    /// `FUN_140648f20`. The entity attribute reset/apply (`FUN_1406491f0`,
    /// `FUN_14060b970`) and the profile dictionary writes (`FUN_1406492c0`)
    /// are not ported (blockers in docs/flash.md); the index and the cvar are.
    pub fn set_difficulty(&mut self, env: &mut Env, index: i32) {
        self.current = index;
        let cvar = env.cvar("ark_difficulty_current");
        env.console.cvars.set_int(cvar, index, true);
    }

    fn preset(&self, index: i32) -> &[i32; VALUES] {
        if index >= 0 && (index as usize) < self.presets.len() {
            &self.presets[index as usize].values
        } else {
            &self.custom
        }
    }

    /// `FUN_140648350`.
    pub fn value(&self, env: &Env, difficulty: i32, slot: usize) -> i32 {
        if self.slots[slot].availability == decl::availability::ALL || env.main_menu_map {
            self.preset(difficulty)[slot]
        } else {
            self.custom[slot]
        }
    }

    /// `FUN_140648300`: the name of a preset, the custom name otherwise.
    pub fn name(&self, index: i32) -> u32 {
        if index >= 0 && (index as usize) < self.presets.len() {
            self.presets[index as usize].name
        } else {
            self.custom_name
        }
    }
}

/// `idSort_Quick::Sort` (`FUN_1403949a0`): idLib's quicksort with an
/// explicit stack and a selection sort below four elements.
pub fn id_sort_quick<T: Copy>(a: &mut [T], cmp: impl Fn(&T, &T) -> i32) {
    if a.len() < 2 {
        return;
    }
    let mut lo_stack = [0i64; 128];
    let mut hi_stack = [0i64; 128];
    let mut depth: i64 = 0;
    let mut lo: i64 = 0;
    let mut hi: i64 = a.len() as i64 - 1;
    let at = |i: i64| i as usize;
    loop {
        while hi - lo > 3 && depth < 0x7f {
            a.swap(at(hi), at((hi + lo) / 2));
            let mut i = lo;
            let mut j = hi - 1;
            loop {
                while cmp(&a[at(i)], &a[at(hi)]) < 0 {
                    i += 1;
                    if i >= j {
                        break;
                    }
                }
                let mut met = false;
                while cmp(&a[at(j)], &a[at(hi)]) > 0 {
                    j -= 1;
                    if j <= i {
                        met = true;
                        break;
                    }
                }
                if met || j <= i {
                    break;
                }
                a.swap(at(i), at(j));
                j -= 1;
                i += 1;
                if i >= j {
                    break;
                }
            }
            let pivot = a[at(hi)];
            while cmp(&a[at(j)], &pivot) >= 0 && lo < j {
                j -= 1;
            }
            while cmp(&a[at(i)], &pivot) <= 0 && i < hi {
                i += 1;
            }
            a[at(hi)] = a[at(i)];
            a[at(i)] = pivot;
            if j - lo < hi - i {
                lo_stack[at(depth)] = i;
                hi_stack[at(depth)] = hi;
                depth += 1;
                hi = j;
            } else {
                lo_stack[at(depth)] = lo;
                hi_stack[at(depth)] = j;
                depth += 1;
                lo = i;
            }
        }
        while lo < hi {
            let mut max = lo;
            for k in lo + 1..=hi {
                if cmp(&a[at(k)], &a[at(max)]) > 0 {
                    max = k;
                }
            }
            a.swap(at(max), at(hi));
            hi -= 1;
        }
        depth -= 1;
        if depth < 0 {
            return;
        }
        lo = lo_stack[at(depth)];
        hi = hi_stack[at(depth)];
    }
}

/// The custom-difficulty helper (`arkUIDifficultyScreen`, section+0x230).
#[derive(Debug, Default)]
pub struct Difficulty {
    /// Rows 1..22: the uiSlot each shows (`FUN_1403934c0`).
    pub sorted: Vec<usize>,
    /// Committed stepper values, preset row first.
    pub committed: Vec<i32>,
    /// `+0x50`.
    pub modified: bool,
}

#[derive(Debug)]
pub struct State {
    pub manager: Option<Manager>,
    pub difficulty: Difficulty,
}

impl State {
    pub fn new(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<State> {
        let manager = Manager::read(read)?;
        let mut sorted: Vec<usize> = (0..ROWS).collect();
        id_sort_quick(&mut sorted, |&a, &b| manager.slots[a].priority - manager.slots[b].priority);
        Ok(State { manager: Some(manager), difficulty: Difficulty { sorted, committed: vec![0; ROWS + 1], modified: false } })
    }
}

fn state(section: &mut Section) -> &mut State {
    match &mut section.kind {
        Kind::Gameplay(s) => s,
        _ => unreachable!("gameplay state"),
    }
}

const ROWS_PATH: &str = "optionsData.customDifficultyData.difficulties";

fn row_stepper(row: usize) -> String {
    format!("{ROWS_PATH}.data[{row}].optionStepper")
}

/// The difficulty manager, taken out while `env` is borrowed with it.
fn manager(_env: &mut Env, section: &mut Section) -> Manager {
    state(section).manager.take().expect("difficulty manager")
}

/// `FUN_140393860`: `customDifficultyData.difficulties`.
fn fill_rows(env: &mut Env, section: &mut Section) -> Result<()> {
    let m = manager(env, section);
    let current = env.console.cvars.int("ark_difficulty_current").unwrap_or(0);
    let sorted = state(section).difficulty.sorted.clone();
    env.set(&format!("{ROWS_PATH}.length"), ((ROWS + 1) as f64).into())?;
    let mut committed = vec![0; ROWS + 1];
    let write_texts = |env: &mut Env, row: usize, s: &Slot| -> Result<()> {
        let element = format!("{ROWS_PATH}.data[{row}]");
        for (member, id) in [("label", s.label), ("categoryName", s.category), ("description", s.description), ("descriptionValues", s.values_description)] {
            let text = env.text(id);
            env.set(&format!("{element}.{member}"), Value::from(text.as_str()))?;
        }
        Ok(())
    };
    write_texts(env, 0, &m.stepper)?;
    let stepper = row_stepper(0);
    let mut names: Vec<String> = m.presets.iter().map(|p| env.text(p.name)).collect();
    names.push(env.text(m.custom_name));
    let values = env.strings_array(names);
    env.set(&format!("{stepper}.values"), values)?;
    env.set(&format!("{stepper}.updateGameCallback"), Value::from("OnDifficultyChange"))?;
    env.set(&format!("{stepper}.refreshEnabled"), true.into())?;
    env.set(&format!("{stepper}.selectedIndex"), (current as f64).into())?;
    committed[0] = current;
    for (k, &slot) in sorted.iter().enumerate() {
        let row = k + 1;
        let s = &m.slots[slot];
        write_texts(env, row, s)?;
        let stepper = row_stepper(row);
        env.set(&format!("{stepper}.refreshEnabled"), true.into())?;
        let levels: Vec<String> = s.levels.iter().enumerate().filter(|(i, &id)| *i < 2 || id != 0).map(|(_, &id)| env.text(id)).collect();
        let values = env.strings_array(levels);
        env.set(&format!("{stepper}.values"), values)?;
        env.set(&format!("{stepper}.updateGameCallback"), Value::from("OnDifficultySettingChange"))?;
        let v = m.value(env, current, slot);
        env.set(&format!("{stepper}.selectedIndex"), (v as f64).into())?;
        committed[row] = v;
        // The path prefix (helper+0x20) is empty in the options section.
        let locked = s.availability == decl::availability::NONE || s.availability == decl::availability::MAIN_MENU;
        if locked {
            env.set(&format!("{stepper}.locked"), true.into())?;
        }
    }
    let st = state(section);
    st.difficulty.committed = committed;
    st.manager = Some(m);
    Ok(())
}

/// Vfunc +0x90 (`FUN_140508770` -> `FUN_140509090`).
pub fn link_fill(env: &mut Env, section: &mut Section, out: usize) -> Result<()> {
    let link = format!("{CURRENT}.settings.data[{out}].link");
    env.set(&format!("{link}.as3callback"), Value::from("_C_ShowCustomDifficulty"))?;
    env.set(&format!("{link}.gameCallbackConfirmation"), Value::from("OnDifficultyModifiedConfirmation"))?;
    env.set(&format!("{link}.gameCallbackCancel"), Value::from("OnDifficultyCancel"))?;
    let m = manager(env, section);
    let current = env.console.cvars.int("ark_difficulty_current").unwrap_or(0);
    let name = env.text(m.name(current));
    state(section).manager = Some(m);
    env.set(&format!("{link}.currentValue"), Value::from(name.as_str()))?;
    // `FUN_140393800`: helper fill (registers OnDifficultyChange and
    // OnDifficultySettingChange for this file).
    state(section).difficulty.modified = false;
    fill_rows(env, section)
}

pub fn load(env: &mut Env, section: &mut Section, index: usize) {
    if let Some((name, invert)) = cvar(index) {
        load_cvar(env, &mut section.decl.settings[index], name, invert);
    }
}

fn preset_row(env: &mut Env) -> Result<i32> {
    env.get_int(&format!("{}.selectedIndex", row_stepper(0)))
}

/// `FUN_1405094d0`.
pub fn apply(env: &mut Env, section: &mut Section) -> Result<bool> {
    let mut any = false;
    let elements = section.elements(env);
    for &(out, index) in &elements {
        if read_back(env, section, out, index, true)? {
            any = true;
            if index == DIFFICULTY {
                // `FUN_140394bc0(helper, 0)`: a custom preset row copies the rows.
                if preset_row(env)? == CUSTOM {
                    let sorted = state(section).difficulty.sorted.clone();
                    for (k, &slot) in sorted.iter().enumerate() {
                        let v = env.get_int(&format!("{}.selectedIndex", row_stepper(k + 1)))?;
                        if let Some(m) = state(section).manager.as_mut() {
                            m.custom[slot] = v;
                        }
                    }
                }
            } else if let Some((name, invert)) = cvar(index) {
                let s = section.decl.settings[index].clone();
                apply_cvar(env, &s, name, invert);
            }
        }
    }
    if elements.iter().any(|e| e.1 == DIFFICULTY) {
        let p = preset_row(env)?;
        let mut m = manager(env, section);
        m.set_difficulty(env, p);
        state(section).manager = Some(m);
    }
    Ok(any)
}

/// `FUN_140508f40`: cvars 5..24, the current difficulty, then 1..4.
/// `ark_ui_show_shortcuts` is not reset.
pub fn restore(env: &mut Env, section: &mut Section) -> Result<()> {
    let mut names: Vec<&str> = (5..=24).filter_map(|i| cvar(i).map(|c| c.0)).collect();
    names.push("ark_difficulty_current");
    names.extend((1..=4).filter_map(|i| cvar(i).map(|c| c.0)));
    for name in names {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    // `FUN_140394240`: the preset row and its rows from the cvar.
    let current = env.console.cvars.int("ark_difficulty_current").unwrap_or(0);
    set_if_different(env, 0, current)?;
    refill_rows(env, section, current)?;
    state(section).difficulty.modified = false;
    // `FUN_140508b10`.
    let p = preset_row(env)?;
    let mut m = manager(env, section);
    m.set_difficulty(env, p);
    state(section).manager = Some(m);
    Ok(())
}

fn set_if_different(env: &mut Env, row: usize, v: i32) -> Result<()> {
    let stepper = row_stepper(row);
    if env.get_int(&format!("{stepper}.selectedIndex"))? != v {
        env.set(&format!("{stepper}.selectedIndex"), (v as f64).into())?;
        env.call(&stepper, "refresh", &[])?;
    }
    Ok(())
}

/// `FUN_140394c70`: rows 1..22 from a preset.
fn refill_rows(env: &mut Env, section: &mut Section, preset: i32) -> Result<()> {
    let sorted = state(section).difficulty.sorted.clone();
    let m = manager(env, section);
    let values: Vec<i32> = sorted.iter().map(|&slot| m.value(env, preset, slot)).collect();
    state(section).manager = Some(m);
    for (k, v) in values.into_iter().enumerate() {
        set_if_different(env, k + 1, v)?;
    }
    Ok(())
}

pub fn handle(env: &mut Env, section: &mut Section, name: &str, _args: &[Value]) -> Option<Result<Vec<Effect>>> {
    if !matches!(name, "OnDifficultyChange" | "OnDifficultySettingChange" | "OnDifficultyModifiedConfirmation" | "OnDifficultyCancel") {
        return None;
    }
    Some((|| -> Result<Vec<Effect>> {
        match name {
            // `FUN_140394200`.
            "OnDifficultyChange" => {
                state(section).difficulty.modified = true;
                let p = preset_row(env)?;
                refill_rows(env, section, p)?;
            }
            // `LAB_140394230`.
            "OnDifficultySettingChange" => {
                state(section).difficulty.modified = true;
                set_if_different(env, 0, CUSTOM)?;
            }
            // `FUN_140508bf0`.
            "OnDifficultyModifiedConfirmation" => {
                let p = preset_row(env)?;
                let m = manager(env, section);
                let name = env.text(m.name(p));
                state(section).manager = Some(m);
                env.set(&format!("{CURRENT}.settings.data[0].link.currentValue"), Value::from(name.as_str()))?;
                let mut committed = Vec::with_capacity(ROWS + 1);
                for row in 0..=ROWS {
                    committed.push(env.get_int(&format!("{}.selectedIndex", row_stepper(row)))?);
                }
                state(section).difficulty.committed = committed;
                env.call_root("_C_BackFromOptionSection")?;
            }
            // `FUN_140508bc0`.
            _ => {
                let committed = state(section).difficulty.committed.clone();
                for (row, v) in committed.into_iter().enumerate() {
                    set_if_different(env, row, v)?;
                }
                env.call_root("_C_BackFromOptionSection")?;
            }
        }
        Ok(Vec::new())
    })())
}

#[cfg(test)]
mod tests {
    use super::id_sort_quick;

    #[test]
    fn id_sort_orders_by_priority() {
        let priorities = [9, 1, 5, 17, 16, 7, 12, 8, 15, 13, 11, 10, 3, 2, 6, 14, 8, 4, 19, 0, 0, 18];
        let mut rows: Vec<usize> = (0..priorities.len()).collect();
        id_sort_quick(&mut rows, |&a, &b| priorities[a] - priorities[b]);
        let sorted: Vec<i32> = rows.iter().map(|&r| priorities[r]).collect();
        let mut expected = priorities.to_vec();
        expected.sort();
        assert_eq!(sorted, expected);
    }
}
