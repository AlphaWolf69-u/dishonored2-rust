//! Authored world particle playback. Graph actions create attached FX entities;
//! their components allocate manager instances before deferred group starts.
use super::{
    particle_systems::ParticleSystems,
    world_effects::{WorldEffectDefinitions, WorldEffectFrames, WorldFxClock},
};
use crate::{
    assets::entities::event_graph::atom,
    gameplay::effects::{
        graph::{FxGraphWorld, FxRuntimeParameters, GraphStep},
        instances::FxGraphInstances,
        pool::EffectPoolLinks,
        world_component::{ReferenceReadiness, WorldFxComponent, WorldFxComponentHost},
        world_node::{CreatedEffect, FxNode},
    },
    gameplay::event_graph::EntityReference,
    gameplay::reference_frame::ReferenceFrame,
    rendering::particles::world_sources::{WorldEffectDefinition, WorldFxSources},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct FxEntity {
    owner: FxEntityOwner,
    parameters: FxRuntimeParameters,
    component: WorldFxComponent,
    suspended: bool,
    pose: ReferenceFrame,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
}
enum FxEntityOwner {
    Node {
        graph: usize,
        node: u32,
    },
    Actor {
        name: String,
        active: Option<u32>,
        remove_on_end: bool,
    },
}
struct Instance {
    id: u32,
    entity: u32,
    definition: Arc<WorldEffectDefinition>,
    parameters: FxRuntimeParameters,
    pose: ReferenceFrame,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
    age: f32,
    head: u16,
    sound_head: u16,
    actor_observer: Option<(usize, u32)>,
}
pub(crate) struct WorldParticleState {
    source: Option<Arc<Vec<crate::assets::entities::effects::FxGraph>>>,
    sounds: Option<super::world_sounds::WorldSounds>,
    sound_systems: crate::gameplay::effects::sound::systems::SoundSystems<
        crate::gameplay::sound::Emitter,
        crate::gameplay::sound::playing::PlayingId,
    >,
    graphs: FxGraphInstances,
    actors: HashMap<String, u32>,
    pending_actor_starts: Vec<u32>,
    timers: Vec<(usize, u32)>,
    entities: HashMap<u32, FxEntity>,
    entity_order: Vec<u32>,
    next_entity: u32,
    next_instance: u32,
    pending_starts: Vec<u16>,
    pool: EffectPoolLinks,
    slots: Vec<Option<Instance>>,
    handles: HashMap<u32, u16>,
    started: Vec<(usize, u32, u32)>,
    completed: Vec<(usize, u32, u32)>,
    actor_completed: Vec<(usize, u32, u32)>,
    warnings: HashSet<String>,
    frame: Option<u32>,
    query: Option<crate::gameplay::world_point_query::WorldPointQuery>,
}
impl Default for WorldParticleState {
    fn default() -> Self {
        let pool = EffectPoolLinks::instances();
        Self {
            source: None,
            sounds: None,
            sound_systems: Default::default(),
            graphs: FxGraphInstances::default(),
            actors: HashMap::new(),
            pending_actor_starts: Vec::new(),
            timers: Vec::new(),
            entities: HashMap::new(),
            entity_order: Vec::new(),
            next_entity: 0,
            next_instance: 0,
            pending_starts: Vec::new(),
            slots: (0..pool.next.len()).map(|_| None).collect(),
            pool,
            handles: HashMap::new(),
            started: Vec::new(),
            completed: Vec::new(),
            actor_completed: Vec::new(),
            warnings: HashSet::new(),
            frame: None,
            query: None,
        }
    }
}
impl Drop for WorldParticleState {
    fn drop(&mut self) {
        if let Some(mut sounds) = self.sounds.clone() {
            for instance in self.slots.iter_mut().flatten() {
                if let Err(error) = self
                    .sound_systems
                    .clear(&mut instance.sound_head, &mut sounds)
                {
                    bevy::log::error!("World FX sound teardown failed: {error}");
                }
            }
        }
    }
}
impl WorldParticleState {
    fn report(&mut self, error: String) {
        if self.warnings.insert(error.clone()) {
            bevy::log::warn!("World particle FX unavailable: {error}");
        }
    }
    fn collect(&mut self, graph: usize) {
        for error in self.graphs.collect(graph) {
            self.report(error);
        }
    }
    fn initialize(
        &mut self,
        sources: &WorldEffectDefinitions,
        assets: &WorldFxSources,
        frames: &WorldEffectFrames,
    ) -> Result<(), String> {
        if self
            .source
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &sources.graphs))
        {
            return Ok(());
        }
        if self.source.is_some() {
            return Err("world FX level changed before the shared pool was reset".into());
        }
        self.graphs = FxGraphInstances::new(&sources.graphs, &assets.declarations, sources.seed);
        // 87e8e0 starts a placed actor's declared effect unless startOff is set.
        // It retains the scene actor after playback; NodeFx-created entities
        // have a different completion lifetime below.
        for actor in sources.actors.iter() {
            let parameters = match &actor.parameters {
                Ok(parameters) => parameters,
                Err(error) => {
                    self.report(error.clone());
                    continue;
                }
            };
            let Some(pose) = frames.frame(&actor.owner)? else {
                self.report(format!("FX actor {} has no scene pose", actor.owner));
                continue;
            };
            let id = self.next_entity;
            self.next_entity = id.wrapping_add(1);
            self.actors.insert(actor.owner.clone(), id);
            self.entity_order.push(id);
            if !parameters.start_off && parameters.effect.is_some() {
                self.pending_actor_starts.push(id);
            }
            self.entities.insert(
                id,
                FxEntity {
                    owner: FxEntityOwner::Actor {
                        name: actor.owner.clone(),
                        active: None,
                        remove_on_end: parameters.remove_on_end,
                    },
                    parameters: FxRuntimeParameters {
                        effect: parameters.effect.clone(),
                        location: Some(EntityReference::Named(actor.owner.clone())),
                        target: parameters.target.clone().map(EntityReference::Named),
                        target_tag: parameters.target_tag.clone(),
                        update_position: parameters.update_position,
                        stop_on_target_removal: parameters.stop_on_target_removal,
                        // A placed idEntityFx owns its pose, rather than the
                        // zero-offset attachment created for NodeFx's component.
                        stop_on_parent_removal: false,
                        ..Default::default()
                    },
                    component: Default::default(),
                    suspended: false,
                    pose,
                    motion: crate::gameplay::particles::spawn::SpawnVelocity {
                        center: pose.position,
                        ..Default::default()
                    },
                },
            );
        }
        for error in self.graphs.loading_end() {
            self.report(error);
        }
        self.source = Some(sources.graphs.clone());
        Ok(())
    }
    fn stop(&mut self, handle: u32, systems: &mut ParticleSystems) -> Result<(), String> {
        if let Some(&slot) = self.handles.get(&handle) {
            if let Some(instance) = self.slots[usize::from(slot)].as_ref() {
                systems.stop(instance.head, instance.age);
                self.sound_systems
                    .stop(instance.sound_head, instance.age, &mut systems.seed)?;
            }
            // 1a31f0 -> 1a1880/1a1990 removes the queued start using the
            // source's last-matching swap removal, before the next drain.
            if let Some(index) = self
                .pending_starts
                .iter()
                .rposition(|pending| *pending == slot)
            {
                self.pending_starts.swap_remove(index);
            }
        }
        Ok(())
    }
    fn start_actor(
        &mut self,
        id: u32,
        systems: &mut ParticleSystems,
        frames: &WorldEffectFrames,
        sources: &WorldFxSources,
    ) -> Result<Option<u32>, String> {
        let mut entity = self.entities.remove(&id).ok_or("FX actor was removed")?;
        let result = (|| {
            let FxEntityOwner::Actor {
                active,
                remove_on_end,
                name,
                ..
            } = &entity.owner
            else {
                return Err("FX actor start received an attached effect entity".into());
            };
            if active.is_some() || entity.parameters.effect.is_none() {
                return Ok(None);
            }
            if *remove_on_end {
                return Err(format!(
                    "FX actor {name}: removal on completion awaits the entity lifecycle owner"
                ));
            }
            InstanceHost {
                state: self,
                systems,
                frames,
                sources,
                entity: &entity,
                id,
            }
            .play()
        })();
        if let FxEntityOwner::Actor { active, .. } = &mut entity.owner {
            if let Ok(Some(handle)) = result {
                *active = Some(handle);
            }
        }
        self.entities.insert(id, entity);
        result
    }
    fn retire(&mut self, slot: u16, systems: &mut ParticleSystems) -> Result<(), String> {
        let Some(mut instance) = self.slots[usize::from(slot)].take() else {
            return Ok(());
        };
        systems.clear(&mut instance.head)?;
        if instance.sound_head != u16::MAX {
            let mut sounds = self.sounds.clone().ok_or("FX sound owner is absent")?;
            self.sound_systems
                .clear(&mut instance.sound_head, &mut sounds)?;
        }
        self.handles.remove(&instance.id);
        if let Some(index) = self
            .pending_starts
            .iter()
            .rposition(|pending| *pending == slot)
        {
            self.pending_starts.swap_remove(index);
        }
        if let Some(mut entity) = self.entities.remove(&instance.entity) {
            // 4b1b70 forwards Finished, invalidates the component's handle,
            // disposes its velocity adapter, then queues deletion of this
            // NodeFx-created entity (packet+1c was set to one by79e6a0).
            match &mut entity.owner {
                FxEntityOwner::Node { graph, node } => {
                    self.completed.push((*graph, *node, instance.id))
                }
                FxEntityOwner::Actor { active, .. } => {
                    if *active == Some(instance.id) {
                        *active = None;
                    }
                    if let Some((graph, node)) = instance.actor_observer {
                        self.actor_completed.push((graph, node, instance.entity));
                    }
                    self.entities.insert(instance.entity, entity);
                }
            }
        }
        let mut head = slot;
        self.pool.release_head(&mut head)?;
        Ok(())
    }
    pub fn advance(
        &mut self,
        graphs: &WorldEffectDefinitions,
        sources: &WorldFxSources,
        frames: &WorldEffectFrames,
        systems: &mut ParticleSystems,
        sounds: &super::world_sounds::WorldSounds,
        frame: u32,
        camera: ReferenceFrame,
        underwater: bool,
        world: &crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
    ) -> Result<(), String> {
        if self.frame == Some(frame) {
            return Ok(());
        }
        if self.source.is_none() {
            let audio = sounds.lock();
            for effect in sources
                .effects
                .values()
                .filter_map(|effect| effect.as_ref().ok())
            {
                for group in &effect.groups {
                    for event in group
                        .sounds
                        .iter()
                        .flat_map(|sound| [&sound.start, &sound.stop])
                        .flatten()
                    {
                        if !audio.ready || !audio.events.contains_key(&event.name) {
                            return Err("world FX awaits prepared sound dependencies and listener publication".into());
                        }
                    }
                }
            }
        }
        self.sounds = Some(sounds.clone());
        {
            let mut audio = sounds.lock();
            audio.pump_notifications();
            for notification in std::mem::take(&mut audio.notifications) {
                match notification.kind {
                    crate::gameplay::sound::playing::NotificationKind::End => {
                        self.sound_systems.finished(notification.id)
                    }
                    crate::gameplay::sound::playing::NotificationKind::SourceReady
                    | crate::gameplay::sound::playing::NotificationKind::EventPlaying => {
                        let event = audio
                            .declarations
                            .get(notification.event.declaration as usize)
                            .ok_or("FX sound callback declaration is absent")?;
                        self.sound_systems.event_info(
                            notification.id,
                            event.declaration.max_distance,
                            notification.event.callbacks,
                            event.declaration.persistent_loop(),
                        );
                    }
                }
            }
        }
        self.initialize(graphs, sources, frames)?;
        let clock = frames
            .clock
            .ok_or("world FX awaits actual gameplay clocks")?;
        // Reserve placed actor instances before ordinary graph actions can
        // query whether those actors are already playing (87e8e0/52ce40).
        let pending = std::mem::take(&mut self.pending_actor_starts);
        let attempted = pending.len();
        let mut acquired = 0;
        let mut rejected = 0;
        for id in pending {
            match self.start_actor(id, systems, frames, sources) {
                Ok(Some(_)) => acquired += 1,
                Ok(None) => {}
                Err(error) => {
                    rejected += 1;
                    self.report(error);
                }
            }
        }
        if attempted != 0 && std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
            eprintln!("World FX actor startup: attempted={attempted} acquired={acquired} rejected={rejected}");
        }
        // Action tasks use their FX entity's actual clock, separately from
        // the manager's group2 delta multiplied by entity scale during update.
        let timers = std::mem::take(&mut self.timers);
        for (index, node) in timers {
            let mut runtime = self.graphs[index].take().ok_or("FX graph busy")?;
            let result = runtime.update_timer(
                node,
                &mut GraphHost {
                    state: self,
                    systems,
                    frames,
                    sources,
                    clock,
                    graph: index,
                },
            );
            self.graphs[index] = Some(runtime);
            match result {
                Ok(true) => self.timers.push((index, node)),
                Ok(false) => {}
                Err(error) => {
                    self.report(format!("{} node {node}: {error}", self.graphs.owner(index)))
                }
            }
            self.collect(index);
        }
        while let Some((index, node, port)) = self.graphs.events.pop() {
            let mut runtime = self.graphs[index].take().ok_or("FX graph busy")?;
            runtime.notify(node, port);
            let result = runtime.step(&mut GraphHost {
                state: self,
                systems,
                frames,
                sources,
                clock,
                graph: index,
            });
            self.graphs[index] = Some(runtime);
            match result {
                Ok(GraphStep::Action {
                    schedule: Some(node),
                }) => {
                    if !self.timers.contains(&(index, node)) {
                        self.timers.push((index, node));
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.report(format!("{} node {node}: {error}", self.graphs.owner(index)))
                }
            }
            self.collect(index);
        }
        let order = self.entity_order.clone();
        for id in order {
            let Some(mut entity) = self.entities.remove(&id) else {
                continue;
            };
            // 87e8e0 publishes updateFxPosition into the dynamic-entity pose
            // update flag. Static actor effects retain their spawn pose; they
            // do not run the attached component's per-frame pose/velocity work.
            if entity.suspended
                || (matches!(entity.owner, FxEntityOwner::Actor { .. })
                    && !entity.parameters.update_position)
            {
                self.entities.insert(id, entity);
                continue;
            }
            let mut component = std::mem::take(&mut entity.component);
            let result = (|| {
                // 79e6a0 ->875e80 creates a separate FX entity bound to its
                // location. Its embedded reference actor derives motion through
                // 7506d0, even when the FX's updatePosition flag is disabled.
                // Attachment has already published its initial bound pose. With
                // a stopped entity clock there is no new reference-actor update.
                if let Some(location) = entity
                    .parameters
                    .location
                    .as_ref()
                    .filter(|_| clock.entity_delta > 0.)
                {
                    if let Some(pose) = frames.resolve(location, &entity.parameters.tag)? {
                        let velocities = crate::gameplay::reference_frame::velocities(
                            entity.pose,
                            pose,
                            clock.entity_delta,
                        );
                        entity.pose = pose;
                        entity.motion = crate::gameplay::particles::spawn::SpawnVelocity {
                            linear: velocities[0],
                            center: pose.position,
                            angular: velocities[1],
                            angular_enabled: velocities[1]
                                .iter()
                                .any(|v| v.to_bits() & 0x7fff_ffff != 0),
                        };
                    }
                }
                if matches!(entity.owner, FxEntityOwner::Actor { .. }) {
                    return Ok(());
                }
                component.tick(
                    entity.parameters.stop_on_target_removal,
                    &mut InstanceHost {
                        state: self,
                        systems,
                        frames,
                        sources,
                        entity: &entity,
                        id,
                    },
                )
            })();
            entity.component = component;
            if let Err(error) = result {
                self.report(error);
                entity.suspended = true;
            }
            self.entities.insert(id, entity);
        }
        for (graph, node, handle) in std::mem::take(&mut self.started) {
            if let Some(runtime) = self.graphs[graph].as_mut() {
                if let Err(error) = runtime.effect_started(node, handle) {
                    self.report(error);
                }
            }
        }
        let mut query = match self.query.take() {
            Some(query) => query,
            None => crate::gameplay::world_point_query::WorldPointQuery::new(u32::MAX)?,
        };
        let screen_offset = crate::platform::console::shared()
            .read()
            .cvars
            .float("pm_screenParticleOffset")
            .ok_or("pm_screenParticleOffset is not registered")?;
        // 1a16d0 drains deferred starts forward before ANY active instance
        // update can consume the manager's shared random stream.
        for slot in std::mem::take(&mut self.pending_starts) {
            let Some(mut instance) = self.slots[usize::from(slot)].take() else {
                continue;
            };
            let result = (|| -> Result<(), String> {
                for group in &instance.definition.groups {
                    let admitted = if group.asset.location
                        == crate::assets::fx::EffectGroupLocation::Everywhere
                    {
                        true
                    } else {
                        crate::gameplay::effects::groups::location_allowed(
                            group.asset.location,
                            world
                                .particle_water_body_height(&mut query, instance.pose.position)?
                                .is_some(),
                        )
                    };
                    if !admitted {
                        continue;
                    }
                    let offsets = crate::gameplay::effects::groups::sample_offsets(
                        &group.asset.timings,
                        0.,
                        &mut systems.seed,
                    );
                    for particle in &group.particles {
                        systems.start(
                            &mut instance.head,
                            particle.as_ref().map_err(Clone::clone)?,
                            offsets,
                        )?;
                    }
                    for sound in &group.sounds {
                        let audio = sounds.lock();
                        let reference = |event: &Option<
                            Arc<crate::rendering::particles::world_sources::WorldSoundEvent>,
                        >| {
                            event
                                .as_ref()
                                .map(|event| {
                                    audio
                                        .events
                                        .get(&event.name)
                                        .ok_or_else(|| {
                                            format!("FX sound {} was not prepared", event.name)
                                        })?
                                        .as_ref()
                                        .copied()
                                        .map_err(Clone::clone)
                                })
                                .transpose()
                        };
                        let start = crate::gameplay::effects::sound::SoundEvent {
                            delay: sound.start_delay,
                            event: reference(&sound.start)?,
                        };
                        let stop = crate::gameplay::effects::sound::SoundEvent {
                            delay: sound.stop_delay,
                            event: reference(&sound.stop)?,
                        };
                        self.sound_systems.start(
                            &mut instance.sound_head,
                            sound.position,
                            start,
                            stop,
                            offsets[0],
                            true,
                            &mut systems.seed,
                        )?;
                    }
                }
                Ok(())
            })();
            self.slots[usize::from(slot)] = Some(instance);
            if let Err(error) = result {
                self.report(error);
                self.retire(slot, systems)?;
            }
        }
        for slot in 0..self.slots.len() {
            let Some(mut instance) = self.slots[slot].take() else {
                continue;
            };
            let result = (|| -> Result<bool, String> {
                if instance.parameters.update_position {
                    if let Some(location) = instance.parameters.location.as_ref() {
                        if let Some(pose) = frames.resolve(location, &instance.parameters.tag)? {
                            instance.pose = pose;
                        }
                    }
                    instance.motion = self
                        .entities
                        .get(&instance.entity)
                        .ok_or("world particle FX owning entity was removed")?
                        .motion;
                }
                let delta = crate::float::multiply(clock.delta, clock.scale);
                instance.age = crate::float::add(instance.age, delta);
                let updated = systems.update_with_motion(
                    &mut instance.head,
                    instance.age,
                    &crate::gameplay::effects::particle::ParticleEffectFrame {
                        owner: instance.pose,
                        camera: Some(camera),
                        screen_offset,
                        view_mask: 1,
                        allowed_mask: 1,
                        bounds_dirty: false,
                        delta,
                    },
                    underwater,
                    Some(&instance.motion),
                )?;
                let mut audio = sounds.clone();
                let sound_count = self.sound_systems.update(
                    &mut instance.sound_head,
                    &mut audio,
                    instance.age,
                    bevy::math::Vec3::from_array(instance.pose.position),
                    bevy::math::Mat3::from_cols(
                        bevy::math::Vec3::from_array(instance.pose.basis[0]),
                        bevy::math::Vec3::from_array(instance.pose.basis[1]),
                        bevy::math::Vec3::from_array(instance.pose.basis[2]),
                    ),
                    &super::world_sounds::SoundFrame {
                        axis: bevy::math::Mat3::from_cols(
                            bevy::math::Vec3::from_array(instance.pose.basis[0]),
                            bevy::math::Vec3::from_array(instance.pose.basis[1]),
                            bevy::math::Vec3::from_array(instance.pose.basis[2]),
                        ),
                        scale: clock.scale,
                    },
                )?;
                Ok(updated != 0 || sound_count != 0)
            })();
            self.slots[slot] = Some(instance);
            match result {
                Ok(true) => {}
                Ok(false) => self.retire(slot as u16, systems)?,
                Err(error) => {
                    self.report(error);
                    self.retire(slot as u16, systems)?;
                }
            }
        }
        for (index, node, handle) in std::mem::take(&mut self.completed) {
            let mut runtime = self.graphs[index].take().ok_or("FX graph busy")?;
            let result = runtime.effect_completed(
                node,
                handle,
                &mut GraphHost {
                    state: self,
                    systems,
                    frames,
                    sources,
                    clock,
                    graph: index,
                },
            );
            self.graphs[index] = Some(runtime);
            if let Err(error) = result {
                self.report(error);
            }
            self.collect(index);
        }
        self.entity_order
            .retain(|id| self.entities.contains_key(id));
        for (graph, node, actor) in std::mem::take(&mut self.actor_completed) {
            if let Err(error) = self.graphs[graph]
                .as_mut()
                .ok_or("FX graph busy")?
                .entity_effect_completed(node, actor)
            {
                self.report(error);
            }
            self.collect(graph);
        }
        self.query = Some(query);
        self.frame = Some(frame);
        Ok(())
    }
}

struct GraphHost<'a> {
    state: &'a mut WorldParticleState,
    systems: &'a mut ParticleSystems,
    frames: &'a WorldEffectFrames,
    sources: &'a WorldFxSources,
    clock: WorldFxClock,
    graph: usize,
}
impl FxGraphWorld for GraphHost<'_> {
    fn entity_effect_ready(&self, entity: &EntityReference) -> Result<bool, String> {
        let EntityReference::Named(name) = entity else {
            return Ok(false);
        };
        Ok(self
            .state
            .actors
            .get(name)
            .and_then(|id| self.state.entities.get(id))
            .is_some_and(|entity| {
                matches!(entity.owner, FxEntityOwner::Actor { active: None, .. })
            }))
    }
    fn start_entity_effect(
        &mut self,
        node: u32,
        entity: &EntityReference,
        target: Option<&EntityReference>,
        target_tag: &str,
    ) -> Result<Option<u32>, String> {
        let EntityReference::Named(name) = entity else {
            return Ok(None);
        };
        let Some(id) = self.state.actors.get(name).copied() else {
            return Ok(None);
        };
        if let Some(target) = target {
            if self.frames.resolve(target, "")?.is_some() {
                self.state
                    .entities
                    .get_mut(&id)
                    .ok_or("FX actor was removed")?
                    .parameters
                    .target = Some(target.clone());
            }
        }
        if !target_tag.is_empty() {
            self.state
                .entities
                .get_mut(&id)
                .ok_or("FX actor was removed")?
                .parameters
                .target_tag = target_tag.to_owned();
        }
        let Some(handle) = self
            .state
            .start_actor(id, self.systems, self.frames, self.sources)?
        else {
            return Ok(None);
        };
        if let Some(slot) = self.state.handles.get(&handle).copied() {
            self.state.slots[usize::from(slot)]
                .as_mut()
                .ok_or("FX actor instance absent")?
                .actor_observer = Some((self.graph, node));
        }
        Ok(Some(id))
    }
    fn stop_entity_effect(
        &mut self,
        node: u32,
        entity: &EntityReference,
    ) -> Result<Option<u32>, String> {
        let EntityReference::Named(name) = entity else {
            return Ok(None);
        };
        let Some(id) = self.state.actors.get(name).copied() else {
            return Ok(None);
        };
        let Some(FxEntity {
            owner:
                FxEntityOwner::Actor {
                    active: Some(handle),
                    ..
                },
            ..
        }) = self.state.entities.get(&id)
        else {
            return Ok(None);
        };
        let handle = *handle;
        self.state.stop(handle, self.systems)?;
        if let Some(slot) = self.state.handles.get(&handle).copied() {
            self.state.slots[usize::from(slot)]
                .as_mut()
                .ok_or("FX actor instance absent")?
                .actor_observer = Some((self.graph, node));
        }
        Ok(Some(id))
    }
    fn parent_value(
        &self,
        name: &str,
        kind: &crate::gameplay::effects::variables::GraphValue,
    ) -> Result<Option<crate::gameplay::effects::variables::GraphValue>, String> {
        self.state.graphs.parent_value(self.graph, name, kind)
    }
    fn effect_ready(&self, effect: &str) -> Result<bool, String> {
        self.sources
            .effects
            .get(effect)
            .map(|result| result.as_ref().map(|_| true).map_err(Clone::clone))
            .unwrap_or_else(|| Err(format!("world FX declaration {effect} was not prepared")))
    }
    fn entity_exists(&self, entity: &EntityReference) -> Result<bool, String> {
        Ok(self.frames.resolve(entity, "")?.is_some())
    }
    fn spawn_effect(
        &mut self,
        _: &str,
        node: u32,
        parameters: &FxRuntimeParameters,
        _: &mut FxNode,
        _: f32,
    ) -> Result<Option<CreatedEffect>, String> {
        if !parameters.target_tag.is_empty() || parameters.target.is_some() {
            return Err("world particle FX target/tag pose producer is not connected".into());
        }
        if parameters.dont_spawn_if_culled {
            return Err("world FX spawn culling awaits its actual view query".into());
        }
        let location = parameters.location.as_ref().ok_or("FX location absent")?;
        let Some(pose) = self.frames.resolve(location, &parameters.tag)? else {
            return Ok(None);
        };
        let id = self.state.next_entity;
        self.state.next_entity = id.wrapping_add(1);
        self.state.entities.insert(
            id,
            FxEntity {
                owner: FxEntityOwner::Node {
                    graph: self.graph,
                    node,
                },
                parameters: parameters.clone(),
                component: Default::default(),
                suspended: false,
                pose,
                motion: crate::gameplay::particles::spawn::SpawnVelocity {
                    center: pose.position,
                    ..Default::default()
                },
            },
        );
        self.state.entity_order.push(id);
        Ok(Some(CreatedEffect {
            entity: id,
            clock: self.clock.time,
        }))
    }
    fn stop_effect(&mut self, _: &str, _: u32, handle: u32) -> Result<bool, String> {
        self.state.stop(handle, self.systems)?;
        Ok(false)
    }
    fn entity_clock(&mut self, entity: u32) -> Result<Option<f64>, String> {
        Ok(self
            .state
            .entities
            .contains_key(&entity)
            .then_some(self.clock.time))
    }
}
struct InstanceHost<'a> {
    state: &'a mut WorldParticleState,
    systems: &'a mut ParticleSystems,
    frames: &'a WorldEffectFrames,
    sources: &'a WorldFxSources,
    entity: &'a FxEntity,
    id: u32,
}
impl WorldFxComponentHost for InstanceHost<'_> {
    fn has_effect(&self) -> bool {
        self.entity.parameters.effect.is_some()
    }
    fn reference_readiness(&self) -> Option<ReferenceReadiness> {
        // This host completes the zero-offset attachment by resolving the
        // published location hierarchy before allowing the component to play.
        let attached = self
            .entity
            .parameters
            .location
            .as_ref()
            .is_some_and(|entity| {
                self.frames
                    .resolve(entity, &self.entity.parameters.tag)
                    .is_ok_and(|frame| frame.is_some())
            });
        Some(ReferenceReadiness {
            state: 8,
            pending_binding: !attached,
        })
    }
    fn play(&mut self) -> Result<Option<u32>, String> {
        let parameters = &self.entity.parameters;
        if parameters.target.is_some() || !parameters.target_tag.is_empty() {
            return Err("world FX target/tag pose producer is not connected".into());
        }
        let name = parameters.effect.as_ref().ok_or("world FX effect absent")?;
        let definition = self
            .sources
            .effects
            .get(name)
            .ok_or("world FX definition absent")?
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        // Admit only complete supported groups, never skip another enabled
        // family and consequently consume an incorrect shared RNG sequence.
        for group in &definition.groups {
            for (family, value) in &group.other_families {
                if let crate::assets::entities::Value::Scope(fields) = value {
                    if crate::rendering::particles::world_sources::descriptor_fields(fields)?
                        .iter()
                        .any(|fields| atom(fields, "m_enabled") != Some("false"))
                    {
                        return Err(format!("{name}: world FX family {family} is not connected"));
                    }
                }
            }
            for sound in &group.sounds {
                let audio = self
                    .state
                    .sounds
                    .as_ref()
                    .ok_or("FX sound owner is absent")?
                    .lock();
                for event in [&sound.start, &sound.stop].into_iter().flatten() {
                    audio
                        .events
                        .get(&event.name)
                        .ok_or("FX sound dependency is absent")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                }
            }
            for particle in &group.particles {
                let particle = particle.as_ref().map_err(Clone::clone)?;
                if particle.owner.parameter_block.is_some() {
                    return Err(format!(
                        "{name}: particle parameter block binding is not connected"
                    ));
                }
            }
        }
        let pose = self
            .frames
            .resolve(
                parameters.location.as_ref().ok_or("FX location absent")?,
                &parameters.tag,
            )?
            .ok_or("FX location was removed")?;
        let mut slot = u16::MAX;
        if !self.state.pool.acquire(&mut slot)? {
            return Ok(None);
        }
        let id = self.state.next_instance;
        self.state.next_instance = id.wrapping_add(1);
        self.state.slots[usize::from(slot)] = Some(Instance {
            id,
            entity: self.id,
            definition,
            parameters: parameters.clone(),
            pose,
            motion: self.entity.motion,
            age: 0.,
            head: u16::MAX,
            sound_head: u16::MAX,
            actor_observer: None,
        });
        self.state.handles.insert(id, slot);
        self.state.pending_starts.push(slot);
        if let FxEntityOwner::Node { graph, node } = self.entity.owner {
            self.state.started.push((graph, node, id));
        }
        if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
            bevy::log::info!(effect = name, instance = id, position = ?pose.position, "World FX instance acquired");
        }
        bevy::log::debug!(
            effect = name,
            instance = id,
            "World particle FX instance created"
        );
        Ok(Some(id))
    }
    fn target_present(&self) -> bool {
        self.entity.parameters.target.is_some()
    }
    fn target_alive(&mut self) -> Result<bool, String> {
        Err("world FX target context is not connected".into())
    }
    fn update_target_pose(&mut self) -> Result<(), String> {
        Err("world FX target context is not connected".into())
    }
    fn request_stop(&mut self, handle: u32) -> Result<(), String> {
        self.state.stop(handle, self.systems)?;
        Ok(())
    }
    fn update_source_velocity(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn detach(&mut self, handle: Option<u32>) -> Result<Option<u32>, String> {
        Ok(handle.filter(|handle| self.state.handles.contains_key(handle)))
    }
    fn stop_immediately(&mut self, handle: u32) -> Result<(), String> {
        if let Some(&slot) = self.state.handles.get(&handle) {
            self.state.retire(slot, self.systems)?;
        }
        Ok(())
    }
}
