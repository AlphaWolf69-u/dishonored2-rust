//!712da0/714560/714cd0/7149b0 on the existing canonical bodies and registry.
use super::*;
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
    pub(super) fn admit_placed_constraints(&mut self, catalog: &archive::Catalog,
        document: &entities::Document, models: &[entities::PlacedModel],
        legacy_owners: &std::collections::HashSet<String>, live: &mut LiveEntities,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let authored_names: std::collections::HashSet<_> = document.definitions.iter()
            .filter(|entry| entry.kind == "entity").map(|entry| entry.id.as_str()).collect();
        let bound_owners: std::collections::HashSet<_> = entities::resolved_entity_binds(document)
            .into_iter().map(|binding| binding.entity_id).collect();
        for binding in crate::assets::constraints::bindings(catalog, document, legacy_owners)? {
            if !binding.definition.passive_rows_supported()
                || binding.definition.joints().iter().any(Option::is_some) {
                //Preserve the previous explicit provider boundary; no fake
                //started component or weld/tether/articulated reinterpretation.
                warn!("constraint {} {} remains unhosted: actual motor/atom/joint/suppression provider required",
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
            let mut providers_ready = true;
            for (endpoint, name) in binding.definition.endpoints().into_iter().enumerate() {
                let Some(name) = name else { continue; };
                if missing[endpoint] { continue; }
                let index = live.find(name).and_then(|id| self.shared_indices.get(&id).copied());
                let handle = index.map(|index| self.document_rigid(index)).transpose()?.flatten();
                if let Some(handle) = handle {
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
                warn!("constraint {} {} remains unhosted: actual named endpoint RigidbodyBase provider required",
                    binding.entity, binding.slot);
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
            return Err("constraint requires its actual unsupported motor/atom-row/suppression provider".into());
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
            let joint = self.constraints.get(handle.slot).unwrap().joint
                .ok_or("constraint Init joint lease absent")?;
            if definition.disable_collision {
                return Err("constraint requires actual852040 collision-suppression batch/lifetime".into());
            }
            //7164c0 is consolidated at the world's actual body-unpublish
            //boundary. The real Removed signal still owns716560 withdrawal.
            self.signals.constraint_removed.subscribe(handle.packed() as usize, generation as usize);
            self.constraints.get_mut(handle.slot).unwrap().observer_phase = 2;
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
                {
                    scene_publication_events!(self, registry, publication, &mut *entity_events);
                    world.storage.set_joint_enabled(&mut world.geometry, &mut publication, joint, true, true)?;
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
