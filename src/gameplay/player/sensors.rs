//! Player sensors (arkCpntPlayerSensor, ids `arkPlayerSensor_t`).
//!
//! The AutoCrouch sensor (arkCpntPlayerSensorAutoCrouch, Start FUN_1403a2430,
//! submit FUN_1403a2f60, results FUN_1403a2730) finds an overhang the player
//! can only pass crouched. Its queries run synchronously on the previous
//! frame's movement request, as the async submit/result pair reads it. The
//! WallCrouch sensor's decl starts disabled and no enabling caller was found,
//! so it is not ported (WALLCROUCH stays unreachable, as in the game).
use super::character_proxy::{PlayerWorld, WorldHit};
use super::physics::PlayerPhysics;
use super::shape::{POSTURE_AUTOCROUCHING, POSTURE_SNEAKING};
use bevy_math::{Mat3, Vec3};

/// arkDeclCpntPlayerSensorAutoCrouch (ctor FUN_1403a2120).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoCrouchDecl {
    pub min_detection_distance: f32,
    pub shape_radius: f32,
    pub min_hole_depth: f32,
    pub push_duration: f32,
    pub uncrouch_if_not_pushing: bool,
}

impl Default for AutoCrouchDecl {
    fn default() -> Self {
        Self { min_detection_distance: 1.0, shape_radius: 0.3, min_hole_depth: 0.3, push_duration: 0.0, uncrouch_if_not_pushing: true }
    }
}

fn box_points(min: Vec3, max: Vec3) -> Vec<Vec3> {
    let mut points = Vec::with_capacity(8);
    for x in [min.x, max.x] {
        for y in [min.y, max.y] {
            for z in [min.z, max.z] {
                points.push(Vec3::new(x, y, z));
            }
        }
    }
    points
}

/// FUN_140112000: a frame whose +X is `direction` and +Z up.
fn frame(direction: Vec3) -> Mat3 {
    let left = Vec3::Z.cross(direction).normalize_or_zero();
    Mat3::from_cols(direction, left, Vec3::Z)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutoCrouchSensor {
    pub decl: AutoCrouchDecl,
    pub enabled: bool,
    sneak_left: Vec<Vec3>,
    sneak_right: Vec<Vec3>,
    crouch: Vec<Vec3>,
    player_radius: f32,
    keep_distance: f32,
    step_height: f32,
    pub detection_distance: f32,
    step_offset: f32,
    direction: Vec3,
    speed: f32,
    pub sneak_distance: f32,
    sneak_distance_no_step: f32,
    pub hole_depth: f32,
    push_timer: f32,
    pub fires: bool,
}

impl AutoCrouchSensor {
    pub fn new(decl: AutoCrouchDecl, physics: &PlayerPhysics) -> Self {
        let r = decl.shape_radius;
        let sneak = physics.posture_settings(POSTURE_SNEAKING);
        let crouch = physics.posture_settings(POSTURE_AUTOCROUCHING);
        let sneak_top = sneak.height + 0.05;
        let mut wedge = box_points(Vec3::new(-r, -r, crouch.step_height), Vec3::new(0.0, r, crouch.height));
        wedge.push(Vec3::new(-r, 0.0, 0.0));
        Self {
            decl,
            enabled: true,
            sneak_left: box_points(Vec3::new(-r, -r, crouch.height), Vec3::new(0.0, 0.0, sneak_top)),
            sneak_right: box_points(Vec3::new(-r, 0.0, crouch.height), Vec3::new(0.0, r, sneak_top)),
            crouch: wedge,
            player_radius: sneak.radius.max(crouch.radius),
            keep_distance: physics.keep_distance(),
            step_height: crouch.step_height,
            detection_distance: decl.min_detection_distance,
            step_offset: 0.0,
            direction: physics.axis().x_axis,
            speed: 0.0,
            sneak_distance: 0.0,
            sneak_distance_no_step: 0.0,
            hole_depth: 0.0,
            push_timer: 0.0,
            fires: false,
        }
    }

    /// FUN_1403a2710.
    pub fn set_detection_distance(&mut self, distance: f32) {
        self.detection_distance = distance.max(self.decl.min_detection_distance);
    }

    fn hit_distance(world: &dyn PlayerWorld, shape: &[Vec3], rotation: Mat3, from: Vec3, direction: Vec3, length: f32) -> (f32, Option<WorldHit>) {
        let mut hits = Vec::new();
        world.cast_shape(shape, rotation, from, direction * length, 0.0, &mut hits, None);
        match hits.into_iter().min_by(|a, b| a.value.total_cmp(&b.value)) {
            Some(hit) => (hit.value * length, Some(hit)),
            None => (length, None),
        }
    }

    /// Submit and results for one frame. `request` is the first priority-0
    /// linear velocity request.
    pub fn update(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics, request: Option<Vec3>, dt: f32, sliding: bool) {
        if !self.enabled {
            return;
        }
        let origin = physics.position();
        self.speed = 0.0;
        if let Some(v) = request {
            let d = Vec3::new(v.x, v.y, 0.0);
            self.speed = d.length();
            if self.speed <= 0.01 {
                self.speed = 0.0;
            } else {
                self.direction = d / self.speed;
            }
        }
        let length = self.detection_distance + self.player_radius + self.decl.min_hole_depth + 0.05;
        let rotation = frame(self.direction);
        let start = origin + Vec3::Z * (self.step_offset + self.keep_distance);
        let (left, _) = Self::hit_distance(world, &self.sneak_left, rotation, start, self.direction, length);
        let (right, _) = Self::hit_distance(world, &self.sneak_right, rotation, start, self.direction, length);
        let (crouch, _) = Self::hit_distance(world, &self.crouch, rotation, start, self.direction, length);
        self.sneak_distance = left.max(right);
        if self.step_offset == 0.0 {
            self.sneak_distance_no_step = left.min(right);
        }
        self.step_offset = 0.0;
        let step_start = origin + Vec3::Z * self.keep_distance;
        if Self::hit_distance(world, &self.crouch, rotation, step_start, self.direction, length).1.is_some() {
            for _ in 0..2 {
                let from = origin + Vec3::Z * self.step_offset;
                let Some(hit) = Self::hit_distance(world, &self.crouch, rotation, from, self.direction, length).1 else { break };
                let h = hit.position.z - origin.z;
                if h <= self.step_offset || h >= self.step_height {
                    break;
                }
                self.step_offset = h;
            }
        }
        self.hole_depth = (crouch - self.sneak_distance).max(0.0);
        let mut deep = self.hole_depth >= self.decl.min_hole_depth;
        let fire = if physics.posture == POSTURE_AUTOCROUCHING {
            if self.decl.uncrouch_if_not_pushing {
                deep = deep && self.speed > 0.0;
            }
            deep || self.sneak_distance < self.player_radius
        } else {
            let m = left.min(right).max(self.sneak_distance_no_step);
            deep && m > 0.0 && self.speed > 0.0
        };
        self.fires = if fire {
            self.push_timer += dt;
            self.push_timer >= self.decl.push_duration || sliding
        } else {
            self.push_timer = 0.0;
            false
        };
    }
}

/// arkDeclCpntPlayerSensorLean (ctor FUN_1403b91c0; the installed decl is
/// `{}`). `m_collisionLayers` 0x802b is not distinguished: the world is the
/// level's static collision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeanDecl {
    pub head_shape_size: Vec3,
    pub lean_amplitude: f32,
    pub lean_up_amplitude: f32,
}

impl Default for LeanDecl {
    fn default() -> Self {
        Self { head_shape_size: Vec3::new(0.05, 0.2, 0.1), lean_amplitude: 0.8, lean_up_amplitude: 0.35 }
    }
}

/// arkCpntPlayerSensorLean (PLAYER_SENSOR 8): two head-box casts along the
/// body's left and right that limit the lateral lean. Submit FUN_1403b9650
/// and results FUN_1403b9460 run synchronously, as for AutoCrouch.
#[derive(Clone, Debug, PartialEq)]
pub struct LeanSensor {
    pub decl: LeanDecl,
    pub enabled: bool,
    head: Vec<Vec3>,
    /// +0x80 / +0x84: fractions of the amplitude.
    pub left_limit: f32,
    pub right_limit: f32,
}

impl LeanSensor {
    pub fn new(decl: LeanDecl) -> Self {
        let size = decl.head_shape_size;
        Self { decl, enabled: false, head: box_points(-size, size), left_limit: 1.0, right_limit: 1.0 }
    }

    /// FUN_1403b93f0 (vtbl+0xd8).
    pub fn enable(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.left_limit = 1.0;
            self.right_limit = 1.0;
        }
    }

    /// `eye` is the camera's gameplay eye of the previous PRE_RENDER;
    /// `axis` columns are the entity's forward, left and up.
    pub fn update(&mut self, world: &dyn PlayerWorld, eye: Vec3, origin: Vec3, axis: Mat3) {
        if !self.enabled {
            return;
        }
        let left = axis.y_axis;
        let mut from = eye - left * (eye.dot(left) - origin.dot(left));
        // 0.05 (0x141c71ff4) and 0.01 (0x141ab3810).
        from.z -= (self.decl.head_shape_size.z - 0.05) + 0.01;
        let amplitude = self.decl.lean_amplitude;
        let limit = |direction: Vec3| {
            let (distance, hit) = AutoCrouchSensor::hit_distance(world, &self.head, axis, from, direction, amplitude);
            if hit.is_some() { distance / amplitude } else { 1.0 }
        };
        self.left_limit = limit(left);
        self.right_limit = limit(-left);
    }

    /// FUN_1403b9220: clamp a lean (negative is left) into the free range;
    /// false when it reached a limit (reaching it exactly counts).
    pub fn clamp(&self, lean: &mut f32) -> bool {
        if *lean <= -self.left_limit {
            *lean = -self.left_limit;
            false
        } else if self.right_limit <= *lean {
            *lean = self.right_limit;
            false
        } else {
            true
        }
    }
}
