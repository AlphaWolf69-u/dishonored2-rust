//! Concrete Cues child component startup6002e0/600420/600490/5d9430.
use super::*;
use crate::gameplay::{
    ai_world::Worlds,
    component_state::entity_lookup::ComponentLookupState,
    entity_events::{Commands, EntityEvents, Event, Listener, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    pickable_component::CheckedComponent,
};
use bevy::prelude::World;

impl PhysicsScene {
    pub(super) fn create_observable(
        &mut self,
        index: usize,
        definition: Arc<crate::assets::ai_cues::ObservableDefinition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.observables,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| {
                Ok(crate::gameplay::ai_cues::Observable::new(
                    definition.clone(),
                ))
            },
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn create_attention_source(
        &mut self,
        index: usize,
        definition: Arc<crate::assets::ai_cues::AttentionDefinition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.attention_sources,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| {
                Ok(crate::gameplay::ai_cues::AttentionSource::new(
                    definition.clone(),
                ))
            },
            |_, _, _| Ok(()),
        )
    }
    fn cue_component_identity(
        &self,
        handle: ComponentHandle,
    ) -> Result<(usize, EntityId, CheckedComponent), String> {
        let owner = self.owner(handle)?;
        let index = owner.entity.ok_or("cue component has no entity")?;
        Ok((
            index,
            self.entities[index].shared,
            CheckedComponent {
                handle,
                generation: owner.generation,
            },
        ))
    }
    fn observable_payload(&self, handle: ComponentHandle) -> Result<Payload, String> {
        let (_, entity, component) = self.cue_component_identity(handle)?;
        let category = self
            .observables
            .get(handle.slot)
            .ok_or("Observable payload absent")?
            .definition
            .category;
        Ok(Payload::AIObservable {
            entity,
            component,
            category,
        })
    }
    pub(super) fn cue_dependency(
        &self,
        index: usize,
        name: &[u8],
        masks: [u32; 2],
    ) -> Result<Option<CheckedComponent>, String> {
        let classes = self.types.classes();
        let class = classes
            .hierarchy()
            .registration_order()
            .iter()
            .copied()
            .find(|&class| {
                classes
                    .metadata(class)
                    .and_then(|m| classes.name_text(m.name))
                    == Some(name)
            })
            .ok_or("Observable dependency class is not registered")?;
        self.entities[index]
            .components
            .find_component(classes.hierarchy().type_range(class)?, masks, |h| {
                Ok(ComponentLookupState {
                    flags: self.owner(h)?.flags,
                    start_phases: self
                        .declarations
                        .get(&h.packed())
                        .map(|d| d.scheduling.start_phases),
                })
            })?
            .map(|handle| {
                Ok(CheckedComponent {
                    handle,
                    generation: self.owner(handle)?.generation,
                })
            })
            .transpose()
    }
    pub(super) fn initialize_cue_component(
        &mut self,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let (index, id, _) = self.cue_component_identity(handle)?;
        let (_, class) = live
            .get(id)
            .ok_or("cue component entity generation stale")?;
        if handle.class_id == self.classes[49] {
            let component = self
                .attention_sources
                .get(handle.slot)
                .ok_or("AttentionSource absent")?;
            if component.initialized {
                return Ok(());
            }
            let meter = if component.definition.default_meter != -1 {
                component.definition.default_meter
            } else if self.throwable_class_is(class, b"arkPlayer")? {
                0
            } else if self.throwable_class_is(class, b"arkNPC")? {
                1
            } else {
                -1
            };
            let component = self.attention_sources.get_mut(handle.slot).unwrap();
            component.meter = meter;
            component.initialized = true;
            component.owner.mark_initialized();
            return Ok(());
        }
        let component = self
            .observables
            .get(handle.slot)
            .ok_or("Observable absent")?;
        if component.initialized {
            return Ok(());
        }
        let dependency = match component.definition.category {
            0 => Some(b"arkCpntPlayerPhysics".as_slice()),
            1 => Some(b"arkCpntPhysicsSystemEvolved".as_slice()),
            4 => Some(b"arkCpntMesmerize".as_slice()),
            5 | 6 => Some(b"arkCpntSwarmBase".as_slice()),
            _ => None,
        };
        let attachment = dependency
            .map(|name| self.cue_dependency(index, name, [0, 0]))
            .transpose()?
            .flatten();
        if dependency.is_some() && attachment.is_none() {
            //Installed166830 returns: the source logs its missing attachment
            //then stores the actual null reference and continues initialization.
            bevy::log::warn!("Observable {} missing authored attachment component",self.entities[index].name);
        }
        let rigid = self.document_rigid(index)?;
        let body = rigid
            .map(|h| {
                self.rigid_component(h)
                    .map(|c| c.body().unwrap_or(0x00ff_ffff))
            })
            .transpose()?;
        let component = self.observables.get_mut(handle.slot).unwrap();
        component.attachment = attachment;
        component.bodies.clear();
        //Rigid's actual primary vtable+d0=1156ea0 returns1, +d8=4c7130
        //copies the current source+f8 full body ID, including invalid before Init.
        if let Some(body) = body {
            component.bodies.push(body);
        }
        component.initialized = true;
        component.owner.mark_initialized();
        Ok(())
    }
    pub(super) fn start_cue_component(
        &mut self,
        world: Option<&World>,
        live: &LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id == self.classes[49] {
            self.attention_sources
                .get_mut(handle.slot)
                .ok_or("AttentionSource absent")?
                .started = true;
            return Ok(());
        }
        let (_, id, checked) = self.cue_component_identity(handle)?;
        let component = self
            .observables
            .get(handle.slot)
            .ok_or("Observable absent")?;
        if component.started {
            return Ok(());
        }
        let recipient = world
            .ok_or("Observable Start requires canonical AIWorld owner")?
            .get_resource::<Worlds>()
            .ok_or("Observable AIWorld owner absent")?
            .recipient(live, id)?;
        let payload = self.observable_payload(handle)?;
        let component = self.observables.get_mut(handle.slot).unwrap();
        component.enabled_count = component.enabled_count.wrapping_add(1);
        if component.enabled_count == 1 {
            events.post(Event {
                id: 0x1be,
                target: Some(recipient),
                payload,
            });
        }
        events.register(
            0xec,
            Some(id),
            Listener::Component {
                owner: self.owner_id,
                handle,
                generation: checked.generation,
                handler: 0xec,
            },
        );
        component.started = true;
        Ok(())
    }
    pub(super) fn stop_cue_component(
        &mut self,
        world: Option<&World>,
        live: &LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if handle.class_id == self.classes[49] {
            self.attention_sources
                .get_mut(handle.slot)
                .ok_or("AttentionSource absent")?
                .started = false;
            return Ok(());
        }
        let (_, id, checked) = self.cue_component_identity(handle)?;
        let component = self
            .observables
            .get(handle.slot)
            .ok_or("Observable absent")?;
        if !component.started {
            return Ok(());
        }
        let recipient = if component.enabled_count > 0 {
            Some(
                world
                    .ok_or("Observable Stop requires AIWorld owner")?
                    .get_resource::<Worlds>()
                    .ok_or("Observable AIWorld owner absent")?
                    .recipient(live, id)?,
            )
        } else {
            None
        };
        if let Some(recipient) = recipient {
            events.post(Event {
                id: 0x1bf,
                target: Some(recipient),
                payload: self.observable_payload(handle)?,
            });
        }
        events.unregister(
            0xec,
            Some(id),
            Listener::Component {
                owner: self.owner_id,
                handle,
                generation: checked.generation,
                handler: 0xec,
            },
        );
        let component = self.observables.get_mut(handle.slot).unwrap();
        component.enabled_count = 0;
        component.disabled_remaining = 0.;
        component.started = false;
        Ok(())
    }
    pub(super) fn cue_component_stop_complete(
        &self,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        if handle.class_id == self.classes[49] {
            return Ok(true);
        }
        let (_, entity, component) = self.cue_component_identity(handle)?;
        //Host lifetime acknowledgement: original queued1BF retains this real
        //pointer until the shared FIFO mutates Vision's same generation roster.
        for position in 0..self.ai_vision.allocated_count() {
            let slot = self.ai_vision.roster.indices[position];
            let vision = self
                .ai_vision
                .get(slot)
                .ok_or("Vision roster payload absent")?;
            if vision
                .rosters
                .iter()
                .flatten()
                .any(|r| r.entity == entity && r.component == component)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn deinitialize_cue_component(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if !self.cue_component_stop_complete(handle)? {
            return Err("Observable Deinit precedes Vision removal acknowledgement".into());
        }
        if handle.class_id == self.classes[49] {
            let component = self
                .attention_sources
                .get_mut(handle.slot)
                .ok_or("AttentionSource absent")?;
            component.meter = -1;
            component.initialized = false;
        } else {
            //NativeDeinit is base7e7530; the source copied body-ID list and
            //attachment storage are owned through the eventual destructor.
            self.observables
                .get_mut(handle.slot)
                .ok_or("Observable absent")?
                .initialized = false;
        }
        Ok(())
    }
    pub(super) fn dispatch_observable_event(
        &mut self,
        live: &LiveEntities,
        worlds: &Worlds,
        listener: Listener,
        event: &Event,
        commands: &mut Commands,
    ) -> Result<(), String> {
        let Listener::Component {
            owner,
            handle,
            generation,
            handler: 0xec,
        } = listener
        else {
            return Err("Observable callback identity differs".into());
        };
        let (_, id, checked) = self.cue_component_identity(handle)?;
        if owner != self.owner_id
            || generation != checked.generation
            || event.target != Some(id)
            || event.id != 0xec
            || live.get(id).is_none()
        {
            return Err("Observable callback generation differs".into());
        }
        let component = self
            .observables
            .get(handle.slot)
            .ok_or("Observable callback absent")?;
        if component.enabled_count <= 0 {
            return Ok(());
        }
        let Payload::Bytes(bytes) = &event.payload else {
            return Err("Observable EC requires actual old/new area packet".into());
        };
        let bytes: &[u8; 8] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "Observable EC area packet length differs")?;
        let previous = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        let current = u32::from_le_bytes(bytes[4..].try_into().unwrap()) as usize;
        for (area, message) in [(previous, 0x1bf), (current, 0x1be)] {
            let recipient = worlds
                .entries()
                .get(area)
                .ok_or("Observable EC AIWorld index invalid")?
                .entity;
            commands.post(Event {
                id: message,
                target: Some(recipient),
                payload: self.observable_payload(handle)?,
            });
        }
        Ok(())
    }
}
