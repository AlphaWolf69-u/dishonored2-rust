//! CPU player FX lifecycle, separate from the retained GPU distortion passes.
//! The view owner supplies its actual underwater state and camera publication.
use super::{particles, sources, PlayingQuad, QuadDraw, ScreenEffectSources};
use crate::{
    assets::fx::screen_quad::ScreenQuadStage,
    gameplay::effects::environment::EnvironmentBlendEffect,
};
use bevy::prelude::{info, warn};

#[derive(Default)]
pub(crate) struct ScreenEffectState {
    underwater: bool,
    age: f32,
    seed: u32,
    frame: Option<u32>,
    quads: Vec<PlayingQuad>,
    particles: Vec<particles::PlayingParticle>,
    query: Option<crate::gameplay::world_point_query::WorldPointQuery>,
}
impl ScreenEffectState {
    /// Execute against the physics publication for this exact view/frame.
    /// Unsupported fallback inputs remain explicit errors, never dry hits.
    pub(crate) fn simulate_particles(
        &mut self,
        frame: u32,
        world: &crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
    ) -> Result<(), String> {
        if self.particles.is_empty() {
            return Ok(());
        }
        if self.query.is_none() {
            self.query = Some(crate::gameplay::world_point_query::WorldPointQuery::new(
                u32::MAX,
            )?);
        }
        let query = self.query.as_mut().unwrap();
        let mut error = None;
        for particle in &mut self.particles {
            let result = (|| {
                let Some(publication) = particle.publication.as_ref().filter(|p| p.visible) else {
                    return Ok(());
                };
                let Some(stage) = particle.stages.first() else {
                    return Ok(());
                };
                let settings = &stage.declaration.clipping;
                let surface = if settings.use_ocean_plane {
                    match world.particle_water_body_height(query, publication.pose.position)? {
                        Some(height) => Some(
                            crate::rendering::particles::clipping::ParticleWaterSurface::Volume(
                                height,
                            ),
                        ),
                        None if settings.water_volume_raycast_distance > 0. => {
                            return Err("particle water query requires the downward-ray fallback"
                                .to_owned());
                        }
                        None => {
                            return Err(
                                "particle water query awaits the active ocean context".to_owned()
                            )
                        }
                    }
                } else {
                    None
                };
                particle.simulate(frame, surface, particle.source_velocity)
            })();
            if let Err(failure) = result {
                error.get_or_insert(failure);
            }
        }
        error.map_or(Ok(()), Err)
    }

    fn start(
        &mut self,
        definition: &sources::EffectDefinition,
        leaving: bool,
    ) -> Result<(), String> {
        for group in &definition.groups {
            let offsets = crate::gameplay::effects::groups::sample_offsets(
                &group.timing,
                self.age,
                &mut self.seed,
            );
            // 222f80 starts particle systems before the group's other families.
            for particle in &group.particles {
                let result = particle
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|definition| {
                        particles::PlayingParticle::start(
                            definition,
                            offsets,
                            leaving,
                            &mut self.seed,
                        )
                    });
                match result {
                    Ok(Some(particle)) => self.particles.push(particle),
                    Ok(None) => {}
                    Err(error) => warn!("Player particle FX unavailable: {error}"),
                }
            }
            for quad in &group.quads {
                self.quads.push(PlayingQuad {
                    material: quad.material,
                    stage: quad.stage,
                    leaving,
                    blend: EnvironmentBlendEffect::new(
                        quad.blend.clone(),
                        offsets,
                        1,
                        &mut self.seed,
                    )?,
                });
            }
        }
        Ok(())
    }
    /// 821350's enter/leave transition and 21f820's retirement-before-update.
    pub(crate) fn advance(
        &mut self,
        sources: &ScreenEffectSources,
        frame: u32,
        delta: f32,
        underwater: bool,
        camera: crate::gameplay::reference_frame::ReferenceFrame,
    ) -> Result<Vec<QuadDraw>, String> {
        if self.frame != Some(frame) {
            self.age += delta;
            self.quads.retain(|q| q.blend.active);
            self.particles.retain(|p| p.effect.active);
            if underwater != self.underwater {
                // Stop the opposite FX instance before starting its replacement.
                for quad in &mut self.quads {
                    if quad.leaving == underwater {
                        quad.blend.stop(self.age);
                    }
                }
                for particle in &mut self.particles {
                    if particle.leaving == underwater {
                        particle.effect.stop(self.age);
                    }
                }
                self.start(
                    if underwater {
                        &sources.underwater
                    } else {
                        &sources.leave_water
                    },
                    !underwater,
                )?;
                self.underwater = underwater;
                info!("Player screen distortion transition: underwater={underwater}");
            }
            for quad in &mut self.quads {
                quad.blend.update(self.age, 1, &mut self.seed)?;
            }
            if !self.particles.is_empty() {
                let offset = crate::platform::console::shared()
                    .read()
                    .cvars
                    .float("pm_screenParticleOffset")
                    .ok_or("pm_screenParticleOffset is not registered")?;
                for particle in &mut self.particles {
                    particle.update(self.age, camera, offset, delta, &mut self.seed)?;
                }
            }
            self.frame = Some(frame);
        }
        let mut draws = Vec::new();
        for stage in [
            ScreenQuadStage::MotionVectorBegin,
            ScreenQuadStage::MotionVectorEnd,
        ] {
            // 31da70's view publication has four slots for each stage.
            draws.extend(
                self.quads
                    .iter()
                    .filter(|q| q.stage == stage && q.blend.blend != 0.)
                    .take(4)
                    .map(|q| QuadDraw {
                        material: q.material,
                        blend: q.blend.blend,
                    }),
            );
        }
        Ok(draws)
    }
}
