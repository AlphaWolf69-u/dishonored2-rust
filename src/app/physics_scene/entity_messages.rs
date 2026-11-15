//! Numeric entity-message callbacks on actual registered rigid components.
//! 808ea0/809d80 registrations and adapter thunks80bbe0..80bc60.
use super::*;
use crate::gameplay::{entity_events::{EntityEvents, Event, Listener, Payload}, entity_lifetime::LiveEntities};

impl PhysicsScene {
    pub(in crate::app) fn pump_entity_messages(&mut self, live: &LiveEntities, events: &mut EntityEvents) -> Result<(), String> {
        let mut registry = self.registry.take().ok_or("entity messages have no component registry")?;
        let result = self.pump_entity_messages_with_registry(live, events, &mut registry);
        self.registry = Some(registry);
        result
    }

    pub(super) fn pump_entity_messages_with_registry(&mut self, live: &LiveEntities,
        events: &mut EntityEvents, registry: &mut ComponentRegistry) -> Result<(), String> {
        let errors = events.pump(|listener, event, _commands| self.entity_message(live, registry, listener, event));
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }

    fn entity_message(&mut self, live: &LiveEntities, registry: &mut ComponentRegistry,
        listener: Listener, event: &Event) -> Result<(), String> {
        let Listener::Component { owner, handle, generation, handler } = listener else {
            return Err(format!("registered entity listener has no admitted callback owner: {listener:?}"));
        };
        if owner != self.owner_id || handle.class_id != self.classes[8] {
            return Err(format!("registered entity callback has no matching physics owner: {listener:?}"));
        }
        let component = self.rigid_bodies.get(handle.slot).ok_or("entity message rigid slot is absent")?;
        if component.owner.generation != generation { return Err("entity message rigid generation is stale".into()); }
        let index = component.owner.entity.ok_or("entity message rigid entity is absent")?;
        let entity = self.entities.get(index).ok_or("entity message owner is absent")?;
        if event.target != Some(entity.shared) || live.get(entity.shared).is_none() || event.id != handler {
            return Err("entity message target or handler differs from its registered owner".into());
        }
        let body = component.body().ok_or("entity message body is absent")?;
        match handler {
            //80bc00/80bc30 ->851a70: keep the same real body and translate
            // its existing derived motion profile between spaces0 and1.
            0xad => self.rigid_message_timespace(handle, body, 0, 1, registry),
            0xac => self.rigid_message_timespace(handle, body, 1, 0, registry),
            //80bc50 ->727100 retains/coalesces the actual typed motion
            // request. Its component update/publication owner consumes it.
            0x149 => match &event.payload {
                Payload::Bytes(bytes) => self.rigid_bodies.get_mut(handle.slot).ok_or("motion request component absent")?.apply_motion_request(bytes),
                _ => Err("rigid velocity request has no source message payload".into()),
            },
            0xfa | 0x148 => {
                let (impulse, point) = if handler == 0xfa {
                    let Payload::Bytes(bytes) = &event.payload else { return Err("rigid impulse has no source payload".into()); };
                    if bytes.len() < 24 { return Err("rigid impulse payload is truncated".into()); }
                    let values: [f32; 6] = std::array::from_fn(|i| f32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap()));
                    let impulse = [values[0],values[1],values[2],0.];
                    let point = [values[3],values[4],values[5],0.];
                    (impulse, (point != [0.;4]).then_some(point))
                } else { ([0.;4], None) };
                let shared = self.world.clone();
                scene_publication_events!(self, registry, publication);
                let mut guard = shared.write().map_err(|_| "entity-message physics world is poisoned")?;
                let world = &mut *guard;
                if handler == 0xfa {
                    world.storage.apply_body_impulse(&mut world.geometry, &mut publication, body, impulse, point)?;
                } else {
                    //80bbe0 ->807bc0 uses zero velocities, not a body impulse.
                    world.storage.set_body_velocity(&mut world.geometry, &mut publication, body, [0.;4], [0.;4], false)?;
                }
                Ok(())
            },
            0xd4 => Err("rigid wind message requires the admitted wind-vector listener owner (80bc60)".into()),
            _ => Err(format!("registered rigid message {handler:x} has no source callback adapter")),
        }
    }

    fn rigid_message_timespace(&mut self, handle: ComponentHandle, body: u32,
        from: usize, to: usize, registry: &mut ComponentRegistry) -> Result<(), String> {
        let shared = self.world.clone();
        let mut guard = shared.write().map_err(|_| "entity-message physics world is poisoned")?;
        let world = &mut *guard;
        if !world.storage.publication_idle() { return Err("timespace message overlaps body publication".into()); }
        let record = world.storage.bodies.get(body).ok_or("timespace body is stale")?;
        if record.flags & 1 != 0 { return Ok(()); } //851a70 ignores static bodies.
        let motion_id = record.motion_id;
        let body_flags = record.flags;
        let property = world.storage.motions.get(motion_id).ok_or("timespace motion is absent")?.properties_id;
        let spaces = world.storage.motion_timespaces.as_ref().ok_or("physics timespace owner is absent")?;
        let Some(original) = spaces.original_for_derived(from, property)? else { return Ok(()); };
        let previous = spaces.factor(from)?;
        let factor = spaces.factor(to)?;
        if !world.storage.motion_properties.added.is_empty() {
            return Err("timespace profile creation requires its real added-signal callback host".into());
        }
        let storage = &mut world.storage;
        let mapped = storage.motion_timespaces.as_mut().ok_or("physics timespace owner is absent")?
            .resolve(&mut storage.motion_properties, original, to, |_,_,_,_| unreachable!("added observers checked absent"))?;
        //c870c0 changes the same motion's property. No property-change
        //subscriber has been admitted by this host; do not invent one.
        storage.motions.get_mut(motion_id).ok_or("timespace motion disappeared")?.properties_id = mapped;
        drop(guard);

        //18d2b0 retains requested vectors while publishing their new scale.
        let component = self.rigid_bodies.get_mut(handle.slot).ok_or("timespace component absent")?;
        let mut flags = component.velocities.requested_linear[3].to_bits();
        let wake = factor != 0. && previous == 0. && flags & 1 != 0;
        let (linear, angular) = if factor == 0. {
            if flags & 2 != 0 || body_flags & 8 != 0 { flags |= 0x3f000001; }
            ([0.;4], [0.;4])
        } else {
            if wake { flags = (flags & !1) | 0x3f000000; }
            component.velocities.requested_linear[3] = f32::from_bits(flags);
            (component.velocities.requested_linear.map(|v| v * factor), component.velocities.requested_angular.map(|v| v * factor))
        };
        component.velocities.requested_linear[3] = f32::from_bits(flags);
        let shared = self.world.clone();
        {
            scene_publication_events!(self, registry, publication);
            let mut guard = shared.write().map_err(|_| "entity-message physics world is poisoned")?;
            let world = &mut *guard;
            world.storage.set_body_velocity(&mut world.geometry, &mut publication, body, linear, angular, wake)?;
        }
        let component = self.rigid_bodies.get_mut(handle.slot).ok_or("timespace component disappeared")?;
        component.velocities.scaled_linear = linear;
        component.velocities.scaled_angular = angular;
        Ok(())
    }
}
