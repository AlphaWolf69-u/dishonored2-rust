//! arkCpntPlayerBody: the DEFAULT-stage loco inputs (FUN_140777280), the
//! body and bob loco nodes with their phase sync (FUN_140775440), the lean
//! (SetLean FUN_1407747e0, EndLean FUN_140775370, ApplyLean FUN_140776b70)
//! and the pose the camera reads `camera_jnt` from (`animation.rs`).
//!
//! The jump/land camera additives play through the animBasic node
//! (`animation.rs`). Arms, footsteps, hit reactions, follow slope and the
//! swim branch are not ported.
use super::animation::{blend_add, sample, wrap180, AnimBasicCommand, AnimBasicNode, LocoNode, PlayMode, Pose};
use super::camera::BodyPose;
use super::arms::Arm;
use super::midnight::Midnight;
use super::footsteps::{Footstep, Footsteps, FootstepSettings};
use super::sensors::LeanSensor;
use super::states::LocoStances;
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::{self, JointRestPose};
use bevy_math::{Mat3, Quat, Vec3};

pub struct BodySkeleton {
    pub rest: Vec<JointRestPose>,
    pub hierarchy: Vec<[u16; 2]>,
    pub parents: Vec<Option<usize>>,
    /// The md6 init offset as the hierarchy root.
    pub root: JointRestPose,
    pub camera: usize,
    /// head, neck, arms control joints.
    pub controls: [usize; 3],
    /// `anchor_jnt` (DAT_14398df10), the mantle retargeting's slave joint.
    pub anchor: Option<usize>,
}

/// arkDeclCpntPlayerBody fields the port uses (none are overridden).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyDecl {
    pub speed_blend_duration: f32,
    pub sync_duration: f32,
    pub min_sync_playrate: f32,
    pub max_sync_playrate: f32,
    /// m_followSlopeRotationSpeed / m_followSlopeRotationSpeedMidnight
    /// (degrees per second; decl ctor FUN_14076f960: 45 / 90).
    pub follow_slope_rotation_speed: f32,
    pub follow_slope_rotation_speed_midnight: f32,
}

impl Default for BodyDecl {
    fn default() -> Self {
        Self {
            speed_blend_duration: 0.1,
            sync_duration: 0.5,
            min_sync_playrate: 0.75,
            max_sync_playrate: 1.5,
            follow_slope_rotation_speed: 45.0,
            follow_slope_rotation_speed_midnight: 90.0,
        }
    }
}

/// FollowSlope (FUN_140775d10) state: +0xdc4 / +0xdc8 the requests
/// (states count them up and down), +0xdd4 the turned up vector, +0xde0 the
/// body yaw offset the camera writes when it hands its yaw back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowSlope {
    pub requests: i32,
    pub midnight_requests: i32,
    up: Vec3,
    pub yaw_offset: f32,
}

impl Default for FollowSlope {
    fn default() -> Self {
        Self { requests: 0, midnight_requests: 0, up: Vec3::Z, yaw_offset: 0.0 }
    }
}

impl FollowSlope {
    /// `midnight` is a midnight playing on the body (FUN_140765f70(proxy, 1)).
    fn update(&mut self, decl: &BodyDecl, dt: f32, ground_normal: Vec3, supported: bool, midnight: bool) {
        let requests = if midnight { self.midnight_requests } else { self.requests };
        let target = if requests > 0 && supported { ground_normal } else { Vec3::Z };
        let c = target.dot(self.up);
        let angle = if c > -1.0 { if c < 1.0 { c.acos() } else { 0.0 } } else { std::f32::consts::PI };
        let rate = 0.017453292 * if midnight { decl.follow_slope_rotation_speed_midnight } else { decl.follow_slope_rotation_speed };
        if angle <= rate * dt {
            self.up = target;
        } else {
            // Turn toward the target by the rate.
            let axis = self.up.cross(target);
            let axis = axis / axis.length_squared().max(f32::MIN_POSITIVE).sqrt();
            let up = Quat::from_axis_angle(axis, rate * dt) * self.up;
            self.up = up / up.length_squared().max(f32::MIN_POSITIVE).sqrt();
        }
    }

    /// The root joint rotation relative to the entity axis.
    fn rotation(&self, entity: Mat3) -> Mat3 {
        let up = entity.transpose() * self.up;
        let left = up.cross(Vec3::X);
        let left = left / left.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let forward = left.cross(up);
        let forward = forward / forward.length_squared().max(f32::MIN_POSITIVE).sqrt();
        Mat3::from_quat(Quat::from_axis_angle(up, self.yaw_offset)) * Mat3::from_cols(forward, left, up)
    }
}

/// The cvars the body and its states read (`ark_playerSmoothedLoco`, the
/// head-bob option `ark_playerBobAmount`, `ark_playerSmallFallCooldown`) and
/// the difficulty's lean limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodySettings {
    pub smoothed_loco: bool,
    pub bob_amount: f32,
    pub small_fall_cooldown: f32,
    pub lean_limits: [f32; 2],
}

impl Default for BodySettings {
    fn default() -> Self {
        Self { smoothed_loco: true, bob_amount: 1.0, small_fall_cooldown: 0.5, lean_limits: [1.0; 2] }
    }
}


/// arkTimeSmoothedValue<8> (FUN_140770040 / FUN_140770680): a moving
/// average of 1/30 s buckets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smooth8 {
    index: usize,
    ring: [f32; 8],
    remaining: f32,
    accum: f32,
    count: u32,
}

const PERIOD: f32 = f32::from_bits(0x3d08_8889);

impl Default for Smooth8 {
    fn default() -> Self {
        Self { index: 7, ring: [0.0; 8], remaining: PERIOD, accum: 0.0, count: 1 }
    }
}

impl Smooth8 {
    pub fn push(&mut self, value: f32, dt: f32) -> f32 {
        self.accum += value;
        self.count += 1;
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            self.index = (self.index + 1) % 8;
            self.ring[self.index] = self.accum / self.count as f32;
            self.remaining += PERIOD;
            while self.remaining <= 0.0 {
                self.remaining += PERIOD;
                self.index = (self.index + 1) % 8;
                self.ring[self.index] = value;
            }
            self.accum = value;
            self.count = 1;
        }
        self.value()
    }

    fn value(&self) -> f32 {
        let sum: f32 = self.ring.iter().sum();
        let snap = |v: f32| if v.abs() <= 1e-18 { 0.0 } else { v };
        let full = snap(sum / 8.0);
        let shifted = snap((sum - self.ring[(self.index + 1) % 8] + self.accum / self.count as f32) / 8.0);
        let k = self.remaining / PERIOD;
        (1.0 - k) * shifted + k * full
    }
}

/// The animBasic command a tree's IO entry holds after the frame's posts
/// (FUN_1406e1fe0 overwrites it): FUN_140772f90 / FUN_1407730c0 map a jump
/// kind to its item (a missing item stops) and FUN_14053eef0 posts only a
/// weight above 0.
fn last_command(posted: &[AdditiveCommand], items: &[Option<usize>; 6]) -> Option<AnimBasicCommand> {
    let mut command = None;
    for &p in posted {
        match p {
            AdditiveCommand::Play { kind, weight } => match items[kind] {
                Some(idx) if weight > 0.0 => {
                    let (mode, play_on_end) = match kind % 3 {
                        0 => (PlayMode::AndStay, false),
                        1 => (PlayMode::Loop, true),
                        _ => (PlayMode::Once, false),
                    };
                    command = Some(AnimBasicCommand { idx: idx as i32, weight, mode, rate: 1.0, start: 0.0, play_on_end });
                }
                Some(_) => {}
                None => command = Some(AnimBasicCommand::STOP),
            },
            AdditiveCommand::Stop => command = Some(AnimBasicCommand::STOP),
        }
    }
    command
}

/// FUN_140775610: a slave's rate toward the master's phase, when their
/// dominant clip durations match within 0.001 s.
pub fn sync_rate(decl: &BodyDecl, master_progress: f32, d: f32, slave_progress: f32, slave: f32) -> f32 {
    let mut rate = 1.0;
    if d > 0.0 && (d - slave).abs() < 0.001 {
        let (pm, ps) = (master_progress, slave_progress);
        if (pm * d - ps * d).abs() >= 0.001 {
            let sync = decl.sync_duration;
            let mut x = (sync + pm * d).rem_euclid(d);
            x = if x < ps * d { (d - ps * d) + x } else { x - ps * d };
            rate = (x / sync).clamp(decl.min_sync_playrate, decl.max_sync_playrate);
        }
    }
    rate
}

/// FUN_1407721f0: angle smoothing on the unwrapped angle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SmoothAngle {
    smooth: Smooth8,
    last: f32,
    unwrapped: f32,
}

impl SmoothAngle {
    fn push(&mut self, angle: f32, dt: f32) -> f32 {
        self.unwrapped += wrap180(angle - self.last);
        self.last = angle;
        wrap180(self.smooth.push(self.unwrapped, dt))
    }
}

/// The body's lean (+0xe08..+0xe18) as the Leaning state sets it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lean {
    /// m_leanLeft (+ is right), m_leanUp, m_leanBlend, m_leanBlendSpeed.
    pub left: f32,
    pub up: f32,
    pub blend: f32,
    pub blend_speed: f32,
    /// +0xe18, not saved.
    pub rotation_blend: f32,
    /// The difficulty's horizontal/vertical limits (FUN_1406481d0 /
    /// FUN_140648200), set by the host.
    pub limits: [f32; 2],
}

impl Lean {
    /// FUN_1407747e0.
    pub fn set(&mut self, left: f32, up: f32, rotation_blend: f32, sensor: &mut LeanSensor) {
        let [h, v] = self.limits;
        self.left = left.clamp(-h, h);
        self.up = up.clamp(0.0, v);
        self.rotation_blend = rotation_blend;
        self.blend = 1.0;
        self.blend_speed = 0.0;
        if (self.left != 0.0 || self.up != 0.0) && !sensor.enabled {
            sensor.enable(true);
        }
    }

    /// FUN_140775370: fade out over `duration`.
    pub fn end(&mut self, duration: f32) {
        if duration > 0.0 {
            self.blend = 1.0;
            self.blend_speed = 1.0 / duration;
        } else {
            *self = Self { limits: self.limits, ..Self::default() };
        }
    }
}

/// An additives-node command the jump stage posts (FUN_140772f90's table:
/// kinds 0..5 are in, loop, out and their sneak variants; FUN_140540a00
/// stops).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdditiveCommand {
    Play { kind: usize, weight: f32 },
    Stop,
}

/// The body's jump/land camera stage (+0xdb4 m_jumpStage: 0 none, 1 in
/// posted, 2 in running, 3 loop, 4 out; +0xdb8 crouch, +0xdbc weight,
/// +0xdc0 stop at the loop).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JumpAnim {
    pub stage: u32,
    crouch: bool,
    weight: f32,
    pub stop_at_loop: bool,
    pub commands: Vec<AdditiveCommand>,
}

impl JumpAnim {
    pub fn crouch(&self) -> bool {
        self.crouch
    }

    /// FUN_140774f10: `direct` starts at the loop.
    pub fn start(&mut self, crouch: bool, weight: f32, direct: bool) {
        if self.stage & !4 != 0 {
            return;
        }
        self.weight = weight;
        self.crouch = crouch;
        self.stage = if direct { 3 } else { 1 };
        let kind = match (direct, crouch) {
            (false, false) => 0,
            (false, true) => 3,
            (true, false) => 1,
            (true, true) => 4,
        };
        self.commands.push(AdditiveCommand::Play { kind, weight });
    }

    /// FUN_140775270: the landing out clip, or a cancel.
    pub fn end(&mut self, crouch: bool, cancel: bool) {
        if !cancel || self.stage == 0 {
            if self.stage & !4 != 0 {
                self.crouch = crouch;
                self.stage = 4;
                self.commands.push(AdditiveCommand::Play { kind: if crouch { 5 } else { 2 }, weight: self.weight });
            }
        } else {
            self.stage = 0;
            self.commands.push(AdditiveCommand::Stop);
        }
    }

    /// FUN_140776a40 (body update).
    fn tick(&mut self) {
        match self.stage {
            1 => self.stage = 2,
            2 => {
                self.commands.push(AdditiveCommand::Play { kind: if self.crouch { 4 } else { 1 }, weight: self.weight });
                self.stage = 3;
            }
            3 if self.stop_at_loop => {
                self.stage = 0;
                self.commands.push(AdditiveCommand::Stop);
                self.stop_at_loop = false;
            }
            4 => self.stage = 0,
            _ => {}
        }
    }
}

pub struct Body {
    pub skeleton: BodySkeleton,
    pub animations: Vec<EdgeAnimation>,
    pub decl: BodyDecl,
    pub body_node: LocoNode,
    pub bob_node: LocoNode,
    speed_smooth: Smooth8,
    angle_smooth: SmoothAngle,
    move_timer: f32,
    /// m_speedInterpolation (+0x2b0), ramped in at 4/s.
    speed_interpolation: f32,
    pub speed: f32,
    pub footstep: Footstep,
    pub footsteps: Footsteps,
    pub footstep_settings: FootstepSettings,
    pub sound_settings: crate::gameplay::sound::component::Settings,
    pub sound_listener: Option<crate::gameplay::sound::listener::Binding>,
    sound_requests: Vec<crate::gameplay::sound::Request>,
    contact_requests: Vec<crate::gameplay::sound::Contact>,
    pub angle: f32,
    bob_amount: f32,
    pub jump: JumpAnim,
    /// The arm parts shown by the states (FUN_140774a10), left and right.
    pub arms_shown: [bool; 2],
    /// +0xc58 m_desiredLegsVisibility (LEGS-owning states) and +0xc51
    /// m_currentLegsVisibility: the body mesh shows its visible-legs
    /// materials, otherwise the hidden ones.
    pub desired_legs: i32,
    pub legs_visible: bool,
    /// The arm parts (left, right).
    pub arms: Vec<Arm>,
    pub midnight: Midnight,
    pub follow_slope: FollowSlope,
    slope_rotation: Mat3,
    /// The tree's `additives` node and the item index of each jump kind
    /// (arkAdditivesSettings +4..+9).
    pub additives: AnimBasicNode,
    pub jump_items: [Option<usize>; 6],
    pub lean: Lean,
    /// `m_leanLeftAnimAlias` / `m_leanUpAnimAlias` clips (paused leaves of
    /// the tree root) and the frames ApplyLean scrubs them to.
    pub lean_clips: [Option<usize>; 2],
    lean_frames: [f32; 2],
}

/// The midnight cast's md6s (CONDMD6): the body's, then the arms'.
pub fn cast_md6<'a>(body: &'a str, arms: &'a [Arm]) -> [&'a str; 3] {
    let arm = |i: usize| arms.get(i).map_or("", |a| a.model.as_str());
    [body, arm(0), arm(1)]
}

/// The midnight cast's trees: the body and the left and right arms.
pub fn cast<'a>(body: &'a [EdgeAnimation], arms: &'a [Arm]) -> super::midnight::Cast<'a> {
    let arm = |i: usize| arms.get(i).map_or(&[][..], |a| a.animations.as_slice());
    [body, arm(0), arm(1)]
}

impl Body {
    pub fn take_sound_requests(&mut self) -> Vec<crate::gameplay::sound::Request> { std::mem::take(&mut self.sound_requests) }
    pub fn take_contact_requests(&mut self) -> Vec<crate::gameplay::sound::Contact> { std::mem::take(&mut self.contact_requests) }

    /// 776310 -> 770990 for the ordinary player controller. Physics supplies
    /// the dedicated ground material probe; navigation supplies the source.
    pub fn update_footsteps(&mut self, world: &dyn super::character_proxy::PlayerWorld, physics: &super::physics::PlayerPhysics,
        attributes: &super::states::Attributes, dt: f32, now: f64)
    {
        use crate::gameplay::sound::{Contact, Emitter, Request};
        use super::states::StateId;
        let target = physics.ground_material_hit.as_ref().and_then(|hit| world.material(hit));
        let source = self.footstep.material.as_deref();
        let buffer = self.footsteps.buffer;
        let mut attempted = false;
        self.footsteps.update(dt, now, self.speed, self.footstep_settings.min_speed, self.footstep_settings.min_interval, || {
            attempted = true;
            target.is_some() && ((source.is_some() && buffer <= 0.0) || self.footstep.sound.is_some())
        });
        let Some(target) = target.filter(|_| attempted) else { return };
        if let Some(source) = source {
            let silent = match self.footsteps.state {
                StateId::RUNNING => attributes.get("ARK_ATTRIBUTE_PLAYER_RUNNING_FOOTSTEP_SILENT") > f32::EPSILON,
                StateId::SPRINTING => attributes.get("ARK_ATTRIBUTE_PLAYER_SPRINTING_FOOTSTEP_SILENT") > f32::EPSILON,
                _ => false,
            };
            if silent != self.footsteps.silent {
                if let Some(event) = self.footstep_settings.silent_events[usize::from(silent)] {
                    self.sound_requests.push(Request::Post { event, emitter: Emitter::PlayerBody });
                }
                self.footsteps.silent = silent;
            }
            if buffer <= 0.0 {
                self.contact_requests.push(Contact {
                    source: source.to_owned(), target: target.to_owned(), position: physics.position(),
                    normal: Vec3::Z, direction: Vec3::ZERO,
                    speed: self.speed.max(physics.gravity_velocity.z.abs()),
                    // c84ca0's proxy body retains Cinfo's fixed motion 0;
                    // its inverse mass is zero, so 092260 returns zero.
                    mass: 0.0,
                });
            }
        }
        if let Some(event) = self.footstep.sound {
            self.sound_requests.push(Request::Post { event, emitter: Emitter::PlayerBody });
        }
    }

    pub fn new(skeleton: BodySkeleton, animations: Vec<EdgeAnimation>, decl: BodyDecl, body_node: LocoNode, bob_node: LocoNode) -> Self {
        Self {
            skeleton,
            animations,
            decl,
            body_node,
            bob_node,
            speed_smooth: Smooth8::default(),
            angle_smooth: SmoothAngle::default(),
            move_timer: 0.0,
            speed_interpolation: 0.0,
            speed: 0.0,
            footstep: Footstep { step_length: 0.0, ..Footstep::default() },
            footsteps: Footsteps::default(),
            footstep_settings: FootstepSettings::default(),
            sound_settings: Default::default(),
            sound_listener: None,
            sound_requests: Vec::new(),
            contact_requests: Vec::new(),
            angle: 0.0,
            bob_amount: 1.0,
            jump: JumpAnim::default(),
            arms_shown: [true; 2],
            desired_legs: 0,
            legs_visible: false,
            arms: Vec::new(),
            midnight: Midnight::default(),
            follow_slope: FollowSlope::default(),
            slope_rotation: Mat3::IDENTITY,
            additives: AnimBasicNode::default(),
            jump_items: [None; 6],
            lean: Lean { limits: [1.0; 2], ..Lean::default() },
            lean_clips: [None; 2],
            lean_frames: [0.0; 2],
        }
    }

    /// FUN_140776b70 (the body update's Lean step): scrub the lean clips and
    /// return the world offset of the view (pp+0x110). `axis` columns are the
    /// entity's forward, left and up.
    pub fn apply_lean(&mut self, dt: f32, sensor: &mut LeanSensor, axis: Mat3) -> Vec3 {
        let [h, v] = self.lean.limits;
        let lean = &mut self.lean;
        lean.left = lean.left.clamp(-h, h).clamp(-sensor.left_limit, sensor.right_limit);
        lean.up = lean.up.clamp(0.0, v);
        let w = lean.rotation_blend * lean.blend;
        let last_frame = |clip: Option<usize>| clip.map(|c| (self.animations[c].frame_count as f32 - 1.0).max(0.0));
        if let Some(n) = last_frame(self.lean_clips[0]) {
            // 0.5 (0x141ab3764).
            self.lean_frames[0] = (w * lean.left + 1.0) * n * 0.5;
        }
        if let Some(n) = last_frame(self.lean_clips[1]) {
            self.lean_frames[1] = n * lean.up * lean.blend;
        }
        let mut lateral = -(sensor.decl.lean_amplitude * lean.left * w);
        let mut vertical = sensor.decl.lean_up_amplitude * lean.up * lean.blend;
        if lean.blend_speed > 0.0 {
            lean.blend -= lean.blend_speed * dt;
            if lean.blend <= 0.0 {
                lean.blend = 0.0;
                lean.blend_speed = 0.0;
                sensor.enable(false);
                lateral = 0.0;
                vertical = 0.0;
            }
        }
        axis.y_axis * lateral + axis.z_axis * vertical
    }

    /// Transition durations for the states' posture timing (FUN_140771270).
    pub fn stance_transition_durations(&self) -> std::collections::HashMap<(String, String), f32> {
        self.body_node.data.transitions.iter().map(|(key, t)| (key.clone(), self.animations[t.clip].duration())).collect()
    }

    /// DEFAULT stage FUN_140777280 then the tree's node updates.
    /// `velocity` is FUN_1407fdb50 (entity velocity minus physics gravity);
    /// `ground_normal` the physics ground-ray average and `supported` the
    /// controller support state.
    pub fn update(&mut self, dt: f32, stances: &LocoStances, velocity: Vec3, axis: Mat3, settings: BodySettings, ground_normal: Vec3, supported: bool) {
        let mut speed = velocity.length();
        if self.jump.stage != 0 {
            speed = 0.0;
        }
        let mut angle = 0.0;
        if velocity.truncate().length() < 0.001 {
            speed = 0.0;
            self.move_timer = 0.0;
        } else {
            if self.move_timer < self.decl.speed_blend_duration {
                speed = 0.0;
                self.move_timer += dt;
            }
            let yaw = |v: Vec3| v.y.atan2(v.x).to_degrees().rem_euclid(360.0);
            angle = wrap180(yaw(velocity) - yaw(axis.x_axis));
        }
        // Clamp by the navigation state's {back, strafe, fwd, strafe}.
        let a = ((angle + 180.0).rem_euclid(360.0)) as i32 % 360;
        let q = ((a / 90) as usize).min(3);
        let mut max = stances.max_speeds[q];
        if a % 90 != 0 {
            let f = (a % 90) as f32 / 90.0;
            max = f * stances.max_speeds[(q + 1) & 3] + (1.0 - f) * max;
        }
        speed = speed.min(max);
        if settings.smoothed_loco {
            speed = self.speed_smooth.push(speed, dt);
            angle = self.angle_smooth.push(angle, dt);
        }
        self.speed = speed;
        self.angle = angle;
        // BodyPart.Update (FUN_140777180): held at 0 while a midnight plays.
        let midnight = self.midnight.playing().is_some();
        self.speed_interpolation = if midnight { 0.0 } else { (self.speed_interpolation + 4.0 * dt).min(1.0) };
        self.bob_amount = if stances.bob_enabled { settings.bob_amount.clamp(0.0, 1.0) } else { 0.0 };
        self.lean.limits = settings.lean_limits;

        self.phase_sync();
        self.jump.tick();
        // The IO entry keeps the last command posted since the tree's last
        // update (FUN_1406e1fe0 overwrites it).
        let posted: Vec<AdditiveCommand> = self.jump.commands.drain(..).collect();
        let command = last_command(&posted, &self.jump_items);
        self.additives.update(dt, command, &self.animations);
        self.follow_slope.update(&self.decl, dt, ground_normal, supported, midnight);
        self.slope_rotation = self.follow_slope.rotation(axis);

        let body_stance = stances.current.clone().unwrap_or_default();
        let bob_stance = stances.bob.clone().unwrap_or_else(|| body_stance.clone());
        self.body_node.update(dt, &body_stance, speed * self.speed_interpolation, angle, &self.animations);
        self.bob_node.update(dt, &bob_stance, speed, angle, &self.animations);
        // The arm parts (FUN_140777180), phase-synced to the body; their jump
        // additives come from the same stage (FUN_1407730c0).
        let body_duration = self.body_node.dominant_duration(&self.animations);
        let body_progress = self.body_node.progress;
        for arm in &mut self.arms {
            let rate = sync_rate(&self.decl, body_progress, body_duration, arm.loco.progress, arm.dominant_duration());
            let command = last_command(&posted, &arm.jump_items);
            arm.update(dt, &body_stance, speed, angle, midnight, rate, command);
        }
        for side in 0..self.arms.len() {
            self.midnight.update_node(side + 1, dt);
        }
        // Clips are sampled once at load, so this cannot fail; a failure
        // would leave the frame without root motion.
        if self.midnight.update(dt, &self.animations, &self.skeleton.rest).is_err() {
            self.midnight.root = None;
        }
        self.update_legs();
    }

    /// FUN_140776e60: show the legs as soon as a LEGS owner wants them;
    /// hiding waits while the body's midnight player is busy (shadow walk,
    /// the bypass flag +0xc50 and the +0x318 guard are not ported).
    fn update_legs(&mut self) {
        let visible = self.desired_legs >= 1;
        if visible == self.legs_visible {
            return;
        }
        if visible || self.midnight.playing().is_none() {
            self.legs_visible = visible;
        }
    }

    /// FUN_140775440 / FUN_140775610 for the bob slave.
    fn phase_sync(&mut self) {
        let d = self.body_node.dominant_duration(&self.animations);
        let slave = self.bob_node.dominant_duration(&self.animations);
        self.bob_node.rate_multiplier = sync_rate(&self.decl, self.body_node.progress, d, self.bob_node.progress, slave);
    }

    /// GetAnimModelSpaceJointTransform (FUN_14053e220 → FUN_14024b280): `joint`
    /// of `clip` at `frame` relative to joint 0, with the origin
    /// translation zeroed: (translation, rotation).
    pub fn anchor_transform(&self, clip: usize, frame: f32, joint: usize) -> Option<(Vec3, Mat3)> {
        let animation = &self.animations[clip];
        let mut joints = animation.sample(&self.skeleton.rest, frame / animation.frame_rate).ok()?;
        joints.first_mut()?.translation = [0.0; 3];
        let matrices = edge_skeleton::pose_matrices(&joints, &self.skeleton.hierarchy, &self.skeleton.root).ok()?;
        let split = |m: &[[f32; 4]; 3]| {
            let r = Mat3::from_cols(Vec3::new(m[0][0], m[1][0], m[2][0]), Vec3::new(m[0][1], m[1][1], m[2][1]), Vec3::new(m[0][2], m[1][2], m[2][2]));
            (Vec3::new(m[0][3], m[1][3], m[2][3]), r)
        };
        let (t0, r0) = split(matrices.first()?);
        let (t, r) = split(matrices.get(joint)?);
        Some((r0.transpose() * (t - t0), r0.transpose() * r))
    }

    /// The model-space pose: body ⊕ bobAmount·bob ⊕ additives, then the lean layers.
    pub fn pose(&mut self) -> Result<BodyPose, String> {
        let skeleton = &self.skeleton;
        let mut pose = match self.body_node.evaluate(&self.animations, &skeleton.rest)? {
            Some(pose) => pose,
            None => Pose { joints: skeleton.rest.clone(), weights: vec![255; skeleton.rest.len()] },
        };
        if self.bob_amount > 1e-4 {
            if let Some(bob) = self.bob_node.evaluate(&self.animations, &skeleton.rest)? {
                blend_add(&mut pose, &bob, self.bob_amount);
            }
        }
        if let Some(additives) = self.additives.evaluate(&self.animations, &skeleton.rest)? {
            blend_add(&mut pose, &additives, 1.0);
        }
        // The midnight blendarray over locoBobAdditives, then its snapshot
        // blender.
        let mut pose = self.midnight.evaluate(pose, &self.animations, &skeleton.rest)?;

        // The tree root's blendarray adds its first children last: leafLeanUp,
        // then leafLeanLeft, both ADD_RIGHT 1.0 at their scrubbed frame.
        for (clip, frame) in self.lean_clips.into_iter().zip(self.lean_frames).rev() {
            if let Some(clip) = clip {
                let animation = &self.animations[clip];
                let layer = sample(animation, &skeleton.rest, frame / animation.frame_rate)?;
                blend_add(&mut pose, &layer, 1.0);
            }
        }
        let matrices = edge_skeleton::pose_matrices(&pose.joints, &skeleton.hierarchy, &skeleton.root)?;
        let mut positions: Vec<Vec3> = matrices.iter().map(|m| Vec3::new(m[0][3], m[1][3], m[2][3])).collect();
        let mut rotations: Vec<Mat3> = matrices
            .iter()
            .map(|m| {
                Mat3::from_cols(
                    Vec3::new(m[0][0], m[1][0], m[2][0]),
                    Vec3::new(m[0][1], m[1][1], m[2][1]),
                    Vec3::new(m[0][2], m[1][2], m[2][2]),
                )
            })
            .collect();
        // FollowSlope's root joint rotation turns the whole skeleton.
        let turn = self.slope_rotation;
        if turn != Mat3::IDENTITY && !positions.is_empty() {
            let pivot = positions[0];
            for (p, r) in positions.iter_mut().zip(&mut rotations) {
                *p = pivot + turn * (*p - pivot);
                *r = turn * *r;
            }
        }
        Ok(BodyPose {
            positions,
            rotations,

            parents: skeleton.parents.clone(),
            camera: skeleton.camera,
            controls: skeleton.controls,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lean_offset_follows_the_sensor_amplitudes_and_fades_out() {
        use crate::gameplay::player::sensors::{LeanDecl, LeanSensor};
        let mut sensor = LeanSensor::new(LeanDecl::default());
        let mut lean = Lean { limits: [1.0; 2], ..Lean::default() };
        lean.set(1.0, 0.5, 1.0, &mut sensor);
        assert!(sensor.enabled);
        let skeleton = BodySkeleton {
            rest: Vec::new(),
            hierarchy: Vec::new(),
            parents: Vec::new(),
            root: JointRestPose { rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3], scale: [1.0; 3] },
            camera: 0,
            controls: [0; 3],
            anchor: None,
        };
        let empty = || LocoNode::new(Default::default());
        let mut body = Body::new(skeleton, Vec::new(), BodyDecl::default(), empty(), empty());
        body.lean = lean;
        // Right (+1) moves the view along -left by the 0.8 amplitude; up by
        // 0.35 * up.
        let offset = body.apply_lean(0.0, &mut sensor, Mat3::IDENTITY);
        assert!(offset.distance(Vec3::new(0.0, -0.8, 0.175)) < 1e-6, "{offset:?}");
        body.lean.end(0.3);
        let offset = body.apply_lean(0.15, &mut sensor, Mat3::IDENTITY);
        assert!(offset.distance(Vec3::new(0.0, -0.8, 0.175)) < 1e-6, "{offset:?}");
        assert!((body.lean.blend - 0.5).abs() < 1e-6);
        let offset = body.apply_lean(0.1, &mut sensor, Mat3::IDENTITY);
        assert!(offset.distance(Vec3::new(0.0, -0.4, 0.0875)) < 1e-6, "{offset:?}");
        let offset = body.apply_lean(0.2, &mut sensor, Mat3::IDENTITY);
        assert_eq!(offset, Vec3::ZERO);
        assert!(!sensor.enabled && body.lean.blend == 0.0);
    }

    #[test]
    fn jump_stage_posts_in_loop_and_out_and_a_cancel_stops() {
        use AdditiveCommand::{Play, Stop};
        let mut jump = JumpAnim::default();
        jump.start(false, 0.33, false);
        assert_eq!((jump.stage, jump.commands.clone()), (1, vec![Play { kind: 0, weight: 0.33 }]));
        jump.tick();
        jump.tick();
        assert_eq!(jump.stage, 3);
        jump.start(true, 1.0, false);
        assert_eq!(jump.commands.last(), Some(&Play { kind: 1, weight: 0.33 }), "a running stage ignores a start");
        jump.end(true, false);
        assert_eq!((jump.stage, jump.commands.last()), (4, Some(&Play { kind: 5, weight: 0.33 })));
        jump.tick();
        assert_eq!(jump.stage, 0);
        jump.start(true, 1.0, true);
        assert_eq!((jump.stage, jump.commands.last()), (3, Some(&Play { kind: 4, weight: 1.0 })));
        jump.end(false, true);
        assert_eq!((jump.stage, jump.commands.last()), (0, Some(&Stop)));
    }

    #[test]
    fn lean_is_clamped_by_the_sensor_limits() {
        use crate::gameplay::player::sensors::{LeanDecl, LeanSensor};
        let mut sensor = LeanSensor::new(LeanDecl::default());
        sensor.right_limit = 0.25;
        let mut lean = 0.2;
        assert!(sensor.clamp(&mut lean) && lean == 0.2);
        lean = 0.25;
        assert!(!sensor.clamp(&mut lean) && lean == 0.25);
        lean = -1.5;
        assert!(!sensor.clamp(&mut lean) && lean == -1.0);
    }

    #[test]
    fn smoother_converges_to_a_constant_input() {
        let mut s = Smooth8::default();
        let mut v = 0.0;
        for _ in 0..240 {
            v = s.push(3.6, 1.0 / 120.0);
        }
        assert!((v - 3.6).abs() < 1e-4, "{v}");
    }
}
