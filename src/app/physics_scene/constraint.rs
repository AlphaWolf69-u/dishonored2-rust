//!712da0/714560/714cd0/7149b0 on the existing canonical bodies and registry.
use super::*;
use bevy::log::warn;
use crate::{assets::constraints::Definition, gameplay::{constraint::Component,
    entity_events::{EntityEvents, Listener}, entity_lifetime::LiveEntities}};

pub(in crate::app) struct ConstraintObservation<'a> {
    pub name: &'a str,
    pub checked: crate::gameplay::pickable_component::CheckedComponent,
    pub joint: Option<crate::gameplay::physics_world::storage::JointId>,
    pub retiring_joint: Option<crate::gameplay::physics_world::storage::JointId>,
    pub bodies: [u32; 2],
    pub initialized: bool,
    pub started: bool,
    pub pending_enable: bool,
    pub owner_frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub definition: &'a Definition,
}

impl PhysicsScene {
    ///Only a rejected loading-time admission formats this bounded snapshot.
    ///Retain each real gate and generation, rather than another anonymous
    ///provider warning that conflates membership with Initialized eligibility.
    fn constraint_provider_status(&self, live: &LiveEntities, registry: &ComponentRegistry,
        endpoints: [Option<&str>; 2], missing: [bool; 2],
    ) -> Result<String, String> {
        use std::fmt::Write as _;
        let range = self.rigid_class_range()?;
        let mut status = format!("rigid_class_range={range:?}");
        for endpoint in 0..2 {
            let id = endpoints[endpoint].and_then(|name| live.find(name));
            let index = id.and_then(|id| self.shared_indices.get(&id).copied());
            write!(status, " endpoint[{endpoint}]={{name:{:?},authored_absent:{},live:{id:?},live_class:{:?},stage:{:?},header_index:{index:?}",
                endpoints[endpoint], missing[endpoint], id.and_then(|id| live.get(id).map(|(_, class)| class)),
                id.and_then(|id| live.stage(id)))
                .map_err(|error| error.to_string())?;
            if let Some(index) = index {
                let header = self.entities.get(index).ok_or("constraint rejection header index stale")?;
                let metadata = self.types.classes().metadata(header.class)
                    .ok_or("constraint rejection actor class metadata absent")?;
                let class = self.types.classes().name_text(metadata.name)
                    .ok_or("constraint rejection actor class name absent")?;
                write!(status, ",construction_handle:{:?},header_id:{:?},header_name:{:?},class:{}({}),retired:{},external:{},entity_phase:{},start_deferred:{},components:[",
                    self.document_rigid_for_construction(index)?,
                    header.shared, header.name, String::from_utf8_lossy(class), header.class,
                    header.retired, header.external, header.components.phase, header.components.start_deferred)
                    .map_err(|error| error.to_string())?;
                for &packed in header.components.entries.values() {
                    let handle = ComponentHandle::from_packed(packed)?;
                    let owner = self.owner(handle)?;
                    let rigid = self.is_rigid_class(handle.class_id);
                    let missing_shape = if rigid { Some(self.rigid_component(handle)?.missing_physics_shape()) } else { None };
                    write!(status, "{{handle:{handle:?},generation:{},flags:{:#x},owner_entity:{:?},declaration:{:?},start_phases:{:?},rigid:{rigid},runtime_mode1:{},registered:{},queued_start:{},queued_stop:{},queued_release:{},missing_shape:{missing_shape:?}}},",
                        owner.generation, owner.flags, owner.entity, owner.declaration,
                        self.declarations.get(&packed).map(|declaration| declaration.scheduling.start_phases),
                        crate::gameplay::component_state::matches_reference_state(owner.flags, [0x20, 0]),
                        registry.registered.values().contains(&packed), registry.pending_start.values().contains(&packed),
                        registry.pending_stop.values().contains(&packed), registry.pending_release.values().contains(&packed))
                        .map_err(|error| error.to_string())?;
                }
                status.push(']');
            }
            status.push('}');
        }
        Ok(status)
    }
    ///852040 resolves a NULL PhysicsBase to the real worldspawn component's
    ///CURRENT body batch. Its constructed empty list is valid; an absent
    ///component, arbitrary anchor body0, or a named NoShape is not that witness.
    fn publish_constraint_collision_ignore(&mut self, handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut dyn super::constraint_removal::ListenerEdits,
    ) -> Result<(), String> {
        let component = self.constraints.get(handle.slot).ok_or("constraint filter payload absent")?;
        let generation = component.owner.generation;
        let mut progress = component.collision_ignore;
        if progress.complete() { return Ok(()); }
        let bodies = component.bodies;
        let endpoints = component.definition.endpoints();
        let world_endpoint = [0, 1].map(|index| endpoints[index].is_none()
            || component.missing_endpoint_world[index]);
        for index in 0..2 {
            if bodies[index] == 0 && !world_endpoint[index] {
                return Err("constraint collision suppression requires the named NoShape PhysicsBase body-batch provider".into());
            }
        }
        let world_bodies = if world_endpoint.iter().any(|&world| world) {
            if self.level_bodies.allocated_count() != 1 {
                return Err("constraint NULL PhysicsBase requires one actual worldspawn PhysicsWorld component".into());
            }
            let slot = self.level_bodies.roster.indices[0];
            let level = self.level_bodies.get(slot).ok_or("constraint worldspawn PhysicsWorld roster stale")?;
            let index = level.owner.entity.ok_or("constraint worldspawn PhysicsWorld header absent")?;
            let header = self.entities.get(index).ok_or("constraint worldspawn header stale")?;
            let metadata = self.types.classes().metadata(header.class)
                .ok_or("constraint worldspawn class metadata absent")?;
            if header.retired || header.external
                || self.types.classes().name_text(metadata.name) != Some(b"idWorldspawn".as_slice()) {
                return Err("constraint NULL PhysicsBase owner is not actual live worldspawn".into());
            }
            level.bodies.as_slice()
        } else { &[] };
        let singleton = [[bodies[0]], [bodies[1]]];
        let batches: [&[u32]; 2] = [0, 1].map(|index| if world_endpoint[index] {
            world_bodies
        } else { singleton[index].as_slice() });
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "constraint filter world poisoned")?;
        let world = &mut *guard;
        let result = {
            scene_publication_events!(self, registry, publication, &mut *entity_events);
            world.storage.publish_collision_ignores(&mut world.geometry, &mut publication,
                batches[0], batches[1], &mut progress)
        };
        //Retain accepted pair/callback progress on every result, before any
        //caller may unwind Init. Never repeat the refcount mutation on retry.
        let component = self.constraints.get_mut(handle.slot)
            .ok_or("constraint filter callback removed its journal owner")?;
        if component.owner.generation != generation {
            return Err("constraint filter callback changed component generation".into());
        }
        component.collision_ignore = progress;
        result
    }
    ///Only already-entered filter requests. Teardown must finish them while
    ///their real body-batch witnesses and activation owner are still retained.
    pub(super) fn finish_accepted_constraint_ignores(&mut self,
        registry: &mut ComponentRegistry,
        events: &mut dyn super::constraint_removal::ListenerEdits,
    ) -> Result<(), String> {
        let mut position = 0;
        while position < self.constraints.allocated_count() {
            let slot = self.constraints.roster.indices[position];
            let progress = self.constraints.get(slot).ok_or("constraint filter journal roster stale")?
                .collision_ignore;
            if progress.accepted() && !progress.complete() {
                self.publish_constraint_collision_ignore(ComponentHandle {
                    class_id: self.classes[59], slot,
                }, registry, events)?;
            }
            if self.constraints.roster.indices.get(position) == Some(&slot) { position += 1; }
        }
        Ok(())
    }
    pub(super) fn finish_accepted_constraint_ignores_owned(&mut self,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let mut registry = self.registry.take().ok_or("constraint filter continuation registry borrowed")?;
        let result = self.finish_accepted_constraint_ignores(&mut registry, events);
        self.registry = Some(registry);
        result
    }
    ///Opt-in caller borrows existing state; no normal-frame trace/history/cache.
    pub(in crate::app) fn visit_constraints(&self, live: &LiveEntities,
        mut visit: impl FnMut(ConstraintObservation<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        for &slot in self.constraints.roster.indices.iter().take(self.constraints.allocated_count()) {
            let component = self.constraints.get(slot).ok_or("constraint observation roster stale")?;
            let Some(index) = component.owner.entity else { continue; };
            let entity = self.entities.get(index).ok_or("constraint observation owner absent")?;
            let (_, class) = live.get(entity.shared).ok_or("constraint observation managed generation stale")?;
            if class != entity.class { return Err("constraint observation class generation differs".into()); }
            visit(ConstraintObservation {
                name: &entity.name,
                checked: crate::gameplay::pickable_component::CheckedComponent {
                    handle: ComponentHandle { class_id: self.classes[59], slot },
                    generation: component.owner.generation,
                },
                joint: component.joint, retiring_joint: component.retiring_joint,
                bodies: component.bodies, initialized: component.initialized,
                started: component.started, pending_enable: component.pending_enable,
                owner_frame: entity.frame.pose, definition: &component.definition,
            })?;
        }
        Ok(())
    }
    fn placed_motor(&self, live: &LiveEntities, name: &str) -> Result<Option<(usize, crate::gameplay::physics_world::storage::JointId)>, String> {
        let Some(index) = live.find(name).and_then(|id| self.shared_indices.get(&id).copied()) else {
            return Ok(None);
        };
        for &packed in self.entities[index].components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            if handle.class_id == self.classes[59] {
                let component = self.constraints.get(handle.slot).ok_or("placed motor component slot absent")?;
                if component.started && component.definition.settings().motor.is_some() {
                    if let Some(joint) = component.joint { return Ok(Some((index, joint))); }
                }
            }
        }
        Ok(None)
    }
    pub(in crate::app) fn driven_joint(&self, live: &LiveEntities, name: &str)
        -> Result<Option<crate::gameplay::physics_world::storage::JointId>, String> {
        Ok(self.placed_motor(live, name)?.map(|(_, joint)| joint))
    }
    pub(in crate::app) fn motor_lod(&mut self, joint: crate::gameplay::physics_world::storage::JointId,
        far: bool) -> Result<(), String> {
        let mut registry = self.registry.take().ok_or("motor LOD registry absent")?;
        let result = (|| -> Result<(), String> {
            let shared = self.world.clone();
            let mut guard = shared.write().map_err(|_| "motor LOD world poisoned")?;
            let world = &mut *guard;
            let Some(bodies) = world.storage.driven_bodies(joint) else { return Ok(()); };
            let handles = bodies.map(|id| -> Result<ComponentHandle, String> {
                let body = world.storage.bodies.get(id).ok_or("motor LOD endpoint retired")?;
                Ok(crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data)?.component)
            });
            let [a, b] = handles;
            let handles = [a?, b?];
            let mut frozen = [self.rigid_component(handles[0])?.freeze_state, self.rigid_component(handles[1])?.freeze_state];
            let result = {
                scene_publication_events!(self, &mut registry, publication);
                world.storage.set_motor_lod(&mut world.geometry, &mut publication, joint, far, &mut frozen)
            };
            for (handle, state) in handles.into_iter().zip(frozen) {
                self.rigid_component_mut(handle)?.freeze_state = state;
            }
            result
        })();
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn motor_parameters(&self, live: &LiveEntities, name: &str)
        -> Result<Option<(bool, f32, f32, f32)>, String> {
        let Some((_, joint)) = self.placed_motor(live, name)? else { return Ok(None); };
        self.world.read().map_err(|_| "motor world poisoned")?.storage.motor_parameters(joint)
    }
    pub(in crate::app) fn set_motor(&mut self, live: &LiveEntities, name: &str, enabled: Option<bool>,
        position: Option<f32>, velocity: Option<f32>, acceleration: Option<f32>) -> Result<bool, String> {
        let Some((index, joint)) = self.placed_motor(live, name)? else { return Ok(false); };
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "motor world poisoned")?;
        let world = &mut *guard;
        if let Some(bodies) = world.storage.update_motor(joint, enabled, position, velocity, acceleration)? {
            let mut registry = self.registry.take().ok_or("motor registry absent")?;
            let result = (|| -> Result<(), String> {
                // 712a90 wakes the two current endpoints after a real parameter change.
                scene_publication_events!(self, &mut registry, publication);
                for body in bodies {
                    world.storage.activate_body(&mut world.geometry, &mut publication, body)?;
                }
                Ok(())
            })();
            self.registry = Some(registry);
            result?;
            self.entity_flags_d4[index] |= 4;
        }
        Ok(true)
    }
    pub(super) fn admit_placed_constraints(&mut self, catalog: &archive::Catalog,
        document: &entities::Document, models: &[entities::PlacedModel],
        admission: &super::super::machinery::ConstraintAdmission, live: &mut LiveEntities,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let authored_names: std::collections::HashSet<_> = document.definitions.iter()
            .filter(|entry| entry.kind == "entity").map(|entry| entry.id.as_str()).collect();
        let bound_owners: std::collections::HashSet<_> = entities::resolved_entity_binds(document)
            .into_iter().map(|binding| binding.entity_id).collect();
        for binding in crate::assets::constraints::bindings(catalog, document, &admission.legacy)? {
            let driven = admission.motors.contains(&binding.entity)
                && binding.definition.settings().motor.is_some()
                && binding.scheduling.starts_on_spawn() && binding.definition.rigid_rows_supported();
            if !(binding.definition.passive_rows_supported() || driven)
                || binding.definition.joints().iter().any(Option::is_some) {
                //Preserve the previous explicit provider boundary; no fake
                //started component or weld/tether/articulated reinterpretation.
                warn!("constraint {} {} remains unhosted: actual motor/atom/joint provider required",
                    binding.entity, binding.slot);
                continue;
            }
            if bound_owners.contains(&binding.entity) {
                warn!("constraint {} {} remains unhosted: bound owner requires its actual hierarchy/joint-demand admission",
                    binding.entity, binding.slot);
                continue;
            }
            let missing = binding.definition.endpoints()
                .map(|name| name.is_some_and(|name| !authored_names.contains(name)));
            if binding.definition.disable_collision
                && binding.definition.endpoints().iter().enumerate()
                    .any(|(index, name)| name.is_none() || missing[index])
                && self.level_bodies.allocated_count() != 1 {
                warn!("constraint {} {} remains unhosted: actual NULL PhysicsBase worldspawn body-batch provider required",
                    binding.entity, binding.slot);
                continue;
            }
            let mut providers_ready = true;
            for (endpoint, name) in binding.definition.endpoints().into_iter().enumerate() {
                let Some(name) = name else { continue; };
                if missing[endpoint] { continue; }
                let index = live.find(name).and_then(|id| self.shared_indices.get(&id).copied());
                //This hostability check is BEFORE the common Initialize
                //drain. Policy1 requires Initialized and would reject every
                //genuine Registered(+queued Start) endpoint at this boundary.
                let handle = index.map(|index| self.document_rigid_for_construction(index)).transpose()?.flatten();
                if let Some(handle) = handle {
                    let owner = self.owner(handle)?;
                    if owner.entity != index || !matches!(owner.flags & 3, 1 | 2)
                        || (owner.flags & 3 == 1
                            && !registry.registered.values().contains(&handle.packed())) {
                        providers_ready = false;
                    }
                    if binding.definition.disable_collision
                        && self.rigid_component(handle)?.missing_physics_shape() {
                        //Its real empty PhysicsBase batch is a distinct
                        //unported provider; body0 must not suppress the map.
                        providers_ready = false;
                    }
                    //The deferred host enable path requires the actual body
                    //Start in this batch (or an already Started body). It does
                    //not emulate c92dd0's unported pending-body batch producer.
                    if binding.scheduling.starts_on_spawn()
                        && self.owner(handle)?.flags & 0xf != 0xa
                        && !registry.pending_start.values().contains(&handle.packed()) {
                        providers_ready = false;
                    }
                } else {
                    providers_ready = false;
                }
            }
            if !providers_ready {
                let status = self.constraint_provider_status(live, registry,
                    binding.definition.endpoints(), missing)?;
                warn!("constraint {} {} remains unhosted: actual named endpoint RigidbodyBase provider required; constraint_starts_on_spawn={} disable_collision={} {status}",
                    binding.entity, binding.slot, binding.scheduling.starts_on_spawn(), binding.definition.disable_collision);
                continue;
            }
            let id = if let Some(entity) = self.entities.iter()
                .find(|entity| !entity.retired && entity.name == binding.entity) {
                entity.shared
            } else {
                let authored = document.definitions.iter()
                    .find(|entry| entry.kind == "entity" && entry.id == binding.entity)
                    .ok_or("constraint actual authored entity absent")?;
                let class_name = entities::scene_motion::atom(&authored.fields, "class")
                    .ok_or("constraint actual authored class absent")?;
                let class = self.types.classes().hierarchy().registration_order().iter().copied()
                    .find(|&index| self.types.classes().metadata(index)
                        .and_then(|metadata| self.types.classes().name_text(metadata.name))
                        == Some(class_name.as_bytes()))
                    .ok_or("constraint actual authored class not registered")?;
                let frame = entities::spawn_frame(authored)?;
                let scale = models.iter().find(|model| model.id == binding.entity)
                    .map_or([1.; 3], |model| model.scale);
                self.allocate_entity_header(live, &binding.entity, class, 0,
                    super::super::runtime_entities::SpawnOptions { frame, scale: None,
                        hidden_disabled: false }, scale,
                    Some(Arc::from(binding.entity.as_str())), true, false)?
            };
            let index = *self.shared_indices.get(&id).ok_or("constraint actual header absent")?;
            let declaration = self.types.declaration(self.factories[59],
                Arc::as_ptr(&binding.definition) as usize, binding.scheduling)?;
            let immediate = declaration.scheduling.starts_on_spawn();
            if let Some(handle) = self.create_constraint_component(index, binding.definition,
                &declaration, registry, immediate)? {
                self.constraints.get_mut(handle.slot).ok_or("constraint admitted payload absent")?
                    .missing_endpoint_world = missing;
                self.declarations.insert(handle.packed(), declaration);
            }
        }
        Ok(())
    }
    pub(super) fn create_constraint_component(&mut self, index: usize,
        definition: Arc<Definition>, declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry, immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        if !definition.passive_rows_supported() {
            return Err("constraint requires its actual unsupported motor/atom-row provider".into());
        }
        if definition.joints().iter().any(Option::is_some) {
            return Err("constraint requires the actual articulated joint-to-body provider".into());
        }
        ComponentFactory { declaration, pool: &mut self.constraints, registry,
            grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                immediate, |_, _, _| Ok(Component::new(definition)), |_, _, _| Ok(()))
    }
    fn constraint_endpoint(&mut self, live: &LiveEntities, name: Option<&str>, missing_is_world: bool,
        registry: &mut ComponentRegistry, events: &mut EntityEvents,
    ) -> Result<u32, String> {
        //712da0 defaults unresolved managed references/absent PhysicsBase to
        //the world's actual body0. Never use this branch for an admitted
        //articulated provider whose selected joint cannot be resolved.
        let Some(name) = name else { return Ok(0); };
        if missing_is_world { return Ok(0); }
        let id = live.find(name).ok_or_else(|| format!(
            "constraint endpoint {name}: actual source actor/provider is not admitted"))?;
        live.get(id).ok_or("constraint endpoint managed generation stale")?;
        let &index = self.shared_indices.get(&id)
            .ok_or("constraint endpoint actual scene provider absent")?;
        let handle = self.document_rigid(index)?
            .ok_or("constraint named endpoint actual RigidbodyBase provider absent")?;
        //Source virtual+e0(...,joint,1) lazily obtains this SAME body's
        //resources. Its initializer is resumable and never creates a proxy.
        self.initialize_placed_rigid(live, handle, registry, events)?;
        let component = self.rigid_component(handle)?;
        match component.body() {
            Some(body) => Ok(body),
            None if component.missing_physics_shape() => Ok(0),
            None => Err("constraint RigidbodyBase Init returned without a body/absence certificate".into()),
        }
    }
    pub(super) fn initialize_constraint(&mut self, live: &LiveEntities,
        events: &mut EntityEvents, registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self.constraints.get(handle.slot).ok_or("constraint Init slot absent")?;
        if component.initialized { return Ok(()); }
        let index = component.owner.entity.ok_or("constraint Init owner absent")?;
        let generation = component.owner.generation;
        let definition = component.definition.clone();
        let missing = component.missing_endpoint_world;
        let id = self.entities[index].shared;
        live.get(id).ok_or("constraint Init managed generation stale")?;
        if component.observer_phase == 0 {
            let endpoints = definition.endpoints();
            for endpoint in 0..2 {
                if self.constraints.get(handle.slot).unwrap().endpoints_resolved & (1 << endpoint) != 0 { continue; }
                let body = self.constraint_endpoint(live, endpoints[endpoint], missing[endpoint], registry, events)?;
                let component = self.constraints.get_mut(handle.slot).unwrap();
                component.bodies[endpoint] = body;
                component.endpoints_resolved |= 1 << endpoint;
            }
            let bodies = self.constraints.get(handle.slot).unwrap().bodies;
            if bodies[0] == bodies[1] {
                let component = self.constraints.get_mut(handle.slot).unwrap();
                component.initialized = true;
                component.owner.mark_initialized();
                return Ok(()); //source no-joint return, not a synthetic body.
            }
            let settings = definition.at_frame(&self.entities[index].name,
                self.entities[index].frame.pose);
            let timespace = usize::try_from(self.entities[index].timespace)
                .map_err(|_| "constraint actual timespace index overflow")?;
            let shared = self.world.clone();
            let mut world = shared.write().map_err(|_| "constraint world owner poisoned")?;
            let joint = world.storage.add_joint(bodies, settings, timespace)?;
            let component = self.constraints.get_mut(handle.slot).unwrap();
            component.joint = Some(joint);
            component.observer_phase = 1;
        }
        if self.constraints.get(handle.slot).unwrap().observer_phase == 1 {
            //7164c0 is consolidated at the world's actual body-unpublish
            //boundary. The real Removed signal still owns716560 withdrawal.
            self.signals.constraint_removed.subscribe(handle.packed() as usize, generation as usize);
            self.constraints.get_mut(handle.slot).unwrap().observer_phase = 2;
        }
        if self.constraints.get(handle.slot).unwrap().observer_phase == 2 {
            if definition.disable_collision {
                self.publish_constraint_collision_ignore(handle, registry, events)?;
            }
            self.constraints.get_mut(handle.slot).ok_or("constraint filter callback released Init owner")?
                .observer_phase = 3;
        }
        for event in [0xad, 0xac] {
            events.register(event, Some(id), Listener::Component {
                owner: self.owner_id, handle, generation, handler: event,
            });
        }
        let component = self.constraints.get_mut(handle.slot).unwrap();
        component.initialized = true;
        component.owner.mark_initialized();
        Ok(())
    }
    ///Called after the shared base Start, retaining actual source enable work
    ///until ALL body Start callbacks in that same batch have published.
    pub(super) fn start_constraint(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self.constraints.get_mut(handle.slot).ok_or("constraint Start slot absent")?;
        if !component.initialized { return Err("constraint Start precedes actual Init".into()); }
        component.started = true;
        if component.joint.is_some() {
            component.pending_enable = true;
            self.constraints_pending_enable = true;
            let index = component.owner.entity.ok_or("constraint Start owner absent")?;
            self.entity_flags_d4[index] |= 4;
        } else {
            warn!("constraint {:?}: equal/unresolved endpoints; no joint created", component.bodies);
        }
        Ok(())
    }
    pub(super) fn stop_constraint(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self.constraints.get_mut(handle.slot).ok_or("constraint Stop slot absent")?;
        component.started = false;
        component.pending_enable = false;
        Ok(()) //7811f0/base Stop does not disable an existing physical joint.
    }
    pub(super) fn deinitialize_constraint(&mut self, handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut dyn super::constraint_removal::ListenerEdits,
    ) -> Result<(), String> {
        let progress = self.constraints.get(handle.slot).ok_or("constraint Deinit slot absent")?
            .collision_ignore;
        if progress.accepted() && !progress.complete() {
            self.publish_constraint_collision_ignore(handle, registry, entity_events)?;
        }
        let component = self.constraints.get(handle.slot).ok_or("constraint Deinit slot absent")?;
        let generation = component.owner.generation;
        let joint = component.joint.or(component.retiring_joint);
        if let Some(joint) = joint {
            let shared = self.world.clone();
            let mut guard = shared.write().map_err(|_| "constraint Deinit world poisoned")?;
            let world = &mut *guard;
            scene_publication_events!(self, registry, publication, &mut *entity_events);
            world.storage.remove_joint(&mut world.geometry, &mut publication, joint, true)?;
        }
        let Some(component) = self.constraints.get_mut(handle.slot) else { return Ok(()); };
        if component.owner.generation != generation {
            return Err("constraint Deinit component generation changed".into());
        }
        if component.joint.is_some() || component.retiring_joint.is_some() {
            return Err("constraint Deinit precedes actual removal acknowledgement".into());
        }
        component.initialized = false;
        component.started = false;
        component.pending_enable = false;
        if let Some(index) = component.owner.entity {
            let id = self.entities.get(index).ok_or("constraint Deinit owner stale")?.shared;
            for event in [0xad, 0xac] {
                entity_events.unregister(event, Some(id), Listener::Component {
                    owner: self.owner_id, handle, generation, handler: event,
                });
            }
            self.entities[index].components.release(handle, component)?;
        }
        self.signals.constraint_removed.unsubscribe(handle.packed() as usize, generation as usize);
        Ok(())
    }
    pub(super) fn constraint_message(&mut self, live: &LiveEntities,
        registry: &mut ComponentRegistry, listener: Listener,
        event: &crate::gameplay::entity_events::Event,
        commands: &mut crate::gameplay::entity_events::Commands,
    ) -> Result<(), String> {
        let Listener::Component { owner, handle, generation, handler } = listener else {
            return Err("constraint timespace listener differs".into());
        };
        if owner != self.owner_id || handle.class_id != self.classes[59]
            || !matches!(handler, 0xad | 0xac) || event.id != handler {
            return Err("constraint timespace callback identity differs".into());
        }
        let component = self.constraints.get(handle.slot).ok_or("constraint callback slot absent")?;
        if component.owner.generation != generation {
            return Err("constraint callback generation stale".into());
        }
        let index = component.owner.entity.ok_or("constraint callback owner absent")?;
        let id = self.entities[index].shared;
        if event.target != Some(id) || live.get(id).is_none() {
            return Err("constraint callback target generation differs".into());
        }
        let joint = component.joint.ok_or("constraint timespace callback lost actual joint")?;
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "constraint timespace world poisoned")?;
        let world = &mut *guard;
        scene_publication_events!(self, registry, publication, &mut *commands);
        world.storage.set_joint_timespace(&mut world.geometry, &mut publication, joint,
            usize::from(handler == 0xad))
    }
    ///A real loading/runtime readiness boundary, not a simulated Start or a
    ///per-frame settled roster scan. False retains the exact component work.
    pub(in crate::app) fn finish_constraint_admission(&mut self,
        registry: &mut ComponentRegistry, entity_events: &mut EntityEvents,
    ) -> Result<bool, String> {
        if !self.constraints_pending_enable { return Ok(true); }
        let mut pending = false;
        let mut position = 0;
        while position < self.constraints.allocated_count() {
            let slot = self.constraints.roster.indices[position];
            let component = self.constraints.get(slot).ok_or("constraint admission roster stale")?;
            if !component.pending_enable { position += 1; continue; }
            let joint = component.joint.ok_or("constraint enable lost actual joint")?;
            let shared = self.world.clone();
            let mut guard = shared.write().map_err(|_| "constraint admission world poisoned")?;
            let world = &mut *guard;
            if self.constraints.get(slot).unwrap().enable_phase == 0 {
                if !world.storage.joint_endpoints_ready(joint)? { pending = true; position += 1; continue; }
                self.constraints.get_mut(slot).unwrap().enable_phase = 1;
            }
            if self.constraints.get(slot).unwrap().enable_phase == 1 {
                // 714cd0 passes zero to c91140 when the motor starts enabled.
                let settings = self.constraints.get(slot).unwrap().definition.settings();
                let keep_asleep = !(settings.motor.is_some() && settings.started);
                {
                    scene_publication_events!(self, registry, publication, &mut *entity_events);
                    world.storage.set_joint_enabled(&mut world.geometry, &mut publication, joint, true, keep_asleep)?;
                }
                self.constraints.get_mut(slot).unwrap().enable_phase = 2;
            }
            {
                scene_publication_events!(self, registry, publication, &mut *entity_events);
                world.storage.synchronize_joint_time(&mut world.geometry, &mut publication, joint)?;
            }
            //A source removal callback may have released this exact slot;
            //the shared registry still controls when any generation is reused.
            if let Some(component) = self.constraints.get_mut(slot) {
                if component.joint == Some(joint) { component.pending_enable = false; component.enable_phase = 0; }
            }
            if self.constraints.roster.indices.get(position) == Some(&slot) { position += 1; }
        }
        pending |= self.constraints.roster.indices.iter().take(self.constraints.allocated_count())
            .any(|&slot| self.constraints.get(slot).is_some_and(|component| component.pending_enable));
        self.constraints_pending_enable = pending;
        Ok(!pending)
    }
}
