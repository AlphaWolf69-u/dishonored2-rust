//! A demanded equipment consumer of the existing runtime material owner.
//! Acquisition/mesh construction runs in MaterialWork, with its AssetBatch,
//! material snapshot, image leases and ordinary publication/retirement.
use super::*;
use crate::assets::equipment::Descriptor;

#[derive(Default)]
pub(super) struct EquipmentAdmissions {
    requested: HashMap<String, std::sync::Weak<Descriptor>>,
    live: Vec<std::sync::Weak<Descriptor>>,
    failed: HashMap<String, String>,
    staged: HashMap<String, Arc<Descriptor>>,
}

/// Called only for the actual completed inventory demand, before installing
/// its gameplay tree. No archive/program/mesh decoding occurs on this thread.
pub(in crate::app) fn request(
    world: &mut World,
    descriptor: Arc<Descriptor>,
) -> Result<(), String> {
    let mut coordinator = world
        .get_resource_mut::<RuntimeMaterialCoordinator>()
        .ok_or("runtime material coordinator is absent")?;
    let key = descriptor.entity.clone();
    if let Some(error) = coordinator.equipment.failed.get(&key) {
        return Err(error.clone());
    }
    let lease = Arc::downgrade(&descriptor);
    if coordinator
        .equipment
        .requested
        .get(&key)
        .is_some_and(|old| old.ptr_eq(&lease))
    {
        return Ok(());
    }
    coordinator.equipment.requested.insert(key, lease.clone());
    coordinator.equipment.live.push(lease);
    coordinator.revision = None;
    coordinator.published_generation = None;
    Ok(())
}

/// CPU asset publication and the shared image binding gate completed. The
/// existing Bevy renderer still performs its ordinary GPU readiness checks.
pub(in crate::app) fn admitted(world: &World, entity: &str) -> bool {
    world
        .get_resource::<super::super::player_arms::PlayerArms>()
        .is_some_and(|arms| arms.has_equipment_model(entity))
}

pub(in crate::app) fn failure(world: &World, entity: &str) -> Option<String> {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|owner| owner.equipment.failed.get(entity))
        .cloned()
}

pub(in crate::app) fn withdraw(world: &mut World, descriptor: &Descriptor) {
    if let Some(mut owner) = world.get_resource_mut::<RuntimeMaterialCoordinator>() {
        owner.equipment.requested.remove(&descriptor.entity);
        owner.equipment.failed.remove(&descriptor.entity);
        owner.revision = None;
        owner.published_generation = None;
    }
    let Some(mut arms) = world.remove_resource::<super::super::player_arms::PlayerArms>() else {
        return;
    };
    let mut commands = super::super::setup_commands::SetupCommands::prepared(world);
    arms.remove_equipment_model(&descriptor.model, &mut commands);
    commands.publish(world);
    world.insert_resource(arms);
}

pub(super) fn needs_update(state: &RuntimeMaterialCoordinator) -> bool {
    state
        .equipment
        .live
        .iter()
        .any(|lease| lease.strong_count() == 0)
}

pub(super) fn retire_expired(state: &mut RuntimeMaterialCoordinator) {
    let before = state.equipment.live.len();
    state
        .equipment
        .live
        .retain(|lease| lease.strong_count() != 0);
    state
        .equipment
        .requested
        .retain(|_, lease| lease.strong_count() != 0);
    if state.equipment.live.len() != before {
        state.revision = None;
    }
}

pub(super) fn requested(world: &World, state: &RuntimeMaterialCoordinator) -> Vec<Arc<Descriptor>> {
    state
        .equipment
        .requested
        .iter()
        .filter(|(key, _)| !state.equipment.failed.contains_key(*key) && !admitted(world, key))
        .filter_map(|(_, lease)| lease.upgrade())
        .collect()
}

pub(super) fn add_material_leases(
    state: &EquipmentAdmissions,
    consumers: &HashSet<MaterialConsumer>,
    leases: &mut Vec<std::sync::Weak<Descriptor>>,
) {
    for consumer in consumers {
        let MaterialConsumer::Equipment(entity) = consumer else {
            continue;
        };
        let Some(lease) = state.requested.get(entity) else {
            continue;
        };
        if !leases.iter().any(|old| old.ptr_eq(lease)) {
            leases.push(lease.clone());
        }
    }
}

pub(super) fn failed(world: &World, state: &mut RuntimeMaterialCoordinator, error: &str) {
    for name in state.equipment.requested.keys() {
        if !admitted(world, name) {
            state
                .equipment
                .failed
                .insert(name.clone(), error.to_owned());
        }
    }
}

/// The loading gate waits only for selected material/mesh/image publication.
/// Native staged draws advance after continue on the normal game clock.
pub(in crate::app) fn startup_pending(world: &World) -> bool {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .is_some_and(|owner| {
            owner.equipment.requested.iter().any(|(entity, lease)| {
                lease.strong_count() != 0
                    && !owner.equipment.failed.contains_key(entity)
                    && !admitted(world, entity)
            })
        })
}

pub(in crate::app) fn startup_error(world: &World) -> Option<String> {
    world
        .get_resource::<RuntimeMaterialCoordinator>()
        .and_then(|owner| owner.equipment.failed.values().next())
        .cloned()
}

fn take_staged(world: &mut World, identity: &str) -> Option<Arc<Descriptor>> {
    world
        .get_resource_mut::<RuntimeMaterialCoordinator>()?
        .equipment
        .staged
        .remove(identity)
}

/// Selection/metadata arbitration remains PlayerActions' input owner. This
/// adapter publishes prepared banks/targets, then commits quantity, sound,
/// ranged and render lifetime changes only from actual equipment callbacks.
pub(in crate::app) fn advance(world: &mut World) {
    use super::super::{
        interaction_focus::DiscoveredDocuments,
        player::PlayerState,
        player_actions::{self, PlayerActions},
        powers::Powers,
    };
    use crate::gameplay::player::equipment_states::Callback;
    let Some(mut actions) = world.remove_resource::<PlayerActions>() else {
        return;
    };
    let Some(mut state) = world.remove_resource::<PlayerState>() else {
        world.insert_resource(actions);
        return;
    };
    let Some(mut inventory) = world.remove_resource::<DiscoveredDocuments>() else {
        world.insert_resource(state);
        world.insert_resource(actions);
        return;
    };
    let mut powers = world.remove_resource::<Powers>();
    if let Some(player) = state.player.as_mut() {
        for callback in player.take_equipment_callbacks() {
            match callback {
                Callback::Equipped { identity, side } => {
                    if let Some(descriptor) = take_staged(world, &identity) {
                        if side == 0 {
                            if let Some(settings) = &descriptor.ranged {
                                if let Err(error) = player_actions::install_ranged(
                                    player,
                                    &identity,
                                    Arc::new(settings.clone()),
                                    actions
                                        .selected_ammunition
                                        .get(&identity)
                                        .and_then(Option::as_deref),
                                    &mut inventory,
                                ) {
                                    bevy::log::warn!(
                                        "Equipment {identity} ranged activation failed: {error}"
                                    );
                                }
                            } else {
                                player.unequip_ranged();
                            }
                        }
                        inventory.equip(&identity, descriptor.equipment.slot, true);
                        player.set_equipped_archetype(side, Some(descriptor.equipment.archetype));
                        actions.accepted(side, descriptor);
                    } else {
                        match actions.activate_power(&identity, player) {
                            Ok(Some((id, equipment))) => {
                                inventory.equip(&identity, equipment.slot, true);
                                player.set_equipped_archetype(side, Some(equipment.archetype));
                                if let Some(powers) = powers.as_mut() {
                                    powers.select(id, &inventory);
                                }
                            }
                            Ok(None) => bevy::log::warn!(
                                "Equipment callback has no prepared target: {identity}"
                            ),
                            Err(error) => {
                                bevy::log::warn!("Power {identity} activation failed: {error}")
                            }
                        }
                    }
                }
                Callback::Unequipped { identity, side } => {
                    if let Some(descriptor) = actions.remove_active(side) {
                        if descriptor.ranged.is_some() {
                            if inventory.quantity(&identity) == 0 {
                                player.retire_ranged(&identity);
                            } else {
                                player.unequip_ranged();
                            }
                        }
                        if inventory.equipped(descriptor.equipment.slot) == Some(identity.as_str())
                        {
                            inventory.unequip(descriptor.equipment.slot);
                        }
                        withdraw(world, &descriptor);
                    }
                    let slot = match side {
                        0 => Some(1),
                        1 => Some(0),
                        _ => None,
                    };
                    if let Some(slot) = slot {
                        if inventory.equipped(slot) == Some(identity.as_str()) {
                            inventory.unequip(slot);
                        }
                    }
                    actions.deactivate_power(&identity, player);
                    player.set_equipped_archetype(side, None);
                }
                Callback::Cancelled { identity, side } => {
                    if let Some(descriptor) = take_staged(world, &identity) {
                        if !actions
                            .active(side)
                            .is_some_and(|active| Arc::ptr_eq(active, &descriptor))
                        {
                            withdraw(world, &descriptor);
                        }
                    }
                    actions.cancel_power_selection(&identity);
                }
            }
        }
        for path in std::mem::take(&mut actions.removed_equipment) {
            let entity = inventory
                .declaration(&path)
                .and_then(|item| item.equipment.clone());
            if let Some(staged) = take_staged(world, &path) {
                withdraw(world, &staged);
            }
            for side in 0..2 {
                if actions.active(side).is_some_and(|active| {
                    entity
                        .as_deref()
                        .is_some_and(|entity| active.entity.eq_ignore_ascii_case(entity))
                }) {
                    if let Some(descriptor) = actions.remove_active(side) {
                        withdraw(world, &descriptor);
                        if descriptor.ranged.is_some() {
                            if inventory.quantity(&path) == 0 {
                                player.retire_ranged(&path);
                            } else {
                                player.unequip_ranged();
                            }
                        }
                        if player.body.equipment[side]
                            .as_ref()
                            .is_some_and(|tree| tree.model == descriptor.model)
                        {
                            player.set_equipped_archetype(side, None);
                            player.body.replace_equipment_tree(side, None);
                            player.body.arms[side].request_equipment("Empty");
                        }
                    }
                }
            }
        }
        for target in std::mem::take(&mut actions.power_targets) {
            let identity = target.identity.clone();
            if !actions.wanted(target.equipment.slot, &identity)
                || inventory.quantity(&identity) == 0
            {
                actions.cancel_power_selection(&identity);
                continue;
            }
            let Some(flags) = actions.request_flags(&identity) else {
                actions.power_targets.push(target);
                continue;
            };
            if let Err(error) = player.request_equipment_selection(target, flags) {
                actions.cancel_power_selection(&identity);
                bevy::log::warn!("Power {identity} staging failed: {error}");
            }
        }
        for ready in actions.take_ready() {
            if !actions.wanted(ready.descriptor.equipment.slot, &ready.item.path) {
                if !actions
                    .active(ready.side())
                    .is_some_and(|active| Arc::ptr_eq(active, &ready.descriptor))
                {
                    withdraw(world, &ready.descriptor);
                }
                continue;
            }
            if !ready.can_apply(player) {
                actions.requeue(ready);
                continue;
            }
            if !ready.is_current(player) {
                if !actions
                    .active(ready.side())
                    .is_some_and(|active| Arc::ptr_eq(active, &ready.descriptor))
                {
                    withdraw(world, &ready.descriptor);
                }
                if let Err(error) = actions.request(ready.item, player) {
                    bevy::log::warn!("Equipment reacquisition failed: {error}");
                }
                continue;
            }
            let identity = ready.item.path.to_string();
            if inventory.quantity(&identity) == 0 {
                withdraw(world, &ready.descriptor);
                continue;
            }
            if let Some(error) = failure(world, &ready.descriptor.entity) {
                bevy::log::warn!("Equipment {identity} material admission failed: {error}");
                withdraw(world, &ready.descriptor);
                continue;
            }
            if !admitted(world, &ready.descriptor.entity) {
                match request(world, ready.descriptor.clone()) {
                    Ok(()) => actions.requeue(ready),
                    Err(error) => {
                        bevy::log::warn!("Equipment {identity} admission failed: {error}")
                    }
                }
                continue;
            }
            let descriptor = ready.descriptor.clone();
            let Some(flags) = actions.request_flags(&identity) else {
                actions.requeue(ready);
                continue;
            };
            match ready.stage(player) {
                Ok(target) => match player.request_equipment_selection(target, flags) {
                    Ok(_) => {
                        world
                            .resource_mut::<RuntimeMaterialCoordinator>()
                            .equipment
                            .staged
                            .insert(identity, descriptor);
                    }
                    Err(error) => {
                        withdraw(world, &descriptor);
                        bevy::log::warn!("Equipment {identity} staging failed: {error}");
                    }
                },
                Err(error) => {
                    withdraw(world, &descriptor);
                    bevy::log::warn!("Equipment {identity} bank publication failed: {error}");
                }
            }
        }
    }
    if let Some(powers) = powers {
        world.insert_resource(powers);
    }
    world.insert_resource(inventory);
    world.insert_resource(state);
    world.insert_resource(actions);
}
