//! Swimming surface inputs from 888ce0 and the five-point query in 88b360.
use crate::entities::Value;
use bevy_math::{Mat3, Vec3};

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
