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

/// Session-owned document declaration identities for the journal. Saving is
/// a separate owner.
#[derive(Resource, Default)]
pub(super) struct DiscoveredDocuments {
    items: Vec<DocumentEntry>,
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
    /// 1407241c0 inserts inventory additions as new entries. The caller must
    /// invoke this only after the gameplay inventory admits the item.
    pub fn record(&mut self, path: String, item_id: u32, kind: crate::assets::inventory_item::ReadableKind) {
        if !self.items.iter().any(|item| item.path == path) {
            self.items.push(DocumentEntry { path, item_id, kind, is_new: true });
        }
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
