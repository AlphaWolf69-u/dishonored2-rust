//! arkBeamDesc (system list +0x398). Offsets are the reflection's; defaults
//! 1401cfe10 (descriptor) and 1401d09c0 (each control point).
use super::particle_stage::SpawnVector;
use super::particle_trail::{
    colour, enumeration, life_curves, ParticleTrailAsset, StandardCycles, StripMapping, StripTessellation,
    TrailLifeCurves,
};
use super::*;
use crate::gameplay::particles::{
    mesh::SpawnMesh,
    parameters::{ScalarParameter, VectorParameter},
    sprite::SpriteFlipbook,
};

/// position_t (start +0x2e0) and endposition_t (end +0x868).
pub struct BeamPosition {
    pub location: SpawnVector,
    pub velocity: SpawnVector,
    pub spawn_mesh: SpawnMesh,
    pub track_origin: bool,
    pub use_target_parameters: bool,
    pub use_target_axis: bool,
    pub track_target_parameters: bool,
}

/// arkBeamDesc::arkControlPoint (0xae8 bytes).
pub struct BeamControlPoint {
    pub initial_position: SpawnVector,
    pub radius: ScalarParameter,
    /// randommode_t: FREE 0, RADIAL_CONSTRAINED 1.
    pub radial_constrained: bool,
    pub randomize_period: [f32; 2],
    pub randomize_position: bool,
    pub radial_velocity: ScalarParameter,
    pub inherited_velocity: ScalarParameter,
    pub initial_velocity: VectorParameter,
    pub life: TrailLifeCurves,
}
impl Default for BeamControlPoint {
    fn default() -> Self {
        Self {
            initial_position: SpawnVector::new([0.; 3]),
            radius: ScalarParameter::constant(f32::from_bits(0x3dcc_cccd)),
            radial_constrained: false,
            randomize_period: [0.; 2],
            randomize_position: false,
            radial_velocity: ScalarParameter::constant(f32::from_bits(0x3c23_d70a)),
            inherited_velocity: ScalarParameter::constant(1.),
            initial_velocity: VectorParameter::constant([0.; 3]),
            life: TrailLifeCurves::plain(1.),
        }
    }
}

/// noise_t (+0x23d8): perlin_t {mode (PERCENT 0, LENGHT 1), octaves,
/// persistence, frequency}, then frequency and scale.
pub struct BeamNoise {
    pub by_length: bool,
    pub octaves: i32,
    pub persistence: f32,
    pub perlin_frequency: f32,
    pub frequency: f32,
    pub scale: f32,
}

pub struct ParticleBeamAsset {
    pub name: String,
    pub enabled: bool,
    pub enable_wind: bool,
    pub enable_velocity_inheritance: bool,
    pub worldspace: bool,
    pub timings: StandardCycles,
    pub material: Option<String>,
    pub flipbook: SpriteFlipbook,
    pub mapping: StripMapping,
    pub start: BeamPosition,
    pub end: BeamPosition,
    pub base_colour: [f32; 4],
    pub colour: VectorParameter,
    pub alpha: ScalarParameter,
    pub width: ScalarParameter,
    pub beam_life: TrailLifeCurves,
    pub start_life: TrailLifeCurves,
    pub end_life: TrailLifeCurves,
    /// controlPointsExtendedParameters_t (+0x23b0): m_index (-1 by
    /// 14010c020) and m_trackPositions (true).
    pub control_point_index: i32,
    pub track_positions: bool,
    pub control_points: Vec<BeamControlPoint>,
    pub reproject_control_points: bool,
    pub tessellation: StripTessellation,
    pub noise: BeamNoise,
}

impl Default for ParticleBeamAsset {
    fn default() -> Self {
        let position = |end: bool| BeamPosition {
            location: SpawnVector::new(if end { [0., 0., 1.] } else { [0.; 3] }),
            velocity: SpawnVector::new([0.; 3]),
            spawn_mesh: SpawnMesh::default(),
            track_origin: false,
            use_target_parameters: false,
            use_target_axis: false,
            track_target_parameters: false,
        };
        let trail_defaults = ParticleTrailAsset::default();
        Self {
            name: String::new(),
            enabled: true,
            enable_wind: false,
            enable_velocity_inheritance: false,
            worldspace: true,
            timings: StandardCycles::default(),
            material: None,
            flipbook: SpriteFlipbook::default(),
            mapping: trail_defaults.mapping,
            start: position(false),
            end: position(true),
            base_colour: [1.; 4],
            colour: VectorParameter::constant([1.; 3]),
            alpha: ScalarParameter::constant(1.),
            width: ScalarParameter::constant(1.),
            beam_life: TrailLifeCurves::new([f32::from_bits(0x3e4c_cccd), f32::from_bits(0x3f4c_cccd)], 0.),
            start_life: TrailLifeCurves::plain(1.),
            end_life: TrailLifeCurves::plain(1.),
            control_point_index: -1,
            track_positions: true,
            control_points: (0..2).map(|_| BeamControlPoint::default()).collect(),
            reproject_control_points: false,
            tessellation: StripTessellation { segments: 10., length_scaled: true, maximum_segments: 256 },
            noise: BeamNoise {
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
    atom(value)?.parse().map_err(|e| format!("invalid beam integer: {e}"))
}

fn position(result: &mut BeamPosition, fields: &Fields, end: bool) -> Result<(), String> {
    for (name, value) in fields {
        match name.as_str() {
            "m_initialLocation" => result.location.edit(scope(value)?)?,
            "m_initialVelocity" => result.velocity.edit(scope(value)?)?,
            "m_spawnMesh" => super::particle_emission_mesh::edit(&mut result.spawn_mesh, scope(value)?)?,
            "m_trackOrigin" => result.track_origin = boolean(value)?,
            "m_useTargetParameters" if end => result.use_target_parameters = boolean(value)?,
            "m_useTargetAxis" if end => result.use_target_axis = boolean(value)?,
            "m_trackTargetParameters" if end => result.track_target_parameters = boolean(value)?,
            _ => return Err(format!("unknown beam position field {name}")),
        }
    }
    Ok(())
}

fn control_point(fields: &Fields) -> Result<BeamControlPoint, String> {
    let mut result = BeamControlPoint::default();
    for (name, value) in fields {
        match name.as_str() {
            "m_initialPosition" => result.initial_position.edit(scope(value)?)?,
            "m_radius" => particles::scalar_parameter(&mut result.radius, scope(value)?)?,
            "m_mode" => {
                result.radial_constrained = match enumeration(value)? {
                    "FREE" => false,
                    "RADIAL_CONSTRAINED" => true,
                    mode => return Err(format!("unknown beam control point mode {mode}")),
                }
            }
            "m_randomizePeriod" => bounds(&mut result.randomize_period, scope(value)?)?,
            "m_randomizePosition" => result.randomize_position = boolean(value)?,
            "m_radialVelocity" => particles::scalar_parameter(&mut result.radial_velocity, scope(value)?)?,
            "m_inheritedVelocity" => particles::scalar_parameter(&mut result.inherited_velocity, scope(value)?)?,
            "m_initialVelocity" => particles::vector_parameter(&mut result.initial_velocity, scope(value)?)?,
            "m_lifeCurves" => life_curves(&mut result.life, scope(value)?)?,
            _ => return Err(format!("unknown beam control point field {name}")),
        }
    }
    Ok(result)
}

pub fn beam(fields: &Fields) -> Result<ParticleBeamAsset, String> {
    let mut result = ParticleBeamAsset::default();
    // The mapping, flipbook and tessellation readers are the trail ones.
    let mut shared = Vec::new();
    for (name, value) in fields {
        match name.as_str() {
            "m_name" => result.name = atom(value)?.to_owned(),
            "m_enabled" => result.enabled = boolean(value)?,
            "m_enableWind" => result.enable_wind = boolean(value)?,
            "m_enableVelocityInheritance" => result.enable_velocity_inheritance = boolean(value)?,
            "m_worldspace" => result.worldspace = boolean(value)?,
            "m_timings" => result.timings.edit(scope(value)?)?,
            "m_material" => {
                let name = atom(value)?;
                result.material = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL")).then(|| name.to_owned());
            }
            "m_flipBook" => super::particle_sprite::flipbook(&mut result.flipbook, scope(value)?)?,
            "m_mapping" | "m_tesselation" => shared.push((name.clone(), value.clone())),
            "m_spawn" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_startPosition" => position(&mut result.start, scope(value)?, false)?,
                        "m_endPosition" => position(&mut result.end, scope(value)?, true)?,
                        "m_baseColor" => colour(&mut result.base_colour, scope(value)?)?,
                        "m_initialColor" => particles::vector_parameter(&mut result.colour, scope(value)?)?,
                        "m_initialAlpha" => particles::scalar_parameter(&mut result.alpha, scope(value)?)?,
                        "m_initialWidth" => particles::scalar_parameter(&mut result.width, scope(value)?)?,
                        _ => return Err(format!("unknown beam spawn field {name}")),
                    }
                }
            }
            "m_beamLifeCurves" => life_curves(&mut result.beam_life, scope(value)?)?,
            "m_startPointLifeCurves" => life_curves(&mut result.start_life, scope(value)?)?,
            "m_endPointLifeCurves" => life_curves(&mut result.end_life, scope(value)?)?,
            "m_controlPointsExtendedParameters" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_index" => result.control_point_index = integer(value)?,
                        "m_trackPositions" => result.track_positions = boolean(value)?,
                        _ => return Err(format!("unknown beam control point parameter {name}")),
                    }
                }
            }
            "m_controlPoints" => descriptor_list(&mut result.control_points, scope(value)?, control_point)?,
            "m_reprojectControlPoints" => result.reproject_control_points = boolean(value)?,
            "m_noise" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_perlin" => {
                            for (name, value) in scope(value)? {
                                match name.as_str() {
                                    "m_mode" => {
                                        result.noise.by_length = match enumeration(value)? {
                                            "PERCENT" => false,
                                            "LENGHT" => true,
                                            mode => return Err(format!("unknown beam noise mode {mode}")),
                                        }
                                    }
                                    "m_octaves" => result.noise.octaves = integer(value)?,
                                    "m_persistence" => result.noise.persistence = number(value)?,
                                    "m_frequency" => result.noise.perlin_frequency = number(value)?,
                                    _ => return Err(format!("unknown beam perlin field {name}")),
                                }
                            }
                        }
                        "m_frequency" => result.noise.frequency = number(value)?,
                        "m_scale" => result.noise.scale = number(value)?,
                        _ => return Err(format!("unknown beam noise field {name}")),
                    }
                }
            }
            _ => return Err(format!("unknown particle beam field {name}")),
        }
    }
    if !shared.is_empty() {
        let shared = super::particle_trail::trail(&shared)?;
        result.mapping = shared.mapping;
        result.tessellation = shared.tessellation;
    }
    Ok(result)
}
