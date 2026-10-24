//! Typed reads of the player's resolved declarations
//! (`entities::player::player_declarations`). Absent fields keep the decl
//! class constructor defaults.
use super::camera::CameraDecl;
use super::controller::{ButtonUsage, StateMapping};
use super::physics::{PhysicsDecl, PhysicsSettings};
use super::shape::{self, ShapeSettings};
use super::states::{
    Attributes, FallingDecl, JumpingDecl, Kind, NavigationDecl, RuleDecl, SprintingDecl, StateDecl, StateId,
};
use crate::entities::player::PlayerDeclarations;
use crate::entities::Value;
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
        physics_shape_transition_speed_factor: number(fields, "m_physicsShapeTransitionSpeedFactor", d.physics_shape_transition_speed_factor)?,
        ..d
    })
}

/// The mode's locomotion state decls; other state kinds are not ported.
pub fn states(declarations: &PlayerDeclarations) -> Result<Vec<StateDecl>, String> {
    let mut result = Vec::new();
    for (kind, fields) in &declarations.states {
        let kind = match kind.as_str() {
            "playerstaterunning" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Running)) => Kind::Running,
            "playerstatesneaking" => Kind::Sneaking,
            "playerstatesprinting" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Sprinting(_))) => {
                let d = SprintingDecl::default();
                Kind::Sprinting(SprintingDecl {
                    minimum_stick: number(fields, "m_minimumStick", d.minimum_stick)?,
                    minimum_speed: number(fields, "m_minimumSpeed", d.minimum_speed)?,
                })
            }
            "playerstatejumping" => {
                let d = JumpingDecl::default();
                Kind::Jumping(JumpingDecl {
                    min_clear_distance_above: number(fields, "m_minClearDistanceAbove", d.min_clear_distance_above)?,
                    jump_height: number(fields, "m_jumpHeight", d.jump_height)?,
                    jump_height_sprinting: number(fields, "m_jumpHeightSprinting", d.jump_height_sprinting)?,
                    jump_height_crouching: number(fields, "m_jumpHeightCrouching", d.jump_height_crouching)?,
                    maximum_velocity: number(fields, "m_maximumVelocity", d.maximum_velocity)?,
                    steepness: number(fields, "m_steepness", d.steepness)?,
                    ceil_collision_factor: number(fields, "m_ceilCollisionFactor", d.ceil_collision_factor)?,
                })
            }
            "playerstatefalling" if !result.iter().any(|s: &StateDecl| matches!(s.kind, Kind::Falling(_))) => {
                let d = FallingDecl::default();
                Kind::Falling(FallingDecl {
                    smooth_gravity_adjustment: number(fields, "m_smoothGravityAdjustment", d.smooth_gravity_adjustment)?,
                })
            }
            _ => continue,
        };
        for key in ["m_bodyParts", "m_lockedBodyParts", "m_exclusiveBodyParts", "m_primary"] {
            if get(fields, key).is_some() && key != "m_primary" {
                return Err(format!("state decl sets {key}; authored body flags are not ported"));
            }
        }
        let mut decl = StateDecl::class_default(kind);
        if let Some(primary) = atom(fields, "m_primary") {
            decl.flags.primary = primary == "true";
        }
        decl.navigation = navigation(fields, decl.navigation.clone())?;
        decl.air_friction = number(fields, "m_airFriction", decl.air_friction)?;
        result.push(decl);
    }
    Ok(result)
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
