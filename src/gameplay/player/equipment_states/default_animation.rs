//!61b3a0 addresses the equipment component's own animation tree, including
//!an outgoing/holstered owner; the current hand may already hold another actor.
use super::super::{equipment::EquipmentTree, Player};
use crate::gameplay::entity_lifetime::EntityId;

impl Player {
    pub fn set_equipment_default_alias(
        &mut self,
        identity: &str,
        entity: EntityId,
        alias: &str,
    ) -> Result<bool, String> {
        let matches = |tree: &&mut EquipmentTree| tree.managed_entity == Some(entity);
        if let Some(tree) = self.body.equipment.iter_mut().flatten().find(matches) {
            return tree.set_default_alias(alias);
        }
        //Only the two real switching slots and the identity-keyed retained
        //owner participate; this is not an all-equipment/frame scan.
        for slot in &mut self.equipment_changes.slots {
            for target in slot.current.iter_mut().chain(slot.incoming.iter_mut()) {
                if target.identity == identity {
                    if let Some(tree) = target
                        .tree
                        .as_mut()
                        .filter(|tree| tree.managed_entity == Some(entity))
                    {
                        return tree.set_default_alias(alias);
                    }
                }
            }
        }
        if let Some(tree) = self
            .equipment_changes
            .owned
            .get_mut(identity)
            .and_then(|target| target.tree.as_mut())
            .filter(|tree| tree.managed_entity == Some(entity))
        {
            return tree.set_default_alias(alias);
        }
        //53f4f0 also returns0 while its own AnimEvolved/tree is absent.
        //Preparation applies the actual current default when that tree arrives.
        Ok(false)
    }
}
