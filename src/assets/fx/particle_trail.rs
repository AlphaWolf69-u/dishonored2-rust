//! arkParticleTrailDesc (0x15b0 bytes): constructor 1401fab30, defaults
//! 1401facf0, spawn_t defaults 14020ed10, life_t defaults 14020f0f0 (trail)
//! and 14020ee80 (segment). Offsets are the reflection's.
use super::particle_stage::SpawnVector;
use super::*;
use crate::gameplay::particles::{
    parameters::{ScalarParameter, VectorParameter},
    sprite::SpriteFlipbook,
};

/// arkStripMapping (+0x1c0): m_mode +0 (PERCENT 0, LENGHT 1), m_scale +4,
/// m_axis +8 (U 0, V 1). 1401f9000: PERCENT, 1, U.
#[derive(Clone, Copy)]
pub struct StripMapping {
    pub by_length: bool,
    pub scale: f32,
    pub along_v: bool,
}
impl Default for StripMapping {
    fn default() -> Self {
        Self { by_length: false, scale: 1., along_v: false }
    }
}

/// arkStripTessellation (+0x1488), 1401f9010: 10 segments per unit length,
/// length scaled, at most 256 segments.
#[derive(Clone, Copy)]
pub struct StripTessellation {
    pub segments: f32,
    pub length_scaled: bool,
    pub maximum_segments: i32,
}

/// arkTrailCommonDesc::acceleration_t.
pub struct TrailAcceleration {
    pub value: VectorParameter,
    pub worldspace: bool,
    pub linear: ScalarParameter,
    pub gravity: ScalarParameter,
}

/// arkTrailCommonDesc::life_t (0x658 bytes).
pub struct TrailLifeCurves {
    pub velocity_over_life: VectorParameter,
    pub acceleration: TrailAcceleration,
    pub colour: VectorParameter,
    pub alpha: ScalarParameter,
    pub width: ScalarParameter,
}
impl TrailLifeCurves {
    /// Beam start/end-point and control-point life curves (1401cfe10,
    /// 1401d09c0): constant alpha, the given gravity, no fade keys.
    pub(super) fn plain(gravity: f32) -> Self {
        let mut curves = Self::new([0.; 2], gravity);
        curves.alpha = ScalarParameter::constant(1.);
        curves
    }
    /// 14020f0f0/14020ee80 share everything but the alpha fade keys
    /// (1401d1110) and the gravity scale.
    pub(super) fn new(fade: [f32; 2], gravity: f32) -> Self {
        let mut alpha = ScalarParameter::constant(1.);
        alpha.curve.maximum = crate::gameplay::effects::curve::EffectFloatCurve {
            times: vec![0., fade[0], fade[1], 1.],
            values: vec![0., 0., 0., 0., 1., 0., 0., 1., 0., 0., 0., 0.],
        };
        Self {
            velocity_over_life: VectorParameter::constant([1.; 3]),
            acceleration: TrailAcceleration {
                value: VectorParameter::constant([0.; 3]),
                worldspace: false,
                linear: ScalarParameter::constant(0.),
                gravity: ScalarParameter::constant(gravity),
            },
            colour: VectorParameter::constant([1.; 3]),
            alpha,
            width: ScalarParameter::constant(1.),
        }
    }
}

/// arkTrailCommonDesc::spawn_t (+0x1d0, 0x4f0 bytes).
pub struct TrailSpawn {
    pub location: SpawnVector,
    pub velocity: SpawnVector,
    pub base_colour: [f32; 4],
    pub colour: VectorParameter,
    pub alpha: ScalarParameter,
    pub width: ScalarParameter,
    pub minimal_emitter_move: f32,
    pub minimal_particle_distance: f32,
    pub interpolation_distance: f32,
}

pub struct ParticleTrailAsset {
    pub name: String,
    pub enabled: bool,
    pub worldspace: bool,
    pub life: ScalarParameter,
    pub material: Option<String>,
    pub flipbook: SpriteFlipbook,
    pub mapping: StripMapping,
    pub spawn: TrailSpawn,
    pub offset_over_life: SpawnVector,
    pub trail_life: TrailLifeCurves,
    pub segment_life: TrailLifeCurves,
    pub tessellation: StripTessellation,
    /// orientation_t (+0x1498): m_isVectorOriented, m_orientationVector.
    pub vector_oriented: bool,
    pub orientation_vector: VectorParameter,
}

impl Default for ParticleTrailAsset {
    fn default() -> Self {
        let mut orientation_vector = VectorParameter::constant([0., 0., 1.]);
        orientation_vector.uniform = false;
        Self {
            name: String::new(),
            enabled: true,
            worldspace: true,
            life: ScalarParameter::constant(0.5),
            material: None,
            flipbook: SpriteFlipbook::default(),
            mapping: StripMapping::default(),
            spawn: TrailSpawn {
                location: SpawnVector::new([0.; 3]),
                velocity: SpawnVector::new([0.; 3]),
                base_colour: [1.; 4],
                colour: VectorParameter::constant([1.; 3]),
                alpha: ScalarParameter::constant(1.),
                width: ScalarParameter::constant(1.),
                minimal_emitter_move: 0.,
                minimal_particle_distance: f32::from_bits(0x3b03_126f),
                interpolation_distance: 0.,
            },
            offset_over_life: SpawnVector::new([0.; 3]),
            trail_life: TrailLifeCurves::new([f32::from_bits(0x3e4c_cccd), f32::from_bits(0x3f4c_cccd)], 0.),
            segment_life: TrailLifeCurves::new([f32::from_bits(0x3dcc_cccd), 0.5], 1.),
            tessellation: StripTessellation { segments: 10., length_scaled: true, maximum_segments: 256 },
            vector_oriented: false,
            orientation_vector,
        }
    }
}

pub(super) fn life_curves(result: &mut TrailLifeCurves, fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_velocityOverLife" => particles::vector_parameter(&mut result.velocity_over_life, scope(value)?)?,
            "m_acceleration" => {
                for (name, value) in scope(value)? {
                    let acceleration = &mut result.acceleration;
                    match name.as_str() {
                        "m_value" => particles::vector_parameter(&mut acceleration.value, scope(value)?)?,
                        "m_worldspace" => acceleration.worldspace = boolean(value)?,
                        "m_linearAcceleration" => {
                            particles::scalar_parameter(&mut acceleration.linear, scope(value)?)?
                        }
                        "m_gravity" => particles::scalar_parameter(&mut acceleration.gravity, scope(value)?)?,
                        _ => return Err(format!("unknown trail acceleration field {name}")),
                    }
                }
            }
            "m_color" => particles::vector_parameter(&mut result.colour, scope(value)?)?,
            "m_alpha" => particles::scalar_parameter(&mut result.alpha, scope(value)?)?,
            "m_width" => particles::scalar_parameter(&mut result.width, scope(value)?)?,
            _ => return Err(format!("unknown trail life-curve field {name}")),
        }
    }
    Ok(())
}

pub(super) fn colour(target: &mut [f32; 4], fields: &Fields) -> Result<(), String> {
    for (name, value) in fields {
        let index = match name.as_str() {
            "r" => 0,
            "g" => 1,
            "b" => 2,
            "a" => 3,
            _ => return Err(format!("unknown trail colour component {name}")),
        };
        target[index] = number(value)?;
    }
    Ok(())
}

pub(super) fn enumeration<'a>(value: &'a Value) -> Result<&'a str, String> {
    Ok(atom(value)?.rsplit("::").next().unwrap_or(""))
}

pub fn trail(fields: &Fields) -> Result<ParticleTrailAsset, String> {
    let mut result = ParticleTrailAsset::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_worldspace" => result.worldspace = boolean(value)?,
            "m_trailLife" => particles::scalar_parameter(&mut result.life, scope(value)?)?,
            "m_material" => {
                let name = atom(value)?;
                result.material = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL")).then(|| name.to_owned());
            }
            "m_flipBook" => super::particle_sprite::flipbook(&mut result.flipbook, scope(value)?)?,
            "m_mapping" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_mode" => {
                            result.mapping.by_length = match enumeration(value)? {
                                "PERCENT" => false,
                                "LENGHT" => true,
                                mode => return Err(format!("unknown strip mapping mode {mode}")),
                            }
                        }
                        "m_scale" => result.mapping.scale = number(value)?,
                        "m_axis" => {
                            result.mapping.along_v = match enumeration(value)? {
                                "U" => false,
                                "V" => true,
                                axis => return Err(format!("unknown strip mapping axis {axis}")),
                            }
                        }
                        _ => return Err(format!("unknown strip mapping field {name}")),
                    }
                }
            }
            "m_spawn" => {
                let spawn = &mut result.spawn;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_initialLocation" => spawn.location.edit(scope(value)?)?,
                        "m_initialVelocity" => spawn.velocity.edit(scope(value)?)?,
                        "m_baseColor" => colour(&mut spawn.base_colour, scope(value)?)?,
                        "m_initialColor" => particles::vector_parameter(&mut spawn.colour, scope(value)?)?,
                        "m_initialAlpha" => particles::scalar_parameter(&mut spawn.alpha, scope(value)?)?,
                        "m_initialWidth" => particles::scalar_parameter(&mut spawn.width, scope(value)?)?,
                        "m_minimalEmitterMove" => spawn.minimal_emitter_move = number(value)?,
                        "m_minimalParticleDistance" => spawn.minimal_particle_distance = number(value)?,
                        "m_interpolationDistance" => spawn.interpolation_distance = number(value)?,
                        _ => return Err(format!("unknown trail spawn field {name}")),
                    }
                }
            }
            "m_offsetOverLife" => result.offset_over_life.edit(scope(value)?)?,
            "m_trailLifeCurves" => life_curves(&mut result.trail_life, scope(value)?)?,
            "m_segmentLifeCurves" => life_curves(&mut result.segment_life, scope(value)?)?,
            "m_tesselation" => {
                for (name, value) in scope(value)? {
                    let tessellation = &mut result.tessellation;
                    match name.as_str() {
                        "m_segments" => tessellation.segments = number(value)?,
                        "m_isLengthScaled" => tessellation.length_scaled = boolean(value)?,
                        "m_maxSegments" => {
                            tessellation.maximum_segments = atom(value)?
                                .parse()
                                .map_err(|e| format!("invalid trail segment limit: {e}"))?
                        }
                        _ => return Err(format!("unknown trail tessellation field {name}")),
                    }
                }
            }
            "m_orientation" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_isVectorOriented" => result.vector_oriented = boolean(value)?,
                        "m_orientationVector" => {
                            particles::vector_parameter(&mut result.orientation_vector, scope(value)?)?
                        }
                        _ => return Err(format!("unknown trail orientation field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown particle trail field {name}")),
        }
    }
    Ok(result)
}

/// arkStandardCycles plus the warmup/delay/duration of an arkTrailDesc or
/// arkBeamDesc m_timings block (+0x10). 1401f8f20 then 140210690/1401cfe10:
/// rate 1, life 0.5, duration 1, at most one burst event.
pub struct StandardCycles {
    pub emitter: crate::gameplay::particles::emitter::EmitterDefinition,
    pub burst: bool,
    pub burst_event: String,
    pub maximum_burst_events: u32,
    pub life: ScalarParameter,
    pub infinite_life: bool,
    pub warmup: [f32; 2],
}
impl Default for StandardCycles {
    fn default() -> Self {
        use crate::gameplay::particles::{emitter::EmitterDefinition, parameters::{EmissionRate, RateParameter}};
        Self {
            emitter: EmitterDefinition {
                cycles: 0,
                duration: [1.; 2],
                delay: [0.; 2],
                simulation_precision: 1.,
                rate: EmissionRate {
                    range: RateParameter { range: [1.; 2], lods: Vec::new() },
                    curve: EffectFloatCurvePair::constant(1.),
                },
            },
            burst: false,
            burst_event: String::new(),
            maximum_burst_events: 1,
            life: ScalarParameter::constant(0.5),
            infinite_life: false,
            warmup: [0.; 2],
        }
    }
}
impl StandardCycles {
    /// 1f82c0's estimate.
    pub fn capacity(&self) -> Result<u64, String> {
        super::particle_stage::standard_cycles_capacity(
            &self.emitter, &self.life, self.infinite_life, self.burst, &self.burst_event, self.maximum_burst_events,
        )
    }
    pub fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        let unsigned = |value: &Value| atom(value)?.parse::<u32>().map_err(|e| format!("invalid timing integer: {e}"));
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
                "m_rate_lod" => edit_list(&mut self.emitter.rate.range.lods, scope(value)?, [20.; 2],
                    |slot, value| bounds(slot, scope(value)?))?,
                "m_life" => particles::scalar_parameter(&mut self.life, scope(value)?)?,
                "m_infiniteLife" => self.infinite_life = boolean(value)?,
                "m_warmup" => bounds(&mut self.warmup, scope(value)?)?,
                "m_delay" => bounds(&mut self.emitter.delay, scope(value)?)?,
                "m_duration" => bounds(&mut self.emitter.duration, scope(value)?)?,
                _ => return Err(format!("unknown timing field {name}")),
            }
        }
        Ok(())
    }
}

/// arkTrailDesc (declaration m_trails): m_timings (+0x10) then the stage
/// trail body shifted by 0x1d8 (defaults 140210690). No m_worldspace.
pub struct DeclarationTrailAsset {
    pub enabled: bool,
    pub enable_wind: bool,
    pub enable_velocity_inheritance: bool,
    pub timings: StandardCycles,
    pub trail: ParticleTrailAsset,
}

impl DeclarationTrailAsset {
    /// 1f82c0's estimate, capped at 64 records by 140211de0.
    pub fn capacity(&self) -> Result<usize, String> {
        Ok(self.timings.capacity()?.min(64) as usize)
    }
}

pub fn declaration_trail(fields: &Fields) -> Result<DeclarationTrailAsset, String> {
    let mut result = DeclarationTrailAsset {
        enabled: true,
        enable_wind: false,
        enable_velocity_inheritance: false,
        timings: StandardCycles::default(),
        trail: ParticleTrailAsset::default(),
    };
    let mut body = Vec::new();
    for (name, value) in fields {
        match name.as_str() {
            "m_enabled" => result.enabled = boolean(value)?,
            "m_enableWind" => result.enable_wind = boolean(value)?,
            "m_enableVelocityInheritance" => result.enable_velocity_inheritance = boolean(value)?,
            "m_timings" => result.timings.edit(scope(value)?)?,
            "m_worldspace" => return Err("declaration trails have no m_worldspace".into()),
            _ => body.push((name.clone(), value.clone())),
        }
    }
    result.trail = trail(&body)?;
    Ok(result)
}
