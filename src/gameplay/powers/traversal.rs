//! Blink collision targeting and mantle hysteresis, 14061e230/14061ede0/
//! 14061f9e0/14061fd20/1406205e0. All vectors use game space (Z up).
//! The host provides the existing player shape and mantle owners; neither a
//! camera ray nor an invented target object substitutes for these queries.
use bevy_math::{Mat3, Vec3};
use crate::{assets::entities::Value, gameplay::options::decl::atom};

// DAT_141ae3c60 = 0x00800000, used before the reciprocal square root in
// 14061ede0/14061fd20/14043f9b0. Do not substitute a gameplay epsilon.
fn normalized(value: Vec3) -> Vec3 { value / value.length_squared().max(f32::MIN_POSITIVE).sqrt() }

#[derive(Clone, Copy, Debug)]
pub struct ShapeInfo {
    /// The values retained beside the shape by 14061e3d0 (+0/+4/+0xc).
    pub radius: f32,
    pub height: f32,
    pub margin: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct Hit { pub fraction: f32, pub position: Vec3, pub normal: Vec3, pub body: u32, pub key: u32 }
#[derive(Clone, Copy, Debug)]
pub struct Breakable { pub entity: crate::gameplay::entity_lifetime::EntityId, pub hit_position: Vec3 }
#[derive(Clone, Copy, Debug)]
pub struct MantleTarget { pub position: Vec3, pub axis: Mat3 }

pub trait BlinkWorld {
    /// 14061e500/14061e6c0 use the indicated shape from the player's 20
    /// acquired slots and arkCollisionQueryFilter(mask, group=0x40).
    fn sweep(&mut self, slot: usize, mask: u32, from: Vec3, direction: Vec3, length: f32) -> Option<Hit>;
    fn sweep_all(&mut self, slot: usize, mask: u32, from: Vec3, direction: Vec3, length: f32, hits: &mut Vec<Hit>);
    /// 14061e1a0 permits a body only when material flag 0x04 is set AND
    /// that body's entity has the breakable component. Return that identity.
    fn blink_breakable(&self, hit: &Hit) -> Option<crate::gameplay::entity_lifetime::EntityId>;
    /// 1406205e0 writes cpntPlayerSensorMantle's origin/axis overrides then
    /// invokes 1403bd2e0. Return its actual destination and axis.
    fn mantle(&mut self, origin: Vec3, axis: Mat3) -> Option<MantleTarget>;
}

#[derive(Clone, Debug)]
pub struct BlinkSettings {
    pub vertical_distance: f32,
    pub snap_angle: f32,
    pub ground_search_distance: f32,
    pub ceiling_search_distance: f32,
    pub collision_mask: u32,
    pub cover_min_height: f32,
    pub cover_max_height: f32,
    pub cover_ratio_sneak: f32,
    pub cover_ratio_stand: f32,
    pub fx_min_height: [f32; 3],
}
impl BlinkSettings {
    pub fn read(fields: &[(String, Value)], collision_mask: u32) -> Result<Self, String> {
        let n = |name, default| match atom(fields, name) { None => Ok(default), Some(value) => value.trim_end_matches('f').parse::<f32>()
            .ok().filter(|v| v.is_finite()).ok_or_else(|| format!("invalid Blink {name}: {value}")) };
        // Defaults: arkDeclCpntPowerBlink constructor 14061d660.
        Ok(Self { vertical_distance: n("m_maxVerticalDistance", 10.0)?, snap_angle: n("m_snapMaxAngle", 2.5)?,
            ground_search_distance: n("m_groundSearchCastDistance", 200.0)?, ceiling_search_distance: n("m_ceilSearchCastDistance", 4.0)?,
            collision_mask, cover_min_height: n("m_coverMinObstacleHeight", 0.8)?, cover_max_height: n("m_coverMaxObstacleHeight", 1.5)?,
            cover_ratio_sneak: n("m_coverHeightRatioSneak", 0.7)?, cover_ratio_stand: n("m_coverHeightRatioStand", 0.7)?,
            fx_min_height: [n("m_blinkTargetFxMinHeightStand", 1.0)?, n("m_blinkTargetFxMinHeightSneak", 0.7)?, n("m_blinkTargetFxMinHeightCrouch", 0.7)?] })
    }
    /// 14061e230 intersects the view direction with the ellipse whose
    /// vertical radius is the decl and horizontal radius is the attribute.
    pub fn range(&self, direction: Vec3, horizontal_distance: f32) -> f32 {
        let horizontal = direction.truncate().length();
        if horizontal <= 1.1920929e-7 { return self.vertical_distance; }
        let tangent = -direction.z / horizontal;
        let denominator = (horizontal_distance * horizontal_distance * tangent * tangent
            + self.vertical_distance * self.vertical_distance).sqrt();
        let vertical = self.vertical_distance * horizontal_distance * tangent / denominator;
        let horizontal = self.vertical_distance * horizontal_distance / denominator;
        (vertical * vertical + horizontal * horizontal).sqrt()
    }
}

pub struct BlinkInput<'a> {
    pub shapes: &'a [ShapeInfo; 20],
    pub current_shape: usize,
    pub physical_position: Vec3,
    pub camera_position: Vec3,
    pub camera_direction: Vec3,
    pub player_axis: Mat3,
    pub horizontal_distance: f32,
    /// Proxy keep distance + keep contact tolerance + 0.1 (14061ff90).
    pub clearance: f32,
    /// The active assassination component's actual selected target position.
    pub assassination_target: Option<Vec3>,
}
#[derive(Clone, Debug)]
pub struct BlinkTarget {
    pub position: Vec3,
    /// RUNNING=0, SNEAKING=6, AUTOCROUCH=10, MANTLE=8.
    pub state: i32,
    pub shape: usize,
    pub mantle: Option<MantleTarget>,
    pub top_fx: Option<Vec3>,
    pub bottom_fx: Option<Vec3>,
}
#[derive(Clone, Debug)]
pub struct BlinkRequest {
    /// Final navigation origin (+0x54 in state12), distinct from the
    /// posture-adjusted movement destination (+0x60).
    pub final_position: Vec3,
    pub position: Vec3,
    pub final_position: Vec3,
    pub camera_source: Vec3,
    pub exit_state: i32,
    pub posture: usize,
    pub mantle: Option<MantleTarget>,
}
#[derive(Clone, Copy)]
struct MantleLock { destination: MantleTarget, origin: Vec3, direction: Vec3, position: Vec3 }

pub struct Blink {
    pub settings: BlinkSettings,
    hits: Vec<Hit>,
    /// Breakables crossed by the current shape sweep; damage is dispatched
    /// by the real breakable owner on traversal, not while aiming.
    pub breakables: Vec<Breakable>,
    lock: Option<MantleLock>,
    pub target: Option<BlinkTarget>,
}
impl Blink {
    pub fn new(settings: BlinkSettings) -> Self {
        Self { settings, hits: Vec::new(), breakables: Vec::new(), lock: None, target: None }
    }
    pub fn cancel(&mut self) { self.target = None; self.lock = None; self.breakables.clear(); self.hits.clear(); }
    /// 14061ec20 prepares arkPlayerStateBlinking (state 12), then invokes
    /// shared mana payment. The host must admit that actual state before
    /// charging; a direct transform write omits collision/proxy restoration.
    pub fn cast_request(&self, input: &BlinkInput<'_>) -> Option<BlinkRequest> {
        let target = self.target.as_ref()?;
        Some(BlinkRequest {
            final_position: target.position,
            position: target.position + Vec3::Z * (input.shapes[target.shape].height
                - input.shapes[target.shape].margin - input.shapes[4].height),
            final_position: target.position,
            camera_source: Vec3::new(input.camera_position.x, input.camera_position.y,
                input.physical_position.z + input.shapes[input.current_shape].height - input.shapes[4].height),
            exit_state: target.state, posture: target.shape, mantle: target.mantle,
        })
    }
    fn distance(&self, world: &mut dyn BlinkWorld, slot: usize, from: Vec3, direction: Vec3, length: f32) -> f32 {
        world.sweep(slot, self.settings.collision_mask, from, direction, length).map_or(length, |hit| hit.fraction * length)
    }
    /// Full targeting pass. It uses the owned scratch collector across
    /// frames; the host does not reacquire shape files on power input.
    pub fn aim(&mut self, world: &mut dyn BlinkWorld, input: &BlinkInput<'_>) -> &BlinkTarget {
        let d = input.camera_direction;
        let range = self.settings.range(d, input.horizontal_distance);
        let shapes = input.shapes;
        let crouch_height = shapes[4].height;
        let current_height = shapes[input.current_shape].height;
        let origin = Vec3::new(input.camera_position.x, input.camera_position.y,
            input.physical_position.z + current_height - crouch_height);
        self.hits.clear(); self.breakables.clear();
        world.sweep_all(4, self.settings.collision_mask, origin, d, range, &mut self.hits);
        self.hits.sort_by(|a,b| a.fraction.total_cmp(&b.fraction));
        let mut normal = None;
        let mut hit_distance = range;
        for hit in &self.hits {
            if let Some(body) = world.blink_breakable(hit) {
                if !self.breakables.iter().any(|entry| entry.entity == body) { self.breakables.push(Breakable { entity: body, hit_position: hit.position }); }
            } else { normal = Some(hit.normal); hit_distance = range * hit.fraction; break; }
        }
        let mut hit_position = origin + d * hit_distance;
        if let Some(target) = input.assassination_target.filter(|target| target.distance(input.camera_position) <= range) {
            hit_position = target;
            normal = Some(normalized((target - input.camera_position).with_z(0.0)));
            hit_distance = target.distance(input.camera_position);
        }
        let mut candidate = hit_position;
        let wall = normal.is_some_and(|normal| normal.z < 0.9);
        if let Some(normal) = normal {
            let horizontal = normal.with_z(0.0);
            if horizontal.length() > 0.9 { candidate += normalized(horizontal) * (shapes[2].radius - shapes[4].radius); }
        }
        // 14061ede0 searches two camera-height variants backwards in 0.4 m
        // steps, capped at ten, to fit the player's actual destination shape.
        let step = 0.4;
        let mut upper = candidate - d * step;
        let mut lower = upper - Vec3::Z * (shapes[2].height - shapes[4].height);
        let mut found = None;
        for index in 1..=10 {
            if hit_distance <= index as f32 * step { break; }
            let upper_distance = self.distance(world, 2, upper, d, step);
            let lower_distance = self.distance(world, 2, lower, d, step);
            if lower_distance - upper_distance > 0.01 && lower_distance > 0.0 {
                found = Some(lower + d * (lower_distance - input.clearance)); break;
            }
            if upper_distance > 0.0 { found = Some(upper + d * (upper_distance - input.clearance)); break; }
            upper -= d * step; lower -= d * step;
        }
        let fallback = Vec3::new(input.camera_position.x, input.camera_position.y,
            input.physical_position.z + current_height - shapes[2].height);
        candidate = found.unwrap_or_else(|| fallback + d * (self.distance(world, 2, fallback, d, range) - input.clearance));
        let (mut candidate, mut shape, mut needs_clearance, mut bottom) = self.clearance(world, input, candidate);
        // Small near-horizontal destinations get the original extra fit pass.
        if (hit_position.z - candidate.z).abs() < 0.05 {
            let direction = (hit_position - candidate).with_z(0.0);
            let length = direction.length();
            let direction = normalized(direction);
            let distance = self.distance(world, 2, candidate + Vec3::Z * 0.005, direction, length) - input.clearance;
            if distance >= 0.0 { (candidate, shape, needs_clearance, bottom) = self.clearance(world, input, candidate + direction * distance); }
        }
        let mantle = world.mantle(candidate, input.player_axis);
        let cover = mantle.is_some_and(|target| {
            let height = target.position.z - candidate.z;
            let ratio = if matches!(input.current_shape, 1 | 2) { self.settings.cover_ratio_sneak } else { self.settings.cover_ratio_stand };
            height >= self.settings.cover_min_height && height <= self.settings.cover_max_height
                && (height - (target.position.z - hit_position.z)) / height < ratio
        });
        let mut selected_mantle = None;
        if needs_clearance && wall && mantle.is_some() && !cover {
            let destination = mantle.unwrap();
            self.lock = Some(MantleLock { destination, origin: candidate, direction: d, position: input.camera_position });
            selected_mantle = Some(destination);
        } else if let Some(lock) = self.lock {
            let cosine = self.settings.snap_angle.to_radians().cos();
            let previous = normalized(lock.position - lock.origin);
            let current = normalized(input.camera_position - lock.origin);
            if previous.dot(current) > cosine && lock.direction.dot(d) > cosine {
                candidate = lock.origin; selected_mantle = Some(lock.destination);
            } else { self.lock = None; }
        }
        let state = if selected_mantle.is_some() { 8 } else { match shape { 0 | 7 => 0, 1 | 3 => 6, _ => 10 } };
        let top_fx = if selected_mantle.is_none() {
            let xy = (bottom - input.camera_position).truncate().length();
            let horizontal = d.truncate().length();
            let height = if horizontal <= 1.1920929e-7 { candidate.z } else { input.camera_position.z + xy * d.z / horizontal };
            let index = if shape == 2 { 2 } else if shape == 1 { 1 } else { 0 };
            Some(bottom.with_z(bottom.z + (height - bottom.z).max(self.settings.fx_min_height[index])))
        } else { None };
        if selected_mantle.is_some() { shape = input.current_shape; }
        self.target = Some(BlinkTarget { position: candidate, state, shape, mantle: selected_mantle,
            top_fx, bottom_fx: selected_mantle.is_none().then_some(bottom) });
        self.target.as_ref().unwrap()
    }
    /// 14061f9e0: downward/upward sweeps select the fitting player posture.
    fn clearance(&self, world: &mut dyn BlinkWorld, input: &BlinkInput<'_>, mut candidate: Vec3) -> (Vec3, usize, bool, Vec3) {
        let ground = self.distance(world, 2, candidate, -Vec3::Z, self.settings.ground_search_distance);
        let ceiling = self.distance(world, 2, candidate, Vec3::Z, self.settings.ceiling_search_distance);
        let mut shape = input.current_shape;
        let height = input.shapes[shape].height;
        let crouch = input.shapes[2].height;
        let bottom = candidate - Vec3::Z * ground;
        let needs_clearance = height > crouch + ground;
        if !needs_clearance { candidate.z -= height - crouch; }
        else {
            let available = crouch + ground + ceiling;
            if available < height {
                if shape == 0 { if available < height { shape = 1; } if available < input.shapes[1].height { shape = 2; } }
                else if shape == 1 && available < height { shape = 2; }
            }
            candidate.z -= ground;
        }
        (candidate, shape, needs_clearance, bottom)
    }
}

/// PlayerStateBlinking's declaration (140857150, reflection14177c170).
#[derive(Clone)]
pub struct BlinkMotionSettings {
    pub speed: f32,
    pub world_time_scale: f32,
    pub unbent_time_scale: f32,
    pub ease_in_duration: f32,
    pub ease_out_duration: f32,
    pub maximum_collision_angle: f32,
    pub breakable_anticipation_distance: f32,
    pub breakable_damage: Option<String>,
}
impl BlinkMotionSettings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let n = |name, default| match atom(fields, name) { None => Ok(default), Some(value) => value.trim_end_matches('f').parse::<f32>()
            .ok().filter(|v| v.is_finite()).ok_or_else(|| format!("invalid Blinking {name}: {value}")) };
        Ok(Self { speed: n("m_speed", 75.0)?, world_time_scale: n("m_worldTimeScale", 0.0)? * 0.01,
            unbent_time_scale: n("m_unbentTimeScale", 1.0)?, ease_in_duration: n("m_easeInDurationInSec", 0.0)?,
            ease_out_duration: n("m_easeOutDurationInSec", 0.0)?, maximum_collision_angle: n("m_maxWorldCollisionAngle", 25.0)?,
            breakable_anticipation_distance: n("m_breakableAnticipationDistance", 2.0)?,
            breakable_damage: atom(fields,"m_breakableDamage").filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("NULL")).map(str::to_owned) })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BlinkContact {
    /// hknpBody flags read by 140857df0 (+0x44, mask0x05).
    pub body_flags: u8,
    pub normal: Vec3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlinkFailure { VelocityTooLow, BlockingContact }
pub enum BlinkStep {
    /// The player's existing velocity request channel owns integration.
    Velocity(Vec3),
    /// 140858010 first clears that channel, places the final posture origin
    /// and resumes the selected state. Restore the pre-Blink velocity only
    /// for the original sprint/slide/fall transition that requests it.
    Finish,
}

/// Real state12 movement, 140858950. No elapsed-duration substitute: the
/// physics owner's measured position and contacts determine every step.
pub struct BlinkMotion {
    pub settings: BlinkMotionSettings,
    pub request: BlinkRequest,
    pub prior_velocity: Vec3,
    first_step: bool,
    finish_next: bool,
}
impl BlinkMotion {
    /// The host's 140857300 entry must first save the posture/velocity,
    /// remove ordinary movement/gravity requests, select posture shape4,
    /// zero velocity, set controller lock0x4000, disable ordinary mantle and
    /// enable Blink-to-mantle plus Blink sensors. It also acquires the world
    /// time lease described by settings; that lease ends on every exit.
    pub fn new(settings: BlinkMotionSettings, request: BlinkRequest, prior_velocity: Vec3) -> Self {
        Self { settings, request, prior_velocity, first_step: true, finish_next: false }
    }
    pub fn step(&mut self, position: Vec3, velocity: Vec3, dt: f32, contacts: &[BlinkContact]) -> Result<BlinkStep, BlinkFailure> {
        if self.finish_next { return Ok(BlinkStep::Finish); }
        // 140857fb0: the first movement update is exempt because entry has
        // explicitly cleared the velocity request channel.
        if !self.first_step && velocity.length_squared() < 0.1 { return Err(BlinkFailure::VelocityTooLow); }
        let direction = normalized(velocity);
        let collision_cosine = self.settings.maximum_collision_angle.to_radians().cos();
        for contact in contacts {
            let dot = direction.dot(contact.normal);
            if contact.body_flags & 5 == 0 || (dot <= 0.0 && dot.abs() > collision_cosine) {
                return Err(BlinkFailure::BlockingContact);
            }
        }
        let delta = self.request.position - position;
        let direction = normalized(delta);
        let desired = direction * self.settings.speed;
        self.first_step = false;
        if (desired * dt).length() <= delta.length() { Ok(BlinkStep::Velocity(desired)) }
        else {
            // The original uses the entity's unbent dt, not frame-wall dt.
            // Its final short step is integrated before next update exits.
            self.finish_next = true;
            Ok(BlinkStep::Velocity(direction * (delta.length() / dt)))
        }
    }
    /// 1408584c0's anticipation test, before dispatching the installed
    /// m_breakableDamage through the shared damage owner. This uses the
    /// game's fourth-power bound as written; do not simplify it to radius².
    pub fn anticipates_breakable(&self, physical_position: Vec3, player_forward: Vec3, hit_position: Vec3, dt: f32) -> bool {
        let projection = player_forward.dot(hit_position - physical_position);
        let projected = player_forward.truncate() * projection;
        let reach = (self.settings.speed * dt).max(self.settings.breakable_anticipation_distance);
        projected.length_squared() < reach * reach * reach * reach
    }
}

/// Far Reach's distance-dependent object velocity branch (14043f9b0).
/// Target classification, pickup/weapon catches and NPC state admission
/// remain with the real target components; callers supply a verified class.
#[derive(Clone, Copy, Debug)]
pub enum PullClass { Movable, Grenade, Ragdoll }
#[derive(Clone, Copy, Debug)]
pub struct PullImpulse {
    pub linear: Vec3,
    pub angular: Vec3,
}
#[derive(Clone, Copy, Debug)]
struct PullRange {
    minimum_distance: f32, maximum_distance: f32,
    minimum_impulse: f32, maximum_impulse: f32,
    minimum_z: f32, maximum_z: f32, vertical_impulse: f32,
    minimum_angular: f32, maximum_angular: f32,
}
pub struct PullSettings { ranges: [PullRange; 3] }
impl PullSettings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let number = |name: &str, default| match atom(fields, name) { None => Ok(default), Some(value) => value.trim_end_matches('f').parse::<f32>()
            .ok().filter(|v| v.is_finite()).ok_or_else(|| format!("invalid Far Reach {name}: {value}")) };
        // Constructor 14043e920; preserve the original misspelling radoll
        // for both angular fields in the installed declaration format.
        let angular = number("m_objectAngularVelocity", 800.0)?;
        let range = |prefix: &str, defaults: [f32; 7], angular: [f32; 2]| -> Result<PullRange, String> {
            Ok(PullRange { minimum_distance: number(&format!("m_{prefix}ImpulseMinDistance"), defaults[0])?,
                maximum_distance: number(&format!("m_{prefix}ImpulseMaxDistance"), defaults[1])?,
                minimum_impulse: number(&format!("m_{prefix}MinImpulse"), defaults[2])?,
                maximum_impulse: number(&format!("m_{prefix}MaxImpulse"), defaults[3])?,
                minimum_z: number(&format!("m_{prefix}MinImpulseZ"), defaults[4])?,
                maximum_z: number(&format!("m_{prefix}MaxImpulseZ"), defaults[5])?,
                vertical_impulse: number(&format!("m_{prefix}VerticalImpulse"), defaults[6])?,
                minimum_angular: angular[0], maximum_angular: angular[1] })
        };
        Ok(Self { ranges: [range("movable", [0.0,13.0,13.0,13.0,6.5,6.5,2.0], [angular;2])?,
            range("grenade", [0.0,13.0,7.0,14.0,3.0,6.0,2.0], [angular;2])?,
            range("ragdoll", [0.0,13.0,12.0,12.0,3.0,3.0,2.0],
                [number("m_radollMinImpulseAngular", 850.0)?, number("m_radollMaxImpulseAngular", 850.0)?])?] })
    }
    /// Target-point offset has already been transformed by the player's
    /// camera owner. The resulting direction points from body to catch point.
    pub fn impulse(&self, class: PullClass, body_position: Vec3, catch_point: Vec3, angular_axis: Vec3) -> PullImpulse {
        let r = self.ranges[match class { PullClass::Movable => 0, PullClass::Grenade => 1, PullClass::Ragdoll => 2 }];
        let displacement = catch_point - body_position;
        let squared = displacement.length_squared();
        let reciprocal = 1.0 / squared.max(f32::MIN_POSITIVE).sqrt();
        let distance = reciprocal * squared;
        let direction = displacement * reciprocal;
        let span = r.maximum_distance - r.minimum_distance;
        let shifted = distance - r.minimum_distance;
        let shifted = if shifted >= 0.0 { if span < shifted { span } else { shifted } } else { 0.0 };
        let t = shifted / span;
        let amount = r.minimum_impulse + (r.maximum_impulse - r.minimum_impulse) * t;
        let vertical = r.minimum_z + (r.maximum_z - r.minimum_z) * t;
        let mut linear = direction * amount;
        linear.z += vertical;
        if direction.z > 0.7071067 { linear.z += r.vertical_impulse; }
        let angular = r.minimum_angular + (r.maximum_angular - r.minimum_angular) * t;
        PullImpulse { linear, angular: angular_axis * -angular.to_radians() }
    }
}
