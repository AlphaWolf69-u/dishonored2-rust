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
use hashbrown::{HashMap, HashSet};
use std::sync::Arc;

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
    decal_head: u16,
    /// Live m_subFXs instances of the started groups, in start order.
    sub_effects: Vec<super::world_sub_effects::SubEffect>,
    actor_observer: Option<(usize, u32)>,
}

/// What a group start (222f80) needs from the FX manager, for top-level
/// instances and for the effects their sub-FX trigger.
pub(super) struct GroupContext<'a> {
    pub(super) systems: &'a mut ParticleSystems,
    pub(super) decals: &'a mut super::decal_effects::DecalSystems,
    pub(super) sound_systems: &'a mut crate::gameplay::effects::sound::systems::SoundSystems<
        crate::gameplay::sound::Emitter,
        crate::gameplay::sound::playing::PlayingId,
    >,
    pub(super) sounds: &'a super::world_sounds::WorldSounds,
    pub(super) effects: &'a HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    pub(super) world: &'a crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
    pub(super) query: &'a mut crate::gameplay::world_point_query::WorldPointQuery,
    pub(super) position: [f32; 3],
}

impl GroupContext<'_> {
    pub(super) fn chains(&mut self) -> super::world_sub_effects::Chains<'_> {
        super::world_sub_effects::Chains {
            systems: self.systems,
            decals: self.decals,
            sound_systems: self.sound_systems,
            sounds: self.sounds,
        }
    }
}

/// Every family of an effect, and of each effect its sub-FX can pick, must be
/// connected and its sound dependencies prepared before playback.
fn admit(
    name: &str,
    definition: &WorldEffectDefinition,
    effects: &HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    sounds: Option<&super::world_sounds::WorldSounds>,
    decal_programs: &super::projected_decals::ProgramTable,
    visited: &mut HashSet<String>,
) -> Result<(), String> {
    if !visited.insert(name.to_owned()) {
        return Ok(());
    }
    for group in &definition.groups {
        for (family, admission) in &group.other_families {
            if *admission.as_ref().map_err(Clone::clone)? {
                return Err(format!("{name}: world FX family {family} is not connected"));
            }
        }
        for decal in &group.decals {
            let decal = decal.as_ref().map_err(Clone::clone)?;
            if !decal.asset.enabled {
                continue;
            }
            for references in decal.declarations.iter().flatten() {
                for (material, world) in [
                    (&references.opaque, false),
                    (&references.alpha, false),
                    (&references.opaque_instanced, true),
                    (&references.alpha_instanced, true),
                ] {
                    if let Some(material) = material {
                        if decal_programs
                            .get(material)
                            .and_then(|variants| variants[usize::from(world)].as_ref())
                            .is_none()
                        {
                            return Err(format!(
                                "{name}: decal material {material} has no prepared receiver program"
                            ));
                        }
                    }
                }
            }
        }
        for sound in &group.sounds {
            let audio = sounds.ok_or("FX sound owner is absent")?.lock();
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
        for sub in &group.asset.sub_effects {
            if !super::world_sub_effects::SubEffect::admitted(sub) {
                continue;
            }
            for child in sub.effects.iter().flatten() {
                let definition = effects
                    .get(child)
                    .ok_or_else(|| format!("{name}: sub-FX {child} was not prepared"))?
                    .as_ref()
                    .map_err(Clone::clone)?;
                admit(child, definition, effects, sounds, decal_programs, visited)?;
            }
        }
    }
    Ok(())
}

/// One instance update's clock and pose, shared by its sub-FX children.
struct UpdateFrame {
    age: f32,
    delta: f32,
    particle: crate::gameplay::effects::particle::ParticleEffectFrame,
    underwater: bool,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
    position: bevy::math::Vec3,
    basis: bevy::math::Mat3,
    sound: super::world_sounds::SoundFrame,
}

/// The particle and sound chains of one FX level; true while any is alive.
fn update_chains(
    head: &mut u16,
    sound_head: &mut u16,
    decal_head: &mut u16,
    frame: &UpdateFrame,
    context: &mut GroupContext,
) -> Result<bool, String> {
    let updated = context.systems.update_with_motion(
        head,
        frame.age,
        &frame.particle,
        frame.underwater,
        Some(&frame.motion),
    )?;
    let mut audio = context.sounds.clone();
    let sound_count = context.sound_systems.update(
        sound_head,
        &mut audio,
        frame.age,
        frame.position,
        frame.basis,
        &frame.sound,
    )?;
    let decal_count = context.decals.update(
        decal_head,
        frame.age,
        frame.particle.owner,
        &mut context.systems.seed,
        |materials| Ok(materials.clone()),
        |_| {},
    )?;
    Ok(updated != 0 || sound_count != 0 || decal_count != 0)
}

/// The sub-FX chain update (220520): each updates its children (223f60), then
/// its schedule (230980); one no longer live is released and unlinked.
fn update_sub_effects(
    sub_effects: &mut Vec<super::world_sub_effects::SubEffect>,
    frame: &UpdateFrame,
    context: &mut GroupContext,
) -> Result<bool, String> {
    let mut index = 0;
    let mut any = false;
    while index < sub_effects.len() {
        let sub = &mut sub_effects[index];
        let chains = update_chains(
            &mut sub.head,
            &mut sub.sound_head,
            &mut sub.decal_head,
            frame,
            context,
        )?;
        let children = update_sub_effects(&mut sub.children, frame, context)?;
        if sub.advance(frame.age, frame.delta, chains || children, context)? {
            any = true;
            index += 1;
        } else {
            sub.clear(&mut context.chains())?;
            sub_effects.remove(index);
        }
    }
    Ok(any)
}

/// 222f80 over an effect's groups: admitted by location, group offsets
/// sampled, then particles, sounds and sub-FX start in family order.
pub(super) fn start_groups(
    definition: &Arc<WorldEffectDefinition>,
    start: f32,
    head: &mut u16,
    sound_head: &mut u16,
    decal_head: &mut u16,
    sub_effects: &mut Vec<super::world_sub_effects::SubEffect>,
    context: &mut GroupContext,
) -> Result<(), String> {
    for (group_index, group) in definition.groups.iter().enumerate() {
        let admitted = if group.asset.location == crate::assets::fx::EffectGroupLocation::Everywhere
        {
            true
        } else {
            crate::gameplay::effects::groups::location_allowed(
                group.asset.location,
                context
                    .world
                    .particle_water_body_height(context.query, context.position)?
                    .is_some(),
            )
        };
        if !admitted {
            continue;
        }
        let offsets = crate::gameplay::effects::groups::sample_offsets(
            &group.asset.timings,
            start,
            &mut context.systems.seed,
        );
        for particle in &group.particles {
            context
                .systems
                .start(head, particle.as_ref().map_err(Clone::clone)?, offsets)?;
        }
        for sound in &group.sounds {
            let audio = context.sounds.lock();
            let reference = |event: &Option<
                Arc<crate::rendering::particles::world_sources::WorldSoundEvent>,
            >| {
                event
                    .as_ref()
                    .map(|event| {
                        audio
                            .events
                            .get(&event.name)
                            .ok_or_else(|| format!("FX sound {} was not prepared", event.name))?
                            .as_ref()
                            .copied()
                            .map_err(Clone::clone)
                    })
                    .transpose()
            };
            let start_event = crate::gameplay::effects::sound::SoundEvent {
                delay: sound.start_delay,
                event: reference(&sound.start)?,
            };
            let stop_event = crate::gameplay::effects::sound::SoundEvent {
                delay: sound.stop_delay,
                event: reference(&sound.stop)?,
            };
            drop(audio);
            context.sound_systems.start(
                sound_head,
                sound.position,
                start_event,
                stop_event,
                offsets[0],
                true,
                &mut context.systems.seed,
            )?;
        }
        for decal in &group.decals {
            context.decals.start(
                decal_head,
                decal.as_ref().map_err(Clone::clone)?.clone(),
                offsets,
                &mut context.systems.seed,
            )?;
        }
        for (index, sub) in group.asset.sub_effects.iter().enumerate() {
            if super::world_sub_effects::SubEffect::admitted(sub) {
                sub_effects.push(super::world_sub_effects::SubEffect::start(
                    definition.clone(),
                    group_index,
                    index,
                    offsets,
                    &mut context.systems.seed,
                ));
            }
        }
    }
    Ok(())
}
pub(crate) struct WorldParticleState {
    source: Option<Arc<Vec<Arc<crate::assets::entities::effects::FxGraph>>>>,
    sounds: Option<super::world_sounds::WorldSounds>,
    sound_systems: crate::gameplay::effects::sound::systems::SoundSystems<
        crate::gameplay::sound::Emitter,
        crate::gameplay::sound::playing::PlayingId,
    >,
    decals: super::decal_effects::DecalSystems,
    decal_programs: Arc<super::projected_decals::ProgramTable>,
    debug_effect: Option<String>,
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
            decals: super::decal_effects::new_systems(),
            decal_programs: Arc::new(Default::default()),
            debug_effect: std::env::var("DH2_DEBUG_PLAY_FX").ok(),
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
    pub(super) fn decal_pool(
        &mut self,
    ) -> &mut crate::rendering::decals::DecalPool<
        crate::rendering::decals::DecalDescription<super::decal_effects::DecalReferences>,
        crate::rendering::decals::CommittedDecal<super::decal_effects::DecalReferences>,
    > {
        self.decals.pool_mut()
    }
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
            if let Some(instance) = self.slots[usize::from(slot)].as_mut() {
                systems.stop(instance.head, instance.age);
                self.decals.stop(instance.decal_head, instance.age);
                self.sound_systems
                    .stop(instance.sound_head, instance.age, &mut systems.seed)?;
                if !instance.sub_effects.is_empty() {
                    let sounds = self.sounds.as_ref().ok_or("FX sound owner is absent")?;
                    let mut chains = super::world_sub_effects::Chains {
                        systems,
                        sound_systems: &mut self.sound_systems,
                        decals: &mut self.decals,
                        sounds,
                    };
                    for sub in &mut instance.sub_effects {
                        sub.stop(instance.age, &mut chains)?;
                    }
                }
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
        self.decals.clear(&mut instance.decal_head, |_| {})?;
        if !instance.sub_effects.is_empty() {
            let sounds = self.sounds.as_ref().ok_or("FX sound owner is absent")?;
            let mut chains = super::world_sub_effects::Chains {
                systems,
                sound_systems: &mut self.sound_systems,
                decals: &mut self.decals,
                sounds,
            };
            for sub in &mut instance.sub_effects {
                sub.clear(&mut chains)?;
            }
        }
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
        decal_scene: &super::projected_decals::DecalScene,
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
        if !Arc::ptr_eq(&self.decal_programs, &decal_scene.programs) {
            self.decal_programs = decal_scene.programs.clone();
        }
        let clock = frames
            .clock
            .ok_or("world FX awaits actual gameplay clocks")?;
        // Diagnostic preview: request an installed effect through the SAME
        // placed-actor start, admission, deferred groups and pool lifecycle.
        // No alternate simulation, shader or material is used by this control.
        if self.debug_effect.is_some()
            && super::debug_graphics_capture::rendering_ready_age()
                .is_some_and(|age| age.as_secs_f64() >= 2.0)
        {
            let effect = self.debug_effect.take().unwrap();
            let pose = frames
                .resolve(&EntityReference::Player, "")?
                .ok_or("FX preview awaits the player pose")?;
            let id = self.next_entity;
            self.next_entity = id.wrapping_add(1);
            self.entity_order.push(id);
            self.entities.insert(
                id,
                FxEntity {
                    owner: FxEntityOwner::Actor {
                        name: "debug.fx-preview".into(),
                        active: None,
                        remove_on_end: false,
                    },
                    parameters: FxRuntimeParameters {
                        effect: Some(effect.clone()),
                        location: Some(EntityReference::Player),
                        update_position: false,
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
            self.pending_actor_starts.push(id);
            bevy::log::info!(effect, position=?pose.position, "Debug FX preview requested through actor playback");
        }
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
            eprintln!(
                "World FX actor startup: attempted={attempted} acquired={acquired} rejected={rejected}"
            );
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
        let order = std::mem::take(&mut self.entity_order);
        for &id in &order {
            // 87e8e0 publishes updateFxPosition into the dynamic-entity pose
            // update flag. Static actor effects retain their spawn pose; they
            // do not run the attached component's per-frame pose/velocity work.
            // Checked in place: moving them out of the map and back is not needed.
            let Some(entity) = self.entities.get(&id) else {
                continue;
            };
            if entity.suspended
                || (matches!(entity.owner, FxEntityOwner::Actor { .. })
                    && !entity.parameters.update_position)
            {
                continue;
            }
            let Some(mut entity) = self.entities.remove(&id) else {
                continue;
            };
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
        // Entities created while ticking were appended to the emptied list.
        let created = std::mem::replace(&mut self.entity_order, order);
        self.entity_order.extend(created);
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
            let definition = instance.definition.clone();
            let result = start_groups(
                &definition,
                0.,
                &mut instance.head,
                &mut instance.sound_head,
                &mut instance.decal_head,
                &mut instance.sub_effects,
                &mut GroupContext {
                    systems,
                    sound_systems: &mut self.sound_systems,
                    decals: &mut self.decals,
                    sounds,
                    effects: &sources.effects,
                    world,
                    query: &mut query,
                    position: instance.pose.position,
                },
            );
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
                let basis = bevy::math::Mat3::from_cols(
                    bevy::math::Vec3::from_array(instance.pose.basis[0]),
                    bevy::math::Vec3::from_array(instance.pose.basis[1]),
                    bevy::math::Vec3::from_array(instance.pose.basis[2]),
                );
                let update = UpdateFrame {
                    age: instance.age,
                    delta,
                    particle: crate::gameplay::effects::particle::ParticleEffectFrame {
                        owner: instance.pose,
                        camera: Some(camera),
                        screen_offset,
                        view_mask: 1,
                        allowed_mask: 1,
                        bounds_dirty: false,
                        delta,
                    },
                    underwater,
                    motion: instance.motion,
                    position: bevy::math::Vec3::from_array(instance.pose.position),
                    basis,
                    sound: super::world_sounds::SoundFrame {
                        axis: basis,
                        scale: clock.scale,
                    },
                };
                let mut context = GroupContext {
                    systems,
                    sound_systems: &mut self.sound_systems,
                    decals: &mut self.decals,
                    sounds,
                    effects: &sources.effects,
                    world,
                    query: &mut query,
                    position: instance.pose.position,
                };
                let alive = update_chains(
                    &mut instance.head,
                    &mut instance.sound_head,
                    &mut instance.decal_head,
                    &update,
                    &mut context,
                )?;
                let sub_effects =
                    update_sub_effects(&mut instance.sub_effects, &update, &mut context)?;
                Ok(alive || sub_effects)
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
        if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
            bevy::log::info!(effect = name, "World FX play requested");
        }
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
        admit(
            name,
            &definition,
            &self.sources.effects,
            self.state.sounds.as_ref(),
            &self.state.decal_programs,
            &mut HashSet::new(),
        )?;
        let pose = self
            .frames
            .resolve(
                parameters.location.as_ref().ok_or("FX location absent")?,
                &parameters.tag,
            )?
            .ok_or("FX location was removed")?;
        let mut slot = u16::MAX;
        if !self.state.pool.acquire(&mut slot)? {
            if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
                bevy::log::info!(effect = name, "World FX instance pool exhausted");
            }
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
            decal_head: u16::MAX,
            sub_effects: Vec::new(),
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
