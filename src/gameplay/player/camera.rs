//! arkCpntPlayerCamera: the DEFAULT-stage update FUN_1407f6140 (deferred
//! target modes, the cosine mode blend, the free FUN_1407f1400 and
//! constrained FUN_1407f0bd0 look integrators, the per-mode joint controls),
//! the speed FOV FUN_1407f7500 and the view placement FUN_1407f6a80
//! (camera_jnt in world space, clamped into the capsule).
//!
//! The view origin is the body's `camera_jnt`. The modes the locomotion
//! states use are ported: AND_ARMS 0 (FUN_1407f4cd0), ANIMATED 2
//! (FUN_1407f5040), CONSTRAINED 3 (FUN_1407f5680), CONSTRAINED_WITH_ARMS 6
//! (FUN_1407f58f0) and CATFALL 14 (FUN_1407f5190). Arm controls, look-at,
//! possession and cinematic modes are not.
use super::body::Smooth8;
use super::physics::PlayerPhysics;

use super::states::CameraTurn;
use bevy_math::{Mat3, Quat, Vec3};

/// arkDeclCpntPlayerCamera (ctor FUN_1407efa90) fields the port uses.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraDecl {
    /// Degrees.
    pub max_pitch_up: f32,
    pub max_pitch_down: f32,
    pub pitch_speed_ratio: f32,
    pub camera_joint: String,
    pub head_joint: String,
    pub neck_joint: String,
    pub arms_joint: String,
    /// head, neck, arms.
    pub forward_ratios: [f32; 3],
    pub backward_ratios: [f32; 3],
    pub speed_fov_additional_value: f32,
    pub speed_fov_max_increase_speed: f32,
    pub speed_fov_max_decrease_speed: f32,
    /// Degrees per second.
    pub constraints_blend_speed: f32,
    pub mode_blend_duration: f32,
    /// m_leftArmControlJointName / m_rightArmControlJointName.
    pub arm_joints: [String; 2],
    /// m_armsInertiaMin/MaxAngularSpeed (deg/s), m_armsInertiaMaxAngle (deg).
    pub arms_inertia_min_angular_speed: f32,
    pub arms_inertia_max_angular_speed: f32,
    pub arms_inertia_max_angle: f32,
    /// m_leftArmPitchShiftRatio / m_rightArmPitchShiftRatio.
    pub arm_pitch_shift_ratios: [f32; 2],
}

impl Default for CameraDecl {
    fn default() -> Self {
        Self {
            max_pitch_up: 80.0,
            max_pitch_down: 80.0,
            pitch_speed_ratio: 0.75,
            camera_joint: String::new(),
            head_joint: String::new(),
            neck_joint: String::new(),
            arms_joint: String::new(),
            forward_ratios: [0.1, 0.2, 0.7],
            backward_ratios: [0.6, 0.3, 0.1],
            speed_fov_additional_value: 10.0,
            speed_fov_max_increase_speed: 15.0,
            speed_fov_max_decrease_speed: 30.0,
            constraints_blend_speed: 200.0,
            mode_blend_duration: 0.25,
            arm_joints: [String::new(), String::new()],
            arms_inertia_min_angular_speed: 10.0,
            arms_inertia_max_angular_speed: 200.0,
            arms_inertia_max_angle: 5.0,
            arm_pitch_shift_ratios: [0.95, 0.925],
        }
    }
}

/// Model-space joint transforms of the body pose (origin at the feet, +X
/// forward, +Z up): position and rotation for each joint plus each joint's
/// parent.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPose {
    pub positions: Vec<Vec3>,
    pub rotations: Vec<Mat3>,
    pub parents: Vec<Option<usize>>,
    pub camera: usize,
    /// head, neck, arms (center_spine03) control joints.
    pub controls: [usize; 3],
}

impl BodyPose {
    fn descends_from(&self, mut joint: usize, ancestor: usize) -> bool {
        loop {
            if joint == ancestor {
                return true;
            }
            match self.parents[joint] {
                Some(parent) => joint = parent,
                None => return false,
            }
        }
    }

    /// FUN_1407f4cd0's joint control: the arms, neck and head joints rotate
    /// about the body's left axis by their share of the pitch (positive pitch
    /// looks down); returns camera_jnt's model-space position and rotation.
    pub fn camera_with_pitch(&self, pitch: f32, forward_ratios: [f32; 3], backward_ratios: [f32; 3]) -> (Vec3, Mat3) {
        let ratios = if pitch > 0.0 { forward_ratios } else { backward_ratios };
        let turn = |r: f32| Mat3::from_quat(Quat::from_axis_angle(Vec3::Y, pitch * r));
        let controls = [(self.controls[2], turn(ratios[2])), (self.controls[1], turn(ratios[1])), (self.controls[0], turn(ratios[0]))];
        self.joint_with_controls(self.camera, &controls)
    }

    /// `joint`'s model-space position and rotation after joint controls,
    /// applied in order, each about its control joint in model space.
    pub fn joint_with_controls(&self, joint: usize, controls: &[(usize, Mat3)]) -> (Vec3, Mat3) {
        let mut position = self.positions[joint];
        let mut rotation = self.rotations[joint];
        for &(control, turn) in controls {
            if !self.descends_from(joint, control) {
                continue;
            }
            let pivot = self.positions[control];
            position = pivot + turn * (position - pivot);
            rotation = turn * rotation;
        }
        (position, rotation)
    }
}

/// arkCpntPlayerCamera::mode_t (table 0x14239dad0).
pub const MODE_AND_ARMS: i32 = 0;
pub const MODE_ANIMATED: i32 = 2;
pub const MODE_CONSTRAINED: i32 = 3;
pub const MODE_CONSTRAINED_WITH_ARMS: i32 = 6;
pub const MODE_CATFALL: i32 = 14;
pub const MODE_KEEP_PREVIOUS: i32 = 18;
/// Modes integrated by FUN_1407f0bd0.
const CONSTRAINED_MODES: u32 = 0x14748;
/// Modes whose yaw offset becomes body yaw on leaving them.
const YAW_OFFSET_MODES: u32 = 0x4148;

fn in_mask(mask: u32, mode: i32) -> bool {
    (0..32).contains(&mode) && mask >> mode & 1 != 0
}

/// FUN_1407f01e0: decl constraint params {maxPitchUp, maxPitchDown,
/// maxYawLeft, maxYawRight} in degrees → {pitch low, pitch high, yaw high,
/// yaw low} in radians.
pub fn constraints(params: [f32; 4]) -> [f32; 4] {
    let r = 0.017453292;
    [-(r * params[0]), r * params[1], r * params[2], -(r * params[3])]
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerCamera {
    pub decl: CameraDecl,
    /// cam+0x25264, radians, positive looks down.
    pub pitch: f32,
    pitch_rate: f32,
    /// cam+0x251a4 / +0x251a8.
    min_pitch: f32,
    max_pitch: f32,
    pub turn: CameraTurn,
    /// cam+0x2547c, degrees.
    pub fov: f32,
    out_of_shape: bool,
    /// +0x2527c: the gameplay eye FUN_1407f4b30 writes after the view.
    pub eye: Vec3,
    /// +0x25200 / +0x25204 and the deferred target requests (+0x25208).
    pub current_mode: i32,
    pub target_mode: i32,
    deferred: Vec<(i32, f32)>,
    /// +0x25258 / +0x2525c.
    blend_duration: f32,
    blend_time: f32,
    /// +0x25274 yaw offset (+ looks left) and its rate +0x252b0.
    pub yaw_offset: f32,
    yaw_rate: f32,
    /// +0x25260: the body yaw offset follows the blend out.
    restore_yaw: bool,
    /// +0x251c0: pitch low/high, yaw high/low (radians).
    pub constraints: [f32; 4],
    /// The poses of this update: (mode, weight, arms lag).
    poses: Vec<(i32, f32, f32)>,
    /// +0x252b4 m_yawInertia's smoother.
    yaw_inertia: Smooth8,
}

pub struct View {
    pub origin: Vec3,
    /// Columns forward, left, up (game Z-up).
    pub axis: Mat3,
}

impl PlayerCamera {
    pub fn new(decl: CameraDecl, base_fov: f32) -> Self {
        let turn = CameraTurn { pitch_speed_ratio: decl.pitch_speed_ratio, ..Default::default() };
        Self {
            min_pitch: -decl.max_pitch_up * 0.017453292,
            max_pitch: decl.max_pitch_down * 0.017453292,
            decl,
            pitch: 0.0,
            pitch_rate: 0.0,
            turn,
            fov: base_fov,
            out_of_shape: false,
            eye: Vec3::ZERO,
            current_mode: MODE_AND_ARMS,
            target_mode: MODE_AND_ARMS,
            deferred: Vec::new(),
            blend_duration: 0.0,
            blend_time: 0.0,
            yaw_offset: 0.0,
            yaw_rate: 0.0,
            restore_yaw: false,
            constraints: constraints([30.0, 30.0, 45.0, 45.0]),
            poses: vec![(MODE_AND_ARMS, 1.0, 0.0)],
            yaw_inertia: Smooth8::default(),
        }
    }

    /// FUN_1407f43b0: queue a target mode (KEEP_PREVIOUS keeps it); a
    /// negative blend takes the decl's `m_modeBlendDuration`.
    pub fn set_target_mode(&mut self, mode: i32, blend: f32) {
        if mode == MODE_KEEP_PREVIOUS {
            return;
        }
        let last = self.deferred.last().map_or(self.target_mode, |&(m, _)| m);
        if mode != last {
            self.deferred.push((mode, blend));
        }
    }

    /// FUN_1407f4930.
    fn start_mode(&mut self, mode: i32, blend: f32, physics: &mut PlayerPhysics, body_yaw: &mut f32) {
        if mode == self.target_mode {
            return;
        }
        self.target_mode = mode;
        if mode == self.current_mode {
            self.blend_duration = 0.0;
            self.blend_time = 0.0;
            return;
        }
        self.blend_duration = if blend < 0.0 { self.decl.mode_blend_duration } else { blend };
        self.blend_time = 0.0;
        if self.blend_duration > 0.0 {
            if in_mask(YAW_OFFSET_MODES, self.current_mode) && !in_mask(YAW_OFFSET_MODES, mode) {
                // The body takes over the head yaw.
                self.restore_yaw = true;
                physics.add_yaw(self.yaw_offset);
                *body_yaw = -self.yaw_offset;
            } else {
                self.restore_yaw = false;
                *body_yaw = 0.0;
            }
        }
    }

    /// The DEFAULT stage FUN_1407f6140 up to the FOV: modes, look
    /// integration and this update's poses. `look` is the controller's
    /// m_inputLook (x pitch, y yaw) and `body_yaw` the body's +0xde0.
    /// `yaw_rate` is the entity's angular velocity z (+0x350).
    pub fn update(&mut self, dt: f32, look: Vec3, look_is_analog: bool, physics: &mut PlayerPhysics, body_yaw: &mut f32, yaw_rate: f32) {
        if dt == 0.0 {
            return;
        }
        while self.current_mode == self.target_mode && !self.deferred.is_empty() {
            let (mode, blend) = self.deferred.remove(0);
            self.start_mode(mode, blend, physics, body_yaw);
        }
        if self.current_mode != self.target_mode && self.blend_duration <= self.blend_time {
            self.current_mode = self.target_mode;
            self.blend_time = self.blend_duration;
        }
        let t = if self.blend_duration > 0.0 { self.blend_time / self.blend_duration } else { 1.0 };
        // 0.5 (DAT_141ab3764), π (DAT_141ad6f00).
        let w = 0.5 - (t * std::f32::consts::PI).cos() * 0.5;
        if self.restore_yaw {
            if self.current_mode == self.target_mode {
                self.restore_yaw = false;
                *body_yaw = 0.0;
            } else {
                *body_yaw = -((1.0 - w) * self.yaw_offset);
            }
        }
        let (current, target) = (self.current_mode, self.target_mode);
        let animated = |m: i32| m == MODE_ANIMATED || m == 11;
        if animated(current) && animated(target) {
            self.pitch = 0.0;
            self.yaw_offset = 0.0;
        } else if in_mask(CONSTRAINED_MODES, target) || current == 13 || target == 13 || in_mask(CONSTRAINED_MODES, current) {
            self.constrained(dt, look, look_is_analog, physics);
        } else {
            self.free(dt, look.x, look_is_analog, physics);
        }
        self.poses.clear();
        let mut pose = |camera: &mut Self, mode: i32, w: f32| {
            // Modes 0, 1 and 2 sample the arms lag once each.
            let lag = if matches!(mode, 0..=2) { camera.arms_lag(yaw_rate, dt) } else { 0.0 };
            camera.poses.push((mode, w, lag));
        };
        if current != target && 1.0 - w > 0.0 {
            pose(self, current, 1.0 - w);
        }
        if w > 0.0 {
            pose(self, target, w);
        }
        if current != target {
            self.blend_time += dt;
        }
    }

    /// FUN_1407f1750 with no vertical term (camera Start passes 0, 0).
    fn arms_lag(&mut self, yaw_rate: f32, dt: f32) -> f32 {
        let r = 0.017453292;
        let (low, high) = (r * self.decl.arms_inertia_min_angular_speed, r * self.decl.arms_inertia_max_angular_speed);
        let b = ((yaw_rate.abs() - low) / (high - low)).clamp(0.0, 1.0);
        let sign = if yaw_rate > 0.0 { 1.0 } else if yaw_rate < 0.0 { -1.0 } else { 0.0 };
        self.yaw_inertia.push(sign * b * r * self.decl.arms_inertia_max_angle, dt)
    }

    /// The shoulder controls of this update's poses (FUN_1407f4cd0, 5040,
    /// 5680, 58f0 arm parts; m_rightArmFollow 1), as entity-space rotations
    /// applied in order about each shoulder. The aiming springs stay at
    /// rest with empty hands.
    pub fn arm_controls(&self, body_yaw: f32) -> [Vec<Mat3>; 2] {
        let ratios = if self.pitch > 0.0 { self.decl.forward_ratios } else { self.decl.backward_ratios };
        let (head, neck, arms) = (self.pitch * ratios[0], self.pitch * ratios[1], self.pitch * ratios[2]);
        let axis = Mat3::from_rotation_z(body_yaw) * Vec3::Y;
        let pitch = |a: f32| Mat3::from_quat(Quat::from_axis_angle(axis, a));
        let mut controls = [Vec::new(), Vec::new()];
        for &(mode, w, lag) in &self.poses {
            // The arms trail a turn: positive yaw rate turns them clockwise.
            let trail = Mat3::from_rotation_z(-lag * w);
            for (side, list) in controls.iter_mut().enumerate() {
                match mode {
                    MODE_AND_ARMS => {
                        list.push(pitch((head + neck) * self.decl.arm_pitch_shift_ratios[side] * w));
                        list.push(trail);
                    }
                    MODE_ANIMATED => list.push(trail),
                    // The arms keep level against the spine03 pitch.
                    MODE_CONSTRAINED => list.push(pitch(-w * arms)),
                    _ => {}
                }
            }
        }
        controls
    }

    fn limit_rate(physics: &PlayerPhysics, mut rate: f32, previous: f32, dt: f32) -> f32 {
        // FUN_1401b3a00 / FUN_1407fcbf0 with the physics angular limits.
        let limit = physics.angular_limit(rate, previous);
        if (rate - previous).abs() > dt * limit {
            rate = previous + (rate - previous).signum() * dt * limit;
        }
        rate
    }

    /// FUN_1407f1400 (the free camera of modes 0/1/2/7/11/12/15).
    fn free(&mut self, dt: f32, input_pitch: f32, look_is_analog: bool, physics: &PlayerPhysics) {
        let rate = if look_is_analog { self.turn.turn_speed_joystick } else { self.turn.turn_speed_mouse } * input_pitch;
        let rate = Self::limit_rate(physics, rate, self.pitch_rate, dt);
        let previous = self.pitch;
        let in_range = self.min_pitch <= previous && previous <= self.max_pitch;
        self.pitch = (previous + dt * rate).clamp(self.min_pitch, self.max_pitch);
        self.pitch_rate = if in_range && dt > 0.0 { (self.pitch - previous) / dt } else { 0.0 };
        self.yaw_offset = 0.0;
    }

    /// FUN_1407f0fa0 / FUN_1407f0b30: one constrained axis, slowing toward
    /// the edge it moves to.
    fn constrained_axis(&self, value: f32, low: f32, high: f32, input: f32, analog: bool, previous: &mut f32, dt: f32, physics: &PlayerPhysics) -> f32 {
        let t = if high - low == 0.0 { 0.0 } else { (value - low) / (high - low) };
        let mut rate = input * if analog { self.turn.turn_speed_joystick } else { self.turn.turn_speed_mouse };
        // 2.0 (DAT_141ab3b20).
        if input > 0.0 {
            let s = (t * 2.0 - 1.0).max(0.0);
            rate *= 1.0 - s * s * s;
        } else if input < 0.0 {
            let s = ((1.0 - t) * 2.0 - 1.0).max(0.0);
            rate *= 1.0 - s * s * s;
        }
        rate = Self::limit_rate(physics, rate, *previous, dt);
        *previous = rate;
        dt * rate + value
    }

    /// FUN_1407f0bd0.
    fn constrained(&mut self, dt: f32, look: Vec3, analog: bool, physics: &PlayerPhysics) {
        let (pitch0, yaw0) = (self.pitch, self.yaw_offset);
        let [lo_p, hi_p, hi_y, lo_y] = self.constraints;
        let mut rate = self.pitch_rate;
        self.pitch = self.constrained_axis(pitch0, lo_p, hi_p, look.x, analog, &mut rate, dt, physics);
        self.pitch_rate = rate;
        let mut rate = self.yaw_rate;
        self.yaw_offset = self.constrained_axis(yaw0, lo_y, hi_y, look.y, analog, &mut rate, dt, physics);
        self.yaw_rate = rate;
        // deg2rad * 0.5 (DAT_143991e20).
        let tolerance = 0.017453292 * 0.5;
        let out = |p: f32, y: f32| lo_p - p > tolerance || p - hi_p > tolerance || lo_y - y > tolerance || y - hi_y > tolerance;
        if !out(self.pitch, self.yaw_offset) {
            return;
        }
        if out(pitch0, yaw0) {
            // Ease a view that starts outside the box back into it.
            let (mut p, mut y) = (hi_p, self.yaw_offset);
            if f32::EPSILON <= hi_p - lo_p {
                let v = self.pitch;
                if lo_p <= v {
                    p = v;
                    if hi_p < v {
                        y *= hi_p / v;
                        p = v * (hi_p / v);
                    }
                } else {
                    y *= lo_p / v;
                    p = v * (lo_p / v);
                }
            }
            let mut yaw = hi_y;
            if f32::EPSILON <= hi_y - lo_y {
                if lo_y <= y {
                    yaw = y;
                    if hi_y < y {
                        p *= hi_y / y;
                        yaw = y * (hi_y / y);
                    }
                } else {
                    p *= lo_y / y;
                    yaw = y * (lo_y / y);
                }
            }
            let (dp, dy) = (p - pitch0, yaw - yaw0);
            let length2 = dp * dp + dy * dy;
            let inverse = 1.0 / length2.max(f32::MIN_POSITIVE).sqrt();
            let step = (length2 * inverse * 0.5).min(0.017453292 * self.decl.constraints_blend_speed * dt);
            self.pitch = dp * inverse * step + pitch0;
            self.yaw_offset = dy * inverse * step + yaw0;
        } else {
            self.pitch = self.pitch.clamp(lo_p, hi_p).max(lo_p);
            self.yaw_offset = if self.yaw_offset < lo_y { lo_y } else if self.yaw_offset <= hi_y { self.yaw_offset } else { hi_y };
        }
        self.pitch_rate = (self.pitch - pitch0) / dt;
        self.yaw_rate = (self.yaw_offset - yaw0) / dt;
    }

    /// The joint controls of this update's poses (pitch about the body's
    /// left, turned by the body yaw offset; yaw about up).
    pub fn controls(&self, pose: &BodyPose, body_yaw: f32) -> Vec<(usize, Mat3)> {
        let ratios = if self.pitch > 0.0 { self.decl.forward_ratios } else { self.decl.backward_ratios };
        let (head, neck, arms) = (self.pitch * ratios[0], self.pitch * ratios[1], self.pitch * ratios[2]);
        let axis = Mat3::from_rotation_z(body_yaw) * Vec3::Y;
        let pitch = |a: f32| Mat3::from_quat(Quat::from_axis_angle(axis, a));
        let yaw = |a: f32| Mat3::from_rotation_z(a);
        let [head_joint, neck_joint, arms_joint] = pose.controls;
        let mut controls = Vec::new();
        for &(mode, w, _) in &self.poses {
            match mode {
                MODE_AND_ARMS => {
                    controls.extend([(arms_joint, pitch(w * arms)), (neck_joint, pitch(w * neck)), (head_joint, pitch(w * head))]);
                }
                MODE_CONSTRAINED => controls.extend([
                    (arms_joint, pitch(w * arms)),
                    (neck_joint, pitch(w * neck)),
                    (head_joint, yaw(w * self.yaw_offset)),
                    (head_joint, pitch(w * head)),
                ]),
                MODE_CONSTRAINED_WITH_ARMS => controls.extend([
                    (arms_joint, pitch(w * arms)),
                    (arms_joint, yaw(w * self.yaw_offset)),
                    (neck_joint, pitch(w * neck)),
                    (head_joint, pitch(w * head)),
                ]),
                MODE_CATFALL => controls.extend([
                    (neck_joint, pitch(w * (neck + arms))),
                    (head_joint, yaw(w * self.yaw_offset)),
                    (head_joint, pitch(w * head)),
                ]),
                // ANIMATED: camera_jnt as animated (arms only).
                _ => {}
            }
        }
        controls
    }

    /// FUN_1407f7500: sprinting (or jumping/falling started from a sprint)
    /// widens the FOV by `additional/80` of the base.
    pub fn update_fov(&mut self, dt: f32, base_fov: f32, speed_state: bool) {
        // 80.0 (DAT_141ac8ff4).
        let target = if speed_state { base_fov + base_fov * self.decl.speed_fov_additional_value / 80.0 } else { base_fov };
        if target > self.fov {
            self.fov += (target - self.fov).min(self.decl.speed_fov_max_increase_speed * dt);
        } else if target < self.fov {
            self.fov -= (self.fov - target).min(self.decl.speed_fov_max_decrease_speed * dt);
        }
    }

    /// FUN_1407f6a80: camera_jnt in world space, kept inside the capsule.
    pub fn view(&mut self, pose: &BodyPose, physics: &PlayerPhysics, allow_out_of_shape: bool, body_yaw: f32) -> View {
        let controls = self.controls(pose, body_yaw);
        let (local, local_rotation) = pose.joint_with_controls(pose.camera, &controls);
        let axis = physics.axis();
        let position = physics.position();
        let base = position + physics.lean_offset;
        let mut origin = position + axis * local;
        let mut delta = origin - base;
        // 0.05 (DAT_141ab3814).
        let radius = physics.active_shape().radius - 0.05;
        let height = physics.ceil_distance - 0.05;
        let mut out = false;
        let horizontal = delta.truncate().length();
        if horizontal > radius {
            let scale = radius / horizontal;
            delta.x *= scale;
            delta.y *= scale;
            out = true;
        }
        if delta.z > height {
            delta.z = height;
            out = true;
        }
        let mut from_joint = true;
        if !allow_out_of_shape && !self.out_of_shape && out {
            origin = base + delta;
            out = false;
            from_joint = false;
        }
        self.out_of_shape = out;
        // FUN_1407f4b30: from the head joint along the entity's forward
        // pitched about its left and turned by the yaw offset; the clamped
        // view itself otherwise.
        self.eye = if from_joint {
            let (head, _) = pose.joint_with_controls(pose.controls[0], &controls);
            let head = position + axis * head;
            let forward = Mat3::from_rotation_z(self.yaw_offset) * (Quat::from_axis_angle(axis.y_axis, self.pitch) * axis.x_axis);
            head + forward * origin.distance(head)

        } else {
            origin
        };
        View { origin, axis: axis * local_rotation }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose() -> BodyPose {
        // spine03 → neck → head → camera, straight up the body.
        BodyPose {
            positions: vec![Vec3::new(0.0, 0.0, 1.3), Vec3::new(0.0, 0.0, 1.5), Vec3::new(0.0, 0.0, 1.6), Vec3::new(0.05, 0.0, 1.7)],
            rotations: vec![Mat3::IDENTITY; 4],
            parents: vec![None, Some(0), Some(1), Some(2)],
            camera: 3,
            controls: [2, 1, 0],
        }
    }

    #[test]
    fn the_pitch_split_turns_camera_jnt_by_the_whole_pitch() {
        let decl = CameraDecl::default();
        let (position, rotation) = pose().camera_with_pitch(0.5, decl.forward_ratios, decl.backward_ratios);
        let forward = rotation * Vec3::X;
        assert!((forward.z + 0.5f32.sin()).abs() < 1e-5, "{forward:?}");
        assert!(position.z < 1.7 && position.x > 0.05, "{position:?}");
        let (level, _) = pose().camera_with_pitch(0.0, decl.forward_ratios, decl.backward_ratios);
        assert!(level.distance(Vec3::new(0.05, 0.0, 1.7)) < 1e-6);
    }

    #[test]
    fn speed_fov_moves_at_the_decl_rates() {
        let decl = CameraDecl { speed_fov_max_increase_speed: 22.0, speed_fov_max_decrease_speed: 45.0, ..Default::default() };
        let mut camera = PlayerCamera::new(decl, 80.0);
        camera.update_fov(0.1, 80.0, true);
        assert!((camera.fov - 82.2).abs() < 1e-4);
        for _ in 0..20 {
            camera.update_fov(0.1, 80.0, true);
        }
        assert!((camera.fov - 90.0).abs() < 1e-4);
        camera.update_fov(0.1, 80.0, false);
        assert!((camera.fov - 85.5).abs() < 1e-4);
    }
}
