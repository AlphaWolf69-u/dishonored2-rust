//! The first-person player: controller input, the locomotion state machine,
//! physics with its character proxy, and the camera, updated in the game
//! frame's stage order (FUN_1408ad260):
//! input dispatch → QUERY_PHYSICS (physics queries) → DEFAULT (physics query
//! results, controller, state machine, camera) → PRE_PHYSICS (physics step).
pub mod animation;
pub mod body;
pub mod camera;
pub mod character_proxy;
pub mod controller;
pub mod decls;
pub mod physics;
pub mod sensors;
pub mod shape;
pub mod simplex_solver;
pub mod states;

use bevy_math::{Mat3, Vec3};
use body::Body;
use camera::{BodyPose, PlayerCamera, View};
use character_proxy::PlayerWorld;
use controller::{ActionEvent, Controller, LookSettings};
use physics::{MotionInput, PlayerPhysics};
use sensors::{AutoCrouchSensor, LeanSensor};
use states::{Attributes, Frame, LocoStances, MotionRequests, StateId, StateMachine};

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
    /// `ark_playerSmoothedLoco` / `ark_playerBobAmount` and the difficulty's
    /// lean limits, set by the host.
    pub body_settings: body::BodySettings,
    requests: MotionRequests,
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
    pub slipping: controller::SlippingThresholds,
}

impl Player {
    /// Spawn with the feet at `position` (idPlayerStart.spawnPosition) facing
    /// `yaw` radians.
    pub fn new(setup: PlayerSetup, position: Vec3, yaw: f32, base_fov: f32) -> Result<Self, String> {
        let stances = LocoStances { transitions: setup.body.stance_transition_durations(), ..Default::default() };
        let pose = setup.body.pose()?;
        let physics = PlayerPhysics::new(&setup.physics, position, yaw);
        let autocrouch = AutoCrouchSensor::new(setup.autocrouch, &physics);
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
            body_settings: body::BodySettings::default(),
            requests: MotionRequests::default(),
        })
    }

    /// One simulated frame. `events` are the bound input events accumulated
    /// since the last simulated frame; `dt` is the player's clock-group delta
    /// and `frame_delta` clock group 0's.
    pub fn frame(&mut self, world: &dyn PlayerWorld, events: &[ActionEvent], settings: &LookSettings, dt: f32, frame_delta: f32, base_fov: f32) {
        self.controller.process_input(events, settings);
        // QUERY_PHYSICS, then the DEFAULT stage: sensor and physics results
        // before the controller (it depends on both), then the states.
        self.physics.update_queries(world);
        let sliding = self.machine.is_active(StateId::SLIDING);
        self.autocrouch.update(world, &self.physics, self.requests.linear_priority0, dt, sliding);
        self.lean_sensor.update(world, self.camera.eye, self.physics.position(), self.physics.axis());
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
                    camera: &mut self.camera.turn,
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
        let input_velocity = self.physics.input_velocity();
        self.body.update(dt, &self.stances, input_velocity, self.physics.axis(), self.body_settings);

        self.physics.lean_offset = self.body.apply_lean(dt, &mut self.lean_sensor, self.physics.axis());

        if dt != 0.0 {
            self.camera.update_pitch(dt, self.controller.input_look.x, self.controller.look_is_analog, &self.physics);
            let speed_state = self.machine.is_active(StateId::SPRINTING)
                || self.machine.from_sprinting(StateId::JUMPING)
                || self.machine.from_sprinting(StateId::FALLING);
            self.camera.update_fov(dt, base_fov, speed_state);
            let input = MotionInput { linear: self.requests.linear, angular: self.requests.angular };
            self.physics.step(world, dt, input);
        }
    }

    /// PRE_RENDER view placement.
    pub fn view(&mut self) -> View {
        // A pose that fails to sample keeps the previous one (sampling of
        // every clip is validated when the body loads).
        if let Ok(pose) = self.body.pose() {
            self.pose = pose;
        }
        let allow = self.machine.allow_camera_out_of_shape();
        self.camera.view(&self.pose, &self.physics, allow)
    }

    pub fn teleport(&mut self, feet: Vec3, yaw: f32) {
        self.physics.teleport(feet, yaw);
    }

    pub fn feet(&self) -> Vec3 {
        self.physics.position()
    }

    pub fn axis(&self) -> Mat3 {
        self.physics.axis()
    }
}
