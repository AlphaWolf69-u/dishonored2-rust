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
    pub camera: PlayerCamera,
    pub stances: LocoStances,
    pub attributes: Attributes,
    pub body: Body,
    /// The last body pose sampled for the view.
    pub pose: BodyPose,
    pub autocrouch: AutoCrouchSensor,
    pub lean_sensor: LeanSensor,
    pub mantle_sensor: MantleSensor,
    pub vault_sensor: VaultSensor,
    /// The mantle dummy's arkCpntRetargeting while it drives the player.
    pub retargeting: Option<Retargeting>,
    scenes: Scenes,
    md6: String,
    health: f32,
    /// The game's LCG at gameLocal+0x600f8 (its seed is not traced).
    random: u32,
    /// `ark_playerSmoothedLoco` / `ark_playerBobAmount` and the difficulty's
    /// lean limits, set by the host.
    pub body_settings: body::BodySettings,
    requests: MotionRequests,
    /// The arms as of the last view (PRE_RENDER).
    arm_render: Vec<ArmRender>,
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
    /// Health decl m_health + ARK_ATTRIBUTE_HEALTH_MAX.
    pub health: f32,
}

impl Player {
    /// Spawn with the feet at `position` (idPlayerStart.spawnPosition) facing
    /// `yaw` radians.
    pub fn new(mut setup: PlayerSetup, position: Vec3, yaw: f32, base_fov: f32) -> Result<Self, String> {
        let stances = LocoStances { transitions: setup.body.stance_transition_durations(), ..Default::default() };
        let pose = setup.body.pose()?;
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
        Ok(Self {
            physics,
            controller,
            machine: StateMachine::new(setup.states, &setup.rules, setup.default_state),
            camera: PlayerCamera::new(setup.camera, base_fov),
            stances,
            attributes: setup.attributes,
            body: setup.body,
            pose,
            autocrouch,
            lean_sensor: LeanSensor::new(setup.lean),
            mantle_sensor,
            vault_sensor,
            retargeting: None,
            scenes: setup.scenes,
            md6: setup.md6,
            health: setup.health,
            random: 0,
            arm_render: Vec::new(),
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
        match self.body.midnight.advance(dt, &self.body.animations) {
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
        self.controller.think(settings, frame_delta);
        self.requests = MotionRequests::default();
        let origin = self.physics.position();
        let axis = self.physics.axis();
        let velocity = self.physics.velocity;
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
                    jump_anim: &mut self.body.jump,
                    midnight: &mut self.body.midnight,
                    scenes: &self.scenes,
                    md6: &self.md6,
                    animations: &self.body.animations,
                    mantle_sensor: &mut self.mantle_sensor,
                    vault_sensor: &mut self.vault_sensor,
                    retargeting: &mut self.retargeting,
                    health: self.health,
                    random: &mut self.random,
                    small_fall_cooldown: self.body_settings.small_fall_cooldown,
                    lean_toggle: settings.lean_toggle,
                    // The host reads keyboard and mouse only.
                    keyboard: true,
                }
            };
        }
        self.machine.controller_support(&mut frame!());
        self.controller.apply_mapping(&mut self.machine);
        self.machine.think(&mut frame!());
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
        if let Ok(pose) = self.body.pose() {
            self.pose = pose;
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
        for (side, arm) in self.body.arms.iter().enumerate() {
            let Ok(joints) = arm.pose() else { continue };
            let (tag_p, tag_r) = self.pose.joint_with_controls(arm.bind_joint, &body_controls);
            let frame_p = tag_p + tag_r * arm.bind.0;
            let frame_r = tag_r * arm.bind.1;
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
            render.push(ArmRender { model: arm.model.clone(), joints: rows, visible: arm.visible });
        }
        self.arm_render = render;
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
