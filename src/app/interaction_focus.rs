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
    items: Vec<String>,
}
impl DiscoveredDocuments {
    pub fn items(&self) -> &[String] { &self.items }
    pub fn record(&mut self, item: String) {
        if !self.items.contains(&item) { self.items.push(item); }
    }
}
