//! Player FX particle instances share their parent screen effect's timing and
//! random generator. Stage simulation and draw publication retain their slots.
use super::sources::ParticleDefinition;
use crate::{
    gameplay::{
        effects::particle::{ParticleEffect, ParticleEffectFrame, ParticlePublication},
        particles::{
            pose::EmitterPose,
            simulation::{StageFrame, StageSimulation},
        },
        reference_frame::ReferenceFrame,
    },
    rendering::particles::{
        clipping::{ParticleClipPlanes, ParticleWaterSurface},
        material::ParticleMaterial,
    },
};
use std::{collections::HashMap, sync::Arc};

/// Inputs supplied by the actual render-view publication.
pub(crate) struct ParticleDrawContext<'a> {
    pub view: &'a crate::rendering::particles::SpriteView,
    pub camera: crate::rendering::particles::view::ParticleCamera,
    pub view_z: [f32; 4],
    pub query_visibility: f32,
    pub alpha_scale: f32,
    pub cvars: &'a crate::platform::cvars::CVars,
    pub areas: &'a [super::super::fog::AreaFogFrame],
}

pub(super) struct PlayingParticle {
    pub leaving: bool,
    pub effect: ParticleEffect,
    pub stages: Vec<StageSimulation>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
    pub system_seed: u32,
    pub first_simulation: bool,
    pub publication: Option<ParticlePublication>,
    pub clipping: Option<ParticleClipPlanes>,
    /// 1fa780 initializes system+1d50 to zero. Player FX publication changes
    /// rate/colour/pose, but supplies no external source-velocity producer.
    pub source_velocity: [f32; 3],
    pub water_flags: u32,
    simulated_frame: Option<u32>,
}
impl PlayingParticle {
    pub fn draw_pose(&self, frame: u32) -> Option<ReferenceFrame> {
        (self.simulated_frame == Some(frame))
            .then(|| {
                self.publication
                    .as_ref()
                    .filter(|p| p.visible)
                    .map(|p| p.pose)
            })
            .flatten()
    }

    pub fn publish_draws(
        &self,
        frame: u32,
        context: ParticleDrawContext<'_>,
        output: &mut Vec<super::super::particle_pass::ParticleDraw>,
    ) -> Result<(), String> {
        if self.simulated_frame != Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let receiver_bounds = crate::rendering::particles::bounds::default_system_bounds(
            &self.stages,
            publication.pose,
            context.view.owner_scale,
        )?;
        let volumes = context
            .areas
            .iter()
            .map(|area| area.bounds)
            .collect::<Vec<_>>();
        let center = std::array::from_fn(|i| {
            crate::float::multiply(
                crate::float::add(receiver_bounds[0][i], receiver_bounds[1][i]),
                0.5,
            )
        });
        let fog = crate::rendering::fog::selection::select(
            &volumes,
            context.camera.position,
            center,
            receiver_bounds,
            context.view_z,
        )
        .map(|selected| {
            super::super::fog_objects::FogObjectParameters::selected(
                &context.areas[selected.index],
                selected.depths,
            )
        })
        .unwrap_or_default();
        let view = crate::rendering::particles::SpriteView {
            camera_position: context.camera.position,
            camera_axes: context.camera.axes,
            custom_basis: publication.pose.basis,
            owner_colour: publication.colour,
            ..*context.view
        };
        for stage in &self.stages {
            let definition = &stage.declaration.stages[stage.index];
            let transform = if definition.worldspace {
                ReferenceFrame::default()
            } else {
                publication.pose
            };
            for (index, sprite) in definition.sprites.iter().enumerate() {
                if !sprite.enabled {
                    continue;
                }
                let Some(name) = sprite.material.as_ref() else {
                    continue;
                };
                let material = self
                    .materials
                    .get(name)
                    .ok_or_else(|| format!("particle draw material absent: {name}"))?;
                let mut instances = Vec::with_capacity(stage.particles.len());
                let bounds = crate::rendering::particles::publication::stage_instances(
                    stage,
                    index,
                    transform,
                    &view,
                    context.query_visibility,
                    context.alpha_scale,
                    context.camera.admission_plane,
                    sprite.minimal_distance,
                    &mut instances,
                )?;
                if bounds.is_none() {
                    continue;
                }
                let transparency =
                    crate::rendering::particles::transparency::TransparencyParameters::read(
                        context.cvars,
                        material.state,
                        true,
                    )?;
                let (state, stencil_reference) = crate::render_state::alpha_surface_draw_state(
                    material.state,
                    transparency.mode != 0,
                    self.water_flags & 8 != 0,
                );
                output.push(super::super::particle_pass::ParticleDraw {
                    material: material.clone(),
                    transparency,
                    state,
                    stencil_reference,
                    grid_level:
                        crate::rendering::particles::geometry::SpriteGrid::level_for_distance(
                            sprite.tessellation_distance,
                            context.camera.emitter_distance,
                        ),
                    instances,
                    clip_plane: self
                        .clipping
                        .as_ref()
                        .map_or(bevy::prelude::Vec4::W, |planes| {
                            bevy::prelude::Vec4::from_array(planes.rendering)
                        }),
                    clip_fade_distance: self.clipping.as_ref().map_or(0., |p| p.fade_distance),
                    fog,
                });
            }
        }
        Ok(())
    }

    pub fn start(
        definition: &ParticleDefinition,
        offsets: [f32; 4],
        leaving: bool,
        seed: &mut u32,
    ) -> Result<Option<Self>, String> {
        let Some(index) =
            crate::gameplay::effects::parameters::reference_index(&definition.declarations, seed)
        else {
            return Ok(None);
        };
        let declaration = definition.declarations[index].as_ref().unwrap();
        let (effect, (stages, system_seed)) = ParticleEffect::new(
            definition.owner.clone(),
            offsets,
            seed,
            |_, mut system_seed| {
                let mut stages = Vec::new();
                for (index, stage) in declaration.stages.iter().enumerate() {
                    if stage.enabled {
                        stages.push(StageSimulation::new(
                            declaration.clone(),
                            index,
                            ReferenceFrame::default(),
                            &mut system_seed,
                        )?);
                    }
                }
                Ok((stages, system_seed))
            },
        )?;
        // These initial states cannot be sampled without their material sheet
        // metadata. Admit no animated sheets until that producer is connected.
        for stage in &stages {
            for sprite in &stage.declaration.stages[stage.index].sprites {
                if !sprite.enabled {
                    continue;
                }
                if let Some(material) = sprite
                    .material
                    .as_ref()
                    .and_then(|n| definition.materials.get(n))
                {
                    if material.option("fx/opt/use_flipbook")? != 0 {
                        return Err(
                            "particle animated material sheet producer is not connected".into()
                        );
                    }
                }
            }
        }
        Ok(Some(Self {
            leaving,
            effect,
            stages,
            materials: definition.materials.clone(),
            system_seed,
            first_simulation: true,
            publication: None,
            clipping: None,
            source_velocity: [0.; 3],
            water_flags: 0,
            simulated_frame: None,
        }))
    }

    /// 1fc170 considers both remaining emission and live particle storage.
    /// Its rate threshold at141ad2734 is exactly 2^-23.
    fn finished(&self) -> bool {
        let rate = self.effect.published_rate();
        self.stages.iter().all(|stage| {
            stage.particles.is_empty()
                && !(f32::from_bits(0x3400_0000) < rate
                    && stage
                        .emitter
                        .cycle
                        .can_emit(stage.emitter.definition.cycles))
        })
    }

    pub fn update(
        &mut self,
        now: f32,
        camera: ReferenceFrame,
        screen_offset: f32,
        delta: f32,
        view_underwater: bool,
        seed: &mut u32,
    ) -> Result<(), String> {
        self.publication = self.effect.update(
            now,
            &ParticleEffectFrame {
                owner: camera,
                camera: Some(camera),
                screen_offset,
                view_mask: 1,
                allowed_mask: 1,
                bounds_dirty: false,
                delta,
            },
            self.finished(),
            seed,
        )?;
        if let Some(stage) = self.stages.first() {
            crate::render_model::update_particle_water_flags(
                &mut self.water_flags,
                stage.declaration.underwater,
                stage.declaration.handle_seen_from_underwater,
                view_underwater,
            );
        }
        Ok(())
    }

    /// Consume a completed physics/ocean lookup at the published FX origin.
    /// None means neither a queried water body nor an active ocean was found;
    /// an unavailable query must be resolved before invoking this method.
    pub fn simulate(
        &mut self,
        frame: u32,
        surface: Option<ParticleWaterSurface>,
        source_velocity: [f32; 3],
    ) -> Result<(), String> {
        if self.simulated_frame == Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let Some(first) = self.stages.first() else {
            self.simulated_frame = Some(frame);
            return Ok(());
        };
        self.clipping = surface.and_then(|surface| {
            ParticleClipPlanes::new(&first.declaration.clipping, publication.pose, surface)
        });
        for stage in &mut self.stages {
            let definition = &stage.declaration.stages[stage.index];
            let worldspace = definition.worldspace;
            let mut delta = publication.simulation_delta;
            if self.first_simulation {
                stage.emitter.pose = EmitterPose::new(if worldspace {
                    publication.pose
                } else {
                    ReferenceFrame::default()
                });
                delta = crate::gameplay::particles::emitter::warmup_delta(
                    delta,
                    definition.warmup,
                    &mut self.system_seed,
                );
            }
            // 1fe2b0 keeps the descriptor box in local space and translates
            // its world-space counterpart by the system origin, without rotation.
            let bounds = stage.declaration.clipping.bounding_box;
            let bounds = [bounds[1], bounds[0]].map(|bound| {
                if worldspace {
                    std::array::from_fn(|i| {
                        crate::float::add(bound[i], publication.pose.position[i])
                    })
                } else {
                    bound
                }
            });
            let plane = self
                .clipping
                .as_ref()
                .map(|p| if worldspace { p.world } else { p.local });
            stage.advance_frame(
                delta,
                &StageFrame {
                    pose: publication.pose,
                    bounds,
                    planes: plane.as_slice(),
                    source_velocity,
                    rate: publication.rate,
                    lod: 0,
                },
            )?;
        }
        self.first_simulation = false;
        self.simulated_frame = Some(frame);
        Ok(())
    }
}
