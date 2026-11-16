//! Entity graph receiver retirement, before the shared identity is removed.
//! Kiscule declaration instances and completion queues stay with the existing
//! world/player FX manager; this adapter never constructs another manager.
use crate::gameplay::entity_lifetime::EntityId;
use bevy::prelude::*;

///841500 invokes the actual named pickedUp(bool,bool), after841030 and
///before76ec20. This is not numeric EntityEvents message69 (a variant tag).
pub(super) fn picked_up(
    world: &mut World,
    entity: EntityId,
    successful: bool,
    from_grapple: bool,
    receipt: &mut crate::gameplay::effects::instances::PickupReceipt,
) -> Result<(), String> {
    use crate::gameplay::entity_lifetime::LiveEntities;
    use crate::rendering::bevy_backend::camera_scene::FxSimulation;
    if !super::runtime_entities::picked_up_builtin_absent(world, entity)? {
        return Err("pickedUp requires its actual entity-class builtin handler".into());
    }
    if !world.contains_resource::<FxSimulation>() {
        return Err("pickedUp named receiver owner is absent".into());
    }
    world.resource_scope(|world, mut effects: Mut<FxSimulation>| {
        effects.dispatch_picked_up_receipt(
            entity,
            successful,
            from_grapple,
            world.resource::<LiveEntities>(),
            receipt,
        )
    })
}

pub(super) fn retire(world: &mut World, entity: EntityId) -> Result<(), String> {
    if let Some(mut effects) =
        world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
    {
        effects.retire_entity_graph(entity)?;
    }
    Ok(())
}
