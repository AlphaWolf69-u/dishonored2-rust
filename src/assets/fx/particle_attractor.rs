//! arkParticleSystemDescStructures::arkAttractorDesc (0xd88 bytes; offsets
//! from its reflection, defaults from 1cd210 and its position_t 1cd520).
use super::particle_stage::SpawnVector;
use super::*;
use crate::gameplay::particles::parameters::ScalarParameter;

pub struct ParticleAttractor {
    /// position_t +0x000..+0x81f.
    pub position: SpawnVector,
    pub position_over_cycle: SpawnVector,
    pub velocity: SpawnVector,
    /// +0x348. 1cde40 samples m_velocity (+0x230) twice and never reads
    /// this field; see docs/original-defects.md.
    pub velocity_over_cycle: SpawnVector,
    pub random_offset: SpawnVector,
    pub random_offset_over_cycle: SpawnVector,
    pub random_radius: ScalarParameter,
    pub random_radius_over_cycle: EffectFloatCurvePair,
    /// arkDelay +0x818, serialized maximum then minimum.
    pub random_period: [f32; 2],
    /// damping_t +0x820.
    pub tolerance_distance: f32,
    pub max_strength_factor: f32,
    pub deceleration_factor: f32,
    pub cut_off_distance: ScalarParameter,
    pub cut_off_distance_over_cycle: EffectFloatCurvePair,
    pub target_radius: ScalarParameter,
    pub target_radius_over_cycle: EffectFloatCurvePair,
    pub strength: ScalarParameter,
    pub strength_over_cycle: EffectFloatCurvePair,
    pub strength_over_life: EffectFloatCurvePair,
}

impl Default for ParticleAttractor {
    fn default() -> Self {
        Self {
            position: SpawnVector::new([0.; 3]),
            position_over_cycle: SpawnVector::new([0.; 3]),
            velocity: SpawnVector::new([0.; 3]),
            velocity_over_cycle: SpawnVector::new([0.; 3]),
            random_offset: SpawnVector::new([0.; 3]),
            random_offset_over_cycle: SpawnVector::new([0.; 3]),
            random_radius: ScalarParameter::constant(0.),
            random_radius_over_cycle: EffectFloatCurvePair::constant(1.),
            random_period: [0.; 2],
            tolerance_distance: f32::from_bits(0x3c23_d70a),
            max_strength_factor: 2.,
            deceleration_factor: 4.,
            cut_off_distance: ScalarParameter::constant(1.),
            cut_off_distance_over_cycle: EffectFloatCurvePair::constant(1.),
            target_radius: ScalarParameter::constant(0.),
            target_radius_over_cycle: EffectFloatCurvePair::constant(1.),
            strength: ScalarParameter::constant(1.),
            strength_over_cycle: EffectFloatCurvePair::constant(1.),
            strength_over_life: EffectFloatCurvePair::constant(1.),
        }
    }
}

pub fn attractor(fields: &Fields) -> Result<ParticleAttractor, String> {
    let mut result = ParticleAttractor::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_position" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_position" => result.position.edit(scope(value)?)?,
                        "m_positionOverCycle" => result.position_over_cycle.edit(scope(value)?)?,
                        "m_velocity" => result.velocity.edit(scope(value)?)?,
                        "m_velocityOverCycle" => result.velocity_over_cycle.edit(scope(value)?)?,
                        "m_randomOffset" => result.random_offset.edit(scope(value)?)?,
                        "m_randomOffsetOverCycle" => {
                            result.random_offset_over_cycle.edit(scope(value)?)?
                        }
                        "m_randomRadius" => {
                            particles::scalar_parameter(&mut result.random_radius, scope(value)?)?
                        }
                        "m_randomRadiusOverCycle" => {
                            curves(&mut result.random_radius_over_cycle, scope(value)?)?
                        }
                        "m_randomPeriod" => bounds(&mut result.random_period, scope(value)?)?,
                        _ => return Err(format!("unknown particle attractor position field {name}")),
                    }
                }
            }
            "m_damping" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_toleranceDistance" => result.tolerance_distance = number(value)?,
                        "m_maxStrengthFactor" => result.max_strength_factor = number(value)?,
                        "m_decelerationFactor" => result.deceleration_factor = number(value)?,
                        _ => return Err(format!("unknown particle attractor damping field {name}")),
                    }
                }
            }
            "m_cutOffDistance" => {
                particles::scalar_parameter(&mut result.cut_off_distance, scope(value)?)?
            }
            "m_cutOffDistanceOverCycle" => {
                curves(&mut result.cut_off_distance_over_cycle, scope(value)?)?
            }
            "m_targetRadius" => {
                particles::scalar_parameter(&mut result.target_radius, scope(value)?)?
            }
            "m_targetRadiusOverCycle" => {
                curves(&mut result.target_radius_over_cycle, scope(value)?)?
            }
            "m_strength" => particles::scalar_parameter(&mut result.strength, scope(value)?)?,
            "m_strengthOverCycle" => curves(&mut result.strength_over_cycle, scope(value)?)?,
            "m_strengthOverLife" => curves(&mut result.strength_over_life, scope(value)?)?,
            _ => return Err(format!("unknown particle attractor field {name}")),
        }
    }
    Ok(result)
}
