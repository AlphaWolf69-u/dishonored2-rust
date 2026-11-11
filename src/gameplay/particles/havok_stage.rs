//! arkHavokParticleStageSimulator (constructor 1401dfc20): a particle stage
//! whose particles are Havok FX bodies. The stage frame (1401fef80) emits
//! pending spawn records (1401e7040); the pipeline step integrates the
//! bodies; its post-step callback (1401e7d40) turns pending records into
//! bodies (1401e41e0), samples the next step's Integrate parameters
//! (1401e78b0) and publishes each body's render state, live list and death.
//!
//! bevy_backend: the original steps the pipeline inside the physics world
//! step (1400908c0, clock 0's delta) on its own schedule; the rewrite steps
//! it right after the stage frame with the same frame delta.
use super::{
    emitter::{EmissionContext, EmitterBindings, ParticleEmitter},
    mesh_appearance::MeshInitialState,
    simulation::StageFrame,
    space::ParticleSpace,
    sprite::SpriteInitialState,
    stage::{Particle, ParticleStage},
    ParticleState,
};
use crate::{
    assets::fx::{particle_declaration::ParticleDeclaration, particle_havok_stage::ParticleHavokStageAsset},
    float::{add, divide, multiply, subtract},
    gameplay::{
        havok_fx::{
            integrate::Integrate,
            physics_collide::{self, CollideScratch, CollideTuning, CollideWorld, StaticCache},
            solve_contacts::SolveContacts,
            world_collide::WorldCollide,
            Pipeline,
        },
        reference_frame::ReferenceFrame,
    },
};
use std::sync::Arc;

const IDENTITY: [[f32; 3]; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
const EPSILON: f32 = f32::from_bits(0x3400_0000);
const FAR: f32 = f32::from_bits(0x7149_f2ca);

/// A spawn record (0x60 bytes at +0x3e8) waiting for the next post-step.
struct PendingSpawn {
    position: [f32; 3],
    inverse_life: f32,
    velocity: [f32; 3],
    seed: u32,
    colour: [f32; 4],
    fraction: f32,
    /// +0x40 orientation and +0x50 angular velocity, with AngularMotion.
    angular: Option<([f32; 4], [f32; 3])>,
}

/// The collision effects 1401dfc20 adds with m_enableCollisions:
/// PhysicsCollide (over WorldCollide) and SolveSphereContacts.
pub struct Collisions {
    pub world: WorldCollide,
    pub solver: SolveContacts,
    pub tuning: CollideTuning,
    cache: Vec<StaticCache>,
    scratch: CollideScratch,
    /// 14081a5d0 of m_layer, masked 0xf7dfffff.
    pub filter: u32,
}

impl Collisions {
    fn allocate(&mut self, bodies: &[i32]) {
        self.world.allocate(bodies);
        self.solver.allocate(bodies);
        for &body in bodies {
            self.cache[body as usize] = StaticCache::default();
        }
    }
}

pub struct HavokStageSimulation {
    pub declaration: Arc<ParticleDeclaration>,
    /// Index into the declaration's Havok stage list.
    pub index: usize,
    pub emitter: ParticleEmitter,
    pub pipeline: Pipeline,
    pub integrate: Integrate,
    pending: Vec<PendingSpawn>,
    /// +0x310/+0x320 per body (position, normalized age; velocity) for the
    /// draw path, in the slot of the body's index.
    pub particles: ParticleStage,
    /// +0x330: each body's life.
    life: Vec<f32>,
    pub sprites: Vec<Vec<Option<SpriteInitialState>>>,
    pub meshes: Vec<Vec<Option<MeshInitialState>>>,
    /// +0x360 live and +0x3d0 dead lists of the last post-step.
    pub live: Vec<i32>,
    dead: Vec<i32>,
    inherited_velocity: super::spawn::SpawnVelocity,
    /// +0x60c..+0x620 and +0x628: the system bounds and clip planes.
    clip_bounds: [[f32; 3]; 2],
    clip_planes: Vec<[f32; 4]>,
    /// +0x6cc..+0x6e0.
    pub bounds: [[f32; 3]; 2],
    pub owner_pose: ReferenceFrame,
    lod: usize,
    mesh_cursor: usize,
    /// +0x3e0.
    kill_all: bool,
    /// The contact solver's material (+0x28..+0x38).
    pub contact_material: [f32; 5],
    pub collisions: Option<Collisions>,
    /// +0x350: each body's remaining collisions before it reacts; sized
    /// only when both m_collisionCounter bounds are non-zero.
    counters: Vec<i8>,
}

impl HavokStageSimulation {
    pub fn definition(&self) -> &ParticleHavokStageAsset {
        &self.declaration.havok_stages[self.index]
    }

    /// 1401dfc20 with the system reset's stage part (1401fd7b0: 1401e6860,
    /// 1401e6800, 1401e78b0).
    pub fn new(
        declaration: Arc<ParticleDeclaration>,
        index: usize,
        pose: ReferenceFrame,
        system_seed: &mut u32,
        world_gravity: [f32; 4],
    ) -> Result<Self, String> {
        let definition = declaration.havok_stages.get(index).ok_or("Havok particle stage index absent")?;
        let stage = &definition.stage;
        if !stage.enabled {
            return Err("disabled Havok particle stage cannot be instantiated".into());
        }
        // 1401e3b00: the standard-cycles estimate, at most 8192, or 256
        // with an enabled trail.
        let mut capacity = stage.estimated_capacity()?.min(0x2000) as usize;
        if stage.trails.iter().any(|trail| trail.enabled) {
            capacity = capacity.min(0x100);
        }
        // +0x590 the cycle seed, +0x594 the spawn seed.
        crate::random::lcg_step(system_seed);
        let spawn_seed = crate::random::lcg_step(system_seed);
        let angular = definition.enable_collisions && definition.collisions.mesh.is_some()
            || definition.collisions.enable_angular_motion;
        let pipeline = Pipeline::new(capacity, definition.collisions.radius, angular);
        let counter = definition.collisions.counter;
        let counters = if counter[0] != 0 && counter[1] != 0 { vec![0; capacity] } else { Vec::new() };
        let emitter = ParticleEmitter::havok(Arc::new(stage.emitter.clone()), spawn_seed, pose)?;
        let sprites = stage.sprites.iter().map(|_| (0..capacity).map(|_| None).collect()).collect();
        let meshes = stage.meshes.iter().map(|_| (0..capacity).map(|_| None).collect()).collect();
        let mut result = Self {
            declaration: declaration.clone(),
            index,
            emitter,
            pipeline,
            integrate: Integrate::new(capacity),
            pending: Vec::new(),
            particles: ParticleStage::new(capacity),
            life: vec![0.; capacity],
            sprites,
            meshes,
            live: Vec::new(),
            dead: Vec::new(),
            inherited_velocity: Default::default(),
            clip_bounds: [[-FAR; 3], [FAR; 3]],
            clip_planes: Vec::new(),
            bounds: [[FAR; 3], [-FAR; 3]],
            owner_pose: pose,
            lod: 0,
            mesh_cursor: 0,
            kill_all: false,
            contact_material: [0.; 5],
            counters,
            collisions: definition.enable_collisions.then(|| Collisions {
                world: WorldCollide::new(capacity),
                solver: SolveContacts::new(capacity),
                tuning: CollideTuning::default(),
                cache: vec![StaticCache::default(); capacity],
                scratch: CollideScratch::default(),
                filter: crate::gameplay::collision_filter::filter_info(definition.collisions.layer) & 0xf7df_ffff,
            }),
        };
        result.set_integrate(world_gravity)?;
        Ok(result)
    }

    /// 1401e78b0: gravity, damping and clamps from simulation_t at the cycle
    /// fraction, drawn in turn from one copy of the cycle seed.
    fn set_integrate(&mut self, world_gravity: [f32; 4]) -> Result<(), String> {
        let simulation = &self.declaration.havok_stages[self.index].simulation;
        let fraction = self.emitter.cycle.fraction;
        let mut seed = self.emitter.cycle_seed();
        let gravity = simulation.gravity.sample(fraction, &mut seed)?;
        self.integrate.gravity = world_gravity.map(|lane| multiply(gravity, lane));
        let unit = |value: f32| if 0. <= value { if 1. < value { 1. } else { value } } else { 0. };
        let clamp = |value: f32| {
            let value = if value <= super::super::havok_fx::integrate::NO_CLAMP {
                value
            } else {
                super::super::havok_fx::integrate::NO_CLAMP
            };
            if value <= 0. { 0. } else { value }
        };
        self.integrate.linear_damping = unit(simulation.linear_damping.sample(fraction, &mut seed)?);
        self.integrate.linear_clamping = clamp(simulation.linear_clamping.sample(fraction, &mut seed)?);
        self.integrate.angular_damping = unit(simulation.angular_damping.sample(fraction, &mut seed)?);
        self.integrate.angular_clamping =
            clamp(multiply(simulation.angular_clamping.sample(fraction, &mut seed)?, f32::from_bits(0x3c8e_fa35)));
        // 1401e1d10 for the contact solver (convex +0x198, else sphere
        // +0x190), on the same seed: solver +0x28 friction, +0x2c
        // restitution, +0x30 stickiness, +0x38 friction threshold and +0x34
        // restitution threshold, the thresholds clamped at zero.
        let definition = &self.declaration.havok_stages[self.index];
        if definition.enable_collisions {
            let material = &definition.collisions.material;
            let positive = |value: f32| if value <= 0. { 0. } else { value };
            let friction = material.friction.sample(fraction, &mut seed)?;
            let restitution = material.restitution.sample(fraction, &mut seed)?;
            let stickiness = material.stickiness.sample(fraction, &mut seed)?;
            let friction_threshold = positive(material.friction_threshold.sample(fraction, &mut seed)?);
            let restitution_threshold = positive(material.restitution_threshold.sample(fraction, &mut seed)?);
            self.contact_material = [friction, restitution, stickiness, restitution_threshold, friction_threshold];
            if let Some(collisions) = self.collisions.as_mut() {
                let solver = &mut collisions.solver;
                solver.friction = friction;
                solver.restitution = restitution;
                solver.stickiness = stickiness;
                solver.restitution_threshold = restitution_threshold;
                solver.friction_threshold = friction_threshold;
            }
        }
        Ok(())
    }

    /// 1401e7840: a frame that only advances the cycle clock.
    pub fn advance_cycles(&mut self, delta: f32) -> Result<(), String> {
        let delta = self.emitter.consume_delay(delta);
        self.emitter.advance_clock(delta, &mut NoBindings)
    }

    /// 1401fef80's simulated branch.
    pub fn advance_frame(
        &mut self,
        delta: f32,
        frame: &StageFrame<'_>,
        source: Option<&super::spawn::SpawnVelocity>,
    ) -> Result<(), String> {
        self.owner_pose = frame.pose;
        self.lod = frame.lod;
        let delta = self.emitter.consume_delay(delta);
        self.emitter.pose.set_basis(frame.pose.basis);
        if self.emitter.delay.remaining() < EPSILON {
            self.emitter.pose.reset_fraction();
            self.emitter.pose.target_position(frame.pose.position);
            self.emitter.pose.set_duration(delta);
            let definition = &self.declaration.havok_stages[self.index];
            // 1401e7cf0.
            if self.declaration.velocity_inheritance.enabled && definition.stage.enable_velocity_inheritance {
                let source = source.ok_or("particle velocity inheritance awaits the owning entity's published motion")?;
                let mut seed = self.emitter.cycle_seed();
                self.inherited_velocity = super::spawn::SpawnVelocity::sample(
                    source,
                    &self.declaration.velocity_inheritance,
                    self.emitter.cycle.fraction,
                    &mut seed,
                )?;
            }
            // 1401e6f50.
            self.clip_bounds = frame.bounds;
            self.clip_planes = frame.planes.to_vec();
            let room = self.pipeline.allocator.capacity() as i32 - self.pending.len() as i32 - self.live.len() as i32;
            let multiplier = frame.rate;
            let burst = definition.stage.burst;
            let mut bindings = Spawner {
                declaration: &self.declaration,
                index: self.index,
                angular: self.pipeline.angular.is_some(),
                gravity: self.integrate.gravity,
                inherited_velocity: &self.inherited_velocity,
                mesh_cursor: &mut self.mesh_cursor,
                emitted: &mut self.pending,
            };
            if burst {
                self.emitter.havok_burst(delta, room, frame.lod, multiplier, &mut bindings)?;
            } else {
                self.emitter.havok_continuous(delta, room, frame.lod, multiplier, &mut bindings)?;
            }
            self.update_renderers(delta)?;
        }
        self.emitter.pose.set_position(frame.pose.position);
        self.emitter.pose.commit_orientation();
        Ok(())
    }

    /// 1401e5c90: the next post-step kills every body.
    pub fn flush(&mut self) {
        self.kill_all = true;
    }

    /// No live body (1401e5ca0) and no further emission (1401e3cd0).
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// 1401df200: the collision hull centroid the bodies are drawn around;
    /// zero until hulls are built (stages with a hull are not admitted).
    pub fn centroid_offset(&self) -> [f32; 3] {
        [0.; 3]
    }

    /// 1401e83f0: sprite (1401eeea0) and mesh (1401ebc50) updates of every
    /// live body at its normalized age.
    fn update_renderers(&mut self, delta: f32) -> Result<(), String> {
        let definition = &self.declaration.havok_stages[self.index].stage;
        for (descriptor, slots) in definition.sprites.iter().zip(self.sprites.iter_mut()) {
            for &body in &self.live {
                let age = self.particles.get(body as usize).map_or(0., |particle| particle.state.age);
                if let Some(state) = slots[body as usize].as_mut() {
                    // approximate_: +0x340 is sized but never written; the
                    // original reads whatever the allocation held.
                    let mut seed = 0;
                    state.advance(&descriptor.appearance, &descriptor.flipbook, age, delta, &mut seed)?;
                }
            }
        }
        for (descriptor, slots) in definition.meshes.iter().zip(self.meshes.iter_mut()) {
            for &body in &self.live {
                let age = self.particles.get(body as usize).map_or(0., |particle| particle.state.age);
                if let Some(state) = slots[body as usize].as_mut() {
                    let mut seed = 0;
                    state.advance(&descriptor.appearance, &descriptor.flipbook, age, delta, &mut seed)?;
                }
            }
        }
        Ok(())
    }

    /// The physics world's step of this pipeline (140ff4380) for the effects
    /// this port admits, then the post-step callback (1401e63f0).
    pub fn step(&mut self, dt: f32, world_gravity: [f32; 4], level: Option<CollideWorld<'_>>) -> Result<(), String> {
        let integrate = &mut self.integrate;
        let mut collisions = self.collisions.as_mut();
        self.pipeline.flush_trash(&mut |bodies, allocated| {
            if allocated {
                integrate.allocate(bodies);
                if let Some(collisions) = collisions.as_deref_mut() {
                    collisions.allocate(bodies);
                }
            }
        });
        self.pipeline.allocator.update_allocated();
        let bodies = self.pipeline.allocator.allocated()?.to_vec();
        let dt = if 0. <= dt { dt } else { 0. };
        // Stage 0 per body: PhysicsCollide's CD pass, then the contact
        // solve, then Integrate.
        if let Some(collisions) = self.collisions.as_mut() {
            let level = level.as_ref().ok_or("Havok particle collisions need the level collision map")?;
            // 1401df5a0: LOW -> collide mode 2, MEDIUM/HIGH -> 1, GROUND_TRUTH -> 4.
            let mode = match self.declaration.havok_stages[self.index].collisions.accuracy {
                0 => 2,
                1 | 2 => 1,
                3 => 4,
                accuracy => return Err(format!("unknown Havok particle collision accuracy {accuracy}")),
            };
            if mode == 4 {
                let states = collisions.solver.bodies.clone();
                physics_collide::static_ground_truth(
                    dt,
                    collisions.tuning,
                    self.pipeline.radius,
                    [collisions.solver.friction, collisions.solver.restitution],
                    self.pipeline.plane,
                    level,
                    &bodies,
                    &self.pipeline.linear,
                    &|body| 3 <= states[body as usize].state,
                    &mut collisions.world,
                    &mut collisions.cache,
                    &mut collisions.scratch,
                )?;
            } else {
                let step = physics_collide::CachedStep {
                    dt,
                    tuning: collisions.tuning,
                    radii: [self.pipeline.radius; 2],
                    own: [collisions.solver.friction, collisions.solver.restitution],
                    plane: self.pipeline.plane,
                    mode,
                    world: level,
                };
                physics_collide::static_cached(
                    &step,
                    &bodies,
                    &self.pipeline.linear,
                    &collisions.solver.bodies,
                    &mut collisions.world,
                    &mut collisions.cache,
                    &mut collisions.scratch,
                )?;
            }
            collisions.solver.solve_spheres(dt, self.pipeline.radius, &bodies, &collisions.world, &mut self.pipeline.linear)?;
        }
        let trash = self.integrate.integrate(
            dt,
            self.pipeline.domain,
            self.pipeline.plane,
            &bodies,
            &mut self.pipeline.linear,
            self.pipeline.angular.as_deref_mut(),
        );
        self.pipeline.trash(&trash);
        // Stage 1: PhysicsCollide's CCD pass.
        if let Some(collisions) = self.collisions.as_mut() {
            let states = &collisions.solver.bodies;
            physics_collide::static_ccd(
                self.pipeline.radius,
                &bodies,
                &|body| states[body as usize].state,
                &mut collisions.cache,
                &mut self.pipeline.linear,
            );
        }
        self.pipeline.steps += 1;
        self.pipeline.time = add(dt, self.pipeline.time);
        self.post_step(world_gravity)
    }

    /// 1401e7d40.
    fn post_step(&mut self, world_gravity: [f32; 4]) -> Result<(), String> {
        self.bounds = [[FAR; 3], [-FAR; 3]];
        self.allocate_pending()?;
        self.set_integrate(world_gravity)?;
        self.pipeline.allocator.update_allocated();
        self.dead.clear();
        self.live.clear();
        let definition = &self.declaration.havok_stages[self.index];
        let infinite = definition.stage.infinite_life;
        let reaction = &definition.collisions;
        let reacts = reaction.kill_on_collision
            || reaction.stop_on_collision
            || reaction.sleep_on_collision
            || !definition.events.collision.name.is_empty()
            || !self.counters.is_empty();
        let (kill, stop, sleep) = (reaction.kill_on_collision, reaction.stop_on_collision, reaction.sleep_on_collision);
        if reacts && !definition.events.collision.name.is_empty() {
            return Err("Havok particle collision events are not connected".into());
        }
        let first_contacts = if kill { 3 } else { 2 };
        let bodies = self.pipeline.allocator.allocated()?.to_vec();
        for body in bodies {
            let index = body as usize;
            let mut killed = self.kill_all;
            // The collision reactions, from the contact solver's state.
            if let Some(collisions) = self.collisions.as_mut().filter(|_| reacts) {
                let state = collisions.solver.bodies[index].state;
                'react: {
                    if state <= 0 {
                        break 'react;
                    }
                    if state <= first_contacts && !self.counters.is_empty() {
                        let counter = &mut self.counters[index];
                        if *counter == 0 {
                            break 'react;
                        }
                        *counter = counter.wrapping_sub(1);
                        if *counter != 0 {
                            break 'react;
                        }
                    }
                    if kill {
                        killed = true;
                    } else if stop {
                        if state < 3 && !definition.events.stop.name.is_empty() {
                            return Err("Havok particle stop events are not connected".into());
                        }
                        let linear = &mut self.pipeline.linear[index];
                        linear[1] = [0., 0., 0., linear[1][3]];
                        if let Some(angular) = self.pipeline.angular.as_mut() {
                            angular[index][1] = [0.; 4];
                        }
                        if sleep {
                            collisions.solver.bodies[index].state = 0x7f;
                        }
                    }
                }
            }
            let [p, v] = self.pipeline.linear[index];
            let mut age = divide(p[3], self.life[body as usize]);
            if 1. < age {
                if !infinite {
                    killed = true;
                } else {
                    age = subtract(age, age.floor());
                }
            }
            let position = [p[0], p[1], p[2]];
            for i in 0..3 {
                if position[i] <= self.bounds[0][i] { self.bounds[0][i] = position[i]; }
                if self.bounds[1][i] <= position[i] { self.bounds[1][i] = position[i]; }
            }
            self.particles.set(
                body as usize,
                Some(Particle {
                    state: ParticleState {
                        position,
                        age,
                        velocity: [v[0], v[1], v[2]],
                        inverse_lifetime: divide(1., self.life[body as usize]),
                    },
                    base_velocity: [0.; 3],
                    accumulated_velocity: [0.; 3],
                    seed: 0,
                }),
            );
            if !killed && self.inside(position) {
                self.live.push(body);
            } else {
                self.dead.push(body);
            }
        }
        // 1401e3ff0.
        for &body in &self.dead {
            self.particles.set(body as usize, None);
        }
        let dead = std::mem::take(&mut self.dead);
        self.pipeline.trash(&dead);
        self.dead = dead;
        self.kill_all = false;
        Ok(())
    }

    /// 1401e3d20: inside the system box and on the kept side of every plane.
    fn inside(&self, p: [f32; 3]) -> bool {
        let boxed = (0..3).all(|i| p[i] <= self.clip_bounds[1][i] && self.clip_bounds[0][i] <= p[i]);
        boxed
            && self.clip_planes.iter().all(|plane| {
                !(add(add(add(multiply(p[0], plane[0]), multiply(p[1], plane[1])), multiply(p[2], plane[2])), plane[3]) < 0.)
            })
    }

    /// 1401e41e0.
    fn allocate_pending(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        let integrate = &mut self.integrate;
        let mut collisions = self.collisions.as_mut();
        let bodies = self.pipeline.allocate(pending.len(), true, |bodies, allocated| {
            if allocated {
                integrate.allocate(bodies);
                if let Some(collisions) = collisions.as_deref_mut() {
                    collisions.allocate(bodies);
                }
            }
        });
        let definition = &self.declaration.havok_stages[self.index].stage;
        let counter = self.declaration.havok_stages[self.index].collisions.counter.map(|bound| bound as i32);
        for (record, &body) in pending.iter().zip(&bodies) {
            let slot = body as usize;
            if !self.counters.is_empty() {
                let (high, mut low) = (counter[0].max(counter[1]), counter[0].min(counter[1]));
                if low < high {
                    let draw = (crate::random::lcg_step(&mut self.emitter.seed) >> 10 & 0x7fff) as i64;
                    low += (draw % (high - low + 1) as i64) as i32;
                }
                self.counters[slot] = low as i8;
            }
            let linear = &mut self.pipeline.linear[slot];
            linear[0] = [record.position[0], record.position[1], record.position[2], linear[0][3]];
            linear[1] = [record.velocity[0], record.velocity[1], record.velocity[2], linear[1][3]];
            if let (Some(angular), Some((orientation, velocity))) = (self.pipeline.angular.as_mut(), record.angular) {
                angular[slot] = [orientation, [velocity[0], velocity[1], velocity[2], 0.]];
            }
            self.life[slot] = divide(1., record.inverse_life);
            for i in 0..3 {
                if record.position[i] <= self.bounds[0][i] { self.bounds[0][i] = record.position[i]; }
                if self.bounds[1][i] <= record.position[i] { self.bounds[1][i] = record.position[i]; }
            }
            let mut seed = record.seed;
            for (descriptor, slots) in definition.sprites.iter().zip(self.sprites.iter_mut()) {
                if !descriptor.enabled {
                    continue;
                }
                if definition.constant_seed {
                    seed = record.seed;
                }
                slots[slot] = Some(SpriteInitialState::new(
                    &descriptor.spawn,
                    record.fraction,
                    self.lod,
                    record.colour,
                    descriptor.flipbook.flips,
                    descriptor.flipbook.cycle_rate.then_some(&descriptor.flipbook.initial),
                    &mut seed,
                )?);
            }
            for (descriptor, slots) in definition.meshes.iter().zip(self.meshes.iter_mut()) {
                if !descriptor.enabled || descriptor.material.is_none() || descriptor.model.is_none() {
                    continue;
                }
                if definition.constant_seed {
                    seed = record.seed;
                }
                slots[slot] = Some(MeshInitialState::new(
                    &descriptor.spawn,
                    record.fraction,
                    self.lod,
                    record.colour,
                    descriptor.flipbook.flips,
                    (descriptor.use_flipbook && descriptor.flipbook.cycle_rate).then_some(&descriptor.flipbook.initial),
                    &mut seed,
                )?);
            }
        }
        Ok(())
    }

    /// The bodies' AngularMotion bases for the draw path (140aa0390), with
    /// the body index as the slot.
    pub fn orientation(&self, body: usize) -> Option<[[f32; 3]; 3]> {
        self.pipeline.angular.as_ref().map(|angular| {
            crate::assets::physworld::quaternion_basis(angular[body][0]).map(|row| [row[0], row[1], row[2]])
        })
    }
}

struct NoBindings;
impl EmitterBindings for NoBindings {
    fn spawn(&mut self, _: EmissionContext, _: &mut u32) -> Result<usize, String> {
        Ok(0)
    }
    fn reset_fields(&mut self, _: &mut u32) -> Result<(), String> {
        Ok(())
    }
    fn advance_shape(&mut self, _: f32, _: u32, _: f32, _: &mut u32) -> Result<(), String> {
        Ok(())
    }
}

struct Spawner<'a> {
    declaration: &'a ParticleDeclaration,
    index: usize,
    angular: bool,
    gravity: [f32; 4],
    inherited_velocity: &'a super::spawn::SpawnVelocity,
    /// +0x608: the spawn mesh's sequential cursor (1401f9540).
    mesh_cursor: &'a mut usize,
    emitted: &'a mut Vec<PendingSpawn>,
}

impl EmitterBindings for Spawner<'_> {
    /// 1401e6d30's field resets: the admitted stages have no fields.
    fn reset_fields(&mut self, _: &mut u32) -> Result<(), String> {
        Ok(())
    }
    fn advance_shape(&mut self, _: f32, _: u32, _: f32, _: &mut u32) -> Result<(), String> {
        Ok(())
    }
    /// 1401e7040.
    fn spawn(&mut self, context: EmissionContext, seed: &mut u32) -> Result<usize, String> {
        let definition = &self.declaration.havok_stages[self.index];
        let asset = &definition.stage;
        let fraction = context.cycle_fraction;
        // 1401df490.
        let life = asset.life.sample(fraction, seed)?;
        if !(EPSILON <= life) {
            return Ok(0);
        }
        // 1401deef0: Havok bodies live in world space; authored values are
        // in the emitter axes unless marked worldspace.
        let space = ParticleSpace::new(context.pose.basis, true);
        let mut position = space.transform(asset.location.value.sample(fraction, seed)?, asset.location.worldspace);
        let mut velocity = space.transform(asset.velocity.value.sample(fraction, seed)?, asset.velocity.worldspace);
        let mut colour = [1.; 4];
        if let Some(sample) = asset.spawn_mesh.sample(self.mesh_cursor, context.cycle_seed, seed, fraction)? {
            let mesh_position = space.transform(sample.position, asset.spawn_mesh.worldspace);
            let mesh_velocity = space.transform(sample.velocity, asset.spawn_mesh.worldspace);
            position = std::array::from_fn(|i| add(mesh_position[i], position[i]));
            velocity = std::array::from_fn(|i| add(mesh_velocity[i], velocity[i]));
            colour = sample.colour;
        }
        position = std::array::from_fn(|i| add(position[i], context.pose.position[i]));
        if let Some(mesh_velocity) = asset.velocity_mesh.sample(context.cycle_seed, seed, fraction)? {
            let mesh_velocity = space.transform(mesh_velocity, asset.velocity_mesh.worldspace);
            velocity = std::array::from_fn(|i| add(mesh_velocity[i], velocity[i]));
        }
        let cone = multiply(asset.velocity_spray.sample(fraction, seed)?, 0.5);
        velocity = super::spawn::spray(velocity, cone, seed);
        let inverse_life = divide(1., life);
        let particle_seed = crate::random::lcg_step(seed);
        let inherited = self.inherited_velocity.at_point(position);
        velocity = std::array::from_fn(|i| add(velocity[i], inherited[i]));
        // 1401df5c0: the rest of the emission slice under gravity alone.
        let delta = context.remaining_delta;
        let age = add(multiply(delta, inverse_life), 0.);
        if 1. < age {
            return Ok(0);
        }
        let mut gravity_seed = particle_seed;
        let gravity_scale = definition.simulation.gravity.sample(age, &mut gravity_seed)?;
        let gravity = self.gravity;
        velocity = std::array::from_fn(|i| add(multiply(multiply(gravity_scale, gravity[i]), delta), velocity[i]));
        position = std::array::from_fn(|i| add(multiply(velocity[i], delta), position[i]));
        let angular = if self.angular {
            let identity = ParticleSpace::new(IDENTITY, true);
            let mut basis = definition.initial_angular_orientation.sample(fraction, &identity, seed)?;
            if !definition.initial_angular_orientation.worldspace {
                basis = basis_times(basis, context.pose.basis);
            }
            let orientation = crate::assets::physworld::rotation_quaternion(
                basis.map(|row| [row[0], row[1], row[2], 0.]),
            );
            let squared = add(add(multiply(velocity[1], velocity[1]), multiply(velocity[0], velocity[0])), multiply(velocity[2], velocity[2]));
            let squared = if squared <= f32::from_bits(0x0080_0000) { f32::from_bits(0x0080_0000) } else { squared };
            let inverse = divide(1., squared.sqrt());
            let direction = velocity.map(|lane| multiply(lane, inverse));
            let turn = angular_velocity(definition, context.pose.basis, direction, fraction, seed)?;
            Some((orientation, rotation_vector(turn)))
        } else {
            None
        };
        self.emitted.push(PendingSpawn {
            position,
            inverse_life,
            velocity,
            seed: particle_seed,
            colour,
            fraction,
            angular,
        });
        Ok(1)
    }
}

/// 1400a55d0: m x a with its per-element sum orders.
fn basis_times(m: [[f32; 3]; 3], a: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let term = |r: usize, c: usize| {
        let row = m[r];
        match (r, c) {
            (0, 0) | (0, 1) | (1, 0) | (2, 0) => add(add(multiply(row[1], a[1][c]), multiply(row[0], a[0][c])), multiply(row[2], a[2][c])),
            (0, 2) | (1, 1) | (2, 1) => add(add(multiply(row[0], a[0][c]), multiply(row[1], a[1][c])), multiply(row[2], a[2][c])),
            _ => add(add(multiply(row[1], a[1][c]), multiply(row[0], a[0][c])), multiply(row[2], a[2][c])),
        }
    };
    std::array::from_fn(|r| std::array::from_fn(|c| term(r, c)))
}

/// 140110990: q1 q2.
fn quaternion_product(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        subtract(add(add(multiply(a[0], b[3]), multiply(b[0], a[3])), multiply(a[1], b[2])), multiply(a[2], b[1])),
        subtract(add(add(multiply(a[1], b[3]), multiply(b[1], a[3])), multiply(a[2], b[0])), multiply(b[2], a[0])),
        subtract(add(add(multiply(a[2], b[3]), multiply(b[2], a[3])), multiply(b[1], a[0])), multiply(a[1], b[0])),
        subtract(subtract(subtract(multiply(b[3], a[3]), multiply(a[0], b[0])), multiply(b[1], a[1])), multiply(a[2], b[2])),
    ]
}

/// 1401f6880: the spawn turn as a quaternion of the Euler rates (degrees,
/// negated, halved) in rotation order, then the linear velocity rate about
/// the spawn direction.
fn angular_velocity(
    definition: &ParticleHavokStageAsset,
    axes: [[f32; 3]; 3],
    direction: [f32; 3],
    fraction: f32,
    seed: &mut u32,
) -> Result<[f32; 4], String> {
    let velocity = &definition.initial_angular_velocity;
    let angles = velocity.value.angles.sample(fraction, seed)?;
    let to_radians = -f32::from_bits(0x3c8e_fa35);
    let angles = angles.map(|angle| multiply(angle, to_radians));
    let axes = if velocity.value.worldspace { IDENTITY } else { axes };
    let table = [0usize, 1, 2, 0, 1, 2, 0, 2, 1, 0, 2, 1, 0];
    let order = velocity.value.order;
    let turn = |axis: [f32; 3], angle: f32| {
        let (sin, cos) = (angle.sin(), angle.cos());
        [multiply(sin, axis[0]), multiply(sin, axis[1]), multiply(sin, axis[2]), cos]
    };
    let mut q = [0.; 4];
    for k in 0..3 {
        let index = table[order * 2 + k];
        let rotation = turn(axes[index], multiply(angles[index], 0.5));
        q = if k == 0 { rotation } else { quaternion_product(q, rotation) };
    }
    let rate = velocity.linear_velocity_rate.sample(fraction, seed)?;
    if direction.iter().any(|lane| lane.to_bits() & 0x7fff_ffff != 0) {
        let angle = multiply(-multiply(f32::from_bits(0x3c8e_fa35), rate), 0.5);
        q = quaternion_product(q, turn(direction, angle));
    }
    Ok(q)
}

/// 140111310 with scale 1: the quaternion's axis times -2 acos(w).
fn rotation_vector(q: [f32; 4]) -> [f32; 3] {
    let squared = add(add(multiply(q[1], q[1]), multiply(q[0], q[0])), multiply(q[2], q[2]));
    let squared = if squared <= f32::from_bits(0x0080_0000) { f32::from_bits(0x0080_0000) } else { squared };
    let inverse = divide(1., squared.sqrt());
    let angle = if -1. < q[3] { if q[3] < 1. { q[3].acos() } else { 0. } } else { std::f32::consts::PI };
    let scale = multiply(multiply(angle, -2.), 1.);
    [
        multiply(multiply(scale, q[0]), inverse),
        multiply(multiply(scale, q[1]), inverse),
        multiply(multiply(scale, q[2]), inverse),
    ]
}

