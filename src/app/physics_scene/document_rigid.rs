//! Ordinary document bodies use the canonical 808ea0/809d80 owners.
use super::*;

impl PhysicsScene {
    pub(super) fn initialize_placed_rigid(
        &mut self,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents,
    ) -> Result<(), String> {
        let generation = self.owner(handle)?.generation;
        let component = self.checked_rigid_component(handle, generation)?;
        let index = component
            .owner
            .entity
            .ok_or("placed rigid entity is absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("placed rigid entity is absent")?;
        let id = entity.shared;
        if live.get(id).is_none() {
            return Err("placed rigid managed entity is stale".into());
        }
        if component.missing_physics_shape(){
            //8097bc dispatches virtual10=7e7530, which unlinks the actual
            //component without initializing or publishing a substitute body.
            let pool=if handle.class_id==self.classes[8]{&mut self.rigid_bodies}else{&mut self.throwables};
            let component=pool.get_mut(handle.slot).ok_or("missing-shape rigid slot absent")?;
            self.entities[index].components.release(handle,component)?;
            self.rigid_placements.remove(&index);
            return Ok(());
        }
        if component.is_initialized() {
            return Ok(());
        }
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if component.has_prepared_resources() {
            let definition = component
                .definition()
                .ok_or("placed rigid definition is absent")?;
            if !world.storage.motion_properties.added.is_empty() {
                return Err(
                    "placed rigid motion-property addition requires its registered observer host"
                        .into(),
                );
            }
            let flags = definition.declaration_flags;
            let damping = definition
                .settings
                .angular_damping
                .ok_or("placed rigid angular damping is absent")?;
            let frame = entity.frame.pose;
            let mut base_profile = 1;
            if damping > 0.0 {
                let mut profile =
                    crate::gameplay::motion_properties::MotionProfile::from_words([0; 16]);
                profile.configure(1, f32::from_bits(0x411c_f5c3));
                let mut words = *profile.words();
                words[7] = damping.to_bits();
                base_profile = world.storage.motion_properties.add(
                    crate::gameplay::motion_properties::MotionProfile::from_words(words),
                    |_, _, _, _| unreachable!("motion-property added signal was checked empty"),
                )?;
            }
            let property = world
                .storage
                .motion_timespaces
                .as_mut()
                .ok_or("placed rigid physics timespaces are absent")?
                .resolve(
                    &mut world.storage.motion_properties,
                    base_profile,
                    usize::try_from(entity.timespace)
                        .map_err(|_| "placed rigid timespace overflow")?,
                    |_, _, _, _| unreachable!("motion-property added signal was checked empty"),
                )?;
            // Resolve density before transferring the retained resource lease.
            let material = self
                .rigid_component(handle)?
                .prepared_material()
                .ok_or("placed rigid materials are absent")?;
            let density = world.storage.materials.material_for_id(material)?.density;
            let (_, resources) = rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .take_prepared_resources()
                .ok_or("placed rigid prepared resource lease is absent")?;
            // The first source material controls density even when the body
            // material is zero because the shape has per-surface tags.
            let basis =
                crate::physworld::rigid_body_basis(frame.basis.map(|v| [v[0], v[1], v[2], 0.0]));
            let info = crate::physworld::body::BodyInfo {
                flags: 0x80
                    | if flags & 2 != 0 { 0x40000 } else { 0 }
                    | if flags & 4 != 0 { 0x20000 } else { 0 },
                filter_info: resources.filter_info,
                material_id: resources.body_material,
                motion_type: 2,
                motion_properties_id: property,
                position: [frame.position[0], frame.position[1], frame.position[2], 0.0],
                orientation: crate::physworld::rotation_quaternion(basis),
                ..Default::default()
            };
            let component = rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?;
            component.begin_create(
                handle,
                resources.shape,
                resources.extra,
                info,
                resources.materials,
                &mut world.geometry,
            )?;
            component.resolve_creation_mass(&world.geometry, density)?;
        }
        if self.rigid_component(handle)?.creation_pending() {
            if let Some(info) = self.rigid_component(handle)?.creation_request()? {
                world
                    .storage
                    .begin_disabled_component_body(handle, generation, info)?;
                rigid_pool_mut!(self, handle.class_id)?
                    .get_mut(handle.slot)
                    .ok_or("rigid component slot is absent")?
                    .creation_entered();
            }
            let body = {
                scene_publication_events!(self, registry, events);
                world.storage.resume_disabled_component_body(
                    &mut world.geometry,
                    &mut events,
                    handle,
                    generation,
                )?
            };
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .creation_completed(body)?;
            let transform = world
                .storage
                .bodies
                .get(body)
                .ok_or("created rigid body absent")?
                .transform;
            self.initial_rigid_frames.insert(
                index,
                crate::gameplay::reference_frame::ReferenceFrame {
                    position: [transform[3][0], transform[3][1], transform[3][2]],
                    basis: std::array::from_fn(|row| {
                        [transform[row][0], transform[row][1], transform[row][2]]
                    }),
                },
            );
        }
        let component = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?;
        if let Some(body) = component.body() {
            world.storage.register_body_materials(body, component.material_table_lease())?;
        }
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

    ///807f80 is the actual virtual78 failed-Initialize callback after the
    ///NoRenderModel/NoPhysicsShape self-unlink.7ea340 has already called7e7c00, canceling
    ///the pending Start before48ff20 destroys this unused pool instance.
    pub(super) fn release_missing_shape_component(&mut self,handle:ComponentHandle,
        registry:&mut ComponentRegistry)->Result<bool,String>{
        if !self.is_rigid_class(handle.class_id){return Ok(false);}
        let component=self.rigid_component(handle)?;
        if !component.missing_physics_shape(){return Ok(false);}
        if component.owner.entity.is_some()||component.owner.declaration.is_some()||component.owner.flags&3!=0{
            return Err("missing-shape rigid failed to self-unlink".into());
        }
        if registry.pending_start.values().contains(&handle.packed())||registry.pending_stop.values().contains(&handle.packed())
            ||registry.pending_release.values().contains(&handle.packed()){
            return Err("missing-shape rigid retains lifecycle work before pool release".into());
        }
        registry.ensure_no_update_references(handle)?;
        if !component.can_release(){return Err("missing-shape rigid acquired live resources".into());}
        let movement=component.movement_body()?;
        let body=self.actors.bodies.get(movement).ok_or("missing-shape rigid interface absent")?;
        if body.actor.is_some()||body.body_id&0x00ff_ffff!=0x00ff_ffff
            ||self.actors.actors.iter().any(|actor|actor.current_body==movement||actor.default_body==movement)
            ||self.free_rigid_movement.contains(&movement){
            return Err("missing-shape rigid interface remains referenced".into());
        }
        self.free_rigid_movement.try_reserve(1).map_err(|error|format!("missing-shape rigid interface release: {error}"))?;
        self.rigid_pool_mut(handle.class_id)?.release(handle.slot)?;
        self.free_rigid_movement.push(movement);
        self.declarations.remove(&handle.packed());
        Ok(true)
    }

    pub(super) fn checked_rigid_component(
        &self,
        handle: ComponentHandle,
        generation: u32,
    ) -> Result<&super::super::rigid_body::RigidBodyComponent, String> {
        let component = self.rigid_component(handle)?;
        if component.owner.generation != generation {
            return Err("RigidBody generation is stale".into());
        }
        Ok(component)
    }

    pub(super) fn prepare_document_rigids(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        interactions: &super::super::interactions::Interactions,
        models: &[entities::PlacedModel],
        texts: &mut crate::assets::declaration_texts::PhysicsSeed,
        sources: &mut crate::physworld::source::ShapeSources,
        registry: &mut ComponentRegistry,
        live: &mut crate::gameplay::entity_lifetime::LiveEntities,
        constructor_tag: crate::gameplay::name_pool::NameId,
    ) -> Result<(), String> {
        let mut animated_models = crate::assets::bmd6::ModelSources::new(catalog);
        let mut blueprint_models = crate::assets::entities::blueprint::Sources::new(catalog);
        let model_indices: HashMap<_, _> = models
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.as_str(), i))
            .collect();
        let definitions: HashMap<_, _> = document
            .definitions
            .iter()
            .filter(|definition| definition.kind == "entity")
            .map(|definition| (definition.id.as_str(), definition))
            .collect();
        let mut indices: HashMap<_, _> = self
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| (entity.name.clone(), index))
            .collect();
        let names: HashMap<_, _> = self
            .types
            .classes()
            .hierarchy()
            .registration_order()
            .iter()
            .map(|&index| {
                let metadata = self
                    .types
                    .classes()
                    .metadata(index)
                    .ok_or("rigid entity class metadata absent")?;
                Ok((
                    self.types
                        .classes()
                        .name_text(metadata.name)
                        .ok_or("rigid entity class name absent")?
                        .to_vec(),
                    index,
                ))
            })
            .collect::<Result<_, String>>()?;
        let mut projections = [0usize; 2];
        let mut materials = super::super::material_loading::MaterialCache::default();
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        for mut binding in entities::rigid_body::bindings(catalog, document)? {
            // The admitted readable and rigid component borrow the same
            // decoded declaration when both refer to the same source entry.
            if let entities::rigid_body::BodyDefinition::Rigid(value) = &mut binding.definition {
                if let Some(shared) = interactions
                    .readable
                    .iter()
                    .find(|entry| entry.entity == binding.entity)
                    .and_then(|entry| entry.rigid.as_ref())
                {
                    *value = shared.clone();
                }
            }
            let definition = binding.definition.rigid();
            (|| -> Result<(), String> {
                let authored = definitions.get(binding.entity.as_str())
                    .ok_or_else(|| format!("placed rigid {}: authored entity definition absent", binding.entity))?;
                let index = if let Some(&index) = indices.get(&binding.entity) { index } else {
                    let class = authored.fields.iter().find_map(|(key, value)| match (key.as_str(), value) {
                        ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')), _ => None,
                    }).ok_or("rigid entity class absent")?;
                    let class = *names.get(class.as_bytes()).ok_or("rigid entity class is not registered")?;
                    let pose = entities::spawn_frame(authored)?;
                    let index = self.entities.len();
                    let shared = live.allocate_managed(Arc::from(binding.entity.as_str()), class, None, 0, constructor_tag)?;
                    self.entities.push(PhysicsEntity { retired: false, external: false, name: binding.entity.clone(), shared,
                        components: EntityComponents::new(index), class, environment: None, flags: 0,
                        entity_flags: 0x001e_8008, entity_flags_d7: 0, priority: 0,
                        frame: crate::gameplay::reference_frame::PublishedFrame { pose,
                            ..crate::gameplay::reference_frame::ReferenceFrameCache::default().published() }, timespace: 0 });
                    self.entity_flags_d4.push(0);
                    indices.insert(binding.entity.clone(), index);
                    index
                };
                for packed in self.entities[index].components.entries.values() {
                    let handle = ComponentHandle::from_packed(*packed)?;
                    if self.is_rigid_class(handle.class_id) {
                        return Err(format!("{}: placed rigid component already admitted", binding.entity));
                    }
                }
                let source_index = model_indices.get(binding.entity.as_str()).copied();
                //808ea0 consumes the RenderModel name and scale, not a color draw.
                // Hidden entities keep that authored descriptor without allocating
                // a mesh/material/GPU projection merely to acquire their bodies.
                let descriptor = if source_index.is_none() {
                    entities::model_descriptor(authored, |path| catalog.read(path)
                        .map_err(|error| error.to_string()).map(|bytes| bytes.map(entities::owned_declaration_text)))?
                } else { None };
                let model = source_index.map(|index| &models[index]).or(descriptor.as_ref());
                let hidden = entities::entity_hidden(authored);
                if hidden { self.entities[index].entity_flags |= 1; }
                let settings = &definition.settings;
                let resources = if let Some(model) = model {
                    let _behavior = settings.physics_behavior.as_deref().and_then(entities::rigid_body_behavior_kind)
                        .ok_or("document rigid behavior absent")?;
                    //809d80 retains the authored dynamic/asleep state. The shared
                    //097bc0/c8e0a0/099a00 driver now owns active-body simulation.
                    let render = if settings.material.is_none() && model.resource_path.starts_with("generated/model/") {
                        crate::assets::bmodel::acquire(catalog, &model.resource_path, crate::assets::bmodel::CommonModels::shared(catalog))?
                    } else { None };
                    let animated_render = if settings.material.is_none() && model.resource_path.starts_with("generated/basemodel/") {
                        Some(animated_models.acquire(catalog, &model.resource_path)?)
                    } else { None };
                    let blueprint_render = if settings.material.is_none() && model.resource_path.starts_with("generated/decls/blueprint/") {
                        Some(blueprint_models.material_paths(model)?)
                    } else { None };
                    let render_surfaces = render.as_deref().map(super::super::rigid_body::MaterialSurfaces::Rigid)
                        .or_else(|| animated_render.as_deref().map(super::super::rigid_body::MaterialSurfaces::Animated))
                        .or_else(|| blueprint_render.as_ref().map(super::super::rigid_body::MaterialSurfaces::Blueprint));
                    if !world.storage.materials.library.added.is_empty() {
                        return Err("rigid resource preparation requires actual material observer host".into());
                    }
                    super::super::rigid_body::prepare_draft_with_surfaces(catalog, sources, texts,
                        settings, model, render_surfaces, Some(&mut materials))
                        .and_then(|draft| draft.map(|draft| draft.publish(&mut world.storage.materials,
                            |_,_,_,_| unreachable!("startup material observers absent"))).transpose())
                        .map_err(|error| format!("placed rigid {} [{}], model={}, hidden={hidden}, color_projection={source_index:?}: {error}",
                            binding.entity, settings.declaration_id, model.resource_path))?
                } else {
                    //808ef9 joins the same virtual10 self-unlink path as
                    //NoPhysicsShape. Do not invent a render model or body.
                    None
                };
                let class = match &binding.definition { entities::rigid_body::BodyDefinition::Rigid(_) => 8,
                    entities::rigid_body::BodyDefinition::Throwable(_) => 12 };
                let declaration = self.types.declaration(self.factories[class], Arc::as_ptr(definition) as usize,
                    definition.scheduling.as_ref().clone())?;
                let bodies = &mut self.actors.bodies;
                let free = &mut self.free_rigid_movement;
                let pool = match class { 8 => &mut self.rigid_bodies, 12 => &mut self.throwables, _ => unreachable!() };
                let handle = ComponentFactory { declaration: &declaration, pool,
                    registry, grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                        declaration.scheduling.starts_on_spawn(), |_,_,_| {
                            let mut component = match &binding.definition {
                                entities::rigid_body::BodyDefinition::Rigid(value) => super::super::rigid_body::RigidBodyComponent::new(value.clone(), bodies, free)?,
                                entities::rigid_body::BodyDefinition::Throwable(value) => super::super::rigid_body::RigidBodyComponent::new_throwable(value.clone(), bodies, free)?,
                            };
                            component.bind_prepared_shape(source_index, resources)?;
                            Ok(component)
                        }, |_,_,_| Ok(()))?.ok_or("document rigid creation disabled")?;
                if let Some(source_index) = source_index { self.rigid_placements.insert(index, source_index); }
                self.declarations.insert(handle.packed(), declaration);
                projections[usize::from(source_index.is_some())] += 1;
                Ok(())
            })().map_err(|error| format!("placed rigid entity={} declaration={}: {error}",
                binding.entity, definition.settings.declaration_id))?;
        }
        materials.finish_body_physics_preparation();
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!(
                "RIGID_RESOURCES phase=prepared descriptor_only={} color_projection={}",
                projections[0], projections[1]
            );
        }
        Ok(())
    }

    pub(super) fn start_placed_rigid(
        &mut self,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents,
    ) -> Result<(), String> {
        let generation = self.owner(handle)?.generation;
        let component = self.checked_rigid_component(handle, generation)?;
        let index = component.owner.entity.ok_or("rigid entity absent")?;
        let entity = &self.entities[index];
        let id = entity.shared;
        self.validate_document_hierarchy(live, index)?;
        let definition = component.definition().ok_or("rigid definition absent")?;
        let authored_behavior = definition
            .settings
            .physics_behavior
            .as_deref()
            .and_then(entities::rigid_body_behavior_kind)
            .ok_or("rigid behavior absent")?;
        let asleep = definition
            .settings
            .start_asleep
            .ok_or("rigid startAsleep absent")?;
        let body = component.body().ok_or("rigid body absent")?;
        let movement = component.movement_body()?;
        let behavior = match component.start_behavior {
            Some(value) => value,
            None => self.bound_document_behavior(live, index, authored_behavior)?,
        };
        let pose = entity.frame.pose;
        let basis = crate::physworld::rigid_body_basis(pose.basis.map(|v| [v[0], v[1], v[2], 0.]));
        let frame = [
            basis[0],
            basis[1],
            basis[2],
            [pose.position[0], pose.position[1], pose.position[2], 0.],
        ];
        if !component.start_entered() {
            let scheduling = self
                .declarations
                .get(&handle.packed())
                .ok_or("rigid scheduling absent")?
                .scheduling
                .clone();
            let priority = entity.priority;
            let flags = &mut rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .owner
                .flags;
            if *flags & (1 << 30) != 0 {
                *flags &= 0xf800003f;
            }
            if crate::gameplay::component_state::prepare_admission(
                flags,
                u8::from(scheduling.priority < 8),
                scheduling.priority,
                priority,
            ) {
                self.rigid_pool_mut(handle.class_id)?
                    .schedule(handle.slot)?;
            }
        }
        let actor = self.reference_actor(index, registry)?;
        if self.actors.bodies[movement].actor != Some(usize::from(actor.slot))
            || self.actors.actors[usize::from(actor.slot)].current_body != movement {
            self.actors.bodies[movement].body_id = body;
            //751050 must select the interface again after a finite Projectile
            //handoff, even when its retained reciprocal owner already matches.
            let updates = &self.pre_updates;
            self.actors.attach_body(
                usize::from(actor.slot),
                movement,
                self.pre_frame,
                |actors, actor| {
                    updates.updates.enqueue(crate::gameplay::physics_world::update_queue::ObjectReference::capture(
                        Some((actors.actors[actor].owner.generation,actor))))?;
                    Ok(())
                },
            )?;
        }
        let activation = if self.owner(handle)?.flags & 0x30 > 0x1f || asleep {
            crate::gameplay::body_storage::BodyActivation::KeepDeactivated
        } else {
            crate::gameplay::body_storage::BodyActivation::Activate
        };
        let step = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?
            .start_publication_step(frame, activation)?;
        rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?
            .start_behavior = Some(behavior);
        if step == 4 {
            return Ok(());
        }
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step == 0 {
            {
                scene_publication_events!(self, registry, events);
                world.storage.set_body_transform(
                    &mut world.geometry,
                    &mut events,
                    body,
                    frame,
                    0,
                )?;
            }
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .advance_start_publication()?;
        }
        if step <= 1 {
            if let Some(quality) = self.rigid_component(handle)?.start_quality() {
                scene_publication_events!(self, registry, events);
                //809d80: c87700(body,7,0) follows c85860 and must finish
                //before c8e930 publishes this same body.
                world.storage.set_body_quality(
                    &mut world.geometry, &mut events, body, quality, 0,
                )?;
            }
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .advance_start_publication()?;
        }
        if step <= 2 {
            {
                scene_publication_events!(self, registry, events);
                world.storage.add_dynamic_body_resumable(
                    &mut world.geometry,
                    &mut events,
                    body,
                    activation,
                )?;
            }
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .advance_start_publication()?;
        }
        {
            scene_publication_events!(self, registry, events);
            match behavior {
                0 => world
                    .storage
                    .static_single_body(&mut world.geometry, &mut events, body)?,
                1 => world
                    .storage
                    .keyframe_single_body(&mut world.geometry, &mut events, body)?,
                // 727a00 compares the current body type with the declaration.
                // BodyInfo created this body dynamic already: equality returns
                // without a c86800 motion conversion (including sleeping loads).
                2 => {}
                _ => return Err("rigid body behavior is invalid".into()),
            }
        }
        let flags = world
            .storage
            .bodies
            .get(body)
            .ok_or("rigid body retired")?
            .flags;
        let component = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?;
        component.on_body_activation(
            false,
            flags,
            &mut self.entity_flags_d4[index],
            registry,
            || Err("inactive rigid callback requested hierarchy update".into()),
        )?;
        if !component.activation_listener_registered() {
            component.register_activation_listener(&mut self.body_activation_events)?;
        }
        if !component.start_listeners_registered() {
            let generation = component.owner.generation;
            component.register_start_listeners(entity_events, self.owner_id, id, false, || {
                if !self.rigid_wind_bodies.contains(&(handle,generation)) {
                    self.rigid_wind_bodies.try_reserve(1).map_err(|error|format!("Wind body registration: {error}"))?;
                    self.rigid_wind_bodies.push((handle,generation));
                }
                Ok(())
            })?;
        }
        component.finish_start(|| Ok(()))
    }

    pub(super) fn stop_placed_rigid(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents,
    ) -> Result<(), String> {
        let body = self
            .rigid_component(handle)?
            .body()
            .ok_or("rigid body absent")?;
        let step = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?
            .stop_external_step()?;
        if step == 3 {
            return Ok(());
        }
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step == 0 {
            world.storage.preflight_keyframe_single_body(body)?;
            let component = rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?;
            let generation = component.owner.generation;
            component.unregister_start_listeners(entity_events, false, || {
                if let Some(index)=self.rigid_wind_bodies.iter().position(|entry|*entry==(handle,generation)) {
                    self.rigid_wind_bodies.swap_remove(index);
                }
                Ok(())
            })?;
            component.unregister_activation_listener(&mut self.body_activation_events)?;
            component.advance_external_stop()?;
        }
        if step <= 1 {
            {
                scene_publication_events!(self, registry, events);
                world
                    .storage
                    .unpublish_bodies(&mut world.geometry, &mut events, &[body], 0)?;
            }
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .advance_external_stop()?;
        }
        let component = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?;
        let movement = component.movement_body()?;
        if let Some(actor) = self.actors.bodies[movement].actor {
            self.actors.detach_body(actor, movement)?;
        }
        component.clear_body_update_pause();
        component.advance_external_stop()?;
        Ok(())
    }

    pub(super) fn deinitialize_placed_rigid(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut crate::gameplay::entity_events::EntityEvents,
    ) -> Result<(), String> {
        let step = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?
            .deinitialize_external_step()?;
        if step == 4 {
            return Ok(());
        }
        if step == 0 {
            let component = rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?;
            component.unregister_initialize_listeners(entity_events, false)?;
            component.advance_external_deinitialize()?;
        }
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "level physics query owner is poisoned")?;
        let world = &mut *guard;
        if step <= 1 {
            let body = self
                .rigid_component(handle)?
                .body()
                .ok_or("rigid body absent")?;
            {
                scene_publication_events!(self, registry, events);
                world
                    .storage
                    .destroy_bodies(&mut world.geometry, &mut events, &[body], 0)?;
            }
            rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?
                .advance_external_deinitialize()?;
        }
        if step <= 2 {
            let component = rigid_pool_mut!(self, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?;
            let index = component.owner.entity.ok_or("rigid entity absent")?;
            self.entities[index].components.release(handle, component)?;
            component.advance_external_deinitialize()?;
        }
        rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?
            .retire_external_geometry(&mut world.geometry)
    }
}
