//! Accepted object beam441220 ->43f9b0 -> queued149 -> real state84.
//! The same managed target, body and resource leases survive each callback.
use super::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
    player::grapple_catch::Targets,
    powers::{
        far_reach::{ObjectKind, ObjectTarget},
        traversal::PullClass,
    },
    reference_frame::{ReferenceFrame, direction_basis},
};
use bevy::prelude::{Mut, World};
pub(super) struct Request {
    target: ObjectTarget,
    release: crate::app::physics_scene::grapple_release::FarReachRelease,
    phase: u8,
    success: bool,
}
impl Request {
    pub fn new(target: ObjectTarget) -> Self {
        Self {
            target,
            release: Default::default(),
            phase: 0,
            success: false,
        }
    }
}
pub(super) struct Released {
    target: EntityId,
    time: f64,
}
impl Runtime {
    pub fn object_admission(&self, world: &World, player: &Player) -> Result<bool, String> {
        let Some(object) = self.object else {
            return Ok(true);
        };
        let live = world.resource::<LiveEntities>();
        if live.get(object.entity).is_none() {
            return Ok(false);
        }
        if object.kind == ObjectKind::Pickup {
            return world
                .resource::<crate::app::physics_scene::PhysicsScene>()
                .grapple_pickup_available(live, player, object.entity);
        }
        Ok(true)
    }
    ///Refresh440db0's actual local-clock interval, including before state84's
    ///Activate. The state owns collision masking/restoration, not this effect.
    fn publish_released(
        &mut self,
        world: &World,
        player: &mut Player,
        clock: &mut ClockRuntime,
    ) -> Result<(), String> {
        let Some(released) = self.released.as_ref() else {
            return Ok(());
        };
        let live = world.resource::<LiveEntities>();
        let targets = if live.get(released.target).is_some() {
            let scene=world.resource::<crate::app::physics_scene::PhysicsScene>();
            crate::app::time_sync::register_entity_time(scene,live,clock,released.target)?;
            let now = f64::from(clock.entity_time(released.target).map_err(str::to_owned)? as f32);
            scene.far_reach_released_targets(
                live,
                released.target,
                now - released.time,
                self.entity_delay,
            )?
        } else {
            Targets::default()
        };
        player.set_grapple_catch_targets(targets);
        if targets.grappled.is_none() {
            self.released = None;
        }
        Ok(())
    }
    pub fn service_objects(
        &mut self,
        world: &mut World,
        player: &mut Player,
        clock: &mut ClockRuntime,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        self.publish_released(world, player, clock)?;
        let Some(mut request) = self.object_request.take() else {
            return Ok(());
        };
        let result = (|| {
            let target = request.target.entity;
            if world.resource::<LiveEntities>().get(target).is_none() {
                self.pending = false;
                self.object = None;
                player.stop_grapple_beam();
                return Ok(true);
            }
            let frame = world
                .resource::<crate::app::physics_scene::PhysicsScene>()
                .published_entity_frame(world.resource::<LiveEntities>(), target)?;
            let origin = bevy_math::Vec3::from_array(frame.pose.position);
            self.beam_destination = origin;
            let remaining = player.retarget_grapple_beam(origin)?;
            if request.phase == 0 {
                if player.machine.is_active(StateId::USE_POWER)
                    || !remaining.is_some_and(|remaining| remaining <= self.start_pull_distance)
                {
                    return Ok(false);
                }
                //441220 selects by the actual registered class hierarchy;
                //87cb90's explicit pose clears binding, keeping source context.
                let path = if request.target.movable {
                    self.object_impact_fx[0].as_deref()
                } else if request.target.kind == ObjectKind::Pickup {
                    self.object_impact_fx[1].as_deref()
                } else {
                    None
                };
                if let Some(path) = path {
                    crate::app::power_execution::post_grapple_impact(
                        world,
                        path,
                        target,
                        ReferenceFrame {
                            position: origin.to_array(),
                            basis: direction_basis(request.target.normal).to_cols_array_2d(),
                        },
                    )?;
                }
                request.phase = 1;
            }
            if request.phase == 1 && request.target.kind == ObjectKind::Pickup {
                request.success = world.resource_scope(
                    |world, scene: Mut<crate::app::physics_scene::PhysicsScene>| {
                        world.resource_scope(|world, live: Mut<LiveEntities>| -> Result<bool, String> {
                            let eligible = scene
                                .grapple_pickable(&live, target)?
                                .is_some_and(|(_, pickable)| pickable.interaction_flags & 2 == 0)
                                && scene.grapple_has_interactable(&live, target)?;
                            if !eligible {
                                return Ok(false);
                            }
                            //8024d0 returns void;43f9b0 marks this handoff
                            //successful even if47's admission rejects the request.
                            let _ = crate::app::pickups::request(
                                world, &scene, &live, player, target,
                            )?;
                            Ok(true)
                        })
                    },
                )?;
                //Actual non-Movable Pickup is a47 handoff or rejection;
                //it never falls through to an invented physical149 release.
                player.stop_grapple_beam();
                request.phase = 5;
            }
            if request.phase == 1 {
                if request.target.kind == ObjectKind::Grenade {
                    world.resource_scope(
                        |world, mut scene: Mut<crate::app::physics_scene::PhysicsScene>| {
                            scene.restore_grapple_grenade_delay(
                                world.resource::<LiveEntities>(),
                                target,
                            )
                        },
                    )?;
                }
                request.phase = 2;
            }
            if request.phase == 2 {
                let view = player.view();
                let offset =
                    self.catch_offsets[usize::from(player.machine.is_active(StateId::SLIDING))];
                let catch = view.origin + view.axis * offset;
                let impulse = self.pull.impulse(
                    if request.target.kind == ObjectKind::Grenade {
                        PullClass::Grenade
                    } else {
                        PullClass::Movable
                    },
                    origin,
                    catch,
                    player.physics.axis().y_axis,
                );
                let linked = if request.target.kind == ObjectKind::Movable {
                    crate::app::runtime_entities::movable_linked_entity(world, target)?
                } else {
                    None
                }; //Grenade jumps directly to43ff84; no Holder lookup.
                request.success = world.resource_scope(
                    |world, mut scene: Mut<crate::app::physics_scene::PhysicsScene>| {
                        scene.release_far_reach_object(
                            world.resource::<LiveEntities>(),
                            events,
                            target,
                            request.target.kind,
                            linked,
                            impulse.linear.to_array(),
                            impulse.angular.to_array(),
                            &mut request.release,
                        )
                    },
                )?;
                request.phase = 3;
            }
            if request.phase == 3 {
                if request.success {
                    crate::app::time_sync::register_entity_time(
                        world.resource::<crate::app::physics_scene::PhysicsScene>(),
                        world.resource::<LiveEntities>(),clock,target,
                    )?;
                    let time = f64::from(clock.entity_time(target).map_err(str::to_owned)? as f32);
                    self.released = Some(Released { target, time });
                    self.publish_released(world, player, clock)?;
                    //8022e0 queues REQUEST84 (kind1), not Quit(kind2).
                    //691110 then masks this same released target on Activate.
                    player.request_grapple_catch();
                } else {
                    player.stop_grapple_beam();
                }
                request.phase = 4;
            }
            if request.phase == 4 {
                if request.success && request.target.kind == ObjectKind::Movable {
                    world.resource_scope(
                        |world, mut scene: Mut<crate::app::physics_scene::PhysicsScene>| {
                            scene
                                .set_breakable_thrown(
                                    world.resource::<LiveEntities>(),
                                    target,
                                    true,
                                )
                                .map(|_| ())
                        },
                    )?;
                }
                request.phase = 5;
            }
            if request.phase == 5 {
                if request.success {
                    let mut blind = Vec::with_capacity(12);
                    blind.extend(1u32.to_le_bytes());
                    blind.extend(0u32.to_le_bytes());
                    blind.extend(self.blind_duration.to_le_bytes());
                    events.post(Event {
                        id: 0x6c,
                        target: Some(target),
                        payload: Payload::Bytes(blind),
                    });
                    let mut notice = Vec::with_capacity(16);
                    notice.extend(0x15u32.to_le_bytes());
                    for value in origin.to_array() {
                        notice.extend(value.to_le_bytes());
                    }
                    events.post(Event {
                        id: 0x73,
                        target: player.managed_entity(),
                        payload: Payload::Bytes(notice),
                    });
                }
                events.post(Event {
                    id: if request.success { 0x181 } else { 0x182 },
                    target: Some(target),
                    payload: Payload::None,
                });
                self.pending = false;
                self.object = None;
            }
            Ok(true)
        })();
        //A host publication error retains the accepted callback cursor. It
        //cannot enqueue a second149 or restart a grenade fuse on retry.
        if !matches!(&result, Ok(true)) {
            self.object_request = Some(request);
        }
        result.map(|_| ())
    }
}
