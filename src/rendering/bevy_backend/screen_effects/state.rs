//! CPU player FX lifecycle, separate from the retained GPU distortion passes.
//! The view owner supplies its actual underwater state and camera publication.
use super::super::particle_systems::ParticleSystems;
use super::{sources, PlayingQuad, QuadDraw, ScreenEffectSources};
use crate::{
    assets::fx::screen_quad::ScreenQuadStage,
    gameplay::effects::environment::EnvironmentBlendEffect,
};
use bevy::prelude::{info, warn};

pub(crate) struct ScreenEffectState {
    underwater: bool,
    age: f32,
    frame: Option<u32>,
    quads: Vec<PlayingQuad>,
    particle_heads: [u16; 2],
}
impl Default for ScreenEffectState {
    fn default() -> Self {
        Self {
            underwater: false,
            age: 0.,
            frame: None,
            quads: Vec::new(),
            particle_heads: [u16::MAX; 2],
        }
    }
}
impl ScreenEffectState {
    fn start(
        &mut self,
        definition: &sources::EffectDefinition,
        leaving: bool,
        systems: &mut ParticleSystems,
    ) -> Result<(), String> {
        for group in &definition.groups {
            let offsets = crate::gameplay::effects::groups::sample_offsets(
                &group.timing,
                self.age,
                &mut systems.seed,
            );
            // 222f80 starts particle systems before the group's other families.
            for particle in &group.particles {
                let result = particle
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|definition| {
                        systems.start(
                            &mut self.particle_heads[usize::from(leaving)],
                            definition,
                            offsets,
                        )
                    });
                if let Err(error) = result {
                    warn!("Player particle FX unavailable: {error}");
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
                        &mut systems.seed,
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
        systems: &mut ParticleSystems,
        frame: u32,
        delta: f32,
        underwater: bool,
        camera: crate::gameplay::reference_frame::ReferenceFrame,
    ) -> Result<Vec<QuadDraw>, String> {
        if self.frame != Some(frame) {
            self.age += delta;
            self.quads.retain(|q| q.blend.active);
            if underwater != self.underwater {
                // Stop the opposite FX instance before starting its replacement.
                for quad in &mut self.quads {
                    if quad.leaving == underwater {
                        quad.blend.stop(self.age);
                    }
                }
                systems.stop(self.particle_heads[usize::from(underwater)], self.age);
                self.start(
                    if underwater {
                        &sources.underwater
                    } else {
                        &sources.leave_water
                    },
                    !underwater,
                    systems,
                )?;
                self.underwater = underwater;
                info!("Player screen distortion transition: underwater={underwater}");
            }
            for quad in &mut self.quads {
                quad.blend.update(self.age, 1, &mut systems.seed)?;
            }
            if self.particle_heads.iter().any(|head| *head != u16::MAX) {
                let offset = crate::platform::console::shared()
                    .read()
                    .cvars
                    .float("pm_screenParticleOffset")
                    .ok_or("pm_screenParticleOffset is not registered")?;
                for head in &mut self.particle_heads {
                    systems.update(
                        head,
                        self.age,
                        &crate::gameplay::effects::particle::ParticleEffectFrame {
                            owner: camera,
                            camera: Some(camera),
                            screen_offset: offset,
                            view_mask: 1,
                            allowed_mask: 1,
                            bounds_dirty: false,
                            delta,
                        },
                        underwater,
                    )?;
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
