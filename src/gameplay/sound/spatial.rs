//! Listener-relative rays and authored attenuation (1410cb7e0 / 1410cd110).
use crate::assets::wwise::Attenuation;
use bevy_math::{Mat3, Vec3};
use super::mix::{conversion_value, power10};

/// 1400c1830: game Z-up vectors to Wwise's X-right/Y-up/Z-forward frame.
/// Host spatial calculations retain game coordinates for both ends of a ray;
/// use this conversion when comparing the actual middleware submissions.
pub fn wwise_vector(vector: Vec3) -> Vec3 { Vec3::new(-vector.y, vector.z, vector.x) }

#[derive(Clone, Copy, Debug)]
pub struct Listener {
    pub position: Vec3,
    /// Columns are right, up and forward in world space.
    pub axis: Mat3,
    pub distance_scale: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub distance: f32,
    pub azimuth: f32,
    pub elevation: f32,
    pub cone_angle: f32,
}

impl Listener {
    /// 1410cb7e0. Orientation affects angles; emitter and listener scaling
    /// affect only distance. Coincident positions retain zero angles.
    pub fn ray(self, position: Vec3, front: Vec3, emitter_scale: f32) -> Ray {
        let relative = position - self.position;
        let length = (relative.y * relative.y + relative.x * relative.x + relative.z * relative.z).sqrt();
        let mut ray = Ray { distance: length / (emitter_scale * self.distance_scale),
            azimuth: 0.0, elevation: 0.0, cone_angle: 0.0 };
        if length <= 0.0 { return ray; }
        let dot = |axis: Vec3| relative.y * axis.y + relative.x * axis.x + relative.z * axis.z;
        let right = dot(self.axis.x_axis);
        let up = dot(self.axis.y_axis);
        let forward = dot(self.axis.z_axis);
        if forward == 0.0 && right == 0.0 {
            ray.elevation = if up == 0.0 { 0.0 } else if up > 0.0 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 };
        } else {
            ray.azimuth = if forward == 0.0 {
                if right > 0.0 { std::f32::consts::FRAC_PI_2 } else { -std::f32::consts::FRAC_PI_2 }
            } else { right.atan2(forward) };
            let elevation = up / length;
            // Retain the executable's ordered comparisons, including its NaN
            // branches, rather than using a clamp with different semantics.
            let elevation = if elevation >= -1.0 { if elevation > 1.0 { 1.0 } else { elevation } } else { -1.0 };
            ray.elevation = elevation.asin();
        }
        let cosine = -(relative.y * front.y + relative.x * front.x + relative.z * front.z) / length;
        let cosine = if cosine < 1.0 { if cosine <= -1.0 { -1.0 } else { cosine } } else { 1.0 };
        ray.cone_angle = cosine.acos();
        ray
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Gains {
    pub dry: f32,
    pub user_auxiliary: f32,
    pub game_auxiliary: f32,
    /// Zero inside the cone, one outside, linear through the intervening arc.
    pub cone_ratio: f32,
}

pub fn curve(attenuation: &Attenuation, mapping: usize, input: f32) -> Option<f32> {
    let index = usize::try_from(*attenuation.curve_indices.get(mapping)?).ok()?;
    let curve = attenuation.curves.get(index)?;
    conversion_value(&curve.points, curve.scaling, input)
}

/// 1410cd110 / 1410cb2c0. A cone attenuates only the dry send. Authored
/// user/game auxiliary sends retain their independently selected curves.
pub fn gains(attenuation: Option<&Attenuation>, ray: Ray, outside_volume_db: f32) -> Gains {
    let mut result = Gains { dry: 1.0, user_auxiliary: 1.0, game_auxiliary: 1.0, cone_ratio: 1.0 };
    let Some(attenuation) = attenuation else { return result };
    result.dry = curve(attenuation, 0, ray.distance).unwrap_or(1.0);
    result.user_auxiliary = curve(attenuation, 1, ray.distance).unwrap_or(1.0);
    result.game_auxiliary = curve(attenuation, 2, ray.distance).unwrap_or(1.0);
    if let Some(cone) = &attenuation.cone {
        result.cone_ratio = if ray.cone_angle <= cone.inside_half_angle { 0.0 }
            else if cone.outside_half_angle <= ray.cone_angle { 1.0 }
            else { (ray.cone_angle - cone.inside_half_angle) / (cone.outside_half_angle - cone.inside_half_angle) };
        result.dry *= power10(result.cone_ratio * outside_volume_db * 0.05);
    }
    result
}

/// 1410cb3b0 for a single direct listener ray without obstruction/occlusion.
/// Distance and cone filters combine by maximum, then with the voice filters.
pub fn filters(attenuation: Option<&Attenuation>, ray: Ray, cone_ratio: f32,
    outside_filters: [f32; 2], voice_filters: [f32; 2]) -> [f32; 2]
{
    std::array::from_fn(|index| {
        let distance = attenuation.and_then(|attenuation| curve(attenuation, 3 + index, ray.distance)).unwrap_or(0.0);
        let cone = if attenuation.is_some_and(|attenuation| attenuation.cone.is_some()) {
            cone_ratio * outside_filters[index]
        } else { 0.0 };
        voice_filters[index].max(distance.max(cone))
    })
}

/// The max-distance lookup is shared by 1410dd2e0 and 141105130.
pub fn max_distance(attenuation: &Attenuation) -> Option<f32> {
    let index = usize::try_from(attenuation.curve_indices[0]).ok()?;
    Some(attenuation.curves.get(index)?.points.last()?.input)
}

/// 141105130: without a dry-volume curve the supplied value is unchanged.
pub fn distance_from_percentage(attenuation: Option<&Attenuation>, percentage: f32) -> f32 {
    attenuation.and_then(max_distance).map_or(percentage, |maximum| percentage * maximum * 0.01)
}

/// 141102990 / 141105a30: the distance priority offset scales from zero to
/// its full value at max distance. Priority is clamped before integer use.
pub fn distance_priority(base: f32, offset: f32, distance: f32, maximum: Option<f32>) -> f32 {
    let Some(maximum) = maximum else { return base };
    let offset = if offset == 0.0 { 0.0 } else if distance < maximum && maximum > 0.0 { distance / maximum * offset } else { offset };
    let value = base + offset;
    if value >= 0.0 { if value > 100.0 { 100.0 } else { value } } else { 0.0 }
}

/// 141139060: spread uses the horizontal projection of scaled distance.
pub fn spread(attenuation: Option<&Attenuation>, ray: Ray) -> f32 {
    attenuation.and_then(|attenuation| curve(attenuation, 5, ray.elevation.cos() * ray.distance)).unwrap_or(0.0)
}

/// 141103420 / 1410cbbb0: focus also uses projected horizontal distance.
pub fn focus(attenuation: Option<&Attenuation>, ray: Ray) -> f32 {
    attenuation.and_then(|attenuation| curve(attenuation, 6, ray.elevation.cos() * ray.distance)).unwrap_or(0.0)
}
