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

/// 141575af0 /141576580: named event, frequency and optional life threshold.
pub struct ParticleEvent {
    pub name: String,
    pub frequency: f32,
    pub life: Option<f32>,
}
impl Default for ParticleEvent {
    fn default() -> Self {
        Self {
            name: String::new(),
            frequency: 1.,
            life: None,
        }
    }
}
impl ParticleEvent {
    fn edit(&mut self, fields: &Fields, life_event: bool) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_name" => self.name = atom(value)?.to_owned(),
                "m_frequency" => self.frequency = number(value)?,
                "m_life" if life_event => self.life = Some(number(value)?),
                _ => return Err(format!("unknown particle event field {name}")),
            }
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct ParticleEvents {
    pub birth: ParticleEvent,
    pub death: ParticleEvent,
    pub life: Vec<ParticleEvent>,
}
impl ParticleEvents {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_birth" => self.birth.edit(scope(value)?, false)?,
                "m_death" => self.death.edit(scope(value)?, false)?,
                "m_life" => {
                    for (name, value) in scope(value)? {
                        if name == "num" {
                            // 1f66a0 initializes life-list elements at normalized age zero.
                            self.life
                                .resize_with(unsigned(value)? as usize, || ParticleEvent {
                                    life: Some(0.),
                                    ..Default::default()
                                });
                        } else {
                            let index = name
                                .strip_prefix("item[")
                                .and_then(|s| s.strip_suffix(']'))
                                .ok_or_else(|| {
                                    format!("unknown particle life-event array field {name}")
                                })?
                                .parse::<usize>()
                                .map_err(|e| format!("particle life-event index: {e}"))?;
                            self.life
                                .get_mut(index)
                                .ok_or("particle life-event index exceeds count")?
                                .edit(scope(value)?, true)?;
                        }
                    }
                }
                _ => return Err(format!("unknown particle events field {name}")),
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
    pub spawn_mesh: crate::gameplay::particles::mesh::SpawnMesh,
    pub velocity_mesh: crate::gameplay::particles::mesh::VelocityMesh,
    pub velocity_orientation: crate::gameplay::particles::spawn::SpawnOrientation,
    pub velocity_spray: ScalarParameter,
    pub motion: MotionParameters,
    pub source_velocity_over_cycle: VectorParameter,
    pub source_velocity_over_life: VectorParameter,
    pub constant_seed: bool,
    pub sprites: Vec<ParticleSpriteAsset>,
    pub meshes: Vec<super::particle_mesh::ParticleMeshAsset>,
    pub events: ParticleEvents,
    pub spawn_fields: Vec<super::particle_field::ParticleField>,
    pub vector_fields: Vec<super::particle_field::ParticleField>,
    pub acceleration_fields: Vec<super::particle_field::ParticleField>,
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
            spawn_mesh: Default::default(),
            velocity_mesh: Default::default(),
            velocity_orientation: Default::default(),
            velocity_spray: ScalarParameter::constant(0.),
            motion: MotionParameters::default(),
            source_velocity_over_cycle: VectorParameter::constant([0.; 3]),
            source_velocity_over_life: VectorParameter::constant([1.; 3]),
            constant_seed: false,
            sprites: vec![ParticleSpriteAsset::default()],
            meshes: Vec::new(),
            events: ParticleEvents::default(),
            spawn_fields: Vec::new(),
            vector_fields: Vec::new(),
            acceleration_fields: Vec::new(),
        }
    }
}

fn unsigned(value: &Value) -> Result<u32, String> {
    atom(value)?
        .parse()
        .map_err(|e| format!("invalid particle unsigned integer: {e}"))
}

impl ParticleStageAsset {
    /// 1f82c0's allocation estimate. The ordinary stage owner (1efcc0) caps
    /// it at 8192 slots, shared by sprite and mesh renderers.
    pub fn estimated_capacity(&self) -> Result<u64, String> {
        use crate::float::{add, divide, multiply, truncate_i64};
        let ceil = |value: f32| truncate_i64(value.ceil()) as u64;
        if !self.infinite_life {
            let life = multiply(
                self.life.curve.maximum_key_value(0.)?,
                self.life.range[0].max(self.life.range[1]),
            );
            if !self.burst {
                let count = multiply(self.emitter.rate.maximum_rate()?, life).ceil();
                return Ok(ceil(multiply(add(count, 1.), f32::from_bits(0x3f86_6666))));
            }
            let duration = self.emitter.duration[0].min(self.emitter.duration[1]);
            let mut cycles = divide(life, duration).ceil();
            if self.emitter.cycles != 0 && self.emitter.cycles as f32 <= cycles {
                cycles = self.emitter.cycles as f32;
            }
            let count = truncate_i64(multiply(self.emitter.rate.maximum_burst(), cycles)) as u64;
            return Ok(if self.burst_event.is_empty() {
                count
            } else {
                (count as u32).wrapping_mul(self.maximum_burst_events) as u64
            });
        }
        let per_cycle = if self.burst {
            ceil(self.emitter.rate.maximum_burst())
        } else {
            ceil(multiply(
                self.emitter.duration[0].max(self.emitter.duration[1]),
                self.emitter.rate.maximum_rate()?,
            ))
        };
        let count = (per_cycle as u32).wrapping_mul(self.emitter.cycles);
        Ok(if self.burst_event.is_empty() {
            count
        } else {
            count.wrapping_mul(self.maximum_burst_events)
        } as u64)
    }

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
                // StandardCycles rate_lod_t inherits MinMaxFloat; 1f6800
                // initializes both endpoints to20, independent of base edits.
                "m_rate_lod" => edit_list(
                    &mut self.emitter.rate.range.lods, scope(value)?, [20.; 2],
                    |slot, value| bounds(slot, scope(value)?),
                )?,
                "m_life" => particles::scalar_parameter(&mut self.life, scope(value)?)?,
                "m_infiniteLife" => self.infinite_life = boolean(value)?,
                "m_warmup" => bounds(&mut self.warmup, scope(value)?)?,
                "m_delay" => bounds(&mut self.emitter.delay, scope(value)?)?,
                "m_duration" => bounds(&mut self.emitter.duration, scope(value)?)?,
                "m_initialLocation" => self.location.edit(scope(value)?)?,
                "m_initialVelocity" => self.velocity.edit(scope(value)?)?,
                "m_velocityVectorFields" => descriptor_list(
                    &mut self.spawn_fields,
                    scope(value)?,
                    super::particle_field::field,
                )?,
                "m_spawnMesh" => particle_emission_mesh::edit(&mut self.spawn_mesh, scope(value)?)?,
                "m_velocityMesh" => particle_emission_mesh::velocity_edit(&mut self.velocity_mesh, scope(value)?)?,
                "m_velocitySprayCone" => {
                    particles::scalar_parameter(&mut self.velocity_spray, scope(value)?)?
                }
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
            "m_events" => result.events.edit(scope(value)?)?,
            "m_vectorFields" => descriptor_list(
                &mut result.vector_fields,
                scope(value)?,
                super::particle_field::field,
            )?,
            "m_accelerationVectorFields" => descriptor_list(
                &mut result.acceleration_fields,
                scope(value)?,
                super::particle_field::field,
            )?,
            "m_meshes" => descriptor_list(&mut result.meshes, scope(value)?, super::particle_mesh::mesh)?,
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
