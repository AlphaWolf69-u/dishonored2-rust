//! Wwise environment tables and per-listener control levels.
use crate::assets::wwise::EnvironmentSettings;
use super::{mix::conversion_value, spatial::Gains};

#[derive(Clone, Copy, Debug, Default)]
pub struct Levels {
    pub obstruction: f32,
    pub occlusion: f32,
}
impl Levels {
    /// 1400c5ba0 combines the listener's existing levels (1400c1990) with
    /// the object's scaled propagation results. The source only clamps the
    /// selected object value above one; it does not clamp the listener floor.
    pub fn for_listener(self, minimum: Self, scale: f32) -> Self {
        fn combine(minimum: f32, value: f32) -> f32 {
            if minimum <= value {
                if 1.0 < value { 1.0 } else { value }
            } else { minimum }
        }
        Self {
            obstruction: combine(minimum.obstruction, self.obstruction * scale),
            occlusion: combine(minimum.occlusion, self.occlusion * scale),
        }
    }
}

/// The object's eight-listener byte arrays at +68/+70 (1410ceaa0).
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlLevels {
    pub obstruction: u8,
    pub occlusion: u8,
}
impl From<Levels> for ControlLevels {
    fn from(levels: Levels) -> Self {
        fn quantize(value: f32) -> u8 {
            let value = value * 255.0;
            // CVTTSS2SI's invalid conversion yields i64::MIN, whose low byte
            // is zero. Valid conversions narrow to a byte without clamping.
            if !value.is_finite() || value >= i64::MAX as f32 || value < i64::MIN as f32 {
                0
            } else { value as i64 as u8 }
        }
        Self { obstruction: quantize(levels.obstruction), occlusion: quantize(levels.occlusion) }
    }
}
impl ControlLevels {
    pub fn obstruction(self) -> f32 { f32::from(self.obstruction) * f32::from_bits(0x3b808081) }
    pub fn occlusion(self) -> f32 { f32::from(self.occlusion) * f32::from_bits(0x3b808081) }
}

fn value(settings: &EnvironmentSettings, group: usize, property: usize, input: f32) -> Option<f32> {
    let curve = &settings.curves[group][property];
    if !curve.enabled { return None; }
    conversion_value(&curve.points, curve.scaling, input)
}

/// 1410ccd30: occlusion attenuates dry and both auxiliary paths;
/// obstruction attenuates only the dry path. Voice/output-bus gains are
/// applied separately by the caller, as in the original mixing stages.
pub fn gains(settings: &EnvironmentSettings, levels: ControlLevels, mut gains: Gains) -> Gains {
    let occlusion = value(settings, 1, 0, levels.occlusion() * 100.0).unwrap_or(1.0);
    let obstruction = value(settings, 0, 0, levels.obstruction() * 100.0).unwrap_or(1.0);
    gains.dry = obstruction * occlusion * gains.dry;
    gains.user_auxiliary = occlusion * gains.user_auxiliary;
    gains.game_auxiliary = occlusion * gains.game_auxiliary;
    gains
}

/// The per-ray occlusion portion of 1410cb3b0. Obstruction filtering is a
/// separate output-path stage; it must not replace these distance/cone maxima.
pub fn occlusion_filters(settings: &EnvironmentSettings, levels: ControlLevels, distance_cone: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|index| {
        let occlusion = value(settings, 1, index + 1, levels.occlusion() * 100.0).unwrap_or(0.0);
        if occlusion > distance_cone[index] { occlusion } else { distance_cone[index] }
    })
}

/// 1410cb3b0 evaluates obstruction on the minimum level of all admitted
/// rays, independently from the occlusion/distance/cone filter reduction.
pub fn obstruction_filters(settings: &EnvironmentSettings, minimum: f32) -> [Option<f32>; 2] {
    std::array::from_fn(|index| value(settings, 0, index + 1, minimum * 100.0))
}

#[derive(Clone, Copy, Debug)]
pub struct FilterRay {
    pub levels: ControlLevels,
    pub distance: [f32; 2],
    pub cone: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct FilterTargets {
    pub voice: [f32; 2],
    /// None preserves the existing obstruction output-path filter target.
    pub output: [Option<f32>; 2],
}

/// 1410cb3b0 reduces admitted listener rays by minimum after taking each
/// ray's occlusion/distance/cone maximum. Voice filters form the final floor;
/// obstruction remains on its separate output path. Caller supplies only rays
/// admitted by the output listener mask (virtual40 in the original function).
pub fn filters(settings: &EnvironmentSettings, rays: &[FilterRay], voice: [f32; 2]) -> Result<FilterTargets, String> {
    if rays.is_empty() { return Err("environment filter reduction requires an admitted ray".into()); }
    let mut minimum = [100.0; 2];
    let mut obstruction = 1.0;
    for ray in rays {
        let per_ray = occlusion_filters(settings, ray.levels, ray.distance);
        for index in 0..2 {
            let selected = if per_ray[index] > ray.cone[index] { per_ray[index] } else { ray.cone[index] };
            if selected <= minimum[index] { minimum[index] = selected; }
        }
        if ray.levels.obstruction() < obstruction { obstruction = ray.levels.obstruction(); }
    }
    Ok(FilterTargets {
        voice: std::array::from_fn(|index| if voice[index] < minimum[index] { minimum[index] } else { voice[index] }),
        output: obstruction_filters(settings, obstruction),
    })
}
