//! The empty-list/null-owner Loot lifecycle (743af0/744b30/744f90/744ef0).
use std::sync::Arc;
use super::component_state::{ComponentInstance, ComponentOwner, declaration::DeclarationScheduling};

#[derive(Debug)]
pub(crate) struct EmptyLootDefinition {
    pub scheduling: DeclarationScheduling,
    pub tags: [Arc<str>; 2],
}
pub(crate) struct EmptyLootComponent {
    pub owner: ComponentOwner,
    pub definition: Arc<EmptyLootDefinition>,
    pub enabled: bool,
}
impl ComponentInstance for EmptyLootComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}
