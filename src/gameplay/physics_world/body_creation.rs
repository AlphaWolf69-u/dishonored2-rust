//! c8e5b0/c92be0 body creation on the same owners used by publication/queries.
use super::{
    BodyAddition, BodyPublication, MotionCreationCallbacks, PublicationContext, PublicationEvents,
    PublicationGeometry,
};
use crate::{
    assets::physworld::{body::BodyInfo, mass::PrincipalMassProperties, shape::ShapeHeader},
    gameplay::{
        body_activation::ActivationOwners,
        body_attachment::{attach_body_motion, attach_unpublished_body_resumable, AttachmentSettings, AttachmentWorld},
        body_lod::BodyLodConfig,
        body_storage::{BodyActivation, BodyCreationAssets, BodyPropertyBits},
        deactivation::DeactivationWorld,
        motion_info::MotionInfo,
        motion_mass::recompute_motion_mass,
        shape_registry::{ShapeRegistry, ShapeSignalStore},
        shape_transform::ShapeMatrix,
        space_cells::SpaceCells,
    },
};
use std::cell::RefCell;

pub(crate) trait BodyCreationGeometry: PublicationGeometry + ShapeSignalStore {
    fn creation_assets(&self, info: &BodyInfo) -> Result<BodyCreationAssets<'_>, String>;
    fn header(&self, shape: usize) -> Result<ShapeHeader, String>;
    fn is_mutable(&self, shape: usize) -> Result<bool, String>;
    fn rotation_limit_measure(&self, shape: usize) -> Result<f32, String>;
    fn cached_mass(&self, shape: usize) -> Result<Option<PrincipalMassProperties>, String>;
    fn calculate_mass(
        &self,
        shape: usize,
        config: [u32; 3],
    ) -> Result<PrincipalMassProperties, String>;
    fn prepare_motion(&self, info: &BodyInfo) -> Result<MotionInfo, String> {
        let distribution = self.creation_assets(info)?.mass_distribution;
        MotionInfo::from_body(
            info,
            self.cached_mass(info.shape)?,
            distribution,
            |config| self.calculate_mass(info.shape, config),
        )
    }
}

pub(crate) trait BodyCreationCallbacks: MotionCreationCallbacks {
    fn body_capacity_requested(&mut self, owners: ActivationOwners<'_>) -> Result<(), String>;
    fn body_created(
        &mut self,
        owners: ActivationOwners<'_>,
        info: &BodyInfo,
        body: u32,
    ) -> Result<(), String>;
}

/// Source world+958 is optional. Its callback receives a local Cinfo copy;
/// the caller's authored request is preserved even when the modifier changes it.
pub(crate) trait BodyInfoModifier {
    fn modify(&mut self, info: &mut BodyInfo) -> Result<(), String>;
}

pub(crate) struct BodyCreation<'a, 'world, 'resources, G, E> {
    pub world: &'a mut BodyPublication<'world>,
    pub callbacks: &'a mut PublicationContext<'resources, G, E>,
    pub registry: &'a mut ShapeRegistry,
    pub lod: &'a BodyLodConfig,
    pub cells: &'a SpaceCells,
    pub properties: &'a mut [BodyPropertyBits],
}

/// One disabled c8e5b0 request owns these IDs until completion. The caller
/// retains the corresponding shape and declaration leases throughout.
pub(crate) struct DisabledCreationProgress {
    pub key: (crate::gameplay::component_state::entity::ComponentHandle, u32),
    pub info: BodyInfo,
    pub(super) body: u32,
    pub(super) motion: u32,
    motion_info: Option<MotionInfo>,
    pub(super) step: u8,
    pub(super) attachment_step: u8,
    callback_pending: bool,
    pub(super) abort_step: u8,
    pub(super) unlink_step: u8,
    pub(super) aborting: bool,
}
impl DisabledCreationProgress {
    pub fn new(key: (crate::gameplay::component_state::entity::ComponentHandle, u32), info: BodyInfo) -> Self {
        Self { key, info, body: u32::MAX, motion: u32::MAX, motion_info: None, step: 0, attachment_step: 0, callback_pending: false, abort_step: 0, unlink_step: 0, aborting: false }
    }
}

pub(super) struct Attachment<'a, G> {
    pub(super) geometry: &'a mut G,
    pub(super) deactivation: &'a mut DeactivationWorld,
}
impl<G: BodyCreationGeometry> AttachmentWorld for Attachment<'_, G> {
    fn shape_flags(&mut self, shape: usize) -> Result<u16, String> {
        Ok(self.geometry.header(shape)?.flags)
    }
    fn rotation_limit_measure(&mut self, shape: usize) -> Result<f32, String> {
        self.geometry.rotation_limit_measure(shape)
    }
    fn shape_bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        self.geometry.bounds(shape, frame)
    }
    fn register_deactivation_body(&mut self, body: u32, motion: u32) -> Result<(), String> {
        let state = *self
            .deactivation
            .state(motion)
            .ok_or("attachment deactivation state is missing")?;
        state.register_body(&mut self.deactivation.islands, body)
    }
}

impl<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>
    BodyCreation<'_, '_, '_, G, E>
{
    pub fn finish_creation_callback(&mut self, progress: &mut DisabledCreationProgress) -> Result<(), String> {
        if !progress.callback_pending { return Ok(()); }
        let owners = ActivationOwners { bodies: self.world.bodies, motions: self.world.motions,
            deactivation: self.world.deactivation, broadphase: self.world.broadphase };
        match progress.step {
            0 => self.callbacks.events.body_capacity_requested(owners)?,
            3 => self.callbacks.events.motion_capacity_requested(owners)?,
            7 => self.callbacks.events.motion_created(owners, progress.motion_info.as_ref().ok_or("motion creation info is absent")?, progress.motion)?,
            10 => self.callbacks.events.body_created(owners, &progress.info, progress.body)?,
            _ => return Err("invalid pending creation callback".into()),
        }
        progress.callback_pending = false;
        // Created observers have now seen this object and its teardown must
        // use the corresponding actual destruction observer.
        if progress.step == 7 || progress.step == 10 { progress.step += 1; }
        Ok(())
    }

    pub fn create_disabled_resumable(&mut self, progress: &mut DisabledCreationProgress) -> Result<u32, String> {
        let info = &progress.info;
        if progress.step == 0 {
            if progress.callback_pending || (info.reserved_body_id & 0xffffff == 0xffffff && !self.world.bodies.has_free_slot()) {
                progress.callback_pending = true;
                self.callbacks.events.body_capacity_requested(ActivationOwners {
                    bodies: self.world.bodies, motions: self.world.motions,
                    deactivation: self.world.deactivation, broadphase: self.world.broadphase,
                })?;
                progress.callback_pending = false;
            }
            progress.step = 1;
        }
        if progress.step == 1 {
            progress.body = if info.reserved_body_id & 0xffffff == 0xffffff {
                self.world.bodies.reserve(self.properties)?
            } else { info.reserved_body_id };
            progress.step = 2;
        }
        if progress.step == 2 {
            if info.reserved_motion_id != 0x7fff_ffff {
                progress.motion = info.reserved_motion_id;
                progress.step = 8;
            } else if info.motion_type == 0 {
                progress.motion = 0;
                progress.step = 8;
            } else {
                progress.motion_info = Some(self.callbacks.geometry.prepare_motion(info)?);
                progress.step = 3;
            }
        }
        if progress.step == 3 {
            if progress.callback_pending || !self.world.motions.has_free_slot() {
                progress.callback_pending = true;
                self.callbacks.events.motion_capacity_requested(ActivationOwners {
                    bodies: self.world.bodies, motions: self.world.motions,
                    deactivation: self.world.deactivation, broadphase: self.world.broadphase,
                })?;
                progress.callback_pending = false;
            }
            progress.step = 4;
        }
        if progress.step == 4 {
            progress.motion = self.world.motions.reserve()?;
            progress.step = 5;
        }
        let motion_info = progress.motion_info.as_ref();
        if progress.step == 5 {
            self.world.motions.initialize(progress.motion, motion_info.ok_or("motion creation info is absent")?, self.cells)?;
            progress.step = 6;
        }
        if progress.step == 6 {
            self.world.deactivation.initialize_motion_state(progress.motion, motion_info.ok_or("motion creation info is absent")?.enable_deactivation)?;
            progress.step = 7;
        }
        if progress.step == 7 {
            progress.callback_pending = true;
            self.callbacks.events.motion_created(ActivationOwners {
                bodies: self.world.bodies, motions: self.world.motions,
                deactivation: self.world.deactivation, broadphase: self.world.broadphase,
            }, motion_info.ok_or("motion creation info is absent")?, progress.motion)?;
            progress.callback_pending = false;
            progress.step = 8;
        }
        if progress.step == 8 {
            let dynamic = self.world.motions.get(progress.motion).ok_or("body creation motion is missing")?.requires_dynamic_body();
            let assets = self.callbacks.geometry.creation_assets(info)?;
            self.world.bodies.initialize(progress.body, info, dynamic, assets)?;
            progress.step = 9;
        }
        if progress.step == 9 {
            let dynamic = self.world.motions.get(progress.motion).ok_or("body creation motion is missing")?.requires_dynamic_body();
            if dynamic {
                let settings = AttachmentSettings { qualities: self.world.qualities, lod: self.lod,
                    tolerance: self.world.tolerance, step: self.world.step, packing: self.world.packing };
                attach_unpublished_body_resumable(self.world.bodies, self.world.motions, progress.body,
                    progress.motion, Some(info.orientation), &settings,
                    &mut Attachment { geometry: self.callbacks.geometry, deactivation: self.world.deactivation },
                    &mut progress.attachment_step)?;
            } else {
                let body = self.world.bodies.get_mut(progress.body).ok_or("created static body is missing")?;
                body.next_attached_body_id = body.id;
                body.refresh_inverse_frame();
                body.update_size_and_static_lod(self.lod.body_is_big_threshold, false);
            }
            progress.step = 10;
        }
        if progress.step == 10 {
            progress.callback_pending = true;
            self.callbacks.events.body_created(ActivationOwners {
                bodies: self.world.bodies, motions: self.world.motions,
                deactivation: self.world.deactivation, broadphase: self.world.broadphase,
            }, info, progress.body)?;
            progress.callback_pending = false;
            progress.step = 11;
        }
        if progress.step == 11 {
            if self.callbacks.geometry.is_mutable(info.shape)? {
                let shape = self.world.bodies.get(progress.body).ok_or("created mutable body is missing")?.shape;
                self.registry.register_body(shape, progress.body, self.callbacks.geometry)?;
            }
            progress.step = 12;
        }
        Ok(progress.body)
    }

    pub fn create(
        &mut self,
        info: &BodyInfo,
        addition: BodyAddition,
        activation: BodyActivation,
        modifier: Option<&mut dyn BodyInfoModifier>,
    ) -> Result<u32, String> {
        if let Some(modifier) = modifier {
            let mut local = info.clone();
            modifier.modify(&mut local)?;
            self.create_resolved(&local, addition, activation)
        } else {
            self.create_resolved(info, addition, activation)
        }
    }

    fn create_resolved(
        &mut self,
        info: &BodyInfo,
        addition: BodyAddition,
        activation: BodyActivation,
    ) -> Result<u32, String> {
        let body = if info.reserved_body_id & 0xffffff == 0xffffff {
            if !self.world.bodies.has_free_slot() {
                self.callbacks
                    .events
                    .body_capacity_requested(ActivationOwners {
                        bodies: self.world.bodies,
                        motions: self.world.motions,
                        deactivation: self.world.deactivation,
                        broadphase: self.world.broadphase,
                    })?;
            }
            self.world.bodies.reserve(self.properties)?
        } else {
            info.reserved_body_id
        };
        let motion = if info.reserved_motion_id == 0x7fff_ffff {
            if info.motion_type == 0 {
                0
            } else {
                let motion = self.callbacks.geometry.prepare_motion(info)?;
                self.world
                    .create_motion(&motion, self.cells, self.callbacks)?
            }
        } else {
            info.reserved_motion_id
        };
        self.initialize(body, motion, info, 2, addition, activation)?;
        Ok(body)
    }

    /// The original caller supplies the internal mode; c8e5b0 always passes2.
    pub fn initialize(
        &mut self,
        body: u32,
        motion: u32,
        info: &BodyInfo,
        mode: i32,
        addition: BodyAddition,
        activation: BodyActivation,
    ) -> Result<(), String> {
        let dynamic = self
            .world
            .motions
            .get(motion)
            .ok_or("body creation motion is missing")?
            .requires_dynamic_body();
        let assets = self.callbacks.geometry.creation_assets(info)?;
        self.world.bodies.initialize(body, info, dynamic, assets)?;
        if dynamic {
            let settings = AttachmentSettings {
                qualities: self.world.qualities,
                lod: self.lod,
                tolerance: self.world.tolerance,
                step: self.world.step,
                packing: self.world.packing,
            };
            attach_body_motion(
                self.world.bodies,
                self.world.motions,
                body,
                motion,
                Some(info.orientation),
                &settings,
                &mut Attachment {
                    geometry: self.callbacks.geometry,
                    deactivation: self.world.deactivation,
                },
            )?;
            if mode != 2 {
                // Both sequential operations resolve the SAME resource owner.
                // Reborrow it for each call rather than copying mass/bounds data.
                let geometry = RefCell::new(&mut *self.callbacks.geometry);
                recompute_motion_mass(
                    self.world.bodies,
                    self.world.motions,
                    motion,
                    self.lod.body_is_big_threshold,
                    |bodies, id| {
                        bodies.mass_frame(
                            id,
                            |shape| geometry.borrow().cached_mass(shape),
                            |shape, config| geometry.borrow().calculate_mass(shape, config),
                        )
                    },
                    |shape, frame| geometry.borrow_mut().bounds(shape, frame),
                )?;
            }
        } else {
            let body = self
                .world
                .bodies
                .get_mut(body)
                .ok_or("created static body is missing")?;
            body.next_attached_body_id = body.id;
            body.refresh_inverse_frame();
            body.update_size_and_static_lod(self.lod.body_is_big_threshold, false);
        }
        self.callbacks.events.body_created(
            ActivationOwners {
                bodies: self.world.bodies,
                motions: self.world.motions,
                deactivation: self.world.deactivation,
                broadphase: self.world.broadphase,
            },
            info,
            body,
        )?;
        if addition != BodyAddition::Disabled {
            self.world
                .add(&[body], addition, activation, self.callbacks)?;
        }
        // The source gate uses the Cinfo shape, then registers the current body
        // shape after creation/addition observers have had a chance to change it.
        if self.callbacks.geometry.is_mutable(info.shape)? {
            let shape = self
                .world
                .bodies
                .get(body)
                .ok_or("created mutable body is missing")?
                .shape;
            self.registry
                .register_body(shape, body, self.callbacks.geometry)?;
        }
        Ok(())
    }
}
