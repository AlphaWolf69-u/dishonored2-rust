//! Actual arkCpntBreakable pool payload and checked numeric event callbacks.
//! The central runtime entity factory owns spawned chunks, bodies and models.
use super::*;
use crate::{
    assets::breakable::{self as assets, Definition},
    gameplay::{
        breakable::{Effect, Health, Message, Runtime},
        component_state::ComponentInstance,
        entity_events::{EntityEvents, Event, Listener, Payload},
        entity_lifetime::{EntityId, LiveEntities, Stage},
        pickable_component::CheckedComponent,
    },
};

pub(super) struct BreakableComponent {
    pub owner: ComponentOwner,
    pub runtime: Runtime,
    pub health: Option<CheckedComponent>,
    pub rigid: Option<CheckedComponent>,
    listeners: bool,
}
impl BreakableComponent {
    pub(super) fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: Default::default(),
            runtime: Runtime::new(definition),
            health: None,
            rigid: None,
            listeners: false,
        }
    }
}
impl ComponentInstance for BreakableComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl PhysicsScene {
    pub(super) fn admit_breakable_components(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        for (source, binding) in assets::bindings(catalog, document)?.into_iter().enumerate() {
            let Some(index) = self
                .entities
                .iter()
                .position(|entity| entity.name == binding.entity_id)
            else {
                continue;
            };
            self.admit_breakable(
                index,
                binding.definition,
                binding.scheduling,
                source,
                registry,
            )?;
        }
        Ok(())
    }
    /// Used by the SAME actor factory for actual runtime chunk/projectile props.
    pub(super) fn admit_breakable(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        scheduling: DeclarationScheduling,
        source: usize,
        registry: &mut ComponentRegistry,
    ) -> Result<ComponentHandle, String> {
        let declaration = self
            .types
            .declaration(self.factories[11], source, scheduling)?;
        let handle = ComponentFactory {
            declaration: &declaration,
            pool: &mut self.breakables,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self
                .entities
                .get_mut(index)
                .ok_or("breakable actor absent")?
                .components,
            declaration.scheduling.starts_on_spawn(),
            |_, _, _| Ok(BreakableComponent::new(definition)),
            |_, _, _| Ok(()),
        )?
        .ok_or("breakable component creation disabled")?;
        self.declarations.insert(handle.packed(), declaration);
        Ok(handle)
    }
    pub(in crate::app) fn breakable_component(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<ComponentHandle>, String> {
        if live.stage(id) != Some(Stage::Spawned) {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        self.entities[index]
            .components
            .find_component([self.classes[11]; 2], [0x20, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|decl| decl.scheduling.start_phases),
                    },
                )
            })
    }
    ///7ed430 constructs every non-null authored actor before Start. The
    /// central factory installs each generation with install_breakable_chunk.
    pub(super) fn initialize_breakable(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable initialization slot absent")?;
        component.runtime.finish_initialize()?;
        component.owner.mark_initialized();
        Ok(())
    }
    pub(in crate::app) fn install_breakable_chunk(
        &mut self,
        handle: ComponentHandle,
        generation: u32,
        step: usize,
        chunk: usize,
        entity: EntityId,
    ) -> Result<(), String> {
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable chunk owner absent")?;
        if handle.class_id != self.classes[11] || component.owner.generation != generation {
            return Err("breakable chunk owner generation stale".into());
        }
        component.runtime.install_chunk(step, chunk, entity)
    }
    pub(super) fn start_breakable(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let index = self
            .owner(handle)?
            .entity
            .ok_or("breakable Start owner absent")?;
        let lookup = |handle| -> Result<_, String> {
            Ok(
                crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                    flags: self.owner(handle)?.flags,
                    start_phases: self
                        .declarations
                        .get(&handle.packed())
                        .map(|decl| decl.scheduling.start_phases),
                },
            )
        };
        let health = self.entities[index]
            .components
            .find_component([self.classes[10]; 2], [0x20, 0], lookup)?
            .ok_or("Breakable requires an actual started Health component (7ee8d0)")?;
        let health_owner = self.owner(health)?;
        if health_owner.flags & 0xc < 8 {
            return Err("Breakable Health dependency is not started".into());
        }
        let checked_health = CheckedComponent {
            handle: health,
            generation: health_owner.generation,
        };
        let rigid = self.entities[index]
            .components
            .find_component(self.rigid_class_range()?, [0, 0], lookup)?
            .map(|handle| -> Result<CheckedComponent, String> {
                Ok(CheckedComponent {
                    handle,
                    generation: self.owner(handle)?.generation,
                })
            })
            .transpose()?;
        let id = self.entities[index].shared;
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable Start slot absent")?;
        component.runtime.start()?;
        component.health = Some(checked_health);
        component.rigid = rigid;
        if !component.listeners {
            for handler in [0xa2, 0x1a6, 0xa5] {
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
            component.listeners = true;
        }
        self.entities[index].entity_flags_d7 |= 8;
        if component.runtime.definition.allow_throw_hits {
            self.entities[index].entity_flags_d7 |= 0x10;
        }
        Ok(())
    }
    pub(super) fn stop_breakable(
        &mut self,
        handle: ComponentHandle,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let index = self
            .owner(handle)?
            .entity
            .ok_or("breakable Stop owner absent")?;
        let id = self.entities[index].shared;
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable Stop slot absent")?;
        if component.listeners {
            for handler in [0xa2, 0x1a6, 0xa5] {
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
            component.listeners = false;
        }
        component.health = None;
        component.rigid = None;
        component.runtime.stop();
        self.entities[index].entity_flags_d7 &= !0x18;
        Ok(())
    }
    pub(super) fn breakable_message(
        &mut self,
        live: &LiveEntities,
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
            return Err("breakable callback has no real component owner".into());
        };
        if owner != self.owner_id || handle.class_id != self.classes[11] || event.id != handler {
            return Err("breakable callback owner/handler mismatch".into());
        }
        let component = self
            .breakables
            .get(handle.slot)
            .ok_or("breakable callback slot absent")?;
        if component.owner.generation != generation || component.owner.flags & 0xc < 8 {
            return Err("breakable callback generation stopped or stale".into());
        }
        let index = component
            .owner
            .entity
            .ok_or("breakable callback unlinked")?;
        let id = self.entities[index].shared;
        if event.target != Some(id) || live.stage(id) != Some(Stage::Spawned) {
            return Err("breakable callback entity generation stale".into());
        }
        let checked = component
            .health
            .ok_or("breakable Health dependency absent")?;
        let health_owner = self.owner(checked.handle)?;
        if checked.handle.class_id != self.classes[10]
            || health_owner.generation != checked.generation
            || health_owner.entity != Some(index)
            || health_owner.flags & 0xc < 8
        {
            return Err("breakable checked Health dependency stale".into());
        }
        let health = self
            .health
            .get(checked.handle.slot)
            .ok_or("breakable Health payload absent")?;
        let state = Health {
            current: health.current,
            infinite: health.receiver.infinite_health,
        };
        let (message, params, request_update) = match (handler, &event.payload) {
            (0xa2, Payload::Damage { params, result }) => (
                Message::Damaged(*result),
                *params,
                result.health_damage > 0.,
            ),
            (0x1a6, Payload::DamageParameters(params)) => {
                (Message::DamageAfterDeath, *params, false)
            }
            (0xa5, Payload::DamageParameters(params)) => (Message::Incapacitated, *params, true),
            _ => return Err("breakable callback lacks its actual typed damage payload".into()),
        };
        self.breakables
            .get_mut(handle.slot)
            .ok_or("breakable callback disappeared")?
            .runtime
            .message(id, message, state, params)?;
        if request_update {
            self.entity_flags_d4[index] |= 4;
        }
        Ok(())
    }
    ///7ee050 asks the same real Health receiver without applying damage.
    /// Type acceptance precedes calculation; a thrown override is independent
    /// of the calculation's directional rejection, exactly as7edfe0.
    pub(in crate::app) fn breakable_would_break(
        &self,
        live: &LiveEntities,
        id: EntityId,
        params: &crate::gameplay::damage::Parameters,
        last_only: bool,
    ) -> Result<bool, String> {
        let Some(handle) = self.breakable_component(live, id)? else {
            return Ok(false);
        };
        let component = self
            .breakables
            .get(handle.slot)
            .ok_or("breakable prediction slot absent")?;
        let checked = component
            .health
            .ok_or("breakable prediction Health reference absent")?;
        let health = self
            .health
            .get(checked.handle.slot)
            .ok_or("breakable prediction Health slot absent")?;
        let index = component
            .owner
            .entity
            .ok_or("breakable prediction owner absent")?;
        if checked.handle.class_id != self.classes[10]
            || health.owner.generation != checked.generation
            || health.owner.entity != Some(index)
            || health.owner.flags & 0xc < 8
        {
            return Err("breakable prediction Health generation stale".into());
        }
        let context = crate::gameplay::damage::Context {
            forward: bevy_math::Vec3::from_array(self.entities[index].frame.pose.basis[0]),
            started: true,
            ..Default::default()
        };
        let result = health.receiver.evaluate(health.current, params, context);
        Ok(component.runtime.predicts_break(
            Health {
                current: health.current,
                infinite: health.receiver.infinite_health,
            },
            result,
            health.receiver.accepts(params.damage_type),
            last_only,
        ))
    }
    /// Expose only a started actual component to Blink/throwable classification.
    pub(in crate::app) fn is_breakable(&self, live: &LiveEntities, id: EntityId) -> bool {
        self.breakable_component(live, id).ok().flatten().is_some()
    }
    pub(in crate::app) fn set_breakable_thrown(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        thrown: bool,
    ) -> Result<bool, String> {
        let Some(handle) = self.breakable_component(live, id)? else {
            return Ok(false);
        };
        self.breakables
            .get_mut(handle.slot)
            .ok_or("breakable thrown slot absent")?
            .runtime
            .set_thrown(thrown);
        Ok(true)
    }
    /// The exclusive host borrows this source command, then acknowledges it
    /// only after its canonical entity/model/physics/FX operation succeeds.
    pub(in crate::app) fn pending_breakable_effect(
        &self,
    ) -> Option<(ComponentHandle, u32, &Effect)> {
        self.entities
            .iter()
            .flat_map(|entity| entity.components.entries.values())
            .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
            .filter(|handle| handle.class_id == self.classes[11])
            .find_map(|handle| {
                let component = self.breakables.get(handle.slot)?;
                Some((
                    handle,
                    component.owner.generation,
                    component.runtime.pending()?,
                ))
            })
    }
    pub(in crate::app) fn acknowledge_breakable_effect(
        &mut self,
        handle: ComponentHandle,
        generation: u32,
    ) -> Result<(), String> {
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable effect owner absent")?;
        if component.owner.generation != generation {
            return Err("breakable effect owner generation changed".into());
        }
        component.runtime.acknowledge()
    }
}
