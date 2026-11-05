//! Swimming surface inputs from 888ce0 and the five-point query in 88b360.
use crate::entities::Value;
use bevy_math::{Mat3, Quat, Vec3};

/// arkDeclPlayerStateSwimming's navigation and vertical controls. The normal
/// and sprinting declarations are separate states in PLAYER_MODE_SWIM.
#[derive(Clone, Debug, PartialEq)]
pub struct SwimmingStateDecl {
    pub id: super::states::StateId,
    pub navigation: super::states::NavigationDecl,
    pub up_action: i32,
    pub down_action: i32,
    pub up_speed: f32,
    pub down_speed: f32,
    pub diving_speed_ratio: f32,
    pub diving_deceleration: f32,
    pub minimum_stick: f32,
    pub minimum_speed: f32,
}

impl SwimmingStateDecl {
    /// 1408460d0: rotate the navigation request about the player's left axis
    /// by the view pitch, then apply the authored vertical actions. The game
    /// passes negative pitch to idRotation, whose matrix uses the opposite
    /// sign convention from glam's axis-angle rotation.
    /// A retained dive impulse takes precedence over ordinary input.
    pub fn velocity(&self, navigation: Vec3, left: Vec3, pitch: f32, up: bool, down: bool, dive: Option<Vec3>) -> Vec3 {
        if let Some(dive) = dive {
            return dive;
        }
        let mut velocity = Quat::from_axis_angle(left, pitch) * navigation;
        if up { velocity.z += self.up_speed; }
        if down { velocity.z -= self.down_speed; }
        velocity
    }
}

/// Completed contact and wave-height data for the player water controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimmingContact {
    pub plane: f32,
    pub head: f32,
    pub body: f32,
    pub ocean_level: Option<f32>,
    pub samples: Option<[[f32; 4]; 5]>,
    pub current: Vec3,
}

/// The four-value height and sixteen-value normal filters constructed in
/// 140888af0 and updated by 1404e3580/1404e37e0 and 140888d20/140889000.
pub struct WaveFilters {
    heights: [f32; 4],
    normals: [Vec3; 16],
    height_index: usize,
    normal_index: usize,
    height_timer: f32,
    normal_timer: f32,
    height_sum: f32,
    normal_sum: Vec3,
    height_count: u32,
}
impl Default for WaveFilters {
    fn default() -> Self {
        let interval = f32::from_bits(0x3d088889);
        Self { heights: [0.0; 4], normals: [Vec3::Z; 16], height_index: 3, normal_index: 15,
            height_timer: interval, normal_timer: interval, height_sum: 0.0, normal_sum: Vec3::Z, height_count: 1 }
    }
}
impl WaveFilters {
    pub fn update(&mut self, height: f32, normal: Vec3, dt: f32) -> (f32, Vec3) {
        let interval = f32::from_bits(0x3d088889);
        self.height_sum += height;
        self.height_count += 1;
        self.height_timer -= dt;
        if self.height_timer <= 0.0 {
            self.height_index = (self.height_index + 1) & 3;
            self.heights[self.height_index] = self.height_sum / self.height_count as f32;
            self.height_timer += interval;
            while self.height_timer <= 0.0 {
                self.height_timer += interval;
                self.height_index = (self.height_index + 1) & 3;
                self.heights[self.height_index] = height;
            }
            self.height_sum = height;
            self.height_count = 1;
        }
        let sum: f32 = self.heights.iter().sum();
        let next = (self.height_index + 1) & 3;
        let before = sum * 0.25;
        let after = (sum - self.heights[next] + self.height_sum / self.height_count as f32) * 0.25;
        let height = before * (self.height_timer / interval) + after * (1.0 - self.height_timer / interval);

        self.normal_sum += normal;
        self.normal_timer -= dt;
        if self.normal_timer <= 0.0 {
            self.normal_index = (self.normal_index + 1) & 15;
            self.normals[self.normal_index] = self.normal_sum.normalize_or_zero();
            self.normal_timer += interval;
            while self.normal_timer <= 0.0 {
                self.normal_timer += interval;
                self.normal_index = (self.normal_index + 1) & 15;
                self.normals[self.normal_index] = normal;
            }
            self.normal_sum = normal;
        }
        let sum: Vec3 = self.normals.iter().copied().sum();
        let next = (self.normal_index + 1) & 15;
        let before = sum.normalize_or_zero();
        let after = (sum - self.normals[next] + self.normal_sum.normalize_or_zero()).normalize_or_zero();
        let normal = (before * (self.normal_timer / interval) + after * (1.0 - self.normal_timer / interval)).normalize_or_zero();
        (height, normal)
    }
}

/// arkCpntPlayerBreath 1404c06f0/1404c07c0/1404c0cf0. Drowning damage
/// remains owned by the health component rather than being simulated here.
#[derive(Clone, Copy, Debug)]
pub struct Breath {
    pub remaining: f32,
    pub regen_timer: f32,
    pub regen: f32,
    pub regen_delay: f32,
}
impl Breath {
    pub fn new(regen: f32, regen_delay: f32) -> Self {
        Self { remaining: 100.0, regen_timer: 0.0, regen, regen_delay }
    }
    pub fn reset_for_water_entry(&mut self) { self.remaining = 100.0; }
    pub fn consume(&mut self, amount: f32) {
        self.remaining = (self.remaining - amount).max(0.0);
        self.regen_timer = 0.0;
    }
    pub fn update(&mut self, dt: f32) {
        if self.regen_timer < self.regen_delay {
            self.regen_timer += dt;
        } else {
            self.remaining = (self.remaining + dt * self.regen).min(100.0);
        }
    }
}

/// 14088a9e0 keeps the water current's direction and speed separately.
#[derive(Clone, Copy, Debug, Default)]
pub struct WaterCurrent {
    direction: Vec3,
    speed: f32,
}
impl WaterCurrent {
    pub fn update(&mut self, target: Vec3, dt: f32, settings: &SwimmingSettings) -> Vec3 {
        let target_speed = target.length();
        let mut direction = target / target_speed.max(f32::MIN_POSITIVE);
        if self.speed <= f32::EPSILON {
            self.speed = 0.0;
            self.direction = direction;
        }
        if target_speed <= f32::EPSILON { direction = self.direction; }
        if self.speed > f32::EPSILON || target_speed > f32::EPSILON {
            let limit = dt * settings.currents_speed_blend;
            self.speed += (target_speed - self.speed).clamp(-limit, limit);
            let angle = self.direction.dot(direction).clamp(-1.0, 1.0).acos();
            let step = dt * settings.currents_rotation_blend.to_radians();
            if angle <= step || self.direction.distance_squared(direction) <= f32::EPSILON * f32::EPSILON {
                self.direction = direction;
            } else {
                let axis = self.direction.cross(direction).normalize_or_zero();
                if axis != Vec3::ZERO {
                    self.direction = (Quat::from_axis_angle(axis, step) * self.direction).normalize_or_zero();
                }
            }
        }
        self.direction * self.speed
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimmingSettings {
    pub minimum_depth: f32,
    pub distance_above_surface: f32,
    pub sampling_radius: f32,
    pub normal_weight: f32,
    pub wave_shift: f32,
    pub maximum_wave_depth: f32,
    pub currents_speed_blend: f32,
    pub currents_rotation_blend: f32,
}
impl Default for SwimmingSettings {
    fn default() -> Self {
        Self {
            minimum_depth: 0.5,
            distance_above_surface: f32::from_bits(0x3e19999a),
            sampling_radius: 1.,
            normal_weight: 0.75,
            wave_shift: f32::from_bits(0x3e99999a),
            maximum_wave_depth: 1.5,
            currents_speed_blend: 5.,
            currents_rotation_blend: 45.,
        }
    }
}
impl SwimmingSettings {
    pub fn read(controller: &[(String, Value)]) -> Result<Self, String> {
        let mut result = Self::default();
        let Some((_, Value::Scope(fields))) =
            controller.iter().find(|(k, _)| k == "m_swimmingSettings")
        else {
            return Ok(result);
        };
        for (name, value) in fields {
            let target = match name.as_str() {
                "m_swimMinWaterDepth" => &mut result.minimum_depth,
                "m_swimDistanceAboveSurface" => &mut result.distance_above_surface,
                "m_waveSamplingRadius" => &mut result.sampling_radius,
                "m_waveNormalWeight" => &mut result.normal_weight,
                "m_waveShiftZ" => &mut result.wave_shift,
                "m_waveMaxDepth" => &mut result.maximum_wave_depth,
                "m_currentsSpeedBlend" => &mut result.currents_speed_blend,
                "m_currentsRotationBlend" => &mut result.currents_rotation_blend,
                _ => return Err(format!("unknown swimming setting {name}")),
            };
            let Value::Atom(value) = value else {
                return Err(format!("non-scalar swimming setting {name}"));
            };
            *target = value.parse().map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(result)
    }
    pub fn wave_points(&self, head: Vec3, body: Vec3, axis: Mat3) -> [[f32; 2]; 5] {
        let offset = |direction: Vec3| {
            (body + direction * self.sampling_radius)
                .truncate()
                .to_array()
        };
        [
            head.truncate().to_array(),
            offset(axis.x_axis),
            offset(-axis.x_axis),
            offset(axis.y_axis),
            offset(-axis.y_axis),
        ]
    }

    /// 14088b360: derive the unsmoothed body wave displacement and normal
    /// from the same five completed samples used for the head water line.
    /// `reference_height` is the player's authored body water reference.
    pub fn wave_target(&self, plane: f32, ocean_level: f32, reference_height: f32, samples: Option<&[[f32; 4]; 5]>) -> (f32, Vec3) {
        let mut offset = 0.0;
        let mut normal = Vec3::Z;
        if let Some(samples) = samples {
            let mut maximum = samples[0][0];
            let first = Vec3::new(samples[0][1], samples[0][2], samples[0][3]);
            let mut sum = if first == Vec3::ZERO { Vec3::Z } else { first };
            for sample in &samples[1..] {
                maximum = if maximum > sample[0] { maximum } else { sample[0] };
                sum += Vec3::new(sample[1], sample[2], sample[3]);
            }
            if !maximum.is_nan() && !sum.is_nan() {
                offset = maximum - (ocean_level - self.wave_shift);
                normal = sum / sum.length_squared().max(f32::MIN_POSITIVE).sqrt();
            }
        }
        let raw_depth = (plane - reference_height) / self.maximum_wave_depth;
        let depth = if raw_depth < 0.0 || raw_depth.is_nan() { 0.0 } else { raw_depth.min(1.0) };
        let strength = 1.0 - depth;
        let scaled = Vec3::new(normal.x * strength * self.normal_weight, normal.y * strength * self.normal_weight, normal.z);
        let normal = scaled / scaled.length_squared().max(f32::MIN_POSITIVE).sqrt();
        (offset * strength, normal)
    }
}

/// MAXSS chooses its second operand for equal or unordered values.
pub fn highest_wave(samples: &[[f32; 4]; 5]) -> f32 {
    let mut height = samples[0][0];
    for sample in &samples[1..] {
        height = if height > sample[0] {
            height
        } else {
            sample[0]
        };
    }
    height
}
