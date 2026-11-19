//! Native 14-slot shortcuts (4de010/4de1e0/4de730/4de920/4df4e0).
use super::super::DiscoveredDocuments;
use crate::{assets::entities::{scene_motion::{atom, child}, Value}, gameplay::player::weapons::Equipment};
use std::sync::Arc;

#[derive(Clone, Copy, Default)]
struct SlotSettings { weapon: bool, power: bool, heart: bool, priority: i32 }
pub(crate) struct ShortcutSettings { slots: [SlotSettings; 14] }
impl ShortcutSettings {
    pub fn read(controller: &[(String, Value)]) -> Result<Self, String> {
        const KEYS: [&str; 14] = ["CONTROLLER_SHORTCUT_1", "CONTROLLER_SHORTCUT_2", "CONTROLLER_SHORTCUT_3",
            "CONTROLLER_SHORTCUT_4", "CONTROLLER_SHORTCUT_5", "CONTROLLER_SHORTCUT_6", "CONTROLLER_SHORTCUT_7",
            "CONTROLLER_SHORTCUT_8", "CONTROLLER_SHORTCUT_9", "CONTROLLER_SHORTCUT_0", "CONTROLLER_QUICK_SHORTCUT_RIGHT",
            "CONTROLLER_QUICK_SHORTCUT_UP", "CONTROLLER_QUICK_SHORTCUT_LEFT", "CONTROLLER_QUICK_SHORTCUT_DOWN"];
        let fields = child(controller, "m_shortcutsSettings");
        let mut slots = [SlotSettings::default(); 14];
        for (index, key) in KEYS.into_iter().enumerate() {
            let fields = child(fields, &format!("enumItem[{key}]"));
            let boolean = |name| match atom(fields, name) {
                None | Some("false" | "0") => Ok(false), Some("true" | "1") => Ok(true),
                Some(value) => Err(format!("invalid shortcut {name}: {value}")),
            };
            slots[index] = SlotSettings { weapon: boolean("m_weapon")?, power: boolean("m_power")?, heart: boolean("m_heart")?,
                priority: atom(fields, "m_priority").unwrap_or("0").parse().map_err(|_| "invalid shortcut priority")? };
        }
        Ok(Self { slots })
    }
}

#[derive(Default, Clone)]
pub(crate) struct ShortcutSlot { equipment: Option<String>, ammo: Option<String>, assigned_by_player: bool }

pub(crate) struct ShortcutSelection {
    pub equipment: String,
    pub ammunition: Option<String>,
    pub equipment_entity: Option<String>,
}

impl DiscoveredDocuments {
    pub(crate) fn wheel_shortcut_pair(&self,index:usize)->(Option<&str>,Option<&str>) {
        self.shortcuts.get(index).map_or((None,None),|slot|(slot.equipment.as_deref(),slot.ammo.as_deref()))
    }
    ///4f6c20 ->4de400(slot,0): clear an unresolved UI pair without
    ///marking it explicitly assigned/cleared by the player.
    pub(crate) fn wheel_prune_shortcut(&mut self,index:usize){
        if let Some(slot)=self.shortcuts.get_mut(index){*slot=ShortcutSlot::default();}
    }
    pub fn configure_shortcuts(&mut self, settings: Arc<ShortcutSettings>) { self.shortcut_settings = Some(settings); }

    /// 619380 appends ranged declaration's actual equipment/ammo pairs.
    pub fn bind_ammunition(&mut self, equipment: &str, ammunition: &str) {
        let pair = (super::key(equipment), super::key(ammunition));
        if !self.ammunition_dependencies.contains(&pair) { self.ammunition_dependencies.push(pair); }
    }

    /// 4df4e0 autoassigns only empty slots never explicitly cleared/assigned
    /// by the player. Equipment archetype comes from its actual component.
    pub fn admit_shortcut_item(&mut self, path: &str, equipment: Option<&Equipment>) {
        let Some(settings) = self.shortcut_settings.clone() else { return };
        let Some(item) = self.declaration(path) else { return };
        if item.hud_index == 0 { return; }
        let item_type = item.type_number;
        let candidates: Vec<_> = (0..14).filter(|&index| {
            let slot = &self.shortcuts[index];
            if slot.equipment.is_some() || slot.ammo.is_some() || slot.assigned_by_player { return false; }
            let settings = settings.slots[index];
            match equipment.map(|item| item.archetype) {
                None if item_type == 4 => settings.weapon,
                Some(2..=4) => settings.weapon,
                Some(5) => settings.power,
                Some(7) => settings.heart,
                Some(9) => true,
                _ => false,
            }
        }).collect();
        for range in [0..10, 10..14] {
            // 4de5f9/4de600 use SETNZ on priority, comparing booleans,
            // not the declared numeric priorities (confirmed disassembly).
            let mut selected = None;
            for &index in &candidates {
                if !range.contains(&index) { continue; }
                if selected.is_none_or(|previous: usize| settings.slots[index].priority != 0 && settings.slots[previous].priority == 0) {
                    selected = Some(index);
                }
            }
            if let Some(index) = selected { self.assign_shortcut(index, path, false); }
        }
    }

    fn assign_shortcut(&mut self, index: usize, path: &str, assigned_by_player: bool) -> bool {
        let path = super::key(path);
        let pair = if let Some((equipment, ammunition)) = self.ammunition_dependencies.iter().find(|(_, ammo)| *ammo == path) {
            if self.quantity(equipment) == 0 || self.quantity(ammunition) == 0 { return false; }
            (equipment.clone(), Some(ammunition.clone()))
        } else {
            if self.quantity(&path) == 0 { return false; }
            (path, None)
        };
        let range = if index < 10 { 0..10 } else { 10..14 };
        if range.clone().any(|slot| self.shortcuts[slot].equipment.as_deref() == Some(pair.0.as_str()) && self.shortcuts[slot].ammo == pair.1) { return false; }
        self.shortcuts[index] = ShortcutSlot { equipment: Some(pair.0), ammo: pair.1, assigned_by_player };
        true
    }

    /// 4df4b0 assigns a concrete inventory pair from the wheel. Index is
    /// the native shortcut enum (0..9 keyboard,10..13 controller).
    pub fn set_shortcut(&mut self, index: usize, equipment: Option<&str>, ammunition: Option<&str>) -> bool {
        let Some(slot) = self.shortcuts.get_mut(index) else { return false };
        *slot = ShortcutSlot { equipment: equipment.map(super::key), ammo: ammunition.map(super::key), assigned_by_player: true };
        true
    }

    /// 4de920 action mapping and 4de730 keyboard cycling. Returns inventory
    /// identities for the equipment request owner, not an activated slot.
    pub(crate) fn preview_shortcut(&mut self, action: i32) {
        if self.shortcut_action == Some(action) && !matches!(action, 23 | 24) { return; }
        self.shortcut_action = Some(action);
        self.shortcut_input = self.shortcut_input.wrapping_add(1);
        self.shortcut_delay = if matches!(action, 23 | 24) {
            crate::platform::console::shared().read().cvars.float("ark_player_shortcut_kb_display_duration").unwrap_or_default()
        } else { 0. };
        self.selected_shortcut = self.shortcut_index(action);
    }

    pub(crate) fn shortcut_preview(&self) -> Option<usize> {
        self.shortcut_action.and(self.selected_shortcut)
    }

    pub(crate) fn release_shortcut(&mut self, action: i32) -> Option<ShortcutSelection> {
        if self.shortcut_action != Some(action) || self.shortcut_delay > 0. { return None; }
        self.shortcut_action = None;
        self.commit_shortcut()
    }

    pub(crate) fn tick_shortcut(&mut self, delta: f32) -> Option<ShortcutSelection> {
        let action = self.shortcut_action.filter(|action| matches!(action, 23 | 24))?;
        self.shortcut_delay -= delta;
        self.release_shortcut(action)
    }

    fn shortcut_index(&self, action: i32) -> Option<usize> {
        let owned = |index: usize| self.shortcuts[index].equipment.as_deref().is_some_and(|path| self.quantity(path) > 0);
        let index = match action {
            25..=34 => (action - 25) as usize,
            37..=40 => (action - 27) as usize,
            23 | 24 => {
                if let Some(previous) = self.selected_shortcut.filter(|index| *index < 10) {
                    let direction = if action == 23 { 1 } else { -1 };
                    let mut candidate = previous;
                    for step in 1..=10 {
                        candidate = (previous as i32 + direction * step).rem_euclid(10) as usize;
                        if candidate == previous || owned(candidate) { break; }
                    }
                    candidate
                } else { (0..10).find(|&index| owned(index))? }
            }
            _ => return None,
        };
        Some(index)
    }

    pub(crate) fn last_shortcut(&self) -> Option<usize> { self.selected_shortcut }

    fn commit_shortcut(&self) -> Option<ShortcutSelection> {
        let index = self.selected_shortcut?;
        let slot = &self.shortcuts[index];
        let path = slot.equipment.as_ref()?;
        if self.quantity(path) == 0 { return None; }
        Some(ShortcutSelection { equipment: path.clone(), ammunition: slot.ammo.clone(),
            equipment_entity: self.declaration(path).and_then(|item| item.equipment.clone()) })
    }
}
