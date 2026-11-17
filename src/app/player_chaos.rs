//! Global incapacitation callback4c1610→4c17b0, with the actual player's
//! generation retained through4c16d0 unsubscription. No NPC is inferred from
//! the name or an unimplemented character description.
use bevy::prelude::*;
use crate::gameplay::{entity_events::{EntityEvents,Listener},entity_lifetime::EntityId};

pub(super) fn bind(world:&mut World,id:EntityId)->Result<(),String>{
    if !world.contains_resource::<EntityEvents>(){return Err("Chaos Init event owner is absent".into())}
    let mut state=world.get_resource_mut::<super::player::PlayerState>().ok_or("Chaos player owner is absent")?;
    let player=state.player.as_mut().ok_or("Chaos player runtime is absent")?;
    if player.managed_entity()!=Some(id){return Err("Chaos Init player generation differs".into())}
    if player.chaos.initialized(){return Ok(())}
    player.chaos.initialize();drop(state);
    world.get_resource_mut::<EntityEvents>().ok_or("Chaos Init event owner is absent")?
        .register(0xa5,None,Listener::PlayerChaos{player:id});
    Ok(())
}
pub(super) fn retire(world:&mut World,id:EntityId)->Result<(),String>{
    world.get_resource_mut::<EntityEvents>().ok_or("Chaos Deinit event owner is absent")?
        .unregister(0xa5,None,Listener::PlayerChaos{player:id});
    if let Some(mut state)=world.get_resource_mut::<super::player::PlayerState>(){
        if let Some(player)=&mut state.player{
            if player.managed_entity()!=Some(id){return Err("Chaos Deinit player generation differs".into())}
            player.chaos.deinitialize();
        }
    }
    Ok(())
}
