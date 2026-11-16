//! Concrete CrewModule584b40/584ba0 and subtype startup callbacks.
use super::*;
use crate::{
    assets::ai_world::crew_modules::Definition,
    gameplay::{
        ai_world::crew_modules::{Component, State},
        component_state::entity_lookup::ComponentLookupState,
        entity_events::{EntityEvents, Event, Listener},
        entity_lifetime::{EntityId, LiveEntities},
        pickable_component::CheckedComponent,
    },
};
macro_rules! module_pools{($(($index:literal,$field:ident)),+)=>{
    impl PhysicsScene{
        fn module_pool(&self,class:i32)->Result<&ComponentPool<Component>,String>{
            $(if class==self.classes[$index]{return Ok(&self.$field);})+Err("CrewModule class absent".into())
        }
        fn module_parts_mut(&mut self,class:i32)->Result<(&mut ComponentPool<Component>,&mut Vec<PhysicsEntity>),String>{
            $(if class==self.classes[$index]{return Ok((&mut self.$field,&mut self.entities));})+Err("CrewModule class absent".into())
        }
    }
}}
module_pools!(
    (38, crew_chase),
    (39, crew_melee),
    (40, crew_search_module),
    (41, crew_link),
    (42, crew_ranged),
    (43, crew_fake),
    (44, crew_flank),
    (45, crew_hideout),
    (46, crew_tether),
    (47, crew_alarm)
);
impl PhysicsScene {
    pub(super) fn is_crew_module_class(&self, class: i32) -> bool {
        self.classes[38..48].contains(&class)
    }
    fn module(&self, handle: ComponentHandle) -> Result<&Component, String> {
        self.module_pool(handle.class_id)?
            .get(handle.slot)
            .ok_or("CrewModule payload absent".into())
    }
    fn module_mut(&mut self, handle: ComponentHandle) -> Result<&mut Component, String> {
        self.module_parts_mut(handle.class_id)?
            .0
            .get_mut(handle.slot)
            .ok_or("CrewModule payload absent".into())
    }
    pub(super) fn module_owner(&self, handle: ComponentHandle) -> Option<&ComponentOwner> {
        self.module(handle).ok().map(|component| &component.owner)
    }
    pub(super) fn module_owner_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Option<&mut ComponentOwner> {
        self.module_mut(handle)
            .ok()
            .map(|component| &mut component.owner)
    }
    pub(super) fn schedule_crew_module(&mut self, handle: ComponentHandle) -> Result<(), String> {
        self.module_parts_mut(handle.class_id)?
            .0
            .schedule(handle.slot)
    }
    pub(super) fn create_crew_module(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        if declaration.class_id != self.classes[definition.pool_index()] {
            return Err("CrewModule source declaration class differs from its real factory".into());
        }
        let (pool, entities) = self.module_parts_mut(declaration.class_id)?;
        ComponentFactory {
            declaration,
            pool,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut entities[index].components,
            immediate,
            |_, _, _| Ok(Component::new(definition.clone())),
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn release_crew_module(
        &mut self,
        index: usize,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (pool, entities) = self.module_parts_mut(handle.class_id)?;
        entities[index].components.release(
            handle,
            pool.get_mut(handle.slot)
                .ok_or("CrewModule release payload absent")?,
        )
    }
    pub(super) fn reclaim_crew_module(&mut self, handle: ComponentHandle) -> Result<(), String> {
        self.module_parts_mut(handle.class_id)?
            .0
            .release(handle.slot)
    }
    fn module_identity(
        &self,
        handle: ComponentHandle,
    ) -> Result<(EntityId, CheckedComponent), String> {
        let owner = &self.module(handle)?.owner;
        let entity = owner
            .entity
            .and_then(|index| self.entities.get(index))
            .ok_or("CrewModule actor absent")?
            .shared;
        Ok((
            entity,
            CheckedComponent {
                handle,
                generation: owner.generation,
            },
        ))
    }
    fn module_for(&self, id: EntityId, index: usize) -> Result<Option<CheckedComponent>, String> {
        let Some(&entity) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let handle = self.entities[entity].components.find_component(
            [self.classes[index]; 2],
            [0, 0],
            |handle| {
                Ok(ComponentLookupState {
                    flags: self.owner(handle)?.flags,
                    start_phases: self
                        .declarations
                        .get(&handle.packed())
                        .map(|value| value.scheduling.start_phases),
                })
            },
        )?;
        handle
            .map(|handle| {
                Ok(CheckedComponent {
                    handle,
                    generation: self.owner(handle)?.generation,
                })
            })
            .transpose()
    }
    pub(super) fn initialize_crew_module(&mut self, handle: ComponentHandle) -> Result<(), String> {
        if self.module(handle)?.initialized {
            return Ok(());
        }
        let (entity, _) = self.module_identity(handle)?;
        let core = self.crew_core_for_entity(entity)?;
        let coordination = self.crew_coordination_for_entity(entity)?;
        let knowledge = self.crew_knowledge_for_entity(entity)?;
        let herd = if matches!(self.module(handle)?.state, State::Chase { .. }) {
            self.crew_herd_for_entity(entity)?
        } else {
            None
        };
        let component = self.module_mut(handle)?;
        component.core = core;
        component.coordination = coordination;
        component.knowledge = knowledge;
        if let State::Chase { herd: field, .. } = &mut component.state {
            *field = herd;
        }
        component.owner.mark_initialized();
        component.initialized = true;
        Ok(())
    }
    pub(super) fn deinitialize_crew_module(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        self.module_mut(handle)?.deinitialize();
        Ok(())
    }
    ///586a00 precedes the common7e75b0 update-list removal callback.
    pub(super) fn update_remove_crew_module(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let (entity, _) = self.module_identity(handle)?;
        if let State::Chase {
            herd: Some(herd), ..
        } = self.module(handle)?.state
        {
            self.reset_crew_herd_path(entity, herd, registry, live, events)?;
        }
        Ok(())
    }
    pub(super) fn start_crew_module(
        &mut self,
        events: &mut EntityEvents,
        _registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if self.module(handle)?.started {
            return Ok(());
        }
        if !self.module(handle)?.initialized {
            return Err("CrewModule Start precedes its actual Initialize".into());
        }
        let (entity, checked) = self.module_identity(handle)?;
        if matches!(self.module(handle)?.state, State::Tether { .. }) {
            let melee = self
                .module_for(entity, 39)?
                .ok_or("Tether requires the actual same-crew Melee module")?;
            let ranged = self
                .module_for(entity, 42)?
                .ok_or("Tether requires the actual same-crew Ranged module")?;
            let Definition::Melee { maximum: melee } = &*self.module(melee.handle)?.definition
            else {
                return Err("Tether melee declaration differs".into());
            };
            let Definition::Ranged { maximum: ranged } = &*self.module(ranged.handle)?.definition
            else {
                return Err("Tether ranged declaration differs".into());
            };
            let maximum = melee.wrapping_add(*ranged);
            if let State::Tether { maximum: field, .. } = &mut self.module_mut(handle)?.state {
                *field = maximum;
            }
        }
        if self.module(handle)?.owner.flags & (1 << 30) == 0 {
            if matches!(
                self.module(handle)?.state,
                State::Link(_) | State::Hideout { .. }
            ) {
                self.fill_crew_module_spots(handle)?;
            }
        }
        if matches!(self.module(handle)?.state, State::Alarm { .. }) {
            events.register(
                0x88,
                None,
                Listener::Component {
                    owner: self.owner_id,
                    handle,
                    generation: checked.generation,
                    handler: 0x88,
                },
            );
        }
        self.module_mut(handle)?.started = true;
        Ok(())
    }
    fn fill_crew_module_spots(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let (entity, _) = self.module_identity(handle)?;
        match &self.module(handle)?.state {
            State::Link(_) => {
                let core = self
                    .module(handle)?
                    .core
                    .ok_or("SpotLink requires actual Core")?;
                if self.crew_core(entity, core)?.binding.is_some() {
                    return Err(
                        "SpotLink binding requires its actual AISearchLink component reader".into(),
                    );
                }
            }
            State::Hideout { .. } => {
                //41c9d0 clears the free-list before testing actual knowledge
                //availability19c. This is not a fabricated no-NPC result.
                if let State::Hideout { spots, .. } = &mut self.module_mut(handle)?.state {
                    spots.available.clear();
                }
                let knowledge = self
                    .module(handle)?
                    .knowledge
                    .ok_or("SpotHideout requires actual KnowledgeBB")?;
                let knowledge = self.crew_knowledge_mut(entity, knowledge)?;
                if knowledge.flags[4] {
                    return Err(
                        "SpotHideout requires its published hideout-knowledge value reader".into(),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn stop_crew_module(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (_, checked) = self.module_identity(handle)?;
        let owner = self.owner_id;
        match &mut self.module_mut(handle)?.state {
            State::Link(spots) => {
                spots.stop();
            }
            State::Hideout { spots, .. } => {
                spots.stop();
            }
            State::Tether { maximum, .. } => *maximum = 0,
            State::Alarm { assignments, .. } => {
                if !assignments.is_empty() {
                    return Err(
                        "Alarm Stop requires actual assigned NPC scripted-action cancellation"
                            .into(),
                    );
                }
                assignments.shrink_to_fit();
                events.unregister(
                    0x88,
                    None,
                    Listener::Component {
                        owner,
                        handle,
                        generation: checked.generation,
                        handler: 0x88,
                    },
                );
            }
            _ => {}
        }
        self.module_mut(handle)?.started = false;
        Ok(())
    }
    pub(super) fn crew_module_message(
        &mut self,
        live: &LiveEntities,
        _registry: &mut ComponentRegistry,
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
            return Err("CrewModule listener differs".into());
        };
        if owner != self.owner_id {
            return Err("CrewModule listener owner differs".into());
        }
        let (entity, checked) = self.module_identity(handle)?;
        if generation != checked.generation
            || !self.module(handle)?.started
            || live.get(entity).is_none()
        {
            return Ok(());
        }
        if handler != 0x88 || event.id != 0x88 {
            return Err("CrewModule source callback is unsupported".into());
        }
        let State::Alarm { phase, .. } = &self.module(handle)?.state else {
            return Err("CrewModule88 receiver is not Alarm".into());
        };
        if *phase != 0 {
            return Ok(());
        }
        let Some(subject) = event.target.filter(|&id| live.get(id).is_some()) else {
            return Ok(());
        };
        let core = self
            .module(handle)?
            .core
            .ok_or("Alarm callback requires actual Core")?;
        if self.crew_core(entity, core)?.members.contains(&subject) {
            if let State::Alarm { phase, .. } = &mut self.module_mut(handle)?.state {
                *phase = 1;
            }
        }
        Ok(())
    }
}
