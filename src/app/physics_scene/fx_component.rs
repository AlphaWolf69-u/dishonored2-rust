//! Actual arkCpntFX actor wrapper (4b15d0/4b1c00/4b1cc0).
//! The existing renderer WorldFxComponent remains the sole FX creation,
//! target-update and stop kernel. This wrapper owns the real component/SELF
//! generation and its publication receipt through the shared FX lifetime.
use super::*;
use crate::{
    assets::fx_component::Definition,
    gameplay::{
        component_state::ComponentInstance,
        entity_events::{EntityEvents, Event, Listener, Payload},
        entity_lifetime::{EntityId, LiveEntities},
    },
};

pub(super) struct Publication {
    pub key: Arc<str>,
    pub ticket: super::super::usable_effects::EffectTicket,
    pub stopping: bool,
    pub detached: Arc<std::sync::atomic::AtomicBool>,
}

///The common FX publication pump borrows the same Scene/Live owners. It does
///not decode resources, allocate gameplay actors, or advance an FX kernel.
pub(in crate::app) fn update(world: &mut bevy::prelude::World) -> Result<(), String> {
    use super::super::usable_effects::{
        component_detach_receipt, effect_completed, EffectRequests, EntityPlacement,
    };
    use crate::gameplay::event_graph::EntityReference;
    use crate::rendering::bevy_backend::world_effects::{ComponentParameters, EntityEffectContext};
    if world
        .get_resource::<PhysicsScene>()
        .is_none_or(|scene| scene.fx_publications.is_empty())
        || !world.contains_resource::<LiveEntities>()
    {
        return Ok(());
    }
    world.resource_scope(|world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
        world.resource_scope(|world, mut live: bevy::prelude::Mut<LiveEntities>| {
            let mut position = 0;
            while position < scene.fx_publications.len() {
                let checked = scene.fx_publications[position];
                let slot = checked.handle.slot;
                if scene
                    .fx_components
                    .get(slot)
                    .is_none_or(|component| component.owner.generation != checked.generation)
                {
                    scene.fx_publications.remove(position);
                    continue;
                }
                let component = scene
                    .fx_components
                    .get(slot)
                    .ok_or("CpntFX roster payload absent")?;
                let index = component.owner.entity.ok_or("CpntFX roster owner absent")?;
                let id = scene.entities[index].shared;
                if component
                    .publication
                    .as_ref()
                    .is_some_and(|publication| effect_completed(&publication.ticket))
                {
                    let publication = scene
                        .fx_components
                        .get_mut(slot)
                        .unwrap()
                        .publication
                        .take()
                        .unwrap();
                    let detached = publication
                        .detached
                        .load(std::sync::atomic::Ordering::Acquire);
                    world
                        .resource_mut::<EffectRequests>()
                        .stop_entity(publication.key);
                    if !detached
                        && !publication.stopping
                        && scene
                            .fx_components
                            .get(slot)
                            .unwrap()
                            .definition
                            .remove_entity_on_end
                        && live.get(id).is_some()
                    {
                        live.request_removal(id)?;
                    }
                }
                let component = scene.fx_components.get(slot).unwrap();
                if component.reference_pending.is_some() {
                    let component = scene.fx_components.get_mut(slot).unwrap();
                    let bound = component.reference_pending.take().unwrap();
                    if let Some(publication) = &component.publication {
                        world.resource_mut::<EffectRequests>().reference_event(
                            publication.key.clone(),
                            bound,
                            false,
                        );
                    }
                }
                let component = scene.fx_components.get(slot).unwrap();
                if !component.started || component.stop_pending {
                    if let Some(publication) = scene
                        .fx_components
                        .get_mut(slot)
                        .unwrap()
                        .publication
                        .as_mut()
                    {
                        if !publication.stopping {
                            world.resource_mut::<EffectRequests>().reference_event(
                                publication.key.clone(),
                                false,
                                true,
                            );
                            publication.stopping = true;
                        }
                    }
                    if scene.fx_components.get(slot).unwrap().publication.is_none() {
                        scene.fx_components.get_mut(slot).unwrap().queued = false;
                        scene.fx_publications.remove(position);
                    } else {
                        position += 1;
                    }
                    continue;
                }
                if component.requested || live.get(id).is_none() {
                    if component.publication.is_none() {
                        scene.fx_components.get_mut(slot).unwrap().queued = false;
                        scene.fx_publications.remove(position);
                    } else {
                        position += 1;
                    }
                    continue;
                }
                if let Some(handle) = scene.entities[index].components.reference_actor {
                    let actor = scene
                        .actors
                        .actors
                        .get(handle.slot)
                        .ok_or("CpntFX actual reference actor absent")?;
                    if actor.owner.flags & 0xc <= 7 || actor.flags & 0x10 != 0 {
                        position += 1;
                        continue;
                    }
                }
                let frame = scene
                    .entity_frame(&live, id)?
                    .ok_or("CpntFX source reference frame absent")?;
                let published = scene.published_entity_frame(&live, id)?;
                let component = scene.fx_components.get(slot).unwrap();
                let Some(path) = component.definition.effect.as_deref() else {
                    scene.fx_components.get_mut(slot).unwrap().queued = false;
                    scene.fx_publications.remove(position);
                    continue;
                };
                let target = component.target.map(|target| {
                    (
                        live.get(target)
                            .map_or(EntityReference::Invalid, |(name, _)| {
                                EntityReference::Named(name.to_owned())
                            }),
                        component.definition.target_tag.to_string(),
                    )
                });
                let name = live
                    .get(id)
                    .ok_or("CpntFX source generation retired during publication")?
                    .0
                    .to_owned();
                let key: Arc<str> = Arc::from(format!(
                    "component-fx:{id:?}:{}:{}",
                    component.owner.generation, component.serial
                ));
                let context = EntityEffectContext {
                    owner: Some(id),
                    culprit: component.culprit,
                    velocities: published.velocities,
                    cannot_incapacitate_player_and_allies: false,
                    player_impact: false,
                    contact_speed: 0.,
                    contact_mass: 0.,
                    contact_stimulus_enabled: None,
                };
                //Both paths pass the actual initial matrix. Follow position
                //is a separate flag; it never chooses one-shot vs persistent.
                let placement = EntityPlacement {
                    target: EntityReference::Named(name),
                    tag: String::new(),
                    local: Default::default(),
                    attached: component.definition.update_position,
                };
                let mut requests = world.resource_mut::<EffectRequests>();
                let ticket = requests.start_component(key.clone(), path, placement, frame, context);
                let detached = component_detach_receipt(&ticket)
                    .ok_or("CpntFX actual detach receipt absent")?;
                requests.set_component_parameters(
                    key.clone(),
                    ComponentParameters {
                        target,
                        dont_spawn_if_culled: component.definition.dont_spawn_if_culled,
                        stop_on_target_removal: component.definition.stop_on_target_removal,
                        stop_on_parent_removal: component.definition.stop_on_parent_removal,
                    },
                );
                drop(requests);
                let component = scene.fx_components.get_mut(slot).unwrap();
                component.requested = true;
                component.publication = Some(Publication {
                    key,
                    ticket,
                    stopping: false,
                    detached,
                });
                position += 1;
            }
            Ok(())
        })
    })
}

pub(in crate::app) fn begin_retirement(world: &mut bevy::prelude::World) {
    if let Some(mut scene) = world.get_resource_mut::<PhysicsScene>() {
        for position in 0..scene.fx_publications.len() {
            let slot = scene.fx_publications[position].handle.slot;
            if let Some(component) = scene.fx_components.get_mut(slot) {
                component.stop_pending = true;
            }
        }
    }
}
pub(super) struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub target: Option<EntityId>,
    pub culprit: Option<EntityId>,
    pub started: bool,
    pub requested: bool,
    pub serial: u64,
    pub publication: Option<Publication>,
    pub stop_pending: bool,
    pub queued: bool,
    pub reference_pending: Option<bool>,
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl PhysicsScene {
    pub(super) fn create_fx_component(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.fx_components,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| {
                Ok(Component {
                    owner: Default::default(),
                    definition,
                    target: None,
                    culprit: None,
                    started: false,
                    requested: false,
                    serial: 0,
                    publication: None,
                    stop_pending: false,
                    queued: false,
                    reference_pending: None,
                })
            },
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn initialize_fx_component(
        &mut self,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let index = self
            .owner(handle)?
            .entity
            .ok_or("CpntFX actual owner absent")?;
        live.get(self.entities[index].shared)
            .ok_or("CpntFX owner generation stale")?;
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX component absent")?;
        if component.target.is_none() {
            component.target = match component.definition.target.as_ref() {
                Some(crate::gameplay::event_graph::EntityReference::Named(name)) => live.find(name),
                Some(crate::gameplay::event_graph::EntityReference::Managed(id)) => live.get(*id).map(|_|*id),
                Some(crate::gameplay::event_graph::EntityReference::Player) => {
                    self.damage_player.as_ref().map(|player| player.id)
                }
                _ => None,
            };
        }
        component.owner.mark_initialized();
        Ok(())
    }
    fn fx_component_handle(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<ComponentHandle, String> {
        live.get(id).ok_or("CpntFX actor generation stale")?;
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("CpntFX actor header absent")?;
        self.entities[index]
            .components
            .find_component([self.classes[57]; 2], [0, 0], |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|decl| decl.scheduling.start_phases),
                    },
                )
            })?
            .ok_or_else(|| "CpntFX actual component absent".into())
    }
    ///64bc20 configures the actual component before7e7b10; the outer
    ///factory owns the single Init/Start queue drain.
    pub(in crate::app) fn configure_animation_fx(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        target: Option<EntityId>,
        culprit: Option<EntityId>,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let handle = self.fx_component_handle(live, id)?;
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX birth component absent")?;
        if component.started || component.requested {
            return Err("CpntFX configured after Start".into());
        }
        component.target = target;
        component.culprit = culprit;
        registry.request_start(
            &mut component.owner,
            handle.packed() as usize,
            false,
            |_, _| Ok(()),
        )
    }
    pub(super) fn start_fx_component(
        &mut self,
        live: &LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX Start component absent")?;
        let index = component.owner.entity.ok_or("CpntFX Start owner absent")?;
        let id = self.entities[index].shared;
        live.get(id).ok_or("CpntFX Start generation stale")?;
        if component.publication.is_some() {
            return Err("CpntFX Start precedes previous FX acknowledgement".into());
        }
        component.started = true;
        component.requested = false;
        component.stop_pending = false;
        component.reference_pending = None;
        component.serial = component.serial.wrapping_add(1);
        if !component.queued {
            self.fx_publications
                .push(crate::gameplay::pickable_component::CheckedComponent {
                    handle,
                    generation: component.owner.generation,
                });
            component.queued = true;
        }
        events.register(
            0x125,
            Some(id),
            Listener::Component {
                owner: self.owner_id,
                handle,
                generation: component.owner.generation,
                handler: 0x125,
            },
        );
        Ok(())
    }
    pub(super) fn stop_fx_component(
        &mut self,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX Stop component absent")?;
        if let Some(index) = component.owner.entity {
            events.unregister(
                0x125,
                Some(self.entities[index].shared),
                Listener::Component {
                    owner: self.owner_id,
                    handle,
                    generation: component.owner.generation,
                    handler: 0x125,
                },
            );
        }
        component.started = false;
        component.stop_pending = true;
        Ok(())
    }
    pub(super) fn deinitialize_fx_component(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX Deinit component absent")?;
        if component.publication.is_some() {
            return Err("CpntFX Deinit precedes actual FX tail acknowledgement".into());
        }
        component.target = None;
        component.culprit = None;
        Ok(())
    }
    pub(in crate::app) fn fx_component_retired(
        &self,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        Ok(self
            .fx_components
            .get(handle.slot)
            .ok_or("CpntFX retirement component absent")?
            .publication
            .is_none())
    }
    pub(super) fn fx_component_message(
        &mut self,
        listener: Listener,
        event: &Event,
    ) -> Result<(), String> {
        let Listener::Component {
            handle,
            generation,
            owner,
            handler,
        } = listener
        else {
            return Err("CpntFX listener kind differs".into());
        };
        if owner != self.owner_id || handler != 0x125 {
            return Err("CpntFX listener owner/handler differs".into());
        }
        let component = self
            .fx_components
            .get_mut(handle.slot)
            .ok_or("CpntFX event component absent")?;
        if component.owner.generation != generation {
            return Ok(());
        }
        if event.id != 0x125 {
            return Err("CpntFX event differs from registered125".into());
        }
        let bound = match &event.payload {
            Payload::Entity(_) => true,
            Payload::None => false,
            _ => return Err("CpntFX125 lacks actual binding payload".into()),
        };
        if component.publication.is_some() {
            component.reference_pending =
                Some(bound && component.reference_pending.unwrap_or(true));
        }
        Ok(())
    }
}
