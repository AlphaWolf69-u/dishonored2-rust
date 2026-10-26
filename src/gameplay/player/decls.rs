//! Typed reads of the player's resolved declarations
//! (`entities::player::player_declarations`). Absent fields keep the decl
//! class constructor defaults.
use super::camera::CameraDecl;
use super::controller::{ButtonUsage, StateMapping};
use super::physics::{PhysicsDecl, PhysicsSettings};
use super::shape::{self, ShapeSettings};
use super::controller::SlippingThresholds;
use super::sensors::{AutoCrouchDecl, LeanDecl, MantleDecl, VaultLimits};
use super::states::{
    Attributes, CatFallDecl, FallingDecl, JumpingDecl, Kind, LeaningDecl, MantleStateDecl, NavigationDecl, RuleDecl, SlidingDecl,
    SlippingDecl, SprintingDecl, StateDecl, StateId, VaultDecl,
};

use crate::entities::player::PlayerDeclarations;
use crate::entities::Value;
use bevy_math::Vec3;
use std::collections::HashMap;

type Fields = [(String, Value)];

fn get<'a>(fields: &'a Fields, key: &str) -> Option<&'a Value> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn atom<'a>(fields: &'a Fields, key: &str) -> Option<&'a str> {
    match get(fields, key)? {
        Value::Atom(text) => Some(text.trim_matches('"')),
        Value::Scope(_) => None,
    }
}

fn scope<'a>(fields: &'a Fields, key: &str) -> Option<&'a Fields> {
    match get(fields, key)? {
        Value::Scope(inner) => Some(inner),
        Value::Atom(_) => None,
    }
}

fn number(fields: &Fields, key: &str, default: f32) -> Result<f32, String> {
    match atom(fields, key) {
        None => Ok(default),
        Some(text) => text
            .trim_end_matches('f')
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("{key}: invalid number {text:?}")),
    }
}

fn boolean(fields: &Fields, key: &str, default: bool) -> Result<bool, String> {
    match atom(fields, key) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(text) => Err(format!("{key}: invalid bool {text:?}")),
    }
}

/// `item[i]` scopes of a list in index order.
fn items<'a>(fields: &'a Fields, key: &str) -> Vec<&'a Fields> {
    let Some(list) = scope(fields, key) else { return Vec::new() };
    let mut entries: Vec<(usize, &Fields)> = list
        .iter()
        .filter_map(|(k, v)| {
            let index = k.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?;
            match v {
                Value::Scope(inner) => Some((index, inner.as_slice())),
                Value::Atom(_) => None,
            }
        })
        .collect();
    entries.sort_by_key(|(i, _)| *i);
    entries.into_iter().map(|(_, f)| f).collect()
}

/// A bodyParts_t scope: m_body 1, m_left_arm 2, m_right_arm 4, m_third_arm 8,
/// m_legs 0x10 (reflection order); unset fields keep the class bits.
fn body_parts(fields: &Fields, key: &str, mut bits: u8) -> Result<u8, String> {
    let Some(parts) = scope(fields, key) else { return Ok(bits) };
    for (name, value) in parts {
        let bit = match name.as_str() {
            "m_body" => 1,
            "m_left_arm" => 2,
            "m_right_arm" => 4,
            "m_third_arm" => 8,
            "m_legs" => 0x10,
            other => return Err(format!("unknown body part {other}")),
        };
        match value {
            Value::Atom(v) if v == "true" => bits |= bit,
            Value::Atom(_) => bits &= !bit,
            Value::Scope(_) => return Err(format!("invalid body part {name}")),
        }
    }
    Ok(bits)
}

/// The atoms of an `item[n]` list, in order.
fn items_atoms(list: &Fields) -> Vec<String> {
    let mut entries: Vec<(usize, String)> = list
        .iter()
        .filter_map(|(k, v)| match v {
            Value::Atom(text) => Some((k.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?, text.trim_matches('"').to_owned())),
            Value::Scope(_) => None,
        })
        .collect();
    entries.sort();
    entries.into_iter().map(|(_, v)| v).collect()
}

/// An arkCollisionLayerMask_t scope: each `m_<LAYER>` member sets or
/// clears its bit of the constructor default.
fn layer_mask(fields: &Fields, key: &str, default: u32) -> Result<u32, String> {
    let Some(members) = scope(fields, key) else { return Ok(default) };
    let mut mask = default;
    for (member, value) in members {
        let bit = crate::gameplay::collision_filter::layer_member(member).ok_or_else(|| format!("unknown {key} member {member}"))?;
        match value {
            Value::Atom(v) if v == "true" => mask |= 1 << bit,
            Value::Atom(v) if v == "false" => mask &= !(1 << bit),
            _ => return Err(format!("invalid {key}.{member}")),
        }
    }
    Ok(mask)
}

/// An arkCollisionLayer_t field.
fn collision_layer(fields: &Fields, key: &str, default: u32) -> Result<u32, String> {
    match atom(fields, key) {
        None => Ok(default),
        Some(name) => crate::gameplay::collision_filter::layer_index(name).ok_or_else(|| format!("unknown {key} {name}")),
    }
}

/// The health component's m_health (installed for the player).
pub fn health(fields: &Fields) -> Result<f32, String> {
    number(fields, "m_health", f32::NAN).and_then(|h| if h.is_nan() { Err("player health decl has no m_health".into()) } else { Ok(h) })
}

/// arkDeclCpntPlayerSensorMantle (ctor FUN_1403baf00).
pub fn mantle_sensor(fields: &Fields) -> Result<MantleDecl, String> {
    let d = MantleDecl::default();
    Ok(MantleDecl {
        max_distance_from_wall: number(fields, "m_maxDistanceFromWall", d.max_distance_from_wall)?,
        forward_cast_height_min: number(fields, "m_forwardCastHeightMin", d.forward_cast_height_min)?,
        forward_cast_height_max: number(fields, "m_forwardCastHeightMax", d.forward_cast_height_max)?,
        mid_mantle_height: number(fields, "m_midMantleHeight", d.mid_mantle_height)?,
        high_mantle_height: number(fields, "m_highMantleHeight", d.high_mantle_height)?,
        max_player_angle: number(fields, "m_maxPlayerAngle", d.max_player_angle)?,
        destination_depth: number(fields, "m_destinationDepth", d.destination_depth)?,
        wall_angle_max: number(fields, "m_wallAngleMax", d.wall_angle_max)?,
        hand_box_size: vector(fields, "m_handBoxSize", d.hand_box_size)?,
        left_hand_offset: vector(fields, "m_leftHandOffset", d.left_hand_offset)?,
        right_hand_offset: vector(fields, "m_rightHandOffset", d.right_hand_offset)?,
        use_player_direction: boolean(fields, "m_usePlayerDirecton", d.use_player_direction)?,
        collision_mask: layer_mask(fields, "m_collisionMask", d.collision_mask)?,
    })
}

/// An eActions_t field (`ACTION_` names, binding_tables.rs `ACTIONS`).
fn action(fields: &Fields, key: &str, default: i32) -> Result<i32, String> {
    match atom(fields, key).and_then(|a| a.strip_prefix("ACTION_")) {
        None => Ok(default),
        Some(name) => crate::platform::bindings::ACTIONS
            .iter()
            .find(|a| a.1 == name)
            .map(|a| a.0)
            .ok_or_else(|| format!("unknown action ACTION_{name}")),
    }
}

/// A `durationTime_t` field (`{ m_seconds = ... }`).
fn seconds(fields: &Fields, key: &str, default: f32) -> Result<f32, String> {
    scope(fields, key).map_or(Ok(default), |duration| number(duration, "m_seconds", default))
}

/// A vector field (`{ x = ...; y = ...; z = ... }`).
fn vector(fields: &Fields, key: &str, default: Vec3) -> Result<Vec3, String> {
    let Some(v) = scope(fields, key) else { return Ok(default) };
    Ok(Vec3::new(number(v, "x", default.x)?, number(v, "y", default.y)?, number(v, "z", default.z)?))
}

/// arkCpntPlayerCamera::mode_t names (table 0x14239dad0; KEEP_PREVIOUS
/// follows MAX).
pub fn camera_mode(name: &str) -> Option<i32> {
    const MODES: [&str; 19] = [
        "AND_ARMS", "WITHOUT_ARMS", "ANIMATED", "CONSTRAINED", "LOOKAT", "NO_CONTROL", "CONSTRAINED_WITH_ARMS", "SHADOW_WALK",
        "SHADOW_GRAPPLE", "CINEMATIC_WITH_ARMS", "CINEMATIC_WITHOUT_ARMS", "CINEMATIC_ANIMATED", "ENTER_POSSESSED_HOST", "CORPSE",
        "CATFALL", "DROWNING", "BLACKMARKET", "MAX", "KEEP_PREVIOUS",
    ];
    let name = name.strip_prefix("MODE_CAMERA_")?;
    MODES.iter().position(|m| *m == name).map(|i| i as i32)
}

/// A constraintParams_t field in degrees: pitch up, down, yaw left, right.
pub fn constraint_params(fields: &Fields, key: &str, default: [f32; 4]) -> Result<[f32; 4], String> {
    let Some(c) = scope(fields, key) else { return Ok(default) };
    Ok([
        number(c, "m_maxPitchUp", default[0])?,
        number(c, "m_maxPitchDown", default[1])?,
        number(c, "m_maxYawLeft", default[2])?,
        number(c, "m_maxYawRight", default[3])?,
    ])
}

/// arkPlayerStateId_t names the port resolves (enum table 0x142391a90).
pub fn state_id(name: &str) -> Option<StateId> {
    let value = match name.strip_prefix("PLAYER_STATE_")? {
        "RUNNING" => 0,
        "SPRINTING" => 1,
        "JUMPING" => 2,
        "FALLING" => 3,
        "CATFALL" => 4,
        "SLIPPING" => 5,
        "SNEAKING" => 6,
        "SLIDING" => 7,
        "MANTLE" => 8,
        "VAULT" => 9,
        "AUTOCROUCH" => 10,
        "WALLCROUCH" => 11,
        "BLINKING" => 12,
        "LEANING" => 18,
        "IDLE" => 22,
        "CINEMATIC" => 23,
        "PEEKING" => 24,
        "GRAPPLE" => 27,
        "SLASH" => 39,
        "NULL" => 91,
        "ANY" => 92,
        _ => return None,
    };
    Some(StateId(value))
}

fn shape_settings(fields: &Fields) -> Result<ShapeSettings, String> {
    let d = ShapeSettings::default();
    Ok(ShapeSettings {
        radius: number(fields, "m_radius", d.radius)?,
        height: number(fields, "m_height", d.height)?,
        step_height: number(fields, "m_stepHeight", d.step_height)?,
        z_offset: number(fields, "m_zOffset", d.z_offset)?,
        extrude_length: number(fields, "m_extrudeLength", d.extrude_length)?,
        gravity_strength: number(fields, "m_gravityStrength", d.gravity_strength)?,
    })
}

pub fn physics(fields: &Fields) -> Result<PhysicsDecl, String> {
    let d = PhysicsDecl::default();
    let s = PhysicsSettings::default();
    let physics = scope(fields, "m_physics").unwrap_or(&[]);
    let mut shapes = Vec::new();
    for item in items(fields, "m_shapes") {
        let posture = match atom(item, "m_posture") {
            None => shape::POSTURE_STANDING,
            Some(name) => shape::posture(name).ok_or_else(|| format!("unknown posture {name}"))?,
        };
        shapes.push((posture, shape_settings(scope(item, "m_shape").unwrap_or(&[]))?));
    }
    Ok(PhysicsDecl {
        ground_query_length: number(fields, "m_groundQueryLength", d.ground_query_length)?,
        ground_ray_query_length: number(fields, "m_groundRayQueryLength", d.ground_ray_query_length)?,
        ceil_query_length: number(fields, "m_ceilQueryLength", d.ceil_query_length)?,
        collision_layer: collision_layer(fields, "m_collisionLayer", d.collision_layer)?,
        ground_collision_mask: layer_mask(fields, "m_groundCollisionMask", d.ground_collision_mask)?,
        ceil_collision_mask: layer_mask(fields, "m_ceilCollisionMask", d.ceil_collision_mask)?,
        physics: PhysicsSettings {
            dynamic_friction: number(physics, "m_dynamicFriction", s.dynamic_friction)?,
            static_friction: number(physics, "m_staticFriction", s.static_friction)?,
            contact_angle_sensitivity: number(physics, "m_contactAngleSensitivity", s.contact_angle_sensitivity)?,
            user_planes: number(physics, "m_userPlanes", s.user_planes as f32)? as i32,
            max_character_speed_for_solver: number(physics, "m_maxCharacterSpeedForSolver", s.max_character_speed_for_solver)?,
            character_strength: number(physics, "m_characterStrength", s.character_strength)?,
            character_mass: number(physics, "m_characterMass", s.character_mass)?,
            max_slope: number(physics, "m_maxSlope", s.max_slope)?,
            penetration_recovery_speed: number(physics, "m_penetrationRecoverySpeed", s.penetration_recovery_speed)?,
            max_cast_iterations: number(physics, "m_maxCastIterations", s.max_cast_iterations as f32)? as i32,
            refresh_manifold_in_check_support: boolean(physics, "m_refreshManifoldInCheckSupport", s.refresh_manifold_in_check_support)?,
        },
        shapes,
    })
}

pub fn camera(fields: &Fields) -> Result<CameraDecl, String> {
    let d = CameraDecl::default();
    Ok(CameraDecl {
        max_pitch_up: number(fields, "m_maxPitchUp", d.max_pitch_up)?,
        max_pitch_down: number(fields, "m_maxPitchDown", d.max_pitch_down)?,
        pitch_speed_ratio: number(fields, "m_pitchSpeedRatio", d.pitch_speed_ratio)?,
        camera_joint: atom(fields, "m_cameraJointName").unwrap_or(&d.camera_joint).to_owned(),
        head_joint: atom(fields, "m_headControlJointName").unwrap_or(&d.head_joint).to_owned(),
        neck_joint: atom(fields, "m_neckControlJointName").unwrap_or(&d.neck_joint).to_owned(),
        arms_joint: atom(fields, "m_armsControlJointName").unwrap_or(&d.arms_joint).to_owned(),
        forward_ratios: [
            number(fields, "m_headControlForwardRatio", d.forward_ratios[0])?,
            number(fields, "m_neckControlForwardRatio", d.forward_ratios[1])?,
            number(fields, "m_armsControlForwardRatio", d.forward_ratios[2])?,
        ],
        backward_ratios: [
            number(fields, "m_headControlBackwardRatio", d.backward_ratios[0])?,
            number(fields, "m_neckControlBackwardRatio", d.backward_ratios[1])?,
            number(fields, "m_armsControlBackwardRatio", d.backward_ratios[2])?,
        ],
        speed_fov_additional_value: number(fields, "m_speedFovAdditionalValue", d.speed_fov_additional_value)?,
        speed_fov_max_increase_speed: number(fields, "m_speedFovMaxIncreaseSpeed", d.speed_fov_max_increase_speed)?,
        speed_fov_max_decrease_speed: number(fields, "m_speedFovMaxDecreaseSpeed", d.speed_fov_max_decrease_speed)?,
        arm_joints: [
            atom(fields, "m_leftArmControlJointName").unwrap_or(&d.arm_joints[0]).to_owned(),
            atom(fields, "m_rightArmControlJointName").unwrap_or(&d.arm_joints[1]).to_owned(),
        ],
        arms_inertia_min_angular_speed: number(fields, "m_armsInertiaMinAngularSpeed", d.arms_inertia_min_angular_speed)?,
        arms_inertia_max_angular_speed: number(fields, "m_armsInertiaMaxAngularSpeed", d.arms_inertia_max_angular_speed)?,
        arms_inertia_max_angle: number(fields, "m_armsInertiaMaxAngle", d.arms_inertia_max_angle)?,
        arm_pitch_shift_ratios: [
            number(fields, "m_leftArmPitchShiftRatio", d.arm_pitch_shift_ratios[0])?,
            number(fields, "m_rightArmPitchShiftRatio", d.arm_pitch_shift_ratios[1])?,
        ],

        constraints_blend_speed: number(fields, "m_constraintsBlendSpeed", d.constraints_blend_speed)?,
        mode_blend_duration: seconds(fields, "m_modeBlendDuration", d.mode_blend_duration)?,
    })
}

/// The controller's `m_statesMapping`; action names are eActions_t names
/// with the `ACTION_` prefix (binding_tables.rs `ACTIONS`).
pub fn states_mapping(fields: &Fields) -> Result<Vec<StateMapping>, String> {
    let mut mapping = Vec::new();
    for item in items(fields, "m_statesMapping") {
        let usage = match atom(item, "m_usage").unwrap_or("BUTTON_PRESSED") {
            "BUTTON_PRESSED" => ButtonUsage::Pressed,
            "BUTTON_DOWN" => ButtonUsage::Down,
            "BUTTON_RELEASED" => ButtonUsage::Released,
            "BUTTON_TOGGLE_DOWN" => ButtonUsage::ToggleDown,
            other => return Err(format!("unknown button usage {other}")),
        };
        let action = match atom(item, "m_action").and_then(|a| a.strip_prefix("ACTION_")) {
            None | Some("NONE") => continue,
            Some(name) => crate::platform::bindings::ACTIONS
                .iter()
                .find(|a| a.1 == name)
                .map(|a| a.0)
                .ok_or_else(|| format!("unknown action ACTION_{name}"))?,
        };
        // A state outside the port's enum subset can never be available.
        let state = atom(item, "m_state").and_then(state_id).unwrap_or(StateId(-1));
        let priority_when_available = number(item, "m_priorityWhenAvailable", -1.0)? as i32;
        mapping.push(StateMapping { usage, action, state, priority_when_available });
    }
    Ok(mapping)
}

pub fn rules(mode: &Fields) -> Result<Vec<RuleDecl>, String> {
    let mut rules = Vec::new();
    for item in items(mode, "m_rules") {
        let (Some(from), Some(to)) = (atom(item, "m_from").and_then(state_id), atom(item, "m_to").and_then(state_id)) else {
            continue;
        };
        rules.push(RuleDecl { from, to, allow: boolean(item, "m_allow", true)?, quit: boolean(item, "m_quit", false)? });
    }
    Ok(rules)
}

pub fn default_state(mode: &Fields) -> StateId {
    atom(mode, "m_defaultStateId").and_then(state_id).unwrap_or(StateId::RUNNING)
}

fn navigation(fields: &Fields, mut d: NavigationDecl) -> Result<NavigationDecl, String> {
    if let Some(name) = atom(fields, "m_posture") {
        d.posture = shape::posture(name).ok_or_else(|| format!("unknown posture {name}"))?;
    }
    if let Some(name) = atom(fields, "m_nextCrushedState") {
        d.next_crushed_state = state_id(name).unwrap_or(StateId::NULL);
    }
    Ok(NavigationDecl {
        max_forward_speed: number(fields, "m_maxForwardSpeed", d.max_forward_speed)?,
        unequipped_speed_increase: number(fields, "m_unequippedSpeedIncrease", d.unequipped_speed_increase)?,
        max_backward_speed: number(fields, "m_maxBackwardSpeed", d.max_backward_speed)?,
        max_strafe_speed: number(fields, "m_maxStrafeSpeed", d.max_strafe_speed)?,
        max_turn_speed_joystick: number(fields, "m_maxTurnSpeedJoystick", d.max_turn_speed_joystick)?,
        max_turn_speed_mouse: number(fields, "m_maxTurnSpeedMouse", d.max_turn_speed_mouse)?,
        max_linear_acceleration: number(fields, "m_maxLinearAcceleration", d.max_linear_acceleration)?,
        max_linear_deceleration: number(fields, "m_maxLinearDeceleration", d.max_linear_deceleration)?,
        max_angular_acceleration_joystick: number(fields, "m_maxAngularAccelerationJoystick", d.max_angular_acceleration_joystick)?,
        max_angular_deceleration_joystick: number(fields, "m_maxAngularDecelerationJoystick", d.max_angular_deceleration_joystick)?,
        forward_angle_snap: number(fields, "m_forwardAngleSnap", d.forward_angle_snap)?,
        strafe_angle_snap: number(fields, "m_strafeAngleSnap", d.strafe_angle_snap)?,
        max_ground_distance: number(fields, "m_maxGroundDistance", d.max_ground_distance)?,
        loco_stance_name: atom(fields, "m_locoStanceName").map(str::to_owned).unwrap_or(d.loco_stance_name.clone()),
        head_bob_stance_name: atom(fields, "m_headBobStanceName").map(str::to_owned).unwrap_or(d.head_bob_stance_name.clone()),
        physics_shape_transition_speed_factor: number(fields, "m_physicsShapeTransitionSpeedFactor", d.physics_shape_transition_speed_factor)?,
        small_fall_min_height: number(fields, "m_smallFallMinHeight", d.small_fall_min_height)?,
        small_fall_anim_weight: number(fields, "m_smallFallAnimWeight", d.small_fall_anim_weight)?,
        camera_mode: match atom(fields, "m_cameraMode") {
            Some(name) => camera_mode(name).ok_or_else(|| format!("unknown camera mode {name}"))?,
            None => d.camera_mode,
        },
        camera_mode_blend_duration: seconds(fields, "m_cameraModeBlendDuration", d.camera_mode_blend_duration)?,
        ..d
    })
}

/// The mode's locomotion state decls; other state kinds are not ported.
pub fn states(declarations: &PlayerDeclarations) -> Result<Vec<StateDecl>, String> {
    let mut result = Vec::new();
    for (kind, fields) in &declarations.states {
        let kind = match kind.as_str() {
            "playerstaterunning" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Running)) => Kind::Running,
            "playerstatesneaking" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Sneaking)) => Kind::Sneaking,
            "playerstatesprinting" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Sprinting(_))) => {
                let d = SprintingDecl::default();
                Kind::Sprinting(SprintingDecl {
                    minimum_stick: number(fields, "m_minimumStick", d.minimum_stick)?,
                    minimum_speed: number(fields, "m_minimumSpeed", d.minimum_speed)?,
                })
            }
            "playerstatejumping" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Jumping(_))) => {
                let d = JumpingDecl::default();
                Kind::Jumping(JumpingDecl {
                    min_clear_distance_above: number(fields, "m_minClearDistanceAbove", d.min_clear_distance_above)?,
                    jump_height: number(fields, "m_jumpHeight", d.jump_height)?,
                    jump_height_sprinting: number(fields, "m_jumpHeightSprinting", d.jump_height_sprinting)?,
                    jump_height_crouching: number(fields, "m_jumpHeightCrouching", d.jump_height_crouching)?,
                    maximum_velocity: number(fields, "m_maximumVelocity", d.maximum_velocity)?,
                    steepness: number(fields, "m_steepness", d.steepness)?,
                    ceil_collision_factor: number(fields, "m_ceilCollisionFactor", d.ceil_collision_factor)?,
                    jump_sound_event: sound_event_id(fields, "m_jumpSoundEvent"),
                    double_jump_sound_event: sound_event_id(fields, "m_doubleJumpSoundEvent"),
                    super_jump_sound_event: sound_event_id(fields, "m_superJumpSoundEvent"),
                })
            }
            "playerstatefalling" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Falling(_))) => {
                let d = FallingDecl::default();
                Kind::Falling(FallingDecl {
                    smooth_gravity_adjustment: number(fields, "m_smoothGravityAdjustment", d.smooth_gravity_adjustment)?,
                    resume_sliding_max_height: number(fields, "m_resumeSlidingMaxHeight", d.resume_sliding_max_height)?,
                    full_fall_anim_height: number(fields, "m_fullFallAnimHeight", d.full_fall_anim_height)?,
                    cat_fall_height: number(fields, "m_catFallHeight", d.cat_fall_height)?,
                    damage_height: number(fields, "m_damageHeight", d.damage_height)?,
                    death_height: number(fields, "m_deathHeight", d.death_height)?,
                    fall_damage: atom(fields, "m_fallDamageDecl").is_some_and(|v| !v.is_empty()),

                })
            }
            "playerstatesliding" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Sliding(_))) => {
                let d = SlidingDecl::default();
                Kind::Sliding(SlidingDecl {
                    in_speed: number(fields, "m_inSpeed", d.in_speed)?,
                    out_speed: number(fields, "m_outSpeed", d.out_speed)?,
                    distance: number(fields, "m_distance", d.distance)?,
                    max_forward_angle: number(fields, "m_maxForwardAngle", d.max_forward_angle)?,
                    max_wall_angle: number(fields, "m_maxWallAngle", d.max_wall_angle)?,
                    min_slope_angle: number(fields, "m_minSlopeAngle", d.min_slope_angle)?,
                    max_slope_angle: number(fields, "m_maxSlopeAngle", d.max_slope_angle)?,
                    slope_influence: number(fields, "m_slopeInfluence", d.slope_influence)?,
                    constraint_params: constraint_params(fields, "m_constraintParams", d.constraint_params)?,

                })
            }
            "playerstateslipping" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Slipping(_))) => {
                let d = SlippingDecl::default();
                Kind::Slipping(SlippingDecl { gravity_strength: number(fields, "m_gravityStrength", d.gravity_strength)? })
            }
            "playerstateautocrouch" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::AutoCrouch)) => Kind::AutoCrouch,
            "playerstatecatfall" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::CatFall(_))) => {
                let d = CatFallDecl::default();
                Kind::CatFall(CatFallDecl {
                    low_damage_midnight: atom(fields, "m_lowDamageMidnight").filter(|v| !v.is_empty()).map(str::to_owned),
                    high_damage_midnight: atom(fields, "m_highDamageMidnight").filter(|v| !v.is_empty()).map(str::to_owned),
                    damage_threshold: number(fields, "m_damageThreshold", d.damage_threshold)?,
                    look_at_params: constraint_params(fields, "m_lookAtParams", d.look_at_params)?,
                })
            }
            "playerstatemantle" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Mantle(_))) => {
                let d = MantleStateDecl::default();
                let three = |key: &str| -> [Option<String>; 3] {
                    let list = scope(fields, key).and_then(|s| scope(s, "m_elems")).unwrap_or(&[]);
                    std::array::from_fn(|i| atom(list, &format!("m_elems[{i}]")).filter(|v| !v.is_empty()).map(str::to_owned))
                };
                Kind::Mantle(MantleStateDecl {
                    low_midnights: three("m_lowMantleMidnights"),
                    mid_midnights: three("m_midMantleMidnights"),
                    high_midnights: three("m_highMantleMidnights"),
                    constraint_params: constraint_params(fields, "m_constraintParams", d.constraint_params)?,
                    low_mantle_duration: number(fields, "m_lowMantleDuration", d.low_mantle_duration)?,
                })
            }
            "playerstatevault" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Vault(_))) => {
                let d = VaultDecl::default();
                let l = d.limits;
                let list = |key: &str| -> Vec<String> {
                    scope(fields, key).map(|s| items_atoms(s)).unwrap_or_default()
                };
                Kind::Vault(VaultDecl {
                    min_speed: number(fields, "m_minSpeed", d.min_speed)?,
                    limits: VaultLimits {
                        max_distance: number(fields, "m_obstacleMaxDistance", l.max_distance)?,
                        min_distance: number(fields, "m_obstacleMinDistance", l.min_distance)?,
                        min_height: number(fields, "m_obstacleMinHeight", l.min_height)?,
                        max_height: number(fields, "m_obstacleMaxHeight", l.max_height)?,
                        max_depth: number(fields, "m_obstacleMaxDepth", l.max_depth)?,
                        clear_above: number(fields, "m_clearDistanceAbove", l.clear_above)?,
                        clear_behind: number(fields, "m_clearDistanceBehind", l.clear_behind)?,
                        clear_left: number(fields, "m_clearDistanceLeft", l.clear_left)?,
                        clear_right: number(fields, "m_clearDistanceRight", l.clear_right)?,
                    },
                    small_midnights: list("m_smallVaultMidnights"),
                    small_distance: number(fields, "m_smallVaultDistance", d.small_distance)?,
                    high_threshold: number(fields, "m_highVaultThreshold", d.high_threshold)?,
                    high_midnights: list("m_highVaultMidnights"),
                    high_distance: number(fields, "m_highVaultDistance", d.high_distance)?,
                    exit_offset: d.exit_offset,
                    constraint_params: constraint_params(fields, "m_constraintParams", d.constraint_params)?,
                })
            }
            "playerstateleaning" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Leaning(_))) => {
                let d = LeaningDecl::default();
                Kind::Leaning(LeaningDecl {
                    lean_speed_gamepad: number(fields, "m_leanSpeedGamepad", d.lean_speed_gamepad)?,
                    lean_speed_keyboard: number(fields, "m_leanSpeedKeyboard", d.lean_speed_keyboard)?,
                    lean_up_speed_gamepad: number(fields, "m_leanUpSpeedGamepad", d.lean_up_speed_gamepad)?,
                    lean_up_speed_keyboard: number(fields, "m_leanUpSpeedKeyboard", d.lean_up_speed_keyboard)?,
                    maximum_linear_speed: number(fields, "m_maximumLinearSpeed", d.maximum_linear_speed)?,
                    minimum_rotation_angle: number(fields, "m_minimumRotationAngle", d.minimum_rotation_angle)?,
                    maximum_rotation_angle: number(fields, "m_maximumRotationAngle", d.maximum_rotation_angle)?,
                    recenter_threshold: number(fields, "m_recenterThreshold", d.recenter_threshold)?,
                    dead_zone_inner_radius: number(fields, "m_deadZoneInnerRadius", d.dead_zone_inner_radius)?,
                    dead_zone_outer_radius: number(fields, "m_deadZoneOuterRadius", d.dead_zone_outer_radius)?,
                    recenter_delay: seconds(fields, "m_recenterDelay", d.recenter_delay)?,
                    exit_delay: seconds(fields, "m_exitDelay", d.exit_delay)?,
                    blend_duration: seconds(fields, "m_blendDuration", d.blend_duration)?,
                    lean_left_button: action(fields, "m_leanLeftButton", d.lean_left_button)?,
                    lean_right_button: action(fields, "m_leanRightButton", d.lean_right_button)?,
                    lean_up_button: action(fields, "m_leanUpButton", d.lean_up_button)?,
                    lean_down_button: action(fields, "m_leanDownButton", d.lean_down_button)?,
                    start_sound_event: sound_event_id(fields, "m_startSoundEvent"),
                    stop_sound_event: sound_event_id(fields, "m_stopSoundEvent"),
                })
            }
            _ => continue,
        };
        let mut decl = StateDecl::class_default(kind);
        if let Some(primary) = atom(fields, "m_primary") {
            decl.flags.primary = primary == "true";
        }
        decl.flags.body_parts = body_parts(fields, "m_bodyParts", decl.flags.body_parts)?;
        decl.flags.locked_body_parts = body_parts(fields, "m_lockedBodyParts", decl.flags.locked_body_parts)?;
        decl.flags.exclusive_body_parts = body_parts(fields, "m_exclusiveBodyParts", decl.flags.exclusive_body_parts)?;
        decl.flags.allow_camera_out_of_shape = boolean(fields, "m_allowCameraOutOfShape", decl.flags.allow_camera_out_of_shape)?;
        decl.navigation = navigation(fields, decl.navigation.clone())?;
        decl.air_friction = number(fields, "m_airFriction", decl.air_friction)?;
        result.push(decl);
    }
    Ok(result)
}

fn sound_event_id(fields: &Fields, key: &str) -> Option<u32> {
    atom(fields, key)
        .filter(|path| !path.is_empty())
        .and_then(|path| path.rsplit('/').next())
        .map(crate::assets::wwise::event_id)
}

pub fn attributes(fields: &Fields) -> Result<Attributes, String> {
    let mut values = HashMap::new();
    let Some(list) = scope(fields, "m_attributes") else { return Ok(Attributes(values)) };
    for (_, value) in list {
        let Value::Scope(item) = value else { continue };
        if let Some(tag) = atom(item, "m_tag") {
            values.insert(tag.to_owned(), number(item, "m_baseValue", 0.0)?);
        }
    }
    Ok(Attributes(values))
}

/// `m_stanceTransitions`: (from, to, body animation).
pub fn stance_transitions(locomotion: &Fields) -> Vec<(String, String, String)> {
    items(locomotion, "m_stanceTransitions")
        .into_iter()
        .filter_map(|item| {
            Some((
                atom(item, "m_fromStance")?.to_owned(),
                atom(item, "m_toStance")?.to_owned(),
                atom(item, "m_bodyAnim").unwrap_or("").to_owned(),
            ))
        })
        .collect()
}

/// `m_stances`: (stance, Idle body animation alias).
pub fn stance_idle_animations(locomotion: &Fields) -> Vec<(String, String)> {
    items(locomotion, "m_stances")
        .into_iter()
        .filter_map(|stance| {
            let name = atom(stance, "m_stance")?;
            let idle = items(stance, "m_speeds").into_iter().find(|speed| atom(speed, "m_speed") == Some("Idle"))?;
            let front = atom(scope(idle, "m_body")?, "m_front")?;
            Some((name.to_owned(), front.to_owned()))
        })
        .collect()
}

/// The controller decl's slipping fields (ctor FUN_1407f7a60).
pub fn slipping_thresholds(controller: &Fields) -> Result<SlippingThresholds, String> {
    Ok(SlippingThresholds::from_degrees(
        number(controller, "m_slippingSlopeAngle", 49.0)?,
        number(controller, "m_slippingAvgSlopeAngle", 36.0)?,
        number(controller, "m_slippingFallSlopeStartAngle", 70.0)?,
        number(controller, "m_slippingFallSlopeStopAngle", 60.0)?,
        number(controller, "m_slippingMinGroundDistance", 0.5)?,
    ))
}

/// arkDeclCpntPlayerSensorAutoCrouch (ctor FUN_1403a2120).
pub fn autocrouch_sensor(fields: &Fields) -> Result<AutoCrouchDecl, String> {
    let d = AutoCrouchDecl::default();
    Ok(AutoCrouchDecl {
        min_detection_distance: number(fields, "m_minDetectionDistance", d.min_detection_distance)?,
        shape_radius: number(fields, "m_shapeRadius", d.shape_radius)?,
        min_hole_depth: number(fields, "m_minHoleDepth", d.min_hole_depth)?,
        push_duration: number(fields, "m_pushDuration", d.push_duration)?,
        uncrouch_if_not_pushing: boolean(fields, "m_uncrouchIfNotPushing", d.uncrouch_if_not_pushing)?,
    })
}

/// arkDeclCpntPlayerSensorLean (ctor FUN_1403b91c0).
pub fn lean_sensor(fields: &Fields) -> Result<LeanDecl, String> {
    let d = LeanDecl::default();
    Ok(LeanDecl {
        head_shape_size: vector(fields, "m_headShapeSize", d.head_shape_size)?,
        lean_amplitude: number(fields, "m_leanAmplitude", d.lean_amplitude)?,
        lean_up_amplitude: number(fields, "m_leanUpAmplitude", d.lean_up_amplitude)?,
        collision_layers: layer_mask(fields, "m_collisionLayers", d.collision_layers)?,
    })
}

/// arkDeclAnimBasic `m_animBasicList` items as (alias, blend in, blend out)
/// and the stop blend-out. An item time below 0 (the list default -1) takes
/// the decl default, clamped at 0 (FUN_140243760 / FUN_140243850; ctor
/// FUN_1407e5cc0: 0.2 each).
pub fn anim_basic_list(fields: &Fields) -> Result<(Vec<(String, f32, f32)>, f32), String> {
    let blend_in = number(fields, "m_defaultBlendInDuration", 0.2)?.max(0.0);
    let blend_out = number(fields, "m_defaultBlendOutDuration", 0.2)?.max(0.0);
    let stop = number(fields, "m_stopBlendOutDuration", 0.2)?.max(0.0);
    let mut list = Vec::new();
    for item in items(fields, "m_animBasicList") {
        let item_in = number(item, "m_blendInDuration", -1.0)?;
        let item_out = number(item, "m_blendOutDuration", -1.0)?;
        list.push((
            atom(item, "m_aliasRef").unwrap_or("").to_owned(),
            if item_in < 0.0 { blend_in } else { item_in },
            if item_out < 0.0 { blend_out } else { item_out },
        ));
    }
    Ok((list, stop))
}

/// The body decl's `m_additives` camera jump aliases in FUN_140772f90's kind
/// order: in, loop, out, sneak in, sneak loop, sneak out.
pub fn camera_jump_aliases(body: &Fields) -> [Option<String>; 6] {
    let additives = scope(body, "m_additives").unwrap_or(&[]);
    [
        "m_cameraJumpInAnimAlias",
        "m_cameraJumpLoopAnimAlias",
        "m_cameraJumpOutAnimAlias",
        "m_cameraJumpSneakInAnimAlias",
        "m_cameraJumpSneakLoopAnimAlias",
        "m_cameraJumpSneakOutAnimAlias",
    ]
    .map(|key| atom(additives, key).filter(|a| !a.is_empty()).map(str::to_owned))
}

/// The body decl's `m_leanLeftAnimAlias` / `m_leanUpAnimAlias`.

pub fn lean_aliases(body: &Fields) -> [Option<String>; 2] {
    ["m_leanLeftAnimAlias", "m_leanUpAnimAlias"].map(|key| atom(body, key).filter(|a| !a.is_empty()).map(str::to_owned))
}

/// One `m_stances[].m_speeds[]` entry: speed-set index (by `m_speedList`),
/// body, bob and arm clip aliases in back/right/front/left order (empty
/// directions filled with front) and clip speeds (front/back, strafe for
/// left and right; default 0).
pub struct LocoSpeedDecl {
    pub speed: usize,
    pub body: Option<[String; 4]>,
    pub bob: Option<[String; 4]>,
    /// `m_leftArm` / `m_rightArm` entries of the arms' equip states
    /// (FUN_1407021d0: an entry with no name has no speed definition).
    pub arms: [Option<[String; 4]>; 2],
    pub speeds: [f32; 4],
}

/// The `m_equipState == equip` item of an equip-state list.
fn equip_item<'a>(fields: &'a Fields, key: &str, equip: &str) -> Option<&'a Fields> {
    items(fields, key).into_iter().find(|item| atom(item, "m_equipState") == Some(equip))
}

fn moves(fields: Option<&Fields>) -> Option<[String; 4]> {
    let fields = fields?;
    let front = atom(fields, "m_front").unwrap_or("").to_owned();
    let pick = |key: &str| atom(fields, key).filter(|v| !v.is_empty()).map(str::to_owned).unwrap_or_else(|| front.clone());
    let all = [pick("m_back"), pick("m_right"), front.clone(), pick("m_left")];
    all.iter().any(|name| !name.is_empty()).then_some(all)
}

/// `m_stances` with their speed sets (FUN_1407030a0 → FUN_140249400) and
/// the arms' clips for their equip states `arm_equip` (left, right).
pub fn loco_stances(locomotion: &Fields, arm_equip: [&str; 2]) -> Result<Vec<(String, Vec<LocoSpeedDecl>)>, String> {
    let speed_list: Vec<String> = scope(locomotion, "m_speedList")
        .map(|list| {
            let mut entries: Vec<(usize, String)> = list
                .iter()
                .filter_map(|(k, v)| match v {
                    Value::Atom(text) => Some((k.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?, text.trim_matches('"').to_owned())),
                    Value::Scope(_) => None,
                })
                .collect();
            entries.sort();
            entries.into_iter().map(|(_, name)| name).collect()
        })
        .unwrap_or_default();
    let mut result = Vec::new();
    for stance in items(locomotion, "m_stances") {
        let Some(name) = atom(stance, "m_stance") else { continue };
        let mut speeds = Vec::new();
        for entry in items(stance, "m_speeds") {
            let Some(speed_name) = atom(entry, "m_speed") else { continue };
            let Some(speed) = speed_list.iter().position(|s| s == speed_name) else { continue };
            let front = number(entry, "m_frontSpeed", 0.0)?;
            let back = number(entry, "m_backSpeed", 0.0)?;
            let strafe = number(entry, "m_strafeSpeed", 0.0)?;
            speeds.push(LocoSpeedDecl {
                speed,
                body: moves(scope(entry, "m_body")),
                bob: moves(scope(entry, "m_bob")),
                arms: [
                    moves(equip_item(entry, "m_leftArm", arm_equip[0])),
                    moves(equip_item(entry, "m_rightArm", arm_equip[1])),
                ],
                speeds: [back, strafe, front, strafe],
            });
        }
        result.push((name.to_owned(), speeds));
    }
    Ok(result)
}

/// An arm's `m_stanceTransitions` (side 0 left, 1 right) for its equip
/// state: (from, to, anim, blend in, blend out; ctor 0.25 / 0.25).
pub fn loco_arm_transitions(locomotion: &Fields, side: usize, equip: &str) -> Result<Vec<(String, String, String, f32, f32)>, String> {
    let (key, blend) = if side == 0 { ("m_leftArm", "m_leftArm") } else { ("m_rightArm", "m_rightArm") };
    let mut result = Vec::new();
    for item in items(locomotion, "m_stanceTransitions") {
        let (Some(from), Some(to)) = (atom(item, "m_fromStance"), atom(item, "m_toStance")) else { continue };
        let Some(anim) = equip_item(item, key, equip).and_then(|e| atom(e, "m_anim")).filter(|a| !a.is_empty()) else { continue };
        result.push((
            from.to_owned(),
            to.to_owned(),
            anim.to_owned(),
            number(item, &format!("{blend}BlendIn"), 0.25)?,
            number(item, &format!("{blend}BlendOut"), 0.25)?,
        ));
    }
    Ok(result)
}

/// The body decl's `m_leftArmAnims` / `m_rightArmAnims` jump additive
/// aliases of an equip state, in FUN_1407730c0's kind order.
pub fn arm_jump_aliases(body: &Fields, side: usize, equip: &str) -> [Option<String>; 6] {
    let key = if side == 0 { "m_leftArmAnims" } else { "m_rightArmAnims" };
    let entry = items(body, key).into_iter().find(|item| atom(item, "m_equipStateName") == Some(equip));
    [
        "m_jumpInAnimAlias",
        "m_jumpLoopAnimAlias",
        "m_jumpOutAnimAlias",
        "m_jumpSneakInAnimAlias",
        "m_jumpSneakLoopAnimAlias",
        "m_jumpSneakOutAnimAlias",
    ]
    .map(|k| entry.and_then(|e| atom(e, k)).filter(|a| !a.is_empty()).map(str::to_owned))
}

/// arkDeclCpntPlayerBody::arkBodyPart m_leftArm / m_rightArm: (m_entity,
/// m_bindTag, m_visible).
pub fn arm_parts(body: &Fields) -> Result<[(Option<String>, String, bool); 2], String> {
    let part = |key: &str| -> Result<(Option<String>, String, bool), String> {
        let p = scope(body, key).unwrap_or(&[]);
        Ok((
            atom(p, "m_entity").filter(|v| !v.is_empty()).map(str::to_owned),
            atom(p, "m_bindTag").unwrap_or("").to_owned(),
            boolean(p, "m_visible", true)?,
        ))
    };
    Ok([part("m_leftArm")?, part("m_rightArm")?])
}

/// `m_stanceTransitions` with their body blend times (ctor FUN_140701770:
/// 0.25 / 0.25).
pub fn loco_transitions(locomotion: &Fields) -> Result<Vec<(String, String, String, f32, f32)>, String> {
    let mut result = Vec::new();
    for item in items(locomotion, "m_stanceTransitions") {
        let (Some(from), Some(to)) = (atom(item, "m_fromStance"), atom(item, "m_toStance")) else { continue };
        result.push((
            from.to_owned(),
            to.to_owned(),
            atom(item, "m_bodyAnim").unwrap_or("").to_owned(),
            number(item, "m_bodyBlendIn", 0.25)?,
            number(item, "m_bodyBlendOut", 0.25)?,
        ));
    }
    Ok(result)
}
