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
    /// Point/ray query scratch, one per simulation worker. Each query only
    /// holds per-call work lists, so which worker simulates a system does not
    /// change its result.
    workers: Vec<(
        crate::gameplay::world_point_query::WorldPointQuery,
        crate::gameplay::world_ray_query::WorldRayQuery,
    )>,
}
impl Default for ParticleSystems {
    fn default() -> Self {
        let links = EffectPoolLinks::particles();
        Self {
            seed: 0,
            slots: (0..links.next.len()).map(|_| None).collect(),
            links,
            workers: Vec::new(),
        }
    }
}
impl ParticleSystems {
    pub(super) fn allocated_count(&self) -> u16 { self.links.allocated_count }

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
                // World-material meshes are model surfaces, outside the FX pass.
                let meshes=stage.declaration.stages[stage.index].meshes.iter()
                    .filter(|mesh|mesh.enabled&&mesh.model.is_some()
                        &&mesh.material.as_ref().is_some_and(|name|!system.world_mesh_materials.contains(name))).count();
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
    ) -> Result<(Vec<ParticleDraw>, Vec<super::particle_system::WorldMeshDraw>), String> {
        let mut draws = Vec::new();
        let mut world_meshes = Vec::new();
        for system in self.slots.iter_mut().flatten() {
            if let Some(pose) = system.draw_pose(frame) {
                system.publish_draws(frame, context(pose)?, &mut draws, &mut world_meshes)?;
            }
        }
        Ok((draws, world_meshes))
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
        let visibility = visibility
            .map(crate::tome::RenderAabbQuery::from_state)
            .transpose()?;
        // 239fa0 prepares this per-view value once before iterating systems.
        let minimum_angular_size = crate::rendering::particles::visibility::minimum_angular_size(
            2. * (1. / view.projection[1][1]).atan(),
        );
        // Systems are independent (each reads the shared world snapshot and
        // writes only itself), so they are simulated on several workers.
        // Errors are reported in slot order, as the serial loop did.
        // DH2_SERIAL_PARTICLE_SIMULATION=1 keeps one worker (measurement).
        static SERIAL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let mut systems: Vec<&mut ParticleSystem> = self.slots.iter_mut().flatten().collect();
        let pool = bevy::tasks::ComputeTaskPool::get();
        let workers = if *SERIAL.get_or_init(|| {
            std::env::var("DH2_SERIAL_PARTICLE_SIMULATION").as_deref() == Ok("1")
        }) {
            1
        } else {
            // This thread also runs scope tasks while it waits.
            (pool.thread_num() + 1).min(systems.len().div_ceil(4)).max(1)
        };
        while self.workers.len() < workers {
            self.workers.push((
                crate::gameplay::world_point_query::WorldPointQuery::new(u32::MAX)?,
                Default::default(),
            ));
        }
        let chunk = systems.len().div_ceil(workers).max(1);
        let visibility = visibility.as_ref();
        let simulate = |chunk_index: usize,
                        systems: &mut [&mut ParticleSystem],
                        (query, ray): &mut (
            crate::gameplay::world_point_query::WorldPointQuery,
            crate::gameplay::world_ray_query::WorldRayQuery,
        )| {
            let mut first = None;
            for (offset, system) in systems.iter_mut().enumerate() {
                if let Err(failure) = system.simulate_in_world(
                    frame,
                    world,
                    query,
                    ray,
                    ocean_height,
                    view,
                    visibility,
                    minimum_angular_size,
                    wind,
                ) {
                    first.get_or_insert((chunk_index * chunk + offset, failure));
                }
            }
            first
        };
        let errors = if workers == 1 {
            vec![simulate(0, &mut systems, &mut self.workers[0])]
        } else {
            pool.scope(|scope| {
                for (chunk_index, (systems, scratch)) in systems
                    .chunks_mut(chunk)
                    .zip(self.workers.iter_mut())
                    .enumerate()
                {
                    let simulate = &simulate;
                    scope.spawn(async move { simulate(chunk_index, systems, scratch) });
                }
            })
        };
        errors
            .into_iter()
            .flatten()
            .min_by_key(|(index, _)| *index)
            .map_or(Ok(()), |(_, failure)| Err(failure))
    }
}
