//! Session inventory operations. Quantities and modifier delivery remain in
//! DiscoveredDocuments; this module supplies its typed metadata and equipment
//! transitions, rather than creating an inventory resource beside it.
use super::DiscoveredDocuments;
use crate::assets::inventory_item::PickableItem;
use std::sync::Arc;

pub(super) fn key(path: &str) -> String { path.replace('\\', "/").to_ascii_lowercase() }

pub(crate) struct InventoryRecord {
    pub declaration: Arc<PickableItem>,
    pub maximum: u32,
    /// Native record +28 is set only by a zero-to-positive addition.
    pub is_new: bool,
}

/// Payloads for the equipment/gameplay owners. These preserve the native
/// distinction between request delivery and equipment activation; the caller
/// must perform the actual entity callbacks, never infer a visual from this.
#[derive(Clone, Debug)]
pub(crate) enum InventoryChange {
    Added { path: String, count: u32, total: u32 },
    Removed { path: String, count: u32, total: u32 },
    Depleted { path: String },
    Equipped { path: String, slot: u8, previous: Option<String> },
    Unequipped { path: String, slot: u8, destroy: bool },
    CoinsAdded { count: i32, total: i32 },
}

/// Values resolved from the player's actual attribute component and the
/// gameflow ammo-pack adjustment (647ee0); no substitute RNG lives here.
pub(crate) struct PickupAttributes {
    pub player: bool,
    pub health_consumable_bonus: f32,
    pub ammo_pack_bonus: i32,
    pub more_bolts: f32,
    pub fewer_bolts: f32,
    pub more_bullets: f32,
    pub fewer_bullets: f32,
}

pub(crate) trait InventoryPickupEffects {
    /// Invoke 71a140 and return true exactly when a health component exists.
    fn apply_health(&mut self, amount: f32) -> bool;
    /// Current 745f60 and maximum 745fb0; None means no mana component.
    fn mana(&self) -> Option<(f32, f32)>;
    /// Invoke the mana owner's 7467d0 behavior, including the player's target.
    fn apply_mana(&mut self, amount: f32);
    /// One draw from the game-local 8ac040 random stream.
    fn random(&mut self) -> f32;
    /// Duplicate blueprint value from the owner's black-market component
    /// declaration +68 (6156e0), not a price inferred from the item name.
    fn duplicate_blueprint_value(&self) -> i32;
    /// 4362a0 on the optional crafting component; absent component is a no-op.
    fn add_raw_whale_bone(&mut self, count: u32);
}

impl DiscoveredDocuments {
    /// 615840's player consumable and ammunition admission. Immediate food
    /// effects are performed once per pickup, independent of pack quantity,
    /// and return zero when consumed instead of entering inventory.
    pub fn pickup(&mut self, path: &str, count: u32, attributes: &PickupAttributes,
        effects: &mut impl InventoryPickupEffects) -> u32 {
        let Some(record) = self.record(path) else { return 0 };
        let item = record.declaration.clone();
        // 6156e0 sends coin/painting value to the currency counters even
        // when the collection maximum for this declaration is zero.
        if matches!(item.type_number, 1 | 15) {
            self.add_coins(count as i32);
            return count;
        }
        if item.type_number == 16 && self.quantity(path) != 0 {
            let value = effects.duplicate_blueprint_value();
            self.add_coins(value);
            return value as u32;
        }
        if item.type_number == 26 {
            effects.add_raw_whale_bone(count);
            return count;
        }
        if item.type_number == 6 && attributes.player {
            let health = item.consumable.health as f32 + attributes.health_consumable_bonus;
            let applied_health = health != 0.0 && effects.apply_health(health);
            if item.consumable.mana > 0 && effects.mana().is_some_and(|(current, maximum)| current < maximum) {
                effects.apply_mana(item.consumable.mana as f32);
                return 0;
            }
            if applied_health { return 0; }
        }
        let mut count = count;
        if item.type_number == 4 && attributes.player && count != 1 {
            let adjusted = (count as i32).wrapping_add(attributes.ammo_pack_bonus).max(1);
            count = adjusted as u32;
            let probabilities = match item.hud_index {
                1..=5 => Some((attributes.more_bolts, attributes.fewer_bolts)),
                10..=11 => Some((attributes.more_bullets, attributes.fewer_bullets)),
                _ => None,
            };
            if let Some((more, fewer)) = probabilities {
                if effects.random() < more { count += 1; }
                if count >= 2 && effects.random() < fewer { count -= 1; }
            }
        }
        self.add_registered(path, count)
    }

    /// 613ff0: loadout clear/forced clear occurs before assignments, and
    /// quantities are assigned through 618de0 rather than added as pickups.
    pub fn apply_loadout(&mut self, loadout: &crate::assets::inventory_item::InventoryLoadout, force_clear: bool) {
        if loadout.clear() || force_clear {
            let owned: Vec<_> = self.quantities.keys().cloned().collect();
            for path in owned { self.set_quantity(&path, 0); }
        }
        self.allow_auto_equip = loadout.allow_auto_equip();
        for (path, quantity) in loadout.assignments() { self.set_quantity(path, quantity); }
    }
    pub fn allow_auto_equip(&self) -> bool { self.allow_auto_equip }

    /// 63c630 creates a record for a collection entry before it owns any
    /// quantity. A zero quantity keeps the record and its declaration alive.
    pub fn register_item(&mut self, declaration: Arc<PickableItem>, maximum: u32) {
        self.records.entry(key(&declaration.path)).or_insert(InventoryRecord {
            declaration, maximum, is_new: false,
        });
    }

    pub fn record(&self, path: &str) -> Option<&InventoryRecord> { self.records.get(&key(path)) }
    pub fn maximum(&self, path: &str) -> u32 { self.record(path).map_or(0, |item| item.maximum) }
    pub fn declaration(&self, path: &str) -> Option<&Arc<PickableItem>> { self.record(path).map(|item| &item.declaration) }

    /// Typed quantity admission, with exactly one item declaration lease.
    /// Health/mana consumable callbacks and randomized ammo pickup bonuses
    /// are resolved by their gameplay owners before calling this operation.
    pub fn add_registered(&mut self, path: &str, count: u32) -> u32 {
        let Some(record) = self.record(path) else { return 0 };
        let item = record.declaration.clone();
        let maximum = record.maximum;
        let admitted = self.add_with_modifiers(path, count, maximum, &item.modifiers);
        if admitted > 0 {
            self.changes.push_back(InventoryChange::Added { path: key(path), count: admitted, total: self.quantity(path) });
        }
        admitted
    }

    /// 614750 -> 6147f0 -> 63c800. Ammo removal is suppressed while the
    /// inventory's nested consume lock is positive. Other classes are not.
    pub fn consume(&mut self, path: &str, count: u32) -> u32 {
        let Some(record) = self.record(path) else { return 0 };
        if record.declaration.type_number == 4 && self.ammo_consumption_locks > 0 { return 0; }
        let item = record.declaration.clone();
        let removed = self.remove_with_modifiers(path, count, &item.modifiers);
        let total = self.quantity(path);
        // 6147f0 broadcasts empty inventory even when the removal request
        // found an already empty record.
        if total == 0 { self.changes.push_back(InventoryChange::Depleted { path: key(path) }); }
        self.changes.push_back(InventoryChange::Removed { path: key(path), count: removed, total });
        removed
    }

    /// The host supplies the actual nested lock counter maintained by
    /// equipment/player state; 6147f0 tests this signed field strictly >0.
    pub fn set_ammo_consumption_locks(&mut self, locks: i32) { self.ammo_consumption_locks = locks; }

    /// 615620 increments coins independently of the quantity collection.
    pub fn add_coins(&mut self, count: i32) {
        self.coins = self.coins.wrapping_add(count);
        self.changes.push_back(InventoryChange::CoinsAdded { count, total: self.coins });
    }
    /// 614670 records consumed currency; 614d40 returns the difference.
    pub fn consume_coins(&mut self, count: i32) { self.consumed_coins = self.consumed_coins.wrapping_add(count); }
    pub fn available_coins(&self) -> i32 { self.coins.wrapping_sub(self.consumed_coins) }

    /// 63cbe0 clamps both the authored maximum and any remaining quantity;
    /// 614680 and 6182f0 deliver the resulting quantity modifier change.
    pub fn set_maximum(&mut self, path: &str, maximum: i32) -> bool {
        let Some(record) = self.records.get_mut(&key(path)) else { return false };
        record.maximum = maximum.max(0) as u32;
        let item = record.declaration.clone();
        let maximum = record.maximum;
        let current = self.quantity(path);
        if current > maximum { self.remove_with_modifiers(path, current - maximum, &item.modifiers); }
        true
    }

    /// 618de0 -> 63cc00 assigns rather than incrementing a quantity.
    pub fn set_quantity(&mut self, path: &str, requested: i32) -> Option<u32> {
        let record = self.record(path)?;
        let item = record.declaration.clone();
        let maximum = record.maximum;
        let quantity = (requested.max(0) as u32).min(maximum);
        let previous = self.quantity(path);
        if previous > quantity { self.remove_with_modifiers(path, previous - quantity, &item.modifiers); }
        else { self.add_with_modifiers(path, quantity - previous, maximum, &item.modifiers); }
        if let Some(record) = self.records.get_mut(&key(path)) { record.is_new = previous == 0 && quantity > 0; }
        Some(quantity)
    }

    /// 614b20 receives the slot from the live equipment component (+68),
    /// whose validity was checked by 619380 (slot <16, archetype !=0).
    /// The caller must supply that metadata after creating the equipment.
    pub fn equip(&mut self, path: &str, slot: u8, require_quantity: bool) -> bool {
        if slot >= 16 || self.record(path).is_none() || (require_quantity && self.quantity(path) == 0) { return false; }
        let path = key(path);
        let previous = self.equipped[slot as usize].replace(path.clone());
        // 614b20 sends the equip message even if the slot already selected
        // this record; activation callbacks only run on a real transition.
        self.changes.push_back(InventoryChange::Equipped { path, slot, previous });
        true
    }

    pub fn equipped(&self, slot: u8) -> Option<&str> { self.equipped.get(slot as usize)?.as_deref() }

    /// 619640/619740 clear only the selected record in its slot. The live
    /// entity is destroyed when empty and deactivated while still owned.
    pub fn unequip(&mut self, slot: u8) -> bool {
        let Some(selected) = self.equipped.get_mut(slot as usize) else { return false };
        let Some(path) = selected.take() else { return false };
        let destroy = self.quantity(&path) == 0;
        self.changes.push_back(InventoryChange::Unequipped { path, slot, destroy });
        true
    }

    pub fn take_changes(&mut self) -> impl Iterator<Item = InventoryChange> + '_ { self.changes.drain(..) }
}
