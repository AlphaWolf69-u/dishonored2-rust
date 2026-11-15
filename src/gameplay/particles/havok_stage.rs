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
            dynamic_collide::{DynamicCollide, DynamicStep, MovingWorld},
            integrate::Integrate,
            physics_collide::{self, CollideScratch, CollideTuning, CollideWorld, StaticCache},
            solve_contacts::{BodyState, SolveContacts},
            solve_convex_contacts::SolveConvexContacts,
            spatial_partition::SpatialPartition,
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

/// 1401dfc20's contact solver: SolveConvexContacts (140fbda60 with the
/// stage's collision hull) when m_mesh is set, else SolveSphereContacts.
pub enum ContactSolver {
    Sphere(SolveContacts),
    Convex(SolveConvexContacts),
}

impl ContactSolver {
    pub fn bodies(&self) -> &[BodyState] {
        match self {
            Self::Sphere(solver) => &solver.bodies,
            Self::Convex(solver) => &solver.bodies,
        }
    }

    pub fn bodies_mut(&mut self) -> &mut [BodyState] {
        match self {
            Self::Sphere(solver) => &mut solver.bodies,
            Self::Convex(solver) => &mut solver.bodies,
        }
    }

    /// The body states, and SolveConvexContacts' shape records with the
    /// per-body index when it has at least two (140fc1370 bVar85), with
    /// +0x28 friction, +0x2c restitution and +0x38 friction threshold.
    #[allow(clippy::type_complexity)]
    fn split_convex(
        &mut self,
    ) -> (
        &mut [BodyState],
        (Option<(&[crate::gameplay::havok_fx::solve_convex_contacts::ConvexShape], &[i32])>, [f32; 3]),
    ) {
        match self {
            Self::Sphere(solver) => {
                let material = [solver.friction, solver.restitution, solver.friction_threshold];
                (&mut solver.bodies, (None, material))
            }
            Self::Convex(solver) => {
                let material = [solver.friction, solver.restitution, solver.friction_threshold];
                let shapes = (solver.shapes.len() >= 2).then_some((&solver.shapes[..], &solver.shape_index[..]));
                (&mut solver.bodies, (shapes, material))
            }
        }
    }

    /// +0x28 friction, +0x2c restitution.
    pub fn own_material(&self) -> [f32; 2] {
        match self {
            Self::Sphere(solver) => [solver.friction, solver.restitution],
            Self::Convex(solver) => [solver.friction, solver.restitution],
        }
    }

    /// 1401e1d10: +0x28..+0x38.
    fn set_material(&mut self, [friction, restitution, stickiness, restitution_threshold, friction_threshold]: [f32; 5]) {
        macro_rules! set {
            ($solver:expr) => {{
                $solver.friction = friction;
                $solver.restitution = restitution;
                $solver.stickiness = stickiness;
                $solver.restitution_threshold = restitution_threshold;
                $solver.friction_threshold = friction_threshold;
            }};
        }
        match self {
            Self::Sphere(solver) => set!(solver),
            Self::Convex(solver) => set!(solver),
        }
    }

    fn allocate(&mut self, bodies: &[i32]) {
        match self {
            Self::Sphere(solver) => solver.allocate(bodies),
            Self::Convex(solver) => solver.allocate(bodies),
        }
    }
}

/// The collision effects 1401dfc20 adds with m_enableCollisions:
/// PhysicsCollide (over WorldCollide) and the contact solver.
pub struct Collisions {
    pub world: WorldCollide,
    pub solver: ContactSolver,
    pub tuning: CollideTuning,
    cache: Vec<StaticCache>,
    scratch: CollideScratch,
    /// 14081a5d0 of m_layer, masked 0xf7dfffff.
    pub filter: u32,
    /// The hkfxEffects::SpatialPartition PhysicsCollide requires (140fbefb0
    /// looks it up or adds it) and its moving-body part.
    partition: SpatialPartition,
    dynamic: DynamicCollide,
}

impl Collisions {
    fn allocate(&mut self, bodies: &[i32]) {
        self.world.allocate(bodies);
        self.solver.allocate(bodies);
        for &body in bodies {
            self.cache[body as usize] = StaticCache::default();
        }
        self.partition.allocate(bodies);
        self.dynamic.allocate(bodies);
    }

    /// Slot 9 of the partition (140fd7180) and PhysicsCollide (140fbf580).
    fn release(&mut self, bodies: &[i32]) {
        self.partition.release(bodies);
        self.dynamic.release(bodies);
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
    /// +0x3b0: one trail renderer (1402107b0) per enabled trail with a
    /// material (1401fada0), else an empty list; no world transform.
    pub trails: Vec<Vec<Option<super::trail::TrailRecord>>>,
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
    /// +0x750..+0x7d0: the stage events' listeners.
    events: HavokEventLists,
    /// +0x698: the attractor states (1401cdff0 from the spawn seed),
    /// prepared each simulated frame (1401e7650) and applied to the bodies
    /// by arkAttractorsHavokEffect (1401ce600).
    attractors: Vec<super::attractor::AttractorState>,
    /// +0x668/+0x678/+0x688: the spawn, vector and acceleration field
    /// states (140216260 from the spawn seed), prepared with the
    /// attractors; arkVectorFieldsHavokEffect (1402165f0) applies the last
    /// two.
    fields: [Vec<super::field::FieldState>; 3],
    /// The level collision for the spawn queries (+0x400, 1401db580).
    pub spawn_world: Option<(Arc<crate::gameplay::navigation::CollisionMap>, Arc<[bool]>)>,
    /// The owner's physics shapes (system +0x1d78) for m_physicsShapesSpawn.
    pub owner_shapes: Option<Arc<super::shape_spawn::OwnerShapes>>,
    /// The dynamic world bodies PhysicsCollide's moving-body part collides
    /// with; none is an empty world.
    pub moving: Option<Arc<MovingWorld>>,
    /// Events for the system's ordinary stages and FX event instances,
    /// delivered by the owning system after each frame and step.
    pub deliveries: Vec<super::events::Delivery>,
    pub fx_pending: Vec<(usize, super::fx_event::FxEventRecord)>,
}

/// One Havok stage event's listeners (1401e6420): the ordinary stages whose
/// m_burstEvent names it and the system's FX event instances, with the
/// frequency threshold (x 32767, truncated) and a life event's age.
struct HavokSubscription {
    stages: Vec<usize>,
    fx: Vec<usize>,
    threshold: i32,
    life: f32,
}

impl HavokSubscription {
    fn new(declaration: &ParticleDeclaration, event: &crate::assets::fx::particle_stage::ParticleEvent) -> Self {
        let stages = if event.name.is_empty() {
            Vec::new()
        } else {
            declaration
                .stages
                .iter()
                .enumerate()
                .filter(|(_, stage)| stage.enabled && !stage.burst_event.is_empty() && stage.burst_event == event.name)
                .map(|(index, _)| index)
                .collect()
        };
        Self {
            stages,
            fx: super::events::fx_receivers(declaration, &event.name),
            threshold: crate::float::truncate_i32(multiply(event.frequency, f32::from_bits(0x46ff_fe00))),
            life: event.life.unwrap_or(0.),
        }
    }

    fn listened(&self) -> bool {
        !(self.stages.is_empty() && self.fx.is_empty())
    }
}

/// +0x758 birth, +0x770 collision, +0x7a0 stop, +0x7b8 death and the
/// +0x7c8 life events.
struct HavokEventLists {
    birth: HavokSubscription,
    collision: HavokSubscription,
    stop: HavokSubscription,
    death: HavokSubscription,
    life: Vec<HavokSubscription>,
}

/// Every listener's handler with the event's position, velocity and normal.
fn deliver(
    subscription: &HavokSubscription,
    deliveries: &mut Vec<super::events::Delivery>,
    fx_pending: &mut Vec<(usize, super::fx_event::FxEventRecord)>,
    record: super::fx_event::FxEventRecord,
) {
    let [position, velocity, normal] = record;
    deliveries.extend(subscription.stages.iter().map(|&receiver| super::events::Delivery {
        receiver,
        input: super::SpawnInput { position, directions: [velocity, normal] },
    }));
    fx_pending.extend(subscription.fx.iter().map(|&instance| (instance, record)));
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
        let mut pipeline = Pipeline::new(capacity, definition.collisions.radius, angular);
        let shape = definition.collisions.hull.as_ref().filter(|_| definition.enable_collisions).map(|hull| hull.shape.clone());
        if let Some(shape) = &shape {
            // 140fbdc80 with the stage's single shape record.
            pipeline.inner_radius = shape.inner;
            pipeline.radius = shape.outer;
        }
        let counter = definition.collisions.counter;
        let counters = if counter[0] != 0 && counter[1] != 0 { vec![0; capacity] } else { Vec::new() };
        let emitter = ParticleEmitter::havok(Arc::new(stage.emitter.clone()), spawn_seed, pose)?;
        let sprites = stage.sprites.iter().map(|_| (0..capacity).map(|_| None).collect()).collect();
        let meshes = stage.meshes.iter().map(|_| (0..capacity).map(|_| None).collect()).collect();
        let trails = stage
            .trails
            .iter()
            .map(|trail| {
                if trail.enabled && trail.material.is_some() {
                    (0..capacity).map(|_| None).collect()
                } else {
                    Vec::new()
                }
            })
            .collect();
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
            trails,
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
            events: HavokEventLists {
                birth: HavokSubscription::new(&declaration, &definition.events.birth),
                collision: HavokSubscription::new(&declaration, &definition.events.collision),
                stop: HavokSubscription::new(&declaration, &definition.events.stop),
                death: HavokSubscription::new(&declaration, &definition.events.death),
                life: definition.events.life.iter().map(|event| HavokSubscription::new(&declaration, event)).collect(),
            },
            deliveries: Vec::new(),
            fx_pending: Vec::new(),
            attractors: Vec::new(),
            fields: [Vec::new(), Vec::new(), Vec::new()],
            spawn_world: None,
            moving: None,
            owner_shapes: None,
            collisions: definition.enable_collisions.then(|| Collisions {
                world: WorldCollide::new(capacity),
                solver: match shape {
                    Some(shape) => {
                        let mut solver = SolveConvexContacts::new(capacity);
                        solver.shapes = vec![shape];
                        ContactSolver::Convex(solver)
                    }
                    None => ContactSolver::Sphere(SolveContacts::new(capacity)),
                },
                tuning: CollideTuning::default(),
                cache: vec![StaticCache::default(); capacity],
                scratch: CollideScratch::default(),
                filter: crate::gameplay::collision_filter::filter_info(definition.collisions.layer) & 0xf7df_ffff,
                partition: SpatialPartition::new(capacity),
                dynamic: DynamicCollide::new(capacity, u32::MAX).expect("the closest-point dispatch enables every shape type"),
            }),
        };
        {
            let stage = &result.declaration.havok_stages[result.index].stage;
            let seed = &mut result.emitter.seed;
            result.fields = [&stage.spawn_fields, &stage.vector_fields, &stage.acceleration_fields]
                .map(|definitions| definitions.iter().map(|definition| super::field::FieldState::new(definition, seed)).collect());
        }
        result.attractors = result
            .definition()
            .stage
            .attractors
            .iter()
            .map(|attractor| super::attractor::AttractorState::new(attractor, result.emitter.seed))
            .collect();
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
                collisions.solver.set_material(self.contact_material);
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
            // 1401e7650: the attractors from the system origin in world
            // space, with a copy of the cycle seed and the spawn seed.
            {
                let space = super::space::ParticleSpace::new(frame.pose.basis, true);
                let mut cycle_seed = self.emitter.cycle_seed();
                let fraction = self.emitter.cycle.fraction;
                let stage = &self.declaration.havok_stages[self.index].stage;
                // 140216430: the fields first, on the same cycle-seed copy;
                // fields aimed at the system target are refused at admission.
                for (states, definitions) in self.fields.iter_mut().zip([&stage.spawn_fields, &stage.vector_fields, &stage.acceleration_fields]) {
                    for (state, definition) in states.iter_mut().zip(definitions) {
                        state.prepare(definition, self.emitter.elapsed, fraction, delta, frame.pose.position, &space, &mut cycle_seed)?;
                    }
                }
                let attractors = &self.declaration.havok_stages[self.index].stage.attractors;
                for (state, attractor) in self.attractors.iter_mut().zip(attractors) {
                    state.prepare(attractor, frame.pose.position, &space, fraction, &mut cycle_seed, &mut self.emitter.seed, delta)?;
                }
            }
            let room = self.pipeline.allocator.capacity() as i32 - self.pending.len() as i32 - self.live.len() as i32;
            let multiplier = frame.rate;
            let burst = definition.stage.burst;
            let mut bindings = Spawner {
                origin_height: frame.pose.position[2],
                spawn_world: self.spawn_world.as_ref(),
                declaration: &self.declaration,
                index: self.index,
                angular: self.pipeline.angular.is_some(),
                gravity: self.integrate.gravity,
                inherited_velocity: &self.inherited_velocity,
                mesh_cursor: &mut self.mesh_cursor,
                owner_shapes: self.owner_shapes.as_deref(),
                emitted: &mut self.pending,
            };
            if burst {
                self.emitter.havok_burst(delta, room, frame.lod, multiplier, &mut bindings)?;
            } else {
                self.emitter.havok_continuous(delta, room, frame.lod, multiplier, &mut bindings)?;
            }
            // 1401e3df0: a birth event per pending record.
            if self.events.birth.listened() {
                for record in &self.pending {
                    let draw = ((crate::random::lcg_step(&mut self.emitter.seed) >> 10) & 0x7fff) as i32;
                    if draw < self.events.birth.threshold {
                        deliver(&self.events.birth, &mut self.deliveries, &mut self.fx_pending, [record.position, record.velocity, [0.; 3]]);
                    }
                }
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

    /// 1401df200: the collision hull centroid (+0x1d10) the bodies are
    /// drawn around; zero without a hull.
    pub fn centroid_offset(&self) -> [f32; 3] {
        self.definition().collisions.hull.as_ref().map_or([0.; 3], |hull| hull.centroid)
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
        // 140211bc0 at the render position and age, identity basis, the
        // system fraction (+0x6b4).
        let fraction = self.emitter.cycle.fraction;
        for (descriptor, slots) in definition.trails.iter().zip(self.trails.iter_mut()) {
            if slots.is_empty() {
                continue;
            }
            for &body in &self.live {
                let Some(particle) = self.particles.get(body as usize) else { continue };
                if let Some(record) = slots[body as usize].as_mut() {
                    record.advance(descriptor, particle.state.age, fraction, delta, particle.state.position, IDENTITY)?;
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
            } else if let Some(collisions) = collisions.as_deref_mut() {
                collisions.release(bodies);
            }
        });
        self.pipeline.allocator.update_allocated();
        let bodies = self.pipeline.allocator.allocated()?.to_vec();
        let dt = if 0. <= dt { dt } else { 0. };
        // The collide mode (1401df5a0): LOW -> 2, MEDIUM/HIGH -> 1,
        // GROUND_TRUTH -> 4.
        let mode = match self.declaration.havok_stages[self.index].collisions.accuracy {
            0 => 2,
            1 | 2 => 1,
            3 => 4,
            accuracy => return Err(format!("unknown Havok particle collision accuracy {accuracy}")),
        };
        let empty = MovingWorld::default();
        let moving = self.moving.as_deref().unwrap_or(&empty);
        // The effects' begin passes: PhysicsCollide gathers the dynamic
        // bodies near each body (140fbf850).
        if let Some(collisions) = self.collisions.as_mut() {
            let Collisions { solver, tuning, filter, partition, dynamic, cache, .. } = collisions;
            let (_, convex) = solver.split_convex();
            let step = Self::dynamic_step(dt, mode, &self.pipeline, *tuning, *filter, convex.1, moving, convex.0);
            let allocator = &self.pipeline.allocator;
            dynamic.begin(&step, partition, &|body| allocator.is_allocated(body), &self.pipeline.linear, cache);
        }
        // arkVectorFieldsHavokEffect (1402165f0): active vector fields set
        // the velocity, active acceleration fields add dt times theirs.
        let stage = &self.declaration.havok_stages[self.index].stage;
        if !stage.vector_fields.is_empty() || !stage.acceleration_fields.is_empty() {
            for &body in &bodies {
                let index = body as usize;
                let cycles = divide(self.pipeline.linear[index][0][3], self.life[index]);
                let age = subtract(cycles, cycles.floor());
                // approximate_: the body's never-written +0x340 seed word.
                let mut seed = 0;
                let p = self.pipeline.linear[index][0];
                let position = [p[0], p[1], p[2]];
                if let Some(velocity) = super::field::sample(&self.fields[1], &stage.vector_fields, position, age, &mut seed)? {
                    let v = &mut self.pipeline.linear[index][1];
                    *v = [velocity[0], velocity[1], velocity[2], v[3]];
                }
                if let Some(acceleration) = super::field::sample(&self.fields[2], &stage.acceleration_fields, position, age, &mut seed)? {
                    let v = &mut self.pipeline.linear[index][1];
                    for i in 0..3 {
                        v[i] = add(multiply(dt, acceleration[i]), v[i]);
                    }
                }
            }
        }
        // arkAttractorsHavokEffect (1401ce600), before the collision effects.
        if !self.attractors.is_empty() {
            let attractors = &self.declaration.havok_stages[self.index].stage.attractors;
            for &body in &bodies {
                let index = body as usize;
                let age = divide(self.pipeline.linear[index][0][3], self.life[index]);
                // approximate_: the strength seed is the body's +0x340 word,
                // sized but never written by the stage.
                let mut seed = 0;
                for (state, attractor) in self.attractors.iter().zip(attractors) {
                    let [p, v] = self.pipeline.linear[index];
                    if let Some(pull) = state.pull([p[0], p[1], p[2]], [v[0], v[1], v[2]]) {
                        let strength = attractor.strength_over_life.sample_particle_rate(age, &mut seed)?;
                        let linear = &mut self.pipeline.linear[index][1];
                        for i in 0..3 {
                            linear[i] = add(multiply(dt, multiply(pull[i], strength)), linear[i]);
                        }
                    }
                }
            }
        }
        // arkAccelerationsHavokEffect (1401ccb30), present when either
        // acceleration is set: the acceleration over the body's cycle age,
        // in the emitter's axes unless worldspace, plus the linear
        // acceleration along its velocity.
        let definition = &self.declaration.havok_stages[self.index];
        if definition.acceleration_over_life.range.iter().flatten().any(|&value| value != 0.)
            || definition.linear_acceleration_over_life.range.iter().any(|&value| value != 0.)
        {
            let basis = self.emitter.pose.sampled.basis;
            for &body in &bodies {
                let index = body as usize;
                let cycles = divide(self.pipeline.linear[index][0][3], self.life[index]);
                let age = subtract(cycles, cycles.floor());
                // approximate_: the body's never-written +0x340 seed word.
                let mut seed = 0;
                let mut a = definition.acceleration_over_life.sample(age, &mut seed)?;
                if !definition.acceleration_worldspace {
                    a = std::array::from_fn(|i| {
                        add(add(multiply(a[1], basis[1][i]), multiply(a[0], basis[0][i])), multiply(a[2], basis[2][i]))
                    });
                }
                let v = self.pipeline.linear[index][1];
                let squared = add(add(multiply(v[0], v[0]), multiply(v[1], v[1])), multiply(v[2], v[2]));
                let floor = f32::from_bits(0x0080_0000);
                let inverse = divide(1., if squared <= floor { floor } else { squared }.sqrt());
                let linear = definition.linear_acceleration_over_life.sample(age, &mut seed)?;
                let velocity = &mut self.pipeline.linear[index][1];
                for i in 0..3 {
                    let total = add(a[i], multiply(multiply(v[i], inverse), linear));
                    velocity[i] = add(multiply(dt, total), velocity[i]);
                }
            }
        }
        // Stage 0 per body: PhysicsCollide's CD pass, then the contact
        // solve, then Integrate.
        if let Some(collisions) = self.collisions.as_mut() {
            let level = level.as_ref().ok_or("Havok particle collisions need the level collision map")?;
            // "Dynamic CD" before "Static CD" (140fc1370).
            {
                let Collisions { solver, tuning, filter, partition, dynamic, cache, world, .. } = &mut *collisions;
                let (states, convex) = solver.split_convex();
                let step = Self::dynamic_step(dt, mode, &self.pipeline, *tuning, *filter, convex.1, moving, convex.0);
                dynamic.collide(&step, partition, &bodies, &self.pipeline.linear, states, cache, world)?;
            }
            if mode == 4 {
                let states = collisions.solver.bodies().to_vec();
                physics_collide::static_ground_truth(
                    dt,
                    collisions.tuning,
                    self.pipeline.radius,
                    collisions.solver.own_material(),
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
                    radii: [self.pipeline.inner_radius, self.pipeline.radius],
                    own: collisions.solver.own_material(),
                    plane: self.pipeline.plane,
                    mode,
                    world: level,
                };
                physics_collide::static_cached(
                    &step,
                    &bodies,
                    &self.pipeline.linear,
                    collisions.solver.bodies(),
                    &mut collisions.world,
                    &mut collisions.cache,
                    &mut collisions.scratch,
                )?;
            }
            match &mut collisions.solver {
                ContactSolver::Sphere(solver) => {
                    solver.solve_spheres(dt, self.pipeline.radius, &bodies, &collisions.world, &mut self.pipeline.linear)?
                }
                ContactSolver::Convex(solver) => {
                    let angular = self.pipeline.angular.as_mut().ok_or("SolveConvexContacts needs AngularMotion")?;
                    solver.solve(dt, self.pipeline.radius, &bodies, &collisions.world, &mut self.pipeline.linear, angular);
                }
            }
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
        // Stage 1: PhysicsCollide's CCD pass, dynamic part first.
        if let Some(collisions) = self.collisions.as_mut() {
            let states = collisions.solver.bodies();
            collisions.dynamic.ccd(
                self.pipeline.inner_radius,
                &bodies,
                &|body| states[body as usize].state,
                moving,
                &mut self.pipeline.linear,
            );
            physics_collide::static_ccd(
                self.pipeline.inner_radius,
                &bodies,
                &|body| states[body as usize].state,
                &mut collisions.cache,
                &mut self.pipeline.linear,
            );
        }
        // The effects' end passes: the partition's spheres and order
        // (140fd75f0) and PhysicsCollide's "Remove dynamic hits" (140fc01d0).
        if let Some(collisions) = self.collisions.as_mut() {
            collisions.partition.end(dt, self.pipeline.radius, &bodies, &self.pipeline.linear);
            collisions.dynamic.end(&bodies);
        }
        self.pipeline.steps += 1;
        self.pipeline.time = add(dt, self.pipeline.time);
        self.post_step(world_gravity)
    }

    /// The inputs of PhysicsCollide's moving-body passes.
    #[allow(clippy::too_many_arguments)]
    fn dynamic_step<'a>(
        dt: f32,
        mode: i32,
        pipeline: &Pipeline,
        tuning: CollideTuning,
        filter: u32,
        [friction, restitution, friction_threshold]: [f32; 3],
        world: &'a MovingWorld,
        convex: Option<(&'a [crate::gameplay::havok_fx::solve_convex_contacts::ConvexShape], &'a [i32])>,
    ) -> DynamicStep<'a> {
        DynamicStep {
            dt,
            tuning,
            radii: [pipeline.inner_radius, pipeline.radius],
            own: [friction, restitution],
            friction_threshold,
            plane: pipeline.plane,
            mode,
            world,
            filter,
            convex,
        }
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
            || self.events.collision.listened()
            || self.events.stop.listened()
            || !self.counters.is_empty();
        let (kill, stop, sleep) = (reaction.kill_on_collision, reaction.stop_on_collision, reaction.sleep_on_collision);
        let first_contacts = if kill { 3 } else { 2 };
        let bodies = self.pipeline.allocator.allocated()?.to_vec();
        for body in bodies {
            let index = body as usize;
            let mut killed = self.kill_all;
            // The collision reactions, from the contact solver's state.
            if let Some(collisions) = self.collisions.as_mut().filter(|_| reacts) {
                let state = collisions.solver.bodies()[index].state;
                'react: {
                    if state <= 0 {
                        break 'react;
                    }
                    if state <= first_contacts {
                        // 1401e3ee0 at the body's last published position.
                        Self::publish_contact(
                            &self.events.collision,
                            &mut self.emitter.seed,
                            &self.particles,
                            &collisions.world,
                            index,
                            &mut self.deliveries,
                            &mut self.fx_pending,
                        );
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
                        if state < 3 {
                            Self::publish_contact(
                                &self.events.stop,
                                &mut self.emitter.seed,
                                &self.particles,
                                &collisions.world,
                                index,
                                &mut self.deliveries,
                                &mut self.fx_pending,
                            );
                        }
                        let linear = &mut self.pipeline.linear[index];
                        linear[1] = [0., 0., 0., linear[1][3]];
                        if let Some(angular) = self.pipeline.angular.as_mut() {
                            angular[index][1] = [0.; 4];
                        }
                        if sleep {
                            collisions.solver.bodies_mut()[index].state = 0x7f;
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
            let previous_age = self.particles.get(index).map_or(0., |particle| particle.state.age);
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
                // 1401e40c0: life events crossed this step, no draw.
                for event in &self.events.life {
                    if previous_age <= event.life && event.life <= age {
                        deliver(event, &mut self.deliveries, &mut self.fx_pending, [position, [v[0], v[1], v[2]], [0.; 3]]);
                    }
                }
                self.live.push(body);
            } else {
                self.dead.push(body);
            }
        }
        // Death events (1401e3ee0 over the dead list), then 1401e3ff0.
        if self.events.death.listened() {
            let world = self.collisions.as_ref().map(|collisions| &collisions.world);
            for &body in &self.dead {
                Self::publish_event(&self.events.death, &mut self.emitter.seed, &self.particles, world, body as usize, &mut self.deliveries, &mut self.fx_pending);
            }
        }
        for &body in &self.dead {
            self.particles.set(body as usize, None);
        }
        let dead = std::mem::take(&mut self.dead);
        self.pipeline.trash(&dead);
        self.dead = dead;
        self.kill_all = false;
        Ok(())
    }

    /// 1401e3ee0 for a body with a collision world.
    fn publish_contact(
        subscription: &HavokSubscription,
        seed: &mut u32,
        particles: &ParticleStage,
        world: &WorldCollide,
        body: usize,
        deliveries: &mut Vec<super::events::Delivery>,
        fx_pending: &mut Vec<(usize, super::fx_event::FxEventRecord)>,
    ) {
        Self::publish_event(subscription, seed, particles, Some(world), body, deliveries, fx_pending);
    }

    /// 1401e3ee0: with listeners, a draw from the spawn seed below the
    /// threshold sends the body's published position (+0x310) and velocity
    /// (+0x320) with its static contact normal (zero without WorldCollide).
    fn publish_event(
        subscription: &HavokSubscription,
        seed: &mut u32,
        particles: &ParticleStage,
        world: Option<&WorldCollide>,
        body: usize,
        deliveries: &mut Vec<super::events::Delivery>,
        fx_pending: &mut Vec<(usize, super::fx_event::FxEventRecord)>,
    ) {
        if !subscription.listened() {
            return;
        }
        let draw = ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as i32;
        if !(draw < subscription.threshold) {
            return;
        }
        let normal = world.map_or([0.; 3], |world| {
            let plane = world.static_contacts[body].plane;
            [plane[0], plane[1], plane[2]]
        });
        let (position, velocity) = particles
            .get(body)
            .map_or(([0.; 3], [0.; 3]), |particle| (particle.state.position, particle.state.velocity));
        deliver(subscription, deliveries, fx_pending, [position, velocity, normal]);
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
            } else if let Some(collisions) = collisions.as_deref_mut() {
                collisions.release(bodies);
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
            // 1402117d0 at the record position with the identity basis.
            for (descriptor, slots) in definition.trails.iter().zip(self.trails.iter_mut()) {
                if slots.is_empty() {
                    continue;
                }
                if definition.constant_seed {
                    seed = record.seed;
                }
                let previous_age = slots[slot].as_ref().map_or(0., |record| record.age);
                slots[slot] = Some(super::trail::TrailRecord::start(
                    previous_age,
                    descriptor,
                    None,
                    true,
                    record.fraction,
                    record.position,
                    IDENTITY,
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
    /// The system origin's height and the level collision for m_spawnCollisions.
    origin_height: f32,
    spawn_world: Option<&'a (Arc<crate::gameplay::navigation::CollisionMap>, Arc<[bool]>)>,
    declaration: &'a ParticleDeclaration,
    index: usize,
    angular: bool,
    gravity: [f32; 4],
    inherited_velocity: &'a super::spawn::SpawnVelocity,
    /// +0x608: the spawn mesh's sequential cursor (1401f9540).
    mesh_cursor: &'a mut usize,
    emitted: &'a mut Vec<PendingSpawn>,
    owner_shapes: Option<&'a super::shape_spawn::OwnerShapes>,
}

/// bevy_backend: the level collision map stands in for the hknp world the
/// spawn queries (140c89cb0 sphere, 140c89b00 ray) run against, filtered by
/// the stage layer (14081a5d0).
impl Spawner<'_> {
    fn world(&self) -> Result<(&crate::gameplay::navigation::CollisionMap, &[bool], u32), String> {
        let (map, owners) = self.spawn_world.ok_or("Havok particle spawn queries need the level collision map")?;
        let layer = self.declaration.havok_stages[self.index].collisions.layer;
        Ok((map, owners, crate::gameplay::collision_filter::filter_info(layer)))
    }

    fn sphere_hit(&self, p: [f32; 3], radius: f32) -> Result<bool, String> {
        let (map, owners, filter) = self.world()?;
        let centre = bevy::math::Vec3::new(p[0], p[2], -p[1]);
        let reach = bevy::math::Vec3::splat(radius);
        let (mut found, mut triangles) = (Vec::new(), Vec::new());
        map.triangles_in(centre - reach, centre + reach, crate::gameplay::collision_filter::QueryFilter::World(filter), owners, &mut found, &mut triangles);
        Ok(triangles.iter().any(|(_, vertices, _)| {
            let (point, _) = crate::gameplay::shape_cast::closest_on_triangle(*vertices, centre);
            (point - centre).length() <= radius
        }))
    }

    fn ray_hit(&self, start: [f32; 3], ray: [f32; 3]) -> Result<bool, String> {
        let (map, owners, filter) = self.world()?;
        let origin = bevy::math::Vec3::new(start[0], start[2], -start[1]);
        let direction = bevy::math::Vec3::new(ray[0], ray[2], -ray[1]);
        let end = origin + direction;
        let (mut found, mut triangles) = (Vec::new(), Vec::new());
        map.triangles_in(origin.min(end), origin.max(end), crate::gameplay::collision_filter::QueryFilter::World(filter), owners, &mut found, &mut triangles);
        Ok(triangles.iter().any(|(_, [a, b, c], _)| {
            // Moller-Trumbore over the segment's parameter range [0, 1].
            let (e1, e2) = (*b - *a, *c - *a);
            let p = direction.cross(e2);
            let det = e1.dot(p);
            if det.abs() < f32::EPSILON {
                return false;
            }
            let inverse = 1. / det;
            let s = origin - *a;
            let u = s.dot(p) * inverse;
            if !(0. ..=1.).contains(&u) {
                return false;
            }
            let q = s.cross(e1);
            let v = direction.dot(q) * inverse;
            if v < 0. || u + v > 1. {
                return false;
            }
            (0. ..=1.).contains(&(e2.dot(q) * inverse))
        }))
    }
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
        // 1401deef0: the owner's physics shapes replace the origin.
        match self.owner_shapes.filter(|_| asset.physics_shapes_spawn.enabled) {
            Some(owner) => {
                let (point, extra) = super::shape_spawn::spawn(&asset.physics_shapes_spawn, owner, fraction, seed)?;
                position = std::array::from_fn(|i| add(point[i], position[i]));
                velocity = std::array::from_fn(|i| add(extra[i], velocity[i]));
            }
            None => position = std::array::from_fn(|i| add(position[i], context.pose.position[i])),
        }
        if let Some(mesh_velocity) = asset.velocity_mesh.sample(context.cycle_seed, seed, fraction)? {
            let mesh_velocity = space.transform(mesh_velocity, asset.velocity_mesh.worldspace);
            velocity = std::array::from_fn(|i| add(mesh_velocity[i], velocity[i]));
        }
        let cone = multiply(asset.velocity_spray.sample(fraction, seed)?, 0.5);
        velocity = super::spawn::spray(velocity, cone, seed);
        let inverse_life = divide(1., life);
        // 1401db2c0: a spawn inside the level geometry is dropped.
        let queries = &definition.spawn_collisions;
        if queries.position_sphere_query && self.sphere_hit(position, f32::from_bits(0x3dcc_cccd))? {
            return Ok(0);
        }
        let particle_seed = crate::random::lcg_step(seed);
        let inherited = self.inherited_velocity.at_point(position);
        velocity = std::array::from_fn(|i| add(velocity[i], inherited[i]));
        // 1401db350: a ray along the velocity (from the system's height when
        // falling, unless the flag is cleared) that hits the level drops it.
        if queries.velocity_ray_cast {
            let squared = add(add(multiply(velocity[0], velocity[0]), multiply(velocity[1], velocity[1])), multiply(velocity[2], velocity[2]));
            let floor = f32::from_bits(0x0080_0000);
            let inverse = divide(1., if squared <= floor { floor } else { squared }.sqrt());
            let mut distance = queries.velocity_ray_cast_distance;
            let down = multiply(velocity[2], inverse);
            let mut start = position;
            if queries.from_origin_height && EPSILON < -down {
                let t = add(divide(subtract(position[2], self.origin_height), -down), queries.offset);
                distance = subtract(distance, t);
                start = [
                    add(position[0], multiply(multiply(velocity[0], inverse), t)),
                    add(position[1], multiply(multiply(velocity[1], inverse), t)),
                    add(position[2], multiply(t, down)),
                ];
            }
            let ray = [
                multiply(multiply(velocity[0], inverse), distance),
                multiply(multiply(velocity[1], inverse), distance),
                multiply(down, distance),
            ];
            if self.ray_hit(start, ray)? {
                return Ok(0);
            }
        }
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

