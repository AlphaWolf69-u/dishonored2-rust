//! Canonical owned buffers used by creation, publication and world queries.
//! This projects the corresponding c8c250 owners; geometry and components are
//! borrowed from the level, never duplicated in a query-only world.
use super::{
    body_creation::{BodyCreation, BodyCreationCallbacks, BodyCreationGeometry, BodyInfoModifier, DisabledCreationProgress},
    BodyAddition, BodyPublication, PublicationContext, PublicationEvents, WorldInfo,
};
use crate::assets::physworld::body::BodyInfo;
use crate::gameplay::{
    body_properties::BodyProperties,
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::{BodyActivation, BodyPropertyBits, BodyStorage},
    broadphase::{BroadphaseQuantization, HybridBroadphase},
    collision_body::{BodyCacheSettings, SweptBoundsStep},
    collision_queries::CollisionQueries,
    constraint_links::{ConstraintLink, ConstraintLinks},
    deactivation::DeactivationWorld,
    motion_storage::MotionStorage,
    materials::MaterialOwner,
    point_query::PointQuantization,
    shape_point_query::ShapePointContact,
    shape_registry::ShapeRegistry,
    signal::Signal,
    space_cells::SpaceCells,
    world_point_query::{WorldPointQuery, WorldPointRequest, WorldQueryShape},
};
mod motion_type;

pub(crate) struct WorldStorage {
    pub info: WorldInfo,
    pub bodies: BodyStorage,
    pub motions: MotionStorage,
    pub deactivation: DeactivationWorld,
    pub broadphase: HybridBroadphase,
    pub quantization: BroadphaseQuantization,
    pub qualities: BodyQualityLibrary,
    pub cells: SpaceCells,
    pub step: SweptBoundsStep,
    pub phase: u32,
    pub step_index: u32,
    pub properties: Vec<BodyPropertyBits>,
    pub materials: MaterialOwner<usize, usize>,
    pub motion_properties: crate::gameplay::motion_properties::MotionPropertiesLibrary<usize, usize>,
    pub(crate) motion_timespaces: Option<crate::gameplay::motion_timespaces::MotionTimespaces>,
    /// Per-body event chains (world+9e8). The source's signal-head word is an
    /// owned typed signal here; no parallel body-to-subscriber map is needed.
    pub body_events: BodyProperties<
        Signal<crate::gameplay::component_state::entity::ComponentHandle, super::BodyEventCallback>,
    >,
    pub constraint_heads: Vec<u32>,
    pub constraints: Vec<ConstraintLink>,
    pub constraint_next: Vec<[u32; 2]>,
    registry: ShapeRegistry,
    removal_scratch: super::publication::RemovalScratch,
    destruction: Option<DestructionProgress>,
    disabled_creation: Option<DisabledCreationProgress>,
    disabled_creation_busy: bool,
    body_mode_progress: Option<motion_type::BodyModeProgress>,
    activation_scratch: crate::gameplay::body_activation::BodyActivationController,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DestructionPhase { Unpublish, Notify, Resources, Motion, Release }
struct DestructionProgress {
    ids: Vec<u32>,
    mode: i32,
    index: usize,
    phase: DestructionPhase,
    motion_step: u8,
    shared_motion: Option<bool>,
}

const TIMESPACE_PROFILE_REMOVED: usize = 0;

impl WorldStorage {
    /// GameLocal 1400932d0: use the two real timespaces only for the game world;
    /// acoustic and other hknpWorld consumers keep their constructor library.
    pub(crate) fn initialize_game_timespaces(&mut self) -> Result<(), String> {
        if self.motion_timespaces.as_ref().is_some_and(|timespaces| timespaces.presets_ready()) {
            return Err("game physics timespaces are already initialized".into());
        }
        if !self.motion_properties.added.is_empty() {
            return Err("initial motion profiles require the original pre-observer phase".into());
        }
        if self.motion_timespaces.is_none() {
            self.motion_timespaces = Some(crate::gameplay::motion_timespaces::MotionTimespaces::new(2)?);
        }
        self.motion_timespaces.as_mut().ok_or("physics timespace owner is absent")?
            .initialize_presets(&mut self.motion_properties, |_, _, _, _| unreachable!("no initial profile observers"))?;
        self.motion_properties.removed.subscribe(self.registry.owner(), TIMESPACE_PROFILE_REMOVED);
        Ok(())
    }

    /// Retain exactly this component generation's unpublished c8e5b0 request.
    /// The component keeps its shape/declaration leases through resume or abort.
    pub(crate) fn begin_disabled_component_body(
        &mut self,
        component: crate::gameplay::component_state::entity::ComponentHandle,
        generation: u32,
        info: &BodyInfo,
    ) -> Result<(), String> {
        if self.disabled_creation.is_some() || self.disabled_creation_busy {
            return Err("another disabled component body is being created".into());
        }
        if !self.publication_idle() { return Err("world publication must finish before component body creation".into()); }
        if info.reserved_body_id & 0xffffff != 0xffffff {
            return Err("component creation cannot adopt an externally reserved body slot".into());
        }
        if info.reserved_motion_id != 0x7fff_ffff {
            let motion = self.motions.get(info.reserved_motion_id)
                .filter(|motion| info.reserved_motion_id != 0 && motion.requires_dynamic_body())
                .ok_or("component creation requires a live dynamic shared motion")?;
            let first = self.bodies.get(motion.first_body_id)
                .ok_or("shared motion has no live first body")?;
            if first.motion_id != info.reserved_motion_id {
                return Err("shared motion's first body has a different owner".into());
            }
        }
        self.disabled_creation = Some(DisabledCreationProgress::new((component, generation), info.clone()));
        Ok(())
    }

    pub(crate) fn disabled_component_creation_owner(&self)
        -> Option<(crate::gameplay::component_state::entity::ComponentHandle, u32)> {
        self.disabled_creation.as_ref().map(|progress| progress.key)
    }

    /// Resume successful reservations/initialization and entered observers
    /// without replaying their already completed phases.
    pub(crate) fn resume_disabled_component_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E,
        component: crate::gameplay::component_state::entity::ComponentHandle, generation: u32,
    ) -> Result<u32, String> {
        if self.disabled_creation_busy { return Err("disabled component body creation is active".into()); }
        let mut progress = self.disabled_creation.take().ok_or("disabled component body creation is absent")?;
        if progress.key != (component, generation) {
            self.disabled_creation = Some(progress);
            return Err("disabled component body belongs to another generation".into());
        }
        if progress.aborting {
            self.disabled_creation = Some(progress);
            return Err("disabled component body creation is being retired".into());
        }
        self.disabled_creation_busy = true;
        let result = self.edit_bodies(geometry, events, |mut creation| creation.create_disabled_resumable(&mut progress));
        self.disabled_creation_busy = false;
        if result.is_err() { self.disabled_creation = Some(progress); }
        result
    }

    /// Complete entered observers, then retire their actual objects. Never
    /// release another component's request or reuse an interrupted reservation.
    pub(crate) fn abort_disabled_component_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E,
        component: crate::gameplay::component_state::entity::ComponentHandle, generation: u32,
    ) -> Result<(), String> {
        if self.disabled_creation_busy { return Err("disabled component body creation is active".into()); }
        let mut progress = self.disabled_creation.take().ok_or("disabled component body creation is absent")?;
        if progress.key != (component, generation) {
            self.disabled_creation = Some(progress);
            return Err("disabled component body belongs to another generation".into());
        }
        progress.aborting = true;
        self.disabled_creation_busy = true;
        let result = (|| -> Result<(), String> {
            if progress.abort_step == 0 {
                self.edit_bodies(geometry, events, |mut creation| creation.finish_creation_callback(&mut progress))?;
                progress.abort_step = 1;
            }
            if progress.abort_step == 1 {
                if progress.step >= 11 {
                    events.destroyed(crate::gameplay::body_activation::ActivationOwners {
                        bodies: &mut self.bodies, motions: &mut self.motions,
                        deactivation: &mut self.deactivation, broadphase: &mut self.broadphase,
                    }, &[progress.body])?;
                }
                progress.abort_step = 2;
            }
            if progress.abort_step == 2 {
                if progress.step >= 9 {
                    self.body_events.release_body(progress.body, |signal| { signal.destroy(); Ok(()) })?;
                }
                progress.abort_step = 3;
            }
            if progress.abort_step == 3 {
                if progress.attachment_step != 0 && progress.info.reserved_motion_id != 0x7fff_ffff {
                    self.bodies.unlink_shared_motion(progress.body, &mut self.motions, &mut self.deactivation,
                        &mut progress.unlink_step)?;
                } else if progress.attachment_step != 0 {
                    let broadphase = &mut self.broadphase;
                    self.bodies.unlink_last_motion(progress.body, &mut self.motions, &mut self.deactivation,
                        &mut progress.unlink_step, |bodies, motions, deactivation, motion| events.motion_destroyed(
                            crate::gameplay::body_activation::ActivationOwners {
                                bodies, motions, deactivation, broadphase: &mut *broadphase,
                            }, motion))?;
                } else if progress.info.reserved_motion_id == 0x7fff_ffff
                    && progress.motion != u32::MAX && progress.motion != 0 {
                    if progress.step == 5 {
                        self.motions.cancel_uninitialized_reservation(progress.motion)?;
                    } else {
                        if progress.step >= 8 {
                            events.motion_destroyed(crate::gameplay::body_activation::ActivationOwners {
                                bodies: &mut self.bodies, motions: &mut self.motions,
                                deactivation: &mut self.deactivation, broadphase: &mut self.broadphase,
                            }, progress.motion)?;
                        }
                        progress.abort_step = 4;
                        self.motions.release_detached(progress.motion)?;
                    }
                }
                progress.abort_step = 5;
            }
            if progress.abort_step == 4 {
                self.motions.release_detached(progress.motion)?;
                progress.abort_step = 5;
            }
            if progress.abort_step == 5 {
                if progress.body != u32::MAX {
                    if progress.step < 9 { self.bodies.cancel_uninitialized_reservation(progress.body)?; }
                    else { self.bodies.release_detached(progress.body)?; }
                }
                progress.abort_step = 6;
            }
            Ok(())
        })();
        self.disabled_creation_busy = false;
        if result.is_err() { self.disabled_creation = Some(progress); }
        result
    }

    pub(crate) fn remove_motion_property(&mut self, id: u16,
        mut dispatch: impl FnMut(&mut crate::gameplay::signal::Signal<usize, usize>, usize, usize, u16) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.motion_properties.removal_idle() && !self.publication_idle() {
            return Err("world physics work must finish before removing a motion property".into());
        }
        let owner = self.registry.owner();
        let timespaces = &mut self.motion_timespaces;
        self.motion_properties.remove(id, |signal, subscriber, callback, profile| {
            if timespaces.is_some() && subscriber == owner && callback == TIMESPACE_PROFILE_REMOVED {
                timespaces.as_mut().ok_or("game physics timespace owner is absent")?.forget_profile(profile)
            } else {
                dispatch(signal, subscriber, callback, profile)
            }
        })
    }


    /// Host mutation fence for resumable owner operations.
    pub(crate) fn publication_idle(&self) -> bool {
        self.removal_scratch.is_idle() && self.destruction.is_none()
            && self.body_mode_progress.is_none() && self.activation_scratch.is_idle()
            && self.disabled_creation.is_none() && !self.disabled_creation_busy
            && self.motion_properties.removal_idle()
    }

    /// 140c8f080 cancels pending publication and removes live broadphase bodies.
    pub(crate) fn unpublish_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, ids: &[u32], mode: i32,
    ) -> Result<(), String> {
        if self.disabled_creation.is_some() || self.disabled_creation_busy { return Err("disabled creation must complete or abort before unpublication".into()); }
        if self.destruction.is_some() { return Err("body destruction must finish before another unpublication".into()); }
        if self.body_mode_progress.is_some() || !self.activation_scratch.is_idle() {
            return Err("body mode or activation update must finish before unpublication".into());
        }
        let mut scratch = std::mem::take(&mut self.removal_scratch);
        let result = self.edit_bodies(geometry, events, |creation|
            creation.world.remove(ids, mode, &mut scratch, creation.callbacks));
        self.removal_scratch = scratch;
        result
    }

    /// 140c8f6b0 releases observers, shape registration, motion and body slots in order.
    pub(crate) fn destroy_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, ids: &[u32], mode: i32,
    ) -> Result<(), String> {
        if self.disabled_creation.is_some() || self.disabled_creation_busy { return Err("disabled creation must complete or abort before destruction".into()); }
        // A failed Stop must finish its publication request before destruction.
        if self.destruction.is_none() && !self.removal_scratch.is_idle() {
            return Err("body unpublication must finish before destruction".into());
        }
        if self.body_mode_progress.is_some() || !self.activation_scratch.is_idle() {
            return Err("body mode or activation update must finish before destruction".into());
        }
        let trace = std::env::var("DH2_DEBUG_COLLISION_SOURCES").as_deref() == Ok("1");
        let start = if trace { crate::loading::thread_cycles() } else { None };
        let before = if trace { self.removal_scratch.retained_bytes() } else { 0 };
        let mut progress = if let Some(progress) = self.destruction.take() {
            if progress.ids != ids || progress.mode != mode {
                self.destruction = Some(progress);
                return Err("another body destruction is incomplete".into());
            }
            progress
        } else {
            let mut owned = Vec::new();
            owned.try_reserve_exact(ids.len()).map_err(|error| format!("body destruction identity: {error}"))?;
            owned.extend_from_slice(ids);
            DestructionProgress { ids: owned, mode, index: 0, phase: DestructionPhase::Unpublish, motion_step: 0, shared_motion: None }
        };
        let result = (|| -> Result<(), String> {
            loop {
                match progress.phase {
                    DestructionPhase::Unpublish => {
                        self.unpublish_bodies(geometry, events, &progress.ids, mode)?;
                        progress.phase = DestructionPhase::Notify;
                    }
                    DestructionPhase::Notify => {
                        events.destroyed(crate::gameplay::body_activation::ActivationOwners {
                            bodies: &mut self.bodies, motions: &mut self.motions,
                            deactivation: &mut self.deactivation, broadphase: &mut self.broadphase,
                        }, &progress.ids)?;
                        progress.phase = DestructionPhase::Resources;
                    }
                    DestructionPhase::Resources => {
                        if progress.index == progress.ids.len() { return Ok(()); }
                        self.release_body_resources(geometry, progress.ids[progress.index])?;
                        progress.phase = DestructionPhase::Motion;
                    }
                    DestructionPhase::Motion => {
                        let id = progress.ids[progress.index];
                        let body = self.bodies.get(id).ok_or("destroyed body disappeared")?;
                        let motion = body.motion_id;
                        let shared = *progress.shared_motion.get_or_insert(body.next_attached_body_id != id);
                        if motion != 0 && motion != 0x7fff_ffff {
                            if !shared {
                                let broadphase = &mut self.broadphase;
                                self.bodies.unlink_last_motion(id, &mut self.motions, &mut self.deactivation,
                                    &mut progress.motion_step,
                                    |bodies, motions, deactivation, motion| events.motion_destroyed(
                                        crate::gameplay::body_activation::ActivationOwners {
                                            bodies, motions, deactivation, broadphase: &mut *broadphase,
                                        }, motion))?;
                            } else {
                                self.bodies.unlink_shared_motion(id, &mut self.motions, &mut self.deactivation,
                                    &mut progress.motion_step)?;
                            }
                        }
                        progress.phase = DestructionPhase::Release;
                    }
                    DestructionPhase::Release => {
                        self.bodies.release_detached(progress.ids[progress.index])?;
                        progress.index += 1;
                        progress.motion_step = 0;
                        progress.shared_motion = None;
                        progress.phase = DestructionPhase::Resources;
                    }
                }
            }
        })();
        if result.is_err() { self.destruction = Some(progress); }
        result?;
        if trace {
            let cycles = start.and_then(|start| crate::loading::thread_cycles().map(|end| end.saturating_sub(start)));
            eprintln!("MEMORY collision body retirement: bodies={} mode={mode} scratch_before={before} scratch_after={} cycles={cycles:?}",
                ids.len(), self.removal_scratch.retained_bytes());
        }
        Ok(())
    }

    /// 140ceab10 and 140cdffd0 release the property chain and mutable-shape registration.
    pub(crate) fn release_body_resources<G: BodyCreationGeometry>(
        &mut self, geometry: &mut G, id: u32,
    ) -> Result<(), String> {
        let shape = self.bodies.get(id).ok_or("retired body is missing")?.shape;
        let mutable = geometry.is_mutable(shape)?;
        if mutable { self.registry.validate_body_removal(shape, id, geometry)?; }
        self.body_events.release_body(id, |signal| { signal.destroy(); Ok(()) })?;
        if mutable {
            self.registry.remove_body(shape, id, geometry)?;
        }
        Ok(())
    }


    pub fn new(info: WorldInfo, identity: usize, materials: MaterialOwner<usize, usize>) -> Result<Self, String> {
        if info.user_body_buffer.is_some() || info.user_motion_buffer.is_some() {
            return Err("external physics buffers require their actual owner".into());
        }
        if info.motion_properties_library.is_some() {
            return Err("external motion-property libraries require their actual owner".into());
        }
        if info.quality_library.is_some()
            || info.broad_phase_config.is_some()
            || info.broad_phase_type != 1
        {
            return Err(
                "custom physics quality/broadphase resources require their actual owner".into(),
            );
        }
        let [x, y, z, _] = info.gravity;
        let squared = crate::float::add(crate::float::add(crate::float::multiply(y, y), crate::float::multiply(x, x)), crate::float::multiply(z, z));
        let gravity = if squared <= 0. { 0. } else {
            let estimate = crate::gameplay::convex_support::faces::reciprocal_sqrt_estimate(squared);
            let correction = crate::float::subtract(3., crate::float::multiply(crate::float::multiply(estimate, squared), estimate));
            crate::float::multiply(crate::float::multiply(crate::float::multiply(correction, estimate), 0.5), squared)
        };
        let mut motion_properties = crate::gameplay::motion_properties::MotionPropertiesLibrary::default();
        // This newly constructed library has no observers yet.
        motion_properties.initialize(gravity, |_, _, _, _| {})?;
        let qualities = BodyQualityLibrary::new(BodyQualityConfig {
            enable_welding_for_default_objects: info.enable_welding_for_default_objects,
            enable_welding_for_critical_objects: info.enable_welding_for_critical_objects,
            enable_contact_caching: info.enable_contact_caching,
        });
        let cells = if info.simulation_type == 1 {
            SpaceCells::dynamic(info.num_splitter_cells as usize)?
        } else {
            SpaceCells::Single
        };
        Ok(Self {
            bodies: BodyStorage::new(info.body_buffer_capacity as usize)?,
            motions: MotionStorage::new(info.motion_buffer_capacity as usize)?,
            deactivation: DeactivationWorld::for_motion_capacity(
                info.motion_buffer_capacity as usize,
            )?,
            broadphase: HybridBroadphase::with_body_capacity(info.body_buffer_capacity as usize),
            quantization: BroadphaseQuantization::new(info.broad_phase_aabb),
            registry: ShapeRegistry::new(identity),
            removal_scratch: Default::default(),
            destruction: None,
            disabled_creation: None,
            disabled_creation_busy: false,
            body_mode_progress: None,
            activation_scratch: Default::default(),
            qualities,
            cells,
            step: SweptBoundsStep::initial(),
            phase: 0,
            step_index: 0,
            properties: Vec::new(),
            materials,
            motion_properties,
            motion_timespaces: None,
            body_events: BodyProperties::default(),
            constraint_heads: Vec::new(),
            constraints: Vec::new(),
            constraint_next: Vec::new(),
            info,
        })
    }

    /// c8e5b0 on the same buffers later read by WorldPointQuery. Required
    /// creation/activation signals resolve through the caller's live component
    /// owner, including changes made by observers during this call.
    pub fn create_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        info: &BodyInfo,
        addition: BodyAddition,
        activation: BodyActivation,
        modifier: Option<&mut dyn BodyInfoModifier>,
    ) -> Result<u32, String> {
        if !self.publication_idle() { return Err("world retirement must finish before publication".into()); }
        self.edit_bodies(geometry, events, |mut creation| {
            creation.create(info, addition, activation, modifier)
        })
    }

    /// c8f000 publishes active and deactivated queues in that order. The
    /// default hybrid broadphase's finalize virtual is 1400998b0 (a no-op).
    pub fn flush_additions<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
    ) -> Result<(), String> {
        if !self.publication_idle() { return Err("world retirement must finish before publication".into()); }
        for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
            let ids = self.bodies.queued_additions(activation).to_vec();
            if !ids.is_empty() { self.add_bodies(geometry, events, &ids, BodyAddition::Immediate, activation)?; }
        }
        self.bodies.reset_queued_additions();
        Ok(())
    }

    /// c8e930 admission is separate from creation during component startup.
    pub fn add_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        ids: &[u32],
        addition: BodyAddition,
        activation: BodyActivation,
    ) -> Result<(), String> {
        if !self.publication_idle() { return Err("world retirement must finish before publication".into()); }
        self.edit_bodies(geometry, events, |creation| {
            creation
                .world
                .add(ids, addition, activation, creation.callbacks)
        })
    }

    pub fn set_material<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        id: u32,
        material: u16,
        mode: i32,
    ) -> Result<bool, String> {
        if !self.publication_idle() { return Err("world retirement must finish before publication".into()); }
        self.edit_bodies(geometry, events, |creation| {
            creation
                .world
                .set_material(id, material, mode, creation.callbacks)
        })
    }

    pub(crate) fn activate_body<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, id: u32,
    ) -> Result<bool, String> {
        let mut scratch = std::mem::take(&mut self.activation_scratch);
        let result = if !self.publication_idle() {
            Err("world publication must finish before body activation".into())
        } else {
            self.edit_bodies(geometry, events, |creation|
                creation.world.activate_body_resumable(id, &mut scratch, creation.callbacks))
        };
        self.activation_scratch = scratch;
        result
    }

    fn edit_bodies<G: BodyCreationGeometry, E: PublicationEvents + BodyCreationCallbacks, R>(
        &mut self,
        geometry: &mut G,
        events: &mut E,
        edit: impl FnOnce(BodyCreation<'_, '_, '_, G, E>) -> Result<R, String>,
    ) -> Result<R, String> {
        let links = || ConstraintLinks {
            heads: &self.constraint_heads,
            constraints: &self.constraints,
            next: &self.constraint_next,
        };
        let tolerance = [self.info.collision_tolerance; 4];
        let mut callbacks = PublicationContext {
            geometry,
            events,
            cache: BodyCacheSettings {
                bounds_tolerance: tolerance,
                cache_margin: self.info.aabb_margin,
                lod: Some(self.info.lod),
                step: &self.step,
                packing: &self.quantization.packed,
            },
            qualities: &self.qualities,
            constraints: links(),
            enable_deactivation: self.info.enable_deactivation,
            phase: self.phase,
            step_index: self.step_index,
        };
        let mut world = BodyPublication {
            bodies: &mut self.bodies,
            motions: &mut self.motions,
            deactivation: &mut self.deactivation,
            broadphase: &mut self.broadphase,
            packing: &self.quantization.packed,
            qualities: &self.qualities,
            constraints: links(),
            tolerance,
            step: &self.step,
            enable_deactivation: self.info.enable_deactivation,
            phase: self.phase,
            step_index: self.step_index,
        };
        edit(BodyCreation {
            world: &mut world,
            callbacks: &mut callbacks,
            registry: &mut self.registry,
            lod: &self.info.lod,
            cells: &self.cells,
            properties: &mut self.properties,
        })
    }

    /// Execute an immediate query against the canonical published bodies/trees.
    pub fn point_query<'a>(
        &self,
        executor: &mut WorldPointQuery,
        request: &WorldPointRequest,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
        contacts: &mut Vec<ShapePointContact>,
    ) -> Result<(), String> {
        executor.all_hits(&self.bodies, &self.broadphase.query_trees(), PointQuantization {
            offset: self.quantization.offset[..3].try_into().unwrap(),
            scale: self.quantization.inverse_scale[..3].try_into().unwrap(),
        }, request.point, request.hit, request.filter, resolve, contacts)
    }

    /// Resolve queued requests against these actual published bodies/trees.
    /// The resource callback resolves shape handles, never supplies contacts.
    pub fn resolve_point_queries<'a>(
        &self,
        executor: &mut WorldPointQuery,
        queries: &mut CollisionQueries<WorldPointRequest, ShapePointContact>,
        resolve: impl FnMut(usize) -> Result<WorldQueryShape<'a>, String>,
    ) -> Result<(), String> {
        executor.resolve_pending(
            queries,
            &self.bodies,
            &self.broadphase.query_trees(),
            PointQuantization {
                offset: self.quantization.offset[..3].try_into().unwrap(),
                scale: self.quantization.inverse_scale[..3].try_into().unwrap(),
            },
            resolve,
        )
    }

    /// Detach mutable-shape observers while the level's resources still live.
    pub fn release(self, geometry: &mut impl BodyCreationGeometry) -> Result<(), String> {
        self.registry.release(geometry)
    }

    pub fn detach_shape_observers(&mut self, geometry: &mut impl BodyCreationGeometry) -> Result<usize, String> {
        self.registry.detach_all(geometry)
    }
}

#[cfg(test)]
mod tests;
