//! Ordinary particle stages, with the defaults installed by 1ef520/1dc0b0.
//! Unsupported authored producers are rejected rather than silently discarded.
use super::particle_sprite::ParticleSpriteAsset;
use super::*;
use crate::gameplay::particles::{
    emitter::EmitterDefinition,
    parameters::{EmissionRate, RateParameter, ScalarParameter, VectorParameter},
    MotionParameters,
};

pub struct SpawnVector {
    pub value: VectorParameter,
    pub worldspace: bool,
}
impl SpawnVector {
    fn new(value: [f32; 3]) -> Self {
        Self {
            value: VectorParameter::constant(value),
            worldspace: false,
        }
    }
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_value" => particles::vector_parameter(&mut self.value, scope(value)?)?,
                "m_worldspace" => self.worldspace = boolean(value)?,
                _ => return Err(format!("unknown particle spawn-vector field {name}")),
            }
        }
        Ok(())
    }
}

pub struct ParticleStageAsset {
    pub name: String,
    pub enabled: bool,
    pub worldspace: bool,
    pub enable_wind: bool,
    pub enable_velocity_inheritance: bool,
    pub emitter: EmitterDefinition,
    pub burst: bool,
    pub burst_event: String,
    pub maximum_burst_events: u32,
    pub life: ScalarParameter,
    pub infinite_life: bool,
    pub warmup: [f32; 2],
    pub spawn_only_from_stage: bool,
    pub use_event_location: bool,
    pub event_velocity: ScalarParameter,
    pub event_normal: ScalarParameter,
    pub location: SpawnVector,
    pub velocity: SpawnVector,
    pub motion: MotionParameters,
    pub source_velocity_over_cycle: VectorParameter,
    pub source_velocity_over_life: VectorParameter,
    pub constant_seed: bool,
    pub sprites: Vec<ParticleSpriteAsset>,
}
impl Default for ParticleStageAsset {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            worldspace: true,
            enable_wind: false,
            enable_velocity_inheritance: false,
            emitter: EmitterDefinition {
                cycles: 0,
                duration: [1.; 2],
                delay: [0.; 2],
                simulation_precision: 1.,
                rate: EmissionRate {
                    range: RateParameter {
                        range: [20.; 2],
                        lods: Vec::new(),
                    },
                    curve: EffectFloatCurvePair::constant(1.),
                },
            },
            burst: false,
            burst_event: String::new(),
            maximum_burst_events: 1,
            life: ScalarParameter::constant(1.),
            infinite_life: false,
            warmup: [0.; 2],
            spawn_only_from_stage: true,
            use_event_location: true,
            event_velocity: ScalarParameter::constant(0.),
            event_normal: ScalarParameter::constant(0.),
            location: SpawnVector::new([0.; 3]),
            velocity: SpawnVector::new([0., 0., 1.]),
            motion: MotionParameters::default(),
            source_velocity_over_cycle: VectorParameter::constant([0.; 3]),
            source_velocity_over_life: VectorParameter::constant([1.; 3]),
            constant_seed: false,
            sprites: vec![ParticleSpriteAsset::default()],
        }
    }
}

fn unsigned(value: &Value) -> Result<u32, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid particle unsigned integer: {e}"))
}

impl ParticleStageAsset {
    fn spawn(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_cycles" => self.emitter.cycles = unsigned(value)?,
                "m_burst" => self.burst = boolean(value)?,
                "m_burstEvent" => self.burst_event = atom(value)?.to_owned(),
                "m_maximumBurstEvents" => self.maximum_burst_events = unsigned(value)?,
                "m_rate" => {
                    let mut rate = ScalarParameter {
                        range: self.emitter.rate.range.range,
                        curve: std::mem::take(&mut self.emitter.rate.curve),
                    };
                    particles::scalar_parameter(&mut rate, scope(value)?)?;
                    self.emitter.rate.range.range = rate.range;
                    self.emitter.rate.curve = rate.curve;
                }
                "m_life" => particles::scalar_parameter(&mut self.life, scope(value)?)?,
                "m_infiniteLife" => self.infinite_life = boolean(value)?,
                "m_warmup" => bounds(&mut self.warmup, scope(value)?)?,
                "m_delay" => bounds(&mut self.emitter.delay, scope(value)?)?,
                "m_duration" => bounds(&mut self.emitter.duration, scope(value)?)?,
                "m_initialLocation" => self.location.edit(scope(value)?)?,
                "m_initialVelocity" => self.velocity.edit(scope(value)?)?,
                "m_useEventLocation" => self.use_event_location = boolean(value)?,
                "m_eventVelocity" => {
                    particles::scalar_parameter(&mut self.event_velocity, scope(value)?)?
                }
                "m_eventNormal" => {
                    particles::scalar_parameter(&mut self.event_normal, scope(value)?)?
                }
                "m_spawnStage" => {
                    for (name, value) in scope(value)? {
                        match name.as_str() {
                            "m_spawnOnlyFromStage" => self.spawn_only_from_stage = boolean(value)?,
                            _ => {
                                return Err(format!(
                                    "unresolved particle parent-stage field {name}"
                                ))
                            }
                        }
                    }
                }
                _ => return Err(format!("unresolved particle spawn producer {name}")),
            }
        }
        Ok(())
    }
}

/// Read an ordinary stage. Meshes, trails, attractors and fields need their
/// concrete producers before this reader can admit declarations using them.
pub fn stage(fields: &Fields) -> Result<ParticleStageAsset, String> {
    let mut result = ParticleStageAsset::default();
    result.motion = particles::motion_parameters(fields)?;
    for (name, value) in fields {
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_worldspace" => result.worldspace = boolean(value)?,
            "m_enableWind" => result.enable_wind = boolean(value)?,
            "m_enableVelocityInheritance" => result.enable_velocity_inheritance = boolean(value)?,
            "m_main" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_name" => result.name = atom(value)?.to_owned(),
                        _ => return Err(format!("unresolved particle stage main field {name}")),
                    }
                }
            }
            "m_spawn" => result.spawn(scope(value)?)?,
            "m_simulationPrecision" => result.emitter.simulation_precision = number(value)?,
            "m_sourceVelocityOverCycle" => {
                particles::vector_parameter(&mut result.source_velocity_over_cycle, scope(value)?)?
            }
            "m_sourceVelocityOverLife" => {
                particles::vector_parameter(&mut result.source_velocity_over_life, scope(value)?)?
            }
            "m_constantSeed" => result.constant_seed = boolean(value)?,
            "m_sprites" => {
                for (name, value) in scope(value)? {
                    if name == "num" {
                        result
                            .sprites
                            .resize_with(unsigned(value)? as usize, ParticleSpriteAsset::default);
                    } else {
                        let index = name
                            .strip_prefix("item[")
                            .and_then(|v| v.strip_suffix(']'))
                            .ok_or_else(|| format!("unknown sprite array field {name}"))?
                            .parse::<usize>()
                            .map_err(|e| format!("sprite index: {e}"))?;
                        let slot = result.sprites.get_mut(index).ok_or_else(|| {
                            format!("sprite index {index} exceeds declared count")
                        })?;
                        *slot = particle_sprite::sprite(scope(value)?)?;
                    }
                }
            }
            "m_velocityOverLife" | "m_linearAccelerationOverLife" | "m_accelerationOverLife" => {}
            "m_maxBoundingRadius" => {}
            _ => return Err(format!("unresolved particle stage producer {name}")),
        }
    }
    Ok(result)
}
