//! Ordinary rigid-world manifold ownership and stream-solver dispatch.
//! Geometry/resources remain borrowed from the canonical world's shape owner.
use super::{
    body_creation::{BodyCreationCallbacks, BodyCreationGeometry},
    contact_modes::{
        validation::{self, ValidationReference},
        Response, TriggerEvent, TriggerTransition,
    },
    storage::{simulation::SimulationContacts, WorldStorage},
    trigger_events::TriggerEvents,
    PublicationEvents,
};
use crate::gameplay::{
    broadphase::OverlapWorkspace,
    collision_body::CollisionBody,
    collision_filter::body_pair_collides,
    constraint_solver::{
        ball_socket::SolverVelocities,
        contact::{
            self,
            correspondence::{FacePair, PointCorrespondence},
            material::PairMaterial,
            restitution::RestitutionCache,
            ContactBuild, ContactCache, ContactPoint, ContactRows, ContactState,
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
    pub resource_shapes: [usize; 2],
    pub material_ids: [u16; 2],
    /// Actual generator manifold+93, consumed by cf32a0. Warm generation
    /// clears it; the cold caller owns its initial value.
    pub new_manifold: bool,
    /// Actual manifold+4, the four-lane minimum of its selected distances.
    pub separation: f32,
}

/// Required full manifold producer; closest-point hits are insufficient.
pub(crate) trait ContactGeometry: BodyCreationGeometry {
    fn contact_manifolds(
        &mut self,
        world: &WorldStorage,
        bodies: [&CollisionBody; 2],
        context_flags: u8,
        workspace: &mut super::contact_geometry::GeometryWorkspace,
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
    callback_state: ContactCallbackState,
    /// ce4cc0's per-pair LOD bindings, copied into cache+0e by d14cb0.
    context_flags: u8,
    event_kind: u8,
    response: Response,
    separation: f32,
    validation: Option<ValidationReference>,
}

/// d14cb0 clears cache+22/+24. Modifier and body callbacks edit the actual
/// retained cache; stream gates borrow its real ContactRows (81b420/81b440).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ContactCallbackState {
    /// Actual persistent cache+0a policy flags, distinct from +22 callbacks.
    pub cache_flags: u16,
    pub contact_flags: u16,
    pub modifier_flags: u32,
    /// The source cache+28/+30 references the current Jacobian. Numeric row
    /// storage may remain for deferred validation after this pointer retires.
    pub stream_live: bool,
}

/// Numeric output is handed to the real body-contact callback publisher. It
/// carries no fabricated event flag; that publisher owns the source event ABI.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SolvedContact {
    pub bodies: [u32; 2],
    pub shape_keys: [u32; 2],
    pub material_ids: [u16; 2],
    pub normal: [f32; 3],
    pub points: [ContactPoint; 4],
    pub impulses: [f32; 4],
    pub count: u8,
    /// cf32a0:1 created,2 persisted. Geometry supplies actual newManifold+93.
    pub event_kind: u8,
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
    serial: u64,
    prepared_callbacks_ready: bool,
    callback_admission: Option<(usize, bool)>,
    geometry: super::contact_geometry::GeometryWorkspace,
    triggers: TriggerEvents,
    validation_commands: Vec<Key>,
    validation_cursor: usize,
    broadphase_triggers: HashMap<[u32; 2], u64>,
    collision_pending: bool,
    collision_pair: usize,
    collision_geometry_ready: bool,
    activation_link: usize,
    activation_side: usize,
    /// cf33d0/cf3460 keep the actual cache and old Jacobian alive until the
    /// requested destroyed/removed callbacks have acknowledged both sides.
    retired: Vec<Manifold>,
    retire_keys: Vec<Key>,
    retired_cursor: usize,
    retired_event: usize,
    retired_side: usize,
    retired_admission: Option<u16>,
}
impl Contacts {
    pub(crate) fn new() -> Self {
        Self {
            manifolds: HashMap::new(),
            pairs: Vec::new(),
            overlaps: Vec::new(),
            overlap_workspace: Default::default(),
            body_links: Vec::new(),
            active_keys: Vec::new(),
            motion_to_solver: Vec::new(),
            projected: Vec::new(),
            serial: 0,
            prepared_callbacks_ready: true,
            callback_admission: None,
            geometry: Default::default(),
            triggers: Default::default(),
            validation_commands: Vec::new(),
            validation_cursor: 0,
            broadphase_triggers: Default::default(),
            collision_pending: false,
            collision_pair: 0,
            collision_geometry_ready: false,
            activation_link: 0,
            activation_side: 0,
            retired: Vec::new(),
            retire_keys: Vec::new(),
            retired_cursor: 0,
            retired_event: 0,
            retired_side: 0,
            retired_admission: None,
        }
    }
    pub(crate) fn forget_body(&mut self, body: u32) -> Result<(), String> {
        self.triggers.forget_body(body)
    }
    pub(crate) fn trigger_events_pending(&self) -> bool {
        self.triggers.pending()
    }
    pub(crate) fn dispatch_trigger_events(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut crate::assets::entities::triggers::TriggerAssets,
        resolve_trigger: impl FnMut(
            crate::gameplay::component_state::entity::ComponentHandle,
        ) -> Result<Option<u32>, String>,
        deliver: impl FnMut(
            &mut WorldStorage,
            &mut crate::assets::entities::triggers::TriggerAssets,
            TriggerTransition,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        self.triggers
            .dispatch(world, geometry, resolve_trigger, deliver)
    }
    pub(crate) fn prepared_callbacks_pending(&self) -> bool {
        !self.prepared_callbacks_ready
    }
    pub(crate) fn callbacks_ready(&self) -> bool {
        self.prepared_callbacks_ready
    }
    pub(crate) fn retired_callbacks_pending(&self) -> bool {
        self.retired_cursor < self.retired.len()
    }
    fn queue_retirement(&mut self, mut entry: Manifold) {
        if let Some(flags) = entry.response.retire(&mut entry.callback_state) {
            self.triggers.push(TriggerEvent {
                bodies: entry.key.bodies,
                shape_keys: entry.key.shapes,
                flags,
            });
        }
        if entry.callback_state.contact_flags & 0xc0 != 0 {
            self.retired.push(entry);
            self.prepared_callbacks_ready = false;
        }
    }
    pub(crate) fn dispatch_retired(
        &mut self,
        world: &WorldStorage,
        step: &SolverParameters,
        mut dispatch: impl FnMut(
            &SolvedContact,
            Option<&mut ContactCallbackState>,
            Option<&mut ContactRows>,
            u8,
            usize,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        while let Some(entry) = self.retired.get_mut(self.retired_cursor) {
            // Admission survives a fallible callback that mutates cache flags.
            let flags = *self
                .retired_admission
                .get_or_insert(entry.callback_state.contact_flags);
            while self.retired_event < 2 {
                let event_kind = [4, 8][self.retired_event];
                let mask = [0x40, 0x80][self.retired_event];
                if flags & mask != 0 {
                    let mut contact = SolvedContact {
                        bodies: entry.key.bodies,
                        shape_keys: entry.key.shapes,
                        material_ids: entry.material_ids,
                        normal: entry.normal,
                        points: [ContactPoint::default(); 4],
                        impulses: entry.cache.normal_impulses,
                        count: 0,
                        event_kind,
                    };
                    // Event8 has a null cache. For event4, cb9320 is called
                    // only after the actual stream gate81b420 succeeds.
                    if event_kind == 4 && entry.callback_state.stream_live {
                        let rows = entry
                            .rows
                            .as_ref()
                            .ok_or("retired live contact stream absent")?;
                        if rows.stream_type == 0 {
                            let bodies = entry.key.bodies.map(|id| world.bodies.get(id));
                            let a = bodies[0]
                                .ok_or("retired contact body A removed before callback")?;
                            let b = bodies[1]
                                .ok_or("retired contact body B removed before callback")?;
                            let motions = [
                                world
                                    .motions
                                    .get(a.motion_id)
                                    .ok_or("retired contact motion A absent")?,
                                world
                                    .motions
                                    .get(b.motion_id)
                                    .ok_or("retired contact motion B absent")?,
                            ];
                            contact.points = rows.reconstruct(motions, step);
                            contact.count = rows.count;
                        }
                    }
                    while self.retired_side < 2 {
                        let cache = (event_kind != 8).then_some(&mut entry.callback_state);
                        let rows = if event_kind == 8 {
                            None
                        } else {
                            entry.rows.as_mut()
                        };
                        dispatch(&contact, cache, rows, event_kind, self.retired_side)?;
                        self.retired_side += 1;
                    }
                }
                self.retired_side = 0;
                self.retired_event += 1;
            }
            self.retired_admission = None;
            self.retired_event = 0;
            self.retired_cursor += 1;
        }
        self.retired.clear();
        self.retired_cursor = 0;
        Ok(())
    }
    pub(crate) fn dispatch_prepared(
        &mut self,
        world: &WorldStorage,
        step: &SolverParameters,
        row_cursor: &mut usize,
        side_cursor: &mut usize,
        mut dispatch: impl FnMut(
            &SolvedContact,
            Option<&mut ContactCallbackState>,
            Option<&mut ContactRows>,
            u8,
            usize,
        ) -> Result<(), String>,
    ) -> Result<(), String> {
        self.dispatch_retired(world, step, &mut dispatch)?;
        while let Some(&key) = self.active_keys.get(*row_cursor) {
            let entry = self
                .manifolds
                .get_mut(&key)
                .ok_or("prepared contact cache retired")?;
            // cf32a0 emits persisted events only when its actual cache has
            // requested them. The created event is unconditional.
            let admitted = match self.callback_admission {
                Some((row, admitted)) if row == *row_cursor => admitted,
                Some(_) => return Err("another contact event row is incomplete".into()),
                None => {
                    let admitted =
                        entry.event_kind == 1 || entry.callback_state.contact_flags & 0x20 != 0;
                    self.callback_admission = Some((*row_cursor, admitted));
                    admitted
                }
            };
            if admitted {
                let contact = SolvedContact {
                    bodies: entry.key.bodies,
                    shape_keys: entry.key.shapes,
                    material_ids: entry.material_ids,
                    normal: entry.normal,
                    points: entry.points,
                    impulses: entry.cache.normal_impulses,
                    count: entry.correspondence.count,
                    event_kind: entry.event_kind,
                };
                let rows = entry
                    .rows
                    .as_mut()
                    .ok_or("prepared contact stream absent")?;
                while *side_cursor < 2 {
                    dispatch(
                        &contact,
                        Some(&mut entry.callback_state),
                        Some(rows),
                        contact.event_kind,
                        *side_cursor,
                    )?;
                    *side_cursor += 1;
                }
            }
            *side_cursor = 0;
            self.callback_admission = None;
            *row_cursor += 1;
        }
        self.prepared_callbacks_ready = true;
        self.callback_admission = None;
        Ok(())
    }
    /// The canonical mutable-shape publisher invokes this with its body ID.
    /// Dropping these numeric rows releases no shape/resource ownership.
    pub(crate) fn invalidate_body(&mut self, body: u32) {
        self.geometry.invalidate_body(body);
        self.retire_keys.clear();
        self.retire_keys.extend(
            self.manifolds
                .keys()
                .filter(|key| key.bodies.contains(&body))
                .copied(),
        );
        while let Some(key) = self.retire_keys.pop() {
            if let Some(entry) = self.manifolds.remove(&key) {
                self.queue_retirement(entry);
            }
        }
        let triggers = &mut self.triggers;
        self.broadphase_triggers.retain(|&bodies, _| {
            if !bodies.contains(&body) {
                return true;
            }
            triggers.push(TriggerEvent {
                bodies,
                shape_keys: [u32::MAX; 2],
                flags: 0x100,
            });
            false
        });
        self.active_keys.retain(|key| !key.bodies.contains(&body));
        self.body_links.retain(|ids| !ids.contains(&body));
    }
    pub(crate) fn clear(&mut self) {
        self.geometry.clear();
        self.triggers.clear();
        self.validation_commands.clear();
        self.validation_cursor = 0;
        self.broadphase_triggers.clear();
        self.manifolds.clear();
        self.retired.clear();
        self.retire_keys.clear();
        self.retired_cursor = 0;
        self.retired_event = 0;
        self.retired_side = 0;
        self.retired_admission = None;
        self.pairs.clear();
        self.overlaps.clear();
        self.body_links.clear();
        self.active_keys.clear();
        self.motion_to_solver.clear();
        self.projected.clear();
        self.prepared_callbacks_ready = true;
        self.callback_admission = None;
        self.collision_pending = false;
        self.collision_pair = 0;
        self.collision_geometry_ready = false;
        self.activation_link = 0;
        self.activation_side = 0;
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
        if !self.collision_pending {
            self.serial = self.serial.wrapping_add(1);
            self.geometry.begin(world);
            self.pairs.clear();
            self.body_links.clear();
            self.active_keys.clear();
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
                                && body_pair_collides(body.filter_info, other.filter_info)
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
            self.collision_pair = 0;
            self.collision_geometry_ready = false;
            self.activation_link = 0;
            self.activation_side = 0;
            self.collision_pending = true;
        }
        while let Some(&ids) = self.pairs.get(self.collision_pair) {
            let mut ids = ids;
            let first = world.bodies.get(ids[0]).ok_or("contact body A retired")?;
            let second = world.bodies.get(ids[1]).ok_or("contact body B retired")?;
            let mut bodies = [first, second];
            if !world.collision_pairs.accepts(bodies)? {
                self.collision_pair += 1;
                continue;
            }
            //ce4cc0 selects broadphase triggers from the actual body material
            //before any child geometry work. Bodies without notifybit10 produce
            //no event/cache; an overlap bound is the source's whole test here.
            let base = bodies.map(|body| {
                world
                    .materials
                    .library
                    .get(body.material_id)
                    .ok_or("body material absent")
            });
            if Response::from_materials([base[0]?, base[1]?])? == Response::Broadphase {
                if (bodies[0].flags | bodies[1].flags) & 0x10 != 0 {
                    if self.broadphase_triggers.insert(ids, self.serial).is_none() {
                        self.triggers.push(TriggerEvent {
                            bodies: ids,
                            shape_keys: [u32::MAX; 2],
                            flags: 0,
                        });
                    }
                }
                self.collision_pair += 1;
                continue;
            }
            //ce4cc0 excludes two immovable bodies from ordinary dispatch;
            //the creator's actual mask50 retains sensor/special exceptions.
            if bodies.iter().all(|body| body.flags & 5 != 0)
                && (bodies[0].flags | bodies[1].flags) & 0x50 == 0
            {
                self.collision_pair += 1;
                continue;
            }
            let serial = self.serial;
            // ca6c20 stores the farthest bound distance at body+7e. The
            // correspondence threshold reads this body cache, not a shape.
            let minimum_body_radius = f32::from_bits(u32::from(bodies[0].packed_radius) << 16)
                .min(f32::from_bits(u32::from(bodies[1].packed_radius) << 16));
            // cc8b20 is the LOD-manager virtual at world+950, not the
            // collision-filter virtual. ce4cc0 resolves each actual body's
            // channel selector and preserves their shared recovery bit80.
            let variants = world.lod_variants.table();
            let mut channels = *variants
                .get(usize::from(bodies[0].lod_flags))
                .and_then(|row| row.get(usize::from(bodies[1].lod_flags)))
                .ok_or("contact LOD flags exceed the source binding table")?;
            let selected = [
                bodies[0].selected_shape(usize::from(channels >> 4))?,
                bodies[1].selected_shape(usize::from(channels & 7))?,
            ];
            let headers = [geometry.header(selected[0])?, geometry.header(selected[1])?];
            if headers[0].dispatch_type > headers[1].dispatch_type
                || (headers[0].dispatch_type == headers[1].dispatch_type
                    && (headers[0].shape_type > headers[1].shape_type
                        || (headers[0].shape_type == headers[1].shape_type
                            && bodies[0].flags & 2 == 0
                            && bodies[1].flags & 1 == 0)))
            {
                ids.swap(0, 1);
                bodies.swap(0, 1);
                channels = ((channels >> 4) & 7) | ((channels & 7) << 4);
            }
            let selector_a = *bodies[0]
                .shape_selectors
                .get(usize::from(channels >> 4))
                .ok_or("contact A shape binding channel invalid")?;
            let selector_b = *bodies[1]
                .shape_selectors
                .get(usize::from(channels & 7))
                .ok_or("contact B shape binding channel invalid")?;
            let context_flags = (selector_a & selector_b & 0x80) | channels;
            let manifolds = &mut self.manifolds;
            let projected = &mut self.projected;
            let links = &mut self.body_links;
            let active_keys = &mut self.active_keys;
            let triggers = &mut self.triggers;
            geometry.contact_manifolds(
                world,
                bodies,
                context_flags,
                &mut self.geometry,
                &mut |patch| {
                    if patch.selected.is_empty() {
                        return Ok(());
                    }
                    let key = Key {
                        bodies: ids,
                        shapes: patch.shape_keys,
                        resources: patch.resource_shapes,
                    };
                    let materials = patch.material_ids.map(|id| {
                        world
                            .materials
                            .library
                            .get(id)
                            .ok_or("leaf material absent")
                    });
                    let response = Response::from_leaf_materials([materials[0]?, materials[1]?])?;
                    let slot = manifolds.entry(key);
                    let created = matches!(&slot, std::collections::hash_map::Entry::Vacant(_));
                    let entry = slot.or_insert_with(|| Manifold {
                        key,
                        seen: serial.wrapping_sub(1),
                        normal: patch.normal,
                        points: [ContactPoint::default(); 4],
                        material_ids: patch.material_ids,
                        correspondence: Default::default(),
                        cache: Default::default(),
                        restitution: Default::default(),
                        rows: None,
                        state: Default::default(),
                        solvers: [None; 2],
                        callback_state: Default::default(),
                        context_flags,
                        event_kind: if patch.new_manifold { 1 } else { 2 },
                        response,
                        separation: patch.separation,
                        validation: None,
                    });
                    let notify = (bodies[0].flags | bodies[1].flags) & 0x10 != 0;
                    if created {
                        if let Some(flags) = response.initialize(notify, &mut entry.callback_state)
                        {
                            triggers.push(TriggerEvent {
                                bodies: ids,
                                shape_keys: key.shapes,
                                flags,
                            });
                        }
                    }
                    entry.response = response;
                    entry.separation = patch.separation;
                    if let Some(flags) =
                        response.collide(notify, patch.separation, &mut entry.callback_state)
                    {
                        triggers.push(TriggerEvent {
                            bodies: ids,
                            shape_keys: key.shapes,
                            flags,
                        });
                    }
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
                    // A host error may resume this pair before callbacks/solve.
                    // Preserve its already-produced created event in that batch.
                    let first_in_batch = entry.seen != serial;
                    if first_in_batch {
                        entry.event_kind = if patch.new_manifold { 1 } else { 2 };
                    }
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
                    }
                    // Source leaf keys are unique in this ordered pair walk.
                    // The existing serial also makes host retries idempotent;
                    // no per-point scan of all earlier manifolds is needed.
                    if first_in_batch {
                        active_keys.push(key);
                    }
                    let ids = entry.correspondence.point_ids();
                    entry
                        .restitution
                        .remap(&ids[..entry.correspondence.count as usize])?;
                    if entry.callback_state.modifier_flags & 0x200 == 0
                        && links.last() != Some(&key.bodies)
                    {
                        links.push(key.bodies);
                    }
                    Ok(())
                },
            )?;
            self.collision_pair += 1;
        }
        // The original retains sleeping contact caches, but retires awake
        // pairs when collision processing no longer produces a manifold.
        // Keep dormant numerical state without accumulating travelled pairs.
        if !self.collision_geometry_ready {
            self.retire_keys.clear();
            self.retire_keys
                .extend(self.manifolds.iter().filter_map(|(&key, entry)| {
                    let retain = entry.seen == self.serial
                        || key.bodies.iter().all(|&id| {
                            world
                                .bodies
                                .get(id)
                                .is_some_and(|body| body.active_list_index == u32::MAX)
                        });
                    (!retain).then_some(key)
                }));
            while let Some(key) = self.retire_keys.pop() {
                if let Some(entry) = self.manifolds.remove(&key) {
                    self.queue_retirement(entry);
                }
            }
            let triggers = &mut self.triggers;
            self.broadphase_triggers.retain(|&bodies, seen| {
                let retain = *seen == self.serial
                    || bodies.iter().all(|&id| {
                        world
                            .bodies
                            .get(id)
                            .is_some_and(|body| body.active_list_index == u32::MAX)
                    });
                if !retain {
                    triggers.push(TriggerEvent {
                        bodies,
                        shape_keys: [u32::MAX; 2],
                        flags: 0x100,
                    });
                }
                retain
            });
            self.geometry.retire_unseen(world);
            self.collision_geometry_ready = true;
        }
        // Wake after all geometry borrows end, before dense motion admission.
        while let Some(&ids) = self.body_links.get(self.activation_link) {
            while self.activation_side < 2 {
                let id = ids[self.activation_side];
                let body = world.bodies.get(id).ok_or("contact wake body retired")?;
                if body.motion_id != 0 {
                    world.activate_body(geometry, events, id)?;
                }
                self.activation_side += 1;
            }
            self.activation_side = 0;
            self.activation_link += 1;
        }
        self.collision_pending = false;
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
            let disabled_stream = entry.callback_state.modifier_flags & 0x200 != 0;
            entry.solvers = bodies.map(|body| {
                if body.motion_id == 0 {
                    return None;
                }
                let index = self.motion_to_solver[body.motion_id as usize];
                //c583a0 maps a sleeping motion's7fffffff solverId to the
                //static slot for a trigger stream; no artificial wake is needed.
                if disabled_stream && index == usize::MAX {
                    None
                } else {
                    Some(index)
                }
            });
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
            if entry.event_kind == 1 {
                entry.callback_state.cache_flags |= pair_flags;
            }
            let mut combined_flags =
                bodies[0].flags | bodies[1].flags | material[0]?.words[1] | material[1]?.words[1];
            for motion in motions {
                combined_flags |= world
                    .motion_properties
                    .get(motion.properties_id)
                    .ok_or("contact motion property absent")?
                    .words()[1];
            }
            // c8c250 stores this world flag at+96c; c51xxx includes it in
            // the Jacobian policy word. It is separate from LOD bindings80.
            if !world.info.enable_penetration_recovery {
                combined_flags |= 0x1000_0000;
            }
            if entry.event_kind == 1 {
                entry.callback_state.modifier_flags |= combined_flags;
            }
            let (recovery_limit, recovery_factor) =
                contact::build::recovery_coefficients(Some(entry.context_flags), combined_flags);
            let mut rows = contact::build(
                &ContactBuild {
                    motions,
                    normal: entry.normal,
                    points: &entry.points[..entry.correspondence.count as usize],
                    step,
                    base_friction: coefficients.base_friction(),
                    friction_delta: coefficients.friction_delta(),
                    immediate_friction: entry.callback_state.cache_flags & 0x2000 != 0,
                    recovery_limit,
                    recovery_factor,
                    // 1412abc30 initializes persistent cache+3a/+32 to
                    // 0x7f7f/0xff before any registered modifier edits it.
                    impulse_limit: Some(contact::ImpulseLimit {
                        maximum: if entry.response == Response::ContactSolver {
                            0.
                        } else {
                            f32::from_bits(0x7f7f_0000)
                        },
                        clipped_fraction: if entry.response == Response::ContactSolver {
                            0
                        } else {
                            0xff
                        },
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
            if entry.callback_state.modifier_flags & 0x200 != 0 {
                rows.stream_type = 3;
                rows.friction = None;
            }
            entry.validation = if entry.response == Response::ContactSolver && rows.stream_type != 3
            {
                Some(ValidationReference::capture(
                    entry.normal,
                    &entry.points[..entry.correspondence.count as usize],
                    motions,
                    bodies[1].motion_id != 0,
                )?)
            } else {
                None
            };
            entry.callback_state.stream_live = true;
            entry.state = ContactState::default();
            entry.rows = Some(rows);
        }
        self.prepared_callbacks_ready =
            self.active_keys.is_empty() && !self.retired_callbacks_pending();
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
            let result = rows.solve(&mut entry.state, &mut a, &mut b, phase.flags & 2 != 0);
            // c54fc9 invokes cf1fe0 once for each final phase, including the
            // optional post-pass. Preserve each validation packet in order.
            if phase.flags & 2 != 0
                && entry.response == Response::ContactSolver
                && rows.stream_type != 3
            {
                if entry.callback_state.contact_flags & 8 == 0 && result.clipped {
                    self.validation_commands.push(*key);
                } else if entry.callback_state.contact_flags & 8 != 0
                    && !result.clipped
                    && entry.callback_state.contact_flags & 0x10 == 0
                {
                    self.triggers.push(TriggerEvent {
                        bodies: entry.key.bodies,
                        shape_keys: entry.key.shapes,
                        flags: 0x102,
                    });
                    entry.callback_state.contact_flags &= !0x18;
                }
            }
            if phase.flags & 2 != 0 && entry.response != Response::Contact {
                // cf1fe0 clears the cache's Jacobian pointer even for disabled
                // type3 streams. It does not clear solved impulse arrays.
                entry.callback_state.stream_live = false;
            }
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
    fn prepared_callbacks_ready(&self) -> bool {
        self.prepared_callbacks_ready
    }
    fn finish(
        &mut self,
        world: &mut WorldStorage,
        _geometry: &mut G,
        _events: &mut E,
        step: &SolverParameters,
    ) -> Result<(), String> {
        // ce9b00 command2 runs after integration using the actual mutable
        // Jacobian and canonical motions, not an additional geometry query.
        while let Some(key) = self.validation_commands.get(self.validation_cursor) {
            if let Some(entry) = self.manifolds.get_mut(key) {
                let bodies = key.bodies.map(|id| world.bodies.get(id));
                if let [Some(a), Some(b)] = bodies {
                    if a.flags & 3 != 0
                        && b.flags & 3 != 0
                        && a.broadphase_id != u32::MAX
                        && b.broadphase_id != u32::MAX
                    {
                        let motions = [
                            world
                                .motions
                                .get(a.motion_id)
                                .ok_or("trigger validation motion A retired")?,
                            world
                                .motions
                                .get(b.motion_id)
                                .ok_or("trigger validation motion B retired")?,
                        ];
                        let reference = entry
                            .validation
                            .as_ref()
                            .ok_or("trigger validation reference absent")?;
                        let rows = entry
                            .rows
                            .as_ref()
                            .ok_or("trigger validation rows absent")?;
                        if validation::crossing_fraction(reference, rows, motions, step, [0.; 4])?
                            < 1.
                        {
                            self.triggers.push(TriggerEvent {
                                bodies: key.bodies,
                                shape_keys: key.shapes,
                                flags: 2,
                            });
                            entry.callback_state.contact_flags |= 8;
                        }
                    }
                }
            }
            self.validation_cursor += 1;
        }
        self.validation_commands.clear();
        self.validation_cursor = 0;
        for key in &self.active_keys {
            let entry = self
                .manifolds
                .get_mut(key)
                .ok_or("solved contact manifold absent")?;
            entry.cache.export(&entry.state);
            if entry.response != Response::Contact {
                entry.rows = None;
            }
        }
        Ok(())
    }
}
