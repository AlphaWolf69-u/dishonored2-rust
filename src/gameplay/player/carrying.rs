//! LootObject48 -> HoldObject49/50 -> ReleaseObject51. Object ownership stays
//! in the live entity/component registry, never in the player's inventory.
//! 14083f590/140841500/14083d770/1408451e0/14083ce70.
use super::{combat::ActionDecl, states::StateId};
use crate::entities::scene_motion::{atom, child};
use crate::{
    assets::entities::Value,
    gameplay::{entity_lifetime::EntityId, pickable_component::PickableDefinition},
};
use bevy_math::Vec3;
use std::sync::Arc;
type Fields = [(String, Value)];

fn number(fields: &Fields, name: &str, default: f32) -> Result<f32, String> {
    atom(fields, name).map_or(Ok(default), |value| {
        value
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("invalid carrying {name}: {value}"))
    })
}
fn boolean(fields: &Fields, name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid carrying {name}: {value}")),
    }
}
fn path(fields: &Fields, name: &str) -> Option<String> {
    atom(fields, name)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
        .map(str::to_owned)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThrowSettings {
    pub speed: f32,
    pub min_pitch: f32,
    pub distance_min: f32,
    pub max_pitch: f32,
    pub distance_max: f32,
}
impl ThrowSettings {
    fn read(fields: &Fields) -> Result<Self, String> {
        Ok(Self {
            speed: number(fields, "m_throwingSpeed", 0.)?,
            min_pitch: number(fields, "m_minPitch", 0.)?,
            distance_min: number(fields, "m_distanceForMinPitch", 0.)?,
            max_pitch: number(fields, "m_maxPitch", 0.)?,
            distance_max: number(fields, "m_distanceForMaxPitch", 0.)?,
        })
    }
    /// 14083d300 interpolates target distance from the camera pitch.
    pub fn target_distance(&self, pitch: f32) -> f32 {
        let delta = self.max_pitch - self.min_pitch;
        let weight = if delta.abs() >= f32::MIN_POSITIVE {
            ((pitch - self.min_pitch) / delta).clamp(0., 1.)
        } else {
            0.
        };
        let clean = |value: f32| {
            if value.abs() <= f32::from_bits(0x219392ef) {
                0.
            } else {
                value
            }
        };
        (1. - weight) * clean(self.distance_min) + weight * clean(self.distance_max)
    }
    /// 1406dd030: original physics gravity vector, not a guessed ballistic
    /// coefficient. 14083de90 then adds player velocity and scales by .7.
    pub fn velocity(
        &self,
        origin: Vec3,
        target: Vec3,
        gravity: Vec3,
        player_velocity: Vec3,
    ) -> Option<Vec3> {
        let duration = origin.distance(target) / self.speed;
        if !duration.is_finite() || duration <= 0. {
            return None;
        }
        Some(((target - origin - gravity * duration * duration) / duration + player_velocity) * 0.7)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct HoldDecl {
    pub distance: f32,
    pub throw_fx: Option<String>,
    pub ordinary: ThrowSettings,
    pub greater: ThrowSettings,
    pub check_line_of_sight: bool,
    pub drop_label: Option<String>,
    pub throw_label: Option<String>,
}
impl HoldDecl {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        Ok(Self {
            distance: number(fields, "m_distanceInFrontOfUser", 0.5)?,
            throw_fx: path(fields, "m_throwFx"),
            ordinary: ThrowSettings::read(child(fields, "m_settingsForThrowObject"))?,
            greater: ThrowSettings::read(child(fields, "m_settingsForGreaterThrow"))?,
            check_line_of_sight: boolean(fields, "m_checkLineOfSightCollision", true)?,
            drop_label: path(fields, "m_uiInteractionStatusLabelDrop"),
            throw_label: path(fields, "m_uiInteractionStatusLabelThrow"),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct GrabDecl {
    pub id: StateId,
    pub action: ActionDecl,
    pub distance: f32,
    pub velocity: f32,
    pub max_grab_velocity: f32,
    pub grappled_linear_velocity: f32,
    pub grappled_angular_velocity: f32,
    pub inventory_full_sound: Option<String>,
    pub object_pickup_z_offset: f32,
    pub instant_loot: bool,
}
impl GrabDecl {
    pub fn read(fields: &Fields) -> Result<Self, String> {
        Ok(Self {
            id: atom(fields, "m_id")
                .and_then(super::decls::state_id)
                .unwrap_or(StateId(47)),
            action: ActionDecl::read(fields)?,
            distance: number(fields, "m_distance", 0.5)?,
            velocity: number(fields, "m_velocity", 10.)?,
            max_grab_velocity: number(fields, "m_maxGrabVelocity", 0.5)?,
            grappled_linear_velocity: number(fields, "m_grappledLinearVelocity", 15.)?,
            grappled_angular_velocity: number(fields, "m_grappledAngularVelocity", 800.)?,
            inventory_full_sound: path(fields, "m_inventoryFullSoundEvent"),
            object_pickup_z_offset: number(fields, "m_objectPickupZOffset", 0.5)?,
            instant_loot: boolean(fields, "m_instantLoot", false)?,
        })
    }
    /// 140841500's ordinary physical pickup branch. `delta` is the exact
    /// tag/center-derived target offset returned by 1408406b0. Reaching the
    /// distance threshold or passing the player's forward plane admits Hold;
    /// neither branch measures an elapsed animation duration.
    pub fn pull_velocity(&self, delta: Vec3, player_forward: Vec3, max_grab: bool) -> Option<Vec3> {
        let squared = delta.length_squared();
        if squared >= self.distance * self.distance && delta.dot(player_forward) <= 0. {
            Some(
                delta / squared.max(f32::from_bits(0x00800000)).sqrt()
                    * if max_grab {
                        self.grappled_linear_velocity
                    } else {
                        self.velocity
                    },
            )
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub(crate) struct Target {
    pub entity: EntityId,
    pub definition: Arc<PickableDefinition>,
    pub grab: Arc<GrabDecl>,
    pub hold: Arc<HoldDecl>,
}
impl Target {
    pub fn hold_state(&self) -> StateId {
        if self
            .definition
            .hold
            .as_ref()
            .is_some_and(|props| props.both_hands)
        {
            StateId(50)
        } else {
            StateId(49)
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Release {
    Drop,
    Throw,
    Consumed,
}
#[derive(Clone)]
pub(crate) enum Change {
    Grab(Target),
    Attach(Target),
    Release { target: Target, kind: Release },
    RestoreEquipment { left: bool },
}

/// Only accepted state callbacks change ownership. The host drains these
/// commands into the same rigid-body and parent-binding owners as the source.
pub struct Runtime {
    pending: Option<Target>,
    pub(crate) held: Option<Target>,
    changes: Vec<Change>,
    pub(crate) release: super::combat::ActionRuntime,
    released: bool,
    pulling_started: bool,
    debug_trace: Option<std::collections::VecDeque<(&'static str, Option<EntityId>, i32, bool)>>,
}
impl Default for Runtime {
    fn default()->Self {
        Self{pending:None,held:None,changes:Vec::new(),release:Default::default(),released:false,pulling_started:false,
            debug_trace:(std::env::var("DH2_DEBUG_ACTION_SCENARIO").ok().is_some_and(|scenario|cfg!(debug_assertions)||matches!(scenario.as_str(),"carry"|"full"|"bolt"|"bolt-bottle"))).then(std::collections::VecDeque::new)}
    }
}
impl Runtime {
    pub(crate) fn tracing_enabled(&self)->bool {self.debug_trace.is_some()}
    pub(crate) fn observed_throw_ack(&self, entity:EntityId)->bool {
        self.debug_trace.as_ref().is_some_and(|rows|rows.iter().any(|row|
            *row==("throw-owner-ack",Some(entity),14,true)))
    }
    pub(crate) fn observed_drop_ack(&self, entity:EntityId)->bool {
        self.debug_trace.as_ref().is_some_and(|rows|rows.iter().any(|row|
            *row==("drop-owner-ack",Some(entity),14,true)))
    }
    ///Debug action evidence only. Disabled gameplay retains no history or
    ///allocations; the enabled fixture keeps at most64 transition receipts.
    pub(crate) fn trace(&mut self,event:&'static str,target:Option<EntityId>,state:i32,accepted:bool){
        if let Some(rows)=&mut self.debug_trace {
            if rows.len()==64{rows.pop_front();}
            rows.push_back((event,target,state,accepted));
        }
    }
    pub(crate) fn write_trace(&self,out:&mut String){
        use std::fmt::Write;
        writeln!(out,"carry_runtime pending={:?} held={:?} pulling_started={} queued_changes={} trace={:?}",self.pending.as_ref().map(|t|t.entity),self.held.as_ref().map(|t|t.entity),self.pulling_started,self.changes.len(),self.debug_trace).unwrap();
    }
    pub(crate) fn has_work(&self) -> bool {
        self.pending.is_some() || self.held.is_some() || !self.changes.is_empty()
    }
    pub(crate) fn has_transactions(&self) -> bool {
        self.pending.is_some() || !self.changes.is_empty()
    }
    pub(crate) fn request(&mut self, target: Target) -> bool {
        let accepted=!(self.pending.is_some() || self.held.is_some() || target.definition.hold.is_none());
        self.trace("request",Some(target.entity),target.definition.pickup_interaction.player_state,accepted);
        if !accepted {
            return false;
        }
        self.pending = Some(target);
        true
    }
    pub(crate) fn grab_state(&self) -> Option<StateId> {
        self.pending
            .as_ref()
            .or(self.held.as_ref())
            .map(|target| StateId(target.definition.pickup_interaction.player_state))
    }
    pub(crate) fn pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn has_held(&self) -> bool {
        self.held.is_some()
    }
    pub(crate) fn begin_grab(&mut self) {
        self.trace("activate-grab",self.pending.as_ref().map(|t|t.entity),48,self.pending.is_some());
        if let Some(target) = &self.pending {
            if !self.pulling_started {
                self.changes.push(Change::Grab(target.clone()));
                self.pulling_started = true;
            }
        }
    }
    /// 140841500 requests Hold only after the pull has actually reached its
    /// source acceptance test. A host frame count never completes a grab.
    pub(crate) fn reached(&mut self, entity: EntityId) -> Option<StateId> {
        self.trace("physical-reached",Some(entity),48,self.pending.as_ref().is_some_and(|t|t.entity==entity));
        let target = self.pending.take_if(|target| target.entity == entity)?;
        let state = target.hold_state();
        self.held = Some(target);
        self.pulling_started = false;
        Some(state)
    }
    pub(crate) fn attach(&mut self) {
        self.trace("activate-hold",self.held.as_ref().map(|t|t.entity),self.held.as_ref().map_or(-1,|t|t.hold_state().0),self.held.is_some());
        self.released = false;
        if let Some(target) = &self.held {
            self.changes.push(Change::Attach(target.clone()));
        }
    }
    /// 1408451e0: INTERACT edge (22) takes precedence over MELEE (19).
    pub(crate) fn execute_release(
        &mut self,
        drop_edge: bool,
        throw_edge: bool,
        mode: i32,
        alternate: bool,
    ) -> bool {
        let kind = if drop_edge || (throw_edge && mode != 14) || alternate {
            Some(Release::Drop)
        } else if throw_edge {
            Some(Release::Throw)
        } else {
            None
        };
        let Some(kind) = kind else {
            return false;
        };
        self.release_object(kind);
        true
    }
    pub(crate) fn release_object(&mut self, kind: Release) {
        if self.released {
            return;
        }
        if let Some(target) = &self.held {
            self.changes.push(Change::Release {
                target: target.clone(),
                kind,
            });
            self.released = true;
        }
    }
    pub(crate) fn leave_hold(&mut self) {
        if let Some(target) = self.held.take() {
            if !self.released {
                self.changes.push(Change::Release {
                    target: target.clone(),
                    kind: Release::Drop,
                });
            }
            self.changes.push(Change::RestoreEquipment {
                left: target
                    .definition
                    .hold
                    .as_ref()
                    .is_some_and(|props| props.both_hands),
            });
        }
        self.released = false;
    }
    pub(crate) fn cancel_grab(&mut self) {
        self.trace("cancel-grab",self.pending.as_ref().map(|t|t.entity),48,self.pulling_started);
        if let Some(target) = self.pending.take() {
            if self.pulling_started {
                self.changes.push(Change::Release {
                    target,
                    kind: Release::Drop,
                });
            }
        }
        self.pulling_started = false;
    }
    pub(crate) fn drain_changes(&mut self, target: &mut std::collections::VecDeque<Change>) {
        target.extend(self.changes.drain(..));
    }
    /// Map stop/death uses the same detach/re-enable path, never destroys a
    /// still-owned world object or waits on an invented release animation.
    pub(crate) fn retire(&mut self) {
        self.cancel_grab();
        self.leave_hold();
    }
}

impl super::Player {
    /// Source interaction modes0(DEFAULT)/1(SWIMMING), using the current
    /// mode's own retained state declarations and body-part lock manager.
    pub(crate) fn carry_interaction_allowed(&self, mode: i32, state: i32) -> Option<bool> {
        let current = if self.swimming_active { 1 } else { 0 };
        if mode != current {
            return Some(false);
        }
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        machine.carry_interaction_allowed(
            StateId(state),
            &self.body_parts,
            &self.carrying,
            self.entity_event_time,
        )
    }
}

impl super::Player {
    ///Installed49/50 hold declarations own m_throwFx. Both retained mode
    ///machines share the ordinary FX source owner; no effect is instantiated.
    pub(crate) fn carrying_throw_fx_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for machine in [&self.machine, &self.swimming_machine] {
            for id in [StateId::HOLD_ONE_HAND, StateId::HOLD_TWO_HAND] {
                if let Some(state) = machine.state(id) {
                    if let super::states::Kind::HoldObject(decl) = &state.decl.kind {
                        if let Some(path) = &decl.throw_fx {
                            paths.push(path.clone());
                        }
                    }
                }
            }
        }
        paths.sort();
        paths.dedup();
        paths
    }
}

impl super::Player {
    pub(crate) fn carrying_hold_decl_for_hands(&self, both_hands: bool) -> Option<&HoldDecl> {
        let machine = if self.swimming_active {
            &self.swimming_machine
        } else {
            &self.machine
        };
        let id = if both_hands {
            StateId::HOLD_TWO_HAND
        } else {
            StateId::HOLD_ONE_HAND
        };
        machine.state(id).and_then(|state| match &state.decl.kind {
            super::states::Kind::HoldObject(decl) => Some(decl.as_ref()),
            _ => None,
        })
    }
}
