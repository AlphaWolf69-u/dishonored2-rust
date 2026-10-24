//! arkCpntPlayerStateMachine (vtable 0x141c73d58) with the locomotion states
//! Running, Sneaking, Sprinting, Jumping and Falling (arkPlayerStateNavigation
//! 0x141c3f198 and subclasses; InAir 0x141c7b238 is the base of the last two).
//!
//! Command processing follows FUN_140804850 and the rule table FUN_1408644e0
//! plus the mode's `m_rules`. States the port does not implement (slipping,
//! sliding, mantle, vault, autocrouch, wallcrouch, catfall, actions and
//! powers) are "missing" from the mode, so requests for them are rejected as
//! the game rejects a state its mode lacks; footsteps, animation, fall damage,
//! stats and sensors are not ported.
use super::character_proxy::SupportState;
use super::controller::Controller;
use super::physics::{PlayerPhysics, UNSET};
use super::shape::{POSTURE_KEEP_PREVIOUS, POSTURE_SNEAKING, POSTURE_STANDING};
use bevy_math::{Mat3, Vec3};
use std::collections::HashMap;

/// arkPlayerStateId_t (enum table 0x142391a90).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub i32);

impl StateId {
    pub const RUNNING: Self = Self(0);
    pub const SPRINTING: Self = Self(1);
    pub const JUMPING: Self = Self(2);
    pub const FALLING: Self = Self(3);
    pub const SNEAKING: Self = Self(6);
    pub const SLIDING: Self = Self(7);
    pub const VAULT: Self = Self(9);
    pub const AUTOCROUCH: Self = Self(10);
    pub const NULL: Self = Self(91);
    pub const ANY: Self = Self(92);
}

/// Body-part flags (decl +0x65/+0x66/+0x67).
pub const BODY: u8 = 1;

/// arkDeclPlayerState base fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateFlags {
    pub primary: bool,
    pub body_parts: u8,
    pub locked_body_parts: u8,
    pub exclusive_body_parts: u8,
    pub allow_self: bool,
    pub quit_self: bool,
    pub cancelable: bool,
    pub canceling: bool,
    pub allow_camera_out_of_shape: bool,
}

/// arkDeclPlayerStateNavigation (ctor FUN_1408684c0).
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationDecl {
    pub posture: usize,
    pub max_forward_speed: f32,
    pub unequipped_speed_increase: f32,
    pub max_backward_speed: f32,
    pub max_strafe_speed: f32,
    pub max_turn_speed_joystick: f32,
    pub max_turn_speed_mouse: f32,
    pub max_linear_acceleration: f32,
    pub max_linear_deceleration: f32,
    pub max_angular_acceleration_joystick: f32,
    pub max_angular_deceleration_joystick: f32,
    pub forward_angle_snap: f32,
    pub strafe_angle_snap: f32,
    pub max_ground_distance: f32,
    pub loco_stance_name: String,
    pub next_crushed_state: StateId,
    pub physics_shape_transition_speed_factor: f32,
}

impl Default for NavigationDecl {
    fn default() -> Self {
        Self {
            posture: POSTURE_STANDING,
            max_forward_speed: 4.0,
            unequipped_speed_increase: 0.0,
            max_backward_speed: 2.0,
            max_strafe_speed: 2.0,
            max_turn_speed_joystick: 300.0,
            max_turn_speed_mouse: 300.0,
            max_linear_acceleration: 100.0,
            max_linear_deceleration: 1000.0,
            max_angular_acceleration_joystick: 1000.0,
            max_angular_deceleration_joystick: 4000.0,
            forward_angle_snap: 30.0,
            strafe_angle_snap: 30.0,
            max_ground_distance: 0.5,
            loco_stance_name: String::new(),
            next_crushed_state: StateId::SNEAKING,
            physics_shape_transition_speed_factor: 1.0,
        }
    }
}

/// arkDeclPlayerStateJumping (ctor FUN_140867ab0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JumpingDecl {
    pub min_clear_distance_above: f32,
    pub jump_height: f32,
    pub jump_height_sprinting: f32,
    pub jump_height_crouching: f32,
    pub maximum_velocity: f32,
    pub steepness: f32,
    pub ceil_collision_factor: f32,
}

impl Default for JumpingDecl {
    fn default() -> Self {
        Self {
            min_clear_distance_above: 0.1,
            jump_height: 1.5,
            jump_height_sprinting: 2.0,
            jump_height_crouching: 1.0,
            maximum_velocity: 13.0,
            steepness: 0.28,
            ceil_collision_factor: 0.5,
        }
    }
}

/// arkDeclPlayerStateFalling (ctor FUN_1408651c0) fields the port uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallingDecl {
    pub smooth_gravity_adjustment: f32,
}

impl Default for FallingDecl {
    fn default() -> Self {
        Self { smooth_gravity_adjustment: 0.75 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SprintingDecl {
    pub minimum_stick: f32,
    pub minimum_speed: f32,
}

impl Default for SprintingDecl {
    fn default() -> Self {
        Self { minimum_stick: 0.5, minimum_speed: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Running,
    Sneaking,
    Sprinting(SprintingDecl),
    Jumping(JumpingDecl),
    Falling(FallingDecl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateDecl {
    pub id: StateId,
    pub flags: StateFlags,
    pub navigation: NavigationDecl,
    /// arkDeclPlayerStateInAir +0x150.
    pub air_friction: f32,
    pub kind: Kind,
}

impl StateDecl {
    /// The class defaults of `kind` (decl ctors) before decl fields.
    pub fn class_default(kind: Kind) -> Self {
        let navigation_flags = StateFlags {
            primary: true,
            body_parts: BODY,
            locked_body_parts: 0,
            exclusive_body_parts: BODY,
            allow_self: false,
            quit_self: false,
            cancelable: false,
            canceling: false,
            allow_camera_out_of_shape: false,
        };
        let (id, locked, air_friction, posture) = match kind {
            Kind::Running => (StateId::RUNNING, 0, 0.0, POSTURE_STANDING),
            Kind::Sneaking => (StateId::SNEAKING, 0, 0.0, POSTURE_STANDING),
            Kind::Sprinting(_) => (StateId::SPRINTING, 0, 0.0, POSTURE_STANDING),
            Kind::Jumping(_) => (StateId::JUMPING, BODY, 0.0, POSTURE_STANDING),
            Kind::Falling(_) => (StateId::FALLING, BODY, 0.1, POSTURE_KEEP_PREVIOUS),
        };
        Self {
            id,
            flags: StateFlags { locked_body_parts: locked, ..navigation_flags },
            navigation: NavigationDecl { posture, ..Default::default() },
            air_friction,
            kind,
        }
    }
}

/// arkPlayerMode::rule_t.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub allow: bool,
    pub quit: bool,
}

/// arkDeclPlayerMode::ruleDecl_t (default allow 1, quit 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RuleDecl {
    pub from: StateId,
    pub to: StateId,
    pub allow: bool,
    pub quit: bool,
}

/// The movement object's request lists (`entity+0x358` +0xe0 / +0x250):
/// one-frame requests; a push goes before older entries of the same
/// priority (FUN_14074fb40) and the consumer FUN_140753a90 takes the first.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionRequests {
    pub linear: Option<Vec3>,
    pub angular: Option<Vec3>,
}

/// arkCpntPlayerBody locomotion stance (body slot 7) and its transition
/// durations (FUN_140770e90 → FUN_140771270).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocoStances {
    pub current: Option<String>,
    /// (from, to) → duration in seconds.
    pub transitions: HashMap<(String, String), f32>,
}

impl LocoStances {
    fn enter(&mut self, stance: &str) -> f32 {
        if stance.is_empty() {
            return 0.0;
        }
        let duration = self
            .current
            .as_ref()
            .and_then(|from| self.transitions.get(&(from.clone(), stance.to_owned())))
            .copied()
            .unwrap_or(0.0);
        self.current = Some(stance.to_owned());
        duration
    }
}

/// Values other components give the states each frame.
pub struct Frame<'a> {
    pub dt: f32,
    pub controller: &'a mut Controller,
    pub physics: &'a mut PlayerPhysics,
    pub body: &'a mut LocoStances,
    pub camera: &'a mut CameraTurn,
    pub requests: &'a mut MotionRequests,
    /// Entity origin, axis (rows forward/left/up as columns) and velocity.
    pub origin: Vec3,
    pub axis: Mat3,
    pub velocity: Vec3,
    pub attributes: &'a Attributes,
    pub sprint_toggle: bool,
    pub look_stick_sensitivity: [f32; 2],
}

/// Player attribute base values (`attributes_player.cpntattributesplayer`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes(pub HashMap<String, f32>);

impl Attributes {
    /// A missing attribute index reads as 0.
    pub fn get(&self, tag: &str) -> f32 {
        self.0.get(tag).copied().unwrap_or(0.0)
    }
}

/// Camera turn speeds written by FUN_1407f04a0 (cam +0x251ac / +0x251b0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraTurn {
    pub turn_speed_joystick: f32,
    pub turn_speed_mouse: f32,
    pub pitch_speed_ratio: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Navigation {
    speed_multiplier: f32,
    agility_run_increase: f32,
    forward_cos_snap: f32,
    strafe_cos_snap: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct InAir {
    from_sneaking: bool,
    from_sprinting: bool,
    from_jumping: bool,
    inherited_velocity: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Jump {
    basic_height: f32,
    height: f32,
    successive: i32,
    timer: f32,
    takeoff_z: f32,
    apex_z: f32,
    max_velocity: f32,
    steepness: f32,
    leaving_to_falling: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fall {
    accumulated_height: f32,
    last_origin: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub decl: StateDecl,
    pub active: bool,
    navigation: Navigation,
    in_air: InAir,
    jump: Jump,
    fall: Fall,
    sneaking_first_frame: bool,
    sprinting_button_down: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Request(StateId),
    Quit(StateId),
    /// FUN_1407fbb20's landing request: the first available of Running,
    /// Sneaking, AutoCrouch and the mode default.
    Landing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StateMachine {
    states: Vec<State>,
    /// m_activeStates, primary states first.
    active: Vec<StateId>,
    queue: Vec<Command>,
    rules: HashMap<(StateId, StateId), Rule>,
    default_state: StateId,
}

fn cos_snap(degrees: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut angle = degrees * 0.017453292;
    if angle < 0.0 || angle >= TAU {
        angle -= (angle * (1.0 / TAU)).floor() * TAU;
    }
    angle.cos()
}

impl StateMachine {
    pub fn empty() -> Self {
        Self { states: Vec::new(), active: Vec::new(), queue: Vec::new(), rules: HashMap::new(), default_state: StateId::RUNNING }
    }

    /// The mode: its state decls, `m_rules` and default state.
    pub fn new(decls: Vec<StateDecl>, rule_decls: &[RuleDecl], default_state: StateId) -> Self {
        let states: Vec<State> = decls
            .into_iter()
            .map(|decl| State {
                navigation: Navigation {
                    speed_multiplier: 1.0,
                    agility_run_increase: 0.0,
                    forward_cos_snap: cos_snap(decl.navigation.forward_angle_snap),
                    strafe_cos_snap: cos_snap(decl.navigation.strafe_angle_snap),
                },
                decl,
                active: false,
                in_air: InAir::default(),
                jump: Jump::default(),
                fall: Fall::default(),
                sneaking_first_frame: false,
                sprinting_button_down: false,
            })
            .collect();
        let mut rules = HashMap::new();
        for a in &states {
            for b in &states {
                let (fa, fb) = (a.decl.flags, b.decl.flags);
                let rule = if a.decl.id == b.decl.id {
                    Rule { allow: fa.allow_self, quit: fa.quit_self }
                } else {
                    Rule {
                        allow: fb.body_parts & fa.locked_body_parts == 0,
                        quit: fb.body_parts & fa.exclusive_body_parts != 0 || (fa.cancelable && fb.canceling),
                    }
                };
                rules.insert((a.decl.id, b.decl.id), rule);
            }
        }
        // FUN_1408649e0: decl rules in order; ANY fills a row or column.
        let ids: Vec<StateId> = states.iter().map(|s| s.decl.id).collect();
        for rule in rule_decls {
            let froms: Vec<StateId> = if rule.from == StateId::ANY { ids.clone() } else { vec![rule.from] };
            let tos: Vec<StateId> = if rule.to == StateId::ANY { ids.clone() } else { vec![rule.to] };
            for &from in &froms {
                for &to in &tos {
                    if ids.contains(&from) && ids.contains(&to) {
                        rules.insert((from, to), Rule { allow: rule.allow, quit: rule.quit });
                    }
                }
            }
        }
        Self { states, active: Vec::new(), queue: Vec::new(), rules, default_state }
    }

    pub fn active_states(&self) -> &[StateId] {
        &self.active
    }

    pub fn is_active(&self, id: StateId) -> bool {
        self.active.contains(&id)
    }

    /// An active in-air state's `fromSprinting` (InAir +0x69).
    pub fn from_sprinting(&self, id: StateId) -> bool {
        self.state(id).is_some_and(|s| s.active && s.in_air.from_sprinting)
    }

    /// Any active state's `m_allowCameraOutOfShape`.
    pub fn allow_camera_out_of_shape(&self) -> bool {
        self.active.iter().any(|&id| self.state(id).is_some_and(|s| s.decl.flags.allow_camera_out_of_shape))
    }

    pub fn state(&self, id: StateId) -> Option<&State> {
        self.states.iter().find(|s| s.decl.id == id)
    }

    fn index(&self, id: StateId) -> Option<usize> {
        self.states.iter().position(|s| s.decl.id == id)
    }

    fn rule(&self, from: StateId, to: StateId) -> Rule {
        self.rules.get(&(from, to)).copied().unwrap_or(Rule { allow: true, quit: false })
    }

    /// FUN_1408022e0.
    pub fn request(&mut self, id: StateId) {
        self.queue.push(Command::Request(id));
    }

    /// FUN_140854190 → FUN_140802a20.
    pub fn quit(&mut self, id: StateId) {
        self.queue.push(Command::Quit(id));
    }

    pub fn request_landing(&mut self) {
        self.queue.push(Command::Landing);
    }

    /// FUN_140803400: the state exists and CanActivate.
    fn available(&self, id: StateId, frame: &Frame) -> bool {
        self.index(id).is_some_and(|index| self.can_activate(index, frame))
    }

    /// Slot 6 CanActivate.
    fn can_activate(&self, index: usize, frame: &Frame) -> bool {
        let state = &self.states[index];
        let physics = &*frame.physics;
        let navigation = physics.can_use_posture(state.decl.navigation.posture);
        match &state.decl.kind {
            Kind::Running => navigation,
            Kind::Sneaking => {
                // FUN_140864ea0.
                let mut ok = navigation;
                if ok {
                    ok = physics.near_ground()
                        && !(self.is_active(StateId::SPRINTING) && self.available(StateId::SLIDING, frame));
                }
                ok
            }
            Kind::Sprinting(decl) => {
                navigation
                    && can_keep_sprinting(decl, frame)
                    && !physics.vertical_active()
                    && physics.near_ground()
            }
            Kind::Jumping(decl) => {
                // FUN_140867f90; slipping and vault are not ported.
                let slip_moving = frame.controller.support_state == SupportState::Sliding && frame.velocity.length_squared() > 0.01;
                (self.is_active(StateId::VAULT) || navigation)
                    && !physics.ceiling_blocks(decl.min_clear_distance_above)
                    && !slip_moving
            }
            Kind::Falling(_) => true,
        }
    }

    /// FUN_1408063f0: commands, the primary default, then the active
    /// states' updates.
    pub fn think(&mut self, frame: &mut Frame) {
        // Commands queued while activating are processed in the same frame.
        self.process_commands(frame);
        for _ in 0..16 {
            if self.queue.is_empty() {
                break;
            }
            self.process_commands(frame);
        }
        if frame.dt != 0.0 {
            for id in self.active.clone() {
                if let Some(index) = self.index(id).filter(|&i| self.states[i].active) {
                    self.update(index, frame);
                }
            }
        }
    }

    /// FUN_140804850.
    fn process_commands(&mut self, frame: &mut Frame) {
        let queue = std::mem::take(&mut self.queue);
        let mut snapshot = self.active.clone();
        let mut accepted: Vec<StateId> = Vec::new();
        let mut quits = Vec::new();
        for command in queue {
            match command {
                Command::Quit(id) => {
                    if let Some(position) = accepted.iter().position(|&a| a == id) {
                        accepted.remove(position);
                    } else if let Some(position) = snapshot.iter().position(|&a| a == id) {
                        snapshot.remove(position);
                        quits.push(id);
                    }
                }
                Command::Landing => {
                    let mut landing = None;
                    for id in [StateId::RUNNING, StateId::SNEAKING, StateId::AUTOCROUCH, self.default_state] {
                        if self.available(id, frame) {
                            landing = Some(id);
                            break;
                        }
                    }
                    if let Some(id) = landing {
                        self.accept(id, &snapshot, &mut accepted, frame);
                    }
                }
                Command::Request(id) => self.accept(id, &snapshot, &mut accepted, frame),
            }
        }
        for &id in &accepted {
            let index = self.index(id).unwrap();
            self.on_request_accepted(index);
        }
        for id in quits {
            if let Some(position) = self.active.iter().position(|&a| a == id) {
                self.active.remove(position);
                let index = self.index(id).unwrap();
                self.deactivate(index, frame);
            }
        }
        for position in (0..self.active.len()).rev() {
            let id = self.active[position];
            if accepted.iter().any(|&r| self.rule(id, r).quit) {
                self.active.remove(position);
                let index = self.index(id).unwrap();
                self.deactivate(index, frame);
            }
        }
        for id in accepted {
            let index = self.index(id).unwrap();
            if !self.states[index].active {
                if self.states[index].decl.flags.primary {
                    self.active.insert(0, id);
                } else {
                    self.active.push(id);
                }
                self.activate(index, frame);
            }
        }
        if !self.active.iter().any(|&id| self.state(id).is_some_and(|s| s.decl.flags.primary)) {
            let mut id = self.default_state;
            for _ in 0..8 {
                let Some(index) = self.index(id) else { break };
                if self.can_activate(index, frame) {
                    self.on_request_accepted(index);
                    self.active.insert(0, id);
                    self.activate(index, frame);
                    break;
                }
                id = self.states[index].decl.navigation.next_crushed_state;
            }
        }
    }

    fn accept(&self, id: StateId, snapshot: &[StateId], accepted: &mut Vec<StateId>, frame: &Frame) {
        let Some(index) = self.index(id) else { return };
        let ok = self.can_activate(index, frame)
            && snapshot.iter().all(|&a| self.rule(a, id).allow)
            && accepted.iter().all(|&r| self.rule(r, id).allow);
        if ok {
            accepted.retain(|&r| !self.rule(r, id).quit);
            accepted.push(id);
        }
    }

    /// Slot 8 OnRequestAccepted (InAir FUN_14083eb00, Jumping FUN_1408681a0,
    /// Falling FUN_140865b90).
    fn on_request_accepted(&mut self, index: usize) {
        if !matches!(self.states[index].decl.kind, Kind::Jumping(_) | Kind::Falling(_)) {
            return;
        }
        let mut in_air = InAir {
            from_sneaking: self.is_active(StateId::SNEAKING),
            from_sprinting: self.is_active(StateId::SPRINTING),
            from_jumping: self.is_active(StateId::JUMPING),
            inherited_velocity: Vec3::ZERO,
        };
        if matches!(self.states[index].decl.kind, Kind::Jumping(_)) && self.is_active(StateId::FALLING) {
            if let Some(falling) = self.state(StateId::FALLING) {
                in_air.from_sneaking = falling.in_air.from_sneaking;
            }
        }
        if matches!(self.states[index].decl.kind, Kind::Falling(_)) {
            if in_air.from_jumping {
                if let Some(jumping) = self.state(StateId::JUMPING) {
                    in_air.from_sneaking = jumping.in_air.from_sneaking;
                    in_air.from_sprinting = jumping.in_air.from_sprinting;
                }
            }
            self.states[index].fall.accumulated_height = 0.0;
        }
        self.states[index].in_air = in_air;
    }

    /// Slot 10 Activate.
    fn activate(&mut self, index: usize, frame: &mut Frame) {
        self.states[index].active = true;
        let decl = self.states[index].decl.clone();
        // Navigation Activate FUN_1408686e0.
        frame.physics.ground_query_length = decl.navigation.max_ground_distance;
        // FUN_140869500 (crush test not ported).
        let duration = frame.body.enter(&decl.navigation.loco_stance_name) * decl.navigation.physics_shape_transition_speed_factor;
        frame.physics.set_posture(decl.navigation.posture, duration);
        match decl.kind {
            Kind::Running => {
                // FUN_140869c20.
                if frame.physics.ceiling_blocks(0.0) || frame.controller.toggled(frame.controller.sneak_action) {
                    self.request(StateId::SNEAKING);
                } else if frame.controller.must_sprint {
                    self.request(StateId::SPRINTING);
                }
            }
            Kind::Sneaking => {
                // FUN_140864b60 (autocrouch/wallcrouch sensors not ported).
                if frame.physics.ceiling_blocks(0.0) {
                    self.request(StateId::AUTOCROUCH);
                } else {
                    let action = frame.controller.sneak_action;
                    frame.controller.set_toggled(action, true);
                }
                frame.controller.stop_sprint();
                self.states[index].sneaking_first_frame = true;
            }
            Kind::Sprinting(_) => {
                // FUN_140869f60.
                frame.controller.must_sprint = true;
                let action = frame.controller.sneak_action;
                frame.controller.set_toggled(action, false);
                self.states[index].sprinting_button_down = frame.controller.down(frame.controller.sprinting_action);
            }
            Kind::Jumping(jumping) => {
                self.activate_in_air(index, frame);
                // FUN_140867b70 (super jump needs SUPER_JUMP_ENABLED; vault
                // and animation are not ported).
                let in_air = self.states[index].in_air;
                let basic = if in_air.from_sprinting {
                    jumping.jump_height_sprinting
                } else if in_air.from_sneaking {
                    jumping.jump_height_crouching
                } else {
                    jumping.jump_height
                };
                let jump = &mut self.states[index].jump;
                *jump = Jump {
                    basic_height: basic,
                    height: basic,
                    successive: 1,
                    timer: 0.0,
                    takeoff_z: frame.origin.z,
                    apex_z: frame.origin.z,
                    max_velocity: jumping.maximum_velocity,
                    steepness: jumping.steepness,
                    leaving_to_falling: false,
                };
                Self::do_jump(jump, frame.physics);
            }
            Kind::Falling(_) => {
                self.activate_in_air(index, frame);
                self.states[index].fall.last_origin = frame.origin;
            }
        }
    }

    /// InAir Activate FUN_14083e870.
    fn activate_in_air(&mut self, index: usize, frame: &Frame) {
        self.states[index].in_air.inherited_velocity = Vec3::new(frame.velocity.x, frame.velocity.y, 0.0);
    }

    /// FUN_1408680a0.
    fn do_jump(jump: &mut Jump, physics: &mut PlayerPhysics) {
        jump.apex_z += jump.height;
        physics.start_vertical(jump.apex_z, jump.max_velocity, jump.steepness);
        jump.successive -= 1;
    }

    /// Slot 12 Deactivate.
    fn deactivate(&mut self, index: usize, frame: &mut Frame) {
        self.states[index].active = false;
        match self.states[index].decl.kind {
            Kind::Jumping(_) => {
                // FUN_140867ef0.
                if frame.physics.vertical_active() {
                    frame.physics.stop_vertical();
                }
                self.states[index].in_air = InAir::default();
            }
            Kind::Falling(_) => {
                // FUN_140865630.
                frame.physics.ground_query_length = self.states[index].decl.navigation.max_ground_distance;
                self.states[index].in_air = InAir::default();
            }
            _ => {}
        }
    }

    /// Slot 13 Update.
    fn update(&mut self, index: usize, frame: &mut Frame) {
        let kind = self.states[index].decl.kind.clone();
        let id = self.states[index].decl.id;
        match kind {
            Kind::Running => {
                self.states[index].navigation.speed_multiplier = frame.attributes.get("ARK_ATTRIBUTE_SPEED_MULTIPLIER");
                self.navigation_update(index, frame);
            }
            Kind::Sneaking => {
                // FUN_140865050.
                self.states[index].navigation.speed_multiplier = frame.attributes.get("ARK_ATTRIBUTE_SPEED_MULTIPLIER")
                    * frame.attributes.get("ARK_ATTRIBUTE_MOVE_SPEED_SNEAKING_FACTOR");
                self.navigation_update(index, frame);
                let stand = frame.physics.posture_settings(POSTURE_STANDING).height;
                let sneak = frame.physics.posture_settings(POSTURE_SNEAKING).height;
                if !self.states[index].sneaking_first_frame
                    && frame.physics.free_to_stand(stand - sneak)
                    && !frame.controller.toggled(frame.controller.sneak_action)
                    && self.available(StateId::RUNNING, frame)
                {
                    self.quit(id);
                    self.request(StateId::RUNNING);
                }
                self.states[index].sneaking_first_frame = false;
            }
            Kind::Sprinting(decl) => {
                // FUN_14086a5b0 (breath is not ported).
                self.states[index].navigation.speed_multiplier = frame.attributes.get("ARK_ATTRIBUTE_SPEED_MULTIPLIER");
                self.states[index].navigation.agility_run_increase = frame.attributes.get("ARK_ATTRIBUTE_SPRINT_SPEED_INCREMENT");
                self.navigation_update(index, frame);
                let action = frame.controller.sprinting_action;
                let down = frame.controller.down(action);
                let stop = if !frame.sprint_toggle {
                    !down
                } else {
                    let stop = down && down != self.states[index].sprinting_button_down;
                    self.states[index].sprinting_button_down = down;
                    if stop {
                        frame.controller.stop_sprint();
                    }
                    stop
                };
                if stop || !can_keep_sprinting(&decl, frame) {
                    self.request(StateId::RUNNING);
                    frame.controller.must_sprint = false;
                }
            }
            Kind::Jumping(decl) => {
                self.in_air_update(index, frame);
                self.jumping_update(index, &decl, frame);
            }
            Kind::Falling(decl) => {
                self.in_air_update(index, frame);
                self.falling_update(index, &decl, frame);
            }
        }
    }

    /// ComputeMoveVelocity (slot 24, FUN_140868a80).
    fn move_velocity(&self, index: usize, frame: &Frame, forward: f32, backward: f32, strafe: f32) -> Vec3 {
        let navigation = &self.states[index].navigation;
        let input = frame.controller.input_move;
        let (fw, lf) = (frame.axis.x_axis, frame.axis.y_axis);
        let longitudinal = if input.x > 0.0 { forward } else { backward };
        let mut velocity = lf * (input.y * strafe) + fw * (input.x * longitudinal);
        let mut magnitude = input.length();
        // 0.1 (0x141ab3b0c), 0.9 (0x141ac95dc).
        if magnitude > 0.1 {
            if magnitude > 0.9 {
                magnitude = 1.0;
            }
            if input.x.abs() > navigation.forward_cos_snap {
                velocity = velocity.normalize_or_zero() * (magnitude * longitudinal);
            } else if input.y.abs() > navigation.strafe_cos_snap {
                velocity = velocity.normalize_or_zero() * (magnitude * strafe);
            }
        }
        velocity
    }

    /// Navigation Update FUN_1408695c0.
    fn navigation_update(&mut self, index: usize, frame: &mut Frame) {
        let decl = self.states[index].decl.navigation.clone();
        let (acceleration, deceleration) = if frame.controller.look_is_analog {
            let k = frame.look_stick_sensitivity[0].max(frame.look_stick_sensitivity[1]);
            (k * decl.max_angular_acceleration_joystick, k * decl.max_angular_deceleration_joystick)
        } else {
            (UNSET, UNSET)
        };
        frame.physics.set_motion_limits(decl.max_linear_acceleration, decl.max_linear_deceleration, acceleration, deceleration);
        // The rewrite has no hand equipment: the player moves unequipped.
        let equipped = false;
        let move_as_unequipped = frame.attributes.get("ARK_ATTRIBUTE_PLAYER_MOVE_AS_UNEQUIPED") > 0.0;
        let bonus = if equipped && !move_as_unequipped { 0.0 } else { decl.unequipped_speed_increase * 0.01 };
        let factor = (1.0 + bonus) * if !equipped { frame.attributes.get("ARK_ATTRIBUTE_UNEQUIPPED_SPEED_FACTOR") } else { 1.0 };
        let navigation = self.states[index].navigation;
        let mut velocity = self.move_velocity(
            index,
            frame,
            factor * decl.max_forward_speed + navigation.agility_run_increase,
            factor * decl.max_backward_speed,
            factor * decl.max_strafe_speed,
        );
        let speed_factor = frame.attributes.0.get("ARK_ATTRIBUTE_PLAYER_NAVIGATION_SPEED_FACTOR").copied().unwrap_or(1.0);
        velocity *= speed_factor * navigation.speed_multiplier;
        // FUN_1407f04a0.
        let scale = frame.attributes.get("ARK_ATTRIBUTE_SPYGLASS_TURNSPEED")
            * frame.attributes.get("ARK_ATTRIBUTE_CAMERA_ROTATION_FACTOR")
            * 0.017453292;
        let joystick = scale * decl.max_turn_speed_joystick;
        let mouse = scale * decl.max_turn_speed_mouse;
        let yaw_rate = if frame.controller.look_is_analog { joystick } else { mouse } * frame.controller.input_look.y;
        frame.camera.turn_speed_mouse = mouse;
        frame.camera.turn_speed_joystick = joystick * frame.camera.pitch_speed_ratio;
        frame.requests.linear = Some(velocity);
        frame.requests.angular = Some(Vec3::new(0.0, 0.0, yaw_rate));
        let navigation = &mut self.states[index].navigation;
        navigation.speed_multiplier = 1.0;
        navigation.agility_run_increase = 0.0;
    }

    /// InAir Update FUN_14083ebc0.
    fn in_air_update(&mut self, index: usize, frame: &mut Frame) {
        self.navigation_update(index, frame);
        let decl = self.states[index].decl.navigation.clone();
        let steering = self.move_velocity(index, frame, decl.max_forward_speed, decl.max_backward_speed, decl.max_strafe_speed);
        let in_air = &mut self.states[index].in_air;
        let inherited = in_air.inherited_velocity;
        let mut combined = Vec3::new(inherited.x + steering.x, inherited.y + steering.y, 0.0);
        let cap2 = inherited.truncate().length_squared().max(steering.truncate().length_squared());
        if combined.length_squared() > cap2 {
            combined = combined.normalize_or_zero() * cap2.sqrt();
        }
        let k = self.states[index].decl.air_friction * frame.dt;
        let in_air = &mut self.states[index].in_air;
        in_air.inherited_velocity -= in_air.inherited_velocity * k;
        frame.requests.linear = Some(combined);
    }

    /// Jumping Update FUN_140868200.
    fn jumping_update(&mut self, index: usize, decl: &JumpingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let jump_held = frame.controller.down(super::controller::ACTION_JUMP);
        let crouch = frame.controller.toggled(frame.controller.sneak_action);
        let from = self.states[index].in_air;
        let jump = self.states[index].jump;
        if !frame.physics.vertical_active() {
            if frame.physics.ground.state == SupportState::Unsupported {
                if jump.successive > 0 && jump_held {
                    Self::do_jump(&mut self.states[index].jump, frame.physics);
                } else {
                    self.quit(id);
                    self.states[index].jump.leaving_to_falling = true;
                    self.request(StateId::FALLING);
                }
            } else {
                self.quit(id);
                if !from.from_sneaking && crouch {
                    self.request(StateId::SNEAKING);
                } else if from.from_sneaking && !crouch {
                    self.request(StateId::RUNNING);
                } else {
                    self.request(if from.from_sprinting { StateId::SPRINTING } else { StateId::RUNNING });
                }
            }
        } else if !frame.physics.ceiling_blocks(decl.min_clear_distance_above) {
            if !(jump.height <= jump.basic_height || jump_held) {
                if frame.origin.z - jump.takeoff_z <= jump.basic_height {
                    frame.physics.start_vertical(jump.takeoff_z + jump.basic_height, decl.maximum_velocity, jump.steepness);
                } else {
                    frame.physics.stop_vertical();
                    self.quit(id);
                    self.states[index].jump.leaving_to_falling = true;
                    self.request(StateId::FALLING);
                }
            }
        } else {
            let mut velocity = frame.physics.gravity_velocity;
            frame.physics.stop_vertical();
            if velocity.z > 0.0 {
                velocity.z = -(velocity.z * decl.ceil_collision_factor);
                frame.physics.set_gravity_velocity(velocity);
            }
            self.quit(id);
            if frame.physics.near_ground() {
                self.request(StateId::RUNNING);
            } else {
                self.states[index].jump.leaving_to_falling = true;
                self.request(StateId::FALLING);
            }
        }
        self.states[index].jump.timer += frame.dt;
    }

    /// Falling Update FUN_140865c50 (landing without catfall/slide/damage).
    fn falling_update(&mut self, index: usize, decl: &FallingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let fall = &mut self.states[index].fall;
        fall.accumulated_height += fall.last_origin.z - frame.origin.z;
        fall.last_origin = frame.origin;
        let from = self.states[index].in_air;
        if frame.physics.ground_distance == f32::MAX && frame.physics.ground.state == SupportState::Unsupported {
            if from.from_jumping && frame.physics.gravity_velocity.z > 0.0 {
                let mut velocity = frame.physics.gravity_velocity;
                velocity.z *= decl.smooth_gravity_adjustment;
                frame.physics.set_gravity_velocity(velocity);
            }
            // pp+0xc0 is the speed scale (1 unless scaled).
            frame.physics.ground_query_length = frame.velocity.z.abs() * frame.dt + self.states[index].decl.navigation.max_ground_distance;
        } else {
            self.states[index].fall.accumulated_height = 0.0;
            self.quit(id);
            if frame.controller.toggled(frame.controller.sneak_action) {
                self.request(StateId::SNEAKING);
            } else if from.from_sprinting {
                self.request(StateId::SPRINTING);
            } else {
                self.request(StateId::RUNNING);
            }
        }
    }
}

/// FUN_14086a0f0.
fn can_keep_sprinting(decl: &SprintingDecl, frame: &Frame) -> bool {
    let input = frame.controller.input_move.truncate();
    let horizontal = frame.physics.input_velocity().truncate();
    input.length_squared() >= decl.minimum_stick * decl.minimum_stick && horizontal.length() >= decl.minimum_speed
}
