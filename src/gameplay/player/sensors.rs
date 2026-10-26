//! Player sensors (arkCpntPlayerSensor, ids `arkPlayerSensor_t`).
//!
//! The AutoCrouch sensor (arkCpntPlayerSensorAutoCrouch, Start FUN_1403a2430,
//! submit FUN_1403a2f60, results FUN_1403a2730) finds an overhang the player
//! can only pass crouched. Its queries run synchronously on the previous
//! frame's movement request, as the async submit/result pair reads it. The
//! WallCrouch sensor's decl starts disabled and no enabling caller was found,
//! so it is not ported (WALLCROUCH stays unreachable, as in the game).
use super::character_proxy::{PlayerWorld, RayHit, WorldHit};
use crate::gameplay::collision_filter::{layer, QueryFilter};

/// Every player sensor query requires its target to block the player.
const PLAYER_FLAG: u32 = 1 << layer::PLAYER;
use super::physics::PlayerPhysics;
use super::shape::{POSTURE_AUTOCROUCHING, POSTURE_SNEAKING, POSTURE_STANDING};
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

    fn hit_distance(world: &dyn PlayerWorld, filter: QueryFilter, shape: &[Vec3], rotation: Mat3, from: Vec3, direction: Vec3, length: f32) -> (f32, Option<WorldHit>) {
        let mut hits = Vec::new();
        world.cast_shape(filter, shape, rotation, from, direction * length, 0.0, &mut hits, None);
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
        // FUN_1403a2f60 submits every cast with layers 0x2b.
        let filter = QueryFilter::layers(0x2b, PLAYER_FLAG);
        let (left, _) = Self::hit_distance(world, filter, &self.sneak_left, rotation, start, self.direction, length);
        let (right, _) = Self::hit_distance(world, filter, &self.sneak_right, rotation, start, self.direction, length);
        let (crouch, _) = Self::hit_distance(world, filter, &self.crouch, rotation, start, self.direction, length);
        self.sneak_distance = left.max(right);
        if self.step_offset == 0.0 {
            self.sneak_distance_no_step = left.min(right);
        }
        self.step_offset = 0.0;
        let step_start = origin + Vec3::Z * self.keep_distance;
        if Self::hit_distance(world, filter, &self.crouch, rotation, step_start, self.direction, length).1.is_some() {
            for _ in 0..2 {
                let from = origin + Vec3::Z * self.step_offset;
                let Some(hit) = Self::hit_distance(world, filter, &self.crouch, rotation, from, self.direction, length).1 else { break };
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
/// `{}`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeanDecl {
    pub head_shape_size: Vec3,
    pub lean_amplitude: f32,
    pub lean_up_amplitude: f32,
    /// m_collisionLayers (+0x84), queried with the PLAYER flag required.
    pub collision_layers: u32,
}

impl Default for LeanDecl {
    fn default() -> Self {
        Self { head_shape_size: Vec3::new(0.05, 0.2, 0.1), lean_amplitude: 0.8, lean_up_amplitude: 0.35, collision_layers: 0x802b }
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
            let filter = QueryFilter::layers(self.decl.collision_layers, PLAYER_FLAG);
            let (distance, hit) = AutoCrouchSensor::hit_distance(world, filter, &self.head, axis, from, direction, amplitude);
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

/// arkNormalFilterOp at the vault sensor +0xa4 (FUN_1403cf960 /
/// FUN_1403cfaf0 / FUN_1403d0010): a ring of four 1/30 s averages.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NormalFilter {
    index: usize,
    samples: [Vec3; 4],
    period: f32,
    remaining: f32,
    accumulator: Vec3,
    count: u32,
}

fn normalize_min(v: Vec3) -> Vec3 {
    v / v.length_squared().max(f32::MIN_POSITIVE).sqrt()
}

impl NormalFilter {
    fn new(normal: Vec3) -> Self {
        // 1/30 (0x141abdad0).
        let period = 1.0 / 30.0;
        Self { index: 0, samples: [normal; 4], period, remaining: period, accumulator: normal, count: 1 }
    }

    fn add(&mut self, normal: Vec3, dt: f32) {
        self.accumulator += normal;
        self.count += 1;
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            self.accumulator = normalize_min(self.accumulator);
            self.index = (self.index + 1) % 4;
            self.samples[self.index] = self.accumulator;
            self.remaining += self.period;
            while self.remaining <= 0.0 {
                self.remaining += self.period;
                self.index = (self.index + 1) % 4;
                self.samples[self.index] = normal;
            }
            self.accumulator = normal;
            self.count = 1;
        }
    }

    fn get(&self) -> Vec3 {
        let sum: Vec3 = self.samples.iter().copied().sum();
        let oldest = self.samples[(self.index + 1) % 4];
        let shifted = normalize_min(sum - oldest + normalize_min(self.accumulator));
        let full = normalize_min(sum);
        let a = self.remaining / self.period;
        let snap = |v: Vec3| Vec3::from_array(v.to_array().map(|c| if c.abs() <= 1e-18 { 0.0 } else { c }));
        normalize_min((1.0 - a) * snap(shifted) + a * snap(full))
    }
}

/// The obstacle limits the vault sensor copies from the Vault state decl.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaultLimits {
    pub max_distance: f32,
    pub min_distance: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub max_depth: f32,
    pub clear_above: f32,
    pub clear_behind: f32,
    pub clear_left: f32,
    pub clear_right: f32,
}

/// arkCpntPlayerSensorVault (PLAYER_SENSOR 6, vtable 0x141b430b8). Issue
/// FUN_1403d0bf0 and results FUN_1403d06c0: the results read a frame's
/// queries in the next frame. Damageables (breakables in the clearance)
/// need dynamic bodies, which the static world has none of.
#[derive(Clone, Debug, PartialEq)]
pub struct VaultSensor {
    pub enabled: bool,
    limits: VaultLimits,
    distance_shape: Vec<Vec3>,
    height_shape: Vec<Vec3>,
    depth_shape: Vec<Vec3>,
    filter: NormalFilter,
    /// +0x40 origin, +0x4c axis (columns forward, lateral, up).
    pub origin: Vec3,
    pub axis: Mat3,
    pub can_vault: bool,
    pub vault_position: Vec3,
    pub vault_exit: Vec3,
    /// +0x10c m_vaultHeight.
    pub vault_height: f32,
    distance_hit: bool,
    obstacle: Vec3,
    /// +0x144 / +0x148, +0x15c / +0x160: kept until a cast hits again.
    height_blocked: bool,
    height: f32,
    depth_blocked: bool,
    depth: f32,
    pending: Option<VaultQueries>,
    /// +0x110 m_collisionMask: the sensor decl's m_collisionLayers (Start
    /// FUN_1403d0210; ctor FUN_1403cf880 0x802b, installed decl empty).
    collision_mask: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct VaultQueries {
    distance_shape: Option<WorldHit>,
    distance_ray: Option<f32>,
    ray_position: Vec3,
    ray_cast: bool,
    /// Min height-cast distance, when cast.
    height: Option<Option<f32>>,
    depth: Option<Option<f32>>,
    origin: Vec3,
    up: Vec3,
}

impl VaultSensor {
    /// FUN_1403d04c0 (from Vault Init); `radius` is the standing radius.
    pub fn new(mut limits: VaultLimits, radius: f32, ground_normal: Vec3) -> Self {
        let r = radius;
        limits.clear_left = limits.clear_left.max(r);
        limits.clear_right = limits.clear_right.max(r);
        let l = limits;
        Self {
            enabled: false,
            distance_shape: box_points(Vec3::new(-r, -r, l.min_height), Vec3::new(0.0, r, l.max_height)),
            height_shape: box_points(
                Vec3::new(-l.min_distance, -l.clear_right, l.max_height),
                Vec3::new(l.max_depth + l.clear_behind, l.clear_left, l.max_height + l.clear_above),
            ),
            depth_shape: box_points(Vec3::new(l.max_depth, -r, l.min_height), Vec3::new(l.max_depth + l.clear_behind, r, l.max_height)),
            limits,
            filter: NormalFilter::new(ground_normal),
            origin: Vec3::ZERO,
            axis: Mat3::IDENTITY,
            can_vault: false,
            vault_position: Vec3::ZERO,
            vault_exit: Vec3::ZERO,
            vault_height: 0.0,
            distance_hit: false,
            obstacle: Vec3::ZERO,
            height_blocked: false,
            height: 0.0,
            depth_blocked: false,
            depth: 0.0,
            pending: None,
            collision_mask: 0x802b,
        }
    }

    /// FUN_1403d02d0.
    pub fn enable(&mut self, enabled: bool, ground_normal: Vec3) {
        self.enabled = enabled;
        if enabled {
            self.filter = NormalFilter::new(ground_normal);
        } else {
            self.pending = None;
            self.can_vault = false;
        }
    }

    /// One frame: the previous queries' results, then this frame's queries.
    pub fn update(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics, dt: f32) {
        if !self.enabled {
            return;
        }
        if let Some(q) = self.pending.take() {
            self.results(world, q);
        }
        self.issue(world, physics, dt);
    }

    fn issue(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics, dt: f32) {
        self.origin = physics.position();
        self.filter.add(physics.ground_ray_normal_average, dt);
        let n = self.filter.get();
        let entity = physics.axis();
        let (forward, left) = (entity.x_axis, entity.y_axis);
        let up = normalize_min(n - left * left.dot(n));
        let side = normalize_min(up.cross(forward));
        let ahead = normalize_min(side.cross(up));
        self.axis = Mat3::from_cols(ahead, side, up);
        let l = self.limits;
        let mut q = VaultQueries { distance_shape: None, distance_ray: None, ray_position: Vec3::ZERO, ray_cast: false, height: None, depth: None, origin: self.origin, up };
        // FUN_1403d0bf0: the height and distance casts add NPC.
        let plain = QueryFilter::layers(self.collision_mask, PLAYER_FLAG);
        let with_npc = QueryFilter::layers(self.collision_mask | 1 << layer::NPC, PLAYER_FLAG);
        let closest = |filter: QueryFilter, shape: &[Vec3], from: Vec3, direction: Vec3, length: f32| {
            let mut hits = Vec::new();
            world.cast_shape(filter, shape, self.axis, from, direction * length, 0.0, &mut hits, None);
            hits.into_iter().min_by(|a, b| a.value.total_cmp(&b.value)).map(|h| (h.value * length, h))
        };
        if self.distance_hit {
            if l.max_height > 0.0 {
                let length = l.max_height - l.min_height;
                q.height = Some(closest(with_npc, &self.height_shape, self.obstacle, -up, length).map(|(d, _)| d));
            }
            if l.max_depth > 0.0 {
                q.depth = Some(closest(plain, &self.depth_shape, self.obstacle, -ahead, l.max_depth).map(|(d, _)| d));
            }
        }
        if l.max_distance > 0.0 {
            q.distance_shape = closest(with_npc, &self.distance_shape, self.origin, ahead, l.max_distance).map(|(_, h)| h);
            q.ray_cast = true;
            if let Some(hit) = world.cast_ray(plain, self.origin + up * l.min_height, ahead, l.max_distance) {
                q.distance_ray = Some(hit.distance);
                q.ray_position = hit.position;
            }
        }
        self.pending = Some(q);
    }

    fn results(&mut self, world: &dyn PlayerWorld, q: VaultQueries) {
        let l = self.limits;
        // A cast with no hits leaves the previous flag and measure.
        if let Some(Some(frac)) = q.height {
            self.height = l.max_height - frac;
            self.height_blocked = !(frac > f32::EPSILON);
        }
        if let Some(Some(frac)) = q.depth {
            self.depth = l.max_depth - frac;
            self.depth_blocked = !(frac > f32::EPSILON);
        }
        self.can_vault = self.distance_hit && !self.height_blocked && !self.depth_blocked;
        if self.can_vault {
            self.vault_position = self.obstacle;
            self.vault_exit = self.obstacle + self.axis.x_axis * self.depth;
            self.vault_height = self.height;
        }
        let project = |p: Vec3| p - q.up * (p - q.origin).dot(q.up);
        let mut ray_ok = false;
        if let Some(distance) = q.distance_ray {
            ray_ok = distance > l.min_distance;
            self.obstacle = q.ray_position;
        }
        if q.ray_cast {
            self.obstacle = project(self.obstacle);
        }
        self.distance_hit = match q.distance_shape {
            Some(hit) if !world.surface(&hit).allow_vault => false,
            Some(hit) => {
                if !ray_ok {
                    ray_ok = true;
                    self.obstacle = project(hit.position);
                }
                ray_ok
            }
            None => false,
        };
    }
}

/// arkDeclCpntPlayerSensorMantle (ctor FUN_1403baf00).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MantleDecl {
    pub max_distance_from_wall: f32,
    pub forward_cast_height_min: f32,
    pub forward_cast_height_max: f32,
    pub mid_mantle_height: f32,
    pub high_mantle_height: f32,
    /// Degrees.
    pub max_player_angle: f32,
    pub destination_depth: f32,
    pub wall_angle_max: f32,
    pub hand_box_size: Vec3,
    pub left_hand_offset: Vec3,
    pub right_hand_offset: Vec3,
    pub use_player_direction: bool,
    /// m_collisionMask (+0xb4): the layers every mantle query reads, with
    /// the PLAYER flag required.
    pub collision_mask: u32,
}

impl Default for MantleDecl {
    fn default() -> Self {
        Self {
            max_distance_from_wall: 1.0,
            forward_cast_height_min: 0.5,
            forward_cast_height_max: 2.0,
            mid_mantle_height: 0.8,
            high_mantle_height: 1.5,
            max_player_angle: 45.0,
            destination_depth: 0.07,
            wall_angle_max: 58.0,
            hand_box_size: Vec3::new(0.1, 0.15, 0.1),
            left_hand_offset: Vec3::new(0.0, 0.2, 0.0),
            right_hand_offset: Vec3::new(0.0, 0.2, 0.0),
            use_player_direction: false,
            collision_mask: 0x2b,
        }
    }
}

/// arkPlayerMantleHeight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MantleHeight {
    Low = 0,
    Mid = 1,
    High = 2,
}

/// FUN_1400efc60: an 8-sided prism (corner directions 0x141ae36e0) of
/// radius `r` from `base` to `base + height`.
fn prism(r: f32, base: f32, height: f32) -> Vec<Vec3> {
    let (a, b) = (0.9238795, 0.38268343);
    let corners = [(a, b), (b, a), (-b, a), (-a, b), (-a, -b), (-b, -a), (b, -a), (a, -b)];
    let mut points = Vec::with_capacity(16);
    for z in [base, base + height] {
        points.extend(corners.iter().map(|&(x, y)| Vec3::new(x * r, y * r, z)));
    }
    points
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ForwardRay {
    collision: bool,
    distance: f32,
    position: Vec3,
    normal: Vec3,
    over_ceiling: bool,
}

/// The queries one frame issues; the next frame reads them.
#[derive(Clone, Debug, Default, PartialEq)]
struct MantleQueries {
    forward: Option<Option<WorldHit>>,
    up: Option<Option<f32>>,
    rays: Option<Vec<Option<RayHit>>>,
    postures: Option<[bool; 3]>,
    hands: Option<[bool; 2]>,
}

/// arkCpntPlayerSensorMantle (PLAYER_SENSOR 1, vtable 0x141b40cc8): the
/// deferred queries (issue FUN_1403bcb20, results FUN_1403bcb00) find a
/// ledge top in front of the player. Moving platforms, elevator area tags
/// and swimming need entities the static world lacks; the position/axis
/// overrides belong to the blink and grapple variants.
#[derive(Clone, Debug, PartialEq)]
pub struct MantleSensor {
    pub decl: MantleDecl,
    pub enabled: bool,
    hand: Vec<Vec3>,
    sneaking_no_height: Vec<Vec3>,
    standing_no_height: Vec<Vec3>,
    standing_height: f32,
    autocrouch_height: f32,
    forward_valid: bool,
    search_direction: Vec3,
    search_axis: Mat3,
    /// m_forwardShapeCastCollisions: STANDING, SNEAKING, AUTOCROUCHING.
    pub collisions: [bool; 3],
    pub ceil_height: f32,
    pub use_left_hand: bool,
    pub use_right_hand: bool,
    /// m_destinationPosition (zero: none) and m_destinationAxis.
    pub destination: Vec3,
    pub destination_axis: Mat3,
    pending: Option<MantleQueries>,
}

impl MantleSensor {
    /// Start FUN_1403bbb50.
    pub fn new(decl: MantleDecl, physics: &PlayerPhysics) -> Self {
        let half = decl.hand_box_size * 0.5;
        let stand = physics.posture_settings(POSTURE_STANDING);
        let sneak = physics.posture_settings(POSTURE_SNEAKING);
        Self {
            decl,
            enabled: true,
            hand: box_points(-half, half),
            sneaking_no_height: prism(sneak.radius, sneak.step_height, sneak.height - sneak.step_height),
            standing_no_height: prism(stand.radius, stand.step_height, stand.height - stand.step_height),
            standing_height: stand.height,
            autocrouch_height: physics.posture_settings(POSTURE_AUTOCROUCHING).height,
            forward_valid: false,
            search_direction: Vec3::X,
            search_axis: Mat3::IDENTITY,
            collisions: [true; 3],
            ceil_height: 0.0,
            use_left_hand: false,
            use_right_hand: false,
            destination: Vec3::ZERO,
            destination_axis: Mat3::IDENTITY,
            pending: None,
        }
    }

    /// FUN_1403bb8d0.
    fn step(&self) -> f32 {
        (self.decl.forward_cast_height_max - self.decl.forward_cast_height_min) / 12.0
    }

    /// One frame: the previous frame's results, then this frame's queries.
    /// `velocity_z` is the entity's vertical velocity.
    pub fn update(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics, dt: f32, velocity_z: f32) {
        if !self.enabled {
            return;
        }
        let q = self.pending.take().unwrap_or_default();
        self.results(world, physics, q, dt, velocity_z);
        self.issue(world, physics);
    }

    fn results(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics, q: MantleQueries, dt: f32, velocity_z: f32) {
        let d = self.decl;
        let origin = physics.position();
        let forward = physics.axis().x_axis;
        if let Some([left, right]) = q.hands {
            self.use_left_hand = left;
            self.use_right_hand = right;
        }
        self.collisions = q.postures.unwrap_or([true; 3]);
        if let Some(up) = q.up {
            self.ceil_height = match up {
                Some(distance) => self.autocrouch_height + distance,
                None => d.forward_cast_height_max,
            };
        }
        self.destination = Vec3::ZERO;
        let step = self.step();
        if let Some(rays) = q.rays {
            let h0 = d.forward_cast_height_min;
            let ceil = physics.ceil_distance;
            let mut results = [ForwardRay::default(); 13];
            for (i, (r, hit)) in results.iter_mut().zip(rays).enumerate() {
                let z = origin.z + h0 + i as f32 * step;
                if let Some(hit) = hit {
                    *r = ForwardRay {
                        collision: true,
                        distance: hit.position.distance(Vec3::new(origin.x, origin.y, z)),
                        position: hit.position,
                        normal: hit.normal,
                        over_ceiling: false,
                    };
                }
                // phys+0xdc, the AUTOCROUCHING height.
                r.over_ceiling = ceil < self.autocrouch_height + h0 + i as f32 * step;
            }
            if let Some(best) = self.edge(&results, step) {
                let e = results[best];
                let nz = e.normal.z.max(0.0);
                let nxy = Vec3::new(e.normal.x, e.normal.y, 0.0);
                let nxy = nxy / nxy.length_squared().max(f32::MIN_POSITIVE).sqrt();
                let p = e.position - nxy * (nz * step + d.destination_depth);
                let start = Vec3::new(p.x, p.y, e.position.z + step + 0.01);
                if let Some(hit) = world.cast_ray(QueryFilter::layers(self.decl.collision_mask, PLAYER_FLAG), start, -Vec3::Z, 2.0 * step) {
                    let surface = WorldHit { position: hit.position, normal: hit.normal, value: 0.0, body: hit.body, key: hit.key };
                    if world.surface(&surface).allow_mantle {
                        let rise = if velocity_z > 0.0 { dt * velocity_z } else { 0.0 };
                        self.destination = Vec3::new(hit.position.x, hit.position.y, hit.position.z + 0.01 + rise);
                    }
                }
            }
            self.destination_axis = self.search_axis;
        }
        self.forward_valid = false;
        if let Some(Some(hit)) = q.forward {
            if world.surface(&hit).allow_mantle {
                let wall = Vec3::new(-hit.normal.x, -hit.normal.y, 0.0);
                let yaw = |v: Vec3| v.y.atan2(v.x).to_degrees().rem_euclid(360.0);
                let mut delta = yaw(forward) - yaw(wall);
                delta -= (delta / 360.0).round() * 360.0;
                if delta > 180.0 {
                    delta -= 360.0;
                }
                if delta.abs() <= d.max_player_angle {
                    let direction = if d.use_player_direction { Vec3::new(forward.x, forward.y, 0.0) } else { wall };
                    self.search_direction = direction / direction.length_squared().max(f32::MIN_POSITIVE).sqrt();
                    self.search_axis = Mat3::from_rotation_z(self.search_direction.y.atan2(self.search_direction.x));
                    self.forward_valid = true;
                }
            }
        }
    }

    /// FUN_1403bb3d0: the highest ray at a wall top within reach.
    fn edge(&self, r: &[ForwardRay; 13], step: f32) -> Option<usize> {
        let d = self.decl;
        let wall = |n: Vec3| {
            let z = n.z.clamp(-1.0, 1.0);
            let elevation = 90.0 - z.acos() * 57.29578;
            elevation < d.wall_angle_max - f32::EPSILON
        };
        let reach = (d.wall_angle_max * 0.017453292).tan() * step;
        let mut best = None;
        for i in 0..12 {
            if r[i].over_ceiling {
                return best;
            }
            let (wc, wn) = (wall(r[i].normal), wall(r[i + 1].normal));
            let ok = if !r[i + 1].collision && wc {
                true
            } else if r[i + 1].distance < r[i].distance + reach {
                wc && !wn
            } else {
                wc || wn
            };
            if r[i].distance <= d.max_distance_from_wall && r[i].collision && ok {
                best = Some(i);
            }
        }
        best
    }

    fn issue(&mut self, world: &dyn PlayerWorld, physics: &PlayerPhysics) {
        let d = self.decl;
        let origin = physics.position();
        let forward = physics.axis().x_axis;
        let mask = QueryFilter::layers(d.collision_mask, PLAYER_FLAG);
        // ShapeCastForward_Deferred FUN_1403bbf00 adds INTERACTABLE.
        let posture_mask = QueryFilter::layers(d.collision_mask | 1 << layer::INTERACTABLE, PLAYER_FLAG);
        let first = |shape: &[Vec3], rotation: Mat3, from: Vec3, displacement: Vec3| {
            let mut hits = Vec::new();
            world.cast_shape(mask, shape, rotation, from, displacement, 0.0, &mut hits, None);
            hits.into_iter().min_by(|a, b| a.value.total_cmp(&b.value))
        };
        let any = |filter: QueryFilter, shape: &[Vec3], rotation: Mat3, from: Vec3, displacement: Vec3| {
            let (mut hits, mut start) = (Vec::new(), Vec::new());
            world.cast_shape(filter, shape, rotation, from, displacement, 0.001, &mut hits, Some(&mut start));
            !hits.is_empty() || start.iter().any(|h| h.value <= 0.001)
        };
        let mut q = MantleQueries::default();
        let shape = if physics.ceil_distance <= self.standing_height { &self.sneaking_no_height } else { &self.standing_no_height };
        q.forward = Some(first(shape, Mat3::IDENTITY, origin, forward * d.max_distance_from_wall));
        let up_length = d.forward_cast_height_max;
        q.up = Some(first(physics.posture_body(POSTURE_AUTOCROUCHING), Mat3::IDENTITY, origin, Vec3::Z * up_length).map(|h| h.value * up_length));
        if self.forward_valid {
            let step = self.step();
            let length = d.max_distance_from_wall + (d.wall_angle_max * 0.017453292).tan() * step;
            q.rays = Some(
                (0..13)
                    .map(|i| world.cast_ray(mask, origin + Vec3::Z * (d.forward_cast_height_min + i as f32 * step), self.search_direction, length))
                    .collect(),
            );
        }
        if self.destination != Vec3::ZERO {
            let dest = self.destination;
            let flat = Vec3::new(dest.x - origin.x, dest.y - origin.y, 0.0);
            let from = Vec3::new(origin.x, origin.y, dest.z + 0.01);
            q.postures = Some([POSTURE_STANDING, POSTURE_SNEAKING, POSTURE_AUTOCROUCHING].map(|p| any(posture_mask, physics.posture_body(p), Mat3::IDENTITY, from, flat)));
            let left = dest + self.destination_axis * d.left_hand_offset;
            let right = dest - self.destination_axis * d.right_hand_offset;
            q.hands = Some([left, right].map(|p| any(mask, &self.hand, Mat3::IDENTITY, p, Vec3::ZERO)));
        }
        self.pending = Some(q);
    }

    /// FUN_1403bca70 / FUN_1403bcab0.
    pub fn height(&self, origin: Vec3) -> MantleHeight {
        let h = self.destination.z - origin.z;
        if h >= self.decl.high_mantle_height {
            MantleHeight::High
        } else if self.decl.mid_mantle_height <= h {
            MantleHeight::Mid
        } else {
            MantleHeight::Low
        }
    }

    /// FUN_1403bb730 / FUN_1403bb760: the final posture and state.
    pub fn final_posture(&self) -> Option<(usize, i32)> {
        match self.collisions {
            [false, _, _] => Some((POSTURE_STANDING, 0)),
            [true, false, _] => Some((POSTURE_SNEAKING, 6)),
            [true, true, false] => Some((POSTURE_AUTOCROUCHING, 10)),
            _ => None,
        }
    }

    /// CanMantle FUN_1403bbd90 (no corpses or shadow walk in the port).
    pub fn can_mantle(&self) -> bool {
        self.destination != Vec3::ZERO && self.collisions != [true; 3]
    }
}
