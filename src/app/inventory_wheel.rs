//! Borrow the canonical inventory/owned equipment for4f5360's wheel rows.
use crate::app::interaction_focus::DiscoveredDocuments;
use crate::gameplay::{
    player::{
        Player,
        consumables::{PotionInventory, PotionKind},
    },
    power_wheel::{Item, Potions, Snapshot},
};

impl DiscoveredDocuments {
    pub(crate) fn wheel_potions(&self, player: &Player) -> Potions {
        let count = |kind| {
            self.potion_item(kind)
                .ok()
                .flatten()
                .map_or(0, |item| self.quantity(&item.path))
        };
        Potions {
            can_health: player.can_use_potion(PotionKind::Health),
            can_mana: player.can_use_potion(PotionKind::Mana),
            health_count: count(PotionKind::Health),
            mana_count: count(PotionKind::Mana),
        }
    }
    pub(crate) fn wheel_snapshot(&mut self, player: &Player) -> Snapshot {
        let mut items = Vec::new();
        let present = |path: &str, equipment: bool| {
            self.record(path).is_some_and(|record| {
                self.quantity(path) > 0 || record.ever_owned && (!equipment || record.maximum >= 2)
            })
        };
        for path in &self.record_order {
            if !present(path, true) {
                continue;
            }
            let Some(record) = self.records.get(path) else {
                continue;
            };
            let Some((equipment, _, _)) = player.equipment_target_metadata(path) else {
                continue;
            };
            if let Some((ammunition, _)) = player.equipment_ammunition(path) {
                for ammo in ammunition {
                    if !present(&ammo.item, false) {
                        continue;
                    }
                    let Some(ammunition) = self.record(&ammo.item) else {
                        continue;
                    };
                    items.push(Item {
                        equipment: record.declaration.clone(),
                        ammunition: Some(ammunition.declaration.clone()),
                        upgrade: None,
                        count: self.quantity(&ammo.item) as i32,
                        maximum: ammunition.maximum as i32,
                        visible: ammunition.wheel_visible,
                    });
                }
            } else if matches!(equipment.archetype, 3 | 4) {
                items.push(Item {
                    equipment: record.declaration.clone(),
                    ammunition: None,
                    upgrade: None,
                    count: self.quantity(path) as i32,
                    maximum: record.maximum as i32,
                    visible: record.wheel_visible,
                });
            } else if matches!(equipment.archetype, 5 | 7 | 9) {
                items.push(Item {
                    equipment: record.declaration.clone(),
                    ammunition: None,
                    upgrade: None,
                    count: -1,
                    maximum: -1,
                    visible: record.wheel_visible,
                });
            }
        }
        let shortcuts = std::array::from_fn(|index| {
            let (equipment, ammo) = self.wheel_shortcut_pair(index);
            let occupied = equipment.is_some() || ammo.is_some();
            let value = items
                .iter()
                .find(|item| {
                    Some(item.equipment.path.as_ref()) == equipment
                        && item.ammunition.as_ref().map(|ammo| ammo.path.as_ref()) == ammo
                })
                .map_or(0, Item::index);
            if value == 0 && occupied {
                self.wheel_prune_shortcut(index);
            }
            value
        });
        let current = self
            .equipped(1)
            .and_then(|path| {
                player.equipment_target_metadata(path).map(|_| {
                    let ammo = player
                        .equipment_ammunition(path)
                        .and_then(|(_, selected)| selected);
                    ammo.and_then(|ammo| self.declaration(ammo))
                        .or_else(|| self.declaration(path))
                        .map_or(-1, |item| item.hud_index)
                })
            })
            .unwrap_or(-1);
        let potions = self.wheel_potions(player);
        Snapshot {
            items: items.into(),
            shortcuts,
            current,
            can_health: potions.can_health,
            can_mana: potions.can_mana,
            health_count: potions.health_count,
            mana_count: potions.mana_count,
        }
    }
    ///4f69b0 writes the edited visibility to both same-generation records.
    pub(crate) fn wheel_visibility(&mut self, item: &Item, visible: bool) {
        if let Some(record) = self.records.get_mut(item.equipment.path.as_ref()) {
            record.wheel_visible = visible;
        }
        if let Some(ammo) = &item.ammunition {
            if let Some(record) = self.records.get_mut(ammo.path.as_ref()) {
                record.wheel_visible = visible;
            }
        }
    }
}
