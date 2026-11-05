//! The first-person player: controller input, the locomotion state machine,
//! physics with its character proxy, and the camera, updated in the game
//! frame's stage order (FUN_1408ad260):
//! input dispatch → QUERY_PHYSICS (physics queries) → DEFAULT (physics query
//! results, controller, state machine, camera) → PRE_PHYSICS (physics step).
pub mod animation;
pub mod arms;
pub mod body;
pub mod camera;
pub mod character_proxy;
pub mod controller;
pub mod decls;
pub mod footsteps;
pub mod health;
pub mod midnight;
pub mod physics;
pub mod swimming;
pub mod sensors;
pub mod shape;
pub mod simplex_solver;
pub mod states;

use bevy_math::{Mat3, Vec3};
use body::Body;
use camera::{BodyPose, PlayerCamera, View};
use character_proxy::{PlayerWorld, SupportState};
use controller::{ActionEvent, Controller, LookSettings};
use physics::{MotionInput, PlayerPhysics};
use sensors::{AutoCrouchSensor, LeanSensor, MantleSensor, VaultSensor};
use states::{Attributes, Frame, Kind, LocoStances, MotionRequests, Retargeting, Scenes, StateId, StateMachine};

/// One first-person arm for the renderer (arkCpntPlayerBody arm parts):
/// its md6 and its joints' world transforms (game Z-up, rows of [R | t]),
/// in the arm skeleton's joint order. `visible` is the part's m_visible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArmRender {
    pub model: String,
    pub joints: Vec<[[f32; 4]; 3]>,
    pub visible: bool,
}

pub struct Player {
    pub physics: PlayerPhysics,
    pub controller: Controller,
    pub machine: StateMachine,
    swimming_machine: StateMachine,
    swimming_active: bool,
    swimming_eye_height: f32,
    swimming_current: swimming::WaterCurrent,
    swimming_waves: swimming::WaveFilters,
    swimming_wave_pose: (f32, Vec3),
    breath: swimming::Breath,
    pub incoming_damage_factor: f32,
    pub health_regen_factor: f32,
    pub camera: PlayerCamera,
    pub stances: LocoStances,
    pub attributes: Attributes,
    pub body: Body,
    /// The last body pose sampled for the view.
    pub pose: BodyPose,
    pub autocrouch: AutoCrouchSensor,
    pub lean_sensor: LeanSensor,
    pub catfall_sensor: sensors::CatfallSensor,
    pub mantle_sensor: MantleSensor,
    pub vault_sensor: VaultSensor,
    pub swimming_settings: swimming::SwimmingSettings,
    swimming_available: bool,
    pub swimming_contact: Option<swimming::SwimmingContact>,
    /// The mantle dummy's arkCpntRetargeting while it drives the player.
    pub retargeting: Option<Retargeting>,
    scenes: Scenes,
    md6: String,
    health: health::Health,
    warned_mana_drowning: bool,
    game_over_requested: bool,
    /// The game's LCG at gameLocal+0x600f8 (its seed is not traced).
    random: u32,
    /// `ark_playerSmoothedLoco` / `ark_playerBobAmount` and the difficulty's
    /// lean limits, set by the host.
    pub body_settings: body::BodySettings,
    requests: MotionRequests,
    /// The arms as of the last view (PRE_RENDER).
    arm_render: Vec<ArmRender>,
    arm_pose_errors: [Option<String>; 2],
}

/// Everything `Player::new` reads from the installed declarations.
pub struct PlayerSetup {
    pub physics: physics::PhysicsDecl,
    pub camera: camera::CameraDecl,
    pub mapping: Vec<controller::StateMapping>,
    pub states: Vec<states::StateDecl>,
    pub rules: Vec<states::RuleDecl>,
    pub default_state: StateId,
    pub attributes: Attributes,
    pub body: Body,
    pub autocrouch: sensors::AutoCrouchDecl,
    pub lean: sensors::LeanDecl,
    pub mantle: sensors::MantleDecl,
    pub slipping: controller::SlippingThresholds,
    /// The midnight scenes the states play, prepared for the body.
    pub scenes: Scenes,
    /// The body md6 the scenes' CONDMD6 nodes match.
    pub md6: String,
    /// Health decl m_health before attributes.
    pub health: health::Settings,
    pub swimming_settings: swimming::SwimmingSettings,
    pub swimming_states: Vec<swimming::SwimmingStateDecl>,
    pub swimming_rules: Vec<states::RuleDecl>,
    pub swimming_default_state: StateId,
    pub swimming_has_mantle: bool,
    pub swimming_death: Option<states::DrowningDeathDecl>,
    pub breath: swimming::Breath,
    pub accepts_water_damage: bool,
}

impl Player {
    /// The current health owned by the player; HUD consumers do not mutate it.
    pub fn health(&self) -> f32 {
        self.health.current()
    }

    pub fn max_health(&self) -> f32 { self.health.maximum(&self.attributes) }
    pub fn is_dead(&self) -> bool { self.health.is_dead() }
    pub fn projected_health(&self) -> f32 { self.health.projected(&self.attributes, self.health_regen_factor) }
    /// Advance the same game-local LCG already used by player scene picks.
    pub fn choose_random(&mut self, count: usize) -> Option<usize> {
        if count == 0 { return None; }
        self.random = self.random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
        Some(((self.random >> 10) & 0x7fff) as usize % count)
    }
    pub fn take_game_over_request(&mut self) -> bool {
        std::mem::take(&mut self.game_over_requested)
    }

    /// 14088a8c0 compares the gameplay eye with the completed head sample.
    pub fn is_underwater(&self) -> bool {
        self.swimming_contact.is_some_and(|water| self.camera.eye.z <= water.head)
    }

    pub fn is_swimming(&self) -> bool { self.swimming_active }

    /// Breath is a percentage in the original component (0..100).
    pub fn breath(&self) -> f32 { self.breath.remaining }

    pub fn arm_pose_errors(&self) -> &[Option<String>; 2] {
        &self.arm_pose_errors
    }

    /// Spawn with the feet at `position` (idPlayerStart.spawnPosition) facing
    /// `yaw` radians.
    pub fn new(mut setup: PlayerSetup, position: Vec3, yaw: f32, base_fov: f32) -> Result<Self, String> {
        let stances = LocoStances { transitions: setup.body.stance_transition_durations(), ..Default::default() };
        let pose = setup.body.pose()?;
        // 140888f10 reads the camera joint's bind-pose Z from the body rig.
        let swimming_eye_height = pose.positions[pose.camera].z;
        let physics = PlayerPhysics::new(&setup.physics, position, yaw);
        let autocrouch = AutoCrouchSensor::new(setup.autocrouch, &physics);
        let mantle_sensor = MantleSensor::new(setup.mantle, &physics);
        let limits = setup.states.iter().find_map(|s| match &s.kind {
            Kind::Vault(v) => Some(v.limits),
            _ => None,
        });
        let limits = limits.unwrap_or_else(|| states::VaultDecl::default().limits);
        let radius = physics.posture_settings(shape::POSTURE_STANDING).radius;
        let vault_sensor = VaultSensor::new(limits, radius, physics.ground_ray_normal_average);
        let mut controller = Controller::new(setup.mapping);
        controller.slipping = setup.slipping;
        let swimming_available = !setup.swimming_states.is_empty();
        let mut swimming_decls: Vec<_> = setup.swimming_states.into_iter().map(|swimming| {
            let mut state = states::StateDecl::class_default(Kind::Swimming(swimming.clone()));
            state.navigation = swimming.navigation.clone();
            state
        }).collect();
        if setup.swimming_has_mantle {
            swimming_decls.push(states::StateDecl::class_default(Kind::SwimMantle));
        }
        if let Some(dead) = setup.swimming_death {
            swimming_decls.push(states::StateDecl::class_default(Kind::Dead(dead)));
        }
        let health = health::Health::new(setup.health, setup.accepts_water_damage, &setup.attributes);
        Ok(Self {
            physics,
            controller,
            machine: StateMachine::new(setup.states, &setup.rules, setup.default_state),
            swimming_machine: StateMachine::new(swimming_decls, &setup.swimming_rules, setup.swimming_default_state),
            swimming_active: false,
            swimming_eye_height,
            swimming_current: swimming::WaterCurrent::default(),
            swimming_waves: swimming::WaveFilters::default(),
            swimming_wave_pose: (0.0, Vec3::Z),
            breath: setup.breath,
            incoming_damage_factor: 1.0,
            health_regen_factor: 1.0,
            camera: PlayerCamera::new(setup.camera, base_fov),
            stances,
            attributes: setup.attributes,
            body: setup.body,
            pose,
            autocrouch,
            lean_sensor: LeanSensor::new(setup.lean),
            catfall_sensor: sensors::CatfallSensor::new(Default::default()),
            mantle_sensor,
            vault_sensor,
            swimming_settings: setup.swimming_settings,
            swimming_available,
            swimming_contact: None,
            retargeting: None,
            scenes: setup.scenes,
            md6: setup.md6,
            health,
            warned_mana_drowning: false,
            game_over_requested: false,
            random: 0,
            arm_render: Vec::new(),
            arm_pose_errors: [None, None],
            body_settings: body::BodySettings::default(),
            requests: MotionRequests::default(),
        })
    }

    /// One simulated frame. `events` are the bound input events accumulated
    /// since the last simulated frame; `dt` is the player's clock-group delta
    /// and `frame_delta` clock group 0's.
    pub fn frame(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent], settings: &LookSettings, dt: f32, frame_delta: f32, now: f64, base_fov: f32) {
        // The midnight manager before the component stages, then its
        // message flush (BREAK_PLAYER_STATE quits a breakable state).
        let cast = body::cast(&self.body.animations, &self.body.arms);
        match self.body.midnight.advance(dt, cast) {
            Ok(messages) => {
                for (event, handle) in messages {
                    self.machine.handle_message(&event, handle);
                }
            }
            Err(error) => eprintln!("Midnight playback: {error}"),
        }
        self.controller.process_input(events, settings);
        // QUERY_PHYSICS, then the DEFAULT stage: sensor and physics results
        // before the controller (it depends on both), then the states.
        self.physics.update_queries(world);
        let sliding = self.machine.is_active(StateId::SLIDING);
        self.autocrouch.update(world, &self.physics, self.requests.linear_priority0, dt, sliding);
        self.lean_sensor.update(world, self.camera.eye, self.physics.position(), self.physics.axis());
        self.mantle_sensor.update(world, &self.physics, dt, self.physics.velocity.z);
        self.vault_sensor.update(world, &self.physics, dt);
        self.catfall_sensor.update(world, &self.physics);
        self.controller.think(settings, frame_delta);
        self.requests = MotionRequests::default();
        let mut origin = self.physics.position();
        let axis = self.physics.axis();
        let velocity = self.physics.velocity;
        let underwater = self.is_underwater();
        let mut swim_mantle_exit = false;
        if let Some(water) = self.swimming_contact {
            let (height, normal) = self.swimming_settings.wave_target(
                water.plane, water.ocean_level.unwrap_or(water.plane),
                origin.z + self.swimming_eye_height, water.samples.as_ref(),
            );
            self.swimming_wave_pose = self.swimming_waves.update(height, normal, dt);
        } else {
            self.swimming_waves = swimming::WaveFilters::default();
            self.swimming_wave_pose = (0.0, Vec3::Z);
        }
        macro_rules! frame {
            () => {
                Frame {
                    dt,
                    world,
                    controller: &mut self.controller,
                    physics: &mut self.physics,
                    body: &mut self.stances,
                    footstep: &mut self.body.footstep,
                    footstep_clock: &mut self.body.footsteps,
                    body_speed: self.body.speed,
                    camera: &mut self.camera,
                    follow_slope: &mut self.body.follow_slope,
                    requests: &mut self.requests,
                    autocrouch: &mut self.autocrouch,
                    origin,
                    axis,
                    velocity,
                    attributes: &self.attributes,
                    sprint_toggle: settings.sprint_toggle,
                    look_stick_sensitivity: settings.look_stick_sensitivity,
                    lean: &mut self.body.lean,
                    lean_sensor: &mut self.lean_sensor,
                    catfall_sensor: &mut self.catfall_sensor,
                    arms_shown: &mut self.body.arms_shown,
                    desired_legs: &mut self.body.desired_legs,
                    jump_anim: &mut self.body.jump,
                    midnight: &mut self.body.midnight,
                    scenes: &self.scenes,
                    md6: body::cast_md6(&self.md6, &self.body.arms),
                    animations: body::cast(&self.body.animations, &self.body.arms),
                    mantle_sensor: &mut self.mantle_sensor,
                    vault_sensor: &mut self.vault_sensor,
                    retargeting: &mut self.retargeting,
                    health: self.health.current(),
                    underwater,
                    swim_mantle_exit: &mut swim_mantle_exit,
                    game_over_requested: &mut self.game_over_requested,
                    random: &mut self.random,
                    small_fall_cooldown: self.body_settings.small_fall_cooldown,
                    lean_toggle: settings.lean_toggle,
                    // The host reads keyboard and mouse only.
                    keyboard: true,
                }
            };
        }
        let in_water = if self.health.is_dead() { self.swimming_active } else { (!self.machine.is_active(StateId::MANTLE) || self.swimming_active) && self.swimming_contact.is_some_and(|water| {
            let depth = water.plane - origin.z + self.physics.ground_ray_distance_min;
            let head_underwater = self.camera.eye.z <= water.head;
            let surface = water.plane + self.swimming_settings.distance_above_surface;
            if self.swimming_active {
                let has_ground = self.physics.ground_distance < f32::MAX
                    || self.physics.ground.state != SupportState::Unsupported;
                let above_surface = !head_underwater && surface + 0.1 < origin.z + self.swimming_eye_height;
                let too_shallow = depth < self.swimming_settings.minimum_depth;
                !(has_ground && (above_surface || too_shallow)
                    && self.controller.slipping.slope_cos < self.physics.shortest_ground_ray_normal().z)
            } else {
                depth >= self.swimming_settings.minimum_depth
                    && (head_underwater || origin.z + self.swimming_eye_height <= surface)
            }
        }) };
        if in_water != self.swimming_active && self.swimming_available {
            self.machine.leave_mode(&mut frame!());
            std::mem::swap(&mut self.machine, &mut self.swimming_machine);
            self.swimming_active = in_water;
            if in_water {
                self.swimming_current = swimming::WaterCurrent::default();
                self.breath.reset_for_water_entry();
                let swim_height = self.physics.posture_settings(shape::POSTURE_SWIMMING).height;
                let old_height = self.physics.posture_settings(self.physics.posture).height;
                self.physics.proxy.position.z -= swim_height - old_height;
                self.physics.set_posture(shape::POSTURE_SWIMMING, 0.0);
                origin = self.physics.position();
            }
        }
        if !self.swimming_active {
            self.machine.controller_support(&mut frame!());
        } else if let Some(water) = self.swimming_contact {
            let current = self.swimming_current.update(water.current, dt, &self.swimming_settings);
            self.physics.set_water_motion(current,
                Some(water.plane + self.swimming_settings.distance_above_surface - self.swimming_eye_height));
        }
        self.controller.apply_mapping(&mut self.machine);
        self.machine.think(&mut frame!());
        if self.swimming_active && swim_mantle_exit {
            self.machine.leave_mode(&mut frame!());
            std::mem::swap(&mut self.machine, &mut self.swimming_machine);
            self.swimming_active = false;
            self.breath.reset_for_water_entry();
            self.machine.request(StateId::MANTLE);
            self.machine.think(&mut frame!());
        }
        if self.swimming_active && self.is_underwater() {
            let cost = self.attributes.get("ARK_ATTRIBUTE_SWIMMING_BREATH_COST_PER_SECOND");
            if cost != 0.0 { self.breath.consume(dt * cost); }
        }
        self.breath.update(dt);
        self.health.update(dt, &self.attributes, self.health_regen_factor);
        // 1404c0cf0 dispatches the authored water damage after breath is
        // exhausted. Mana-first damage requires the mana component; don't
        // silently substitute health damage when that component is missing.
        if self.is_underwater() && self.breath.remaining <= 0.0 && self.health.accepts_water() {
            let mana_factor = self.attributes.get("ARK_ATTRIBUTE_DROWNING_DAMAGES_MANA_FIRST_FACTOR");
            if mana_factor > 0.0 {
                if !self.warned_mana_drowning {
                    eprintln!("Drowning with mana-first damage requires the player mana component");
                    self.warned_mana_drowning = true;
                }
            } else {
                let scale = self.attributes.0.get("ARK_ATTRIBUTE_INCOMING_DAMAGE_SCALE").copied().unwrap_or(1.0);
                let damage = dt * self.attributes.get("ARK_ATTRIBUTE_DROWNING_DAMAGE_PER_SECOND")
                    * self.incoming_damage_factor * scale;
                if damage > 0.0 && !self.health.is_dead() {
                    self.health.damage(damage, &self.attributes);
                    if self.health.is_dead() && self.swimming_active {
                        self.machine.request(StateId::DEAD);
                    }
                }
            }
        }
        // Camera DEFAULT (FUN_1407f6140): modes, look integration, FOV.
        if dt != 0.0 {
            let yaw_offset = &mut self.body.follow_slope.yaw_offset;
            let yaw_rate = self.physics.angular_velocity;
            self.camera.update(dt, self.controller.input_look, self.controller.look_is_analog, &mut self.physics, yaw_offset, yaw_rate);
            let speed_state = self.machine.is_active(StateId::SPRINTING)
                || self.machine.from_sprinting(StateId::JUMPING)
                || self.machine.from_sprinting(StateId::FALLING);
            self.camera.update_fov(dt, base_fov, speed_state);
        }
        // Body DEFAULT (FUN_140777280).
        let input_velocity = self.physics.input_velocity();
        let supported = self.controller.support_state != SupportState::Unsupported;
        let ground_normal = self.physics.ground_ray_normal_average;
        self.body.update(dt, &self.stances, input_velocity, self.physics.axis(), self.body_settings, ground_normal, supported);
        self.body.update_footsteps(world, &self.physics, &self.attributes, dt, now);
        self.physics.lean_offset = self.body.apply_lean(dt, &mut self.lean_sensor, self.physics.axis());
        // ANIMATION (arkCpntAnimEvolved FUN_14054f500): the midnight root
        // motion as priority-1 velocities.
        if let (Some((delta, yaw)), true) = (self.body.midnight.root, dt != 0.0) {
            let axis = self.physics.axis();
            self.requests.push_linear(1, axis * delta / dt);
            self.requests.push_angular(1, Vec3::new(0.0, 0.0, yaw * 0.017453292 / dt));
        }
        self.retarget(dt);
        if dt != 0.0 {
            let input = MotionInput { linear: self.requests.linear, angular: self.requests.angular };
            self.physics.step(world, dt, input);
        }
    }

    /// FUN_1407583e0 for the player slave of the mantle dummy (orientation
    /// mode 0, no tag offset, a static dummy): priority-3 velocities that
    /// blend the player from its start onto the anchor's target.
    fn retarget(&mut self, dt: f32) {
        let Some(mut r) = self.retargeting else { return };
        if dt == 0.0 {
            return;
        }
        let t = if r.duration > 0.0 && r.timer / r.duration <= 1.0 { r.timer / r.duration } else { 1.0 };
        let master = Mat3::from_rotation_z(r.target_yaw);
        // The slave joint relative to joint 0 with the origin translation
        // removed (FUN_14024b280), in the slave's current anim.
        let mut joint = (Vec3::ZERO, Mat3::IDENTITY);
        if r.anchor {
            let current = self.body.midnight.main_clip();
            if let Some((clip, _)) = current {
                r.last_clip = Some(clip);
            }
            let sampled = current.or_else(|| r.last_clip.map(|c| (c, (self.body.animations[c].frame_count as f32 - 1.0).max(0.0))));
            if let (Some((clip, frame)), Some(anchor)) = (sampled, self.body.skeleton.anchor) {
                if let Some(local) = self.body.anchor_transform(clip, frame, anchor) {
                    joint = local;
                }
            }
        }
        let target_axis = joint.1.transpose() * master;
        let target = r.target - target_axis * joint.0;
        let snap = |v: Vec3| Vec3::from_array(v.to_array().map(|c| if c.abs() <= 1e-18 { 0.0 } else { c }));
        let p = snap(r.start) * (1.0 - t) + snap(target) * t;
        self.requests.push_linear(3, (p - self.physics.position()) / dt);
        if r.orientation {
            let target_yaw = target_axis.x_axis.y.atan2(target_axis.x_axis.x);
            let yaw = r.start_yaw + physics::wrap_angle(target_yaw - r.start_yaw) * t;
            let rate = physics::wrap_angle(yaw - self.physics.yaw) / dt;
            self.requests.push_angular(3, Vec3::new(0.0, 0.0, rate));
        } else {
            // The existing angular request, re-pushed.
            let current = self.requests.angular.unwrap_or(Vec3::ZERO);
            self.requests.push_angular(3, Vec3::new(0.0, 0.0, current.z));
        }
        r.timer += dt;
        self.retargeting = Some(r);
    }

    /// PRE_RENDER view placement.

    pub fn view(&mut self) -> View {
        // A pose that fails to sample keeps the previous one (sampling of
        // every clip is validated when the body loads).
        let sampled = if let Ok(pose) = self.body.pose() {
            self.pose = pose;
            true
        } else { false };
        if sampled && self.swimming_active {
            // 140846450 applies the wave height at the root and rotates the
            // arms joint to the smoothed water normal before camera controls.
            let (height, normal) = self.swimming_wave_pose;
            for position in &mut self.pose.positions { position.z += height; }
            let axis = self.physics.axis();
            let left = normal.cross(axis.x_axis).normalize_or_zero();
            let forward = left.cross(normal).normalize_or_zero();
            let rotation = axis.transpose() * Mat3::from_cols(forward, left, normal);
            let joint = self.pose.controls[2];
            let pivot = self.pose.positions[joint];
            for i in 0..self.pose.positions.len() {
                if self.pose.descends_from(i, joint) {
                    self.pose.positions[i] = pivot + rotation * (self.pose.positions[i] - pivot);
                    self.pose.rotations[i] = rotation * self.pose.rotations[i];
                }
            }
        }
        let allow = self.machine.allow_camera_out_of_shape();
        let body_yaw = self.body.follow_slope.yaw_offset;
        self.update_arms(body_yaw);
        self.camera.view(&self.pose, &self.physics, allow, body_yaw)
    }

    /// The arm entities' joints: each arm bound to its body tag (spine03 of
    /// the body pose after the camera's joint controls, FUN_1407506d0), its
    /// tree pose, then the camera's shoulder controls (FUN_14053bcd0: world
    /// rotations about the shoulder, children carried).
    fn update_arms(&mut self, body_yaw: f32) {
        let body_controls = self.camera.controls(&self.pose, body_yaw);
        let shoulder_controls = self.camera.arm_controls(body_yaw);
        let (position, axis) = (self.physics.position(), self.physics.axis());
        let mut render = Vec::with_capacity(self.body.arms.len());
        let poses: Vec<_> = {
            let Body { arms, midnight, .. } = &mut self.body;
            arms.iter().enumerate().map(|(side, arm)| arm.pose(midnight, side + 1)).collect()
        };
        for ((side, arm), joints) in self.body.arms.iter().enumerate().zip(poses) {
            let joints = match joints {
                Ok(joints) => {
                    self.arm_pose_errors[side.min(1)] = None;
                    joints
                }
                Err(error) => {
                    let previous = &mut self.arm_pose_errors[side.min(1)];
                    if previous.as_deref() != Some(error.as_str()) {
                        eprintln!("Player arm {} pose: {error}", arm.model);
                    }
                    *previous = Some(error);
                    continue;
                }
            };
            let (frame_p, frame_r) = self.arm_frame(arm, &body_controls);
            let shoulder = arm.skeleton.shoulder;
            let pivot = frame_p + frame_r * joints[shoulder].0;
            let rows = joints
                .iter()
                .enumerate()
                .map(|(j, &(p, r))| {
                    let (mut p, mut r) = (frame_p + frame_r * p, frame_r * r);
                    if arm.descends_from(j, shoulder) {
                        for c in &shoulder_controls[side.min(1)] {
                            p = pivot + *c * (p - pivot);
                            r = *c * r;
                        }
                    }
                    let (p, r) = (position + axis * p, axis * r);
                    std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
                })
                .collect();
            render.push(ArmRender { model: arm.model.clone(), joints: rows, visible: arm.visible && self.body.arms_shown[side.min(1)] });
        }
        // The body entity's own model: hidden-legs materials (nothing drawn)
        // unless the legs are visible (FUN_140776e60).
        if self.body.legs_visible {
            let joints = (0..self.pose.positions.len())
                .map(|j| {
                    let (p, r) = self.pose.joint_with_controls(j, &body_controls);
                    let (p, r) = (position + axis * p, axis * r);
                    std::array::from_fn(|i| [r.x_axis[i], r.y_axis[i], r.z_axis[i], p[i]])
                })
                .collect();
            render.push(ArmRender { model: self.md6.clone(), joints, visible: true });
        }
        self.arm_render = render;
    }

    fn arm_frame(&self, arm: &arms::Arm, controls: &[(usize, Mat3)]) -> (Vec3, Mat3) {
        let (tag_p, tag_r) = self.pose.joint_with_controls(arm.bind_joint, controls);
        (tag_p + tag_r * arm.bind.0, tag_r * arm.bind.1)
    }

    /// The player's entity and model-tag frames, before render-camera collision
    /// adjustments. Uses the body pose/control matrices shared with its arms.
    pub fn reference_frames(&self) -> (crate::gameplay::reference_frame::ReferenceFrame,
        hashbrown::HashMap<String, crate::gameplay::reference_frame::ReferenceFrame>) {
        use crate::gameplay::reference_frame::ReferenceFrame;
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let root = ReferenceFrame { position: origin.to_array(), basis: axis.to_cols_array_2d() };
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let mut frames = hashbrown::HashMap::new();
        let joint_frame = |joint| {
            let (position, rotation) = self.pose.joint_with_controls(joint, &controls);
            (position, rotation)
        };
        let world_frame = |position: Vec3, rotation: Mat3| ReferenceFrame {
            position: (origin + axis * position).to_array(),
            basis: (axis * rotation).to_cols_array_2d(),
        };
        for (joint, name) in self.body.joint_names.iter().enumerate() {
            let (position, rotation) = joint_frame(joint);
            frames.insert(name.clone(), world_frame(position, rotation));
        }
        // Tag lookup precedes bare joint lookup in 4b19b0.
        for (name, &(joint, offset, orientation)) in &self.body.tags {
            let (position, rotation) = joint_frame(joint);
            frames.insert(name.clone(), world_frame(position + rotation * offset, rotation * orientation));
        }
        (root, frames)
    }

    pub fn sound_rate(&self, emitter: crate::gameplay::sound::Emitter, clock_rate: f32) -> Option<f32> {
        use crate::gameplay::sound::Emitter;
        let settings = match emitter {
            Emitter::PlayerBody => self.body.sound_settings,
            Emitter::PlayerLeftArm => self.body.arms.first()?.sound_settings?,
            Emitter::PlayerRightArm => self.body.arms.get(1)?.sound_settings?,
            Emitter::Menu | Emitter::Effect(_) | Emitter::Component { .. } => return None,
        };
        Some(settings.rate(clock_rate))
    }

    /// Audio without a joint attachment uses its entity's origin and axis
    /// (140709ee0 / 1400c5b70). Bound arms use the same frame as their model
    /// (1407506d0), before shoulder controls modify individual joints.
    pub fn sound_transform(&self, emitter: crate::gameplay::sound::Emitter) -> Option<(Vec3, Mat3)> {
        use crate::gameplay::sound::Emitter;
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let side = match emitter {
            Emitter::PlayerBody => return Some((origin, axis)),
            Emitter::PlayerLeftArm => 0,
            Emitter::PlayerRightArm => 1,
            Emitter::Menu | Emitter::Effect(_) | Emitter::Component { .. } => return None,
        };
        let arm = self.body.arms.get(side)?;
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let (position, rotation) = self.arm_frame(arm, &controls);
        Some((origin + axis * position, axis * rotation))
    }

    /// 14070a7b0: the listener uses camera_jnt for orientation and its
    /// declared main/peeking joint for position. This precedes render-camera
    /// collision corrections and retains the entity fallback for absent tags.
    pub fn sound_listener(&self) -> Option<(crate::gameplay::sound::spatial::Listener, &crate::gameplay::sound::listener::Settings)> {
        let binding = self.body.sound_listener.as_ref()?;
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let controls = self.camera.controls(&self.pose, self.body.follow_slope.yaw_offset);
        let listener = binding.transform(origin, axis, self.machine.is_active(StateId(0x18)), |index| {
            self.pose.positions.get(index)?;
            self.pose.rotations.get(index)?;
            let (position, rotation) = self.pose.joint_with_controls(index, &controls);
            Some((origin + axis * position, axis * rotation))
        })?;
        Some((listener, &binding.settings))
    }

    pub fn teleport(&mut self, feet: Vec3, yaw: f32) {
        self.physics.teleport(feet, yaw);
    }

    /// Both arms (left, right) as of the last `view`; empty until the arms
    /// are loaded.
    pub fn arms(&self) -> &[ArmRender] {
        &self.arm_render
    }

    pub fn feet(&self) -> Vec3 {
        self.physics.position()
    }

    pub fn axis(&self) -> Mat3 {
        self.physics.axis()
    }
}
