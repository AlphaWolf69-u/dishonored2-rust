//! Retained source constraint rows attached to the SAME canonical body world.
//!712da0/713850 create; c91140 enables; c91030 removes. No body simulator here.
use super::*;
use crate::{
    entities::machinery::ConstraintSettings,
    gameplay::constraint_solver::{
        ball_socket::SolverVelocities,
        joint::{solve_hinge, BodyInput, Joint, JointRows},
        joint_atoms::{self, AtomJoint},
        step::{ConstraintParameters, SolverParameters},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JointId {
    slot: usize,
    generation: u32,
}
impl JointId {
    pub(crate) fn slot(self) -> usize {
        self.slot
    }
    pub(crate) fn generation(self) -> u32 {
        self.generation
    }
}
/// On-demand diagnostic values; no retained snapshot/cache or simulated body.
#[derive(Debug)]
pub(crate) struct JointObservation {
    pub id: JointId,
    pub bodies: [u32; 2],
    pub motions: [u32; 2],
    pub body_flags: [u32; 2],
    pub solver_ids: [u32; 2],
    pub frames: [crate::gameplay::shape_transform::ShapeMatrix; 2],
    pub linear: [[f32; 4]; 2],
    pub angular: [[f32; 4]; 2],
    pub linked: [bool; 2],
    pub enabled: bool,
    pub prepared: bool,
    pub transition_pending: bool,
    pub timespace: usize,
    pub factor: f32,
    pub kernel: &'static str,
    pub cone_limit: Option<[f32; 2]>,
    pub cone_axis_dot: Option<f32>,
    pub pivot_separation_squared: f32,
}
enum Kernel {
    Hinge(Joint),
    Atom(AtomJoint),
}
struct Entry {
    bodies: [u32; 2],
    generation: u32,
    enabled: bool,
    prepared: bool,
    kernel: Kernel,
    hinge_rows: Option<JointRows>,
    enable: Option<EnableProgress>,
    timespace: usize,
    factor: f32,
    scale: Option<(f32, f32)>,
}
#[derive(Clone, Copy)]
struct EnableProgress {
    cursor: usize,
    phase: u8,
    wake: bool,
    enabled: bool,
}
#[derive(Default)]
pub(super) struct JointOwner {
    entries: Vec<Option<Entry>>,
    generations: Vec<u32>,
    free: Vec<usize>,
    factors: Option<[u32; 2]>,
}
impl JointOwner {
    fn entry(&self, id: JointId) -> Result<&Entry, String> {
        self.entries
            .get(id.slot)
            .and_then(Option::as_ref)
            .filter(|entry| entry.generation == id.generation)
            .ok_or("constraint joint generation is stale".into())
    }
    fn entry_mut(&mut self, id: JointId) -> Result<&mut Entry, String> {
        self.entries
            .get_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|entry| entry.generation == id.generation)
            .ok_or("constraint joint generation is stale".into())
    }
}
impl WorldStorage {
    fn joint_mutation_ready(&self) -> Result<(), String> {
        if !self.publication_idle() || !self.simulation_idle() {
            return Err("constraint mutation awaits canonical publication/solver boundary".into());
        }
        Ok(())
    }
    pub(crate) fn add_joint(
        &mut self,
        bodies: [u32; 2],
        settings: ConstraintSettings,
        timespace: usize,
    ) -> Result<JointId, String> {
        self.joint_mutation_ready()?;
        self.motion_timespaces
            .as_ref()
            .ok_or("joint physics timespaces absent")?
            .factor(timespace)?;
        if bodies[0] == bodies[1] {
            return Err(
                "equal source constraint endpoints must skip construction before add_joint".into(),
            );
        }
        if settings.linear_constraint != "LINEAR_CONSTRAINT_FIXED"
            || !matches!(
                settings.angular_constraint.as_str(),
                "ANGULAR_CONSTRAINT_HINGE"
                    | "ANGULAR_CONSTRAINT_LIMITED_HINGE"
                    | "ANGULAR_CONSTRAINT_CONE"
            )
        {
            return Err(format!(
                "constraint {}/{} has no admitted numerical atom stack",
                settings.linear_constraint, settings.angular_constraint
            ));
        }
        if settings.angular_axis >= 3 {
            return Err("constraint angular axis exceeds source vector lanes".into());
        }
        let a = self
            .bodies
            .get(bodies[0])
            .ok_or("constraint body A generation absent")?;
        let b = self
            .bodies
            .get(bodies[1])
            .ok_or("constraint body B generation absent")?;
        let ma = self
            .motions
            .get(a.motion_id)
            .ok_or("constraint body A motion absent")?;
        let mb = self
            .motions
            .get(b.motion_id)
            .ok_or("constraint body B motion absent")?;
        let a_input = BodyInput {
            pose: &ma.pose,
            inverse_mass: &ma.inverse_mass,
            body_transform: &a.transform,
        };
        let b_input = BodyInput {
            pose: &mb.pose,
            inverse_mass: &mb.inverse_mass,
            body_transform: &b.transform,
        };
        let kernel = if settings.angular_constraint == "ANGULAR_CONSTRAINT_CONE" {
            Kernel::Atom(AtomJoint::new(
                0,
                0,
                joint_atoms::cone_stack(&settings, a_input, b_input),
                true,
            ))
        } else {
            Kernel::Hinge(Joint::new(settings, 0, 0, a_input, b_input))
        };
        let slot = if let Some(&slot) = self.joint_owner.free.last() {
            slot
        } else {
            self.constraints.len()
        };
        if slot >= 0x7fff_ffff {
            return Err("constraint slots exhausted source ID range".into());
        }
        let generation = self
            .joint_owner
            .generations
            .get(slot)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("constraint joint generation exhausted")?;
        let required = bodies
            .iter()
            .map(|body| (body & 0x00ff_ffff) as usize + 1)
            .max()
            .unwrap();
        self.constraint_heads
            .resize(self.constraint_heads.len().max(required), 0x7fff_ffff);
        let next = bodies.map(|body| self.constraint_heads[(body & 0x00ff_ffff) as usize]);
        let link = ConstraintLink {
            bodies,
            flags: 4,
            link_index: slot,
        };
        let entry = Entry {
            bodies,
            generation,
            enabled: false,
            prepared: false,
            kernel,
            hinge_rows: None,
            enable: None,
            timespace,
            factor: 1.,
            scale: None,
        };
        if slot == self.constraints.len() {
            self.constraints.push(link);
            self.constraint_next.push(next);
            self.joint_owner.entries.push(Some(entry));
            self.joint_owner.generations.push(generation);
        } else {
            self.joint_owner.free.pop();
            self.constraints[slot] = link;
            self.constraint_next[slot] = next;
            self.joint_owner.entries[slot] = Some(entry);
            self.joint_owner.generations[slot] = generation;
        }
        for body in bodies {
            self.constraint_heads[(body & 0x00ff_ffff) as usize] = slot as u32;
        }
        self.joint_owner.factors = None;
        Ok(JointId { slot, generation })
    }
    /// Host startup boundary: component Start records its real enable request,
    /// then the existing shared Start batch publishes BOTH endpoint bodies.
    /// Never synthesize sequential Added notifications for c92dd0's batch.
    pub(crate) fn joint_endpoints_ready(&self, id: JointId) -> Result<bool, String> {
        for body in self.joint_owner.entry(id)?.bodies {
            let actual = self
                .bodies
                .get(body)
                .ok_or("constraint endpoint generation retired before joint removal")?;
            if body != 0 && actual.broadphase_id == u32::MAX {
                return Ok(false);
            }
            let change = self.bodies.change_indices()[(body & 0xffffff) as usize];
            if change != u32::MAX
                && self
                    .bodies
                    .changes()
                    .get(change as usize)
                    .ok_or("constraint pending-add record absent")?
                    .queue_index
                    != u32::MAX
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// c91140/c910d0 -> c92dd0, after the actual canonical startup batch.
    /// Its pending-add branch is deliberately not emulated per endpoint.
    pub(crate) fn set_joint_enabled<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: JointId,
        enabled: bool,
        wake: bool,
    ) -> Result<(), String> {
        // An accepted callback continuation owns its existing body-publication
        // cursor; the ordinary idle fence applies before accepting a new one.
        if self.joint_owner.entry(id)?.enable.is_none() {
            self.joint_mutation_ready()?;
        }
        if !self.simulation_idle() {
            return Err("constraint enable awaits solver boundary".into());
        }
        let entry = self.joint_owner.entry(id)?;
        if entry
            .enable
            .is_some_and(|progress| progress.enabled != enabled)
        {
            return Err("another constraint transition is incomplete".into());
        }
        if entry.enabled == enabled && entry.enable.is_none() {
            return Ok(());
        }
        if entry.enable.is_none() {
            let bodies = entry.bodies;
            let valid = self.info.enable_deactivation
                && bodies
                    .iter()
                    .all(|&id| self.bodies.get(id).is_some_and(|body| body.flags & 3 != 0));
            if !self.joint_endpoints_ready(id)? {
                return Err(
                    "constraint transition awaits successful shared endpoint-publication batch"
                        .into(),
                );
            }
            let progress = EnableProgress {
                cursor: 0,
                phase: if valid { 2 } else { 4 },
                wake,
                enabled,
            };
            self.joint_owner.entry_mut(id)?.enable = Some(progress);
            if enabled {
                self.constraints[id.slot].flags &= !4;
            } else {
                self.constraints[id.slot].flags |= 4;
            }
            if !enabled {
                self.joint_owner.entry_mut(id)?.enabled = false;
            }
        }
        loop {
            let progress = self
                .joint_owner
                .entry(id)?
                .enable
                .ok_or("constraint enable cursor absent")?;
            let bodies = self.joint_owner.entry(id)?.bodies;
            match progress.phase {
                2 => {
                    if !progress.wake && progress.cursor < 2 {
                        self.activate_body(geometry, events, bodies[progress.cursor])?;
                        self.joint_owner
                            .entry_mut(id)?
                            .enable
                            .as_mut()
                            .unwrap()
                            .cursor += 1;
                    } else {
                        self.joint_owner
                            .entry_mut(id)?
                            .enable
                            .as_mut()
                            .unwrap()
                            .phase = 3;
                    }
                }
                3 => {
                    if bodies.iter().all(|&id| {
                        self.bodies
                            .get(id)
                            .is_some_and(|body| body.broadphase_id != u32::MAX)
                    }) {
                        self.deactivation.link_bodies(
                            &self.bodies,
                            &self.motions,
                            bodies[0],
                            bodies[1],
                            self.step_index,
                        )?;
                    }
                    self.joint_owner
                        .entry_mut(id)?
                        .enable
                        .as_mut()
                        .unwrap()
                        .phase = 4;
                }
                4 => {
                    events.constraint_changed(
                        crate::gameplay::body_activation::ActivationOwners {
                            bodies: &mut self.bodies,
                            motions: &mut self.motions,
                            deactivation: &mut self.deactivation,
                            broadphase: &mut self.broadphase,
                        },
                        id.slot,
                        id.generation,
                        if enabled {
                            super::super::events::ConstraintChange::Enabled
                        } else {
                            super::super::events::ConstraintChange::Disabled
                        },
                    )?;
                    let entry = self.joint_owner.entry_mut(id)?;
                    entry.enabled = enabled;
                    entry.enable = None;
                    self.joint_owner.factors = None;
                    return Ok(());
                }
                _ => return Err("constraint enable phase exceeds source transition".into()),
            }
        }
    }
    //852040/851b80 pair-reference suppression is not admitted here: NULL
    //PhysicsBase denotes the actual PhysicsWorld body batch, not anchor0,
    //and its reference-count decrement lifetime is not yet source-proven.
    //The typed passive admission therefore requires disableCollision=false.
    fn unlink_joint_side(&mut self, slot: usize, body: u32) -> Result<(), String> {
        let head = self
            .constraint_heads
            .get_mut((body & 0x00ff_ffff) as usize)
            .ok_or("constraint adjacency head absent")?;
        let side = usize::from(self.constraints[slot].bodies[0] != body);
        let replacement = self.constraint_next[slot][side];
        if *head == slot as u32 {
            *head = replacement;
            return Ok(());
        }
        let mut current = *head;
        let mut visited = 0;
        while current != 0x7fff_ffff {
            if visited >= self.constraints.len() {
                return Err("cyclic constraint removal adjacency".into());
            }
            let link = self
                .constraints
                .get(current as usize)
                .ok_or("constraint removal link absent")?;
            let previous_side = usize::from(link.bodies[0] != body);
            let next = self
                .constraint_next
                .get_mut(link.link_index)
                .ok_or("constraint removal next pair absent")?;
            if next[previous_side] == slot as u32 {
                next[previous_side] = replacement;
                return Ok(());
            }
            current = next[previous_side];
            visited += 1;
        }
        Err("constraint removal generation is not linked to its body".into())
    }
    pub(crate) fn remove_joint<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: JointId,
        wake: bool,
    ) -> Result<(), String> {
        // A failed Init/Start may already own a partially acknowledged
        // transition. Drain that SAME request before accepting its inverse.
        if let Some(progress) = self.joint_owner.entry(id)?.enable {
            self.set_joint_enabled(geometry, events, id, progress.enabled, progress.wake)?;
        }
        if self.joint_owner.entry(id)?.scale.is_some() {
            self.synchronize_joint_time(geometry, events, id)?;
        }
        self.set_joint_enabled(geometry, events, id, false, wake)?;
        events.constraint_changed(
            crate::gameplay::body_activation::ActivationOwners {
                bodies: &mut self.bodies,
                motions: &mut self.motions,
                deactivation: &mut self.deactivation,
                broadphase: &mut self.broadphase,
            },
            id.slot,
            id.generation,
            super::super::events::ConstraintChange::Removed,
        )?;
        let bodies = self.joint_owner.entry(id)?.bodies;
        self.unlink_joint_side(id.slot, bodies[0])?;
        self.unlink_joint_side(id.slot, bodies[1])?;
        self.constraints[id.slot].flags |= 4;
        self.constraint_next[id.slot] = [0x7fff_ffff; 2];
        self.joint_owner.entries[id.slot] = None;
        self.joint_owner.free.push(id.slot);
        Ok(())
    }
    /// Host consolidation of7164c0 at the canonical body-removal boundary:
    /// numeric removal signals invalidate the SAME component before body reuse.
    pub(super) fn retire_body_joints<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        bodies: &[u32],
    ) -> Result<(), String> {
        for &body in bodies {
            self.bodies
                .get(body)
                .ok_or("constraint endpoint removal generation is stale")?;
            loop {
                let head = self
                    .constraint_heads
                    .get((body & 0xffffff) as usize)
                    .copied()
                    .unwrap_or(0x7fff_ffff);
                if head == 0x7fff_ffff {
                    break;
                }
                let entry = self
                    .joint_owner
                    .entries
                    .get(head as usize)
                    .and_then(Option::as_ref)
                    .ok_or("removed endpoint constraint owner absent")?;
                if !entry.bodies.contains(&body) {
                    return Err("removed endpoint constraint generation mismatches".into());
                }
                self.remove_joint(
                    geometry,
                    events,
                    JointId {
                        slot: head as usize,
                        generation: entry.generation,
                    },
                    true,
                )?;
            }
        }
        Ok(())
    }
    pub(crate) fn set_joint_timespace<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: JointId,
        timespace: usize,
    ) -> Result<(), String> {
        self.motion_timespaces
            .as_ref()
            .ok_or("joint physics timespaces absent")?
            .factor(timespace)?;
        self.joint_owner.entry_mut(id)?.timespace = timespace;
        self.synchronize_joint_time(geometry, events, id)
    }
    pub(crate) fn synchronize_joint_time<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: JointId,
    ) -> Result<(), String> {
        let entry = self.joint_owner.entry(id)?;
        if entry.scale.is_none() {
            let factor = self
                .motion_timespaces
                .as_ref()
                .ok_or("joint physics timespaces absent")?
                .factor(entry.timespace)?;
            let old = entry.factor;
            if !(entry.enabled || old == 0.) || !((factor - old).abs() > f32::from_bits(0x34000000))
            {
                return Ok(());
            }
            let entry = self.joint_owner.entry_mut(id)?;
            entry.factor = factor;
            entry.scale = Some((old, factor));
        }
        let (old, factor) = self
            .joint_owner
            .entry(id)?
            .scale
            .ok_or("joint time synchronization cursor absent")?;
        if factor == 0. {
            self.set_joint_enabled(geometry, events, id, false, false)?;
        } else if old == 0. {
            self.set_joint_enabled(geometry, events, id, true, true)?;
        }
        //098890 ->c9cfe0 caches factor in the malleable data; row building
        //uses this exact value through098ca0's coefficient interpolation.
        self.joint_owner.entry_mut(id)?.scale = None;
        Ok(())
    }
    pub(super) fn synchronize_joint_timespaces<
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
    >(
        &mut self,
        geometry: &mut G,
        events: &mut E,
    ) -> Result<(), String> {
        if self.joint_owner.entries.len() == self.joint_owner.free.len() {
            return Ok(());
        }
        let spaces = self
            .motion_timespaces
            .as_ref()
            .ok_or("joint physics timespaces absent")?;
        let factors = [spaces.factor(0)?.to_bits(), spaces.factor(1)?.to_bits()];
        if self.joint_owner.factors == Some(factors) {
            return Ok(());
        }
        for slot in 0..self.joint_owner.entries.len() {
            if let Some(entry) = self.joint_owner.entries[slot].as_ref() {
                self.synchronize_joint_time(
                    geometry,
                    events,
                    JointId {
                        slot,
                        generation: entry.generation,
                    },
                )?;
            }
        }
        self.joint_owner.factors = Some(factors);
        Ok(())
    }
    pub(crate) fn constraint_counts(&self) -> (usize, usize, usize) {
        let slots = self.joint_owner.entries.len();
        let free = self.joint_owner.free.len();
        (slots, slots - free, free)
    }
    pub(crate) fn observe_joint(&self, id: JointId) -> Result<JointObservation, String> {
        use crate::gameplay::{
            constraint_solver::ball_socket::rotate_direction, shape_transform::compose,
        };
        let entry = self.joint_owner.entry(id)?;
        let a = self
            .bodies
            .get(entry.bodies[0])
            .ok_or("observed constraint body A generation absent")?;
        let b = self
            .bodies
            .get(entry.bodies[1])
            .ok_or("observed constraint body B generation absent")?;
        let ma = self
            .motions
            .get(a.motion_id)
            .ok_or("observed constraint motion A absent")?;
        let mb = self
            .motions
            .get(b.motion_id)
            .ok_or("observed constraint motion B absent")?;
        let (frames, kernel, cone_limit, cone_axis_dot) = match &entry.kernel {
            Kernel::Atom(joint) => {
                let frames = [
                    compose(a.transform, joint.stack.transform_a),
                    compose(b.transform, joint.stack.transform_b),
                ];
                let dot = joint.stack.cone.map(|limit| {
                    let left = frames[0][usize::from(limit.axis)];
                    let right = frames[1][usize::from(limit.reference)];
                    (left[1] * right[1] + left[0] * right[0]) + left[2] * right[2]
                });
                (
                    frames,
                    "cone",
                    joint.stack.cone.map(|limit| [limit.minimum, limit.maximum]),
                    dot,
                )
            }
            Kernel::Hinge(joint) => {
                let frame = |motion: &crate::gameplay::motion::Motion,
                             local: [[f32; 4]; 3],
                             lever: [f32; 3]| {
                    let mut result = [[0.; 4]; 4];
                    for axis in 0..3 {
                        let value = rotate_direction(
                            motion.pose.orientation,
                            [local[axis][0], local[axis][1], local[axis][2]],
                        );
                        result[axis] = [value[0], value[1], value[2], 0.];
                    }
                    let offset = rotate_direction(motion.pose.orientation, lever);
                    result[3] = [
                        motion.pose.center[0] + offset[0],
                        motion.pose.center[1] + offset[1],
                        motion.pose.center[2] + offset[2],
                        1.,
                    ];
                    result
                };
                (
                    [
                        frame(ma, joint.local_a, joint.lever_a),
                        frame(mb, joint.local_b, joint.lever_b),
                    ],
                    "hinge",
                    None,
                    None,
                )
            }
        };
        let linked = |body: u32| -> Result<bool, String> {
            let mut current = self
                .constraint_heads
                .get((body & 0xffffff) as usize)
                .copied()
                .unwrap_or(0x7fff_ffff);
            let mut visited = 0;
            while current != 0x7fff_ffff {
                if visited >= self.constraints.len() {
                    return Err("observed constraint adjacency cyclic".into());
                }
                if current as usize == id.slot {
                    return Ok(true);
                }
                let link = self
                    .constraints
                    .get(current as usize)
                    .ok_or("observed constraint link absent")?;
                let side = usize::from(link.bodies[0] != body);
                current = self
                    .constraint_next
                    .get(link.link_index)
                    .ok_or("observed constraint next absent")?[side];
                visited += 1;
            }
            Ok(false)
        };
        let difference = [
            frames[0][3][0] - frames[1][3][0],
            frames[0][3][1] - frames[1][3][1],
            frames[0][3][2] - frames[1][3][2],
        ];
        Ok(JointObservation {
            id,
            bodies: entry.bodies,
            motions: [a.motion_id, b.motion_id],
            body_flags: [a.flags, b.flags],
            solver_ids: [ma.solver_id, mb.solver_id],
            frames,
            linear: [ma.linear_velocity, mb.linear_velocity],
            angular: [ma.angular_velocity, mb.angular_velocity],
            linked: [linked(a.id)?, linked(b.id)?],
            enabled: entry.enabled,
            prepared: entry.prepared,
            transition_pending: entry.enable.is_some() || entry.scale.is_some(),
            timespace: entry.timespace,
            factor: entry.factor,
            kernel,
            cone_limit,
            cone_axis_dot,
            pivot_separation_squared: (difference[1] * difference[1]
                + difference[0] * difference[0])
                + difference[2] * difference[2],
        })
    }
    pub(super) fn validate_no_joints(&self) -> Result<(), String> {
        if self.joint_owner.entries.len() != self.joint_owner.free.len() {
            return Err(
                "physics world release awaits actual constraint removal acknowledgement".into(),
            );
        }
        Ok(())
    }
    pub(super) fn validate_body_no_joint(&self, body: u32) -> Result<(), String> {
        if self
            .constraint_heads
            .get((body & 0xffffff) as usize)
            .is_some_and(|&head| head != 0x7fff_ffff)
        {
            return Err(
                "body resource release precedes actual constraint removal acknowledgement".into(),
            );
        }
        Ok(())
    }
    pub(super) fn enabled_joint_pairs(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        self.joint_owner
            .entries
            .iter()
            .flatten()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.bodies)
    }
    pub(super) fn joint_slot_count(&self) -> usize {
        self.joint_owner.entries.len()
    }
    pub(super) fn enabled_joint_pair_at(&self, index: usize) -> Option<[u32; 2]> {
        self.joint_owner
            .entries
            .get(index)?
            .as_ref()
            .filter(|entry| entry.enabled)
            .map(|entry| entry.bodies)
    }
    /// Build rows from the current real bodies/motions once, after contact
    /// activation populated the world's existing active solver indices.
    pub(super) fn prepare_joints(
        &mut self,
        step: &SolverParameters,
        mapping: &[u32],
        velocities: &mut Vec<SolverVelocities>,
    ) -> Result<(), String> {
        let mut fixed = None;
        for entry in self.joint_owner.entries.iter_mut().flatten() {
            entry.prepared = false;
            entry.hinge_rows = None;
            if !entry.enabled {
                if let Kernel::Atom(joint) = &mut entry.kernel {
                    joint.skip();
                }
                continue;
            }
            let a = self
                .bodies
                .get(entry.bodies[0])
                .ok_or("enabled constraint body A generation retired")?;
            let b = self
                .bodies
                .get(entry.bodies[1])
                .ok_or("enabled constraint body B generation retired")?;
            if (a.id != 0 && a.broadphase_id == u32::MAX)
                || (b.id != 0 && b.broadphase_id == u32::MAX)
            {
                continue;
            }
            let ma = self
                .motions
                .get(a.motion_id)
                .ok_or("enabled constraint motion A absent")?;
            let mb = self
                .motions
                .get(b.motion_id)
                .ok_or("enabled constraint motion B absent")?;
            let mut indices = [0usize; 2];
            let mut active = true;
            for (side, motion) in [a.motion_id, b.motion_id].into_iter().enumerate() {
                if motion == 0 {
                    if let Some(index) = fixed {
                        indices[side] = index;
                    } else {
                        let index = velocities.len();
                        // Actual world's fixed motion0, not a fabricated body.
                        let motion = self
                            .motions
                            .get(0)
                            .ok_or("constraint fixed motion0 absent")?;
                        let mut properties = self
                            .motion_properties
                            .get(motion.properties_id)
                            .ok_or("constraint fixed-motion property absent")?
                            .solver_properties();
                        properties.gravity_factor = 0.;
                        velocities.push(SolverVelocities::begin_motion(
                            [
                                motion.linear_velocity[0],
                                motion.linear_velocity[1],
                                motion.linear_velocity[2],
                            ],
                            [
                                motion.angular_velocity[0],
                                motion.angular_velocity[1],
                                motion.angular_velocity[2],
                            ],
                            motion
                                .inverse_mass
                                .map(|v| f32::from_bits(u32::from(v) << 16)),
                            properties,
                            step,
                        ));
                        fixed = Some(index);
                        indices[side] = index;
                    }
                } else if let Some(&index) = mapping
                    .get(motion as usize)
                    .filter(|&&index| index != u32::MAX)
                {
                    indices[side] = index as usize;
                } else {
                    active = false;
                    break;
                }
            }
            if !active || indices[0] == indices[1] {
                if let Kernel::Atom(joint) = &mut entry.kernel {
                    joint.skip();
                }
                continue;
            }
            let query = ConstraintParameters::from_timespace(step, entry.factor);
            let atom_step = joint_atoms::atom_step(step, &query);
            let a_input = BodyInput {
                pose: &ma.pose,
                inverse_mass: &ma.inverse_mass,
                body_transform: &a.transform,
            };
            let b_input = BodyInput {
                pose: &mb.pose,
                inverse_mass: &mb.inverse_mass,
                body_transform: &b.transform,
            };
            match &mut entry.kernel {
                Kernel::Atom(joint) => {
                    joint.a = indices[0];
                    joint.b = indices[1];
                    joint.build(a_input, b_input, &atom_step);
                }
                Kernel::Hinge(joint) => {
                    joint.a = indices[0];
                    joint.b = indices[1];
                    entry.hinge_rows = Some(joint.build(
                        a_input,
                        b_input,
                        step,
                        &query,
                        crate::float::multiply(step.seconds, entry.factor),
                    ));
                }
            }
            entry.prepared = true;
        }
        Ok(())
    }
    pub(super) fn solve_joints(
        &mut self,
        step: &SolverParameters,
        phase: crate::gameplay::constraint_solver::step::SolverPhase,
        velocities: &mut [SolverVelocities],
    ) {
        for entry in self
            .joint_owner
            .entries
            .iter_mut()
            .flatten()
            .filter(|entry| entry.prepared)
        {
            match &mut entry.kernel {
                Kernel::Atom(joint) => joint.solve(velocities, step),
                Kernel::Hinge(joint) => {
                    if let Some(rows) = &mut entry.hinge_rows {
                        solve_hinge(joint, rows, velocities, step, phase.velocity_fraction);
                    }
                }
            }
        }
    }
    pub(super) fn export_joints(
        &mut self,
        step: &SolverParameters,
        velocities: &[SolverVelocities],
    ) {
        for entry in self
            .joint_owner
            .entries
            .iter_mut()
            .flatten()
            .filter(|entry| entry.prepared)
        {
            match &mut entry.kernel {
                Kernel::Atom(joint) => joint.export(velocities, step),
                Kernel::Hinge(joint) => {
                    if let Some(rows) = &entry.hinge_rows {
                        joint.export(rows, velocities, step);
                    }
                }
            }
        }
    }
}
