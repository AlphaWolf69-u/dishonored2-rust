//! arkCpntPlayerStateMachine (vtable 0x141c73d58) with the locomotion states
//! Running, Sneaking, Sprinting, Sliding, Slipping, AutoCrouch, Jumping and
//! Falling (arkPlayerStateNavigation 0x141c3f198 and subclasses; InAir
//! 0x141c7b238 is the base of Jumping and Falling), and Leaning (vtable
//! 0x141c82b28), which runs beside them.
//!
//! Command processing follows FUN_140804850 and the rule table FUN_1408644e0
//! plus the mode's `m_rules`, including CatFall, Mantle and Vault. States the
//! port does not implement (actions and powers) are "missing" from the mode,
//! so requests for them are rejected as the game rejects a state its mode
//! lacks. Fall damage, stats, slide slow-time, shoves and breakables are not
//! ported.
use super::body::{FollowSlope, JumpAnim, Lean};
use super::camera::{self, PlayerCamera, MODE_ANIMATED, MODE_CATFALL, MODE_CONSTRAINED, MODE_CONSTRAINED_WITH_ARMS, MODE_DROWNING};
use super::midnight::{Midnight, PreparedScene};
use super::sensors::{MantleHeight, MantleSensor, VaultLimits, VaultSensor};
use super::shape::{POSTURE_LEAN_FORWARD, POSTURE_VAULTING};
use crate::edge_animation::EdgeAnimation;
use std::sync::Arc;
use super::character_proxy::{PlayerWorld, SupportState};
use crate::gameplay::collision_filter::{layer, QueryFilter};
use super::controller::{Controller, ACTION_JUMP, ACTION_LEAN_LEFT, ACTION_LEAN_RIGHT, ACTION_MOVE_BACKWARD, ACTION_MOVE_FORWARD};
use super::physics::{PlayerPhysics, UNSET};
use super::sensors::{AutoCrouchSensor, CatfallSensor, LeanSensor};
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
    pub const BLINKING: Self = Self(12);
    pub const USE_POWER: Self = Self(63);
    pub const DEAD: Self = Self(15);
    pub const LEANING: Self = Self(18);
    pub const GRAPPLE: Self = Self(27);
    pub const READ_NOTE: Self = Self(52);
    pub const FIRE_WEAPON: Self = Self(56);
    pub const CANCEL_FIRE_WEAPON: Self = Self(57);
    pub const REQUEST_RELOAD_WEAPON: Self = Self(58);
    pub const RELOAD_WEAPON: Self = Self(59);
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
    pub enhanced_damage_height: f32,
    pub enhanced_death_height: f32,
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
            enhanced_damage_height: f32::from_bits(0x412d_47ae),
            enhanced_death_height: f32::from_bits(0x41a4_0000),
            fall_damage: false,
        }
    }
}

/// arkDeclPlayerStateCatFall (ctor FUN_14085a780) fields the port uses.
#[derive(Clone, Debug, PartialEq)]
pub struct CatFallDecl {
    pub low_damage_midnight: Option<String>,
    pub high_damage_midnight: Option<String>,
    /// m_lowDamageMidnightLeftArm / RightArm and the high-damage pair
    /// (+0x78 / +0x80, +0xb0 / +0xb8); the sword variants need equipment.
    pub low_damage_arm_midnights: [Option<String>; 2],
    pub high_damage_arm_midnights: [Option<String>; 2],
    pub damage_threshold: f32,
    /// m_lookAtParams (degrees: pitch up, down, yaw left, right).
    pub look_at_params: [f32; 4],
}

impl Default for CatFallDecl {
    fn default() -> Self {
        Self {
            low_damage_midnight: None,
            high_damage_midnight: None,
            low_damage_arm_midnights: [None, None],
            high_damage_arm_midnights: [None, None],
            damage_threshold: 10.0,
            look_at_params: [50.0, 0.0, 50.0, 50.0],
        }
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
    Swimming(super::swimming::SwimmingStateDecl),
    SwimMantle,
    ReadNote,
            Kind::Leaning(_) | Kind::CatFall(_) | Kind::Mantle(_) | Kind::Vault(_) | Kind::SwimMantle | Kind::ReadNote | Kind::UsePower(_) | Kind::Blinking | Kind::Dead(_) => {}
