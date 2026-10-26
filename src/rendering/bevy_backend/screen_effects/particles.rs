//! Player FX particle instances share their parent screen effect's timing and
//! random generator. Stage simulation and draw publication retain their slots.
use super::sources::ParticleDefinition;
use crate::{
    gameplay::{
        effects::particle::{ParticleEffect, ParticleEffectFrame, ParticlePublication},
        particles::simulation::StageSimulation,
        reference_frame::ReferenceFrame,
    },
    rendering::particles::material::ParticleMaterial,
};
use std::{collections::HashMap, sync::Arc};

pub(super) struct PlayingParticle {
    pub leaving: bool,
    pub effect: ParticleEffect,
    pub stages: Vec<StageSimulation>,
    pub materials: HashMap<String, Arc<ParticleMaterial>>,
    pub system_seed: u32,
    pub first_simulation: bool,
    pub publication: Option<ParticlePublication>,
}
impl PlayingParticle {
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
        Ok(())
    }
}
