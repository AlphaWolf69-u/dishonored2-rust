//! The rigid-body component's single WorldStorage identity.
//!
//! The entity/declaration factory supplies the authored creation record and
//! resolved material table. This owner never reconstructs a body from the
//! flattened navigation triangles.
use crate::{
    assets::{entities::triggers::TriggerAssets, physworld::{body::BodyInfo, source::StandaloneShape}},
    gameplay::{
        body_storage::BodyActivation,
        collision_user_data::CollisionUserData,
        component_state::{entity::ComponentHandle, ComponentInstance, ComponentOwner},
        physics_world::{body_creation::BodyCreationCallbacks, storage::WorldStorage, PublicationEvents},
    },
};

/// Preparation output for the component's original 140808ea0 resource
/// acquisition. Call on the existing level preparation worker, never on the
/// gameplay thread. Model materials must come from the live render model's
/// resolved material declarations, in surface-slot order.
/// Worker-to-entity handoff. The actual entity factory consumes this once
/// into its component, after checking the reflected slot and render source.
pub(super) struct PreparedRigidRender {
    pub slot: String,
    pub source_index: usize,
    pub settings: crate::entities::RigidBodySettings,
    pub resources: Result<RigidBodyResources, String>,
}

pub(super) struct RigidBodyResources {
    pub shape: StandaloneShape,
    pub extra: Option<(StandaloneShape, [u16; 2])>,
    pub materials: Vec<u16>,
    pub body_material: u16,
    pub filter_info: u32,
}

pub(super) fn prepare_resources(
    catalog: &crate::assets::archive::Catalog,
    shape_sources: &mut crate::physworld::source::ShapeSources,
    texts: &mut crate::assets::declaration_texts::PhysicsSeed,
    material_owner: &mut crate::gameplay::materials::MaterialOwner<usize, usize>,
    settings: &crate::entities::RigidBodySettings,
    model: &crate::entities::PlacedModel,
    render_model: Option<&crate::assets::bmodel::Model>,
    render_materials: Option<&mut super::material_loading::MaterialCache>,
    mut dispatch: impl FnMut(&mut crate::gameplay::signal::Signal<usize, usize>, usize, usize, u16),
) -> Result<RigidBodyResources, String> {
    let kind = settings.shape_type.as_deref().and_then(crate::entities::rigid_body_shape_kind)
        .and_then(|kind| ["hkt", "cvx", "msh", "box", "cvd"].get(kind as usize).copied())
        .ok_or("rigid-body shape type is invalid")?;
    let path = super::view_geometry::model_havok_path(model, kind, false)
        .ok_or("rigid-body render model has no supported shape resource path")?;
    let shape = shape_sources.acquire_standalone(catalog, &path, crate::physworld::source::ShapeUse::Physics)?
        .ok_or_else(|| format!("rigid-body shape resource is missing: {path}"))?;
    // The outer resource's signed +0x38 is the material-slot count. Native
    // allocates count+1 only when positive, otherwise it uses one scalar.
    let header = shape.wrappers.first().map_or(shape.source_header, |alias| alias.header);
    let raw_count = i32::from_le_bytes(header.to_le_bytes());
    let count = if raw_count > 0 { usize::try_from(raw_count).map_err(|_| "invalid material count")?
        .checked_add(1).ok_or("rigid-body material count overflow")? } else { 1 };
    let mut materials = Vec::new();
    materials.try_reserve_exact(count).map_err(|error| format!("rigid-body material table: {error}"))?;
    if let Some(override_name) = settings.material.as_deref() {
        let declaration = material_owner.declaration_with_texts(catalog, override_name, texts)?;
        let id = material_owner.resolve(declaration, &mut dispatch)?;
        materials.resize(count, id);
    } else {
        // 140807c80 visits exactly the shape-derived slot count, not every
        // render surface. Unused material slots must not block this shape.
        let render_model = render_model.ok_or("rigid-body render material source is unsupported")?;
        let render_materials = render_materials.ok_or("rigid-body render material cache is absent")?;
        if render_model.surfaces.len() < count { return Err("rigid-body render material slots are incomplete".into()); }
        for (index, surface) in render_model.surfaces[..count].iter().enumerate() {
            let name = model.custom_materials.get(index).and_then(Option::as_deref)
                .unwrap_or(&surface.material_path);
            let id = if let Some(name) = render_materials.body_physics_material(catalog, name)? {
                let declaration = material_owner.declaration_with_texts(catalog, name.as_ref(), texts)?;
                material_owner.resolve(declaration, &mut dispatch)?
            } else { 0 };
            materials.push(id);
        }
    }
    let mut extra = None;
    if let Some(name) = settings.extra_hd_material.as_deref().filter(|name| !name.is_empty() && *name != "NULL") {
        let path = super::view_geometry::model_havok_path(model, "hkt", true)
            .ok_or("rigid-body render model has no supported extra-HD path")?;
        if let Some(child) = shape_sources.acquire_standalone(catalog, &path, crate::physworld::source::ShapeUse::Physics)? {
            let header = child.wrappers.first().map_or(child.source_header, |alias| alias.header);
            let raw_extra_count = i32::from_le_bytes(header.to_le_bytes());
            let extra_count = if raw_extra_count > 0 {
                usize::try_from(raw_extra_count).map_err(|_| "invalid extra-HD material count")?
                    .checked_add(1).ok_or("extra-HD material count overflow")?
            } else { 1 };
            let base = u16::try_from(materials.len()).map_err(|_| "extra-HD material offset exceeds shape tag")?;
            if base > 0x7fff { return Err("extra-HD material offset exceeds shape tag".into()); }
            let declaration = material_owner.declaration_with_texts(catalog, name, texts)?;
            let id = material_owner.resolve(declaration, &mut dispatch)?;
            let end = materials.len().checked_add(extra_count).ok_or("extra-HD material table overflow")?;
            materials.try_reserve(extra_count).map_err(|error| format!("extra-HD material table: {error}"))?;
            materials.resize(end, id);
            let tags = [if raw_count > 0 { 0x8000 } else { 0 },
                base | if raw_extra_count > 0 { 0x8000 } else { 0 }];
            extra = Some((child, tags));
        }
    }
    let body_material = if extra.is_none() && count == 1 { materials[0] } else { 0 };
    let filter_info = settings.collision_filter().ok_or("rigid-body collision filter is invalid")?;
    Ok(RigidBodyResources { shape, extra, materials, body_material, filter_info })
}

/// Body values resolved by the real material/mass and motion-property
/// owners. They cannot be inferred from whether a render placement moves.
pub(super) struct RigidBodyCreationValues {
    pub mass: f32,
    pub mass_distribution: usize,
    pub motion_properties_id: u16,
    /// Reflected declaration byte +0xc4 from 140806eb0.
    pub declaration_flags: u8,
}

/// 140808ea0 starts from the native 140ca65f0 body-info constructor and
/// applies its component values. Shape and user data are bound by `create`.
pub(super) fn body_info(model: &crate::entities::PlacedModel,
    resources: &RigidBodyResources, values: RigidBodyCreationValues) -> BodyInfo {
    let basis = crate::physworld::rigid_body_basis(model.rotation.map(|v| [v[0], v[1], v[2], 0.0]));
    let flags = 0x80 | if values.declaration_flags & 2 != 0 { 0x40000 } else { 0 }
        | if values.declaration_flags & 4 != 0 { 0x20000 } else { 0 };
    BodyInfo { flags, filter_info: resources.filter_info, material_id: resources.body_material,
        motion_type: 2,
        position: [model.position[0], model.position[1], model.position[2], 0.0],
        orientation: crate::physworld::rotation_quaternion(basis), mass: values.mass,
        mass_distribution: values.mass_distribution, motion_properties_id: values.motion_properties_id,
        ..BodyInfo::default() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Initialization { Empty, GeometryReady, Creating, BodyCreated, BaseReady, Ready, AbortListeners, AbortBody, AbortShape, Retired }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartPhase { Transform, Publish, BodyPublished, Complete, RollbackUnpublish, RollbackRemove, RollbackCallbacks }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopPhase { BeforeUnpublish, Unpublish, AfterUnpublish, Complete }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeinitPhase { Unregister, Destroy, Base, RetireShape, Complete }

struct StartProgress {
    frame: crate::gameplay::shape_transform::ShapeMatrix,
    activation: BodyActivation,
    phase: StartPhase,
}

pub(super) use crate::entities::rigid_body::Definition as RigidBodyDefinition;

pub(super) struct RigidBodyComponent {
    pub owner: ComponentOwner,
    /// Embedded +0xe8 movement interface in the shared actor-body table.
    movement_body: Option<usize>,
    pub velocities: crate::gameplay::collision_user_data::velocity::CollisionVelocities,
    definition: Option<std::sync::Arc<RigidBodyDefinition>>,
    /// The actual render source and its acquired shape/material lease, held by
    /// this component until Initialize consumes it or admission rolls back.
    prepared: Option<(usize, RigidBodyResources)>,
    shape: Option<usize>,
    body: Option<u32>,
    component: Option<ComponentHandle>,
    initialize_listeners: Option<(usize, crate::gameplay::entity_lifetime::EntityId, u32)>,
    start_listeners: Option<(usize, crate::gameplay::entity_lifetime::EntityId, u32, bool)>,
    activation_listener: Option<(u32, u32)>,
    /// arkCpntPhysicsBase +0x98..+0xb2, owned by the 0x149 listener.
    requested_motion: [f32; 6],
    requested_motion_flags: u8,
    requested_motion_additive: bool,
    requested_motion_pending: bool,
    /// arkCpntPhysicsBase +0xd2, written by rigid freeze/thaw callbacks.
    pub(super) freeze_state: u8,
    /// Bit 0 of +0x1ac tracks the component's own update-roster pause.
    update_paused_by_body: bool,
    materials: Vec<u16>,
    pending_info: Option<BodyInfo>,
    creation_mass_resolved: bool,
    initialization: Initialization,
    start: Option<StartProgress>,
    stop: Option<StopPhase>,
    deinit: Option<DeinitPhase>,
}

impl Default for RigidBodyComponent {
    fn default() -> Self {
        Self { owner: ComponentOwner::default(), movement_body: None, velocities: Default::default(), definition: None, prepared: None, shape: None, body: None, component: None, initialize_listeners: None, start_listeners: None, activation_listener: None,
            requested_motion: [0.0; 6], requested_motion_flags: 0, requested_motion_additive: false, requested_motion_pending: false, freeze_state: 0, update_paused_by_body: false,
            materials: Vec::new(), pending_info: None, creation_mass_resolved: false, initialization: Initialization::Empty, start: None, stop: None, deinit: None }
    }
}

impl ComponentInstance for RigidBodyComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}

impl RigidBodyComponent {
    pub(super) fn new(definition: std::sync::Arc<RigidBodyDefinition>,
        bodies: &mut Vec<crate::gameplay::physics_actor::ActorBody>, free: &mut Vec<usize>) -> Result<Self, String> {
        let movement_body = if let Some(&index) = free.last() {
            let body = bodies.get(index).ok_or("free movement interface is absent")?;
            if body.actor.is_some() || body.body_id & 0x00ff_ffff != 0x00ff_ffff {
                return Err("free movement interface is still live".into());
            }
            free.pop();
            index
        } else {
            bodies.try_reserve(1).map_err(|error| format!("rigid movement interface allocation: {error}"))?;
            let index = bodies.len();
            bodies.push(crate::gameplay::physics_actor::ActorBody { actor: None, body_id: 0x00ff_ffff });
            index
        };
        Ok(Self { definition: Some(definition), movement_body: Some(movement_body), ..Self::default() })
    }
    pub(super) fn movement_body(&self) -> Result<usize, String> {
        self.movement_body.ok_or_else(|| "rigid movement interface is absent".into())
    }
    pub(super) fn definition(&self) -> Option<&RigidBodyDefinition> { self.definition.as_deref() }
    pub(super) fn body(&self) -> Option<u32> { self.body }
    pub(super) fn material_table(&self) -> &[u16] { &self.materials }
    pub(super) fn is_initialized(&self) -> bool { self.initialization == Initialization::Ready }
    pub(super) fn creation_pending(&self) -> bool { matches!(self.initialization, Initialization::GeometryReady | Initialization::Creating) }
    pub(super) fn base_initialize_pending(&self) -> bool { self.initialization == Initialization::BodyCreated }
    pub(super) fn initialize_listeners_pending(&self) -> bool { self.initialization == Initialization::BaseReady && self.initialize_listeners.is_none() }

    pub(super) fn can_release(&self) -> bool {
        self.prepared.is_none() && self.shape.is_none() && self.body.is_none() && self.initialize_listeners.is_none() && self.start_listeners.is_none() && self.activation_listener.is_none()
            && matches!(self.initialization, Initialization::Empty | Initialization::Retired)
    }

    pub(super) fn has_prepared_render(&self) -> bool { self.prepared.is_some() }

    pub(super) fn bind_prepared_render(&mut self, source_index: usize, resources: RigidBodyResources) -> Result<(), String> {
        if self.initialization != Initialization::Empty || self.prepared.is_some() {
            return Err("rigid-body render resource is already bound or initialized".into());
        }
        self.prepared = Some((source_index, resources));
        Ok(())
    }

    /// The caller retains the returned source and resources through the real
    /// Initialize boundary, or retires them when admission is rolled back.
    pub(super) fn prepared_material(&self) -> Option<u16> { self.prepared.as_ref()?.1.materials.first().copied() }

    pub(super) fn take_prepared_render(&mut self) -> Option<(usize, RigidBodyResources)> {
        self.prepared.take()
    }

    pub(super) fn retire_uninitialized_render(&mut self) -> Result<(), String> {
        if self.prepared.is_some() && self.initialization != Initialization::Empty {
            return Err("live rigid-body render lease requires native deinitialization".into());
        }
        self.prepared = None;
        Ok(())
    }

    /// 140727100 consumes six little-endian floats, a flags byte and an
    /// additive byte from the actual 0x149 entity-message payload.
    pub(super) fn apply_motion_request(&mut self, payload: &[u8]) -> Result<(), String> {
        if self.start.as_ref().is_none_or(|start| start.phase != StartPhase::Complete) {
            return Err("rigid-body motion request requires a started component".into());
        }
        if payload.len() < 26 { return Err("rigid-body motion request payload is truncated".into()); }
        let values: [f32; 6] = std::array::from_fn(|i| {
            f32::from_le_bytes(payload[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let additive = payload[25] != 0;
        self.requested_motion_pending = true;
        if additive {
            for i in 0..3 { self.requested_motion[i] = values[i] + self.requested_motion[i]; }
            for i in 3..6 { self.requested_motion[i] = self.requested_motion[i] + values[i]; }
            self.requested_motion_flags |= payload[24];
        } else {
            self.requested_motion = values;
            self.requested_motion_flags = payload[24];
        }
        self.requested_motion_additive = additive;
        Ok(())
    }

    /// 14080b680 forms the requested velocity from the live collision-data
    /// payload. The caller publishes it through 18d020's queued path and,
    /// when requested, runs the actual 140853220 component notification.
    pub(super) fn pending_motion_request(&self) -> Option<([f32; 4], [f32; 4], bool)> {
        if !self.requested_motion_pending { return None; }
        let mut linear = [self.requested_motion[0], self.requested_motion[1], self.requested_motion[2], 0.0];
        let mut angular = [self.requested_motion[3], self.requested_motion[4], self.requested_motion[5], 0.0];
        if self.requested_motion_additive {
            for i in 0..4 {
                linear[i] += self.velocities.requested_linear[i];
                angular[i] += self.velocities.requested_angular[i];
            }
        }
        const ZERO_THRESHOLD: f32 = f32::from_bits(0x3089_705f);
        if (linear[1] * linear[1] + linear[0] * linear[0]) + linear[2] * linear[2] < ZERO_THRESHOLD {
            linear = [0.0; 4];
        }
        if (angular[1] * angular[1] + angular[0] * angular[0]) + angular[2] * angular[2] < ZERO_THRESHOLD {
            angular = [0.0; 4];
        }
        Some((linear, angular, self.requested_motion_flags != 0))
    }

    /// 140726f50 runs only after velocity publication and notification.
    pub(super) fn clear_motion_request(&mut self) {
        self.requested_motion = [0.0; 6];
        self.requested_motion_flags = 0;
        self.requested_motion_additive = false;
        self.requested_motion_pending = false;
    }

    /// 140808ea0 registers the actual entity-scoped timespace callbacks in
    /// this order; the host supplies the registry-thread/deferred decision.
    pub(super) fn register_initialize_listeners(&mut self,
        events: &mut crate::gameplay::entity_events::EntityEvents, owner: usize,
        entity: crate::gameplay::entity_lifetime::EntityId, deferred: bool) -> Result<(), String> {
        if self.initialization != Initialization::BaseReady || self.initialize_listeners.is_some() {
            return Err("rigid-body initialize listeners are not ready".into());
        }
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        let generation = self.owner.generation;
        for event in [0xad, 0xac] {
            let listener = crate::gameplay::entity_events::Listener::Component {
                owner, handle, generation, handler: event,
            };
            if deferred { events.defer_register(event, Some(entity), listener); }
            else { events.register(event, Some(entity), listener); }
        }
        self.initialize_listeners = Some((owner, entity, generation));
        Ok(())
    }

    /// 140809bc0 removes the same two bindings before destroying the body.
    pub(super) fn unregister_initialize_listeners(&mut self,
        events: &mut crate::gameplay::entity_events::EntityEvents, deferred: bool) -> Result<(), String> {
        let Some((owner, entity, generation)) = self.initialize_listeners else { return Ok(()) };
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        if self.owner.generation != generation { return Err("rigid-body listener generation is stale".into()); }
        for event in [0xad, 0xac] {
            let listener = crate::gameplay::entity_events::Listener::Component {
                owner, handle, generation, handler: event,
            };
            if deferred { events.defer_unregister(event, Some(entity), listener); }
            else { events.unregister(event, Some(entity), listener); }
        }
        self.initialize_listeners = None;
        Ok(())
    }

    /// 140809d80 installs the body's activation callback before wind and
    /// entity listeners. The wrapper owns its per-body signal.
    pub(super) fn register_activation_listener(&mut self,
        events: &mut crate::gameplay::physics_world::body_activation_events::BodyActivationEvents) -> Result<(), String> {
        if self.activation_listener.is_some() || self.start.as_ref().is_none_or(|start| start.phase != StartPhase::BodyPublished) {
            return Err("rigid-body activation listener is not ready".into());
        }
        let body = self.body.ok_or("rigid-body activation body is absent")?;
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        let generation = self.owner.generation;
        events.subscribe(body, crate::gameplay::physics_world::body_activation_events::ActivationSubscriber { handle, generation })?;
        self.activation_listener = Some((body, generation));
        Ok(())
    }

    /// 14080a260 removes the body activation callback before unpublication.
    pub(super) fn unregister_activation_listener(&mut self,
        events: &mut crate::gameplay::physics_world::body_activation_events::BodyActivationEvents) -> Result<(), String> {
        let Some((body, generation)) = self.activation_listener else { return Ok(()) };
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        if self.owner.generation != generation { return Err("rigid-body activation listener generation is stale".into()); }
        events.unsubscribe(body, crate::gameplay::physics_world::body_activation_events::ActivationSubscriber { handle, generation })?;
        self.activation_listener = None;
        Ok(())
    }

    /// 1408097f0 with rigid vtable +0x1b0 (140808440). All callbacks borrow
    /// the actual component registry and entity/hierarchy owners.
    pub(super) fn on_body_activation(&mut self, active_event: bool, body_flags: u32,
        entity_flags_d4: &mut u8,
        registry: &mut crate::gameplay::component_state::ComponentRegistry,
        schedule_entity_update: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        let dynamic_active = body_flags & 1 == 0 && body_flags & 8 != 0;
        if dynamic_active {
            if *entity_flags_d4 & 2 != 0 { *entity_flags_d4 |= 4; }
            if self.update_paused_by_body {
                registry.resume_update_flags(handle, &mut self.owner.flags)?;
                self.update_paused_by_body = false;
            }
        } else if !self.update_paused_by_body {
            registry.pause_update_flags(handle, &mut self.owner.flags)?;
            self.update_paused_by_body = true;
        }
        if active_event && self.owner.flags & 0x30 == 0x30 {
            schedule_entity_update()?;
        }
        Ok(())
    }

    /// 14080a260 clears +0x1ac bit 0 after detaching the entity interface,
    /// before it invokes the base Stop virtual.
    pub(super) fn clear_body_update_pause(&mut self) { self.update_paused_by_body = false; }

    /// 140809d80 installs the optional wind listener first, followed by the
    /// three unconditional entity listeners. The callback uses the real wind
    /// owner; it is invoked only when the declaration's factor is positive.
    pub(super) fn register_start_listeners(&mut self,
        events: &mut crate::gameplay::entity_events::EntityEvents, owner: usize,
        entity: crate::gameplay::entity_lifetime::EntityId, deferred: bool,
        register_wind: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        if self.start_listeners.is_some() || self.activation_listener.is_none() || self.start.as_ref().is_none_or(|start| start.phase != StartPhase::BodyPublished) {
            return Err("rigid-body start listeners are not ready".into());
        }
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        let generation = self.owner.generation;
        let wind = self.definition.as_ref().ok_or("rigid-body declaration is absent")?.settings.wind_speed_factor
            .ok_or("rigid-body wind factor is absent")? > 0.0;
        if wind { register_wind()?; }
        for event in [0xd4, 0xfa, 0x149, 0x148] {
            if event == 0xd4 && !wind { continue; }
            let listener = crate::gameplay::entity_events::Listener::Component { owner, handle, generation, handler: event };
            if deferred { events.defer_register(event, Some(entity), listener); }
            else { events.register(event, Some(entity), listener); }
        }
        self.start_listeners = Some((owner, entity, generation, wind));
        Ok(())
    }

    /// 14080a260 removes the three unconditional listeners first, then the
    /// optional wind listener and its actual wind-owner registration.
    pub(super) fn unregister_start_listeners(&mut self,
        events: &mut crate::gameplay::entity_events::EntityEvents, deferred: bool,
        unregister_wind: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let Some((owner, entity, generation, wind)) = self.start_listeners else { return Ok(()) };
        let handle = self.component.ok_or("rigid-body component identity is absent")?;
        if self.owner.generation != generation { return Err("rigid-body start listener generation is stale".into()); }
        for event in [0x149, 0xfa, 0x148, 0xd4] {
            if event == 0xd4 && !wind { continue; }
            let listener = crate::gameplay::entity_events::Listener::Component { owner, handle, generation, handler: event };
            if deferred { events.defer_unregister(event, Some(entity), listener); }
            else { events.unregister(event, Some(entity), listener); }
        }
        if wind { unregister_wind()?; }
        self.start_listeners = None;
        Ok(())
    }

    /// 140808ea0 creates the body disabled, with the actual component-owned
    /// collision userData. `info` and `materials` must already reflect the
    /// real declaration, material library, entity pose and shape recipe.
    /// Retain the acquired geometry and exact creation request before any
    /// fallible body operation; retries never reacquire either resource.
    pub(super) fn begin_create(
        &mut self, handle: ComponentHandle, lease: StandaloneShape,
        extra: Option<(StandaloneShape, [u16; 2])>,
        mut info: BodyInfo, materials: Vec<u16>, geometry: &mut TriggerAssets,
    ) -> Result<(), String> {
        if self.initialization != Initialization::Empty || self.owner.entity.is_none()
            || self.owner.declaration.is_none() {
            return Err("rigid-body component is not ready for initialization".into());
        }
        let shape = geometry.register_standalone_shape(lease, extra)?;
        self.shape = Some(shape);
        self.component = Some(handle);
        self.materials = materials;
        info.shape = shape;
        info.user_data = CollisionUserData { component: handle }.encode();
        self.pending_info = Some(info);
        self.creation_mass_resolved = false;
        self.initialization = Initialization::GeometryReady;
        Ok(())
    }

    /// 808ea0 -> cb95d0: density applies to the retained shape's actual
    /// volume, never to a render bounding box or an assumed unit mass.
    pub(super) fn resolve_creation_mass(&mut self, geometry: &TriggerAssets, density: f32) -> Result<(), String> {
        use crate::gameplay::physics_world::body_creation::BodyCreationGeometry;
        if self.initialization != Initialization::GeometryReady { return Err("rigid-body geometry is not ready for mass resolution".into()); }
        let info = self.pending_info.as_mut().ok_or("rigid-body creation info is absent")?;
        let settings = &self.definition.as_ref().ok_or("rigid-body definition is absent")?.settings;
        info.mass = if let Some(mass) = settings.descriptor_mass_override { mass } else {
            let properties = match geometry.cached_mass(info.shape)? {
                Some(properties) => properties,
                None => geometry.calculate_mass(info.shape, [0, 0x3fc0_0000, 0xc47a_0000])?,
            };
            properties.volume * density
        };
        self.creation_mass_resolved = true;
        Ok(())
    }

    /// Split the world callback borrow from this same component's cursor.
    pub(super) fn creation_request(&self) -> Result<Option<&BodyInfo>, String> {
        if !self.creation_mass_resolved { return Err("rigid-body actual shape mass resolution is incomplete".into()); }
        match self.initialization {
            Initialization::GeometryReady => Ok(Some(self.pending_info.as_ref().ok_or("rigid-body creation info is absent")?)),
            Initialization::Creating => Ok(None),
            _ => Err("rigid-body creation is not pending".into()),
        }
    }
    pub(super) fn creation_entered(&mut self) { self.initialization = Initialization::Creating; }
    pub(super) fn creation_completed(&mut self, body: u32) -> Result<(), String> {
        if self.initialization != Initialization::Creating { return Err("rigid-body creation is not entered".into()); }
        self.body = Some(body);
        self.pending_info = None;
        self.initialization = Initialization::BodyCreated;
        Ok(())
    }

    /// Resume the same generation and retained WorldStorage cursor after a
    /// callback failure; no duplicate body, motion, or shape is created.
    pub(super) fn resume_create<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, handle: ComponentHandle, geometry: &mut TriggerAssets,
        world: &mut WorldStorage, events: &mut E,
    ) -> Result<(), String> {
        if self.component != Some(handle) {
            return Err("rigid-body creation component changed during retry".into());
        }
        if self.initialization == Initialization::GeometryReady {
            let info = self.pending_info.as_ref().ok_or("rigid-body creation request absent")?;
            world.begin_disabled_component_body(handle, self.owner.generation, info)?;
            self.initialization = Initialization::Creating;
        }
        if self.initialization != Initialization::Creating { return Err("rigid-body body creation is not pending".into()); }
        let body = world.resume_disabled_component_body(geometry, events, handle, self.owner.generation)?;
        self.body = Some(body);
        self.pending_info = None;
        self.initialization = Initialization::BodyCreated;
        Ok(())
    }

    /// Roll back only an in-flight Initialize. WorldStorage first unwinds
    /// the exact generation's partial body/motion work; the component keeps
    /// its geometry lease until that operation has actually completed.
    pub(super) fn abort_create<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, handle: ComponentHandle, geometry: &mut TriggerAssets,
        world: &mut WorldStorage, events: &mut E,
        undo_listeners: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.component != Some(handle) {
            return Err("rigid-body rollback component identity changed".into());
        }
        if self.initialization == Initialization::Creating {
            world.abort_disabled_component_body(geometry, events, handle, self.owner.generation)?;
            self.initialization = Initialization::GeometryReady;
        }
        if matches!(self.initialization, Initialization::BodyCreated | Initialization::BaseReady) {
            self.initialization = Initialization::AbortListeners;
        }
        if self.initialization == Initialization::AbortListeners {
            undo_listeners()?;
            if self.initialize_listeners.is_some() {
                return Err("rigid-body initialize listeners remain registered".into());
            }
            self.initialization = Initialization::AbortBody;
        }
        if self.initialization == Initialization::AbortBody {
            let body = self.body.ok_or("rigid-body identity is absent")?;
            world.destroy_bodies(geometry, events, &[body], 0)?;
            self.body = None;
            self.initialization = Initialization::AbortShape;
        }
        if self.initialization == Initialization::GeometryReady {
            self.initialization = Initialization::AbortShape;
        }
        if self.initialization != Initialization::AbortShape {
            return Err("rigid-body creation is not abortable".into());
        }
        let shape = self.shape.ok_or("rigid-body shape lease is absent")?;
        geometry.retire_standalone_shape(shape)?;
        self.shape = None;
        self.pending_info = None;
        self.component = None;
        self.materials.clear();
        self.initialization = Initialization::Retired;
        Ok(())
    }

    /// Host cancellation of an unpublished scene splits the same rollback
    /// cursor around callbacks borrowing this component's actual pool.
    pub(super) fn unpublished_start_request(&mut self)
        -> Option<(crate::gameplay::shape_transform::ShapeMatrix, BodyActivation, u8)> {
        let progress = self.start.as_mut()?;
        let step = match progress.phase {
            StartPhase::Transform => 0, StartPhase::Publish => 1,
            StartPhase::BodyPublished | StartPhase::Complete | StartPhase::RollbackUnpublish => 2,
            StartPhase::RollbackRemove => 3,
            StartPhase::RollbackCallbacks => 4,
        };
        Some((progress.frame, progress.activation, step))
    }
    pub(super) fn advance_unpublished_start(&mut self, step: u8) -> Result<(), String> {
        let progress = self.start.as_mut().ok_or("unpublished rigid Start cursor absent")?;
        progress.phase = match step {
            0 => StartPhase::RollbackCallbacks,
            1 => StartPhase::RollbackUnpublish,
            2 => StartPhase::RollbackRemove,
            3 => StartPhase::RollbackCallbacks,
            4 => { self.start = None; self.stop = Some(StopPhase::Complete); return Ok(()); },
            _ => return Err("invalid unpublished rigid Start step".into()),
        };
        Ok(())
    }
    pub(super) fn unpublished_abort_step(&mut self) -> Result<u8, String> {
        if self.start.is_some() { return Err("rigid Start must unwind before creation abort".into()); }
        Ok(match self.initialization {
            Initialization::Empty => 0,
            Initialization::Creating => 1,
            Initialization::BodyCreated | Initialization::BaseReady | Initialization::Ready | Initialization::AbortListeners => 2,
            Initialization::AbortBody => 3,
            Initialization::GeometryReady | Initialization::AbortShape => 4,
            Initialization::Retired => 5,
        })
    }
    pub(super) fn advance_unpublished_abort(&mut self, step: u8) -> Result<(), String> {
        self.initialization = match step {
            0 => { self.prepared = None; Initialization::Retired },
            1 => Initialization::GeometryReady,
            2 => {
                if self.initialize_listeners.is_some() { return Err("rigid Initialize listeners remain registered".into()); }
                Initialization::AbortBody
            },
            3 => { self.body = None; Initialization::AbortShape },
            4 => {
                self.shape = None; self.pending_info = None; self.component = None; self.materials.clear();
                Initialization::Retired
            },
            _ => return Err("invalid unpublished rigid creation step".into()),
        };
        Ok(())
    }
    pub(super) fn retained_shape(&self) -> Option<usize> { self.shape }

    /// Run 140726f10 after creating the body and before registering listeners.
    pub(super) fn initialize_base(
        &mut self, base: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.initialization != Initialization::BodyCreated {
            return Err("rigid-body creation is not awaiting base initialization".into());
        }
        base()?;
        self.initialization = Initialization::BaseReady;
        Ok(())
    }

    pub(super) fn finish_initialize(&mut self) -> Result<(), String> {
        if self.initialization != Initialization::BaseReady || self.initialize_listeners.is_none() {
            return Err("rigid-body initialize listeners are incomplete".into());
        }
        self.owner.mark_initialized();
        self.initialization = Initialization::Ready;
        Ok(())
    }

    /// The motion-backed body part of 140809d80. The caller must first perform the
    /// real base-Start and entity attachment callbacks. Preserve the exact
    /// frame and publication stage across any later listener failure.
    /// The same Start cursor, split around the scene observer's component
    /// borrow so activation callbacks see this actual component in its pool.
    pub(super) fn start_publication_step(&mut self, frame: crate::gameplay::shape_transform::ShapeMatrix,
        activation: BodyActivation) -> Result<u8, String> {
        if self.initialization != Initialization::Ready { return Err("rigid-body initialization is incomplete".into()); }
        if self.stop == Some(StopPhase::Complete) { self.stop = None; }
        if self.stop.is_some() { return Err("rigid-body stop is incomplete".into()); }
        if let Some(progress) = &self.start {
            if progress.activation != activation || !progress.frame.iter().flatten().zip(frame.iter().flatten()).all(|(a,b)| a.to_bits() == b.to_bits()) {
                return Err("rigid-body start inputs changed during publication".into());
            }
        } else {
            self.clear_motion_request();
            self.update_paused_by_body = self.owner.flags & 0x30 > 0x1f;
            self.start = Some(StartProgress { frame, activation, phase: StartPhase::Transform });
        }
        match self.start.as_ref().ok_or("rigid-body Start cursor is absent")?.phase {
            StartPhase::Transform => Ok(0), StartPhase::Publish => Ok(1),
            StartPhase::BodyPublished => Ok(2), StartPhase::Complete => Ok(3),
            _ => Err("rigid-body start rollback is in progress".into()),
        }
    }
    pub(super) fn advance_start_publication(&mut self) -> Result<(), String> {
        let progress = self.start.as_mut().ok_or("rigid-body Start cursor is absent")?;
        progress.phase = match progress.phase {
            StartPhase::Transform => StartPhase::Publish,
            StartPhase::Publish => StartPhase::BodyPublished,
            _ => return Err("rigid-body Start publication cannot advance".into()),
        };
        Ok(())
    }
    pub(super) fn activation_listener_registered(&self) -> bool { self.activation_listener.is_some() }
    pub(super) fn start_listeners_registered(&self) -> bool { self.start_listeners.is_some() }

    pub(super) fn publish_start_body<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, frame: crate::gameplay::shape_transform::ShapeMatrix,
        activation: BodyActivation, geometry: &mut TriggerAssets,
        world: &mut WorldStorage, events: &mut E,
    ) -> Result<(), String> {
        if self.initialization != Initialization::Ready { return Err("rigid-body initialization is incomplete".into()); }
        if self.stop == Some(StopPhase::Complete) { self.stop = None; }
        if self.stop.is_some() { return Err("rigid-body stop is incomplete".into()); }
        let body = self.body.ok_or("rigid-body identity is absent")?;
        if let Some(progress) = &self.start {
            if progress.activation != activation || !progress.frame.iter().flatten().zip(frame.iter().flatten())
                .all(|(a, b)| a.to_bits() == b.to_bits()) {
                return Err("rigid-body start inputs changed during publication".into());
            }
        } else {
            self.start = Some(StartProgress { frame, activation, phase: StartPhase::Transform });
        }
        let progress = self.start.as_mut().expect("start progress exists");
        if matches!(progress.phase, StartPhase::RollbackUnpublish | StartPhase::RollbackRemove | StartPhase::RollbackCallbacks) {
            return Err("rigid-body start rollback is in progress".into());
        }
        if progress.phase == StartPhase::Transform {
            world.set_body_transform(geometry, events, body, progress.frame, 0)?;
            self.update_paused_by_body = self.owner.flags & 0x30 > 0x1f;
            progress.phase = StartPhase::Publish;
        }
        if progress.phase == StartPhase::Publish {
            world.add_dynamic_body_resumable(geometry, events, body, progress.activation)?;
            progress.phase = StartPhase::BodyPublished;
        }
        Ok(())
    }

    /// Finish an entered publication before undoing it. The caller owns the
    /// source-ordered attachment and listener undo and its own retry cursor.
    pub(super) fn rollback_start<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut TriggerAssets, world: &mut WorldStorage, events: &mut E,
        undo_callbacks: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let body = self.body.ok_or("rigid-body identity is absent")?;
        let progress = self.start.as_mut().ok_or("rigid-body start has not begun")?;
        if progress.phase == StartPhase::Transform {
            world.set_body_transform(geometry, events, body, progress.frame, 0)?;
            progress.phase = StartPhase::RollbackCallbacks;
        }
        if progress.phase == StartPhase::Publish {
            world.add_dynamic_body_resumable(geometry, events, body, progress.activation)?;
            progress.phase = StartPhase::RollbackUnpublish;
        }
        if matches!(progress.phase, StartPhase::BodyPublished | StartPhase::Complete) {
            progress.phase = StartPhase::RollbackUnpublish;
        }
        if matches!(progress.phase, StartPhase::RollbackUnpublish | StartPhase::RollbackRemove) {
            world.unpublish_bodies(geometry, events, &[body], 0)?;
            progress.phase = StartPhase::RollbackCallbacks;
        }
        if progress.phase == StartPhase::RollbackCallbacks {
            undo_callbacks()?;
            self.start = None;
        }
        Ok(())
    }

    /// Finish only after the real collision settings, body-activation, wind,
    /// and entity-message listeners have been installed in source order.
    /// Their host must preserve its own progress when a callback fails.
    pub(super) fn finish_start(&mut self, finish_callbacks: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        let progress = self.start.as_mut().ok_or("rigid-body start has not begun")?;
        match progress.phase {
            StartPhase::Complete => return Ok(()),
            StartPhase::BodyPublished => {},
            _ => return Err("rigid-body publication is incomplete".into()),
        }
        finish_callbacks()?;
        if self.start_listeners.is_none() { return Err("rigid-body start listeners are incomplete".into()); }
        progress.phase = StartPhase::Complete;
        Ok(())
    }

    pub(super) fn start_entered(&self) -> bool { self.start.is_some() }
    pub(super) fn stop_external_step(&mut self) -> Result<u8, String> {
        if self.stop == Some(StopPhase::Complete) { return Ok(3); }
        if self.start.as_ref().is_none_or(|start| start.phase != StartPhase::Complete) { return Err("rigid-body Start is incomplete".into()); }
        Ok(match self.stop.get_or_insert(StopPhase::BeforeUnpublish) {
            StopPhase::BeforeUnpublish => 0, StopPhase::Unpublish => 1,
            StopPhase::AfterUnpublish => 2, StopPhase::Complete => 3,
        })
    }
    pub(super) fn advance_external_stop(&mut self) -> Result<(), String> {
        self.stop = Some(match self.stop.ok_or("rigid-body Stop cursor is absent")? {
            StopPhase::BeforeUnpublish => {
                if self.start_listeners.is_some() || self.activation_listener.is_some() { return Err("rigid-body Stop listeners remain registered".into()); }
                StopPhase::Unpublish
            },
            StopPhase::Unpublish => StopPhase::AfterUnpublish,
            StopPhase::AfterUnpublish => { self.start = None; StopPhase::Complete },
            StopPhase::Complete => return Ok(()),
        });
        Ok(())
    }
    pub(super) fn deinitialize_external_step(&mut self) -> Result<u8, String> {
        if self.deinit == Some(DeinitPhase::Complete) { return Ok(4); }
        if self.initialization != Initialization::Ready || self.start.is_some()
            || self.stop.is_some_and(|phase| phase != StopPhase::Complete) {
            return Err("rigid-body must complete Stop before Deinitialize".into());
        }
        Ok(match self.deinit.get_or_insert(DeinitPhase::Unregister) {
            DeinitPhase::Unregister => 0, DeinitPhase::Destroy => 1, DeinitPhase::Base => 2,
            DeinitPhase::RetireShape => 3, DeinitPhase::Complete => 4,
        })
    }
    pub(super) fn advance_external_deinitialize(&mut self) -> Result<(), String> {
        self.deinit = Some(match self.deinit.ok_or("rigid-body Deinitialize cursor is absent")? {
            DeinitPhase::Unregister => {
                if self.initialize_listeners.is_some() { return Err("rigid-body Initialize listeners remain registered".into()); }
                DeinitPhase::Destroy
            },
            DeinitPhase::Destroy => { self.body = None; DeinitPhase::Base },
            DeinitPhase::Base => DeinitPhase::RetireShape,
            _ => return Err("rigid-body Deinitialize cursor cannot advance".into()),
        });
        Ok(())
    }
    pub(super) fn retire_external_geometry(&mut self, geometry: &mut TriggerAssets) -> Result<(), String> {
        if self.deinit != Some(DeinitPhase::RetireShape) { return Err("rigid-body geometry retirement is out of order".into()); }
        geometry.retire_standalone_shape(self.shape.ok_or("rigid-body shape lease is absent")?)?;
        self.shape = None; self.component = None; self.materials.clear();
        self.initialization = Initialization::Retired; self.deinit = Some(DeinitPhase::Complete);
        Ok(())
    }

    /// 14080a260 unregisters its live message/wind/activation callbacks,
    /// unpublishes with mode zero, then detaches the entity interface and runs
    /// the base Stop. Hosts retain progress within each callback group.
    pub(super) fn stop<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut TriggerAssets, world: &mut WorldStorage, events: &mut E,
        before_unpublish: impl FnOnce() -> Result<(), String>,
        after_unpublish: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.stop == Some(StopPhase::Complete) { return Ok(()); }
        if self.start.as_ref().is_none_or(|start| start.phase != StartPhase::Complete) {
            return Err("rigid-body start is incomplete".into());
        }
        let body = self.body.ok_or("rigid-body identity is absent")?;
        let phase = self.stop.get_or_insert(StopPhase::BeforeUnpublish);
        if *phase == StopPhase::BeforeUnpublish {
            before_unpublish()?;
            if self.start_listeners.is_some() || self.activation_listener.is_some() { return Err("rigid-body start listeners remain registered".into()); }
            *phase = StopPhase::Unpublish;
        }
        if *phase == StopPhase::Unpublish {
            world.unpublish_bodies(geometry, events, &[body], 0)?;
            *phase = StopPhase::AfterUnpublish;
        }
        if *phase == StopPhase::AfterUnpublish {
            after_unpublish()?;
            *phase = StopPhase::Complete;
            self.start = None;
        }
        Ok(())
    }

    /// 140809bc0 unregisters the Initialize listeners, destroys the body
    /// with mode zero, clears its ID and runs base Deinitialize. The shape
    /// lease is released only after the body and callbacks are gone.
    pub(super) fn deinitialize<E: PublicationEvents + BodyCreationCallbacks>(
        &mut self, geometry: &mut TriggerAssets, world: &mut WorldStorage, events: &mut E,
        unregister: impl FnOnce() -> Result<(), String>,
        base_deinitialize: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.deinit == Some(DeinitPhase::Complete) { return Ok(()); }
        if self.initialization != Initialization::Ready || self.start.is_some()
            || self.stop.is_some_and(|phase| phase != StopPhase::Complete) {
            return Err("rigid-body must be initialized and stopped before deinitialization".into());
        }
        let phase = self.deinit.get_or_insert(DeinitPhase::Unregister);
        if *phase == DeinitPhase::Unregister {
            unregister()?;
            if self.initialize_listeners.is_some() {
                return Err("rigid-body initialize listeners remain registered".into());
            }
            *phase = DeinitPhase::Destroy;
        }
        if *phase == DeinitPhase::Destroy {
            let body = self.body.ok_or("rigid-body identity is absent")?;
            world.destroy_bodies(geometry, events, &[body], 0)?;
            self.body = None;
            *phase = DeinitPhase::Base;
        }
        if *phase == DeinitPhase::Base {
            base_deinitialize()?;
            *phase = DeinitPhase::RetireShape;
        }
        if *phase == DeinitPhase::RetireShape {
            let shape = self.shape.ok_or("rigid-body shape lease is absent")?;
            geometry.retire_standalone_shape(shape)?;
            self.shape = None;
            self.component = None;
            self.materials.clear();
            self.initialization = Initialization::Retired;
            *phase = DeinitPhase::Complete;
        }
        Ok(())
    }
}
