//! arkCpntPlayerCamera in MODE_CAMERA_AND_ARMS: pitch integration
//! FUN_1407f1400, the speed FOV FUN_1407f7500 and the view placement
//! FUN_1407f6a80 (camera_jnt in world space, clamped into the capsule).
//!
//! The view origin is the body's `camera_jnt`. The spine/neck/head pitch
//! split (FUN_1407f4cd0) rotates that joint; the body animation pose is the
//! skeleton supplied by the caller (`BodyPose`).
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
        let mut position = self.positions[self.camera];
        let mut rotation = self.rotations[self.camera];
        // Arms (spine03) first, then neck, then head, each in model space.
        for (control, ratio) in [(self.controls[2], ratios[2]), (self.controls[1], ratios[1]), (self.controls[0], ratios[0])] {
            if !self.descends_from(self.camera, control) {
                continue;
            }
            let pivot = self.positions[control];
            let turn = Mat3::from_quat(Quat::from_axis_angle(Vec3::Y, pitch * ratio));
            position = pivot + turn * (position - pivot);
            rotation = turn * rotation;
        }
        (position, rotation)
    }
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
        }
    }

    /// FUN_1407f1400 (the free camera of modes 0/1/2/7/11/12/15).
    pub fn update_pitch(&mut self, dt: f32, input_pitch: f32, look_is_analog: bool, physics: &PlayerPhysics) {
        let mut rate = if look_is_analog { self.turn.turn_speed_joystick } else { self.turn.turn_speed_mouse } * input_pitch;
        // FUN_1401b3a00 with the physics angular limits.
        let limit = physics.angular_limit(rate, self.pitch_rate);
        if (rate - self.pitch_rate).abs() > dt * limit {
            rate = self.pitch_rate + (rate - self.pitch_rate).signum() * dt * limit;
        }
        let previous = self.pitch;
        let in_range = self.min_pitch <= previous && previous <= self.max_pitch;
        self.pitch = (previous + dt * rate).clamp(self.min_pitch, self.max_pitch);
        self.pitch_rate = if in_range && dt > 0.0 { (self.pitch - previous) / dt } else { 0.0 };
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
    pub fn view(&mut self, pose: &BodyPose, physics: &PlayerPhysics, allow_out_of_shape: bool) -> View {
        let (local, local_rotation) = pose.camera_with_pitch(self.pitch, self.decl.forward_ratios, self.decl.backward_ratios);
        let axis = physics.axis();
        let base = physics.position();
        let mut origin = base + axis * local;
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
        if !allow_out_of_shape && !self.out_of_shape && out {
            origin = base + delta;
            out = false;
        }
        self.out_of_shape = out;
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
