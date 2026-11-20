//! Constrained ambient rigid bodies. Assets own recipes; this module owns poses,
//! motor history and solver state. No renderer, archive I/O or wall clock.
use super::{
    constraint_solver::{
        ball_socket::{rotate_direction,SolverVelocities},
        motion::{MotionHistory, MotionProperties},
        step::{ConstraintParameters, SolverParameters},
        Basis,
    },
    deactivation::{
        DeactivationProfile, MotionDeactivation, MODE_ALWAYS_ACTIVE, MODE_DEACTIVATE,
        MODE_KEEP_DEACTIVATED,
    },
    entity_lod::EntityLod,
    rigid_body::{BodyMotion, BodyPose},
};
use crate::{entities::machinery::ConstraintSettings, physworld};
mod atoms;
use crate::gameplay::constraint_solver::joint::{BodyInput,Joint,JointRows,solve_hinge};
fn joint_input<'a>(body:&'a MachineryBody,transform:&'a [[f32;4];4])->BodyInput<'a>{
    BodyInput{pose:&body.motion.pose,inverse_mass:&body.motion.inverse_mass,body_transform:transform}
}

fn vector(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.]
}
fn xyz(v: [f32; 4]) -> [f32; 3] {
    [v[0], v[1], v[2]]
}
fn inverse(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
fn transform(q: [f32; 4], basis: Basis) -> Basis {
    let rotation = physworld::quaternion_basis(q);
    basis.map(|axis| {
        std::array::from_fn(|i| {
            if i < 3 {
                (axis[1] * rotation[1][i] + axis[0] * rotation[0][i]) + axis[2] * rotation[2][i]
            } else {
                0.
            }
        })
    })
}

pub struct MachineryBody {
    pub placement: Option<usize>,
    pub motion: BodyMotion,
    pub properties: MotionProperties,
    initial: BodyPose,
    linear: [f32; 3],
    angular: [f32; 3],
    integrated_linear: [f32; 3],
    pub wind: Option<BodyWindState>,
    /// The body's collision, which the Havok particles collide with.
    pub collision: Option<std::sync::Arc<crate::entities::machinery::recipes::BodyCollision>>,
    /// Owner entity placement: its LOD level, static owners included. Only
    /// the first body of an entity group carries it.
    pub entity_placement: Option<usize>,
    /// The first body of this body's owner entity (a physics system owns
    /// several); freezing and LOD levels act on the whole group.
    pub group: usize,
    /// Body-space mass frame (orientation, center): motion = body * frame.
    mass_frame: ([f32; 4], [f32; 4]),
    pub lod: EntityLod,
    deactivation: MotionDeactivation,
    profile: DeactivationProfile,
    /// hknpBody IS_ACTIVE (+0x44 bit 3).
    active: bool,
    /// arkCpntRigidBody +0xd2: 0 thawed, 1 frozen while active, 2 frozen asleep.
    frozen: u8,
    candidate: bool,
}

/// A wind-manager rigid body: its `m_windSpeedFactor`, aero faces and the
/// shape mass frame that relates the motion to the hknpBody transform.
pub struct BodyWindState {
    pub speed_factor: f32,
    pub faces: Vec<super::aerodynamics::AeroFace>,
    hulls: Vec<crate::entities::machinery::recipes::BodyHull>,
    convex_radius: f32,
    frame_orientation: [f32; 4],
    frame_center: [f32; 4],
    /// Overlapped trigger winds, highest 1405ff500 priority first. Enter
    /// events insert before the first entry of lower-or-equal priority
    /// (80bc60 -> 7b7b10); exits remove the entry.
    pub triggers: Vec<usize>,
}
impl MachineryBody {
    pub fn new(placement: Option<usize>, motion: BodyMotion, angular_damping: f32) -> Self {
        Self {
            placement,
            initial: motion.pose,
            motion,
            linear: [0.; 3],
            angular: [0.; 3],
            integrated_linear: [0.; 3],
            wind: None,
            collision: None,
            entity_placement: placement,
            group: 0,
            mass_frame: ([0., 0., 0., 1.], [0.; 4]),
            lod: EntityLod::spawned(0),
            deactivation: MotionDeactivation::new(0, MODE_DEACTIVATE),
            profile: DeactivationProfile::default(),
            active: true,
            frozen: 0,
            candidate: false,
            // Motion profile 1, c9f050; observed default machinery profile.
            properties: MotionProperties {
                gravity_factor: 1.,
                maximum_linear_velocity: 200.,
                maximum_angular_velocity: 100.,
                linear_damping: 0.,
                angular_damping,
                linear_threshold: 0.,
                linear_reduction: 0.,
            },
        }
    }
    pub fn fixed() -> Self {
        Self::new(
            None,
            BodyMotion {
                pose: BodyPose {
                    center: [0.; 4],
                    orientation: [0., 0., 0., 1.],
                },
                inverse_mass: [0; 4],
            },
            0.,
        )
    }
    /// A dynamic motion; static owners and the world body share motion 0.
    pub fn dynamic(&self) -> bool {
        self.motion.inverse_mass_values()[3] != 0.
    }
    pub fn active(&self) -> bool {
        self.active
    }
    /// The hknpBody transform (columns and translation): the motion frame
    /// composed with the inverse body-space mass frame.
    pub fn body_transform(&self) -> [[f32; 4]; 4] {
        let [x, y, z, w] = self.motion.pose.orientation;
        let ([fx, fy, fz, fw], c) = self.mass_frame;
        let [qx, qy, qz] = [-fx, -fy, -fz];
        let body = [
            ((qz * y - z * qy) + w * qx) + x * fw,
            ((qx * z - x * qz) + w * qy) + y * fw,
            ((qy * x - y * qx) + w * qz) + z * fw,
            fw * w - ((y * qy + x * qx) + z * qz),
        ];
        let basis = physworld::quaternion_basis(body);
        let center = self.motion.pose.center;
        let origin: [f32; 4] = std::array::from_fn(|i| {
            if i == 3 {
                1.
            } else {
                center[i] - ((c[1] * basis[1][i] + c[0] * basis[0][i]) + c[2] * basis[2][i])
            }
        });
        [basis[0], basis[1], basis[2], origin].map(|mut v| {
            if v[3] != 1. {
                v[3] = 0.;
            }
            v
        })
    }
    /// hknpBody transform columns: the motion frame composed with the
    /// inverse shape mass frame, carrying the body-space mass center in w.
    fn body_columns(&self, wind: &BodyWindState) -> super::aerodynamics::BodyColumns {
        let [x, y, z, w] = self.motion.pose.orientation;
        let [fx, fy, fz, fw] = wind.frame_orientation;
        let [qx, qy, qz] = [-fx, -fy, -fz];
        let body = [
            ((qz * y - z * qy) + w * qx) + x * fw,
            ((qx * z - x * qz) + w * qy) + y * fw,
            ((qy * x - y * qx) + w * qz) + z * fw,
            fw * w - ((y * qy + x * qx) + z * qz),
        ];
        let basis = physworld::quaternion_basis(body);
        std::array::from_fn(|c| {
            [basis[c][0], basis[c][1], basis[c][2], wind.frame_center[c]]
        })
    }

    /// Trigger enter/exit after a physics step. `shapes` are the wind
    /// manager records (priority, started, shape) in trigger order.
    pub fn update_wind_triggers(
        &mut self,
        shapes: &[(i32, bool, super::trigger_volume::TriggerVolume)],
    ) {
        let Some(state) = &self.wind else {
            return;
        };
        let columns = self.body_columns(state);
        let rotate = |v: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|i| {
                (v[1] * columns[1][i] + v[0] * columns[0][i]) + v[2] * columns[2][i]
            })
        };
        let offset = rotate([columns[0][3], columns[1][3], columns[2][3]]);
        let origin: [f32; 3] = std::array::from_fn(|i| self.motion.pose.center[i] - offset[i]);
        let hulls: Vec<Vec<[f32; 3]>> = state
            .hulls
            .iter()
            .map(|hull| {
                hull.vertices
                    .iter()
                    .map(|v| {
                        let r = rotate(*v);
                        std::array::from_fn(|i| r[i] + origin[i])
                    })
                    .collect()
            })
            .collect();
        let radius = state.convex_radius;
        let inside: Vec<usize> = shapes
            .iter()
            .enumerate()
            .filter(|(_, (_, _, shape))| shape.touches(&hulls, radius))
            .map(|(index, _)| index)
            .collect();
        let state = self.wind.as_mut().unwrap();
        state.triggers.retain(|index| inside.contains(index));
        // Same-step enter events arrive in reverse trigger order: the live
        // Dust capture keeps vistatun, fosterbldg_shutter, vista (file order)
        // front to back with equal priorities and insert-before-equal.
        for index in inside.into_iter().rev() {
            if state.triggers.contains(&index) {
                continue;
            }
            let priority = shapes[index].0;
            let at = state
                .triggers
                .iter()
                .position(|existing| shapes[*existing].0 <= priority)
                .unwrap_or(state.triggers.len());
            state.triggers.insert(at, index);
        }
    }

    /// The body motion center (77edf0 sample point).
    /// The hknpMotion record PhysicsCollide reads: centre of mass,
    /// orientation, linear velocity, angular velocity in motion space.
    pub fn motion_record(&self) -> crate::gameplay::havok_fx::dynamic_collide::MotionRecord {
        crate::gameplay::havok_fx::dynamic_collide::MotionRecord {
            centre: self.motion.pose.center,
            orientation: self.motion.pose.orientation,
            linear: [self.linear[0], self.linear[1], self.linear[2], 0.],
            angular: [self.angular[0], self.angular[1], self.angular[2], 0.],
        }
    }

    pub fn mass_center(&self) -> [f32; 3] {
        xyz(self.motion.pose.center)
    }

    /// 14077f050: the wind drag velocity change for `seconds` of the body's
    /// actor clock, or None below the inactive-body thresholds.
    fn wind_change(&self, wind: [f32; 3], seconds: f32) -> Option<([f32; 4], [f32; 4])> {
        let state = self.wind.as_ref()?;
        let columns = self.body_columns(state);
        let motion = super::aerodynamics::MotionMass {
            center: self.motion.pose.center,
            orientation: self.motion.pose.orientation,
            inverse_mass: self.motion.inverse_mass,
        };
        super::aerodynamics::wind_velocity_change(
            &columns,
            &state.faces,
            &motion,
            vector(self.linear),
            wind,
            state.speed_factor,
            seconds,
            self.active,
        )
    }
    fn add_velocity(&mut self, linear: [f32; 4], angular: [f32; 4]) {
        let orientation = self.motion.pose.orientation;
        let world = super::aerodynamics::rotate(orientation, self.angular);
        self.linear = std::array::from_fn(|i| self.linear[i] + linear[i]);
        self.angular = super::aerodynamics::rotate_inverse(
            orientation,
            std::array::from_fn(|i| world[i] + angular[i]),
        );
    }

    /// Rigid delta from initial mass pose, independent of the render model's
    /// scale, shear and visual-origin offset.
    pub fn delta(&self) -> [[f32; 4]; 3] {
        let current = physworld::quaternion_basis(self.motion.pose.orientation);
        let initial = physworld::quaternion_basis(self.initial.orientation);
        let mut rows = [[0.; 4]; 3];
        for r in 0..3 {
            for c in 0..3 {
                rows[r][c] = (current[0][r] * initial[0][c] + current[1][r] * initial[1][c])
                    + current[2][r] * initial[2][c];
            }
            rows[r][3] = self.motion.pose.center[r]
                - ((rows[r][0] * self.initial.center[0] + rows[r][1] * self.initial.center[1])
                    + rows[r][2] * self.initial.center[2]);
        }
        rows
    }
}


#[derive(Clone, Copy)]
enum SolveSlot {
    Atom(usize),
    Hinge(usize),
}

/// A simulated physics system and its body-to-joint map for skinning.
pub struct SystemState {
    pub placement: usize,
    pub skeleton: String,
    /// Machinery body index per physics-system body, in cinfo order.
    pub bodies: Vec<usize>,
    pub ragdoll: std::sync::Arc<[physworld::physics_system::RagdollEntry]>,
    /// Entity spawn transform (position, orientation).
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

#[derive(Default)]
pub struct MachineryWorld {
    windmills: std::collections::HashMap<String, WindmillMotor>,
    pub bodies: Vec<MachineryBody>,
    pub systems: Vec<SystemState>,
    joints: Vec<Joint>,
    atom_joints: Vec<atoms::AtomJoint>,
    /// Constraints in creation order: 1412a3480 lists the constraint array
    /// in index order and 1412a5ae0's stable radix sort only groups it by
    /// island, so coupled constraints solve in the order they were added.
    solve_order: Vec<SolveSlot>,
    rows: Vec<Option<JointRows>>,
    velocities: Vec<SolverVelocities>,
    /// hknpWorld +0x818 clear: no motion checks, sleep or activation modes.
    pub deactivation_disabled: bool,
    /// Bodies attached to a simulated body (attachBodies, 140c8fc00): each
    /// stays its own hknp body with its own shape, moved by the shared
    /// motion at its spawn offset from the parent body.
    attached: Vec<AttachedCollision>,
}

struct AttachedCollision {
    parent: usize,
    /// The attached body transform in the parent body's space, at spawn.
    offset: [[f32; 4]; 4],
    collision: std::sync::Arc<crate::entities::machinery::recipes::BodyCollision>,
}

/// `a * b` for column transforms (three basis columns, then the origin).
fn compose(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|c| std::array::from_fn(|i| {
        if i == 3 {
            b[c][3]
        } else {
            let rotated = (a[0][i] * b[c][0] + a[1][i] * b[c][1]) + a[2][i] * b[c][2];
            if c == 3 { rotated + a[3][i] } else { rotated }
        }
    }))
}

/// The inverse of a rigid column transform.
fn rigid_inverse(a: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let dot = |c: usize, v: [f32; 4]| (a[c][0] * v[0] + a[c][1] * v[1]) + a[c][2] * v[2];
    std::array::from_fn(|c| std::array::from_fn(|i| match (c, i) {
        (3, 3) => 1.,
        (_, 3) => 0.,
        (3, _) => -dot(i, a[3]),
        _ => a[i][c],
    }))
}
struct WindmillMotor {
    constraint: String,
    default_speed: f32,
    override_speed: f32,
    enabled: bool,
}
impl MachineryWorld {
    /// 7149b0/716560 withdraw an endpoint's component constraints before its
    /// body retires. Keep other body indices stable for physics-system skins.
    pub(crate) fn retire_body(&mut self, index: usize) {
        for joint_index in (0..self.joints.len()).rev() {
            let joint = &self.joints[joint_index];
            if joint.a != index && joint.b != index { continue; }
            self.windmills.retain(|_, motor| motor.constraint != joint.settings.constraint_id);
            self.joints.remove(joint_index);
            self.solve_order.retain_mut(|slot| match slot {
                SolveSlot::Hinge(i) if *i == joint_index => false,
                SolveSlot::Hinge(i) if *i > joint_index => { *i -= 1; true }
                _ => true,
            });
        }
        for joint_index in (0..self.atom_joints.len()).rev() {
            let joint = &self.atom_joints[joint_index];
            if !joint.component || joint.a != index && joint.b != index { continue; }
            self.atom_joints.remove(joint_index);
            self.solve_order.retain_mut(|slot| match slot {
                SolveSlot::Atom(i) if *i == joint_index => false,
                SolveSlot::Atom(i) if *i > joint_index => { *i -= 1; true }
                _ => true,
            });
        }
        self.attached.retain(|body| body.parent != index);
        self.bodies[index] = MachineryBody::fixed();
    }


    /// 14080ac90: a bound dynamic rigid body joins its parent's motion
    /// (14080af20 -> 140c8fc00 attachBodies), then c90a90 rebuilds that
    /// motion from every attached body's mass (c2bd80 combine). The parent
    /// keeps its body transform, motion properties and activation; the
    /// combined frame becomes its mass frame (and wind w lanes).
    fn attach_bound_bodies<'a>(
        &mut self,
        bodies: &'a [crate::entities::machinery::recipes::BodyRecipe],
        indices: &mut std::collections::HashMap<&'a str, usize>,
    ) -> Result<(), String> {
        use crate::physworld::mass::{combine_mass, MassElement, MassProperties};
        let root = |mut i: usize| {
            while let Some(parent) = bodies[i].attached_to {
                i = parent;
            }
            i
        };
        let mut rings = std::collections::BTreeMap::<usize, Vec<usize>>::new();
        for (i, body) in bodies.iter().enumerate().filter(|(_, b)| b.attached_to.is_some()) {
            let r = root(i);
            let parent = *indices
                .get(bodies[r].entity_id.as_str())
                .ok_or_else(|| format!("{}: bound to an unsimulated body", body.entity_id))?;
            indices.insert(body.entity_id.as_str(), parent);
            rings.entry(r).or_insert_with(|| vec![r]).push(i);
        }
        for (r, ring) in rings {
            let index = indices[bodies[r].entity_id.as_str()];
            let element = |b: &crate::entities::machinery::recipes::BodyRecipe| -> Result<MassElement, String> {
                let mass = b.mass.ok_or_else(|| format!("{}: attached body has no mass", b.entity_id))?;
                let basis = physworld::quaternion_basis(b.orientation);
                Ok(MassElement {
                    properties: mass.to_tensor(),
                    transform: [basis[0], basis[1], basis[2], [b.position[0], b.position[1], b.position[2], 1.]],
                })
            };
            let elements = ring.iter().map(|&i| element(&bodies[i])).collect::<Result<Vec<_>, _>>()?;
            let mut combined = MassProperties { volume: 0., mass: 0., center: [0.; 4], inertia: [[0.; 4]; 3] };
            let _ = combine_mass(&elements, &mut combined);
            let motion = BodyMotion::from_mass_frame(combined.to_principal(), [0., 0., 0., 1.], [0.; 4]);
            let body = &mut self.bodies[index];
            let transform = body.body_transform();
            // Mass frame = body^-1 * motion, so body_transform() is unchanged.
            let q = physworld::rotation_quaternion([transform[0], transform[1], transform[2]]);
            let [bx, by, bz, bw] = [-q[0], -q[1], -q[2], q[3]];
            let [x, y, z, w] = motion.pose.orientation;
            let frame = [
                ((by * z - bz * y) + bw * x) + bx * w,
                ((bz * x - bx * z) + bw * y) + by * w,
                ((bx * y - by * x) + bw * z) + bz * w,
                bw * w - ((by * y + bx * x) + bz * z),
            ];
            let d: [f32; 3] = std::array::from_fn(|i| motion.pose.center[i] - transform[3][i]);
            let center: [f32; 4] = std::array::from_fn(|c| {
                if c == 3 {
                    0.
                } else {
                    (transform[c][1] * d[1] + transform[c][0] * d[0]) + transform[c][2] * d[2]
                }
            });
            for &i in &ring[1..] {
                let Some(collision) = bodies[i].collision.clone() else { continue };
                let basis = physworld::quaternion_basis(bodies[i].orientation);
                let p = bodies[i].position;
                let spawn = [basis[0], basis[1], basis[2], [p[0], p[1], p[2], 1.]]
                    .map(|mut v| { if v[3] != 1. { v[3] = 0.; } v });
                self.attached.push(AttachedCollision {
                    parent: index,
                    offset: compose(&rigid_inverse(&transform), &spawn),
                    collision,
                });
            }
            // delta() is relative to the spawn motion pose: re-base it so the
            // body still renders at its authored placement.
            body.initial = motion.pose;
            body.motion = motion;
            body.mass_frame = (frame, center);
            if let Some(wind) = &mut body.wind {
                wind.frame_orientation = frame;
                wind.frame_center = center;
            }
        }
        Ok(())
    }

    pub fn from_recipes(
        recipes: &crate::entities::machinery::recipes::MachineryRecipes,
    ) -> Result<Self, String> {
        Self::from_recipes_with_constraint_admission(recipes, |_|{})
    }
    /// Host admission metadata is borrowed only during the existing recipe
    /// construction; it retains no second owner-name map on simulated joints.
    pub(crate) fn from_recipes_with_constraint_admission(
        recipes:&crate::entities::machinery::recipes::MachineryRecipes,
        admitted:impl FnMut(&str),
    )->Result<Self,String>{
        let mut world = Self::default();
        world.bodies.push(MachineryBody::fixed());
        world.append_recipes(recipes, admitted)?;
        Ok(world)
    }

    pub(crate) fn append_recipes(&mut self, recipes: &crate::entities::machinery::recipes::MachineryRecipes,
        mut admitted: impl FnMut(&str)) -> Result<(), String> {
        let world = self;
        let mut indices = std::collections::HashMap::<&str, usize>::new();
        for assembly in &recipes.assemblies {
            for body in &assembly.bodies {
                if indices.contains_key(body.entity_id.as_str()) || body.attached_to.is_some() {
                    continue;
                }
                let translation = vector(body.position);
                let motion = match body.mass {
                    Some(mass) => BodyMotion::from_mass_frame(mass, body.orientation, translation),
                    None => BodyMotion {
                        pose: BodyPose {
                            center: translation,
                            orientation: body.orientation,
                        },
                        inverse_mass: [0; 4],
                    },
                };
                indices.insert(body.entity_id.as_str(), world.bodies.len());
                let mut created = MachineryBody::new(
                    body.mass.map(|_| body.placement),
                    motion,
                    body.angular_damping,
                );
                created.entity_placement = Some(body.placement);
                created.group = world.bodies.len();
                if let Some(mass) = body.mass {
                    created.mass_frame = (mass.orientation, mass.center);
                }
                // 140c908f0 -> 140ce90e0. Motion ids follow creation order
                // here; they only phase the check frames (id & 3).
                created.deactivation =
                    MotionDeactivation::new(world.bodies.len() as u32, MODE_DEACTIVATE);
                // 809d80 -> 140c8e930: m_startAsleep adds the body inactive.
                created.active = !body.start_asleep || world.deactivation_disabled;
                created.collision = body.collision.clone();
                if let (Some(wind), Some(mass)) = (&body.wind, body.mass) {
                    created.wind = Some(BodyWindState {
                        speed_factor: wind.speed_factor,
                        faces: wind.faces.clone(),
                        hulls: wind.hulls.clone(),
                        convex_radius: wind.convex_radius,
                        frame_orientation: mass.orientation,
                        frame_center: mass.center,
                        triggers: Vec::new(),
                    });
                }
                world.bodies.push(created);
            }
            world.attach_bound_bodies(&assembly.bodies, &mut indices)?;
        }
        // 14076abc0 -> 14008e560: physics-system bodies at the entity transform
        // applied to each body cinfo pose, added awake; its constraints are
        // the ragdoll atom stacks between body frames.
        let mut system_keys = std::collections::HashMap::<String, usize>::new();
        for system in &recipes.systems {
            let group = world.bodies.len();
            let mut members = Vec::new();
            let q = system.orientation;
            for (k, body) in system.system.bodies.iter().enumerate() {
                let p = crate::gameplay::constraint_solver::ball_socket::rotate_direction(
                    q,
                    [body.position[0], body.position[1], body.position[2]],
                );
                let translation = [
                    p[0] + system.position[0],
                    p[1] + system.position[1],
                    p[2] + system.position[2],
                    0.,
                ];
                let [x, y, z, w] = q;
                let [bx, by, bz, bw] = body.orientation;
                let orientation = [
                    ((y * bz - z * by) + w * bx) + x * bw,
                    ((z * bx - x * bz) + w * by) + y * bw,
                    ((x * by - y * bx) + w * bz) + z * bw,
                    w * bw - ((y * by + x * bx) + z * bz),
                ];
                let motion = BodyMotion::from_mass_frame(body.mass, orientation, translation);
                // 14076b6c0 -> 140c9b620 -> 140c8e930 adds the bodies inactive only
                // for a paused component (state +0x1c bits 4-5 >= 2, set by the
                // pause path 1407e75b0); at spawn the entity LOD is 0, so they start
                // active.
                let mut created = MachineryBody::new(None, motion, 0.);
                created.entity_placement = (k == 0).then_some(system.placement);
                created.group = group;
                created.mass_frame = (body.mass.orientation, body.mass.center);
                created.deactivation =
                    MotionDeactivation::new(world.bodies.len() as u32, MODE_DEACTIVATE);
                system_keys.insert(format!("{}#{}", system.entity_id, body.name), world.bodies.len());
                members.push(world.bodies.len());
                world.bodies.push(created);
            }
            for constraint in &system.system.constraints {
                world.solve_order.push(SolveSlot::Atom(world.atom_joints.len()));
                world.atom_joints.push(atoms::AtomJoint::new(
                    members[constraint.body_a],
                    members[constraint.body_b],
                    constraint.atoms,
                    false,
                ));
            }
            world.systems.push(SystemState {
                placement: system.placement,
                skeleton: system.skeleton.clone(),
                bodies: members,
                ragdoll: system.ragdoll.clone(),
                position: system.position,
                orientation: system.orientation,
            });
        }
        for assembly in &recipes.assemblies {
            for joint in &assembly.joints {
                let endpoint = |id: &Option<String>, bone: &Option<String>| -> Result<usize, String> {
                    match (id, bone) {
                        (None, _) => Ok(0),
                        (Some(id), Some(bone)) => system_keys
                            .get(&format!("{id}#{bone}"))
                            .copied()
                            .ok_or_else(|| format!("unresolved physics-system endpoint {id}#{bone}")),
                        (Some(id), None) => indices
                            .get(id.as_str())
                            .copied()
                            .ok_or_else(|| format!("unresolved machinery endpoint {id}")),
                    }
                };
                let a = endpoint(&joint.entity_a, &joint.joint_a)?;
                let b = endpoint(&joint.entity_b, &joint.joint_b)?;
                // 140712da0 returns without creating a constraint when both
                // resolved body IDs are equal, including two missing owners.
                if a != b && joint.angular_constraint == "ANGULAR_CONSTRAINT_CONE" {
                    let stack = atoms::cone_stack(joint, &world.bodies[a], &world.bodies[b]);
                    world.solve_order.push(SolveSlot::Atom(world.atom_joints.len()));
                    world.atom_joints.push(atoms::AtomJoint::new(a, b, stack, true));
                    admitted(&joint.constraint_id);
                } else if a != b {
                    world.add_joint(joint.clone(), a, b)?;
                    admitted(&joint.constraint_id);
                    // 140714cd0 -> 140c91140 -> 140c92dd0: an enabled motor
                    // adds the constraint with activation; others keep sleep.
                    if joint.motor.is_some() && joint.started {
                        world.wake_joint(world.joints.len() - 1);
                    }
                }
            }
        }
        for windmill in &recipes.windmills {
            world.set_motor_parameters(
                &windmill.constraint,
                None,
                None,
                Some(windmill.speed_degrees * f32::from_bits(0x3c8efa35)),
                None,
            );
            world.windmills.insert(
                windmill.entity.clone(),
                WindmillMotor {
                    constraint: windmill.constraint.clone(),
                    default_speed: windmill.speed_degrees,
                    override_speed: -1.,
                    enabled: true,
                },
            );
        }
        Ok(())
    }
    pub fn add_joint(
        &mut self,
        settings: ConstraintSettings,
        a: usize,
        b: usize,
    ) -> Result<(), String> {
        if a == b || a >= self.bodies.len() || b >= self.bodies.len() {
            return Err(format!("invalid endpoints for {}", settings.constraint_id));
        }
        if settings.linear_constraint != "LINEAR_CONSTRAINT_FIXED"
            || !matches!(
                settings.angular_constraint.as_str(),
                "ANGULAR_CONSTRAINT_HINGE" | "ANGULAR_CONSTRAINT_LIMITED_HINGE"
            )
        {
            return Err(format!(
                "unsupported ambient joint {}: {}/{}",
                settings.constraint_id, settings.linear_constraint, settings.angular_constraint
            ));
        }
        self.solve_order.push(SolveSlot::Hinge(self.joints.len()));
        self.joints.push(Joint::new(settings,a,b,
            joint_input(&self.bodies[a], &self.bodies[a].body_transform()),
            joint_input(&self.bodies[b], &self.bodies[b].body_transform())));
        Ok(())
    }
    /// Largest separation between the two sides of an authored fixed pivot.
    pub fn maximum_anchor_error(&self) -> f32 {
        self.joints
            .iter()
            .map(|joint| {
                let anchor = |index: usize, lever| {
                    let pose = self.bodies[index].motion.pose;
                    let offset = rotate_direction(pose.orientation, lever);
                    std::array::from_fn::<_, 3, _>(|i| pose.center[i] + offset[i])
                };
                let a = anchor(joint.a, joint.lever_a);
                let b = anchor(joint.b, joint.lever_b);
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
            })
            .fold(0., f32::max)
    }
    /// The dynamic bodies with collision as the Havok particles' moving
    /// world, indexed like `bodies`. approximate_: each AABB is the shape's
    /// calcAabb at the body transform; hknp's packed broadphase AABB
    /// (1280e20, decoded by c85490) also grows by the body's motion and
    /// contact distance, which the machinery solver does not track.
    pub fn moving_world(&self) -> crate::gameplay::havok_fx::dynamic_collide::MovingWorld {
        use crate::gameplay::havok_fx::{dynamic_collide::MovingBody, world_collide::SurfaceMaterial};
        let mut materials = Vec::new();
        let bodies = self
            .bodies
            .iter()
            .enumerate()
            .map(|(index, body)| {
                let collision = body.collision.as_ref().filter(|_| body.dynamic())?;
                let transform = body.body_transform();
                let bounds = collision.shapes.bounds(collision.shapes.root(), transform).ok()?;
                let material = collision.material.as_ref().map_or(0xffff, |material| {
                    materials.push(SurfaceMaterial::from_physics(material));
                    (materials.len() - 1) as u16
                });
                Some(MovingBody {
                    id: index as u32,
                    active: body.active(),
                    transform,
                    motion: body.motion_record(),
                    bounds,
                    shapes: collision.shapes.clone(),
                    material,
                    filter: collision.filter,
                })
            })
            .collect::<Vec<_>>();
        // Attached bodies follow, with the parent's motion and activation.
        let mut bodies = bodies;
        for attached in &self.attached {
            let parent = &self.bodies[attached.parent];
            let collision = &attached.collision;
            let id = bodies.len() as u32;
            let transform = compose(&parent.body_transform(), &attached.offset);
            let Ok(bounds) = collision.shapes.bounds(collision.shapes.root(), transform) else {
                bodies.push(None);
                continue;
            };
            let material = collision.material.as_ref().map_or(0xffff, |material| {
                materials.push(SurfaceMaterial::from_physics(material));
                (materials.len() - 1) as u16
            });
            bodies.push(Some(MovingBody {
                id,
                active: parent.active(),
                transform,
                motion: parent.motion_record(),
                bounds,
                shapes: collision.shapes.clone(),
                material,
                filter: collision.filter,
            }));
        }
        crate::gameplay::havok_fx::dynamic_collide::MovingWorld { bodies, materials }
    }

    pub fn joint_count(&self) -> usize {
        self.joints.len()
    }
    /// 743340: retain the requested override; negative values restore the
    /// declaration's speed. Generator enable/stop gameplay has a separate owner.
    pub fn set_windmill_speed(&mut self, entity: &str, degrees: f32) -> bool {
        let Some(motor) = self.windmills.get_mut(entity) else {
            return false;
        };
        motor.override_speed = degrees;
        let (constraint, velocity) = (
            motor.constraint.clone(),
            if degrees < 0. {
                motor.default_speed
            } else {
                degrees
            },
        );
        if motor.enabled {
            self.set_motor_parameters(
                &constraint,
                None,
                None,
                Some(velocity * f32::from_bits(0x3c8efa35)),
                None,
            );
        }
        true
    }
    pub fn motor_parameters(&self, id: &str) -> Option<(bool, f32, f32, f32)> {
        self.joints.iter().find(|j| j.settings.constraint_id == id)?.motor_parameters()
    }
    /// Component setters715a60/715b60/715bb0/715b00. The same live motor owns
    /// its history across changes; changing a target must not recreate a body.
    pub fn set_motor_parameters(
        &mut self,
        id: &str,
        enabled: Option<bool>,
        position: Option<f32>,
        velocity: Option<f32>,
        acceleration: Option<f32>,
    ) -> bool {
        let Some(index) = self.joints.iter().position(|j| j.settings.constraint_id == id && j.motor.is_some()) else {
            return false;
        };
        if self.joints[index].update_motor(enabled, position, velocity, acceleration) {
            self.wake_joint(index);
        }
        true
    }

    /// 140c91230(world, body, 0): activate a dynamic body that is not kept
    /// deactivated, restarting its stillness counter. The deactivation
    /// manager's group pass (140cdd770) then activates its island mates.
    fn activate(&mut self, index: usize) -> bool {
        if self.deactivation_disabled {
            return false;
        }
        let body = &self.bodies[index];
        if !body.dynamic() || body.active || body.deactivation.mode() == MODE_KEEP_DEACTIVATED {
            return false;
        }
        let island = body.deactivation.island();
        for (i, other) in self.bodies.iter_mut().enumerate() {
            let mate = island != -1
                && other.deactivation.island() == island
                && other.deactivation.mode() != MODE_KEEP_DEACTIVATED;
            if i == index || mate {
                other.active = true;
                other.deactivation.restart();
                other.deactivation.set_island(-1);
            }
        }
        true
    }
    /// 140c91230(world, body, 1) for a single body (island -1).
    fn deactivate(&mut self, index: usize) {
        let body = &mut self.bodies[index];
        if body.deactivation.mode() == MODE_ALWAYS_ACTIVE {
            return;
        }
        body.active = false;
        body.deactivation.set_island(-1);
    }
    /// 140712a90: activate both constraint bodies that are not static.
    fn wake_joint(&mut self, joint: usize) {
        let (a, b) = (self.joints[joint].a, self.joints[joint].b);
        self.activate(a);
        self.activate(b);
    }
    /// 140c87ca0: mode 2 first deactivates an active motion.
    fn set_activation_mode(&mut self, index: usize, mode: u8) {
        if self.deactivation_disabled {
            return;
        }
        if mode == MODE_KEEP_DEACTIVATED && self.bodies[index].active {
            self.bodies[index].deactivation.set_mode(MODE_DEACTIVATE);
            self.deactivate(index);
        }
        self.bodies[index].deactivation.set_mode(mode);
    }
    fn members(&self, index: usize) -> Vec<usize> {
        let group = self.bodies[index].group;
        (0..self.bodies.len()).filter(|&i| i == index || (group != 0 && self.bodies[i].group == group)).collect()
    }
    /// arkCpntRigidBody 808220 / arkCpntPhysicsSystem 140769d90 (vtable
    /// +0x138): every body of the owner.
    fn freeze(&mut self, index: usize) {
        for index in self.members(index) {
            let body = &self.bodies[index];
            if body.frozen != 0 || !body.dynamic() {
                continue;
            }
            self.bodies[index].frozen = if body.active { 1 } else { 2 };
            self.set_activation_mode(index, MODE_KEEP_DEACTIVATED);
        }
    }
    /// arkCpntRigidBody 808010 / arkCpntPhysicsSystem 140769a40 (vtable
    /// +0x140) -> 807890 for a body that was active when frozen.
    fn thaw(&mut self, index: usize) {
        for index in self.members(index) {
            let frozen = self.bodies[index].frozen;
            if frozen == 0 {
                continue;
            }
            self.set_activation_mode(index, MODE_DEACTIVATE);
            if frozen == 1 {
                self.activate(index);
            }
            self.bodies[index].frozen = 0;
        }
    }
    /// Component constraint endpoints: hinge joints, then component atom joints.
    fn component_endpoints(&self) -> Vec<(usize, usize)> {
        self.joints
            .iter()
            .map(|j| (j.a, j.b))
            .chain(self.atom_joints.iter().filter(|j| j.component).map(|j| (j.a, j.b)))
            .collect()
    }
    /// 14087bb70 -> arkCpntRigidBody 140727390: an owner level change reaches
    /// each attached constraint when it crosses the frozen state, which needs
    /// every attached owner (the world counts as frozen) at level 6+.
    pub fn entity_level_changed(&mut self, body: usize, old: i32, new: i32) {
        let group = self.bodies[body].group;
        let owned = |i: usize, bodies: &[MachineryBody]| i == body || (group != 0 && bodies[i].group == group);
        for (joint, (a, b)) in self.component_endpoints().into_iter().enumerate() {
            let (mine, other) = match (owned(a, &self.bodies), owned(b, &self.bodies)) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => continue,
            };
            let _ = mine;
            let leader = self.bodies[other].group;
            let other_frozen = other == 0 || self.bodies[leader].lod.level > 5;
            let call = if new > 5 && other_frozen { old <= 5 } else { old > 5 };
            if call {
                if joint < self.joints.len() {
                    self.constraint_level_changed(joint, old, new);
                } else if new < 6 {
                    self.thaw(a);
                    self.thaw(b);
                } else {
                    self.freeze(a);
                    self.freeze(b);
                }
            }
        }
    }
    /// arkCpntConstraint 1407149f0 (vtable +0x68).
    fn constraint_level_changed(&mut self, joint: usize, old: i32, new: i32) {
        let (a, b) = (self.joints[joint].a, self.joints[joint].b);
        if new < 6 {
            if old > 5 {
                let j = &mut self.joints[joint];
                if j.motor_suspended && j.motor.is_some() && !j.settings.started {
                    j.settings.started = true;
                    self.wake_joint(joint);
                }
                self.joints[joint].motor_suspended = false;
                self.thaw(a);
                self.thaw(b);
            }
        } else if old < 6 {
            let j = &mut self.joints[joint];
            if j.motor.is_some() && j.settings.started {
                j.settings.started = false;
                j.motor_suspended = true;
                self.wake_joint(joint);
            }
            self.freeze(a);
            self.freeze(b);
        }
    }
    /// 807680: a body is skipped when static or kept deactivated; an
    /// inactive body takes the change only above its thresholds, and
    /// setBodyVelocity (140c85dc0) activates it first.
    pub fn apply_wind(&mut self, index: usize, wind: [f32; 3], seconds: f32) {
        let body = &self.bodies[index];
        if !body.dynamic() || body.deactivation.mode() == MODE_KEEP_DEACTIVATED {
            return;
        }
        let Some((linear, angular)) = body.wind_change(wind, seconds) else {
            return;
        };
        if !self.bodies[index].active && !self.activate(index) {
            return;
        }
        self.bodies[index].add_velocity(linear, angular);
    }

    /// 140cdc150: groups of active motions linked by constraints deactivate
    /// together when every member is a candidate; an always-active member
    /// blocks its group.
    fn deactivate_islands(&mut self) {
        let count = self.bodies.len();
        let mut parent: Vec<usize> = (0..count).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let member = |b: &MachineryBody| b.dynamic() && b.active;
        let links: Vec<(usize, usize)> = self
            .joints
            .iter()
            .map(|j| (j.a, j.b))
            .chain(self.atom_joints.iter().map(|j| (j.a, j.b)))
            .collect();
        for &(a, b) in &links {
            if member(&self.bodies[a]) && member(&self.bodies[b]) {
                let (x, y) = (root(&mut parent, a), root(&mut parent, b));
                parent[x.max(y)] = x.min(y);
            }
        }
        let mut groups = std::collections::BTreeMap::<usize, Vec<usize>>::new();
        for i in (0..count).filter(|&i| member(&self.bodies[i])) {
            groups.entry(root(&mut parent, i)).or_default().push(i);
        }
        for (island, members) in groups {
            let sleeps = members.iter().all(|&i| {
                let b = &self.bodies[i];
                b.candidate && b.deactivation.mode() != MODE_ALWAYS_ACTIVE
            });
            if sleeps {
                for i in members {
                    self.bodies[i].active = false;
                    self.bodies[i].deactivation.set_island(island as i32);
                }
            }
        }
    }
    /// 140cdd770: an active constrained body activates its linked group
    /// members that are not kept deactivated.
    fn activate_linked(&mut self) {
        if self.deactivation_disabled {
            return;
        }
        let links: Vec<(usize, usize)> = self
            .joints
            .iter()
            .map(|j| (j.a, j.b))
            .chain(self.atom_joints.iter().map(|j| (j.a, j.b)))
            .collect();
        loop {
            let mut changed = false;
            for &(a, b) in &links {
                for (from, to) in [(a, b), (b, a)] {
                    let (f, t) = (&self.bodies[from], &self.bodies[to]);
                    if f.dynamic() && f.active && t.dynamic() && !t.active && self.activate(to) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }
    /// The two physics timespaces supplied by 8af460 (clock groups3/4).
    pub fn step(&mut self, deltas: [f32; 2]) {
        let Some(step) = SolverParameters::from_game_deltas(&deltas, [0., 0., -9.81, 0.]) else {
            return;
        };
        self.activate_linked();
        self.velocities.clear();
        self.velocities.extend(self.bodies.iter().map(|body| {
            let mut properties = body.properties;
            let solved = body.dynamic() && body.active;
            if !solved {
                properties.gravity_factor = 0.;
            }
            // A deactivated motion leaves the solver (140cd7820) and keeps
            // its velocity in the motion until it is activated again.
            let (linear, angular, mass) = if solved || !body.dynamic() {
                (body.linear, body.angular, body.motion.inverse_mass_values())
            } else {
                ([0.; 3], [0.; 3], [0.; 4])
            };
            SolverVelocities::begin_motion(linear, angular, mass, properties, &step)
        }));
        self.rows.clear();
        // Query coefficients measured at the game's motor callback: its
        // impulse dt/inverse dt are substep values, callback timing is the
        // full step. Stabilized constraint tau/damping are separate from
        // the velocity solver's .6/1 coefficients.
        let query = ConstraintParameters::from_step(&step);
        // Physics-system constraints precede the constraint components in
        // the solver (14076abc0 adds the system before components start).
        let atom_step = atoms::atom_step(&step, &query);
        for joint in &mut self.atom_joints {
            let inactive = [joint.a, joint.b]
                .iter()
                .any(|&i| self.bodies[i].dynamic() && !self.bodies[i].active);
            if inactive {
                joint.skip();
            } else {
                joint.build(joint_input(&self.bodies[joint.a],&self.bodies[joint.a].body_transform()),
                    joint_input(&self.bodies[joint.b],&self.bodies[joint.b].body_transform()),&atom_step);
            }
        }
        for joint in &mut self.joints {
            let a = &self.bodies[joint.a];
            let b = &self.bodies[joint.b];
            // approximate_: a constraint touching a deactivated motion is not
            // solved; freezing and island deactivation keep both sides alike.
            if [a, b].iter().any(|body| body.dynamic() && !body.active) {
                self.rows.push(None);
                continue;
            }
            self.rows.push(Some(joint.build(joint_input(a,&a.body_transform()),
                joint_input(b,&b.body_transform()),&step,&query,deltas[0])));
        }
        for substep in 0..step.substeps {
            let phase = step.phase(substep, 0);
            for slot in &self.solve_order {
                match *slot {
                    SolveSlot::Atom(i) => self.atom_joints[i].solve(&mut self.velocities, &step),
                    SolveSlot::Hinge(i) => {
                        if let Some(rows) = &mut self.rows[i] {
                            solve_hinge(&self.joints[i], rows, &mut self.velocities, &step, phase.velocity_fraction);
                        }
                    }
                }
            }
            if substep + 1 < step.substeps {
                for (body, velocity) in self.bodies.iter().zip(&mut self.velocities) {
                    let gravity = std::array::from_fn(|i| {
                        if velocity.inverse_mass[3] == 0. {
                            0.
                        } else {
                            body.properties.gravity_factor * step.gravity_substep[i]
                        }
                    });
                    velocity.advance_substep(
                        body.properties,
                        &step,
                        body.properties.maximum_angular_velocity,
                        gravity,
                    );
                }
            }
        }
        for (body, velocity) in self.bodies.iter_mut().zip(&self.velocities) {
            if velocity.inverse_mass[3] == 0. {
                continue;
            }
            let previous = body.linear;
            let result = velocity.finish_motion(
                body.properties,
                &step,
                MotionHistory {
                    linear: body.linear,
                    integrated_linear: body.integrated_linear,
                    maximum_angular_velocity: body.properties.maximum_angular_velocity,
                    maximum_linear_change: f32::from_bits(0x7f7f0000),
                },
            );
            body.motion.pose.integrate(
                vector(result.integration_linear),
                vector(result.integration_angular),
                [step.seconds; 4],
            );
            body.linear = result.linear;
            body.angular = result.angular;
            body.integrated_linear = result.integration_linear;
            // 1412b5130 -> 140ce91c0 on the integrated motion.
            body.candidate = !self.deactivation_disabled && body.deactivation.check(
                body.motion.pose.center,
                body.motion.pose.orientation,
                vector(body.linear),
                vector(previous),
                &body.profile,
            );
        }
        if !self.deactivation_disabled {
            self.deactivate_islands();
        }
        for joint in &mut self.atom_joints {
            joint.export(&self.velocities, &step);
        }
        // Exported solver motor history (14129c3f0 schema15), not accumulated
        // impulse or the transient saturation float from the iteration loop.
        for (joint, rows) in self.joints.iter_mut().zip(&self.rows) {
            let Some(rows) = rows else {
                continue;
            };
            joint.export(rows,&self.velocities,&step);
        }
    }
}
/// One substep of a hinge joint's rows (motor, friction, limit, angular
/// locks, then the ball socket).


#[cfg(test)]
mod tests {
    use super::*;
    fn rotor(axis: usize) -> MachineryWorld {
        // The rotor fixtures replay the solver pipeline without the
        // deactivation maintenance task.
        let mut world = MachineryWorld { deactivation_disabled: true, ..Default::default() };
        world.bodies.push(MachineryBody::fixed());
        let mass = physworld::mass::PrincipalMassProperties {
            volume: 1.,
            mass: 10.,
            center: [0.; 4],
            inertia: [2., 2., 2., 0.],
            orientation: [0., 0., 0., 1.],
        };
        world.bodies.push(MachineryBody::new(
            Some(0),
            BodyMotion::from_mass_frame(mass, [0., 0., 0., 1.], [0.; 4]),
            0.,
        ));
        world
            .add_joint(
                ConstraintSettings {
                    constraint_id: "rotor".into(),
                    entity_a: Some("blade".into()),
                    entity_b: None,
                    joint_a: None,
                    joint_b: None,
                    pivot: [0.; 3],
                    axis: std::array::from_fn(|i| if i == axis { 1. } else { 0. }),
                    basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    angular_axis: axis,
                    angular_constraint: "ANGULAR_CONSTRAINT_HINGE".into(),
                    angular_minimum: -1.,
                    angular_maximum: 1.,
                    angular_cone: 1.,
                    started: true,
                    target_position: 0.,
                    linear_constraint: "LINEAR_CONSTRAINT_FIXED".into(),
                    angular_friction: -1.,
                    angular_limits_tau_factor: 1.,
                    motor: Some(crate::entities::machinery::AngularMotorSettings {
                        mode: crate::entities::machinery::MotorMode::Velocity,
                        target_velocity: 3.,
                        target_acceleration: 6.,
                        force: 1_000_000.,
                    }),
                },
                1,
                0,
            )
            .unwrap();
        world
    }
    fn resting_hinge() -> MachineryWorld {
        let mut world = rotor(0);
        world.deactivation_disabled = false;
        world.joints[0].motor = None;
        world
    }
    #[test]
    fn resting_hinge_deactivates_after_its_checks() {
        let mut world = resting_hinge();
        let mut slept = None;
        for frame in 0..600 {
            world.step([1. / 60.; 2]);
            if slept.is_none() && !world.bodies[1].active() {
                slept = Some(frame);
            }
        }
        // Four checks at 4 frames, then the slow rule completes the counter.
        let slept = slept.expect("a still hinge deactivates");
        assert!((10..40).contains(&slept), "slept at frame {slept}");
        assert_eq!(world.bodies[1].deactivation.island(), 1);
        let pose = world.bodies[1].motion.pose;
        world.step([1. / 60.; 2]);
        assert_eq!(world.bodies[1].motion.pose, pose);
    }
    #[test]
    fn lod_freeze_keeps_bodies_deactivated_until_thawed() {
        let mut world = resting_hinge();
        world.bodies[1].angular = [2., 0., 0.];
        world.bodies[1].lod.level = 7;
        world.entity_level_changed(1, 0, 7);
        assert!(!world.bodies[1].active());
        assert_eq!(world.bodies[1].deactivation.mode(), MODE_KEEP_DEACTIVATED);
        assert_eq!(world.bodies[1].frozen, 1);
        let pose = world.bodies[1].motion.pose;
        for _ in 0..10 {
            world.step([1. / 60.; 2]);
        }
        assert_eq!(world.bodies[1].motion.pose, pose);
        assert!(!world.activate(1), "mode 2 refuses activation");
        world.bodies[1].lod.level = 3;
        world.entity_level_changed(1, 7, 3);
        assert!(world.bodies[1].active());
        assert_eq!((world.bodies[1].frozen, world.bodies[1].deactivation.mode()), (0, 0));
        // The retained velocity resumes after the thaw.
        world.step([1. / 60.; 2]);
        assert_ne!(world.bodies[1].motion.pose, pose);
    }
    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_dust_speaker_cables_settle_like_the_live_game() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let read = |p: &str| {
            catalog
                .read(p)
                .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let mut placements = crate::entities::render_placements(&document);
        placements.extend(crate::entities::placed_models_with_inheritance(&document, read).unwrap());
        let recipes = crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        for problem in &recipes.unsupported {
            if problem.contains("cable") || problem.contains("turnbuckle") {
                panic!("{problem}");
            }
        }
        assert_eq!(recipes.systems.len(), 3, "speaker cable systems");
        let mut world = MachineryWorld::from_recipes(&recipes).unwrap();
        let cable = recipes.systems.iter().position(|s| s.entity_id == "ld_cable_01_1").unwrap();
        let middle = world.systems[cable].bodies[6];
        let start = world.bodies[middle].body_transform()[3];
        for frame in 0..900 {
            world.step([1. / 60.; 2]);
            if frame % 150 == 0 {
                let t = world.bodies[middle].body_transform()[3];
                println!("frame {frame}: joint6 {:?} active {}", &t[..3], world.bodies[middle].active());
            }
        }
        let end = world.bodies[middle].body_transform()[3];
        println!("ld_cable_01_1 joint6 start {:?} end {:?}; live -17.994 -57.672 -16.101", &start[..3], &end[..3]);
        for system in &world.systems {
            for &i in &system.bodies {
                let pose = world.bodies[i].motion.pose;
                assert!(pose.center.iter().chain(&pose.orientation).all(|v| v.is_finite()));
            }
        }
        assert!(end[2] < start[2], "the cable sags under gravity");
        // Live capture: the cable entity at LOD 6/7 holds all twelve bodies in
        // activation mode 2 through its arkCpntConstraint endpoints.
        let leader = world.systems[cable].bodies[0];
        world.bodies[leader].lod.level = 7;
        world.entity_level_changed(leader, 0, 7);
        for &i in &world.systems[cable].bodies {
            assert!(!world.bodies[i].active());
            assert_eq!(world.bodies[i].deactivation.mode(), MODE_KEEP_DEACTIVATED);
        }
    }

    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_dust_speakers_weigh_on_their_turnbuckles_like_the_live_game() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let read = |p: &str| {
            catalog
                .read(p)
                .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let mut placements = crate::entities::render_placements(&document);
        placements.extend(crate::entities::placed_models_with_inheritance(&document, read).unwrap());
        let recipes = crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        let world = MachineryWorld::from_recipes(&recipes).unwrap();
        let half = |v: u16| half::f16::from_bits(v).to_f32();
        let mut checked = 0;
        for name in ["ld_kar_speaker_turnbuckle_01_3", "ld_kar_speaker_turnbuckle_01_4", "ld_kar_speaker_turnbuckle_01_5"] {
            let recipe = recipes.assemblies.iter().flat_map(|a| &a.bodies).find(|b| b.entity_id == name).unwrap();
            let body = world.bodies.iter().find(|b| b.placement == Some(recipe.placement)).unwrap();
            // Live Kirin District motions of the same prefab (5 kg turnbuckle
            // + bound 10 kg speaker): halfs [1.282 1.293 1.667 1.384], centre
            // 2.230 m below the turnbuckle body.
            let mut lanes = body.motion.inverse_mass.map(half);
            lanes[..2].sort_by(f32::total_cmp);
            assert_eq!(lanes, [1.2822266, 1.2929688, 1.6669922, 1.3837891], "{name}");
            let offset: f32 = (0..3).map(|i| (body.motion.pose.center[i] - recipe.position[i]).powi(2)).sum::<f32>().sqrt();
            assert!((offset - 2.230).abs() < 2e-3, "{name}: centre offset {offset}");
            let delta = body.delta();
            for r in 0..3 {
                for c in 0..4 {
                    let identity = if r == c { 1. } else { 0. };
                    assert!((delta[r][c] - identity).abs() < 1e-4, "{name}: spawn render delta {delta:?}");
                }
            }
            let transform = body.body_transform();
            let basis = physworld::quaternion_basis(recipe.orientation);
            for c in 0..3 {
                for r in 0..3 {
                    assert!((transform[c][r] - basis[c][r]).abs() < 1e-5, "{name}: body axes moved");
                }
                assert!((transform[3][c] - recipe.position[c]).abs() < 1e-4, "{name}: body origin moved");
            }
            checked += 1;
        }
        assert_eq!(checked, 3);
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_dust_machinery_stays_constrained() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let bytes = catalog
            .read("maps/campaign/dust/dust_p_highchaos.entities")
            .unwrap()
            .unwrap();
        let document = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
        let mut placements = crate::entities::render_placements(&document);
        placements.extend(
            crate::entities::placed_models_with_inheritance(&document, |p| {
                catalog
                    .read(p)
                    .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                    .map_err(|e| e.to_string())
            })
            .unwrap(),
        );
        let recipes =
            crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        assert!(recipes.unsupported.is_empty());
        let mut world = MachineryWorld::from_recipes(&recipes).unwrap();
        let started = std::time::Instant::now();
        let mut maximum = 0f32;
        let mut limit_violation = 0f32;
        // Cables spawn straight and sag under their attached speakers, so the
        // limits are judged once settled (the last 10 of 40 seconds).
        for frame in 0..2400 {
            world.step([1. / 60.; 2]);
            maximum = maximum.max(world.maximum_anchor_error());
            for joint in &world.joints {
                if joint.settings.angular_constraint == "ANGULAR_CONSTRAINT_LIMITED_HINGE" {
                    let angle = joint.previous_limit_angle;
                    if frame >= 1800 {
                        limit_violation = limit_violation.max(
                            (joint.settings.angular_minimum - angle)
                                .max(angle - joint.settings.angular_maximum),
                        );
                    }
                }
            }
            assert!(world.bodies.iter().all(|body| body
                .motion
                .pose
                .center
                .into_iter()
                .chain(body.motion.pose.orientation)
                .all(f32::is_finite)));
        }
        println!(
            "{} joints, max anchor error {maximum}, simulation {:?}",
            world.joint_count(),
            started.elapsed()
        );
        assert!(
            limit_violation < 0.15,
            "authored angular limit violation {limit_violation}"
        );
        assert!(
            maximum < 0.1,
            "authored machinery anchor separation {maximum}"
        );
        let moving = world
            .bodies
            .iter()
            .filter(|b| b.angular.iter().any(|v| v.abs() > 0.01))
            .count();
        assert!(moving >= 45, "only {moving} bodies are moving");
    }
    #[test]
    fn rotor_trajectories_match_executable_pipeline() {
        let mut cases = 0;
        for line in include_str!("../../tests/fixtures/rotor_trajectory.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let mut world = rotor(0);
            let dt = v(0) / 120.;
            for _ in 0..v(1) as usize {
                world.step([dt; 2]);
            }
            let body = &world.bodies[1];
            for (i, actual) in body
                .motion
                .pose
                .center
                .into_iter()
                .chain(body.motion.pose.orientation)
                .chain(body.angular)
                .enumerate()
            {
                if i == 3 {
                    continue;
                } // Havok center padding is not a physical coordinate.
                assert!(
                    (actual - v(2 + i)).abs() < 0.015,
                    "trajectory{cases} field{i}: {actual} vs {}",
                    v(2 + i)
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 9);
    }
    #[test]
    fn constrained_rotor_stays_at_pivot_and_reaches_authored_speed() {
        for substeps in [1, 2, 4] {
            for axis in 0..3 {
                let mut world = rotor(axis);
                let mut previous = world.bodies[1].motion.pose.orientation;
                let mut distance = 0.;
                for frame in 0..1200 {
                    world.step([substeps as f32 / 120.; 2]);
                    let body = &world.bodies[1];
                    assert!(
                        body.motion.pose.center[..3].iter().all(|v| v.abs() < 0.003),
                        "rotor drift {:?}",
                        body.motion.pose.center
                    );
                    let q = body.motion.pose.orientation;

                    let dot = previous
                        .iter()
                        .zip(q)
                        .map(|(a, b)| a * b)
                        .sum::<f32>()
                        .abs()
                        .min(1.);
                    if frame >= 1200 - 120 / substeps {
                        distance += 2. * dot.acos();
                    }
                    previous = q;
                    assert!(body
                        .linear
                        .into_iter()
                        .chain(body.angular)
                        .all(f32::is_finite));
                }
                println!(
                    "axis{axis} substeps{substeps} radians per second={distance}, stored={:?}",
                    world.bodies[1].angular
                );
                assert!(
                    (distance - 3.).abs() < 0.03,
                    "axis{axis} substeps{substeps} speed {distance}"
                );
            }
        }
    }
}
