//! Ranged aiming geometry (638ae0/639120/639450). Callers supply the actual
//! camera projection and canonical world queries; this module never acquires
//! a model, samples another pose, or substitutes a camera-origin projectile.
use bevy_math::{Mat3, Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct RangedTargetDecl { pub layers: u32, pub maximum_distance: f32, pub enabled: bool }
impl RangedTargetDecl {
    ///3c3bf0/3a3640, reflection1417baa10. The installed mask augments the
    /// constructor's WORLD_LOW/LOW_HIGH/HIGH bits through the shared reader.
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        Ok(Self { layers: super::super::decls::layer_mask(fields, "m_collisionLayers", 7)?,
            maximum_distance: crate::gameplay::damage::number(fields, "m_maximumDistance", 400.0)?,
            enabled: super::boolean(fields, "m_enabledAtStart", true)? })
    }
}

/// 638570/638800 return whether the cast was clear and, when blocked, the
/// nearest distance along the segment. The filters use layers=0x1ffff and
/// requiredFlags=0x800 (ray) / 0x20000 (projectile sight-check shape).
pub trait SightQueries {
    fn ray_distance(&mut self, from: Vec3, to: Vec3) -> Option<f32>;
    fn shape_distance(&mut self, from: Vec3, to: Vec3) -> Option<f32>;
}

/// 63a520 -> ca1570/ca15b0/ca3970. The builder's default shrink flag
/// subtracts min(.01, shortest full extent*.25) before assigning .01 radius.
/// ca3970's fixed mask table orders +++,-++,+-+,--+,++-,-+-,+--,--- and
/// writes each support ID into W as3f000000|index.
pub fn sight_box(bounds_min: Vec3, bounds_max: Vec3) -> Option<[[f32; 4]; 8]> {
    let half = bounds_min.y.abs().max(bounds_max.y.abs()).max(bounds_min.z.abs().max(bounds_max.z.abs()));
    if half <= 0.0 { return None; }
    let half = half - (half * 0.5).max(0.0).min(0.01);
    Some(std::array::from_fn(|index| [
        if index & 1 == 0 { half } else { -half },
        if index & 2 == 0 { half } else { -half },
        if index & 4 == 0 { half } else { -half },
        f32::from_bits(0x3f00_0000 | index as u32),
    ]))
}

#[derive(Clone, Copy, Debug)]
pub struct Sight {
    pub eye: Vec3,
    pub camera_axis: Mat3,
    pub muzzle: Vec3,
    pub weapon_axis: Mat3,
    /// The projectile render-model bounds cached by63a520, before rotation.
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
    pub max_deviation_degrees: f32,
}

impl Sight {
    /// 638ae0: retain the real muzzle when its approach is clear; otherwise
    /// move along the camera forward line by the measured obstruction limit.
    pub fn spawn_position(&self, aim: Vec3, aimed_override: bool, force: bool, queries: &mut impl SightQueries) -> Vec3 {
        let forward = self.camera_axis.x_axis;
        let delta = aim - self.muzzle;
        let direction = delta / delta.length_squared().max(f32::MIN_POSITIVE).sqrt();
        let mut muzzle_ray_clear = false;
        let mut muzzle_shape_distance = f32::from_bits(0x7149_f2ca);
        if !aimed_override {
            let cosine = direction.dot(self.weapon_axis.x_axis);
            let angle = cosine.clamp(-1.0, 1.0).acos() * f32::from_bits(0x4265_2ee0);
            if angle < self.max_deviation_degrees || force {
                muzzle_ray_clear = queries.ray_distance(self.eye, self.muzzle).is_none();
                if muzzle_ray_clear {
                    if let Some(distance) = queries.shape_distance(self.muzzle, aim) { muzzle_shape_distance = distance; }
                    else { return self.muzzle; }
                }
            }
        }
        let mut along = forward.dot(self.muzzle - self.eye);
        let projected = self.eye + forward * along;
        let extent = self.bounds_min.y.abs().max(self.bounds_max.y.abs())
            .max(self.bounds_min.z.abs().max(self.bounds_max.z.abs()));
        let half_length = ((self.bounds_max.x - self.bounds_min.x) * 0.5).max(extent);
        let ray = queries.ray_distance(self.eye, projected + forward * self.bounds_max.x);
        let shape = queries.shape_distance(self.eye, aim);
        if ray.is_none() && shape.is_none() { return projected; }
        let far = f32::from_bits(0x7149_f2ca);
        let ray_distance = ray.unwrap_or(far);
        let shape_distance = shape.unwrap_or(far);
        if muzzle_ray_clear && ray.is_none() && self.bounds_max.x < muzzle_shape_distance
            && (aim - self.eye).length() * 0.5 < shape_distance {
            let projected_hit = forward.dot(self.muzzle + direction * muzzle_shape_distance - self.eye).abs();
            if shape_distance - half_length < projected_hit { return self.muzzle; }
        }
        along = along.min(ray_distance.min(shape_distance) - self.bounds_max.x);
        self.eye + forward * along
    }
}

/// 639450 consumes all three samples even when dispersion is zero. The
/// caller owns the shared world seed used by8ac040, rather than a shot RNG.
pub fn dispersed_screen(ndc: Vec2, viewport: Vec2, minimum: f32, maximum: f32,
    ratio: f32, extra_chance: f32, seed: &mut u32) -> Vec2 {
    dispersed_screen_samples(ndc, viewport, minimum, maximum, ratio, extra_chance,
        std::array::from_fn(|_| crate::random::lcg_unit(seed)))
}
pub fn dispersed_screen_samples(ndc: Vec2, viewport: Vec2, minimum: f32, maximum: f32,
    ratio: f32, extra_chance: f32, samples: [f32; 3]) -> Vec2 {
    let angle = samples[0] * (2.0 * std::f32::consts::PI) - std::f32::consts::PI;
    let amount = (maximum - minimum) * ratio + minimum;
    let mut radius = samples[1] * amount * (viewport.y / 720.0);
    if samples[2] * 100.0 < extra_chance { radius = maximum.max(amount); }
    Vec2::new(angle.cos() * radius + (ndc.x + 1.0) * viewport.x * 0.5,
        angle.sin() * radius + (1.0 - ndc.y) * viewport.y * 0.5)
}

/// 0f1940/0f19a0's acceleration/deceleration integrator used by63c0b0.
pub fn smooth_ratio(value: &mut f32, velocity: &mut f32, target: f32, smoothing: [f32; 3], dt: f32) {
    if dt == 0.0 { return; }
    let delta = target - *value;
    let toward_velocity = if delta * *velocity < 0.0 { -*velocity } else { *velocity };
    let [acceleration, deceleration, maximum_speed] = smoothing;
    let accelerating = (toward_velocity * *velocity / deceleration).abs() <= delta.abs();
    let change = if accelerating { acceleration } else { -deceleration } * dt;
    if delta < 0.0 {
        *velocity = (*velocity - change).max((-maximum_speed).max(delta / dt)).min(0.0);
    } else {
        *velocity = (*velocity + change).min(maximum_speed.min(delta / dt)).max(0.0);
    }
    *value += *velocity * dt;
}

/// 639120 preserves the original aim distance after camera unprojection.
pub fn dispersed_aim(eye: Vec3, original: Vec3, unprojected: Vec3) -> Vec3 {
    let direction = unprojected - eye;
    eye + direction / direction.length_squared().max(f32::MIN_POSITIVE).sqrt() * (original - eye).length()
}
