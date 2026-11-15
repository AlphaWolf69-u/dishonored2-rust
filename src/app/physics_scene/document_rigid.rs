//! Ordinary document bodies use the canonical 808ea0/809d80 owners.
use super::*;

impl PhysicsScene {
    pub(super) fn initialize_placed_rigid(&mut self, live: &crate::gameplay::entity_lifetime::LiveEntities,
        handle: ComponentHandle, registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        let generation = self.owner(handle)?.generation;
        let component = self.rigid_component(handle, generation)?;
        let index = component.owner.entity.ok_or("placed rigid entity is absent")?;
        let entity = self.entities.get(index).ok_or("placed rigid entity is absent")?;
        let id = entity.shared;
        if live.get(id).is_none() { return Err("placed rigid managed entity is stale".into()); }
        if component.is_initialized() { return Ok(()); }
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if component.has_prepared_resources() {
            let definition = component.definition().ok_or("placed rigid definition is absent")?;
            // Rotation locks require the source ref-counted MassDistribution;
            // reject that authored branch before consuming the prepared lease.
            if definition.settings.lock_rotation.into_iter().any(|locked| locked) {
                return Err("placed rigid rotation locks require the authored mass-distribution owner".into());
            }
            if !world.storage.motion_properties.added.is_empty() {
                return Err("placed rigid motion-property addition requires its registered observer host".into());
            }
            let flags = definition.declaration_flags;
            let damping = definition.settings.angular_damping.ok_or("placed rigid angular damping is absent")?;
            let frame = entity.frame.pose;
            let mut base_profile = 1;
            if damping > 0.0 {
                let mut profile = crate::gameplay::motion_properties::MotionProfile::from_words([0; 16]);
                profile.configure(1, f32::from_bits(0x411c_f5c3));
                let mut words = *profile.words();
                words[7] = damping.to_bits();
                base_profile = world.storage.motion_properties.add(crate::gameplay::motion_properties::MotionProfile::from_words(words),
                    |_, _, _, _| unreachable!("motion-property added signal was checked empty"))?;
            }
            let property = world.storage.motion_timespaces.as_mut().ok_or("placed rigid physics timespaces are absent")?
                .resolve(&mut world.storage.motion_properties, base_profile, usize::try_from(entity.timespace).map_err(|_| "placed rigid timespace overflow")?,
                    |_, _, _, _| unreachable!("motion-property added signal was checked empty"))?;
            // Resolve density before transferring the retained resource lease.
            let material = self.rigid_bodies.get(handle.slot).ok_or("placed rigid component is absent")?
                .prepared_material().ok_or("placed rigid materials are absent")?;
            let density = world.storage.materials.material_for_id(material)?.density;
            let (_, resources) = self.rigid_bodies.get_mut(handle.slot).ok_or("placed rigid component is absent")?
                .take_prepared_resources().ok_or("placed rigid prepared resource lease is absent")?;
            // The first source material controls density even when the body
            // material is zero because the shape has per-surface tags.
            let basis = crate::physworld::rigid_body_basis(frame.basis.map(|v| [v[0], v[1], v[2], 0.0]));
            let info = crate::physworld::body::BodyInfo {
                flags: 0x80 | if flags & 2 != 0 { 0x40000 } else { 0 } | if flags & 4 != 0 { 0x20000 } else { 0 },
                filter_info: resources.filter_info, material_id: resources.body_material,
                motion_type: 2, motion_properties_id: property,
                position: [frame.position[0], frame.position[1], frame.position[2], 0.0],
                orientation: crate::physworld::rotation_quaternion(basis),
                ..Default::default()
            };
            let component = self.rigid_bodies.get_mut(handle.slot).ok_or("placed rigid component is absent")?;
            component.begin_create(handle, resources.shape, resources.extra, info, resources.materials, &mut world.geometry)?;
            component.resolve_creation_mass(&world.geometry, density)?;
        }
        if self.rigid_bodies.get(handle.slot).ok_or("placed rigid component is absent")?.creation_pending() {
            if let Some(info) = self.rigid_bodies.get(handle.slot).ok_or("placed rigid component is absent")?.creation_request()? {
                world.storage.begin_disabled_component_body(handle, generation, info)?;
                self.rigid_bodies.get_mut(handle.slot).ok_or("placed rigid component is absent")?.creation_entered();
            }
            let body = {
                scene_publication_events!(self, registry, events);
                world.storage.resume_disabled_component_body(&mut world.geometry, &mut events, handle, generation)?
            };
            self.rigid_bodies.get_mut(handle.slot).ok_or("placed rigid component is absent")?.creation_completed(body)?;
            let transform = world.storage.bodies.get(body).ok_or("created rigid body absent")?.transform;
            self.initial_rigid_frames.insert(index, crate::gameplay::reference_frame::ReferenceFrame {
                position: [transform[3][0], transform[3][1], transform[3][2]],
                basis: std::array::from_fn(|row| [transform[row][0], transform[row][1], transform[row][2]]),
            });
        }
        let component = self.rigid_bodies.get_mut(handle.slot).ok_or("placed rigid component is absent")?;
        if component.base_initialize_pending() {
            // 726ae0 clears +b4 low three bits. No pre-Initialize request can
            // reach this newly admitted component; 726f10's pending branch is
            // therefore absent, rather than replaced by an empty callback.
            component.initialize_base(|| Ok(()))?;
        }
        if component.initialize_listeners_pending() {
            component.register_initialize_listeners(entity_events, self.owner_id, id, false)?;
        }
        component.finish_initialize()
    }

    fn rigid_component(&self, handle: ComponentHandle, generation: u32)
        -> Result<&super::super::rigid_body::RigidBodyComponent, String> {
        if handle.class_id != self.classes[8] { return Err("component is not RigidBody".into()); }
        let component = self.rigid_bodies.get(handle.slot).ok_or("RigidBody component absent")?;
        if component.owner.generation != generation { return Err("RigidBody generation is stale".into()); }
        Ok(component)
    }

    pub(super) fn prepare_document_rigids(&mut self, catalog: &archive::Catalog, document: &entities::Document,
        interactions: &super::super::interactions::Interactions, models: &[entities::PlacedModel],
        texts: &mut crate::assets::declaration_texts::PhysicsSeed,
        sources: &mut crate::physworld::source::ShapeSources,
        registry: &mut ComponentRegistry) -> Result<(), String> {
        let model_indices: HashMap<_,_> = models.iter().enumerate().map(|(i,m)| (m.id.as_str(),i)).collect();
        let definitions: HashMap<_,_> = document.definitions.iter().filter(|definition| definition.kind == "entity")
            .map(|definition| (definition.id.as_str(), definition)).collect();
        let mut projections = [0usize; 2];
        let mut materials = super::super::material_loading::MaterialCache::default();
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        for binding in &interactions.readable {
            let Some(definition) = &binding.rigid else { continue; };
            (|| -> Result<(), String> {
                let pickable = self.readable_components.get(&binding.entity).ok_or("admitted readable component absent")?;
                let pickable_owner = &self.pickables.get(pickable.handle.slot).ok_or("admitted readable slot absent")?.owner;
                if pickable_owner.generation != pickable.generation { return Err("admitted readable generation is stale".into()); }
                let index = pickable_owner.entity.ok_or("admitted readable entity absent")?;
                if self.entities.get(index).is_none_or(|entity| entity.name != binding.entity) {
                    return Err("admitted readable entity identity differs".into());
                }
                let authored = definitions.get(binding.entity.as_str())
                    .ok_or_else(|| format!("readable rigid {}: authored entity definition absent", binding.entity))?;
                let source_index = model_indices.get(binding.entity.as_str()).copied();
                //808ea0 consumes the RenderModel name and scale, not a color draw.
                // Hidden entities keep that authored descriptor without allocating
                // a mesh/material/GPU projection merely to acquire their bodies.
                let descriptor;
                let model = match source_index {
                    Some(index) => &models[index],
                    None => {
                        descriptor = entities::model_descriptor(authored, |path| catalog.read(path)
                            .map_err(|error| error.to_string()).map(|bytes| bytes.map(entities::owned_declaration_text)))?
                            .ok_or_else(|| format!("readable rigid {}: no authored RenderModel descriptor", binding.entity))?;
                        &descriptor
                    }
                };
                let hidden = entities::entity_hidden(authored);
                if hidden { self.entities[index].entity_flags |= 1; }
                let settings = &definition.settings;
                if settings.lock_rotation.iter().any(|v| *v)
                    || settings.wind_speed_factor.ok_or("rigid wind factor absent")? > 0.0
                    || definition.declaration_flags & 0x80 != 0 {
                    return Err(format!("{}: authored rigid requires unhosted rotation/wind/critical owner", binding.entity));
                }
                let behavior = settings.physics_behavior.as_deref().and_then(entities::rigid_body_behavior_kind)
                    .ok_or("document rigid behavior absent")?;
                // 809d80 publishes the newly created dynamic body deactivated
                // when startAsleep is true. It does not run a simulation step.
                // Keep the real dynamic motion in the shared world; changing it
                // to keyframed/static here would change later pickup callbacks.
                if behavior == 2 && settings.start_asleep != Some(true) {
                    return Err("awake document rigid requires the canonical active simulation owner".into());
                }
                let render = if settings.material.is_none() {
                    crate::assets::bmodel::acquire(catalog, &model.resource_path, crate::assets::bmodel::CommonModels::shared(catalog))?
                } else { None };
                if !world.storage.materials.library.added.is_empty() {
                    return Err("rigid resource preparation requires actual material observer host".into());
                }
                let resources = super::super::rigid_body::prepare_resources(catalog, sources, texts,
                    &mut world.storage.materials, settings, model, render.as_deref(), Some(&mut materials),
                    |_,_,_,_| unreachable!("startup material observers absent"))
                    .map_err(|error| format!("readable rigid {} [{}], model={}, hidden={hidden}, color_projection={source_index:?}: {error}",
                        binding.entity, settings.declaration_id, model.resource_path))?;
                let declaration = self.types.declaration(self.factories[8], Arc::as_ptr(definition) as usize,
                    definition.scheduling.as_ref().clone())?;
                let bodies = &mut self.actors.bodies;
                let free = &mut self.free_rigid_movement;
                let handle = ComponentFactory { declaration: &declaration, pool: &mut self.rigid_bodies,
                    registry, grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                        declaration.scheduling.starts_on_spawn(), |_,_,_| {
                            let mut component = super::super::rigid_body::RigidBodyComponent::new(definition.clone(), bodies, free)?;
                            component.bind_prepared_resources(source_index, resources)?;
                            Ok(component)
                        }, |_,_,_| Ok(()))?.ok_or("document rigid creation disabled")?;
                if let Some(source_index) = source_index { self.rigid_placements.insert(index, source_index); }
                self.declarations.insert(handle.packed(), declaration);
                projections[usize::from(source_index.is_some())] += 1;
                Ok(())
            })().map_err(|error| format!("readable rigid entity={} declaration={}: {error}",
                binding.entity, definition.settings.declaration_id))?;
        }
        materials.finish_body_physics_preparation();
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("READABLE_RIGID_RESOURCES phase=prepared descriptor_only={} color_projection={}", projections[0], projections[1]);
        }
        Ok(())
    }

    pub(super) fn start_placed_rigid(&mut self, live: &crate::gameplay::entity_lifetime::LiveEntities,
        handle: ComponentHandle, registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        let generation = self.owner(handle)?.generation;
        let component = self.rigid_component(handle, generation)?;
        let index = component.owner.entity.ok_or("rigid entity absent")?;
        let entity = &self.entities[index];
        let id = entity.shared;
        self.validate_document_hierarchy(live, index)?;
        let definition = component.definition().ok_or("rigid definition absent")?;
        let authored_behavior = definition.settings.physics_behavior.as_deref().and_then(entities::rigid_body_behavior_kind)
            .ok_or("rigid behavior absent")?;
        let asleep = definition.settings.start_asleep.ok_or("rigid startAsleep absent")?;
        let body = component.body().ok_or("rigid body absent")?;
        let movement = component.movement_body()?;
        let behavior = match component.start_behavior {
            Some(value) => value,
            None => self.bound_document_behavior(live, index, authored_behavior)?,
        };
        let pose = entity.frame.pose;
        let basis = crate::physworld::rigid_body_basis(pose.basis.map(|v| [v[0],v[1],v[2],0.]));
        let frame = [basis[0],basis[1],basis[2],[pose.position[0],pose.position[1],pose.position[2],0.]];
        if !component.start_entered() {
            let scheduling = self.declarations.get(&handle.packed()).ok_or("rigid scheduling absent")?.scheduling.clone();
            let priority = entity.priority;
            let flags = &mut self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.owner.flags;
            if *flags & (1<<30) != 0 { *flags &= 0xf800003f; }
            if crate::gameplay::component_state::prepare_admission(flags, u8::from(scheduling.priority<8), scheduling.priority, priority) {
                self.rigid_bodies.schedule(handle.slot)?;
            }
        }
        if self.actors.bodies[movement].actor.is_none() {
            let actor = self.reference_actor(index, registry)?;
            self.actors.bodies[movement].body_id = body;
            // This admission rejects parent/binds; no authored motion request
            // exists before Start. Unexpected requests cannot be swallowed.
            self.actors.attach_body(usize::from(actor.slot), movement, self.post_frame,
                |_,_| Err("rigid attachment has an unhosted pre-physics motion request".into()))?;
        }
        let activation = if self.owner(handle)?.flags & 0x30 > 0x1f || asleep {
            crate::gameplay::body_storage::BodyActivation::KeepDeactivated
        } else { crate::gameplay::body_storage::BodyActivation::Activate };
        let step = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.start_publication_step(frame, activation)?;
        self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.start_behavior = Some(behavior);
        if step == 3 { return Ok(()); }
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step == 0 {
            { scene_publication_events!(self, registry, events);
              world.storage.set_body_transform(&mut world.geometry, &mut events, body, frame, 0)?; }
            self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.advance_start_publication()?;
        }
        if step <= 1 {
            { scene_publication_events!(self, registry, events);
              world.storage.add_dynamic_body_resumable(&mut world.geometry, &mut events, body, activation)?; }
            self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.advance_start_publication()?;
        }
        { scene_publication_events!(self, registry, events);
          match behavior {
              0 => world.storage.static_single_body(&mut world.geometry, &mut events, body)?,
              1 => world.storage.keyframe_single_body(&mut world.geometry, &mut events, body)?,
              // 727a00 compares the current body type with the declaration.
              // BodyInfo created this body dynamic already: equality returns
              // without a c86800 motion conversion (including sleeping loads).
              2 => {},
              _ => return Err("rigid body behavior is invalid".into()),
          }
        }
        let flags = world.storage.bodies.get(body).ok_or("rigid body retired")?.flags;
        let component = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?;
        component.on_body_activation(false, flags, &mut self.entity_flags_d4[index], registry,
            || Err("inactive rigid callback requested hierarchy update".into()))?;
        if !component.activation_listener_registered() { component.register_activation_listener(&mut self.body_activation_events)?; }
        if !component.start_listeners_registered() {
            component.register_start_listeners(entity_events, self.owner_id, id, false,
                || Err("positive wind factor was not admitted".into()))?;
        }
        component.finish_start(|| Ok(()))
    }

    pub(super) fn stop_placed_rigid(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        let body = self.rigid_bodies.get(handle.slot).ok_or("rigid absent")?.body().ok_or("rigid body absent")?;
        let step = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.stop_external_step()?;
        if step == 3 { return Ok(()); }
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step == 0 {
            world.storage.preflight_keyframe_single_body(body)?;
            let component = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?;
            component.unregister_start_listeners(entity_events, false, || Err("unadmitted wind registration".into()))?;
            component.unregister_activation_listener(&mut self.body_activation_events)?;
            component.advance_external_stop()?;
        }
        if step <= 1 {
            { scene_publication_events!(self, registry, events);
              world.storage.unpublish_bodies(&mut world.geometry, &mut events, &[body], 0)?; }
            self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.advance_external_stop()?;
        }
        let component = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?;
        let movement = component.movement_body()?;
        if let Some(actor) = self.actors.bodies[movement].actor { self.actors.detach_body(actor,movement)?; }
        component.clear_body_update_pause();
        component.advance_external_stop()?;
        Ok(())
    }

    pub(super) fn deinitialize_placed_rigid(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents) -> Result<(), String> {
        let step = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.deinitialize_external_step()?;
        if step == 4 { return Ok(()); }
        if step == 0 {
            let component = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?;
            component.unregister_initialize_listeners(entity_events,false)?;
            component.advance_external_deinitialize()?;
        }
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step <= 1 {
            let body = self.rigid_bodies.get(handle.slot).ok_or("rigid absent")?.body().ok_or("rigid body absent")?;
            { scene_publication_events!(self, registry, events);
              world.storage.destroy_bodies(&mut world.geometry, &mut events, &[body],0)?; }
            self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.advance_external_deinitialize()?;
        }
        if step <= 2 {
            let component = self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?;
            let index = component.owner.entity.ok_or("rigid entity absent")?;
            self.entities[index].components.release(handle,component)?;
            component.advance_external_deinitialize()?;
        }
        self.rigid_bodies.get_mut(handle.slot).ok_or("rigid absent")?.retire_external_geometry(&mut world.geometry)
    }
}
