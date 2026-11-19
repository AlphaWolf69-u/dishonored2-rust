//! 45e510/45e920 bind the existing in-game movie's 14 shortcut rows.
use super::HudMovie;
use crate::app::interaction_focus::DiscoveredDocuments;
use crate::assets::inventory_item::PickableItem;
use crate::gameplay::player::Player;
use std::sync::Arc;

pub(in crate::app) struct Row {
    item: Arc<PickableItem>,
    count: i32,
    maximum: i32,
}

pub(in crate::app) struct Update {
    rows: Option<Box<[Option<Row>; 14]>>,
    slot: Option<usize>,
    show: bool,
    hide: bool,
}

pub(in crate::app) fn snapshot(
    player: &Player,
    inventory: &DiscoveredDocuments,
    previous: (u64, bool),
) -> Update {
    let pressed = previous.0 != inventory.shortcut_input;
    let enabled = crate::platform::console::shared()
        .read()
        .cvars
        .int("ark_ui_show_shortcuts")
        != Some(0);
    let slot = pressed.then(|| inventory.last_shortcut()).flatten();
    let rows = (enabled && slot.is_some()).then(|| {
        Box::new(std::array::from_fn(|index| {
            let (equipment, ammunition) = inventory.wheel_shortcut_pair(index);
            let equipment = equipment?;
            let present = |path: &str, equipment: bool| {
                inventory.record(path).is_some_and(|record| {
                    inventory.quantity(path) > 0
                        || record.ever_owned && (!equipment || record.maximum >= 2)
                })
            };
            if !present(equipment, true) {
                return None;
            }
            let ammo = ammunition.filter(|path| present(path, false));
            let path = ammo.unwrap_or(equipment);
            let archetype = player.equipment_target_metadata(equipment)?.0.archetype;
            let (count, maximum) = if ammo.is_some() || matches!(archetype, 3 | 4) {
                (
                    inventory.quantity(path) as i32,
                    inventory.maximum(path) as i32,
                )
            } else if matches!(archetype, 5 | 7 | 9) {
                (-1, -1)
            } else {
                return None;
            };
            Some(Row {
                item: inventory.declaration(path)?.clone(),
                count,
                maximum,
            })
        }))
    });
    Update {
        rows,
        slot: slot.filter(|_| enabled),
        show: enabled && pressed && !previous.1,
        hide: !enabled || inventory.shortcut_preview().is_none(),
    }
}

impl HudMovie {
    pub(in crate::app) fn shortcuts(&mut self, update: Update) -> Result<(), String> {
        if let Some(rows) = update.rows {
            let strings = self
                .player
                .state
                .borrow()
                .strings
                .clone()
                .ok_or("shortcut HUD strings absent")?;
            for (index, row) in rows.iter().enumerate() {
                let (provider, slot) = if index < 10 {
                    ("shortcutPCData", index)
                } else {
                    ("shortcutDPadData", index - 10)
                };
                let path = format!("_shortcuts.shortcutsData.{provider}.data[{slot}]");
                let (icon, label, count, total) = row
                    .as_ref()
                    .map(|row| {
                        (
                            row.item.hud_index,
                            strings.lookup(&row.item.name).to_string(),
                            row.count,
                            row.maximum,
                        )
                    })
                    .unwrap_or_default();
                for (field, value) in [
                    ("itemIndex", icon.into()),
                    ("label", label.into()),
                    ("ammoCount", count.into()),
                    ("ammoTotal", total.into()),
                ] {
                    self.player
                        .set_value(&format!("{path}.{field}"), value)
                        .map_err(|error| self.player.describe(&error))?;
                }
            }
        }
        if update.show {
            let method = if update.slot.is_some_and(|slot| slot >= 10) {
                "_C_Show"
            } else {
                "_C_ShowPCDockBar"
            };
            self.player
                .call_path_method("_shortcuts", method, &[])
                .map_err(|error| self.player.describe(&error))?;
        }
        if let Some(slot) = update.slot {
            self.player
                .call_path_method("_shortcuts", "_C_InputEvent", &[(slot as i32).into()])
                .map_err(|error| self.player.describe(&error))?;
        }
        if std::env::var_os("DH2_DEBUG_ACTION_SCENARIO").is_some() {
            if let Some(index) = update.slot {
                let (provider, slot) = if index < 10 {
                    ("shortcutPCData", index)
                } else {
                    ("shortcutDPadData", index - 10)
                };
                let row = format!("_shortcuts.shortcutsData.{provider}.data[{slot}].itemIndex");
                let icon = self
                    .player
                    .get_value(&row)
                    .map_err(|error| self.player.describe(&error))?
                    .primitive_to_number();
                let visible = self
                    .player
                    .get_value("_shortcuts.visible")
                    .map_err(|error| self.player.describe(&error))?;
                eprintln!(
                    "ACTION_SMOKE_SHORTCUT_MOVIE slot={index} item_index={icon} visible={visible:?} show_called={} hide_pending={}",
                    update.show, update.hide
                );
            }
        }
        if update.hide {
            self.player
                .call_path_method("_shortcuts", "_C_Hide", &[])
                .map_err(|error| self.player.describe(&error))?;
        }
        self.last = None;
        Ok(())
    }
}
