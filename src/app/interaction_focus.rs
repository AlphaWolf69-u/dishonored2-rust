//! Focused world interaction consumed by the gameplay and HUD owners.
use bevy::prelude::Resource;
#[path = "inventory.rs"]
mod inventory;
pub(super) use inventory::{InventoryChange, InventoryRecord, InventoryPickupEffects, PickupAttributes, PlayerPickupEffects, ShortcutSettings, ShortcutSelection,
    InventoryScriptEvent, InventoryScriptReceiver, InventoryPublication};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct InteractionPrompt {
    pub entity: String,
    /// Authored string-table keys, resolved by the HUD's language owner.
    pub name: String,
    pub usage_text: String,
    /// arkUIInteractions_t, registered by 141a1b710.
    pub interaction_type: i32,
    pub binding: String,
    pub name_postfix: String,
    pub hold: bool,
    pub locked: bool,
    pub player_state: String,
}

#[derive(Resource, Default)]
pub(super) struct FocusedInteraction {
    /// `None` clears the prior target and its displayed interaction.
    pub current: Option<InteractionPrompt>,
}

/// Model placements whose owning entity has actually been removed by a
/// gameplay event. Most levels never populate this set.
#[derive(Resource, Default)]
pub(super) struct RetiredPlacements(pub std::collections::HashSet<usize>);

/// The initial loadout is prepared off-thread and consumed once by the
/// session inventory, independently of level interaction availability.
#[derive(Resource, Default)]
pub(super) struct InitialInventory {
    pub items: Option<Vec<InitialItem>>,
    pub declarations: Option<Vec<(std::sync::Arc<crate::assets::inventory_item::PickableItem>, u32)>>,
    pub loadout: Option<crate::assets::inventory_item::InventoryLoadout>,
}
impl InitialInventory {
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.declarations.iter().flatten().map(|(item, _)| item.path.as_ref())
    }
}

pub(super) struct InitialItem {
    pub path: String,
    pub quantity: u32,
    pub maximum: u32,
    pub modifiers: crate::assets::inventory_item::ItemModifiers,
}

enum ModifierChange {
    Quantity { path: String, modifiers: crate::assets::inventory_item::ItemModifiers, added: bool, count: u32 },
    Apply { modifier: crate::assets::inventory_item::AttributeModifier, added: bool, count: u32 },
}

struct OwnedEnhancement {
    path: String,
    count: u32,
    modifiers: crate::assets::inventory_item::ItemModifiers,
}

pub(super) fn initialize_inventory(mut inventory: bevy::prelude::ResMut<DiscoveredDocuments>,
    initial: Option<bevy::prelude::ResMut<InitialInventory>>) {
    if let Some(mut initial) = initial {
        if let Some(entries) = initial.declarations.take() {
            for (item, maximum) in entries { inventory.register_item(item, maximum); }
        }
        if let Some(loadout) = initial.loadout.take() {
            if !inventory.initialized {
                inventory.initialized = true;
                inventory.apply_loadout(&loadout, false);
            }
        }
        if let Some(entries) = initial.items.take() { inventory.initialize(entries); }
    }
}

/// The inventory quantity event adds/removes each authored modifier once per
/// admitted item (1406170b0 -> 140616940). Keep changes queued until the
/// player's actual attribute component and game clock exist.
pub(super) fn apply_inventory_modifiers(
    mut inventory: bevy::prelude::ResMut<DiscoveredDocuments>,
    player: Option<bevy::prelude::ResMut<super::player::PlayerState>>,
    clock: bevy::prelude::Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
) {
    let Some(mut player) = player else { return };
    let Some(player) = player.player.as_mut() else { return };
    let Some(now) = clock.clock().ok().and_then(|clock| clock.time(3)) else { return };
    loop {
        for (tag, added) in player.take_attribute_modifier_events() {
            if tag == "ARK_ATTRIBUTE_MAGIC_SUPPRESSED" { inventory.suppression_event(added); }
        }
        let Some(change) = inventory.modifier_changes.pop_front() else { break };
        match change {
            ModifierChange::Quantity { path, modifiers, added, count } => {
                if modifiers.enhancement {
                    let index = inventory.enhancements.iter().position(|item| item.path.eq_ignore_ascii_case(&path));
                    if added {
                        if let Some(index) = index { inventory.enhancements[index].count += count; }
                        else { inventory.enhancements.push(OwnedEnhancement { path, count, modifiers: modifiers.clone() }); }
                    } else if let Some(index) = index {
                        let entry = &mut inventory.enhancements[index];
                        entry.count = entry.count.saturating_sub(count);
                        if entry.count == 0 { inventory.enhancements.remove(index); }
                    }
                }
                let suppressed = modifiers.enhancement && player.attribute_differs_from_base("ARK_ATTRIBUTE_MAGIC_SUPPRESSED");
                if suppressed && !modifiers.suppressed.is_empty() {
                    inventory.enqueue_modifiers(&modifiers.suppressed, added, count);
                }
                if !suppressed || !modifiers.suppressed.is_empty() {
                    inventory.enqueue_modifiers(&modifiers.normal, added, count);
                }
            }
            ModifierChange::Apply { modifier, added, count } => {
                for _ in 0..count {
                    player.apply_attribute_modifier(&modifier.item, modifier.index, &modifier.tag,
                        modifier.increase, modifier.percentage, modifier.duration, added, now);
                }
            }
        }
    }
}

/// Session-owned document declaration identities for the journal. Saving is
/// a separate owner.
#[derive(Resource)]
pub(super) struct DiscoveredDocuments {
    items: Vec<DocumentEntry>,
    maps: Vec<JournalItem>,
    audiographs: Vec<JournalItem>,
    quantities: std::collections::HashMap<String, u32>,
    initialized: bool,
    modifier_changes: std::collections::VecDeque<ModifierChange>,
    enhancements: Vec<OwnedEnhancement>,
    /// Metadata for the same quantity records, retaining the declaration
    /// identity across level retirement (63c630). No quantities live here.
    records: std::collections::HashMap<String, InventoryRecord>,
    record_order: Vec<String>,
    equipped: [Option<String>; 16],
    changes: std::collections::VecDeque<InventoryChange>,
    ammo_consumption_locks: i32,
    coins: i32,
    consumed_coins: i32,
    allow_auto_equip: bool,
    instant_equip_first_frame: bool,
    auto_equip_candidates: Vec<(String, u8)>,
    shortcut_settings: Option<std::sync::Arc<inventory::ShortcutSettings>>,
    shortcuts: [inventory::ShortcutSlot; 14],
    selected_shortcut: Option<usize>,
    ammunition_dependencies: Vec<(String, String)>,
}
impl Default for DiscoveredDocuments {
    fn default() -> Self {
        Self { items: Vec::new(), maps: Vec::new(), audiographs: Vec::new(), quantities: Default::default(),
            initialized: false, modifier_changes: Default::default(), enhancements: Vec::new(), records: Default::default(),
            record_order: Vec::new(), equipped: Default::default(), changes: Default::default(), ammo_consumption_locks: 0,
            coins: 0, consumed_coins: 0, allow_auto_equip: false,
            // 613440 sets inventory +190 |=6: bit1 is true until the
            // first allowed 619760 arbitration clears bits0/1 with &fc.
            instant_equip_first_frame: true,
            auto_equip_candidates: Vec::new(), shortcut_settings: None, shortcuts: Default::default(),
            selected_shortcut: None, ammunition_dependencies: Vec::new() }
    }
}
#[derive(Clone, Debug)]
pub(super) struct DocumentEntry {
    /// 63c630 stores the inventory record's declaration pointer. History
    /// retains that same lease while the admitted journal entry exists.
    pub declaration: std::sync::Arc<crate::assets::inventory_item::PickableItem>,
    pub path: String,
    pub item_id: u32,
    pub kind: crate::assets::inventory_item::ReadableKind,
    pub is_new: bool,
}

#[derive(Clone, Debug)]
pub(super) struct JournalItem {
    pub declaration: std::sync::Arc<crate::assets::inventory_item::PickableItem>,
    pub path: String,
    /// Map entries use the map-info title ID (falling back to item name);
    /// audiographs use the inventory item's name ID.
    pub item_id: u32,
    pub is_new: bool,
}
impl DiscoveredDocuments {
    pub fn items(&self) -> &[DocumentEntry] { &self.items }
    pub fn maps(&self) -> &[JournalItem] { &self.maps }
    pub fn audiographs(&self) -> &[JournalItem] { &self.audiographs }
    pub fn is_initialized(&self) -> bool { self.initialized }
    pub fn quantity_snapshot(&self) -> std::collections::HashMap<String, u32> { self.quantities.clone() }
    /// The same session inventory owns power items and readable items.
    pub fn quantity(&self, path: &str) -> u32 {
        if path.bytes().all(|byte| !byte.is_ascii_uppercase() && byte != b'\\') {
            self.quantities.get(path).copied().unwrap_or(0)
        } else {
            self.quantities.get(&inventory::key(path)).copied().unwrap_or(0)
        }
    }

    /// Add up to the authored maximum and report the quantity admitted.
    pub fn add_item(&mut self, path: &str, quantity: u32, maximum: u32) -> u32 {
        let key = inventory::key(path);
        let current = self.quantities.get(&key).copied().unwrap_or(0);
        let admitted = quantity.min(maximum.saturating_sub(current));
        if let Some(record) = self.records.get_mut(&key) { record.is_new = current == 0 && admitted > 0; }
        if admitted != 0 { self.quantities.insert(key, current + admitted); }
        admitted
    }

    pub fn add_with_modifiers(&mut self, path: &str, quantity: u32, maximum: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> u32 {
        let previous = self.quantity(path);
        let admitted = self.add_item(path, quantity, maximum);
        if admitted != 0 {
            self.changes.push_back(InventoryChange::QuantityChanged { path: inventory::key(path), previous, current: previous + admitted });
            self.modifier_changes.push_back(ModifierChange::Quantity { path: path.to_owned(), modifiers: modifiers.clone(), added: true, count: admitted });
            if previous == 0 { self.changes.push_back(InventoryChange::ShortcutItemAdded { path: inventory::key(path) }); }
            if previous == 0 && self.record(path).is_some_and(|record| record.declaration.equipment.is_some()) {
                self.changes.push_back(InventoryChange::EquipmentNeeded { path: inventory::key(path) });
            }
        }
        admitted
    }

    pub fn remove_with_modifiers(&mut self, path: &str, quantity: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> u32 {
        let key = inventory::key(path);
        let current = self.quantities.get(&key).copied().unwrap_or(0);
        let removed = quantity.min(current);
        if removed == current { self.quantities.remove(&key); }
        else if removed != 0 { self.quantities.insert(key.clone(), current - removed); }
        if removed != 0 {
            self.changes.push_back(InventoryChange::QuantityChanged { path: key.clone(), previous: current, current: current - removed });
            if let Some(record) = self.records.get_mut(&key) { record.is_new &= current > removed; }
            if removed == current {
                self.auto_equip_candidates.retain(|(path, _)| path != &key);
                if self.record(path).is_some_and(|record| record.declaration.equipment.is_some()) {
                    self.changes.push_back(InventoryChange::EquipmentRemoved { path: key.clone() });
                }
            }
            self.modifier_changes.push_back(ModifierChange::Quantity { path: path.to_owned(), modifiers: modifiers.clone(), added: false, count: removed });
        }
        removed
    }

    /// 140455850/140455ab0 first admit one selected item, then request the
    /// authored rune cost. UI/gameflow own eligibility checks; the native
    /// operation does not roll back an accepted item if rune removal fails.
    pub fn purchase(&mut self, item: &str, maximum: u32,
        item_modifiers: &crate::assets::inventory_item::ItemModifiers,
        rune_item: &str, rune_cost: u32,
        rune_modifiers: &crate::assets::inventory_item::ItemModifiers) -> bool {
        if self.add_with_modifiers(item, 1, maximum, item_modifiers) != 1 { return false; }
        self.addition_notification(item, 1);
        let removed = self.remove_with_modifiers(rune_item, rune_cost, rune_modifiers);
        self.removal_notifications(rune_item, removed, self.quantity(rune_item));
        true
    }

    pub fn initialize(&mut self, entries: impl IntoIterator<Item = InitialItem>) {
        if self.initialized { return; }
        self.initialized = true;
        for item in entries { self.add_with_modifiers(&item.path, item.quantity, item.maximum, &item.modifiers); }
    }

    fn enqueue_modifiers(&mut self, modifiers: &[crate::assets::inventory_item::AttributeModifier], added: bool, count: u32) {
        for modifier in modifiers {
            self.modifier_changes.push_back(ModifierChange::Apply { modifier: modifier.clone(), added, count });
        }
    }

    /// 140615410 reacts to every matching modifier message, not just a
    /// transition of the resulting attribute value.
    fn suppression_event(&mut self, added: bool) {
        for owned in &self.enhancements {
            let (modifiers, apply) = if owned.modifiers.suppressed.is_empty() {
                (&owned.modifiers.normal, !added)
            } else { (&owned.modifiers.suppressed, added) };
            for modifier in modifiers {
                self.modifier_changes.push_back(ModifierChange::Apply { modifier: modifier.clone(), added: apply, count: owned.count });
            }
        }
    }
    /// 1407241c0 inserts inventory additions as new entries. The caller must
    /// invoke this only after the gameplay inventory admits the item.
    pub fn admit(&mut self, path: String, item_id: u32, kind: crate::assets::inventory_item::ReadableKind,
        maximum: u32, declaration: std::sync::Arc<crate::assets::inventory_item::PickableItem>) -> bool {
        self.register_item(declaration.clone(), maximum);
        if self.add_registered(&path, 1) == 0 { return false; }
        if self.items.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) { return true; }
        self.items.push(DocumentEntry { declaration, path, item_id, kind, is_new: true });
        true
    }

    pub fn admit_map(&mut self, path: String, item_id: u32, maximum: u32,
        declaration: std::sync::Arc<crate::assets::inventory_item::PickableItem>) -> bool {
        self.register_item(declaration.clone(), maximum);
        if self.add_registered(&path, 1) == 0 { return false; }
        if !self.maps.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) {
            self.maps.push(JournalItem { declaration, path, item_id, is_new: true });
        }
        true
    }

    pub fn admit_audiograph(&mut self, path: String, item_id: u32, maximum: u32,
        declaration: std::sync::Arc<crate::assets::inventory_item::PickableItem>) -> bool {
        self.register_item(declaration.clone(), maximum);
        if self.add_registered(&path, 1) == 0 { return false; }
        if !self.audiographs.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) {
            self.audiographs.push(JournalItem { declaration, path, item_id, is_new: true });
        }
        true
    }

    pub fn mark_maps_seen(&mut self, item_id: Option<u32>) {
        mark_journal_items_seen(&mut self.maps, item_id);
    }

    pub fn mark_audiographs_seen(&mut self, item_id: Option<u32>) {
        mark_journal_items_seen(&mut self.audiographs, item_id);
    }
    /// 140724bf0 clears the selected item; the journal passes -1 to clear all
    /// entries of the selected document/book kind.
    pub fn mark_seen(&mut self, kind: crate::assets::inventory_item::ReadableKind, item_id: Option<u32>) {
        for item in &mut self.items {
            if item.kind == kind && item_id.is_none_or(|id| id == item.item_id) {
                item.is_new = false;
                if item_id.is_some() { break; }
            }
        }
    }
}

fn mark_journal_items_seen(items: &mut [JournalItem], item_id: Option<u32>) {
    for item in items {
        if item_id.is_none_or(|id| id == item.item_id) {
            item.is_new = false;
            if item_id.is_some() { break; }
        }
    }
}
