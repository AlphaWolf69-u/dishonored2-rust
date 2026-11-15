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
pub(crate) fn layer_mask(fields: &Fields, key: &str, default: u32) -> Result<u32, String> {
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
pub fn health(fields: &Fields) -> Result<super::health::Settings, String> {
    let maximum = number(fields, "m_health", f32::NAN)?;
    if maximum.is_nan() { return Err("player health decl has no m_health".into()); }
    Ok(super::health::Settings { maximum,
        regen_amount: number(fields, "m_regenAmount", 0.0)?,
        regen_delay: number(fields, "m_regenDelay", 0.0)?,
        regen_rate: number(fields, "m_regenRate", 0.0)?,
        heal_rate: number(fields, "m_healRate", 30.0)?,
    })
}

pub fn mana(fields: &Fields) -> Result<super::mana::Settings, String> {
    let maximum = number(fields, "m_mana", f32::NAN)?;
    if maximum.is_nan() { return Err("player mana decl has no m_mana".into()); }
    Ok(super::mana::Settings { maximum,
        regen_amount: number(fields, "m_regenAmount", 0.0)?,
        regen_delay: number(fields, "m_regenDelay", 0.0)?,
        regen_rate: number(fields, "m_regenRate", 0.0)?,
    })
}

pub fn adrenaline(fields: &Fields) -> Result<super::adrenaline::Settings, String> {
    Ok(super::adrenaline::Settings {
        start_on_spawn: boolean(fields, "m_startOnEntitySpawn", true)?,
        maximum: number(fields, "m_maximumAdrenaline", 100.0)?,
        reduce_delay: number(fields, "m_reduceDelay", 5.0)?,
        reduce_rate: number(fields, "m_reduceRate", 5.0)?,
        burst_rate: number(fields, "m_burstRate", 5.0)?,
    })
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
        "READ_NOTE" => 52,
        "FIRE_WEAPON" => 56,
        "CANCEL_FIRE_WEAPON" => 57,
        "REQUEST_RELOAD_WEAPON" => 58,
        "RELOAD_WEAPON" => 59,
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
        "DEAD" => 15,
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

fn footstep(fields: &Fields, key: &str, default: super::footsteps::Footstep) -> Result<super::footsteps::Footstep, String> {
    let Some(fields) = scope(fields, key) else { return Ok(default) };
    Ok(super::footsteps::Footstep {
        material: match atom(fields, "m_physMat") {
            Some("") => None,
            Some(name) => Some(name.to_owned()),
            None => default.material,
        },
        sound: if atom(fields, "m_soundEvent").is_some() { sound_event_id(fields, "m_soundEvent") } else { default.sound },
        step_length: number(fields, "m_stepLength", default.step_length)?,
    })
}

pub fn footstep_settings(fields: &Fields) -> Result<super::footsteps::FootstepSettings, String> {
    let defaults = super::footsteps::FootstepSettings::default();
    Ok(super::footsteps::FootstepSettings {
        min_speed: number(fields, "m_minSpeedforFootSteps", defaults.min_speed)?,
        min_interval: number(fields, "m_minIntervalBetweenFootSteps", defaults.min_interval)?,
        silent_events: [sound_event_id(fields, "m_silentFootStepOffSoundEvent"), sound_event_id(fields, "m_silentFootStepOnSoundEvent")],
    })
}

fn navigation(fields: &Fields, mut d: NavigationDecl) -> Result<NavigationDecl, String> {
    if let Some(name) = atom(fields, "m_posture") {
        d.posture = shape::posture(name).ok_or_else(|| format!("unknown posture {name}"))?;
    }
    if let Some(name) = atom(fields, "m_nextCrushedState") {
        d.next_crushed_state = state_id(name).unwrap_or(StateId::NULL);
    }
    Ok(NavigationDecl {
        footstep: footstep(fields, "m_footStep", d.footstep.clone())?,
        footstep_upstairs: footstep(fields, "m_footStepUpstairs", d.footstep_upstairs.clone())?,
        footstep_downstairs: footstep(fields, "m_footStepDownstairs", d.footstep_downstairs.clone())?,
        progressive_footstep_ratio: number(fields, "m_progressiveFootStepRatio", d.progressive_footstep_ratio)?,
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

/// Swimming state declarations (constructor 140845e50; field table
/// 14178b540). Preserve the distinct running and sprinting settings.
pub fn swimming_states(declarations: &PlayerDeclarations) -> Result<Vec<super::swimming::SwimmingStateDecl>, String> {
    let mut result = Vec::new();
    for (kind, fields) in &declarations.swimming_states {
        if kind != "playerstateswimming" { continue; }
        let id = atom(fields, "m_id").and_then(state_id).unwrap_or(StateId::RUNNING);
        result.push(super::swimming::SwimmingStateDecl {
            id,
            navigation: navigation(fields, NavigationDecl::default())?,
            up_action: action(fields, "m_upButton", super::controller::ACTION_JUMP)?,
            down_action: action(fields, "m_downButton", super::controller::ACTION_CROUCH)?,
            up_speed: number(fields, "m_upSpeed", 0.5)?,
            down_speed: number(fields, "m_downSpeed", 0.5)?,
            diving_speed_ratio: number(fields, "m_divingSpeedRatio", 0.5)?,
            diving_deceleration: number(fields, "m_divingDeceleration", 20.0)?,
            minimum_stick: number(fields, "m_minimumStick", 0.5)?,
            minimum_speed: number(fields, "m_minimumSpeed", 1.0)?,
        });
    }
    Ok(result)
}

pub fn swimming_death(declarations: &PlayerDeclarations) -> Result<Option<super::states::DrowningDeathDecl>, String> {
    let Some((_, fields)) = declarations.swimming_states.iter().find(|(kind, _)| kind == "playerstatedead") else { return Ok(None) };
    Ok(Some(drowning_death(fields)?))
}

fn drowning_death(fields: &Fields) -> Result<super::states::DrowningDeathDecl, String> {
    Ok(super::states::DrowningDeathDecl {
        duration: seconds(fields, "m_drowningDuration", 6.0)?,
        vertical_speed: number(fields, "m_drowningVerticalSpeed", -1.0)?,
        yaw_speed: number(fields, "m_drowningYawSpeed", 15.0)?,
        look_duration: seconds(fields, "m_drowningLookDuration", 4.0)?,
        fx_delay: seconds(fields, "m_drowningScreenFXDelay", 3.0)?,
        fx: atom(fields, "m_drowningScreenFX").filter(|value| !value.is_empty()).map(str::to_owned),
    })
}

pub fn breath(fields: &Fields) -> Result<super::swimming::Breath, String> {
    Ok(super::swimming::Breath::new(
        number(fields, "m_breathRegen", 20.0)?,
        number(fields, "m_breathRegenDelay", 1.0)?,
    ))
}

pub fn drowning_damage_decl(fields: &Fields) -> Option<&str> {
    atom(fields, "m_drowningDamage").filter(|value| !value.is_empty())
}
pub fn fall_damage_decl(declarations: &PlayerDeclarations) -> Option<&str> {
    declarations.states.iter().find(|(kind, _)| kind == "playerstatefalling")
        .and_then(|(_, fields)| atom(fields, "m_fallDamageDecl"))
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
}

/// Base health filters (140717800/140717dc0) for the downward-origin water
/// damage dispatched by 1404c0cf0. An allowed mask takes precedence.
pub fn accepts_water_damage(health: &Fields, damage: &Fields) -> Result<bool, String> {
    if atom(damage, "m_damageType") != Some("DAMAGETYPE_WATER") {
        return Err("drowning damage uses an unsupported damage type".into());
    }
    let contains_water = |value: &str| value.split(|c: char| c.is_ascii_whitespace() || c == '|' || c == ',')
        .any(|name| name == "DAMAGETYPE_WATER");
    let allowed = atom(health, "m_allowedDamageTypes").filter(|value| !value.is_empty() && *value != "0");
    let accepted = match allowed {
        Some(mask) => contains_water(mask),
        None => !atom(health, "m_ignoredDamageTypes").is_some_and(contains_water),
    };
    let direction = atom(health, "m_allowDamageDirection").unwrap_or("");
    if !direction.is_empty() && direction != "0" {
        return Err(format!("health damage-direction enum is not decoded: {direction}"));
    }
    Ok(accepted)
}

#[derive(Clone, Copy)]
enum SupportedDefaultState {
    Dead, ReadNote, UsePower, Blinking, Running, Sneaking, Sprinting, Jumping, Falling, Sliding,
    Slipping, AutoCrouch, CatFall, Mantle, Vault, Leaning,
    FireWeapon, CancelFireWeapon, RequestReloadWeapon, ReloadWeapon,
}

fn supported_default_state(kind: &str) -> Option<SupportedDefaultState> {
    Some(match kind {
        "playerstatedead" => SupportedDefaultState::Dead,
        "playerstatereadnote" => SupportedDefaultState::ReadNote,
            SupportedDefaultState::UsePower if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::UsePower(_))) => Kind::UsePower(std::sync::Arc::new(super::power_states::UsePowerDecl::read(fields)?)),
            SupportedDefaultState::Blinking if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Blinking)) => Kind::Blinking,
