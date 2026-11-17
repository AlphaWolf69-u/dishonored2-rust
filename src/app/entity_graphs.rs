//! Entity graph receiver retirement, before the shared identity is removed.
//! Kiscule declaration instances and completion queues stay with the existing
//! world/player FX manager; this adapter never constructs another manager.
use crate::gameplay::entity_lifetime::EntityId;
use bevy::prelude::*;

///One691110 ->8847f0/884980 call. The accepted catch journal owns this
///receipt until builtin and reverse-order observer delivery acknowledge.
#[derive(Default)]
pub(super) struct GrappledReceipt {
    target: Option<EntityId>,
    builtin: bool,
    observers: Option<Vec<EntityId>>,
    next: usize,
    complete: bool,
}

///141398270 registers grappled(void), event11b linked to notice81. The
///ordinals come from installed PE constructor order, not a name hash.
pub(super) fn grappled(
    world: &mut World,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    target: EntityId,
    receipt: &mut GrappledReceipt,
) -> Result<(), String> {
    if receipt.target.is_some_and(|previous| previous != target) {
        return Err("grappled receipt target changed".into());
    }
    if receipt.complete { return Ok(()); }
    let (_, class) = live.get(target).ok_or("grappled target generation is stale")?;
    receipt.target = Some(target);
    if !receipt.builtin {
        let catalog = world.get_resource::<super::SceneData>()
            .and_then(|scene| scene.catalog.as_ref())
            .ok_or("grappled canonical class metadata owner is absent")?;
        let types = catalog.component_types.lock()
            .map_err(|_| "grappled class metadata owner is poisoned")?
            .clone().ok_or("grappled executable class metadata is not resident")?;
        let metadata = types.classes().metadata(class)
            .ok_or("grappled actual class metadata is absent")?;
        let name = types.classes().name_text(metadata.name)
            .ok_or("grappled actual class name is absent")?;
        match name {
            //Movable8e7870, Pickup8b6890 and Grenade8e79e0 delegate11b
            //to8cf620 ->9154b0's literal void branch. Dynamic8f2260
            //explicitly consumes11b with the same void result. Their
            //concrete virtual30 is399620(false), not a script override.
            b"idEntity" | b"idDynamicEntity" | b"arkMovable" |
            b"arkPickup" | b"arkGrenade" => {},
            _ => return Err(format!("grappled builtin dispatch is not implemented for {}",
                String::from_utf8_lossy(name))),
        }
        receipt.builtin = true;
    }
    if receipt.observers.is_none() {
        receipt.observers = Some(live.event_observer_snapshot(target)?);
    }
    while let Some(&observer) = receipt.observers.as_ref().unwrap().get(receipt.next) {
        if live.grappled_observer_matches(observer)? {
            return Err("grappled notice callback is not implemented".into());
        }
        receipt.next += 1;
    }
    //The admitted Kiscule receivers subscribe only to Pickup, Breakable
    //and Gameflow notices; none registers81. NPC bark/behavior recipients
    //are separate concrete owners and are not admitted by this frontend.
    receipt.complete = true;
    Ok(())
}

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
