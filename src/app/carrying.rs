//! Carry transactions composed with the live Pickable and physics owners.
//! There is no second physics actor, inventory entry, or presentation model.
use crate::gameplay::{
    entity_lifetime::EntityId,
    player::{
        carrying::{Change, GrabDecl, HoldDecl, Release, Target},
        Player,
    },
};
use bevy::prelude::Resource;
use std::{collections::VecDeque, sync::Arc};

/// The canonical PhysicsScene implements this boundary using its real body
/// and reciprocal parent/reference-frame links (875e80/8762b0). All operations
/// refer to the already admitted entity generation; names never re-resolve it.
pub(super) trait Host {
    fn live(&self, entity: EntityId) -> bool;
    fn retired(&mut self, target: &Target) -> Result<(), String>;
    /// 76e2b0 marks the pickable unavailable and stops its physics participation;
    /// the original begin-pickup sound is posted through its actual audio owner.
    fn begin_pull(&mut self, target: &Target) -> Result<(), String>;
    /// 841500 moves toward the original tag-derived target using source
    /// velocity requests. True is the original distance/front-dot arrival test.
    fn pull(
        &mut self,
        target: &Target,
        decl: &GrabDecl,
        hold: &HoldDecl,
        player: &Player,
    ) -> Result<bool, String>;
    fn attach(&mut self, target: &Target, decl: &HoldDecl, player: &Player) -> Result<(), String>;
    /// 83cfb0 uses collision-safe placement, then 76ec80 +8762b0 restores the
    /// same body's participation and clears reciprocal parent ownership.
    fn drop(&mut self, target: &Target, decl: &HoldDecl, player: &Player) -> Result<(), String>;
    /// 83de90 uses source sensor9 target, original gravity and player velocity,
    /// queued message149, angular velocity and original Throwable callbacks.
    fn throw(&mut self, target: &Target, decl: &HoldDecl, player: &Player) -> Result<(), String>;
    /// Consumption is an explicit owner transfer, not ordinary interaction.
    fn consumed(&mut self, target: &Target) -> Result<(), String>;
}

#[derive(Resource, Default)]
pub(super) struct Carrying {
    pulling: Option<Target>,
    // Reuse the transaction buffer; no allocation on ordinary carried frames.
    changes: VecDeque<Change>,
}

impl Carrying {
    pub(super) fn has_work(&self, player: &Player) -> bool {
        self.pulling.is_some() || !self.changes.is_empty() || player.carrying.has_work()
    }
    pub(super) fn has_transactions(&self,player:&Player)->bool{
        self.pulling.is_some()||!self.changes.is_empty()||player.carrying.has_transactions()
    }
    /// Call after the player state pass, before stepping the same physics
    /// scene. Paused/loading frames do not call this gameplay update.
    pub(super) fn update(
        &mut self,
        player: &mut Player,
        host: &mut impl Host,
    ) -> Result<(), String> {
        if let Some(target)=player.carrying.held.as_ref().filter(|target|!host.live(target.entity)){
            host.retired(target)?;
            player.carrying.leave_hold();
        }
        self.flush(player, host)?;
        let Some(target) = self.pulling.as_ref() else {
            return Ok(());
        };
        if !host.live(target.entity) {
            player.carrying.cancel_grab();
            self.pulling = None;
            return self.flush(player, host);
        }
        if host.pull(target, &target.grab, &target.hold, player)? {
            let entity = target.entity;
            self.pulling = None;
            player.carried_object_reached(entity);
        }
        Ok(())
    }

    fn flush(&mut self, player: &mut Player, host: &mut impl Host) -> Result<(), String> {
        player.carrying.drain_changes(&mut self.changes);
        while let Some(change) = self.changes.front().cloned() {
            match change {
                Change::Grab(target) => {
                    if !host.live(target.entity) {
                        host.retired(&target)?;
                        player.carrying.cancel_grab();
                        self.changes.pop_front();
                        continue;
                    }
                    host.begin_pull(&target)?;
                    player.carrying.trace("begin-owner-ack",Some(target.entity),target.grab.id.0,true);
                    self.pulling = Some(target);
                }
                Change::Attach(target) => {
                    let declaration = target.hold.clone();
                    if host.live(target.entity) {
                        host.attach(&target, &declaration, player)?;
                    } else {
                        host.retired(&target)?;
                        player.carrying.leave_hold();
                    }
                }
                Change::Release { target, kind } => {
                    if self
                        .pulling
                        .as_ref()
                        .is_some_and(|value| value.entity == target.entity)
                    {
                        self.pulling = None;
                    }
                    if !host.live(target.entity) {
                        host.retired(&target)?;
                        self.changes.pop_front();
                        continue;
                    }
                    let declaration = target.hold.clone();
                    match kind {
                        Release::Drop => host.drop(&target, &declaration, player)?,
                        Release::Throw => {
                            host.throw(&target, &declaration, player)?;
                            // Only after the actual83de90 host transaction succeeds.
                            player.carrying.trace("throw-owner-ack",Some(target.entity),14,true);
                        }
                        Release::Consumed => host.consumed(&target)?,
                    }
                }
                // The state machine owns source equipment requests and shown
                // flags. The physics host must not emit them a second time.
                Change::RestoreEquipment { left: _ } => {}
            }
            // A transaction leaves the journal only after its canonical owner
            // acknowledges success; errors retain it and all later work.
            self.changes.pop_front();
        }
        Ok(())
    }

    /// Drain real detach work before the physics/managed entity owners retire.
    pub(super) fn retire(
        &mut self,
        player: &mut Player,
        host: &mut impl Host,
    ) -> Result<(), String> {
        player.carrying.retire();
        self.flush(player, host)?;
        self.pulling = None;
        Ok(())
    }
}

/// The interaction sensor passes its checked component's exact retained
/// declaration. This call never reads archives or collects its inventory item.
pub(super) fn request(
    player: &mut Player,
    entity: EntityId,
    definition: Arc<crate::gameplay::pickable_component::PickableDefinition>,
) -> bool {
    if !definition.usable_by_player || definition.hold.is_none() {
        return false;
    }
    player.request_carried_object(entity, definition)
}
