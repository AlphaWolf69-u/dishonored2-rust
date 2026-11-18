//! Authored world particle playback. Graph actions create attached FX entities;
//! their components allocate manager instances before deferred group starts.
use super::{
    particle_systems::ParticleSystems,
    world_effects::{WorldEffectDefinitions, WorldEffectFrames, WorldFxClock, FrameReferences, ManagedFrames},
};
use crate::{
    assets::entities::event_graph::atom,
    gameplay::effects::{
        graph::{FxGraphWorld, FxRuntimeParameters, GraphStep},
        instances::{FxGraphInstances, GraphEvent},
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

mod lifecycle_observation;
pub(crate) use lifecycle_observation::OwnerSnapshot;

struct FxEntity {
    explicit_target: Option<crate::gameplay::particles::beam::BeamTarget>,
    owner: FxEntityOwner,
    parameters: FxRuntimeParameters,
    component: WorldFxComponent,
    suspended: bool,
    pose: ReferenceFrame,
    local: Option<ReferenceFrame>,
    dynamic_sources: Option<Arc<WorldFxSources>>,
    dynamic_retirement: Option<Arc<super::world_effects::DynamicEffectLifetime>>,
    entity_context: Option<super::world_effects::EntityEffectContext>,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
}
enum FxEntityOwner {
    Node {
        graph: usize,
        node: u32,
    },
    /// The FX instance still owns its tails after its completion receiver
    /// has been removed; retirement must not notify that receiver (199df0).
    Detached,
    Actor {
        name: String,
        active: Option<u32>,
        remove_on_end: bool,
    },
}
struct Instance {
    gameplay_events: Vec<super::world_gameplay_effects::Event>,
    id: u32,
    entity: u32,
    definition: Arc<WorldEffectDefinition>,
    parameters: FxRuntimeParameters,
    pose: ReferenceFrame,
    local: Option<ReferenceFrame>,
    dynamic_sources: Option<Arc<WorldFxSources>>,
    dynamic_retirement: Option<Arc<super::world_effects::DynamicEffectLifetime>>,
    entity_context: Option<super::world_effects::EntityEffectContext>,
    motion: crate::gameplay::particles::spawn::SpawnVelocity,
    age: f32,
    head: u16,
    light_head: u16,
    sound_head: u16,
    decal_head: u16,
    distortion_head: u16,
    fov_head: u16,
    quad_head: u16,
    shake_head: u16,
    fade_head: u16,
    rumble_head: u16,
    environment_head: u16,
    /// Live m_subFXs instances of the started groups, in start order.
    sub_effects: Vec<super::world_sub_effects::SubEffect>,
    actor_observer: Option<(usize, u32)>,
    /// The record target (+0x80/+0x8c) arkCpntFX writes into every record
    /// of the instance (1404b20b0); 14021dc10's origin and identity until a
    /// target entity is resolved.
    target: crate::gameplay::particles::beam::BeamTarget,
}

/// What a group start (222f80) needs from the FX manager, for top-level
/// instances and for the effects their sub-FX trigger.
pub(super) struct GroupContext<'a> {
    pub(super) view_mask:u8,
    pub(super) gameplay:
        &'a mut crate::gameplay::effects::gameplay::Requests,
    pub(super) frame: ReferenceFrame,
    pub(super) entity_context: Option<super::world_effects::EntityEffectContext>,
    pub(super) systems: &'a mut ParticleSystems,
    pub(super) lights: &'a mut super::world_light_effects::LightSystems,
    pub(super) decals: &'a mut super::decal_effects::DecalSystems,
    pub(super) distortion: &'a mut super::world_screen_distortion::Systems,
    pub(super) fovs: &'a mut super::world_fov::Systems,
    pub(super) quads: &'a mut super::world_screen_quads::Systems,
    pub(super) shakes: &'a mut super::world_camera_shakes::Systems,
    pub(super) fades: &'a mut super::world_screen_fades::Systems,
    pub(super) rumble: &'a mut super::world_rumble::Systems,
    pub(super) environments: &'a mut crate::rendering::environment_effect_runtime::EnvironmentEffectRuntime<Arc<crate::rendering::environment_resources::PreparedEnvironment>, u64>,
    pub(super) sound_systems: &'a mut crate::gameplay::effects::sound::systems::SoundSystems<
        crate::gameplay::sound::Emitter,
        crate::gameplay::sound::playing::PlayingId,
    >,
    pub(super) sounds: &'a super::world_sounds::WorldSounds,
    pub(super) effects: &'a HashMap<String, Result<Arc<WorldEffectDefinition>, String>>,
    pub(super) world: &'a crate::gameplay::world_point_query::level::LevelPointQueries,
    pub(super) query: &'a mut crate::gameplay::world_point_query::WorldPointQuery,
    pub(super) position: [f32; 3],
}

impl GroupContext<'_> {
    pub(super) fn chains(&mut self) -> super::world_sub_effects::Chains<'_> {
        super::world_sub_effects::Chains {
            systems: self.systems,
            decals: self.decals,
            distortion: self.distortion,
            rumble: self.rumble,
            environments: self.environments,
            fovs: self.fovs,
            quads: self.quads,
            shakes: self.shakes,
            fades: self.fades,
            lights: self.lights,
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
        for distortion in &group.screen_distortions { distortion.as_ref().map_err(Clone::clone)?; }
        for rumble in &group.rumbles { rumble.as_ref().map_err(Clone::clone)?; }
        for fov in &group.fovs { fov.as_ref().map_err(Clone::clone)?; }
        for quad in &group.quads { quad.as_ref().map_err(Clone::clone)?; }
        for light in &group.lights {
            let light=light.as_ref().map_err(Clone::clone)?;
            if light.enabled && !matches!(light.kind,0|2) { return Err(format!("{name}: FX light type{} awaits a verified canonical render consumer",light.kind)); }
        }
        for (family, admission) in definition.other_families(group) {
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
    light_head: &mut u16,
    sound_head: &mut u16,
    decal_head: &mut u16,
    distortion_head: &mut u16,
    fov_head: &mut u16,
    quad_head: &mut u16,
    shake_head: &mut u16,
    fade_head: &mut u16,
    rumble_head: &mut u16,
    environment_head: &mut u16,
    gameplay_events: &mut Vec<super::world_gameplay_effects::Event>,
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
    let light_count = context.lights.update(light_head, frame.age, frame.particle.owner, &mut context.systems.seed)?;
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
    let mut view_live = None;
    let mut gameplay_live = false;
    let mut index = 0;
    while index < gameplay_events.len() {
        if view_live.is_none() && !gameplay_events[index].is_entity() {
            view_live = Some(update_view_chains(fov_head,distortion_head,quad_head,environment_head,frame,context)?);
        }
        if gameplay_events[index].advance(
            frame.age,
            &mut context.systems.seed,
            context.frame,
            context.entity_context,
            context.gameplay,
        )? {
            gameplay_live = true;
            index += 1;
        } else {
            gameplay_events.remove(index);
        }
    }
    let view_live = match view_live {
        Some(live) => live,
        None => update_view_chains(fov_head,distortion_head,quad_head,environment_head,frame,context)?,
    };
    let shake_count = context.shakes.update(shake_head, frame.age,
        frame.particle.view_mask & frame.particle.allowed_mask != 0, &mut context.systems.seed,
        frame.particle.camera, frame.particle.owner.position)?;
    let fade_count = context.fades.update(fade_head, frame.age, &mut context.systems.seed)?;
    let rumble_count = context.rumble.update(rumble_head, frame.age, frame.delta,
        frame.particle.owner.position, context.entity_context.map_or(0., |context| context.contact_speed),
        &mut context.systems.seed)?;
    Ok(rumble_count != 0 || updated != 0 || light_count != 0 || sound_count != 0 || decal_count != 0 || view_live || shake_count != 0 || fade_count != 0 || gameplay_live)
}

///222f80/223f60 preserve FOV, distortion and quad family RNG order.
fn update_view_chains(fov:&mut u16,distortion:&mut u16,quads:&mut u16,environments:&mut u16,
    frame:&UpdateFrame,context:&mut GroupContext)->Result<bool,String>{
    let fov=context.fovs.update(fov,frame.age,&mut context.systems.seed)?;
    let distortion=context.distortion.update(distortion,frame.age,
        frame.particle.view_mask&frame.particle.allowed_mask!=0,&mut context.systems.seed)?;
    let quads=context.quads.update(quads,frame.age,&mut context.systems.seed)?;
    let environments=context.environments.update_group(environments,frame.age,frame.particle.view_mask,&mut context.systems.seed)?;
    Ok(fov!=0||distortion!=0||quads!=0||environments!=0)
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
            &mut sub.light_head,
            &mut sub.sound_head,
            &mut sub.decal_head,
            &mut sub.distortion_head,
            &mut sub.fov_head,
            &mut sub.quad_head,
            &mut sub.shake_head,
            &mut sub.fade_head,
            &mut sub.rumble_head,
            &mut sub.environment_head,
            &mut sub.gameplay_events,
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
    light_head: &mut u16,
    sound_head: &mut u16,
    decal_head: &mut u16,
    distortion_head: &mut u16,
    fov_head: &mut u16,
    quad_head: &mut u16,
    shake_head: &mut u16,
    fade_head: &mut u16,
    rumble_head: &mut u16,
    environment_head: &mut u16,
    gameplay_events: &mut Vec<super::world_gameplay_effects::Event>,
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
        for light in &group.lights {
            context.lights.start(light_head, light.as_ref().map_err(Clone::clone)?.clone(), offsets, context.frame, &mut context.systems.seed)?;
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
        for source in &group.entity_defs {
            //221ac0 pushes each family onto its own chain head. Keep all
            //entity events before explosions, reverse-start order per family.
            gameplay_events.insert(
                0,
                super::world_gameplay_effects::Event::entity(
                    source.clone(),
                    offsets[0],
                    &mut context.systems.seed,
                ),
            );
        }
        for fov in &group.fovs {
            context.fovs.start(fov_head,fov.as_ref().map_err(Clone::clone)?.clone(),offsets,&mut context.systems.seed)?;
        }
        for distortion in &group.screen_distortions {
            context.distortion.start(distortion_head, distortion.as_ref().map_err(Clone::clone)?.clone(),
                offsets, &mut context.systems.seed)?;
        }
        for quad in &group.quads {
            context.quads.start(quad_head,quad.as_ref().map_err(Clone::clone)?.clone(),offsets,3,&mut context.systems.seed)?;
        }
        // 22d700: each camera shake; 22a570: each screen fade.
        for shake in &group.asset.camera_shakes {
            context.shakes.start(shake_head, shake, offsets, &mut context.systems.seed)?;
        }
        for fade in &group.asset.screen_fades {
            context.fades.start(fade_head, fade, offsets, &mut context.systems.seed)?;
        }
        //222f80 bit9: environment overrides follow quads and precede explosions.
        for source in &group.environments {
            let descriptor = &group.asset.environments[source.index];
            context.environments.start(environment_head, source.environment.clone(), descriptor.blend.clone(),
                offsets, context.view_mask, &mut context.systems.seed)?;
        }
        for source in &group.explosions {
            let first_explosion = gameplay_events
                .iter()
                .position(|event| !event.is_entity())
                .unwrap_or(gameplay_events.len());
            gameplay_events.insert(
                first_explosion,
                super::world_gameplay_effects::Event::explosion(
                    source.clone(),
                    offsets[0],
                    &mut context.systems.seed,
                ),
            );
        }
        //222f80 family bit13 starts after explosions and before sub-FX.
        for rumble in &group.rumbles {
            context.rumble.start(rumble_head, rumble.as_ref().map_err(Clone::clone)?.clone(), offsets, &mut context.systems.seed)?;
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
    gameplay: crate::gameplay::effects::gameplay::Requests,
    source: Option<Arc<Vec<Arc<crate::assets::entities::effects::FxGraph>>>>,
    sounds: Option<super::world_sounds::WorldSounds>,
    sound_systems: crate::gameplay::effects::sound::systems::SoundSystems<
        crate::gameplay::sound::Emitter,
        crate::gameplay::sound::playing::PlayingId,
    >,
    lights: super::world_light_effects::LightSystems,
    decals: super::decal_effects::DecalSystems,
    distortion: super::world_screen_distortion::Systems,
    rumble: super::world_rumble::Systems,
    environments: crate::rendering::environment_effect_runtime::EnvironmentEffectRuntime<Arc<crate::rendering::environment_resources::PreparedEnvironment>, u64>,
    fovs: super::world_fov::Systems,
    quads: super::world_screen_quads::Systems,
    shakes: super::world_camera_shakes::Systems,
    fades: super::world_screen_fades::Systems,
    decal_programs: Arc<super::projected_decals::ProgramTable>,
    debug_effect: Option<String>,
    graphs: FxGraphInstances,
    entity_graphs: HashMap<crate::gameplay::entity_lifetime::EntityId, u64>,
    started_entity_graphs: HashSet<crate::gameplay::entity_lifetime::EntityId>,
    breakable_subscribers: HashMap<crate::gameplay::entity_lifetime::EntityId, Vec<(u64, u32)>>,
    pickup_subscribers: HashMap<crate::gameplay::entity_lifetime::EntityId, Vec<(u64, u32)>>,
    actors_initialized: bool,
    closed: bool,
    actors: HashMap<String, u32>,
    dynamic: HashMap<String, (u32, u64)>,
    dynamic_snapshot: Option<Arc<Vec<super::world_effects::DynamicEffect>>>,
    dynamic_retiring: HashSet<u32>,
    dynamic_retired: u64,
    pending_actor_starts: Vec<u32>,
    timers: Vec<(usize, u32)>,
    entities: HashMap<u32, FxEntity>,
    entity_order: Vec<u32>,
    next_entity: u32,
    next_instance: u32,
    ///Actual60af50 class journal, sharing this manager's real instances,
    ///canonical header generations and normal stop/tail callbacks.
    contact_instances: crate::gameplay::effects::contact_instances::ContactInstances<
        crate::assets::fx::definition::Definition,
    >,
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
            gameplay: Default::default(),
            source: None,
            sounds: None,
            sound_systems: Default::default(),
            lights: Default::default(),
            decals: super::decal_effects::new_systems(),
            distortion: Default::default(),
            rumble: Default::default(),
            environments: Default::default(),
            fovs: Default::default(),
            quads: Default::default(),
            shakes: Default::default(),
            fades: Default::default(),
            decal_programs: Arc::new(Default::default()),
            debug_effect: std::env::var("DH2_DEBUG_PLAY_FX").ok(),
            graphs: FxGraphInstances::default(),
            entity_graphs: HashMap::new(),
            started_entity_graphs: HashSet::new(),
            pickup_subscribers: HashMap::new(),
            breakable_subscribers: HashMap::new(),
            actors_initialized: false,
            closed: false,
            actors: HashMap::new(),
            dynamic: HashMap::new(),
            dynamic_snapshot: None,
            dynamic_retiring: HashSet::new(),
            dynamic_retired: 0,
            pending_actor_starts: Vec::new(),
            timers: Vec::new(),
            entities: HashMap::new(),
            entity_order: Vec::new(),
            next_entity: 0,
            next_instance: 0,
            contact_instances: Default::default(),
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
    pub(crate) fn environment_pool(&self)->crate::rendering::environment_effect_runtime::SharedEnvironmentPool{self.environments.shared_pool()}
    pub(crate) fn visit_environment_orders(&self,mask:u8,visit:impl FnMut(u64))->Result<(),String>{self.environments.visit_orders(mask,visit)}
    pub(crate) fn flush_environment_retirements(&self,view:u64,slots:&mut crate::rendering::environment_overrides::EnvironmentOverrides<Arc<crate::rendering::environment_resources::PreparedEnvironment>>)->Result<(),String>{
        self.environments.flush_retirements(&[view],|_,slot|slots.retire(slot))
    }
    pub(crate) fn publish_environment(&mut self,order:u64,view:u64,mask:u8,slots:&mut crate::rendering::environment_overrides::EnvironmentOverrides<Arc<crate::rendering::environment_resources::PreparedEnvironment>>)->Result<(),String>{
        self.environments.publish_order(order,view,mask,slots)
    }
    pub(crate) fn publish_environments(&mut self, view:u64, mask:u8,
        slots:&mut crate::rendering::environment_overrides::EnvironmentOverrides<Arc<crate::rendering::environment_resources::PreparedEnvironment>>)->Result<(),String>{
        self.environments.flush_retirements(&[view], |_, slot| slots.retire(slot))?;
        self.environments.publish(view,mask,slots)
    }
    pub(crate) fn apply_fov(&self,base:f32)->f32 {
        //21da80 does not call the view FOV reducer for an empty registered
        //list. Preserve the user's base value without applying its clamp.
        let mut samples=self.fovs.samples();
        let Some(first)=samples.next() else{return base;};
        let value=samples.fold(first.apply(base,base),|current,sample|sample.apply(base,current));
        if value<1.{1.}else if value>150.{150.}else{value}
    }
    pub(crate) fn shake_frame(&self) -> crate::gameplay::effects::camera_shake::ShakeAccumulator {
        self.shakes.frame()
    }
    pub(crate) fn screen_fade_frame(&self) -> Option<[f32; 4]> {
        self.fades.frame()
    }
    pub(crate) fn screen_quad_frame(&self)->crate::gameplay::effects::screen_quad::Frame{self.quads.frame(1)}
    pub(crate) fn rumble_frame(&self) -> Option<(u32,[f32;4])> { self.rumble.completed() }
    pub(crate) fn screen_distortion_frame(&self) -> crate::gameplay::effects::screen_distortion::Frame {
        self.distortion.frame()
    }
    pub(crate) fn light_publication(&self) -> impl Iterator<Item=(u16,Option<&crate::gameplay::effects::light::LightSample>)> { self.lights.publication() }

    pub(crate) fn story_boolean(&self, worldspawn: crate::gameplay::entity_lifetime::EntityId, qualified: &str) -> Result<Option<bool>, String> {
        if self.source.is_none() || self.closed { return Err("story Kiscule manager is not initialized".into()); }
        let Some(&root) = self.entity_graphs.get(&worldspawn) else { return Ok(None) };
        self.graphs.story_boolean(root, qualified)
    }
    pub(crate) fn bind_story_root(&mut self,worldspawn:crate::gameplay::entity_lifetime::EntityId)->Result<(),String>{
        //5134e0 may genuinely find no program on a world's actual header.
        //Keep that null lookup distinct from inventing a graph/default flag.
        if let Some(&root)=self.entity_graphs.get(&worldspawn){self.graphs.bind_story_root(root)?;}
        Ok(())
    }
    fn prepare_named_graph(&mut self,index:usize,live:Option<&crate::gameplay::entity_lifetime::LiveEntities>)->Result<(),String>{
        let actors=&self.entity_graphs;
        self.graphs.prepare_named_bindings(index,&mut |name|{
            let live=live?;
            let entity=live.find(name)?;
            actors.get(&entity).copied()
        })
    }
    pub(crate) fn pending_gameplay(&self) -> Option<&crate::gameplay::effects::gameplay::Request> {
        self.gameplay.front()
    }
    pub(crate) fn acknowledge_gameplay(&mut self) -> Result<(), String> {
        self.gameplay
            .pop_front()
            .ok_or_else(|| "FX gameplay event acknowledgement has no pending event".to_owned())
            .map(|_| ())
    }
    pub(crate) fn clear_gameplay(&mut self) {
        self.gameplay.clear();
    }
    pub(crate) fn set_gameflow(
        &mut self,
        mailbox: Arc<crate::gameplay::gameflow::program::Mailbox>,
    ) {
        self.graphs.set_gameflow(mailbox);
    }
    pub(crate) fn poll_script_spawns(&mut self){for error in self.graphs.poll_spawns(){self.report(error);}}
    pub(crate) fn set_spawner(&mut self, spawner: Arc<crate::gameplay::effects::spawner::EntitySpawner>) {
        self.graphs.set_spawner(spawner);
    }

    /// One shared graph manager is constructed before source startup callbacks.
    /// Real admitted entities own gameplay roots; other existing map FX roots
    /// keep their explicitly host-scoped relevance filter in this same manager.
    pub(crate) fn prepare_graphs(
        &mut self,
        definitions: &WorldEffectDefinitions,
        declarations: &crate::assets::entities::effects::declarations::FxDeclarations,
        live: Option<&crate::gameplay::entity_lifetime::LiveEntities>,
    ) -> Result<(), String> {
        if self.closed || self.source.is_some() {
            return Err("world graph manager is already prepared or closing".into());
        }
        self.graphs = FxGraphInstances::new_level(&[], declarations, definitions.seed);
        if let Some(live) = live {
            let by_name: HashMap<_, _> = definitions
                .graphs
                .iter()
                .map(|graph| (graph.owner.to_ascii_lowercase(), graph))
                .collect();
            for &entity in live.spawned_entities() {
                let (name, _) = live.get(entity).ok_or("spawned graph entity is stale")?;
                if let Some(graph) = by_name.get(&name.to_ascii_lowercase()) {
                    self.admit_entity_graph(entity, Arc::clone(graph), declarations, live)?;
                }
            }
        }
        for graph in definitions.graphs.iter() {
            if self.graphs.has_root_owner(&graph.owner) {
                continue;
            }
            let root = self
                .graphs
                .admit_render_root(Arc::clone(graph), declarations)?;
            self.graphs.queue_loading_end(root)?;
        }
        //DeclInstance entry refresh may read another actor's variable while
        //draining LoadingEnd itself, before an ordinary node step is visited.
        let actors=&self.entity_graphs;
        self.graphs.prepare_named_all(|name|{
            let live=live?;
            let entity=live.find(name)?;
            actors.get(&entity).copied()
        })?;
        for error in self.graphs.startup() {
            self.report(error);
        }
        self.source = Some(Arc::clone(&definitions.graphs));
        self.report_graph_lifetime("prepared");
        Ok(())
    }
    fn report_graph_lifetime(&self, phase: &str) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") {
            return;
        }
        let (instances, definitions) = self.graphs.definition_counts();
        let roots = self.graphs.root_owners().count();
        let subscriptions: usize = self
            .pickup_subscribers
            .values()
            .chain(self.breakable_subscribers.values())
            .map(Vec::len)
            .sum();
        eprintln!(
            "ENTITY_GRAPH_LIFETIME phase={phase} managed_roots={} host_roots={} instances={instances} definitions={definitions} subscriptions={subscriptions} pending={} closed={}",
            self.entity_graphs.len(),
            roots.saturating_sub(self.entity_graphs.len()),
            self.graphs.events.pending().count(),
            self.closed
        );
    }
    pub(crate) fn admit_entity_graph(
        &mut self,
        entity: crate::gameplay::entity_lifetime::EntityId,
        graph: Arc<crate::assets::entities::effects::FxGraph>,
        declarations: &crate::assets::entities::effects::declarations::FxDeclarations,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<u64, String> {
        use crate::gameplay::entity_lifetime::Stage;
        if self.closed {
            return Err("graph manager is closing".into());
        }
        let (name, _) = live.get(entity).ok_or("graph owner has retired")?;
        if live.stage(entity) != Some(Stage::Spawned) || !name.eq_ignore_ascii_case(&graph.owner) {
            return Err("graph does not belong to the admitted spawned entity".into());
        }
        if self.entity_graphs.contains_key(&entity) || self.graphs.has_root_owner(name) {
            return Err("entity already owns a graph in the shared manager".into());
        }
        let root = self.graphs.admit_root(graph, declarations)?;
        self.entity_graphs.insert(entity, root);
        if let Err(error) = self.start_entity_graph(entity, live) {
            self.retire_entity_graph(entity)?;
            return Err(error);
        }
        Ok(root)
    }
    fn start_entity_graph(
        &mut self,
        entity: crate::gameplay::entity_lifetime::EntityId,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        let root = *self
            .entity_graphs
            .get(&entity)
            .ok_or("entity graph is absent")?;
        if self.started_entity_graphs.contains(&entity) {
            return Err("entity graph already started".into());
        }
        let indices = self
            .graphs
            .retirement_subtree(root)
            .ok_or("graph declaration subtree is unavailable")?;
        // 7becf0 starts each instance's actual receivers before LoadingEnd.
        for index in indices {
            let graph = self
                .graphs
                .instance_id(index)
                .ok_or("graph generation is unavailable")?;
            let (subscriptions, errors) = self.graphs.pickup_subscriptions(graph)?;
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
            for (node, target) in subscriptions {
                self.register_pickup_receiver(graph, node, target, live)?;
            }
            let (subscriptions, errors) = self.graphs.breakable_subscriptions(graph)?;
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
            for (node, target) in subscriptions {
                self.register_breakable_receiver(graph, node, target, live)?;
            }
        }
        self.graphs.queue_loading_end(root)?;
        self.started_entity_graphs.insert(entity);
        Ok(())
    }
    fn register_breakable_receiver(
        &mut self,
        graph: u64,
        node: u32,
        target: EntityReference,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        let target = match target {
            EntityReference::Named(name) => live.find(&name),
            EntityReference::Managed(id) => live.get(id).map(|_|id),
            EntityReference::Invalid => None,
            EntityReference::Player => {
                return Err(
                    "breakable notification requires an admitted player entity binding".into(),
                );
            }
        };
        if let Some(target) = target
            .filter(|id| live.stage(*id) == Some(crate::gameplay::entity_lifetime::Stage::Spawned))
        {
            let listeners = self.breakable_subscribers.entry(target).or_default();
            if !listeners.contains(&(graph, node)) {
                listeners.push((graph, node));
            }
        }
        Ok(())
    }
    fn reset_breakable_receiver(
        &mut self,
        graph: u64,
        node: u32,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        self.breakable_subscribers.retain(|_, listeners| {
            listeners.retain(|entry| *entry != (graph, node));
            !listeners.is_empty()
        });
        let index = self
            .graphs
            .instance_index(graph)
            .ok_or("breakable reset graph retired")?;
        self.graphs[index]
            .as_mut()
            .ok_or("breakable reset graph busy")?
            .reset_notification_count(node);
        if let Some(target) = self.graphs.breakable_subscription(graph, node)? {
            self.register_breakable_receiver(graph, node, target, live)?;
        }
        Ok(())
    }
    /// 8848a0/7986b0 use actual subscriptions, in reverse registration order.
    pub(crate) fn dispatch_broken(
        &mut self,
        target: crate::gameplay::entity_lifetime::EntityId,
        step: i32,
        removed: bool,
        location: [f32; 3],
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<Vec<String>, String> {
        if self.closed || self.source.is_none() {
            return Err("breakable graph manager is not active".into());
        }
        let name = live
            .get(target)
            .ok_or("broken entity identity is stale")?
            .0
            .to_owned();
        let listeners = self
            .breakable_subscribers
            .get(&target)
            .cloned()
            .unwrap_or_default();
        let mut errors = Vec::new();
        for (graph, node) in listeners.into_iter().rev() {
            errors.extend(self.graphs.broken(
                graph,
                node,
                EntityReference::Named(name.clone()),
                step,
                removed,
                location,
            )?);
        }
        Ok(errors)
    }
    fn register_pickup_receiver(
        &mut self,
        graph: u64,
        node: u32,
        target: EntityReference,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        use crate::gameplay::entity_lifetime::Stage;
        let target = match target {
            EntityReference::Named(name) => live.find(&name),
            EntityReference::Managed(id) => live.get(id).map(|_|id),
            EntityReference::Invalid => None,
            // This host does not admit a managed player slot yet. Never bind
            // a symbolic player to worldspawn or an arbitrary map entity.
            EntityReference::Player => {
                self.report(format!("pickup node {node} has no admitted player receiver; no placeholder subscription was created"));
                None
            }
        };
        if let Some(target) = target.filter(|id| live.stage(*id) == Some(Stage::Spawned)) {
            let listeners = self.pickup_subscribers.entry(target).or_default();
            if !listeners.contains(&(graph, node)) {
                listeners.push((graph, node));
            }
        }
        Ok(())
    }
    fn reset_pickup_receiver(
        &mut self,
        graph: u64,
        node: u32,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<(), String> {
        self.pickup_subscribers.retain(|_, listeners| {
            listeners.retain(|entry| *entry != (graph, node));
            !listeners.is_empty()
        });
        let index = self
            .graphs
            .instance_index(graph)
            .ok_or("pickup reset graph retired")?;
        self.graphs[index]
            .as_mut()
            .ok_or("pickup reset graph busy")?
            .reset_notification_count(node);
        if let Some(target) = self.graphs.pickup_subscription(graph, node)? {
            self.register_pickup_receiver(graph, node, target, live)?;
        }
        Ok(())
    }
    /// 883d10 snapshots only this entity's receiver list and delivers it in
    /// reverse registration order. Work joins the existing manager queue.
    pub(crate) fn dispatch_picked_up_receipt(
        &mut self,
        target:crate::gameplay::entity_lifetime::EntityId,
        successful:bool,from_grapple:bool,
        live:&crate::gameplay::entity_lifetime::LiveEntities,
        receipt:&mut crate::gameplay::effects::instances::PickupReceipt,
    )->Result<(),String>{
        if let Some(error)=&receipt.failure{return Err(error.clone());}
        if self.closed || self.source.is_none(){return Err("pickup graph manager is not active".into());}
        if live.stage(target)!=Some(crate::gameplay::entity_lifetime::Stage::Spawned){
            return Err("pickup notification target is not a spawned entity".into());
        }
        let arguments=(target,successful,from_grapple);
        if let Some(saved)=receipt.arguments{
            if saved!=arguments{return Err("pickup receipt belongs to another actual event".into());}
        }else{
            if let Some(listeners)=self.pickup_subscribers.get(&target){
                receipt.receivers.try_reserve_exact(listeners.len()).map_err(|error|format!("pickup receiver snapshot: {error}"))?;
                receipt.receivers.extend(listeners.iter().rev().copied());
            }
            receipt.arguments=Some(arguments);
        }
        while let Some(&(graph,node))=receipt.receivers.get(receipt.next){
            // Once a callback mutates its notification/action stack, retrying
            //the same failing callback would double delivery. Preserve a real
            //failure receipt rather than acknowledging or replaying it.
            match self.graphs.picked_up(graph,node,successful,from_grapple){
                Ok(errors)=>{
                    receipt.next+=1;
                    receipt.errors.extend(errors);
                }
                Err(error)=>{receipt.failure=Some(error.clone());return Err(error);}
            }
        }
        if !receipt.errors.is_empty(){
            let error=receipt.errors.join("; ");receipt.failure=Some(error.clone());return Err(error);
        }
        Ok(())
    }
    pub(crate) fn dispatch_picked_up(
        &mut self,
        target: crate::gameplay::entity_lifetime::EntityId,
        successful: bool,
        from_grapple: bool,
        live: &crate::gameplay::entity_lifetime::LiveEntities,
    ) -> Result<Vec<String>, String> {
        let mut receipt=crate::gameplay::effects::instances::PickupReceipt::default();
        if let Err(error)=self.dispatch_picked_up_receipt(target,successful,from_grapple,live,&mut receipt){
            if receipt.errors.is_empty() || receipt.next!=receipt.receivers.len(){return Err(error);}
        }
        let receivers=receipt.receivers.len();
        let errors=receipt.errors;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!(
                "ENTITY_GRAPH_PICKUP target={target:?} receivers={receivers} successful={successful} from_grapple={from_grapple} errors={}",
                errors.len()
            );
        }
        Ok(errors)
    }
    pub(crate) fn retire_entity_graph(
        &mut self,
        entity: crate::gameplay::entity_lifetime::EntityId,
    ) -> Result<(), String> {
        let indices = match self.entity_graphs.get(&entity) {
            Some(root) => self
                .graphs
                .retirement_subtree(*root)
                .ok_or("entity graph subtree is busy")?,
            None => {
                self.pickup_subscribers.remove(&entity);
                self.breakable_subscribers.remove(&entity);
                return Ok(());
            }
        };
        let ids: HashSet<_> = indices
            .iter()
            .filter_map(|&index| self.graphs.instance_id(index))
            .collect();
        self.pickup_subscribers.retain(|target, listeners| {
            listeners.retain(|(graph, _)| !ids.contains(graph));
            *target != entity && !listeners.is_empty()
        });
        self.breakable_subscribers.retain(|target, listeners| {
            listeners.retain(|(graph, _)| !ids.contains(graph));
            *target != entity && !listeners.is_empty()
        });
        self.detach_graph_receivers(&indices.iter().copied().collect());
        self.graphs.retire_detached(&indices)?;
        self.started_entity_graphs.remove(&entity);
        self.entity_graphs.remove(&entity);
        self.report_graph_lifetime("entity-retired");
        Ok(())
    }
    pub(super) fn detach_graph_receivers(&mut self, graphs: &HashSet<usize>) {
        for entity in self.entities.values_mut() {
            if matches!(&entity.owner, FxEntityOwner::Node { graph, .. } if graphs.contains(graph))
            {
                entity.owner = FxEntityOwner::Detached;
            }
        }
        for instance in self.slots.iter_mut().flatten() {
            if instance
                .actor_observer
                .is_some_and(|(graph, _)| graphs.contains(&graph))
            {
                instance.actor_observer = None;
            }
        }
        // 7c10b0 skips queued work once its node instance has been detached.
        self.graphs
            .events
            .retain(|event| !graphs.contains(&event.graph()));
        self.timers.retain(|(graph, _)| !graphs.contains(graph));
        self.started.retain(|(graph, _, _)| !graphs.contains(graph));
        self.completed
            .retain(|(graph, _, _)| !graphs.contains(graph));
        self.actor_completed
            .retain(|(graph, _, _)| !graphs.contains(graph));
    }
    pub(super) fn dynamic_retired(&self) -> u64 {
        self.dynamic_retired
    }
    /// Prepare the live WORLD projections and publish compact frame records.
    /// Their target arrays remain shared with the retained committed records.
    pub(super) fn snapshot_decals(
        &mut self,
        tree: &crate::local_caster_query::LocalWorldQuery,
        limit: usize,
    ) -> Result<
        Vec<(
            u32,
            u64,
            crate::rendering::decals::CommittedDecal<super::decal_effects::DecalReferences>,
        )>,
        String,
    > {
        let pool = self.decal_pool();
        pool.reconfigure_when_empty(limit, |_| {})?;
        pool.prepare_world(tree)?;
        Ok((0..pool.limit() as u32)
            .filter_map(|slot| {
                let entry = pool.slot(slot)?;
                Some((slot, entry.owner()?.get(), entry.renderer.as_ref()?.clone()))
            })
            .collect())
    }

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
        frames: &FrameReferences<'_>,
        live: Option<&crate::gameplay::entity_lifetime::LiveEntities>,
    ) -> Result<(), String> {
        if self
            .source
            .as_ref()
            .is_some_and(|old| !Arc::ptr_eq(old, &sources.graphs))
        {
            return Err("world FX level changed before the shared pool was reset".into());
        }
        if self.source.is_none() {
            self.prepare_graphs(sources, &assets.declarations, live)?;
        }
        if self.actors_initialized {
            return Ok(());
        }
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
                    local: None,
                    dynamic_sources: None,
                    dynamic_retirement: None,
                    entity_context: None,
                    explicit_target: None,
                    motion: crate::gameplay::particles::spawn::SpawnVelocity {
                        center: pose.position,
                        ..Default::default()
                    },
                },
            );
        }
        self.actors_initialized = true;
        Ok(())
    }
    fn stop(&mut self, handle: u32, systems: &mut ParticleSystems) -> Result<(), String> {
        if let Some(&slot) = self.handles.get(&handle) {
            if let Some(instance) = self.slots[usize::from(slot)].as_mut() {
                for event in &mut instance.gameplay_events {
                    event.stop();
                }
                systems.stop(instance.head, instance.age);
                self.lights.stop(instance.light_head, instance.age, false)?;
                self.distortion.stop(instance.distortion_head, instance.age, false)?;
                self.rumble.stop(instance.rumble_head, instance.age, false)?;
                self.environments.stop_chain(instance.environment_head, instance.age, false)?;
                self.fovs.stop(instance.fov_head,instance.age,false)?;
                self.quads.stop(instance.quad_head,instance.age,false)?;
                self.shakes.stop(instance.shake_head, instance.age, false)?;
                self.fades.stop(instance.fade_head, instance.age, false)?;
                self.decals.stop(instance.decal_head, instance.age);
                self.sound_systems
                    .stop(instance.sound_head, instance.age, &mut systems.seed)?;
                if !instance.sub_effects.is_empty() {
                    let sounds = self.sounds.as_ref().ok_or("FX sound owner is absent")?;
                    let mut chains = super::world_sub_effects::Chains {
                        systems,
                        sound_systems: &mut self.sound_systems,
                        lights: &mut self.lights,
                    distortion: &mut self.distortion,
                    rumble: &mut self.rumble,
                    environments: &mut self.environments,
                    fovs: &mut self.fovs,
                    quads: &mut self.quads,
                    shakes: &mut self.shakes,
                    fades: &mut self.fades,
                        decals: &mut self.decals,
                        sounds,
                    };
                    for sub in &mut instance.sub_effects {
                        sub.stop(instance.age, &mut chains)?;
                    }
                }
            }
            //60ad40 removes the contact journal row AFTER every real stop
            //request succeeded. Family tails keep their existing owner/ACK.
            self.contact_instances.remove(handle);
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
        frames: &FrameReferences<'_>,
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
        // bevy_backend: keep the owner and its advancing chain heads live
        // until every fallible host release succeeds. Taking the slot first
        // drops that progress on an error but leaves handles/pool links live.
        let Some(instance) = self.slots[usize::from(slot)].as_mut() else {
            return Ok(());
        };
        systems.clear(&mut instance.head)?;
        self.lights.clear(&mut instance.light_head)?;
        self.distortion.clear(&mut instance.distortion_head)?;
        self.rumble.clear(&mut instance.rumble_head)?;
        self.environments.clear(&mut instance.environment_head)?;
        self.fovs.clear(&mut instance.fov_head)?;
        self.quads.clear(&mut instance.quad_head)?;
        self.shakes.clear(&mut instance.shake_head)?;
        self.fades.clear(&mut instance.fade_head)?;
        self.decals.clear(&mut instance.decal_head, |_| {})?;
        if !instance.sub_effects.is_empty() {
            let sounds = self.sounds.as_ref().ok_or("FX sound owner is absent")?;
            let mut chains = super::world_sub_effects::Chains {
                systems,
                sound_systems: &mut self.sound_systems,
                lights: &mut self.lights,
                    distortion: &mut self.distortion,
                    rumble: &mut self.rumble,
                    environments: &mut self.environments,
                    fovs: &mut self.fovs,
                    quads: &mut self.quads,
                    shakes: &mut self.shakes,
                    fades: &mut self.fades,
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
        // All remaining operations use the completed release record; no
        // resource-family clear can now discard a still-owned chain.
        let instance = self.slots[usize::from(slot)]
            .take()
            .expect("cleared FX instance remains owned until retirement");
        let one_shot = instance
            .dynamic_retirement
            .as_ref()
            .filter(|lifetime| lifetime.once)
            .cloned();
        self.contact_instances.remove(instance.id);
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
                FxEntityOwner::Detached => {}
                FxEntityOwner::Actor { active, .. } => {
                    if *active == Some(instance.id) {
                        *active = None;
                    }
                    if let Some((graph, node)) = instance.actor_observer {
                        self.actor_completed.push((graph, node, instance.entity));
                    }
                    if self.dynamic_retiring.remove(&instance.entity) {
                        self.dynamic_retired = self.dynamic_retired.wrapping_add(1);
                    } else if one_shot.is_none() {
                        self.entities.insert(instance.entity, entity);
                    }
                }
            }
        }
        let mut head = slot;
        self.pool.release_head(&mut head)?;
        if let Some(lifetime) = one_shot {
            lifetime
                .completed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        Ok(())
    }

    /// 199df0 clears each remaining instance's completion callback before
    /// releasing its effect families, then drains deferred instance removal.
    /// Actual component Stop and graph destruction precede this owner phase;
    /// the host's synthetic FX entities are cleared here with their owner.
    pub(super) fn shutdown(&mut self, systems: &mut ParticleSystems) -> Result<(), String> {
        let trace = std::env::var_os("DH2_DEBUG_FX_RESOURCES").is_some();
        let started = trace.then(std::time::Instant::now);
        let cycles = trace.then(crate::loading::thread_cycles).flatten();
        let before = trace.then(|| {
            (
                self.slots.iter().flatten().count(),
                self.handles.len(),
                self.pending_starts.len(),
                systems.allocated_count(),
                self.sound_systems.allocated_count(),
                self.decals.allocated_count(),
            )
        });
        let result = self.shutdown_inner(systems);
        if let Some((instances, handles, starts, particles, sounds, decals)) = before {
            let elapsed = started.expect("trace start").elapsed().as_secs_f64() * 1000.0;
            let cycles = cycles
                .zip(crate::loading::thread_cycles())
                .map(|(start, end)| end.saturating_sub(start));
            eprintln!(
                "FX_RESOURCES shutdown_ok={} instances={}->{} handles={}->{} pending_starts={}->{} shared_particle_slots={}->{} sound_slots={}->{} decal_slots={}->{} elapsed_ms={elapsed:.3} thread_cycles={cycles:?}",
                result.is_ok(),
                instances,
                self.slots.iter().flatten().count(),
                handles,
                self.handles.len(),
                starts,
                self.pending_starts.len(),
                particles,
                systems.allocated_count(),
                sounds,
                self.sound_systems.allocated_count(),
                decals,
                self.decals.allocated_count()
            );
        }
        result
    }

    fn shutdown_inner(&mut self, systems: &mut ParticleSystems) -> Result<(), String> {
        // 7c0f30 destroys graph receivers before separately owned FX tails.
        // Closing is persistent across a fallible family clear: no later view
        // update may resurrect graph work while shutdown is being retried.
        self.closed = true;
        let indices: Vec<_> = self.graphs.live_indices().collect();
        self.detach_graph_receivers(&indices.iter().copied().collect());
        self.graphs.retire_detached(&indices)?;
        // A family clear can notify another still-live instance in this host.
        // Disarm every completion route before the first fallible clear.
        for instance in self.slots.iter_mut().flatten() {
            instance.actor_observer = None;
        }
        for entity in self.entities.values_mut() {
            if matches!(entity.owner, FxEntityOwner::Node { .. }) {
                entity.owner = FxEntityOwner::Detached;
            }
        }
        self.pickup_subscribers.clear();
        self.breakable_subscribers.clear();
        self.entity_graphs.clear();
        self.started_entity_graphs.clear();
        self.report_graph_lifetime("graphs-retired");
        self.started.clear();
        self.completed.clear();
        self.actor_completed.clear();
        for position in 0..self.entity_order.len() {
            let entity_id = self.entity_order[position];
            let handle = self
                .entities
                .get(&entity_id)
                .and_then(|entity| match &entity.owner {
                    FxEntityOwner::Actor { active, .. } => *active,
                    FxEntityOwner::Node { .. } | FxEntityOwner::Detached => {
                        entity.component.handle()
                    }
                });
            if let Some(slot) = handle.and_then(|handle| self.handles.get(&handle).copied()) {
                self.shutdown_instance(slot, systems)?;
            }
        }
        // The real manager can also own detached/pending instances that no
        // longer have an entity entry. Keep those leases until cleared too.
        for index in 0..self.slots.len() {
            if self.slots[index].is_some() {
                self.shutdown_instance(index as u16, systems)?;
            }
        }
        if !self.contact_instances.is_empty() {
            return Err("ContactSystem retains live definition buckets after FX shutdown".into());
        }
        if !self.distortion.drained() { return Err("FX FSDisto pool retains a live chain after shutdown".into()); }
        self.distortion.shutdown();
        if !self.environments.drained() { return Err("FX environment pool retains a live chain after shutdown".into()); }
        if !self.rumble.drained() { return Err("FX rumble pool retains a live chain after shutdown".into()); }
        self.rumble.shutdown();
        if !self.fovs.drained()||!self.quads.drained(){return Err("FX FOV/quad pool retains a live chain after shutdown".into());}
        self.fovs.shutdown();self.quads.shutdown();
        if !self.shakes.drained() || !self.fades.drained() { return Err("FX shake/fade pool retains a live chain after shutdown".into()); }
        self.shakes.shutdown();
        self.fades.shutdown();
        if !self.lights.drained() { return Err("FX light pool retains a live chain after instance shutdown".into()); }
        self.pending_starts.clear();
        self.pending_actor_starts.clear();
        self.entities.clear();
        self.entity_order.clear();
        self.actors.clear();
        self.dynamic.clear();
        self.dynamic_retiring.clear();
        self.dynamic_snapshot = None;
        Ok(())
    }

    fn shutdown_instance(
        &mut self,
        slot: u16,
        systems: &mut ParticleSystems,
    ) -> Result<(), String> {
        let instance = self
            .slots
            .get_mut(usize::from(slot))
            .and_then(Option::as_mut)
            .ok_or("FX shutdown instance absent")?;
        instance.actor_observer = None;
        if let Some(entity) = self.entities.get_mut(&instance.entity) {
            if matches!(entity.owner, FxEntityOwner::Node { .. }) {
                entity.owner = FxEntityOwner::Detached;
            }
        }
        self.retire(slot, systems)
    }

    fn sync_dynamic(
        &mut self,
        snapshot: &super::world_effects::DynamicEffects,
        systems: &mut ParticleSystems,
        frames: &FrameReferences<'_>,
    ) -> Result<(), String> {
        if self
            .dynamic_snapshot
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &snapshot.0))
        {
            return Ok(());
        }
        let started = super::world_effects::trace_lifecycle().then(std::time::Instant::now);
        let removed: Vec<_> = self
            .dynamic
            .iter()
            .filter_map(|(owner, &(id, revision))| {
                (!snapshot
                    .0
                    .iter()
                    .any(|entry| entry.owner == *owner && entry.revision == revision))
                .then(|| (owner.clone(), id))
            })
            .collect();
        for (owner, id) in removed {
            if started.is_some() {
                eprintln!("FX_LIFECYCLE stop owner={owner}");
            }
            self.dynamic.remove(&owner);
            self.pending_actor_starts.retain(|pending| *pending != id);
            let active = self
                .entities
                .get(&id)
                .and_then(|entity| match &entity.owner {
                    FxEntityOwner::Actor { active, .. } => *active,
                    _ => None,
                });
            if let Some(handle) = active {
                self.stop(handle, systems)?;
                self.dynamic_retiring.insert(id);
            } else {
                self.entities.remove(&id);
                self.dynamic_retired = self.dynamic_retired.wrapping_add(1);
            }
        }
        for entry in snapshot.0.iter() {
            let pose = if let Some(world) = entry.initial_world {
                world
            } else {
                let parent = frames
                    .resolve(&entry.target, &entry.tag)?
                    .ok_or("dynamic FX target pose is unavailable")?;
                crate::gameplay::reference_frame::compose(parent, entry.local)
            };
            //87cb90's explicit matrix is a detached placement. Context keeps
            //the real owner, clock, velocities and callbacks; it does not turn
            //that owner into a transform parent (or bind an Invalid target).
            let has_parent = entry.initial_world.is_none();
            let location = has_parent.then(|| entry.target.clone());
            let local = has_parent.then_some(entry.local);
            if let Some(&(id, _)) = self.dynamic.get(&entry.owner) {
                // 87dc10 changes an existing effect's placement without a
                // new Start. Keep its age, source lease and active handle.
                if let Some(entity) = self.entities.get_mut(&id) {
                    entity.explicit_target = entry.explicit_target;
                    entity.pose = pose;
                    entity.local = local;
                    entity.parameters.location = location.clone();
                    entity.parameters.tag.clone_from(&entry.tag);
                    entity.parameters.update_position = entry.attached;
                    if let Some(parameters)=&entry.component_parameters {
                        entity.parameters.target=parameters.target.as_ref().map(|(target,_)|target.clone());
                        entity.parameters.target_tag=parameters.target.as_ref().map_or_else(String::new,|(_,tag)|tag.clone());
                        entity.parameters.dont_spawn_if_culled=parameters.dont_spawn_if_culled;
                        entity.parameters.stop_on_target_removal=parameters.stop_on_target_removal;
                        entity.parameters.stop_on_parent_removal=parameters.stop_on_parent_removal;
                    }
                }
                for instance in self
                    .slots
                    .iter_mut()
                    .flatten()
                    .filter(|instance| instance.entity == id)
                {
                    if let Some(target) = entry.explicit_target {
                        instance.target = target;
                    }
                    instance.pose = pose;
                    instance.local = local;
                    instance.parameters.location = location.clone();
                    instance.parameters.tag.clone_from(&entry.tag);
                    instance.parameters.update_position = entry.attached;
                    if let Some(parameters)=&entry.component_parameters {
                        instance.parameters.target=parameters.target.as_ref().map(|(target,_)|target.clone());
                        instance.parameters.target_tag=parameters.target.as_ref().map_or_else(String::new,|(_,tag)|tag.clone());
                        instance.parameters.dont_spawn_if_culled=parameters.dont_spawn_if_culled;
                        instance.parameters.stop_on_target_removal=parameters.stop_on_target_removal;
                        instance.parameters.stop_on_parent_removal=parameters.stop_on_parent_removal;
                    }
                }
                self.dynamic_reference_event(id,entry,systems,frames)?;
                continue;
            }
            let id = self.next_entity;
            self.next_entity = id.wrapping_add(1);
            self.entities.insert(
                id,
                FxEntity {
                    owner: FxEntityOwner::Actor {
                        name: entry.owner.clone(),
                        active: None,
                        remove_on_end: false,
                    },
                    parameters: FxRuntimeParameters {
                        effect: Some(entry.effect.clone()),
                        location,
                        tag: entry.tag.clone(),
                        update_position: entry.attached,
                        target: entry.component_parameters.as_ref().and_then(|parameters|parameters.target.as_ref()).map(|(target,_)|target.clone()),
                        target_tag: entry.component_parameters.as_ref().and_then(|parameters|parameters.target.as_ref()).map_or_else(String::new,|(_,tag)|tag.clone()),
                        dont_spawn_if_culled: entry.component_parameters.as_ref().is_some_and(|parameters|parameters.dont_spawn_if_culled),
                        stop_on_target_removal: entry.component_parameters.as_ref().is_some_and(|parameters|parameters.stop_on_target_removal),
                        stop_on_parent_removal: entry.component_parameters.as_ref().is_some_and(|parameters|parameters.stop_on_parent_removal),
                        ..Default::default()
                    },
                    component: Default::default(),
                    suspended: false,
                    pose,
                    local,
                    dynamic_sources: Some(entry.sources.clone()),
                    dynamic_retirement: Some(entry.retirement.clone()),
                    entity_context: entry.entity_context,
                    explicit_target: entry.explicit_target,
                    motion: crate::gameplay::particles::spawn::SpawnVelocity {
                        center: pose.position,
                        linear: entry
                            .entity_context
                            .map_or([0.; 3], |context| context.velocities[0]),
                        angular: entry
                            .entity_context
                            .map_or([0.; 3], |context| context.velocities[1]),
                        angular_enabled: entry.entity_context.is_some(),
                    },
                },
            );
            self.entity_order.push(id);
            self.pending_actor_starts.push(id);
            self.dynamic
                .insert(entry.owner.clone(), (id, entry.revision));
            self.dynamic_reference_event(id,entry,systems,frames)?;
            if started.is_some() {
                eprintln!(
                    "FX_LIFECYCLE start owner={} effect={}",
                    entry.owner, entry.effect
                );
            }
        }
        self.dynamic_snapshot = Some(snapshot.0.clone());
        if let Some(started) = started {
            eprintln!(
                "FX_LIFECYCLE owner_update_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        Ok(())
    }
    ///4b22d0/4b1cc0, delivered through the same immutable FX publication.
    ///The receipt lives with the existing component kernel and real lifetime.
    fn dynamic_reference_event(&mut self,id:u32,entry:&super::world_effects::DynamicEffect,
        systems:&mut ParticleSystems,frames:&FrameReferences<'_>)->Result<(),String>{
        let Some((revision,parent_present,stopping))=entry.reference_event else{return Ok(());};
        let Some(mut entity)=self.entities.remove(&id)else{return Ok(());};
        let mut component=std::mem::take(&mut entity.component);
        let stop_on_parent_removal=stopping||entity.parameters.stop_on_parent_removal;
        let result=component.reference_event_once(revision,parent_present,stop_on_parent_removal,
            &mut InstanceHost{state:self,systems,frames,sources:&entry.sources,entity:&mut entity,id});
        //Immediate native stop can retire while this source wrapper is
        //borrowed out of the map. Publish that real handle loss here too.
        if let FxEntityOwner::Actor{active,..}=&mut entity.owner{
            if active.is_some_and(|handle|!self.handles.contains_key(&handle)){*active=None;}
        }
        if result.is_ok()&&stopping{
            entity.suspended=true;
            self.pending_actor_starts.retain(|&pending|pending!=id);
            if component.handle().is_none(){
                if let Some(lifetime)=&entity.dynamic_retirement{
                    lifetime.completed.store(true,std::sync::atomic::Ordering::Release);
                }
            }
        }
        entity.component=component;
        self.entities.insert(id,entity);
        result
    }
    /// 14019d100 + 1401a2ff0 for one particle FX event: a detached FX
    /// entity at the event pose whose instance starts with the deferred
    /// starts.
    fn start_event_effect(
        &mut self,
        start: crate::gameplay::particles::fx_event::FxEventStart,
        target: crate::gameplay::particles::beam::BeamTarget,
        sources: &WorldFxSources,
    ) -> Result<(), String> {
        let definition = sources
            .effects
            .get(&start.fx)
            .ok_or_else(|| format!("particle FX event effect {} was not prepared", start.fx))?
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        admit(
            &start.fx,
            &definition,
            &sources.effects,
            self.sounds.as_ref(),
            &self.decal_programs,
            &mut HashSet::new(),
        )?;
        let mut slot = u16::MAX;
        if !self.pool.acquire(&mut slot)? {
            return Ok(());
        }
        let pose = ReferenceFrame {
            position: start.position,
            basis: start.axis,
        };
        let parameters = FxRuntimeParameters {
            effect: Some(start.fx),
            ..Default::default()
        };
        let entity = self.next_entity;
        self.next_entity = entity.wrapping_add(1);
        let motion = crate::gameplay::particles::spawn::SpawnVelocity {
            center: pose.position,
            ..Default::default()
        };
        self.entities.insert(
            entity,
            FxEntity {
                owner: FxEntityOwner::Detached,
                parameters: parameters.clone(),
                component: Default::default(),
                suspended: false,
                pose,
                local: None,
                dynamic_sources: None,
                dynamic_retirement: None,
                entity_context: None,
                explicit_target: None,
                motion,
            },
        );
        let id = self.next_instance;
        self.next_instance = id.wrapping_add(1);
        self.slots[usize::from(slot)] = Some(Instance {
            gameplay_events: Vec::new(),
            id,
            entity,
            definition,
            parameters,
            pose,
            local: None,
            dynamic_sources: None,
            dynamic_retirement: None,
            entity_context: None,
            motion,
            age: 0.,
            head: u16::MAX,
            light_head: u16::MAX,
            sound_head: u16::MAX,
            decal_head: u16::MAX,
            distortion_head: u16::MAX,
            fov_head: u16::MAX,
            quad_head: u16::MAX,
            shake_head: u16::MAX,
            fade_head: u16::MAX,
            rumble_head: u16::MAX,
            environment_head: u16::MAX,
            sub_effects: Vec::new(),
            actor_observer: None,
            target,
        });
        self.handles.insert(id, slot);
        self.pending_starts.push(slot);
        Ok(())
    }

    pub fn advance(
        &mut self,
        graphs: &WorldEffectDefinitions,
        sources: &WorldFxSources,
        dynamic: &super::world_effects::DynamicEffects,
        frames: &WorldEffectFrames,
        systems: &mut ParticleSystems,
        sounds: &super::world_sounds::WorldSounds,
        decal_scene: &super::projected_decals::DecalScene,
        frame: u32,
        camera: ReferenceFrame,
        underwater: bool,
        world: &crate::gameplay::world_point_query::level::LevelPointQueries,
        live: Option<&crate::gameplay::entity_lifetime::LiveEntities>,
        managed: Option<&dyn ManagedFrames>,
    ) -> Result<(), String> {
        let references=FrameReferences{frames,managed};
        let frames=&references;
        if self.closed || self.frame == Some(frame) {
            return Ok(());
        }
        if !self.actors_initialized {
            //bevy_backend:87e8e0 starts placed actors with loaded declarations.
            //Our asynchronous EntityDef/Explosion acquisition must acknowledge
            //those actual dependencies before the one-shot actor admission.
            //Do not consume pending_actor_starts while that host preparation
            //is incomplete; this is not a retry of a native pool refusal.
            if !sources.gameplay_ready() {
                return Ok(());
            }
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
                            .and_then(Option::as_ref)
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
        self.initialize(graphs, sources, frames, live)?;
        self.sync_dynamic(dynamic, systems, frames)?;
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
                .is_some_and(|age| age.as_secs_f64() >= debug_effect_start())
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
                    local: None,
                    dynamic_sources: None,
                    dynamic_retirement: None,
                    entity_context: None,
                    explicit_target: None,
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
            self.prepare_named_graph(index,live)?;
            self.graphs.prepare_story_bindings(index)?;
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
        for error in self.graphs.poll_gameflow() {
            self.report(error);
        }
        for error in self.graphs.poll_spawns() {
            self.report(error);
        }
        self.graphs.events.begin_update();
        while let Some(event) = self.graphs.events.pop() {
            self.prepare_named_graph(event.graph(),live)?;
            let (index, node, port) = match event {
                GraphEvent::Action { graph, node, port } => (graph, node, port),
                GraphEvent::LoadingEnd { graph, node } => {
                    self.graphs[graph]
                        .as_mut()
                        .ok_or("startup graph busy")?
                        .loading_end_node(node)?;
                    self.collect(graph);
                    continue;
                }
            };
            self.graphs.prepare_story_bindings(index)?;
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
                Ok(GraphStep::BreakableReset { node }) => {
                    let graph = self
                        .graphs
                        .instance_id(index)
                        .ok_or("breakable reset graph retired")?;
                    self.reset_breakable_receiver(
                        graph,
                        node,
                        live.ok_or("breakable reset has no managed entity owner")?,
                    )?;
                }
                Ok(GraphStep::PickupReset { node }) => {
                    let graph = self
                        .graphs
                        .instance_id(index)
                        .ok_or("pickup reset graph retired")?;
                    self.reset_pickup_receiver(
                        graph,
                        node,
                        live.ok_or("pickup reset has no managed entity owner")?,
                    )?;
                }
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
                    && !entity.parameters.update_position && entity.parameters.target.is_none())
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
                        let pose = entity.local.map_or(pose, |local| {
                            crate::gameplay::reference_frame::compose(pose, local)
                        });
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
        self.distortion.begin_frame();
        self.shakes.begin_frame();
        //1a3390 selects the last real RenderView callback.806600's
        //interface virtual20=486a50 returns RenderView+a8, the same origin
        //published in this selected camera frame. No Player/body substitute.
        self.rumble.begin_frame(Some(camera.position));
        // FX the particle systems' m_fxEvents started (1401da820: an FX
        // entity at the event with the system's origin or target as its
        // record target, then 1401a2ff0).
        for (start, target) in systems.take_fx_starts() {
            if let Err(error) = self.start_event_effect(start, target, sources) {
                self.report(error);
            }
        }
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
                &mut instance.light_head,
                &mut instance.sound_head,
                &mut instance.decal_head,
                &mut instance.distortion_head,
                &mut instance.fov_head,
                &mut instance.quad_head,
                &mut instance.shake_head,
                &mut instance.fade_head,
                &mut instance.rumble_head,
                &mut instance.environment_head,
                &mut instance.gameplay_events,
                &mut instance.sub_effects,
                &mut GroupContext {
                    view_mask: 1,
                    gameplay: &mut self.gameplay,
                    frame: instance.pose,
                    entity_context: instance.entity_context,
                    systems,
                    sound_systems: &mut self.sound_systems,
                    lights: &mut self.lights,
                    distortion: &mut self.distortion,
                    rumble: &mut self.rumble,
                    environments: &mut self.environments,
                    fovs: &mut self.fovs,
                    quads: &mut self.quads,
                    shakes: &mut self.shakes,
                    fades: &mut self.fades,
                    decals: &mut self.decals,
                    sounds,
                    effects: &instance
                        .dynamic_sources
                        .as_deref()
                        .unwrap_or(sources)
                        .effects,
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
                            let pose = instance.local.map_or(pose, |local| {
                                crate::gameplay::reference_frame::compose(pose, local)
                            });
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
                        target: instance.target,
                    },
                    underwater,
                    motion: instance.motion,
                    position: bevy::math::Vec3::from_array(instance.pose.position),
                    basis,
                    sound: super::world_sounds::SoundFrame {
                        entity_context: instance.entity_context,
                        axis: basis,
                        scale: clock.scale,
                        energy: instance.entity_context.map_or(0., |context| {
                            context.contact_speed * context.contact_speed * context.contact_mass
                        }),
                    },
                };
                let mut context = GroupContext {
                    view_mask: update.particle.view_mask,
                    gameplay: &mut self.gameplay,
                    frame: instance.pose,
                    entity_context: instance.entity_context,
                    systems,
                    sound_systems: &mut self.sound_systems,
                    lights: &mut self.lights,
                    distortion: &mut self.distortion,
                    rumble: &mut self.rumble,
                    environments: &mut self.environments,
                    fovs: &mut self.fovs,
                    quads: &mut self.quads,
                    shakes: &mut self.shakes,
                    fades: &mut self.fades,
                    decals: &mut self.decals,
                    sounds,
                    effects: &instance
                        .dynamic_sources
                        .as_deref()
                        .unwrap_or(sources)
                        .effects,
                    world,
                    query: &mut query,
                    position: instance.pose.position,
                };
                let alive = update_chains(
                    &mut instance.head,
                    &mut instance.light_head,
                    &mut instance.sound_head,
                    &mut instance.decal_head,
                &mut instance.distortion_head,
                &mut instance.fov_head,
                &mut instance.quad_head,
                &mut instance.shake_head,
                &mut instance.fade_head,
                &mut instance.rumble_head,
                &mut instance.environment_head,
                    &mut instance.gameplay_events,
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
        self.distortion.finish_frame();
        self.shakes.finish_frame();
        self.fades.finish_frame();
        self.rumble.finish_frame(frame);
        Ok(())
    }
}

struct GraphHost<'a> {
    state: &'a mut WorldParticleState,
    systems: &'a mut ParticleSystems,
    frames: &'a FrameReferences<'a>,
    sources: &'a WorldFxSources,
    clock: WorldFxClock,
    graph: usize,
}
impl FxGraphWorld for GraphHost<'_> {
    fn managed_entity(&self,entity:&EntityReference)->Result<Option<crate::gameplay::entity_lifetime::EntityId>,String>{
        self.frames.managed.ok_or("script entity resolver is unavailable")?.entity(entity)
    }
    fn managed_spawn_tag(&self,id:crate::gameplay::entity_lifetime::EntityId)->Result<String,String>{self.frames.managed.ok_or("script spawn tag resolver unavailable")?.spawn_tag(id)}
    fn managed_frame(&self,id:crate::gameplay::entity_lifetime::EntityId)->Result<Option<ReferenceFrame>,String>{
        self.frames.managed.ok_or("script frame resolver is unavailable")?.frame(id,"")
    }

    fn entity_effect_ready(&self, entity: &EntityReference) -> Result<bool, String> {
        let Some(name) = self.frames.entity_name(entity)? else {
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
        let Some(name) = self.frames.entity_name(entity)? else {
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
        let Some(name) = self.frames.entity_name(entity)? else {
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
            .map(|result| {
                result
                    .as_ref()
                    .map(|definition| definition.gameplay_ready())
                    .map_err(Clone::clone)
            })
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
                local: None,
                dynamic_sources: None,
                dynamic_retirement: None,
                entity_context: None,
                explicit_target: None,
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
    frames: &'a FrameReferences<'a>,
    sources: &'a WorldFxSources,
    entity: &'a FxEntity,
    id: u32,
}
impl InstanceHost<'_> {
    /// 1404b1880: the target entity's world pose, or the pose of its target
    /// tag or joint (1404b19b0); None while the target entity is absent.
    fn target_pose(&self) -> Result<Option<crate::gameplay::particles::beam::BeamTarget>, String> {
        if let Some(target) = self.entity.explicit_target {
            return Ok(Some(target));
        }
        let parameters = &self.entity.parameters;
        let Some(target) = parameters.target.as_ref() else {
            return Ok(None);
        };
        Ok(self
            .frames
            .resolve(target, &parameters.target_tag)?
            .map(|pose| crate::gameplay::particles::beam::BeamTarget {
                position: pose.position,
                basis: pose.basis,
            }))
    }
}
impl WorldFxComponentHost for InstanceHost<'_> {
    fn has_effect(&self) -> bool {
        self.entity.parameters.effect.is_some()
    }
    fn reference_readiness(&self) -> Option<ReferenceReadiness> {
        let sources = self
            .entity
            .dynamic_sources
            .as_deref()
            .unwrap_or(self.sources);
        if self
            .entity
            .parameters
            .effect
            .as_ref()
            .and_then(|name| sources.effects.get(name))
            .and_then(|result| result.as_ref().ok())
            .is_some_and(|definition| !definition.gameplay_ready())
        {
            return Some(ReferenceReadiness {
                state: 0,
                pending_binding: false,
            });
        }
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
        //A genuine one-shot refusal/completion is not a pending asset and
        //cannot be replayed while main-thread withdrawal is observed later.
        if self
            .entity
            .dynamic_retirement
            .as_ref()
            .is_some_and(|lifetime| {
                lifetime.once
                    && lifetime
                        .completed
                        .load(std::sync::atomic::Ordering::Acquire)
            })
        {
            return Ok(None);
        }
        let parameters = &self.entity.parameters;
        // 1404b1d80: an existing target entity initializes its tag or joint
        // (1404b19b0) and writes the target pose into the new record
        // (1404b20b0); otherwise the record keeps 14021dc10's defaults.
        let target = self.target_pose()?.unwrap_or_default();
        let name = parameters.effect.as_ref().ok_or("world FX effect absent")?;
        if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
            bevy::log::info!(effect = name, "World FX play requested");
        }
        let sources = self
            .entity
            .dynamic_sources
            .as_deref()
            .unwrap_or(self.sources);
        let definition = sources
            .effects
            .get(name)
            .ok_or("world FX definition absent")?
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        if !definition.gameplay_ready() {
            return Ok(None);
        }
        // Admit only complete supported groups, never skip another enabled
        // family and consequently consume an incorrect shared RNG sequence.
        admit(
            name,
            &definition,
            &sources.effects,
            self.state.sounds.as_ref(),
            &self.state.decal_programs,
            &mut HashSet::new(),
        )?;
        let pose = if parameters.location.is_none() && self.entity.dynamic_retirement.is_some() {
            // Explicit-world dynamic starts already carry the actual source
            // frame. No later parent movement or retirement can change it.
            self.entity.pose
        } else {
            let pose = self
                .frames
                .resolve(
                    parameters.location.as_ref().ok_or("FX location absent")?,
                    &parameters.tag,
                )?
                .ok_or("FX location was removed")?;
            self.entity.local.map_or(pose, |local| {
                crate::gameplay::reference_frame::compose(pose, local)
            })
        };
        //The header Arc, not path spelling, is60af50's declaration identity.
        let contact_definition = self
            .entity
            .entity_context
            .filter(|context| context.contact_stimulus_enabled.is_some())
            .map(|_| definition.source.clone());
        let mut slot = u16::MAX;
        if !self.state.pool.acquire(&mut slot)? {
            if contact_definition.is_some() {
                let lifetime = self
                    .entity
                    .dynamic_retirement
                    .as_ref()
                    .filter(|lifetime| lifetime.once)
                    .ok_or("material contact refusal has no actual one-shot source owner")?;
                //No slot/family exists: actual allocation refusal closes this
                //accepted packet, not an audio/GPU readiness acknowledgement.
                lifetime
                    .completed
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            if std::env::var_os("DH2_TRACE_WORLD_FX").is_some() {
                bevy::log::info!(effect = name, "World FX instance pool exhausted");
            }
            return Ok(None);
        }
        let id = self.state.next_instance;
        self.state.next_instance = id.wrapping_add(1);
        self.state.slots[usize::from(slot)] = Some(Instance {
            gameplay_events: Vec::new(),
            id,
            entity: self.id,
            definition,
            parameters: parameters.clone(),
            pose,
            local: self.entity.local,
            dynamic_sources: self.entity.dynamic_sources.clone(),
            dynamic_retirement: self.entity.dynamic_retirement.clone(),
            entity_context: self.entity.entity_context,
            motion: self.entity.motion,
            age: 0.,
            head: u16::MAX,
            light_head: u16::MAX,
            sound_head: u16::MAX,
            decal_head: u16::MAX,
            distortion_head: u16::MAX,
            fov_head: u16::MAX,
            quad_head: u16::MAX,
            shake_head: u16::MAX,
            fade_head: u16::MAX,
            rumble_head: u16::MAX,
            environment_head: u16::MAX,
            sub_effects: Vec::new(),
            actor_observer: None,
            target,
        });
        self.state.handles.insert(id, slot);
        if let Some(definition) = contact_definition {
            use crate::gameplay::effects::contact_instances::Admission;
            match self.state.contact_instances.admission(&definition) {
                Admission::Exhausted => {
                    //60af50→166830 returns a warning, then60a8b0 destroys
                    //the actual unstarted instance1a31f0. No family/RNG start.
                    self.state.report(format!(
                        "ContactSystem:too many unique active contact definitions; ignored {name}"
                    ));
                    self.state.retire(slot, self.systems)?;
                    return Ok(None);
                }
                Admission::Available { oldest } => {
                    if let Some(oldest) = oldest {
                        self.state.stop(oldest, self.systems)?;
                    }
                    self.state.contact_instances.insert(definition, id)?;
                }
            }
        }
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
    /// 1404b2200's 1406dc560: the target entity still exists.
    fn target_alive(&mut self) -> Result<bool, String> {
        let target = self
            .entity
            .parameters
            .target
            .as_ref()
            .ok_or("world FX target absent")?;
        Ok(self.frames.resolve(target, "")?.is_some())
    }
    /// 1404b20b0: the target pose (1404b1880) into every record of the
    /// instance.
    fn update_target_pose(&mut self, handle: u32) -> Result<(), String> {
        let Some(target) = self.target_pose()? else {
            return Ok(());
        };
        let Some(&slot) = self.state.handles.get(&handle) else {
            return Ok(());
        };
        if let Some(instance) = self.state.slots[usize::from(slot)].as_mut() {
            instance.target = target;
        }
        Ok(())
    }
    fn request_stop(&mut self, handle: u32) -> Result<(), String> {
        self.state.stop(handle, self.systems)?;
        Ok(())
    }
    fn update_source_velocity(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn detach(&mut self, handle: Option<u32>) -> Result<Option<u32>, String> {
        let handle=handle.filter(|handle| self.state.handles.contains_key(handle));
        if handle.is_some(){
            if let Some(detached)=self.entity.dynamic_retirement.as_ref().and_then(|lifetime|lifetime.detached.as_ref()){
                detached.store(true,std::sync::atomic::Ordering::Release);
            }
        }
        Ok(handle)
    }
    fn stop_immediately(&mut self, handle: u32) -> Result<(), String> {
        if let Some(&slot) = self.state.handles.get(&handle) {
            self.state.retire(slot, self.systems)?;
        }
        Ok(())
    }
}

/// DH2_DEBUG_PLAY_FX_START=<seconds after complete rendering> (default 2).
fn debug_effect_start() -> f64 {
    static VALUE: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| std::env::var("DH2_DEBUG_PLAY_FX_START").ok().and_then(|value| value.parse().ok()).unwrap_or(2.0))
}
