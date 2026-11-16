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
    pub sound_volume: Option<CheckedComponent>,
    listeners: bool,
}
impl BreakableComponent {
    pub(super) fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: Default::default(),
            runtime: Runtime::new(definition),
            health: None,
            rigid: None,
            sound_volume: None,
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
            //4c7c50's predictive breakability query uses877720 mode2:
            //141c70e88[2] = {0x28,0}; Start's Health query is mode1 below.
            .find_component([self.classes[11]; 2], [0x28, 0], |handle| {
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
            //7ed9f0 explicitly asks877720 mode1 for its Health dependency.
            .find_component(self.health_class_range()?, [0x20, 0], lookup)?
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
        //7ed9f0 caches this actual mode1 reference through Stop. A later
        //phase change must not select a different same-class component.
        let sound_volume=self.entities[index].components
            .find_component([self.classes[17];2],[0x20,0],lookup)?
            .map(|handle|->Result<CheckedComponent,String>{Ok(CheckedComponent{
                handle,generation:self.owner(handle)?.generation})}).transpose()?;
        let component = self
            .breakables
            .get_mut(handle.slot)
            .ok_or("breakable Start slot absent")?;
        component.runtime.start()?;
        component.health = Some(checked_health);
        component.rigid = rigid;
        component.sound_volume = sound_volume;
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
        component.sound_volume = None;
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
        if !self.is_health_class(checked.handle.class_id)
            || health_owner.generation != checked.generation
            || health_owner.entity != Some(index)
            || health_owner.flags & 0xc < 8
        {
            return Err("breakable checked Health dependency stale".into());
        }
        let health = self.health_payload(checked.handle)?;
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
    /// of the calculation's directional rejection;7ee050 delegates to7edfe0.
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
        let health = self.health_payload(checked.handle)?;
        let index = component
            .owner
            .entity
            .ok_or("breakable prediction owner absent")?;
        if !self.is_health_class(checked.handle.class_id)
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
        let result = self.evaluate_health(live, checked.handle, params, context)?;
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
    ///7edfe0's raw amount gate, used by the immediate Throwable contact
    ///prediction. This deliberately does not run717800 or filter damage type.
    pub(in crate::app) fn breakable_would_break_amount(
        &self,
        live: &LiveEntities,
        id: EntityId,
        amount: f32,
        last_only: bool,
    ) -> Result<bool, String> {
        let Some(handle) = self.breakable_component(live, id)? else {
            return Ok(false);
        };
        let component = self
            .breakables
            .get(handle.slot)
            .ok_or("Breakable amount prediction absent")?;
        let checked = component
            .health
            .ok_or("Breakable amount Health reference absent")?;
        let health = self.health_payload(checked.handle)?;
        if !self.is_health_class(checked.handle.class_id)
            || health.owner.generation != checked.generation
            || health.owner.entity != component.owner.entity
            || health.owner.flags & 0xc < 8
        {
            return Err("Breakable amount Health reference stale".into());
        }
        Ok(component.runtime.should_break(
            Health {
                current: health.current,
                infinite: health.receiver.infinite_health,
            },
            amount,
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
    ///7ed9a0/7ecc90 needs this owner's accepted step journal finished before
    ///destruction. The host's global command dispatcher is not a prerequisite
    ///for retiring unrelated actors: inspect their canonical component list.
    pub(in crate::app) fn entity_has_pending_breakable_effect(&self, id: EntityId) -> bool {
        self.shared_indices.get(&id).is_some_and(|&index| {
            self.entities[index]
                .components
                .entries
                .values()
                .iter()
                .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
                .filter(|handle| handle.class_id == self.classes[11])
                .any(|handle| {
                    self.breakables
                        .get(handle.slot)
                        .is_some_and(|component| component.runtime.pending().is_some())
                })
        })
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
    ///7ecc90 destroys only chunks still owned by the component array.
    pub(in crate::app) fn begin_breakable_retirement(
        &mut self,
        id: EntityId,
    ) -> Result<Vec<EntityId>, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(Vec::new());
        };
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
            .find(|handle| handle.class_id == self.classes[11]);
        let Some(handle) = handle else {
            return Ok(Vec::new());
        };
        let runtime = &mut self
            .breakables
            .get_mut(handle.slot)
            .ok_or("retiring Breakable payload absent")?
            .runtime;
        if runtime.pending().is_some() {
            return Err("Breakable source journal must drain before destructor".into());
        }
        runtime.begin_retirement();
        Ok(runtime.retained_chunks().collect())
    }
    pub(in crate::app) fn breakable_retirement_complete(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<bool, String> {
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(true);
        };
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
            .find(|handle| handle.class_id == self.classes[11]);
        let Some(handle) = handle else {
            return Ok(true);
        };
        let runtime = &mut self
            .breakables
            .get_mut(handle.slot)
            .ok_or("retiring Breakable payload absent")?
            .runtime;
        let completed: Vec<_> = runtime
            .retained_chunks()
            .filter(|id| live.get(*id).is_none())
            .collect();
        for child in completed {
            runtime.retire_chunk(child)?;
        }
        Ok(runtime.stopped())
    }

    ///7ecc00's synchronous remove invocation transfers lifetime to GameLocal's
    ///ordinary queue before the component clears its retained pointer.
    pub(in crate::app) fn acknowledge_breakable_chunk_removal(
        &mut self,
        id: EntityId,
        child: EntityId,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable destructor owner absent")?;
        let handle = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .filter_map(|&packed| ComponentHandle::from_packed(packed).ok())
            .find(|handle| handle.class_id == self.classes[11])
            .ok_or("Breakable destructor component absent")?;
        self.breakables
            .get_mut(handle.slot)
            .ok_or("Breakable destructor payload absent")?
            .runtime
            .retire_chunk(child)
    }

    pub(in crate::app) fn published_entity_frame(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<crate::gameplay::reference_frame::PublishedFrame, String> {
        if live.get(id).is_none() {
            return Err("Breakable source entity generation stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable source actor absent")?;
        //7ee0e0/7ee610 copy entity+300/+30c/+33c/+348. Prediction and
        //binding caches remain with the reference actor's publication owner.
        Ok(self.entities[index].frame)
    }
    pub(in crate::app) fn breakable_hidden(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        hidden: bool,
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Err("Breakable visibility entity generation stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable visibility actor absent")?;
        self.entities[index].entity_flags =
            (self.entities[index].entity_flags & !1) | u32::from(hidden);
        Ok(())
    }
    ///7e7c00 queues the real rigid component; its existing typed Stop callback
    ///owns body unpublication and listener teardown, not this effect producer.
    pub(in crate::app) fn breakable_stop_rigid(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
    ) -> Result<(), String> {
        let Some(handle) = self.breakable_component(live, id)? else {
            return Err("Breakable owner component absent".into());
        };
        let Some(checked) = self
            .breakables
            .get(handle.slot)
            .ok_or("Breakable Stop payload absent")?
            .rigid
        else {
            return Ok(());
        };
        if self.owner(checked.handle)?.generation != checked.generation {
            return Err("Breakable rigid reference stale".into());
        }
        let mut registry = self
            .registry
            .take()
            .ok_or("Breakable Stop registry absent")?;
        let result = registry.request_stop(
            checked.handle,
            &mut super::interaction_components::Startup { world: None,
                scene: self,
                live,
                events,
            },
        );
        self.registry = Some(registry);
        result
    }
    ///87c3d0/87c2a0/87c280 before87caa0: copy the actual owner pose and both
    ///velocities into the preconstructed chunk, preserving its authored scale.
    pub(in crate::app) fn breakable_release_chunk(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        owner: EntityId,
        child: EntityId,
    ) -> Result<(), String> {
        let frame = self.published_entity_frame(live, owner)?;
        let index = *self
            .shared_indices
            .get(&child)
            .ok_or("Breakable chunk actor absent")?;
        if live.get(child).is_none() {
            return Err("Breakable chunk generation stale".into());
        }
        self.entities[index].frame.pose = frame.pose;
        self.entities[index].frame.velocities = frame.velocities;
        if let Some(handle) = self.entities[index].components.reference_actor {
            let actor = self
                .actors
                .actors
                .get_mut(handle.slot)
                .ok_or("Breakable chunk reference actor absent")?;
            actor.set_transform(frame.pose, false);
            actor.frame.published_velocities = frame.velocities;
            actor.motion.velocities = frame.velocities;
            actor.motion.dirty = [true; 2];
        }
        self.entity_flags_d4[index] |= 4;
        //The host can resume after a fallible real Start callback. Repeated
        //queue requests are idempotent in ComponentRegistry; clearing the
        //source disabled bit alone is not an acknowledgement of Start.
        self.entities[index].entity_flags &= !0x101;
        self.entities[index].components.start_deferred = false;
        let requested_context = live.entity_context(child)?.requested;
        let handles: Vec<_> = self.entities[index]
            .components
            .entries
            .values()
            .iter()
            .map(|&packed| ComponentHandle::from_packed(packed))
            .collect::<Result<_, _>>()?;
        let mut registry = self
            .registry
            .take()
            .ok_or("Breakable release registry absent")?;
        let result = (|| {
            for handle in handles {
                let declaration = self
                    .declarations
                    .get(&handle.packed())
                    .ok_or("Breakable chunk scheduling absent")?;
                if (declaration.scheduling.start_phases >> (requested_context & 31)) & 1 == 0 {
                    registry.request_stop(
                        handle,
                        &mut super::interaction_components::Startup { world: None,
                            scene: self,
                            live,
                            events,
                        },
                    )?;
                } else if declaration.scheduling.starts_on_spawn()
                    && self.owner(handle)?.flags & 3 != 3
                {
                    registry.request_start(
                        self.owner_mut(handle)?,
                        handle.packed() as usize,
                        false,
                        |_, _| Ok(()),
                    )?;
                }
            }
            self.run_runtime_startup(world, live, events, &mut registry)
        })();
        self.registry = Some(registry);
        result?;
        if self.document_rigid(index)?.is_some() {
            self.publish_rigid_motion(
                live,
                child,
                frame.pose,
                frame.velocities[0],
                frame.velocities[1],
            )?;
        }
        Ok(())
    }
    ///7ee0e0 keeps the owner's existing parent/tag; it never binds a chunk
    ///to the owner itself, nor creates a binding when the owner is unbound.
    pub(in crate::app) fn breakable_keep_binding(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        owner: EntityId,
        child: EntityId,
    ) -> Result<(), String> {
        let Some(parent) = live.parent(owner)? else {
            return Ok(());
        };
        let index = *self
            .shared_indices
            .get(&owner)
            .ok_or("Breakable binding owner absent")?;
        let handle = self.entities[index]
            .components
            .reference_actor
            .ok_or("Breakable bound reference actor absent")?;
        let actor = self
            .actors
            .actors
            .get(handle.slot)
            .ok_or("Breakable bound actor absent")?;
        let placement = self
            .document_hierarchy
            .bindings
            .iter()
            .find(|binding| binding.child == index)
            .and_then(|binding| binding.parent_placement);
        let pose = super::throwable::BindingPose {
            joint: actor.binding.joint,
            offset: actor.binding.offset,
            orientation: actor.binding.orientation,
            parent_placement: placement,
        };
        self.bind_entity(live, child, parent, pose, events)
    }

    ///7ed000 invokes738890 through7ed9f0's actual cached mode1 component.
    ///The canonical acoustic world owns both live zone coefficients.
    pub(in crate::app) fn breakable_sound_occlusion(
        &mut self,
        id: EntityId,
        strength: f32,
        acoustic: Option<&mut crate::gameplay::sound::acoustic_world::AcousticWorld>,
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable sound owner absent")?;
        if self.entities[index].retired {
            return Err("Breakable sound owner retired".into());
        }
        let lookup=|handle|->Result<_,String>{Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState{
            flags:self.owner(handle)?.flags,start_phases:self.declarations.get(&handle.packed()).map(|decl|decl.scheduling.start_phases)})};
        let handle=self.entities[index].components.find_component([self.classes[11];2],[0x20,0],lookup)?
            .ok_or("Breakable sound callback has no actual Breakable owner")?;
        let Some(checked)=self.breakables.get(handle.slot).ok_or("Breakable sound payload absent")?.sound_volume else{return Ok(())};
        if checked.handle.class_id!=self.classes[17]{return Err("Breakable sound reference class differs".into());}
        let component=self.sound_volumes.get_mut(checked.handle.slot).ok_or("Breakable SoundVolumeControl retired")?;
        if component.owner.generation!=checked.generation || component.owner.entity!=Some(index){
            return Err("Breakable SoundVolumeControl reference generation stale".into());
        }
        let definition=component.definition.as_ref().ok_or("Breakable SoundVolumeControl definition absent")?;
        let acoustic=acoustic.ok_or("Breakable SoundVolumeControl canonical acoustic world absent")?;
        if component.state.set_strength(definition,acoustic,strength,true)?{
            self.entity_flags_d4[index]|=4;
        }
        Ok(())
    }

    pub(in crate::app) fn breakable_has_rigid(&self, id: EntityId) -> Result<bool, String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable rigid entity absent")?;
        Ok(self.document_rigid(index)?.is_some())
    }
    ///7278e0 stores the actual culprit ID (including a stale non-null ID)
    ///and8778030's entity-local time.853220 enables provenance notification
    ///only when that stored ID equals the actual managed Player ID.
    pub(in crate::app) fn set_rigid_instigator(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        culprit: EntityId,
        entity_time: f64,
        entity_delta: f32,
    ) -> Result<(), String> {
        if live.get(id).is_none() {
            return Err("rigid instigator target generation stale".into());
        }
        let enabled=self.damage_player.is_some_and(|player|player.id==culprit);
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Breakable instigator actor absent")?;
        let Some(handle) = self.document_rigid(index)? else {
            return Ok(());
        };
        {
            let component = self.rigid_component_mut(handle)?;
            component.instigator = Some(culprit);
            component.instigator_time = entity_time;
            component.instigator_clock_delta = entity_delta;
            if component.owner.flags & 3 <= 1 {
                component.instigator_pending_flags = (component.instigator_pending_flags & !3) | u8::from(enabled);
                return Ok(());
            }
        }
        self.set_rigid_instigator_notification(live, handle, enabled)
    }
    fn set_rigid_instigator_notification(
        &mut self,
        live: &LiveEntities,
        handle: ComponentHandle,
        enabled: bool,
    ) -> Result<(), String> {
        let component = self.rigid_component(handle)?;
        let body = component.body().ok_or("instigator rigid body absent")?;
        let generation = component.owner.generation;
        let changed = {
            let mut query = self
                .world
                .write()
                .map_err(|_| "instigator physics world poisoned")?;
            let record = query
                .storage
                .bodies
                .get_mut(body)
                .ok_or("instigator body generation stale")?;
            let old = record.flags & 0x80000 != 0;
            if old == enabled {
                return Ok(());
            }
            //853300's lower body flag; no new shape, body or motion is created.
            record.flags = (record.flags & !0x80000) | (u32::from(enabled) << 19);
            true
        };
        if changed {
            let subscriber =
                crate::gameplay::physics_world::body_activation_events::ActivationSubscriber {
                    handle,
                    generation,
                    callback: 1,
                };
            if enabled {
                self.body_activation_events
                    .subscribe_instigator(body, subscriber)?;
                self.rigid_component_mut(handle)?.instigator_notification =
                    Some((body, generation));
            } else {
                self.body_activation_events.unsubscribe(body, subscriber)?;
                self.rigid_component_mut(handle)?.instigator_notification = None;
            }
            //727950/7137e0 traverse actual constraint neighbors. The host
            //represents admitted constraint endpoints in the canonical world
            //adjacency, rather than retaining a second component-pointer list.
            //Arbitrary entity children and authored-but-unconstructed joints
            //do not become physics neighbors implicitly.
            let neighbors = {
                let query = self
                    .world
                    .read()
                    .map_err(|_| "instigator constraint world poisoned")?;
                let storage = &query.storage;
                let mut next = storage
                    .constraint_heads
                    .get((body & 0x00ff_ffff) as usize)
                    .copied()
                    .unwrap_or(0x7fff_ffff);
                let mut visited = 0;
                let mut neighbors = Vec::new();
                while next != 0x7fff_ffff {
                    if visited >= storage.constraints.len() {
                        return Err("cyclic instigator constraint adjacency".into());
                    }
                    let constraint = storage
                        .constraints
                        .get(next as usize)
                        .ok_or("instigator constraint record absent")?;
                    let side = if constraint.bodies[0] == body {
                        0
                    } else if constraint.bodies[1] == body {
                        1
                    } else {
                        return Err("instigator constraint endpoint differs".into());
                    };
                    let other = constraint.bodies[1 - side];
                    if let Some(record) = storage
                        .bodies
                        .get(other)
                        .filter(|record| record.user_data != 0)
                    {
                        let other_handle =
                            crate::gameplay::collision_user_data::CollisionUserData::decode(
                                record.user_data,
                            )?
                            .component;
                        if self.is_rigid_class(other_handle.class_id) {
                            let other_component = self.rigid_component(other_handle)?;
                            let other_entity = other_component
                                .owner
                                .entity
                                .ok_or("constrained rigid entity absent")?;
                            if other_component.body() != Some(other) {
                                return Err("constrained rigid body identity stale".into());
                            }
                            if live.get(self.entities[other_entity].shared).is_some() {
                                neighbors.push(other_handle);
                            }
                        }
                    }
                    next = storage
                        .constraint_next
                        .get(constraint.link_index)
                        .ok_or("instigator constraint next pair absent")?[side];
                    visited += 1;
                }
                neighbors
            };
            for handle in neighbors {
                self.set_rigid_instigator_notification(live, handle, enabled)?;
            }
        }
        Ok(())
    }
    ///808ea0->726f10 consumes853220's pre-initialize request only after
    ///the real body and component identity exist.
    pub(super) fn finish_rigid_instigator_initialize(
        &mut self,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let flags = self.rigid_component(handle)?.instigator_pending_flags;
        if flags & 1 == 0 {
            return Ok(());
        }
        self.set_rigid_instigator_notification(live, handle, true)?;
        self.rigid_component_mut(handle)?.instigator_pending_flags &= !3;
        Ok(())
    }
}
