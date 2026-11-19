//! arkCpntPlayerStateMachine (vtable 0x141c73d58) with the locomotion states
//! Running, Sneaking, Sprinting, Sliding, Slipping, AutoCrouch, Jumping and
//! Falling (arkPlayerStateNavigation 0x141c3f198 and subclasses; InAir
//! 0x141c7b238 is the base of Jumping and Falling), and Leaning (vtable
//! 0x141c82b28), which runs beside them.
//!
//! Command processing follows FUN_140804850 and the rule table FUN_1408644e0
//! plus the mode's `m_rules`, including CatFall, Mantle and Vault. Action
//! families use the combat, equipment and power state owners. Unsupported
//! authored states remain absent from their mode and are rejected by the
//! same admission rules; navigation never substitutes another state for them.
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
mod throw_weapon_state;
pub mod use_heart;
pub mod play_heart_bark;

/// arkPlayerStateId_t (enum table 0x142391a90).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub i32);

impl StateId {
    pub const EQUIP_RIGHT: Self = Self(53);
    pub const DRAW_RIGHT: Self = Self(54);
    pub const EQUIP_LEFT: Self = Self(64);
    pub const DRAW_LEFT: Self = Self(65);
    pub const UNDRAW: Self = Self(81);
    pub const POWER_WHEEL: Self = Self(82);
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
    pub const USE_HEART: Self = Self(67);
    pub const PLAY_HEART_BARK: Self = Self(66);
    pub const USE_WINDBLAST: Self = Self(71);
    pub const DEAD: Self = Self(15);
    pub const LEANING: Self = Self(18);
    pub const GRAPPLE: Self = Self(27);
    pub const GRAPPLE_CATCH: Self = Self(84);
    pub const READ_NOTE: Self = Self(52);
    pub const LOOT_OBJECT: Self = Self(47);
    pub const GRAB_OBJECT: Self = Self(48);
    pub const CANCEL_ACTION: Self = Self(74);
    pub const HOLD_ONE_HAND: Self = Self(49);
    pub const HOLD_TWO_HAND: Self = Self(50);
    pub const RELEASE_OBJECT: Self = Self(51);
    pub const SLASH: Self = Self(39);
    pub const RECOIL: Self = Self(40);
    pub const BLOCK: Self = Self(44);
    pub const FIRE_WEAPON: Self = Self(56);
    pub const THROW_WEAPON: Self = Self(62);
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
    /// +68, m_unlocksCodeLockedBodyParts; independent of m_allowSelf.
    pub unlocks_code_locked_body_parts: bool,
    pub allow_self: bool,
    pub quit_self: bool,
    pub cancelable: bool,
    ///Declaration69 bit8, separately retained from the Cancel74 count bit4.
    pub cancelable_by_ui:bool,
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
#[derive(Clone, Debug, PartialEq)]
pub struct FallingDecl {
    /// Physics material references at +0x190/+0x198 (865c50/8658c0).
    pub landing_material: Option<Arc<str>>,
    pub dead_landing_material: Option<Arc<str>>,
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
            landing_material: None,
            dead_landing_material: None,
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
    /// (+0x78 / +0x80, +0xb0 / +0xb8).
    pub low_damage_arm_midnights: [Option<String>; 2],
    /// Normal/sneaking sword right-arm overrides (85a910, decl+88/+90).
    pub low_damage_sword_midnights: [Option<String>; 2],
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
            low_damage_sword_midnights: [None, None],
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
#[derive(Clone, Debug, PartialEq)]
pub struct SlidingDecl {
    /// m_slideFootStep, physics material at +0x188 (86ef50).
    pub contact_material: Option<Arc<str>>,
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
            contact_material: None,
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
    CancelAction(Arc<super::combat::ActionDecl>),
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
    PowerWheel,
    UseHeart(Arc<use_heart::Declaration>),
    PlayHeartBark(Arc<play_heart_bark::Declaration>),
    GrappleCatch(super::grapple_catch::Declaration),
    LootObject(Arc<super::carrying::GrabDecl>),
    GrabObject(Arc<super::carrying::GrabDecl>),
    HoldObject(Arc<super::carrying::HoldDecl>),
    ReleaseObject(Arc<super::combat::ActionDecl>),
    EquipChange(Arc<super::equipment_states::Declaration>),
    DrawEquipment(bool),
    Undraw(f32),
    FireWeapon(Arc<super::combat::FireDecl>),
    ThrowWeapon,
    Slash(Arc<super::combat::SlashDecl>),
    Recoil(Arc<super::combat::ActionDecl>),
    Block(Arc<super::combat::BlockDecl>),
    CancelFireWeapon,
    RequestReloadWeapon,
    ReloadWeapon,
    UsePower(Arc<super::power_states::UsePowerDecl>),
    Blinking,
    Grapple(Arc<super::grapple::Decl>),
    Dead(DrowningDeathDecl),
}

/// arkDeclPlayerStateDead defaults from 14085b970.
#[derive(Clone, Debug, PartialEq)]
pub struct DrowningDeathDecl {
    pub duration: f32,
    pub vertical_speed: f32,
    pub yaw_speed: f32,
    pub look_duration: f32,
    pub fx_delay: f32,
    pub fx: Option<String>,
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
            unlocks_code_locked_body_parts: false,
            allow_self: false,
            quit_self: false,
            cancelable: false,
            cancelable_by_ui:false,
            canceling: false,
            allow_camera_out_of_shape: false,
            breakable: false,
        };
        let mut navigation = NavigationDecl::default();
        let mut flags = navigation_flags;
        let (id, air_friction) = match kind {
            Kind::CancelAction(_) => {
                //85a610/620 inherits common853c90 secondary empty masks.
                flags=StateFlags{primary:false,body_parts:0,locked_body_parts:0,exclusive_body_parts:0,canceling:true,..flags};
                (StateId::CANCEL_ACTION,0.)
            }
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
            Kind::Swimming(ref swimming) => (swimming.id, 0.0),
            Kind::SwimMantle => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0, exclusive_body_parts: 0, ..flags };
                (StateId::MANTLE, 0.0)
            }
            Kind::ReadNote | Kind::PowerWheel => {
                // 844d60 ->853c90: secondary state, empty body-part masks.
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0, exclusive_body_parts: 0, ..flags };
                (if matches!(kind, Kind::PowerWheel) { StateId::POWER_WHEEL } else { StateId::READ_NOTE }, 0.0)
            }
            Kind::UseHeart(_) => {
                //855800 inherits853c90 secondary empty body-part masks.
                flags=StateFlags {primary:false,body_parts:0,locked_body_parts:0,exclusive_body_parts:0,..flags};
                (StateId::USE_HEART,0.)
            }
            Kind::PlayHeartBark(_) => {flags.primary=false;flags.body_parts=0;flags.locked_body_parts=0;flags.exclusive_body_parts=0;(StateId::PLAY_HEART_BARK,0.)}
            Kind::GrappleCatch(_) => {
                flags=StateFlags{primary:false,body_parts:0,locked_body_parts:0,exclusive_body_parts:0,allow_self:true,..flags};
                (StateId::GRAPPLE_CATCH,0.)
            }
            Kind::LootObject(ref grab) | Kind::GrabObject(ref grab) => {
                flags = StateFlags { primary:false,body_parts:8,locked_body_parts:8,exclusive_body_parts:0,
                    allow_camera_out_of_shape:true,..flags };
                (grab.id,0.)
            }
            Kind::HoldObject(_) => {
                flags = StateFlags { primary:false,body_parts:0,locked_body_parts:4,exclusive_body_parts:0,..flags };
                (StateId::HOLD_ONE_HAND,0.)
            }
            Kind::ReleaseObject(_) => {
                flags = StateFlags { primary:false,body_parts:0,locked_body_parts:0,exclusive_body_parts:0,..flags };
                (StateId::RELEASE_OBJECT,0.)
            }
            Kind::EquipChange(ref equip) => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0,
                    exclusive_body_parts: 0, allow_self: true, breakable: true, ..flags };
                (equip.state(), 0.0)
            }
            Kind::DrawEquipment(left) => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0,
                    exclusive_body_parts: 0, ..flags };
                (if left { StateId::DRAW_LEFT } else { StateId::DRAW_RIGHT }, 0.0)
            }
            Kind::Undraw(_) => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0,
                    exclusive_body_parts: 0, ..flags };
                (StateId::UNDRAW, 0.0)
            }
            Kind::ThrowWeapon => {
                //84a5e0 is the base PlayerState, with left/exclusive masks2.
                flags = StateFlags { primary:false,body_parts:2,exclusive_body_parts:2,..flags };
                (StateId::THROW_WEAPON,0.0)
            }
            Kind::FireWeapon(ref fire) => {
                // 140866440: secondary, left arm exclusive, breakable.
                flags = StateFlags { primary: false, body_parts: 2, locked_body_parts: 0,
                    exclusive_body_parts: 2, allow_self: fire.action.togglable, breakable: true, ..flags };
                (StateId::FIRE_WEAPON, 0.0)
            }
            Kind::Slash(ref slash) => {
                // 14086a710: right arm owned and locked, secondary.
                flags = StateFlags { primary: false, body_parts: 4, locked_body_parts: 4,
                    exclusive_body_parts: 0, allow_self: slash.action.togglable, ..flags };
                (StateId::SLASH, 0.0)
            }
            Kind::Block(ref block) => {
                // 140858d50 has the same right-arm masks as Slash.
                flags = StateFlags { primary: false, body_parts: 4, locked_body_parts: 4,
                    exclusive_body_parts: 0, allow_self: block.action.togglable, ..flags };
                (StateId::BLOCK, 0.0)
            }
            Kind::Recoil(_) => {
                //863cb0/863f70: right-arm ownership/lock and body-exclusive
                //mask1 are separate fields in the actual declaration.
                flags = StateFlags {primary:false,body_parts:4,locked_body_parts:4,exclusive_body_parts:1,..flags};
                (StateId::RECOIL,0.)
            }
            Kind::CancelFireWeapon => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0,
                    exclusive_body_parts: 0, ..flags };
                (StateId::CANCEL_FIRE_WEAPON, 0.0)
            }
            Kind::RequestReloadWeapon => {
                flags = StateFlags { primary: false, body_parts: 0, locked_body_parts: 0,
                    exclusive_body_parts: 0, ..flags };
                (StateId::REQUEST_RELOAD_WEAPON, 0.0)
            }
            Kind::ReloadWeapon => {
                // 140845660 has the same left-arm masks as FireWeapon.
                flags = StateFlags { primary: false, body_parts: 2, locked_body_parts: 0,
                    exclusive_body_parts: 2, breakable: true, ..flags };
                (StateId::RELOAD_WEAPON, 0.0)
            }
            Kind::UsePower(ref power) => {
                // 1408560c0: action state owns the left hand.
                flags = StateFlags { primary: false, body_parts: 2, locked_body_parts: 0,
                    exclusive_body_parts: 2, ..flags };
                (if power.windblast { StateId::USE_WINDBLAST } else { StateId::USE_POWER }, 0.0)
            }
            Kind::Grapple(_) => {
                // 837390: base non-primary BODY owner, locked BODY,
                // no exclusive parts; allowCameraOutOfShape bit6a0 set.
                flags = StateFlags { primary: false, body_parts: BODY, locked_body_parts: BODY,
                    exclusive_body_parts: 0, allow_camera_out_of_shape: true, ..flags };
                (StateId::GRAPPLE, 0.0)
            }
            Kind::Blinking => {
                // 140857150: secondary BODY state; no navigation defaults.
                flags = StateFlags { primary: false, body_parts: BODY, locked_body_parts: BODY,
                    exclusive_body_parts: 0, ..flags };
                (StateId::BLINKING, 0.0)
            }
            Kind::Dead(_) => {
                flags = StateFlags { primary: true, body_parts: 0x1f, locked_body_parts: 0x1f, exclusive_body_parts: 0x1f, allow_camera_out_of_shape: true, ..flags };
                (StateId::DEAD, 0.0)
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
/// durations (FUN_140770e90 Р В Р вЂ Р Р†Р вЂљР’В Р Р†Р вЂљРІвЂћСћ FUN_140771270).
#[derive(Clone, Debug, PartialEq)]
pub struct LocoStances {
    pub current: Option<String>,
    /// Body slot 8, the head-bob stance.
    pub bob: Option<String>,
    /// (from, to) Р В Р вЂ Р Р†Р вЂљР’В Р Р†Р вЂљРІвЂћСћ duration in seconds.
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
    /// The owning player's counts survive main/swimming mode transitions.
    pub body_parts: &'a mut super::body_parts::BodyParts,
    pub attribute_requests: &'a mut Vec<super::AttributeModifierRequest>,
    pub equipment_changes: &'a mut super::equipment_states::Runtime,
    /// The real selected ranged component; no inventory or projectile copy.
    pub weapon: &'a mut Option<super::weapons::Runtime>,
    pub throwable: &'a mut Option<super::weapons::throwable::WeaponRuntime>,
    pub holstered_throwables: &'a mut HashMap<String,super::weapons::throwable::WeaponRuntime>,
    pub body_additives: &'a super::animation::AnimBasicNode,
    pub body_additive_command: &'a mut Option<super::animation::AnimBasicCommand>,
    /// Equipment action component handles, rather than locomotion blends.
    pub equipment_transitioning: [bool; 2],
    /// Arm order: left (inventory slot1), right (inventory slot0).
    pub equipment_archetypes: [Option<u32>; 2],
    pub action_time: f64,
    pub powers: &'a mut super::power_states::Runtime,
    pub mana: &'a mut super::mana::Mana,
    pub mana_replenishes: bool,
    pub dt: f32,
    pub world: &'a dyn PlayerWorld,
    pub controller: &'a mut Controller,
    pub carrying: &'a mut super::carrying::Runtime,
    pub interaction_present: bool,
    pub physics: &'a mut PlayerPhysics,
    pub body: &'a mut LocoStances,
    pub footstep: &'a mut super::footsteps::Footstep,
    pub footstep_clock: &'a mut super::footsteps::Footsteps,
    pub contact_requests: &'a mut Vec<crate::gameplay::sound::Contact>,
    pub sound_requests: &'a mut Vec<crate::gameplay::sound::Request>,
    pub body_speed: f32,
    pub camera: &'a mut PlayerCamera,
    pub follow_slope: &'a mut FollowSlope,
    pub requests: &'a mut MotionRequests,
    pub autocrouch: &'a mut AutoCrouchSensor,
    /// Entity origin, axis (rows forward/left/up as columns) and velocity.
    pub origin: Vec3,
    pub axis: Mat3,
    pub camera_axis: Mat3,
    pub velocity: Vec3,
    pub attributes: &'a Attributes,
    pub sprint_toggle: bool,
    pub look_stick_sensitivity: [f32; 2],
    pub lean: &'a mut Lean,
    pub lean_sensor: &'a mut LeanSensor,
    pub catfall_sensor: &'a mut CatfallSensor,
    /// The arm parts' shown state (FUN_140774a10), left and right.
    pub arms_shown: &'a mut [bool; 2],
    /// body+0xc58 m_desiredLegsVisibility.
    pub desired_legs: &'a mut i32,
    pub jump_anim: &'a mut JumpAnim,
    pub midnight: &'a mut Midnight,
    pub scenes: &'a Scenes,
    /// The player body md6 (CONDMD6 choices).
    pub md6: [&'a str; super::midnight::TARGETS],
    /// The body's and both arms' animations (the midnight cast).
    pub animations: super::midnight::Cast<'a>,
    pub mantle_sensor: &'a mut MantleSensor,
    pub vault_sensor: &'a mut VaultSensor,
    pub retargeting: &'a mut Option<Retargeting>,
    /// Maximum health (health decl m_health + ARK_ATTRIBUTE_HEALTH_MAX),
    /// used by 8657c0's fall-damage calculation.
    pub health: f32,
    pub damage_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub damage_recipient: super::damage::Recipient<'a>,
    pub fall_damage: Option<&'a crate::assets::damage::Declaration>,
    pub fall_incapacitated: &'a mut bool,
    pub underwater: bool,
    pub swimming: bool,
    ///846450 publishes these only from the active Swimming state;
    ///846270 clears them when another primary state takes over.
    pub swimming_motion: Option<(Vec3, f32)>,
    pub swim_mantle_exit: &'a mut bool,
    pub game_over_requested: &'a mut bool,
    pub drowning_fx_requested: &'a mut Option<String>,
    /// The game's global LCG at gameLocal+0x600f8.
    pub random: &'a mut u32,
    pub heart_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub heart_target: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub heart_bark_playing: bool,
    pub heart_requests: &'a mut std::collections::VecDeque<super::weapons::heart::UseRequest>,
    /// `ark_playerSmallFallCooldown` (cvar 0x143996320, "0.5").
    pub small_fall_cooldown: f32,
    pub lean_toggle: bool,
    /// `!lastInputWasGamepad` (inputMgr+0xa544, FUN_140039d50).
    pub keyboard: bool,
}

impl Frame<'_> {
    /// 770990: an explicit impact material overrides only the current source
    /// material. The actual ground target and optional body sound remain shared.
    fn emit_contact(&mut self, material: Option<&str>) {
        use crate::gameplay::sound::{Contact, Emitter, Request};
        if let Some(target) = self.physics.ground_material_hit.as_ref()
            .and_then(|hit| self.world.material(hit)) {
            if self.footstep_clock.buffer <= 0.0 {
                if let Some(source) = material.or(self.footstep.material.as_deref()) {
                    self.contact_requests.push(Contact {
                        source: source.to_owned(), target: target.to_owned(),
                        target_material_id: self.physics.ground_material_hit.as_ref().and_then(|hit|hit.material_id),
                        position: self.origin, normal: Vec3::Z, direction: Vec3::ZERO,
                        speed: self.body_speed.max(self.physics.gravity_velocity.z.abs()),
                        // The character proxy uses fixed motion0 (092260).
                        mass: 0.0,
                        stimulus_enabled: self.controller.possession_state() == 14,
                    });
                }
            }
        }
        if let Some(event) = self.footstep.sound {
            self.sound_requests.push(Request::Post { event, emitter: Emitter::PlayerBody });
        }
    }
}

/// Player attribute base values (`attributes_player.cpntattributesplayer`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes(pub HashMap<String, f32>, pub HashMap<String, AttributeValue>);

/// 14060b900 keeps the authored base, accumulated modifiers and limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttributeValue {
    pub base: f32,
    pub increase: f32,
    pub percentage: f32,
    pub limits: Option<(f32, f32)>,
}

impl Attributes {
    /// A missing attribute index reads as 0.
    pub fn get(&self, tag: &str) -> f32 {
        self.0.get(tag).copied().unwrap_or(0.0)
    }
    /// The authored base (record +8); a missing attribute reads its
    /// current value, so current and base compare equal.
    pub fn base(&self, tag: &str) -> f32 {
        self.1.get(tag).map_or_else(|| self.get(tag), |attribute| attribute.base)
    }
    /// Add or remove a modifier and update the current value; unknown tags
    /// are ignored as in the original attribute lookup.
    pub fn modifier(&mut self, tag: &str, increase: f32, percentage: f32, added: bool) -> bool {
        let Some(attribute) = self.1.get_mut(tag) else { return false };
        let sign = if added { 1.0 } else { -1.0 };
        attribute.increase += increase * sign;
        attribute.percentage += percentage * sign;
        let mut current = attribute.percentage * attribute.base * 0.01 + attribute.base + attribute.increase;
        if let Some((min, max)) = attribute.limits {
            current = if min <= current { if max < current { max } else { current } } else { min };
        }
        if let Some(value) = self.0.get_mut(tag) { *value = current; }
        true
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
    /// +0xa0: CATFALL was requested (its sensor stays on into CatFall).
    catfall_taken: bool,
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
            Kind::CatFall(d) => d
                .low_damage_midnight
                .iter()
                .chain(d.high_damage_midnight.iter())
                .chain(d.low_damage_arm_midnights.iter().flatten())
                .chain(d.high_damage_arm_midnights.iter().flatten())
                .collect(),
            Kind::Mantle(d) => d.low_midnights.iter().chain(&d.mid_midnights).chain(&d.high_midnights).flatten().collect(),
            Kind::Vault(d) => d.small_midnights.iter().chain(&d.high_midnights).collect(),
            Kind::LootObject(d) | Kind::GrabObject(d) => d.action.midnight.iter().collect(),
            Kind::ReleaseObject(d) | Kind::CancelAction(d) => d.midnight.iter().collect(),
            Kind::Grapple(d) => d.translate_midnight.iter().chain(&d.wall_midnight).chain(&d.ceil_midnight).collect(),
            Kind::UsePower(d) => d.out_of_mana.iter().chain(&d.out_of_mana_slide).chain(&d.action.midnight).collect(),
            Kind::UseHeart(d) => d.beat.iter().collect(),
            Kind::PlayHeartBark(d) => d.action.midnight.iter().chain(d.use_midnight.iter()).collect(),
            Kind::EquipChange(d) => d.power_switch.iter().chain(d.power_switch_slide.iter()).collect(),
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
    equipment_restore: [bool; 2],
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
    read_note_requests: Option<crate::gameplay::interaction_definition::StateRequestPair>,
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
    swimming_dive: (Vec3, f32),
    death_timer: f32,
    undraw_elapsed: f32,
    draw_pending: StateId,
    death_transition_pending: bool,
    death_fx_pending: bool,
    fire: super::combat::FireRuntime,
    throw_weapon: super::weapons::throwable::ThrowState,
    slash: super::combat::SlashRuntime,
    recoil: super::combat::RecoilRuntime,
    block: super::combat::BlockRuntime,
    heart: use_heart::Runtime,
    heart_bark: super::combat::ActionRuntime,
    cancel_action: super::combat::ActionRuntime,
    /// 14085b490 +56/+57: hands which were holding actual equipment.
    catfall_restore: [bool; 2],
    code_locks: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    RequestEquipment { side: usize, flags: u32, source: StateId },
    Request(StateId),
    /// FUN_1408024d0 with Sliding's params {uid, 7, speed}.
    RequestSlide(f32),
    /// Falling Land()'s CATFALL request with params {uid, damage}.
    RequestCatFall(f32),
    Quit(StateId),
    ///7f9530/BF ->7fbeb0 ->86bb60, the authored slash interrupt window.
    FinishSlash,
    /// FUN_1407fbb20's landing request: the first available of Running,
    /// Sneaking, AutoCrouch and the mode default.
    Landing,
    RequestReadNote,
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
    ///Source state-machine+2f0, maintained by actual activation/exit only.
    cancelable_count:i32,
    read_note: super::read_note::Runtime,
    wheel_movie_ready: bool,
    wheel_logic_active: bool,
    wheel_requested: bool,
    world_pickup: super::world_pickup::Runtime,
    pub(super) grapple_catch:super::grapple_catch::Runtime,
    slash_rules: Option<super::slash_rules::Runtime>,
    ///One actual callback receipt, only enabled for the opt-in WORLD power
    ///diagnostic. No animation/event history is retained in normal gameplay.
    recoil_interrupt_diagnostic:Option<(u64,u32,u32,bool)>,
}

pub(crate) fn cos_snap(degrees: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut angle = degrees * 0.017453292;
    if angle < 0.0 || angle >= TAU {
        angle -= (angle * (1.0 / TAU)).floor() * TAU;
    }
    angle.cos()
}

/// acos clamped as the decompiled callers do (Р В РЎСџР В РІР‚С™ at Р В Р вЂ Р Р†Р вЂљР’В°Р вЂ™Р’В¤ -1, 0 at Р В Р вЂ Р Р†Р вЂљР’В°Р СћРЎвЂ™ 1).
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
        Self { states: Vec::new(), active: Vec::new(), queue: Vec::new(), rules: HashMap::new(), default_state: StateId::RUNNING, pending_sounds: Vec::new(), cancelable_count:0, read_note: Default::default(), wheel_movie_ready:false,wheel_logic_active:false,wheel_requested:false, world_pickup: Default::default(), grapple_catch:Default::default(), slash_rules: None,
            recoil_interrupt_diagnostic:matches!(std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(),Ok("world-power"|"world-power-poke")).then_some((0,0,0,false)) }
    }

    pub(super) fn world_pickup_busy(&self)->bool{self.world_pickup.busy()}
    pub(super) fn world_pickup_target(&self)->Option<crate::gameplay::entity_lifetime::EntityId>{self.world_pickup.target()}
    pub(super) fn accepted_world_pickup(&self)->Option<crate::gameplay::entity_lifetime::EntityId>{self.world_pickup.accepted()}
    pub(super) fn rejected_world_pickup(&mut self)->Option<crate::gameplay::entity_lifetime::EntityId>{self.world_pickup.rejected()}
    pub(super) fn request_world_pickup(&mut self,id:crate::gameplay::entity_lifetime::EntityId)->bool{
        if !matches!(self.state(StateId::LOOT_OBJECT).map(|state|&state.decl.kind),Some(Kind::LootObject(_)))||!self.world_pickup.request(id){return false}
        self.request(StateId::LOOT_OBJECT);true
    }
    pub(super) fn offer_world_pickup(&mut self){self.world_pickup.offer();self.quit(StateId::LOOT_OBJECT);}
    pub(super) fn acknowledge_world_pickup(&mut self,id:crate::gameplay::entity_lifetime::EntityId)->Result<(),String>{self.world_pickup.acknowledge(id)?;self.quit(StateId::LOOT_OBJECT);Ok(())}
    pub(super) fn lose_world_pickup(&mut self,id:crate::gameplay::entity_lifetime::EntityId)->Result<(),String>{
        self.world_pickup.lose(id)?;self.quit(StateId::LOOT_OBJECT);Ok(())
    }
    pub(super) fn reject_world_pickup(&mut self,id:crate::gameplay::entity_lifetime::EntityId)->Result<(),String>{
        self.world_pickup.reject_active(id)?;self.quit(StateId::LOOT_OBJECT);Ok(())
    }
    pub(super) fn finish_world_pickup(&mut self){self.world_pickup.reject();self.world_pickup.offer_from_exit();self.quit(StateId::LOOT_OBJECT);}
    pub(super) fn world_pickup_from_exit(&self)->bool{self.world_pickup.offered_from_exit}
    pub(super) fn world_pickup_allowed(&self,id:StateId,parts:&super::body_parts::BodyParts,now:f64)->Option<bool>{
        let Some(state)=self.state(id) else{return Some(false)};
        let Kind::LootObject(declaration)=&state.decl.kind else{return Some(false)};
        Some(!self.world_pickup.busy()&&parts.can_activate(state.decl.flags.body_parts,state.decl.flags.unlocks_code_locked_body_parts)
            &&self.world_pickup.action.can_activate(now,&declaration.action)
            &&self.active.iter().all(|&active|self.rule(active,id).allow))
    }
    fn start_world_pickup_scene(&mut self,index:usize,declaration:&super::carrying::GrabDecl,frame:&mut Frame){
        let handle=Self::play(declaration.action.midnight.as_ref(),1.,self.states[index].decl.flags.body_parts,frame);
        self.states[index].midnight=handle;
        self.world_pickup.action.after_scene(handle,&declaration.action);
        self.world_pickup.scene_started=true;
    }

    /// The mode: its state decls, `m_rules` and default state.
    pub fn new(decls: Vec<StateDecl>, rule_decls: &[RuleDecl], default_state: StateId) -> Self {
        let states: Vec<State> = decls
            .into_iter()
            .map(|decl| State {
                read_note_requests: matches!(decl.kind, Kind::ReadNote).then(crate::gameplay::interaction_definition::StateRequestPair::new),
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
                swimming_dive: (Vec3::ZERO, 0.0),
                death_timer: 0.0,
                undraw_elapsed: 0.0,
                draw_pending: StateId::NULL,
                death_transition_pending: false,
                death_fx_pending: false,
                fire: Default::default(),
                throw_weapon: Default::default(),
                slash: Default::default(),
                recoil: Default::default(),
                block: Default::default(),
                heart: Default::default(),
                heart_bark: Default::default(),
                cancel_action:Default::default(),
                catfall_restore: [false; 2],
                code_locks: 0,
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
        Self { states, active: Vec::new(), queue: Vec::new(), rules, default_state, pending_sounds: Vec::new(), cancelable_count:0, read_note: Default::default(), wheel_movie_ready:false,wheel_logic_active:false,wheel_requested:false, world_pickup: Default::default(), grapple_catch:Default::default(), slash_rules: None,
            recoil_interrupt_diagnostic:matches!(std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref(),Ok("world-power"|"world-power-poke")).then_some((0,0,0,false)) }
    }

    pub fn install_slash_rules(&mut self, definition: Arc<crate::assets::slash_rules::Definition>) {
        self.slash_rules = Some(super::slash_rules::Runtime::new(definition));
    }

    /// 140854050/090 and 1408029f0/2a10: one counted code lock per
    /// requesting state and body part. These do not rewrite mode rules.
    fn code_lock(&mut self, index: usize, part: usize, enabled: bool, frame: &mut Frame) {
        let bit = 1 << part;
        let held = self.states[index].code_locks & bit != 0;
        if enabled && !held {
            self.states[index].code_locks |= bit;
            frame.body_parts.soft_lock(part);
        } else if !enabled && held {
            self.states[index].code_locks &= !bit;
            frame.body_parts.soft_unlock(part);
        }
    }

    pub fn active_states(&self) -> &[StateId] {
        &self.active
    }

    /// Mode switch 140805f70 deactivates the outgoing mode before entering
    /// its replacement; each mode retains its own state instances.
    pub fn leave_mode(&mut self, frame: &mut Frame) {
        self.read_note.reject();
        self.world_pickup.reject();
        self.read_note.deactivate();
        for id in self.active.clone().into_iter().rev() {
            if let Some(index) = self.index(id) {
                self.deactivate(index, frame);
            }
        }
        self.active.clear();
        self.queue.clear();
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

    /// 62bf40 reads the actual active FireWeapon state and source phase4.
    pub fn ranged_is_charging(&self) -> bool {
        self.state(StateId::FIRE_WEAPON).is_some_and(|state| state.active && state.fire.phase == super::combat::FirePhase::Charging)
    }

    /// 62f160 reads the real Slash's focused flag for weapon damage/material.
    pub fn melee_focused(&self) -> bool {
        self.state(StateId::SLASH).is_some_and(|state| state.active && state.slash.focused)
    }

    ///81fda0 reads the actual active Slash+d1 (focus hold), distinct from
    ///62f160's already-released focused attack+d2. No mutable state escapes.
    pub(super) fn melee_focus_active(&self) -> bool {
        self.state(StateId::SLASH).is_some_and(|state| state.active && state.slash.focus_active)
    }

    ///86bec0 qualifies a focused release against the real Action+38 clock
    ///and Slash declaration+a0, independently of the focus FX envelope.
    pub fn melee_focus_charge(&self) -> Option<(f32, f32)> {
        let state = self.state(StateId::SLASH)
            .filter(|state| state.active && state.slash.focus_active)?;
        let Kind::Slash(declaration) = &state.decl.kind else { return None; };
        Some((state.slash.action.elapsed(), declaration.focus_hold_duration))
    }

    pub(super) fn write_combat_diagnostics(&self, report: &mut String, owner: &str) {
        use std::fmt::Write;
        let _ = write!(report, "  {owner} active_ids({})=", self.active.len());
        for id in self.active.iter().take(32) { let _ = write!(report, " {:?}", id); }
        let _ = writeln!(report, " queued_commands={} cancelable_count={} slash_interrupt_pending={}", self.queue.len(), self.cancelable_count, self.queue.iter().any(|command| matches!(command, Command::FinishSlash)));
        if let Some(state) = self.state(StateId::SLASH) {
            let s = &state.slash;
            let _ = writeln!(report, "    slash active={} action={:?} last_type={} last_time={} chained={} left_response={} focus_ready={} focus_active={} focused={} can_focus={} release_seen={} press_again={} selection={} dispersion_active={}", state.active, s.action, s.last_type, s.last_time, s.chained_scene, s.secondary_scene, s.focus_ready, s.focus_active, s.focused, s.can_focus, s.release_seen, s.press_again, s.selected.is_some(), s.dispersion_active);
        }
        for id in [StateId::FIRE_WEAPON, StateId::THROW_WEAPON, StateId::USE_HEART, StateId::PLAY_HEART_BARK] {
            if let Some(state) = self.state(id) {
                if state.active {
                    let _ = writeln!(report, "    state={:?} scene={} fire={:?} throw={:?} heart={:?} bark_action={:?}", id, state.midnight, state.fire, state.throw_weapon, state.heart, state.heart_bark);
                }
            }
        }
    }

    ///630810/6302e0 set the retained Recoil outcome, stop Slash, and request40.
    pub fn request_melee_recoil(&mut self, kind: usize) {
        let selection=self.state(StateId::SLASH).and_then(|state|state.slash.selected.clone());
        if let Some(index)=self.index(StateId::RECOIL) {
            self.states[index].recoil.selection=selection;
            self.states[index].recoil.kind=kind;
            self.quit(StateId::SLASH);
            self.request(StateId::RECOIL);
        }
    }
    pub fn melee_recoil_interrupt_receipt(&self)->Option<(u64,u32,u32,bool)>{self.recoil_interrupt_diagnostic}
    pub fn melee_left_response(&self)->Option<u32>{self.state(StateId::SLASH).filter(|state|state.active)
        .map(|state|state.slash.secondary_scene).filter(|handle|*handle!=0)}

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
    pub fn read_note_busy(&self) -> bool { self.read_note.busy() }
    pub fn read_note_request_pending(&self) -> bool { self.read_note.pending() }
    pub fn set_read_note_admission(&mut self, available: bool) { self.read_note.admission = available; }
    ///692180 requires the real wheel's loaded Iggy player (4b0b40), in
    ///addition to the ordinary body-part and mode-rule admission above.
    pub fn wheel_status(&mut self, movie_ready:bool, logic_active:bool) {
        self.wheel_movie_ready=movie_ready;
        self.wheel_logic_active=logic_active;
    }
    pub fn take_wheel_request(&mut self)->bool { std::mem::take(&mut self.wheel_requested) }

    pub fn request_read_note(&mut self, presentation: Arc<super::read_note::Presentation>) -> bool {
        if !self.read_note.request(presentation) { return false; }
        self.queue.push(Command::RequestReadNote);
        true
    }

    pub fn take_read_note_result(&mut self) -> Option<super::read_note::ActivationResult> {
        self.read_note.take_result()
    }

    pub fn read_note_presentation(&self) -> Option<&Arc<super::read_note::Presentation>> {
        self.read_note.descriptor.as_ref()
    }

    /// 1408451b0/140845160: journal items replace the same descriptor without
    /// activating player state 52 or admitting the item to inventory again.
    pub fn set_read_note_presentation(&mut self, presentation: Arc<super::read_note::Presentation>) -> bool {
        if self.index(StateId::READ_NOTE).is_none() { return false; }
        self.read_note.descriptor = Some(presentation);
        true
    }

    pub fn finish_read_note(&mut self) {
        // Journal helpers can populate an inactive state. Its original raw
        // descriptor fields do not own strings; release our host Arc lease
        // when that UI context ends, even when no active state will exit.
        if !self.is_active(StateId::READ_NOTE) { self.read_note.deactivate(); }
        self.quit(StateId::READ_NOTE);
    }

    pub fn request(&mut self, id: StateId) {
        self.queue.push(Command::Request(id));
    }
    /// 140803280's current-mode gate for the parameterized Blink request.
    /// Its target/shape payload supplies CanActivate; other queued requests
    /// still undergo the normal batch arbitration and rejection callback.
    pub fn grapple_request_allowed(&self) -> bool {
        self.state(StateId::GRAPPLE).is_some_and(|state| matches!(state.decl.kind, Kind::Grapple(_)))
            && self.active.iter().all(|&active| self.rule(active, StateId::GRAPPLE).allow)
    }
    pub fn blink_request_allowed(&self) -> bool {
        self.state(StateId::BLINKING).is_some_and(|state| matches!(state.decl.kind, Kind::Blinking))
            && self.active.iter().all(|&active| self.rule(active, StateId::BLINKING).allow)
    }

    pub fn request_equipment_left(&mut self, flags: u32, source: StateId) {
        self.queue.push(Command::RequestEquipment { side: 0, flags, source });
    }

    pub(super) fn admit_instant_equipment(&mut self, runtime: &mut super::equipment_states::Runtime) {
        for (side, id) in [(0, StateId::EQUIP_LEFT), (1, StateId::EQUIP_RIGHT)] {
            if !runtime.instant_pending(side) { continue; }
            let Some(index) = self.index(id) else { continue };
            if !matches!(self.states[index].decl.kind, Kind::EquipChange(_))
                || self.active.iter().any(|from| !self.rule(*from, id).allow) { continue; }
            runtime.begin_instant(side);
            // This source request has completed both skipped phases. Preserve
            // all other queued player actions for the first gameplay frame.
            self.queue.retain(|command| !matches!(command, Command::Request(state) if *state == id));
        }
    }

    pub fn request_equipment_right(&mut self, flags: u32, source: StateId) {
        self.queue.push(Command::RequestEquipment { side: 1, flags, source });
    }

    /// Continue only equipment callbacks after the owning Player swaps the
    /// actual trees. This is not a second simulation/update of other states.
    pub fn advance_equipment_changes(&mut self, frame: &mut Frame) {
        for id in [StateId::EQUIP_RIGHT, StateId::EQUIP_LEFT] {
            let Some(index) = self.index(id).filter(|index| self.states[*index].active) else { continue };
            let Kind::EquipChange(decl) = &self.states[index].decl.kind else { continue };
            let sneak = [StateId::SNEAKING, StateId::AUTOCROUCH, StateId::WALLCROUCH, StateId::SLIDING]
                .into_iter().any(|id| self.is_active(id));
            match super::equipment_states::advance(decl, sneak, self.is_active(StateId::SLIDING), &self.active, frame) {
                Ok(true) => self.quit(id),
                Ok(false) => {},
                Err(error) => { frame.midnight.last_start_error = Some(error); self.quit(id); },
            }
        }
        //The registered scene completion owns its accepted phase even when
        //806350 has deactivated that mode's state (837240 ->835cf0).
        for (side,id) in [(0,StateId::EQUIP_LEFT),(1,StateId::EQUIP_RIGHT)] {
            if frame.equipment_changes.has_owned_phase(side) && !self.is_active(id) {
                let sneak = [StateId::SNEAKING, StateId::AUTOCROUCH, StateId::WALLCROUCH, StateId::SLIDING]
                    .into_iter().any(|id| self.is_active(id));
                if let Err(error) = super::equipment_states::continue_owned(side, sneak, self.is_active(StateId::SLIDING), &self.active, frame) {
                    frame.midnight.last_start_error = Some(error);
                }
            }
        }
    }

    /// FUN_140854190 Р В Р вЂ Р Р†Р вЂљР’В Р Р†Р вЂљРІвЂћСћ FUN_140802a20.
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
            .is_some_and(|s| matches!(&s.decl.kind, Kind::Sliding(d) if speed > d.out_speed))
    }

    /// Slot 6 CanActivate.
    fn can_activate(&self, index: usize, frame: &Frame) -> bool {
        let state = &self.states[index];
        // 140854890/1408035c0/600: component requests are player-wide;
        // authored mode rules separately govern state-to-state conflicts.
        if !frame.body_parts.can_activate(state.decl.flags.body_parts,
            state.decl.flags.unlocks_code_locked_body_parts) { return false; }
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
            Kind::ReadNote => state.decl.flags.body_parts == 0 && self.read_note.admission,
            Kind::CancelAction(declaration)=>self.cancelable_count>0
                && state.cancel_action.can_activate(frame.action_time,declaration),
            Kind::PowerWheel => self.wheel_movie_ready,
            Kind::UseHeart(_) => true,
            Kind::PlayHeartBark(d) => frame.heart_entity.is_some() && !frame.heart_bark_playing && self.states[index].heart_bark.can_activate(frame.action_time,&d.action),
            Kind::GrappleCatch(_) => true,
            Kind::LootObject(decl) => self.world_pickup.pending() && self.world_pickup.action.can_activate(frame.action_time,&decl.action),
            Kind::GrabObject(decl) => frame.carrying.pending() && frame.carrying.grab_state()==Some(decl.id) && frame.carrying.release.can_activate(frame.action_time,&decl.action),
            Kind::HoldObject(_) => frame.carrying.held.as_ref().is_some_and(|target|target.hold_state()==state.decl.id),
            Kind::ReleaseObject(decl) => frame.carrying.has_held() && frame.carrying.release.can_activate(frame.action_time,decl),
            Kind::EquipChange(ref decl) => decl.side().is_some_and(|side| frame.equipment_changes.has_request(side)),
            Kind::DrawEquipment(left) => {
                let side = usize::from(!*left);
                !frame.equipment_changes.equipped(side)
                    && frame.equipment_changes.preferred(side).is_some()
                    && self.state(if *left { StateId(64) } else { StateId(53) }).is_some()
            },
            //846b50: common body-part gate above, no cancellable action,
            //and neither primary nor secondary sensor33 descriptor present.
            Kind::Undraw(_) => !self.is_active(StateId::DEAD)
                && self.cancelable_count < 1 && !frame.interaction_present,
            Kind::Recoil(decl) => state.recoil.action.can_activate(frame.action_time,decl)
                && !frame.equipment_transitioning[1],
            Kind::Slash(decl) => frame.equipment_archetypes[1] == Some(1)
                && !frame.equipment_transitioning[1] && self.slash_rules.is_some()
                && state.slash.action.can_activate(frame.action_time, &decl.action),
            Kind::Block(decl) => frame.equipment_archetypes[1] == Some(1)
                && !frame.equipment_transitioning[1] && state.block.action.can_activate(frame.action_time, &decl.action),
            Kind::ThrowWeapon => frame.equipment_archetypes[0]==Some(4) && !frame.equipment_transitioning[0]
                && frame.throwable.as_ref().is_some_and(|weapon|weapon.can_fire()),
            Kind::FireWeapon(decl) => {
                let action = frame.controller.mapping.iter().find(|mapping| mapping.state == state.decl.id)
                    .map_or(0, |mapping| mapping.action);
                let transitioning = state.decl.flags.body_parts & 2 != 0 && frame.equipment_transitioning[0]
                    || state.decl.flags.body_parts & 4 != 0 && frame.equipment_transitioning[1];
                !transitioning && state.fire.action.can_activate(frame.action_time, &decl.action)
                    // bevy_backend: reject unavailable projectile births at
                    // admission. A loaded gun must not take the empty-ammo
                    // animation merely because the host factory is missing.
                    && frame.weapon.as_ref().is_some_and(|weapon| (weapon.magazine <= 0 || weapon.bevy_backend_projectile_ready)
                        && (weapon.can_fire() || frame.controller.pressed(action)
                            && (state.midnight == 0 || frame.midnight.finished(state.midnight))))
            }
            Kind::CancelFireWeapon => self.state(StateId::FIRE_WEAPON).is_some_and(|fire| fire.active
                && matches!(fire.fire.phase, super::combat::FirePhase::TargetDelay | super::combat::FirePhase::Targeting
                    | super::combat::FirePhase::TargetAcquired | super::combat::FirePhase::Charging)),
            Kind::RequestReloadWeapon | Kind::ReloadWeapon => frame.weapon.as_ref().is_some_and(|weapon| weapon.can_reload())
                || frame.throwable.as_ref().is_some_and(|weapon|weapon.magazine==0&&weapon.total_ammo>0),
            Kind::UsePower(decl) => super::power_states::can_use(frame, decl),
            Kind::Grapple(_) => frame.powers.grapple_pending && frame.powers.grapple.is_some(),
            Kind::Blinking => frame.powers.blink_pending && frame.powers.blink.is_some(),
            Kind::Swimming(_) => navigation,
            Kind::SwimMantle => !frame.underwater && frame.mantle_sensor.can_mantle(),
            Kind::Dead(_) => true,
            // FUN_14085d9a0: the base body-part test passes with no parts.
            Kind::Leaning(_) => physics.near_ground(),
            // FUN_14085b2e0 (swimming is not ported).
            Kind::CatFall(_) => true,
            //860180 uses sensor2 only while the actual Blinking state is active.
            Kind::Mantle(_) => !self.is_active(StateId::MANTLE) && (frame.mantle_sensor.can_mantle()
                || self.is_active(StateId::BLINKING) && frame.powers.blink_mantle_sensor.as_ref().is_some_and(|sensor| sensor.can_mantle())),
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

    /// 140803280 for the installed READ_NOTE decl. Its empty body-part
    /// mask makes 140854890 unconditional; parameterized item and game-logic
    /// checks remain with their actual owners. A modified nonempty mask needs
    /// the code-lock owner, which is not yet hosted.
    pub fn read_note_state_allowed(&self) -> Option<bool> {
        let Some(state) = self.state(StateId::READ_NOTE) else { return Some(false) };
        if state.decl.flags.body_parts != 0 { return None; }
        Some(self.active.iter().all(|&active| self.rule(active, StateId::READ_NOTE).allow))
    }

    /// 803280/840b20 with a real parameterized interaction target. The host
    /// checks the target component/physics/sensor before this state/body gate;
    /// prompt inspection does not manufacture a pending pickup request.
    pub(crate) fn carry_interaction_allowed(&self,id:StateId,parts:&super::body_parts::BodyParts,
        carrying:&super::carrying::Runtime,now:f64)->Option<bool> {
        let Some(state)=self.state(id) else{return Some(false);};
        let Kind::GrabObject(decl)=&state.decl.kind else{return Some(false);};
        Some(!carrying.pending()&&!carrying.has_held()
            &&parts.can_activate(state.decl.flags.body_parts,state.decl.flags.unlocks_code_locked_body_parts)
            &&carrying.release.can_activate(now,&decl.action)
            &&self.active.iter().all(|&active|self.rule(active,id).allow))
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
            frame.midnight.last_start_error = Some(format!("{path}: scene not loaded"));
            return 0;
        };
        match frame.midnight.start_with_parts(scene, frame.md6, rate, frame.animations, parts, frame.random) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("Midnight scene {path}: {error}");
                frame.midnight.last_start_error = Some(format!("{path}: {error}"));
                0
            }
        }
    }

    /// 140866fe0 chooses the actual selected ranged component's mode.
    fn activate_fire(&mut self, index: usize, decl: &super::combat::FireDecl, frame: &mut Frame) {
        let Some(weapon) = frame.weapon.as_ref() else {
            self.quit(StateId::FIRE_WEAPON);
            return;
        };
        self.states[index].fire.begin(decl, weapon.can_fire(), weapon.target_flags & 2 != 0, weapon.target_flags & 4 != 0);
        match self.states[index].fire.phase {
            super::combat::FirePhase::Firing => self.play_weapon_action(index, super::weapons::Action::Fire, decl, frame),
            super::combat::FirePhase::OutOfAmmo => self.play_weapon_action(index, super::weapons::Action::OutOfAmmo, decl, frame),
            _ => {}
        }
    }

    /// 140866dc0/d30/140867100: absence of an authored fire scene posts
    /// WEAPON_FIRE. Failure to prepare an existing scene is not that case.
    fn play_weapon_action(&mut self, index: usize, action: super::weapons::Action,
        decl: &super::combat::FireDecl, frame: &mut Frame) {
        let sliding = self.is_active(StateId::SLIDING);
        let path = frame.weapon.as_ref().and_then(|weapon| weapon.scene(action, sliding)).map(str::to_owned);
        let rate = frame.weapon.as_ref().map_or(1.0, |weapon| weapon.scene_playrate(action, frame.attributes));
        let burst = frame.weapon.as_ref().filter(|weapon| weapon.target_flags & 1 != 0 && weapon.magazine > 1)
            .map(|weapon| weapon.magazine - 1);
        let handle = if let (Some(count), Some(path)) = (burst, path.as_ref()) {
            if let Some(scene) = frame.scenes.get(path) {
                match frame.midnight.start_with_variables(scene, frame.md6, rate, frame.animations, 0xf, 0.0,
                    &[("Burst", true)], &[("BurstLoop", count)], frame.random) {
                    Ok(handle) => handle,
                    Err(error) => { frame.midnight.last_start_error = Some(format!("{path}: {error}")); 0 }
                }
            } else { Self::play(Some(path), rate, 0xf, frame) }
        } else if path.is_some() { Self::play(path.as_ref(), rate, 0xf, frame) } else { 0 };
        self.states[index].midnight = handle;
        self.states[index].fire.action.after_scene(handle, &decl.action);
        if let Some(weapon) = frame.weapon.as_mut() {
            weapon.set_scene(handle);
            //866dc0 ->772e20 posts the body's actual additive only when a
            //real Fire scene exists. Burst point keys do not replay recoil.
            if action == super::weapons::Action::Fire && path.is_some() && handle != 0 {
                if let Some(item) = weapon.settings.recoil_alias.as_deref().and_then(|alias| frame.body_additives.index_of(alias)) {
                    let weight = weapon.settings.recoil_weight_attribute.as_deref().map_or(1.0, |tag| frame.attributes.get(tag));
                    if weight > 0.0 {
                        *frame.body_additive_command = Some(super::animation::AnimBasicCommand {
                            idx: item as i32, weight, mode: super::animation::PlayMode::Once,
                            rate: 1.0, start: 0.0, play_on_end: false });
                    }
                }
            }
            if action == super::weapons::Action::Fire {
                weapon.charged_shot = decl.charged_shot_hold_duration <= self.states[index].fire.charge_time;
            }
            if action == super::weapons::Action::Fire && path.is_none() { weapon.emit_fire(); }
        }
    }

    fn update_fire(&mut self, index: usize, decl: &super::combat::FireDecl, frame: &mut Frame) {
        use super::combat::{ActionStep, FirePhase};
        if frame.weapon.is_none() { self.quit(StateId::FIRE_WEAPON); return; }
        let action = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::FIRE_WEAPON)
            .map_or(0, |mapping| mapping.action);
        let button_down = frame.controller.down(action);
        match self.states[index].fire.phase {
            FirePhase::Charging => {
                // 140867330 does not count charge or release before the
                // real charge scene has started; a reload can defer it.
                if self.states[index].midnight == 0 {
                    if !self.is_active(StateId::RELOAD_WEAPON) {
                        self.play_weapon_action(index, super::weapons::Action::Charge, decl, frame);
                    }
                    return;
                }
                self.states[index].fire.charge_time += frame.dt;
                if self.states[index].fire.released(button_down) {
                    self.play_weapon_action(index, super::weapons::Action::Fire, decl, frame);
                }
            }
            FirePhase::TargetDelay | FirePhase::Targeting | FirePhase::TargetAcquired => {
                if self.states[index].fire.released(button_down) {
                    self.play_weapon_action(index, super::weapons::Action::Fire, decl, frame);
                } else if self.states[index].fire.advance_target_delay(frame.dt) {
                    // 140867330 disables the competing target state and
                    // activates the actual sensor16 request. World target
                    // discovery consumes this typed effect; no target is made up.
                    self.quit(StateId(0x4c));
                    if let Some(weapon) = frame.weapon.as_mut() { weapon.begin_targeting(); }
                }
            }
            FirePhase::Firing | FirePhase::OutOfAmmo => {
                if self.states[index].fire.phase == FirePhase::OutOfAmmo && button_down { return; }
                let finished = frame.midnight.finished(self.states[index].midnight);
                if self.states[index].fire.action.update(frame.dt, &decl.action, button_down, finished, false) == ActionStep::Quit {
                    self.quit(StateId::FIRE_WEAPON);
                }
                // The native FireWeapon's action execute-finished vtable
                // slot+b8 is a no-op; the fire key owns the actual shot.
            }
            FirePhase::Inactive => self.quit(StateId::FIRE_WEAPON),
        }
    }

    fn play_from(path: Option<&String>, rate: f32, parts: u8, seconds: f32, frame: &mut Frame) -> u32 {
        let Some(path) = path else { return 0 };
        let Some(scene) = frame.scenes.get(path) else {
            frame.midnight.last_start_error = Some(format!("{path}: scene not loaded"));
            return 0;
        };
        match frame.midnight.start_with_parts_at_seconds(scene, frame.md6, rate, frame.animations, parts, seconds, frame.random) {
            Ok(handle) => handle,
            Err(error) => { frame.midnight.last_start_error = Some(format!("{path}: {error}")); 0 }
        }
    }

    /// The no-NPC-target branch of 14086cbf0. Target-dependent fatalities,
    /// throw hits and blink assaults require their actual sensor owners;
    /// this branch does not manufacture those entities or conditions.
    fn play_slash(&mut self, index: usize, decl: &super::combat::SlashDecl, frame: &mut Frame) {
        let slash = &self.states[index].slash;
        let context = super::slash_rules::Context {
            current_posture: if frame.physics.posture == 3 { 1 } else { frame.physics.posture },
            last_slash_type: slash.last_type,
            time_since_last_slash: (frame.action_time - slash.last_time) as f32,
            is_focused: slash.focused,
            ..Default::default()
        };
        let Some(selection) = self.slash_rules.as_mut().and_then(|rules| rules.select_shared(&context)) else { return };
        let prior = self.states[index].slash.chained_scene;
        let offset = if prior != 0 && !frame.midnight.finished(prior) { selection.chained_start_seconds } else { 0.0 };
        // The native attribute lookup leaves the starting rate at one when
        // this optional attribute is absent; a real stored zero stays zero.
        let rate = frame.attributes.0.get("ARK_ATTRIBUTE_SLASH_SPEED_FACTOR").copied().unwrap_or(1.0);
        let handle = Self::play_from(selection.midnight.as_ref(), rate, 7, offset, frame);
        if handle != 0 {
            // 86c6a0: the left equipment supplies its own response; it is
            // cast to the left arm, independently from the sword/body scene.
            let secondary = if (0..3).contains(&selection.slash_type) {
                let path = frame.equipment_changes.preferred(0)
                    .and_then(|equipment| equipment.slash_midnights[selection.slash_type as usize].clone());
                Self::play_from(path.as_ref(), rate, 2, offset, frame)
            } else { 0 };
            let state = &mut self.states[index];
            state.midnight = handle;
            state.slash.action.after_scene(handle, &decl.action);
            state.slash.chained_scene = handle;
            state.slash.secondary_scene = secondary;
            state.slash.last_time = frame.action_time;
            state.slash.last_type = selection.slash_type;
            state.slash.selected = Some(selection);
            if state.slash.last_type < 3 && !state.slash.dispersion_active {
                frame.attribute_requests.push(decl.dispersion.clone());
                state.slash.dispersion_active = true;
            }
        }
        self.states[index].slash.press_again = false;
    }

    fn activate_slash(&mut self, index: usize, decl: &super::combat::SlashDecl, frame: &mut Frame) {
        let can_focus = !self.is_active(StateId::SLIDING);
        let slash = &mut self.states[index].slash;
        slash.focus_ready = false;
        slash.focus_active = false;
        slash.focused = false;
        slash.can_focus = can_focus;
        slash.release_seen = false;
        slash.press_again = false;
        slash.action.activate(&decl.action, false);
        if !frame.equipment_changes.equipped(0) { self.request_equipment_left(4, StateId::SLASH); }
        // 140854e50's derived OnExecute is 14086c630: a normal swing
        // begins immediately. Holding focus is enabled by an authored key.
        self.play_slash(index, decl, frame);
    }

    fn update_slash(&mut self, index: usize, decl: &super::combat::SlashDecl, frame: &mut Frame) {
        use super::combat::ActionStep;
        let action = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::SLASH)
            .map_or(0, |mapping| mapping.action);
        let down = frame.controller.down(action);
        let finished = frame.midnight.finished(self.states[index].midnight);
        let held = self.states[index].slash.action.elapsed();
        match self.states[index].slash.action.update(frame.dt, &decl.action, down, finished, false) {
            ActionStep::Quit => {
                self.quit(StateId::SLASH);
                return;
            }
            ActionStep::Execute => {
                if self.states[index].slash.focus_active {
                    let slash = &mut self.states[index].slash;
                    slash.focus_active = false;
                    slash.focused = held >= decl.focus_hold_duration;
                    self.play_slash(index, decl, frame);
                }
                if self.states[index].slash.focused {
                    // 86bec0 leaves the previous slash direction LEFT after
                    // a focused release, independently of the selected rule.
                    self.states[index].slash.last_type = 2;
                }
                if self.states[index].slash.action.finish_execution(&decl.action) == ActionStep::Quit {
                    self.quit(StateId::SLASH);
                    return;
                }
            }
            ActionStep::None => {}
        }
        // 14086d060 waits for FOCUSED_ATTACK_CAN_START, keeps focus out
        // of sliding, and starts the actual declared focus animation.
        if self.states[index].slash.focus_ready && !self.states[index].slash.focus_active
            && !self.states[index].slash.focused && !self.states[index].slash.release_seen
        {
            if let Some(path) = decl.action.midnight.as_ref() {
                let secondary = self.states[index].slash.secondary_scene;
                if secondary != 0 { frame.midnight.stop(secondary); }
                let handle = Self::play(Some(path), 1.0, 5, frame);
                let slash = &mut self.states[index].slash;
                slash.secondary_scene = 0;
                slash.focus_active = true;
                slash.press_again = false;
                slash.action.after_scene(handle, &decl.action);
                slash.action.set_wait_for_midnight(false);
                if slash.dispersion_active {
                    let mut request = decl.dispersion.clone();
                    request.added = false;
                    frame.attribute_requests.push(request);
                    slash.dispersion_active = false;
                }
                self.states[index].midnight = handle;
            }
        }
        let slash = &mut self.states[index].slash;
        if !down { slash.release_seen = true; }
        if slash.release_seen && down { slash.press_again = true; }
        if slash.focus_active && self.is_active(StateId::SLIDING) { self.quit(StateId::SLASH); }
    }

    ///86bb60 runs at the controller's authored BF animation window. The
    ///follow-up is admitted through the same queue
    ///and mode/body rules, so it cannot invent a second simultaneous attack.
    fn finish_slash(&mut self, index: usize, frame: &Frame) {
        let action = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::SLASH)
            .map_or(0, |mapping| mapping.action);
        let block = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::BLOCK)
            .map_or(0, |mapping| mapping.action);
        self.quit(StateId::SLASH);
        if frame.controller.down(block) { self.request(StateId::BLOCK); }
        else if self.states[index].slash.press_again && frame.controller.down(action) { self.request(StateId::SLASH); }
        self.states[index].slash.press_again = false;
    }

    fn play_block(&mut self, index: usize, decl: &super::combat::BlockDecl, phase: u8, frame: &mut Frame) {
        let sneak = [StateId::SNEAKING, StateId::AUTOCROUCH, StateId::WALLCROUCH].into_iter().any(|id| self.is_active(id));
        let path = match phase {
            1 => if sneak { decl.midnight_sneak_in.as_ref().or(decl.midnight_in.as_ref()) } else { decl.midnight_in.as_ref() },
            2 => decl.midnight_idle.as_ref(),
            3 => if sneak { decl.midnight_sneak_out.as_ref().or(decl.midnight_out.as_ref()) } else { decl.midnight_out.as_ref() },
            _ => None,
        };
        self.states[index].block.phase = phase;
        if let Some(path) = path {
            let handle = Self::play(Some(path), 1.0, 7, frame);
            self.states[index].midnight = handle;
            self.states[index].block.action.after_scene(handle, &decl.action);
        }
    }

    fn update_block(&mut self, index: usize, decl: &super::combat::BlockDecl, frame: &mut Frame) {
        use super::combat::ActionStep;
        if self.states[index].block.phase != 3 && (1..=4).contains(&self.states[index].block.phase)
            && frame.midnight.finished(self.states[index].midnight)
        { self.play_block(index, decl, 2, frame); }
        match self.states[index].block.left_hand_phase {
            0 => {
                if !frame.equipment_changes.equipped(0) {
                    frame.arms_shown[0] = false;
                    self.code_lock(index, 1, true, frame);
                    self.states[index].block.left_hand_phase = 2;
                } else if self.states[index].block.action.elapsed() >= decl.free_left_hand_duration {
                    self.request_equipment_left(0x81, StateId::BLOCK);
                    self.states[index].block.left_hand_phase = 1;
                }
            }
            1 => {
                self.code_lock(index, 1, true, frame);
                if !frame.equipment_changes.equipped(0) {
                    frame.arms_shown[0] = false;
                    self.states[index].block.left_hand_phase = 2;
                }
            }
            _ => {}
        }
        let action = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::BLOCK)
            .map_or(0, |mapping| mapping.action);
        let down = frame.controller.down(action);
        let finished = frame.midnight.finished(self.states[index].midnight);
        match self.states[index].block.action.update(frame.dt, &decl.action, down, finished, false) {
            ActionStep::Execute => {
                self.play_block(index, decl, 3, frame);
                self.quit(StateId::BLOCK);
            }
            ActionStep::Quit => self.quit(StateId::BLOCK),
            ActionStep::None => {}
        }
    }

    /// The midnight messages of this frame (flushed before DEFAULT):
    /// BREAK_PLAYER_STATE quits the breakable state playing that scene
    /// (FUN_140802e10).
    pub fn handle_message(&mut self, event: &str, handle: u32) {
        if event == "ARK_MDNT_MSG_RECOIL_CAN_BE_INTERRUPTED" {
            if self.recoil_interrupt_diagnostic.is_some() {
                let current=self.state(StateId::RECOIL).map(|state|(state.midnight,state.active)).unwrap_or_default();
                let receipt=self.recoil_interrupt_diagnostic.as_mut().unwrap();
                *receipt=(receipt.0.wrapping_add(1),handle,current.0,current.1);
            }
            //7fbe80 (controller C0 subscriber) forwards directly to854190
            //on its actual Recoil owner. The authored key releases the state
            //lock without stopping the still-blending recovery animation.
            if self.index(StateId::RECOIL).is_some() { self.quit(StateId::RECOIL); }
            return;
        }
        if event=="ARK_MDNT_MSG_TRIGGER_PLAYER_STATE_ACTION" {
            //854fd0/855750 routes the actual0xf6 scene handle to Action.
            for id in self.active.clone() {
                let Some(index)=self.index(id) else{continue;};
                let Kind::CancelAction(declaration)=self.states[index].decl.kind.clone() else{continue;};
                if self.states[index].cancel_action.message(&declaration,handle)==super::combat::ActionStep::Execute
                    && self.states[index].cancel_action.finish_execution(&declaration)==super::combat::ActionStep::Quit {self.quit(id);}
            }
        }
        if event == "ARK_MDNT_MSG_SLASH_CAN_BE_INTERRUPTED" {
            //7fbeb0 checks the actual Slash active flag, not scene completion.
            if self.is_active(StateId::SLASH) { self.queue.push(Command::FinishSlash); }
            return;
        }
        if event == "ARK_MDNT_MSG_FOCUSED_ATTACK_CAN_START" {
            // 14086d3d0, msg154: copy the active Slash's allow-focus flag.
            if let Some(index) = self.index(StateId::SLASH).filter(|index| self.states[*index].active) {
                self.states[index].slash.focus_ready = self.states[index].slash.can_focus;
            }
            return;
        }
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
        //7fbeb0 is a pre-component Midnight callback: publish its actual
        //Quit/Request pair before7f8040 appends this frame's input requests.
        while let Some(position) = self.queue.iter().position(|command| matches!(command, Command::FinishSlash)) {
            self.queue.remove(position);
            if let Some(index) = self.index(StateId::SLASH).filter(|&index| self.states[index].active) {
                self.finish_slash(index, frame);
            }
        }
        self.prioritized_input(frame);
        // 14063bfc0: keep an explicit/empty-magazine reload pending until
        // the state machine can admit its actual reload state.
        if !self.is_active(StateId::RELOAD_WEAPON)
            && frame.weapon.as_mut().is_some_and(|weapon| weapon.take_reload_request())
            && self.allowed(StateId::RELOAD_WEAPON, frame)
        {
            self.request(StateId::RELOAD_WEAPON);
        }
        // Commands queued while activating are processed in the same frame.
        self.process_pending(frame);
        if frame.dt != 0.0 {
            for id in self.active.clone() {
                if let Some(index) = self.index(id).filter(|&i| self.states[i].active) {
                    self.update(index, frame);
                }
            }
        }
    }

    ///8063f0 drains shared accepted requests before806350 changes modes.
    ///No state update, input sampling, clock or physics step is repeated.
    pub(super) fn process_pending(&mut self, frame: &mut Frame) {
        self.process_commands(frame);
        for _ in 0..16 {
            if self.queue.is_empty() { break; }
            self.process_commands(frame);
        }
    }

    ///7f8040 complements controller7f8270 for every action, not only
    ///INTERACT: select the highest-priority currently available pressed
    ///mapping per action, keeping the first declaration on equal priority.
    fn prioritized_input(&mut self, frame: &Frame) {
        let mut choices: [Option<(StateId, i32)>; 64] = [None; 64];
        let mut order = [0usize; 64];
        let mut count = 0;
        for mapping in &frame.controller.mapping {
            if mapping.usage != super::controller::ButtonUsage::Pressed
                || mapping.priority_when_available < 0
                || !frame.controller.pressed(mapping.action)
                || !self.allowed(mapping.state, frame) { continue; }
            let action = mapping.action as usize & 63;
            match choices[action] {
                None => {
                    order[count] = action;
                    count += 1;
                    choices[action] = Some((mapping.state, mapping.priority_when_available));
                }
                Some((_, priority)) if priority < mapping.priority_when_available => {
                    choices[action] = Some((mapping.state, mapping.priority_when_available));
                }
                _ => {}
            }
        }
        for &action in &order[..count] {
            if let Some((state, _)) = choices[action] { self.request(state); }
        }
    }

    /// FUN_140804850.
    fn process_commands(&mut self, frame: &mut Frame) {
        let queue = std::mem::take(&mut self.queue);
        let mut snapshot = self.active.clone();
        let mut accepted: Vec<(StateId, Option<f32>)> = Vec::new();
        let mut quits = Vec::new();
        let mut read_note_requested = false;
        let mut grab_requested = false;
        let mut loot_requested = false;
        let mut blink_requested = false;
        let mut grapple_requested = false;
        for command in queue {
            match command {
                Command::RequestEquipment { side, flags, source } => {
                    frame.equipment_changes.request_stored(side, flags, source);
                    self.accept(if side == 0 { StateId::EQUIP_LEFT } else { StateId::EQUIP_RIGHT }, None, &snapshot, &mut accepted, frame);
                }
                Command::Quit(id) => {
                    if let Some(position) = accepted.iter().position(|&(a, _)| a == id) {
                        accepted.remove(position);
                    } else if let Some(position) = snapshot.iter().position(|&a| a == id) {
                        snapshot.remove(position);
                        quits.push(id);
                    }
                }
                Command::FinishSlash => {
                    if let Some(index) = self.index(StateId::SLASH).filter(|&index| self.states[index].active) {
                        self.finish_slash(index, frame);
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
                Command::RequestReadNote => {
                    read_note_requested = true;
                    self.accept(StateId::READ_NOTE, None, &snapshot, &mut accepted, frame);
                }
                Command::Request(id) => {
                    blink_requested |= id == StateId::BLINKING;
                    loot_requested |= id == StateId::LOOT_OBJECT;
                    grab_requested |= matches!(self.state(id).map(|state|&state.decl.kind),Some(Kind::GrabObject(_)));
                    grapple_requested |= id == StateId::GRAPPLE;
                    if id==StateId::GRAB_OBJECT && frame.carrying.tracing_enabled() {
                        //One bounded diagnostic receipt, not a replacement
                        //admission decision. Bits name all actual48 gates.
                        let gates=self.state(id).map_or(0,|state| {
                            1 | (u32::from(frame.carrying.pending())<<1)
                                | (u32::from(frame.carrying.grab_state()==Some(id))<<2)
                                | (u32::from(frame.body_parts.can_activate(state.decl.flags.body_parts,state.decl.flags.unlocks_code_locked_body_parts))<<3)
                                | (u32::from(match &state.decl.kind{Kind::GrabObject(d)=>frame.carrying.release.can_activate(frame.action_time,&d.action),_=>false})<<4)
                                | (u32::from(snapshot.iter().all(|&from|self.rule(from,id).allow))<<5)
                                | (u32::from(accepted.iter().all(|&(from,_)|self.rule(from,id).allow))<<6)
                        });
                        frame.carrying.trace("state48-gates(exists,pending,id,parts,cooldown,oldrules,newrules)",None,gates as i32,gates==127);
                    }
                    self.accept(id, None, &snapshot, &mut accepted, frame);
                    if id==StateId::GRAB_OBJECT && frame.carrying.tracing_enabled() {
                        frame.carrying.trace("state-request48",None,id.0,accepted.iter().any(|(state,_)|*state==id));
                    }
                },
                Command::RequestSlide(speed) => self.accept(StateId::SLIDING, Some(speed), &snapshot, &mut accepted, frame),
                Command::RequestCatFall(damage) => self.accept(StateId::CATFALL, Some(damage), &snapshot, &mut accepted, frame),
            }
        }
        // 857a30 consumes the saved Falling instance before that instance
        // can receive a new 865b90 request. This host batches request hooks
        // before deactivation, so resolve that one exit callback first;
        // otherwise a new FALLING request would erase its existing debt.
        if self.is_active(StateId::BLINKING)
            && (quits.contains(&StateId::BLINKING)
                || accepted.iter().any(|&(id, _)| self.rule(StateId::BLINKING, id).quit)) {
            self.land_blink_fall_debt(frame);
        }
        for &(id, _) in &accepted {
            if id == StateId::MANTLE {
                //8603b0 runs at request admission, before Blink is withdrawn.
                frame.powers.blink_mantle_requested = self.is_active(StateId::BLINKING);
            }
            if id == StateId::BLINKING {
                frame.powers.blink_from_sprinting = self.is_active(StateId::SPRINTING);
                frame.powers.blink_from_sliding = self.is_active(StateId::SLIDING);
                frame.powers.blink_from_falling = self.is_active(StateId::FALLING);
            }
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
        if read_note_requested { self.read_note.reject(); }
        if loot_requested && !self.is_active(StateId::LOOT_OBJECT) { self.world_pickup.reject(); }
        if grab_requested {
            let active=self.active.iter().any(|id|matches!(self.state(*id).map(|state|&state.decl.kind),Some(Kind::GrabObject(_))));
            frame.carrying.trace("state-request48-finished",None,48,active);
            if !active {frame.carrying.cancel_grab();}
        }
        if grapple_requested && !self.is_active(StateId::GRAPPLE) {
            frame.powers.grapple_pending = false;
            if let Some(run) = frame.powers.grapple.as_mut() { run.finished = true; run.aborted = true; }
        }
        if blink_requested && !self.is_active(StateId::BLINKING) {
            // A rejected parameterized request returns its real payload to
            // the world owner; no timeout or synthetic state owns it.
            frame.powers.blink_pending = false;
            frame.powers.blink_finished = true;
            frame.powers.blink_aborted = true;
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
        if let Kind::CancelAction(declaration)=&self.states[index].decl.kind {
            let declaration=declaration.clone();
            let active=self.states[index].active;
            self.states[index].cancel_action.requested_again(&declaration,active);
            return;
        }
        if let Kind::PlayHeartBark(decl)=&self.states[index].decl.kind {
            let decl=decl.action.clone();let active=self.states[index].active;
            self.states[index].heart_bark.requested_again(&decl,active);return;
        }
        if let Kind::LootObject(declaration)=&self.states[index].decl.kind {
            self.world_pickup.action.requested_again(&declaration.action,self.states[index].active);
            return;
        }
        if let Kind::Slash(decl) = &self.states[index].decl.kind {
            let decl = decl.action.clone();
            let active = self.states[index].active;
            self.states[index].slash.action.requested_again(&decl, active);
            return;
        }
        if let Kind::Block(decl) = &self.states[index].decl.kind {
            let decl = decl.action.clone();
            let active = self.states[index].active;
            self.states[index].block.action.requested_again(&decl, active);
            return;
        }
        if let Kind::FireWeapon(decl) = &self.states[index].decl.kind {
            let decl = decl.action.clone();
            let active = self.states[index].active;
            self.states[index].fire.action.requested_again(&decl, active);
            return;
        }
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
        if !self.states[index].active && self.states[index].decl.flags.cancelable {
            self.cancelable_count=self.cancelable_count.wrapping_add(1);
        }
        self.states[index].active = true;
        let decl = self.states[index].decl.clone();
        // Base Activate FUN_140853d40: a LEGS owner wants the legs shown
        // (FUN_140774900 +1).
        if decl.flags.body_parts & LEGS != 0 {
            *frame.desired_legs += 1;
        }
        match &decl.kind {
            Kind::CancelAction(action)=>{
                if self.states[index].cancel_action.activate(action,false)==super::combat::ActionStep::Execute {
                    let handle=Self::play(action.midnight.as_ref(),1.,decl.flags.body_parts,frame);
                    self.states[index].midnight=handle;
                    self.states[index].cancel_action.after_scene(handle,action);
                }
                return;
            }
            Kind::GrappleCatch(declaration)=>{self.grapple_catch.activate(declaration);return;}
            Kind::PlayHeartBark(declaration) => {self.activate_heart_bark(index,declaration,frame);return;}
            Kind::LootObject(declaration) => {
                self.world_pickup.activate();
                if self.world_pickup.action.activate(&declaration.action,false)==super::combat::ActionStep::Execute {
                    self.start_world_pickup_scene(index,declaration,frame);
                }
                return;
            }
            Kind::GrabObject(_) => {frame.carrying.begin_grab();return;}
            Kind::HoldObject(_) => {
                frame.carrying.attach();
                frame.arms_shown[1]=false;
                self.request_equipment_right(0x21,decl.id);
                if frame.carrying.held.as_ref().and_then(|target|target.definition.hold.as_ref()).is_some_and(|props|props.both_hands) {
                    frame.arms_shown[0]=false;self.request_equipment_left(0x21,decl.id);
                }
                return;
            }
            Kind::ReleaseObject(action) => {
                frame.carrying.release.activate(action,false);
                let handle=Self::play(action.midnight.as_ref(),1.,decl.flags.body_parts,frame);
                self.states[index].midnight=handle;
                frame.carrying.release.after_scene(handle,action);
                self.execute_object_release(index,frame);
                return;
            }
            Kind::Slash(slash) => {
                self.activate_slash(index, slash, frame);
                return;
            }
            Kind::Recoil(action) => {
                self.states[index].recoil.action.activate(action,false);
                let path=self.states[index].recoil.selection.as_ref()
                    .and_then(|selection|selection.recoil_midnights.get(self.states[index].recoil.kind))
                    .and_then(Option::as_ref).or(action.midnight.as_ref()).cloned();
                let handle=Self::play(path.as_ref(),1.,7,frame);
                self.states[index].midnight=handle;
                self.states[index].recoil.action.after_scene(handle,action);
                return;
            }
            Kind::Block(block) => {
                self.states[index].block.action.activate(&block.action, false);
                self.states[index].block.left_hand_phase = 0;
                self.play_block(index, block, 1, frame);
                return;
            }
            Kind::EquipChange(equipment) => {
                if let Some(side) = equipment.side().filter(|side| frame.equipment_changes.requests_other_hand(*side)) {
                    if side == 0 { self.request_equipment_right(4, StateId::NULL); }
                    else { self.request_equipment_left(4, StateId::NULL); }
                }
                self.advance_equipment_changes(frame);
                return;
            }
            Kind::DrawEquipment(left) => {
                let side = usize::from(!*left);
                let held = if *left { frame.controller.pressed(20) } else { frame.controller.pressed(19) || frame.controller.pressed(21) };
                let equipment = frame.equipment_changes.preferred(side);
                self.states[index].draw_pending = if held { equipment.map_or(StateId::NULL, |equipment| equipment.action_state) } else { StateId::NULL };
                let flags = if held && self.states[index].draw_pending != StateId::NULL
                    && equipment.is_some_and(|equipment| equipment.archetype != 2) { 0x18 } else { 8 };
                if *left { self.request_equipment_left(flags, StateId::NULL); }
                else { self.request_equipment_right(flags, StateId::NULL); }
                return;
            }
            Kind::Undraw(_) => {
                self.states[index].undraw_elapsed = 0.0;
                return;
            }
            Kind::ThrowWeapon => { self.run_throw_weapon(index,frame,0);return; }
            Kind::FireWeapon(fire) => {
                self.states[index].fire.action.activate(&fire.action, false);
                self.activate_fire(index, fire, frame);
                return;
            }
            Kind::CancelFireWeapon => {
                self.quit(StateId::FIRE_WEAPON);
                // 140866370 clears native ACTION_FIRE_LEFT (20).
                frame.controller.clear_down(20);
                self.quit(decl.id);
                return;
            }
            Kind::RequestReloadWeapon => {
                if let Some(weapon) = frame.weapon.as_mut() { weapon.request_reload(); }
                self.quit(decl.id);
                return;
            }
            Kind::ReloadWeapon => {
                if frame.throwable.is_some(){self.play_throwable_reload(index,frame);return;}
                let sliding = self.is_active(StateId::SLIDING);
                let path = frame.weapon.as_ref().and_then(|weapon| weapon.scene(super::weapons::Action::Reload, sliding))
                    .map(str::to_owned);
                if path.is_some() {
                    // 1408454f0 casts left arm + left equipment (mask 0xa).
                    let rate = frame.weapon.as_ref().map_or(1.0, |weapon|
                        weapon.scene_playrate(super::weapons::Action::Reload, frame.attributes));
                    let handle = Self::play(path.as_ref(), rate, 0xa, frame);
                    self.states[index].midnight = handle;
                    if let Some(weapon) = frame.weapon.as_mut() { weapon.set_scene(handle); weapon.reload_requested = false; }
                } else {
                    if let Some(weapon) = frame.weapon.as_mut() { weapon.emit_reload(); weapon.reload_requested = false; }
                }
                return;
            }
            _ => {}
        }
        if let Kind::CatFall(catfall) = &decl.kind {
            frame.midnight.last_start_error = None;
            // FUN_14085a910 (equipment, the wind blast, fail chance and
            // footsteps are not ported).
            self.states[index].damage = slide_speed.unwrap_or(0.0);
            frame.physics.set_posture(POSTURE_LEAN_FORWARD, 0.0);
            frame.camera.constraints = camera::constraints(catfall.look_at_params);
            frame.camera.set_target_mode(MODE_CATFALL, -1.0);
            // The catfall sensor's hand hits choose the shown hands.
            let mut shown = [frame.catfall_sensor.left_hit, frame.catfall_sensor.right_hit];
            frame.catfall_sensor.enable(false);
            let high = self.states[index].damage >= catfall.damage_threshold && catfall.high_damage_midnight.is_some();
            // 14085b490 distinguishes an equipped entity from an active
            // equip action: while that action exists, a non-null action
            // scene means the hand is not yet available. Carry/grapple
            // owners supply additional restore reasons when implemented;
            // no carried entity is fabricated here.
            let equipped = [StateId::EQUIP_LEFT, StateId::EQUIP_RIGHT].map(|id| {
                let side = usize::from(id == StateId::EQUIP_RIGHT);
                if self.is_active(id) { !frame.equipment_transitioning[side] }
                else { frame.equipment_changes.equipped(side) }
            });
            self.states[index].catfall_restore = equipped;
            self.request_equipment_left(0x21, StateId::CATFALL);
            if !equipped[1] || high { self.request_equipment_right(0x21, StateId::CATFALL); }
            let (scene, arm_scenes) = if high {
                shown = [true, true];
                (catfall.high_damage_midnight.as_ref(), &catfall.high_damage_arm_midnights)
            } else {
                (catfall.low_damage_midnight.as_ref(), &catfall.low_damage_arm_midnights)
            };
            let mut arm_scenes = arm_scenes.each_ref().map(Option::as_ref);
            if !high && frame.equipment_archetypes[1] == Some(1) {
                let [normal, sneaking] = catfall.low_damage_sword_midnights.each_ref().map(Option::as_ref);
                arm_scenes[1] = if frame.controller.toggled(frame.controller.sneak_action) {
                    sneaking.or(normal).or(arm_scenes[1])
                } else { normal.or(arm_scenes[1]) };
            }
            frame.catfall_sensor.last_selected_hands = Some(shown);
            // FUN_140774a10: hide the arms that are not shown.
            for (side, show) in shown.into_iter().enumerate() {
                if !show {
                    frame.arms_shown[side] = false;
                }
            }
            frame.follow_slope.midnight_requests += 1;
            let rate = frame.attributes.0.get("ARK_ATTRIBUTE_CATFALL_SPEED_FACTOR").copied().unwrap_or(1.0);
            // FUN_14085b400: with both arm scenes the arms play them (masks 2
            // and 4) beside the main one (1); otherwise the main one casts
            // all (7).
            if arm_scenes[0].is_some() && arm_scenes[1].is_some() {
                Self::play(arm_scenes[0], rate, 2, frame);
                Self::play(arm_scenes[1], rate, 4, frame);
                self.states[index].midnight = Self::play(scene, rate, 1, frame);
            } else {
                self.states[index].midnight = Self::play(scene, rate, 7, frame);
            }
            return;
        }
        if let Kind::Mantle(mantle) = &decl.kind {
            if frame.powers.blink_mantle_requested && !frame.powers.mantle_sensor_swapped {
                if let Some(sensor) = frame.powers.blink_mantle_sensor.as_mut() {
                    std::mem::swap(frame.mantle_sensor, sensor);
                    frame.powers.mantle_sensor_swapped = true;
                }
            }
            frame.powers.blink_mantle_requested = false;
            frame.midnight.last_start_error = None;
            self.activate_mantle(index, mantle, frame);
            return;
        }
        if matches!(decl.kind, Kind::PowerWheel) {
            //692010: common Activate then request game-logic state8.
            self.wheel_requested=true;
            return;
        }
        if matches!(decl.kind, Kind::ReadNote) {
            // 844bf0 ->8450b0: only an accepted state owns the descriptor.
            self.read_note.activate();
            return;
        }
        if matches!(decl.kind,Kind::UseHeart(_)) {self.activate_heart(index);return;}
        if let Kind::UsePower(power) = &decl.kind {
            let sliding = self.is_active(StateId::SLIDING);
            if !super::power_states::activate_use(frame, sliding, power) { self.quit(decl.id); }
            return;
        }
        if let Kind::Grapple(grapple) = &decl.kind {
            super::grapple_state::activate(frame, grapple); return;
        }
        if matches!(decl.kind, Kind::Blinking) {
            super::power_states::activate_blink(frame);
            //857300 quits ordinary Slash through854190; the focused-hold
            //branch has a separate Blink-assault policy and stays owned.
            if self.is_active(StateId::SLASH) && !self.melee_focus_active() {
                self.quit(StateId::SLASH);
            }
            return;
        }
        if matches!(decl.kind, Kind::SwimMantle) {
            // 140845b50 requests the outgoing mode and starts its mantle.
            *frame.swim_mantle_exit = true;
            self.quit(decl.id);
            return;
        }
        if let Kind::Dead(dead) = &decl.kind {
            // 14085ba50 starts the drowning countdown and the animated
            // drowning camera when the fatal damage type is WATER.
            self.states[index].death_timer = dead.duration;
            self.states[index].death_transition_pending = true;
            self.states[index].death_fx_pending = true;
            frame.camera.set_target_mode(MODE_DROWNING, dead.look_duration);
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
                self.states[index].fall.catfall_taken = false;
                frame.catfall_sensor.enable(true);
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
            Kind::Swimming(swimming) => {
                // 140845ed0: entering the normal swim state carries a
                // fraction of the player's velocity as a decaying dive.
                let speed = frame.velocity.length();
                let direction = frame.velocity / speed.max(f32::MIN_POSITIVE);
                let impulse = if swimming.id == StateId::SPRINTING { 0.0 } else {
                    speed * frame.physics.speed_scale() * swimming.diving_speed_ratio
                };
                self.states[index].swimming_dive = (direction, impulse);
            }
            // Activated above, without the navigation base.
            Kind::Leaning(_) | Kind::CatFall(_) | Kind::Mantle(_) | Kind::Vault(_) | Kind::SwimMantle | Kind::ReadNote | Kind::PowerWheel | Kind::UseHeart(_) | Kind::PlayHeartBark(_) | Kind::UsePower(_) | Kind::Blinking | Kind::Grapple(_) | Kind::Dead(_)
                | Kind::FireWeapon(_) | Kind::ThrowWeapon | Kind::Recoil(_) | Kind::CancelFireWeapon | Kind::RequestReloadWeapon | Kind::ReloadWeapon
                | Kind::EquipChange(_) | Kind::DrawEquipment(_) | Kind::Undraw(_) | Kind::Slash(_) | Kind::Block(_) | Kind::LootObject(_) | Kind::GrabObject(_) | Kind::HoldObject(_) | Kind::ReleaseObject(_) | Kind::GrappleCatch(_) | Kind::CancelAction(_) => {}
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

    ///846330: underwater empty hands yield to a real body Midnight or
    ///one/two-handed hold; at the surface only a left-part action claims it.
    fn swimming_arms(&self, frame: &mut Frame, leaving: bool) {
        if frame.underwater && frame.midnight.playing().is_none() {
            let requested = |id| self.is_active(id) || self.queue.iter()
                .any(|command| matches!(command, Command::Request(state) if *state == id));
            let two = requested(StateId::HOLD_TWO_HAND);
            let one = requested(StateId::HOLD_ONE_HAND);
            *frame.arms_shown = [!two, !two && !one];
        } else {
            let left = self.active.iter().filter_map(|id| self.state(*id))
                .any(|state| state.decl.flags.body_parts & 2 != 0);
            *frame.arms_shown = [left || leaving, leaving];
        }
    }

    /// Slot 12 Deactivate.
    fn deactivate(&mut self, index: usize, frame: &mut Frame) {
        if self.states[index].active && self.states[index].decl.flags.cancelable {
            self.cancelable_count=self.cancelable_count.wrapping_sub(1);
        }
        self.states[index].active = false;
        // Base Deactivate FUN_1408541a0.
        if self.states[index].decl.flags.body_parts & LEGS != 0 {
            *frame.desired_legs -= 1;
        }
        match self.states[index].decl.kind.clone() {
            Kind::CancelAction(_)=>{
                self.states[index].cancel_action.deactivate(frame.action_time);
                self.states[index].midnight=0;
                return;
            }
            Kind::GrappleCatch(_)=>{self.grapple_catch.deactivate();return;}
            Kind::Recoil(_) => {
                self.states[index].recoil.action.deactivate(frame.action_time);
                self.states[index].recoil.selection=None;
                self.states[index].recoil.kind=0;
                self.states[index].midnight=0;
                return;
            }
            Kind::Slash(decl) => {
                if self.states[index].slash.focus_active {
                    let slash = &self.states[index].slash;
                    let context = super::slash_rules::Context {
                        is_focused: true,
                        current_posture: if frame.physics.posture == 3 { 1 } else { frame.physics.posture },
                        last_slash_type: slash.last_type,
                        time_since_last_slash: (frame.action_time - slash.last_time) as f32,
                        ..Default::default()
                    };
                    if let Some(selection) = self.slash_rules.as_mut().and_then(|rules| rules.select_shared(&context)) {
                        if selection.cancel_midnight.is_some() {
                            Self::play(selection.cancel_midnight.as_ref(), 1.0, 5, frame);
                        }
                    }
                }
                self.states[index].slash.action.deactivate(frame.action_time);
                self.states[index].slash.focus_active = false;
                self.states[index].slash.focus_ready = false;
                self.states[index].slash.focused = false;
                if self.states[index].slash.dispersion_active {
                    let mut request = decl.dispersion.clone();
                    request.added = false;
                    frame.attribute_requests.push(request);
                    self.states[index].slash.dispersion_active = false;
                }
                self.states[index].midnight = 0;
                return;
            }
            Kind::LootObject(_) => {
                self.world_pickup.offer_from_exit();
                self.world_pickup.action.deactivate(frame.action_time);
                return;
            }
            Kind::GrabObject(_) => {frame.carrying.cancel_grab();return;}
            Kind::HoldObject(_) => {
                let left=frame.carrying.held.as_ref().and_then(|target|target.definition.hold.as_ref()).is_some_and(|props|props.both_hands);
                frame.carrying.leave_hold();frame.arms_shown[1]=true;
                self.request_equipment_right(0x16,self.states[index].decl.id);
                if left {frame.arms_shown[0]=true;self.request_equipment_left(0x16,self.states[index].decl.id);}
                return;
            }
            Kind::ReleaseObject(_) => {frame.carrying.release.deactivate(frame.action_time);return;}
            Kind::Block(_) => {
                if self.states[index].block.left_hand_phase == 2 { frame.arms_shown[0] = true; }
                self.code_lock(index, 1, false, frame);
                if self.states[index].block.left_hand_phase > 0 {
                    self.request_equipment_left(6, StateId::BLOCK);
                }
                self.states[index].block.left_hand_phase = 0;
                self.states[index].block.phase = 0;
                self.states[index].block.action.deactivate(frame.action_time);
                self.states[index].midnight = 0;
                return;
            }
            Kind::EquipChange(_) | Kind::DrawEquipment(_) | Kind::Undraw(_) => return,
            Kind::ThrowWeapon => { self.run_throw_weapon(index,frame,2);return; }
            Kind::FireWeapon(_) => {
                // 140866a20 stops only an interrupted charging scene.
                if self.states[index].fire.phase == super::combat::FirePhase::Charging {
                    frame.midnight.stop(self.states[index].midnight);
                }
                if let Some(weapon) = frame.weapon.as_mut() {
                    weapon.finish_scene(self.states[index].midnight);
                    if weapon.targeting_requested { weapon.end_targeting(); }
                }
                self.states[index].fire.phase = super::combat::FirePhase::Inactive;
                self.states[index].fire.action.deactivate(frame.action_time);
                self.states[index].midnight = 0;
                return;
            }
            Kind::CancelFireWeapon | Kind::RequestReloadWeapon => return,
            Kind::ReloadWeapon => {
                if let Some(weapon) = frame.weapon.as_mut() { weapon.finish_scene(self.states[index].midnight); }
                if let Some(weapon) = frame.throwable.as_mut() { if weapon.active_scene==self.states[index].midnight{weapon.active_scene=0;} }
                self.states[index].midnight = 0;
                return;
            }
            Kind::CatFall(_) => {
                // FUN_14085b1e0 (the scene keeps playing): show the hidden
                // arms again.
                *frame.arms_shown = [true; 2];
                // 14085b1e0 uses the exact fast restore request, not an
                // inventory add or a fresh equipment entity acquisition.
                if self.states[index].catfall_restore[0] { self.request_equipment_left(0x46, StateId::CATFALL); }
                if self.states[index].catfall_restore[1] { self.request_equipment_right(0x46, StateId::CATFALL); }
                self.states[index].catfall_restore = [false; 2];
                let crouch = frame.controller.toggled(frame.controller.sneak_action);
                self.request(if crouch { StateId::SNEAKING } else { StateId::RUNNING });
                frame.follow_slope.midnight_requests -= 1;
                return;
            }
            Kind::Swimming(_) => { self.swimming_arms(frame, true); },
            Kind::ReadNote => { self.read_note.deactivate(); return; },
            Kind::PowerWheel => { self.wheel_requested=false; return; },
            Kind::UseHeart(_) => {self.deactivate_heart(index,frame);return;},
            Kind::PlayHeartBark(_) => {self.states[index].heart_bark.deactivate(frame.action_time);return;},
            Kind::UsePower(_) => {
                let sliding = self.is_active(StateId::SLIDING);
                super::power_states::deactivate_use(frame, sliding);
                self.pending_sounds.append(&mut frame.powers.sound_events);
                return;
            },
            Kind::Blinking => {
                self.land_blink_fall_debt(frame);
                super::power_states::deactivate_blink(frame);
                return;
            },
            Kind::Grapple(grapple) => {
                self.land_falling_if_contact(frame);
                super::grapple_state::deactivate(frame, &grapple); return;
            },
            Kind::SwimMantle => return,
            Kind::Mantle(_) => {
                // FUN_14085fd40.
                if self.states[index].mantle.equipment_restore[0] { self.request_equipment_left(0x46, StateId::MANTLE); }
                if self.states[index].mantle.equipment_restore[1] { self.request_equipment_right(0x46, StateId::MANTLE); }
                self.states[index].mantle.equipment_restore = [false; 2];
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
                if frame.powers.mantle_sensor_swapped {
                    if let Some(sensor) = frame.powers.blink_mantle_sensor.as_mut() { std::mem::swap(frame.mantle_sensor, sensor); }
                    frame.powers.mantle_sensor_swapped = false;
                }
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
                if !self.states[index].fall.catfall_taken {
                    frame.catfall_sensor.enable(false);
                }
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
            Kind::Swimming(_) => {
                frame.physics.set_water_motion(Vec3::ZERO, None);
                *frame.arms_shown = [true; 2];
            }
            _ => {}
        }
    }

    /// 140858010: restore velocity only in the native support/sprint/slide/
    /// vault branches; the target navigation state owns the final posture.
    fn finish_blink_navigation(&mut self, target: StateId, frame: &mut Frame) {
        let Some(motion) = frame.powers.blink.as_ref() else { return };
        let velocity = motion.prior_velocity;
        let mut selected = target;
        //858010 tests controller+2d0 PLAYER_MODE, not possession+5d0.
        //The admitted modes here are Default0 and Swim1; Swimming resumes
        //its target navigation without the ground/falling recovery branch.
        let falling_support = target != StateId::MANTLE && !frame.swimming && !frame.physics.near_ground();
        if target != StateId::MANTLE && !frame.swimming && frame.physics.near_ground() { frame.controller.support_state = SupportState::Supported; }
        if falling_support {
            // Force=true in 1407fbb20 returns FALLING even when the prior
            // support state was already unsupported.
            frame.controller.support_state = SupportState::Unsupported;
            frame.requests.push_linear(0, velocity);
            selected = StateId::FALLING;
            self.request(selected);
        } else if target == StateId::VAULT {
            frame.requests.push_linear(0, velocity); self.request(selected);
        } else if target == StateId::MANTLE {
            frame.requests.push_linear(0, Vec3::ZERO); self.request(selected);
        } else if frame.powers.blink_from_sprinting {
            frame.requests.push_linear(0, velocity); selected = StateId::SPRINTING; self.request(selected);
        } else if frame.powers.blink_from_sliding && self.slide_available(velocity.truncate().length()) {
            frame.requests.push_linear(0, velocity); selected = StateId::SLIDING;
            self.queue.push(Command::RequestSlide(velocity.truncate().length()));
        } else if target != StateId::AUTOCROUCH && frame.controller.toggled(frame.controller.sneak_action) && self.available(StateId::SNEAKING, frame) {
            frame.requests.push_linear(0, Vec3::ZERO); selected = StateId::SNEAKING; self.request(selected);
        } else { frame.requests.push_linear(0, Vec3::ZERO); self.request(selected); }
        if let Some(state) = self.state(selected) {
            if matches!(state.decl.kind, Kind::Running | Kind::Sneaking | Kind::Sprinting(_) | Kind::Sliding(_) | Kind::Slipping(_)
                | Kind::AutoCrouch | Kind::Jumping(_) | Kind::Falling(_) | Kind::Swimming(_)) {
                // 140868e60 reapplies an already active navigation posture;
                // allowSelf=false must not leave the temporary Blink shape.
                frame.physics.set_posture(state.decl.navigation.posture, 0.0);
                frame.body.enter(&state.decl.navigation.loco_stance_name);
            }
        }
    }

    /// 1408451e0 releases the real held actor, then quits both states.
    fn execute_object_release(&mut self,index:usize,frame:&mut Frame) {
        if self.is_active(StateId(75)){return;}
        if frame.carrying.execute_release(frame.controller.pressed(22),frame.controller.pressed(19),frame.controller.possession_state(),false) {
            if let Some(target)=&frame.carrying.held {self.quit(target.hold_state());}
        }
        self.quit(self.states[index].decl.id);
    }

    /// Slot 13 Update.
    fn update(&mut self, index: usize, frame: &mut Frame) {
        let kind = self.states[index].decl.kind.clone();
        let id = self.states[index].decl.id;
        match kind {
            Kind::CancelAction(declaration)=>{
                use super::combat::ActionStep;
                let action=frame.controller.mapping.iter().find(|m|m.state==id).map_or(0,|m|m.action);
                let finished=frame.midnight.finished(self.states[index].midnight);
                let step=self.states[index].cancel_action.update(frame.dt,&declaration,frame.controller.down(action),finished,false);
                if step==ActionStep::Quit || (step==ActionStep::Execute
                    && self.states[index].cancel_action.finish_execution(&declaration)==ActionStep::Quit){self.quit(id);}
            }
            Kind::GrappleCatch(declaration)=>{self.grapple_catch.update(frame.dt,frame.controller.pressed(declaration.action),frame.camera.eye,frame.camera_axis.x_axis);}
            Kind::LootObject(declaration) => {
                use super::combat::ActionStep;
                let action=frame.controller.mapping.iter().find(|mapping|mapping.state==id).map_or(0,|mapping|mapping.action);
                let finished=frame.midnight.finished(self.states[index].midnight);
                match self.world_pickup.action.update(frame.dt,&declaration.action,frame.controller.down(action),finished,false) {
                    ActionStep::Quit=>self.quit(id),
                    ActionStep::Execute if !self.world_pickup.scene_started=>self.start_world_pickup_scene(index,&declaration,frame),
                    //Loot's +b8 action callback is actually0998b0 (no-op).
                    ActionStep::Execute|ActionStep::None=>{}
                }
                if self.world_pickup.target().is_none(){self.quit(id);}
            }
            Kind::GrabObject(_) => {if !frame.carrying.pending(){self.quit(id);}}
            Kind::HoldObject(_) => {if !frame.carrying.has_held(){self.quit(id);}}
            Kind::ReleaseObject(decl) => {
                use super::combat::ActionStep;
                let action=frame.controller.mapping.iter().find(|mapping|mapping.state==id).map_or(0,|mapping|mapping.action);
                let handle=self.states[index].midnight;
                match frame.carrying.release.update(frame.dt,&decl,frame.controller.down(action),frame.midnight.finished(handle),false) {
                    ActionStep::Execute=>self.execute_object_release(index,frame),
                    ActionStep::Quit=>self.quit(id),ActionStep::None=>{}
                }
            }
            Kind::Recoil(decl) => {
                use super::combat::ActionStep;
                let finished=frame.midnight.finished(self.states[index].midnight);
                let step=self.states[index].recoil.action.update(frame.dt,&decl,false,finished,false);
                if step==ActionStep::Quit || (step==ActionStep::Execute
                    && self.states[index].recoil.action.finish_execution(&decl)==ActionStep::Quit) {self.quit(id);}
            }
            Kind::Slash(decl) => self.update_slash(index, &decl, frame),
            Kind::Block(decl) => self.update_block(index, &decl, frame),
            Kind::EquipChange(_) => self.advance_equipment_changes(frame),
            Kind::DrawEquipment(left) => {
                let pending = self.states[index].draw_pending;
                if pending == StateId::NULL { self.quit(id); }
                else if frame.equipment_changes.equipped(usize::from(!left)) {
                    self.request(pending);
                    self.states[index].draw_pending = StateId::NULL;
                }
            },
            Kind::Undraw(hold) => {
                let action = frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::UNDRAW)
                    .map_or(0, |mapping| mapping.action);
                if !frame.controller.down(action) { self.quit(id); return; }
                // 802f90 checks active states' own body-part masks; the
                // left PLACE_WEAPON_READY state61 is the original exception.
                let left = self.active.iter().filter_map(|id| self.state(*id))
                    .any(|state| state.decl.flags.body_parts & 2 != 0) && !self.is_active(StateId(61));
                let right = self.active.iter().filter_map(|id| self.state(*id))
                    .any(|state| state.decl.flags.body_parts & 4 != 0);
                if left || right { self.states[index].undraw_elapsed = 0.0; }
                else {
                    if self.states[index].undraw_elapsed >= hold {
                        let flags = if !frame.equipment_changes.equipped(0) && !frame.equipment_changes.equipped(1) { 4 } else { 0 };
                        self.request_equipment_left(flags, StateId::NULL);
                        self.request_equipment_right(flags, StateId::NULL);
                        self.quit(id);
                    }
                    self.states[index].undraw_elapsed += frame.dt;
                }
            },
            Kind::ThrowWeapon => self.run_throw_weapon(index,frame,1),
            Kind::FireWeapon(decl) => self.update_fire(index, &decl, frame),
            Kind::CancelFireWeapon | Kind::RequestReloadWeapon => {},
            Kind::ReloadWeapon => {
                if frame.midnight.finished(self.states[index].midnight) { self.quit(id); }
            }
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
            Kind::Swimming(decl) => {
                self.navigation_update(index, frame);
                self.swimming_arms(frame, false);
                if let Some((velocity, max_z)) = frame.swimming_motion {
                    frame.physics.set_water_motion(velocity, Some(max_z));
                }
                let dive = &mut self.states[index].swimming_dive;
                if dive.1 > 0.0 {
                    frame.physics.instant_motion();
                    dive.1 = (dive.1 - frame.dt * decl.diving_deceleration)
                        .max(0.0)
                        .min(frame.velocity.length() * frame.physics.speed_scale());
                }
                if decl.id == StateId::SPRINTING
                    && (frame.physics.input_velocity().length() < decl.minimum_speed
                        || frame.controller.input_move.truncate().length_squared() < decl.minimum_stick * decl.minimum_stick)
                {
                    self.quit(id);
                    self.request(StateId::RUNNING);
                }
            }
            Kind::SwimMantle | Kind::ReadNote => {}
            Kind::PowerWheel => {
                //692200 leaves when the real game-logic wheel is gone.
                if !self.wheel_requested && !self.wheel_logic_active { self.quit(id); }
            }
            Kind::UseHeart(declaration) => self.update_heart(index,&declaration,frame),
            Kind::PlayHeartBark(declaration) => self.update_heart_bark(index,&declaration,frame),
            Kind::UsePower(decl) => {
                let sliding = self.is_active(StateId::SLIDING);
                if !super::power_states::update_use(frame, sliding, &decl) { self.quit(id); }
            }
            Kind::Grapple(decl) => {
                if let Some(next) = super::grapple_state::update(frame, &decl) {
                    self.quit(id);
                    if next != StateId::NULL { self.request(next); }
                }
            }
            Kind::Blinking => {
                if let Some(next) = super::power_states::update_blink(frame) {
                    self.quit(id);
                    self.finish_blink_navigation(next, frame);
                }
            }
            Kind::Dead(dead) => {
                // 14085c4e0 submits vertical and angular motion requests.
                frame.requests.push_linear(0, Vec3::new(0.0, 0.0, dead.vertical_speed));
                frame.requests.push_angular(0, Vec3::new(0.0, 0.0, dead.yaw_speed.to_radians()));
                let state = &mut self.states[index];
                if state.death_fx_pending && dead.duration - state.death_timer >= dead.fx_delay {
                    *frame.drowning_fx_requested = dead.fx.clone();
                    state.death_fx_pending = false;
                }
                if state.death_transition_pending && state.death_timer < 0.0 {
                    state.death_transition_pending = false;
                    *frame.game_over_requested = true;
                }
                state.death_timer -= frame.dt;
            }
        }
    }

    /// Mantle Activate FUN_14085eaf0 (shadow walk, corpses, moving
    /// platforms, legs, footsteps and equipment are not ported).
    fn activate_mantle(&mut self, index: usize, decl: &MantleStateDecl, frame: &mut Frame) {
        frame.mantle_sensor.last_selected_hands = Some([frame.mantle_sensor.use_left_hand, frame.mantle_sensor.use_right_hand]);
        let sensor = &*frame.mantle_sensor;
        let Some((final_posture, final_state)) = sensor.final_posture() else { return };
        let height = sensor.height();
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
        // 14085eaf0 holds only the hands actually used by a non-low mantle,
        // and only when that hand owns an equipped entity.
        let equipment_restore = [sensor.use_left_hand, sensor.use_right_hand].map(|used| used && height != MantleHeight::Low);
        let equipment_restore = std::array::from_fn(|side| equipment_restore[side] && frame.equipment_changes.equipped(side));
        self.states[index].mantle = MantleRun { final_posture, final_state: final_state.0, destination, destination_yaw, height: height as u8, destination_body, equipment_restore };
        if scenes[slot].is_some() {
            let rate = frame.attributes.0.get("ARK_ATTRIBUTE_MANTLE_SPEED_FACTOR").copied().unwrap_or(1.0);
            // Mantle casting 140860030 uses the sensor's hand flags.
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
        if equipment_restore[0] { self.request_equipment_left(0x21, StateId::MANTLE); }
        if equipment_restore[1] { self.request_equipment_right(0x21, StateId::MANTLE); }
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
            if slide.blend_in_timer <= 0.0 {
                frame.emit_contact(decl.contact_material.as_deref());
            }
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
        //8695c0 checks the actual equipment manager's two hand pointers
        //(+68/+70), independently from retained inventory selections.
        let equipped = frame.equipment_changes.equipped(0) || frame.equipment_changes.equipped(1);
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
        if let Kind::Swimming(swimming) = &self.states[index].decl.kind {
            let (direction, speed) = self.states[index].swimming_dive;
            velocity = swimming.velocity(
                velocity, frame.axis.y_axis, frame.camera.pitch,
                frame.controller.down(swimming.up_action),
                frame.controller.down(swimming.down_action),
                (speed > 0.0).then_some(direction * speed),
            );
        }
        // FUN_1407f04a0.
        let scale = frame.attributes.get("ARK_ATTRIBUTE_SPYGLASS_TURNSPEED")
            * frame.attributes.get("ARK_ATTRIBUTE_CAMERA_ROTATION_FACTOR")
            * 0.017453292;
        let joystick = scale * decl.max_turn_speed_joystick;
        let mouse = scale * decl.max_turn_speed_mouse;
        let mut yaw_rate = if frame.controller.look_is_analog { joystick } else { mouse } * frame.controller.input_look.y;
        // 8695c0 applies the actual Slash's focus ratios to navigation's
        // requested velocities, after calculating the ordinary move/turn.
        if let Some(slash) = self.state(StateId::SLASH).filter(|state| state.active && state.slash.focus_active) {
            if let Kind::Slash(decl) = &slash.decl.kind {
                velocity *= decl.focus_linear_speed_ratio;
                yaw_rate *= decl.focus_angular_speed_ratio;
            }
        }
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

    /// Falling Update865c50 accumulates physical motion before Land8658c0.
    fn falling_update(&mut self, index: usize, decl: &FallingDecl, frame: &mut Frame) {
        let id = self.states[index].decl.id;
        let fall = &mut self.states[index].fall;
        fall.accumulated_height += fall.last_origin.z - frame.origin.z;
        fall.last_origin = frame.origin;
        let accumulated = fall.accumulated_height;
        let from = self.states[index].in_air;
        if (frame.physics.ground_distance >= f32::MAX && frame.physics.ground.state == SupportState::Unsupported)
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
        if self.land_falling(index, decl, frame) { return; }
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
        // 865c50 clears the body's contact buffer before the landing impact.
        // Resuming a slide leaves this emission to Sliding's own timed impact.
        let resumes_slide = from.from_sliding && !from.from_jumping
            && accumulated < decl.resume_sliding_max_height && self.slide_available(slide_speed);
        if !resumes_slide {
            frame.footstep_clock.buffer = 0.0;
            frame.emit_contact(decl.landing_material.as_deref());
            let destination = if frame.controller.toggled(crouch_action) { StateId::SNEAKING }
                else if from.from_sprinting { StateId::SPRINTING } else { StateId::RUNNING };
            if self.state(destination).is_some_and(|state| state.decl.navigation.footstep.sound.is_some()) {
                frame.emit_contact(None);
            }
        }
    }

    /// 857a30/838600 call Land on the actual mode's stored Falling instance
    /// even after it was deactivated. 7fdaa0 reads the physics proxy's
    /// support result (+48 -> +20), independently of controller support.
    pub(super) fn land_falling_if_contact(&mut self, frame: &mut Frame) -> bool {
        if frame.physics.ground_distance >= f32::MAX
            && frame.physics.ground.state == SupportState::Unsupported { return false; }
        let Some(index) = self.index(StateId::FALLING) else { return false; };
        let Kind::Falling(decl) = self.states[index].decl.kind.clone() else { return false; };
        self.land_falling(index, &decl, frame)
    }

    fn land_blink_fall_debt(&mut self, frame: &mut Frame) {
        if !frame.powers.blink_from_falling { return; }
        // 857a30 clears its saved pointer after the conditional Land call.
        // Do not accumulate Blink's position delta or reset an airborne fall.
        frame.powers.blink_from_falling = false;
        self.land_falling_if_contact(frame);
    }

    /// Land8658c0 owns damage/CatFall and clears only the consumed height.
    /// Navigation choices and landing-camera work belong to Update865c50;
    /// Blink and Grapple exits must not run those ordinary update branches.
    fn land_falling(&mut self, index: usize, decl: &FallingDecl, frame: &mut Frame) -> bool {
        let accumulated = self.states[index].fall.accumulated_height;
        let id = self.states[index].decl.id;
        let damage = if decl.fall_damage { fall_damage(decl, accumulated, frame) } else { 0.0 };
        //8658c0 completes the real Damage recipient before reading death.
        // Maximum health above is only8657c0's damage scaling input.
        let mut dead = false;
        if damage > 0.0 {
            if let Some(declaration) = frame.fall_damage {
                let mut parameters = crate::gameplay::damage::Parameters::from_declaration(declaration);
                parameters.set_amount(declaration, damage, true);
                parameters.attacker = frame.damage_entity;
                parameters.hit_direction = Vec3::Z;
                if damage == f32::MAX {
                    parameters.damage_type.0 |= crate::gameplay::damage::DamageType::INSTANT_KILL.0;
                }
                let result = frame.damage_recipient.receive(&parameters, frame.attributes,
                    frame.mana, frame.mana_replenishes, frame.controller, frame.physics.axis().x_axis);
                *frame.fall_incapacitated |= result.incapacitated;
                dead = frame.damage_recipient.health.is_dead();
            }
        }
        self.states[index].fall.catfall_taken = false;
        if dead {
            frame.emit_contact(decl.dead_landing_material.as_deref());
        } else if accumulated > decl.cat_fall_height && self.available(StateId::CATFALL, frame) {
            self.states[index].fall.catfall_taken = true;
            self.quit(id);
            self.queue.push(Command::RequestCatFall(damage));
        }
        self.states[index].fall.accumulated_height = 0.0;
        dead || self.states[index].fall.catfall_taken
    }
}

/// FUN_1408657c0, with 8651c0 defaults and maximum health.
fn fall_damage(decl: &FallingDecl, height: f32, frame: &Frame) -> f32 {
    let enhanced = frame.attributes.1.get("ARK_ATTRIBUTE_FALL_DAMAGE_REDUCTION")
        .is_some_and(|attribute| frame.attributes.get("ARK_ATTRIBUTE_FALL_DAMAGE_REDUCTION") != attribute.base);
    let (d, death) = if enhanced { (decl.enhanced_damage_height, decl.enhanced_death_height) }
        else { (decl.damage_height, decl.death_height) };
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
