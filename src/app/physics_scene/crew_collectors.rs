//! Collector pools own actual typed state and their crew-local references.
//! Base Init57cbf0/Deinit57cc40; subclass Start/Stop remain distinct.
use super::*;
use crate::{
    assets::ai_world::crew_collectors::Definition,
    gameplay::{
        ai_world::crew_collectors::{Component, State},
        entity_events::{EntityEvents, Event, Listener, Payload},
        entity_lifetime::{EntityId, LiveEntities},
        pickable_component::CheckedComponent,
    },
};

impl PhysicsScene {
    ///586a00→3eb5f0 resets the real Herd pointer/vector then requests Stop
    ///through the same outer registry. It is not an update-counter pause.
    pub(super) fn reset_crew_herd_path(&mut self,id:EntityId,checked:CheckedComponent,
        registry:&mut ComponentRegistry,live:&mut LiveEntities,events:&mut EntityEvents)->Result<(),String>{
        let (actual_id,actual)=self.collector_identity(checked.handle)?;
        if checked.handle.class_id!=self.classes[37]||actual_id!=id||actual!=checked{return Err("Crew Herd reset generation differs".into())}
        let State::Herd(formation)=&mut self.collector_mut(checked.handle)?.state else{return Err("Crew Herd formation payload differs".into())};
        formation.settings=None;
        //Installed DAT1431af620..628 is the actual zero-vector constant.
        formation.origin=[0.;3];
        registry.request_stop(checked.handle,&mut super::interaction_components::Startup{world:None,scene:self,live,events})
    }
    pub(super) fn is_crew_collector_class(&self, class: i32) -> bool {
        self.classes[33..38].contains(&class)
    }
    fn collector_pool(&self, class: i32) -> Result<&ComponentPool<Component>, String> {
        if class == self.classes[33] {
            return Ok(&self.ai_crew_area);
        }
        if class == self.classes[34] {
            return Ok(&self.ai_crew_combat);
        }
        if class == self.classes[35] {
            return Ok(&self.ai_crew_notification);
        }
        if class == self.classes[36] {
            return Ok(&self.ai_crew_search);
        }
        if class == self.classes[37] {
            return Ok(&self.ai_crew_herd);
        }
        Err("unknown Crew collector class".into())
    }
    fn collector_pool_mut(&mut self, class: i32) -> Result<&mut ComponentPool<Component>, String> {
        if class == self.classes[33] {
            return Ok(&mut self.ai_crew_area);
        }
        if class == self.classes[34] {
            return Ok(&mut self.ai_crew_combat);
        }
        if class == self.classes[35] {
            return Ok(&mut self.ai_crew_notification);
        }
        if class == self.classes[36] {
            return Ok(&mut self.ai_crew_search);
        }
        if class == self.classes[37] {
            return Ok(&mut self.ai_crew_herd);
        }
        Err("unknown Crew collector class".into())
    }
    fn collector(&self, handle: ComponentHandle) -> Result<&Component, String> {
        self.collector_pool(handle.class_id)?
            .get(handle.slot)
            .ok_or_else(|| "Crew collector payload absent".into())
    }
    fn collector_mut(&mut self, handle: ComponentHandle) -> Result<&mut Component, String> {
        self.collector_pool_mut(handle.class_id)?
            .get_mut(handle.slot)
            .ok_or_else(|| "Crew collector payload absent".into())
    }
    pub(super) fn collector_owner(&self, handle: ComponentHandle) -> Option<&ComponentOwner> {
        self.collector(handle).ok().map(|value| &value.owner)
    }
    pub(super) fn collector_owner_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Option<&mut ComponentOwner> {
        self.collector_mut(handle)
            .ok()
            .map(|value| &mut value.owner)
    }
    pub(super) fn schedule_crew_collector(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        self.collector_pool_mut(handle.class_id)?
            .schedule(handle.slot)
    }
    pub(super) fn create_crew_collector(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        let class = declaration.class_id;
        let (pool, entities) = if class == self.classes[33] {
            (&mut self.ai_crew_area, &mut self.entities)
        } else if class == self.classes[34] {
            (&mut self.ai_crew_combat, &mut self.entities)
        } else if class == self.classes[35] {
            (&mut self.ai_crew_notification, &mut self.entities)
        } else if class == self.classes[36] {
            (&mut self.ai_crew_search, &mut self.entities)
        } else if class == self.classes[37] {
            (&mut self.ai_crew_herd, &mut self.entities)
        } else {
            return Err("unknown Crew collector factory".into());
        };
        ComponentFactory {
            declaration,
            pool,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entities[index].components,
            immediate,
            |_, _, _| Component::new(definition.clone()),
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn release_crew_collector_binding(
        &mut self,
        index: usize,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (pool, entities) = if handle.class_id == self.classes[33] {
            (&mut self.ai_crew_area, &mut self.entities)
        } else if handle.class_id == self.classes[34] {
            (&mut self.ai_crew_combat, &mut self.entities)
        } else if handle.class_id == self.classes[35] {
            (&mut self.ai_crew_notification, &mut self.entities)
        } else if handle.class_id == self.classes[36] {
            (&mut self.ai_crew_search, &mut self.entities)
        } else if handle.class_id == self.classes[37] {
            (&mut self.ai_crew_herd, &mut self.entities)
        } else {
            return Err("unknown Crew collector release".into());
        };
        entities[index].components.release(
            handle,
            pool.get_mut(handle.slot)
                .ok_or("Crew collector release payload absent")?,
        )
    }
    pub(super) fn reclaim_crew_collector(&mut self, handle: ComponentHandle) -> Result<(), String> {
        self.collector_pool_mut(handle.class_id)?
            .release(handle.slot)?;
        Ok(())
    }
    fn collector_identity(
        &self,
        handle: ComponentHandle,
    ) -> Result<(EntityId, CheckedComponent), String> {
        let owner = &self.collector(handle)?.owner;
        let id = owner
            .entity
            .and_then(|index| self.entities.get(index))
            .ok_or("Crew collector actor absent")?
            .shared;
        Ok((
            id,
            CheckedComponent {
                handle,
                generation: owner.generation,
            },
        ))
    }
    pub(super) fn initialize_crew_collector(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, _) = self.collector_identity(handle)?;
        if self.collector(handle)?.initialized {
            return Ok(());
        }
        let core = self.crew_core_for_entity(id)?;
        let knowledge = self.crew_knowledge_for_entity(id)?;
        let component = self.collector_mut(handle)?;
        component.initialize_herd();
        component.core = core;
        component.knowledge = knowledge;
        component.initialized = true;
        component.owner.mark_initialized();
        Ok(())
    }
    pub(super) fn start_crew_collector(
        &mut self,
        events: &mut EntityEvents,
        _registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.collector_identity(handle)?;
        let component = self.collector(handle)?;
        if component.started {
            return Ok(());
        }
        if !component.initialized {
            return Err("Crew collector Start precedes Initialize".into());
        }
        let core = component
            .core
            .ok_or("Crew collector Start requires actual Core")?;
        let knowledge = component
            .knowledge
            .ok_or("Crew collector Start requires actual KnowledgeBB")?;
        let core = self.crew_core(id, core)?;
        let member_count = core.members.len();
        let binding = core.binding;
        match &component.state {
            State::Area => {
                //569b30 performs no area write when Core's managed dispatch
                //binding is null (the actual default child constructor).
                if binding.is_some() {
                    return Err(
                        "Crew Area dispatch binding needs its actual dispatch area declaration"
                            .into(),
                    );
                }
            }
            State::Notification { .. } => events.register(
                0x5b,
                Some(id),
                collector_listener(self.owner_id, checked, 0x5b),
            ),
            State::Search { .. } => {
                if member_count != 0 {
                    return Err("Crew Search Start requires admitted member callback owners".into());
                }
                for event in [0x58, 0x59] {
                    events.register(
                        event,
                        Some(id),
                        collector_listener(self.owner_id, checked, event),
                    );
                }
                if let State::Search { dispatched, .. } = &mut self.collector_mut(handle)?.state {
                    *dispatched = true;
                }
            }
            State::Combat(_) => {}
            //3e89a0 leaves settings+30 null. It is filled by the herding
            //producer before3ea5e0 may run; default crews never start Herd.
            State::Herd(_) => {
                return Err("Crew Herd Start requires actual assigned herding settings".into())
            }
        }
        let _ = knowledge;
        self.collector_mut(handle)?.started = true;
        Ok(())
    }
    pub(super) fn stop_crew_collector(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (id, checked) = self.collector_identity(handle)?;
        if !self.collector(handle)?.started {
            return Ok(());
        }
        let knowledge = self
            .collector(handle)?
            .knowledge
            .ok_or("Crew collector Stop has no KnowledgeBB")?;
        match &self.collector(handle)?.state {
            State::Area => self.crew_knowledge_mut(id, knowledge)?.clear_area(),
            State::Combat(path) => {
                if path.submitted.is_some() {
                    return Err("Crew combat path cancellation requires its admitted navigation query owner".into());
                }
                self.crew_knowledge_mut(id, knowledge)?.clear_combat();
                if let State::Combat(path) = &mut self.collector_mut(handle)?.state {
                    path.path_points.clear();
                    path.last_time = -1.;
                    path.target = None;
                }
            }
            State::Notification { .. } => events.unregister(
                0x5b,
                Some(id),
                collector_listener(self.owner_id, checked, 0x5b),
            ),
            State::Search { .. } => {
                for event in [0x58, 0x59] {
                    events.unregister(
                        event,
                        Some(id),
                        collector_listener(self.owner_id, checked, event),
                    );
                }
                self.crew_knowledge_mut(id, knowledge)?.clear_search();
            }
            State::Herd(_) => {
                if let State::Herd(formation) = &mut self.collector_mut(handle)?.state {
                    formation.destinations.clear();
                    formation.active = false;
                }
            }
        }
        self.collector_mut(handle)?.started = false;
        Ok(())
    }
    pub(super) fn deinitialize_crew_collector(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        self.collector_mut(handle)?.deinitialize()
    }
    pub(super) fn crew_collector_message(
        &mut self,
        live: &LiveEntities,
        registry: &mut ComponentRegistry,
        listener: Listener,
        event: &Event,
    ) -> Result<(), String> {
        let Listener::Component {
            owner,
            handle,
            generation,
            handler,
        } = listener
        else {
            return Err("Crew collector listener kind differs".into());
        };
        let (id, checked) = self.collector_identity(handle)?;
        if owner != self.owner_id
            || generation != checked.generation
            || event.target != Some(id)
            || event.id != handler
            || live.get(id).is_none()
        {
            return Err("Crew collector callback generation/target differs".into());
        }
        if handler == 0x5b {
            let Payload::Bytes(bytes) = &event.payload else {
                return Err("Crew notification packet is not source bytes".into());
            };
            let packet: [u8; 16] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| "Crew notification packet size differs")?;
            registry.resume_updates(handle, self)?; //56dd70→7e79a0→7e79c0.
            let State::Notification { pending } = &mut self.collector_mut(handle)?.state else {
                return Err("Crew notification callback payload differs".into());
            };
            pending.push(crate::gameplay::ai_world::crew_collectors::Notification(
                packet,
            ));
            return Ok(());
        }
        Err(format!(
            "Crew collector message {handler:x} needs its actual member producer"
        ))
    }
}
fn collector_listener(owner: usize, checked: CheckedComponent, handler: u16) -> Listener {
    Listener::Component {
        owner,
        handle: checked.handle,
        generation: checked.generation,
        handler,
    }
}
