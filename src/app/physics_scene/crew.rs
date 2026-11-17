//! Same-entity Crew component lookup877720; no second actor registry.
use super::*;
use crate::assets::ai_world::crew::Definition;
use crate::gameplay::{
    ai_world::crew,
    entity_events::{EntityEvents, Event, Listener, Payload},
    entity_lifetime::EntityId,
    pickable_component::CheckedComponent,
};
impl PhysicsScene {
    ///5a75a0/5a75c0 invoke these concrete d8/d0 hooks after the base
    ///component update-owner transition has completed.
    pub(super) fn crew_update_subscription(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
        enabled: bool,
    ) -> Result<(), String> {
        if handle.class_id != self.classes[32] {
            return Ok(());
        }
        let id = self.crew_entity(handle)?;
        let v = self
            .crew_decisions
            .get(handle.slot)
            .ok_or("CrewDecision update subscription owner absent")?;
        let listener = Listener::Component {
            owner: self.owner_id,
            handle,
            generation: v.owner.generation,
            handler: 0x23,
        };
        if enabled {
            events.register(0x23, Some(id), listener)
        } else {
            events.unregister(0x23, Some(id), listener)
        }
        Ok(())
    }
    pub(super) fn is_crew_class(&self, class: i32) -> bool {
        self.classes[29..33].contains(&class)
    }
    pub(super) fn crew_owner(&self, handle: ComponentHandle) -> Option<&ComponentOwner> {
        if handle.class_id == self.classes[29] {
            return self.crew_cores.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[30] {
            return self.crew_knowledge.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[31] {
            return self.crew_coordination.get(handle.slot).map(|v| &v.owner);
        }
        if handle.class_id == self.classes[32] {
            return self.crew_decisions.get(handle.slot).map(|v| &v.owner);
        }
        None
    }
    pub(super) fn crew_owner_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Option<&mut ComponentOwner> {
        if handle.class_id == self.classes[29] {
            return self.crew_cores.get_mut(handle.slot).map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[30] {
            return self
                .crew_knowledge
                .get_mut(handle.slot)
                .map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[31] {
            return self
                .crew_coordination
                .get_mut(handle.slot)
                .map(|v| &mut v.owner);
        }
        if handle.class_id == self.classes[32] {
            return self
                .crew_decisions
                .get_mut(handle.slot)
                .map(|v| &mut v.owner);
        }
        None
    }
    pub(super) fn schedule_crew(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id == self.classes[29] {
            return self.crew_cores.schedule(handle.slot);
        }
        if handle.class_id == self.classes[30] {
            return self.crew_knowledge.schedule(handle.slot);
        }
        if handle.class_id == self.classes[31] {
            return self.crew_coordination.schedule(handle.slot);
        }
        if handle.class_id == self.classes[32] {
            return self.crew_decisions.schedule(handle.slot);
        }
        Err("unknown Crew core class".into())
    }
    pub(super) fn create_crew(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        match &*definition {
            Definition::Core { .. } => ComponentFactory {
                declaration,
                pool: &mut self.crew_cores,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| crew::Core::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Knowledge => ComponentFactory {
                declaration,
                pool: &mut self.crew_knowledge,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| crew::Knowledge::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Coordination => ComponentFactory {
                declaration,
                pool: &mut self.crew_coordination,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| crew::Coordination::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
            Definition::Decision { .. } => ComponentFactory {
                declaration,
                pool: &mut self.crew_decisions,
                registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                immediate,
                |_, _, _| crew::Decision::new(definition.clone()),
                |_, _, _| Ok(()),
            ),
        }
    }
    pub(super) fn release_crew_binding(
        &mut self,
        index: usize,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id == self.classes[29] {
            return self.entities[index].components.release(
                handle,
                self.crew_cores
                    .get_mut(handle.slot)
                    .ok_or("CrewCore payload absent")?,
            );
        }
        if handle.class_id == self.classes[30] {
            return self.entities[index].components.release(
                handle,
                self.crew_knowledge
                    .get_mut(handle.slot)
                    .ok_or("CrewKnowledge payload absent")?,
            );
        }
        if handle.class_id == self.classes[31] {
            return self.entities[index].components.release(
                handle,
                self.crew_coordination
                    .get_mut(handle.slot)
                    .ok_or("CrewCoordination payload absent")?,
            );
        }
        if handle.class_id == self.classes[32] {
            return self.entities[index].components.release(
                handle,
                self.crew_decisions
                    .get_mut(handle.slot)
                    .ok_or("CrewDecision payload absent")?,
            );
        }
        Err("unknown Crew release class".into())
    }
    pub(super) fn reclaim_crew(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id == self.classes[29] {
            self.crew_cores.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[30] {
            self.crew_knowledge.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[31] {
            self.crew_coordination.release(handle.slot)?;
            return Ok(());
        }
        if handle.class_id == self.classes[32] {
            self.crew_decisions.release(handle.slot)?;
            return Ok(());
        }
        Err("unknown Crew reclaim class".into())
    }
    fn crew_entity(&self, handle: ComponentHandle) -> Result<EntityId, String> {
        let index = self
            .owner(handle)?
            .entity
            .ok_or("Crew component actor absent")?;
        self.entities
            .get(index)
            .filter(|entity| !entity.retired)
            .map(|entity| entity.shared)
            .ok_or("Crew actor retired".into())
    }
    pub(super) fn initialize_crew(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let id = self.crew_entity(handle)?;
        if handle.class_id == self.classes[29] {
            let decision = self.checked_crew_component(id, 32)?;
            let knowledge = self.crew_knowledge_for_entity(id)?;
            let component = self
                .crew_cores
                .get_mut(handle.slot)
                .ok_or("CrewCore payload absent")?;
            if component.initialized {
                return Ok(());
            }
            let Definition::Core { rules } = &*component.definition else {
                return Err("CrewCore declaration differs".into());
            };
            component.runtime = Some(crew::RuleRuntime::new(rules, id));
            component.decision = decision;
            component.knowledge = knowledge;
            component.owner.mark_initialized();
            component.initialized = true;
        } else if handle.class_id == self.classes[30] {
            let v = self
                .crew_knowledge
                .get_mut(handle.slot)
                .ok_or("CrewKnowledge payload absent")?;
            v.owner.mark_initialized();
            v.initialized = true;
        } else if handle.class_id == self.classes[31] {
            let v = self
                .crew_coordination
                .get_mut(handle.slot)
                .ok_or("CrewCoordination payload absent")?;
            v.owner.mark_initialized();
            v.initialized = true;
        } else if handle.class_id == self.classes[32] {
            let v = self
                .crew_decisions
                .get_mut(handle.slot)
                .ok_or("CrewDecision payload absent")?;
            v.owner.mark_initialized();
            v.initialized = true;
        } else {
            return Err("unknown Crew Initialize class".into());
        }
        Ok(())
    }
    pub(super) fn start_crew(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let id = self.crew_entity(handle)?;
        if handle.class_id == self.classes[29] {
            let component = self
                .crew_cores
                .get_mut(handle.slot)
                .ok_or("CrewCore payload absent")?;
            if component.started {
                return Ok(());
            }
            if !component.initialized {
                return Err("CrewCore Start precedes Initialize".into());
            }
            for handler in [0x27, 0x5c] {
                events.register(
                    handler,
                    Some(id),
                    Listener::Component {
                        owner: self.owner_id,
                        handle,
                        generation: component.owner.generation,
                        handler,
                    },
                );
            }
            component.requester = Some(id);
            component.request_active = true;
            component.reset_requests();
            events.post(Event {
                id: 0x56,
                target: Some(id),
                payload: Payload::CrewBinding {
                    entity: component.binding,
                    name: component.binding_name.clone(),
                },
            });
            component.started = true;
        } else if handle.class_id == self.classes[30] {
            self.crew_knowledge
                .get_mut(handle.slot)
                .ok_or("CrewKnowledge payload absent")?
                .started = true;
        } else if handle.class_id == self.classes[31] {
            self.crew_coordination
                .get_mut(handle.slot)
                .ok_or("CrewCoordination payload absent")?
                .started = true;
        } else if handle.class_id == self.classes[32] {
            let has_core = self.crew_core_for_entity(id)?.is_some();
            let v = self
                .crew_decisions
                .get_mut(handle.slot)
                .ok_or("CrewDecision payload absent")?;
            if v.started {
                return Ok(());
            }
            if !v.initialized {
                return Err("CrewDecision Start precedes Initialize".into());
            }
            if v.owner.flags & (1 << 30) == 0 || v.runtime.is_none() {
                let Definition::Decision { tree } = &*v.definition else {
                    return Err("CrewDecision declaration class differs".into());
                };
                let mut runtime =
                    crate::gameplay::ai_world::crew_tree::Instance::new(tree.clone(), id, has_core);
                runtime.start()?;
                v.runtime = Some(runtime);
            }
            for handler in [0x25, 0x23] {
                events.register(
                    handler,
                    Some(id),
                    Listener::Component {
                        owner: self.owner_id,
                        handle,
                        generation: v.owner.generation,
                        handler,
                    },
                );
            }
            v.started = true;
        } else {
            return Err("unknown Crew Start class".into());
        }
        Ok(())
    }
    pub(super) fn stop_crew(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let id = self.crew_entity(handle)?;
        if handle.class_id == self.classes[29] {
            let component = self
                .crew_cores
                .get_mut(handle.slot)
                .ok_or("CrewCore payload absent")?;
            if !component.started {
                return Ok(());
            }
            for handler in [0x27, 0x5c] {
                events.unregister(
                    handler,
                    Some(id),
                    Listener::Component {
                        owner: self.owner_id,
                        handle,
                        generation: component.owner.generation,
                        handler,
                    },
                );
            }
            component.members.clear();
            component.reset_requests();
            component.requester = None;
            component.request_active = false;
            component.members_changed = false;
            events.post(Event {
                id: 0x57,
                target: Some(id),
                payload: Payload::CrewBinding {
                    entity: component.binding,
                    name: component.binding_name.clone(),
                },
            });
            let decision = component.decision;
            component.started = false;
            if let Some(decision) = decision {
                if self.checked_crew_component(id, 32)? != Some(decision) {
                    return Err("CrewCore Stop Decision generation differs".into());
                }
                let v = self
                    .crew_decisions
                    .get_mut(decision.handle.slot)
                    .ok_or("CrewCore Stop Decision absent")?;
                let Definition::Decision { tree } = &*v.definition else {
                    return Err("CrewCore Stop Decision declaration differs".into());
                };
                v.pending_tree = Some(tree.clone());
            }
        } else if handle.class_id == self.classes[30] {
            let v = self
                .crew_knowledge
                .get_mut(handle.slot)
                .ok_or("CrewKnowledge payload absent")?;
            v.reset();
            v.started = false;
        } else if handle.class_id == self.classes[31] {
            let v = self
                .crew_coordination
                .get_mut(handle.slot)
                .ok_or("CrewCoordination payload absent")?;
            v.reset();
            v.started = false;
        } else if handle.class_id == self.classes[32] {
            let v = self
                .crew_decisions
                .get_mut(handle.slot)
                .ok_or("CrewDecision payload absent")?;
            if !v.started {
                return Ok(());
            }
            if let Some(runtime) = v.runtime.as_mut() {
                runtime.stop()
            }
            v.runtime = None;
            v.pending_tree = None;
            for handler in [0x25, 0x23] {
                events.unregister(
                    handler,
                    Some(id),
                    Listener::Component {
                        owner: self.owner_id,
                        handle,
                        generation: v.owner.generation,
                        handler,
                    },
                );
            }
            v.started = false;
        } else {
            return Err("unknown Crew Stop class".into());
        }
        Ok(())
    }
    pub(super) fn deinitialize_crew(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if handle.class_id == self.classes[29] {
            let v = self
                .crew_cores
                .get_mut(handle.slot)
                .ok_or("CrewCore payload absent")?;
            v.runtime = None;
            v.decision = None;
            v.knowledge = None;
            v.initialized = false;
        } else if handle.class_id == self.classes[30] {
            self.crew_knowledge
                .get_mut(handle.slot)
                .ok_or("CrewKnowledge payload absent")?
                .initialized = false;
        } else if handle.class_id == self.classes[31] {
            self.crew_coordination
                .get_mut(handle.slot)
                .ok_or("CrewCoordination payload absent")?
                .initialized = false;
        } else if handle.class_id == self.classes[32] {
            self.crew_decisions
                .get_mut(handle.slot)
                .ok_or("CrewDecision payload absent")?
                .initialized = false;
        } else {
            return Err("unknown Crew Deinitialize class".into());
        }
        Ok(())
    }
    pub(super) fn crew_message(&mut self, listener: Listener, event: &Event) -> Result<(), String> {
        let Listener::Component {
            owner,
            handle,
            generation,
            handler,
        } = listener
        else {
            return Err("Crew listener is not its actual component".into());
        };
        if owner != self.owner_id {
            return Err("Crew listener owner differs".into());
        }
        let id = self.crew_entity(handle)?;
        if handle.class_id == self.classes[32] {
            let v = self
                .crew_decisions
                .get_mut(handle.slot)
                .ok_or("CrewDecision callback payload absent")?;
            if v.owner.generation != generation || !v.started {
                return Ok(());
            }
            if event.target != Some(id) || event.id != handler {
                return Err("CrewDecision callback target or handler differs".into());
            }
            return match handler {
                0x25 => {
                    let Payload::CrewTree(tree) = &event.payload else {
                        return Err("CrewDecision25 has no compiled-tree packet".into());
                    };
                    let Definition::Decision { tree: default_tree } = &*v.definition else {
                        return Err("CrewDecision declaration differs".into());
                    };
                    v.pending_tree = Some(tree.clone().unwrap_or_else(|| default_tree.clone()));
                    Ok(())
                }
                0x23 => {
                    let Payload::Bytes(bytes) = &event.payload else {
                        return Err("CrewDecision23 stimulus packet is not source bytes".into());
                    };
                    let bytes: [u8; 4] = bytes
                        .as_slice()
                        .try_into()
                        .map_err(|_| "CrewDecision23 packet length differs")?;
                    if let Some(runtime) = v.runtime.as_mut() {
                        runtime.stimulus(u32::from_le_bytes(bytes))?;
                    }
                    Ok(())
                }
                _ => Err("CrewDecision has no such source subscription".into()),
            };
        }
        if handle.class_id != self.classes[29] {
            return Err("Crew listener class differs".into());
        }
        let component = self
            .crew_cores
            .get(handle.slot)
            .ok_or("CrewCore callback payload absent")?;
        if component.owner.generation != generation || !component.started {
            return Ok(());
        }
        if event.target != Some(id) || event.id != handler {
            return Err("CrewCore callback target or handler differs".into());
        }
        match handler {
            0x27 => {
                let runtime = component
                    .runtime
                    .as_ref()
                    .ok_or("CrewCore rule runtime absent")?;
                if runtime.instances.is_empty() {
                    //57aa06..57aa16 tail-jumps to166830 after restoring the
                    //stack. Returning from that diagnostic leaves the current
                    //Decision tree untouched; it does not dereference NULL.
                    bevy::log::warn!("arkCpntAICrewCore::EvaluateSituationTree - Evaluation failed");
                    return Ok(());
                }
                Err("Crew situation evaluation requires its admitted objective/action owner".into())
            }
            0x5c => {
                let Payload::Entity(selected) = event.payload else {
                    return Err("CrewCore5C has no actual selected-entity packet".into());
                };
                let knowledge = component
                    .knowledge
                    .ok_or("CrewCore Knowledge callback target absent")?;
                let value = self.crew_knowledge_mut(id, knowledge)?;
                value.selected = Some(selected);
                value.flags[0x1a6 - 0x198] = true;
                Ok(())
            }
            _ => Err("CrewCore has no such source subscription".into()),
        }
    }
    ///952060 visits only the real accepted prefix for phase3. Time is the
    ///actual owning entity clock8778030, not wall time or a copied AI clock.
    pub(in crate::app) fn update_crew_decisions(
        &mut self,
        world: &bevy::prelude::World,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let clock = world
            .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("CrewDecision entity clock owner absent")?;
        if !clock.advanced() {
            return Ok(());
        }
        let marker=clock.clock().map_err(str::to_owned)?.frame_marker();
        let count = self.crew_decisions.roster.sorted_start as usize;
        for cursor in 0..count {
            let slot = self.crew_decisions.roster.indices[cursor];
            let handle = ComponentHandle {
                class_id: self.classes[32],
                slot,
            };
            let id = self.crew_entity(handle)?;
            let time = clock.entity_time(id).map_err(str::to_owned)?;
            let v = self
                .crew_decisions
                .get_mut(slot)
                .ok_or("CrewDecision accepted payload absent")?;
            if v.updated_frame==Some(marker){continue;}
            if !v.started {
                return Err("CrewDecision stopped payload remains in accepted phase3 roster".into());
            }
            let runtime = v
                .runtime
                .as_mut()
                .ok_or("CrewDecision live interpreter absent")?;
            if let Some(tree) = v
                .pending_tree
                .as_ref()
                .filter(|tree| !Arc::ptr_eq(tree, runtime.definition()))
            {
                let mut replacement = crate::gameplay::ai_world::crew_tree::Instance::new(
                    tree.clone(),
                    id,
                    runtime.has_ai_core(),
                );
                replacement.start()?;
                runtime.stop();
                *runtime = replacement;
                events.post(Event {
                    id: 0x26,
                    target: Some(id),
                    payload: Payload::CrewTree(Some(tree.clone())),
                });
                v.pending_tree = None;
            }
            let result = runtime.update(time)?;
            if result != 3 {
                runtime.reset();
                let Definition::Decision { tree: default_tree } = &*v.definition else {
                    return Err("CrewDecision declaration differs".into());
                };
                let is_default = Arc::ptr_eq(runtime.definition(), default_tree);
                events.post(Event {
                    id: 0x27,
                    target: Some(id),
                    payload: Payload::CrewTreeComplete {
                        default_tree: is_default,
                        result: i32::from(result),
                        entity: id,
                    },
                });
                if !is_default {
                    v.pending_tree = Some(default_tree.clone());
                }
            }
            v.updated_frame=Some(marker);
        }
        Ok(())
    }
    fn checked_crew_component(
        &self,
        id: EntityId,
        slot: usize,
    ) -> Result<Option<CheckedComponent>, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let entity = self
            .entities
            .get(index)
            .filter(|entity| !entity.retired && entity.shared == id)
            .ok_or("Crew actor generation stale")?;
        for &word in entity.components.entries.values() {
            let handle = ComponentHandle::from_packed(word)?;
            if handle.class_id == self.classes[slot] {
                return Ok(Some(CheckedComponent {
                    handle,
                    generation: self.owner(handle)?.generation,
                }));
            }
        }
        Ok(None)
    }
    pub(super) fn crew_core_for_entity(
        &self,
        id: EntityId,
    ) -> Result<Option<CheckedComponent>, String> {
        self.checked_crew_component(id, 29)
    }
    pub(super) fn crew_knowledge_for_entity(
        &self,
        id: EntityId,
    ) -> Result<Option<CheckedComponent>, String> {
        self.checked_crew_component(id, 30)
    }
    pub(super) fn crew_coordination_for_entity(
        &self,
        id: EntityId,
    ) -> Result<Option<CheckedComponent>, String> {
        self.checked_crew_component(id, 31)
    }
    pub(super) fn crew_herd_for_entity(
        &self,
        id: EntityId,
    ) -> Result<Option<CheckedComponent>, String> {
        self.checked_crew_component(id, 37)
    }
    pub(super) fn crew_core(
        &self,
        id: EntityId,
        checked: CheckedComponent,
    ) -> Result<&crew::Core, String> {
        if self.crew_core_for_entity(id)? != Some(checked) {
            return Err("CrewCore generation differs".into());
        }
        self.crew_cores
            .get(checked.handle.slot)
            .ok_or("CrewCore payload absent".into())
    }
    pub(super) fn crew_knowledge_mut(
        &mut self,
        id: EntityId,
        checked: CheckedComponent,
    ) -> Result<&mut crew::Knowledge, String> {
        if self.crew_knowledge_for_entity(id)? != Some(checked) {
            return Err("CrewKnowledge generation differs".into());
        }
        self.crew_knowledge
            .get_mut(checked.handle.slot)
            .ok_or("CrewKnowledge payload absent".into())
    }
    pub(super) fn crew_coordination_mut(
        &mut self,
        id: EntityId,
        checked: CheckedComponent,
    ) -> Result<&mut crew::Coordination, String> {
        if self.crew_coordination_for_entity(id)? != Some(checked) {
            return Err("CrewCoordination generation differs".into());
        }
        self.crew_coordination
            .get_mut(checked.handle.slot)
            .ok_or("CrewCoordination payload absent".into())
    }
}
