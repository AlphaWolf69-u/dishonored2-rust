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
