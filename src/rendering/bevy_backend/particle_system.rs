//! Particle FX instances consume their parent owner, camera and shared RNG.
//! Stage simulation and draw publication retain their slots.
use crate::rendering::particles::definition::ParticleDefinition;
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
    pub probes: Option<&'a crate::probe_lighting::ProbeLightingSet>,
    pub query_visibility: f32,
    pub alpha_scale: f32,
    pub cvars: &'a crate::platform::cvars::CVars,
    pub areas: &'a [super::fog::AreaFogFrame],
}

pub(crate) struct ParticleSystem {
    pub effect: ParticleEffect,
    pub stages: Vec<StageSimulation>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
    pub system_seed: u32,
    pub first_simulation: bool,
    pub publication: Option<ParticlePublication>,
    pub clipping: Option<ParticleClipPlanes>,
    /// 1fa780 initializes system+1d50 to zero; world FX then publishes the
    /// owning entity's linear velocity through 227b90's ordinary packet.
    pub source_velocity: [f32; 3],
    /// Actual owning-entity motion from the FX manager's publication packet.
    pub source_motion: Option<crate::gameplay::particles::spawn::SpawnVelocity>,
    /// 201400 refreshes system+1d00 before the queued simulation task.
    wind: [f32; 3],
    pub water_flags: u32,
    simulated_frame: Option<u32>,
    visible_frame: Option<u32>,
    visibility: crate::rendering::particles::visibility::ParticleVisibilityState,
    force_simulate: bool,
    force_visible: bool,
    probe_query: crate::probe_lighting::ProbeQuery,
    debug_reported_child_draw: bool,
}
impl ParticleSystem {
    pub fn draw_pose(&self, frame: u32) -> Option<ReferenceFrame> {
        (self.visible_frame == Some(frame))
            .then(|| {
                self.publication
                    .as_ref()
                    .filter(|p| p.visible)
                    .map(|p| p.pose)
            })
            .flatten()
    }

    pub fn publish_draws(
        &mut self,
        frame: u32,
        context: ParticleDrawContext<'_>,
        output: &mut Vec<super::particle_pass::ParticleDraw>,
    ) -> Result<(), String> {
        if self.visible_frame != Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let Some(first) = self.stages.first() else {
            return Ok(());
        };
        let owner_scale = crate::float::multiply(
            context.view.owner_scale,
            first
                .declaration
                .distance
                .size
                .sample_visibility(context.camera.emitter_distance)?,
        );
        let alpha_scale = crate::float::multiply(
            context.alpha_scale,
            first
                .declaration
                .distance
                .alpha
                .sample_visibility(context.camera.emitter_distance)?,
        );
        let receiver_bounds = crate::rendering::particles::bounds::system_bounds(
            &self.stages,
            &first.declaration.visibility,
            publication.pose,
            owner_scale,
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
            super::fog_objects::FogObjectParameters::selected(
                &context.areas[selected.index],
                selected.depths,
            )
        })
        .unwrap_or_default();
        // Particle root+1c40 defaults to zero in1fcaa0. Its virtual+88
        // (1fc4c0) then samples the emitter origin, not the live bounds centre.
        // 2b3dc0 ->385f20 ->385da0 uses the shared cached Voronoi query.
        let needs_lighting = self
            .stages
            .iter()
            .flat_map(|stage| stage.declaration.stages[stage.index].sprites.iter())
            .filter(|sprite| sprite.enabled)
            .try_fold(false, |needed, sprite| {
                let Some(name) = sprite.material.as_ref() else {
                    return Ok::<_, String>(needed);
                };
                let material = self
                    .materials
                    .get(name)
                    .ok_or_else(|| format!("particle draw material absent: {name}"))?;
                Ok(needed || material.lighting.is_some())
            })?;
        let lighting = if needs_lighting {
            Some(
                context
                    .probes
                    .ok_or("particle probe network is unavailable")?
                    .interpolate_query_sh(publication.pose.position, &mut self.probe_query)
                    .ok_or("particle origin has no authored probe tetrahedron")?
                    .object_shader_record(),
            )
        } else {
            None
        };
        let view = crate::rendering::particles::SpriteView {
            camera_position: context.camera.position,
            camera_axes: context.camera.axes,
            custom_basis: publication.pose.basis,
            owner_colour: publication.colour,
            // 1fda30 samples both curves at emitter-to-camera distance;
            // 1ee9b0 consumes these as context+d0/d4 (system+1de0/1de4).
            owner_scale,
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
                    alpha_scale,
                    context.camera.admission_plane,
                    sprite.minimal_distance,
                    &mut instances,
                )?;
                if bounds.is_none() {
                    continue;
                }
                let mut transparency =
                    crate::rendering::particles::transparency::TransparencyParameters::read(
                        context.cvars,
                        material.state,
                        true,
                    )?;
                if material.distortion { transparency.mode = 0; }
                let (state, stencil_reference) = crate::render_state::alpha_surface_draw_state(
                    material.state,
                    transparency.mode != 0,
                    self.water_flags & 8 != 0,
                );
                if !self.debug_reported_child_draw
                    && definition.burst
                    && !definition.burst_event.is_empty()
                    && !instances.is_empty()
                {
                    bevy::log::info!(
                        stage = definition.name,
                        event = definition.burst_event,
                        material = name,
                        particles = instances.len(),
                        "Particle child-stage draw published"
                    );
                    self.debug_reported_child_draw = true;
                }
                output.push(super::particle_pass::ParticleDraw {
                    material: material.clone(),
                    lighting: if material.option("fx/opt/use_lighting")? != 0 {
                        lighting
                    } else {
                        None
                    },
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

    pub fn start_selected(
        definition: &ParticleDefinition,
        index: usize,
        offsets: [f32; 4],
        seed: &mut u32,
    ) -> Result<Self, String> {
        let declaration = definition.declarations[index].as_ref().unwrap();
        // 1c4b50 derives decl+618 bit1 from finite, non-looping lifetime
        // stages without an external burst-event producer. 1c4460 derives
        // bit4 from enabled sprite materials with a dark-vision layer.
        let force_simulate = declaration.stages.iter().any(|stage| {
            stage.enabled
                && stage.emitter.cycles != 0
                && !stage.infinite_life
                && !(stage.burst && !stage.burst_event.is_empty())
        });
        let force_visible = declaration
            .stages
            .iter()
            .filter(|stage| stage.enabled)
            .flat_map(|stage| &stage.sprites)
            .filter(|sprite| sprite.enabled)
            .filter_map(|sprite| sprite.material.as_ref())
            .filter_map(|name| definition.materials.get(name))
            .any(|material| material.darkvision_layer != 0);
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
        Ok(Self {
            effect,
            stages,
            materials: definition.materials.clone(),
            system_seed,
            first_simulation: true,
            publication: None,
            clipping: None,
            source_velocity: [0.; 3],
            source_motion: None,
            wind: [0.; 3],
            water_flags: 0,
            simulated_frame: None,
            visible_frame: None,
            visibility: Default::default(),
            force_simulate,
            force_visible,
            probe_query: Default::default(),
            debug_reported_child_draw: false,
        })
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
        frame: &ParticleEffectFrame,
        view_underwater: bool,
        seed: &mut u32,
    ) -> Result<(), String> {
        self.publication = self.effect.update(now, frame, self.finished(), seed)?;
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
    pub fn simulate_in_world(
        &mut self,
        frame: u32,
        world: &crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
        point_query: &mut crate::gameplay::world_point_query::WorldPointQuery,
        ray_query: &mut crate::gameplay::world_ray_query::WorldRayQuery,
        ocean_height: Option<f32>,
        view: &crate::rendering::view_history::ViewMatrices,
        visibility: Option<&crate::tome::RenderAabbQuery<'_>>,
        minimum_angular_size: f32,
        wind: &crate::gameplay::wind::WindSnapshot,
    ) -> Result<(), String> {
        if self.simulated_frame == Some(frame) {
            return Ok(());
        }
        let Some(publication) = self.publication.as_ref().filter(|p| p.visible) else {
            return Ok(());
        };
        let Some(stage) = self.stages.first() else {
            return Ok(());
        };
        let declaration = &stage.declaration;
        let delta = publication.simulation_delta;
        let camera = crate::rendering::particles::view::ParticleCamera::select(
            publication.pose.position,
            view.position,
            view.position,
            [
                std::array::from_fn(|i| -view.view[2][i]),
                std::array::from_fn(|i| -view.view[0][i]),
                std::array::from_fn(|i| view.view[1][i]),
            ],
        );
        let size = declaration
            .distance
            .size
            .sample_visibility(camera.emitter_distance)?;
        let alpha = declaration
            .distance
            .alpha
            .sample_visibility(camera.emitter_distance)?;
        let bounds = crate::rendering::particles::bounds::system_bounds(
            &self.stages,
            &declaration.visibility,
            publication.pose,
            size,
        )?;
        let frustum_visible = || {
            !crate::render_model::transformed_aabb_clip_cull(
                &std::array::from_fn(|i| view.view_projection[i / 4][i % 4]),
                bounds[0],
                bounds[1],
            )
            .culled
        };
        let root_visible = if declaration.visibility.always_visible || self.force_visible {
            frustum_visible()
        } else {
            match visibility {
                Some(query) => query.visible(bounds)?,
                None => frustum_visible(),
            }
        };
        let schedule = self.visibility.prepare(
            &declaration.visibility,
            crate::rendering::particles::visibility::ParticleAdmission {
                bounds,
                camera: camera.position,
                emitter_distance: camera.emitter_distance,
                minimum_angular_size,
                root_visible,
                distance_alpha: alpha,
                distance_size: size,
            },
            delta,
            self.first_simulation,
            self.force_simulate || publication.stop,
        );
        if schedule.flush {
            for stage in &mut self.stages {
                stage.flush();
            }
        }
        if !schedule.update_cycles {
            self.visible_frame = None;
            self.simulated_frame = Some(frame);
            return Ok(());
        }
        // Cycle-only updates require no water clipping query or integration.
        if !schedule.simulate {
            for stage in &mut self.stages {
                stage.advance_cycles(schedule.delta, publication.pose)?;
            }
            self.visible_frame = None;
            self.simulated_frame = Some(frame);
            return Ok(());
        }
        let settings = &self.stages[0].declaration.clipping;
        self.wind = if self.stages[0].declaration.wind.enabled {
            wind.velocity(world, point_query, publication.pose.position)?
        } else {
            [0.; 3]
        };
        let surface = if settings.use_ocean_plane {
            match world.particle_water_height(
                point_query,
                ray_query,
                publication.pose.position,
                settings.water_volume_raycast_distance,
            )? {
                Some(height) => Some(ParticleWaterSurface::Volume(height)),
                None => ocean_height.map(ParticleWaterSurface::Ocean),
            }
        } else {
            None
        };
        self.simulate(frame, surface, self.source_velocity, schedule.delta)?;
        self.visibility.simulated();
        self.visible_frame = schedule.draw.then_some(frame);
        Ok(())
    }

    pub fn simulate(
        &mut self,
        frame: u32,
        surface: Option<ParticleWaterSurface>,
        source_velocity: [f32; 3],
        delta: f32,
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
        for stage_slot in 0..self.stages.len() {
            let stage = &mut self.stages[stage_slot];
            let definition = &stage.declaration.stages[stage.index];
            let worldspace = definition.worldspace;
            let mut delta = delta;
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
            stage.advance_frame_with_sources(
                delta,
                &StageFrame {
                    pose: publication.pose,
                    bounds,
                    planes: plane.as_slice(),
                    source_velocity,
                    rate: publication.rate,
                    lod: 0,
                },
                self.wind,
                self.source_motion.as_ref(),
            )?;
            let mut deliveries = std::mem::take(&mut stage.events.pending);
            for delivery in deliveries.drain(..) {
                if let Some(receiver) = self
                    .stages
                    .iter_mut()
                    .find(|stage| stage.index == delivery.receiver)
                {
                    let worldspace = receiver.declaration.stages[receiver.index].worldspace;
                    receiver.particles.enqueue(
                        delivery.input,
                        worldspace,
                        receiver.owner_pose.position,
                        receiver.owner_pose.basis,
                    );
                }
            }
            self.stages[stage_slot].events.pending = deliveries;
        }
        self.first_simulation = false;
        self.simulated_frame = Some(frame);
        Ok(())
    }
}
