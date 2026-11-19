//! Projectile finite-frame actor hooks. One canonical component/body registry.
use super::*;
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    component_state::ComponentInstance,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities, Stage},
    pickable_component::CheckedComponent,
    projectiles::{self, Contact, ContactFlags, Handoff, Hit, SpawnRequest},
    reference_frame::ReferenceFrame,
    shape_ray_query::ShapeRayContact,
};
use bevy_math::Vec3;

pub(in crate::app) struct ProjectileStep {
    pub entity: EntityId,
    pub frame: ReferenceFrame,
    pub velocity: Vec3,
    pub hidden: bool,
    pub retire: bool,
    pub handoff: Handoff,
    pub witness: Option<ShapeRayContact>,
    pub contact_entity: Option<EntityId>,
    /// Bounce FA point impulse goes to the projectile; ordinary contact goes
    ///to the witnessed target body (629590/623540 versus623af0).
    pub impulse: Option<Vec3>,
    pub contact_impulse: Option<Vec3>,
    pub response: projectiles::Response,
    pub publish_motion: bool,
    pub remove_when_hidden: bool,
    pub impact_fx: Option<Arc<str>>,
    pub hit_direction: Vec3,
    pub stop_component: bool,
    pub suppress_pickable: bool,
    pub contact_effects: Vec<ShapeRayContact>,
    pub contact_speed: f32,
    pub contact_mass: f32,
    pub contact_stimulus_enabled: bool,
}
impl PhysicsScene {
    ///627110/6272e0 register/withdraw normal numeric14c with810810/811160.
    ///The producer4d86c1 allocates exactly32 bytes with8112c0; this is not
    ///the separate named-event/variant dispatcher used by pickedUp.
    pub(super) fn projectile_message(
        &mut self,
        live: &LiveEntities,
        registry: &mut ComponentRegistry,
        listener: crate::gameplay::entity_events::Listener,
        event: &crate::gameplay::entity_events::Event,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
    ) -> Result<(), String> {
        use crate::gameplay::entity_events::{Listener, Payload};
        let Listener::Component {
            owner,
            handle,
            generation,
            handler,
        } = listener
        else {
            return Err("projectile listener kind differs".into());
        };
        if owner != self.owner_id
            || handle.class_id != self.classes[13]
            || handler != 0x14c
            || event.id != handler
        {
            return Err("projectile deflection callback owner differs".into());
        }
        let component = self
            .projectiles
            .get(handle.slot)
            .ok_or("projectile deflection payload absent")?;
        if component.owner.generation != generation || !component.deflect_listener {
            return Err("projectile deflection listener generation stale".into());
        }
        let index = component
            .owner
            .entity
            .ok_or("projectile deflection actor absent")?;
        let id = self.entities[index].shared;
        if event.target != Some(id) || live.get(id).is_none() {
            return Err("projectile deflection target stale".into());
        }
        let Payload::ProjectileDeflection {
            linear,
            angular,
            keep_finite,
            culprit,
        } = &event.payload
        else {
            return Err("projectile deflection requires its actual typed32-byte payload".into());
        };
        //626ff0 always posts both priority2 velocity requests before the
        //optional physical handoff. The shared actor owns their lifetimes.
        self.request_actor_velocity(id, *linear, *angular)?;
        let transfer = {
            let runtime = self
                .projectiles
                .get_mut(handle.slot)
                .and_then(|component| component.runtime.as_mut())
                .ok_or("projectile deflection runtime absent")?;
            let transfer = !*keep_finite && !runtime.impact_stopped && !runtime.picked_up;
            if transfer {
                runtime.impact_stopped = true;
            }
            transfer
        };
        if transfer {
            let checked = self
                .projectiles
                .get(handle.slot)
                .unwrap()
                .throwable
                .ok_or("deflection checked Throwable absent")?;
            let throwable = self.rigid_component(checked.handle)?;
            if throwable.owner.generation != checked.generation {
                return Err("deflection Throwable generation stale".into());
            }
            if throwable.owner.flags >> 2 & 3 != 2 {
                self.detach_projectile_motion(handle)?;
                registry.request_start(
                    &mut self.rigid_component_mut(checked.handle)?.owner,
                    checked.handle.packed() as usize,
                    false,
                    |_, _| Ok(()),
                )?;
            }
            //Throwable vtable141b64b68+188=727880. Bound entities retain
            //their parent's motion type; unbound bodies use actual dynamic2.
            if self.entities[index].entity_flags & 0x80 == 0 {
                let body = self
                    .rigid_component(checked.handle)?
                    .body()
                    .ok_or("deflection body absent")?;
                self.rigid_component_mut(checked.handle)?.start_behavior = Some(2);
                let shared = self.world.clone();
                let mut guard = shared
                    .write()
                    .map_err(|_| "deflection body world poisoned")?;
                let world = &mut *guard;
                scene_publication_events!(self, registry, publication);
                world
                    .storage
                    .dynamic_single_body(&mut world.geometry, &mut publication, body)?;
            }
        }
        if let Some(culprit) = *culprit {
            let clock = clock
                .ok_or("projectile deflection instigator requires the canonical entity clock")?;
            self.set_rigid_instigator(
                live,
                id,
                culprit,
                clock.entity_time(id).map_err(str::to_owned)?,
                clock.entity_delta(id).map_err(str::to_owned)?,
            )?;
        }
        Ok(())
    }
    /// Common3b8430/76e960 selection reference, borrowed from the real pool.
    /// Runtime pickups do not create another interaction definition/item.
    pub(in crate::app) fn interaction_pickable(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<
        Option<(
            CheckedComponent,
            &crate::gameplay::pickable_component::PickableComponent,
        )>,
        String,
    > {
        if live.stage(id) != Some(Stage::Spawned) {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let entity = &self.entities[index];
        if entity.retired || entity.entity_flags & 1 != 0 {
            return Ok(None);
        }
        let handle =
            entity
                .components
                .find_component(self.pickable_range(), [0x28, 0], |handle| {
                    Ok(
                        crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                            flags: self.owner(handle)?.flags,
                            start_phases: self
                                .declarations
                                .get(&handle.packed())
                                .map(|declaration| declaration.scheduling.start_phases),
                        },
                    )
                })?;
        let Some(handle) = handle else {
            return Ok(None);
        };
        let component = self.pickable(handle)
            .ok_or("interaction Pickable slot absent")?;
        if !component.interaction_enabled
            || component.interaction_flags & 0xa != 0
            || !component.definition.usable_by_player
        {
            return Ok(None);
        }
        Ok(Some((
            CheckedComponent {
                handle,
                generation: component.owner.generation,
            },
            component,
        )))
    }
    ///624540 tests the checked Throwable's requested state before7e7b10.
    pub(in crate::app) fn projectile_throwable_started(&self, handle: ComponentHandle) -> Result<bool, String> {
        let checked = self.projectiles.get(handle.slot).and_then(|component| component.throwable)
            .ok_or("projectile checked Throwable absent")?;
        let owner = self.owner(checked.handle)?;
        if owner.generation != checked.generation { return Err("projectile Throwable generation stale".into()); }
        Ok(owner.flags >> 2 & 3 == 2)
    }
    ///628df0 snapshots children in reverse binding-list order using the
    ///parent's explicit impact/pickup matrix and their actual local offsets.
    pub(in crate::app) fn projectile_child_frames(&self, live: &LiveEntities, id: EntityId,
        frame: ReferenceFrame) -> Result<super::super::projectiles::Pickup, String> {
        let &index = self.shared_indices.get(&id).ok_or("projectile child frame owner absent")?;
        let mut children = Vec::new();
        children.try_reserve(live.children(id)?.len()).map_err(|error| format!("projectile children: {error}"))?;
        for &child in live.children(id)?.iter().rev() {
            let &child_index = self.shared_indices.get(&child).ok_or("projectile child header absent")?;
            let actor = self.entities[child_index].components.reference_actor
                .and_then(|handle| self.actors.actors.get(handle.slot))
                .ok_or("projectile child reference actor absent")?;
            if actor.binding.entity != Some(index) { return Err("projectile child binding differs from live parent".into()); }
            let local = crate::gameplay::reference_frame::joints::bind_local_frame(
                ReferenceFrame::default(), [1.; 3], actor.binding.offset, actor.binding.orientation);
            children.push(super::super::projectiles::PickupChild {
                entity: child, frame: crate::gameplay::reference_frame::compose(frame, local), unbound: false,
            });
        }
        Ok(super::super::projectiles::Pickup { children, next: 0 })
    }
    ///628df0 uses the same child publication for pickup(detach=true) and
    ///the newly started stuck body(detach=false). The caller retains this
    ///cursor across a fallible callback, so delivered children are not replayed.
    pub(in crate::app) fn publish_projectile_children(&mut self, live: &mut LiveEntities,
        events: &mut EntityEvents, pending: &mut super::super::projectiles::Pickup,
        detach: bool) -> Result<(), String> {
        while let Some(child) = pending.children.get_mut(pending.next) {
            if detach && !child.unbound {
                self.unbind_entity(live, child.entity, events)?;
                child.unbound = true;
            }
            let index = *self.shared_indices.get(&child.entity).ok_or("projectile child publication owner absent")?;
            if self.document_rigid(index)?.is_some() {
                let current = self.published_entity_frame(live, child.entity)?;
                self.publish_rigid_motion(live, child.entity, child.frame, current.velocities[0], current.velocities[1])?;
            } else {
                let actor = self.entities[index].components.reference_actor.ok_or("projectile child actor absent")?;
                self.actors.actors[usize::from(actor.slot)].set_transform(child.frame, false);
                self.entities[index].frame.pose = child.frame;
                let generation = self.actors.actors[usize::from(actor.slot)].owner.generation;
                self.request_post_physics_actor(usize::from(actor.slot), generation)?;
            }
            pending.next += 1;
        }
        Ok(())
    }
    ///76e2b0 ->6274d0: pickup transfers a real finite projectile back to its
    ///Throwable. Child transforms use628df0's retained actor binding offsets;
    ///the same attachment owner restores body modes and reciprocal links.
    pub(in crate::app) fn begin_projectile_pickup(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
    ) -> Result<(), String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("projectile pickup actor absent")?;
        let handle = self.entities[index].components.find_component(
            [self.classes[13]; 2],
            [0x20, 0],
            |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|declaration| declaration.scheduling.start_phases),
                    },
                )
            },
        )?;
        let Some(handle) = handle else {
            return Ok(());
        };
        if self
            .projectiles
            .get(handle.slot)
            .ok_or("picked projectile payload absent")?
            .pickup
            .is_none()
        {
            let runtime = self
                .projectiles
                .get(handle.slot)
                .and_then(|component| component.runtime.as_ref())
                .ok_or("picked projectile runtime absent")?;
            if runtime.picked_up {
                return Ok(());
            }
            //76e2b0 detaches the picked entity's own parent before6274d0.
            self.unbind_entity(live, id, events)?;
            //6274d0 passes the entity's published +300/+30c words, not an
            //actor prediction which has not yet been committed to that entity.
            let frame = self.entities[index].frame.pose;
            let pending = self.projectile_child_frames(live, id, frame)?;
            let component = self.projectiles.get_mut(handle.slot).unwrap();
            component.runtime.as_mut().unwrap().picked_up = true;
            component.pickup = Some(pending);
        }
        let mut pending = self.projectiles.get_mut(handle.slot).unwrap().pickup.take()
            .ok_or("projectile pickup child cursor absent")?;
        let result = self.publish_projectile_children(live, events, &mut pending, true);
        self.projectiles.get_mut(handle.slot).unwrap().pickup = Some(pending);
        result?;
        let throwable = self
            .projectiles
            .get(handle.slot)
            .unwrap()
            .throwable
            .ok_or("picked projectile checked Throwable absent")?;
        let throwable_owner = self.owner(throwable.handle)?;
        if throwable_owner.generation != throwable.generation {
            return Err("picked projectile Throwable generation stale".into());
        }
        if self.owner(handle)?.flags & 0xc > 7 && throwable_owner.flags >> 2 & 3 != 2 {
            self.detach_projectile_motion(handle)?;
            let mut registry = self
                .registry
                .take()
                .ok_or("projectile pickup registry absent")?;
            let result = (|| {
                registry.request_start(
                    &mut self.rigid_component_mut(throwable.handle)?.owner,
                    throwable.handle.packed() as usize,
                    false,
                    |_, _| Ok(()),
                )
            })();
            self.registry = Some(registry);
            result?;
        }
        self.projectiles.get_mut(handle.slot).unwrap().pickup = None;
        Ok(())
    }
    ///627f60 tests the actual Pickable_Simple header after ThrowableStick.
    ///An absent or stopped component is not a recoverable sleeping dart.
    pub(in crate::app) fn projectile_recoverable(&self, id: EntityId) -> Result<bool, String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("projectile recovery actor absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("projectile recovery actor slot absent")?;
        if entity.retired {
            return Err("projectile recovery actor retired".into());
        }
        let handle =
            entity
                .components
                .find_component(self.pickable_range(), [0x28, 0], |handle| {
                    let component = self.pickable(handle)
                        .ok_or("projectile Pickable header absent")?;
                    Ok(
                        crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                            flags: component.owner.flags,
                            start_phases: self
                                .declarations
                                .get(&handle.packed())
                                .map(|decl| decl.scheduling.start_phases),
                        },
                    )
                })?;
        Ok(handle
            .and_then(|handle| self.pickable(handle))
            .is_some_and(|component| component.owner.flags & 0xc != 0xc))
    }
    fn projectile_contact_parameters(
        &self,
        handle: ComponentHandle,
    ) -> Result<(f32, bool), String> {
        let checked = self
            .projectiles
            .get(handle.slot)
            .and_then(|component| component.throwable)
            .ok_or("projectile source Throwable reference absent")?;
        let component = self.rigid_component(checked.handle)?;
        if component.owner.generation != checked.generation {
            return Err("projectile source Throwable generation stale".into());
        }
        let id = component.body().ok_or("projectile source body absent")?;
        let world = self
            .world
            .read()
            .map_err(|_| "projectile source mass world poisoned")?;
        let body = world
            .storage
            .bodies
            .get(id)
            .ok_or("projectile source body generation stale")?;
        //092260 checks source-body broadphase admission before reading the
        //fourth packed inverse-mass lane. A dormant finite body returns zero.
        if body.broadphase_id == u32::MAX {
            return Ok((0., body.flags & (1 << 19) != 0));
        }
        let motion = world
            .storage
            .motions
            .get(body.motion_id)
            .ok_or("projectile source motion absent")?;
        let inverse = f32::from_bits(u32::from(motion.inverse_mass[3]) << 16);
        let mass = if inverse == 0. {
            0.
        } else {
            crate::gameplay::constraint_solver::reciprocal(inverse)
        };
        Ok((mass, body.flags & (1 << 19) != 0))
    }
    pub(in crate::app) fn suppress_projectile_pickable(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
    ) -> Result<(), String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("projectile recovery actor absent")?;
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|value| ComponentHandle::from_packed(*value).ok())
            .find(|handle| self.is_pickable_class(handle.class_id));
        let Some(handle) = handle else { return Ok(()) };
        let mut registry = self
            .registry
            .take()
            .ok_or("projectile recovery registry absent")?;
        let result = registry.request_stop(
            handle,
            &mut interaction_components::Startup {
                world: None,
                scene: self,
                live,
                events,
            },
        );
        self.registry = Some(registry);
        result
    }
    fn projectile_hit_region(&self, hit: ShapeRayContact) -> Result<u32, String> {
        self.collision_hit_region(hit.target)
    }
    pub(super) fn collision_hit_region(&self, hit: crate::gameplay::collision_hit::ShapeHitInfo) -> Result<u32, String> {
        let handle = CollisionUserData::decode(hit.user_data)?.component;
        //Collision-data virtual+30 is06d040 (returns NONE0) for all
        //admitted payloads: Rigid141c74578, Trigger141c5ba98,
        //PhysicsWorld141c7ff38. NPC limb packets require their real owner.
        if self.is_rigid_class(handle.class_id)
            || handle.class_id == self.classes[0]
            || handle.class_id == self.classes[14]
        {
            Ok(0)
        } else {
            Err("projectile collision region has no actual admitted callback".into())
        }
    }
    pub(in crate::app) fn request_projectile_stop(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile Stop request class mismatch".into());
        }
        let mut registry = self
            .registry
            .take()
            .ok_or("projectile Stop registry absent")?;
        let result = registry.request_stop(
            handle,
            &mut interaction_components::Startup {
                world: None,
                scene: self,
                live,
                events,
            },
        );
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn projectile_effect_context(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<crate::rendering::bevy_backend::world_effects::EntityEffectContext, String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("projectile FX actor absent")?;
        let entity = &self.entities[index];
        let actor = entity
            .components
            .reference_actor
            .ok_or("projectile FX reference actor absent")?;
        let frame = self
            .actors
            .actors
            .get(actor.slot)
            .ok_or("projectile FX frame absent")?
            .frame
            .published();
        Ok(
            crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                owner: Some(id),
                culprit: live.owner(id)?,
                velocities: frame.velocities,
                contact_speed: 0.,
                contact_mass: 0.,
                contact_stimulus_enabled: None,
                cannot_incapacitate_player_and_allies: self.entity_flags_d4[index] & 1 != 0,
                player_impact: false,
            },
        )
    }
    ///6262d0's +34/+35 are Trigger+c0's Water dependency. Registration
    ///1413de760 names that RTTI range arkCpntTrigger; it is not RigidBody.
    fn projectile_water_state(&self, hit: ShapeRayContact) -> Result<Option<bool>, String> {
        if hit.target.user_data == 0 {
            return Ok(None);
        }
        let component = CollisionUserData::decode(hit.target.user_data)?.component;
        if component.class_id != self.classes[0] {
            return Ok(None);
        }
        let trigger = self
            .triggers
            .get(component.slot)
            .ok_or("projectile trigger payload absent")?;
        trigger
            .water
            .map(|water| Ok(self.owner(water)?.flags & 0xc > 7))
            .transpose()
    }
    ///627110 installs the actual embedded reference-motion interface. Its
    ///six virtuals at141b9c910 match the shared BasicBody implementation.
    pub(super) fn start_projectile(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile Start class mismatch".into());
        }
        let component = self
            .projectiles
            .get(handle.slot)
            .ok_or("projectile Start payload absent")?;
        if component.reference_frame_registered {
            return Ok(());
        }
        let index = component
            .owner
            .entity
            .ok_or("projectile Start entity absent")?;
        let movement = component
            .movement_body
            .ok_or("projectile reference interface absent")?;
        let actor = self.entities[index]
            .components
            .reference_actor
            .ok_or("projectile actual reference actor absent")?;
        let updates = &self.pre_updates;
        self.actors.attach_body(
            usize::from(actor.slot),
            movement,
            self.pre_frame,
            |actors, index| {
                updates.updates.enqueue(
                    crate::gameplay::physics_world::update_queue::ObjectReference::capture(Some((
                        actors.actors[index].owner.generation,
                        index,
                    ))),
                )?;
                Ok(())
            },
        )?;
        self.projectiles
            .get_mut(handle.slot)
            .ok_or("projectile Start payload disappeared")?
            .reference_frame_registered = true;
        let component = self.projectiles.get_mut(handle.slot).unwrap();
        if !component.deflect_listener {
            events.register(
                0x14c,
                Some(self.entities[index].shared),
                crate::gameplay::entity_events::Listener::Component {
                    owner: self.owner_id,
                    handle,
                    generation: component.owner.generation,
                    handler: 0x14c,
                },
            );
            component.deflect_listener = true;
        }
        Ok(())
    }
    pub(in crate::app) fn detach_projectile_motion(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile reference detach class mismatch".into());
        }
        let component = self
            .projectiles
            .get(handle.slot)
            .ok_or("projectile reference payload absent")?;
        if !component.reference_frame_registered {
            return Ok(());
        }
        let body = component
            .movement_body
            .ok_or("projectile reference interface absent")?;
        let actor = self
            .actors
            .bodies
            .get(body)
            .ok_or("projectile reference slot absent")?
            .actor
            .ok_or("registered projectile reference actor absent")?;
        self.actors.detach_body(actor, body)?;
        self.projectiles
            .get_mut(handle.slot)
            .ok_or("projectile reference payload disappeared")?
            .reference_frame_registered = false;
        Ok(())
    }
    ///6272e0 detaches finite motion and stops both persistent trail variants.
    ///The app consumes that request through the canonical FX owner; component
    ///runtime remains retained across Stop.
    pub(super) fn stop_projectile(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile Stop payload absent")?;
        if component.deflect_listener {
            let index = component
                .owner
                .entity
                .ok_or("projectile Stop actor absent")?;
            events.unregister(
                0x14c,
                Some(self.entities[index].shared),
                crate::gameplay::entity_events::Listener::Component {
                    owner: self.owner_id,
                    handle,
                    generation: component.owner.generation,
                    handler: 0x14c,
                },
            );
            component.deflect_listener = false;
        }
        self.detach_projectile_motion(handle)?;
        self.projectiles
            .get_mut(handle.slot)
            .ok_or("projectile Stop payload absent")?
            .effects_stop_pending = true;
        Ok(())
    }
    pub(in crate::app) fn acknowledge_projectile_effect_stop(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        self.projectiles
            .get_mut(handle.slot)
            .ok_or("projectile FX Stop payload absent")?
            .effects_stop_pending = false;
        Ok(())
    }
    ///6270e0 releases the query scratch; cached checked dependencies cease
    ///being usable at Deinitialize, before the pool's destructor releases them.
    pub(super) fn deinitialize_projectile(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        self.detach_projectile_motion(handle)?;
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile deinit payload absent")?;
        component.throwable = None;
        component.hits.clear();
        Ok(())
    }
    pub(super) fn release_projectile_interface(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile release payload absent")?;
        if component.reference_frame_registered {
            return Err("projectile reference interface still registered".into());
        }
        if let Some(body) = component.movement_body {
            let slot = self
                .actors
                .bodies
                .get(body)
                .ok_or("projectile release reference slot absent")?;
            if slot.actor.is_some() {
                return Err("projectile reference interface still attached".into());
            }
            self.free_rigid_movement
                .try_reserve(1)
                .map_err(|error| format!("projectile reference slot return: {error}"))?;
            self.free_rigid_movement.push(body);
            component.movement_body = None;
        }
        Ok(())
    }
    pub(in crate::app) fn projectile_audio_emitter(
        &self,
        id: EntityId,
    ) -> Result<Option<crate::gameplay::sound::Emitter>, String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("projectile audio actor absent")?;
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|value| ComponentHandle::from_packed(*value).ok())
            .find(|handle| handle.class_id == self.classes[4]);
        handle
            .map(|handle| {
                let component = self
                    .audios
                    .get(handle.slot)
                    .ok_or("projectile audio payload absent")?;
                Ok(crate::gameplay::sound::Emitter::Component {
                    handle: handle.packed(),
                    generation: component.owner.generation,
                })
            })
            .transpose()
    }
    pub(in crate::app) fn projectile_batch(
        &self,
        out: &mut Vec<(ComponentHandle, EntityId)>,
        effect_stops: &mut Vec<(ComponentHandle, EntityId)>,
    ) -> Result<(), String> {
        out.clear();
        effect_stops.clear();
        for component in self.projectiles.iter() {
            let Some(runtime) = &component.runtime else {
                continue;
            };
            if component.owner.flags & 0xc != 8 && !component.effects_stop_pending {
                continue;
            }
            let index = component
                .owner
                .entity
                .ok_or("initialized projectile lost its actor")?;
            if self.entities[index].retired {
                continue;
            }
            let handle = self.entities[index]
                .components
                .entries
                .values()
                .iter()
                .filter_map(|value| ComponentHandle::from_packed(*value).ok())
                .find(|handle| handle.class_id == self.classes[13])
                .ok_or("initialized projectile lost its component identity")?;
            if component.effects_stop_pending {
                effect_stops.push((handle, runtime.identity));
            }
            if component.owner.flags & 0xc == 8 {
                out.push((handle, runtime.identity));
            }
        }
        Ok(())
    }
    ///Opt-in action diagnostics must retain real stopped projectiles too.
    ///The production update batch intentionally contains only Started actors.
    ///No state is recorded and no component is started by this observation.
    pub(in crate::app) fn diagnostic_projectile_batch(&self,out:&mut Vec<(ComponentHandle,EntityId)>)->Result<(),String>{
        out.clear();
        for component in self.projectiles.iter(){
            let Some(runtime)=&component.runtime else{continue;};
            let index=component.owner.entity.ok_or("observed projectile actor absent")?;
            let entity=self.entities.get(index).ok_or("observed projectile slot absent")?;
            if entity.retired{continue;}
            let handle=entity.components.entries.values().iter()
                .filter_map(|value|ComponentHandle::from_packed(*value).ok())
                .find(|handle|handle.class_id==self.classes[13])
                .ok_or("observed projectile component identity absent")?;
            out.push((handle,runtime.identity));
        }
        Ok(())
    }
    pub(in crate::app) fn projectile_runtime(
        &self,
        handle: ComponentHandle,
    ) -> Result<&projectiles::Projectile, String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile context class mismatch".into());
        }
        self.projectiles
            .get(handle.slot)
            .and_then(|component| component.runtime.as_ref())
            .ok_or_else(|| "projectile context is not initialized".into())
    }
    pub(in crate::app) fn projectile_prepared(
        &self,
        handle: ComponentHandle,
    ) -> Result<Arc<crate::assets::projectiles::Prepared>, String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile source class mismatch".into());
        }
        self.projectiles
            .get(handle.slot)
            .map(|component| component.prepared.clone())
            .ok_or_else(|| "projectile source payload absent".into())
    }
    pub(in crate::app) fn projectile_gravity(&self) -> Result<Vec3, String> {
        let world = self.world.read().map_err(|_| "projectile world poisoned")?;
        Ok(Vec3::from_array(
            world.storage.info.gravity[..3].try_into().unwrap(),
        ))
    }
    ///623af0/8519d0 applies the target body's impulse at the actual ray
    ///witness. This is separate from the projectile's bounce point impulse.
    pub(in crate::app) fn projectile_contact_impulse(
        &mut self,
        witness: ShapeRayContact,
        impulse: Vec3,
    ) -> Result<(), String> {
        let body = witness
            .target
            .body_id
            .ok_or("projectile contact impulse has no actual body")?;
        let mut registry = self
            .registry
            .take()
            .ok_or("projectile impulse registry unavailable")?;
        let result = (|| {
            let shared = self.world.clone();
            scene_publication_events!(self, &mut registry, publication);
            let mut world = shared
                .write()
                .map_err(|_| "projectile impulse world poisoned")?;
            let world = &mut *world;
            world.storage.apply_body_impulse(
                &mut world.geometry,
                &mut publication,
                body,
                [impulse.x, impulse.y, impulse.z, 0.],
                Some(witness.position),
            )
        })();
        self.registry = Some(registry);
        result.map(|_| ())
    }
    pub(in crate::app) fn configure_projectile(
        &mut self,
        id: EntityId,
        request: SpawnRequest,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("projectile actual actor absent")?;
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|v| ComponentHandle::from_packed(*v).ok())
            .find(|h| h.class_id == self.classes[13])
            .ok_or("entity lacks actual Projectile component")?;
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        if component.request.is_some() || component.runtime.is_some() {
            return Err("projectile source already configured".into());
        }
        component.request = Some(request);
        Ok(())
    }
    pub(super) fn initialize_projectile(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile initializer received another actual class".into());
        }
        let index = self
            .projectiles
            .get(handle.slot)
            .ok_or("projectile payload absent")?
            .owner
            .entity
            .ok_or("projectile unlinked")?;
        let throwable = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|v| ComponentHandle::from_packed(*v).ok())
            .find(|h| h.class_id == self.classes[12])
            .ok_or("Projectile Initialize requires actual attached Throwable")?;
        let generation = self.rigid_component(throwable)?.owner.generation;
        let id = self.entities[index].shared;
        let request = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?
            .request
            .take()
            .ok_or("projectile birth has no accepted source request")?;
        let mut npc_launcher = false;
        if let Some(attacker) = request
            .attacker
            .and_then(|id| self.shared_indices.get(&id).copied())
        {
            let classes = self.types.classes();
            let mut class = self.entities[attacker].class;
            loop {
                let metadata = classes
                    .metadata(class)
                    .ok_or("projectile launcher class metadata absent")?;
                if classes.name_text(metadata.name) == Some(b"arkNPC") {
                    npc_launcher = true;
                    break;
                }
                let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                    break;
                };
                class = base;
            }
        }
        //639970's real launcher dependency precedes Initialize626eb0;
        //record it on the attached Throwable, preserving dormant body state.
        if let Some(equipment) = request.equipment {
            self.rigid_component_mut(throwable)?
                .throwable_mut()?
                .set_instigator(equipment);
        }
        self.entity_flags_d4[index] |= 4;
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        component.initialize(
            id,
            request,
            CheckedComponent {
                handle: throwable,
                generation,
            },
        )?;
        if npc_launcher {
            component
                .runtime
                .as_mut()
                .ok_or("projectile runtime initialization absent")?
                .process_all_contacts = true;
        }
        //626eb0 supplies entity+33c before629590 ever consumes that value.
        //The runtime kernel and reference actor borrow the same launch value.
        let velocity = component
            .runtime
            .as_ref()
            .ok_or("projectile launch runtime absent")?
            .velocity
            .to_array();
        self.entities[index].frame.velocities[0] = velocity;
        if let Some(actor) = self.entities[index].components.reference_actor {
            self.actors
                .actors
                .get_mut(actor.slot)
                .ok_or("projectile launch actor absent")?
                .frame
                .published_velocities[0] = velocity;
        }
        Ok(())
    }
    ///629590 is driven with this entity's actual time-space clock. The caller
    ///publishes the returned frame through its existing reference-frame actor,
    ///and performs real Throwable handoff before the next physics step.
    pub(in crate::app) fn step_projectile(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
        dt: f32,
        gravity: Vec3,
        target: Option<Vec3>,
        damage_factor: f32,
        sleep_dart_pickup_chance: Option<f32>,
        overlapping_spawner: bool,
        mut random: impl FnMut() -> f32,
    ) -> Result<ProjectileStep, String> {
        if handle.class_id != self.classes[13] {
            return Err("projectile update class mismatch".into());
        }
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload absent")?;
        let hidden = component.hidden;
        let mut runtime = component
            .runtime
            .take()
            .ok_or("Projectile has not initialized")?;
        let result = (|| {
            if live.stage(runtime.identity) != Some(Stage::Spawned) {
                return Err("projectile identity is stale or retiring".into());
            }
            let mut output = ProjectileStep {
                entity: runtime.identity,
                frame: runtime.frame(),
                velocity: runtime.velocity,
                hidden,
                retire: false,
                handoff: Handoff::None,
                witness: None,
                contact_entity: None,
                impulse: None,
                contact_impulse: None,
                response: projectiles::Response::None,
                publish_motion: false,
                remove_when_hidden: false,
                impact_fx: None,
                hit_direction: Vec3::ZERO,
                stop_component: runtime.impact_stopped || runtime.picked_up,
                suppress_pickable: false,
                contact_effects: Vec::new(),
                contact_speed: 0.,
                contact_mass: 0.,
                contact_stimulus_enabled: false,
            };
            if output.retire || runtime.has_hit_and_stopped || runtime.picked_up {
                return Ok(output);
            }
            if dt == 0. || !runtime.movement_enabled {
                return Ok(output);
            }
            let &index = self
                .shared_indices
                .get(&runtime.identity)
                .ok_or("projectile flight actor absent")?;
            let entity = &self.entities[index];
            runtime.position = Vec3::from_array(entity.frame.pose.position);
            runtime.axis = bevy_math::Mat3::from_cols_array_2d(&entity.frame.pose.basis);
            runtime.velocity = Vec3::from_array(entity.frame.velocities[0]);
            runtime.home(dt, target);
            let actor = entity
                .components
                .reference_actor
                .and_then(|handle| self.actors.actors.get(handle.slot))
                .ok_or("projectile finite motion owner absent")?;
            //629a6f ->750de0(priority2,R9=false) only reads the original
            //ordered requests. It neither advances their expiry nor applies
            //body motion. An absent priority2 uses the actual cached linear
            //request only when7506d0's valid byte is set.
            if let Some(request) = actor.requests[0]
                .entries()
                .iter()
                .find(|request| request.input.priority == 2)
            {
                runtime.velocity = Vec3::from_array(request.input.velocity);
            } else if actor.motion.valid[0] {
                runtime.velocity = Vec3::from_array(actor.motion.velocities[0]);
            }
            let Some(flight) = runtime.flight(dt, gravity) else {
                return Ok(output);
            };
            output.publish_motion = true;
            let direction = (flight.end - flight.start).normalize_or_zero();
            runtime.orient_for_segment(dt, direction);
            let query_end = flight.end + direction * runtime.forward_extent;
            let world = self
                .world
                .read()
                .map_err(|_| "projectile canonical query world poisoned")?;
            let component = self
                .projectiles
                .get_mut(handle.slot)
                .ok_or("projectile payload absent")?;
            component.ray_hits(
                &world,
                &mut self.executor,
                flight.start,
                query_end,
                overlapping_spawner,
            )?;
            let mut selected = None;
            for hit in &self
                .projectiles
                .get(handle.slot)
                .ok_or("projectile payload absent")?
                .hits
            {
                let target = if hit.target.user_data == 0 {
                    None
                } else {
                    let index = CollisionUserData::decode(hit.target.user_data)?
                        .entity(|h| self.owner(h).ok())?;
                    Some(
                        self.entities
                            .get(index)
                            .ok_or("projectile contact actor absent")?
                            .shared,
                    )
                };
                if target == Some(runtime.identity)
                    || overlapping_spawner && target == runtime.attacker
                    || target.is_some_and(|id| runtime.pass_through_entities.contains(&id))
                {
                    continue;
                }
                //625ab0 rejects an inactive water trigger before effects,
                //damage or surface response; it is not a solid obstacle.
                if self.projectile_water_state(*hit)? == Some(false) {
                    continue;
                }
                //6259b0: active water passes through when authored. Keep
                //its real contact for626800/material FX, then continue the
                //same sorted hit collector rather than bouncing at the plane.
                if self.projectile_water_state(*hit)? == Some(true)
                    && runtime.declaration.pass_through_water
                {
                    output.contact_effects.push(*hit);
                    if let Some(target) = target {
                        runtime.pass_through_entities.push(target);
                    }
                    continue;
                }
                // The source's shooter overlap collector is owned by the
                //source adapter; unignored actor contacts remain real hits.
                selected = Some((*hit, target));
                break;
            }
            let surface = selected
                .map(|(hit, _)| {
                    if hit.target.material_id == u16::MAX {
                        Ok::<_, String>((true, true))
                    } else {
                        let m = world
                            .storage
                            .materials
                            .material_for_id(hit.target.material_id)?;
                        Ok((m.projectile_can_stick, m.projectile_can_break))
                    }
                })
                .transpose()?;
            drop(world);
            if selected.is_some() || !output.contact_effects.is_empty() {
                output.contact_speed = flight.velocity.length();
                (output.contact_mass, output.contact_stimulus_enabled) =
                    self.projectile_contact_parameters(handle)?;
            }
            for witness in &output.contact_effects {
                let hit = Hit {
                    position: Vec3::new(
                        witness.position[0],
                        witness.position[1],
                        witness.position[2],
                    ),
                    normal: Vec3::new(witness.normal[0], witness.normal[1], witness.normal[2]),
                    direction,
                    region: self.projectile_hit_region(*witness)?,
                };
                if let Some(parameters) = runtime.hit_parameters(hit, damage_factor, false) {
                    self.damage_collision_target(
                        live,
                        events,
                        witness.target.user_data,
                        &parameters,
                    )?;
                }
            }
            output.hit_direction = direction;
            if let Some((witness, target_id)) = selected {
                output.contact_entity = target_id;
                output.contact_effects.push(witness);
                let hit = Hit {
                    position: Vec3::new(
                        witness.position[0],
                        witness.position[1],
                        witness.position[2],
                    ),
                    normal: Vec3::new(witness.normal[0], witness.normal[1], witness.normal[2]),
                    direction: (flight.end - flight.start).normalize_or_zero(),
                    region: self.projectile_hit_region(witness)?,
                };
                let (can_stick, can_break) = surface.ok_or("projectile material result absent")?;
                let class_matches = |id: EntityId, name: &[u8]| -> Result<bool, String> {
                    let (_, mut class) = live.get(id).ok_or("projectile target identity stale")?;
                    loop {
                        let classes = self.types.classes();
                        let metadata = classes
                            .metadata(class)
                            .ok_or("target class metadata absent")?;
                        if classes.name_text(metadata.name) == Some(name) {
                            return Ok(true);
                        }
                        let Some(base) = classes.hierarchy().node(class).and_then(|n| n.base)
                        else {
                            return Ok(false);
                        };
                        class = base;
                    }
                };
                let player = target_id
                    .map(|id| class_matches(id, b"arkPlayer"))
                    .transpose()?
                    .unwrap_or(false);
                let npc = target_id
                    .map(|id| class_matches(id, b"arkNPC"))
                    .transpose()?
                    .unwrap_or(false);
                if npc {
                    return Err(
                        "projectile NPC contact requires actual NPC armor/deflection/sleep owner"
                            .into(),
                    );
                }
                if let Some(parameters) = runtime.hit_parameters(hit, damage_factor, false) {
                    self.damage_collision_target(
                        live,
                        events,
                        witness.target.user_data,
                        &parameters,
                    )?;
                }
                //625e80 targets the actual Pickable_Simple component,
                //not the trail or the Throwable. Sleep darts use the
                //player's bound +f8 pickup-chance attribute in percent.
                let can_recover = sleep_dart_pickup_chance.map_or_else(
                    || runtime.declaration.stick.can_recover,
                    |chance| random() * 100. < chance,
                );
                output.suppress_pickable = !can_recover;
                if !runtime.process_all_contacts {
                    runtime.impact_stopped = true;
                }
                let contact = Contact {
                    player,
                    npc,
                    //6262d0 tests the collision COMPONENT RTTI, not whether
                    //the ray happened to hit something in the world.
                    world: if witness.target.user_data == 0 {
                        false
                    } else {
                        let handle = CollisionUserData::decode(witness.target.user_data)?.component;
                        let classes = self.types.classes();
                        let metadata = classes
                            .metadata(handle.class_id as usize)
                            .ok_or("projectile collision component class absent")?;
                        classes.name_text(metadata.name) == Some(b"arkCpntPhysicsWorld")
                    },
                    can_stick,
                    //624300: Pickable decl+a0 is holdAndThrowProps+30
                    //(141738440), not another physics-material stick bit.
                    target_allows_stick: if let Some(target) = target_id {
                        match self.grapple_pickable(live, target)? {
                            Some((_, pickable)) if pickable.definition.hold.is_none() => {
                                let index = *self.shared_indices.get(&target)
                                    .ok_or("projectile stick target scene owner absent")?;
                                self.loot_component_lookup(index, 7, [0x28, 0])?.is_some()
                            }
                            _ => true,
                        }
                    } else { true },
                    can_break,
                    angle: projectiles::incidence_angle(hit.normal, runtime.velocity),
                    active_water_trigger: self.projectile_water_state(witness)? == Some(true),
                    absorbed_by_armor: false,
                    desired_deflection: 0,
                    npc_break_probability: None,
                };
                if (!contact.active_water_trigger || !runtime.declaration.pass_through_water)
                    && !(runtime.declaration.dont_explode_on_characters && (player || npc))
                {
                    output.impact_fx = runtime.explosion_fx_override.clone().or_else(|| {
                        runtime
                            .declaration
                            .effects
                            .explosion
                            .as_deref()
                            .map(Arc::from)
                    });
                }
                output.hit_direction = hit.direction;
                let mut flags = ContactFlags {
                    always_break_on_hit: runtime.flags.always_break_on_hit,
                    always_stick_to_npcs: runtime.flags.always_stick_to_npcs,
                    pinned_limb: false,
                    bounce_once: runtime.flags.bounce_once_instead_of_breaking,
                };
                let response =
                    projectiles::response(&runtime.declaration, &contact, &mut flags, &mut random);
                runtime.flags.bounce_once_instead_of_breaking = flags.bounce_once;
                let motion = runtime.apply_response(response, hit, flight.velocity, &mut random);
                if matches!(motion.handoff, Handoff::Stick) {
                    let axis = runtime.sticking_axis(hit.normal, &mut random);
                    let frame = runtime.stick_frame(hit, axis, false, false, random());
                    runtime.position = Vec3::from_array(frame.position);
                    runtime.axis = bevy_math::Mat3::from_cols_array_2d(&frame.basis);
                }
                if response != projectiles::Response::None {
                    output.remove_when_hidden = true;
                }
                if response != projectiles::Response::Bounce {
                    if let Some(body_id) = witness.target.body_id {
                        let world = self.world.read().map_err(|_| "projectile world poisoned")?;
                        let body = world
                            .storage
                            .bodies
                            .get(body_id)
                            .ok_or("projectile hit body generation stale")?;
                        if body.flags & 5 == 0 {
                            let damage = runtime.declaration.damage.as_ref().ok_or(
                                "projectile contact impulse has no required damage declaration",
                            )?;
                            let factor = if damage.maximum > 0. {
                                (damage.maximum + runtime.damage_increment) / damage.maximum
                            } else {
                                1.
                            };
                            output.contact_impulse = Some(
                                hit.direction * (runtime.declaration.contact_impulse * factor),
                            );
                        }
                    }
                }
                output.handoff = motion.handoff;
                output.response = response;
                output.impulse = motion.impulse;
                output.hidden = motion.hidden;
                output.witness = Some(witness);
            } else {
                runtime.publish_flight(flight);
            }
            output.frame = runtime.frame();
            output.velocity = runtime.velocity;
            output.stop_component |= runtime.impact_stopped || runtime.picked_up;
            output.retire |= flight.out_of_range;
            Ok(output)
        })();
        let component = self
            .projectiles
            .get_mut(handle.slot)
            .ok_or("projectile payload disappeared")?;
        if let Ok(output) = &result {
            component.hidden = output.hidden;
        }
        component.runtime = Some(runtime);
        result
    }
}
