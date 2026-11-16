//! Entity graph receiver retirement, before the shared identity is removed.
//! Kiscule declaration instances and completion queues stay with the existing
//! world/player FX manager; this adapter never constructs another manager.
use crate::gameplay::entity_lifetime::EntityId;
use bevy::prelude::*;

///One884980 ->883d10 invocation, retained by the grenade's accepted explosion
///journal. No resource or graph definition is copied into this receipt.
#[derive(Default)]
pub(super) struct EquipmentExplodedReceipt {
    target: Option<EntityId>,
    builtin: bool,
    observers: Option<Vec<EntityId>>,
    next: usize,
    complete: bool,
}

///490840 invokes playerEquipmentExploded on the actual player, with no args.
///The installed PE CRT order assigns this eventf5 (notice7e);8f2500 casef5
///calls7ac710, a literal no-op.884980 then dispatches the linked notice.
///This dedicated frontend is deliberately not an unknown named-event ACK.
pub(super) fn player_equipment_exploded(
    world: &mut World,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    target: EntityId,
    receipt: &mut EquipmentExplodedReceipt,
) -> Result<(), String> {
    if receipt.target.is_some_and(|previous| previous != target) {
        return Err("playerEquipmentExploded receipt target changed".into());
    }
    if receipt.complete { return Ok(()); }
    if super::player_lifetime::id_with_live(world, live) != Some(target) {
        return Err("playerEquipmentExploded requires the actual live player".into());
    }
    receipt.target = Some(target);
    if !receipt.builtin {
        //8f2500 f5 ->7ac710: there is no builtin equipment/state mutation.
        receipt.builtin = true;
    }
    if receipt.observers.is_none() {
        receipt.observers = Some(live.event_observer_snapshot(target)?);
    }
    let observers = receipt.observers.as_ref().unwrap();
    while let Some(&observer) = observers.get(receipt.next) {
        if live.equipment_exploded_observer_matches(observer)? {
            return Err("playerEquipmentExploded notice callback is not implemented".into());
        }
        receipt.next += 1;
    }
    //The canonical FX graph owner currently admits only Pickup, Breakable
    //and Gameflow notice subscriptions, none of which register notice7e.
    //PlayerHandsNotification's concrete641a40..641d40 callbacks concern
    //equip/unequip/change, and do not subscribe to EquipmentExploded.
    //When a new notice family is admitted it must extend this closed set.
    receipt.complete = true;
    Ok(())
}

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
    super::runtime_entities::picked_up_builtin(world, entity, successful)?;
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
