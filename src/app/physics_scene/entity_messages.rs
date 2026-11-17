//! Numeric entity-message callbacks on actual registered rigid components.
//! 808ea0/809d80 registrations and adapter thunks80bbe0..80bc60.
use super::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Listener, Payload},
    entity_lifetime::LiveEntities,
};

impl PhysicsScene {
    pub(in crate::app) fn pump_entity_messages(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
        player: Option<&mut crate::gameplay::player::Player>,
        ai_worlds: Option<&mut crate::gameplay::ai_world::Worlds>,
        hearts:Option<&mut crate::app::heart::Hearts>,
        statistics:Option<&mut crate::app::player_statistics::PlayerStatistics>,
    ) -> Result<(), String> {
        let mut registry = self
            .registry
            .take()
            .ok_or("entity messages have no component registry")?;
        let result = self.pump_entity_messages_with_registry(live, events, &mut registry, clock, player, ai_worlds,hearts,statistics);
        self.registry = Some(registry);
        result
    }

    pub(super) fn pump_entity_messages_with_registry(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
        mut player: Option<&mut crate::gameplay::player::Player>,
        mut ai_worlds: Option<&mut crate::gameplay::ai_world::Worlds>,
        mut hearts:Option<&mut crate::app::heart::Hearts>,
        mut statistics:Option<&mut crate::app::player_statistics::PlayerStatistics>,
    ) -> Result<(), String> {
        let errors = events.pump(|listener, event, _commands| {
            if matches!(listener,Listener::Component{handle,..} if handle.class_id==self.classes[57]) {
                return self.fx_component_message(listener,event);
            }
            if matches!(listener,Listener::Component{handle,..} if handle.class_id==self.classes[55]){
                return self.dispatch_openable_message(live,listener,event,_commands);
            }
            if matches!(listener,Listener::Component{handle,..} if handle.class_id==self.classes[53]) {
                return self.dispatch_interactive_map(live,listener,event);
            }
            if let Listener::AIStatistics { handle, generation, handler } = listener {
                if handler != event.id {
                    return Err("AIStatistics message differs from its registered handler".into());
                }
                return self.ai_statistics_event(statistics.as_deref_mut(),live,handle,generation,event);
            }
            if let Listener::HeartComponent{entity}=listener {
                let owner=hearts.as_deref_mut().ok_or("Heart DE callback owner absent")?;
                owner.speech_finished(live,entity,event)?;
                if let Some(player)=player.as_deref_mut(){player.heart_bark_status(owner.speaking());}
                return Ok(());
            }
            if matches!(listener, Listener::AIWorldStartup { .. }) {
                return self.dispatch_ai_startup(live,registry,listener,event,_commands,clock,ai_worlds.as_deref());
            }
            if matches!(listener,Listener::Component{handle,..} if self.is_smart_object_class(handle.class_id)) {
                return self.dispatch_smart_object_event(live,listener,event);
            }
            if matches!(listener,Listener::Component{handle,..} if self.classes.get(48)==Some(&handle.class_id)) {
                return self.dispatch_observable_event(live,ai_worlds.as_deref().ok_or("Observable callback has no canonical AIWorld list")?,listener,event,_commands);
            }
            if matches!(listener, Listener::AIWorldDanger { .. }) {
                let owner = ai_worlds.as_deref_mut().ok_or("AI danger listener has no canonical world owner")?;
                return crate::app::ai_world::dispatch_owner(self, live, owner, registry, &listener, event).map(|_| ());
            }
            self.entity_message(live, registry, listener, event, clock, player.as_deref_mut())
        });
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn entity_message(
        &mut self,
        live: &LiveEntities,
        registry: &mut ComponentRegistry,
        listener: Listener,
        event: &Event,
        clock: Option<&crate::gameplay::game_clock_runtime::ClockRuntime>,
        player: Option<&mut crate::gameplay::player::Player>,
    ) -> Result<(), String> {
        if let Listener::ControllerFeedback { generation } = listener {
            return self.controller_feedback.as_ref().ok_or("rumble callback owner absent")?
                .message(generation, event, clock);
        }
        if let Listener::PlayerChaos {player:identity}=listener {
            if event.id!=0xa5{return Err("PlayerChaos listener received a different message".into())}
            if live.get(identity).is_none(){return Err("PlayerChaos callback generation is stale".into())}
            let Payload::Incapacitation{params,non_counting}=&event.payload else{return Err("PlayerChaos A5 packet is not the retained incapacitation source".into())};
            let player=player.ok_or("PlayerChaos callback owner is absent")?;
            if player.managed_entity()!=Some(identity){return Err("PlayerChaos callback owner generation differs".into())}
            //4c14b0 requires the real AICharacterDescription payload after
            //excluding both actual Doppelganger classes. Current admitted
            //non-NPC actors take that genuine null-component branch.
            let target=event.target.filter(|&target|live.get(target).is_some());
            if let Some(target)=target.filter(|_|!*non_counting&&params.damage_type.0&1==0) {
                let excluded=self.heart_context_component_present(target,b"arkCpntDoppelganger")?
                    ||self.heart_context_component_present(target,b"arkCpntDoppelgangerNpc")?;
                if !excluded&&self.heart_context_component_present(target,b"arkCpntAICharacterDescription")? {
                    return Err("PlayerChaos requires actual AICharacterDescription morality/armed fields and7806e0/71b690 responsibility providers".into());
                }
            }
            return player.chaos.incapacitated(params,*non_counting,None);
        }
        if let Listener::HeartState {player:identity}=listener {
            if live.get(identity).is_none() || event.target!=Some(identity) {return Err("UseHeart message identity differs from live subscription".into())}
            if event.id!=0x11d {return Err("UseHeart listener received a different message".into())}
            let Payload::Bytes(bytes)=&event.payload else {return Err("UseHeart rate payload is not bytes".into())};
            let bytes:[u8;4]=bytes.as_slice().try_into().map_err(|_|"UseHeart rate packet size differs")?;
            let player=player.ok_or("UseHeart registered Player owner absent")?;
            if player.managed_entity()!=Some(identity) {return Err("UseHeart registered Player generation differs".into())}
            return player.heart_playrate(f32::from_le_bytes(bytes));
        }
        let Listener::Component {
            owner,
            handle,
            generation,
            handler,
        } = listener
        else {
            return Err(format!(
                "registered entity listener has no admitted callback owner: {listener:?}"
            ));
        };
        if handle.class_id == self.classes[4] {
            return self.audio_message(live, listener, event);
        }
        //7ed9f0's A2/A5/1A6 subscribers belong to the actual Breakable
        //pool, never the RigidBody/Throwable payload projection.
        if handle.class_id == self.classes[11] {
            return self.breakable_message(live, listener, event);
        }
        if handle.class_id == self.classes[19] {
            return self.grenade_message(live,listener,event);
        }
        if handle.class_id == self.classes[13] {
            return self.projectile_message(live, registry, listener, event, clock);
        }
        if self.is_crew_collector_class(handle.class_id) {
            return self.crew_collector_message(live, registry, listener, event);
        }
        if self.is_crew_module_class(handle.class_id) {
            return self.crew_module_message(live, registry, listener, event);
        }
        if self.is_crew_class(handle.class_id) {
            return self.crew_message(listener,event);
        }
        if owner != self.owner_id || !self.is_rigid_class(handle.class_id) {
            return Err(format!(
                "registered entity callback has no matching physics owner: {listener:?}"
            ));
        }
        let component = self.rigid_component(handle)?;
        if component.owner.generation != generation {
            return Err("entity message rigid generation is stale".into());
        }
        let index = component
            .owner
            .entity
            .ok_or("entity message rigid entity is absent")?;
        let entity = self
            .entities
            .get(index)
            .ok_or("entity message owner is absent")?;
        if event.target != Some(entity.shared)
            || live.get(entity.shared).is_none()
            || event.id != handler
        {
            return Err(
                "entity message target or handler differs from its registered owner".into(),
            );
        }
        let body = component.body().ok_or("entity message body is absent")?;
        match handler {
            //4c7fc0 registers125 ->4c9980 ->4c7570. The retained
            //binding packet determines this transition, not a later parent
            //lookup after another queued bind/unbind may have occurred.
            0x125 => {
                let bound = match &event.payload {
                    Payload::Entity(_) => true,
                    Payload::None => false,
                    _ => return Err("Throwable125 has no actual binding notification".into()),
                };
                self.throwable_binding_changed(handle, bound, registry)
            }
            //80bc00/80bc30 ->851a70: keep the same real body and translate
            // its existing derived motion profile between spaces0 and1.
            0xad => self.rigid_message_timespace(handle, body, 0, 1, registry),
            0xac => self.rigid_message_timespace(handle, body, 1, 0, registry),
            //80bc50 ->727100 retains/coalesces the actual typed motion
            // request. Its component update/publication owner consumes it.
            0x149 => match &event.payload {
                Payload::Bytes(bytes) => rigid_pool_mut!(self, handle.class_id)?
                    .get_mut(handle.slot)
                    .ok_or("rigid component slot is absent")?
                    .apply_motion_request(bytes),
                _ => Err("rigid velocity request has no source message payload".into()),
            },
            0xfa | 0x148 => {
                let (impulse, point) = if handler == 0xfa {
                    let Payload::Bytes(bytes) = &event.payload else {
                        return Err("rigid impulse has no source payload".into());
                    };
                    if bytes.len() < 24 {
                        return Err("rigid impulse payload is truncated".into());
                    }
                    let values: [f32; 6] = std::array::from_fn(|i| {
                        f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
                    });
                    let impulse = [values[0], values[1], values[2], 0.];
                    let point = [values[3], values[4], values[5], 0.];
                    (impulse, (point != [0.; 4]).then_some(point))
                } else {
                    ([0.; 4], None)
                };
                let shared = self.world.clone();
                scene_publication_events!(self, registry, publication);
                let mut guard = shared
                    .write()
                    .map_err(|_| "entity-message physics world is poisoned")?;
                let world = &mut *guard;
                if handler == 0xfa {
                    world.storage.apply_body_impulse(
                        &mut world.geometry,
                        &mut publication,
                        body,
                        impulse,
                        point,
                    )?;
                } else {
                    //80bbe0 ->807bc0 uses zero velocities, not a body impulse.
                    world.storage.set_body_velocity(
                        &mut world.geometry,
                        &mut publication,
                        body,
                        [0.; 4],
                        [0.; 4],
                        0,
                    )?;
                }
                Ok(())
            }
            0xd4 => {
                let Payload::WindContact {
                    wind,
                    body: payload_body,
                    entered,
                    source_index,
                    priority,
                } = &event.payload
                else {
                    return Err("rigid Wind message has no source payload".into());
                };
                let (wind, payload_body, entered, source_index, priority) =
                    (*wind, *payload_body, *entered, *source_index, *priority);
                if payload_body != body {
                    return Err("rigid Wind message body generation differs".into());
                }
                let owner = self.owner(wind)?;
                if wind.class_id != self.classes[2] || owner.entity.is_none() {
                    return Err("rigid Wind message owner differs".into());
                }
                let component = rigid_pool_mut!(self, handle.class_id)?
                    .get_mut(handle.slot)
                    .ok_or("rigid Wind slot absent")?;
                if entered {
                    component
                        .wind_triggers
                        .try_reserve(1)
                        .map_err(|error| format!("rigid Wind list: {error}"))?;
                    let index = component
                        .wind_triggers
                        .iter()
                        .position(|entry| entry.2 <= priority)
                        .unwrap_or(component.wind_triggers.len());
                    component
                        .wind_triggers
                        .insert(index, (wind, source_index, priority));
                } else if let Some(index) = component
                    .wind_triggers
                    .iter()
                    .position(|entry| entry.0 == wind)
                {
                    component.wind_triggers.remove(index);
                }
                Ok(())
            }
            _ => Err(format!(
                "registered rigid message {handler:x} has no source callback adapter"
            )),
        }
    }

    fn rigid_message_timespace(
        &mut self,
        handle: ComponentHandle,
        body: u32,
        from: usize,
        to: usize,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "entity-message physics world is poisoned")?;
        let world = &mut *guard;
        if !world.storage.publication_idle() {
            return Err("timespace message overlaps body publication".into());
        }
        let record = world
            .storage
            .bodies
            .get(body)
            .ok_or("timespace body is stale")?;
        if record.flags & 1 != 0 {
            return Ok(());
        } //851a70 ignores static bodies.
        let motion_id = record.motion_id;
        let body_flags = record.flags;
        let property = world
            .storage
            .motions
            .get(motion_id)
            .ok_or("timespace motion is absent")?
            .properties_id;
        let spaces = world
            .storage
            .motion_timespaces
            .as_ref()
            .ok_or("physics timespace owner is absent")?;
        let Some(original) = spaces.original_for_derived(from, property)? else {
            return Ok(());
        };
        let previous = spaces.factor(from)?;
        let factor = spaces.factor(to)?;
        if !world.storage.motion_properties.added.is_empty() {
            return Err(
                "timespace profile creation requires its real added-signal callback host".into(),
            );
        }
        let storage = &mut world.storage;
        let mapped = storage
            .motion_timespaces
            .as_mut()
            .ok_or("physics timespace owner is absent")?
            .resolve(
                &mut storage.motion_properties,
                original,
                to,
                |_, _, _, _| unreachable!("added observers checked absent"),
            )?;
        //c870c0 changes the same motion's property. No property-change
        //subscriber has been admitted by this host; do not invent one.
        storage
            .motions
            .get_mut(motion_id)
            .ok_or("timespace motion disappeared")?
            .properties_id = mapped;
        drop(guard);

        //18d2b0 retains requested vectors while publishing their new scale.
        let component = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?;
        let mut flags = component.velocities.requested_linear[3].to_bits();
        let wake = factor != 0. && previous == 0. && flags & 1 != 0;
        let (linear, angular) = if factor == 0. {
            if flags & 2 != 0 || body_flags & 8 != 0 {
                flags |= 0x3f000001;
            }
            ([0.; 4], [0.; 4])
        } else {
            if wake {
                flags = (flags & !1) | 0x3f000000;
            }
            component.velocities.requested_linear[3] = f32::from_bits(flags);
            (
                component.velocities.requested_linear.map(|v| v * factor),
                component.velocities.requested_angular.map(|v| v * factor),
            )
        };
        component.velocities.requested_linear[3] = f32::from_bits(flags);
        let shared = self.world.clone();
        {
            scene_publication_events!(self, registry, publication);
            let mut guard = shared
                .write()
                .map_err(|_| "entity-message physics world is poisoned")?;
            let world = &mut *guard;
            //18d2b0 explicitly wakes a zero-timespace body before c85dc0;
            // this is independent of that setter's source mode argument0.
            if wake {
                world
                    .storage
                    .activate_body(&mut world.geometry, &mut publication, body)?;
            }
            world.storage.set_body_velocity(
                &mut world.geometry,
                &mut publication,
                body,
                linear,
                angular,
                0,
            )?;
        }
        let component = rigid_pool_mut!(self, handle.class_id)?
            .get_mut(handle.slot)
            .ok_or("rigid component slot is absent")?;
        component.velocities.scaled_linear = linear;
        component.velocities.scaled_angular = angular;
        Ok(())
    }
}
