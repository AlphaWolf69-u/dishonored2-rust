//! Focused world interaction consumed by the gameplay and HUD owners.
use bevy::prelude::Resource;

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
    pub registered: Option<RegisteredItems>,
}

/// The collection's registered slots retain the level list by lease and an
/// index per admitted identity, not another copy of each resource path.
#[derive(Clone)]
pub(super) struct RegisteredItems {
    list: std::sync::Arc<crate::assets::map_resources::LevelList>,
    indices: std::sync::Arc<[usize]>,
    limits: std::sync::Arc<crate::assets::inventory_item::ReadableLimits>,
}
impl RegisteredItems {
    pub fn new(list: std::sync::Arc<crate::assets::map_resources::LevelList>,
        limits: std::sync::Arc<crate::assets::inventory_item::ReadableLimits>) -> Self {
        let mut seen = std::collections::HashSet::new();
        let indices = list.iter().enumerate().filter_map(|(index, resource)| {
            let path = registered_item_path(resource)?;
            (limits.maximum(path) > 0 && seen.insert(path)).then_some(index)
        }).collect::<Vec<_>>().into();
        drop(seen);
        Self { list, indices, limits }
    }
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.indices.iter().filter_map(|&index| registered_item_path(&self.list[index]))
    }
    pub fn same_collection(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.list, &other.list) && std::sync::Arc::ptr_eq(&self.indices, &other.indices)
    }
    pub fn maximum(&self, path: &str) -> u32 { self.limits.maximum(path) }
}

pub(super) fn registered_item_path(resource: &crate::assets::map_resources::Resource) -> Option<&str> {
    if !resource.kind.eq_ignore_ascii_case("inventoryitem") { return None; }
    resource.path.strip_prefix("generated/decls/inventoryitem/")?.strip_suffix(".decl")
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
        if let Some(registered) = initial.registered.take() { inventory.registered = Some(registered); }
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
#[derive(Resource, Default)]
pub(super) struct DiscoveredDocuments {
    items: Vec<DocumentEntry>,
    maps: Vec<JournalItem>,
    audiographs: Vec<JournalItem>,
    quantities: std::collections::HashMap<String, u32>,
    initialized: bool,
    registered: Option<RegisteredItems>,
    modifier_changes: std::collections::VecDeque<ModifierChange>,
    enhancements: Vec<OwnedEnhancement>,
}
#[derive(Clone, Debug)]
pub(super) struct DocumentEntry {
    pub path: String,
    pub item_id: u32,
    pub kind: crate::assets::inventory_item::ReadableKind,
    pub is_new: bool,
}

#[derive(Clone, Debug)]
pub(super) struct JournalItem {
    pub path: String,
    /// Map entries use the map-info title ID (falling back to item name);
    /// audiographs use the inventory item's name ID.
    pub item_id: u32,
    pub is_new: bool,
}
impl DiscoveredDocuments {
    pub fn items(&self) -> &[DocumentEntry] { &self.items }
    pub fn registered_items(&self) -> Option<RegisteredItems> { self.registered.clone() }
    /// A journal request takes one small snapshot; zero-quantity candidates
    /// remain available separately through `registered_items`.
    pub fn quantity_snapshot(&self) -> std::collections::HashMap<String, u32> { self.quantities.clone() }
    pub fn maps(&self) -> &[JournalItem] { &self.maps }
    pub fn audiographs(&self) -> &[JournalItem] { &self.audiographs }
    pub fn is_initialized(&self) -> bool { self.initialized }
    /// The same session inventory owns power items and readable items.
    pub fn quantity(&self, path: &str) -> u32 {
        if path.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            self.quantities.get(path).copied().unwrap_or(0)
        } else {
            self.quantities.get(&path.to_ascii_lowercase()).copied().unwrap_or(0)
        }
    }

    /// Add up to the authored maximum and report the quantity admitted.
    pub fn add_item(&mut self, path: &str, quantity: u32, maximum: u32) -> u32 {
        let key = path.to_ascii_lowercase();
        let current = self.quantities.get(&key).copied().unwrap_or(0);
        let admitted = quantity.min(maximum.saturating_sub(current));
        if admitted != 0 { self.quantities.insert(key, current + admitted); }
        admitted
    }

    pub fn add_with_modifiers(&mut self, path: &str, quantity: u32, maximum: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> u32 {
        let admitted = self.add_item(path, quantity, maximum);
        if admitted != 0 {
            self.modifier_changes.push_back(ModifierChange::Quantity { path: path.to_owned(), modifiers: modifiers.clone(), added: true, count: admitted });
        }
        admitted
    }

    pub fn remove_with_modifiers(&mut self, path: &str, quantity: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> u32 {
        let key = path.to_ascii_lowercase();
        let current = self.quantities.get(&key).copied().unwrap_or(0);
        let removed = quantity.min(current);
        if removed == current { self.quantities.remove(&key); }
        else if removed != 0 { self.quantities.insert(key, current - removed); }
        if removed != 0 {
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
        self.remove_with_modifiers(rune_item, rune_cost, rune_modifiers);
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
        maximum: u32, modifiers: &crate::assets::inventory_item::ItemModifiers) -> bool {
        if self.add_with_modifiers(&path, 1, maximum, modifiers) == 0 { return false; }
        if self.items.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) { return true; }
        self.items.push(DocumentEntry { path, item_id, kind, is_new: true });
        true
    }

    pub fn admit_map(&mut self, path: String, item_id: u32, maximum: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> bool {
        if self.add_with_modifiers(&path, 1, maximum, modifiers) == 0 { return false; }
        if !self.maps.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) {
            self.maps.push(JournalItem { path, item_id, is_new: true });
        }
        true
    }

    pub fn admit_audiograph(&mut self, path: String, item_id: u32, maximum: u32,
        modifiers: &crate::assets::inventory_item::ItemModifiers) -> bool {
        if self.add_with_modifiers(&path, 1, maximum, modifiers) == 0 { return false; }
        if !self.audiographs.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) {
            self.audiographs.push(JournalItem { path, item_id, is_new: true });
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
