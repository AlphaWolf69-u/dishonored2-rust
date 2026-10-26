//! Connect ordinary stage emission, stable particles and per-sprite slot state.
//! Physics/mesh/field producers must be supplied before their descriptors can
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

/// Inputs already expressed in the stage's simulation space. The system owner
/// prepares clipping planes and source velocity from its world/view context.
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
}

struct Bindings<'a> {
    definition: &'a ParticleStageAsset,
    particles: &'a mut ParticleStage,
    sprites: &'a mut [Vec<Option<SpriteInitialState>>],
    owner_basis: [[f32; 3]; 3],
    lod: usize,
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
        // 1f9390. Authored periodic/mesh/physics shape descriptors reject.
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
        let position = space.transform(
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
        let mut particle = Particle {
            state: ParticleState::new(position, velocity, life),
            base_velocity: velocity,
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
                [1.; 4],
                definition.flipbook.flips,
                None,
                &mut sprite_seed,
            )?;
            let mut update_seed = particle_seed;
            state.advance_rotation(
                &definition.appearance,
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
        if definition.enable_wind || definition.enable_velocity_inheritance {
            return Err("particle wind/inheritance providers are not connected".into());
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
        let mut bindings = Bindings {
            definition,
            particles: &mut particles,
            sprites: &mut sprites,
            owner_basis: pose.basis,
            lod: 0,
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
        })
    }

    /// 1ffff0 prepares pose history once for the whole frame, then limits each
    /// outer slice by 1/60, four percent of cycle duration and its next boundary.
    /// Emission advances the interpolation fraction across all those slices.
    pub fn advance_frame(&mut self, delta: f32, frame: &StageFrame<'_>) -> Result<(), String> {
        use crate::float::subtract;
        let worldspace = self.declaration.stages[self.index].worldspace;
        let delta = self.emitter.consume_delay(delta);
        self.emitter.pose.set_basis(if worldspace {
            frame.pose.basis
        } else {
            ReferenceFrame::default().basis
        });
        self.emitter.pose.reset_fraction();
        self.emitter.pose.target_position(if worldspace {
            frame.pose.position
        } else {
            [0.; 3]
        });
        self.emitter.pose.set_duration(delta);
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
            self.advance_chunk(step, frame)?;
            remaining = subtract(remaining, step);
        }
        self.emitter.pose.set_position(if worldspace {
            frame.pose.position
        } else {
            [0.; 3]
        });
        self.emitter.pose.commit_orientation();
        Ok(())
    }

    /// Existing particles advance before newly emitted particles integrate the
    /// remaining slice immediately in 1f4e00.
    fn advance_chunk(&mut self, delta: f32, frame: &StageFrame<'_>) -> Result<(), String> {
        let declaration = self.declaration.clone();
        let definition = &declaration.stages[self.index];
        let space = ParticleSpace::new(frame.pose.basis, definition.worldspace);
        let sprites = &mut self.sprites;
        self.particles.advance(
            delta,
            self.emitter.simulation_step,
            definition.infinite_life,
            frame.bounds,
            frame.planes,
            |_, particle, seed| {
                // 1f5620 samples global wind strength before source-over-life
                // even when this stage's wind vector is disabled/zero.
                crate::random::lcg_step(seed);
                crate::random::lcg_step(seed);
                let source = definition
                    .source_velocity_over_life
                    .sample(particle.age, seed)?;
                Ok(std::array::from_fn(|i| {
                    multiply(source[i], frame.source_velocity[i])
                }))
            },
            |_, age, seed| definition.motion.sample(age, seed, &space, None),
            |index, particle, seed| {
                for (sprite, slots) in definition.sprites.iter().zip(sprites.iter_mut()) {
                    if let Some(state) = &mut slots[index] {
                        state.advance_rotation(&sprite.appearance, particle.age, delta, seed)?;
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
        };
        let free = bindings.particles.capacity() - bindings.particles.len();
        if !definition.spawn_only_from_stage {
            if definition.burst {
                self.emitter
                    .burst(delta, free, frame.lod, frame.rate, &mut bindings)?;
            } else {
                self.emitter
                    .continuous(delta, free, frame.lod, frame.rate, &mut bindings)?;
            }
        }
        Ok(())
    }
}
