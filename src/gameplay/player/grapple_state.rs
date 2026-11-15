//! State27 world-surface traversal, 837a40/838600/8389e0/83a2c0.
//! Sensor27 supplies accepted geometry and shared query shapes; physics
//! integrates authored velocity. NPC catching is a separate actor protocol.
use super::{Player, grapple::{self, Decl, Motion, PreparedSensor, Target},
    states::{Frame, Scenes, StateId}, physics::{self, UNSET}};
use bevy_math::Vec3;
use std::sync::Arc;

pub struct Request {
    pub target: Target,
    pub travel_target: Target,
    pub target_position: Vec3,
    pub target_normal: Vec3,
    pub safe_position: Vec3,
    pub target_velocity: Vec3,
    pub start_posture: usize,
    pub exit_state: StateId,
    pub to_mantle: bool,
    pub can_jump: bool,
    pub jump_to_target: bool,
    pub snapped_ground: bool,
    pub sensor: Arc<PreparedSensor>,
    pub mantle: Option<super::sensors::MantleSensor>,
}
#[derive(Default)]
pub struct Feedback {
    pub stick: Option<Arc<crate::assets::wwise::SoundEventDeclaration>>,
    pub unstick: Option<Arc<crate::assets::wwise::SoundEventDeclaration>>,
}
#[derive(Clone, Copy, Debug)]
pub enum EquipmentEvent { Stick, Unstick }
pub struct Runtime {
    pub request: Request,
    motion: Option<Motion>,
    scenes: Arc<Scenes>,
    scene: u32,
    first_update: bool,
    sticking: bool,
    push_time: f32,
    gravity_disabled: bool,
    extra_legs: bool,
    stick_lock: bool,
    pub finished: bool,
    pub aborted: bool,
    source: Vec3,
    start_yaw: f32,
    start_pitch: f32,
    target_yaw: f32,
    target_pitch: f32,
    target_yaw_offset: f32,
    hits: Vec<super::character_proxy::WorldHit>,
    sensor: grapple::Sensor,
    target_offset: Vec3,
    safe_distance: f32,
    feedback: Arc<Feedback>,
    grappling_posture: usize,
    travel_target: Target,
}
impl Runtime {
    fn new(request: Request, scenes: Arc<Scenes>, feedback: Arc<Feedback>) -> Self {
        let sensor = grapple::Sensor::from_prepared(request.sensor.clone());
        let travel_target = request.travel_target;
        Self { request, motion: None, scenes, scene: 0, first_update: true,
            sticking: false, push_time: 0.0, gravity_disabled: false,
            extra_legs: false, stick_lock: false, finished: false, aborted: false,
            source: Vec3::ZERO, start_yaw: 0.0, start_pitch: 0.0,
            target_yaw: 0.0, target_pitch: 0.0, target_yaw_offset: 0.0, hits: Vec::new(), sensor, target_offset: Vec3::ZERO, safe_distance: 0.0, feedback, grappling_posture: 8, travel_target }
    }
    pub fn sticking(&self) -> bool { self.sticking }
    pub fn target_position(&self) -> Vec3 { self.request.target_position + self.target_offset }
    fn play(&mut self, path: Option<&str>, parts: u8, frame: &mut Frame<'_>) {
        let Some(path) = path else { return };
        let Some(scene) = self.scenes.get(path).or_else(|| frame.scenes.get(path)) else {
            frame.midnight.last_start_error = Some(format!("Grapple scene was not prepared: {path}"));
            self.aborted = true; return;
        };
        match frame.midnight.start_with_parts(scene, frame.md6, 1.0, frame.animations, parts) {
            Ok(handle) => self.scene = handle,
            Err(error) => { frame.midnight.last_start_error = Some(error); self.aborted = true; }
        }
    }
}

pub fn activate(frame: &mut Frame<'_>, decl: &Decl) {
    let Some(mut run) = frame.powers.grapple.take() else { return };
    frame.powers.grapple_pending = false;
    run.source = frame.physics.position(); run.start_yaw = frame.physics.yaw;
    run.start_pitch = frame.camera.pitch;
    // 837a40: translate postures8/9/10 correspond to the saved navigation
    // state; these are installed player shapes, not replacement capsules.
    let posture = match run.request.exit_state { StateId::SNEAKING => 9, StateId::AUTOCROUCH => 10, _ => 8 };
    let posture = posture.max(run.request.start_posture);
    run.grappling_posture = posture;
    frame.physics.set_posture(posture, 0.0);
    frame.body.bob_enabled = false; frame.body.max_speeds = [0.0; 4];
    frame.jump_anim.start(run.request.exit_state == StateId::SNEAKING, 1.0, false);
    frame.physics.instant_motion(); frame.physics.stop_vertical();
    frame.physics.set_gravity_velocity(Vec3::ZERO);
    let mut can_jump = run.request.can_jump;
    let mut extra_speed = 0.0;
    if run.travel_target == Target::Ground {
        let setup = grapple::jump_setup(decl, grapple::JumpInput {
            position: run.source, target_position: run.request.target_position,
            velocity: frame.velocity, player_forward: frame.axis.x_axis,
            camera_direction: frame.camera_axis.x_axis,
            was_sneaking: run.request.exit_state == StateId::SNEAKING,
            snapped_ground: run.request.snapped_ground, to_mantle: run.request.to_mantle,
            can_jump, initial_target: run.request.target,
        });
        can_jump = setup.can_jump; extra_speed = setup.extra_horizontal_speed;
        if let Some((target, speed, gain)) = setup.vertical {
            frame.physics.start_vertical(target, speed, gain); run.request.jump_to_target = true;
        } else { run.request.jump_to_target = false; }
    } else {
        frame.camera.set_target_mode(1, decl.camera_blend_duration);
        run.play(decl.translate_midnight.as_deref(), 5, frame);
        if run.travel_target == Target::Wall { *frame.desired_legs += 1; run.extra_legs = true; }
    }
    if !run.request.jump_to_target { frame.physics.disable_gravity(true); run.gravity_disabled = true; }
    let mut motion = Motion::new(decl, run.travel_target, run.source, run.request.safe_position,
        frame.velocity, run.request.to_mantle, can_jump);
    motion.add_jump_speed(extra_speed); run.motion = Some(motion);
    camera_target(&mut run, frame, decl);
    frame.powers.grapple = Some(run);
}

fn camera_target(run: &mut Runtime, frame: &Frame<'_>, decl: &Decl) {
    // 83bd00 computes the target camera against the endpoint's surface frame.
    let mut query_target = run.request.target_position + run.target_offset;
    let normal = if run.request.target == Target::Ceil {
        query_target.z -= 0.05;
        (query_target - run.source).with_z(0.0).normalize_or_zero()
    } else {
        if run.request.target == Target::Wall { query_target += run.request.target_normal * decl.distance_to_wall; }
        run.request.target_normal.normalize_or_zero()
    };
    run.target_yaw = run.start_yaw + physics::wrap_angle(normal.y.atan2(normal.x) - run.start_yaw);
    let mut from_target = run.source - query_target;
    from_target.z -= frame.physics.posture_settings(run.grappling_posture).height;
    let direction = from_target.normalize_or_zero();
    let horizontal = Vec3::new(direction.x, direction.y, 0.0).normalize_or_zero();
    let basis = crate::gameplay::reference_frame::direction_basis(normal);
    let normal_horizontal = basis.x_axis.with_z(0.0).normalize_or_zero();
    let dot = horizontal.dot(normal_horizontal).clamp(-1.0, 1.0);
    run.target_yaw_offset = dot.acos() * if horizontal.dot(basis.y_axis) < 0.0 { -1.0 } else { 1.0 };
    let turn = bevy_math::Quat::from_rotation_z(-run.target_yaw_offset);
    let pitch_dot = (turn * basis.x_axis).dot(direction).clamp(-1.0, 1.0);
    run.target_pitch = pitch_dot.acos() * if direction.z >= 0.0 { -1.0 } else { 1.0 };
}

fn update_target(run: &mut Runtime, frame: &mut Frame<'_>, decl: &Decl) {
    // 83a330: only ceiling/ground traversal steers the endpoint. Each
    // candidate passes the real sensor's material-aware clearance first.
    if run.travel_target != Target::Wall {
        let offset = run.target_offset + frame.dt * (
            frame.axis.x_axis * frame.controller.input_move.x * decl.max_forward_speed
            + frame.axis.y_axis * frame.controller.input_move.y * decl.max_strafe_speed);
        let candidate = run.request.target_position + offset;
        if run.request.sensor.clear(frame.world, frame.physics, frame.camera.eye, candidate, &mut run.hits) {
            run.target_offset = offset;
            if run.request.to_mantle && offset.truncate().length_squared() > f32::from_bits(0x3c23_d70b) {
                run.request.to_mantle = false;
            }
        }
    }
    let raw_target = run.request.target_position + run.target_offset;
    let mut query_target = raw_target;
    match run.request.target {
        Target::Wall => query_target += run.request.target_normal * decl.distance_to_wall,
        Target::Ceil => query_target.z -= 0.05,
        Target::Ground => {},
    }
    if !run.request.to_mantle {
        let safe = run.sensor.state_safe_position(frame.world, frame.physics, run.source,
            query_target, run.request.target_normal, run.grappling_posture,
            run.request.safe_position, run.safe_distance);
        run.request.safe_position = safe.position; run.safe_distance = safe.distance;
        if safe.shape_changed { run.grappling_posture = safe.posture; frame.physics.set_posture(safe.posture, 0.0); }
    } else { run.request.safe_position = query_target; }
    camera_target(run, frame, decl);
    if let Some(motion) = run.motion.as_mut() {
        motion.safe_position = run.request.safe_position;
        motion.set_reference_distance(run.source.distance(raw_target));
    }
}

fn stick(run: &mut Runtime, frame: &mut Frame<'_>, decl: &Decl) {
    run.sticking = true;
    frame.powers.grapple_equipment_events.push(EquipmentEvent::Stick);
    if let Some(sound) = &run.feedback.stick { frame.powers.sound_events.push(sound.id); }
    frame.body_parts.soft_lock(2); run.stick_lock = true;
    frame.jump_anim.end(run.request.exit_state == StateId::SNEAKING, true);
    if run.travel_target == Target::Wall {
        frame.camera.set_target_mode(8, decl.camera_blend_duration);
        frame.camera.constraints = super::camera::constraints(decl.camera_constraints_wall);
        run.play(decl.wall_midnight.as_deref(), 7, frame);
    } else {
        frame.camera.set_pitch_limits(decl.camera_ceil_max_pitch_up, decl.camera_ceil_max_pitch_down);
        run.play(decl.ceil_midnight.as_deref(), 7, frame);
    }
}

fn turn(frame: &mut Frame<'_>, joystick: f32, mouse: f32, lock_yaw: bool) {
    // 7f04a0 uses the canonical attribute owner and writes radians.
    let scale = frame.attributes.get("ARK_ATTRIBUTE_SPYGLASS_TURNSPEED")
        * frame.attributes.get("ARK_ATTRIBUTE_CAMERA_ROTATION_FACTOR") * 0.017453292;
    let joystick = joystick * scale; let mouse = mouse * scale;
    frame.camera.turn.turn_speed_mouse = mouse;
    frame.camera.turn.turn_speed_joystick = joystick * frame.camera.turn.pitch_speed_ratio;
    let yaw = if lock_yaw { 0.0 } else {
        (if frame.controller.look_is_analog { joystick } else { mouse }) * frame.controller.input_look.y
    };
    frame.requests.push_angular(0, Vec3::new(0.0, 0.0, yaw));
}

fn jump_action(frame: &Frame<'_>) -> i32 {
    frame.controller.mapping.iter().find(|mapping| mapping.state == StateId::JUMPING).map_or(0, |mapping| mapping.action)
}
fn next(run: &Runtime, frame: &Frame<'_>, jump: bool) -> StateId {
    if run.request.to_mantle { return run.request.exit_state; }
    match run.travel_target {
        Target::Wall => if jump || frame.controller.down(jump_action(frame)) { StateId::JUMPING } else { StateId::FALLING },
        Target::Ceil => StateId::FALLING,
        Target::Ground => if frame.physics.ground_distance < f32::MAX
            || frame.physics.ground.state != super::character_proxy::SupportState::Unsupported {
            run.request.exit_state
        } else { StateId::FALLING },
    }
}
/// Returns the source's actual requested navigation state. A rejected
/// clearance query exits through that same owner rather than moving a body.
pub fn update(frame: &mut Frame<'_>, decl: &Decl) -> Option<StateId> {
    // 83a2c0 invokes61c320 even on its first-update skip.
    frame.powers.refresh_owned_cooldown(6);
    let mut run = frame.powers.grapple.take()?;
    let result = if run.aborted { Some(StateId::FALLING) }
    else if run.first_update { run.first_update = false; None }
    else if run.sticking {
        let (joystick, mouse) = if run.travel_target == Target::Wall {
            (decl.max_turn_speed_joystick_wall, decl.max_turn_speed_mouse_wall)
        } else { (decl.max_turn_speed_joystick_ceil, decl.max_turn_speed_mouse_ceil) };
        turn(frame, joystick, mouse, run.travel_target == Target::Wall);
        let moving = frame.controller.input_move.truncate().length_squared() > 0.01;
        run.push_time = if moving { run.push_time + frame.dt } else { 0.0 };
        let jump = frame.controller.pressed(jump_action(frame));
        frame.requests.push_linear(0, Vec3::ZERO);
        frame.physics.proxy.position = run.request.safe_position;
        if run.travel_target == Target::Wall { frame.physics.yaw = run.request.target_normal.y.atan2(run.request.target_normal.x); }
        if jump || moving && run.push_time >= decl.push_delay {
            if let Some(sound) = &run.feedback.unstick { frame.powers.sound_events.push(sound.id); }
            Some(next(&run, frame, jump))
        } else { None }
    } else {
        if run.request.jump_to_target && !frame.physics.vertical_active() && !run.gravity_disabled {
            let posture = match run.request.exit_state { StateId::SNEAKING => 1, StateId::AUTOCROUCH => 2, _ => 0 };
            if frame.physics.posture == run.grappling_posture { frame.physics.set_posture(posture, 0.0); }
            if frame.physics.ground_distance < 0.0 { frame.physics.proxy.position.z -= frame.physics.ground_distance; }
            frame.physics.disable_gravity(true); run.gravity_disabled = true;
        }
        update_target(&mut run, frame, decl);
        let motion = run.motion.as_mut()?;
        let actual = frame.physics.velocity - run.request.target_velocity;
        let mut actual_delta = actual; let mut wanted_delta = motion.velocity;
        if run.request.jump_to_target { actual_delta.z = 0.0; wanted_delta.z = 0.0; }
        let delta = run.request.safe_position - frame.physics.position();
        let width = frame.physics.active_shape().radius + frame.physics.active_shape().extrude_length + 0.01;
        let direction = delta.normalize_or_zero();
        let blocked = !run.request.jump_to_target && actual_delta.length() < wanted_delta.length() - 1.0
            && frame.physics.proxy.contact_hits().any(|hit| {
                let offset = hit.position - frame.physics.position();
                offset.truncate().length_squared() <= width * width
                    && (actual_delta.length() < 0.1 || direction.dot(hit.normal) < -f32::from_bits(0x3f35_04f2))
            });
        let velocity = motion.interpolate_to_target(decl, frame.physics.position(), frame.dt,
            run.request.jump_to_target, run.request.to_mantle) + run.request.target_velocity;
        let clear = run.request.sensor.clear(frame.world, frame.physics, frame.camera.eye,
            frame.camera.eye + velocity * frame.dt, &mut run.hits);
        if !clear || blocked {
            if run.travel_target != Target::Ground && (frame.physics.position() - run.source).length_squared() < f32::from_bits(0x3c23_d70b) {
                stick(&mut run, frame, decl); None
            } else {
                run.travel_target = Target::Ground;
                Some(next(&run, frame, false))
            }
        } else {
            frame.physics.set_motion_limits(UNSET, UNSET,
                if frame.keyboard { decl.max_angular_acceleration_mouse } else { decl.max_angular_acceleration_joystick },
                if frame.keyboard { decl.max_angular_deceleration_mouse } else { decl.max_angular_deceleration_joystick });
            if run.travel_target == Target::Ground {
                turn(frame, decl.max_turn_speed_joystick, decl.max_turn_speed_mouse, false);
            } else { frame.requests.push_angular(0, Vec3::ZERO); }
            frame.requests.push_linear(0, velocity);
            if run.travel_target == Target::Wall {
                let ratio = motion.camera_blend_ratio;
                frame.physics.yaw = (1.0 - ratio) * run.start_yaw + ratio * run.target_yaw;
                frame.camera.pitch = (1.0 - ratio) * run.start_pitch + ratio * run.target_pitch;
                frame.camera.yaw_offset = ratio * run.target_yaw_offset;
            } else if run.travel_target == Target::Ceil {
                let ratio = motion.distance_blend_ratio.sqrt();
                frame.camera.pitch = (1.0 - ratio) * run.start_pitch + ratio * run.target_pitch;
            }
            if motion.goal_reached {
                if run.travel_target == Target::Ground {
                    frame.physics.stop_vertical();
                    if run.request.to_mantle {
                        frame.physics.instant_motion();
                        frame.physics.proxy.position = run.request.safe_position;
                        frame.requests.push_linear(0, Vec3::ZERO);
                    }
                    Some(next(&run, frame, false))
                }
                else {
                    // 838d90 snaps only at the authored arrival branch.
                    frame.physics.proxy.position = run.request.safe_position;
                    stick(&mut run, frame, decl);
                    None
                }
            } else { None }
        }
    };
    if result.is_some() {
        if run.travel_target == Target::Ceil && frame.controller.down(jump_action(frame)) {
            frame.physics.add_impulse(frame.camera_axis.x_axis, decl.ceil_jump_impulse.strength, decl.ceil_jump_impulse.duration);
        }
        run.finished = true;
    }
    frame.powers.grapple = Some(run); result
}
pub fn deactivate(frame: &mut Frame<'_>, decl: &Decl) {
    let Some(run) = frame.powers.grapple.as_mut() else { return };
    if run.sticking { frame.powers.grapple_equipment_events.push(EquipmentEvent::Unstick); }
    if run.stick_lock { frame.body_parts.soft_unlock(2); run.stick_lock = false; }
    if run.gravity_disabled { frame.physics.disable_gravity(false); run.gravity_disabled = false; }
    if frame.physics.vertical_active() { frame.physics.stop_vertical(); }
    let posture = match run.request.exit_state { StateId::SNEAKING => 1, StateId::AUTOCROUCH => 2, _ => 0 };
    frame.physics.set_posture(posture, 0.0);
    frame.body.bob_enabled = true; frame.body.max_speeds = [UNSET; 4];
    frame.jump_anim.end(run.request.exit_state == StateId::SNEAKING, false);
    if run.extra_legs { *frame.desired_legs -= 1; run.extra_legs = false; }
    frame.camera.set_target_mode(0, decl.camera_blend_duration); frame.camera.reset_pitch_limits();
    if frame.physics.ground_distance < 0.0 { frame.physics.proxy.position.z -= frame.physics.ground_distance; }
    if run.scene != 0 { frame.midnight.stop(run.scene); run.scene = 0; }
    run.finished = true; frame.powers.grapple_pending = false;
}

impl Player {
    pub fn begin_grapple(&mut self, request: Request) -> bool {
        if self.powers.grapple_pending || self.machine.is_active(StateId::GRAPPLE)
            || !self.machine.grapple_request_allowed() { return false; }
        let scenes = self.powers.grapple_scenes.clone();
        self.powers.grapple = Some(Runtime::new(request, scenes, self.powers.grapple_feedback.clone()));
        self.powers.grapple_pending = true; self.machine.request(StateId::GRAPPLE); true
    }
    pub fn grapple_state(&self) -> Option<&Runtime> { self.powers.grapple.as_ref() }
    pub fn register_grapple_feedback(&mut self, feedback: Feedback) { self.powers.grapple_feedback = Arc::new(feedback); }
    pub fn take_grapple_equipment_events(&mut self) -> Vec<EquipmentEvent> { std::mem::take(&mut self.powers.grapple_equipment_events) }
    pub fn register_grapple_scenes(&mut self, scenes: Scenes) { self.powers.grapple_scenes = Arc::new(scenes); }
    pub fn set_grapple_target(&mut self, safe_position: Vec3, target_velocity: Vec3) {
        if let Some(run) = self.powers.grapple.as_mut() {
            run.request.safe_position = safe_position; run.request.target_velocity = target_velocity;
            if let Some(motion) = run.motion.as_mut() { motion.safe_position = safe_position; }
        }
    }
    pub fn take_finished_grapple(&mut self) -> Option<Request> {
        if !self.machine.is_active(StateId::GRAPPLE) && self.powers.grapple.as_ref().is_some_and(|run| run.finished) {
            self.powers.grapple.take().map(|run| run.request)
        } else { None }
    }
    pub fn cancel_grapple(&mut self) { self.machine.quit(StateId::GRAPPLE); }
}
