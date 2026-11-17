//! State84 GrabGrappledEntity691110/691510/691970. The state owns managed
//! targets and accepted callbacks; the host uses the existing collision world.
use super::{
    states::{Kind, StateId},
    Player,
};
use crate::gameplay::entity_lifetime::EntityId;
use bevy_math::Vec3;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub action: i32,
    pub ragdoll_distance: f32,
    pub movable_distance: f32,
    pub maximum_angle: f32,
    pub no_collision_duration: f32,
    pub occlusion_layers: u32,
}
///Actual440db0/440e90/440f30 projections from the selected Far Reach owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Targets {
    pub grappled: Option<EntityId>,
    pub movable: Option<EntityId>,
    pub ragdoll: Option<EntityId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Mask(EntityId),
    NotifyGrappled(EntityId),
    Restore(EntityId),
    ReleaseBeam,
}
#[derive(Clone, Copy)]
pub(crate) struct Tick {
    pub dt: f32,
    pub origin: Vec3,
    pub forward: Vec3,
    pub targets: Targets,
    pub phase: u8,
}
#[derive(Default)]
pub(crate) struct Runtime {
    pub targets: Targets,
    pub active: bool,
    pub target: Option<EntityId>,
    pub blocked: bool,
    pub action_pressed: bool,
    pub remaining: f32,
    changes: VecDeque<Change>,
    ///A fallible host acknowledges this accepted source Update before another
    ///one may replace its input. No input is synthesized during shutdown.
    pub tick: Option<Tick>,
}
impl Runtime {
    pub fn activate(&mut self, decl: &Declaration) {
        self.release();
        self.blocked = false;
        self.action_pressed = false;
        self.remaining = 0.;
        self.active = true;
        if let Some(id) = self.targets.grappled {
            self.target = Some(id);
            self.remaining = decl.no_collision_duration;
            self.changes.push_back(Change::Mask(id));
            self.changes.push_back(Change::NotifyGrappled(id));
        }
    }
    pub fn release(&mut self) {
        if let Some(id) = self.target.take() {
            self.changes.push_back(Change::Restore(id));
        }
    }
    pub fn deactivate(&mut self) {
        self.active = false;
        self.release();
        self.tick = None;
        self.changes.push_back(Change::ReleaseBeam);
    }
    pub fn update(&mut self, dt: f32, pressed: bool, origin: Vec3, forward: Vec3) {
        if self.tick.is_some() {
            return;
        } // Retain accepted host input until acknowledgement.
        self.action_pressed |= pressed;
        //The host must service each accepted Update. The normal gameplay
        //adapter does so before the next player state pass.
        if self.tick.is_none() {
            self.tick = Some(Tick {
                dt,
                origin,
                forward,
                targets: self.targets,
                phase: 0,
            });
        }
    }
    pub fn front(&self) -> Option<Change> {
        self.changes.front().copied()
    }
    pub fn acknowledge(&mut self, change: Change) -> Result<(), String> {
        if self.front() != Some(change) {
            return Err("GrappleCatch callback acknowledgement differs".into());
        }
        self.changes.pop_front();
        Ok(())
    }
    pub fn has_work(&self) -> bool {
        self.active || self.target.is_some() || !self.changes.is_empty() || self.tick.is_some()
    }
    ///6912c0: closest bounding-sphere edge, then the original cone predicate.
    pub fn accepts(
        &self,
        bounds: [[f32; 3]; 2],
        origin: Vec3,
        forward: Vec3,
        distance: f32,
        cosine: f32,
    ) -> bool {
        if self.blocked || !self.action_pressed {
            return false;
        }
        let lo = Vec3::from_array(bounds[0]);
        let hi = Vec3::from_array(bounds[1]);
        let center = (lo + hi) * 0.5;
        let radius = (lo - center).length();
        let delta = center - origin;
        let squared = delta.length_squared();
        let inverse = 1. / squared.max(f32::MIN_POSITIVE).sqrt();
        !(distance < inverse * squared - radius) && !(delta.dot(forward) * inverse < cosine)
    }
}
///6917c0's HUD getter cone. It deliberately has no near-distance test.
pub(crate) fn in_cone(bounds: [[f32; 3]; 2], origin: Vec3, forward: Vec3, cosine: f32) -> bool {
    let center = (Vec3::from_array(bounds[0]) + Vec3::from_array(bounds[1])) * 0.5;
    let delta = center - origin;
    cosine <= delta.dot(forward) * (1. / delta.length_squared().max(f32::MIN_POSITIVE).sqrt())
}
impl Player {
    ///8022e0 queues command1 (Request), not command2 (Quit).43f9b0 issues
    ///this only after149 and publishing the real released target/time.
    pub fn request_grapple_catch(&mut self) {
        if self.swimming_active {
            self.swimming_machine.request(StateId::GRAPPLE_CATCH);
        } else {
            self.machine.request(StateId::GRAPPLE_CATCH);
        }
    }
    pub(crate) fn loot_hold_available(&self, both_hands: bool) -> bool {
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        let hold = if both_hands {
            StateId::HOLD_TWO_HAND
        } else {
            StateId::HOLD_ONE_HAND
        };
        matches!(
            machine.state(StateId::LOOT_OBJECT).map(|s| &s.decl.kind),
            Some(Kind::LootObject(_))
        ) && matches!(
            machine.state(hold).map(|s| &s.decl.kind),
            Some(Kind::HoldObject(_))
        )
    }
    pub(crate) fn loot_hold_target(
        &self,
        id: EntityId,
        definition: std::sync::Arc<crate::gameplay::pickable_component::PickableDefinition>,
    ) -> Result<Option<super::carrying::Target>, String> {
        let Some(props) = &definition.hold else {
            return Ok(None);
        };
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        let grab = match &machine
            .state(StateId::LOOT_OBJECT)
            .ok_or("Loot47 declaration absent")?
            .decl
            .kind
        {
            Kind::LootObject(d) => d.clone(),
            _ => return Err("Loot47 declaration class differs".into()),
        };
        let hold_state = if props.both_hands {
            StateId::HOLD_TWO_HAND
        } else {
            StateId::HOLD_ONE_HAND
        };
        let hold = match &machine
            .state(hold_state)
            .ok_or("Loot47 Hold declaration absent")?
            .decl
            .kind
        {
            Kind::HoldObject(d) => d.clone(),
            _ => return Err("Loot47 Hold declaration class differs".into()),
        };
        Ok(Some(super::carrying::Target {
            entity: id,
            definition,
            grab,
            hold,
        }))
    }
    ///841500's hold branch enters49/50 directly; physical pull was owned by
    ///the accepted47 transaction, so no Grab48 or duplicate Begin is issued.
    pub(crate) fn receive_loot_hold(
        &mut self,
        target: super::carrying::Target,
    ) -> Result<(), String> {
        if self.carrying.has_work() {
            return Err("Loot47 hold transfer conflicts with an owned carry transaction".into());
        }
        let id = target.hold_state();
        let machine = if self.swimming_active {
            &mut self.swimming_machine
        } else {
            &mut self.machine
        };
        if !matches!(
            machine.state(id).map(|s| &s.decl.kind),
            Some(Kind::HoldObject(_))
        ) {
            return Err("Loot47 target mode has no actual Hold declaration".into());
        }
        self.carrying.held = Some(target);
        machine.request(id);
        Ok(())
    }
    pub fn set_grapple_catch_targets(&mut self, targets: Targets) {
        self.machine.grapple_catch.targets = targets;
        self.swimming_machine.grapple_catch.targets = targets;
    }
    pub fn quit_grapple_catch(&mut self) {
        self.machine.quit(StateId::GRAPPLE_CATCH);
        self.swimming_machine.quit(StateId::GRAPPLE_CATCH);
    }
    ///691750/6916e0 are HUD getters, not CanActivate. They deliberately do
    ///not test active-state membership; the source tests only30/31 and the
    ///real Far Reach component's current retained target projections.
    pub(crate) fn grapple_catch_prompt_candidates(&self) -> Option<([Option<EntityId>; 2], f32)> {
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        let Kind::GrappleCatch(decl) = &machine.state(StateId::GRAPPLE_CATCH)?.decl.kind else {
            return None;
        };
        let run = &machine.grapple_catch;
        if run.blocked || run.action_pressed {
            return None;
        }
        Some((
            [run.targets.ragdoll, run.targets.movable],
            super::states::cos_snap(decl.maximum_angle),
        ))
    }
    pub(crate) fn grapple_catch_has_work(&self) -> bool {
        self.machine.grapple_catch.has_work() || self.swimming_machine.grapple_catch.has_work()
    }
    pub(crate) fn grapple_catch_declaration(&self, mode: usize) -> Option<&Declaration> {
        let machine = if mode == 0 {
            &self.machine
        } else {
            &self.swimming_machine
        };
        match &machine.state(StateId::GRAPPLE_CATCH)?.decl.kind {
            Kind::GrappleCatch(d) => Some(d),
            _ => None,
        }
    }
    pub(crate) fn grapple_catch_runtime(&mut self, mode: usize) -> &mut Runtime {
        if mode == 0 {
            &mut self.machine.grapple_catch
        } else {
            &mut self.swimming_machine.grapple_catch
        }
    }
    pub(crate) fn finish_grapple_catch(&mut self) {
        self.machine.grapple_catch.deactivate();
        self.swimming_machine.grapple_catch.deactivate();
        self.quit_grapple_catch();
    }
}
