//! Ordinary rigid-world manifold ownership and stream-solver dispatch.
//! Geometry/resources remain borrowed from the canonical world's shape owner.
use super::{
    PublicationEvents,
    body_creation::{BodyCreationCallbacks, BodyCreationGeometry},
    storage::{WorldStorage, simulation::SimulationContacts},
};
use crate::gameplay::{
    broadphase::OverlapWorkspace,
    collision_body::CollisionBody,
    collision_filter::pair_collides,
    constraint_solver::{
        ball_socket::SolverVelocities,
        contact::{
            self, ContactBuild, ContactCache, ContactPoint, ContactRows, ContactState,
            correspondence::{FacePair, PointCorrespondence},
            material::PairMaterial,
            restitution::RestitutionCache,
        },
        step::{SolverParameters, SolverPhase},
    },
};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ManifoldCandidate {
    pub id: u8,
    pub projected_xy: [f32; 2],
    pub point_on_b: [f32; 4],
    pub distance: f32,
}
pub(crate) struct ManifoldCandidates<'a> {
    pub candidates: &'a [ManifoldCandidate],
    pub selected: &'a [u8],
    pub faces: FacePair,
    pub normal: [f32; 3],
    pub shape_keys: [u32; 2],
    pub material_ids: [u16; 2],
}

/// Required full manifold producer; closest-point hits are insufficient.
pub(crate) trait ContactGeometry: BodyCreationGeometry {
    fn contact_manifolds(
        &mut self,
        bodies: [&CollisionBody; 2],
        tolerance: f32,
        submit: &mut dyn FnMut(ManifoldCandidates<'_>) -> Result<(), String>,
    ) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    bodies: [u32; 2],
    shapes: [u32; 2],
    resources: [usize; 2],
}
struct Manifold {
    key: Key,
    seen: u64,
    normal: [f32; 3],
    points: [ContactPoint; 4],
    material_ids: [u16; 2],
    correspondence: PointCorrespondence,
    cache: ContactCache,
    restitution: RestitutionCache,
    rows: Option<ContactRows>,
    state: ContactState,
    solvers: [Option<usize>; 2],
}

/// Numeric output is handed to the real body-contact callback publisher. It
/// carries no fabricated event flag; that publisher owns the source event ABI.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SolvedContact {
    pub bodies: [u32; 2],
    pub shape_keys: [u32; 2],
    pub normal: [f32; 3],
    pub points: [ContactPoint; 4],
    pub impulses: [f32; 4],
    pub count: u8,
}

pub(crate) struct Contacts {
    manifolds: HashMap<Key, Manifold>,
    pairs: Vec<[u32; 2]>,
    overlaps: Vec<u32>,
    overlap_workspace: OverlapWorkspace,
    body_links: Vec<[u32; 2]>,
    active_keys: Vec<Key>,
    motion_to_solver: Vec<usize>,
    projected: Vec<[f32; 2]>,
    output: Vec<SolvedContact>,
    serial: u64,
    /// Collision-context+10 low byte copied into real cache+0e by d14cb0.
    context_flags: u8,
}
impl Contacts {
    pub(crate) fn new(context_flags: u8) -> Self {
        Self {
            manifolds: HashMap::new(),
            pairs: Vec::new(),
            overlaps: Vec::new(),
            overlap_workspace: Default::default(),
            body_links: Vec::new(),
            active_keys: Vec::new(),
            motion_to_solver: Vec::new(),
            projected: Vec::new(),
            output: Vec::new(),
            serial: 0,
            context_flags,
        }
    }
    pub(crate) fn solved(&self) -> &[SolvedContact] {
        &self.output
    }
    /// The canonical mutable-shape publisher invokes this with its body ID.
    /// Dropping these numeric rows releases no shape/resource ownership.
    pub(crate) fn invalidate_body(&mut self, body: u32) {
        self.manifolds.retain(|key, _| !key.bodies.contains(&body));
        self.active_keys.retain(|key| !key.bodies.contains(&body));
        self.body_links.retain(|ids| !ids.contains(&body));
        self.output
            .retain(|contact| !contact.bodies.contains(&body));
    }
    pub(crate) fn clear(&mut self) {
        self.manifolds.clear();
        self.pairs.clear();
        self.overlaps.clear();
        self.body_links.clear();
        self.active_keys.clear();
        self.motion_to_solver.clear();
        self.projected.clear();
        self.output.clear();
    }
}

impl<G: ContactGeometry, E: PublicationEvents + BodyCreationCallbacks> SimulationContacts<G, E>
    for Contacts
{
    fn pre_collide(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut G,
        events: &mut E,
        _step: &SolverParameters,
    ) -> Result<(), String> {
        self.serial = self.serial.wrapping_add(1);
        self.pairs.clear();
        self.body_links.clear();
        self.active_keys.clear();
        self.output.clear();
        self.manifolds.retain(|key, value| {
            value.rows = None;
            key.bodies.iter().all(|&id| world.bodies.get(id).is_some())
        });
        // Only actual active body publications issue broadphase work. The
        // packed AABBs admit real shape pairs; they never become colliders.
        for &id in world.bodies.active_body_ids() {
            let body = world.bodies.get(id).ok_or("active contact body is stale")?;
            self.overlaps.clear();
            world.broadphase.filtered_overlaps_with_workspace(
                body.bounds,
                &world.quantization.packed,
                &mut self.overlap_workspace,
                |_| true,
                |_, index| {
                    world.bodies.get_by_index(index).is_some_and(|other| {
                        other.id != id
                            && other.motion_id != body.motion_id
                            && pair_collides(body.filter_info, other.filter_info)
                    })
                },
                &mut self.overlaps,
            )?;
            for &index in &self.overlaps {
                let other = world
                    .bodies
                    .get_by_index(index)
                    .ok_or("contact broadphase body is stale")?;
                let pair = if id < other.id {
                    [id, other.id]
                } else {
                    [other.id, id]
                };
                self.pairs.push(pair);
            }
        }
        self.pairs.sort_unstable();
        self.pairs.dedup();
        for &ids in &self.pairs {
            let mut ids = ids;
            let first = world.bodies.get(ids[0]).ok_or("contact body A retired")?;
            let second = world.bodies.get(ids[1]).ok_or("contact body B retired")?;
            // d144c0 places a dynamic receiver before a static body.
            if first.flags & 1 != 0 && second.flags & 2 != 0 {
                ids.swap(0, 1);
            }
            let bodies = [
                world.bodies.get(ids[0]).ok_or("contact body A retired")?,
                world.bodies.get(ids[1]).ok_or("contact body B retired")?,
            ];
            let serial = self.serial;
            // ca6c20 stores the farthest bound distance at body+7e. The
            // correspondence threshold reads this body cache, not a shape.
            let minimum_body_radius = f32::from_bits(u32::from(bodies[0].packed_radius) << 16)
                .min(f32::from_bits(u32::from(bodies[1].packed_radius) << 16));
            let manifolds = &mut self.manifolds;
            let projected = &mut self.projected;
            let links = &mut self.body_links;
            let active_keys = &mut self.active_keys;
            geometry.contact_manifolds(bodies, world.info.collision_tolerance, &mut |patch| {
                if patch.selected.is_empty() {
                    return Ok(());
                }
                let key = Key {
                    bodies: ids,
                    shapes: patch.shape_keys,
                    resources: bodies.map(|body| body.shape),
                };
                let entry = manifolds.entry(key).or_insert_with(|| Manifold {
                    key,
                    seen: serial,
                    normal: patch.normal,
                    points: [ContactPoint::default(); 4],
                    material_ids: patch.material_ids,
                    correspondence: Default::default(),
                    cache: Default::default(),
                    restitution: Default::default(),
                    rows: None,
                    state: Default::default(),
                    solvers: [None; 2],
                });
                projected.clear();
                projected.extend(patch.candidates.iter().map(|p| p.projected_xy));
                if patch
                    .candidates
                    .iter()
                    .enumerate()
                    .any(|(i, p)| usize::from(p.id) != i)
                {
                    return Err("contact candidate storage does not preserve source IDs".into());
                }
                let order = entry.correspondence.update(
                    patch.faces,
                    patch.selected,
                    projected,
                    minimum_body_radius,
                    &mut entry.cache,
                )?;
                entry.normal = patch.normal;
                entry.material_ids = patch.material_ids;
                entry.seen = serial;
                for (slot, &id) in order[..entry.correspondence.count as usize]
                    .iter()
                    .enumerate()
                {
                    let point = patch.candidates[id as usize];
                    entry.points[slot] = ContactPoint {
                        position: point.point_on_b[..3].try_into().unwrap(),
                        distance: point.distance,
                    };
                    if !active_keys.contains(&key) {
                        active_keys.push(key);
                    }
                }
                let ids = entry.correspondence.point_ids();
                entry
                    .restitution
                    .remap(&ids[..entry.correspondence.count as usize])?;
                if !links.contains(&key.bodies) {
                    links.push(key.bodies);
                }
                Ok(())
            })?;
        }
        // The original retains sleeping contact caches, but retires awake
        // pairs when collision processing no longer produces a manifold.
        // Keep dormant numerical state without accumulating travelled pairs.
        self.manifolds.retain(|key, entry| {
            entry.seen == self.serial
                || key.bodies.iter().all(|&id| {
                    world
                        .bodies
                        .get(id)
                        .is_some_and(|body| body.active_list_index == u32::MAX)
                })
        });
        // Wake after all geometry borrows end, before dense motion admission.
        for &ids in &self.body_links {
            for id in ids {
                let body = world.bodies.get(id).ok_or("contact wake body retired")?;
                if body.motion_id != 0 {
                    world.activate_body(geometry, events, id)?;
                }
            }
        }
        Ok(())
    }
    fn prepare(
        &mut self,
        world: &WorldStorage,
        _geometry: &mut G,
        step: &SolverParameters,
        motions: &[u32],
        _velocities: &[SolverVelocities],
    ) -> Result<(), String> {
        self.motion_to_solver
            .resize(world.motions.capacity(), usize::MAX);
        self.motion_to_solver.fill(usize::MAX);
        for (solver, &motion) in motions.iter().enumerate() {
            self.motion_to_solver[motion as usize] = solver;
        }
        for key in &self.active_keys {
            let entry = self
                .manifolds
                .get_mut(key)
                .ok_or("active contact manifold absent")?;
            let bodies = entry
                .key
                .bodies
                .map(|id| world.bodies.get(id).ok_or("prepared contact body retired"));
            let bodies = [bodies[0]?, bodies[1]?];
            let motions = bodies.map(|b| {
                world
                    .motions
                    .get(b.motion_id)
                    .ok_or("prepared contact motion retired")
            });
            let motions = [motions[0]?, motions[1]?];
            entry.solvers = bodies
                .map(|b| (b.motion_id != 0).then(|| self.motion_to_solver[b.motion_id as usize]));
            if entry
                .solvers
                .iter()
                .flatten()
                .any(|&slot| slot == usize::MAX)
            {
                return Err("contact motion missing from active solver roster".into());
            }
            let material = entry.material_ids.map(|id| {
                world
                    .materials
                    .library
                    .get(id)
                    .ok_or("contact material absent")
            });
            let coefficients = PairMaterial::from_records(material[0]?, material[1]?);
            let qualities = bodies.map(|b| {
                world
                    .qualities
                    .get(b.quality_id)
                    .ok_or("contact quality absent")
            });
            let qualities = [qualities[0]?, qualities[1]?];
            let pair_flags = if qualities[0].priority < qualities[1].priority {
                qualities[1].requested_flags & qualities[0].supported_flags
            } else {
                qualities[0].requested_flags & qualities[1].supported_flags
            } as u16;
            let combined_flags =
                bodies[0].flags | bodies[1].flags | material[0]?.words[1] | material[1]?.words[1];
            let (recovery_limit, recovery_factor) =
                contact::build::recovery_coefficients(Some(self.context_flags), combined_flags);
            let mut rows = contact::build(
                &ContactBuild {
                    motions,
                    normal: entry.normal,
                    points: &entry.points[..entry.correspondence.count as usize],
                    step,
                    base_friction: coefficients.base_friction(),
                    friction_delta: coefficients.friction_delta(),
                    immediate_friction: pair_flags & 0x2000 != 0,
                    recovery_limit,
                    recovery_factor,
                    // 1412abc30 initializes persistent cache+3a/+32 to
                    // 0x7f7f/0xff before any registered modifier edits it.
                    impulse_limit: Some(contact::ImpulseLimit {
                        maximum: f32::from_bits(0x7f7f_0000),
                        clipped_fraction: 0xff,
                    }),
                },
                &mut entry.cache,
            )?;
            entry.restitution.apply(
                &mut rows,
                &mut entry.cache,
                motions,
                step,
                coefficients.restitution(),
            )?;
            entry.state = ContactState::default();
            entry.rows = Some(rows);
        }
        Ok(())
    }
    fn solve(
        &mut self,
        _step: &SolverParameters,
        phase: SolverPhase,
        velocities: &mut [SolverVelocities],
    ) {
        let zero = SolverVelocities {
            linear: [0.; 4],
            angular: [0.; 4],
            inverse_mass: [0.; 4],
            initial_linear: [0.; 4],
            initial_angular: [0.; 4],
        };
        for key in &self.active_keys {
            let entry = self
                .manifolds
                .get_mut(key)
                .expect("prepared contact manifold");
            let Some(rows) = &entry.rows else { continue };
            let [a, b] = entry
                .solvers
                .map(|slot| slot.map_or(zero, |i| velocities[i]));
            let (mut a, mut b) = (a, b);
            rows.solve(&mut entry.state, &mut a, &mut b, phase.flags & 2 != 0);
            if let Some(slot) = entry.solvers[0] {
                velocities[slot] = a;
            }
            if let Some(slot) = entry.solvers[1] {
                velocities[slot] = b;
            }
        }
    }
    fn body_links(&self) -> &[[u32; 2]] {
        &self.body_links
    }
    fn finish(
        &mut self,
        _world: &mut WorldStorage,
        _geometry: &mut G,
        _events: &mut E,
        _step: &SolverParameters,
    ) -> Result<(), String> {
        self.output.clear();
        for key in &self.active_keys {
            let entry = self
                .manifolds
                .get_mut(key)
                .ok_or("solved contact manifold absent")?;
            entry.cache.export(&entry.state);
            self.output.push(SolvedContact {
                bodies: entry.key.bodies,
                shape_keys: entry.key.shapes,
                normal: entry.normal,
                points: entry.points,
                impulses: entry.state.normal_impulses,
                count: entry.correspondence.count,
            });
        }
        Ok(())
    }
}
