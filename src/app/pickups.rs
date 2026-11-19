//!841030 inventory delivery for the real LootObject state. The state/physics
//!owners admit and move the target; this owner acknowledges inventory and the
//!same Pickable generation, without creating another item/resource cache.
use super::*;
use crate::gameplay::{
    entity_lifetime::EntityId,
    pickable_component::{CheckedComponent, PickableDefinition},
};
use bevy::prelude::{Mut, World};
use std::sync::Arc;

///The actual47 transaction shared by43f9b0 arrival and691970 movable catch.
pub(in crate::app) fn request(
    world: &mut World,
    scene: &physics_scene::PhysicsScene,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    player: &mut crate::gameplay::player::Player,
    id: EntityId,
) -> Result<bool, String> {
    let mut interactions = world
        .get_resource_mut::<interactions::Interactions>()
        .ok_or("GrappleCatch interaction owner absent")?;
    request_borrowed(&mut interactions, scene, live, player, id, true)
}

///83f590/841500's same parameterized47 transaction for ordinary interaction
///and Far Reach. Authored47 hold props use49/50 after reaching the player;
///their presence does not turn the request into a different Grab48 class.
pub(in crate::app) fn request_borrowed(
    interactions: &mut interactions::Interactions,
    scene: &physics_scene::PhysicsScene,
    live: &crate::gameplay::entity_lifetime::LiveEntities,
    player: &mut crate::gameplay::player::Player,
    id: EntityId,
    from_grapple: bool,
) -> Result<bool, String> {
    if !scene.grapple_pickup_available(live, player, id)? {
        return Ok(false);
    }
    if interactions.pending_pickup.is_some() {
        return Ok(false);
    }
    let Some((reference, pickable)) = scene.grapple_pickable(live, id)? else {
        return Ok(false);
    };
    if pickable.interaction_flags & 2 != 0 {
        return Ok(false);
    }
    let definition = pickable.definition.clone();
    let item = pickable.item.clone();
    let maximum = item
        .as_ref()
        .map_or(0, |item| interactions.limits.maximum(&item.path));
    let hold = player.loot_hold_target(id, definition.clone())?;
    if !player.request_loot_object(id) {
        return Ok(false);
    }
    interactions.pending_pickup = Some(Pending::new(id, reference, definition, item, hold, maximum, from_grapple));
    Ok(true)
}


pub(super) struct Pending {
    pub entity: EntityId,
    pub reference: CheckedComponent,
    pub definition: Arc<PickableDefinition>,
    pub item: Option<Arc<crate::assets::inventory_item::PickableItem>>,
    pub hold:Option<crate::gameplay::player::carrying::Target>,
    pub maximum: u32,
    pub phase: u8,
    pub admitted: u32,
    currency_notice: Option<i32>,
    pub hud_notified: bool,
    pub from_grapple: bool,
    pub notice: crate::gameplay::effects::instances::PickupReceipt,
    pub rejected_notice: bool,
    pub rejected_sound: Option<u32>,
}
impl Pending {
    pub(super) fn new(entity: EntityId, reference: CheckedComponent, definition: Arc<PickableDefinition>,
        item: Option<Arc<crate::assets::inventory_item::PickableItem>>,
        hold: Option<crate::gameplay::player::carrying::Target>, maximum: u32, from_grapple: bool) -> Self {
        Self { entity, reference, definition, item, hold, maximum, from_grapple, phase: 0, admitted: 0,
            currency_notice: None, hud_notified: false, notice: Default::default(), rejected_notice: false, rejected_sound: None }
    }
    ///615d40/615d90: admitted ammo uses the current inventory record maximum;
    ///the collection's prepared value supplies a not-yet-hosted record.
    pub(super) fn inventory_full(
        &self,
        inventory: &interaction_focus::DiscoveredDocuments,
    ) -> bool {
        if self.definition.hold.is_some(){return false;}
        let Some(item)=&self.item else{return false;};
        //615d40's actual exemption bitmap0x38022, before record lookup.
        if item.type_number < 18 && 0x38022u32 & (1 << item.type_number) != 0 {
            return false;
        }
        inventory
            .record(&item.path)
            .map_or(self.maximum, |record| record.maximum)
            <= inventory.quantity(&item.path)
    }
}

/// Only inventory families whose actual gameplay callback owners are hosted.
///Read52 has its separate source841500 branch; unhosted special families must not
///be admitted merely because a declaration has a generic inventory record.
pub(super) fn inventory_family_supported(item: &crate::assets::inventory_item::PickableItem) -> bool {
    matches!(item.type_number, 0..=11 | 14..=23 | 26 | 27)
}
pub(super) fn inventory_family_available(
    item: &crate::assets::inventory_item::PickableItem,
    player: &crate::gameplay::player::Player,
) -> bool {
    inventory_family_supported(item) && (item.type_number != 6
        || item.consumable.health as f32 + player.attributes.get("ARK_ATTRIBUTE_HEALTH_CONSUMABLE_BONUS") >= 0.
        || player.inventory_health_damage_ready())
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    update_pending(world, false)
}

fn update_pending(world: &mut World, closing: bool) -> Result<(), String> {
    let Some(mut pending) = world
        .get_resource_mut::<interactions::Interactions>()
        .and_then(|mut interactions| interactions.pending_pickup.take())
    else {
        return Ok(());
    };
    let result = advance(world, &mut pending, closing);
    if !matches!(result, Ok(true)) {
        world
            .resource_mut::<interactions::Interactions>()
            .pending_pickup = Some(pending);
    }
    result.map(|_| ())
}

fn advance(world: &mut World, pending: &mut Pending, closing: bool) -> Result<bool, String> {
    if pending.rejected_notice {
        //83f590's full-inventory branch sends pickedUp(false,fromGrapple)
        //before841490 posts its prepared full sound. A missing target alone
        //does not send this named notification.
        super::entity_graphs::picked_up(
            world,
            pending.entity,
            false,
            pending.from_grapple,
            &mut pending.notice,
        )?;
        if let Some(event) = pending.rejected_sound.take() {
            world.write_message(super::audio::SoundEvent(
                crate::gameplay::sound::Request::Post {
                    event,
                    emitter: crate::gameplay::sound::Emitter::PlayerBody,
                },
            ));
        }
        pending.rejected_notice = false;
    }
    let rejected = world
        .get_resource_mut::<player::PlayerState>()
        .and_then(|mut state| {
            state
                .player
                .as_mut()
                .and_then(|player| player.take_rejected_loot_object())
        });
    if rejected == Some(pending.entity) {
        super::time_sync::detach_entity_time(world,pending.entity)?;
        physics_scene::carrying_host::acknowledge_loot_pull(world, pending.entity);
        return Ok(true);
    }
    if pending.phase == 0
        && world
            .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
            .is_none_or(|live| {
                live.stage(pending.entity) != Some(crate::gameplay::entity_lifetime::Stage::Spawned)
            })
    {
        world
            .resource_mut::<player::PlayerState>()
            .player
            .as_mut()
            .ok_or("lost pickup player absent")?
            .lose_loot_object(pending.entity)?;
        super::time_sync::detach_entity_time(world,pending.entity)?;
        return Ok(false);
    }
    let accepted = world
        .get_resource::<player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| player.pending_loot_object());
    if accepted != Some(pending.entity) {
        return Ok(false);
    }
    let readable=pending.item.as_ref().is_some_and(|item|item.note_type!=-1);
    let offered_from_exit=world.resource::<player::PlayerState>().player.as_ref()
        .ok_or("Loot47 time detach player absent")?.loot_offered_from_exit(pending.entity);
    //83fe70's readable branch queues52 directly, bypassing its ordinary
    //cancel restoration/80e050 branch.841500 detaches before queueing52.
    if !(offered_from_exit && (pending.hold.is_some() || readable)) {super::time_sync::detach_entity_time(world,pending.entity)?;}
    if readable {return finish_readable(world,pending,offered_from_exit,closing);}
    if pending.hold.is_some() {return finish_hold(world,pending);}
    if pending.item.is_none() {
        if pending.phase==0 {pending.phase=1;}
        finish(world,pending)?;
        world.resource_mut::<player::PlayerState>().player.as_mut().ok_or("Loot47 consume player absent")?.acknowledge_loot_object(pending.entity)?;
        physics_scene::carrying_host::acknowledge_loot_pull(world,pending.entity);
        return Ok(true);
    }
    let item=pending.item.as_ref().ok_or("Loot47 item lease disappeared")?;
    if !inventory_family_supported(item) {
        return Err(
            "Loot47 item family has no admitted gameplay callback owner".into(),
        );
    }
    //841030 repeats615d40 at the actual inventory callback: quantity can
    //change while the same target is physically pulled. A newly full target
    //still receives841500's pickedUp/hide, but no pack RNG/food effects.
    if pending.phase == 0 && pending.inventory_full(world.resource::<interaction_focus::DiscoveredDocuments>()) {
        pending.admitted = 0;
        pending.phase = 1;
    }
    if pending.phase == 0 {
        let ammo_pack_bonus = if item.type_number == 4 && pending.definition.quantity != 1 {
            let state = world.resource::<player::PlayerState>();
            let difficulty = crate::platform::console::shared().read().cvars
                .int("ark_difficulty_current").unwrap_or(0);
            //647ee0 returns zero while the difficulty manager is uninitialized.
            state.difficulty.as_ref().map(|manager|manager.ammo_quantity_offset(difficulty))
                .transpose()?.unwrap_or(0)
        } else { 0 };
        pending.admitted = world.resource_scope(
            |world, mut inventory: Mut<interaction_focus::DiscoveredDocuments>| {
                let mut path = item.path.clone();
                let mut count = pending.definition.quantity as u32;
                let new_game_plus = world.get_resource::<super::powers::Powers>()
                    .is_some_and(|powers| powers.new_game_plus);
                if matches!(item.type_number, 7 | 8) {
                    let powers = world.get_resource::<super::powers::Powers>();
                    if powers.is_none_or(|powers| !powers.has_mark(&inventory)) {
                        inventory.add_coins(pending.definition.quantity);
                        pending.currency_notice = Some(pending.definition.quantity);
                        return Ok(count);
                    }
                    count = item.rune_increase as u32;
                    if let Some(powers) = powers {
                        let Some(rune) = inventory.declaration(&powers.definitions.1) else {
                            // 614f30 found no record in the player's collection.
                            return Ok(0);
                        };
                        path = rune.path.clone();
                    }
                }
                let mut state = world.resource_mut::<player::PlayerState>();
                let player = state
                    .player
                    .as_mut()
                    .ok_or("accepted pickup player absent")?;
                let attributes = interaction_focus::PickupAttributes::for_player(player, ammo_pack_bonus);
                if item.type_number == 6
                    && item.consumable.health as f32 + attributes.health_consumable_bonus < 0.0
                    && !player.inventory_health_damage_ready() {
                    return Err("negative food health callback has no actual player identity".into());
                }
                inventory.register_item(item.clone(), pending.maximum);
                let mut negative_health = |player: &mut crate::gameplay::player::Player, amount: f32| {
                    player.inventory_health_damage(amount);
                };
                let admitted = inventory.pickup(
                    &path, count,
                    &attributes,
                    &mut interaction_focus::PlayerPickupEffects { player, negative_health: &mut negative_health },
                );
                if item.type_number == 16 && admitted > 1 {
                    pending.currency_notice = Some(admitted as i32);
                    if new_game_plus {
                        inventory.record_converted_blueprint(admitted as i32);
                    }
                }
                Ok::<_, String>(admitted)
            },
        )?;
        pending.phase = 1;
    }
    finish(world, pending)?;
    world
        .resource_mut::<player::PlayerState>()
        .player
        .as_mut()
        .ok_or("pickup player disappeared")?
        .acknowledge_loot_object(pending.entity)?;
    physics_scene::carrying_host::acknowledge_loot_pull(world, pending.entity);
    Ok(true)
}

fn finish_readable(world:&mut World,pending:&mut Pending,from_exit:bool,closing:bool)->Result<bool,String> {
    if pending.phase==0 {
        if closing {
            //An outgoing unactivated52 is cancelled by the existing read
            //owner's close/retire path. No UI/inventory/pickup is fabricated.
            world.resource_mut::<player::PlayerState>().player.as_mut().ok_or("Loot47 readable player absent")?.acknowledge_loot_object(pending.entity)?;
            physics_scene::carrying_host::acknowledge_loot_pull(world,pending.entity);
            return Ok(true);
        }
        let item=pending.item.as_ref().ok_or("Loot47 readable item absent")?.clone();
        let queued=world.resource_scope(|world,mut interactions:Mut<interactions::Interactions>|{
            interactions.request_loot_readable(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),pending.entity,pending.reference,item)
        })?;
        if !queued {return Ok(false);}
        pending.phase=1;
    }
    if pending.phase==1 {
        //841500 completes76ec20(false);83fe70 only queues the52 request.
        if !from_exit {
            world.resource_scope(|world,mut scene:Mut<physics_scene::PhysicsScene>|{
                scene.finish_loot_without_hide(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),pending.reference)
            })?;
        }
        pending.phase=2;
    }
    world.resource_mut::<player::PlayerState>().player.as_mut().ok_or("Loot47 readable player absent")?.acknowledge_loot_object(pending.entity)?;
    physics_scene::carrying_host::acknowledge_loot_pull(world,pending.entity);
    Ok(true)
}

fn finish_hold(world:&mut World,pending:&mut Pending)->Result<bool,String> {
    if world.resource::<player::PlayerState>().player.as_ref().ok_or("Loot47 hold player absent")?.loot_offered_from_exit(pending.entity) {
        if pending.phase==0 {
            world.resource_scope(|world,mut scene:Mut<physics_scene::PhysicsScene>|scene.cancel_held_loot(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),pending.reference))?;
            pending.phase=1;
        }
        if pending.phase==1 {
            //83fe70 orders76ec80/D7 restoration before80e050.
            super::time_sync::detach_entity_time(world,pending.entity)?;
            pending.phase=2;
        }
        world.resource_mut::<player::PlayerState>().player.as_mut().unwrap().acknowledge_loot_object(pending.entity)?;
        physics_scene::carrying_host::acknowledge_loot_pull(world,pending.entity);
        return Ok(true);
    }
    if pending.phase==0 {
        let target=pending.hold.as_ref().ok_or("Loot47 hold target missing")?.clone();
        world.get_resource_mut::<player::PlayerState>().and_then(|mut s|s.player.as_mut().map(|p|p.receive_loot_hold(target)))
            .ok_or("Loot47 hold player absent")??;
        pending.phase=1;
    }
    if pending.phase==1 {
        world.resource_scope(|world,mut scene:Mut<physics_scene::PhysicsScene>|{
            scene.finish_held_loot(world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),pending.reference)
        })?;
        pending.phase=2;
    }
    world.resource_mut::<player::PlayerState>().player.as_mut().ok_or("Loot47 hold player absent")?
        .acknowledge_loot_object(pending.entity)?;
    physics_scene::carrying_host::acknowledge_loot_pull(world,pending.entity);
    Ok(true)
}

fn finish(world: &mut World, pending: &mut Pending) -> Result<(), String> {
    if pending.phase == 1 {
        //841030 invokes6734b0 or currency6733c0 explicitly. This
        //is independent of9B; silent loadout/script additions do not log.
        if !pending.hud_notified {
            if let Some(item) = &pending.item {
                if pending.currency_notice.is_some() || matches!(item.type_number, 1 | 15) || (pending.admitted > 0 && item.type_number != 6) {
                    let hud = world.get_non_send_resource::<super::pause_menu::PauseMenu>()
                        .ok_or("pickup HUD delivery owner absent")?;
                    if let Some(value) = pending.currency_notice {
                        hud.queue_currency(value)?;
                    } else if matches!(item.type_number, 1 | 15) {
                        hud.queue_currency(pending.definition.quantity)?;
                    } else if pending.admitted > 0 && item.type_number != 6 {
                        hud.queue_pickup(item.clone(), pending.admitted)?;
                    }
                }
            }
            pending.hud_notified = true;
        }
        super::entity_graphs::picked_up(
            world,
            pending.entity,
            pending.item.is_some(),
            pending.from_grapple,
            &mut pending.notice,
        )?;
        pending.phase = 2;
    }
    if pending.phase == 2 {
        world.resource_scope(|world, mut scene: Mut<physics_scene::PhysicsScene>| {
            world.resource_scope(
                |world, mut sources: Mut<shadow_updates::ModelShadowSources>| {
                    world.resource_scope(
                        |world, mut hidden: Mut<interaction_focus::RetiredPlacements>| {
                            scene.finish_document_pickup(
                                world.resource::<crate::gameplay::entity_lifetime::LiveEntities>(),
                                pending.reference,
                                &mut sources,
                                world.resource::<RenderSourcePlacements>(),
                                &mut hidden,
                            )
                        },
                    )
                },
            )
        })?;
        pending.phase = 3;
    }
    if pending.phase == 3 {
        world_models::set_visible(world, pending.entity, false)?;
        pending.phase = 4;
    }
    Ok(())
}

/// Only already accepted work is drained here. The player state owns original
///deactivation/remaining-target commit before its source actor is destroyed.
pub(super) fn finish_accepted(world: &mut World) -> Result<(), String> {
    if let Some(player) = world
        .get_resource_mut::<player::PlayerState>()
        .as_deref_mut()
        .and_then(|state| state.player.as_mut())
    {
        player.finish_loot_object();
    }
    update_pending(world, true)
}

/// A callback which mutated inventory or accepted a failure notification must
///finish its named-event/hide acknowledgements before generation reuse.
pub(super) fn blocks_retirement(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<interactions::Interactions>()
        .and_then(|interactions| interactions.pending_pickup.as_ref())
        .is_some_and(|pending| {
            pending.entity == id && (pending.phase != 0 || pending.rejected_notice)
        })
}
pub(super) fn accepted_pending(world: &World) -> bool {
    world
        .get_resource::<interactions::Interactions>()
        .and_then(|interactions| interactions.pending_pickup.as_ref())
        .is_some_and(|pending| pending.phase != 0 || pending.rejected_notice)
}
