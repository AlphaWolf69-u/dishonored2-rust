//! arkHavokParticleStageDesc (0x1d20 bytes, declaration list +0x5c8).
//! Offsets are the reflection's (chunk_001017 3060-3900); defaults are the
//! list element constructor 1401dd300 and 1401de940, spawn_t 1401dd7c0 with
//! the standard cycles 1401f8f20, the angular spawn values 1401f8940 and
//! 1401f89d0, and the spawn collisions 1401f8c90.
use super::particle_stage::{spawn_orientation, unsigned, ParticleEvent, ParticleStageAsset};
use super::*;
use crate::gameplay::particles::{
    parameters::{ScalarParameter, VectorParameter},
    spawn::SpawnOrientation,
};

/// collisions_t::arkCollisionMaterial (+0x28 in collisions_t).
pub struct HavokCollisionMaterial {
    pub friction: ScalarParameter,
    pub restitution: ScalarParameter,
    pub stickiness: ScalarParameter,
    pub restitution_threshold: ScalarParameter,
    pub friction_threshold: ScalarParameter,
}

/// collisions_t (+0x28).
pub struct HavokCollisions {
    /// m_collisionCounter (arkCounter, 1401f88c0 with 0): [m_max, m_min].
    pub counter: [u32; 2],
    pub kill_on_collision: bool,
    pub stop_on_collision: bool,
    pub sleep_on_collision: bool,
    pub kill_on_ocean_collision: bool,
    pub self_collisions: bool,
    /// m_mesh (idStaticParticleModelData): the collision hull source.
    pub mesh: Option<String>,
    pub bounding_box: bool,
    pub radius: f32,
    pub enable_angular_motion: bool,
    pub material: HavokCollisionMaterial,
    /// m_accuracy (accuracy_t, default 1 MEDIUM).
    pub accuracy: u32,
    /// m_layer (arkCollisionLayer_t, default 4).
    pub layer: u32,
    /// UpdateCollisionShape (1401df6a0) of m_mesh and the convex solver's
    /// shape record (140fbdea0), filled by the definition loader.
    pub hull: Option<std::sync::Arc<CollisionHull>>,
}

/// A stage's collision hull: at most 16 vertices around the centroid
/// (+0x1d00), the centroid (+0x1d10) and SolveConvexContacts' record.
pub struct CollisionHull {
    pub vertices: Vec<[f32; 4]>,
    pub centroid: [f32; 3],
    pub shape: crate::gameplay::havok_fx::solve_convex_contacts::ConvexShape,
}

impl CollisionHull {
    /// 1401df6a0 over the model's vertices, then 140fbdea0.
    pub fn build(model: &crate::assets::particle_model::ParticleModel, bounding_box: bool) -> Result<Self, String> {
        let points: Vec<[f32; 4]> = model.vertices.iter().map(|v| [v.position[0], v.position[1], v.position[2], 0.]).collect();
        let (vertices, centroid) = crate::gameplay::havok_fx::convex_hull::collision_shape(&points)
            .map_err(|error| format!("arkHavokParticleStageDesc::UpdateCollisionShape() failed: {error}"))?;
        let shape = crate::gameplay::havok_fx::solve_convex_contacts::ConvexShape::from_hull(&vertices, bounding_box)?;
        Ok(Self { vertices, centroid, shape })
    }
}

/// simulation_t (+0x440): the Integrate parameters 1401e78b0 samples.
pub struct HavokSimulation {
    pub gravity: ScalarParameter,
    pub linear_damping: ScalarParameter,
    pub linear_clamping: ScalarParameter,
    pub angular_damping: ScalarParameter,
    pub angular_clamping: ScalarParameter,
}

/// arkInitialAngularVelocity (spawn_t +0xea8): the orientation-shaped value
/// block (1401f89d0) and a further curve at +0x118, value 0.
pub struct HavokAngularVelocity {
    pub value: SpawnOrientation,
    /// m_linearVelocityRate (+0x118).
    pub linear_velocity_rate: ScalarParameter,
}

/// arkParticleSystemDescStructures::arkSpawnCollisions (+0x18b0, 1401f8c90):
/// 1401db580 reads +0 the velocity ray cast (off) with its distance +8
/// (-50), +0xc the position sphere query (off) with its radius +0x10
/// (0.1), and an unnamed flag +1 (on) with an offset +4 (0).
pub struct HavokSpawnCollisions {
    pub position_sphere_query: bool,
    pub velocity_ray_cast: bool,
    pub velocity_ray_cast_distance: f32,
}

/// events_t (+0x1c68).
#[derive(Default)]
pub struct HavokEvents {
    pub birth: ParticleEvent,
    pub collision: ParticleEvent,
    pub ocean_collision: ParticleEvent,
    pub stop: ParticleEvent,
    pub death: ParticleEvent,
    pub life: Vec<ParticleEvent>,
}

pub struct ParticleHavokStageAsset {
    /// The fields shared with ordinary stages: enable flags, name, spawn
    /// cycles and placement, sprites, meshes, trails, fields, attractors,
    /// sorting and seed.
    pub stage: ParticleStageAsset,
    /// m_main.m_boundsMargins (+0x10).
    pub bounds_margins: [f32; 3],
    /// +0x20.
    pub enable_collisions: bool,
    pub collisions: HavokCollisions,
    pub simulation: HavokSimulation,
    pub initial_angular_orientation: SpawnOrientation,
    pub initial_angular_velocity: HavokAngularVelocity,
    pub spawn_collisions: HavokSpawnCollisions,
    /// m_wind (+0x18c8): drag over cycle and life (1 each).
    pub wind_drag_over_cycle: ScalarParameter,
    pub wind_drag_over_life: ScalarParameter,
    /// m_accelerationOverLife (+0x1a88): zero vector with a unit curve.
    pub acceleration_over_life: VectorParameter,
    /// m_linearAccelerationOverLife (+0x1ba0), 0.
    pub linear_acceleration_over_life: ScalarParameter,
    pub events: HavokEvents,
}

impl Default for ParticleHavokStageAsset {
    fn default() -> Self {
        Self {
            stage: ParticleStageAsset::default(),
            bounds_margins: [f32::from_bits(0x3dcc_cccd); 3],
            enable_collisions: true,
            collisions: HavokCollisions {
                counter: [0; 2],
                kill_on_collision: false,
                stop_on_collision: false,
                sleep_on_collision: false,
                kill_on_ocean_collision: false,
                self_collisions: false,
                mesh: None,
                bounding_box: false,
                radius: f32::from_bits(0x3c23_d70a),
                enable_angular_motion: false,
                material: HavokCollisionMaterial {
                    friction: ScalarParameter::constant(0.),
                    restitution: ScalarParameter::constant(0.),
                    stickiness: ScalarParameter::constant(0.),
                    restitution_threshold: ScalarParameter::constant(12.),
                    friction_threshold: ScalarParameter::constant(12.),
                },
                accuracy: 1,
                layer: 4,
                hull: None,
            },
            simulation: HavokSimulation {
                gravity: ScalarParameter::constant(1.),
                linear_damping: ScalarParameter::constant(0.),
                linear_clamping: ScalarParameter::constant(f32::from_bits(0x5f7f_fff0)),
                angular_damping: ScalarParameter::constant(0.),
                angular_clamping: ScalarParameter::constant(f32::from_bits(0x5f7f_fff0)),
            },
            initial_angular_orientation: SpawnOrientation::default(),
            initial_angular_velocity: HavokAngularVelocity {
                value: SpawnOrientation::default(),
                linear_velocity_rate: ScalarParameter::constant(0.),
            },
            spawn_collisions: HavokSpawnCollisions {
                position_sphere_query: false,
                velocity_ray_cast: false,
                velocity_ray_cast_distance: -50.,
            },
            wind_drag_over_cycle: ScalarParameter::constant(1.),
            wind_drag_over_life: ScalarParameter::constant(1.),
            acceleration_over_life: VectorParameter::constant([0.; 3]),
            linear_acceleration_over_life: ScalarParameter::constant(0.),
            events: HavokEvents::default(),
        }
    }
}

fn curve(parameter: &mut ScalarParameter, value: &Value) -> Result<(), String> {
    particles::scalar_parameter(parameter, scope(value)?)
}

impl HavokCollisions {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_collisionCounter" => {
                    for (name, value) in scope(value)? {
                        match name.as_str() {
                            "m_max" => self.counter[0] = unsigned(value)?,
                            "m_min" => self.counter[1] = unsigned(value)?,
                            _ => return Err(format!("unknown Havok particle collision counter field {name}")),
                        }
                    }
                }
                "m_killOnCollision" => self.kill_on_collision = boolean(value)?,
                "m_stopOnCollision" => self.stop_on_collision = boolean(value)?,
                "m_sleepOnCollision" => self.sleep_on_collision = boolean(value)?,
                "m_killOnOceanCollision" => self.kill_on_ocean_collision = boolean(value)?,
                "m_selfCollisions" => self.self_collisions = boolean(value)?,
                "m_mesh" => self.mesh = Some(atom(value)?.to_owned()).filter(|mesh| !mesh.is_empty()),
                "m_boundingBox" => self.bounding_box = boolean(value)?,
                "m_radius" => self.radius = number(value)?,
                "m_enableAngularMotion" => self.enable_angular_motion = boolean(value)?,
                "m_material" => {
                    let material = &mut self.material;
                    for (name, value) in scope(value)? {
                        match name.as_str() {
                            "m_friction" => curve(&mut material.friction, value)?,
                            "m_restitution" => curve(&mut material.restitution, value)?,
                            "m_stickiness" => curve(&mut material.stickiness, value)?,
                            "m_restitutionThreshold" => curve(&mut material.restitution_threshold, value)?,
                            "m_frictionThreshold" => curve(&mut material.friction_threshold, value)?,
                            _ => return Err(format!("unknown Havok particle collision material field {name}")),
                        }
                    }
                }
                "m_accuracy" => {
                    self.accuracy = match atom(value)?.rsplit("::").next().unwrap_or("") {
                        // accuracy_t (1419fe300).
                        "LOW" => 0,
                        "MEDIUM" => 1,
                        "HIGH" => 2,
                        "GROUND_TRUTH" => 3,
                        accuracy => return Err(format!("unknown Havok particle collision accuracy {accuracy}")),
                    }
                }
                "m_layer" => {
                    let layer = atom(value)?;
                    self.layer = crate::gameplay::collision_filter::layer_index(layer)
                        .ok_or_else(|| format!("unknown Havok particle collision layer {layer}"))?;
                }
                _ => return Err(format!("unknown Havok particle collisions field {name}")),
            }
        }
        Ok(())
    }
}

impl HavokEvents {
    fn edit(&mut self, fields: &Fields) -> Result<(), String> {
        for (name, value) in fields {
            match name.as_str() {
                "m_birth" => self.birth.edit(scope(value)?, false)?,
                "m_collision" => self.collision.edit(scope(value)?, false)?,
                "m_oceanCollision" => self.ocean_collision.edit(scope(value)?, false)?,
                "m_stop" => self.stop.edit(scope(value)?, false)?,
                "m_death" => self.death.edit(scope(value)?, false)?,
                "m_life" => {
                    for (name, value) in scope(value)? {
                        if name == "num" {
                            self.life.resize_with(unsigned(value)? as usize, || ParticleEvent {
                                life: Some(0.),
                                ..Default::default()
                            });
                        } else {
                            let index = name
                                .strip_prefix("item[")
                                .and_then(|s| s.strip_suffix(']'))
                                .ok_or_else(|| format!("unknown Havok particle life-event field {name}"))?
                                .parse::<usize>()
                                .map_err(|e| format!("Havok particle life-event index: {e}"))?;
                            self.life
                                .get_mut(index)
                                .ok_or("Havok particle life-event index exceeds count")?
                                .edit(scope(value)?, true)?;
                        }
                    }
                }
                _ => return Err(format!("unknown Havok particle events field {name}")),
            }
        }
        Ok(())
    }
}

pub fn havok_stage(fields: &Fields) -> Result<ParticleHavokStageAsset, String> {
    let mut result = ParticleHavokStageAsset::default();
    // A disabled stage gets no simulator (1401dfc20 is built only for
    // enabled stages), so its other fields are not admitted.
    if fields.iter().any(|(name, value)| name == "m_enabled" && matches!(value, Value::Atom(value) if value == "false")) {
        result.stage.enabled = false;
        return Ok(result);
    }
    // The shared fields go through the ordinary stage reader, field by field.
    let mut shared: Vec<(String, Value)> = Vec::new();
    for (name, value) in fields {
        match name.as_str() {
            "m_main" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_name" => result.stage.name = atom(value)?.to_owned(),
                        "m_boundsMargins" => particles::vector(&mut result.bounds_margins, scope(value)?)?,
                        _ => return Err(format!("unknown Havok particle main field {name}")),
                    }
                }
            }
            "m_enableCollisions" => result.enable_collisions = boolean(value)?,
            "m_collisions" => result.collisions.edit(scope(value)?)?,
            "m_cycleSimulation" => {
                let simulation = &mut result.simulation;
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_gravity" => curve(&mut simulation.gravity, value)?,
                        "m_linearDamping" => curve(&mut simulation.linear_damping, value)?,
                        "m_linearClamping" => curve(&mut simulation.linear_clamping, value)?,
                        "m_angularDamping" => curve(&mut simulation.angular_damping, value)?,
                        "m_angularClamping" => curve(&mut simulation.angular_clamping, value)?,
                        _ => return Err(format!("unknown Havok particle simulation field {name}")),
                    }
                }
            }
            "m_spawn" => {
                let mut ordinary: Vec<(String, Value)> = Vec::new();
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_initialAngularOrientation" => {
                            spawn_orientation(&mut result.initial_angular_orientation, scope(value)?)?
                        }
                        "m_initialAngularVelocity" => {
                            let velocity = &mut result.initial_angular_velocity;
                            let mut orientation: Vec<(String, Value)> = Vec::new();
                            for (name, value) in scope(value)? {
                                if name == "m_linearVelocityRate" {
                                    curve(&mut velocity.linear_velocity_rate, value)?;
                                } else {
                                    orientation.push((name.clone(), value.clone()));
                                }
                            }
                            spawn_orientation(&mut velocity.value, &orientation)?;
                        }
                        _ => ordinary.push((name.clone(), value.clone())),
                    }
                }
                result.stage.spawn(&ordinary)?;
            }
            "m_spawnCollisions" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_positionSphereQuery" => result.spawn_collisions.position_sphere_query = boolean(value)?,
                        "m_velocityRayCast" => result.spawn_collisions.velocity_ray_cast = boolean(value)?,
                        "m_velocityRayCastDistance" => {
                            result.spawn_collisions.velocity_ray_cast_distance = number(value)?
                        }
                        _ => return Err(format!("unknown Havok particle spawn collisions field {name}")),
                    }
                }
            }
            "m_wind" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_dragOverCycle" => curve(&mut result.wind_drag_over_cycle, value)?,
                        "m_dragOverLife" => curve(&mut result.wind_drag_over_life, value)?,
                        _ => return Err(format!("unknown Havok particle wind field {name}")),
                    }
                }
            }
            "m_accelerationOverLife" => {
                for (name, value) in scope(value)? {
                    match name.as_str() {
                        "m_value" => particles::vector_parameter(&mut result.acceleration_over_life, scope(value)?)?,
                        _ => return Err(format!("unknown Havok particle acceleration field {name}")),
                    }
                }
            }
            "m_linearAccelerationOverLife" => {
                particles::scalar_parameter(&mut result.linear_acceleration_over_life, scope(value)?)?
            }
            "m_events" => result.events.edit(scope(value)?)?,
            "m_enabled" | "m_enableWind" | "m_enableVelocityInheritance" | "m_vectorFields"
            | "m_accelerationVectorFields" | "m_attractors" | "m_sorting" | "m_constantSeed"
            | "m_sprites" | "m_meshes" | "m_trails" => shared.push((name.clone(), value.clone())),
            _ => return Err(format!("unresolved Havok particle stage producer {name}")),
        }
    }
    let name = std::mem::take(&mut result.stage.name);
    let spawn = std::mem::take(&mut result.stage);
    let mut stage = super::particle_stage::stage(&shared)?;
    // Keep the spawn block read above and the shared fields read now.
    stage.name = name;
    stage.emitter = spawn.emitter;
    stage.burst = spawn.burst;
    stage.burst_event = spawn.burst_event;
    stage.maximum_burst_events = spawn.maximum_burst_events;
    stage.life = spawn.life;
    stage.infinite_life = spawn.infinite_life;
    stage.warmup = spawn.warmup;
    stage.spawn_only_from_stage = spawn.spawn_only_from_stage;
    stage.spawn_stage = spawn.spawn_stage;
    stage.use_event_location = spawn.use_event_location;
    stage.event_velocity = spawn.event_velocity;
    stage.event_normal = spawn.event_normal;
    stage.location = spawn.location;
    stage.velocity = spawn.velocity;
    stage.spawn_mesh = spawn.spawn_mesh;
    stage.velocity_mesh = spawn.velocity_mesh;
    stage.velocity_orientation = spawn.velocity_orientation;
    stage.velocity_periodic_orientation = spawn.velocity_periodic_orientation;
    stage.velocity_spray = spawn.velocity_spray;
    stage.spawn_fields = spawn.spawn_fields;
    result.stage = stage;
    Ok(result)
}
