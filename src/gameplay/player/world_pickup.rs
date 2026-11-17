//!47/0x2f LootObject's real target ID and callback lifetime.841500 offers the
//!inventory callback on physical reach;83fe70 offers remaining targets when
//!the state deactivates. The host owns actual Pickable/item resource leases.
use super::{
    combat::ActionRuntime,
    states::{Kind, StateId},
};
use crate::gameplay::entity_lifetime::EntityId;
use std::collections::VecDeque;
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Runtime {
    pending: Option<EntityId>,
    target: Option<EntityId>,
    offered: bool,
    pub(super) offered_from_exit:bool,
    rejected: VecDeque<EntityId>,
    pub action: ActionRuntime,
    pub scene_started: bool,
}
impl Runtime {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.target.is_some() || !self.rejected.is_empty()
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn request(&mut self, id: EntityId) -> bool {
        if self.busy() {
            false
        } else {
            self.pending = Some(id);
            true
        }
    }
    pub fn activate(&mut self) {
        self.target = self.pending.take();
        self.offered = false;
        self.offered_from_exit=false;
        self.scene_started = false;
    }
    pub fn reject(&mut self) {
        if let Some(id) = self.pending.take() {
            self.rejected.push_back(id);
        }
    }
    pub fn target(&self) -> Option<EntityId> {
        self.target
    }
    pub fn offer(&mut self) {
        if self.target.is_some() {
            self.offered = true;
        }
    }
    pub fn offer_from_exit(&mut self) {
        if !self.offered {self.offered_from_exit=true;}
        self.offer();
    }
    pub fn accepted(&self) -> Option<EntityId> {
        self.target.filter(|_| self.offered)
    }
    pub fn rejected(&mut self) -> Option<EntityId> {
        self.rejected.pop_front()
    }
    pub fn lose(&mut self, id: EntityId) -> Result<(), String> {
        if self.target == Some(id) {
            self.target = None;
            self.offered = false;
            self.rejected.push_back(id);
            return Ok(());
        }
        if self.pending == Some(id) {
            self.reject();
            return Ok(());
        }
        Err("lost loot target does not match the retained request".into())
    }
    pub fn reject_active(&mut self, id: EntityId) -> Result<(), String> {
        if self.target != Some(id) {
            return Err("full loot rejection does not match an uncommitted active target".into());
        }
        self.target = None;
        self.offered = false;
        self.rejected.push_back(id);
        Ok(())
    }
    pub fn acknowledge(&mut self, id: EntityId) -> Result<(), String> {
        if self.accepted() != Some(id) {
            return Err("loot callback does not match the accepted target".into());
        }
        self.target = None;
        self.offered = false;
        Ok(())
    }
}
impl super::Player {
    pub fn loot_interaction_allowed(&self, mode: i32, state: i32) -> Option<bool> {
        let current = i32::from(self.swimming_active);
        if current != mode {
            return Some(false);
        }
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        machine.world_pickup_allowed(StateId(state), &self.body_parts, self.entity_event_time)
    }
    pub fn request_loot_object(&mut self, id: EntityId) -> bool {
        if self.machine.world_pickup_busy() || self.swimming_machine.world_pickup_busy() {
            return false;
        }
        let machine = if self.swimming_active {
            &mut self.swimming_machine
        } else {
            &mut self.machine
        };
        machine.request_world_pickup(id)
    }
    pub fn world_pickup_target(&self) -> Option<EntityId> {
        self.machine
            .world_pickup_target()
            .or_else(|| self.swimming_machine.world_pickup_target())
    }
    pub fn world_pickup_mode(&self) -> Option<usize> {
        [&self.machine, &self.swimming_machine]
            .into_iter()
            .position(|machine| machine.world_pickup_target().is_some())
    }
    pub fn world_pickup_decl(&self) -> Option<&super::carrying::GrabDecl> {
        [&self.machine, &self.swimming_machine]
            .into_iter()
            .find(|machine| machine.world_pickup_target().is_some())
            .and_then(|machine| machine.state(StateId::LOOT_OBJECT))
            .and_then(|state| match &state.decl.kind {
                Kind::LootObject(decl) => Some(decl.as_ref()),
                _ => None,
            })
    }
    pub fn world_pickup_reached(&mut self, id: EntityId) -> Result<(), String> {
        for machine in [&mut self.machine, &mut self.swimming_machine] {
            if machine.world_pickup_target() == Some(id) {
                machine.offer_world_pickup();
                return Ok(());
            }
        }
        Err("loot physical completion has no accepted target".into())
    }
    pub(crate) fn loot_offered_from_exit(&self,id:EntityId)->bool {
        [&self.machine,&self.swimming_machine].into_iter().any(|m|m.world_pickup_target()==Some(id)&&m.world_pickup_from_exit())
    }
    pub fn pending_loot_object(&self) -> Option<EntityId> {
        self.machine
            .accepted_world_pickup()
            .or_else(|| self.swimming_machine.accepted_world_pickup())
    }
    pub fn acknowledge_loot_object(&mut self, id: EntityId) -> Result<(), String> {
        for machine in [&mut self.machine, &mut self.swimming_machine] {
            if machine.world_pickup_target() == Some(id) {
                return machine.acknowledge_world_pickup(id);
            }
        }
        Err("loot acknowledgement target is absent".into())
    }
    ///841500's checked-ID miss drops that entry without inventory or a
    ///full-inventory sound. The host calls this before committing its journal.
    pub fn lose_loot_object(&mut self, id: EntityId) -> Result<(), String> {
        for machine in [&mut self.machine, &mut self.swimming_machine] {
            if machine.world_pickup_busy() {
                if machine.lose_world_pickup(id).is_ok() {
                    return Ok(());
                }
            }
        }
        Err("lost loot target is absent".into())
    }
    pub fn reject_loot_object(&mut self, id: EntityId) -> Result<(), String> {
        for machine in [&mut self.machine, &mut self.swimming_machine] {
            if machine.world_pickup_target() == Some(id) {
                return machine.reject_world_pickup(id);
            }
        }
        Err("full loot rejection target is absent".into())
    }
    pub fn take_rejected_loot_object(&mut self) -> Option<EntityId> {
        self.machine
            .rejected_world_pickup()
            .or_else(|| self.swimming_machine.rejected_world_pickup())
    }
    ///83fe70 deactivation commits accepted remaining targets; unaccepted
    ///requests return their actual ID to the host lease owner.
    pub fn finish_loot_object(&mut self) {
        self.machine.finish_world_pickup();
        self.swimming_machine.finish_world_pickup();
    }
}
