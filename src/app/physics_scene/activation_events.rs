//! World-wrapper observers on the canonical body's existing publication signals.
//! 1400932d0 installs these five subscriptions before component Start.
use super::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    collision_user_data::{CollisionUserData, velocity::VelocityDataStore},
    physics_world::{PublicationEvent, body_activation_events::BodyActivationEvents},
};

/// Called exactly once by the scene owner before any component body publishes.
pub(in crate::app::physics_scene) fn install(
    signals: &mut PublicationSignals<usize, usize>,
    owner: usize,
) {
    signals.added.subscribe(owner, 0x1400941e0);
    signals.removed.subscribe(owner, 0x140094460);
    signals.destroyed.subscribe(owner, 0x1400943d0);
    signals.activation.activated.subscribe(owner, 0x140094130);
    signals.activation.deactivated.subscribe(owner, 0x140094280);
}

/// Borrowed dispatch into the real registry/pools. The mandatory entity
/// callback borrows the entity's actual d4 flags and hierarchy update owner;
/// no flags shadow vector or alternate component/world registry is retained.
pub(in crate::app::physics_scene) struct WorldActivationObserver<'a, F> {
    pub owner: usize,
    pub rigid: [(
        i32,
        &'a mut ComponentPool<crate::app::rigid_body::RigidBodyComponent>,
    ); 2],
    pub body_events: &'a mut BodyActivationEvents,
    pub contacts: &'a mut crate::gameplay::water_contacts::WaterContactManager,
    pub registry: &'a mut ComponentRegistry,
    pub entity_activation: &'a mut F,
}

impl<F> WorldActivationObserver<'_, F>
where
    F: FnMut(
        usize,
        &mut crate::app::rigid_body::RigidBodyComponent,
        bool,
        u32,
        &mut ComponentRegistry,
        &mut PhysicsActorLinks,
    ) -> Result<(), String>,
{
    pub(in crate::app::physics_scene) fn dispatch(
        &mut self,
        subscriber: usize,
        callback: usize,
        owners: ActivationOwners<'_>,
        event: PublicationEvent<'_>,
        components: &mut TriggerComponentStore<'_>,
    ) -> Result<(), String> {
        if subscriber != self.owner {
            return Err("physics observer belongs to another world".into());
        }
        match (callback, event) {
            (0x1400941e0, PublicationEvent::Added(ids)) => {
                for &body in ids {
                    let flags = owners.bodies.get(body).ok_or("added body is stale")?.flags;
                    self.body_events.added(body, flags)?;
                }
            }
            (0x140094460, PublicationEvent::Removed(ids)) => {
                for &body in ids {
                    self.body_events.removed(body)?;
                }
                //651800/651960 subscribe to the same real removed-body signal.
                self.contacts.remove_bodies(ids);
            }
            (0x1400943d0, PublicationEvent::Destroyed(ids)) => {
                for &body in ids {
                    self.body_events.destroyed(body)?;
                }
            }
            (
                0x140094130,
                PublicationEvent::Activation {
                    ids,
                    active: true,
                    timespaces,
                },
            )
            | (
                0x140094280,
                PublicationEvent::Activation {
                    ids,
                    active: false,
                    timespaces,
                },
            ) => {
                let active = matches!(event, PublicationEvent::Activation { active: true, .. });
                for &body in ids {
                    let rigid = &mut self.rigid;
                    if !self.body_events.begin_transition(body, active, |body| {
                        let record = owners.bodies.get(body).ok_or("activation body is stale")?;
                        let property = owners
                            .motions
                            .get(record.motion_id)
                            .ok_or("activation motion is absent")?
                            .properties_id;
                        let Some(factor) =
                            timespaces.and_then(|spaces| spaces.factor_for_derived(property))
                        else {
                            return Ok(());
                        };
                        let handle = CollisionUserData::decode(record.user_data)?.component;
                        let velocities = if let Some((_, pool)) = rigid
                            .iter_mut()
                            .find(|(class, _)| *class == handle.class_id)
                        {
                            let component = pool
                                .get_mut(handle.slot)
                                .ok_or("deactivation rigid component is absent")?;
                            if component.body() != Some(body) {
                                return Err("deactivation rigid body is stale".into());
                            }
                            &mut component.velocities
                        } else {
                            components.velocities(record.user_data)?
                        };
                        // No fallible operation follows this mutation. The
                        // wrapper records preparation before invoking listeners.
                        velocities.body_deactivated(factor);
                        Ok(())
                    })? {
                        continue;
                    }
                    let registry = &mut *self.registry;
                    let entity_activation = &mut *self.entity_activation;
                    let mut expired_instigator = None;
                    self.body_events
                        .emit_transition(body, active, |listener, body, active| {
                            let (_, pool) = rigid
                                .iter_mut()
                                .find(|(class, _)| *class == listener.handle.class_id)
                                .ok_or("activation listener class is unsupported")?;
                            let component = pool
                                .get_mut(listener.handle.slot)
                                .ok_or("activation listener is absent")?;
                            if component.owner.generation != listener.generation
                                || component.body() != Some(body)
                            {
                                return Err(
                                    "activation listener generation or body is stale".into()
                                );
                            }
                            if listener.callback == 1 {
                                //852730: a deactivation in a progressing entity
                                //clock ends player-instigated collision filtering.
                                if !active && component.instigator_clock_delta > 0. {
                                    owners
                                        .bodies
                                        .get_mut(body)
                                        .ok_or("instigator body disappeared")?
                                        .flags &= !0x80000;
                                    component.instigator_notification = None;
                                    expired_instigator = Some(listener);
                                }
                                return Ok(());
                            }
                            if listener.callback != 0 {
                                return Err("unknown rigid activation callback".into());
                            }
                            let entity = component
                                .owner
                                .entity
                                .ok_or("activation listener entity is absent")?;
                            let flags = owners
                                .bodies
                                .get(body)
                                .ok_or("activation listener body disappeared")?
                                .flags;
                            entity_activation(
                                entity,
                                component,
                                active,
                                flags,
                                registry,
                                components.actors,
                            )
                        })?;
                    if let Some(listener) = expired_instigator {
                        self.body_events.unsubscribe(body, listener)?;
                    }
                }
            }
            _ => return Err("physics observer callback does not match its world signal".into()),
        }
        Ok(())
    }
}
