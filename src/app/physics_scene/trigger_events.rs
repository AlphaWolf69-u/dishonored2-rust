//!779d30/779e00: collision-data acceptance, native touch and ordered d2/d3/d4.
use super::*;
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::LiveEntities,
    physics_world::contact_modes::TriggerTransition,
    wind::WindManager,
};

impl PhysicsScene {
    pub(super) fn dispatch_trigger_transition(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut TriggerAssets,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        manager: Option<&WindManager>,
        transition: TriggerTransition,
    ) -> Result<(), String> {
        let trigger = CollisionUserData::decode(transition.userdata[0])?.component;
        let other = CollisionUserData::decode(transition.userdata[1])?.component;
        if trigger.class_id != self.classes[0] {
            return Err("trigger transition has no Trigger owner".into());
        }
        let component = self
            .triggers
            .get(trigger.slot)
            .ok_or("trigger transition component absent")?;
        let movement = self
            .actors
            .bodies
            .get(component.movement_body)
            .ok_or("trigger transition movement absent")?;
        if movement.body_id != transition.trigger {
            return Err("trigger transition body generation differs".into());
        }
        let index = component
            .owner
            .entity
            .ok_or("trigger transition entity absent")?;
        let target = self
            .entities
            .get(index)
            .ok_or("trigger transition header absent")?
            .shared;
        let other_index = self
            .owner(other)?
            .entity
            .ok_or("trigger other entity absent")?;
        let other_id = self
            .entities
            .get(other_index)
            .ok_or("trigger other header absent")?
            .shared;
        if live.get(target).is_none() || live.get(other_id).is_none() {
            return Err("trigger transition has retired entity".into());
        }
        let wind = component.wind;
        let water = component.water;
        let settings = component
            .collision
            .ok_or("trigger collision inputs absent")?;
        let accepted = if self.is_rigid_class(other.class_id) {
            let declaration = self
                .rigid_component(other)?
                .definition()
                .ok_or("trigger rigid declaration absent")?;
            declaration.declaration_flags & 1 != 0
                && settings.accepts_layer(
                    declaration
                        .settings
                        .collision_filter()
                        .ok_or("trigger rigid layer absent")?
                        & 31,
                )
        } else if other.class_id == self.classes[0] || other.class_id == self.classes[14] {
            true //Trigger/PhysicsWorld collision-data virtual10=11d080.
        } else {
            return Err("unported collision-data virtual10 in Trigger receiver".into());
        };
        let current = (transition.other, transition.entered);
        let component = self
            .triggers
            .get_mut(trigger.slot)
            .ok_or("trigger transition component absent")?;
        if let Some((body, entered, _)) = component.transition_delivery {
            if (body, entered) != current {
                return Err("trigger delivery retry differs".into());
            }
        } else {
            component.transition_delivery = Some((current.0, current.1, 0));
        }
        let stage = component.transition_delivery.unwrap().2;
        if stage == 0 {
            if accepted {
                self.trigger_touch(
                    live,
                    events,
                    trigger,
                    index,
                    other_index,
                    transition.entered,
                )?;
            }
            self.triggers
                .get_mut(trigger.slot)
                .unwrap()
                .transition_delivery
                .as_mut()
                .unwrap()
                .2 = 1;
        }
        if stage <= 1 {
            //779d30 delivers Wind independently of collision-data acceptance.
            if let Some(wind) =
                wind.filter(|_| (1u32 << (transition.other_filter_info & 31)) & 0x5018 != 0)
            {
                let owner = self.owner(wind)?;
                let wind_entity = owner.entity.ok_or("trigger Wind entity absent")?;
                let name = &self
                    .entities
                    .get(wind_entity)
                    .ok_or("trigger Wind header absent")?
                    .name;
                let manager = manager.ok_or("canonical WindManager absent for Wind transition")?;
                let source_index = manager
                    .trigger_entities
                    .iter()
                    .position(|entry| entry.eq_ignore_ascii_case(name))
                    .ok_or("trigger Wind has no canonical payload")?;
                let priority = manager
                    .trigger_shapes
                    .get(source_index)
                    .ok_or("trigger Wind declaration absent")?
                    .0;
                events.post(Event {
                    id: 0xd4,
                    target: Some(other_id),
                    payload: Payload::WindContact {
                        wind,
                        body: transition.other,
                        entered: transition.entered,
                        source_index,
                        priority,
                    },
                });
            }
            self.triggers
                .get_mut(trigger.slot)
                .unwrap()
                .transition_delivery
                .as_mut()
                .unwrap()
                .2 = 2;
        }
        if let Some(water) = water {
            self.water_body_contact(
                world,
                geometry,
                water,
                transition.other,
                transition.entered,
                events,
            )?;
        }
        self.triggers
            .get_mut(trigger.slot)
            .unwrap()
            .transition_delivery = None;
        Ok(())
    }

    fn trigger_touch(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        trigger: ComponentHandle,
        index: usize,
        other_index: usize,
        entered: bool,
    ) -> Result<(), String> {
        let target = self.entities[index].shared;
        let other = self.entities[other_index].shared;
        let component = self
            .triggers
            .get(trigger.slot)
            .ok_or("native Trigger component absent")?;
        if entered && component.spawn_owner_bit30 && live.trigger_is_touching(target, other)? {
            return Ok(());
        }
        //7854d0/7855d0 are reflected native handlers on arkTrigger. idEntity
        //has no touch/untouch handler. No invented script callback is invoked.
        let (_, mut class) = live.get(target).ok_or("native Trigger entity retired")?;
        let mut native = false;
        loop {
            let classes = self.types.classes();
            let metadata = classes
                .metadata(class)
                .ok_or("native Trigger class absent")?;
            if classes.name_text(metadata.name) == Some(b"arkTrigger") {
                native = true;
                break;
            }
            let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                break;
            };
            class = base;
        }
        if native {
            if entered {
                if self.entities[other_index].entity_flags & 0x10000 == 0 {
                    live.trigger_touch(target, other)?;
                }
            } else {
                live.trigger_untouch(target, other)?;
            }
        }
        if entered {
            self.entity_flags_d4[index] |= 4;
        }
        events.post(Event {
            id: 0xd2,
            target: Some(target),
            payload: Payload::TriggerTouch {
                trigger,
                other,
                entered,
            },
        });
        events.post(Event {
            id: 0xd3,
            target: Some(other),
            payload: Payload::TriggerContact { trigger, entered },
        });
        Ok(())
    }
    ///884980 ->883d10 ->8dc740(12c) ->7856e0. The reflected Remove
    ///notice is separate from numeric entity messages and body contact exits.
    pub(in crate::app) fn dispatch_remove_notice(
        &mut self,
        live: &mut LiveEntities,
        subject: crate::gameplay::entity_lifetime::EntityId,
    ) -> Result<(), String> {
        let snapshot = live.event_observer_snapshot(subject)?;
        for observer in snapshot {
            let (_, mut class) = live.get(observer).ok_or("Remove notice observer retired")?;
            let mut native = false;
            loop {
                let classes = self.types.classes();
                let metadata = classes
                    .metadata(class)
                    .ok_or("Remove notice observer class absent")?;
                if classes.name_text(metadata.name) == Some(b"arkTrigger") {
                    native = true;
                    break;
                }
                let Some(base) = classes.hierarchy().node(class).and_then(|node| node.base) else {
                    break;
                };
                class = base;
            }
            if !native {
                return Err("Remove notice has an unported receiver callback".into());
            }
            //7856e0 invokes actual native untouch, then removes any surviving
            //list entry.7855d0 already performs both reciprocal unregisters.
            live.trigger_untouch(observer, subject)?;
        }
        Ok(())
    }
}
