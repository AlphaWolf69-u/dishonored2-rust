//! arkCpntGrenade491180/4911b0/4913d0 and490600..491a20.
//! The real component owns fuse state; existing FX, audio and entity lifetimes
//! own resource preparation and active/stopped tails.
use super::super::usable_effects::{EffectRequests, EffectTicket, EntityPlacement};
use super::*;
use crate::gameplay::{
    component_state::entity_lookup::ComponentLookupState,
    entity_events::{EntityEvents, Event, Listener, Payload},
    entity_lifetime::{EntityId, LiveEntities, Stage},
    grenade::{self, Component},
    pickable_component::CheckedComponent,
    reference_frame::ReferenceFrame,
};
use bevy::prelude::World;

pub(super) struct Resources {
    scope: Arc<str>,
    ///Dependency ownership is separate from the live FX instance's scope.
    ///Born runtime actors borrow their own prewarmed prototype reservation;
    ///held/placed components own the existing per-component reservation.
    dependency_scope: Arc<str>,
    owns_reservation: bool,
    armed: Option<(Arc<str>, EffectTicket)>,
    stopped: Vec<EffectTicket>,
    explosion: Option<EffectTicket>,
    closing: bool,
    active_scope: Option<Arc<str>>,
    ///bevy_backend: weak certificate for the same entity's real externally
    ///hosted cpntWeapon. It never acquires or owns an equipment resource.
    held_weapon: Option<std::sync::Weak<crate::assets::equipment::Descriptor>>,
    // bevy_backend: retry cursor for the synchronous490840 call. Random draws,
    // FX birth and owner notifications occur once even if a host owner is pending.
    exploding: Option<Explosion>,
    remove_after_explosion: bool,
}
struct Explosion {
    phase: u8,
    held: bool,
    failed: bool,
    flags: u8,
    path: Option<Arc<str>>,
    frame: ReferenceFrame,
    culprit: Option<EntityId>,
    equipment_notice_target: Option<EntityId>,
    equipment_notice: super::super::entity_graphs::EquipmentExplodedReceipt,
}
pub(super) struct Action {
    handle: ComponentHandle,
    generation: u32,
    handler: u16,
    payload: Payload,
}
fn placement(name: &str) -> EntityPlacement {
    EntityPlacement {
        target: crate::gameplay::event_graph::EntityReference::Named(name.to_owned()),
        tag: String::new(),
        local: ReferenceFrame::default(),
        attached: true,
    }
}
impl PhysicsScene {
    ///76e2b0 requests the actual Grenade at reference mode1, then491550.
    pub(super) fn begin_grenade_pickup(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<(), String> {
        let Some(checked) = self.grenade_component(live, id)? else {
            return Ok(());
        };
        let component = self.grenades.get_mut(checked.handle.slot).unwrap();
        if crate::gameplay::component_state::matches_reference_state(
            component.owner.flags,
            [0x20, 0],
        ) {
            component.picked_up();
        }
        Ok(())
    }
    fn grenade_key(&self, handle: ComponentHandle) -> Result<(u64, u32), String> {
        if handle.class_id != self.classes[19] {
            return Err("Grenade component class differs".into());
        }
        Ok((
            handle.packed(),
            self.grenades
                .get(handle.slot)
                .ok_or("Grenade payload absent")?
                .owner
                .generation,
        ))
    }
    fn grenade_index(&self, handle: ComponentHandle) -> Result<usize, String> {
        self.grenades
            .get(handle.slot)
            .ok_or("Grenade payload absent")?
            .owner
            .entity
            .ok_or("Grenade entity absent".into())
    }
    pub(in crate::app) fn grenade_component(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<CheckedComponent>, String> {
        if live.stage(id) != Some(Stage::Spawned) {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        let handle =
            self.entities[index]
                .components
                .find_component([self.classes[19]; 2], [0, 0], |h| {
                    Ok(ComponentLookupState {
                        flags: self.owner(h)?.flags,
                        start_phases: self
                            .declarations
                            .get(&h.packed())
                            .map(|d| d.scheduling.start_phases),
                    })
                })?;
        handle
            .map(|h| {
                Ok(CheckedComponent {
                    handle: h,
                    generation: self.owner(h)?.generation,
                })
            })
            .transpose()
    }
    pub(in crate::app) fn grenade_set_targeted(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        targeted: bool,
    ) -> Result<(), String> {
        let checked = self
            .grenade_component(live, id)?
            .ok_or("targeted actor has no actual Grenade")?;
        let component = self.grenades.get_mut(checked.handle.slot).unwrap();
        component.flags = (component.flags & !grenade::EXPLODE_ON_CONTACT)
            | if targeted {
                grenade::EXPLODE_ON_CONTACT
            } else {
                0
            };
        Ok(())
    }
    pub(in crate::app) fn configure_grenade_birth(
        &mut self,
        live: &mut LiveEntities,
        id: EntityId,
        attacker: Option<EntityId>,
        linear: [f32; 3],
        angular: [f32; 3],
    ) -> Result<(), String> {
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Grenade birth header absent")?;
        if self.entities[index].retired {
            return Err("Grenade birth identity retired".into());
        }
        live.set_owner(id, attacker)?;
        self.entities[index].frame.velocities = [linear, angular];
        Ok(())
    }
    pub(in crate::app) fn grenade_flags(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<u8>, String> {
        Ok(self
            .grenade_component(live, id)?
            .map(|c| self.grenades.get(c.handle.slot).unwrap().flags))
    }
    pub(in crate::app) fn prepare_grenade_resources(
        &mut self,
        world: &mut World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.grenade_key(handle)?;
        if self.grenade_resources.contains_key(&key) {
            return Ok(());
        }
        let component = self
            .grenades
            .get(handle.slot)
            .ok_or("Grenade preparation payload absent")?;
        let mut paths: Vec<_> = component.definition.fx_paths().map(str::to_owned).collect();
        paths.sort();
        paths.dedup();
        let scope: Arc<str> = Arc::from(format!("grenade:{:016x}:{}", key.0, key.1));
        let id = self.entities[self.grenade_index(handle)?].shared;
        let prepared = super::super::runtime_entities::grenade_dependency_scope(world,id,&component.definition)?;
        let owns_reservation = prepared.is_none();
        let dependency_scope = prepared.unwrap_or_else(||scope.clone());
        if owns_reservation {
            world
                .get_resource_mut::<EffectRequests>()
                .ok_or("Grenade FX source owner absent")?
                .reserve(scope.clone(), paths);
        }
        self.grenade_resources.insert(
            key,
            Resources {
                scope,
                dependency_scope,
                owns_reservation,
                armed: None,
                stopped: Vec::new(),
                explosion: None,
                closing: false,
                active_scope: None,
                held_weapon: None,
                exploding: None,
                remove_after_explosion: false,
            },
        );
        Ok(())
    }
    pub(in crate::app) fn grenade_resources_ready(
        &self,
        world: &World,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        let key = self.grenade_key(handle)?;
        let Some(resources) = self.grenade_resources.get(&key) else {
            return Ok(false);
        };
        if resources.closing {
            return Ok(false);
        }
        super::super::usable_effects::reservation_ready(world, &resources.dependency_scope)
    }
    pub(in crate::app) fn debug_grenade_resources(&self,world:&World,handle:ComponentHandle)->String {
        match self.grenade_key(handle).and_then(|key|self.grenade_resources.get(&key).ok_or("scope=absent".into())) {
            Ok(resources)=>format!("Grenade {handle:?} closing={} armed={} stopped={} explosion={}\n    {}",resources.closing,resources.armed.is_some(),resources.stopped.len(),resources.explosion.is_some(),
                super::super::usable_effects::debug_reservation(world,&resources.dependency_scope)),
            Err(error)=>format!("Grenade {handle:?} {error}"),
        }
    }
    pub(super) fn initialize_grenade(
        &mut self,
        handle: ComponentHandle,
        live: &LiveEntities,
    ) -> Result<(), String> {
        self.grenade_key(handle)?;
        if self.grenades.get(handle.slot).unwrap().initialized {
            return Ok(());
        }
        let index = self.grenade_index(handle)?;
        live.get(self.entities[index].shared)
            .ok_or("Grenade Initialize managed generation stale")?;
        //491180 caches Pickable_Simple (DAT1439c5a30), not Throwable.
        let pickable = self.entities[index]
            .components
            .find_component([self.classes[6]; 2], [0, 0], |h| {
                Ok(ComponentLookupState {
                    flags: self.owner(h)?.flags,
                    start_phases: self
                        .declarations
                        .get(&h.packed())
                        .map(|d| d.scheduling.start_phases),
                })
            })?
            .map(|h| {
                Ok::<_, String>(CheckedComponent {
                    handle: h,
                    generation: self.owner(h)?.generation,
                })
            })
            .transpose()?;
        let component = self.grenades.get_mut(handle.slot).unwrap();
        component.owner.mark_initialized();
        component.pickable = pickable;
        component.initialized = true;
        Ok(())
    }
    pub(super) fn start_grenade(
        &mut self,
        mut world: Option<&mut World>,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if let Some(world) = world.as_deref_mut() {
            self.prepare_grenade_resources(world, handle)?;
        }
        let component = self
            .grenades
            .get(handle.slot)
            .ok_or("Grenade Start payload absent")?;
        if component.started {
            return Ok(());
        }
        if !component.initialized {
            return Err("Grenade Start precedes Initialize".into());
        }
        let index = self.grenade_index(handle)?;
        let id = self.entities[index].shared;
        live.get(id).ok_or("Grenade Start generation stale")?;
        let generation = component.owner.generation;
        if component.start_phase == 0 {
            for handler in [0x12c, 0x12d, 0x131, 0xa2]
                .into_iter()
                .chain((component.flags & grenade::EXPLODE_ON_CONTACT != 0).then_some(0x132))
            {
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
            self.grenades.get_mut(handle.slot).unwrap().start_phase = 1;
        }
        let restored = self.grenades.get(handle.slot).unwrap().owner.flags & (1 << 30) != 0;
        if self.grenades.get(handle.slot).unwrap().start_phase == 1 {
            if component_armed(self, handle)? {
                use crate::gameplay::player::animation::{AnimBasicCommand, PlayMode};
                self.post_initialized_runtime_animation(
                    id,
                    AnimBasicCommand {
                        idx: 0,
                        weight: 1.,
                        mode: if restored {
                            PlayMode::StayOnLastFrame
                        } else {
                            PlayMode::AndStay
                        },
                        rate: 1.,
                        start: 0.,
                        play_on_end: false,
                    },
                )?;
            }
            self.grenades.get_mut(handle.slot).unwrap().start_phase = 2;
        }
        if restored {
            if self.grenades.get(handle.slot).unwrap().exploded() {
                let world = world.ok_or("restored exploded Grenade requires visibility owner")?;
                self.set_runtime_actor_visible(world, live, id, false)?;
                live.request_removal(id)?;
                self.dispatch_remove_notice(live, id)?;
                self.grenades.get_mut(handle.slot).unwrap().started = true;
                return Ok(());
            }
            if component_armed(self, handle)? {
                self.grenade_fx(
                    world.ok_or("restored armed Grenade Start requires its FX owner")?,
                    handle,
                    false,
                )?;
            }
        }
        self.grenades.get_mut(handle.slot).unwrap().started = true;
        Ok(())
    }
    pub(super) fn grenade_message(
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
            return Err("Grenade callback owner differs".into());
        };
        if owner != self.owner_id || self.owner(handle)?.generation != generation {
            return Err("Grenade callback generation stale".into());
        }
        let index = self.grenade_index(handle)?;
        if event.id != handler
            || event.target != Some(self.entities[index].shared)
            || live.get(self.entities[index].shared).is_none()
        {
            return Err("Grenade callback target differs".into());
        }
        self.grenade_actions
            .try_reserve(1)
            .map_err(|e| format!("Grenade callback journal: {e}"))?;
        self.grenade_actions.push_back(Action {
            handle,
            generation,
            handler,
            payload: event.payload.clone(),
        });
        Ok(())
    }
    fn grenade_fx(
        &mut self,
        world: &mut World,
        handle: ComponentHandle,
        in_hand: bool,
    ) -> Result<(), String> {
        let key = self.grenade_key(handle)?;
        if !self.grenade_resources_ready(world, handle)? {
            return Err("Grenade armed FX source is not ready".into());
        }
        let component = self.grenades.get(handle.slot).unwrap();
        let mode = if in_hand {
            1
        } else if component.flags & grenade::UNDERWATER != 0 {
            2
        } else {
            0
        };
        let path = match mode {
            1 => component.definition.in_hand_fx.as_deref(),
            2 => component.definition.underwater_fx.as_deref(),
            _ => component.definition.armed_fx.as_deref(),
        };
        let index = self.grenade_index(handle)?;
        let resources = self.grenade_resources.get_mut(&key).unwrap();
        if resources
            .armed
            .as_ref()
            .is_some_and(|(current, _)| path == Some(current.as_ref()))
        {
            return Ok(());
        }
        let mut requests = world
            .get_resource_mut::<EffectRequests>()
            .ok_or("Grenade FX request owner absent")?;
        if let Some((_, ticket)) = resources.armed.take() {
            requests.stop_entity(
                resources
                    .active_scope
                    .take()
                    .unwrap_or_else(|| resources.scope.clone()),
            );
            resources.stopped.push(ticket);
        }
        if let Some(path) = path {
            let ticket = requests.start_entity(
                resources.scope.clone(),
                path,
                placement(&self.entities[index].name),
            );
            resources.armed = Some((Arc::from(path), ticket));
        }
        Ok(())
    }
    fn stop_grenade_fx(
        &mut self,
        world: &mut World,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.grenade_key(handle)?;
        let mut requests = world
            .get_resource_mut::<EffectRequests>()
            .ok_or("Grenade FX Stop owner absent")?;
        if let Some(resources) = self.grenade_resources.get_mut(&key) {
            if let Some((_, ticket)) = resources.armed.take() {
                requests.stop_entity(
                    resources
                        .active_scope
                        .take()
                        .unwrap_or_else(|| resources.scope.clone()),
                );
                resources.stopped.push(ticket);
            }
        }
        Ok(())
    }
    fn grenade_held_equipment(
        &self,
        world: &World,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<bool, String> {
        let Some(parent) = live.parent(id)? else {
            return Ok(false);
        };
        let Some((_, class)) = live.get(parent) else {
            return Ok(false);
        };
        Ok(self.throwable_class_is(class, b"arkPlayer")?
            && self.grenade_equipment_parent(world, id)?)
    }
    fn grenade_player_owned(
        &self,
        world: &World,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<bool, String> {
        if self.grenade_held_equipment(world, live, id)? {
            return Ok(true);
        }
        let player = world
            .get_resource::<super::super::player::PlayerState>()
            .and_then(|s| s.player.as_ref())
            .and_then(|p| p.managed_entity());
        Ok(live.owner(id)?.is_some_and(|owner| Some(owner) == player))
    }
    fn grenade_equipment_parent(&self, world: &World, id: EntityId) -> Result<bool, String> {
        //491080 requires the same entity's actual cpntWeapon, independently
        //from its scene actor bridge or Grenade component presence.
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("Grenade held header absent")?;
        let handle =
            self.entities[index]
                .components
                .find_component([self.classes[19]; 2], [0, 0], |h| {
                    Ok(ComponentLookupState {
                        flags: self.owner(h)?.flags,
                        start_phases: self
                            .declarations
                            .get(&h.packed())
                            .map(|d| d.scheduling.start_phases),
                    })
                })?;
        let source = handle
            .and_then(|h| {
                self.grenades
                    .get(h.slot)
                    .map(|c| (h.packed(), c.owner.generation))
            })
            .and_then(|key| self.grenade_resources.get(&key))
            .and_then(|r| r.held_weapon.as_ref());
        let _ = world;
        Ok(source.is_some_and(|source| source.strong_count() != 0))
    }
    fn grenade_attribute(world: &World, player_owned: bool, delay: bool) -> Result<f32, String> {
        let attributes = &world
            .get_resource::<super::super::player::PlayerState>()
            .and_then(|s| s.player.as_ref())
            .ok_or("Grenade AttributesPlayer owner absent")?
            .attributes;
        let tag = match (delay, player_owned) {
            (true, true) => "ARK_ATTRIBUTE_GRENADE_PLAYER_DELAY_FACTOR",
            (true, false) => "ARK_ATTRIBUTE_GRENADE_ENEMY_DELAY_FACTOR",
            (false, true) => "ARK_ATTRIBUTE_PLAYER_GRENADE_EXPLOSION_FAIL_CHANCE",
            (false, false) => "ARK_ATTRIBUTE_NPC_GRENADE_EXPLOSION_FAIL_CHANCE",
        };
        Ok(if attributes.1.contains_key(tag) {
            attributes.get(tag)
        } else if delay {
            1.
        } else {
            0.
        })
    }
    fn grenade_update_pause(&mut self, handle: ComponentHandle, pause: bool) -> Result<(), String> {
        if self.owner(handle)?.flags & 0xc < 8 {
            return Ok(());
        }
        let mut registry = self
            .registry
            .take()
            .ok_or("Grenade update registry borrowed")?;
        let result = if pause {
            registry.pause_updates(handle, self)
        } else {
            registry.resume_updates(handle, self)
        };
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn grenade_arm(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        in_hand: bool,
        bypass_hit_floor: bool,
    ) -> Result<(), String> {
        let checked = self
            .grenade_component(live, id)?
            .ok_or("Arm target has no actual Grenade")?;
        if self.grenades.get(checked.handle.slot).unwrap().exploded() {
            return Ok(());
        }
        if !self.grenade_resources_ready(world, checked.handle)? {
            return Err("Grenade arm source dependencies pending".into());
        }
        let player_owned = self.grenade_player_owned(world, live, id)?;
        let factor = Self::grenade_attribute(world, player_owned, true)?;
        if self.grenades.get(checked.handle.slot).unwrap().armed() {
            return self.grenade_fx(world, checked.handle, in_hand);
        }
        self.remove_grenade_pickable(world, live, events, checked.handle, false)?;
        self.grenades
            .get_mut(checked.handle.slot)
            .unwrap()
            .arm_fuse(factor, bypass_hit_floor);
        self.grenade_update_pause(checked.handle, false)?;
        self.grenade_fx(world, checked.handle, in_hand)?;
        if let Some(owner) = live.owner(id)?.filter(|&id| live.get(id).is_some()) {
            events.post(Event {
                id: 0x1a3,
                target: Some(owner),
                payload: Payload::None,
            });
        }
        Ok(())
    }
    pub(in crate::app) fn grenade_disarm(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
    ) -> Result<(), String> {
        let checked = self
            .grenade_component(live, id)?
            .ok_or("Disarm target has no actual Grenade")?;
        if self
            .grenades
            .get_mut(checked.handle.slot)
            .unwrap()
            .disarm_fuse()
        {
            self.grenade_update_pause(checked.handle, true)?;
            self.stop_grenade_fx(world, checked.handle)?;
            if let Some(owner) = live.owner(id)?.filter(|&id| live.get(id).is_some()) {
                events.post(Event {
                    id: 0x1a4,
                    target: Some(owner),
                    payload: Payload::None,
                });
            }
        }
        Ok(())
    }
    pub(in crate::app) fn grenade_reset(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        _events: &mut EntityEvents,
        id: EntityId,
    ) -> Result<(), String> {
        let checked = self
            .grenade_component(live, id)?
            .ok_or("Reset target has no actual Grenade")?;
        self.stop_grenade_fx(world, checked.handle)?;
        let component = self.grenades.get(checked.handle.slot).unwrap();
        if component.armed() && !component.exploded() {
            self.grenade_update_pause(checked.handle, true)?;
        }
        self.grenades
            .get_mut(checked.handle.slot)
            .unwrap()
            .reset_fuse();
        Ok(())
    }
    pub(in crate::app) fn admit_held_grenade(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        definition: Arc<crate::assets::grenades::Definition>,
        scheduling: Arc<DeclarationScheduling>,
        weapon: &Arc<crate::assets::equipment::Descriptor>,
    ) -> Result<CheckedComponent, String> {
        if weapon.throwable.is_none()
            || weapon
                .grenade
                .as_ref()
                .is_none_or(|decl| !Arc::ptr_eq(decl, &definition))
        {
            return Err(
                "held Grenade admission lacks its actual same-entity WeaponThrowable source".into(),
            );
        }
        if let Some(checked) = self.grenade_component(live, id)? {
            return Ok(checked);
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("held Grenade requires its actual equipment reference header")?;
        if !self.entities[index].external {
            return Err("held Grenade identity belongs to a different owner".into());
        }
        let declaration = self.types.declaration(
            self.factories[19],
            Arc::as_ptr(&definition) as usize,
            (*scheduling).clone(),
        )?;
        let mut registry = self
            .registry
            .take()
            .ok_or("held Grenade component registry borrowed")?;
        let result = (|| {
            let handle = ComponentFactory {
                declaration: &declaration,
                pool: &mut self.grenades,
                registry: &mut registry,
                grow_one_at_a_time: false,
            }
            .create(
                &mut self.entities[index].components,
                declaration.scheduling.starts_on_spawn(),
                |_, _, _| Ok(Component::new(definition)),
                |_, _, _| Ok(()),
            )?
            .ok_or("held Grenade component creation disabled")?;
            let checked = CheckedComponent {
                handle,
                generation: self
                    .grenades
                    .get(handle.slot)
                    .ok_or("held Grenade payload absent")?
                    .owner
                    .generation,
            };
            self.declarations.insert(handle.packed(), declaration);
            //This is published equipment admission: its actual component
            //callbacks need the existing FX/resource owners in World.
            self.run_runtime_startup(world, live, events, &mut registry)?;
            self.prepare_grenade_resources(world, checked.handle)?;
            let key = self.grenade_key(checked.handle)?;
            self.grenade_resources.get_mut(&key).unwrap().held_weapon =
                Some(Arc::downgrade(weapon));
            Ok(checked)
        })();
        self.registry = Some(registry);
        result
    }
    pub(in crate::app) fn grenade_transfer(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        target: EntityId,
        source: EntityId,
    ) -> Result<(), String> {
        if target == source {
            return Err("Grenade transfer requires distinct actual entities".into());
        }
        let to = self
            .grenade_component(live, target)?
            .ok_or("thrown actor has no actual Grenade")?;
        let from = self
            .grenade_component(live, source)?
            .ok_or("held actor has no actual Grenade")?;
        if !self.grenade_resources_ready(world, to.handle)? {
            return Err("thrown Grenade source dependencies pending".into());
        }
        let (flags, total, remaining, minimum) = {
            let c = self.grenades.get(from.handle.slot).unwrap();
            (c.flags, c.total, c.remaining, c.minimum)
        };
        let target_state = self.grenades.get_mut(to.handle.slot).unwrap();
        let copied = grenade::ARMED | grenade::UNDERWATER | grenade::EXPLODED;
        target_state.flags = (target_state.flags & !copied) | (flags & copied);
        target_state.total = total;
        target_state.remaining = remaining;
        target_state.minimum = minimum;
        if flags & grenade::EXPLODED != 0 {
            live.request_removal(target)?;
            self.dispatch_remove_notice(live, target)?;
            return self.grenade_reset(world, live, events, source);
        }
        //490f70 moves the same live armed FX actor to the new grenade.
        //The ordinary FX owner updates its binding, preserving particle/sound state.
        let from_key = self.grenade_key(from.handle)?;
        let to_key = self.grenade_key(to.handle)?;
        // Preflight every fallible host lookup before withdrawing the source's
        // live ticket.490f70 transfers ownership, never drops/recreates that FX.
        let target_name = self.entities[self.grenade_index(to.handle)?].name.clone();
        if !self.grenade_resources.contains_key(&from_key)
            || !self.grenade_resources.contains_key(&to_key)
            || world.get_resource::<EffectRequests>().is_none()
        {
            return Err("Grenade FX transfer owners absent".into());
        }
        if self.grenade_resources[&from_key].armed.is_some() {
            self.stop_grenade_fx(world, to.handle)?;
        }
        let moved = self
            .grenade_resources
            .get_mut(&from_key)
            .and_then(|r| r.armed.take());
        if let Some(moved) = moved {
            let source_resources = self.grenade_resources.get_mut(&from_key).unwrap();
            let source_scope = source_resources
                .active_scope
                .take()
                .unwrap_or_else(|| source_resources.scope.clone());
            world
                .get_resource_mut::<EffectRequests>()
                .unwrap()
                .update_entity(source_scope.clone(), placement(&target_name));
            //Scope ownership moves with the real FX instance; reservations remain
            //separate dependency leases until each component's own retirement.
            let resources = self.grenade_resources.get_mut(&to_key).unwrap();
            resources.armed = Some(moved);
            self.grenade_resources
                .get_mut(&to_key)
                .unwrap()
                .active_scope = Some(source_scope);
        }
        if flags & grenade::ARMED != 0 && flags & grenade::EXPLODED == 0 {
            self.grenade_fx(world, to.handle, false)?;
        }
        self.grenade_reset(world, live, events, source)
    }
    pub(super) fn stop_grenade(
        &mut self,
        world: Option<&mut World>,
        live: &LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.grenade_key(handle)?;
        let index = self.grenade_index(handle)?;
        let component = self.grenades.get(handle.slot).unwrap();
        if component.start_phase != 0 {
            let id = self.entities[index].shared;
            let generation = component.owner.generation;
            for handler in [0x12c, 0x12d, 0x131, 0xa2]
                .into_iter()
                .chain((component.flags & grenade::EXPLODE_ON_CONTACT != 0).then_some(0x132))
            {
                events.unregister(
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
        }
        if self
            .grenade_actions
            .iter()
            .any(|action| action.handle == handle && action.generation == key.1)
        {
            return Err("Grenade Stop precedes accepted callback drain".into());
        }
        if self
            .grenade_resources
            .get(&key)
            .is_some_and(|r| r.exploding.is_some())
        {
            return Err("Grenade Stop precedes accepted explosion completion".into());
        }
        self.grenades.get_mut(handle.slot).unwrap().started = false;
        self.grenades.get_mut(handle.slot).unwrap().start_phase = 0;
        let world = world;
        if self.grenades.get(handle.slot).unwrap().flags & grenade::DANGER_REGISTERED != 0 {
            let id = self.entities[index].shared;
            super::super::ai_world::remove_grenade_danger(
                world.as_deref().ok_or("Grenade danger Stop owner absent")?,
                live,
                events,
                id,
            )?;
            self.grenades.get_mut(handle.slot).unwrap().flags &= !grenade::DANGER_REGISTERED;
        }
        if self.grenade_resources.contains_key(&key) {
            let world = world.ok_or("published Grenade Stop requires FX owner")?;
            self.stop_grenade_fx(world, handle)?;
            self.grenade_resources.get_mut(&key).unwrap().closing = true;
        }
        let _ = live;
        Ok(())
    }
    pub(super) fn grenade_stop_complete(
        &self,
        world: Option<&World>,
        handle: ComponentHandle,
    ) -> Result<bool, String> {
        let key = self.grenade_key(handle)?;
        let Some(resources) = self.grenade_resources.get(&key) else {
            return Ok(true);
        };
        let world = world.ok_or("Grenade retirement requires FX acknowledgements")?;
        Ok(!resources.remove_after_explosion
            && resources.exploding.is_none()
            && resources.armed.is_none()
            && resources
                .stopped
                .iter()
                .all(|t| super::super::usable_effects::entity_retired(world, t))
            && resources
                .explosion
                .as_ref()
                .is_none_or(|t| super::super::usable_effects::entity_retired(world, t)))
    }
    pub(super) fn deinitialize_grenade(
        &mut self,
        world: Option<&mut World>,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        if !self.grenade_stop_complete(world.as_deref(), handle)? {
            return Err("Grenade Deinitialize precedes FX retirement".into());
        }
        let key = self.grenade_key(handle)?;
        if let Some(resources) = self.grenade_resources.remove(&key) {
            if resources.owns_reservation {
                world
                    .ok_or("Grenade dependency release owner absent")?
                    .get_resource_mut::<EffectRequests>()
                    .ok_or("Grenade FX dependency release absent")?
                    .release_reservation(resources.dependency_scope);
            }
        }
        let component = self.grenades.get_mut(handle.slot).unwrap();
        component.pickable = None;
        component.initialized = false;
        Ok(())
    }
    fn grenade_contact(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let index = self.grenade_index(handle)?;
        let id = self.entities[index].shared;
        if self.grenades.get(handle.slot).unwrap().flags & grenade::DANGER_REGISTERED == 0 {
            let radius = self
                .grenades
                .get(handle.slot)
                .unwrap()
                .definition
                .danger_radius;
            super::super::ai_world::post_grenade_danger(
                world,
                live,
                events,
                id,
                3,
                self.entities[index].frame.velocities[0],
                self.entities[index].frame.pose.position,
                radius,
            )?;
            self.grenades.get_mut(handle.slot).unwrap().flags |= grenade::DANGER_REGISTERED;
        }
        self.grenades.get_mut(handle.slot).unwrap().contact();
        let projectile = self.entities[index].components.find_component(
            [self.classes[13]; 2],
            [0x28, 0],
            |h| {
                Ok(ComponentLookupState {
                    flags: self.owner(h)?.flags,
                    start_phases: self
                        .declarations
                        .get(&h.packed())
                        .map(|d| d.scheduling.start_phases),
                })
            },
        )?;
        if projectile
            .and_then(|h| self.projectiles.get(h.slot))
            .is_some_and(|p| p.prepared.declaration.can_break)
        {
            self.remove_grenade_pickable(world, live, events, handle, true)?;
        }
        Ok(())
    }
    fn grenade_explode(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
    ) -> Result<(), String> {
        let key = self.grenade_key(handle)?;
        if !self.grenade_resources_ready(world, handle)? {
            return Err("Grenade explosion source dependencies pending".into());
        }
        let index = self.grenade_index(handle)?;
        let id = self.entities[index].shared;
        if self.grenade_resources[&key].exploding.is_none() {
            let held = self.grenade_held_equipment(world, live, id)?;
            let player_owned = self.grenade_player_owned(world, live, id)?;
            let chance = Self::grenade_attribute(world, player_owned, false)?;
            let culprit = live.owner(id)?.filter(|&owner| live.get(owner).is_some());
            let equipment_notice_target = if player_owned {
                Some(
                    world
                        .get_resource::<super::super::player::PlayerState>()
                        .and_then(|s| s.player.as_ref())
                        .and_then(|p| p.managed_entity())
                        .ok_or("Grenade explosion Player notice target absent")?,
                )
            } else {
                None
            };
            //878d20's context2 branch requires the actual complete absence
            //certificate; an unhosted AITampering is never treated as absent.
            if !held {
                self.remove_or_ghost_removes(live, id)?;
            }
            let mut position = self.entities[index].frame.pose.position;
            //The camera substitution tests the parent Player independently
            //of the stricter same-entity cpntWeapon held/cooking predicate.
            if live
                .parent(id)?
                .and_then(|parent| live.get(parent))
                .map(|(_, class)| self.throwable_class_is(class, b"arkPlayer"))
                .transpose()?
                .unwrap_or(false)
            {
                position = world
                    .get_resource::<super::super::player::PlayerState>()
                    .and_then(|s| s.player.as_ref())
                    .ok_or("held Grenade player absent")?
                    .published_camera_frame()
                    .0
                    .to_array();
            }
            let mut state = world
                .get_resource_mut::<super::super::player::PlayerState>()
                .ok_or("Grenade shared RNG owner absent")?;
            let rng = state
                .player
                .as_mut()
                .ok_or("Grenade shared RNG player absent")?;
            //490840 draws failure only outside the hand and rotation only when
            //the selected authored explosion FX actually exists.
            let failed = !held && rng.inventory_random() * 100. < chance;
            let component = self.grenades.get(handle.slot).unwrap();
            let path = if failed {
                component.definition.failed_explosion_fx.as_deref()
            } else {
                component.definition.explosion_fx.as_deref()
            }
            .map(Arc::from);
            let angle = if path.is_some() {
                rng.inventory_random() * f32::from_bits(0x40c90fdb)
            } else {
                0.
            };
            let (sines, cosines) = crate::gameplay::shape_transform::rotation::sin_cos([angle; 4]);
            let (sin, cos) = (sines[0], cosines[0]);
            self.grenade_resources.get_mut(&key).unwrap().exploding = Some(Explosion {
                phase: 0,
                held,
                failed,
                flags: component.flags,
                path,
                frame: ReferenceFrame {
                    position,
                    basis: [[cos, sin, 0.], [-sin, cos, 0.], [0., 0., 1.]],
                },
                culprit,
                equipment_notice_target,
                equipment_notice: Default::default(),
            });
        }
        loop {
            let phase = self.grenade_resources[&key]
                .exploding
                .as_ref()
                .unwrap()
                .phase;
            match phase {
                0 => self.stop_grenade_fx(world, handle)?,
                1 => {
                    if let Some(pickable) = self.entities[index].components.find_component(
                        [self.classes[6]; 2],
                        [0x28, 0],
                        |h| {
                            Ok(ComponentLookupState {
                                flags: self.owner(h)?.flags,
                                start_phases: self
                                    .declarations
                                    .get(&h.packed())
                                    .map(|d| d.scheduling.start_phases),
                            })
                        },
                    )? {
                        let mut registry = self
                            .registry
                            .take()
                            .ok_or("Grenade Pickable Stop registry borrowed")?;
                        let result = registry.request_stop(
                            pickable,
                            &mut super::interaction_components::Startup {
                                world: Some(world),
                                scene: self,
                                live,
                                events,
                            },
                        );
                        self.registry = Some(registry);
                        result?;
                    }
                }
                2 => {
                    let resources = self.grenade_resources.get_mut(&key).unwrap();
                    let pending = resources.exploding.as_ref().unwrap();
                    if let Some(path) = pending.path.as_ref() {
                        let context =
                            crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                                owner: Some(id),
                                culprit: pending.culprit,
                                cannot_incapacitate_player_and_allies: pending.flags
                                    & grenade::CANNOT_INCAPACITATE
                                    != 0,
                                player_impact: false,
                                velocities: [[0.; 3]; 2],
                                contact_speed: 0.,
                                contact_mass: 0.,
                                contact_stimulus_enabled: Some(true),
                            };
                        let ticket = world
                            .get_resource_mut::<EffectRequests>()
                            .ok_or("Grenade explosion FX owner absent")?
                            .start_context_once(
                                Arc::from(format!("{}:explosion", resources.scope)),
                                path,
                                EntityPlacement {
                                    target: crate::gameplay::event_graph::EntityReference::Invalid,
                                    tag: String::new(),
                                    local: ReferenceFrame::default(),
                                    attached: false,
                                },
                                pending.frame,
                                context,
                            );
                        resources.explosion = Some(ticket);
                        //6dd720 stores the managed source identity with the
                        //6dd550/removeOrGhost completion callback, outside hand.
                        resources.remove_after_explosion = !pending.held;
                    }
                }
                3 => self.grenades.get_mut(handle.slot).unwrap().flags |= grenade::EXPLODED,
                4 => {
                    let pending = self
                        .grenade_resources
                        .get_mut(&key)
                        .unwrap()
                        .exploding
                        .as_mut()
                        .unwrap();
                    if let Some(target) = pending.equipment_notice_target {
                        super::super::entity_graphs::player_equipment_exploded(
                            world,
                            live,
                            target,
                            &mut pending.equipment_notice,
                        )?;
                    }
                }
                5 => {
                    let pending = self.grenade_resources[&key].exploding.as_ref().unwrap();
                    if pending.held {
                        self.grenade_update_pause(handle, true)?;
                    } else if !pending.failed {
                        //490840 invokes878d20(false), then entity Hide8784b0.
                        live.set_requested_entity_context(id, 2)?;
                        self.entities[index].entity_flags =
                            (self.entities[index].entity_flags & !0x2000) | 0x800;
                        self.set_runtime_actor_visible(world, live, id, false)?;
                    }
                }
                6 => {
                    if self.grenades.get(handle.slot).unwrap().flags & grenade::DANGER_REGISTERED
                        != 0
                    {
                        super::super::ai_world::remove_grenade_danger(world, live, events, id)?;
                        self.grenades.get_mut(handle.slot).unwrap().flags &=
                            !grenade::DANGER_REGISTERED;
                    }
                }
                7 => {
                    if let Some(owner) = self.grenade_resources[&key]
                        .exploding
                        .as_ref()
                        .unwrap()
                        .culprit
                        .filter(|&owner| live.get(owner).is_some())
                    {
                        events.post(Event {
                            id: 0x1a5,
                            target: Some(owner),
                            payload: Payload::None,
                        });
                    }
                }
                8 => super::super::time_sync::request_entity_unbent(
                    world,
                    self,
                    live,
                    id,
                    self.grenades
                        .get(handle.slot)
                        .unwrap()
                        .definition
                        .unbent_duration,
                )?,
                _ => {
                    self.grenade_resources.get_mut(&key).unwrap().exploding = None;
                    super::super::gameplay_action_diagnostics::grenade::exploded(world, id);
                    return Ok(());
                }
            }
            self.grenade_resources
                .get_mut(&key)
                .unwrap()
                .exploding
                .as_mut()
                .unwrap()
                .phase += 1;
        }
    }
    fn drain_grenade_actions(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        while let Some(action) = self.grenade_actions.front() {
            let handle = action.handle;
            let generation = action.generation;
            let handler = action.handler;
            let payload = action.payload.clone();
            if self.owner(handle)?.generation != generation {
                return Err("accepted Grenade callback generation retired".into());
            }
            let id = self.entities[self.grenade_index(handle)?].shared;
            match (handler, payload) {
                (0x12c, _) => {
                    self.grenades.get_mut(handle.slot).unwrap().flags |= grenade::UNDERWATER;
                    if component_armed(self, handle)?
                        && !self.grenades.get(handle.slot).unwrap().exploded()
                    {
                        self.grenade_fx(world, handle, false)?;
                    }
                    self.grenade_contact(world, live, events, handle)?;
                }
                (0x12d, _) => {
                    self.grenades.get_mut(handle.slot).unwrap().flags &= !grenade::UNDERWATER;
                    if component_armed(self, handle)?
                        && !self.grenades.get(handle.slot).unwrap().exploded()
                    {
                        self.grenade_fx(world, handle, false)?;
                    }
                }
                (0x131, _) => self.grenade_contact(world, live, events, handle)?,
                (0x132, _) => {
                    let applies = self.grenades.get(handle.slot).unwrap().armed()
                        && self.grenades.get(handle.slot).unwrap().flags
                            & grenade::EXPLODE_ON_CONTACT
                            != 0;
                    self.grenades
                        .get_mut(handle.slot)
                        .unwrap()
                        .explode_contact();
                    if applies {
                        super::super::time_sync::request_entity_unbent(
                            world,
                            self,
                            live,
                            id,
                            self.grenades
                                .get(handle.slot)
                                .unwrap()
                                .definition
                                .unbent_duration,
                        )?;
                    }
                }
                (0xa2, Payload::Damage { params, result }) => {
                    if result.health_damage > 0. {
                        self.grenade_arm(world, live, events, id, false, false)?;
                        self.grenades
                            .get_mut(handle.slot)
                            .unwrap()
                            .damage_fuse(params.damage_type.0 & (1 << 44) != 0);
                    }
                }
                _ => return Err(format!("Grenade message{handler:x} lacks actual payload")),
            }
            self.grenade_actions.pop_front();
        }
        Ok(())
    }
    fn advance_grenades(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        let clock = world
            .get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
            .ok_or("Grenade clock owner absent")?;
        if !clock.advanced() {
            return Ok(());
        }
        let marker = clock.clock().map_err(str::to_owned)?.frame_marker();
        let count = self.grenades.roster.sorted_start as usize;
        for cursor in 0..count {
            let slot = self.grenades.roster.indices[cursor];
            let handle = ComponentHandle {
                class_id: self.classes[19],
                slot,
            };
            let component = self.grenades.get(slot).ok_or("scheduled Grenade absent")?;
            if !component.started
                || !component.armed()
                || component.exploded()
                || component.updated_frame == Some(marker)
            {
                continue;
            }
            let index = self.grenade_index(handle)?;
            let id = self.entities[index].shared;
            let held = self.grenade_held_equipment(world, live, id)?;
            let delta = world
                .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                .clock()
                .map_err(str::to_owned)?
                .entity_delta(self.entities[index].timespace)
                .ok_or("Grenade source timespace invalid")?;
            let hidden = self.entities[index].entity_flags & 0x80 != 0;
            let component = self.grenades.get_mut(slot).unwrap();
            let explode = if component.ticked_frame != Some(marker) {
                let explode = component.advance_fuse(delta, hidden, held);
                component.ticked_frame = Some(marker);
                explode
            } else {
                component.remaining <= 0.
            };
            if explode {
                self.grenade_explode(world, live, events, handle)?;
            } else if self.grenades.get(slot).unwrap().flags & grenade::DANGER_REGISTERED != 0 {
                let radius = self.grenades.get(slot).unwrap().definition.danger_radius;
                super::super::ai_world::post_grenade_danger(
                    world,
                    live,
                    events,
                    id,
                    0,
                    self.entities[index].frame.velocities[0],
                    self.entities[index].frame.pose.position,
                    radius,
                )?;
            }
            self.grenades.get_mut(slot).unwrap().updated_frame = Some(marker);
        }
        Ok(())
    }
    fn finish_grenade_explosions(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        // Rare accepted calls only; no scheduled scan or future fuse tick.
        while let Some(key) = self
            .grenade_resources
            .iter()
            .find_map(|(key, r)| r.exploding.as_ref().map(|_| *key))
        {
            self.grenade_explode(world, live, events, ComponentHandle::from_packed(key.0)?)?;
        }
        Ok(())
    }
    fn dispatch_grenade_fx_completions(
        &mut self,
        world: &mut World,
        live: &LiveEntities,
    ) -> Result<(), String> {
        for (&(packed, generation), resources) in &mut self.grenade_resources {
            if !resources.remove_after_explosion
                || !resources
                    .explosion
                    .as_ref()
                    .is_some_and(super::super::usable_effects::effect_completed)
            {
                continue;
            }
            let handle = ComponentHandle::from_packed(packed)?;
            let component = self
                .grenades
                .get(handle.slot)
                .ok_or("Grenade FX completion component absent")?;
            if component.owner.generation != generation {
                return Err("Grenade FX completion generation stale".into());
            }
            let id = self.entities[component
                .owner
                .entity
                .ok_or("Grenade FX completion entity absent")?]
            .shared;
            //6dd550 simply does nothing when the managed identity is gone.
            if live.get(id).is_some() {
                super::super::entity_functions::enqueue_remove_or_ghost_with_live(world, live, id, 0.)?;
            }
            resources.remove_after_explosion = false;
        }
        Ok(())
    }
    fn remove_grenade_pickable(
        &mut self,
        world: &mut World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        handle: ComponentHandle,
        after_hit: bool,
    ) -> Result<(), String> {
        let Some(checked) = self
            .grenades
            .get(handle.slot)
            .ok_or("Grenade Pickable cache absent")?
            .pickable
        else {
            return Ok(());
        };
        let component = self
            .pickables
            .get(checked.handle.slot)
            .ok_or("Grenade Pickable cache is stale")?;
        if component.owner.generation != checked.generation {
            return Err("Grenade Pickable generation differs".into());
        }
        let hold = component.definition.hold.is_some();
        if component.interaction_flags & 2 != 0 || hold != after_hit {
            return Ok(());
        }
        let mut state = self.gameplay_component_drain.take().unwrap_or_default();
        let result = super::lifetime::request_component_release(
            world,
            self,
            live,
            events,
            checked.handle,
            checked.generation,
            &mut state,
        );
        self.gameplay_component_drain = Some(state);
        result?;
        self.grenades.get_mut(handle.slot).unwrap().pickable = None;
        Ok(())
    }
}
fn component_armed(scene: &PhysicsScene, handle: ComponentHandle) -> Result<bool, String> {
    Ok(scene
        .grenades
        .get(handle.slot)
        .ok_or("Grenade state absent")?
        .armed())
}

///Drain only accepted native callbacks at shutdown; no new fuse frame/input.
pub(in crate::app) fn finish_accepted(world: &mut World) -> Result<(), String> {
    run(world, false, true)
}
///8af8a0 executes source stages0/1/2 before the entity Think879ba0.
pub(in crate::app) fn before_actors(world:&mut World)->Result<(),String>{run(world,true,false)}
pub(in crate::app) fn update(world: &mut World) -> Result<(), String> {
    run(world, true, true)
}
fn run(world: &mut World, advance: bool, after_actors:bool) -> Result<(), String> {
    if !world.contains_resource::<PhysicsScene>() {
        return Ok(());
    }
    let mut scene = world.remove_resource::<PhysicsScene>().unwrap();
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("Grenade managed owner absent".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("Grenade event owner absent".into());
    };
    let result = scene
        .drain_grenade_actions(world, &mut live, &mut events)
        .and_then(|()| scene.finish_grenade_explosions(world, &mut live, &mut events))
        .and_then(|()| scene.dispatch_grenade_fx_completions(world, &live))
        .and_then(|()| scene.finish_ai_cues(world,&mut live,&mut events,false))
        .and_then(|()| {
            if !advance{return Ok(())}
            let clock=world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>().ok_or("AI stage clock owner absent")?;
            if !clock.advanced(){return Ok(())}
            let marker=clock.clock().map_err(str::to_owned)?.frame_marker();
            match scene.ai_stage_frame {
                Some((accepted,phase)) if accepted!=marker&&phase<9=>return Err("previous accepted AI source-stage frame is unfinished".into()),
                Some((accepted,_)) if accepted==marker=>{},
                _=>scene.ai_stage_frame=Some((marker,0)),
            }
            loop {
                let phase=scene.ai_stage_frame.ok_or("AI stage retry position absent")?.1;
                if phase==7&&!after_actors{return Ok(())}
                match phase {
                    0=>{
                        //8ab880 drains7ea6e0 before the update task stages.
                        //Concrete Crew callbacks need these actual borrowed
                        //owners; a base-only Scene callback is insufficient.
                        let mut registry=scene.registry.take().ok_or("AI stage component registry borrowed")?;
                        let result=registry.drain_update_changes(&mut super::interaction_components::Startup{
                            world:Some(world),scene:&mut scene,live:&mut live,events:&mut events});
                        scene.registry=Some(registry);result?;
                    }
                    //Dynamic SearchSpot's update_mask1 is actual stage0;
                    //its accepted messages pump before stage1 Timers.
                    1=>scene.update_dynamic_search_spots(world,&live)?,
                    2=>super::super::entity_messages::finish_borrowed(world,&mut scene,&live,&mut events)?,
                    3=>scene.update_ai_timers(world,&live,&mut events)?,
                    4=>super::super::entity_messages::finish_borrowed(world,&mut scene,&live,&mut events)?,
                    5=>{
                        scene.finish_ai_cues(world,&mut live,&mut events,true)?;
                        scene.advance_grenades(world,&mut live,&mut events)?;
                        scene.update_ai_danger_sources(&live)?;
                    }
                    6=>super::super::entity_messages::finish_borrowed(world,&mut scene,&live,&mut events)?,
                    7=>scene.update_crew_decisions(world,&mut events)?,
                    8=>super::super::entity_messages::finish_borrowed(world,&mut scene,&live,&mut events)?,
                    9=>return Ok(()),
                    _=>return Err("AI stage retry position invalid".into()),
                }
                scene.ai_stage_frame=Some((marker,phase+1));
            }
        });
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}

///Preload only the once-published placed pool. Dynamic admissions prepare
///their own exact component generation before the weapon becomes ready.
pub(in crate::app) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(scene) = world.get_resource::<PhysicsScene>() else {
        return Ok(());
    };
    if scene.grenades_prepared {
        return Ok(());
    }
    let mut scene = world.remove_resource::<PhysicsScene>().unwrap();
    let result = (|| {
        for position in 0..scene.grenades.allocated_count() {
            let slot = scene.grenades.roster.indices[position];
            let handle = ComponentHandle {
                class_id: scene.classes[19],
                slot,
            };
            scene.prepare_grenade_resources(world, handle)?;
        }
        scene.grenades_prepared = true;
        Ok(())
    })();
    world.insert_resource(scene);
    result
}
pub(in crate::app) fn startup_pending(world: &World) -> Result<bool, String> {
    let Some(scene) = world.get_resource::<PhysicsScene>() else {
        return Ok(false);
    };
    if !scene.grenades_prepared {
        return Ok(true);
    }
    for position in 0..scene.grenades.allocated_count() {
        let slot = scene.grenades.roster.indices[position];
        if !scene.grenade_resources_ready(
            world,
            ComponentHandle {
                class_id: scene.classes[19],
                slot,
            },
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}
