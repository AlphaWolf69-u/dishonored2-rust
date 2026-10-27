//! Connect ordinary stage emission, stable particles and per-sprite slot state.
//! Physics/field producers must be supplied before their descriptors can
//! be admitted; this owner never invents replacement spawn geometry.
use super::{
    emitter::{EmissionContext, EmitterBindings, ParticleEmitter},
    space::ParticleSpace,
    sprite::SpriteInitialState,
    mesh_appearance::MeshInitialState,
    stage::{Particle, ParticleStage},
    ParticleState,
};
use crate::{
    assets::fx::{particle_declaration::ParticleDeclaration, particle_stage::ParticleStageAsset},
    float::{add, multiply},
    gameplay::reference_frame::ReferenceFrame,
};
use std::sync::Arc;

/// Clipping inputs are in simulation space. Source velocity comes from the
/// system's world-space source (+1d50), before the stage's cycle modulation.
pub struct StageFrame<'a> {
    pub pose: ReferenceFrame,
    pub bounds: [[f32; 3]; 2],
    pub planes: &'a [[f32; 4]],
    pub source_velocity: [f32; 3],
    pub rate: f32,
    pub lod: usize,
}

pub struct StageSimulation {
    pub declaration: Arc<ParticleDeclaration>,
    pub index: usize,
    pub emitter: ParticleEmitter,
    pub particles: ParticleStage,
    /// Outer index is the descriptor's sprite index; inner indices remain
    /// stable when particles die or the free list reuses a previous slot.
    pub sprites: Vec<Vec<Option<SpriteInitialState>>>,
    pub meshes: Vec<Vec<Option<MeshInitialState>>>,
    mesh_cursor: usize,
    periodic_orientation: super::periodic::PeriodicOrientationState,
    inherited_velocity: super::spawn::SpawnVelocity,
    pub events: super::events::StageEvents,
    /// Last frame supplied to this stage (1f4a00), also used by incoming events.
    pub owner_pose: ReferenceFrame,
    /// 1fa780 initializes the source/target point to world zero.
    pub target_position: [f32; 3],
    fields: [Vec<super::field::FieldState>; 3],
    field_velocity: Vec<[f32; 3]>,
}

struct Bindings<'a> {
    definition: &'a ParticleStageAsset,
    events: &'a mut super::events::StageEvents,
    fields: &'a mut [Vec<super::field::FieldState>; 3],
    field_velocity: &'a mut [[f32; 3]],
    owner_position: [f32; 3],
    particles: &'a mut ParticleStage,
    sprites: &'a mut [Vec<Option<SpriteInitialState>>],
    meshes: &'a mut [Vec<Option<MeshInitialState>>],
    owner_basis: [[f32; 3]; 3],
    lod: usize,
    mesh_cursor: &'a mut usize,
    periodic_orientation: &'a mut super::periodic::PeriodicOrientationState,
    inherited_velocity: &'a super::spawn::SpawnVelocity,
}

impl EmitterBindings for Bindings<'_> {
    fn reset_fields(&mut self, seed: &mut u32) -> Result<(), String> {
        for (states, definitions) in self.fields.iter_mut().zip([
            &self.definition.spawn_fields,
            &self.definition.vector_fields,
            &self.definition.acceleration_fields,
        ]) {
            for (state, definition) in states.iter_mut().zip(definitions) {
                state.reset(definition, seed);
            }
        }
        Ok(())
    }
    fn advance_shape(
        &mut self,
        fraction: f32,
        cycle_seed: u32,
        delta: f32,
        seed: &mut u32,
    ) -> Result<(), String> {
        self.periodic_orientation.advance(&self.definition.velocity_periodic_orientation,
            fraction, cycle_seed, delta, seed)
    }
    fn spawn(&mut self, context: EmissionContext, seed: &mut u32) -> Result<usize, String> {
        let definition = self.definition;
        let life = definition.life.sample(context.cycle_fraction, seed)?;
        if !(f32::from_bits(0x3400_0000) <= life) {
            return Ok(self.particles.capacity() - self.particles.len());
        }
        let basis = if definition.worldspace {
            context.pose.basis
        } else {
            self.owner_basis
        };
        let space = ParticleSpace::new(basis, definition.worldspace);
        let mut position = space.transform(
            definition
                .location
                .value
                .sample(context.cycle_fraction, seed)?,
            definition.location.worldspace,
        );
        let mut velocity = space.transform(
            definition
                .velocity
                .value
                .sample(context.cycle_fraction, seed)?,
            definition.velocity.worldspace,
        );
        if let Some((event_position, event_velocity)) = context.event {
            position = std::array::from_fn(|i| add(position[i], event_position[i]));
            velocity = std::array::from_fn(|i| add(velocity[i], event_velocity[i]));
        }
        let mut mesh_colour = [1.; 4];
        if let Some(sample) = definition.spawn_mesh.sample(
            self.mesh_cursor,
            context.cycle_seed,
            seed,
            context.cycle_fraction,
        )? {
            let mesh_position = space.transform(sample.position, definition.spawn_mesh.worldspace);
            let mesh_velocity = space.transform(sample.velocity, definition.spawn_mesh.worldspace);
            position = std::array::from_fn(|i| add(mesh_position[i], position[i]));
            velocity = std::array::from_fn(|i| add(mesh_velocity[i], velocity[i]));
            mesh_colour = sample.colour;
        }
        if let Some(mesh_velocity) = definition.velocity_mesh.sample(context.cycle_seed, seed, context.cycle_fraction)? {
            let mesh_velocity = space.transform(mesh_velocity, definition.velocity_mesh.worldspace);
            velocity = std::array::from_fn(|i| add(mesh_velocity[i], velocity[i]));
        }
        let orientation =
            definition
                .velocity_orientation
                .sample(context.cycle_fraction, &space, seed)?;
        velocity = std::array::from_fn(|i| {
            add(
                add(
                    multiply(velocity[1], orientation[1][i]),
                    multiply(velocity[0], orientation[0][i]),
                ),
                multiply(velocity[2], orientation[2][i]),
            )
        });
        velocity = self.periodic_orientation.apply(velocity);
        let cone = multiply(
            definition
                .velocity_spray
                .sample(context.cycle_fraction, seed)?,
            0.5,
        );
        velocity = super::spawn::spray(velocity, cone, seed);
        let position = std::array::from_fn(|i| add(context.pose.position[i], position[i]));
        let mut particle_seed = crate::random::lcg_step(seed);
        let mut motion_seed = particle_seed;
        if let Some(field) = super::field::sample(
            &self.fields[0],
            &definition.spawn_fields,
            position,
            0.,
            &mut motion_seed,
        )? {
            velocity = std::array::from_fn(|i| add(field[i], velocity[i]));
        }
        particle_seed = motion_seed;
        let inherited = self.inherited_velocity.at_point(position);
        let base_velocity = std::array::from_fn(|i| add(inherited[i], velocity[i]));
        let mut particle = Particle {
            state: ParticleState::new(position, velocity, life),
            base_velocity,
            accumulated_velocity: [0.; 3],
            seed: particle_seed,
        };
        let mut field_velocity = [0.; 3];
        if let Some(field) = super::field::sample(
            &self.fields[1],
            &definition.vector_fields,
            position,
            particle.state.age,
            &mut motion_seed,
        )? {
            field_velocity = field;
        }
        if !particle.state.advance(
            context.remaining_delta,
            definition.infinite_life,
            particle.base_velocity,
            field_velocity,
            &mut particle.accumulated_velocity,
            |age| {
                definition
                    .motion
                    .sample_with_field(age, &mut motion_seed, &space, None, |seed| {
                        super::field::sample(
                            &self.fields[2],
                            &definition.acceleration_fields,
                            position,
                            age,
                            seed,
                        )
                    })
            },
        )? {
            return Ok(self.particles.capacity() - self.particles.len());
        }
        let age = particle.state.age;
        let birth_state = particle.state;
        let Some(slot) = self.particles.insert(particle) else {
            return Ok(0);
        };
        self.field_velocity[slot] = field_velocity;
        let mut sprite_seed = particle_seed;
        for (definition, slots) in definition.sprites.iter().zip(self.sprites.iter_mut()) {
            if !definition.enabled {
                continue;
            }
            if self.definition.constant_seed {
                sprite_seed = particle_seed;
            }
            let mut state = SpriteInitialState::new(
                &definition.spawn,
                context.cycle_fraction,
                self.lod,
                mesh_colour,
                definition.flipbook.flips,
                definition
                    .flipbook
                    .cycle_rate
                    .then_some(&definition.flipbook.initial),
                &mut sprite_seed,
            )?;
            let mut update_seed = particle_seed;
            state.advance(
                &definition.appearance,
                &definition.flipbook,
                age,
                context.remaining_delta,
                &mut update_seed,
            )?;
            slots[slot] = Some(state);
        }
        for (descriptor, slots) in definition.meshes.iter().zip(self.meshes.iter_mut()) {
            if !descriptor.enabled || descriptor.material.is_none() || descriptor.model.is_none() {continue;}
            if self.definition.constant_seed {sprite_seed=particle_seed;}
            let mut state=MeshInitialState::new(&descriptor.spawn,context.cycle_fraction,self.lod,
                mesh_colour,descriptor.flipbook.flips,
                (descriptor.use_flipbook && descriptor.flipbook.cycle_rate).then_some(&descriptor.flipbook.initial),
                &mut sprite_seed)?;
            let mut update_seed=particle_seed;
            state.advance(&descriptor.appearance,&descriptor.flipbook,age,context.remaining_delta,&mut update_seed)?;
            slots[slot]=Some(state);
        }
        self.events.birth(
            &birth_state,
            seed,
            ReferenceFrame {
                position: self.owner_position,
                basis: self.owner_basis,
            },
            definition.worldspace,
        );
        Ok(self.particles.capacity() - self.particles.len())
    }
}

impl StageSimulation {
    pub fn flush(&mut self) {
        self.particles.flush();
        self.events.pending.clear();
        for meshes in &mut self.meshes { for mesh in meshes { *mesh=None; } }
        for sprites in &mut self.sprites {
            for sprite in sprites {
                *sprite = None;
            }
        }
    }

    /// 1ffff0's clock-only branch passes the whole post-delay interval to
    /// 1f6160, without integrating particles or sampling spawn parameters.
    pub fn advance_cycles(&mut self, delta: f32, pose: ReferenceFrame) -> Result<(), String> {
        self.owner_pose = pose;
        let delta = self.emitter.consume_delay(delta);
        self.prepare_pose(delta, pose);
        let mut bindings = Bindings {
            definition: &self.declaration.stages[self.index],
            events: &mut self.events,
            fields: &mut self.fields,
            field_velocity: &mut self.field_velocity,
            owner_position: pose.position,
            particles: &mut self.particles,
            sprites: &mut self.sprites,
            meshes: &mut self.meshes,
            owner_basis: pose.basis,
            lod: 0,
            mesh_cursor: &mut self.mesh_cursor,
            periodic_orientation: &mut self.periodic_orientation,
            inherited_velocity: &self.inherited_velocity,
        };
        self.emitter.advance_clock(delta, &mut bindings)?;
        self.commit_pose(pose);
        Ok(())
    }
    pub fn new(
        declaration: Arc<ParticleDeclaration>,
        index: usize,
        pose: ReferenceFrame,
        system_seed: &mut u32,
    ) -> Result<Self, String> {
        let definition = declaration
            .stages
            .get(index)
            .ok_or("particle stage index absent")?;
        if !definition.enabled {
            return Err("disabled particle stage cannot be instantiated".into());
        }
        let capacity = definition.estimated_capacity()?.min(8192) as usize;
        let mut particles = ParticleStage::new(capacity);
        let mut sprites = definition
            .sprites
            .iter()
            .map(|_| (0..capacity).map(|_| None).collect())
            .collect::<Vec<Vec<Option<SpriteInitialState>>>>();
        let mut meshes = definition.meshes.iter()
            .map(|_| (0..capacity).map(|_| None).collect()).collect::<Vec<Vec<Option<MeshInitialState>>>>();
        // 1efcc0 retains the first draw as its cycle seed and the second as
        // its mutable spawn seed. Periodic orientation precedes field setup.
        let cycle_seed = crate::random::lcg_step(system_seed);
        let mut spawn_seed = crate::random::lcg_step(system_seed);
        let mut periodic_orientation = super::periodic::PeriodicOrientationState::new(
            &definition.velocity_periodic_orientation, cycle_seed, &mut spawn_seed)?;
        let mut fields = [
            &definition.spawn_fields,
            &definition.vector_fields,
            &definition.acceleration_fields,
        ]
        .map(|definitions| {
            definitions
                .iter()
                .map(|definition| super::field::FieldState::new(definition, &mut spawn_seed))
                .collect()
        });
        let mut field_velocity = vec![[0.; 3]; capacity];
        let mut events = super::events::StageEvents::new(&declaration, index);
        let mut mesh_cursor = 0;
        let inherited_velocity = super::spawn::SpawnVelocity::default();
        let mut bindings = Bindings {
            events: &mut events,
            fields: &mut fields,
            field_velocity: &mut field_velocity,
            owner_position: pose.position,
            definition,
            particles: &mut particles,
            sprites: &mut sprites,
            meshes: &mut meshes,
            owner_basis: pose.basis,
            lod: 0,
            mesh_cursor: &mut mesh_cursor,
            periodic_orientation: &mut periodic_orientation,
            inherited_velocity: &inherited_velocity,
        };
        let emitter = ParticleEmitter::new(
            Arc::new(definition.emitter.clone()),
            spawn_seed,
            if definition.worldspace {
                pose
            } else {
                ReferenceFrame::default()
            },
            &mut bindings,
        )?;
        Ok(Self {
            declaration,
            index,
            emitter,
            particles,
            sprites,
            meshes,
            mesh_cursor,
            periodic_orientation,
            inherited_velocity,
            events,
            owner_pose: pose,
            target_position: [0.; 3],
            fields,
            field_velocity,
        })
    }

    /// 1ffff0 prepares pose history once for the whole frame, then limits each
    /// outer slice by 1/60, four percent of cycle duration and its next boundary.
    /// Emission advances the interpolation fraction across all those slices.
    pub fn advance_frame(&mut self, delta: f32, frame: &StageFrame<'_>) -> Result<(), String> {
        self.advance_frame_with_wind(delta, frame, [0.; 3])
    }

    pub fn advance_frame_with_wind(
        &mut self,
        delta: f32,
        frame: &StageFrame<'_>,
        wind: [f32; 3],
    ) -> Result<(), String> {
        self.advance_frame_with_sources(delta, frame, wind, None)
    }

    pub fn advance_frame_with_sources(
        &mut self,
        delta: f32,
        frame: &StageFrame<'_>,
        wind: [f32; 3],
        source: Option<&super::spawn::SpawnVelocity>,
    ) -> Result<(), String> {
        use crate::float::subtract;
        self.owner_pose = frame.pose;
        let delta = self.emitter.consume_delay(delta);
        self.prepare_pose(delta, frame.pose);
        let wind = self.wind(frame, wind)?;
        let source_velocity = self.source_velocity(frame)?;
        if self.declaration.velocity_inheritance.enabled
            && self.declaration.stages[self.index].enable_velocity_inheritance
        {
            let source = source.ok_or(
                "particle velocity inheritance awaits the owning entity's published motion",
            )?;
            // 1f6220 takes a fresh copy, independently of wind/source curves.
            let mut seed = self.emitter.cycle_seed();
            self.inherited_velocity = super::spawn::SpawnVelocity::sample(
                source,
                &self.declaration.velocity_inheritance,
                self.emitter.cycle.fraction,
                &mut seed,
            )?;
            if !self.declaration.stages[self.index].worldspace {
                self.inherited_velocity.make_local(frame.pose);
            }
        }
        let mut remaining = delta;
        while remaining > 0. {
            let duration = self.emitter.cycle.duration;
            let cycle_left = subtract(duration, self.emitter.cycle.elapsed);
            let mut step = remaining
                .min(f32::from_bits(0x3c88_8889))
                .min(multiply(duration, f32::from_bits(0x3d23_d70a)));
            if f32::from_bits(0x3727_c5ac) < cycle_left && cycle_left <= step {
                step = cycle_left;
            }
            if !step.is_finite() || step <= 0. {
                return Err("particle cycle produces an invalid outer time slice".into());
            }
            self.advance_chunk(step, frame, source_velocity, wind)?;
            remaining = subtract(remaining, step);
        }
        self.commit_pose(frame.pose);
        Ok(())
    }

    fn prepare_pose(&mut self, delta: f32, pose: ReferenceFrame) {
        let worldspace = self.declaration.stages[self.index].worldspace;
        self.emitter.pose.set_basis(if worldspace {
            pose.basis
        } else {
            ReferenceFrame::default().basis
        });
        self.emitter.pose.reset_fraction();
        self.emitter
            .pose
            .target_position(if worldspace { pose.position } else { [0.; 3] });
        self.emitter.pose.set_duration(delta);
    }

    fn commit_pose(&mut self, pose: ReferenceFrame) {
        self.emitter
            .pose
            .set_position(if self.declaration.stages[self.index].worldspace {
                pose.position
            } else {
                [0.; 3]
            });
        self.emitter.pose.commit_orientation();
    }

    /// 1f62a0 samples source-over-cycle once before all outer frame slices.
    /// Local stages apply the owner's inverse basis after component products.
    fn source_velocity(&self, frame: &StageFrame<'_>) -> Result<[f32; 3], String> {
        let definition = &self.declaration.stages[self.index];
        let mut seed = self.emitter.cycle_seed();
        let scale = definition
            .source_velocity_over_cycle
            .sample(self.emitter.cycle.fraction, &mut seed)?;
        let value = std::array::from_fn(|i| multiply(frame.source_velocity[i], scale[i]));
        if definition.worldspace {
            return Ok(value);
        }
        let basis = ParticleSpace::new(frame.pose.basis, false).basis(true);
        Ok(std::array::from_fn(|i| {
            add(
                add(
                    multiply(value[1], basis[1][i]),
                    multiply(value[0], basis[0][i]),
                ),
                multiply(value[2], basis[2][i]),
            )
        }))
    }

    /// 1f63b0 uses a local cycle-seed copy. Local stages dot the world wind
    /// with the emitter axes directly; this is not the inherited-velocity inverse.
    fn wind(&self, frame: &StageFrame<'_>, world_wind: [f32; 3]) -> Result<[f32; 3], String> {
        let definition = &self.declaration.stages[self.index];
        if !definition.enable_wind {
            return Ok([0.; 3]);
        }
        let mut seed = self.emitter.cycle_seed();
        let strength = self
            .declaration
            .wind
            .over_cycle
            .sample(self.emitter.cycle.fraction, &mut seed)?;
        let wind = world_wind.map(|value| multiply(strength, value));
        Ok(if definition.worldspace {
            wind
        } else {
            std::array::from_fn(|i| {
                add(
                    add(
                        multiply(wind[1], frame.pose.basis[i][1]),
                        multiply(wind[0], frame.pose.basis[i][0]),
                    ),
                    multiply(wind[2], frame.pose.basis[i][2]),
                )
            })
        })
    }

    /// Existing particles advance before newly emitted particles integrate the
    /// remaining slice immediately in 1f4e00.
    fn advance_chunk(
        &mut self,
        delta: f32,
        frame: &StageFrame<'_>,
        source_velocity: [f32; 3],
        wind: [f32; 3],
    ) -> Result<(), String> {
        let declaration = self.declaration.clone();
        let definition = &declaration.stages[self.index];
        let space = ParticleSpace::new(frame.pose.basis, definition.worldspace);
        let origin = if definition.worldspace {
            frame.pose.position
        } else {
            [0.; 3]
        };
        let target = if definition.worldspace {
            self.target_position
        } else {
            space.transform(
                std::array::from_fn(|i| {
                    crate::float::subtract(self.target_position[i], frame.pose.position[i])
                }),
                true,
            )
        };
        let mut cycle_seed = self.emitter.cycle_seed();
        for (states, definitions) in self.fields.iter_mut().zip([
            &definition.spawn_fields,
            &definition.vector_fields,
            &definition.acceleration_fields,
        ]) {
            for (state, definition) in states.iter_mut().zip(definitions) {
                state.prepare(
                    definition,
                    self.emitter.elapsed,
                    self.emitter.cycle.fraction,
                    delta,
                    if definition.target_position {
                        target
                    } else {
                        origin
                    },
                    &space,
                    &mut cycle_seed,
                )?;
            }
        }
        let field_velocity = &mut self.field_velocity;
        let fields = &self.fields;
        let sprites = &mut self.sprites;
        let meshes = &mut self.meshes;
        let events = &mut self.events;
        let event_seed = &mut self.emitter.seed;
        self.particles.advance_with_events(
            delta,
            self.emitter.simulation_step,
            definition.infinite_life,
            frame.bounds,
            frame.planes,
            |index, particle, seed| {
                if let Some(value) = super::field::sample(
                    &fields[1],
                    &definition.vector_fields,
                    particle.position,
                    particle.age,
                    seed,
                )? {
                    field_velocity[index] = value;
                }
                // 1f5620 samples wind-over-life even for a zero wind vector,
                // before the existing source-over-life parameter samples.
                let strength = declaration.wind.over_life.sample(particle.age, seed)?;
                let force = wind.map(|value| add(0., multiply(strength, value)));
                let source = definition
                    .source_velocity_over_life
                    .sample(particle.age, seed)?;
                Ok((
                    std::array::from_fn(|i| {
                        add(
                            add(field_velocity[index][i], force[i]),
                            multiply(source[i], source_velocity[i]),
                        )
                    }),
                    None,
                ))
            },
            |_, position, age, seed, forces| {
                definition
                    .motion
                    .sample_with_field(age, seed, &space, forces, |seed| {
                        super::field::sample(
                            &fields[2],
                            &definition.acceleration_fields,
                            position,
                            age,
                            seed,
                        )
                    })
            },
            |index, particle, seed| {
                for (sprite, slots) in definition.sprites.iter().zip(sprites.iter_mut()) {
                    if let Some(state) = &mut slots[index] {
                        state.advance(
                            &sprite.appearance,
                            &sprite.flipbook,
                            particle.age,
                            delta,
                            seed,
                        )?;
                    }
                }
                for (mesh,slots) in definition.meshes.iter().zip(meshes.iter_mut()) {
                    if let Some(state)=&mut slots[index] {
                        state.advance(&mesh.appearance,&mesh.flipbook,particle.age,delta,seed)?;
                    }
                }
                Ok(())
            },
            |particle, previous_age, alive| {
                events.integrated(
                    particle,
                    previous_age,
                    alive,
                    event_seed,
                    frame.pose,
                    definition.worldspace,
                );
                Ok(())
            },
        )?;
        let mut bindings = Bindings {
            events: &mut self.events,
            fields: &mut self.fields,
            field_velocity: &mut self.field_velocity,
            owner_position: frame.pose.position,
            definition,
            particles: &mut self.particles,
            sprites: &mut self.sprites,
            meshes: &mut self.meshes,
            owner_basis: frame.pose.basis,
            lod: frame.lod,
            mesh_cursor: &mut self.mesh_cursor,
            periodic_orientation: &mut self.periodic_orientation,
            inherited_velocity: &self.inherited_velocity,
        };
        let free = bindings.particles.capacity() - bindings.particles.len();
        // 1ffff0 calls ordinary emission after integration. The parent's
        // spawnOnlyFromStage flag is not a blanket disable for this path:
        // it only participates in linked parent-stage setup. Such links are
        // still rejected by the reader until their producer is connected.
        if definition.burst && !definition.burst_event.is_empty() {
            let mut inputs = std::mem::take(&mut bindings.particles.inputs);
            self.emitter.event_burst(
                delta,
                free,
                frame.lod,
                frame.rate,
                &inputs,
                definition.use_event_location,
                &definition.event_velocity,
                &definition.event_normal,
                &mut bindings,
            )?;
            inputs.clear();
            bindings.particles.inputs = inputs;
        } else if definition.burst {
            self.emitter
                .burst(delta, free, frame.lod, frame.rate, &mut bindings)?;
        } else {
            self.emitter
                .continuous(delta, free, frame.lod, frame.rate, &mut bindings)?;
        }
        Ok(())
    }
}
