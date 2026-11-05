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
pub(super) struct InitialInventory(pub Option<Vec<(String, u32, u32)>>);

pub(super) fn initialize_inventory(mut inventory: bevy::prelude::ResMut<DiscoveredDocuments>,
    initial: Option<bevy::prelude::ResMut<InitialInventory>>) {
    if let Some(entries) = initial.and_then(|mut initial| initial.0.take()) {
        inventory.initialize(entries);
    }
}

/// Session-owned document declaration identities for the journal. Saving is
/// a separate owner.
#[derive(Resource, Default)]
pub(super) struct DiscoveredDocuments {
    items: Vec<DocumentEntry>,
    quantities: std::collections::HashMap<String, u32>,
    initialized: bool,
}
#[derive(Clone, Debug)]
pub(super) struct DocumentEntry {
    pub path: String,
    pub item_id: u32,
    pub kind: crate::assets::inventory_item::ReadableKind,
    pub is_new: bool,
}
impl DiscoveredDocuments {
    pub fn items(&self) -> &[DocumentEntry] { &self.items }
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

    pub fn initialize(&mut self, entries: impl IntoIterator<Item = (String, u32, u32)>) {
        if self.initialized { return; }
        self.initialized = true;
        for (path, quantity, maximum) in entries { self.add_item(&path, quantity, maximum); }
    }
    /// 1407241c0 inserts inventory additions as new entries. The caller must
    /// invoke this only after the gameplay inventory admits the item.
    pub fn admit(&mut self, path: String, item_id: u32, kind: crate::assets::inventory_item::ReadableKind, maximum: u32) -> bool {
        if self.add_item(&path, 1, maximum) == 0 { return false; }
        if self.items.iter().any(|item| item.path.eq_ignore_ascii_case(&path)) { return true; }
        self.items.push(DocumentEntry { path, item_id, kind, is_new: true });
        true
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
