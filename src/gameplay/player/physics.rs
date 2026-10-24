//! Player physics: arkCpntPlayerPhysics (vtable 0x141c73268) with its
//! arkPhysicsPlayer (gamelib\havok\arkPhysicsPlayer.cpp, ctor FUN_1401b2290).
//!
//! The per-frame step FUN_1401b4760 runs in PRE_PHYSICS; the ground/ceiling
//! queries FUN_1408002e0 are issued in QUERY_PHYSICS and read by
//! FUN_1407ff230 in DEFAULT. Impulses (+0x220), the external/one-frame
//! velocities (+0x54/+0x64, no writer found), the position offset (+0x110),
//! moving-ground velocity, safe restore, the AI obstacle and the hitbox body
//! are not ported: the rewrite's collision world is static.
use super::character_proxy::{CharacterProxy, PlayerWorld, ProxySettings, SupportState, WorldHit};
use super::shape::{PostureShape, ShapeSettings, POSTURE_AUTOCROUCHING, POSTURE_COUNT, POSTURE_KEEP_PREVIOUS, POSTURE_SWIMMING};
use bevy_math::{Mat3, Vec3};

/// "Unset" sentinel (DAT_141ae8a70).
pub const UNSET: f32 = 1e30;
/// World gravity (DAT_141ad57d0 → hknpWorld+0x970); constant for the game.
pub const GRAVITY: Vec3 = Vec3::new(0.0, 0.0, -9.81);
const EPSILON: f32 = f32::EPSILON;

/// arkPhysicsPlayerSettings (0x38), defaults FUN_1401b2b30.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsSettings {
    pub dynamic_friction: f32,
    pub static_friction: f32,
    pub contact_angle_sensitivity: f32,
    pub user_planes: i32,
    pub max_character_speed_for_solver: f32,
    pub character_strength: f32,
    pub character_mass: f32,
    /// Degrees.
    pub max_slope: f32,
    pub penetration_recovery_speed: f32,
    pub max_cast_iterations: i32,
    pub refresh_manifold_in_check_support: bool,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self {
            dynamic_friction: 1.0,
            static_friction: 0.0,
            contact_angle_sensitivity: 10.0,
            user_planes: 4,
            max_character_speed_for_solver: 10.0,
            character_strength: 10000.0,
            character_mass: 55.0,
            max_slope: 50.0,
            penetration_recovery_speed: 10.0,
            max_cast_iterations: 5,
            refresh_manifold_in_check_support: false,
        }
    }
}

/// arkDeclCpntPlayerPhysics (ctor FUN_1407fc960) fields the port uses.
#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsDecl {
    pub ground_query_length: f32,
    pub ground_ray_query_length: f32,
    pub ceil_query_length: f32,
    pub physics: PhysicsSettings,
    /// `m_shapes`: (posture, settings) in item order.
    pub shapes: Vec<(usize, ShapeSettings)>,
}

impl Default for PhysicsDecl {
    fn default() -> Self {
        Self {
            ground_query_length: 0.5,
            ground_ray_query_length: 20.0,
            ceil_query_length: 2.0,
            physics: PhysicsSettings::default(),
            shapes: Vec::new(),
        }
    }
}

/// arkPhysicsGround (0x24) from the support test (FUN_1401b20e0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub state: SupportState,
    pub normal: Vec3,
    pub velocity: Vec3,
    pub angular: f32,
}

impl Default for Ground {
    fn default() -> Self {
        Self { state: SupportState::Unsupported, normal: Vec3::Z, velocity: Vec3::ZERO, angular: 0.0 }
    }
}

/// The movement object's resolved requests (`move+0x88` linear, `+0x94`
/// angular, valid flags `+0xa0/+0xa1`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionInput {
    pub linear: Option<Vec3>,
    pub angular: Option<Vec3>,
}

/// Vertical move (`+0x80..+0x94`), started by FUN_1407fdca0.
#[derive(Clone, Copy, Debug, PartialEq)]
struct VerticalMove {
    target_z: f32,
    last_speed: f32,
    max_speed: f32,
    gain: f32,
    ground_vertical_speed: f32,
}

pub const RAY_CENTER: usize = 0;

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerPhysics {
    shapes: Vec<PostureShape>,
    current: usize,
    target: usize,
    timer: f32,
    duration: f32,
    /// m_shapes / m_settings (cpnt+0x58 / +0xa8), by posture.
    posture_shape: [usize; POSTURE_COUNT],
    posture_settings: [ShapeSettings; POSTURE_COUNT],
    pub posture: usize,
    pub ground: Ground,
    /// +0x48, gravity / vertical velocity accumulator.
    pub gravity_velocity: Vec3,
    /// +0x60.
    max_z: f32,
    /// +0x7c.
    gravity_override: f32,
    vertical: Option<VerticalMove>,
    /// +0x98.
    slip: bool,
    linear_acceleration: f32,
    linear_deceleration: f32,
    angular_acceleration: f32,
    angular_deceleration: f32,
    no_smoothing: bool,
    smoothed_velocity: Vec3,
    yaw_rate: f32,
    speed_scale: f32,
    speed_scale_target: f32,
    speed_scale_rate: f32,
    /// +0xd0, radians in (-π, π].
    pub yaw: f32,
    /// Output velocity of the last step (entity +0x33c).
    pub velocity: Vec3,
    pub proxy: CharacterProxy,
    disable_gravity_count: i32,
    // arkCpntPlayerPhysics query state.
    pub ground_query_length: f32,
    ground_ray_query_length: f32,
    ceil_query_length: f32,
    pub ground_distance: f32,
    pub ceil_distance: f32,
    pub penetrating: bool,
    pub ground_ray_distance: [f32; 5],
    pub ground_ray_normal: [Vec3; 5],
    pub ground_ray_distance_min: f32,
    pub ground_ray_distance_max: f32,
}

/// FUN_1401b3060 angle wrap into (-π, π].
pub fn wrap_angle(mut angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if angle >= TAU || angle < 0.0 {
        angle -= (angle * (1.0 / TAU)).floor() * TAU;
    }
    if angle > PI {
        angle -= TAU;
    }
    angle
}

/// FUN_14010e6a0(axis 2): rows forward (c, s, 0), left (-s, c, 0), up.
pub fn yaw_axis(yaw: f32) -> Mat3 {
    let (s, c) = yaw.sin_cos();
    Mat3::from_cols(Vec3::new(c, s, 0.0), Vec3::new(-s, c, 0.0), Vec3::Z)
}

impl PlayerPhysics {
    /// arkCpntPlayerPhysics start FUN_1407fe310 and the arkPhysicsPlayer ctor.
    pub fn new(decl: &PhysicsDecl, position: Vec3, yaw: f32) -> Self {
        let (first_posture, first) = decl.shapes.first().copied().unwrap_or((0, ShapeSettings::default()));
        let mut shapes = vec![PostureShape::new(first)];
        let mut posture_shape = [usize::MAX; POSTURE_COUNT];
        let mut posture_settings = [first; POSTURE_COUNT];
        posture_shape[first_posture] = 0;
        for &(posture, settings) in decl.shapes.iter().skip(1) {
            if posture < POSTURE_COUNT && posture_shape[posture] == usize::MAX {
                posture_shape[posture] = shapes.len();
                posture_settings[posture] = settings;
                shapes.push(PostureShape::new(settings));
            }
        }
        for index in &mut posture_shape {
            if *index == usize::MAX {
                *index = 0;
            }
        }
        let settings = decl.physics;
        let proxy = CharacterProxy::new(
            position,
            yaw_axis(yaw),
            shapes[0].body.clone(),
            ProxySettings {
                dynamic_friction: settings.dynamic_friction,
                static_friction: settings.static_friction,
                keep_distance: 0.01,
                keep_contact_tolerance: 0.1,
                contact_angle_sensitivity: settings.contact_angle_sensitivity,
                user_planes: settings.user_planes,
                max_character_speed_for_solver: settings.max_character_speed_for_solver,
                character_strength: settings.character_strength,
                character_mass: settings.character_mass,
                max_slope_cosine: (settings.max_slope * 0.017453292).cos(),
                penetration_recovery_speed: settings.penetration_recovery_speed,
                max_cast_iterations: settings.max_cast_iterations,
                refresh_manifold_in_check_support: settings.refresh_manifold_in_check_support,
            },
        );
        Self {
            shapes,
            current: 0,
            target: 0,
            timer: 0.0,
            duration: 0.0,
            posture_shape,
            posture_settings,
            posture: first_posture,
            ground: Ground::default(),
            gravity_velocity: Vec3::ZERO,
            max_z: UNSET,
            gravity_override: UNSET,
            vertical: None,
            slip: false,
            linear_acceleration: UNSET,
            linear_deceleration: UNSET,
            angular_acceleration: UNSET,
            angular_deceleration: UNSET,
            no_smoothing: false,
            smoothed_velocity: Vec3::ZERO,
            yaw_rate: 0.0,
            speed_scale: 1.0,
            speed_scale_target: 1.0,
            speed_scale_rate: 0.0,
            yaw: wrap_angle(yaw),
            velocity: Vec3::ZERO,
            proxy,
            disable_gravity_count: 0,
            ground_query_length: decl.ground_query_length,
            ground_ray_query_length: decl.ground_ray_query_length,
            ceil_query_length: decl.ceil_query_length,
            ground_distance: f32::MAX,
            ceil_distance: f32::MAX,
            penetrating: false,
            ground_ray_distance: [decl.ground_ray_query_length; 5],
            ground_ray_normal: [Vec3::Z; 5],
            ground_ray_distance_min: decl.ground_ray_query_length,
            ground_ray_distance_max: -decl.ground_ray_query_length,
        }
    }

    pub fn position(&self) -> Vec3 {
        self.proxy.position
    }

    pub fn axis(&self) -> Mat3 {
        yaw_axis(self.yaw)
    }

    /// FUN_1407fda30: settings of the current posture.
    pub fn posture_settings(&self, posture: usize) -> &ShapeSettings {
        &self.posture_settings[posture.min(POSTURE_COUNT - 1)]
    }

    pub fn current_settings(&self) -> &ShapeSettings {
        &self.shapes[self.current].settings
    }

    /// Settings of the posture shape the collision currently uses.
    pub fn active_shape(&self) -> &ShapeSettings {
        self.current_settings()
    }

    /// FUN_1407ff0a0 → FUN_1401b6ae0 (angular values in degrees).
    pub fn set_motion_limits(&mut self, linear_acceleration: f32, linear_deceleration: f32, angular_acceleration: f32, angular_deceleration: f32) {
        self.linear_acceleration = linear_acceleration;
        self.linear_deceleration = linear_deceleration;
        self.angular_acceleration = angular_acceleration * 0.017453292;
        self.angular_deceleration = angular_deceleration * 0.017453292;
    }

    /// Teleport (FUN_1407fe920 → FUN_1401b6b10) with the optional gravity and
    /// smoothing reset.
    pub fn teleport(&mut self, position: Vec3, yaw: f32) {
        self.proxy.position = position;
        self.proxy.velocity = Vec3::ZERO;
        self.max_z = UNSET;
        self.yaw = wrap_angle(yaw);
        self.vertical = None;
        self.gravity_velocity = Vec3::ZERO;
        self.smoothed_velocity = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
    }

    /// FUN_1401b3a00's limit for moving a rate from `previous` to `target`.
    pub fn angular_limit(&self, target: f32, previous: f32) -> f32 {
        if self.no_smoothing {
            f32::INFINITY
        } else if target * previous < 0.0 || target.abs() < previous.abs() {
            self.angular_deceleration
        } else {
            self.angular_acceleration
        }
    }

    /// FUN_1407fdca0.
    pub fn start_vertical(&mut self, target_z: f32, max_speed: f32, gain: f32) {
        self.vertical = Some(VerticalMove {
            target_z,
            last_speed: 0.0,
            max_speed,
            gain,
            ground_vertical_speed: self.ground.velocity.z,
        });
    }

    /// FUN_1407ff1c0.
    pub fn stop_vertical(&mut self) {
        self.vertical = None;
        self.gravity_velocity.z = 0.0;
    }

    /// FUN_1407fdc10.
    pub fn vertical_active(&self) -> bool {
        self.vertical.is_some()
    }

    /// FUN_1407ff080.
    pub fn set_gravity_velocity(&mut self, velocity: Vec3) {
        self.gravity_velocity = velocity;
    }

    /// FUN_1407fd830 / FUN_1407fd970: counted gravity-strength disable.
    pub fn disable_gravity(&mut self, disable: bool) {
        if disable {
            self.disable_gravity_count += 1;
            if self.disable_gravity_count == 1 {
                self.gravity_override = 0.0;
                self.gravity_velocity = Vec3::ZERO;
            }
        } else {
            self.disable_gravity_count -= 1;
            if self.disable_gravity_count == 0 {
                self.gravity_override = UNSET;
                self.gravity_velocity = Vec3::ZERO;
            }
        }
    }

    fn shape_height(&self, index: usize) -> f32 {
        self.shapes[index].settings.height
    }

    /// FUN_1407ff0e0 → FUN_1401b6b70 (immediate commands).
    pub fn set_posture(&mut self, posture: usize, duration: f32) {
        if posture >= POSTURE_COUNT {
            return;
        }
        self.posture = posture;
        let index = self.posture_shape[posture];
        let mut grew = false;
        if self.target != self.current && self.timer > 0.0 {
            grew = self.shape_height(self.target) - self.shape_height(self.current) > EPSILON;
            self.current = self.target;
        }
        self.target = index;
        self.duration = if self.timer <= 0.0 { duration } else { self.duration + duration };
        let growing = self.shape_height(self.target) - self.shape_height(self.current) > EPSILON;
        if growing {
            grew = false;
        }
        if duration <= EPSILON {
            self.current = self.target;
            grew = true;
            self.timer = 0.0;
            self.duration = 0.0;
        }
        if (duration > EPSILON && growing) || grew {
            let shape = if grew { self.current } else { self.target };
            self.proxy.set_shape(self.shapes[shape].body.clone());
        }
    }

    /// FUN_1407ff1d0.
    pub fn can_use_posture(&self, posture: usize) -> bool {
        if posture == POSTURE_AUTOCROUCHING
            || posture == POSTURE_SWIMMING
            || (posture == POSTURE_KEEP_PREVIOUS && self.posture == POSTURE_AUTOCROUCHING)
        {
            return true;
        }
        let posture = if posture == POSTURE_KEEP_PREVIOUS { self.posture } else { posture };
        self.posture_settings(posture).height <= self.ceil_distance
    }

    /// FUN_1407fdc30.
    pub fn ceiling_blocks(&self, extra: f32) -> bool {
        self.ceil_distance <= self.proxy.settings.keep_distance + self.posture_settings(self.posture).height + 1e-4 + extra
    }

    /// `groundDistance < FLT_MAX || support != UNSUPPORTED`.
    pub fn near_ground(&self) -> bool {
        self.ground_distance < f32::MAX || self.ground.state != SupportState::Unsupported
    }

    /// FUN_1407fcb90 (crush tests are not ported: static world).
    pub fn free_to_stand(&self, extra: f32) -> bool {
        !self.penetrating && self.near_ground() && !self.ceiling_blocks(extra + 0.1)
    }

    /// FUN_1407fdb50, x/y: owner velocity minus gravity (moving ground,
    /// impulses and external velocities are zero here).
    pub fn input_velocity(&self) -> Vec3 {
        self.velocity - self.gravity_velocity
    }

    /// QUERY_PHYSICS FUN_1408002e0 and its results FUN_1407ff230.
    pub fn update_queries(&mut self, world: &dyn PlayerWorld) {
        let query = if self.shape_height(self.target) - self.shape_height(self.current) > EPSILON { self.target } else { self.current };
        let settings = self.shapes[query].settings;
        let cast_offset = settings.z_offset + (settings.height - (settings.z_offset + settings.step_height)) * 0.5;
        let origin = self.proxy.position;
        let axis = self.axis();
        let start = origin + Vec3::Z * cast_offset;
        let mut hits: Vec<WorldHit> = Vec::new();
        // Touch: the posture's body shape at the origin.
        world.cast_shape(&self.shapes[self.posture_shape[self.posture]].body, axis, origin, Vec3::ZERO, 0.0, &mut hits, None);
        self.penetrating = !hits.is_empty();
        let ceil_floor = &self.shapes[query].ceil_floor;
        let ground_length = self.ground_query_length + cast_offset;
        self.ground_distance = f32::MAX;
        if self.penetrating {
            if let Some(hit) = world.cast_ray(start, -Vec3::Z, ground_length) {
                self.ground_distance = hit.distance - cast_offset;
            }
        } else {
            hits.clear();
            world.cast_shape(ceil_floor, axis, start, Vec3::new(0.0, 0.0, -ground_length), 0.0, &mut hits, None);
            if let Some(fraction) = hits.iter().map(|h| h.value).min_by(f32::total_cmp) {
                self.ground_distance = ground_length * fraction - cast_offset;
            }
        }
        self.ceil_distance = f32::MAX;
        if self.penetrating {
            if let Some(hit) = world.cast_ray(start, Vec3::Z, self.ceil_query_length) {
                self.ceil_distance = hit.distance + cast_offset;
            }
        } else {
            hits.clear();
            world.cast_shape(ceil_floor, axis, start, Vec3::new(0.0, 0.0, self.ceil_query_length), 0.0, &mut hits, None);
            if let Some(fraction) = hits.iter().map(|h| h.value).min_by(f32::total_cmp) {
                self.ceil_distance = fraction * self.ceil_query_length + cast_offset + settings.step_height;
            }
        }
        let reach = settings.radius - 0.01;
        let offsets = [Vec3::ZERO, axis.x_axis * reach, -axis.x_axis * reach, axis.y_axis * reach, -axis.y_axis * reach];
        let length = self.ground_ray_query_length + cast_offset;
        self.ground_ray_distance_min = self.ground_ray_query_length;
        self.ground_ray_distance_max = -self.ground_ray_query_length;
        let mut sum = Vec3::ZERO;
        for (index, offset) in offsets.into_iter().enumerate() {
            let (distance, normal) = match world.cast_ray(start + offset, -Vec3::Z, length) {
                Some(hit) => (hit.distance - cast_offset, hit.normal),
                None => (self.ground_ray_query_length, Vec3::Z),
            };
            self.ground_ray_distance[index] = distance;
            self.ground_ray_normal[index] = normal;
            self.ground_ray_distance_min = self.ground_ray_distance_min.min(distance);
            self.ground_ray_distance_max = self.ground_ray_distance_max.max(distance);
            sum += normal;
        }
        let _average_normal = sum.normalize_or_zero();
    }

    /// FUN_1401b3ac0: linear acceleration limiter.
    fn smooth_input(&mut self, input: Vec3, dt: f32) -> Vec3 {
        let mut input = input;
        if !self.no_smoothing && input != self.smoothed_velocity {
            let delta = input - self.smoothed_velocity;
            let limit = if self.smoothed_velocity.dot(delta) < 0.0 { self.linear_deceleration } else { self.linear_acceleration } * dt;
            let length2 = delta.length_squared();
            if length2 > limit * limit {
                input = self.smoothed_velocity + delta * (limit / length2.max(f32::MIN_POSITIVE).sqrt());
            }
        }
        self.smoothed_velocity = input;
        input
    }

    /// FUN_1401b4760: one PRE_PHYSICS step.
    pub fn step(&mut self, world: &dyn PlayerWorld, dt: f32, input: MotionInput) {
        // 1. Posture transition.
        if self.duration > 0.0 {
            if self.duration <= self.timer {
                self.timer = 0.0;
                self.duration = 0.0;
                self.current = self.target;
                self.proxy.set_shape(self.shapes[self.target].body.clone());
            } else {
                self.timer += dt;
            }
        }
        let shape = self.shapes[self.current].settings;
        // 2. Input and linear smoothing.
        let mut velocity = self.smooth_input(input.linear.unwrap_or(Vec3::ZERO), dt);
        // 3. Gravity strength.
        let gravity_strength = if self.gravity_override == UNSET { shape.gravity_strength } else { self.gravity_override };
        let mut gravity = self.gravity_velocity;
        // 4. Vertical move.
        if let Some(mut vertical) = self.vertical {
            vertical.target_z += vertical.ground_vertical_speed * dt;
            let blocked = vertical.last_speed != 0.0
                && (vertical.last_speed - (self.velocity.z - vertical.ground_vertical_speed)).abs() > 0.01;
            let dz = vertical.target_z - self.proxy.position.z;
            let speed = if blocked || dz < 0.01 / self.speed_scale {
                self.vertical = None;
                0.0
            } else {
                let rate = dz / dt;
                if rate >= 1.0 / self.speed_scale { (rate * vertical.gain).min(vertical.max_speed) } else { rate }
            };
            vertical.last_speed = speed;
            gravity.z = speed;
            if self.vertical.is_some() {
                self.vertical = Some(vertical);
            }
        }
        // 6. Ground support.
        let surface = self.proxy.check_support(-Vec3::Z);
        self.ground = Ground {
            state: surface.state,
            normal: surface.normal,
            velocity: Vec3::ZERO,
            angular: 0.0,
        };
        // 7. Downhill ground-plane projection, keeping horizontal speed.
        let vertical_mode = self.vertical.is_some();
        let normal = self.ground.normal;
        if !vertical_mode && gravity_strength > 0.0 && self.gravity_override == UNSET && normal.dot(velocity) > 0.0 {
            let projected = velocity - normal * velocity.dot(normal);
            let length = projected.truncate().length();
            if length > 0.01 {
                velocity = projected * (velocity.truncate().length() / length);
            }
        }
        // 8. Gravity.
        if !vertical_mode {
            if !self.slip {
                match self.ground.state {
                    SupportState::Unsupported => gravity += GRAVITY * (dt * gravity_strength),
                    SupportState::Sliding | SupportState::Supported => gravity = Vec3::ZERO,
                }
            } else {
                let along = GRAVITY - normal * GRAVITY.dot(normal);
                gravity += along * (dt * gravity_strength);
            }
        }
        self.gravity_velocity = gravity;
        // 9. Compose.
        let composed = if vertical_mode {
            Vec3::new(velocity.x + gravity.x, velocity.y + gravity.y, gravity.z)
        } else {
            velocity + gravity
        };
        // 10. Speed scale and integrate.
        if self.speed_scale < self.speed_scale_target {
            self.speed_scale = (self.speed_scale + self.speed_scale_rate * dt).min(self.speed_scale_target);
        } else if self.speed_scale > self.speed_scale_target {
            self.speed_scale = (self.speed_scale + self.speed_scale_rate * dt).max(self.speed_scale_target);
        }
        self.proxy.velocity = composed * self.speed_scale;
        self.proxy.rotation = self.axis();
        self.proxy.integrate(world, dt);
        // 11. Read back.
        let out = self.proxy.velocity;
        if self.proxy.position.z > self.max_z {
            self.proxy.position.z = self.max_z;
        }
        // 12. Yaw.
        let mut yaw_rate = input.angular.map_or(0.0, |angular| angular.z);
        if !self.no_smoothing && yaw_rate != self.yaw_rate {
            let delta = yaw_rate - self.yaw_rate;
            let limit = if yaw_rate * self.yaw_rate < 0.0 || yaw_rate.abs() < self.yaw_rate.abs() {
                self.angular_deceleration
            } else {
                self.angular_acceleration
            };
            if delta.abs() > dt * limit {
                yaw_rate = self.yaw_rate + delta.signum() * dt * limit;
            }
        }
        self.yaw_rate = yaw_rate;
        yaw_rate += self.ground.angular;
        let output_velocity = out / self.speed_scale;
        if !self.slip && self.gravity_velocity.z < 0.0 && output_velocity.z - self.gravity_velocity.z > 0.1 {
            self.gravity_velocity.z = 0.0;
        }
        self.slip = false;
        self.yaw = wrap_angle(self.yaw + yaw_rate * dt);
        self.velocity = output_velocity;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::player::character_proxy::RayHit;
    use crate::gameplay::shape_cast;

    pub(crate) struct Floor(pub Vec<[Vec3; 3]>);

    impl PlayerWorld for Floor {
        fn cast_shape(&self, shape: &[Vec3], rotation: Mat3, from: Vec3, displacement: Vec3, tolerance: f32, cast: &mut Vec<WorldHit>, start: Option<&mut Vec<WorldHit>>) {
            let points: Vec<Vec3> = shape.iter().map(|p| from + rotation * *p).collect();
            let mut start = start;
            for (key, triangle) in self.0.iter().enumerate() {
                if let Some((fraction, contact)) = shape_cast::cast(&points, displacement, triangle, 0.001) {
                    cast.push(WorldHit { position: contact.position, normal: contact.normal, value: fraction, body: 0, key: key as u32 });
                }
                if let Some(start) = start.as_deref_mut().filter(|_| tolerance > 0.0) {
                    let contact = shape_cast::closest(&points, triangle);
                    if contact.distance <= tolerance {
                        start.push(WorldHit { position: contact.position, normal: contact.normal, value: contact.distance, body: 0, key: key as u32 });
                    }
                }
            }
        }
        fn cast_ray(&self, from: Vec3, direction: Vec3, length: f32) -> Option<RayHit> {
            (from.z > 0.0 && direction.z < 0.0 && from.z <= length).then(|| RayHit { distance: from.z, position: Vec3::new(from.x, from.y, 0.0), normal: Vec3::Z })
        }
    }

    pub(crate) fn floor() -> Floor {
        Floor(vec![
            [Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0)],
            [Vec3::new(-50.0, -50.0, 0.0), Vec3::new(50.0, 50.0, 0.0), Vec3::new(-50.0, 50.0, 0.0)],
        ])
    }

    pub(crate) fn decl() -> PhysicsDecl {
        let standing = ShapeSettings { radius: 0.4, height: 1.8, step_height: 0.5, gravity_strength: 3.0, ..Default::default() };
        let sneaking = ShapeSettings { height: 1.2, ..standing };
        PhysicsDecl {
            physics: PhysicsSettings { max_slope: 65.0, ..Default::default() },
            shapes: vec![(0, standing), (1, sneaking)],
            ..Default::default()
        }
    }

    #[test]
    fn falls_with_three_times_world_gravity_then_lands() {
        let world = floor();
        let mut physics = PlayerPhysics::new(&decl(), Vec3::new(0.0, 0.0, 2.0), 0.0);
        let dt = 1.0 / 120.0;
        physics.step(&world, dt, MotionInput::default());
        assert!((physics.velocity.z + 29.43 * dt).abs() < 1e-3, "{:?}", physics.velocity);
        for _ in 0..240 {
            physics.step(&world, dt, MotionInput::default());
        }
        assert_eq!(physics.ground.state, SupportState::Supported);
        assert!((physics.position().z - 0.01).abs() < 0.003, "{:?}", physics.position());
        assert_eq!(physics.gravity_velocity, Vec3::ZERO);
    }

    #[test]
    fn vertical_move_rises_to_the_apex_at_the_clamped_speed() {
        let world = floor();
        let mut physics = PlayerPhysics::new(&decl(), Vec3::new(0.0, 0.0, 0.01), 0.0);
        let dt = 1.0 / 120.0;
        for _ in 0..10 {
            physics.step(&world, dt, MotionInput::default());
        }
        physics.start_vertical(physics.position().z + 1.5, 5.1, 0.74);
        physics.step(&world, dt, MotionInput::default());
        assert!((physics.velocity.z - 5.1).abs() < 1e-3, "{:?}", physics.velocity);
        let mut peak: f32 = 0.0;
        for _ in 0..240 {
            physics.step(&world, dt, MotionInput::default());
            peak = peak.max(physics.position().z);
        }
        assert!(!physics.vertical_active());
        assert!(peak > 1.49 && peak < 1.53, "{peak}");
    }

    #[test]
    fn linear_acceleration_limits_the_input_velocity() {
        let world = floor();
        let mut physics = PlayerPhysics::new(&decl(), Vec3::new(0.0, 0.0, 0.01), 0.0);
        physics.set_motion_limits(10.0, 1000.0, UNSET, UNSET);
        let input = MotionInput { linear: Some(Vec3::new(2.4, 0.0, 0.0)), angular: None };
        physics.step(&world, 0.1, input);
        assert!((physics.velocity.x - 1.0).abs() < 1e-3, "{:?}", physics.velocity);
    }

    #[test]
    fn queries_measure_ground_and_ceiling() {
        let mut world = floor();
        world.0.push([Vec3::new(-50.0, -50.0, 2.5), Vec3::new(50.0, 50.0, 2.5), Vec3::new(50.0, -50.0, 2.5)]);
        let mut physics = PlayerPhysics::new(&decl(), Vec3::new(0.0, 0.0, 0.3), 0.0);
        physics.update_queries(&world);
        assert!((physics.ground_distance - 0.3).abs() < 0.002, "{}", physics.ground_distance);
        assert!((physics.ceil_distance - 2.2).abs() < 0.003, "{}", physics.ceil_distance);
        assert!(physics.can_use_posture(0));
    }

    #[test]
    fn angle_wrap_matches_the_engine_range() {
        use std::f32::consts::PI;
        assert!((wrap_angle(3.0 * PI / 2.0) + PI / 2.0).abs() < 1e-5);
        assert!((wrap_angle(-PI / 2.0) + PI / 2.0).abs() < 1e-5);
        assert_eq!(wrap_angle(PI), PI);
    }
}
