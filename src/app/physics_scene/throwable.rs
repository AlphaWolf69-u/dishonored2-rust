//! Live entity binding and Throwable lifecycle adapters,875e80/8762b0.
use super::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    reference_frame::{ReferenceFrame, relative_binding},
};
#[derive(Clone, Copy)]
pub(in crate::app) struct BindingPose {
    pub joint: i16,
    pub offset: [f32; 3],
    pub orientation: [f32; 4],
    pub parent_placement: Option<usize>,
}
impl PhysicsScene {
    pub(in crate::app) fn bind_entity(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        parent: EntityId,
        pose: BindingPose,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let child = *self
            .shared_indices
            .get(&child)
            .ok_or("bound child has no actual scene owner")?;
        let parent = *self
            .shared_indices
            .get(&parent)
            .ok_or("bound parent has no actual scene owner")?;
        if live.get(self.entities[child].shared).is_none()
            || live.get(self.entities[parent].shared).is_none()
        {
            return Err("bound managed identity is stale".into());
        }
        if let Some(binding) = self
            .document_hierarchy
            .bindings
            .iter()
            .find(|binding| binding.child == child)
        {
            let actor = self.entities[child]
                .components
                .reference_actor
                .and_then(|h| self.actors.actors.get(h.slot))
                .ok_or("bound actor is absent")?;
            if binding.parent == parent
                && actor.binding.joint == pose.joint
                && actor.binding.offset == pose.offset
                && actor.binding.orientation == pose.orientation
            {
                return Ok(());
            }
            return Err("entity must detach its previous live binding before rebinding".into());
        }
        let mut registry = self
            .registry
            .take()
            .ok_or("binding component registry is absent")?;
        let result = (|| {
            let child_handle = self.reference_actor(child, &mut registry)?;
            let parent_handle = self.reference_actor(parent, &mut registry)?;
            let ca = child_handle.slot as usize;
            let pa = parent_handle.slot as usize;
            let cg = self.actors.actors[ca].owner.generation;
            let pg = self.actors.actors[pa].owner.generation;
            self.actors.actors[pa]
                .children
                .try_reserve(1)
                .map_err(|e| format!("actor child binding: {e}"))?;
            self.document_hierarchy
                .bindings
                .try_reserve(1)
                .map_err(|e| format!("entity binding roster: {e}"))?;
            let initial = self.entities[child].frame.pose;
            let parent_frame = self.entities[parent].frame.pose;
            live.link_child(self.entities[parent].shared, self.entities[child].shared)?;
            self.actors.actors[pa].children.push(ca);
            let actor = &mut self.actors.actors[ca];
            actor.parent = Some(pa);
            actor.binding.entity = Some(parent);
            actor.binding.joint = pose.joint;
            actor.binding.offset = pose.offset;
            actor.binding.orientation = pose.orientation;
            actor.flags |= 0x10;
            self.entities[child].entity_flags |= 0x80;
            self.document_hierarchy.record_runtime_binding(
                child,
                parent,
                pose.parent_placement,
                pose.joint,
                (child_handle, cg),
                (parent_handle, pg),
                initial,
                parent_frame,
            );
            self.actors.propagate_body_flag(pa, true)?;
            events.post(Event {
                id: 0x125,
                target: Some(self.entities[child].shared),
                payload: Payload::Entity(self.entities[parent].shared),
            });
            self.request_post_physics_actor(pa, pg)?;
            Ok(())
        })();
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn bind_to_player(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        player: &crate::gameplay::player::Player,
        tag: &str,
        offset: [f32; 3],
        orientation: [f32; 4],
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let parent = player
            .managed_entity()
            .ok_or("held object's real player identity is absent")?;
        let (joint, tag_offset, tag_orientation) = player.reference_tag_binding(tag)?;
        let index = *self
            .shared_indices
            .get(&parent)
            .ok_or("held object's real player reference actor is absent")?;
        if joint >= 0 {
            self.document_hierarchy
                .external_joints
                .insert((index, joint), player.reference_local_joint(joint)?);
        }
        //752900 adds explicit offset to a real model tag and multiplies its
        //rotation (110ad0). Bare bones/empty tags expose zero/identity here.
        let offset = std::array::from_fn(|i| offset[i] + tag_offset[i]);
        let orientation = crate::gameplay::reference_frame::multiply_binding_quaternion(
            tag_orientation,
            orientation,
        );
        self.bind_entity(
            live,
            child,
            parent,
            BindingPose {
                joint,
                offset,
                orientation,
                parent_placement: None,
            },
            events,
        )
    }
    pub(in crate::app) fn unbind_entity(
        &mut self,
        live: &mut LiveEntities,
        child: EntityId,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&child)
            .ok_or("unbound child has no actual scene owner")?;
        if live.parent(child)?.is_none() {
            return Ok(());
        }
        //8762d0/727450 restores the actual body mode before topology disappears.
        let mut registry = self.registry.take().ok_or("unbind registry is absent")?;
        let restore = (|| {
            self.entities[index].entity_flags &= !0x80;
            if let Some(handle) = self.document_rigid(index)? {
                if self.owner(handle)?.flags & 3 > 1 {
                    let body = self
                        .rigid_component(handle)?
                        .body()
                        .ok_or("bound body absent")?;
                    let shared = self.world.clone();
                    let mut guard = shared.write().map_err(|_| "unbind world poisoned")?;
                    let world = &mut *guard;
                    scene_publication_events!(self, &mut registry, publication);
                    world.storage.dynamic_single_body(
                        &mut world.geometry,
                        &mut publication,
                        body,
                    )?;
                }
            }
            self.detach_document(live, index)
        })();
        self.registry = Some(registry);
        restore?;
        events.post(Event {
            id: 0x125,
            target: Some(child),
            payload: Payload::None,
        });
        Ok(())
    }
}

impl PhysicsScene {
    pub(in crate::app) fn throwable_handle(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<ComponentHandle, String> {
        if live.get(id).is_none() {
            return Err("Throwable entity generation is stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Throwable entity has no actual scene owner")?;
        for &packed in self.entities[index].components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            if handle.class_id == self.classes[12] {
                let component = self.rigid_component(handle)?;
                if component.owner.entity != Some(index) || !component.throwable()?.initialized {
                    return Err("Throwable actual component is not initialized".into());
                }
                return Ok(handle);
            }
        }
        Err("entity has no actual Throwable component".into())
    }
    /// The base808ea0 must finish first, including its real disabled body.
    pub(super) fn initialize_throwable_component(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let index = component
            .owner
            .entity
            .ok_or("Throwable component is unlinked")?;
        let timespace = self.entities[index].timespace as usize;
        //4c7930 checks actual Breakable and WhaleOilTank components. Only
        //the first currently has a factory; no tank is inferred by model name.
        let receiver = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .any(|&packed| {
                ComponentHandle::from_packed(packed).is_ok_and(|h| h.class_id == self.classes[11])
            });
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        super::super::throwable::initialize(
            self.rigid_component_mut(handle)?,
            &mut world.storage,
            &mut world.geometry,
            timespace,
            receiver,
        )
    }
    ///4c7fc0 runs after809d80. Fresh bound instances apply their actual
    /// attached filter before enabling quality/contact events and125.
    pub(super) fn start_throwable_component(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        entity_events: &mut EntityEvents,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let index = component
            .owner
            .entity
            .ok_or("Throwable component is unlinked")?;
        let id = self.entities[index].shared;
        let restoring = component.owner.flags & 0x4000_0000 != 0;
        let enabled = !restoring || component.throwable()?.flags() & 2 != 0;
        let attached = !restoring && self.entities[index].entity_flags & 0x80 != 0;
        let filter = if attached {
            let state = component.throwable()?;
            let mut settings = state.source.rigid.settings.clone();
            settings.collision = Some(state.source.settings.attached_collision);
            Some(
                settings
                    .collision_filter()
                    .ok_or("Throwable attached filter is invalid")?,
            )
        } else {
            None
        };
        let body = component
            .body()
            .ok_or("Throwable Start has no actual body")?;
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        if let Some(filter) = filter {
            scene_publication_events!(self, registry, publication);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)?;
        }
        {
            let quality = self.rigid_component(handle)?.throwable()?.quality(enabled);
            scene_publication_events!(self, registry, publication);
            if let Some(quality) = quality {
                world.storage.set_body_quality(
                    &mut world.geometry,
                    &mut publication,
                    body,
                    quality,
                    0,
                )?;
            }
        }
        self.rigid_component_mut(handle)?
            .throwable_mut()?
            .quality_applied(enabled);
        let flags = world
            .storage
            .bodies
            .get(body)
            .ok_or("Throwable body retired during Start")?
            .flags;
        let mode = if flags & 1 != 0 {
            0
        } else if flags & 4 != 0 {
            1
        } else {
            2
        };
        let contact = self
            .rigid_component(handle)?
            .throwable()?
            .contact_events(mode);
        {
            scene_publication_events!(self, registry, publication);
            world.storage.set_body_user_flags(
                &mut world.geometry,
                &mut publication,
                body,
                0x40,
                contact,
                0,
            )?;
        }
        drop(guard);
        let owner = self.owner_id;
        super::super::throwable::start_listener(
            self.rigid_component_mut(handle)?,
            entity_events,
            owner,
            handle,
            id,
        )
    }
    pub(super) fn stop_throwable_component(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        super::super::throwable::stop_listener(self.rigid_component_mut(handle)?, events)
    }
    pub(super) fn throwable_binding_changed(
        &mut self,
        live: &LiveEntities,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let index = component
            .owner
            .entity
            .ok_or("Throwable component is unlinked")?;
        let bound = live.parent(self.entities[index].shared)?.is_some();
        let state = component.throwable()?;
        if !state.stuck() && !state.collision_update_pending() {
            return Ok(());
        }
        let mut settings = state.source.rigid.settings.clone();
        if bound {
            settings.collision = Some(state.source.settings.attached_collision);
        }
        let filter = settings
            .collision_filter()
            .ok_or("Throwable binding filter is invalid")?;
        let body = component
            .body()
            .ok_or("Throwable binding has no actual body")?;
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "Throwable world owner is poisoned")?;
        let world = &mut *guard;
        {
            scene_publication_events!(self, registry, publication);
            world
                .storage
                .set_body_filter(&mut world.geometry, &mut publication, body, filter)?;
        }
        if !bound {
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .detached();
        }
        Ok(())
    }
    pub(in crate::app) fn start_throwable(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let handle = self.throwable_handle(live, id)?;
        let mut registry = self
            .registry
            .take()
            .ok_or("Throwable Start registry is absent")?;
        let result = (|| {
            // Vtable141b64b68+38 is0998b0, the source queued-start no-op.
            registry.request_start(
                &mut self.rigid_component_mut(handle)?.owner,
                handle.packed() as usize,
                false,
                |_, _| Ok(()),
            )?;
            registry.initialize_and_start(&mut interaction_components::Startup {
                scene: self,
                live,
                events,
            })?;
            registry.drain_update_changes(&mut interaction_components::Startup {
                scene: self,
                live,
                events,
            })
        })();
        self.registry = Some(registry);
        result
    }
    ///4c6b70's non-skeletal branch: the actual collision payload identifies
    ///the target entity. The body/shape witness is retained across callbacks.
    pub(in crate::app) fn stick_throwable(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
        frame: ReferenceFrame,
        target_body: u32,
        shape_key: u32,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        use crate::gameplay::throwable::{Binding, PendingStick, StickPhase};
        let handle = self.throwable_handle(live, id)?;
        let index = self
            .rigid_component(handle)?
            .owner
            .entity
            .ok_or("sticking Throwable is unlinked")?;
        let (parent, dynamic_target) = {
            let world = self.world.read().map_err(|_| "stick world poisoned")?;
            let body = world
                .storage
                .bodies
                .get(target_body)
                .ok_or("stick target body generation stale")?;
            let target =
                crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data)?
                    .component;
            let entity = self
                .owner(target)?
                .entity
                .ok_or("stick target collision payload is unlinked")?;
            (self.entities[entity].shared, body.flags & 1 == 0)
        };
        if live.get(parent).is_none() {
            return Err("stick target entity generation stale".into());
        }
        let request = Binding {
            parent,
            body: target_body,
            shape_key,
            frame,
        };
        let state = self.rigid_component_mut(handle)?.throwable_mut()?;
        if let Some(pending) = state.pending_stick {
            let old = pending.request;
            if old.parent != parent
                || old.body != target_body
                || old.shape_key != shape_key
                || old.frame.position != frame.position
                || old.frame.basis != frame.basis
            {
                return Err("sticking request differs from suspended source request".into());
            }
        } else {
            state.pending_stick = Some(PendingStick {
                request,
                phase: StickPhase::Quality,
                dynamic_target,
            });
        }
        let mut registry = self.registry.take().ok_or("stick registry absent")?;
        let result = (|| {
            let phase = self
                .rigid_component(handle)?
                .throwable()?
                .pending_stick
                .unwrap()
                .phase;
            if phase == StickPhase::Quality {
                if dynamic_target {
                    let quality = self.rigid_component(handle)?.throwable()?.quality(false);
                    if let Some(quality) = quality {
                        let body = self
                            .rigid_component(handle)?
                            .body()
                            .ok_or("sticking body absent")?;
                        let shared = self.world.clone();
                        let mut guard = shared.write().map_err(|_| "stick world poisoned")?;
                        let world = &mut *guard;
                        scene_publication_events!(self, &mut registry, publication);
                        world.storage.set_body_quality(
                            &mut world.geometry,
                            &mut publication,
                            body,
                            quality,
                            0,
                        )?;
                    }
                    self.rigid_component_mut(handle)?
                        .throwable_mut()?
                        .quality_applied(false);
                }
                self.rigid_component_mut(handle)?
                    .throwable_mut()?
                    .pending_stick
                    .as_mut()
                    .unwrap()
                    .phase = StickPhase::Frame;
            }
            if self
                .rigid_component(handle)?
                .throwable()?
                .pending_stick
                .unwrap()
                .phase
                == StickPhase::Frame
            {
                let body = self
                    .rigid_component(handle)?
                    .body()
                    .ok_or("sticking body absent")?;
                let matrix = std::array::from_fn(|row| {
                    if row == 3 {
                        [frame.position[0], frame.position[1], frame.position[2], 1.]
                    } else {
                        [
                            frame.basis[row][0],
                            frame.basis[row][1],
                            frame.basis[row][2],
                            0.,
                        ]
                    }
                });
                let shared = self.world.clone();
                let mut guard = shared.write().map_err(|_| "stick world poisoned")?;
                let world = &mut *guard;
                {
                    scene_publication_events!(self, &mut registry, publication);
                    world.storage.set_body_transform(
                        &mut world.geometry,
                        &mut publication,
                        body,
                        matrix,
                        0,
                    )?;
                }
                self.entities[index].frame.pose = frame;
                if let Some(actor) = self.entities[index].components.reference_actor {
                    self.actors.actors[actor.slot as usize].set_transform(frame, false);
                }
                self.rigid_component_mut(handle)?
                    .throwable_mut()?
                    .pending_stick
                    .as_mut()
                    .unwrap()
                    .phase = StickPhase::Bind;
            }
            Ok(())
        })();
        self.registry = Some(registry);
        result?;
        if self
            .rigid_component(handle)?
            .throwable()?
            .pending_stick
            .unwrap()
            .phase
            == StickPhase::Bind
        {
            let parent_index = *self
                .shared_indices
                .get(&parent)
                .ok_or("stick parent scene owner absent")?;
            let (offset, orientation) =
                relative_binding(self.entities[parent_index].frame.pose, frame);
            self.bind_entity(
                live,
                id,
                parent,
                BindingPose {
                    joint: -1,
                    offset,
                    orientation,
                    parent_placement: None,
                },
                events,
            )?;
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .pending_stick
                .as_mut()
                .unwrap()
                .phase = StickPhase::Notify;
        }
        if self
            .rigid_component(handle)?
            .throwable()?
            .pending_stick
            .unwrap()
            .phase
            == StickPhase::Notify
        {
            events.post(Event {
                id: 0x184,
                target: Some(parent),
                payload: Payload::Entity(id),
            });
            self.rigid_component_mut(handle)?
                .throwable_mut()?
                .mark_stuck(request);
        }
        Ok(())
    }
}
