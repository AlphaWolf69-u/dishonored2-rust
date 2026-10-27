//! One particle-family pool for player and world FX. Parent effects own chain
//! heads; the shared manager owns slots, random state and physics queries.
use super::{
    particle_pass::ParticleDraw,
    particle_system::{ParticleDrawContext, ParticleSystem},
};
use bevy::{
    prelude::{Entity, Resource},
    render::extract_resource::ExtractResource,
};
use std::sync::Arc;

pub(crate) struct ParticleQueryPublication {
    pub camera: Entity,
    pub frame: u32,
    pub state: Option<crate::tome::RenderQueryState>,
    pub wind: crate::gameplay::wind::WindSnapshot,
    pub probes: Option<Arc<crate::probe_lighting::ProbeLightingSet>>,
}
#[derive(Resource, Clone, Default, ExtractResource)]
pub(crate) struct ParticleQueryFrame(pub Option<Arc<ParticleQueryPublication>>);
use crate::{
    gameplay::{
        effects::{particle::ParticleEffectFrame, pool::EffectPoolLinks},
        reference_frame::ReferenceFrame,
    },
    rendering::particles::definition::ParticleDefinition,
};

pub(crate) struct ParticleSystems {
    pub seed: u32,
    links: EffectPoolLinks,
    slots: Vec<Option<ParticleSystem>>,
    query: Option<crate::gameplay::world_point_query::WorldPointQuery>,
    ray: crate::gameplay::world_ray_query::WorldRayQuery,
}
impl Default for ParticleSystems {
    fn default() -> Self {
        let links = EffectPoolLinks::particles();
        Self {
            seed: 0,
            slots: (0..links.next.len()).map(|_| None).collect(),
            links,
            query: None,
            ray: Default::default(),
        }
    }
}
impl ParticleSystems {
    pub fn start(
        &mut self,
        head: &mut u16,
        definition: &ParticleDefinition,
        offsets: [f32; 4],
    ) -> Result<(), String> {
        // 222f80 selects the declaration before acquiring a slot; a full pool
        // consumes the selection draw but none of 227cd0's setup samples.
        let Some(index) = crate::gameplay::effects::parameters::reference_index(
            &definition.declarations,
            &mut self.seed,
        ) else {
            return Ok(());
        };
        if !self.links.acquire(head)? {
            return Ok(());
        }
        match ParticleSystem::start_selected(definition, index, offsets, &mut self.seed) {
            Ok(system) => self.slots[usize::from(*head)] = Some(system),
            Err(error) => {
                self.links.release_head(head)?;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn stop(&mut self, mut head: u16, now: f32) {
        while head != u16::MAX {
            if let Some(system) = self.slots[usize::from(head)].as_mut() {
                system.effect.stop(now);
            }
            head = self.links.next[usize::from(head)];
        }
    }
    pub fn clear(&mut self, head: &mut u16) -> Result<(), String> {
        while *head != u16::MAX {
            self.slots[usize::from(*head)] = None;
            self.links.release_head(head)?;
        }
        Ok(())
    }
    pub fn update(
        &mut self,
        head: &mut u16,
        now: f32,
        frame: &ParticleEffectFrame,
        underwater: bool,
    ) -> Result<u32, String> {
        self.update_with_motion(head, now, frame, underwater, None)
    }

    pub fn update_with_motion(
        &mut self,
        head: &mut u16,
        now: f32,
        frame: &ParticleEffectFrame,
        underwater: bool,
        motion: Option<&crate::gameplay::particles::spawn::SpawnVelocity>,
    ) -> Result<u32, String> {
        let slots = &mut self.slots;
        let seed = &mut self.seed;
        self.links.update_chain(head, |index| {
            let slot = &mut slots[usize::from(index)];
            let system = slot.as_mut().ok_or("particle pool slot absent")?;
            if system.effect.active {
                system.source_motion = motion.copied();
                system.source_velocity = motion.map_or([0.; 3], |motion| motion.linear);
                system.update(now, frame, underwater, seed)?;
                Ok(true)
            } else {
                *slot = None;
                Ok(false)
            }
        })
    }
    pub fn capacity(&self) -> (usize, usize) {
        let mut instances = 0;
        let mut draws = 0;
        for system in self.slots.iter().flatten() {
            for stage in &system.stages {
                let sprites = stage.declaration.stages[stage.index]
                    .sprites
                    .iter()
                    .filter(|sprite| sprite.enabled && sprite.material.is_some())
                    .count();
                let meshes=stage.declaration.stages[stage.index].meshes.iter()
                    .filter(|mesh|mesh.enabled&&mesh.material.is_some()&&mesh.model.is_some()).count();
                instances += stage.particles.capacity() * (sprites+meshes);
                draws += sprites+meshes;
            }
        }
        (instances, draws)
    }
    pub fn publish_draws<'a>(
        &mut self,
        frame: u32,
        mut context: impl FnMut(ReferenceFrame) -> Result<ParticleDrawContext<'a>, String>,
    ) -> Result<Vec<ParticleDraw>, String> {
        let mut draws = Vec::new();
        for system in self.slots.iter_mut().flatten() {
            if let Some(pose) = system.draw_pose(frame) {
                system.publish_draws(frame, context(pose)?, &mut draws)?;
            }
        }
        Ok(draws)
    }
    pub fn simulate(
        &mut self,
        frame: u32,
        world: &crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
        ocean_height: Option<f32>,
        view: &crate::rendering::view_history::ViewMatrices,
        visibility: Option<&crate::tome::RenderQueryState>,
        wind: &crate::gameplay::wind::WindSnapshot,
    ) -> Result<(), String> {
        if self.links.allocated_count == 0 {
            return Ok(());
        }
        if self.query.is_none() {
            self.query = Some(crate::gameplay::world_point_query::WorldPointQuery::new(
                u32::MAX,
            )?);
        }
        let query = self.query.as_mut().unwrap();
        let visibility = visibility
            .map(crate::tome::RenderAabbQuery::from_state)
            .transpose()?;
        // 239fa0 prepares this per-view value once before iterating systems.
        let minimum_angular_size = crate::rendering::particles::visibility::minimum_angular_size(
            2. * (1. / view.projection[1][1]).atan(),
        );
        let mut error = None;
        for system in self.slots.iter_mut().flatten() {
            if let Err(failure) = system.simulate_in_world(
                frame,
                world,
                query,
                &mut self.ray,
                ocean_height,
                view,
                visibility.as_ref(),
                minimum_angular_size,
                wind,
            ) {
                error.get_or_insert(failure);
            }
        }
        error.map_or(Ok(()), Err)
    }
}
