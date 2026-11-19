//! WhaleOilTank741910/741d50/7422a0/741ed0/7425e0/742010.
use super::*;
use crate::assets::{runtime_entity::Definition as EntityDefinition, whale_oil_tank::Definition};
use crate::gameplay::{
    component_state::{ComponentInstance, ComponentOwner},
    entity_events::{EntityEvents, Event, Listener, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    whale_oil_tank::Runtime,
};

pub(super) struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub pusher_definition: Option<Arc<EntityDefinition>>,
    pub pusher: Option<EntityId>,
    pub runtime: Runtime,
    pub initialized: bool,
    pub updating: bool,
    pub updated_frame: Option<i32>,
    pub ejections: std::collections::VecDeque<[f32; 3]>,
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
    ///On-demand witness; borrows the actual component generation.
    pub(in crate::app) fn whale_oil_tank_snapshot(&self,live:&LiveEntities,id:EntityId)
        ->Result<Option<(crate::gameplay::pickable_component::CheckedComponent,i32,f32,bool,bool,bool,Option<EntityId>)>,String>{
        if live.get(id).is_none(){return Ok(None);}
        let Some(&index)=self.shared_indices.get(&id) else{return Ok(None);};
        let Some(checked)=self.cue_dependency(index,b"arkCpntWhaleOilTank",[0,0])? else{return Ok(None);};
        let component=self.whale_oil_tanks.get(checked.handle.slot).ok_or("WhaleOilTank observation payload absent")?;
        Ok(Some((checked,component.runtime.energy,component.runtime.remaining,component.runtime.thrown,
            component.runtime.started,component.updating,component.pusher)))
    }
    pub(super) fn create_whale_oil_tank(
        &mut self,
        index: usize,
        definition: Arc<Definition>,
        pusher: Option<Arc<EntityDefinition>>,
        declaration: &ComponentDeclaration,
        registry: &mut ComponentRegistry,
        immediate: bool,
    ) -> Result<Option<ComponentHandle>, String> {
        ComponentFactory {
            declaration,
            pool: &mut self.whale_oil_tanks,
            registry,
            grow_one_at_a_time: false,
        }
        .create(
            &mut self.entities[index].components,
            immediate,
            |_, _, _| {
                let mut owner = ComponentOwner::default();
                owner.request_update(1);
                Ok(Component {
                    owner,
                    definition,
                    pusher_definition: pusher,
                    pusher: None,
                    runtime: Default::default(),
                    initialized: false,
                    updating: true,
                    updated_frame: None,
                    ejections: Default::default(),
                })
            },
            |_, _, _| Ok(()),
        )
    }
    pub(super) fn initialize_whale_oil_tank(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        registry: &mut ComponentRegistry,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .whale_oil_tanks
            .get(handle.slot)
            .ok_or("WhaleOilTank Initialize payload absent")?;
        let index = component
            .owner
            .entity
            .ok_or("WhaleOilTank Initialize owner absent")?;
        let id = self.entities[index].shared;
        let generation = component.owner.generation;
        let pusher = component.pusher_definition.clone();
        if !component.initialized {
            self.whale_oil_tanks
                .get_mut(handle.slot)
                .unwrap()
                .owner
                .mark_initialized();
            for handler in [0x127, 0xa2, 0x132] {
                events.register(
                    handler,
                    Some(id),
                    Listener::Component {
                        owner: self.owner_id,
                        handle,
                        generation,
                        handler,
                    },
                );
            }
            let component = self.whale_oil_tanks.get_mut(handle.slot).unwrap();
            if component.owner.flags & (1 << 30) == 0 {
                component.runtime.start(
                    component.definition.starting_energy,
                    component.definition.maximum_energy,
                );
            }
            component.initialized = true;
        }
        if self.whale_oil_tanks.get(handle.slot).unwrap().owner.flags & (1 << 30) == 0
            && self
                .whale_oil_tanks
                .get(handle.slot)
                .unwrap()
                .pusher
                .is_none()
        {
            if let Some(definition) = pusher {
                let frame = self.entities[index].frame.pose;
                let child = super::super::runtime_entities::spawn_component_child_in_registry(
                    world,
                    self,
                    live,
                    events,
                    registry,
                    definition,
                    super::super::runtime_entities::SpawnOptions {
                        frame,
                        scale: None,
                        hidden_disabled: false,
                    },
                    None,
                    id,
                )?;
                //Retain before fallible binding; retry must never create twice.
                self.whale_oil_tanks.get_mut(handle.slot).unwrap().pusher = Some(child);
                self.bind_entity_in_registry(
                    live,
                    id,
                    child,
                    super::throwable::BindingPose {
                        joint: -1,
                        offset: [0.; 3],
                        orientation: [0., 0., 0., 1.],
                        parent_placement: None,
                        initial_pose: Some(frame), //Explicit zero offset/identity; pusher was born in this same frame.
                    },
                    events,
                    registry,
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn start_whale_oil_tank(&mut self, handle: ComponentHandle) -> Result<(), String> {
        let component = self
            .whale_oil_tanks
            .get_mut(handle.slot)
            .ok_or("WhaleOilTank Start payload absent")?;
        let index = component
            .owner
            .entity
            .ok_or("WhaleOilTank Start owner absent")?;
        component.runtime.started = true;
        if component.runtime.energy > 0 {
            self.entities[index].entity_flags_d7 |= 4;
        } else {
            self.entities[index].entity_flags_d7 &= !4;
        }
        self.whale_oil_tank_updates_pending |= component.updating;
        Ok(())
    }
    pub(super) fn stop_whale_oil_tank(&mut self, handle: ComponentHandle) -> Result<(), String> {
        self.whale_oil_tanks
            .get_mut(handle.slot)
            .ok_or("WhaleOilTank Stop payload absent")?
            .runtime
            .started = false;
        Ok(())
    }
    pub(super) fn deinitialize_whale_oil_tank(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let component = self
            .whale_oil_tanks
            .get_mut(handle.slot)
            .ok_or("WhaleOilTank Deinitialize payload absent")?;
        let index = component
            .owner
            .entity
            .ok_or("WhaleOilTank Deinitialize owner absent")?;
        let id = self.entities[index].shared;
        for handler in [0xa2, 0x127, 0x132] {
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
        if let Some(child) = component.pusher {
            if live.get(child).is_some() {
                live.request_removal(child)?;
            }
        }
        component.initialized = false;
        component.runtime.damage = None;
        component.updating = false;
        Ok(())
    }
    pub(super) fn whale_oil_tank_child_retired(
        &self,
        live: &LiveEntities,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        Ok(self
            .whale_oil_tanks
            .get(handle.slot)
            .ok_or("WhaleOilTank retirement payload absent")?
            .pusher
            .is_none_or(|child| live.get(child).is_none()))
    }
    pub(super) fn dispatch_whale_oil_tank(
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
            return Err("WhaleOilTank listener type differs".into());
        };
        let component = self
            .whale_oil_tanks
            .get(handle.slot)
            .ok_or("WhaleOilTank message payload absent")?;
        if owner != self.owner_id || component.owner.generation != generation || handler != event.id
        {
            return Err("WhaleOilTank callback generation differs".into());
        }
        let index = component
            .owner
            .entity
            .ok_or("WhaleOilTank callback owner absent")?;
        let id = self.entities[index].shared;
        live.get(id)
            .ok_or("WhaleOilTank callback managed generation stale")?;
        match handler {
            0xa2 => {
                let Payload::Damage { params, .. } = &event.payload else {
                    return Err("WhaleOilTank A2 lacks actual damage parameters".into());
                };
                let params = *params;
                let Some(health) = self.health_component(live, id)? else {
                    self.whale_oil_tanks
                        .get_mut(handle.slot)
                        .unwrap()
                        .runtime
                        .thrown = false;
                    return Ok(());
                };
                let current = self.health_payload(health)?.current;
                //741ed0 uses the real inflictor Observable's declaration tag.
                let window_tag = self
                    .whale_oil_tanks
                    .get(handle.slot)
                    .unwrap()
                    .definition
                    .window_tag
                    .clone();
                let window_source = if let Some(source) = params
                    .inflictor
                    .filter(|source| live.get(*source).is_some())
                {
                    if let Some(&source_index) = self.shared_indices.get(&source) {
                        self.cue_dependency(source_index, b"arkCpntObservable", [4, 0])?
                            .map(|checked| {
                                self.observables
                                    .get(checked.handle.slot)
                                    .ok_or("WhaleOilTank source Observable absent")
                                    .map(|observable| {
                                        observable.definition.object_tag.as_ref()
                                            == window_tag.as_ref()
                                    })
                            })
                            .transpose()?
                            .unwrap_or(false)
                    } else {
                        false
                    }
                } else {
                    false
                };
                let component = self.whale_oil_tanks.get_mut(handle.slot).unwrap();
                let arm = component.runtime.damaged(
                    params,
                    current,
                    window_source,
                    component.definition.delayed_types,
                    component.definition.delayed_seconds,
                );
                if arm {
                    component.updating = true;
                    self.whale_oil_tank_updates_pending = true;
                }
            }
            0x132 => self
                .whale_oil_tanks
                .get_mut(handle.slot)
                .unwrap()
                .runtime
                .immediate_explosion(),
            0x127 => {
                //742110 applies the actual ejection vector through the same
                //PhysicsBase witness/impulse owner. No inferred throw event.
                let Payload::Bytes(bytes) = &event.payload else {
                    return Err("WhaleOilTank127 lacks its actual vector payload".into());
                };
                if bytes.len() < 12 {
                    return Err("WhaleOilTank127 vector payload length differs".into());
                }
                let vector = std::array::from_fn(|axis| {
                    f32::from_le_bytes(bytes[axis * 4..axis * 4 + 4].try_into().unwrap())
                });
                self.whale_oil_tanks
                    .get_mut(handle.slot)
                    .unwrap()
                    .ejections
                    .push_back(vector);
                self.whale_oil_tank_updates_pending = true;
            }
            _ => return Err("unsupported WhaleOilTank numeric handler".into()),
        }
        Ok(())
    }
    pub(in crate::app) fn set_whale_oil_tank_thrown(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        thrown: bool,
    ) -> Result<bool, String> {
        live.get(id).ok_or("WhaleOilTank thrown actor stale")?;
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(false);
        };
        let Some(checked) = self.cue_dependency(index, b"arkCpntWhaleOilTank", [0x28, 0])? else {
            return Ok(false);
        };
        self.whale_oil_tanks
            .get_mut(checked.handle.slot)
            .ok_or("WhaleOilTank thrown payload absent")?
            .runtime
            .thrown = thrown;
        Ok(true)
    }
}

///Same-frame accepted callback drain; outgoing levels call with advance=false.
pub(in crate::app) fn finish_accepted(world: &mut bevy::prelude::World) -> Result<(), String> {
    run(world, false)
}
pub(in crate::app) fn update(world: &mut bevy::prelude::World) -> Result<(), String> {
    run(world, true)
}
fn run(world: &mut bevy::prelude::World, advance: bool) -> Result<(), String> {
    if world
        .get_resource::<PhysicsScene>()
        .is_none_or(|scene| !scene.whale_oil_tank_updates_pending)
    {
        return Ok(());
    }
    world.resource_scope(|world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
        world.resource_scope(|world, mut live: bevy::prelude::Mut<LiveEntities>| {
            world.resource_scope(|world, mut events: bevy::prelude::Mut<EntityEvents>| {
                let clock = world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>();
                let marker = clock.clock().map_err(str::to_owned)?.frame_marker();
                let advance = advance && clock.advanced();
                let mut registry = scene
                    .registry
                    .take()
                    .ok_or("WhaleOilTank callback registry borrowed")?;
                let accepted = scene.finish_whale_oil_tank_ejections(world, &live, &mut registry);
                scene.registry = Some(registry);
                accepted?;
                let mut pending = false;
                for cursor in 0..scene.whale_oil_tanks.allocated_count() {
                    let slot = scene.whale_oil_tanks.roster.indices[cursor];
                    let component = scene
                        .whale_oil_tanks
                        .get(slot)
                        .ok_or("WhaleOilTank roster payload absent")?;
                    let index = component
                        .owner
                        .entity
                        .ok_or("WhaleOilTank update owner absent")?;
                    let id = scene.entities[index].shared;
                    let component = scene.whale_oil_tanks.get(slot).unwrap();
                    if !component.runtime.started || !component.updating {
                        continue;
                    }
                    if !advance || component.updated_frame == Some(marker) {
                        pending = true;
                        continue;
                    }
                    let delta = world
                        .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                        .clock()
                        .map_err(str::to_owned)?
                        .entity_delta(scene.entities[index].timespace)
                        .ok_or("WhaleOilTank timespace invalid")?;
                    let component = scene.whale_oil_tanks.get_mut(slot).unwrap();
                    let fire = component.runtime.advance(delta);
                    component.updated_frame = Some(marker);
                    if !fire {
                        pending = true;
                        continue;
                    }
                    let damage = component.runtime.damage;
                    //741c40 is a no-op without retained damage or actual Health.
                    if let Some(mut damage) = damage {
                        if let Some(health) = scene.health_component(&live, id)? {
                            let current = scene.health_payload(health)?.current;
                            scene.health_payload_mut(health)?.receiver.infinite_health = false;
                            damage.amount = current; //780a80(...currentHealth,false).
                            scene.damage_entity(&mut live, &mut events, id, &damage)?;
                            if let Some(child) = scene.whale_oil_tanks.get(slot).unwrap().pusher {
                                if live.get(child).is_some() {
                                    live.request_removal(child)?;
                                }
                            }
                        }
                    }
                    let component = scene.whale_oil_tanks.get_mut(slot).unwrap();
                    component.updating = false;
                    component.runtime.damage = None;
                }
                scene.whale_oil_tank_updates_pending = pending;
                Ok(())
            })
        })
    })
}
impl PhysicsScene {
    fn apply_whale_oil_tank_ejection(
        &mut self,
        live: &LiveEntities,
        index: usize,
        local: [f32; 3],
        point: [f32; 3],
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let id = self.entities[index].shared;
        live.get(id)
            .ok_or("WhaleOilTank127 source generation stale")?;
        let Some(handle) = self.document_rigid(index)? else {
            //166830 returns in the installed build: report and retain the
            //native no-body return, rather than turning it into a game abort.
            bevy::log::warn!(
                "WhaleOilTank127 cannot expulse {}: actual rigid owner absent",
                self.entities[index].name
            );
            return Ok(());
        };
        let body = self
            .rigid_component(handle)?
            .body()
            .ok_or("WhaleOilTank127 real body absent")?;
        let basis = self.entities[index].frame.pose.basis;
        let impulse: [f32; 4] = std::array::from_fn(|axis| {
            if axis == 3 {
                0.
            } else {
                local[0] * basis[0][axis] + local[1] * basis[1][axis] + local[2] * basis[2][axis]
            }
        });
        let result = (|| {
            let shared = self.world.clone();
            scene_publication_events!(self, registry, publication);
            let mut guard = shared
                .write()
                .map_err(|_| "WhaleOilTank127 physics owner poisoned")?;
            let world = &mut *guard;
            world.storage.apply_body_impulse(
                &mut world.geometry,
                &mut publication,
                body,
                impulse,
                Some([point[0], point[1], point[2], 0.]),
            )?;
            Ok(())
        })();
        result
    }
}

///CPU-level placement plan; actual child resource scopes remain in RuntimeEntities.
pub(in crate::app) struct Prepared {
    pub actor: EntityId,
    pub definition: Arc<Definition>,
    pub pusher: Option<Arc<EntityDefinition>>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
    pub scope: Arc<str>,
    pub handle: Option<crate::gameplay::pickable_component::CheckedComponent>,
    pub requested: bool,
}
impl PhysicsScene {
    pub(in crate::app) fn prepare_placed_whale_oil_tanks(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        models: &[entities::PlacedModel],
        live: &mut LiveEntities,
    ) -> Result<(), String> {
        let mut children = HashMap::<String, Arc<EntityDefinition>>::new();
        for binding in crate::assets::whale_oil_tank::bindings(catalog, document)? {
            let actor =
                if let Some(entity) = self
                    .entities
                    .iter()
                    .find(|entity| !entity.retired && entity.name == binding.entity)
                {
                    entity.shared
                } else {
                    let authored = document
                        .definitions
                        .iter()
                        .find(|entry| entry.kind == "entity" && entry.id == binding.entity)
                        .ok_or("WhaleOilTank placed declaration absent")?;
                    let class = entities::scene_motion::atom(&authored.fields, "class")
                        .ok_or("WhaleOilTank authored class absent")?;
                    let class =
                        self.types
                            .classes()
                            .hierarchy()
                            .registration_order()
                            .iter()
                            .copied()
                            .find(|&index| {
                                self.types.classes().metadata(index).and_then(|metadata| {
                                    self.types.classes().name_text(metadata.name)
                                }) == Some(class.as_bytes())
                            })
                            .ok_or("WhaleOilTank authored class not registered")?;
                    let frame = entities::spawn_frame(authored)?;
                    let scale = models
                        .iter()
                        .find(|model| model.id == binding.entity)
                        .map_or([1.; 3], |model| model.scale);
                    let id=self.allocate_entity_header(
                        live,
                        &binding.entity,
                        class,
                        0,
                        super::super::runtime_entities::SpawnOptions {
                            frame,
                            scale: None,
                            hidden_disabled: false,
                        },
                        scale,
                        Some(Arc::from(binding.entity.as_str())),
                        true,
                        false,
                    )?;
                    if entities::entity_hidden(authored){let index=self.shared_indices[&id];self.entities[index].entity_flags|=1;}
                    id
                };
            let pusher = if let Some(path) = binding.definition.pusher.as_deref() {
                Some(if let Some(child) = children.get(path) {
                    child.clone()
                } else {
                    let child = EntityDefinition::acquire(catalog, path, None)?;
                    children.insert(path.to_owned(), child.clone());
                    child
                })
            } else {
                None
            };
            self.prepared_whale_oil_tanks.push(Prepared {
                actor,
                definition: binding.definition,
                pusher,
                scheduling: binding.scheduling,
                scope: Arc::from(format!("whale-oil-tank:{}:{actor:?}", self.owner_id)),
                handle: None,
                requested: false,
            });
        }
        Ok(())
    }
    pub(in crate::app) fn take_prepared_whale_oil_tanks(&mut self) -> Vec<Prepared> {
        std::mem::take(&mut self.prepared_whale_oil_tanks)
    }
    pub(in crate::app) fn admit_prepared_whale_oil_tanks(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        plans: &mut [Prepared],
    ) -> Result<(), String> {
        let mut registry = self
            .registry
            .take()
            .ok_or("WhaleOilTank admission registry borrowed")?;
        let result = (|| {
            for plan in plans {
                live.get(plan.actor)
                    .ok_or("WhaleOilTank admission generation stale")?;
                if plan.handle.is_some() {
                    continue;
                }
                let index = *self
                    .shared_indices
                    .get(&plan.actor)
                    .ok_or("WhaleOilTank admission header absent")?;
                let declaration = self.types.declaration(
                    self.factories[60],
                    Arc::as_ptr(&plan.definition) as usize,
                    plan.scheduling.clone(),
                )?;
                let handle = self
                    .create_whale_oil_tank(
                        index,
                        plan.definition.clone(),
                        plan.pusher.clone(),
                        &declaration,
                        &mut registry,
                        declaration.scheduling.starts_on_spawn(),
                    )?
                    .ok_or("WhaleOilTank construction disabled")?;
                self.declarations.insert(handle.packed(), declaration);
                plan.handle = Some(crate::gameplay::pickable_component::CheckedComponent {
                    handle,
                    generation: self.owner(handle)?.generation,
                });
            }
            self.run_runtime_startup(world, live, events, &mut registry)
        })();
        self.registry = Some(registry);
        result
    }
}

impl PhysicsScene {
    pub(super) fn finish_whale_oil_tank_ejections(
        &mut self,
        world: &bevy::prelude::World,
        live: &LiveEntities,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        for cursor in 0..self.whale_oil_tanks.allocated_count() {
            let slot = self.whale_oil_tanks.roster.indices[cursor];
            let index = self
                .whale_oil_tanks
                .get(slot)
                .ok_or("WhaleOilTank ejection payload absent")?
                .owner
                .entity
                .ok_or("WhaleOilTank ejection owner absent")?;
            while let Some(vector) = self
                .whale_oil_tanks
                .get(slot)
                .unwrap()
                .ejections
                .front()
                .copied()
            {
                let center = if let Some(&placement) = self.rigid_placements.get(&index) {
                    let bounds = world
                        .resource::<super::super::shadow_updates::ModelShadowSources>()
                        .0
                        .iter()
                        .find(|source| source.placement == placement)
                        .ok_or("WhaleOilTank127 actual model bounds absent")?
                        .bounds;
                    std::array::from_fn(|axis| (bounds[0][axis] + bounds[1][axis]) * 0.5)
                } else {
                    self.entities[index].frame.pose.position
                };
                self.apply_whale_oil_tank_ejection(live, index, vector, center, registry)?;
                self.whale_oil_tanks
                    .get_mut(slot)
                    .unwrap()
                    .ejections
                    .pop_front();
            }
        }
        Ok(())
    }
}
