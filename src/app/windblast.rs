//! Live cpntWindblast rigid-body family. 140448590 spawns before payment;
//! 1404daa60 consumes completed queries, then1404dab70 issues the next batch.
//! The synchronous host query completes into a retained result packet; only
//! the following source UpdateResults phase dispatches that packet.
use super::physics_scene::{
    PhysicsScene,
    windblast::{Target, TargetsQuery},
};
use crate::{
    archive::Catalog,
    assets::{
        damage::Declaration as DamageDeclaration,
        declaration_texts::read_common_or_catalog,
        entities::{Value, player},
    },
    gameplay::{
        component_state::types::ComponentTypes,
        entity_lifetime::{EntityId, LiveEntities},
        game_clock_runtime::ClockRuntime,
        options::decl::{atom, boolean, list_items, scope},
        powers::blast::{self, Definition},
        reference_frame::ReferenceFrame,
        world_ray_query::WorldRayQuery,
    },
};
use bevy::prelude::Resource;
use std::{collections::HashSet, sync::Arc};

/// Canonical FX owner implements this interface. Its acquired definitions,
/// render handles, sound callbacks and emitted tails remain in that owner.
pub(super) trait Effects {
    fn start(
        &mut self,
        live: &LiveEntities,
        entity: EntityId,
        path: &str,
        frame: ReferenceFrame,
        spawner: Option<EntityId>,
    ) -> Result<(), String>;
    fn impact(
        &mut self,
        live: &LiveEntities,
        entity: EntityId,
        target: EntityId,
        path: &str,
        tag: Option<&str>,
        parent_frame: ReferenceFrame,
        frame: ReferenceFrame,
    ) -> Result<(), String>;
    fn stop(&mut self, entity: EntityId) -> Result<(), String>;
    fn retired(&self, entity: EntityId) -> bool;
}
struct WindFx {
    path: String,
    spawner_origin: bool,
}
struct Prepared {
    path: String,
    definition: Definition,
    damage: Option<Arc<DamageDeclaration>>,
    // Keep actual acquired entity descriptors/component projection with all
    // active instances and pending query/FX callbacks, rather than rereading.
    _entity_chain: Vec<Vec<(String, Value)>>,
    _component: Vec<(String, Value)>,
    class: usize,
    fx: Vec<WindFx>,
}
struct SensorState {
    frontier: f32,
    pending: Vec<Target>,
}
struct Dispatch {
    target: Target,
    intensity: f32,
    stage: u8,
}
struct Instance {
    entity: EntityId,
    spawner: EntityId,
    prepared: Arc<Prepared>,
    frame: ReferenceFrame,
    elapsed: f32,
    pitch: f32,
    range: f32,
    falloff: f32,
    sensors: Vec<SensorState>,
    hit: HashSet<EntityId>,
    dispatch: Vec<Dispatch>,
    stopping: bool,
}

/// A source worker's payload. It owns acquired descriptors and dependency
/// leases, with no live cast, physics scratch, or managed entity identity.
pub(super) struct PreparedSources {
    catalog: u64,
    types: Arc<ComponentTypes>,
    prepared: Vec<Arc<Prepared>>,
}
impl PreparedSources {
    /// The acquisition caller must have admitted this actual owned item.
    /// The resolved Power component supplies all four authored variants.
    pub fn acquire_power(
        catalog: &Catalog,
        power: &crate::assets::power_journal::Power,
    ) -> Result<Self, String> {
        if crate::assets::power_journal::power_id(&power.power_id) != Some(10) {
            return Err("Windblast acquisition requires actual power id10".into());
        }
        let settings = crate::gameplay::powers::windblast::Settings::read(&power.component)?;
        let runtime = Windblasts::prepare(
            catalog,
            [(false, false), (true, false), (false, true), (true, true)]
                .into_iter()
                .filter_map(|(greater, shockwave)| settings.entity(greater, shockwave)),
        )?;
        Ok(Self {
            catalog: runtime.catalog,
            types: runtime.types,
            prepared: runtime.prepared,
        })
    }
    /// Filter inventory before acquiring class metadata, entity descriptors,
    /// damage declarations, or FX dependencies. An unowned power yields none.
    pub fn acquire_owned<'a>(
        catalog: &Catalog,
        powers: impl IntoIterator<Item = &'a crate::assets::power_journal::Power>,
        mut owns_item: impl FnMut(&str) -> bool,
    ) -> Result<Option<Self>, String> {
        let Some(power) = powers.into_iter().find(|power| {
            crate::assets::power_journal::power_id(&power.power_id) == Some(10)
                && owns_item(&power.item)
        }) else {
            return Ok(None);
        };
        Self::acquire_power(catalog, power).map(Some)
    }
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        self.prepared.iter().flat_map(|source| {
            source
                .fx
                .iter()
                .map(|fx| fx.path.as_str())
                .chain(source.definition.physics.impact_fx.as_deref())
        })
    }
}
#[derive(Resource)]
pub(super) struct Windblasts {
    catalog: u64,
    types: Arc<ComponentTypes>,
    prepared: Vec<Arc<Prepared>>,
    active: Vec<Instance>,
    capsule: TargetsQuery,
    ray: WorldRayQuery,
    targets: Vec<Target>,
}
impl Windblasts {
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
    pub fn prepare_power(
        catalog: &Catalog,
        power: &crate::assets::power_journal::Power,
    ) -> Result<Self, String> {
        Ok(Self::from_prepared(PreparedSources::acquire_power(
            catalog, power,
        )?))
    }
    pub fn from_prepared(sources: PreparedSources) -> Self {
        Self {
            catalog: sources.catalog,
            types: sources.types,
            prepared: sources.prepared,
            active: Vec::new(),
            capsule: TargetsQuery::default(),
            ray: WorldRayQuery::default(),
            targets: Vec::new(),
        }
    }
    /// Merge a completed worker into the existing runtime. Live casts keep
    /// their same source Arc, managed generation, timers, and query packets.
    pub fn absorb(&mut self, sources: PreparedSources) -> Result<(), String> {
        if self.catalog != sources.catalog {
            return Err("Windblast sources belong to a retired catalog".into());
        }
        for incoming in &sources.prepared {
            let existing = self.types.classes();
            let prepared = sources.types.classes();
            let class_name = prepared
                .metadata(incoming.class)
                .and_then(|class| prepared.name_text(class.name));
            if class_name.is_none()
                || existing
                    .metadata(incoming.class)
                    .and_then(|class| existing.name_text(class.name))
                    != class_name
            {
                return Err("Windblast source class differs from its live metadata owner".into());
            }
        }
        for incoming in sources.prepared {
            if !self
                .prepared
                .iter()
                .any(|source| source.path == incoming.path)
            {
                self.prepared.push(incoming);
            }
        }
        Ok(())
    }
    /// The actual temporary entity remains owned here until its FX tails
    /// acknowledge retirement. Publish these same frames to the shared FX
    /// resolver rather than making render-only substitute entities.
    pub fn frames<'a>(
        &'a self,
        live: &'a LiveEntities,
    ) -> impl Clone + Iterator<Item = (&'a str, ReferenceFrame)> + 'a {
        self.active.iter().filter_map(move |instance| {
            let (name, class) = live.get(instance.entity)?;
            (class == instance.prepared.class).then_some((name, instance.frame))
        })
    }
    /// Actual published ephemeral entity pose for the common named-frame
    /// resolver. Its identity and source remain alive through FX tail release.
    pub fn frame(&self, live: &LiveEntities, entity: EntityId) -> Option<ReferenceFrame> {
        let instance = self
            .active
            .iter()
            .find(|instance| instance.entity == entity)?;
        let (_, class) = live.get(entity)?;
        (class == instance.prepared.class).then_some(instance.frame)
    }
    pub fn prepare<'a>(
        catalog: &Catalog,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, String> {
        let types = ComponentTypes::acquire(catalog)?;
        let mut prepared: Vec<Arc<Prepared>> = Vec::new();
        for path in paths {
            if prepared.iter().any(|source| source.path == path) {
                continue;
            }
            let mut read = |name: &str| read_common_or_catalog(catalog, name);
            let chain = player::entity_chain(path, &mut read)?;
            let class_name = chain
                .iter()
                .rev()
                .find_map(|fields| atom(fields, "class"))
                .ok_or("Windblast entity class is absent")?;
            //874c50 directly constructs idEntity: +d4=0, constructor MAIN.
            // Other authored classes require their actual constructor owner.
            if class_name != "idEntity" {
                return Err(format!(
                    "Windblast entity {path} requires unported {class_name} constructor"
                ));
            }
            let classes = types.classes();
            let class = classes
                .hierarchy()
                .registration_order()
                .iter()
                .copied()
                .find(|&index| {
                    classes
                        .metadata(index)
                        .and_then(|m| classes.name_text(m.name))
                        == Some(class_name.as_bytes())
                })
                .ok_or("installed Windblast entity class is not registered")?;
            let fields = player::entity_component_from_chain(&chain, "cpntwindblast", &mut read)?
                .ok_or_else(|| format!("Windblast entity {path} has no cpntWindblast"))?;
            let definition = Definition::read(&fields)?;
            if boolean(&fields, "m_isPulsing", false) {
                return Err(format!(
                    "Windblast entity {path} requires its pulsing activation owner"
                ));
            }
            if definition.sensors.is_empty() {
                return Err(format!("Windblast entity {path} has no authored sensors"));
            }
            let mut fx = Vec::new();
            if let Some(items) = scope(&fields, "m_windFx") {
                for item in list_items(items) {
                    let Value::Scope(item) = item else {
                        return Err("Windblast FX item is not a scope".into());
                    };
                    if let Some(path) = atom(item, "m_fx")
                        .filter(|p| !p.is_empty() && !p.eq_ignore_ascii_case("NULL"))
                    {
                        fx.push(WindFx {
                            path: path.to_owned(),
                            spawner_origin: boolean(item, "m_useSpawnerOrigin", false),
                        });
                    }
                }
            }
            let damage = definition
                .physics
                .damage_decl
                .as_deref()
                .map(|path| DamageDeclaration::acquire(catalog, path))
                .transpose()?;
            prepared.push(Arc::new(Prepared {
                path: path.to_owned(),
                definition,
                damage,
                _entity_chain: chain,
                _component: fields,
                class,
                fx,
            }));
        }
        Ok(Self {
            catalog: catalog.identity(),
            types,
            prepared,
            active: Vec::new(),
            capsule: TargetsQuery::default(),
            ray: WorldRayQuery::default(),
            targets: Vec::new(),
        })
    }
    /// These paths must be prepared by the same canonical FX owner before the
    /// level publishes this runtime. No source I/O occurs during a cast.
    pub fn effect_paths(&self) -> impl Iterator<Item = &str> {
        self.prepared.iter().flat_map(|source| {
            source
                .fx
                .iter()
                .map(|fx| fx.path.as_str())
                .chain(source.definition.physics.impact_fx.as_deref())
        })
    }
    pub fn damage_paths(&self) -> impl Iterator<Item = &str> {
        self.prepared
            .iter()
            .filter_map(|source| source.definition.physics.damage_decl.as_deref())
    }
    pub fn spawn(
        &mut self,
        path: &str,
        spawner: EntityId,
        frame: ReferenceFrame,
        camera_pitch: f32,
        range: f32,
        falloff: f32,
        live: &mut LiveEntities,
        clock: &mut ClockRuntime,
        effects: &mut impl Effects,
    ) -> Result<bool, String> {
        if live.get(spawner).is_none() {
            return Err("Windblast spawner generation is stale".into());
        }
        let Some(prepared) = self
            .prepared
            .iter()
            .find(|source| source.path == path)
            .cloned()
        else {
            return Ok(false);
        };
        let tag = self.types.intern_name(Some(b"MAIN"))?;
        let entity = live.allocate_managed_auto_name(path, prepared.class, 0, tag)?;
        // The actual component is created below, before publishing inherited
        // entity Initialize/Spawn. No static-header stand-in is registered.
        let instance = Instance {
            entity,
            spawner,
            pitch: prepared.definition.impact_pitch(camera_pitch),
            range,
            falloff,
            sensors: prepared
                .definition
                .sensors
                .iter()
                .map(|_| SensorState {
                    frontier: 0.,
                    pending: Vec::new(),
                })
                .collect(),
            prepared,
            frame,
            elapsed: 0.,
            hit: HashSet::new(),
            dispatch: Vec::new(),
            stopping: false,
        };
        let publication = (|| {
            live.initialized(entity)?;
            clock.register_entity_time(entity, 0)?;
            live.initialize_entity_context(entity, live.entity_context(spawner)?.current)?;
            live.spawned(entity)?;
            live.publish_spawn_tag(entity, "MAIN")?;
            for fx in &instance.prepared.fx {
                effects.start(
                    live,
                    entity,
                    &fx.path,
                    frame,
                    fx.spawner_origin.then_some(spawner),
                )?;
            }
            Ok::<(), String>(())
        })();
        self.active.push(instance);
        if let Err(error) = publication {
            // Preserve the real identity/source if an entered FX callback
            // still needs Stop/tail acknowledgement; clear follows this path.
            self.active.last_mut().unwrap().stopping = true;
            effects.stop(entity)?;
            return Err(error);
        }
        Ok(true)
    }
    pub fn update(
        &mut self,
        scene: &mut PhysicsScene,
        live: &mut LiveEntities,
        clock: &mut ClockRuntime,
        thickness: f32,
        minimum_impulse: f32,
        effects: &mut impl Effects,
        mut damage: impl FnMut(
            &mut PhysicsScene,
            &mut LiveEntities,
            EntityId,
            EntityId,
            EntityId,
            &DamageDeclaration,
            f32,
            [f32; 3],
        ) -> Result<bool, String>,
    ) -> Result<(), String> {
        for instance in &mut self.active {
            if live.get(instance.entity).is_none() {
                return Err("Windblast component identity is stale".into());
            }
            if live.get(instance.spawner).is_none() && !instance.stopping {
                effects.stop(instance.entity)?;
                instance.stopping = true;
            }
            if instance.stopping {
                continue;
            }
            let delta = clock.entity_delta(instance.entity).map_err(str::to_owned)?;
            if delta <= 0. {
                continue;
            }
            let definition = &instance.prepared.definition;
            //4d6f90: take the largest sensor intensity once per real entity.
            self.targets.clear();
            for (sensor, state) in definition.sensors.iter().zip(&mut instance.sensors) {
                for target in state.pending.drain(..) {
                    let Some(target) = scene.windblast_refresh_target(live, target)? else {
                        continue;
                    };
                    let intensity =
                        sensor.intensity(instance.frame, target.center, instance.falloff);
                    if live.get(target.entity).is_none() || instance.hit.contains(&target.entity) {
                        continue;
                    }
                    if let Some(previous) = instance
                        .dispatch
                        .iter_mut()
                        .find(|old| old.target.entity == target.entity)
                    {
                        previous.intensity = previous.intensity.max(intensity);
                    } else {
                        instance.dispatch.push(Dispatch {
                            target,
                            intensity,
                            stage: 0,
                        });
                    }
                }
            }
            for dispatch in &mut instance.dispatch {
                let target = dispatch.target;
                let intensity = dispatch.intensity;
                if live.get(target.entity).is_none() {
                    dispatch.stage = 2;
                    continue;
                }
                let impulse = blast::impulse(
                    instance.frame.position,
                    target.center,
                    instance.pitch,
                    definition.physics.impulse_near,
                    definition.physics.impulse_far,
                    intensity,
                );
                if dispatch.stage == 0 {
                    if target.movable {
                        if impulse.iter().map(|v| v * v).sum::<f32>()
                            > minimum_impulse * minimum_impulse
                        {
                            scene.windblast_impulse(live, target.entity, impulse, target.center)?;
                        }
                    } else if let Some(declaration) = &instance.prepared.damage {
                        //4d8380 only damages the non-movable physics branch;
                        //movable receivers receive the impulse above instead.
                        let amount = definition.physics.damage_near
                            + (definition.physics.damage_far - definition.physics.damage_near)
                                * intensity;
                        let direction = (bevy_math::Vec3::from_array(target.center)
                            - bevy_math::Vec3::from_array(instance.frame.position))
                        .normalize_or_zero()
                        .to_array();
                        damage(
                            scene,
                            live,
                            instance.spawner,
                            instance.entity,
                            target.entity,
                            declaration,
                            amount,
                            direction,
                        )?;
                    }
                    dispatch.stage = 1;
                }
                if dispatch.stage == 1 {
                    if let Some(path) = &definition.physics.impact_fx {
                        let mut frame = scene
                            .entity_frame(live, target.entity)?
                            .ok_or("Windblast target frame disappeared")?;
                        let parent_frame = frame;
                        let forward = bevy_math::Vec3::from_array(frame.position)
                            - bevy_math::Vec3::from_array(instance.frame.position);
                        let forward = forward.normalize_or_zero();
                        let left =
                            bevy_math::Vec3::from_array(instance.frame.basis[2]).cross(forward);
                        let up = forward.cross(left).normalize_or_zero();
                        frame.basis = [
                            forward.to_array(),
                            up.cross(forward).normalize_or_zero().to_array(),
                            up.to_array(),
                        ];
                        effects.impact(
                            live,
                            instance.entity,
                            target.entity,
                            path,
                            definition.physics.impact_tag.as_deref(),
                            parent_frame,
                            frame,
                        )?;
                    }
                    dispatch.stage = 2;
                }
                instance.hit.insert(target.entity);
            }
            instance.dispatch.retain(|dispatch| dispatch.stage != 2);
            for (sensor, state) in definition.sensors.iter().zip(&mut instance.sensors) {
                state.frontier += sensor.speed * delta;
                if sensor.finished(state.frontier) {
                    continue;
                }
                scene.windblast_rigid_targets(
                    live,
                    sensor.capsule(instance.frame, instance.range),
                    definition.collision_layers,
                    &mut self.capsule,
                    &mut self.targets,
                )?;
                for &target in &self.targets {
                    if target.entity == instance.spawner || instance.hit.contains(&target.entity) {
                        continue;
                    }
                    let points = blast::samples(target.bounds);
                    if !points.iter().any(|point| {
                        sensor.contains(instance.frame, *point, state.frontier, delta, thickness)
                    }) {
                        continue;
                    }
                    if scene.windblast_visible(
                        &mut self.ray,
                        instance.frame.position,
                        target,
                        &points,
                        definition.occlusion_layers,
                    )? {
                        state.pending.push(target);
                    }
                }
            }
            instance.elapsed += delta;
            if instance.elapsed > definition.lifetime {
                instance.stopping = true;
                effects.stop(instance.entity)?;
            }
        }
        self.retire_completed(live, clock, effects)
    }
    fn retire_completed(
        &mut self,
        live: &mut LiveEntities,
        clock: &mut ClockRuntime,
        effects: &impl Effects,
    ) -> Result<(), String> {
        let mut index = 0;
        while index < self.active.len() {
            let instance = &self.active[index];
            if !instance.stopping || !effects.retired(instance.entity) {
                index += 1;
                continue;
            }
            // All query results are owned here and callbacks have completed.
            // Only now may the actual entity and descriptor leases retire.
            if live.retire(instance.entity).is_none() {
                return Err("Windblast entity still owns callbacks/attachments".into());
            }
            clock.unregister_entity_time(instance.entity);
            self.active.swap_remove(index);
        }
        Ok(())
    }
    pub fn clear(
        &mut self,
        live: &mut LiveEntities,
        clock: &mut ClockRuntime,
        effects: &mut impl Effects,
    ) -> Result<bool, String> {
        self.begin_retirement(effects)?;
        self.poll_retirement(live, clock, effects)
    }
    /// Enter before the shared physics/FX/audio shutdown. This does not wait
    /// on systems gated off when ScenePublished becomes false.
    pub fn begin_retirement(&mut self, effects: &mut impl Effects) -> Result<(), String> {
        for instance in &mut self.active {
            if !instance.stopping {
                effects.stop(instance.entity)?;
                instance.stopping = true;
            }
            for sensor in &mut instance.sensors {
                sensor.pending.clear();
            }
            instance.dispatch.clear();
        }
        Ok(())
    }
    /// Call after canonical FX shutdown has released actual renderer/sound
    /// leases, before the managed LiveEntities drain; no frame delay is used.
    pub fn poll_retirement(
        &mut self,
        live: &mut LiveEntities,
        clock: &mut ClockRuntime,
        effects: &impl Effects,
    ) -> Result<bool, String> {
        self.retire_completed(live, clock, effects)?;
        Ok(self.active.is_empty())
    }
}
