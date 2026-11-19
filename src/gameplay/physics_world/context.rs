//! Resource and observer bindings for the shared world's publication routines.
use super::{BodyPublication, PublicationCallbacks, TransformCallbacks};
use crate::gameplay::{
    body_activation::ActivationOwners, body_quality::BodyQualityLibrary,
    collision_body::BodyCacheSettings, constraint_links::ConstraintLinks,
    shape_transform::ShapeMatrix,
};

/// Null is an actual selected-shape state. A resource lookup failure must be
/// returned as an error; it must never be treated as a null shape.
pub trait PublicationGeometry {
    fn is_null(&mut self, shape: usize) -> Result<bool, String>;
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String>;
}

/// Only observer dispatch crosses this boundary. Cache rebuilding, broadphase
/// updates, activation and attached-body traversal run on the real world owners.
pub trait PublicationEvents {
    fn activation(
        &mut self,
        owners: ActivationOwners<'_>,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String>;
    fn activation_with_timespaces(
        &mut self, owners: ActivationOwners<'_>, ids: &[u32], active: bool,
        _timespaces: Option<&crate::gameplay::motion_timespaces::MotionTimespaces>,
    ) -> Result<(), String> { self.activation(owners, ids, active) }
    fn added(&mut self, owners: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String>;
    fn removed(&mut self, _owners: ActivationOwners<'_>, _ids: &[u32]) -> Result<(), String> {
        Err("body-removal observer dispatch is unavailable".into())
    }
    fn destroyed(&mut self, _owners: ActivationOwners<'_>, _ids: &[u32]) -> Result<(), String> {
        Err("body-destruction observer dispatch is unavailable".into())
    }
    fn motion_destroyed(&mut self, _owners: ActivationOwners<'_>, _motion: u32) -> Result<(), String> {
        Err("motion-destruction observer dispatch is unavailable".into())
    }
    /// c892c0: the distinct world+8a0 static/motion-backed transition signal.
    fn motion_type_changed(&mut self, _owners: ActivationOwners<'_>, _body: u32, _is_static: bool) -> Result<(), String> {
        Err("body motion-kind observer dispatch is unavailable".into())
    }
    /// c91140/c910d0/c91030 world constraint signals, before slot reuse.
    fn constraint_changed(&mut self,_owners:ActivationOwners<'_>,_slot:usize,_generation:u32,
        _change:super::events::ConstraintChange)->Result<(),String>{
        Err("constraint observer dispatch is unavailable".into())
    }
    fn transform_changed(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        static_only: bool,
    ) -> Result<(), String>;
}

/// Borrowed level resources and current world settings. This owns no body,
/// motion, island or index copies: callbacks receive the same live owners that
/// are being changed by BodyPublication and the deferred command processor.
pub struct PublicationContext<'a, G, E> {
    pub geometry: &'a mut G,
    pub events: &'a mut E,
    pub cache: BodyCacheSettings<'a>,
    pub qualities: &'a BodyQualityLibrary,
    pub constraints: ConstraintLinks<'a>,
    pub enable_deactivation: bool,
    pub phase: u32,
    pub step_index: u32,
    pub(crate) timespaces: Option<&'a crate::gameplay::motion_timespaces::MotionTimespaces>,
}

impl<G, E: super::MotionCreationCallbacks> super::MotionCreationCallbacks
    for PublicationContext<'_, G, E>
{
    fn motion_capacity_requested(&mut self, owners: ActivationOwners<'_>) -> Result<(), String> {
        self.events.motion_capacity_requested(owners)
    }
    fn motion_created(
        &mut self,
        owners: ActivationOwners<'_>,
        info: &crate::gameplay::motion_info::MotionInfo,
        motion: u32,
    ) -> Result<(), String> {
        self.events.motion_created(owners, info, motion)
    }
}

impl<G, E: crate::gameplay::collision_user_data::velocity::VelocityDataStore>
    crate::gameplay::collision_user_data::velocity::VelocityDataStore
    for PublicationContext<'_, G, E>
{
    type Reference = E::Reference;
    fn referenced_body(&self, reference: Self::Reference) -> Result<u32, String> {
        self.events.referenced_body(reference)
    }
    fn velocities(
        &mut self,
        user_data: u64,
    ) -> Result<&mut crate::gameplay::collision_user_data::velocity::CollisionVelocities, String>
    {
        self.events.velocities(user_data)
    }
}

impl<G: PublicationGeometry, E: PublicationEvents> PublicationCallbacks
    for PublicationContext<'_, G, E>
{
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        self.geometry.bounds(shape, frame)
    }
    fn activation(
        &mut self,
        owners: ActivationOwners<'_>,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String> {
        self.events.activation_with_timespaces(owners, ids, active, self.timespaces)
    }
    fn added(&mut self, owners: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String> {
        self.events.added(owners, ids)
    }
    fn removed(&mut self, owners: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String> {
        self.events.removed(owners, ids)
    }
}

impl<G: PublicationGeometry, E: PublicationEvents> TransformCallbacks
    for PublicationContext<'_, G, E>
{
    fn refresh_transform_cache(
        &mut self,
        owners: ActivationOwners<'_>,
        id: u32,
    ) -> Result<(), String> {
        let body = owners
            .bodies
            .get_mut(id)
            .ok_or("cache-update body is missing")?;
        let selected = body.selected_shape(3)?;
        let shape = (!self.geometry.is_null(selected)?).then_some(selected);
        let motion = owners
            .motions
            .get(body.motion_id)
            .ok_or("cache-update motion is missing")?;
        let quality = self
            .qualities
            .get(body.quality_id)
            .ok_or("cache-update quality is missing")?;
        body.update_from_motion(
            motion,
            shape,
            quality.requested_flags,
            &self.cache,
            |shape, frame| self.geometry.bounds(shape, frame),
        )?;
        Ok(())
    }

    fn update_attached_transform(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        mode: i32,
    ) -> Result<(), String> {
        // Reborrow immutable adjacency, rather than cloning constraints or the
        // shared mutable world. The same event dispatcher observes activation.
        let mut world = BodyPublication {
            bodies: owners.bodies,
            motions: owners.motions,
            deactivation: owners.deactivation,
            broadphase: owners.broadphase,
            packing: self.cache.packing,
            qualities: self.qualities,
            constraints: ConstraintLinks {
                heads: self.constraints.heads,
                constraints: self.constraints.constraints,
                next: self.constraints.next,
            },
            tolerance: self.cache.bounds_tolerance,
            step: self.cache.step,
            enable_deactivation: self.enable_deactivation,
            phase: self.phase,
            step_index: self.step_index,
        };
        world.update_attached_bodies(body, mode, self)
    }

    fn transform_changed(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        static_only: bool,
    ) -> Result<(), String> {
        self.events.transform_changed(owners, body, static_only)
    }
}
