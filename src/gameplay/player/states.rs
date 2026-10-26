//! arkCpntPlayerStateMachine (vtable 0x141c73d58) with the locomotion states
//! Running, Sneaking, Sprinting, Sliding, Slipping, AutoCrouch, Jumping and
//! Falling (arkPlayerStateNavigation 0x141c3f198 and subclasses; InAir
//! 0x141c7b238 is the base of Jumping and Falling), and Leaning (vtable
//! 0x141c82b28), which runs beside them.
//!
//! Command processing follows FUN_140804850 and the rule table FUN_1408644e0
//! plus the mode's `m_rules`. States the port does not implement (mantle,
//! vault, wallcrouch, catfall, actions and powers) are "missing"
//! from the mode, so requests for them are rejected as the game rejects a
//! state its mode lacks. Footsteps, fall damage, stats, slide slow-time,
//! shoves, breakables and camera constraint modes are not ported.
use super::body::{FollowSlope, JumpAnim, Lean};
use super::camera::{self, PlayerCamera, MODE_ANIMATED, MODE_CATFALL, MODE_CONSTRAINED, MODE_CONSTRAINED_WITH_ARMS};
use super::midnight::{Midnight, PreparedScene};
use super::sensors::{MantleHeight, MantleSensor, VaultLimits, VaultSensor};
use super::shape::{POSTURE_LEAN_FORWARD, POSTURE_VAULTING};
use crate::edge_animation::EdgeAnimation;
use std::sync::Arc;
use super::character_proxy::{PlayerWorld, SupportState};
use crate::gameplay::collision_filter::{layer, QueryFilter};
use super::controller::{Controller, ACTION_JUMP, ACTION_LEAN_LEFT, ACTION_LEAN_RIGHT, ACTION_MOVE_BACKWARD, ACTION_MOVE_FORWARD};
use super::physics::{PlayerPhysics, UNSET};
use super::sensors::{AutoCrouchSensor, LeanSensor};
use super::shape::{POSTURE_AUTOCROUCHING, POSTURE_KEEP_PREVIOUS, POSTURE_SLIDING, POSTURE_SNEAKING, POSTURE_STANDING};
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
    pub const CATFALL: Self = Self(4);
    pub const SLIPPING: Self = Self(5);
    pub const SNEAKING: Self = Self(6);
    pub const SLIDING: Self = Self(7);
    pub const MANTLE: Self = Self(8);
    pub const VAULT: Self = Self(9);
    pub const AUTOCROUCH: Self = Self(10);
    pub const WALLCROUCH: Self = Self(11);
    pub const LEANING: Self = Self(18);
    pub const GRAPPLE: Self = Self(27);
    pub const NULL: Self = Self(91);
    pub const ANY: Self = Self(92);
}

/// Body-part flags (decl +0x65/+0x66/+0x67).
pub const BODY: u8 = 1;
pub const LEGS: u8 = 0x10;

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
    /// +0x69 bit 0x20: a midnight break event quits it.
    pub breakable: bool,
}

/// arkDeclPlayerStateNavigation (ctor FUN_1408684c0).
#[derive(Clone, Debug, PartialEq)]
pub struct NavigationDecl {
    pub footstep: super::footsteps::Footstep,
    pub footstep_upstairs: super::footsteps::Footstep,
    pub footstep_downstairs: super::footsteps::Footstep,
    pub progressive_footstep_ratio: f32,
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
    /// m_headBobStanceName (empty: the loco stance).
    pub head_bob_stance_name: String,
    pub next_crushed_state: StateId,
    pub physics_shape_transition_speed_factor: f32,
    /// +0x148 / +0x14c.
    pub small_fall_min_height: f32,
    pub small_fall_anim_weight: f32,
    /// +0xc0 m_cameraMode / +0xc4 m_cameraModeBlendDuration (-1: the
    /// camera decl's).
    pub camera_mode: i32,
    pub camera_mode_blend_duration: f32,
}

impl Default for NavigationDecl {
    fn default() -> Self {
        Self {
            footstep: super::footsteps::Footstep::default(),
            footstep_upstairs: super::footsteps::Footstep::default(),
            footstep_downstairs: super::footsteps::Footstep::default(),
            progressive_footstep_ratio: 1.0,
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
            head_bob_stance_name: String::new(),
            next_crushed_state: StateId::SNEAKING,
            physics_shape_transition_speed_factor: 1.0,
            small_fall_min_height: 0.0,
            small_fall_anim_weight: 0.33,
            camera_mode: 0,
            camera_mode_blend_duration: -1.0,
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
    pub jump_sound_event: Option<u32>,
    pub double_jump_sound_event: Option<u32>,
    pub super_jump_sound_event: Option<u32>,
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
            jump_sound_event: None,
            double_jump_sound_event: None,
            super_jump_sound_event: None,
        }
    }
}

/// arkDeclPlayerStateFalling (ctor FUN_1408651c0) fields the port uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallingDecl {
    pub smooth_gravity_adjustment: f32,
    pub resume_sliding_max_height: f32,
    /// +0x170.
    pub full_fall_anim_height: f32,
    /// +0x158 m_catFallHeight, +0x15c m_damageHeight, +0x164 m_deathHeight.
    pub cat_fall_height: f32,
    pub damage_height: f32,
    pub death_height: f32,
    /// m_fallDamageDecl is set.
    pub fall_damage: bool,
}

impl Default for FallingDecl {
    fn default() -> Self {
        Self {
            smooth_gravity_adjustment: 0.75,
            resume_sliding_max_height: 4.0,
            full_fall_anim_height: 4.0,
            cat_fall_height: 6.0,
            damage_height: f32::from_bits(0x4107_0a3d),
            death_height: f32::from_bits(0x4160_7ae1),
            fall_damage: false,
        }
    }
}

/// arkDeclPlayerStateCatFall (ctor FUN_14085a780) fields the port uses.
#[derive(Clone, Debug, PartialEq)]
pub struct CatFallDecl {
    pub low_damage_midnight: Option<String>,
    pub high_damage_midnight: Option<String>,
    pub damage_threshold: f32,
    /// m_lookAtParams (degrees: pitch up, down, yaw left, right).
    pub look_at_params: [f32; 4],
}

impl Default for CatFallDecl {
    fn default() -> Self {
        Self { low_damage_midnight: None, high_damage_midnight: None, damage_threshold: 10.0, look_at_params: [50.0, 0.0, 50.0, 50.0] }
    }
}

/// arkDeclPlayerStateMantle (ctor FUN_14085e7b0).
#[derive(Clone, Debug, PartialEq)]
pub struct MantleStateDecl {
    /// [crouching, sneaking, idle] final postures, by height.
    pub low_midnights: [Option<String>; 3],
    pub mid_midnights: [Option<String>; 3],
    pub high_midnights: [Option<String>; 3],
    pub constraint_params: [f32; 4],
    pub low_mantle_duration: f32,
}

impl Default for MantleStateDecl {
    fn default() -> Self {
        Self {
            low_midnights: Default::default(),
            mid_midnights: Default::default(),
            high_midnights: Default::default(),
            constraint_params: [30.0, 30.0, 45.0, 45.0],
            low_mantle_duration: 0.3,
        }
    }
}

/// arkDeclPlayerStateVault (ctor FUN_140871350) fields the port uses.
#[derive(Clone, Debug, PartialEq)]
pub struct VaultDecl {
    pub min_speed: f32,
    pub limits: VaultLimits,
    pub small_midnights: Vec<String>,
    pub small_distance: f32,
    pub high_threshold: f32,
    pub high_midnights: Vec<String>,
    pub high_distance: f32,
    /// +0xf0 (unnamed): the minimum exit offset.
    pub exit_offset: f32,
    pub constraint_params: [f32; 4],
}

impl Default for VaultDecl {
    fn default() -> Self {
        Self {
            min_speed: 6.0,
            limits: VaultLimits {
                max_distance: 4.0,
                min_distance: 1.0,
                min_height: 0.25,
                max_height: 1.0,
                max_depth: 1.0,
                clear_above: 2.0,
                clear_behind: 3.0,
                clear_left: 0.5,
                clear_right: 0.5,
            },
            small_midnights: Vec::new(),
            small_distance: 1.5,
            high_threshold: 0.5,
            high_midnights: Vec::new(),
            high_distance: 1.5,
            exit_offset: 0.25,
            constraint_params: [70.0, 70.0, 70.0, 70.0],
        }
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

/// arkDeclPlayerStateSliding (ctor FUN_14086d3f0) fields the port uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlidingDecl {
    pub in_speed: f32,
    pub out_speed: f32,
    pub distance: f32,
    pub max_forward_angle: f32,
    pub max_wall_angle: f32,
    pub min_slope_angle: f32,
    pub max_slope_angle: f32,
    pub slope_influence: f32,
    /// +0x164 m_constraintParams (degrees: pitch up, down, yaw left, right).
    pub constraint_params: [f32; 4],
}

impl Default for SlidingDecl {
    fn default() -> Self {
        Self {
            in_speed: 6.0,
            out_speed: 4.0,
            distance: 10.0,
            max_forward_angle: 45.0,
            max_wall_angle: 20.0,
            min_slope_angle: 30.0,
            max_slope_angle: 90.0,
            slope_influence: 0.33,
            constraint_params: [45.0, 10.0, 50.0, 50.0],
        }
    }
}

/// arkDeclPlayerStateSlipping (ctor FUN_14086fc90).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlippingDecl {
    pub gravity_strength: f32,
}

impl Default for SlippingDecl {
    fn default() -> Self {
        Self { gravity_strength: 1.0 }
    }
}

/// arkDeclPlayerStateLeaning (ctor FUN_14085d010). Durations are seconds;
/// buttons are eActions_t. HUD labels are not ported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeaningDecl {
    pub lean_speed_gamepad: f32,
    pub lean_speed_keyboard: f32,
    pub lean_up_speed_gamepad: f32,
    pub lean_up_speed_keyboard: f32,
    pub maximum_linear_speed: f32,
    /// Degrees.
    pub minimum_rotation_angle: f32,
    pub maximum_rotation_angle: f32,
    pub recenter_threshold: f32,
    pub dead_zone_inner_radius: f32,
    pub dead_zone_outer_radius: f32,
    pub recenter_delay: f32,
    pub exit_delay: f32,
    pub blend_duration: f32,
    pub lean_left_button: i32,
    pub lean_right_button: i32,
    pub lean_up_button: i32,
    pub lean_down_button: i32,
    /// `m_startSoundEvent` (`arkDeclPlayerStateLeaning` +0xa8).
    pub start_sound_event: Option<u32>,
    /// `m_stopSoundEvent` (`arkDeclPlayerStateLeaning` +0xb0).
    pub stop_sound_event: Option<u32>,
}

impl Default for LeaningDecl {
    fn default() -> Self {
        Self {
            lean_speed_gamepad: 3.5,
            lean_speed_keyboard: 3.5,
            lean_up_speed_gamepad: 2.5,
            lean_up_speed_keyboard: 2.5,
            maximum_linear_speed: 1.5,
            minimum_rotation_angle: 25.0,
            maximum_rotation_angle: 60.0,
            recenter_threshold: 0.1,
            dead_zone_inner_radius: 0.25,
            dead_zone_outer_radius: 1.0,
            recenter_delay: 0.1,
            exit_delay: 0.1,
            blend_duration: 0.45,
            lean_left_button: ACTION_LEAN_LEFT,
            lean_right_button: ACTION_LEAN_RIGHT,
            lean_up_button: ACTION_MOVE_FORWARD,
            lean_down_button: ACTION_MOVE_BACKWARD,
            start_sound_event: None,
            stop_sound_event: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Running,
    Sneaking,
    Sprinting(SprintingDecl),
    Sliding(SlidingDecl),
    Slipping(SlippingDecl),
    AutoCrouch,
    Jumping(JumpingDecl),
    Falling(FallingDecl),
    Leaning(LeaningDecl),
    CatFall(CatFallDecl),
    Mantle(MantleStateDecl),
    Vault(VaultDecl),
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
            breakable: false,
        };
        let mut navigation = NavigationDecl::default();
        let mut flags = navigation_flags;
        let (id, air_friction) = match kind {
            Kind::Running => (StateId::RUNNING, 0.0),
            Kind::Sneaking => (StateId::SNEAKING, 0.0),
            Kind::Sprinting(_) => (StateId::SPRINTING, 0.0),
            Kind::Sliding(_) => {
                flags.body_parts |= BODY | LEGS;
                flags.locked_body_parts |= BODY | LEGS;
                navigation.max_ground_distance = 3.0;
                (StateId::SLIDING, 0.0)
            }
            Kind::Slipping(_) => {
                navigation.max_linear_acceleration = 10.0;
                navigation.max_linear_deceleration = 10.0;
                (StateId::SLIPPING, 0.0)
            }
            Kind::AutoCrouch => {
                navigation.posture = POSTURE_AUTOCROUCHING;
                navigation.max_forward_speed = 1.0;
                navigation.max_backward_speed = 0.5;
                navigation.max_strafe_speed = 0.5;
                (StateId::AUTOCROUCH, 0.0)
            }
            Kind::Jumping(_) => {
                flags.locked_body_parts = BODY;
                (StateId::JUMPING, 0.0)
            }
            Kind::Falling(_) => {
                flags.locked_body_parts = BODY;
                navigation.posture = POSTURE_KEEP_PREVIOUS;
                (StateId::FALLING, 0.1)
            }
            Kind::Leaning(_) => {
                // Not a navigation state: no body parts, allowSelf.
                flags = StateFlags { primary: false, body_parts: 0, exclusive_body_parts: 0, allow_self: true, ..flags };
                (StateId::LEANING, 0.0)
            }
            // Body + both arms (bits 1, 2, 4), breakable.
            Kind::CatFall(_) => {
                flags = StateFlags { primary: false, body_parts: 7, locked_body_parts: 7, exclusive_body_parts: 0, breakable: true, ..flags };
                (StateId::CATFALL, 0.0)
            }
            Kind::Mantle(_) => {
                flags = StateFlags { primary: false, body_parts: 0x17, locked_body_parts: 0x17, exclusive_body_parts: 0, ..flags };
                (StateId::MANTLE, 0.0)
            }
            Kind::Vault(_) => {
                flags = StateFlags { primary: false, body_parts: 0x15, locked_body_parts: 0x05, exclusive_body_parts: 0x11, ..flags };
                (StateId::VAULT, 0.0)
            }
        };
        Self { id, flags, navigation, air_friction, kind }
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
/// one-frame requests. A push goes before older entries of lower or equal
/// priority (FUN_14074fb40) and the consumer FUN_140753a90 takes the first:
/// the highest priority wins, the newest on a tie.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionRequests {
    pub linear: Option<Vec3>,
    pub angular: Option<Vec3>,
    linear_priority: i32,
    angular_priority: i32,
    /// The first priority-0 linear request (FUN_140750de0), as sensors read it.
    pub linear_priority0: Option<Vec3>,
}

impl MotionRequests {
    pub fn push_linear(&mut self, priority: i32, velocity: Vec3) {
        if self.linear.is_none() || crate::gameplay::motion_requests::takes_precedence(priority,self.linear_priority) {
            self.linear = Some(velocity);
            self.linear_priority = priority;
        }
        if priority == 0 {
            self.linear_priority0 = Some(velocity);
        }
    }

    pub fn push_angular(&mut self, priority: i32, velocity: Vec3) {
        if self.angular.is_none() || crate::gameplay::motion_requests::takes_precedence(priority,self.angular_priority) {
            self.angular = Some(velocity);
            self.angular_priority = priority;
        }
    }
}

/// arkCpntPlayerBody locomotion stance (body slot 7) and its transition
/// durations (FUN_140770e90 → FUN_140771270).
#[derive(Clone, Debug, PartialEq)]
pub struct LocoStances {
    pub current: Option<String>,
    /// Body slot 8, the head-bob stance.
    pub bob: Option<String>,
    /// (from, to) → duration in seconds.
    pub transitions: HashMap<(String, String), f32>,
    /// body+0xd98 m_maxSpeeds {back, strafe, fwd, strafe} (1e30 unset).
    pub max_speeds: [f32; 4],
    /// body+0xc48: the head bob plays (mantle clears it).
    pub bob_enabled: bool,
}

impl Default for LocoStances {
    fn default() -> Self {
        Self { current: None, bob: None, transitions: HashMap::new(), max_speeds: [UNSET; 4], bob_enabled: true }
    }
}

impl LocoStances {
    /// FUN_140770e70: the transition time between two stances (0 if none).
    pub fn duration(&self, from: &str, to: &str) -> f32 {
        self.transitions.get(&(from.to_owned(), to.to_owned())).copied().unwrap_or(0.0)
    }

    /// Enter `stance` (body slot 7); returns the transition time from the
    /// current stance.
    pub fn enter(&mut self, stance: &str) -> f32 {
        if stance.is_empty() {
            return 0.0;
        }
        let duration = self.current.as_deref().map_or(0.0, |from| self.duration(from, stance));
        self.current = Some(stance.to_owned());
        duration
    }
}

/// Values other components give the states each frame.
pub struct Frame<'a> {
    pub dt: f32,
    pub world: &'a dyn PlayerWorld,
    pub controller: &'a mut Controller,
    pub physics: &'a mut PlayerPhysics,
    pub body: &'a mut LocoStances,
    pub footstep: &'a mut super::footsteps::Footstep,
    pub footstep_clock: &'a mut super::footsteps::Footsteps,
    pub body_speed: f32,
    pub camera: &'a mut PlayerCamera,
    pub follow_slope: &'a mut FollowSlope,
    pub requests: &'a mut MotionRequests,
    pub autocrouch: &'a mut AutoCrouchSensor,
    /// Entity origin, axis (rows forward/left/up as columns) and velocity.
    pub origin: Vec3,
    pub axis: Mat3,
    pub velocity: Vec3,
    pub attributes: &'a Attributes,
    pub sprint_toggle: bool,
    pub look_stick_sensitivity: [f32; 2],
    pub lean: &'a mut Lean,
    pub lean_sensor: &'a mut LeanSensor,
    pub jump_anim: &'a mut JumpAnim,
    pub midnight: &'a mut Midnight,
    pub scenes: &'a Scenes,
    /// The player body md6 (CONDMD6 choices).
    pub md6: &'a str,
    pub animations: &'a [EdgeAnimation],
    pub mantle_sensor: &'a mut MantleSensor,
    pub vault_sensor: &'a mut VaultSensor,
    pub retargeting: &'a mut Option<Retargeting>,
    /// The player's health (health decl m_health + ARK_ATTRIBUTE_HEALTH_MAX);
    /// the rewrite has no damage, so it stays full.
    pub health: f32,
    /// The game's global LCG at gameLocal+0x600f8.
    pub random: &'a mut u32,
    /// `ark_playerSmallFallCooldown` (cvar 0x143996320, "0.5").
    pub small_fall_cooldown: f32,
    pub lean_toggle: bool,
    /// `!lastInputWasGamepad` (inputMgr+0xa544, FUN_140039d50).
    pub keyboard: bool,
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
    /// +0x4a m_smallFallStarted / +0x4c m_smallFallCooldown.
    small_fall_started: bool,
    small_fall_cooldown: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct InAir {
    from_sneaking: bool,
    from_sprinting: bool,
    from_sliding: bool,
    from_jumping: bool,
    from_grappling: bool,
    /// Jumping +0xb8.
    from_vault: bool,
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
    sound_event: Option<u32>,
    leaving_to_falling: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fall {
    accumulated_height: f32,
    last_origin: Vec3,
}

/// arkPlayerStateSliding runtime (size 0x108).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Slide {
    from_standing: f32,
    to_sneaking: f32,
    to_autocrouch: f32,
    to_standing: f32,
    blend_in_timer: f32,
    direction: Vec3,
    last_direction: Vec3,
    max_distance: f32,
    distance: f32,
    max_forward_cos: f32,
    max_wall_cos: f32,
    speed: f32,
    blend_out_state: Option<StateId>,
    blend_out_done: bool,
    keep_sprinting: bool,
    want_jump: bool,
}

/// The installed midnight scenes the states play, by decl path.
pub type Scenes = std::collections::HashMap<String, Arc<PreparedScene>>;

/// Midnight resources required by the player states currently hosted here.
pub fn scene_paths(states: &[StateDecl]) -> Vec<String> {
    let mut result = Vec::new();
    for state in states {
        let paths: Vec<&String> = match &state.kind {
            Kind::CatFall(d) => d.low_damage_midnight.iter().chain(d.high_damage_midnight.iter()).collect(),
            Kind::Mantle(d) => d.low_midnights.iter().chain(&d.mid_midnights).chain(&d.high_midnights).flatten().collect(),
            Kind::Vault(d) => d.small_midnights.iter().chain(&d.high_midnights).collect(),
            _ => Vec::new(),
        };
        for path in paths {
            if !result.contains(path) { result.push(path.clone()); }
        }
    }
    result
}

/// arkCpntRetargeting of the mantle dummy with the player as its slave
/// (FUN_140860680 setup, FUN_1407583e0 update, FUN_140757c10 release).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Retargeting {
    pub duration: f32,
    pub timer: f32,
    /// The slave joint anchor_jnt (None: the identity joint).
    pub anchor: bool,
    pub orientation: bool,
    pub start: Vec3,
    pub start_yaw: f32,
    pub target: Vec3,
    pub target_yaw: f32,
    /// The slave's last anim (FUN_140758f10: with no current anim, its last
    /// frame).
    pub last_clip: Option<usize>,
}

/// Mantle runtime (arkPlayerStateMantle +0xac params, +0xe0 dummy).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct MantleRun {
    final_posture: usize,
    final_state: i32,
    destination: Vec3,
    destination_yaw: f32,
    height: u8,
    destination_body: u32,
}

/// Mantle Activate / Deactivate: the destination entity's collision with
/// the proxy (FUN_1407fd740 / FUN_1407fd880 with layers 3, or
/// FUN_1407fd780 / FUN_1407fd8c0 for an arkCpntRigidBody's body).
fn set_mantle_collision(physics: &mut PlayerPhysics, body: u32, enabled: bool) {
    if body == 0 {
        physics.set_proxy_layers(1 << layer::WORLD_LOW | 1 << layer::WORLD_LOW_HIGH, enabled);
    } else {
        physics.proxy.set_body_ignored(body, !enabled);
    }
}

/// Vault runtime (arkPlayerStateVault +0x38..+0x7c).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct VaultRun {
    phase: u8,
    velocity: Vec3,
    forward: Vec3,
    interpolate: Vec3,
    exit: Vec3,
    height: f32,
}

/// arkPlayerStateLeaning +0x34..+0x58 (sounds and the save flag aside).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Leaning {
    up_enabled: bool,
    under_control: bool,
    left_input: f32,
    up_input: f32,
    left: f32,
    up: f32,
    forward: Vec3,
    rotation_blend: f32,
    recenter_timer: f32,
    exit_timer: f32,
    /// +0x36: the native has posted the start cue and awaits the stop cue.
    sound_active: bool,
}

/// FUN_14085d380.
fn dead_zone(value: f32, decl: &LeaningDecl) -> f32 {
    let sign = if value > 0.0 { 1.0 } else if value < 0.0 { -1.0 } else { 0.0 };
    let a = value.abs();
    let (inner, outer) = (decl.dead_zone_inner_radius, decl.dead_zone_outer_radius);
    if inner >= outer {
        sign * a
    } else if a <= inner {
        0.0
    } else if a >= outer {
        sign
    } else {
        sign * (a - inner) / (outer - inner)
    }
}

fn horizontal_unit(v: Vec3) -> Vec3 {
    let h = Vec3::new(v.x, v.y, 0.0);
    h / h.length_squared().max(f32::MIN_POSITIVE).sqrt()
}

/// The latch of the strongest input in one direction.
fn latch(input: f32, target: f32) -> f32 {
    if input * target > 0.0 {
        if target < 0.0 { input.min(target) } else { input.max(target) }
    } else {
        target
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub decl: StateDecl,
    pub active: bool,
    navigation: Navigation,
    in_air: InAir,
    jump: Jump,
    fall: Fall,
    slide: Slide,
    /// Slipping +0x6c.
    slipping_velocity: Vec3,
    /// Slipping +0x68.
    slipping_sneaking: bool,
    first_frame: bool,
    sprinting_button_down: bool,
    leaning: Leaning,
    /// +0x24: the state's midnight scene handle.
    pub midnight: u32,
    /// CatFall m_damageAmount.
    damage: f32,
    mantle: MantleRun,
    vault: VaultRun,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Request(StateId),
    /// FUN_1408024d0 with Sliding's params {uid, 7, speed}.
    RequestSlide(f32),
    /// Falling Land()'s CATFALL request with params {uid, damage}.
    RequestCatFall(f32),
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
    pending_sounds: Vec<u32>,
}

fn cos_snap(degrees: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut angle = degrees * 0.017453292;
    if angle < 0.0 || angle >= TAU {
        angle -= (angle * (1.0 / TAU)).floor() * TAU;
    }
    angle.cos()
}

/// acos clamped as the decompiled callers do (π at ≤ -1, 0 at ≥ 1).
fn slope_of(z: f32) -> f32 {
    if z <= -1.0 {
        std::f32::consts::PI
    } else if z >= 1.0 {
        0.0
    } else {
        z.acos()
    }
}

/// FUN_140001050.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (1.0 - t) * a + t * b
}

impl StateMachine {
    pub fn empty() -> Self {
        Self { states: Vec::new(), active: Vec::new(), queue: Vec::new(), rules: HashMap::new(), default_state: StateId::RUNNING, pending_sounds: Vec::new() }
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
                    small_fall_started: false,
                    small_fall_cooldown: 0.0,
                },
                decl,
                active: false,
                in_air: InAir::default(),
                jump: Jump::default(),
                fall: Fall::default(),
                slide: Slide::default(),
                slipping_velocity: Vec3::ZERO,
                slipping_sneaking: false,
                first_frame: false,
                sprinting_button_down: false,
                leaning: Leaning::default(),
                midnight: 0,
                damage: 0.0,
                mantle: MantleRun::default(),
                vault: VaultRun::default(),
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
        Self { states, active: Vec::new(), queue: Vec::new(), rules, default_state, pending_sounds: Vec::new() }
    }

    pub fn active_states(&self) -> &[StateId] {
        &self.active
    }

    pub fn take_sound_events(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.pending_sounds)
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

    fn stance(&self, id: StateId) -> &str {
        self.state(id).map_or("", |s| s.decl.navigation.loco_stance_name.as_str())
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

    /// FUN_140803400: the state exists and CanActivate (no mode rules).
    fn available(&self, id: StateId, frame: &Frame) -> bool {
        self.index(id).is_some_and(|index| self.can_activate(index, frame))
    }

    /// FUN_1408034b0 with Sliding's params: slot 7 (FUN_14086ea10) is the
    /// attribute/body-part gate and `speed > outSpeed`.
    fn slide_available(&self, speed: f32) -> bool {
        self.state(StateId::SLIDING)
            .is_some_and(|s| matches!(s.decl.kind, Kind::Sliding(d) if speed > d.out_speed))
    }

    /// Slot 6 CanActivate.
    fn can_activate(&self, index: usize, frame: &Frame) -> bool {
        let state = &self.states[index];
        let physics = &*frame.physics;
        let navigation = physics.can_use_posture(state.decl.navigation.posture);
        match &state.decl.kind {
            Kind::Running | Kind::Slipping(_) | Kind::AutoCrouch => navigation,
            Kind::Sneaking => {
                // FUN_140864ea0 (wallcrouch sensor not ported).
                let mut ok = navigation;
                if ok {
                    ok = physics.near_ground()
                        && !(self.is_active(StateId::SPRINTING) && self.available(StateId::SLIDING, frame));
                    if self.is_active(StateId::AUTOCROUCH) && frame.autocrouch.fires {
                        ok = false;
                    }
                }
                ok
            }
            Kind::Sprinting(decl) => {
                navigation
                    && can_keep_sprinting(decl, frame)
                    && !physics.vertical_active()
                    && physics.near_ground()
            }
            Kind::Sliding(decl) => navigation && self.slide_can_activate(decl, frame),
            Kind::Jumping(decl) => {
                // FUN_140867f90.
                let slip_moving = frame.controller.support_state == SupportState::Sliding && frame.velocity.length_squared() > 0.01;
                (self.is_active(StateId::VAULT) || navigation)
                    && !physics.ceiling_blocks(decl.min_clear_distance_above)
                    && !slip_moving
                    && !self.allowed(StateId::VAULT, frame)
            }
            Kind::Falling(_) => true,
            // FUN_14085d9a0: the base body-part test passes with no parts.
            Kind::Leaning(_) => physics.near_ground(),
            // FUN_14085b2e0 (swimming is not ported).
            Kind::CatFall(_) => true,
            // FUN_140860180 (blink-to-mantle is not ported).
            Kind::Mantle(_) => !self.is_active(StateId::MANTLE) && frame.mantle_sensor.can_mantle(),
            // FUN_140872380.
            Kind::Vault(decl) => {
                let v = frame.velocity;
                physics.near_ground()
                    && frame.vault_sensor.enabled
                    && frame.vault_sensor.can_vault
                    && (self.is_active(StateId::SLIDING) || v.x * v.x + v.y * v.y >= decl.min_speed * decl.min_speed)
            }
        }
    }

    /// FUN_1408031b0: available and allowed by every active state.
    fn allowed(&self, id: StateId, frame: &Frame) -> bool {
        self.available(id, frame) && self.active.iter().all(|&a| self.rule(a, id).allow)
    }

    /// The installed scene at `path` started on the player (FUN_1408549c0);
    /// 0 when it is missing or fails to start.
    fn play(path: Option<&String>, rate: f32, parts: u8, frame: &mut Frame) -> u32 {
        let Some(path) = path else { return 0 };
        let Some(scene) = frame.scenes.get(path) else {
            eprintln!("Midnight scene {path} was not loaded");
            return 0;
        };
        match frame.midnight.start_with_parts(scene, frame.md6, rate, frame.animations, parts) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("Midnight scene {path}: {error}");
                0
            }
        }
    }

    /// The midnight messages of this frame (flushed before DEFAULT):
    /// BREAK_PLAYER_STATE quits the breakable state playing that scene
    /// (FUN_140802e10).
    pub fn handle_message(&mut self, event: &str, handle: u32) {
        if event != "ARK_MDNT_MSG_BREAK_PLAYER_STATE" {
            return;
        }
        for id in self.active.clone() {
            if let Some(state) = self.state(id) {
                if state.decl.flags.breakable && state.midnight == handle && handle != 0 {
                    self.quit(id);
                }
            }
        }
    }

    /// FUN_14086e410 after the navigation test.
    fn slide_can_activate(&self, decl: &SlidingDecl, frame: &Frame) -> bool {
        let physics = &*frame.physics;
        if !physics.near_ground() {
            return true;
        }
        let normal = physics.ground_ray_normal_average;
        if frame.axis.x_axis.dot(normal) < 0.0 && slope_of(normal.z) >= decl.max_slope_angle * 0.017453292 {
            return false;
        }
        let velocity = frame.velocity.truncate().normalize_or_zero();
        let facing = frame.axis.x_axis.truncate().normalize_or_zero();
        if !(velocity.dot(facing) > cos_snap(decl.max_forward_angle)) {
            return false;
        }
        // Synchronous obstacle cast of the SLIDING shape (movable and
        // flagged entities are not distinguished: the world is static).
        let to_autocrouch = frame.body.duration(self.stance(StateId::SLIDING), self.stance(StateId::AUTOCROUCH));
        let factor = self.state(StateId::SLIDING).map_or(1.0, |s| s.decl.navigation.physics_shape_transition_speed_factor);
        let mut hits = Vec::new();
        // The physics decl's ground and ceil masks, PLAYER required.
        frame.world.cast_shape(
            QueryFilter::layers(physics.ground_and_ceil_mask(), 1 << layer::PLAYER),
            physics.posture_body(POSTURE_SLIDING),
            frame.axis,
            frame.origin,
            frame.velocity * (to_autocrouch * factor),
            0.0,
            &mut hits,
            None,
        );
        let step = physics.posture_settings(POSTURE_SLIDING).step_height;
        !hits.iter().any(|hit| hit.position.z > frame.origin.z + step)
    }

    /// FUN_1407fbb20 including the slipping block (every controller think).
    pub fn controller_support(&mut self, frame: &mut Frame) {
        let was_slip_fall = frame.controller.is_slip_falling;
        frame.controller.is_slip_falling = false;
        let support = frame.physics.ground.state;
        let mut result = None;
        if support != frame.controller.support_state {
            match support {
                SupportState::Unsupported => {
                    if frame.physics.ground_distance >= f32::MAX {
                        frame.controller.support_state = support;
                        result = Some(StateId::FALLING);
                    }
                }
                SupportState::Sliding | SupportState::Supported => {
                    frame.controller.support_state = support;
                    result = Some(StateId::RUNNING);
                }
            }
        }
        let slip_active = self.is_active(StateId::SLIPPING);
        let normal = frame.physics.shortest_ground_ray_normal();
        let thresholds = frame.controller.slipping;
        let slip = self.available(StateId::SLIPPING, frame)
            && normal.z <= thresholds.slope_cos
            && (frame.physics.ground_ray_distance_max > thresholds.min_ground_distance
                || frame.physics.ground_ray_normal_average.z <= thresholds.average_slope_cos);
        let falling = self.is_active(StateId::FALLING);
        let slip_fall = slip && normal.z <= if was_slip_fall { thresholds.fall_stop_cos } else { thresholds.fall_start_cos };
        frame.controller.is_slip_falling = slip_fall;
        if slip_fall {
            frame.controller.support_state = SupportState::Unsupported;
            frame.physics.set_slip();
            if !falling {
                self.request(StateId::FALLING);
            }
            return;
        }
        if slip {
            frame.controller.support_state = SupportState::Sliding;
            if !slip_active {
                self.request(StateId::SLIPPING);
            }
            return;
        }
        if slip_active || was_slip_fall {
            match frame.controller.support_state {
                SupportState::Supported => result = Some(StateId::RUNNING),
                SupportState::Unsupported if !falling => {
                    self.request(StateId::FALLING);
                    return;
                }
                _ => {}
            }
        }
        match result {
            Some(StateId::FALLING) => self.request(StateId::FALLING),
            Some(_) => self.request_landing(),
            None => {}
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
        let mut accepted: Vec<(StateId, Option<f32>)> = Vec::new();
        let mut quits = Vec::new();
        for command in queue {
            match command {
                Command::Quit(id) => {
                    if let Some(position) = accepted.iter().position(|&(a, _)| a == id) {
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
                        self.accept(id, None, &snapshot, &mut accepted, frame);
                    }
                }
                Command::Request(id) => self.accept(id, None, &snapshot, &mut accepted, frame),
                Command::RequestSlide(speed) => self.accept(StateId::SLIDING, Some(speed), &snapshot, &mut accepted, frame),
                Command::RequestCatFall(damage) => self.accept(StateId::CATFALL, Some(damage), &snapshot, &mut accepted, frame),
            }
        }
        for &(id, _) in &accepted {
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
            if accepted.iter().any(|&(r, _)| self.rule(id, r).quit) {
                self.active.remove(position);
                let index = self.index(id).unwrap();
                self.deactivate(index, frame);
            }
        }
        // Every accepted state activates, an already active one (allowSelf)
        // again (vtbl+0x50 / +0x58 are called unconditionally).
        for (id, speed) in accepted {
            let index = self.index(id).unwrap();
            if !self.states[index].active {
                if self.states[index].decl.flags.primary {
                    self.active.insert(0, id);
                } else {
                    self.active.push(id);
                }
            }
            self.activate(index, speed, frame);
        }
        if !self.active.iter().any(|&id| self.state(id).is_some_and(|s| s.decl.flags.primary)) {
            let mut id = self.default_state;
            for _ in 0..8 {
                let Some(index) = self.index(id) else { break };
                if self.can_activate(index, frame) {
                    self.on_request_accepted(index);
                    self.active.insert(0, id);
                    self.activate(index, None, frame);
                    break;
                }
                id = self.states[index].decl.navigation.next_crushed_state;
            }
        }
    }

    fn accept(&self, id: StateId, slide_speed: Option<f32>, snapshot: &[StateId], accepted: &mut Vec<(StateId, Option<f32>)>, frame: &Frame) {
        let Some(index) = self.index(id) else { return };
        let can = match slide_speed {
            Some(speed) if id == StateId::SLIDING => self.slide_available(speed),
            _ => self.can_activate(index, frame),
        };
        let ok = can
            && snapshot.iter().all(|&a| self.rule(a, id).allow)
            && accepted.iter().all(|&(r, _)| self.rule(r, id).allow);
        if ok {
            accepted.retain(|&(r, _)| !self.rule(r, id).quit);
            accepted.push((id, slide_speed));
        }
    }

    /// Slot 8 OnRequestAccepted (InAir FUN_14083eb00, Jumping FUN_1408681a0,
    /// Falling FUN_140865b90, Slipping FUN_140870030).
    fn on_request_accepted(&mut self, index: usize) {
        if matches!(self.states[index].decl.kind, Kind::Slipping(_)) {
            self.states[index].slipping_sneaking = self.is_active(StateId::SNEAKING);
            return;
        }
        if !matches!(self.states[index].decl.kind, Kind::Jumping(_) | Kind::Falling(_)) {
            return;
        }
        let mut in_air = InAir {
            from_sneaking: self.is_active(StateId::SNEAKING),
            from_sprinting: self.is_active(StateId::SPRINTING),
            from_sliding: self.is_active(StateId::SLIDING),
            from_jumping: self.is_active(StateId::JUMPING),
            from_grappling: self.is_active(StateId::GRAPPLE),
            from_vault: self.is_active(StateId::VAULT),
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
                    in_air.from_sliding = jumping.in_air.from_sliding;
                }
            }
            self.states[index].fall.accumulated_height = 0.0;
        }
        self.states[index].in_air = in_air;
    }

    /// Slot 10 Activate (slot 11 with Sliding's params speed).
    fn activate(&mut self, index: usize, slide_speed: Option<f32>, frame: &mut Frame) {
        self.states[index].active = true;
        let decl = self.states[index].decl.clone();
        if let Kind::CatFall(catfall) = &decl.kind {
            // FUN_14085a910 (arms, equipment, the wind blast, fail chance and
            // footsteps are not ported).
            self.states[index].damage = slide_speed.unwrap_or(0.0);
            frame.physics.set_posture(POSTURE_LEAN_FORWARD, 0.0);
            frame.camera.constraints = camera::constraints(catfall.look_at_params);
            frame.camera.set_target_mode(MODE_CATFALL, -1.0);
            let high = self.states[index].damage >= catfall.damage_threshold && catfall.high_damage_midnight.is_some();
            let scene = if high { catfall.high_damage_midnight.as_ref() } else { catfall.low_damage_midnight.as_ref() };
            frame.follow_slope.midnight_requests += 1;
            let rate = frame.attributes.0.get("ARK_ATTRIBUTE_CATFALL_SPEED_FACTOR").copied().unwrap_or(1.0);
            // CatFall 14085b400 casts body and both empty arms (mask 7).
            self.states[index].midnight = Self::play(scene, rate, 7, frame);
            return;
        }
        if let Kind::Mantle(mantle) = &decl.kind {
            self.activate_mantle(index, mantle, frame);
            return;
        }
        if let Kind::Vault(vault) = &decl.kind {
            // FUN_1408715a0.
            let sensor = &*frame.vault_sensor;
            let forward = sensor.axis.x_axis;
            let speed = frame.velocity.length().max(vault.min_speed);
            let height = sensor.vault_height;
            let distance = if height > vault.high_threshold { vault.high_distance } else { vault.small_distance };
            let radius = frame.physics.active_shape().radius;
            self.states[index].vault = VaultRun {
                phase: 0,
                velocity: forward * speed,
                forward,
                interpolate: sensor.vault_position - forward * distance,
                exit: sensor.vault_exit + forward * vault.exit_offset.max(radius),
                height,
            };
            frame.camera.constraints = camera::constraints(vault.constraint_params);
            frame.camera.set_target_mode(MODE_CONSTRAINED, -1.0);
            frame.physics.disable_gravity(true);
            frame.physics.set_posture(POSTURE_VAULTING, 0.0);
            return;
        }
        if let Kind::Leaning(leaning) = decl.kind {
            // FUN_14085d170: continue from the body's (possibly fading) lean.
            let lean = &*frame.lean;
            let state = Leaning {
                up_enabled: true,
                under_control: false,
                left_input: 0.0,
                up_input: 0.0,
                left: lean.left * lean.blend * lean.rotation_blend,
                up: lean.up * lean.blend,
                forward: horizontal_unit(frame.axis.x_axis),
                rotation_blend: 1.0,
                recenter_timer: leaning.recenter_delay,
                exit_timer: leaning.exit_delay,
                sound_active: false,
            };
            frame.lean.set(state.left, state.up, 1.0, frame.lean_sensor);
            self.states[index].leaning = state;
            return;
        }
        // Navigation Activate FUN_1408686e0.
        frame.physics.ground_query_length = decl.navigation.max_ground_distance;
        let navigation = &mut self.states[index].navigation;
        navigation.small_fall_started = false;
        navigation.small_fall_cooldown = frame.small_fall_cooldown;
        // FUN_140869500 (crush test not ported).
        let duration = frame.body.enter(&decl.navigation.loco_stance_name) * decl.navigation.physics_shape_transition_speed_factor;
        frame.physics.set_posture(decl.navigation.posture, duration);
        frame.camera.set_target_mode(decl.navigation.camera_mode, decl.navigation.camera_mode_blend_duration);
        // Slot 8: the head-bob stance, else the loco stance (Navigation Init).
        let bob = if decl.navigation.head_bob_stance_name.is_empty() { &decl.navigation.loco_stance_name } else { &decl.navigation.head_bob_stance_name };
        if !bob.is_empty() {
            frame.body.bob = Some(bob.clone());
        }
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
                // FUN_140864b60 (wallcrouch sensor not ported).
                if frame.physics.ceiling_blocks(0.0) || frame.autocrouch.fires {
                    self.request(StateId::AUTOCROUCH);
                } else {
                    let action = frame.controller.sneak_action;
                    frame.controller.set_toggled(action, true);
                }
                frame.controller.stop_sprint();
                self.states[index].first_frame = true;
            }
            Kind::Sprinting(_) => {
                // FUN_140869f60.
                let normal = frame.physics.ground_ray_normal_average;
                frame.vault_sensor.enable(true, normal);
                frame.controller.must_sprint = true;
                let action = frame.controller.sneak_action;
                frame.controller.set_toggled(action, false);
                self.states[index].sprinting_button_down = frame.controller.down(frame.controller.sprinting_action);
            }
            Kind::Sliding(sliding) => self.activate_slide(index, &sliding, slide_speed, frame),
            Kind::Slipping(_) => {
                // FUN_14086fba0 (grapple inertia not ported).
                self.states[index].slipping_velocity = Vec3::ZERO;
            }
            Kind::AutoCrouch => {
                // FUN_140856e00.
                self.states[index].first_frame = true;
                let action = frame.controller.sneak_action;
                frame.controller.set_toggled(action, true);
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
                // A jump out of a vault adds the obstacle height (+0x10c).
                let height = if in_air.from_vault { basic + frame.vault_sensor.vault_height } else { basic };
                let jump = &mut self.states[index].jump;
                *jump = Jump {
                    basic_height: basic,
                    height,
                    successive: 1,
                    timer: 0.0,
                    takeoff_z: frame.origin.z,
                    apex_z: frame.origin.z,
                    max_velocity: jumping.maximum_velocity,
                    steepness: jumping.steepness,
                    sound_event: jumping.jump_sound_event,
                    leaving_to_falling: false,
                };
                Self::do_jump(jump, frame.physics, &jumping, &mut self.pending_sounds);
                // 0x140867d03.
                frame.jump_anim.start(in_air.from_sneaking, 1.0, false);
            }
            Kind::Falling(falling) => {
                self.activate_in_air(index, frame);
                self.states[index].fall.last_origin = frame.origin;
                // FUN_140865310: a walk-off fall weighs the camera additive by
                // its depth below the feet.
                let in_air = self.states[index].in_air;
                if !in_air.from_jumping {
                    if !frame.controller.is_slip_falling {
                        let weight = (frame.physics.ground_ray_distance_max / falling.full_fall_anim_height).min(1.0);
                        frame.jump_anim.start(in_air.from_sneaking, weight, false);
                    }
                } else {
                    frame.jump_anim.stop_at_loop = false;
                }
            }
            // Activated above, without the navigation base.
            Kind::Leaning(_) | Kind::CatFall(_) | Kind::Mantle(_) | Kind::Vault(_) => {}
        }
    }

    /// Sliding Activate FUN_14086d670 / FUN_14086d960 and Init FUN_14086db40.
    fn activate_slide(&mut self, index: usize, decl: &SlidingDecl, speed: Option<f32>, frame: &mut Frame) {
        let normal = frame.physics.ground_ray_normal_average;
        frame.vault_sensor.enable(true, normal);

        frame.camera.constraints = super::camera::constraints(decl.constraint_params);
        // FollowSlope tilts the body to the slope while sliding (unless the
        // state machine's +0x1c bit 30 is set, which the player never sets).
        frame.follow_slope.requests += 1;
        let action = frame.controller.sneak_action;
        frame.controller.set_toggled(action, false);
        frame.controller.stop_sprint();
        frame.controller.must_sprint = false;
        let (standing, sliding) = (self.stance(StateId::SPRINTING).to_owned(), self.stance(StateId::SLIDING).to_owned());
        let sneaking = self.stance(StateId::SNEAKING).to_owned();
        let autocrouch = self.stance(StateId::AUTOCROUCH).to_owned();
        let horizontal = Vec3::new(frame.velocity.x, frame.velocity.y, 0.0);
        let start = speed.unwrap_or(horizontal.length()).min(decl.in_speed);
        let t = (decl.in_speed - start) / (decl.in_speed - decl.out_speed);
        let direction = horizontal.normalize_or_zero();
        let slide = Slide {
            from_standing: frame.body.duration(&standing, &sliding),
            to_sneaking: frame.body.duration(&sliding, &sneaking),
            to_autocrouch: frame.body.duration(&sliding, &autocrouch),
            to_standing: frame.body.duration(&sliding, &standing),
            blend_in_timer: frame.body.duration(&standing, &sliding),
            direction,
            last_direction: direction,
            max_distance: decl.distance,
            distance: decl.distance * t,
            max_forward_cos: cos_snap(decl.max_forward_angle),
            max_wall_cos: cos_snap(decl.max_wall_angle),
            speed: start,
            blend_out_state: None,
            blend_out_done: false,
            keep_sprinting: true,
            want_jump: false,
        };
        frame.autocrouch.set_detection_distance(slide.max_distance - slide.distance);
        self.states[index].slide = slide;
    }

    /// InAir Activate FUN_14083e870.
    fn activate_in_air(&mut self, index: usize, frame: &Frame) {
        self.states[index].in_air.inherited_velocity = Vec3::new(frame.velocity.x, frame.velocity.y, 0.0);
    }

    /// FUN_1408680a0.
    /// FUN_1408680a0 posts the current +0xb0 cue, decrements the remaining
    /// impulses, then replaces that cue with m_doubleJumpSoundEvent if one remains.
    fn do_jump(jump: &mut Jump, physics: &mut PlayerPhysics, decl: &JumpingDecl, sounds: &mut Vec<u32>) {
        if let Some(event) = jump.sound_event { sounds.push(event); }
        jump.apex_z += jump.height;
        physics.start_vertical(jump.apex_z, jump.max_velocity, jump.steepness);
        jump.successive -= 1;
        if jump.successive > 0 {
            jump.sound_event = decl.double_jump_sound_event;
        }
    }

    /// Slot 12 Deactivate.
    fn deactivate(&mut self, index: usize, frame: &mut Frame) {
        self.states[index].active = false;
        match self.states[index].decl.kind.clone() {
            Kind::CatFall(_) => {
                // FUN_14085b1e0 (the scene keeps playing).
                let crouch = frame.controller.toggled(frame.controller.sneak_action);
                self.request(if crouch { StateId::SNEAKING } else { StateId::RUNNING });
                frame.follow_slope.midnight_requests -= 1;
                return;
            }
            Kind::Mantle(_) => {
                // FUN_14085fd40.
                set_mantle_collision(frame.physics, self.states[index].mantle.destination_body, true);
                frame.controller.support_state = SupportState::Supported;
                frame.physics.disable_gravity(false);
                frame.body.bob_enabled = true;
                frame.body.max_speeds = [UNSET; 4];
                if frame.retargeting.take().is_some() {
                    frame.physics.skip_inertia(false);
                    frame.physics.disable_gravity(false);
                }
                frame.midnight.stop(self.states[index].midnight);
                self.states[index].midnight = 0;
                return;
            }
            Kind::Vault(_) => {
                // FUN_140871d90.
                frame.physics.set_posture(POSTURE_STANDING, 0.0);
                if self.states[index].vault.phase != 2 {
                    frame.physics.disable_gravity(false);
                }
                if self.states[index].midnight != 0 {
                    frame.midnight.stop(self.states[index].midnight);
                    self.states[index].midnight = 0;
                }
                if frame.physics.ground_distance < 0.0 {
                    frame.physics.proxy.position.z -= frame.physics.ground_distance;
                }
                return;
            }
            _ => {}
        }
        if let Kind::Leaning(leaning) = self.states[index].decl.kind {
            // FUN_14085d490.
            if self.states[index].leaning.sound_active {
                if let Some(event) = leaning.stop_sound_event { self.pending_sounds.push(event); }
                self.states[index].leaning.sound_active = false;
            }
            frame.lean.end(leaning.blend_duration);
            return;
        }
        // Navigation Deactivate FUN_140868d70.
        *frame.footstep = super::footsteps::Footstep { step_length: 0.0, ..Default::default() };
        frame.footstep_clock.configure(frame.footstep, self.states[index].decl.id);
        frame.body.max_speeds = [UNSET; 4];
        if self.states[index].navigation.small_fall_started {
            frame.jump_anim.end(false, true);
        }
        match self.states[index].decl.kind {
            Kind::Jumping(_) => {
                // FUN_140867ef0.
                if frame.physics.vertical_active() {
                    frame.physics.stop_vertical();
                }
                if !self.states[index].jump.leaving_to_falling {
                    let crouch = frame.jump_anim.crouch();
                    frame.jump_anim.end(crouch, false);
                } else {
                    frame.jump_anim.stop_at_loop = true;
                }
                self.states[index].in_air = InAir::default();
            }
            Kind::Falling(_) => {
                // FUN_140865630.
                if frame.jump_anim.stage & !4 != 0 {
                    let crouch = frame.jump_anim.crouch();
                    frame.jump_anim.end(crouch, false);
                }
                frame.physics.ground_query_length = self.states[index].decl.navigation.max_ground_distance;
                self.states[index].in_air = InAir::default();
            }
            Kind::Sprinting(_) => {
                // FUN_14086a220.
                let normal = frame.physics.ground_ray_normal_average;
                frame.vault_sensor.enable(false, normal);
            }
            Kind::Sliding(_) => {
                // FUN_14086e050.
                let normal = frame.physics.ground_ray_normal_average;
                frame.vault_sensor.enable(false, normal);
                if frame.follow_slope.requests > 0 {
                    frame.follow_slope.requests -= 1;
                }
                frame.autocrouch.set_detection_distance(0.0);
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
                // FUN_140865050 (wallcrouch sensor not ported).
                self.states[index].navigation.speed_multiplier = frame.attributes.get("ARK_ATTRIBUTE_SPEED_MULTIPLIER")
                    * frame.attributes.get("ARK_ATTRIBUTE_MOVE_SPEED_SNEAKING_FACTOR");
                self.navigation_update(index, frame);
                let stand = frame.physics.posture_settings(POSTURE_STANDING).height;
                let sneak = frame.physics.posture_settings(POSTURE_SNEAKING).height;
                if frame.autocrouch.fires {
                    self.request(StateId::AUTOCROUCH);
                } else if !self.states[index].first_frame
                    && frame.physics.free_to_stand(stand - sneak)
                    && !frame.controller.toggled(frame.controller.sneak_action)
                    && self.available(StateId::RUNNING, frame)
                {
                    self.quit(id);
                    self.request(StateId::RUNNING);
                }
                self.states[index].first_frame = false;
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
            Kind::Sliding(decl) => self.sliding_update(index, &decl, frame),
            Kind::Slipping(decl) => self.slipping_update(index, &decl, frame),
            Kind::AutoCrouch => {
                // FUN_1408570c0.
                self.navigation_update(index, frame);
                let sneak = frame.physics.posture_settings(POSTURE_SNEAKING).height;
                let crouch = frame.physics.posture_settings(POSTURE_AUTOCROUCHING).height;
                if !self.states[index].first_frame
                    && frame.physics.free_to_stand(sneak - crouch)
                    && !frame.autocrouch.fires
                    && self.available(StateId::SNEAKING, frame)
                {
                    self.quit(id);
                    self.request(StateId::SNEAKING);
                }
                self.states[index].first_frame = false;
            }
            Kind::Jumping(decl) => {
                self.in_air_update(index, frame);
                self.jumping_update(index, &decl, frame);
            }
            Kind::Falling(decl) => {
                self.in_air_update(index, frame);
                self.falling_update(index, &decl, frame);
            }
            Kind::Leaning(decl) => self.leaning_update(index, &decl, frame),
            Kind::CatFall(_) => {
                // FUN_14085b8a0.
                frame.requests.push_angular(0, Vec3::ZERO);
                frame.requests.push_linear(0, Vec3::ZERO);
                if frame.midnight.finished(self.states[index].midnight) {
                    self.quit(id);
                }
            }
            Kind::Mantle(_) => self.mantle_update(index, frame),
            Kind::Vault(decl) => self.vault_update(index, &decl, frame),
        }
    }

    /// Mantle Activate FUN_14085eaf0 (shadow walk, corpses, moving
    /// platforms, legs, footsteps and equipment are not ported).
    fn activate_mantle(&mut self, index: usize, decl: &MantleStateDecl, frame: &mut Frame) {
        let sensor = &*frame.mantle_sensor;
        let Some((final_posture, final_state)) = sensor.final_posture() else { return };
        let height = sensor.height(frame.origin);
        let destination = sensor.destination;
        let axis = sensor.destination_axis.x_axis;
        let destination_yaw = axis.y.atan2(axis.x);
        frame.requests.push_angular(0, Vec3::ZERO);
        let mut final_state = StateId(final_state);
        if final_state != StateId::AUTOCROUCH && frame.controller.toggled(frame.controller.sneak_action) {
            final_state = StateId::SNEAKING;
        }
        if height != MantleHeight::Low {
            let stance = self.stance(final_state).to_owned();
            frame.body.enter(&stance);
        }
        frame.body.bob_enabled = false;
        frame.body.max_speeds = [0.0; 4];
        let slot = if final_state == StateId::SNEAKING { 1 } else if final_state == StateId::AUTOCROUCH { 0 } else { 2 };
        let scenes = match height {
            MantleHeight::Low => &decl.low_midnights,
            MantleHeight::Mid => &decl.mid_midnights,
            MantleHeight::High => &decl.high_midnights,
        };
        let destination_body = sensor.destination_body;
        self.states[index].mantle = MantleRun { final_posture, final_state: final_state.0, destination, destination_yaw, height: height as u8, destination_body };
        if scenes[slot].is_some() {
            let rate = frame.attributes.0.get("ARK_ATTRIBUTE_MANTLE_SPEED_FACTOR").copied().unwrap_or(1.0);
            // Mantle casting 140860030 uses the sensor's hand flags. The
            // corpse/equipment branch is absent while both hands are empty.
            let parts = 1 | if sensor.use_left_hand { 2 } else { 0 } | if sensor.use_right_hand { 4 } else { 0 };
            self.states[index].midnight = Self::play(scenes[slot].as_ref(), rate, parts, frame);
            // FUN_140860680: retarget the player onto the dummy at the destination.
            let low = height == MantleHeight::Low;
            *frame.retargeting = Some(Retargeting {
                // 0.2 (DAT_141ab3818).
                duration: if low { decl.low_mantle_duration } else { 0.2 },
                timer: 0.0,
                anchor: !low,
                orientation: !low,
                start: frame.origin,
                start_yaw: frame.physics.yaw,
                target: destination,
                target_yaw: destination_yaw,
                last_clip: None,
            });
            // FUN_140757730: no smoothing; axis z disables gravity again.
            frame.physics.skip_inertia(true);
            frame.physics.disable_gravity(true);
        }
        frame.physics.instant_motion();
        frame.physics.set_gravity_velocity(Vec3::ZERO);
        let posture = if final_posture == POSTURE_AUTOCROUCHING { POSTURE_AUTOCROUCHING } else { POSTURE_SNEAKING };
        frame.physics.set_posture(posture, 0.0);
        frame.physics.disable_gravity(true);
        // The level (idWorldspawn) drops WORLD_LOW and WORLD_LOW_HIGH from
        // the proxy for the move; a rigid body only loses its pair.
        set_mantle_collision(frame.physics, destination_body, false);
        if height == MantleHeight::Low {
            frame.camera.constraints = camera::constraints(decl.constraint_params);
            frame.camera.set_target_mode(MODE_CONSTRAINED_WITH_ARMS, -1.0);
        } else {
            frame.camera.set_target_mode(MODE_ANIMATED, -1.0);
        }
    }

    /// Mantle Update FUN_1408607f0.
    fn mantle_update(&mut self, index: usize, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let run = self.states[index].mantle;
        // FUN_14085fa90: the AUTOCROUCHING shape overlapping at the dummy.
        let (mut cast, mut start) = (Vec::new(), Vec::new());
        let rotation = Mat3::from_rotation_z(run.destination_yaw);
        let at = run.destination + Vec3::Z * 0.01;
        frame.world.cast_shape(QueryFilter::layers(0x802b, 1 << layer::PLAYER), frame.physics.posture_body(POSTURE_AUTOCROUCHING), rotation, at, Vec3::ZERO, 0.001, &mut cast, Some(&mut start));
        let overlap = start.iter().any(|h| h.value <= 0.0);
        let mut final_state = StateId(run.final_state);
        if overlap {
            final_state = StateId::AUTOCROUCH;
            self.states[index].mantle.final_state = final_state.0;
            frame.physics.set_posture(POSTURE_AUTOCROUCHING, 0.0);
        }
        let handle = self.states[index].midnight;
        if !overlap && handle != 0 && !frame.midnight.finished(handle) {
            return;
        }
        if final_state == StateId::AUTOCROUCH {
            let action = frame.controller.sneak_action;
            frame.controller.set_toggled(action, false);
        }
        self.quit(id);
        self.request(final_state);
    }

    /// Vault Update FUN_1408725d0 (breakables, NPC damage, the left-arm
    /// scenes and window scenes need entities the static world lacks).
    fn vault_update(&mut self, index: usize, decl: &VaultDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let action = frame.controller.sneak_action;
        frame.controller.set_toggled(action, false);
        let run = self.states[index].vault;
        match run.phase {
            0 => {
                frame.requests.push_angular(0, Vec3::ZERO);
                frame.requests.push_linear(0, run.velocity);
                let lost = !frame.physics.near_ground();
                if lost || ((frame.origin - run.interpolate).dot(run.forward) >= 0.0 && !self.start_vault(index, decl, frame)) {
                    self.quit(id);
                    self.request(StateId::SPRINTING);
                }
            }
            1 => {
                frame.requests.push_angular(0, Vec3::ZERO);
                frame.requests.push_linear(0, run.velocity);
                if (frame.origin - run.exit).dot(run.forward) >= 0.0 {
                    frame.physics.set_posture(POSTURE_STANDING, 0.0);
                    frame.physics.disable_gravity(false);
                    // The scene keeps playing untracked.
                    self.states[index].midnight = 0;
                    self.states[index].vault.phase = 2;
                }
            }
            _ => {
                let keep = self.state(StateId::SPRINTING).and_then(|s| match &s.decl.kind {
                    Kind::Sprinting(d) => Some(can_keep_sprinting(d, frame)),
                    _ => None,
                }) == Some(true);
                frame.requests.push_angular(0, Vec3::ZERO);
                frame.requests.push_linear(0, if keep { run.velocity } else { Vec3::ZERO });
                if frame.physics.near_ground() {
                    if frame.controller.support_state == SupportState::Supported {
                        self.quit(id);
                        let speed = run.velocity.truncate().length();
                        if frame.controller.down(frame.controller.slide_action) && self.slide_available(speed) {
                            self.queue.push(Command::RequestSlide(speed));
                        } else {
                            if !keep {
                                frame.controller.must_sprint = false;
                                frame.controller.stop_sprint();
                            }
                            self.request(if keep { StateId::SPRINTING } else { StateId::RUNNING });
                        }
                    }
                } else {
                    // FUN_1407fbb20(ctrl, force).
                    self.quit(id);
                    frame.controller.support_state = SupportState::Unsupported;
                    self.request(StateId::FALLING);
                }
            }
        }
    }

    /// FUN_140872490 with the scene choice FUN_140872080 (the game's LCG).
    fn start_vault(&mut self, index: usize, decl: &VaultDecl, frame: &mut Frame) -> bool {
        let high = self.states[index].vault.height > decl.high_threshold;
        let list = if high || decl.small_midnights.is_empty() { &decl.high_midnights } else { &decl.small_midnights };
        if list.is_empty() {
            return false;
        }
        *frame.random = frame.random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
        let pick = ((*frame.random >> 10) & 0x7fff) as usize % list.len();
        // Vault 140872490 casts all three parts with no carried equipment.
        let handle = Self::play(Some(&list[pick]), 1.0, 7, frame);
        if handle == 0 {
            return false;
        }
        self.states[index].midnight = handle;
        self.states[index].vault.phase = 1;
        true
    }

    /// Leaning Update FUN_14085dcc0.
    fn leaning_update(&mut self, index: usize, decl: &LeaningDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let dt = frame.dt;
        let controller = &*frame.controller;
        let (mut forward, mut lateral) = (controller.input_move.x, controller.input_move.y);
        if forward.abs() <= lateral.abs() {
            forward = 0.0;
        } else {
            lateral = 0.0;
        }
        let lean_button = controller.lean_action;
        let (left_down, right_down) = (controller.down(decl.lean_left_button), controller.down(decl.lean_right_button));
        let lean_down = controller.down(lean_button);
        let any_held = lean_down || left_down || right_down;
        let (up_down, down_down) = (controller.down(decl.lean_up_button), controller.down(decl.lean_down_button));
        let mut st = self.states[index].leaning;
        let up_allowed0 = st.up_enabled
            && !frame.physics.ceiling_blocks(frame.lean_sensor.decl.lean_up_amplitude)
            && !self.is_active(StateId::AUTOCROUCH);
        let (dz_up, dz_right) = (dead_zone(forward, decl), dead_zone(-lateral, decl));
        let keyboard = frame.keyboard;
        let hold_mode = keyboard && !frame.lean_toggle;
        let (up_allowed, want_left, want_right, want_up, want_down) = if hold_mode {
            // Holding only LEAN recenters a lateral lean.
            let recent_left = st.left > 0.0 && !right_down && !left_down;
            let recent_right = st.left < 0.0 && !right_down && !left_down;
            (up_allowed0, (any_held && left_down) || recent_left, (any_held && right_down) || recent_right, up_allowed0 && lean_down, up_allowed0 && !lean_down)
        } else {
            let up_allowed = any_held && up_allowed0;
            (up_allowed, any_held && left_down, any_held && right_down, up_allowed && up_down, up_allowed && down_down)
        };
        // Exit on speed or on turning away from the activation forward.
        let c = horizontal_unit(frame.axis.x_axis).dot(st.forward);
        let angle = if c > -1.0 { if c < 1.0 { c.acos() } else { 0.0 } } else { std::f32::consts::PI } * 57.29578;
        let leaving = decl.maximum_linear_speed * decl.maximum_linear_speed < frame.velocity.x * frame.velocity.x + frame.velocity.y * frame.velocity.y
            || decl.maximum_rotation_angle <= angle;
        let mut check_exit = true;
        if !hold_mode {
            if any_held {
                // The keys or the stick drive the lean, not the legs.
                frame.requests.push_linear(2, Vec3::ZERO);
                st.exit_timer = decl.exit_delay;
                check_exit = false;
            }
        } else if !any_held {
            self.quit(id);
            self.states[index].leaning = st;
            return;
        }
        if check_exit {
            if leaving {
                let t = st.exit_timer;
                st.exit_timer -= dt;
                if t < 0.0 {
                    self.quit(id);
                    self.states[index].leaning = st;
                    return;
                }
            } else {
                st.exit_timer = decl.exit_delay;
            }
        }
        let (mut target_left, mut target_up) = if !keyboard {
            (dz_right, if up_allowed { dz_up } else { 0.0 })
        } else {
            let left = if want_left { -1.0 } else { 0.0 } + if want_right { 1.0 } else { 0.0 };
            let up = if up_allowed { want_up as i32 as f32 - want_down as i32 as f32 } else { 0.0 };
            (left, up)
        };
        let lean = &*frame.lean;
        let current_left = if st.rotation_blend > 0.0 { lean.blend * lean.left * lean.rotation_blend } else { 0.0 };
        let current_up = lean.blend * lean.up;
        // Reversing goes to the full opposite input.
        if current_left > 0.0 && target_left < 0.0 {
            target_left = -1.0;
        } else if current_left < 0.0 && target_left > 0.0 {
            target_left = 1.0;
        }
        if current_up > 0.0 && target_up < 0.0 {
            target_up = -1.0;
        }
        if target_left != 0.0 {
            st.left_input = latch(st.left_input, target_left);
        }
        if target_up != 0.0 {
            st.up_input = latch(st.up_input, target_up);
        }
        let has_input = st.left_input != 0.0 || st.up_input != 0.0;
        st.under_control |= has_input;
        let rotation = ((angle - decl.minimum_rotation_angle) / (decl.maximum_rotation_angle - decl.minimum_rotation_angle)).clamp(0.0, 1.0);
        st.rotation_blend = st.rotation_blend.min(1.0 - rotation);
        let mut step_left = dt * if keyboard { decl.lean_speed_keyboard } else { decl.lean_speed_gamepad };
        let mut step_up = dt * if keyboard { decl.lean_up_speed_keyboard } else { decl.lean_up_speed_gamepad };
        let (mut new_left, new_up);
        let threshold = decl.recenter_threshold;
        if !st.under_control || (!has_input && current_up * current_up + current_left * current_left < threshold * threshold) {
            // Recenter, from 0.1 s after Activate (the timer is never reset).
            new_left = current_left;
            let mut up = current_up;
            if st.recenter_timer <= 0.0 {
                if st.left_input != 0.0 {
                    step_left *= st.left_input.abs();
                }
                new_left = if current_left > step_left {
                    current_left - step_left
                } else if current_left >= -step_left {
                    st.left_input = 0.0;
                    0.0
                } else {
                    current_left + step_left
                };
                if st.up_input != 0.0 {
                    step_up *= st.up_input.abs();
                }
                up = if current_up > step_up {
                    current_up - step_up
                } else {
                    st.up_input = 0.0;
                    0.0
                };
                if new_left == 0.0 && up == 0.0 && !any_held {
                    self.quit(id);
                }
            }
            new_up = up;
        } else {
            new_left = current_left + step_left * st.left_input;
            if !frame.lean_sensor.clamp(&mut new_left) {
                st.left_input = 0.0;
            }
            new_up = (current_up + step_up * st.up_input).clamp(0.0, 1.0);
        }
        // Crossing the centre stops there.
        if (st.left < 0.0 && new_left > 0.0) || (st.left > 0.0 && new_left < 0.0) {
            st.left_input = 0.0;
            new_left = 0.0;
        }
        st.recenter_timer -= dt;
        let motion = (st.left - new_left).abs().max((st.up - new_up).abs());
        st.left = new_left;
        st.up = new_up;
        frame.lean.set(new_left, new_up, st.rotation_blend, frame.lean_sensor);
        if !st.sound_active && motion > 0.0 {
            if let Some(event) = decl.start_sound_event { self.pending_sounds.push(event); }
            st.sound_active = true;
        } else if st.sound_active && motion <= 0.0 {
            if let Some(event) = decl.stop_sound_event { self.pending_sounds.push(event); }
            st.sound_active = false;
        }
        self.states[index].leaning = st;
    }


    /// Sliding Update FUN_14086ef50 (wallcrouch, vault and slow-time are not
    /// ported).
    fn sliding_update(&mut self, index: usize, decl: &SlidingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        self.navigation_update(index, frame);
        frame.body.max_speeds = [decl.in_speed; 4];
        let factor = frame.attributes.0.get("ARK_ATTRIBUTE_MOVE_SPEED_SLIDING_FACTOR").copied().unwrap_or(1.0);
        self.states[index].navigation.speed_multiplier = factor;
        let velocity = frame.velocity;
        let inverse = 1.0 / velocity.truncate().length_squared().max(f32::MIN_POSITIVE).sqrt();
        let slope_distance = self.slope_distance(decl, frame);
        let ac_hit = frame.autocrouch.fires;
        let mut slide = self.states[index].slide;
        slide.max_distance = slide.max_distance.min(slope_distance);
        slide.want_jump |= frame.controller.pressed(ACTION_JUMP);
        let mut next = None;
        let sneak_available = self.available(StateId::SNEAKING, frame);
        if slide.want_jump && slide.blend_in_timer <= 0.0 && slide.blend_out_done {
            next = Some(StateId::JUMPING);
        } else if slide.speed < decl.out_speed
            || (velocity.truncate() * inverse).dot(slide.last_direction.truncate()) < slide.max_wall_cos
            || slide.max_distance <= slide.distance
        {
            next = Some(if !sneak_available || ac_hit { StateId::AUTOCROUCH } else { StateId::SNEAKING });
        } else {
            slide.distance += slide.speed * frame.dt;
            slide.speed = lerp(decl.in_speed, decl.out_speed, slide.distance / slide.max_distance);
            let navigation = self.states[index].decl.navigation.clone();
            let mut steer = self.move_velocity(index, frame, navigation.max_forward_speed, navigation.max_backward_speed, navigation.max_strafe_speed);
            if steer.length_squared() > 1e-4 {
                steer *= 1.0 - (steer / steer.length()).dot(slide.direction).abs();
            }
            let w = (slide.direction * slide.speed + steer).truncate();
            let pushed = (w / w.length_squared().max(f32::MIN_POSITIVE).sqrt() * slide.speed * factor).extend(0.0);
            slide.last_direction = pushed.normalize_or_zero();
            frame.requests.push_linear(0, pushed);
            if !slide.blend_out_done {
                let settings = *frame.physics.posture_settings(POSTURE_SLIDING);
                if frame.physics.posture == POSTURE_SLIDING
                    && ac_hit
                    && frame.autocrouch.hole_depth <= settings.extrude_length + settings.radius
                    && frame.autocrouch.sneak_distance <= settings.extrude_length + settings.radius
                {
                    if let Some(posture) = self.state(StateId::AUTOCROUCH).map(|s| s.decl.navigation.posture) {
                        frame.physics.set_posture(posture, 0.0);
                    }
                }
                let time_left = (slide.max_distance - slide.distance) * 2.0 / (slide.speed + decl.out_speed);
                let candidate = if slide.want_jump {
                    Some((StateId::RUNNING, slide.to_standing))
                } else if !sneak_available {
                    ac_hit.then_some((StateId::AUTOCROUCH, slide.to_autocrouch)).filter(|&(_, d)| d >= time_left)
                } else if ac_hit {
                    Some((StateId::AUTOCROUCH, slide.to_autocrouch)).filter(|&(_, d)| d >= time_left)
                } else {
                    Some((StateId::SNEAKING, slide.to_sneaking)).filter(|&(_, d)| d >= time_left)
                };
                if let Some((state, duration)) = candidate {
                    slide.blend_out_state = Some(state);
                    slide.blend_out_done = true;
                    if !slide.keep_sprinting || state != StateId::SNEAKING {
                        if let Some(target) = self.state(state) {
                            let (posture, stance) = (target.decl.navigation.posture, target.decl.navigation.loco_stance_name.clone());
                            let camera_mode = target.decl.navigation.camera_mode;
                            frame.physics.set_posture(posture, duration);
                            frame.body.enter(&stance);
                            frame.camera.set_target_mode(camera_mode, -1.0);
                            frame.follow_slope.requests -= 1;
                        }

                    }
                }
            }
        }
        if !frame.physics.near_ground() {
            next = Some(StateId::FALLING);
        } else if next.is_none() && slide.blend_out_state == Some(StateId::SNEAKING) && ac_hit {
            next = Some(StateId::AUTOCROUCH);
        }
        if slide.blend_in_timer >= 0.0 {
            slide.blend_in_timer -= frame.dt;
        }
        if slide.keep_sprinting {
            if frame.controller.input_move.truncate().length_squared() < 0.25 {
                slide.keep_sprinting = false;
            }
            if slide.keep_sprinting && slide.blend_out_done && next.is_none() && slide.blend_out_state == Some(StateId::SNEAKING) {
                next = Some(StateId::SNEAKING);
            }
        }
        if let Some(next) = next {
            if !slide.blend_out_done {
                slide.blend_out_state = Some(next);
            }
            self.quit(id);
            if slide.keep_sprinting && next == StateId::SNEAKING && self.available(StateId::SPRINTING, frame) {
                frame.controller.resume_sprint();
                self.request(StateId::SPRINTING);
            } else {
                frame.controller.must_sprint = false;
                frame.controller.stop_sprint();
                self.request(next);
            }
        }
        let remaining = (slide.max_distance - slide.distance).max(0.0);
        frame.autocrouch.set_detection_distance(remaining);
        self.states[index].slide = slide;
    }

    /// FUN_14086ddf0: facing uphill shortens the slide.
    fn slope_distance(&self, decl: &SlidingDecl, frame: &Frame) -> f32 {
        let mut distance = decl.distance;
        let normal = frame.physics.ground_ray_normal_average;
        if decl.max_slope_angle > 0.0 && frame.axis.x_axis.dot(normal) < 0.0 {
            let hi = decl.max_slope_angle * 0.017453292;
            let lo = decl.min_slope_angle * 0.017453292;
            let angle = slope_of(normal.z).min(hi);
            if angle >= lo && (hi - lo).abs() >= f32::MIN_POSITIVE {
                distance -= (angle - lo) / (hi - lo) * decl.distance * decl.slope_influence;
            }
        }
        distance
    }

    /// Slipping Update FUN_1408700a0.
    fn slipping_update(&mut self, index: usize, decl: &SlippingDecl, frame: &mut Frame) {
        let speed = frame.velocity.length();
        let state = &mut self.states[index];
        if state.slipping_velocity != Vec3::ZERO {
            state.slipping_velocity = state.slipping_velocity.normalize() * speed;
        }
        let normal = frame.physics.shortest_ground_ray_normal();
        let gravity = super::physics::GRAVITY;
        state.slipping_velocity += (gravity - normal * normal.dot(gravity)) * (decl.gravity_strength * frame.dt);
        let crouch = frame.controller.toggled(frame.controller.sneak_action);
        let sneaking = state.slipping_sneaking;
        if sneaking && !crouch && frame.physics.can_use_posture(POSTURE_STANDING) {
            let stance = self.stance(StateId::RUNNING).to_owned();
            frame.body.enter(&stance);
        } else if !sneaking && crouch {
            let stance = self.stance(StateId::SNEAKING).to_owned();
            frame.body.enter(&stance);
        }
        self.states[index].slipping_sneaking = crouch;
        self.navigation_update(index, frame);
    }

    /// ComputeMoveVelocity (slot 24, FUN_140868a80; Slipping FUN_14086fd00).
    fn move_velocity(&self, index: usize, frame: &Frame, forward: f32, backward: f32, strafe: f32) -> Vec3 {
        let state = &self.states[index];
        let navigation = &state.navigation;
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
        if matches!(state.decl.kind, Kind::Slipping(_)) {
            let normal = frame.physics.shortest_ground_ray_normal();
            velocity -= normal * normal.dot(velocity);
            let k = (state.slipping_velocity.normalize_or_zero().dot(velocity.normalize_or_zero()) + 1.0) * 0.5;
            velocity = velocity * k + state.slipping_velocity;
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
        frame.body.max_speeds = [decl.max_backward_speed, decl.max_strafe_speed, decl.max_forward_speed, decl.max_strafe_speed];
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
        let turn = &mut frame.camera.turn;
        turn.turn_speed_mouse = mouse;
        turn.turn_speed_joystick = joystick * turn.pitch_speed_ratio;
        frame.requests.push_linear(0, velocity);
        frame.requests.push_angular(0, Vec3::new(0.0, 0.0, yaw_rate));
        *frame.footstep = super::footsteps::navigation_step(&decl, frame.physics.ground.normal.dot(frame.axis.x_axis), frame.body_speed);
        let id = self.states[index].decl.id;
        frame.footstep_clock.configure(frame.footstep, id);
        let navigation = &mut self.states[index].navigation;
        // The small-fall camera additive (0x140869b9c / 0x140869bbe).
        if decl.small_fall_min_height > 0.0 && navigation.small_fall_cooldown <= 0.0 {
            let crouch = matches!(id, StateId::SNEAKING | StateId::AUTOCROUCH | StateId::WALLCROUCH);
            if !navigation.small_fall_started {
                if decl.small_fall_min_height <= frame.physics.ground_distance
                    && frame.physics.gravity_velocity.z < -1.0
                    && frame.jump_anim.stage == 0
                {
                    frame.jump_anim.start(crouch, decl.small_fall_anim_weight, false);
                    navigation.small_fall_started = true;
                }
            } else if frame.jump_anim.stage == 3 {
                frame.jump_anim.end(crouch, false);
                navigation.small_fall_started = false;
                navigation.small_fall_cooldown = frame.small_fall_cooldown;
            }
        }
        navigation.small_fall_cooldown -= frame.dt;
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
        frame.requests.push_linear(0, combined);
    }

    /// Jumping Update FUN_140868200.
    fn jumping_update(&mut self, index: usize, decl: &JumpingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let jump_held = frame.controller.down(ACTION_JUMP);
        let crouch = frame.controller.toggled(frame.controller.sneak_action);
        let from = self.states[index].in_air;
        let jump = self.states[index].jump;
        if !frame.physics.vertical_active() {
            if frame.physics.ground.state == SupportState::Unsupported {
                if jump.successive > 0 && jump_held {
                    let jump = &mut self.states[index].jump;
                    Self::do_jump(jump, frame.physics, decl, &mut self.pending_sounds);
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

    /// Falling Update FUN_140865c50 (landing without catfall and damage).
    fn falling_update(&mut self, index: usize, decl: &FallingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let fall = &mut self.states[index].fall;
        fall.accumulated_height += fall.last_origin.z - frame.origin.z;
        fall.last_origin = frame.origin;
        let accumulated = fall.accumulated_height;
        let from = self.states[index].in_air;
        if (frame.physics.ground_distance == f32::MAX && frame.physics.ground.state == SupportState::Unsupported)
            || frame.controller.is_slip_falling
        {
            if from.from_jumping && frame.physics.gravity_velocity.z > 0.0 {
                let mut velocity = frame.physics.gravity_velocity;
                velocity.z *= decl.smooth_gravity_adjustment;
                frame.physics.set_gravity_velocity(velocity);
            }
            // pp+0xc0 is the speed scale (1 unless scaled).
            frame.physics.ground_query_length = frame.velocity.z.abs() * frame.dt + self.states[index].decl.navigation.max_ground_distance;
            return;
        }
        // Land() FUN_1408658c0: fall damage, then CATFALL above
        // m_catFallHeight. Health and death are not ported: no fall kills.
        let damage = if decl.fall_damage { fall_damage(decl, accumulated, frame) } else { 0.0 };
        self.states[index].fall.accumulated_height = 0.0;
        if accumulated > decl.cat_fall_height && self.available(StateId::CATFALL, frame) {
            self.quit(id);
            self.queue.push(Command::RequestCatFall(damage));
            return;
        }
        self.quit(id);
        let slide_speed = frame.velocity.truncate().length();
        let crouch_action = frame.controller.sneak_action;
        if from.from_sliding && !from.from_jumping && accumulated < decl.resume_sliding_max_height && self.slide_available(slide_speed) {
            self.queue.push(Command::RequestSlide(slide_speed));
        } else if ((from.from_jumping && from.from_sprinting) || from.from_grappling)
            && frame.controller.down(crouch_action)
            && self.available(StateId::SLIDING, frame)
        {
            frame.controller.stop_sprint();
            frame.controller.set_toggled(crouch_action, false);
            self.request(StateId::SLIDING);
        } else if frame.controller.toggled(crouch_action) {
            self.request(StateId::SNEAKING);
        } else if from.from_sprinting {
            self.request(StateId::SPRINTING);
        } else {
            self.request(StateId::RUNNING);
        }
        // The landing camera additive.
        let crouch = frame.controller.toggled(crouch_action);
        frame.jump_anim.end(crouch, false);
    }
}

/// FUN_1408657c0 (no enhanced heights: the reduction attribute keeps its
/// base value without modifiers).
fn fall_damage(decl: &FallingDecl, height: f32, frame: &Frame) -> f32 {
    let (d, death) = (decl.damage_height, decl.death_height);
    if height - d < death {
        let factor = frame.attributes.0.get("ARK_ATTRIBUTE_FALL_DAMAGE_FACTOR").copied().unwrap_or(1.0);
        factor * frame.health * ((height - d) / (death - d))
    } else {
        f32::MAX
    }
}

/// FUN_14086a0f0.
fn can_keep_sprinting(decl: &SprintingDecl, frame: &Frame) -> bool {
    let input = frame.controller.input_move.truncate();
    let horizontal = frame.physics.input_velocity().truncate();
    input.length_squared() >= decl.minimum_stick * decl.minimum_stick && horizontal.length() >= decl.minimum_speed
}
