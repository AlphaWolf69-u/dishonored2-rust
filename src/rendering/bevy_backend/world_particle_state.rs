//! Authored world particle playback. Graph actions create attached FX entities;
//! their components allocate manager instances before deferred group starts.
use super::{
    particle_systems::ParticleSystems,
    world_effects::{WorldEffectFrames, WorldEffectGraphs, WorldFxClock},
};
use crate::{
    assets::entities::{effects::FxNodeParameters, event_graph::atom},
    gameplay::effects::{
        graph::{FxGraphRuntime, FxGraphWorld, GraphStep},
        pool::EffectPoolLinks,
        world_component::{ReferenceReadiness, WorldFxComponent, WorldFxComponentHost},
        world_node::{CreatedEffect, FxNode},
    },
    gameplay::reference_frame::ReferenceFrame,
    rendering::particles::world_sources::{WorldEffectDefinition, WorldFxSources},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

struct FxEntity {
    graph: usize,
    node: u32,
    parameters: FxNodeParameters,
    component: WorldFxComponent,
    suspended: bool,
    pose: ReferenceFrame,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
}
struct Instance {
    id: u32,
    entity: u32,
    definition: Arc<WorldEffectDefinition>,
    parameters: FxNodeParameters,
    pose: ReferenceFrame,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
    age: f32,
    head: u16,
}
pub(crate) struct WorldParticleState {
    source: Option<Arc<Vec<crate::assets::entities::effects::FxGraph>>>,
    graphs: Vec<Option<FxGraphRuntime>>,
    events: Vec<(usize, u32, u32)>,
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
    warnings: HashSet<String>,
    frame: Option<u32>,
    query: Option<crate::gameplay::world_point_query::WorldPointQuery>,
}
impl Default for WorldParticleState {
    fn default() -> Self {
        let pool = EffectPoolLinks::instances();
        Self {
            source: None,
            graphs: Vec::new(),
            events: Vec::new(),
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
            warnings: HashSet::new(),
            frame: None,
            query: None,
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
        if let Some(runtime) = self.graphs[graph].as_mut() {
            self.events.extend(
                runtime
                    .take_pending()
                    .into_iter()
                    .map(|(node, port)| (graph, node, port)),
            );
        }
    }
    fn initialize(&mut self, sources: &WorldEffectGraphs) -> Result<(), String> {
        if self
            .source
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &sources.0))
        {
            return Ok(());
        }
        if self.source.is_some() {
            return Err("world FX level changed before the shared pool was reset".into());
        }
        let mut random = crate::random::MwcRandom::event_manager(sources.1);
        for graph in sources.0.iter() {
            let mut samples = HashMap::new();
            for node in &graph.nodes {
                if node.class == "arkKisculeNodeRandomFloat" {
                    let min = atom(&node.params, "m_min")
                        .unwrap_or("0")
                        .parse::<f32>()
                        .map_err(|_| "invalid graph random minimum")?;
                    let max = atom(&node.params, "m_max")
                        .unwrap_or("1")
                        .parse::<f32>()
                        .map_err(|_| "invalid graph random maximum")?;
                    samples.insert(node.id, random.between(min, max));
                }
            }
            let mut runtime = FxGraphRuntime::new(graph.clone(), samples);
            runtime.loading_end()?;
            self.graphs.push(Some(runtime));
            self.collect(self.graphs.len() - 1);
        }
        self.source = Some(sources.0.clone());
        Ok(())
    }
    fn stop(&mut self, handle: u32, systems: &mut ParticleSystems) {
        if let Some(&slot) = self.handles.get(&handle) {
            if let Some(instance) = self.slots[usize::from(slot)].as_ref() {
                systems.stop(instance.head, instance.age);
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
    }
    fn retire(&mut self, slot: u16, systems: &mut ParticleSystems) -> Result<(), String> {
        let Some(mut instance) = self.slots[usize::from(slot)].take() else {
            return Ok(());
        };
        systems.clear(&mut instance.head)?;
        self.handles.remove(&instance.id);
        if let Some(index) = self
            .pending_starts
            .iter()
            .rposition(|pending| *pending == slot)
        {
            self.pending_starts.swap_remove(index);
        }
        if let Some(entity) = self.entities.remove(&instance.entity) {
            // 4b1b70 forwards Finished, invalidates the component's handle,
            // disposes its velocity adapter, then queues deletion of this
            // NodeFx-created entity (packet+1c was set to one by79e6a0).
            self.completed
                .push((entity.graph, entity.node, instance.id));
        }
        let mut head = slot;
        self.pool.release_head(&mut head)?;
        Ok(())
    }
    pub fn advance(
        &mut self,
        graphs: &WorldEffectGraphs,
        sources: &WorldFxSources,
        frames: &WorldEffectFrames,
        systems: &mut ParticleSystems,
        frame: u32,
        camera: ReferenceFrame,
        underwater: bool,
        world: &crate::gameplay::world_point_query::snapshot::PointQuerySnapshot,
    ) -> Result<(), String> {
        if self.frame == Some(frame) {
            return Ok(());
        }
        self.initialize(graphs)?;
        let clock = frames
            .clock
            .ok_or("world FX awaits actual gameplay clocks")?;
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
                Err(error) => self.report(error),
            }
            self.collect(index);
        }
        while let Some((index, node, port)) = self.events.pop() {
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
                Err(error) => self.report(error),
            }
            self.collect(index);
        }
        let order = self.entity_order.clone();
        for id in order {
            let Some(mut entity) = self.entities.remove(&id) else {
                continue;
            };
            if entity.suspended {
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
                if let Some(location) = entity.parameters.location.as_deref().filter(|_| clock.entity_delta > 0.) {
                    if let Some(pose) = frames.frame(location)? {
                        let velocities = crate::gameplay::reference_frame::velocities(entity.pose, pose, clock.entity_delta);
                        entity.pose = pose;
                        entity.motion = crate::gameplay::particles::spawn::SpawnVelocity {
                            linear: velocities[0], center: pose.position, angular: velocities[1],
                            angular_enabled: velocities[1].iter().any(|v| v.to_bits() & 0x7fff_ffff != 0),
                        };
                    }
                }
                component.tick(
                    entity.parameters.stop_on_target_removal,
                    &mut ComponentHost {
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
                        if let Some(pose) = frames.frame(location)? {
                            instance.pose = pose;
                        }
                    }
                    instance.motion = self.entities.get(&instance.entity)
                        .ok_or("world particle FX owning entity was removed")?.motion;
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
                Ok(updated != 0)
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
    fn effect_ready(&self, effect: &str) -> Result<bool, String> {
        self.sources
            .effects
            .get(effect)
            .map(|result| result.as_ref().map(|_| true).map_err(Clone::clone))
            .unwrap_or_else(|| Err(format!("world FX declaration {effect} was not prepared")))
    }
    fn entity_exists(&self, entity: &str) -> Result<bool, String> {
        Ok(self.frames.frame(entity)?.is_some())
    }
    fn spawn_effect(
        &mut self,
        _: &str,
        node: u32,
        parameters: &FxNodeParameters,
        _: &mut FxNode,
        _: f32,
    ) -> Result<Option<CreatedEffect>, String> {
        if !parameters.tag.is_empty()
            || !parameters.target_tag.is_empty()
            || parameters.target.is_some()
        {
            return Err("world particle FX target/tag pose producer is not connected".into());
        }
        if parameters.dont_spawn_if_culled {
            return Err("world FX spawn culling awaits its actual view query".into());
        }
        let location = parameters.location.as_deref().ok_or("FX location absent")?;
        let Some(pose) = self.frames.frame(location)? else { return Ok(None) };
        let id = self.state.next_entity;
        self.state.next_entity = id.wrapping_add(1);
        self.state.entities.insert(
            id,
            FxEntity {
                graph: self.graph,
                node,
                parameters: parameters.clone(),
                component: Default::default(),
                suspended: false,
                pose,
                motion: crate::gameplay::particles::spawn::SpawnVelocity {
                    center: pose.position, ..Default::default()
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
        self.state.stop(handle, self.systems);
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
struct ComponentHost<'a> {
    state: &'a mut WorldParticleState,
    systems: &'a mut ParticleSystems,
    frames: &'a WorldEffectFrames,
    sources: &'a WorldFxSources,
    entity: &'a FxEntity,
    id: u32,
}
impl WorldFxComponentHost for ComponentHost<'_> {
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
            .as_deref()
            .is_some_and(|name| self.frames.frame(name).is_ok_and(|frame| frame.is_some()));
        Some(ReferenceReadiness {
            state: 8,
            pending_binding: !attached,
        })
    }
    fn play(&mut self) -> Result<Option<u32>, String> {
        let parameters = &self.entity.parameters;
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
            .frame(parameters.location.as_deref().ok_or("FX location absent")?)?
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
        });
        self.state.handles.insert(id, slot);
        self.state.pending_starts.push(slot);
        self.state
            .started
            .push((self.entity.graph, self.entity.node, id));
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
        self.state.stop(handle, self.systems);
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
