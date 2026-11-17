//!6521b0/651020: the fluid manager's four source query rings and force walk.
use super::*;
use crate::gameplay::{
    aerodynamics::MotionMass,
    collision_user_data::CollisionUserData,
    game_clock::GameClock,
    water_forces::{Input, Scratch},
};
use crate::rendering::ocean::{OceanQueryHandle, ocean_query_manager};
#[derive(Clone)]
pub(in crate::app) struct WaterImpact {
    pub body: u32,
    pub water: ComponentHandle,
    pub owner: crate::gameplay::entity_lifetime::EntityId,
    pub source_material: Arc<crate::assets::entities::physics_material::resources::Definition>,
    pub target_material: Arc<crate::assets::entities::physics_material::resources::Definition>,
    pub point: [f32; 3],
    pub normal: [f32; 3],
    pub direction: [f32; 3],
    pub speed: f32,
    pub mass: f32,
    pub body_flags: u32,
}
impl std::fmt::Debug for WaterImpact {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        f.debug_struct("WaterImpact")
            .field("body",&self.body).field("water",&self.water)
            .field("owner",&self.owner)
            .field("source_material",&self.source_material.name)
            .field("target_material",&self.target_material.name)
            .field("point",&self.point).field("normal",&self.normal)
            .field("direction",&self.direction).field("speed",&self.speed)
            .field("mass",&self.mass).field("body_flags",&self.body_flags).finish()
    }
}
#[derive(Default)]
pub(super) struct WaterSimulation {
    rings: [Vec<OceanQueryHandle>; 4],
    ring: usize,
    points: Vec<[f32; 2]>,
    results: Vec<[f32; 4]>,
    result_counts: Vec<usize>,
    scratch: Scratch,
    changes: Vec<(u32, [f32; 4], [f32; 4])>,
    marker: Option<i32>,
    prepared: bool,
    complete: bool,
}
impl WaterSimulation {
    pub(super) fn release(&mut self) -> Result<(), String> {
        let mut shared = ocean_query_manager()
            .lock()
            .map_err(|_| "water ocean owner poisoned")?;
        for ring in &mut self.rings {
            for handle in ring.iter_mut() {
                shared.queue.release(handle)?
            }
            ring.clear();
        }
        self.points.clear();
        self.results.clear();
        self.result_counts.clear();
        self.changes.clear();
        self.marker = None;
        self.prepared = false;
        self.complete = false;
        Ok(())
    }
}
fn random(seed: &mut u32) -> f32 {
    crate::random::lcg_step(seed) as f32 * f32::from_bits(0x2f800000)
}
fn jitter(
    plane: [f32; 4],
    center: [f32; 4],
    amplitude: f32,
    seed: &mut u32,
) -> ([f32; 4], [f32; 3]) {
    //651020 consumes z,y,x then the height sample, all from af0390.
    let z = random(seed) * 2.;
    let y = random(seed);
    let x = random(seed);
    let height = random(seed);
    let d = plane[3] + height * amplitude;
    let distance = (center[2] * plane[2] + center[0] * plane[0]) + (d + center[1] * plane[1]);
    let point = [
        center[0] + plane[0] * (0. - distance),
        center[1] + plane[1] * (0. - distance),
        center[2]
            + plane[2]
                * (0.
                    - (((center[1] * plane[1] + d) + center[0] * plane[0]) + center[2] * plane[2])),
    ];
    let mut n = [
        plane[0] + (x * 2. - 1.) * amplitude,
        plane[1] + (y * 2. - 1.) * amplitude,
        plane[2] + (z - 1.) * amplitude,
        0.,
    ];
    let inverse = crate::gameplay::constraint_solver::inverse_length(
        (n[1] * n[1] + n[0] * n[0]) + n[2] * n[2],
    );
    for lane in &mut n[..3] {
        *lane *= inverse
    }
    n[3] = 0. - ((point[2] * n[2] + point[1] * n[1]) + point[0] * n[0]);
    (n, point)
}
impl PhysicsScene {
    pub(in crate::app) fn pending_water_impact(&self) -> Option<&WaterImpact> {
        self.water_impacts.front()
    }
    pub(in crate::app) fn acknowledge_water_impact(&mut self) -> Result<(), String> {
        let impact = self
            .water_impacts
            .pop_front()
            .ok_or("no accepted water impact to acknowledge")?;
        self.contacts.acknowledge_splash(impact.body);
        Ok(())
    }
    pub(super) fn release_water_queries(&mut self) -> Result<(), String> {
        if !self.water_impacts.is_empty() {
            return Err(
                "water contact effects must be accepted before query owner retirement".into(),
            );
        }
        if !self.water_notifications.is_empty() {
            return Err(
                "water entity notifications must drain before query owner retirement".into(),
            );
        }
        self.water_simulation.release()
    }

    pub(super) fn advance_rigid_water(
        &mut self,
        clock: &GameClock,
        cursor: &mut usize,
    ) -> Result<(), String> {
        let state = &mut self.water_simulation;
        if state.marker != Some(clock.frame_marker()) {
            if state.marker.is_some() && !state.complete {
                return Err("water frame replaced before accepted changes completed".into());
            }
            state.marker = Some(clock.frame_marker());
            state.prepared = false;
            state.complete = false;
            state.changes.clear();
            *cursor = 0;
        }
        if state.complete {
            return Ok(());
        }
        if !state.prepared {
            let mut shared = ocean_query_manager()
                .lock()
                .map_err(|_| "water ocean owner poisoned")?;
            let ring = state.ring;
            state.results.resize(state.rings[ring].len() * 32, [0.; 4]);
            state.result_counts.clear();
            state.result_counts.resize(state.rings[ring].len(), 0);
            {
                let crate::rendering::ocean::SharedOceanQueries {
                    queue, completed, ..
                } = &mut *shared;
                for (batch, handle) in state.rings[ring].iter_mut().enumerate() {
                    if !handle.is_invalid() {
                        state.result_counts[batch] = queue
                            .take(
                                handle,
                                completed.each_ref().map(Vec::as_slice),
                                &mut state.results[batch * 32..],
                            )?
                            .unwrap_or(0);
                    }
                }
            }
            state.rings[ring].clear();
            state.points.clear();
            let mut seed = clock.frame_marker() as u32;
            let delta = clock.delta(2).ok_or("water group2 clock absent")?;
            let guard = self
                .world
                .read()
                .map_err(|_| "water physics world poisoned")?;
            for (id, contacts, queries, splash_pending, splash_queued) in
                self.contacts.simulation_bodies()
            {
                let prior = queries[ring].index.take().filter(|&index| {
                    state
                        .result_counts
                        .get(index / 32)
                        .is_some_and(|&count| index % 32 < count)
                });
                let Some(water) = contacts
                    .components
                    .first()
                    .and_then(|h| self.waters.get(h.slot))
                else {
                    continue;
                };
                if water.owner.flags & 0xc < 8 {
                    continue;
                }
                let Some(body) = guard.storage.bodies.get(id) else {
                    return Err("water simulation retains stale body".into());
                };
                if body.flags & 3 == 0 || body.flags & 5 != 0 || body.broadphase_id == u32::MAX {
                    continue;
                }
                let handle = CollisionUserData::decode(body.user_data)?.component;
                let pool = if handle.class_id == self.classes[8] {
                    &self.rigid_bodies
                } else if handle.class_id == self.classes[12] {
                    &self.throwables
                } else {
                    return Err("water force body is not PhysicsBase".into());
                };
                let rigid = pool
                    .get(handle.slot)
                    .ok_or("water force rigid payload absent")?;
                if rigid.body() != Some(id) {
                    return Err("water force body owner generation differs".into());
                }
                let motion = guard
                    .storage
                    .motions
                    .get(body.motion_id)
                    .ok_or("water motion absent")?;
                let binding = self
                    .water_bindings
                    .get(water.owner.declaration.ok_or("water binding absent")?)
                    .ok_or("water binding index absent")?;
                let ocean = shared.active && binding.is_ocean;
                if ocean && prior.is_none() {
                    queries[ring].xy = [motion.pose.center[0], motion.pose.center[1]];
                    queries[ring].index = Some(state.points.len());
                    state.points.push(queries[ring].xy);
                    continue;
                }
                let (plane, impact_point) = if let Some(index) = prior {
                    let result = state
                        .results
                        .get(index)
                        .ok_or("water ocean result index exceeds consumed batches")?;
                    let xy = queries[ring].xy;
                    let n = [result[1], result[2], result[3]];
                    (
                        [
                            n[0],
                            n[1],
                            n[2],
                            0. - ((result[0] * n[2] + xy[1] * n[1]) + xy[0] * n[0]),
                        ],
                        [xy[0], xy[1], result[0]],
                    )
                } else {
                    jitter(
                        water.plane().ok_or("started water plane absent")?,
                        motion.pose.center,
                        binding.wavelet_amplitude,
                        &mut seed,
                    )
                };
                let shape = guard
                    .geometry
                    .shapes
                    .get(body.shape)
                    .and_then(Option::as_ref)
                    .ok_or("water force shape lease absent")?;
                let property = shape
                    .physics_property_if_present()?
                    .ok_or("water force requires actual key1 shape property")?;
                let linear = rigid.velocities.requested_linear;
                let angular = rigid.velocities.requested_angular;
                let change = state.scratch.calculate(&Input {
                    geometry: &property.geometry,
                    world_plane: plane,
                    body_frame: &body.transform,
                    body_density: [contacts.density.ok_or("water density uninitialized")?[0]; 4],
                    water_flow_and_density: water
                        .flow_and_density()
                        .ok_or("water flow not published")?,
                    body_mass: guard
                        .storage
                        .bodies
                        .mass_properties(id)
                        .map(|m| m.mass)
                        .unwrap_or(-1.),
                    gravity: guard.storage.info.gravity,
                    motion: &MotionMass {
                        center: motion.pose.center,
                        orientation: motion.pose.orientation,
                        inverse_mass: motion.inverse_mass,
                    },
                    linear,
                    angular,
                    delta,
                })?;
                if let Some(change) = change {
                    if *splash_pending && !*splash_queued {
                        let owner = self
                            .entities
                            .get(rigid.owner.entity.ok_or("water impact actor absent")?)
                            .ok_or("water impact actor header absent")?
                            .shared;
                        let declaration = &self.water_declarations[water
                            .owner
                            .declaration
                            .ok_or("water impact declaration absent")?];
                        let water_handle = *contacts
                            .components
                            .first()
                            .ok_or("water impact membership absent")?;
                        //651020 passes actual declaration pointers, not the
                        //water trigger body's possibly different material ID.
                        //Capture their canonical leases before force/body
                        //publication can retire the original witnesses.
                        let (source_material, target_material) =
                            super::water_impacts::material_leases(
                                &guard.storage.materials,
                                rigid.material_table().first().copied().unwrap_or(body.material_id),
                                declaration.materials[usize::from(declaration.is_ocean)],
                            )?;
                        let speed2 =
                            (linear[1] * linear[1] + linear[0] * linear[0]) + linear[2] * linear[2];
                        let clamp = if speed2 <= f32::from_bits(0x00800000) {
                            f32::from_bits(0x00800000)
                        } else {
                            speed2
                        };
                        let inverse = 1. / clamp.sqrt();
                        self.water_impacts.push_back(WaterImpact {
                            body: id,
                            water: water_handle,
                            owner,
                            source_material,
                            target_material,
                            point: impact_point,
                            normal: [plane[0], plane[1], plane[2]],
                            direction: std::array::from_fn(|i| linear[i] * inverse),
                            speed: inverse * speed2,
                            //092260 reads the admitted body's actual fourth
                            //packed inverse-mass lane, not authored mass data.
                            mass: if body.broadphase_id == u32::MAX {
                                0.
                            } else {
                                let inverse =
                                    f32::from_bits(u32::from(motion.inverse_mass[3]) << 16);
                                if inverse == 0. {
                                    0.
                                } else {
                                    crate::gameplay::constraint_solver::reciprocal(inverse)
                                }
                            },
                            body_flags: body.flags,
                        });
                        *splash_queued = true;
                    }
                    let squared = |v: [f32; 4]| (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2];
                    if body.flags & 8 != 0
                        || squared(change.linear) > f32::from_bits(0x38d1b717)
                        || squared(change.angular) > f32::from_bits(0x3d800000)
                    {
                        state.changes.push((
                            id,
                            std::array::from_fn(|i| linear[i] + change.linear[i]),
                            std::array::from_fn(|i| angular[i] + change.angular[i]),
                        ));
                    }
                }
                if ocean {
                    let point = change
                        .map(|c| c.submerged_center)
                        .unwrap_or(motion.pose.center);
                    queries[ring].xy = [point[0], point[1]];
                    queries[ring].index = Some(state.points.len());
                    state.points.push(queries[ring].xy);
                }
            }
            let register_ring = shared.register_ring;
            for batch in state.points.chunks(32) {
                state.rings[ring].push(shared.queue.register(register_ring, batch)?)
            }
            state.ring = (ring + 1) & 3;
            state.prepared = true;
        }
        while let Some(&(body, linear, angular)) = self.water_simulation.changes.get(*cursor) {
            self.queue_rigid_velocity(body, linear, angular)?;
            *cursor += 1;
        }
        self.water_simulation.complete = true;
        Ok(())
    }
}
