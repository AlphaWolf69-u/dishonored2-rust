//! Existing world ownership for the c86800 singleton keyframe and c87640 filter paths.
use super::*;
use crate::gameplay::physics_world::TransformCallbacks;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Change { Static, Keyframe, Filter(u32) }
pub(super) struct BodyModeProgress {
    body: u32, change: Change, step: u8, unlink: u8, attachment: u8,
    motion: u32, motion_step: u8, motion_info: Option<crate::gameplay::motion_info::MotionInfo>,
}

impl WorldStorage {
    /// c90020's source singleton branch returns without splitting a motion.
    /// A real compound split must be hosted before that body can be admitted.
    pub(crate) fn preflight_keyframe_single_body(&self, id: u32) -> Result<(), String> {
        let body = self.bodies.get(id).ok_or("keyframe body is absent")?;
        if body.next_attached_body_id != id { return Err("keyframe requires the compound motion split owner".into()); }
        if body.flags & 1 == 0 { self.motions.get(body.motion_id).ok_or("keyframe motion is absent")?; }
        Ok(())
    }

    /// 80b2a0 -> c86800(type1), after the actual c90020 singleton no-op.
    pub(crate) fn keyframe_single_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, id: u32) -> Result<(), String> {
        self.change_body_mode(geometry, events, id, Change::Keyframe)
    }

    /// c86800(type0), retaining the body and its broadphase identity.
    pub(crate) fn static_single_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, id: u32) -> Result<(), String> {
        self.change_body_mode(geometry, events, id, Change::Static)
    }

    /// c87640 stores the real collision filter then refreshes collision state
    /// and the existing general body signal; no body is unpublished or replaced.
    pub(crate) fn set_body_filter<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, id: u32, filter: u32) -> Result<(), String> {
        self.change_body_mode(geometry, events, id, Change::Filter(filter))
    }

    fn change_body_mode<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, id: u32, change: Change) -> Result<(), String> {
        let mut progress = if let Some(progress) = self.body_mode_progress.take() {
            if progress.body != id || progress.change != change {
                self.body_mode_progress = Some(progress);
                return Err("another body mode publication is incomplete".into());
            }
            progress
        } else {
            if !self.publication_idle() { return Err("body publication must finish before a mode change".into()); }
            let body = self.bodies.get(id).ok_or("body mode owner is absent")?;
            match change {
                Change::Static => {
                    if body.flags & 1 != 0 { return Ok(()); }
                    self.preflight_keyframe_single_body(id)?;
                }
                Change::Keyframe => {
                    self.preflight_keyframe_single_body(id)?;
                    if body.flags & 4 != 0 { return Ok(()); }
                }
                Change::Filter(filter) if body.filter_info == filter => return Ok(()),
                Change::Filter(_) => {}
            }
            BodyModeProgress { body: id, change, step: 0, unlink: 0, attachment: 0,
                motion: u32::MAX, motion_step: 0, motion_info: None }
        };
        let result = (|| {
            if change == Change::Static && progress.step == 0 {
                if self.bodies.get(id).ok_or("static body is absent")?.flags & 8 == 0 {
                    self.activate_body(geometry, events, id)?;
                }
                progress.step = 1;
            }
            self.edit_bodies(geometry, events, |creation| {
            use crate::gameplay::body_activation::ActivationOwners;
            if change == Change::Static {
                if progress.step == 1 {
                    let body = creation.world.bodies.get_mut(id).ok_or("static body retired")?;
                    body.flags = (body.flags & !6) | 1;
                    let bounds = creation.callbacks.geometry.bounds(body.selected_shape(3)?, body.transform)?;
                    body.bounds = creation.world.packing.pack_bounds(bounds, creation.world.tolerance);
                    if body.broadphase_id != u32::MAX { creation.world.broadphase.mark_bodies([id]); }
                    progress.step = 2;
                }
                if progress.step == 2 {
                    if creation.world.bodies.get(id).ok_or("static body retired")?.broadphase_id != u32::MAX {
                        creation.world.bodies.remove_active_attached(id, creation.world.motions)?;
                    }
                    progress.step = 3;
                }
                if progress.step == 3 {
                    let broadphase = &mut *creation.world.broadphase;
                    let callbacks = &mut *creation.callbacks;
                    creation.world.bodies.unlink_last_motion(id, creation.world.motions, creation.world.deactivation,
                        &mut progress.unlink, |bodies, motions, deactivation, motion| callbacks.events.motion_destroyed(
                            ActivationOwners { bodies, motions, deactivation, broadphase }, motion))?;
                    progress.step = 4;
                }
                if progress.step == 4 {
                    let body = creation.world.bodies.get_mut(id).ok_or("static body retired")?;
                    body.motion_id = 0;
                    body.refresh_inverse_frame();
                    progress.step = 5;
                }
                if progress.step == 5 { creation.world.invalidate_collision_cache(id, 0, creation.callbacks)?; progress.step = 6; }
                if progress.step == 6 {
                    creation.world.bodies.get_mut(id).ok_or("static body retired")?.flags &= !8;
                    progress.step = 7;
                }
                if progress.step == 7 {
                    creation.callbacks.events.motion_type_changed(ActivationOwners {
                        bodies: creation.world.bodies, motions: creation.world.motions,
                        deactivation: creation.world.deactivation, broadphase: creation.world.broadphase }, id, true)?;
                    progress.step = 8;
                }
                if progress.step == 8 {
                    creation.callbacks.transform_changed(ActivationOwners { bodies: creation.world.bodies,
                        motions: creation.world.motions, deactivation: creation.world.deactivation,
                        broadphase: creation.world.broadphase }, id, false)?;
                    progress.step = 9;
                }
                return Ok(());
            }
            if change == Change::Keyframe && creation.world.bodies.get(id).ok_or("keyframe body retired")?.flags & 1 != 0
                || progress.motion_info.is_some() {
                if progress.motion_step == 0 {
                    let body = creation.world.bodies.get(id).ok_or("keyframe body retired")?;
                    let geometry = &*creation.callbacks.geometry;
                    let mass = creation.world.bodies.mass_frame(id, |shape| geometry.cached_mass(shape),
                        |shape, config| geometry.calculate_mass(shape, config))?;
                    let properties = creation.world.bodies.mass_properties(id).ok_or("keyframe mass sidecar is absent")?;
                    let mut info = crate::gameplay::motion_info::MotionInfo::default();
                    info.pose = crate::gameplay::rigid_body::BodyPose::from_mass_frame(mass,
                        crate::physworld::rotation_quaternion(body.transform[..3].try_into().map_err(|_| "body basis is incomplete")?), body.transform[3]);
                    info.inverse_mass = 0.0;
                    info.inverse_inertia = [0.0; 4];
                    info.properties_id = properties.motion_properties_id;
                    progress.motion_info = Some(info);
                    progress.motion_step = 1;
                }
                if progress.motion_step == 1 {
                    if !creation.world.motions.has_free_slot() {
                        creation.callbacks.events.motion_capacity_requested(ActivationOwners {
                            bodies: creation.world.bodies, motions: creation.world.motions,
                            deactivation: creation.world.deactivation, broadphase: creation.world.broadphase })?;
                    }
                    progress.motion_step = 2;
                }
                if progress.motion_step == 2 { progress.motion = creation.world.motions.reserve()?; progress.motion_step = 3; }
                if progress.motion_step == 3 {
                    creation.world.motions.initialize(progress.motion, progress.motion_info.as_ref().ok_or("keyframe motion info is absent")?, creation.cells)?;
                    progress.motion_step = 4;
                }
                if progress.motion_step == 4 {
                    creation.world.deactivation.initialize_motion_state(progress.motion, true)?;
                    progress.motion_step = 5;
                }
                if progress.motion_step == 5 {
                    creation.callbacks.events.motion_created(ActivationOwners { bodies: creation.world.bodies,
                        motions: creation.world.motions, deactivation: creation.world.deactivation,
                        broadphase: creation.world.broadphase }, progress.motion_info.as_ref().ok_or("keyframe motion info is absent")?, progress.motion)?;
                    progress.motion_step = 6;
                }
                if progress.motion_step == 6 {
                    let body = creation.world.bodies.get_mut(id).ok_or("keyframe body retired")?;
                    body.flags = (body.flags & !1) | 2;
                    if body.broadphase_id != u32::MAX { creation.world.broadphase.mark_bodies([id]); }
                    body.motion_id = 0x7fff_ffff;
                    progress.motion_step = 7;
                }
                if progress.motion_step == 7 {
                    let settings = crate::gameplay::body_attachment::AttachmentSettings {
                        qualities: creation.world.qualities, lod: creation.lod, tolerance: creation.world.tolerance,
                        step: creation.world.step, packing: creation.world.packing };
                    crate::gameplay::body_attachment::attach_body_motion_resumable(creation.world.bodies, creation.world.motions,
                        id, progress.motion, None, &settings,
                        &mut super::super::body_creation::Attachment { geometry: creation.callbacks.geometry,
                            deactivation: creation.world.deactivation }, &mut progress.attachment)?;
                    progress.motion_step = 8;
                }
                if progress.motion_step == 8 { creation.world.invalidate_collision_cache(id, 0, creation.callbacks)?; progress.motion_step = 9; }
                if progress.motion_step == 9 {
                    creation.callbacks.events.motion_type_changed(ActivationOwners { bodies: creation.world.bodies,
                        motions: creation.world.motions, deactivation: creation.world.deactivation,
                        broadphase: creation.world.broadphase }, id, false)?;
                    progress.motion_step = 10;
                }
                progress.step = 2;
            }
            if progress.step == 0 {
                let body = creation.world.bodies.get_mut(id).ok_or("body mode owner retired")?;
                match change {
                    Change::Keyframe => {
                        creation.world.motions.get_mut(body.motion_id).ok_or("keyframe motion retired")?.inverse_mass = [0; 4];
                        body.flags |= 4;
                    }
                    Change::Filter(filter) => body.filter_info = filter,
                    Change::Static => unreachable!("static transition completed above"),
                }
                progress.step = 1;
            }
            if progress.step == 1 {
                creation.world.invalidate_collision_cache(id, 0, creation.callbacks)?;
                progress.step = 2;
            }
            if progress.step == 2 {
                creation.callbacks.transform_changed(crate::gameplay::body_activation::ActivationOwners {
                    bodies: &mut *creation.world.bodies, motions: &mut *creation.world.motions,
                    deactivation: &mut *creation.world.deactivation, broadphase: &mut *creation.world.broadphase,
                }, id, false)?;
                progress.step = 3;
            }
            Ok(())
            })
        })();
        if result.is_err() { self.body_mode_progress = Some(progress); }
        result
    }
}
