//! Connect ordinary stage emission, stable particles and per-sprite slot state.
//! Physics/field producers must be supplied before their descriptors can
//! be admitted; this owner never invents replacement spawn geometry.
use super::{
    emitter::{EmissionContext, EmitterBindings, ParticleEmitter},
    space::ParticleSpace,
    sprite::SpriteInitialState,
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
    mesh_cursor: usize,
    inherited_velocity: super::spawn::SpawnVelocity,
}

struct Bindings<'a> {
    definition: &'a ParticleStageAsset,
    particles: &'a mut ParticleStage,
    sprites: &'a mut [Vec<Option<SpriteInitialState>>],
    owner_basis: [[f32; 3]; 3],
    lod: usize,
    mesh_cursor: &'a mut usize,
    inherited_velocity: &'a super::spawn::SpawnVelocity,
}

impl EmitterBindings for Bindings<'_> {
    fn reset_fields(&mut self, _seed: &mut u32) -> Result<(), String> {
        // The stage reader rejects authored vector-field producers. Its
        // default empty lists consume no random values in 1f4ae0.
        Ok(())
    }
    fn advance_shape(
        &mut self,
        _fraction: f32,
        _cycle_seed: u32,
        _delta: f32,
        _seed: &mut u32,
    ) -> Result<(), String> {
        // Default periodic orientation has zero frequency and is disabled by
        // 1f9390. Authored periodic/physics shape descriptors reject. The
        // PMODEL cursor advances only when an accepted particle is spawned.
        Ok(())
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
        let cone = multiply(
            definition
                .velocity_spray
                .sample(context.cycle_fraction, seed)?,
            0.5,
        );
        velocity = super::spawn::spray(velocity, cone, seed);
        let position = std::array::from_fn(|i| add(context.pose.position[i], position[i]));
        let particle_seed = crate::random::lcg_step(seed);
        let mut motion_seed = particle_seed;
        let inherited = self.inherited_velocity.at_point(position);
        let base_velocity = std::array::from_fn(|i| add(inherited[i], velocity[i]));
        let mut particle = Particle {
            state: ParticleState::new(position, velocity, life),
            base_velocity,
            accumulated_velocity: [0.; 3],
            seed: particle_seed,
        };
        if !particle.state.advance(
            context.remaining_delta,
            definition.infinite_life,
            particle.base_velocity,
            [0.; 3],
            &mut particle.accumulated_velocity,
            |age| {
                definition
                    .motion
                    .sample(age, &mut motion_seed, &space, None)
            },
        )? {
            return Ok(self.particles.capacity() - self.particles.len());
        }
        let age = particle.state.age;
        let Some(slot) = self.particles.insert(particle) else {
            return Ok(0);
        };
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
        Ok(self.particles.capacity() - self.particles.len())
    }
}

impl StageSimulation {
    pub fn flush(&mut self) {
        self.particles.flush();
        for sprites in &mut self.sprites {
            for sprite in sprites {
                *sprite = None;
            }
        }
    }

    /// 1ffff0's clock-only branch passes the whole post-delay interval to
    /// 1f6160, without integrating particles or sampling spawn parameters.
    pub fn advance_cycles(&mut self, delta: f32, pose: ReferenceFrame) -> Result<(), String> {
        let delta = self.emitter.consume_delay(delta);
        self.prepare_pose(delta, pose);
        let mut bindings = Bindings {
            definition: &self.declaration.stages[self.index],
            particles: &mut self.particles,
            sprites: &mut self.sprites,
            owner_basis: pose.basis,
            lod: 0,
            mesh_cursor: &mut self.mesh_cursor,
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
        if definition.enable_velocity_inheritance {
            return Err("particle velocity inheritance provider is not connected".into());
        }
        if !definition.burst_event.is_empty() {
            return Err("particle burst-event provider is not connected".into());
        }
        let capacity = definition.estimated_capacity()?.min(8192) as usize;
        let mut particles = ParticleStage::new(capacity);
        let mut sprites = definition
            .sprites
            .iter()
            .map(|_| (0..capacity).map(|_| None).collect())
            .collect::<Vec<Vec<Option<SpriteInitialState>>>>();
        // 1efcc0 retains the first draw as its cycle seed and the second as
        // its mutable spawn seed. Field/shape defaults consume neither.
        let _cycle_seed = crate::random::lcg_step(system_seed);
        let spawn_seed = crate::random::lcg_step(system_seed);
        let mut mesh_cursor = 0;
        let inherited_velocity = super::spawn::SpawnVelocity::default();
        let mut bindings = Bindings {
            definition,
            particles: &mut particles,
            sprites: &mut sprites,
            owner_basis: pose.basis,
            lod: 0,
            mesh_cursor: &mut mesh_cursor,
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
            mesh_cursor,
            inherited_velocity,
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
        use crate::float::subtract;
        let delta = self.emitter.consume_delay(delta);
        self.prepare_pose(delta, frame.pose);
        let wind = self.wind(frame, wind)?;
        let source_velocity = self.source_velocity(frame)?;
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
        let sprites = &mut self.sprites;
        self.particles.advance_with_forces(
            delta,
            self.emitter.simulation_step,
            definition.infinite_life,
            frame.bounds,
            frame.planes,
            |_, particle, seed| {
                // 1f5620 samples wind-over-life even for a zero wind vector,
                // before the existing source-over-life parameter samples.
                let strength = declaration.wind.over_life.sample(particle.age, seed)?;
                let force = wind.map(|value| add(0., multiply(strength, value)));
                let source = definition
                    .source_velocity_over_life
                    .sample(particle.age, seed)?;
                Ok((
                    std::array::from_fn(|i| multiply(source[i], source_velocity[i])),
                    Some(force),
                ))
            },
            |_, age, seed, forces| definition.motion.sample(age, seed, &space, forces),
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
                Ok(())
            },
        )?;
        let mut bindings = Bindings {
            definition,
            particles: &mut self.particles,
            sprites: &mut self.sprites,
            owner_basis: frame.pose.basis,
            lod: frame.lod,
            mesh_cursor: &mut self.mesh_cursor,
            inherited_velocity: &self.inherited_velocity,
        };
        let free = bindings.particles.capacity() - bindings.particles.len();
        // 1ffff0 calls ordinary emission after integration. The parent's
        // spawnOnlyFromStage flag is not a blanket disable for this path:
        // it only participates in linked parent-stage setup. Such links are
        // still rejected by the reader until their producer is connected.
        if definition.burst {
            self.emitter
                .burst(delta, free, frame.lod, frame.rate, &mut bindings)?;
        } else {
            self.emitter
                .continuous(delta, free, frame.lod, frame.rate, &mut bindings)?;
        }
        Ok(())
    }
}
