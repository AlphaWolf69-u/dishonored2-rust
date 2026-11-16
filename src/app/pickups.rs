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

pub(super) struct Pending {
    pub entity: EntityId,
    pub reference: CheckedComponent,
    pub definition: Arc<PickableDefinition>,
    pub item: Arc<crate::assets::inventory_item::PickableItem>,
    pub maximum: u32,
    pub phase: u8,
    pub admitted: u32,
    pub from_grapple: bool,
    pub notice: crate::gameplay::effects::instances::PickupReceipt,
    pub rejected_notice: bool,
    pub rejected_sound: Option<u32>,
}
impl Pending {
    ///615d40/615d90: admitted ammo uses the current inventory record maximum;
    ///the collection's prepared value supplies a not-yet-hosted record.
    pub(super) fn inventory_full(
        &self,
        inventory: &interaction_focus::DiscoveredDocuments,
    ) -> bool {
        inventory
            .record(&self.item.path)
            .map_or(self.maximum, |record| record.maximum)
            <= inventory.quantity(&self.item.path)
    }
}

struct AmmoEffects<'a>(&'a mut crate::gameplay::player::Player);
impl interaction_focus::InventoryPickupEffects for AmmoEffects<'_> {
    fn apply_health(&mut self, _: f32) -> bool {
        unreachable!("ammo cannot invoke consumable health")
    }
    fn mana(&self) -> Option<(f32, f32)> {
        unreachable!("ammo cannot invoke consumable mana")
    }
    fn apply_mana(&mut self, _: f32) {
        unreachable!("ammo cannot invoke consumable mana")
    }
    fn random(&mut self) -> f32 {
        self.0.inventory_random()
    }
    fn duplicate_blueprint_value(&self) -> i32 {
        unreachable!("ammo is not a blueprint")
    }
    fn add_raw_whale_bone(&mut self, _: u32) {
        unreachable!("ammo is not crafting material")
    }
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    let Some(mut pending) = world
        .get_resource_mut::<interactions::Interactions>()
        .and_then(|mut interactions| interactions.pending_pickup.take())
    else {
        return Ok(());
    };
    let result = advance(world, &mut pending);
    if !matches!(result, Ok(true)) {
        world
            .resource_mut::<interactions::Interactions>()
            .pending_pickup = Some(pending);
    }
    result.map(|_| ())
}

fn advance(world: &mut World, pending: &mut Pending) -> Result<bool, String> {
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
        return Ok(false);
    }
    let accepted = world
        .get_resource::<player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| player.pending_loot_object());
    if accepted != Some(pending.entity) {
        return Ok(false);
    }
    if pending.item.type_number != 4 || pending.definition.quantity != 1 {
        return Err(
            "runtime pickup requires the actual item-family/pack gameplay callbacks".into(),
        );
    }
    if pending.phase == 0 {
        pending.admitted = world.resource_scope(
            |world, mut inventory: Mut<interaction_focus::DiscoveredDocuments>| {
                let mut state = world.resource_mut::<player::PlayerState>();
                let player = state
                    .player
                    .as_mut()
                    .ok_or("accepted pickup player absent")?;
                //6156e0 never reads the gameflow ammo-pack bonus for a single
                //round. Unsupported pack counts are rejected above, not guessed.
                let attributes = interaction_focus::PickupAttributes::for_player(player, 0);
                inventory.register_item(pending.item.clone(), pending.maximum);
                Ok::<_, String>(inventory.pickup(
                    &pending.item.path,
                    1,
                    &attributes,
                    &mut AmmoEffects(player),
                ))
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

fn finish(world: &mut World, pending: &mut Pending) -> Result<(), String> {
    if pending.phase == 1 {
        super::entity_graphs::picked_up(
            world,
            pending.entity,
            true,
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
    update(world)
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
