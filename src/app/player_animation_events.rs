//! One delivery owner for the player's actual Midnight track callbacks.
//! 7dfbd0/7dfdd0 advance once before components; equipment recipients are
//! captured before state/equipment changes and retained until delivery ACK.
use crate::gameplay::{entity_lifetime::EntityId, player::AnimationEvent};
use bevy::prelude::*;

struct Delivery {
    event: AnimationEvent,
    equipment: [Option<EntityId>; 2],
    bound: std::sync::Arc<[EntityId]>,
}
#[derive(Resource, Default)]
struct Events {
    entries: Vec<Delivery>,
    cursor: usize,
    consumer: u8,
    staging: Vec<AnimationEvent>,
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    let mut owner = world.remove_resource::<Events>().unwrap_or_default();
    let mut source = None;
    let mut equipment = [None; 2];
    if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
        if let Some(player) = &mut state.player {
            source = player.managed_entity();
            equipment = std::array::from_fn(|side| {
                player.body.equipment[side]
                    .as_ref()
                    .and_then(|tree| tree.managed_entity)
            });
            owner.staging.extend(player.drain_animation_events());
        }
    }
    let result = (|| {
        if !owner.staging.is_empty() {
            //84e0c0 reads the bound cast entity's real child list. A body
            //key includes holstered equipment, whose tree is not in the
            //selected-arm array. One source snapshot serves this event batch.
            let bound = if let Some(source) = source {
                let live = world
                    .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                    .ok_or("animation callback has no actual entity registry")?;
                std::sync::Arc::<[EntityId]>::from(live.children(source)?)
            } else {
                std::sync::Arc::from([])
            };
            owner
                .entries
                .extend(owner.staging.drain(..).map(|event| Delivery {
                    event,
                    equipment,
                    bound: bound.clone(),
                }));
        }
        while let Some(delivery) = owner.entries.get(owner.cursor) {
            if owner.consumer == 0 {
                super::melee::handle_animation_event(
                    world,
                    &delivery.event,
                    delivery.equipment,
                    &delivery.bound,
                )?;
                owner.consumer = 1;
            }
            if owner.consumer == 1 {
                super::weapon_throwing::handle_animation_event(
                    world,
                    &delivery.event,
                    delivery.equipment,
                )?;
                owner.consumer = 2;
            }
            owner.cursor += 1;
            owner.consumer = 0;
        }
        owner.entries.clear();
        owner.cursor = 0;
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

/// Drain accepted track callbacks before dismantling their source equipment.
/// This never advances animation or admits new input while closing a level.
pub(super) fn finish_accepted(world: &mut World) -> Result<(), String> {
    update(world)
}

pub(super) fn blocks_retirement(world: &World, id: EntityId) -> bool {
    world.get_resource::<Events>().is_some_and(|owner| {
        owner.entries[owner.cursor..]
            .iter()
            .any(|delivery| delivery.equipment.contains(&Some(id)) || delivery.bound.contains(&id))
    })
}

pub(super) fn shutdown(world: &mut World) -> Result<(), String> {
    finish_accepted(world)?;
    world.remove_resource::<Events>();
    Ok(())
}
