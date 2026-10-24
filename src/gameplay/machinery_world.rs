//! Constrained ambient rigid bodies. Assets own recipes; this module owns poses,
//! motor history and solver state. No renderer, archive I/O or wall clock.
use super::{
    constraint_solver::{
        self as solver,
        angular::{hinge_limit_row, AngularFrictionRow, AngularLimitRow},
        ball_socket::{
            rotate_direction, AnchorFrame, BallSocketBody, BallSocketBuild, BallSocketRow,
            SolverVelocities,
        },
        motion::{MotionHistory, MotionProperties},
        step::{ConstraintParameters, SolverParameters},
        AngularLockRow, AngularMotorRow, AngularVelocityAccumulator, Basis, ConstraintBody,
        HingeHistory, HingeRowInput,
    },
    deactivation::{
        DeactivationProfile, MotionDeactivation, MODE_ALWAYS_ACTIVE, MODE_DEACTIVATE,
        MODE_KEEP_DEACTIVATED,
    },
    entity_lod::EntityLod,
    machinery::{AngularMotorHistory, ConstraintMotor, MotorStep},
    rigid_body::{BodyMotion, BodyPose},
};
use crate::{entities::machinery::ConstraintSettings, physworld};

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
fn transpose(b: Basis) -> Basis {
    std::array::from_fn(|c| std::array::from_fn(|r| if r < 3 { b[r][c] } else { 0. }))
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
    /// Owner entity placement: its LOD level, static owners included.
    pub entity_placement: Option<usize>,
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
            entity_placement: placement,
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
    fn constraint_body(&self) -> ConstraintBody {
        let mass = self.motion.inverse_mass_values();
        ConstraintBody {
            principal: transpose(physworld::quaternion_basis(self.motion.pose.orientation)),
            center: xyz(self.motion.pose.center),
            inverse_inertia: xyz(mass),
            inverse_mass: mass[3],
        }
    }
    fn anchor(&self) -> BallSocketBody {
        BallSocketBody {
            frame: AnchorFrame {
                center: self.motion.pose.center,
                world_to_mass: inverse(self.motion.pose.orientation),
            },
            inverse_mass: self.motion.inverse_mass_values(),
        }
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

struct Joint {
    settings: ConstraintSettings,
    /// arkCpntConstraint +0x8c bit 1: the motor 1407149f0 disabled on freeze.
    motor_suspended: bool,
    a: usize,
    b: usize,
    local_a: Basis,
    local_b: Basis,
    lever_a: [f32; 3],
    lever_b: [f32; 3],
    motor: Option<ConstraintMotor>,
    measurement: AngularMotorHistory,
    hinge: HingeHistory,
    previous_motor_value: f32,
    previous_limit_angle: f32,
    previous_friction_value: f32,
}
struct JointRows {
    angular: [AngularLockRow; 2],
    motor: Option<AngularMotorRow>,
    limit: Option<AngularLimitRow>,
    friction: Option<AngularFrictionRow>,
    pivot: BallSocketRow,
    motor_scale: f32,
    friction_scale: f32,
    limit_impulse: f32,
}
#[derive(Default)]
pub struct MachineryWorld {
    windmills: std::collections::HashMap<String, WindmillMotor>,
    pub bodies: Vec<MachineryBody>,
    joints: Vec<Joint>,
    rows: Vec<Option<JointRows>>,
    velocities: Vec<SolverVelocities>,
    /// hknpWorld +0x818 clear: no motion checks, sleep or activation modes.
    pub deactivation_disabled: bool,
}
struct WindmillMotor {
    constraint: String,
    default_speed: f32,
    override_speed: f32,
    enabled: bool,
}
impl MachineryWorld {
    pub fn from_recipes(
        recipes: &crate::entities::machinery::recipes::MachineryRecipes,
    ) -> Result<Self, String> {
        let mut world = Self::default();
        world.bodies.push(MachineryBody::fixed());
        let mut indices = std::collections::HashMap::<&str, usize>::new();
        for assembly in &recipes.assemblies {
            for body in &assembly.bodies {
                if indices.contains_key(body.entity_id.as_str()) {
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
                // 140c908f0 -> 140ce90e0. Motion ids follow creation order
                // here; they only phase the check frames (id & 3).
                created.deactivation =
                    MotionDeactivation::new(world.bodies.len() as u32, MODE_DEACTIVATE);
                // 809d80 -> 140c8e930: m_startAsleep adds the body inactive.
                created.active = !body.start_asleep || world.deactivation_disabled;
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
        }
        for assembly in &recipes.assemblies {
            for joint in &assembly.joints {
                let endpoint = |id: &Option<String>| -> Result<usize, String> {
                    match id {
                        None => Ok(0),
                        Some(id) => indices
                            .get(id.as_str())
                            .copied()
                            .ok_or_else(|| format!("unresolved machinery endpoint {id}")),
                    }
                };
                let a = endpoint(&joint.entity_a)?;
                let b = endpoint(&joint.entity_b)?;
                // 140712da0 returns without creating a constraint when both
                // resolved body IDs are equal, including two missing owners.
                if a != b {
                    world.add_joint(joint.clone(), a, b)?;
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
        Ok(world)
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
        let frame = settings.basis.map(vector);
        let local = |body: &MachineryBody| transform(inverse(body.motion.pose.orientation), frame);
        let lever = |body: &MachineryBody| {
            rotate_direction(
                inverse(body.motion.pose.orientation),
                std::array::from_fn(|i| settings.pivot[i] - body.motion.pose.center[i]),
            )
        };
        let local_a = local(&self.bodies[a]);
        let local_b = local(&self.bodies[b]);
        let lever_a = lever(&self.bodies[a]);
        let lever_b = lever(&self.bodies[b]);
        let motor = settings.motor.map(ConstraintMotor::from_settings);
        // The measurement history is relative BODY rotation, not the mass
        // principal axes. The constant mass frames cancel in relative changes;
        // initialize to the actual relative constraint frame before first step.
        let mut measurement = AngularMotorHistory::default();
        measurement.measure(frame, frame, xyz(local_a[settings.angular_axis]), 1., 1.);
        self.joints.push(Joint {
            settings,
            motor_suspended: false,
            a,
            b,
            local_a,
            local_b,
            lever_a,
            lever_b,
            motor,
            measurement,
            hinge: HingeHistory::default(),
            previous_motor_value: 0.,
            previous_limit_angle: 0.,
            previous_friction_value: 0.,
        });
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
        let j = self
            .joints
            .iter()
            .find(|j| j.settings.constraint_id == id)?;
        let m = j.motor.as_ref()?;
        Some((
            j.settings.started,
            j.settings.target_position,
            m.target_velocity,
            m.target_acceleration,
        ))
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
        let Some(j) = self
            .joints
            .iter_mut()
            .find(|j| j.settings.constraint_id == id)
        else {
            return false;
        };
        let Some(m) = j.motor.as_mut() else {
            return false;
        };
        let before = (j.settings.started, j.settings.target_position, m.target_velocity, m.target_acceleration);
        if let Some(v) = enabled {
            j.settings.started = v;
        }
        if let Some(v) = position.filter(|_| m.mode == super::machinery::MotorMode::Position) {
            j.settings.target_position = v;
        }
        if let Some(v) = velocity {
            m.target_velocity = if m.mode == super::machinery::MotorMode::Velocity {
                v
            } else {
                v.abs()
            };
        }
        if let Some(v) = acceleration {
            m.target_acceleration = v.abs();
        }
        let after = (j.settings.started, j.settings.target_position, m.target_velocity, m.target_acceleration);
        if before != after {
            let index = self.joints.iter().position(|j| j.settings.constraint_id == id).unwrap();
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
    /// arkCpntRigidBody 808220 (vtable +0x138).
    fn freeze(&mut self, index: usize) {
        let body = &self.bodies[index];
        if body.frozen != 0 || !body.dynamic() {
            return;
        }
        self.bodies[index].frozen = if body.active { 1 } else { 2 };
        self.set_activation_mode(index, MODE_KEEP_DEACTIVATED);
    }
    /// arkCpntRigidBody 808010 (vtable +0x140) -> 807890 for a body that
    /// was active when frozen.
    fn thaw(&mut self, index: usize) {
        let frozen = self.bodies[index].frozen;
        if frozen == 0 {
            return;
        }
        self.set_activation_mode(index, MODE_DEACTIVATE);
        if frozen == 1 {
            self.activate(index);
        }
        self.bodies[index].frozen = 0;
    }
    /// 14087bb70 -> arkCpntRigidBody 140727390: an owner level change reaches
    /// each attached constraint when it crosses the frozen state, which needs
    /// every attached owner (the world counts as frozen) at level 6+.
    pub fn entity_level_changed(&mut self, body: usize, old: i32, new: i32) {
        for joint in 0..self.joints.len() {
            let (a, b) = (self.joints[joint].a, self.joints[joint].b);
            if a != body && b != body {
                continue;
            }
            let other = if a == body { b } else { a };
            let other_frozen = other == 0 || self.bodies[other].lod.level > 5;
            let call = if new > 5 && other_frozen { old <= 5 } else { old > 5 };
            if call {
                self.constraint_level_changed(joint, old, new);
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
        for joint in &self.joints {
            if member(&self.bodies[joint.a]) && member(&self.bodies[joint.b]) {
                let (x, y) = (root(&mut parent, joint.a), root(&mut parent, joint.b));
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
    pub fn step(&mut self, deltas: [f32; 5]) {
        let Some(step) = SolverParameters::from_game_deltas(&deltas, [0., 0., -9.81, 0.]) else {
            return;
        };
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
        let constraint_tau = query.stabilized_tau;
        let constraint_damping = query.stabilized_damping;
        let inverse_step_tau = query.inverse_step_tau;
        for joint in &mut self.joints {
            let a = &self.bodies[joint.a];
            let b = &self.bodies[joint.b];
            // approximate_: a constraint touching a deactivated motion is not
            // solved; freezing and island deactivation keep both sides alike.
            if [a, b].iter().any(|body| body.dynamic() && !body.active) {
                self.rows.push(None);
                continue;
            }
            let fa = transform(a.motion.pose.orientation, joint.local_a);
            let fb = transform(b.motion.pose.orientation, joint.local_b);
            let ca = a.constraint_body();
            let cb = b.constraint_body();
            let axis = joint.settings.angular_axis;
            let perpendicular = (axis + 1) % 3;
            let other = (axis + 2) % 3;
            let angular = [
                solver::angular_lock_row(
                    ca,
                    cb,
                    xyz(fa[axis]),
                    xyz(fb[other]),
                    xyz(fb[perpendicular]),
                    inverse_step_tau,
                    step.damping,
                ),
                solver::angular_lock_row(
                    ca,
                    cb,
                    xyz(fa[axis]),
                    xyz(fb[perpendicular]).map(|v| -v),
                    xyz(fb[other]),
                    inverse_step_tau,
                    step.damping,
                ),
            ];
            let entity_seconds = deltas[3]; // Default entity timespace 0 -> clock group3 (877eb0).
            let scale = entity_seconds / step.seconds;
            let measured = if entity_seconds > 0. {
                joint.measurement.measure(
                    fa,
                    fb,
                    std::array::from_fn(|i| if i == axis { 1. } else { 0. }),
                    entity_seconds,
                    scale,
                )
            } else {
                0.
            };
            let motor = joint
                .motor
                .as_mut()
                .filter(|_| joint.settings.started)
                .map(|motor| {
                    solver::hinge_motor_row(
                        motor,
                        &mut joint.hinge,
                        HingeRowInput {
                            axis,
                            frame_a: fa,
                            frame_b: fb,
                            principal_a: ca.principal,
                            principal_b: cb.principal,
                            inverse_inertia_a: ca.inverse_inertia,
                            inverse_inertia_b: cb.inverse_inertia,
                            target_angle: joint.settings.target_position,
                            step: MotorStep {
                                seconds: step.seconds,
                                inverse_seconds: step.inverse_seconds,
                                time_scale: scale,
                                measured_velocity: measured,
                                position_error: 0.,
                                previous_position_error: 0.,
                                previous_solver_value: joint.previous_motor_value,
                            },
                            warm_start_scale: step.inverse_substep_seconds,
                            impulse_seconds: step.substep_seconds,
                        },
                    )
                });
            let limit = (joint.settings.angular_constraint == "ANGULAR_CONSTRAINT_LIMITED_HINGE")
                .then(|| {
                    hinge_limit_row(
                        ca,
                        cb,
                        fa,
                        fb,
                        axis,
                        joint.settings.angular_minimum,
                        joint.settings.angular_maximum,
                        &mut joint.previous_limit_angle,
                        step.inverse_substep_seconds,
                        constraint_tau,
                        constraint_damping,
                    )
                });
            let friction = if joint.settings.angular_friction >= 0. && motor.is_none() {
                let mut ja = solver::project(ca.principal, xyz(fa[axis]));
                let jb = solver::project(cb.principal, xyz(fa[axis]).map(|v| -v));
                let inv: [f32; 3] = std::array::from_fn(|i| {
                    ja[i] * ja[i] * ca.inverse_inertia[i] + jb[i] * jb[i] * cb.inverse_inertia[i]
                });
                let effective_mass =
                    solver::reciprocal((inv[1] + inv[0]) + inv[2] + f32::from_bits(0x34000000));
                ja[0] = solver::schema_axis(ja[0], 14);
                Some(AngularFrictionRow {
                    axis_a: ja,
                    axis_b: jb,
                    effective_mass,
                    correction: inverse_step_tau * joint.previous_friction_value,
                    maximum_impulse: joint.settings.angular_friction * step.substep_seconds,
                })
            } else {
                None
            };
            let huge = f32::from_bits(0x7f7fffee);
            let pivot = BallSocketRow::build(BallSocketBuild {
                a: a.anchor(),
                b: b.anchor(),
                lever_a: joint.lever_a,
                lever_b: joint.lever_b,
                inverse_step: step.inverse_substep_seconds,
                tau: constraint_tau,
                damping: step.damping,
                mass_stabilization: 0.,
                maximum_force: huge,
                maximum_impulse: huge,
                angular_impulse_limit: huge,
            });
            self.rows.push(Some(JointRows {
                angular,
                motor,
                limit,
                friction,
                pivot,
                motor_scale: 1.,
                friction_scale: 1.,
                limit_impulse: 0.,
            }));
        }
        for substep in 0..step.substeps {
            let phase = step.phase(substep, 0);
            for (joint, rows) in self.joints.iter().zip(&mut self.rows) {
                let Some(rows) = rows else {
                    continue;
                };
                let mut a = self.velocities[joint.a];
                let mut b = self.velocities[joint.b];
                let mut angular_a = AngularVelocityAccumulator {
                    current: xyz(a.angular),
                    initial: xyz(a.initial_angular),
                    inverse_inertia: xyz(a.inverse_mass),
                };
                let mut angular_b = AngularVelocityAccumulator {
                    current: xyz(b.angular),
                    initial: xyz(b.initial_angular),
                    inverse_inertia: xyz(b.inverse_mass),
                };
                if let Some(row) = rows.motor {
                    rows.motor_scale = row.solve(
                        &mut angular_a,
                        &mut angular_b,
                        step.velocity_scale,
                        phase.velocity_fraction,
                    )[1];
                }
                if let Some(row) = rows.friction {
                    rows.friction_scale = row.solve(&mut angular_a, &mut angular_b)[1];
                }
                if let Some(row) = rows.limit {
                    row.solve(
                        &mut angular_a,
                        &mut angular_b,
                        step.velocity_scale,
                        step.substep_seconds,
                        &mut rows.limit_impulse,
                    );
                }
                for row in rows.angular {
                    row.solve(&mut angular_a, &mut angular_b);
                }
                a.angular = vector(angular_a.current);
                b.angular = vector(angular_b.current);
                rows.pivot
                    .solve(&mut a, &mut b, step.substep_seconds, step.velocity_scale);
                self.velocities[joint.a] = a;
                self.velocities[joint.b] = b;
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
        // Exported solver motor history (14129c3f0 schema15), not accumulated
        // impulse or the transient saturation float from the iteration loop.
        for (joint, rows) in self.joints.iter_mut().zip(&self.rows) {
            let Some(rows) = rows else {
                continue;
            };
            let accumulator = |index: usize| {
                let v = self.velocities[index];
                AngularVelocityAccumulator {
                    current: xyz(v.angular),
                    initial: xyz(v.initial_angular),
                    inverse_inertia: xyz(v.inverse_mass),
                }
            };
            let (a, b) = (joint.a, joint.b);
            let projection = |ja, jb| {
                solver::angular::integration_projection(
                    ja,
                    jb,
                    accumulator(a),
                    accumulator(b),
                    step.substeps,
                    step.inverse_velocity_scale,
                    step.velocity_scale,
                )
            };
            if let Some(row) = rows.friction {
                joint.previous_friction_value =
                    (step.velocity_scale * step.substep_seconds * row.correction
                        - projection(row.axis_a, row.axis_b) * step.seconds)
                        * rows.friction_scale;
            }
            if let Some(row) = rows.limit {
                joint.previous_limit_angle = -(row.correction * step.substep_seconds
                    - projection(row.axis_a, row.axis_b) * step.seconds);
            }
            if let Some(row) = rows.motor {
                let accumulator = |index: usize| {
                    let v = self.velocities[index];
                    AngularVelocityAccumulator {
                        current: xyz(v.angular),
                        initial: xyz(v.initial_angular),
                        inverse_inertia: xyz(v.inverse_mass),
                    }
                };
                joint.previous_motor_value = solver::angular::motor_history_value(
                    row,
                    accumulator(joint.a),
                    accumulator(joint.b),
                    step.substeps,
                    step.seconds,
                    step.substep_seconds,
                    step.inverse_velocity_scale,
                    step.velocity_scale,
                    rows.motor_scale,
                );
            }
        }
    }
}
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
            world.step([1. / 60.; 5]);
            if slept.is_none() && !world.bodies[1].active() {
                slept = Some(frame);
            }
        }
        // Four checks at 4 frames, then the slow rule completes the counter.
        let slept = slept.expect("a still hinge deactivates");
        assert!((10..40).contains(&slept), "slept at frame {slept}");
        assert_eq!(world.bodies[1].deactivation.island(), 1);
        let pose = world.bodies[1].motion.pose;
        world.step([1. / 60.; 5]);
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
            world.step([1. / 60.; 5]);
        }
        assert_eq!(world.bodies[1].motion.pose, pose);
        assert!(!world.activate(1), "mode 2 refuses activation");
        world.bodies[1].lod.level = 3;
        world.entity_level_changed(1, 7, 3);
        assert!(world.bodies[1].active());
        assert_eq!((world.bodies[1].frozen, world.bodies[1].deactivation.mode()), (0, 0));
        // The retained velocity resumes after the thaw.
        world.step([1. / 60.; 5]);
        assert_ne!(world.bodies[1].motion.pose, pose);
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
        for _ in 0..600 {
            world.step([1. / 60.; 5]);
            maximum = maximum.max(world.maximum_anchor_error());
            for joint in &world.joints {
                if joint.settings.angular_constraint == "ANGULAR_CONSTRAINT_LIMITED_HINGE" {
                    let angle = joint.previous_limit_angle;
                    limit_violation = limit_violation.max(
                        (joint.settings.angular_minimum - angle)
                            .max(angle - joint.settings.angular_maximum),
                    );
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
                world.step([dt; 5]);
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
                    world.step([substeps as f32 / 120.; 5]);
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
