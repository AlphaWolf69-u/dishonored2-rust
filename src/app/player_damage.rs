//! 717950/717f50/717fe0 through idEntity876660/8767a0/8769f0.
//! Player simulation owns health and its local DEAD mode; LiveEntities owns
//! the shared identity/context and EntityEvents owns numeric message delivery.
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::LiveEntities,
    player::EntityEvent,
};
use bevy::prelude::{Mut, World};

/// Run after the real player frame and before the ordinary entity event pump.
/// Drain the player's existing queue in place, retaining its allocation.
pub(super) fn flush(world: &mut World) -> Result<(), String> {
    let Some(id) = super::player_lifetime::id(world) else {
        return Ok(());
    };
    if !world.contains_resource::<LiveEntities>() || !world.contains_resource::<EntityEvents>() {
        return Err("player damage notifications have no live entity/event owner".into());
    }
    world.resource_scope(|world, mut state: Mut<super::player::PlayerState>| {
        let Some(player) = state.player.as_mut() else {
            return Ok(());
        };
        world.resource_scope(|world, mut live: Mut<LiveEntities>| {
            let mut events = world.resource_mut::<EntityEvents>();
            flush_events(player, &mut live, &mut events, id)?;
            Ok(())
        })
    })
}

///Same876660/8767a0 bridge for synchronous source Damage events emitted
///outside the player frame (DamageZone39cc30). Owners stay borrowed directly.
pub(in crate::app) fn flush_events(
    player: &mut crate::gameplay::player::Player,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
    id: crate::gameplay::entity_lifetime::EntityId,
) -> Result<(), String> {
    for event in player.drain_entity_events() {
        match event {
            EntityEvent::Damage {
                params,
                result,
                was_dead,
            } => {
                // 717950 preserves actual subsequent damage for the
                // owning corpse/breakable handlers in message1A6.
                if was_dead {
                    events.post(Event {
                        id: 0x1a6,
                        target: Some(id),
                        payload: Payload::DamageParameters(params),
                    });
                    continue;
                }
                if result.incapacitated {
                    live.set_requested_entity_context(id, 2)?;
                    events.post(Event {
                        id: 0xa5,
                        target: Some(id),
                        payload: Payload::DamageParameters(params),
                    });
                }
                // 876660 queues Damaged, then synchronously invokes
                // attacker Damaging, whose8767a0 bridge queues A3.
                events.post(Event {
                    id: 0xa2,
                    target: Some(id),
                    payload: Payload::Damage { params, result },
                });
                if let Some(attacker) = params
                    .attacker
                    .filter(|attacker| live.get(*attacker).is_some())
                {
                    events.post(Event {
                        id: 0xa3,
                        target: Some(attacker),
                        payload: Payload::Damaging {
                            target: id,
                            params,
                            result,
                        },
                    });
                }
            }
        }
    }
    Ok(())
}
