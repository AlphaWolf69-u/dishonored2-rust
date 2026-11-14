//! c8e930 body admission using the shared buffers, index and activation code.
use crate::gameplay::{
    body_activation::{ActivationCallbacks, ActivationOwners, BodyActivationController},
    body_quality::BodyQualityLibrary,
    body_storage::{BodyActivation, BodyStorage},
    broadphase::{BroadphaseBody, HybridBroadphase, PackedBodyBounds, PackedQuantization},
    collision_body::SweptBoundsStep,
    constraint_links::ConstraintLinks,
    deactivation::{queue_attached_bodies, DeactivationWorld},
    motion_storage::MotionStorage,
    point_query::PointQueryBody,
    shape_transform::ShapeMatrix,
};
mod transform;
mod velocity;
mod deactivation;
mod material;
pub use transform::TransformCallbacks;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyAddition {
    Disabled,
    Immediate,
    Queued,
}

/// Level-owned scratch for removal batches; retained capacities avoid repeated
/// allocation on component stop and are released with the world.
#[derive(Default)]
pub(crate) struct RemovalScratch {
    published: Vec<u32>,
    hits: Vec<u32>,
    locations: Vec<(u32, u32)>,
    request: Vec<u32>,
    progress: Option<RemovalProgress>,
    activation: BodyActivationController,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum RemovalPhase { Notify, Body, OverlapQuery, OverlapHit, OverlapActivate, Active, ActiveSolver, Broadphase, Finalize, Reconcile }
struct RemovalProgress {
    mode: i32,
    phase: RemovalPhase,
    index: usize,
    overlap_index: usize,
    overlap_body: u32,
}

impl RemovalScratch {
    pub(crate) fn is_idle(&self) -> bool { self.progress.is_none() }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.published.capacity() * std::mem::size_of::<u32>()
            + self.hits.capacity() * std::mem::size_of::<u32>()
            + self.locations.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.request.capacity() * std::mem::size_of::<u32>()
            + self.activation.retained_bytes()
    }
}

/// Required resource/event boundaries. Geometry must use the selected shape's
/// actual calcAabb. Broadphase queries run directly on the shared index.
pub trait PublicationCallbacks {
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String>;
    fn activation(
        &mut self,
        owners: ActivationOwners<'_>,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String>;
    fn added(&mut self, owners: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String>;
    fn removed(&mut self, _owners: ActivationOwners<'_>, _ids: &[u32]) -> Result<(), String> {
        Err("body-removal observer dispatch is unavailable".into())
    }
}

pub struct BodyPublication<'a> {
    pub bodies: &'a mut BodyStorage,
    pub motions: &'a mut MotionStorage,
    pub deactivation: &'a mut DeactivationWorld,
    pub broadphase: &'a mut HybridBroadphase,
    pub packing: &'a PackedQuantization,
    pub qualities: &'a BodyQualityLibrary,
    pub constraints: ConstraintLinks<'a>,
    pub tolerance: [f32; 4],
    pub step: &'a SweptBoundsStep,
    pub enable_deactivation: bool,
    pub phase: u32,
    pub step_index: u32,
}

struct Activation<'a, C> {
    broadphase: &'a mut HybridBroadphase,
    callbacks: &'a mut C,
}
impl<C: PublicationCallbacks> ActivationCallbacks for Activation<'_, C> {
    fn notify(
        &mut self,
        deactivation: &mut DeactivationWorld,
        bodies: &mut BodyStorage,
        motions: &mut MotionStorage,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String> {
        self.callbacks.activation(
            ActivationOwners {
                bodies,
                motions,
                deactivation,
                broadphase: self.broadphase,
            },
            ids,
            active,
        )
    }
    fn update_broadphase(
        &mut self,
        _: &mut BodyStorage,
        ids: &[u32],
        stride: u32,
    ) -> Result<(), String> {
        if stride != 4 {
            return Err("body publication activation stride is not contiguous".into());
        }
        self.broadphase.mark_bodies(ids.iter().copied());
        Ok(())
    }
}

impl BodyPublication<'_> {
    pub(crate) fn remove(
        &mut self, ids: &[u32], mode: i32, scratch: &mut RemovalScratch,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        let RemovalScratch { published, hits, locations, request, progress, activation } = scratch;
        if let Some(active) = progress.as_ref() {
            if request.as_slice() != ids || active.mode != mode { return Err("another body removal is incomplete".into()); }
        } else {
            request.clear();
            request.try_reserve_exact(ids.len()).map_err(|error| format!("body removal identity: {error}"))?;
            request.extend_from_slice(ids);
            self.bodies.prepare_removal(ids, published)?;
            if published.is_empty() { return Ok(()); }
            *progress = Some(RemovalProgress { mode, phase: RemovalPhase::Notify, index: 0,
                overlap_index: 0, overlap_body: 0 });
        }
        let active = progress.as_mut().expect("removal progress exists");
        loop {
            match active.phase {
                RemovalPhase::Notify => {
                    callbacks.removed(ActivationOwners {
                        bodies: &mut *self.bodies, motions: &mut *self.motions,
                        deactivation: &mut *self.deactivation, broadphase: &mut *self.broadphase,
                    }, published)?;
                    active.phase = RemovalPhase::Body;
                }
                RemovalPhase::Body => {
                    if active.index == published.len() { active.index = 0; active.phase = RemovalPhase::Broadphase; continue; }
                    let id = published[active.index];
                    let body = self.bodies.get(id).ok_or("removed body disappeared")?;
                    if body.broadphase_id == u32::MAX { active.index += 1; continue; }
                    let flags = body.flags;
                    if flags & 1 != 0 {
                        active.phase = RemovalPhase::OverlapQuery;
                    } else {
                        if flags & 9 == 0 { self.deactivation.detach_removed_body(self.bodies, id, mode != 0)?; }
                        active.phase = RemovalPhase::Active;
                    }
                }
                RemovalPhase::OverlapQuery => {
                    let id = published[active.index];
                    let bounds = self.bodies.get(id).ok_or("removed static body is missing")?.bounds;
                    hits.clear();
                    self.broadphase.overlap_bodies(bounds, self.packing, hits)?;
                    active.overlap_index = 0;
                    active.phase = RemovalPhase::OverlapHit;
                }
                RemovalPhase::OverlapHit => {
                    if active.overlap_index == hits.len() {
                        active.index += 1; active.phase = RemovalPhase::Body; continue;
                    }
                    let other = hits[active.overlap_index];
                    if published.contains(&other) { active.overlap_index += 1; continue; }
                    let body = self.bodies.get_by_index(other & 0xffffff).ok_or("removal overlap body is missing")?;
                    if body.flags & 9 != 0 { active.overlap_index += 1; continue; }
                    let actual = body.id;
                    if mode == 0 {
                        if !self.deactivation.queue_removed_static(self.bodies, published[active.index], actual)? {
                            active.overlap_body = actual;
                            active.phase = RemovalPhase::OverlapActivate;
                            continue;
                        }
                    } else {
                        self.deactivation.queue_island(self.bodies, actual, 0)?;
                    }
                    active.overlap_index += 1;
                }
                RemovalPhase::OverlapActivate => {
                    activation.activate_resumable(self.bodies, self.motions, self.deactivation,
                        active.overlap_body, self.phase, &mut Activation { broadphase: &mut *self.broadphase, callbacks })?;
                    active.overlap_index += 1;
                    active.phase = RemovalPhase::OverlapHit;
                }
                RemovalPhase::Active => {
                    let id = published[active.index];
                    let body = self.bodies.get(id).ok_or("removed body disappeared")?;
                    if body.flags & 8 != 0 {
                        let sole = body.next_attached_body_id == id;
                        self.bodies.remove_active(id)?;
                        if sole { active.phase = RemovalPhase::ActiveSolver; continue; }
                    }
                    active.index += 1;
                    active.phase = RemovalPhase::Body;
                }
                RemovalPhase::ActiveSolver => {
                    let motion = self.bodies.get(published[active.index]).ok_or("removed body disappeared")?.motion_id;
                    self.motions.remove_from_solver(motion)?;
                    active.index += 1;
                    active.phase = RemovalPhase::Body;
                }
                RemovalPhase::Broadphase => {
                    if active.index == published.len() { active.index = 0; active.phase = RemovalPhase::Finalize; continue; }
                    let id = published[active.index];
                    let body = self.bodies.get(id).ok_or("unpublished body disappeared")?;
                    locations.clear();
                    locations.push((id, body.broadphase_id));
                    self.broadphase.remove_bodies(locations)?;
                    active.index += 1;
                }
                RemovalPhase::Finalize => {
                    if active.index == published.len() { break; }
                    let id = published[active.index];
                    let body = self.bodies.get_mut(id).ok_or("unpublished body disappeared")?;
                    body.broadphase_id = u32::MAX;
                    body.status |= 1;
                    active.phase = RemovalPhase::Reconcile;
                }
                RemovalPhase::Reconcile => {
                    let motion = self.bodies.get(published[active.index]).ok_or("unpublished body disappeared")?.motion_id;
                    self.bodies.reconcile_removed_motion(motion, self.motions, self.deactivation)?;
                    active.index += 1;
                    active.phase = RemovalPhase::Finalize;
                }
            }
        }
        *progress = None;
        Ok(())
    }

    fn activate(
        &mut self,
        id: u32,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<bool, String> {
        self.set_activation(id, false, callbacks)
    }

    /// c91230's immediate activation/deactivation entry used by command1e.
    pub fn set_activation(
        &mut self,
        id: u32,
        deactivate: bool,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<bool, String> {
        BodyActivationController::default().set(
            self.bodies,
            self.motions,
            self.deactivation,
            id,
            deactivate,
            self.phase,
            &mut Activation {
                broadphase: self.broadphase,
                callbacks,
            },
        )
    }

    /// c91680: query before insertion, then re-read each returned body's flags
    /// after earlier activation callbacks. Static/already-active hits are skipped.
    pub fn wake_overlaps(
        &mut self,
        bounds: PackedBodyBounds,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        self.wake_overlaps_except(bounds, None, callbacks)
    }

    fn wake_overlaps_except(
        &mut self,
        bounds: PackedBodyBounds,
        exclude: Option<u32>,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        let mut hits = Vec::new();
        self.broadphase
            .overlap_bodies(bounds, self.packing, &mut hits)?;
        for id in hits {
            if exclude == Some(id) { continue; }
            if self
                .bodies
                .get_by_index(id & 0xffffff)
                .ok_or("overlap body is missing")?
                .flags
                & 9
                == 0
            {
                self.activate(id, callbacks)?;
            }
        }
        Ok(())
    }

    pub fn add(
        &mut self,
        ids: &[u32],
        mode: BodyAddition,
        mut activation: BodyActivation,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        if ids.is_empty() || mode == BodyAddition::Disabled {
            return Ok(());
        }
        if !self.enable_deactivation {
            activation = BodyActivation::Activate;
        }
        if mode == BodyAddition::Queued {
            return self.bodies.queue_additions(ids, activation);
        }

        let mut activate = Vec::new();
        for &id in ids {
            let body = self.bodies.get(id).ok_or("added body is missing")?;
            if body.flags & 2 == 0 {
                let bounds = callbacks.bounds(body.selected_shape(3)?, body.transform)?;
                let packed = self.packing.pack_bounds(bounds, self.tolerance);
                self.bodies.get_mut(id).unwrap().bounds = packed;
                self.bodies.mark_bounds_changed(id)?;
                if activation == BodyActivation::Activate {
                    self.wake_overlaps(packed, callbacks)?;
                }
            } else {
                let motion_id = body.motion_id;
                self.bodies.get_mut(id).unwrap().status |= 1;
                if activation == BodyActivation::Activate {
                    let body = self.bodies.get(id).unwrap();
                    let motion = self
                        .motions
                        .get(motion_id)
                        .ok_or("added motion is missing")?;
                    if motion.solver_id == 0x7fffffff || body.flags & 8 == 0 {
                        activate.push(id);
                    }
                } else {
                    self.bodies.get_mut(id).unwrap().flags &= !8;
                    queue_attached_bodies(self.bodies, id, &mut self.deactivation.pending_bodies)?;
                }
                let body = self.bodies.get_mut(id).unwrap();
                let motion = self
                    .motions
                    .get(motion_id)
                    .ok_or("added motion is missing")?;
                let quality = self
                    .qualities
                    .get(body.quality_id)
                    .ok_or("added body quality is missing")?;
                body.try_update_swept_bounds(
                    motion,
                    quality.requested_flags,
                    self.tolerance,
                    self.step,
                    self.packing,
                    |shape, matrix| callbacks.bounds(shape, matrix),
                )?;
            }
            self.bodies.reset_previous_bounds(id)?;
        }
        let input = ids
            .iter()
            .map(|&id| {
                let body = self.bodies.get(id).ok_or("prepared body disappeared")?;
                Ok(BroadphaseBody {
                    body: PointQueryBody {
                        index: id,
                        bounds: self.packing.query_bounds(body.bounds),
                    },
                    flags: body.flags,
                    status: body.status,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut error = None;
        self.broadphase.add_bodies(&input, |id, location| {
            // Splits also update bodies already in the tree.
            if let Some(body) = self.bodies.get_by_index(id & 0xffffff).map(|b| b.id) {
                self.bodies.get_mut(body).unwrap().broadphase_id = location;
            } else {
                error = Some("published broadphase body is missing".to_owned());
            }
        })?;
        if let Some(error) = error {
            return Err(error);
        }
        for &id in ids {
            if self
                .bodies
                .get(id)
                .ok_or("published body disappeared")?
                .flags
                & 2
                != 0
            {
                self.constraints.activate_neighbors(
                    self.enable_deactivation,
                    self.bodies,
                    id,
                    |first, second| {
                        self.deactivation.link_bodies(
                            self.bodies,
                            self.motions,
                            first,
                            second,
                            self.step_index,
                        )
                    },
                )?;
            }
        }
        for id in activate {
            self.activate(id, callbacks)?;
        }
        callbacks.added(
            ActivationOwners {
                bodies: self.bodies,
                motions: self.motions,
                deactivation: self.deactivation,
                broadphase: self.broadphase,
            },
            ids,
        )
    }
}
