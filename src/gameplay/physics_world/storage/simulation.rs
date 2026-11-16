//! The ordinary world's active-motion solve and final publication.
//! Solver arithmetic stays in the already ported 12812f0/12b4ce0/12b5130
//! kernels; collision manifolds and their callbacks have their own live owner.
use super::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    constraint_solver::{
        ball_socket::SolverVelocities,
        motion::{MotionHistory, MotionProperties, MotionPublication},
        step::{SolverParameters, SolverPhase},
    },
    deactivation::queue_attached_bodies,
};

/// Required collision owner. There are intentionally no empty default methods:
/// a free-flight integrator is not a replacement for an admitted rigid world.
pub(crate) trait SimulationContacts<G, E> {
    /// Collide actual published shapes and activate their real sleeping
    /// receivers before the solver's active-motion roster is captured.
    fn pre_collide(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut G,
        events: &mut E,
        step: &SolverParameters,
    ) -> Result<(), String>;
    /// Build the real contact rows against this stable motion/solver mapping.
    fn prepare(
        &mut self,
        world: &WorldStorage,
        geometry: &mut G,
        step: &SolverParameters,
        motions: &[u32],
        velocities: &[SolverVelocities],
    ) -> Result<(), String>;
    fn solve(
        &mut self,
        step: &SolverParameters,
        phase: SolverPhase,
        velocities: &mut [SolverVelocities],
    );
    /// Actual contact body IDs, retaining both generations and member identity.
    /// The simulation resolves their shared motions through canonical storage.
    fn body_links(&self) -> &[[u32; 2]];
    /// Export real contact results through the component/event owners only
    /// after the numerical solve and body publication have completed.
    fn finish(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut G,
        events: &mut E,
        step: &SolverParameters,
    ) -> Result<(), String>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Collide,
    Prepare,
    Solve,
    Integrate,
    Publish,
    Sleep,
    LinkSleep,
    Export,
}

struct Pending {
    step: SolverParameters,
    phase: Phase,
    cursor: usize,
}

/// Numeric workspaces belong to this WorldStorage and are released with it.
/// They retain no declarations, geometry, renderer handles or managed actors.
#[derive(Default)]
pub(super) struct SimulationScratch {
    pending: Option<Pending>,
    motions: Vec<u32>,
    velocities: Vec<SolverVelocities>,
    properties: Vec<MotionProperties>,
    histories: Vec<MotionHistory>,
    publications: Vec<MotionPublication>,
    bodies: Vec<u32>,
    candidates: Vec<bool>,
    parents: Vec<usize>,
    sleep_groups: Vec<bool>,
    motion_to_solver: Vec<u32>,
}

impl SimulationScratch {
    pub(super) fn is_idle(&self) -> bool {
        self.pending.is_none()
    }
    fn clear(&mut self) {
        self.clear_numeric();
        self.pending = None;
    }
    fn clear_numeric(&mut self) {
        for &motion in &self.motions {
            if let Some(index) = self.motion_to_solver.get_mut(motion as usize) {
                *index = u32::MAX;
            }
        }
        self.motions.clear();
        self.velocities.clear();
        self.properties.clear();
        self.histories.clear();
        self.publications.clear();
        self.bodies.clear();
        self.candidates.clear();
        self.parents.clear();
        self.sleep_groups.clear();
    }
}

fn vector(value: [f32; 3]) -> [f32; 4] {
    [value[0], value[1], value[2], 0.]
}
fn xyz(value: [f32; 4]) -> [f32; 3] {
    [value[0], value[1], value[2]]
}
fn root(parents: &mut [usize], mut index: usize) -> usize {
    while parents[index] != index {
        parents[index] = parents[parents[index]];
        index = parents[index];
    }
    index
}

impl WorldStorage {
    pub(crate) fn simulation_idle(&self) -> bool {
        !self.simulation_busy && self.simulation.is_idle()
    }

    /// The source host calls097bc0's actual timespace/profile/velocity pass
    /// before this entry. `step` is its largest advancing game-clock delta,
    /// prepared by SolverParameters::from_game_deltas, never render time.
    pub(crate) fn step_simulation<G, E, C>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        step: &SolverParameters,
        contacts: &mut C,
    ) -> Result<bool, String>
    where
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
        C: SimulationContacts<G, E>,
    {
        if self.simulation_busy {
            return Err("physics simulation reentered its live world".into());
        }
        if !self.publication_idle() {
            return Err("physics publication must finish before simulation".into());
        }
        if step.seconds <= 0. || step.substeps == 0 || step.iterations == 0 {
            return Ok(false);
        }
        let mut scratch = std::mem::take(&mut self.simulation);
        self.simulation_busy = true;
        if scratch.pending.is_none() {
            scratch.clear();
            scratch.pending = Some(Pending {
                step: *step,
                phase: Phase::Collide,
                cursor: 0,
            });
        }
        let result = self.continue_simulation(geometry, events, contacts, &mut scratch);
        self.simulation_busy = false;
        self.simulation = scratch;
        result
    }

    fn continue_simulation<G, E, C>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        contacts: &mut C,
        scratch: &mut SimulationScratch,
    ) -> Result<bool, String>
    where
        G: BodyCreationGeometry,
        E: PublicationEvents + BodyCreationCallbacks,
        C: SimulationContacts<G, E>,
    {
        let step = scratch
            .pending
            .as_ref()
            .ok_or("simulation cursor is absent")?
            .step;
        if scratch.pending.as_ref().unwrap().phase == Phase::Collide {
            self.step.duration = [step.seconds; 4];
            // The selected-shape swept bounds consume this physics step, not
            // the constructor's16ms backing value or the render frame delta.
            for index in 0..self.bodies.active_body_ids().len() {
                let id = self.bodies.active_body_ids()[index];
                let body = self
                    .bodies
                    .get_mut(id)
                    .ok_or("active bound body is stale")?;
                let motion = self
                    .motions
                    .get(body.motion_id)
                    .ok_or("active bound motion is absent")?;
                let quality = self
                    .qualities
                    .get(body.quality_id)
                    .ok_or("active body quality is absent")?;
                let previous = body.bounds;
                body.try_update_swept_bounds(
                    motion,
                    quality.requested_flags,
                    [self.info.collision_tolerance; 4],
                    &self.step,
                    &self.quantization.packed,
                    |shape, frame| geometry.bounds(shape, frame),
                )?;
                if body.bounds != previous {
                    self.broadphase.mark_bodies([id]);
                }
            }
            self.broadphase
                .update_marked_bodies(&mut self.bodies, &self.quantization.packed)?;
            contacts.pre_collide(self, geometry, events, &step)?;
            // A preparation invariant may have failed before any numerical
            // publication. Rebuilding that roster cannot duplicate a motion.
            scratch.clear_numeric();
            // cd77b0's per-cell lists already contain the permanent anchor at
            // index0. Capture each real active motion in its existing order.
            scratch.motions.extend(
                self.motions
                    .cell_lists()
                    .iter()
                    .flat_map(|list| list.iter().copied())
                    .filter(|&id| id != 0),
            );
            // Allocate this numeric lookup only as real active IDs demand it;
            // an entirely static world never needs a motion-capacity mirror.
            let required = scratch
                .motions
                .iter()
                .copied()
                .max()
                .map_or(0, |id| id as usize + 1);
            if required > self.motions.capacity() {
                return Err("active motion exceeds its world buffer".into());
            }
            if required > scratch.motion_to_solver.len() {
                scratch.motion_to_solver.resize(required, u32::MAX);
            }
            for (index, &id) in scratch.motions.iter().enumerate() {
                let slot = scratch
                    .motion_to_solver
                    .get_mut(id as usize)
                    .ok_or("solver motion exceeds its world storage")?;
                if *slot != u32::MAX {
                    return Err("active motion occurs in more than one solver cell".into());
                }
                *slot = u32::try_from(index).map_err(|_| "solver index overflow")?;
                let motion = self
                    .motions
                    .get(id)
                    .ok_or("active solver motion is absent")?;
                self.deactivation
                    .state(id)
                    .ok_or("active motion has no deactivation owner")?;
                queue_attached_bodies(&self.bodies, motion.first_body_id, &mut scratch.bodies)?;
                let mut properties = self
                    .motion_properties
                    .get(motion.properties_id)
                    .ok_or("active solver motion property is absent")?
                    .solver_properties();
                let inverse = motion
                    .inverse_mass
                    .map(|value| f32::from_bits(u32::from(value) << 16));
                // 12812f0 suppresses gravity for zero inverse mass, including
                // real keyframed motions; their authored velocity still solves.
                if inverse[3] == 0. {
                    properties.gravity_factor = 0.;
                }
                scratch.properties.push(properties);
                scratch.velocities.push(SolverVelocities::begin_motion(
                    xyz(motion.linear_velocity),
                    xyz(motion.angular_velocity),
                    inverse,
                    properties,
                    &step,
                ));
                // 12812f0 stores the rotation-per-step cap divided by full dt
                // back into a signed bfloat16 lane without a rounding bias.
                let angular =
                    f32::from_bits(u32::from(motion.limits[1]) << 16) * step.inverse_seconds;
                let packed = ((angular.to_bits() as i32) >> 16)
                    .clamp(i16::MIN as i32, i16::MAX as i32) as u16;
                scratch.histories.push(MotionHistory {
                    linear: xyz(motion.linear_velocity),
                    integrated_linear: motion
                        .linear_velocity_cage
                        .map(|value| f32::from_bits(u32::from(value) << 16)),
                    maximum_angular_velocity: f32::from_bits(u32::from(packed) << 16),
                    maximum_linear_change: f32::from_bits(u32::from(motion.limits[0]) << 16),
                });
            }
            scratch.pending.as_mut().unwrap().phase = Phase::Prepare;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Prepare {
            contacts.prepare(self, geometry, &step, &scratch.motions, &scratch.velocities)?;
            scratch.pending.as_mut().unwrap().phase = Phase::Solve;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Solve {
            // d0f5c0's source order: all iterations, optional post-step solve,
            // intermediate velocity integration, then final publication.
            for substep in 0..step.substeps {
                for iteration in 0..step.iterations {
                    contacts.solve(
                        &step,
                        step.phase(substep, iteration),
                        &mut scratch.velocities,
                    );
                }
                if self.info.solver_enable_post_step {
                    let mut phase = step.phase(substep, step.iterations - 1);
                    phase.flags |= 4;
                    contacts.solve(&step, phase, &mut scratch.velocities);
                }
                if substep + 1 < step.substeps {
                    for ((velocity, properties), history) in scratch
                        .velocities
                        .iter_mut()
                        .zip(&scratch.properties)
                        .zip(&scratch.histories)
                    {
                        let gravity = std::array::from_fn(|axis| {
                            properties.gravity_factor * step.gravity_substep[axis]
                        });
                        velocity.advance_substep(
                            *properties,
                            &step,
                            history.maximum_angular_velocity,
                            gravity,
                        );
                    }
                }
            }
            scratch.publications.extend(
                scratch
                    .velocities
                    .iter()
                    .zip(&scratch.properties)
                    .zip(&scratch.histories)
                    .map(|((velocity, properties), history)| {
                        velocity.finish_motion(*properties, &step, *history)
                    }),
            );
            scratch.pending.as_mut().unwrap().phase = Phase::Integrate;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Integrate {
            while scratch.pending.as_ref().unwrap().cursor < scratch.motions.len() {
                let index = scratch.pending.as_ref().unwrap().cursor;
                let id = scratch.motions[index];
                let result = scratch.publications[index];
                let mut motion = *self.motions.get(id).ok_or("solved motion disappeared")?;
                let profile = self
                    .motion_properties
                    .get(motion.properties_id)
                    .ok_or("solved deactivation profile disappeared")?
                    .deactivation();
                let deactivation = self
                    .deactivation
                    .state_mut(id)
                    .ok_or("solved deactivation state is absent")?;
                let previous = motion.previous_step_linear_velocity;
                motion.pose.integrate(
                    vector(result.integration_linear),
                    vector(result.integration_angular),
                    [step.seconds; 4],
                );
                motion.linear_velocity = vector(result.linear);
                motion.angular_velocity = vector(result.angular);
                motion.previous_step_linear_velocity = vector(result.integration_linear);
                motion.previous_step_angular_velocity = vector(result.integration_angular);
                // 12b5130 clears the velocity cage and restores its factor1.
                motion.linear_velocity_cage = [0; 3];
                motion.integration_factor = 0x3f80;
                let candidate = self.info.enable_deactivation
                    && deactivation.check(
                        motion.pose.center,
                        motion.pose.orientation,
                        motion.previous_step_linear_velocity,
                        previous,
                        &profile,
                    );
                scratch.candidates.push(candidate);
                // All fallible inputs above precede the motion write. No
                // callback can observe a half-integrated motion on retry.
                *self.motions.get_mut(id).expect("validated solved motion") = motion;
                scratch.pending.as_mut().unwrap().cursor += 1;
            }
            scratch.pending.as_mut().unwrap().cursor = 0;
            scratch.pending.as_mut().unwrap().phase = Phase::Publish;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Publish {
            self.step.duration = [step.seconds; 4];
            while scratch.pending.as_ref().unwrap().cursor < scratch.bodies.len() {
                let id = scratch.bodies[scratch.pending.as_ref().unwrap().cursor];
                // Host dispatch of12816e0 through its existing selected-shape,
                // quality, bounds and motion-cache owner; no render proxy pose.
                self.edit_bodies(geometry, events, |creation| {
                    creation.callbacks.refresh_transform_cache(
                        ActivationOwners {
                            bodies: creation.world.bodies,
                            motions: creation.world.motions,
                            deactivation: creation.world.deactivation,
                            broadphase: creation.world.broadphase,
                        },
                        id,
                    )
                })?;
                if self
                    .bodies
                    .get(id)
                    .ok_or("integrated body disappeared")?
                    .broadphase_id
                    != u32::MAX
                {
                    self.broadphase.mark_bodies([id]);
                }
                scratch.pending.as_mut().unwrap().cursor += 1;
            }
            scratch.parents.extend(0..scratch.motions.len());
            for &[a, b] in contacts.body_links() {
                let a = self
                    .bodies
                    .get(a)
                    .ok_or("first contact-link body is stale")?
                    .motion_id;
                let b = self
                    .bodies
                    .get(b)
                    .ok_or("second contact-link body is stale")?
                    .motion_id;
                let index = |id: u32| {
                    scratch
                        .motion_to_solver
                        .get(id as usize)
                        .copied()
                        .filter(|&index| index != u32::MAX)
                };
                let (Some(a), Some(b)) = (index(a), index(b)) else {
                    continue;
                };
                let (a, b) = (
                    root(&mut scratch.parents, a as usize),
                    root(&mut scratch.parents, b as usize),
                );
                scratch.parents[a.max(b)] = a.min(b);
            }
            scratch.sleep_groups.resize(scratch.motions.len(), true);
            for index in 0..scratch.motions.len() {
                let group = root(&mut scratch.parents, index);
                let state = self
                    .deactivation
                    .state(scratch.motions[index])
                    .ok_or("sleep candidate state disappeared")?;
                scratch.sleep_groups[group] &= scratch.candidates[index]
                    && state.mode() != crate::gameplay::deactivation::MODE_ALWAYS_ACTIVE;
            }
            scratch.pending.as_mut().unwrap().cursor = 0;
            scratch.pending.as_mut().unwrap().phase = Phase::Sleep;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Sleep {
            while scratch.pending.as_ref().unwrap().cursor < scratch.motions.len() {
                let index = scratch.pending.as_ref().unwrap().cursor;
                let group = root(&mut scratch.parents, index);
                if scratch.sleep_groups[group] {
                    let first = self
                        .motions
                        .get(scratch.motions[index])
                        .ok_or("sleep motion disappeared")?
                        .first_body_id;
                    self.edit_bodies(geometry, events, |creation| {
                        creation
                            .world
                            .set_activation(first, true, creation.callbacks)
                            .map(|_| ())
                    })?;
                }
                scratch.pending.as_mut().unwrap().cursor += 1;
            }
            scratch.pending.as_mut().unwrap().cursor = 0;
            scratch.pending.as_mut().unwrap().phase = Phase::LinkSleep;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::LinkSleep {
            while scratch.pending.as_ref().unwrap().cursor < contacts.body_links().len() {
                let [first, second] =
                    contacts.body_links()[scratch.pending.as_ref().unwrap().cursor];
                // cd8310 creates/merges the real sleeping islands after solver
                // deactivation, retaining the actual body pair and step stamp.
                self.deactivation.link_bodies(
                    &self.bodies,
                    &self.motions,
                    first,
                    second,
                    self.step_index,
                )?;
                scratch.pending.as_mut().unwrap().cursor += 1;
            }
            scratch.pending.as_mut().unwrap().phase = Phase::Export;
        }
        if scratch.pending.as_ref().unwrap().phase == Phase::Export {
            self.broadphase
                .update_marked_bodies(&mut self.bodies, &self.quantization.packed)?;
            contacts.finish(self, geometry, events, &step)?;
            self.step_index = self.step_index.wrapping_add(1);
            scratch.clear();
        }
        Ok(true)
    }
}
