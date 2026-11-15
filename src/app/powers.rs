//! Shared power definitions and inventory transactions. No archive access on
//! input, casting, or Journal callbacks: acquisition belongs to level setup.
use std::sync::Arc;
use bevy::prelude::Resource;
use crate::assets::{archive::Catalog, entities::Value, power_journal::{self, Power, Upgrade}};
use crate::gameplay::{options::decl::{list_items, scope}, pause_state::Character, powers::{Cast, Phase, Settings}};
use super::{interaction_focus::DiscoveredDocuments, journal_movie::PowerData};

#[derive(Resource)]
pub(super) struct Powers {
    pub definitions: Arc<(Vec<Power>, String)>,
    starting: Vec<String>,
    character: Character,
    /// Saved game manager state; never guessed from a map name.
    pub new_game_plus: bool,
    casts: Vec<Cast>,
    selected: Option<usize>,
}
impl Powers {
    pub fn prepare<'a>(catalog: &Catalog, system: &[(String, Value)], mark: &[(String, Value)],
        character: Character, items: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let definitions = Arc::new(power_journal::definitions_from_system(catalog, system, items)?);
        let casts = definitions.0.iter().map(|power| {
            let id = power_journal::power_id(&power.power_id).ok_or_else(|| format!("unknown power {}", power.power_id))?;
            Ok(Cast::new(Settings::read(id, &power.component)?))
        }).collect::<Result<Vec<_>, String>>()?;
        let starting = scope(mark, "m_startingPowers").map(list_items).unwrap_or_default().into_iter()
            .filter_map(|value| match value { Value::Atom(path) => Some(path.trim_matches('"').to_owned()), _ => None }).collect();
        Ok(Self { definitions, starting, character, new_game_plus: false, casts, selected: None })
    }
    /// 1404c1f20: ownership of any starting power is the mark's state.
    pub fn has_mark(&self, inventory: &DiscoveredDocuments) -> bool {
        self.starting.iter().any(|path| inventory.quantity(path) > 0)
    }
    pub fn snapshot(&self, inventory: &DiscoveredDocuments) -> PowerData {
        PowerData { definitions: self.definitions.clone(), quantities: inventory.quantity_snapshot(),
            has_mark: self.has_mark(inventory), new_game_plus: self.new_game_plus, character: self.character }
    }
    fn permitted(&self, power: &Power, inventory: &DiscoveredDocuments) -> bool {
        self.new_game_plus || inventory.quantity(&power.item) > 0 || power.usable_by.contains(match self.character {
            Character::Corvo => "USAGE_BY_CORVO", Character::Emily => "USAGE_BY_EMILY",
        })
    }
    /// Equipment admission uses actual owned items. Selecting an unowned
    /// Journal entry never grants or casts its power.
    pub fn select(&mut self, id: i32, inventory: &DiscoveredDocuments) -> bool {
        let Some(index) = self.casts.iter().position(|cast| cast.settings.id == id) else { return false };
        if inventory.quantity(&self.definitions.0[index].item) == 0 { return false; }
        // Caller must finish the outgoing cast through the world/animation
        // owners before changing the equipped slot (14061c1a0/14061c2a0).
        if self.selected.is_some_and(|old| self.casts[old].phase() != Phase::Idle) { return false; }
        self.selected = Some(index); true
    }
    pub fn selected(&self) -> Option<&Cast> { self.selected.map(|index| &self.casts[index]) }
    pub fn selected_mut(&mut self) -> Option<&mut Cast> { self.selected.map(|index| &mut self.casts[index]) }
    /// Resolve a movie request against the same definitions it displayed.
    /// 140521580 dispatches to 140455850/140455ab0; inventory owns both
    /// admission and modifier notifications, then rune removal (140614750).
    pub fn purchase(&self, inventory: &mut DiscoveredDocuments, root: bool, id: i32) -> bool {
        if !self.has_mark(inventory) { return false; }
        let mut selected = None;
        for power in &self.definitions.0 {
            if !self.permitted(power, inventory) { continue; }
            if root {
                if power_journal::power_id(&power.power_id) == Some(id) {
                    selected = Some((power.item.as_str(), power.rune_cost)); break;
                }
            } else if inventory.quantity(&power.item) > 0 {
                if let Some(upgrade) = eligible_upgrade(&power.upgrades, id, inventory) {
                    selected = upgrade.item.as_deref().zip(upgrade.info.as_ref().map(|info| info.rune_cost)); break;
                }
            }
        }
        let Some((path, cost)) = selected else { return false };
        if cost < 0 || inventory.quantity(path) != 0 || inventory.quantity(&self.definitions.1) < cost as u32 { return false; }
        let Some(item) = inventory.declaration(path).cloned() else { return false };
        let Some(rune) = inventory.declaration(&self.definitions.1).cloned() else { return false };
        inventory.purchase(path, inventory.maximum(path), &item.modifiers, &self.definitions.1, cost as u32, &rune.modifiers)
    }
}
fn eligible_upgrade<'a>(nodes: &'a [Upgrade], id: i32, inventory: &DiscoveredDocuments) -> Option<&'a Upgrade> {
    for node in nodes {
        if power_journal::upgrade_id(&node.id) == Some(id) { return Some(node); }
        if node.item.as_ref().is_some_and(|path| inventory.quantity(path) > 0) {
            if let Some(found) = eligible_upgrade(&node.children, id, inventory) { return Some(found); }
        }
    }
    None
}
