//!Loot membership and source743af0/743c30/743cf0/744ef0 ownership.
use super::{
    component_state::{ComponentInstance, ComponentOwner, declaration::DeclarationScheduling},
    entity_lifetime::EntityId,
    name_pool::NameId,
};
use std::sync::Arc;
#[derive(Debug)]
pub(crate) struct LootDefinition {
    pub scheduling: DeclarationScheduling,
    pub tags: [Arc<str>; 2],
}
///Source24-byte item record: managed entity, attachment tag,+10/+11 flags.
///EntityId carries the actual managed generation, never an owning resource.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LootMember {
    pub entity: EntityId,
    pub tag: NameId,
    ///744880 admits these only when owner's D1 bits0x28 are set.
    pub requires_incapacitated_owner: bool,
    pub collected: bool,
}
pub(crate) struct LootComponent {
    pub owner: ComponentOwner,
    pub definition: Arc<LootDefinition>,
    pub enabled: bool,
    pub members: Vec<LootMember>,
    ///Host retry cursor for744ef0's ascending reflected Remove calls.
    pub retiring: Option<usize>,
    pub starting: Option<usize>,
}
impl LootComponent {
    pub fn new(definition: Arc<LootDefinition>) -> Self {
        Self {
            owner: Default::default(),
            definition,
            enabled: true,
            members: Vec::new(),
            retiring: None,
            starting: None,
        }
    }
}
impl ComponentInstance for LootComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
