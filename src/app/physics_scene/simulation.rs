//! Canonical game-clock physics frame, 097bc0 -> c8e0a0 -> 099a00.
//! Retained phase/cursors make a failed publication retry the accepted frame,
//! rather than reapply queued impulses or integrate it a second time.
use super::*;
use crate::app::model_motion::ModelPoseUpdates;
use crate::gameplay::{
    body_activation::ActivationOwners,
    collision_user_data::{CollisionUserData, velocity::CollisionVelocities},
    constraint_solver::step::SolverParameters,
    entity_events::EntityEvents,
    entity_lifetime::LiveEntities,
    game_clock::GameClock,
    game_clock_runtime::ClockRuntime,
    physics_world::{PublicationEvents, contacts::Contacts},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pre,
    Water,
    Wind,
    Commands,
    Begin,
    Profiles,
    Scale,
    Solve,
    ContactCallbacks,
    TriggerEvents,
    DeferredContacts,
    ContactCommands,
    Capture,
    Notifications,
    Post,
}
struct Frame {
    marker: i32,
    clock: GameClock,
    deltas: [f32; 2],
    phase: Phase,
    solver: Option<SolverParameters>,
    cursor: usize,
    side_cursor: usize,
    velocity: Option<VelocityChange>,
}
struct VelocityChange {
    body: u32,
    handle: ComponentHandle,
    linear: [f32; 4],
    angular: [f32; 4],
    flags: u32,
    wake: bool,
    stage: u8,
}
pub(super) struct PhysicsFrameDriver {
    contacts: Contacts,
    frame: Option<Frame>,
    completed: Option<i32>,
    bodies: Vec<u32>,
    shutdown_cursor: usize,
    last_solver: Option<SolverParameters>,
}
impl PhysicsFrameDriver {
    pub(super) fn observed_completed_frame(&self)->Option<i32> {
        if self.is_idle() {self.completed} else {None}
    }
    pub(super) fn observe_world_support(&self, storage: &WorldStorage, body: u32) -> Option<crate::gameplay::physics_world::contacts::WorldSupportObservation> {
        if !self.is_idle() { return None; }
        self.contacts.observe_world_support(storage, body, self.completed?)
    }
    pub(super) fn observe_body_contact(&self,storage:&WorldStorage,body:u32)->Option<crate::gameplay::physics_world::contacts::BodyContactObservation>{
        if !self.is_idle(){return None;}
        self.contacts.observe_body_contact(storage,body,self.completed?)
    }

    pub(super) fn write_body_diagnostics(&self, storage: &WorldStorage, body: u32, report: &mut String) {
        self.contacts.write_body_diagnostics(storage, body, report);
    }

    pub(super) fn is_idle(&self) -> bool {
        self.frame.is_none()
    }

    /// Mutable shape/body retirement invalidates numeric manifolds without
    /// owning a second resource reference or changing the body's identity.
    pub(super) fn invalidate_body(&mut self, body: u32) -> Result<(), String> {
        if !self.is_idle() {
            return Err("physics contact invalidation overlaps an accepted frame".into());
        }
        self.contacts.invalidate_body(body);
        Ok(())
    }

    /// Contacts resolves its context flags from each actual body pair.
    pub(super) fn new() -> Self {
        Self {
            contacts: Contacts::new(),
            frame: None,
            completed: None,
            bodies: Vec::new(),
            shutdown_cursor: 0,
            last_solver: None,
        }
    }
}
impl PhysicsScene {
    fn collision_velocities_mut(
        &mut self,
        handle: ComponentHandle,
        body: u32,
    ) -> Result<&mut CollisionVelocities, String> {
        if handle.class_id == self.classes[0] {
            let component = self
                .triggers
                .get_mut(handle.slot)
                .ok_or("physics velocity trigger is absent")?;
            let movement = self
                .actors
                .bodies
                .get(component.movement_body)
                .ok_or("physics velocity trigger movement is absent")?;
            if movement.body_id != body {
                return Err("physics velocity trigger body is stale".into());
            }
            Ok(&mut component.velocities)
        } else {
            let component = self.rigid_component_mut(handle)?;
            if component.body() != Some(body) {
                return Err("physics velocity rigid body is stale".into());
            }
            Ok(&mut component.velocities)
        }
    }

    fn drain_retired_contacts(
        &mut self,
        contacts: &mut Contacts,
        step: Option<&SolverParameters>,
        live: &LiveEntities,
    ) -> Result<(), String> {
        if !contacts.retired_callbacks_pending() {
            return Ok(());
        }
        let step = step.ok_or("retired contact has no accepted physical solver step")?;
        let shared = self.world.clone();
        let guard = shared
            .read()
            .map_err(|_| "retired contact world is poisoned")?;
        contacts.dispatch_retired(
            &guard.storage,
            step,
            |contact, cache, rows, event_kind, side| {
                self.capture_throwable_contact(
                    &guard.storage,
                    live,
                    contact,
                    cache,
                    rows,
                    event_kind,
                    side,
                )
            },
        )
    }

    fn drain_trigger_events(
        &mut self,
        contacts: &mut Contacts,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        manager: Option<&crate::gameplay::wind::WindManager>,
    ) -> Result<(), String> {
        if !contacts.trigger_events_pending() {
            return Ok(());
        }
        let shared = self.world.clone();
        let mut guard = shared
            .write()
            .map_err(|_| "trigger event world is poisoned")?;
        let world = &mut *guard;
        let scene = std::cell::RefCell::new(self);
        contacts.dispatch_trigger_events(
            &mut world.storage,
            &mut world.geometry,
            |handle| {
                let scene = scene.borrow();
                if handle.class_id != scene.classes[0] {
                    return Err("trigger event subscriber class differs".into());
                }
                let Some(component) = scene.triggers.get(handle.slot) else {
                    return Ok(None);
                };
                if component.owner.entity.is_none() {
                    return Ok(None);
                }
                let movement = scene
                    .actors
                    .bodies
                    .get(component.movement_body)
                    .ok_or("trigger event movement owner absent")?;
                Ok(Some(movement.body_id))
            },
            |world, geometry, transition| {
                scene
                    .borrow_mut()
                    .dispatch_trigger_transition(world, geometry, live, events, manager, transition)
            },
        )
    }

    ///Sourcecontact cache retirement queues actual Exit notifications while
    ///both bodies and their native per-body listener chains are still alive.
    pub(super) fn retire_body_contacts(
        &mut self,
        mut world: Option<&mut bevy::prelude::World>,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        ids: &[u32],
    ) -> Result<(), String> {
        let mut driver = self
            .simulation
            .take()
            .ok_or("contact retirement driver is absent or reentered")?;
        let result = (|| {
            if !driver.is_idle() {
                return Err("body contact retirement overlaps an accepted physics frame".into());
            }
            for &body in ids {
                driver.invalidate_body(body)?;
            }
            self.drain_retired_contacts(&mut driver.contacts, driver.last_solver.as_ref(), live)?;
            let manager = world
                .as_deref()
                .and_then(|world| {
                    world.get_resource::<super::super::vegetation_wind::VegetationWind>()
                })
                .map(|wind| wind.manager());
            self.drain_trigger_events(&mut driver.contacts, live, events, manager)?;
            if !self.water_notifications.is_empty() {
                let world = world
                    .as_deref_mut()
                    .ok_or("water contact retirement requires its actual model/event owners")?;
                self.drain_water_notifications(world, live, events)?;
            }
            for &body in ids {
                driver.contacts.forget_body(body)?;
            }
            Ok(())
        })();
        self.simulation = Some(driver);
        result
    }

    fn dispatch_frame_commands(&mut self) -> Result<(), String> {
        let shared = self.world.clone();
        let commands = &mut self.body_commands;
        let mut registry = self
            .registry
            .take()
            .ok_or("physics command registry is absent")?;
        let result = (|| {
            scene_publication_events!(self, &mut registry, events);
            let mut guard = shared
                .write()
                .map_err(|_| "physics command world is poisoned")?;
            let world = &mut *guard;
            world
                .storage
                .dispatch_body_commands(&mut world.geometry, &mut events, commands)
        })();
        self.registry = Some(registry);
        result
    }

    /// Actual opcode6 FIFO dispatch. The retirement owner also drains this
    /// before body Stop; a failed listener retains both queue and cursor.
    pub(super) fn drain_activation_commands(&mut self, cursor: &mut usize) -> Result<(), String> {
        let shared = self.world.clone();
        let mut registry = self
            .registry
            .take()
            .ok_or("physics notification registry is absent")?;
        let result = (|| {
            scene_publication_events!(self, &mut registry, events);
            let mut guard = shared
                .write()
                .map_err(|_| "physics notification world is poisoned")?;
            let storage = &mut guard.storage;
            while *cursor < storage.deactivation.body_commands.len() {
                let command = storage.deactivation.body_commands[*cursor];
                events.activation_with_timespaces(
                    ActivationOwners {
                        bodies: &mut storage.bodies,
                        motions: &mut storage.motions,
                        deactivation: &mut storage.deactivation,
                        broadphase: &mut storage.broadphase,
                    },
                    &[command.body],
                    command.activated,
                    storage.motion_timespaces.as_ref(),
                )?;
                *cursor += 1;
            }
            storage.deactivation.body_commands.clear();
            Ok::<_, String>(())
        })();
        self.registry = Some(registry);
        result?;
        *cursor = 0;
        Ok(())
    }

    pub(super) fn advance_physics_frame(
        &mut self,
        runtime: &ClockRuntime,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        poses: &mut ModelPoseUpdates,
        joint_frames: Option<&super::super::joint_frames::JointFrames>,
        player: Option<&crate::gameplay::player::Player>,
        manager: Option<&crate::gameplay::wind::WindManager>,
        world_wind: Option<&crate::gameplay::wind::Wind>,
    ) -> Result<(), String> {
        crate::gameplay::physics_world::thread_policy::enter();
        let clock = runtime.clock().map_err(str::to_owned)?;
        let mut driver = self
            .simulation
            .take()
            .ok_or("physics frame driver is absent or reentered")?;
        let result = self.continue_physics_frame(
            &mut driver,
            clock,
            runtime,
            live,
            events,
            poses,
            joint_frames,
            player,
            manager,
            world_wind,
        );
        self.simulation = Some(driver);
        result
    }

    fn continue_physics_frame(
        &mut self,
        driver: &mut PhysicsFrameDriver,
        clock: &GameClock,
        runtime: &ClockRuntime,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        poses: &mut ModelPoseUpdates,
        joint_frames: Option<&super::super::joint_frames::JointFrames>,
        player: Option<&crate::gameplay::player::Player>,
        manager: Option<&crate::gameplay::wind::WindManager>,
        world_wind: Option<&crate::gameplay::wind::Wind>,
    ) -> Result<(), String> {
        if driver.frame.is_none() {
            if driver.completed == Some(clock.frame_marker()) {
                return Ok(());
            }
            driver.frame = Some(Frame {
                marker: clock.frame_marker(),
                clock: clock.clone(),
                deltas: [
                    clock.delta(3).ok_or("physics clock group3 is absent")?,
                    clock.delta(4).ok_or("physics clock group4 is absent")?,
                ],
                phase: Phase::Pre,
                solver: None,
                cursor: 0,
                side_cursor: 0,
                velocity: None,
            });
        }
        loop {
            let frame = driver
                .frame
                .as_mut()
                .ok_or("physics frame cursor is absent")?;
            match frame.phase {
                Phase::Pre => {
                    //852730 reads the real owner's 877eb0 clock when body
                    //deactivation ends player-instigated collision filtering.
                    for pool in [&mut self.rigid_bodies, &mut self.throwables] {
                        for position in 0..pool.allocated_count() {
                            let slot = pool.roster.indices[position];
                            let component = pool
                                .get_mut(slot)
                                .ok_or("physics clock rigid slot is absent")?;
                            let Some(entity) = component.owner.entity else {
                                continue;
                            };
                            let timespace = self
                                .entities
                                .get(entity)
                                .ok_or("physics clock rigid entity is absent")?
                                .timespace;
                            component.instigator_clock_delta = frame
                                .clock
                                .entity_delta(timespace)
                                .ok_or("physics clock rigid timespace is absent")?;
                        }
                    }
                    self.advance_rigid_pre_physics(&frame.clock, live, joint_frames)?;
                    frame.cursor = 0;
                    frame.phase = Phase::Water;
                }
                Phase::Water => {
                    self.advance_rigid_water(&frame.clock, &mut frame.cursor)?;
                    frame.cursor = 0;
                    frame.phase = Phase::Wind;
                }
                Phase::Wind => {
                    self.advance_rigid_wind(&frame.clock, manager, world_wind, &mut frame.cursor)?;
                    frame.cursor = 0;
                    frame.phase = Phase::Commands;
                }
                Phase::Commands => {
                    self.dispatch_frame_commands()?;
                    frame.phase = Phase::Begin;
                }
                Phase::Begin => {
                    let mut guard = self
                        .world
                        .write()
                        .map_err(|_| "physics frame world is poisoned")?;
                    let storage = &mut guard.storage;
                    let spaces = storage
                        .motion_timespaces
                        .as_mut()
                        .ok_or("physics timespace owner is absent")?;
                    let maximum = spaces.begin_step(frame.deltas)?;
                    frame.solver = maximum.and_then(|_| {
                        SolverParameters::from_game_deltas(&frame.deltas, storage.info.gravity)
                    });
                    frame.phase = if maximum.is_none() {
                        Phase::Notifications
                    } else if spaces.step_requires_update() {
                        Phase::Profiles
                    } else {
                        Phase::Solve
                    };
                }
                Phase::Profiles => {
                    let mut guard = self
                        .world
                        .write()
                        .map_err(|_| "physics profile world is poisoned")?;
                    let storage = &mut guard.storage;
                    if !storage.motion_properties.changed.is_empty() {
                        return Err(
                            "physics profile change has no admitted signal callback host".into(),
                        );
                    }
                    storage
                        .motion_timespaces
                        .as_mut()
                        .ok_or("physics timespace owner is absent")?
                        .update_step_profiles(&mut storage.motion_properties, |_, _, _, _| {
                            unreachable!("checked empty changed signal")
                        })?;
                    let (all, _, _) = storage
                        .motion_timespaces
                        .as_ref()
                        .ok_or("physics timespace owner is absent")?
                        .step_body_policy()?;
                    driver.bodies.clear();
                    if all {
                        for index in 1..=storage.bodies.maximum_index() {
                            if let Some(body) = storage
                                .bodies
                                .get_by_index(index)
                                .filter(|body| body.flags & 3 != 0)
                            {
                                driver.bodies.push(body.id);
                            }
                        }
                    } else {
                        driver
                            .bodies
                            .extend_from_slice(storage.bodies.active_body_ids());
                    }
                    frame.cursor = 0;
                    frame.phase = Phase::Scale;
                }
                Phase::Scale => {
                    while frame.cursor < driver.bodies.len() {
                        if frame.velocity.is_none() {
                            let body = driver.bodies[frame.cursor];
                            let shared = self.world.clone();
                            let guard = shared
                                .read()
                                .map_err(|_| "physics velocity world is poisoned")?;
                            let storage = &guard.storage;
                            let record = storage
                                .bodies
                                .get(body)
                                .ok_or("physics scaled body is stale")?;
                            let motion = storage
                                .motions
                                .get(record.motion_id)
                                .ok_or("physics scaled motion is absent")?;
                            let spaces = storage
                                .motion_timespaces
                                .as_ref()
                                .ok_or("physics timespace owner is absent")?;
                            let (_, old, factors) = spaces.step_body_policy()?;
                            let mut selected = None;
                            for space in 0..2 {
                                if spaces
                                    .original_for_derived(space, motion.properties_id)?
                                    .is_some()
                                {
                                    selected = Some(space);
                                    break;
                                }
                            }
                            let Some(space) = selected else {
                                frame.cursor += 1;
                                continue;
                            };
                            let body_flags = record.flags;
                            let handle = CollisionUserData::decode(record.user_data)?.component;
                            drop(guard);
                            let cache = self.collision_velocities_mut(handle, body)?;
                            let factor = factors[space];
                            let mut flags = cache.requested_linear[3].to_bits();
                            let wake = factor != 0. && old[space] == 0. && flags & 1 != 0;
                            let (linear, angular) = if factor == 0. {
                                if flags & 2 != 0 || body_flags & 8 != 0 {
                                    flags |= 0x3f00_0001;
                                }
                                ([0.; 4], [0.; 4])
                            } else {
                                if wake {
                                    flags = (flags & !1) | 0x3f00_0000;
                                }
                                let mut requested = cache.requested_linear;
                                requested[3] = f32::from_bits(flags);
                                (
                                    requested.map(|v| v * factor),
                                    cache.requested_angular.map(|v| v * factor),
                                )
                            };
                            frame.velocity = Some(VelocityChange {
                                body,
                                handle,
                                linear,
                                angular,
                                flags,
                                wake,
                                stage: 0,
                            });
                        }
                        let change = frame
                            .velocity
                            .as_mut()
                            .ok_or("physics velocity cursor is absent")?;
                        if change.stage == 0 {
                            if change.wake {
                                let mut registry = self
                                    .registry
                                    .take()
                                    .ok_or("physics activation registry is absent")?;
                                let result = (|| {
                                    let shared = self.world.clone();
                                    scene_publication_events!(self, &mut registry, events);
                                    let mut guard = shared
                                        .write()
                                        .map_err(|_| "physics activation world is poisoned")?;
                                    let world = &mut *guard;
                                    world.storage.activate_body(
                                        &mut world.geometry,
                                        &mut events,
                                        change.body,
                                    )
                                })();
                                self.registry = Some(registry);
                                result?;
                            }
                            change.stage = 1;
                        }
                        // 18d2b0 changes the requested flags after explicit
                        // wake, before the immediate velocity setter callbacks.
                        self.collision_velocities_mut(change.handle, change.body)?
                            .requested_linear[3] = f32::from_bits(change.flags);
                        if change.stage == 1 {
                            let mut registry = self
                                .registry
                                .take()
                                .ok_or("physics velocity registry is absent")?;
                            let result = (|| {
                                let shared = self.world.clone();
                                scene_publication_events!(self, &mut registry, events);
                                let mut guard = shared
                                    .write()
                                    .map_err(|_| "physics velocity world is poisoned")?;
                                let world = &mut *guard;
                                world.storage.set_body_velocity(
                                    &mut world.geometry,
                                    &mut events,
                                    change.body,
                                    change.linear,
                                    change.angular,
                                    0,
                                )
                            })();
                            self.registry = Some(registry);
                            result?;
                            change.stage = 2;
                        }
                        let cache = self.collision_velocities_mut(change.handle, change.body)?;
                        cache.requested_linear[3] = f32::from_bits(change.flags);
                        cache.scaled_linear = change.linear;
                        cache.scaled_angular = change.angular;
                        frame.velocity = None;
                        frame.cursor += 1;
                    }
                    let mut guard = self
                        .world
                        .write()
                        .map_err(|_| "physics timespace world is poisoned")?;
                    let spaces = guard
                        .storage
                        .motion_timespaces
                        .as_mut()
                        .ok_or("physics timespace owner is absent")?;
                    spaces.finish_body_pass()?;
                    spaces.finish_step(frame.deltas)?;
                    frame.phase = Phase::Solve;
                }
                Phase::Solve => {
                    let shared = self.world.clone();
                    let mut registry = self
                        .registry
                        .take()
                        .ok_or("physics solve registry is absent")?;
                    let result = (|| {
                        scene_publication_events!(self, &mut registry, events);
                        let mut guard = shared
                            .write()
                            .map_err(|_| "physics solve world is poisoned")?;
                        let world = &mut *guard;
                        if let Some(solver) = frame.solver.as_ref() {
                            return world.storage.step_simulation(
                                &mut world.geometry,
                                &mut events,
                                solver,
                                &mut driver.contacts,
                            );
                        }
                        Ok::<_, String>(true)
                    })();
                    self.registry = Some(registry);
                    if !result? {
                        frame.cursor = 0;
                        frame.side_cursor = 0;
                        frame.phase = Phase::ContactCallbacks;
                        continue;
                    }
                    if let Some(step) = frame.solver {
                        driver.last_solver = Some(step);
                    }
                    frame.phase = Phase::TriggerEvents;
                }
                Phase::ContactCallbacks => {
                    let shared = self.world.clone();
                    let guard = shared
                        .read()
                        .map_err(|_| "prepared contact world is poisoned")?;
                    let step = frame
                        .solver
                        .as_ref()
                        .ok_or("prepared contact has no accepted solver step")?;
                    driver.contacts.dispatch_prepared(
                        &guard.storage,
                        step,
                        &mut frame.cursor,
                        &mut frame.side_cursor,
                        |contact, cache, rows, event_kind, side| {
                            self.capture_throwable_contact(
                                &guard.storage,
                                live,
                                contact,
                                cache,
                                rows,
                                event_kind,
                                side,
                            )
                        },
                    )?;
                    frame.phase = Phase::Solve;
                }
                Phase::TriggerEvents => {
                    //097bc0:094700 immediately follows completion of the
                    //real physics task graph, before outer4c9560 callbacks.
                    self.drain_trigger_events(&mut driver.contacts, live, events, manager)?;
                    frame.phase = Phase::DeferredContacts;
                }
                Phase::DeferredContacts => {
                    //8af460 phase6:7e93f0's table141c710b8 selects
                    //Throwable factory+40 ->951cf0 ->4c9560. Phase5's
                    //+38 callback is4c9810, so these are current-step
                    //records drained after097bc0, not pre-solve updates.
                    self.advance_throwable_contacts(live, events, runtime, player)?;
                    frame.phase = Phase::ContactCommands;
                }
                Phase::ContactCommands => {
                    self.dispatch_frame_commands()?;
                    let guard = self
                        .world
                        .read()
                        .map_err(|_| "physics capture world is poisoned")?;
                    driver.bodies.clear();
                    driver
                        .bodies
                        .extend_from_slice(guard.storage.bodies.active_body_ids());
                    frame.cursor = 0;
                    frame.phase = Phase::Capture;
                }
                Phase::Capture => {
                    while frame.cursor < driver.bodies.len() {
                        let body = driver.bodies[frame.cursor];
                        let shared = self.world.clone();
                        let guard = shared
                            .read()
                            .map_err(|_| "physics capture world is poisoned")?;
                        let storage = &guard.storage;
                        let record = storage
                            .bodies
                            .get(body)
                            .ok_or("physics capture body is stale")?;
                        let motion = storage
                            .motions
                            .get(record.motion_id)
                            .ok_or("physics capture motion is absent")?;
                        let factor = storage
                            .motion_timespaces
                            .as_ref()
                            .ok_or("physics timespace owner is absent")?
                            .factor_for_derived(motion.properties_id);
                        if let Some(factor) = factor {
                            let handle = CollisionUserData::decode(record.user_data)?.component;
                            let linear = motion.linear_velocity;
                            // Stored angular lanes are principal-inertia local;
                            // 099a00's actual body accessor returns world lanes.
                            let angular = motion.world_angular_velocity();
                            drop(guard);
                            self.collision_velocities_mut(handle, body)?
                                .capture_body_velocities(
                                    linear,
                                    angular,
                                    if factor == 0. { 0. } else { 1. / factor },
                                );
                        }
                        frame.cursor += 1;
                    }
                    frame.phase = Phase::Notifications;
                    frame.cursor = 0;
                }
                Phase::Notifications => {
                    self.drain_activation_commands(&mut frame.cursor)?;
                    frame.phase = Phase::Post;
                }
                Phase::Post => {
                    self.advance_rigid_actors(&frame.clock, live, poses, joint_frames)?;
                    driver.completed = Some(frame.marker);
                    driver.frame = None;
                    driver.bodies.clear();
                    return Ok(());
                }
            }
        }
    }
}
pub(in crate::app) fn update(
    scene: Option<bevy::prelude::ResMut<PhysicsScene>>,
    clock: Option<bevy::prelude::Res<ClockRuntime>>,
    live: Option<bevy::prelude::ResMut<LiveEntities>>,
    events: Option<bevy::prelude::ResMut<EntityEvents>>,
    poses: Option<bevy::prelude::ResMut<ModelPoseUpdates>>,
    joints: Option<bevy::prelude::Res<super::super::joint_frames::JointFrames>>,
    player: Option<bevy::prelude::Res<super::super::player::PlayerState>>,
    wind: Option<bevy::prelude::Res<super::super::vegetation_wind::VegetationWind>>,
    world_wind: Option<bevy::prelude::Res<super::super::fog_scene::WorldWind>>,
    profile: Option<bevy::prelude::Res<crate::frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_ref().map(|p| p.measure(crate::frame_profile::PHYSICS_FRAME));
    let Some(mut scene) = scene else {
        return;
    };
    if scene.retirement.is_some() {
        return;
    }
    let result = (|| {
        let runtime = clock
            .as_ref()
            .ok_or("physics frame clock owner is absent")?;
        if !runtime.advanced() {
            return Ok(());
        }
        let mut live = live.ok_or("physics frame live entity owner is absent")?;
        let mut events = events.ok_or("physics frame entity event owner is absent")?;
        let mut poses = poses.ok_or("physics frame pose owner is absent")?;
        let player = player.as_ref().and_then(|state| state.player.as_ref());
        scene.advance_physics_frame(
            runtime,
            &mut live,
            &mut events,
            &mut poses,
            joints.as_deref(),
            player,
            wind.as_ref().map(|wind| wind.manager()),
            world_wind.as_ref().and_then(|wind| wind.0.as_ref()),
        )
    })();
    if let Err(error) = result {
        bevy::log::error!("Physics frame failed: {error}");
    }
}

/// Finish only already accepted work before component Stop. Pausing the game
/// clock cannot strand a prepared solver, queued impulse or activation signal.
pub(in crate::app) fn finish_accepted(world: &mut bevy::prelude::World) -> Result<(), String> {
    let Some(mut scene) = world.remove_resource::<PhysicsScene>() else {
        return Ok(());
    };
    let Some(mut driver) = scene.simulation.take() else {
        world.insert_resource(scene);
        return Err("physics shutdown driver is absent or reentered".into());
    };
    // Scoped owners return even if a source callback suspends retirement.
    let result = world
        .try_resource_scope(|world, mut live: bevy::prelude::Mut<LiveEntities>| {
            world
                .try_resource_scope(|world, mut events: bevy::prelude::Mut<EntityEvents>| {
                    world
                        .try_resource_scope(
                            |world, mut poses: bevy::prelude::Mut<ModelPoseUpdates>| {
                                let runtime = world
                                    .get_resource::<ClockRuntime>()
                                    .ok_or("physics shutdown clock owner is absent")?;
                                let player = world
                                    .get_resource::<super::super::player::PlayerState>()
                                    .and_then(|state| state.player.as_ref());
                                let joints =
                                    world.get_resource::<super::super::joint_frames::JointFrames>();
                                let manager = world
                                    .get_resource::<super::super::vegetation_wind::VegetationWind>()
                                    .map(|wind| wind.manager());
                                let world_wind = world
                                    .get_resource::<super::super::fog_scene::WorldWind>()
                                    .and_then(|wind| wind.0.as_ref());
                                if let Some(clock) =
                                    driver.frame.as_ref().map(|frame| frame.clock.clone())
                                {
                                    scene.continue_physics_frame(
                                        &mut driver,
                                        &clock,
                                        runtime,
                                        &mut live,
                                        &mut events,
                                        &mut poses,
                                        joints,
                                        player,
                                        manager,
                                        world_wind,
                                    )?;
                                }
                                //4c9560 can own accepted records even if no new physical step
                                //is allowed. Finish them without creating a replacement frame.
                                scene.advance_throwable_contacts(
                                    &mut live,
                                    &mut events,
                                    runtime,
                                    player,
                                )?;
                                scene.drain_retired_contacts(
                                    &mut driver.contacts,
                                    driver.last_solver.as_ref(),
                                    &live,
                                )?;
                                scene.drain_trigger_events(
                                    &mut driver.contacts,
                                    &mut live,
                                    &mut events,
                                    manager,
                                )?;
                                scene.dispatch_frame_commands()?;
                                scene.drain_activation_commands(&mut driver.shutdown_cursor)
                            },
                        )
                        .ok_or("physics shutdown pose owner is absent")?
                })
                .ok_or("physics shutdown entity event owner is absent")?
        })
        .ok_or_else(|| "physics shutdown live entities are absent".to_owned())
        .and_then(|result| result);
    scene.simulation = Some(driver);
    world.insert_resource(scene);
    result
}
