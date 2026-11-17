//! Base world health on actual admitted entity/component identities.
//! 717e70/7181c0 startup, 717800 calculation, 717950/717fe0/878d20 death.
use super::*;
use crate::gameplay::{
    component_state::ComponentInstance,
    damage::{Context, DamageResult, Parameters, Receiver},
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities, Stage},
};

impl PhysicsScene {
    pub(in crate::app) fn mark_player_incapacitated(&mut self,id:EntityId)->Result<(),String>{
        let index=*self.shared_indices.get(&id).ok_or("incapacitated Player has no canonical scene header")?;
        let actor=self.entities.get_mut(index).ok_or("incapacitated Player header is absent")?;
        if actor.retired||actor.shared!=id{return Err("incapacitated Player generation is stale".into())}
        actor.entity_flags=(actor.entity_flags&!0x2000)|0x800;
        Ok(())
    }
    ///8769f0 records bit0 in the A5 packet at production, using the actual
    ///idEntity+d1 flags and current28c context before deferred context commit.
    pub(in crate::app) fn incapacitation_non_counting(&self,live:&LiveEntities,id:EntityId)->Result<bool,String>{
        let index=*self.shared_indices.get(&id).ok_or("incapacitation source has no canonical header")?;
        let actor=self.entities.get(index).ok_or("incapacitation source header is absent")?;
        if actor.retired||actor.shared!=id{return Err("incapacitation source generation is stale".into())}
        if actor.entity_flags&0x2800==0{return Ok(false)}
        let context=live.entity_context(id)?.current;
        if context.wrapping_sub(1)&0xffff_fffa==0&&context!=6{return Ok(false)}
        Ok(context==0||context==3||context==4)
    }
}

pub(super) struct HealthComponent {
    pub owner: ComponentOwner,
    pub receiver: Receiver,
    pub maximum: f32,
    pub current: f32,
    pub attacker: Option<EntityId>,
}
impl ComponentInstance for HealthComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
///39edc0 changes the virtual calculation; the base lifecycle is inherited.
pub(super) struct DoorHealthComponent {
    pub base: HealthComponent,
}
impl ComponentInstance for DoorHealthComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.base.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.base.owner
    }
}
#[derive(Clone, Copy)]
pub(super) struct PlayerDamageContext {
    pub id: EntityId,
    pub greater_strength_modified: bool,
}
///Publish the real player's attribute owner after its update and before world
///damage producers. This bridges the ECS borrow; it creates no actor/attribute.
pub(in crate::app) fn publish_player_context(
    world: &mut bevy::prelude::World,
) -> Result<(), String> {
    let context = match super::super::player_lifetime::id(world) {
        None => None,
        Some(id) => {
            let player = world
                .get_resource::<super::super::player::PlayerState>()
                .and_then(|state| state.player.as_ref())
                .ok_or("HealthDoor real Player owner absent")?;
            let tag = "ARK_ATTRIBUTE_STRENGTH_GREATER_STRENGTH";
            Some(PlayerDamageContext {
                id,
                greater_strength_modified: player
                    .attributes
                    .1
                    .get(tag)
                    .is_some_and(|value| player.attributes.get(tag) != value.base),
            })
        }
    };
    if let Some(mut scene) = world.get_resource_mut::<PhysicsScene>() {
        scene.damage_player = context;
    }
    Ok(())
}
impl PhysicsScene {
    pub(super) fn is_health_class(&self, class: i32) -> bool {
        class == self.classes[10] || class == self.classes[16]
    }
    pub(super) fn health_class_range(&self) -> Result<[i32; 2], String> {
        self.types
            .classes()
            .hierarchy()
            .type_range(self.types.factory(self.factories[10])?.class)
    }
    pub(super) fn health_payload(
        &self,
        handle: ComponentHandle,
    ) -> Result<&HealthComponent, String> {
        if handle.class_id == self.classes[10] {
            self.health
                .get(handle.slot)
                .ok_or("Health payload absent".into())
        } else if handle.class_id == self.classes[16] {
            self.health_doors
                .get(handle.slot)
                .map(|door| &door.base)
                .ok_or("HealthDoor payload absent".into())
        } else {
            Err("component is not an admitted Health subtype".into())
        }
    }
    pub(super) fn health_payload_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<&mut HealthComponent, String> {
        if handle.class_id == self.classes[10] {
            self.health
                .get_mut(handle.slot)
                .ok_or("Health payload absent".into())
        } else if handle.class_id == self.classes[16] {
            self.health_doors
                .get_mut(handle.slot)
                .map(|door| &mut door.base)
                .ok_or("HealthDoor payload absent".into())
        } else {
            Err("component is not an admitted Health subtype".into())
        }
    }
    fn actor_is_class(
        &self,
        live: &LiveEntities,
        id: EntityId,
        name: &[u8],
    ) -> Result<bool, String> {
        let Some((_, mut class)) = live.get(id) else {
            return Ok(false);
        };
        loop {
            let metadata = self
                .types
                .classes()
                .metadata(class)
                .ok_or("damage actor class absent")?;
            if self.types.classes().name_text(metadata.name) == Some(name) {
                return Ok(true);
            }
            let Some(base) = self
                .types
                .classes()
                .hierarchy()
                .node(class)
                .and_then(|node| node.base)
            else {
                return Ok(false);
            };
            class = base;
        }
    }
    pub(super) fn evaluate_health(
        &self,
        live: &LiveEntities,
        handle: ComponentHandle,
        params: &Parameters,
        context: Context,
    ) -> Result<DamageResult, String> {
        let health = self.health_payload(handle)?;
        if handle.class_id == self.classes[10] {
            return Ok(health.receiver.evaluate(health.current, params, context));
        }
        let attacker = params.attacker.filter(|&id| live.get(id).is_some());
        let attacker_player = attacker
            .map(|id| self.actor_is_class(live, id, b"arkPlayer"))
            .transpose()?
            .unwrap_or(false);
        let greater_strength = if attacker_player && params.damage_type.0 & 0x80 != 0 {
            let player = self
                .damage_player
                .ok_or("HealthDoor requires current actual Player attribute publication")?;
            if Some(player.id) != attacker {
                return Err("HealthDoor attacker differs from actual Player owner".into());
            }
            player.greater_strength_modified
        } else {
            false
        };
        let npc_reaction = if let Some(inflictor) = params
            .inflictor
            .filter(|&id| params.damage_type.0 & 0x10 != 0 && live.get(id).is_some())
        {
            if self.actor_is_class(live, inflictor, b"arkNPC")? {
                return Err(
                    "HealthDoor NPC hit requires actual AIPhysicsReaction mode2 component+150 bit2"
                        .into(),
                );
            }
            false
        } else {
            false
        };
        Ok(health.receiver.evaluate_door(
            health.current,
            health.maximum,
            params,
            context,
            crate::gameplay::damage::DoorContext {
                attacker_exists: attacker.is_some(),
                attacker_is_player: attacker_player,
                greater_strength_modified: greater_strength,
                declaration_focus_blink_attack: params.declaration_focus_blink_attack,
                inflictor_npc_breaking_reaction: npc_reaction,
            },
        ))
    }
    /// A source may name only a real spawned entity with an admitted health
    /// component. Authored breakable tags and render placements do not create
    /// an entity/component identity or fabricate a damage receiver.
    pub(in crate::app) fn named_damage_target(
        &self,
        live: &LiveEntities,
        name: &str,
    ) -> Result<Option<EntityId>, String> {
        let Some(id) = live.find(name) else {
            return Ok(None);
        };
        Ok(self.health_component(live, id)?.map(|_| id))
    }
    pub(super) fn admit_health_components(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let bindings = entities::damage::health_bindings(document, |path| {
            crate::assets::declaration_texts::read_common_or_catalog(catalog, path)
        })?;
        for (source, binding) in bindings.into_iter().enumerate() {
            // An actual actor owner admits its components. Never allocate an
            // NPC or door header as a substitute for their missing behavior.
            if binding.class != "cpnthealth" && binding.class != "cpnthealthdoor" {
                continue;
            }
            let Some(index) = self
                .entities
                .iter()
                .position(|entity| entity.name == binding.entity_id)
            else {
                continue;
            };
            let kind = if binding.class == "cpnthealthdoor" {
                16
            } else {
                10
            };
            let declaration =
                self.types
                    .declaration(self.factories[kind], source, binding.scheduling)?;
            let base = HealthComponent {
                owner: Default::default(),
                receiver: binding.receiver,
                maximum: binding.maximum,
                current: 1.,
                attacker: None,
            };
            let handle = if kind == 10 {
                ComponentFactory {
                    declaration: &declaration,
                    pool: &mut self.health,
                    registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(base),
                    |_, _, _| Ok(()),
                )?
            } else {
                ComponentFactory {
                    declaration: &declaration,
                    pool: &mut self.health_doors,
                    registry,
                    grow_one_at_a_time: false,
                }
                .create(
                    &mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(DoorHealthComponent { base }),
                    |_, _, _| Ok(()),
                )?
            }
            .ok_or("Health component creation disabled")?;
            self.declarations.insert(handle.packed(), declaration);
        }
        Ok(())
    }
    /// Query the actual base health component through the entity's own list.
    /// A collider without this component never becomes damageable implicitly.
    pub(in crate::app) fn entity_health(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<(f32, f32)>, String> {
        let Some(handle) = self.health_component(live, id)? else {
            return Ok(None);
        };
        let health = self.health_payload(handle)?;
        Ok(Some((health.current, health.maximum)))
    }
    pub(super) fn health_component(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<Option<ComponentHandle>, String> {
        if live.stage(id) != Some(Stage::Spawned) {
            return Err("damage target identity is stale or not spawned".into());
        }
        let Some(&index) = self.shared_indices.get(&id) else {
            return Ok(None);
        };
        //879270 Damage selects Health with877720 mode2, table141c70e88
        //{0x28,0}; the same mode is used by7a7e30/7a8030 health reads.
        self.entities[index].components.find_component(
            self.health_class_range()?,
            [0x28, 0],
            |handle| {
                Ok(
                    crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                        flags: self.owner(handle)?.flags,
                        start_phases: self
                            .declarations
                            .get(&handle.packed())
                            .map(|declaration| declaration.scheduling.start_phases),
                    },
                )
            },
        )
    }
    /// 717950 consumes a source's authored parameters and changes the same
    /// live entity flags/context used by normal component lifecycle updates.
    /// 876660/8767a0/8769f0 bridge the reflected events into the actual
    /// numeric entity message queue, retaining their source payloads.
    pub(in crate::app) fn damage_entity(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        id: EntityId,
        params: &Parameters,
    ) -> Result<Option<DamageResult>, String> {
        let Some(handle) = self.health_component(live, id)? else {
            return Ok(None);
        };
        let index = self
            .owner(handle)?
            .entity
            .ok_or("health target is unlinked")?;
        if self.entities[index].entity_flags & 0x800 != 0 {
            events.post(Event {
                id: 0x1a6,
                target: Some(id),
                payload: Payload::DamageParameters(*params),
            });
            return Ok(Some(DamageResult::default()));
        }
        let context = Context {
            forward: bevy_math::Vec3::from_array(self.entities[index].frame.pose.basis[0]),
            started: self.owner(handle)?.flags & 0xc >= 8,
            ..Default::default()
        };
        let result = self.evaluate_health(live, handle, params, context)?;
        let accepted_type = self
            .health_payload(handle)?
            .receiver
            .accepts(params.damage_type);
        let sleep = !result.incapacitated
            && accepted_type
            && params
                .damage_type
                .intersects(crate::gameplay::damage::DamageType::SLEEP)
            && self.entities[index].entity_flags & 0x2800 == 0;
        {
            let health = self.health_payload_mut(handle)?;
            if result.accepted {
                if !health.receiver.infinite_health {
                    health.current =
                        (health.current - result.health_damage).clamp(0., health.maximum);
                }
                if result.incapacitated {
                    health.current = 0.;
                    health.attacker = params.attacker;
                }
            }
            if sleep {
                health.attacker = params.attacker;
            }
        }
        if result.accepted && result.incapacitated {
            self.entities[index].entity_flags =
                (self.entities[index].entity_flags & !0x2000) | 0x800;
            live.set_requested_entity_context(id, 2)?;
            events.post(Event {
                id: 0xa5,
                target: Some(id),
                payload: Payload::Incapacitation { params:*params, non_counting:self.incapacitation_non_counting(live,id)? },
            });
        }
        //717950 checks the receiver type again before718110, independently
        //of the calculation's facing filter.
        if sleep {
            live.set_requested_entity_context(id, 1)?;
            self.entities[index].entity_flags |= 0x2000;
            events.post(Event {
                id: 0xa5,
                target: Some(id),
                payload: Payload::Incapacitation { params:*params, non_counting:self.incapacitation_non_counting(live,id)? },
            });
        }
        events.post(Event {
            id: 0xa2,
            target: Some(id),
            payload: Payload::Damage {
                params: *params,
                result,
            },
        });
        if let Some(attacker) = params
            .attacker
            .filter(|attacker| live.get(*attacker).is_some())
        {
            events.post(Event {
                id: 0xa3,
                target: Some(attacker),
                payload: Payload::Damaging {
                    target: id,
                    params: *params,
                    result,
                },
            });
        }
        Ok(Some(result))
    }
    /// Reuse collision user data's canonical component/entity resolution.
    pub(in crate::app) fn damage_collision_target(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        user_data: u64,
        params: &Parameters,
    ) -> Result<Option<DamageResult>, String> {
        if user_data == 0 {
            return Ok(None);
        }
        let index = crate::gameplay::collision_user_data::CollisionUserData::decode(user_data)?
            .entity(|handle| self.owner(handle).ok())?;
        let id = self
            .entities
            .get(index)
            .ok_or("damage collider entity absent")?
            .shared;
        self.damage_entity(live, events, id, params)
    }
}
