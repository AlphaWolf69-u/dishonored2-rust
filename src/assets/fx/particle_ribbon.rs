//! arkRibbonDesc (0x13b0 bytes, declaration list +0x5f8). Offsets are the
//! reflection's; defaults 140203b70 (spawner_t 140204000).
use super::particle_stage::SpawnVector;
use super::particle_trail::{colour, enumeration, StripMapping, StripTessellation};
use super::*;
use crate::gameplay::particles::{
    mesh::{SpawnMesh, VelocityMesh},
    parameters::{ScalarParameter, VectorParameter},
    sprite::SpriteFlipbook,
};

/// timings_t (+0x10): cycles +0, warmup +4, delay +0xc, duration +0x14
/// (each [m_max, m_min]), rate +0x20, life +0xe8 ([m_max, m_min]).
pub struct RibbonTimings {
    pub cycles: u32,
    pub warmup: [f32; 2],
    pub delay: [f32; 2],
    pub duration: [f32; 2],
    pub rate: ScalarParameter,
    pub life: [f32; 2],
}

/// spawner_t (+0x100).
pub struct RibbonSpawn {
    pub location: SpawnVector,
    pub velocity: SpawnVector,
    pub spawn_mesh: SpawnMesh,
    pub velocity_mesh: VelocityMesh,
    pub spray_cone: ScalarParameter,
}

/// acceleration_t (+0xa90).
pub struct RibbonAcceleration {
    pub value: VectorParameter,
    pub worldspace: bool,
    pub gravity: ScalarParameter,
}

/// life_t (+0xe40 cycle, +0x10e0 segment).
pub struct RibbonLife {
    pub colour: VectorParameter,
    pub alpha: ScalarParameter,
    pub width: ScalarParameter,
}
impl Default for RibbonLife {
    fn default() -> Self {
        Self {
            colour: VectorParameter::constant([1.; 3]),
            alpha: ScalarParameter::constant(1.),
            width: ScalarParameter::constant(1.),
        }
    }
}

/// section_t (+0x1380).
pub struct RibbonSection {
    pub start_from_origin: bool,
    pub minimum_life: f32,
    pub maximum_life: f32,
}

/// noise_t (+0x1398): perlin_t {mode (PERCENT 0, LENGTH 1), octaves,
/// persistence, frequency}, then frequency and scale.
pub struct RibbonNoise {
    pub by_length: bool,
    pub octaves: i32,
    pub persistence: f32,
    pub perlin_frequency: f32,
    pub frequency: f32,
    pub scale: f32,
}

pub struct ParticleRibbonAsset {
    pub name: String,
    pub enabled: bool,
    pub enable_wind: bool,
    pub enable_velocity_inheritance: bool,
    pub worldspace: bool,
    pub timings: RibbonTimings,
    pub spawn: RibbonSpawn,
    pub velocity_over_life: VectorParameter,
    pub acceleration: RibbonAcceleration,
    pub linear_acceleration: ScalarParameter,
    pub material: Option<String>,
    pub flipbook: SpriteFlipbook,
    pub mapping: StripMapping,
    pub base_colour: [f32; 4],
    pub cycle_life: RibbonLife,
    pub segment_life: RibbonLife,
    pub section: RibbonSection,
    pub tessellation: StripTessellation,
    pub noise: RibbonNoise,
}

impl Default for ParticleRibbonAsset {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            enable_wind: false,
            enable_velocity_inheritance: false,
            worldspace: true,
            timings: RibbonTimings {
                cycles: 0,
                warmup: [0.; 2],
                delay: [0.; 2],
                duration: [1.; 2],
                rate: ScalarParameter::constant(20.),
                life: [1.; 2],
            },
            spawn: RibbonSpawn {
                location: SpawnVector::new([0.; 3]),
                velocity: SpawnVector::new([0., 0., 1.]),
                spawn_mesh: SpawnMesh::default(),
                velocity_mesh: VelocityMesh::default(),
                spray_cone: ScalarParameter::constant(0.),
            },
            velocity_over_life: VectorParameter::constant([1.; 3]),
            acceleration: RibbonAcceleration {
                value: VectorParameter::constant([0.; 3]),
                worldspace: false,
                gravity: ScalarParameter::constant(0.),
            },
            linear_acceleration: ScalarParameter::constant(0.),
            material: None,
            flipbook: SpriteFlipbook::default(),
            mapping: StripMapping::default(),
            base_colour: [1.; 4],
            cycle_life: RibbonLife::default(),
            segment_life: RibbonLife::default(),
            section: RibbonSection { start_from_origin: true, minimum_life: 0., maximum_life: 1. },
            tessellation: StripTessellation { segments: 100., length_scaled: true, maximum_segments: 256 },
            noise: RibbonNoise {
                by_length: false,
                octaves: 2,
                persistence: f32::from_bits(0x3f66_6666),
                perlin_frequency: 100.,
                frequency: 10.,
                scale: 0.,
            },
        }
    }
}

fn integer(value: &Value) -> Result<i32, String> {
    atom(value)?.parse().map_err(|e| format!("invalid ribbon integer: {e}"))
}

fn life(result: &mut RibbonLife, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_color" => particles::vector_parameter(&mut result.colour, scope(value)?)?,
            "m_alpha" => particles::scalar_parameter(&mut result.alpha, scope(value)?)?,
            "m_width" => particles::scalar_parameter(&mut result.width, scope(value)?)?,
            _ => return Err(format!("unknown ribbon life field {name}")),
        }
    }
    Ok(())
}

pub fn ribbon(fields: &Fields) -> Result<ParticleRibbonAsset, String> {
    let mut result = ParticleRibbonAsset::default();
    // The mapping reader is the trail one.
    let mut shared = Vec::new();
    for (name, value) in fields {
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_enableWind" => result.enable_wind = boolean(value)?,
            "m_enableVelocityInheritance" => result.enable_velocity_inheritance = boolean(value)?,
            "m_worldspace" => result.worldspace = boolean(value)?,
            "m_main" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_name" => result.name = atom(value)?.to_owned(),
                        _ => return Err(format!("unknown ribbon main field {name}")),
                    }
                }
            }
            "m_timings" => {
                let timings = &mut result.timings;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_cycles" => {
                            timings.cycles = atom(value)?.parse().map_err(|e| format!("invalid ribbon cycles: {e}"))?
                        }
                        "m_warmup" => bounds(&mut timings.warmup, scope(value)?)?,
                        "m_delay" => bounds(&mut timings.delay, scope(value)?)?,
                        "m_duration" => bounds(&mut timings.duration, scope(value)?)?,
                        "m_rate" => particles::scalar_parameter(&mut timings.rate, scope(value)?)?,
                        "m_life" => bounds(&mut timings.life, scope(value)?)?,
                        _ => return Err(format!("unknown ribbon timing field {name}")),
                    }
                }
            }
            "m_spawn" => {
                let spawn = &mut result.spawn;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_initialLocation" => spawn.location.edit(scope(value)?)?,
                        "m_initialVelocity" => spawn.velocity.edit(scope(value)?)?,
                        "m_spawnMesh" => super::particle_emission_mesh::edit(&mut spawn.spawn_mesh, scope(value)?)?,
                        "m_velocityMesh" => {
                            super::particle_emission_mesh::velocity_edit(&mut spawn.velocity_mesh, scope(value)?)?
                        }
                        "m_velocitySprayCone" => particles::scalar_parameter(&mut spawn.spray_cone, scope(value)?)?,
                        _ => return Err(format!("unknown ribbon spawn field {name}")),
                    }
                }
            }
            "m_velocityOverLife" => particles::vector_parameter(&mut result.velocity_over_life, scope(value)?)?,
            "m_accelerationOverLife" => {
                let acceleration = &mut result.acceleration;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_value" => particles::vector_parameter(&mut acceleration.value, scope(value)?)?,
                        "m_worldspace" => acceleration.worldspace = boolean(value)?,
                        "m_gravity" => particles::scalar_parameter(&mut acceleration.gravity, scope(value)?)?,
                        _ => return Err(format!("unknown ribbon acceleration field {name}")),
                    }
                }
            }
            "m_linearAccelerationOverLife" => {
                particles::scalar_parameter(&mut result.linear_acceleration, scope(value)?)?
            }
            "m_material" => {
                let name = atom(value)?;
                result.material = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL")).then(|| name.to_owned());
            }
            "m_flipBook" => super::particle_sprite::flipbook(&mut result.flipbook, scope(value)?)?,
            "m_mapping" => shared.push((name.clone(), value.clone())),
            "m_tesselation" => {
                let tessellation = &mut result.tessellation;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_segments" => tessellation.segments = number(value)?,
                        "m_isLengthScaled" => tessellation.length_scaled = boolean(value)?,
                        "m_maxSegments" => tessellation.maximum_segments = integer(value)?,
                        _ => return Err(format!("unknown ribbon tessellation field {name}")),
                    }
                }
            }
            "m_baseColor" => colour(&mut result.base_colour, scope(value)?)?,
            "m_cycleLifeCurves" => life(&mut result.cycle_life, scope(value)?)?,
            "m_segmentLifeCurves" => life(&mut result.segment_life, scope(value)?)?,
            "m_section" => {
                let section = &mut result.section;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_startFromOrigin" => section.start_from_origin = boolean(value)?,
                        "m_minimumParticleLife" => section.minimum_life = number(value)?,
                        "m_maximumParticleLife" => section.maximum_life = number(value)?,
                        _ => return Err(format!("unknown ribbon section field {name}")),
                    }
                }
            }
            "m_noise" => {
                let noise = &mut result.noise;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_perlin" => {
                            for (name, value) in scope(value)? {
                                match name.as_str() {
                                    "m_mode" => {
                                        noise.by_length = match enumeration(value)? {
                                            "PERCENT" => false,
                                            "LENGTH" => true,
                                            mode => return Err(format!("unknown ribbon noise mode {mode}")),
                                        }
                                    }
                                    "m_octaves" => noise.octaves = integer(value)?,
                                    "m_persistence" => noise.persistence = number(value)?,
                                    "m_frequency" => noise.perlin_frequency = number(value)?,
                                    _ => return Err(format!("unknown ribbon perlin field {name}")),
                                }
                            }
                        }
                        "m_frequency" => noise.frequency = number(value)?,
                        "m_scale" => noise.scale = number(value)?,
                        _ => return Err(format!("unknown ribbon noise field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown particle ribbon field {name}")),
        }
    }
    if !shared.is_empty() {
        result.mapping = super::particle_trail::trail(&shared)?.mapping;
    }
    Ok(result)
}
