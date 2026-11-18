//! Canonical per-view environment FX projection and88e220 publication. Source
//! acquisition, timing and pool admission remain with their original owners.
use crate::{
    rendering::{
        bevy_backend::{
            camera_scene::FxSimulation,
            environment_images::{EnvironmentImages, Projection as ImageProjection},
        },
        environment_effect_runtime::ENVIRONMENT_CAPACITY,
        environment_frame::Frame,
        environment_overrides::EnvironmentOverrides,
        environment_resources::{PreparedEnvironment, PreparedValue},
    },
};
use bevy::{
    prelude::*,
    tasks::{block_on, futures_lite::future::poll_once, IoTaskPool, Task},
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};
const IMAGES: [&str; 3] = [
    "env/tonemapping/colorcube",
    "env/bloom/lensdirt",
    "atm/worlddistancefog/colorlut",
];
fn identity(environment: &PreparedEnvironment) -> usize {
    std::ptr::from_ref(environment) as usize
}
struct Projection {
    environment: Weak<PreparedEnvironment>,
    images: [Option<Handle<Image>>; 3],
    lifetime: Arc<AtomicBool>,
}
impl Projection {
    fn withdraw(&self) {
        self.lifetime.store(false, Ordering::Release);
        if let Some(env) = self.environment.upgrade() {
            env.prepared.store(false, Ordering::Release);
        }
    }
}
impl Drop for Projection {
    fn drop(&mut self) { self.withdraw(); }
}

struct Prepared {
    images: super::resident_images::PreparedImages,
    projections: Vec<(usize, Projection)>,
}
struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Prepared, String>>>,
}
#[derive(Clone, Copy)]
enum Producer {
    World,
    Pause,
    Focus,
}
#[derive(Resource)]
pub(super) struct EnvironmentEffects {
    pub frame: Frame,
    pub revision: u64,
    view: Option<Entity>,
    slots: EnvironmentOverrides<Arc<PreparedEnvironment>>,
    orders: Vec<(u64, Producer)>,
    projections: HashMap<usize, Projection>,
    /// Structural view-row release schedules one sweep, not per-frame Weak scans.
    prune_dirty: bool,
    /// Withdrawn projections keep their actual handles until render receipt ACK.
    retiring_projections: Vec<Projection>,
    /// Coalesced actual receipt withdrawal, independent of unrelated GPU uploads.
    retiring_withdrawal: Option<u64>,
    pending: Option<Pending>,
    error: Option<String>,
    closed: bool,
    pause_bound: bool,
    seen_world: Weak<crate::rendering::particles::world_sources::WorldFxSources>,
    seen_pause: bool,
    seen_focus: Option<usize>,
    withdrawal: Option<u64>,
}
impl Default for EnvironmentEffects {
    fn default() -> Self {
        Self {
            frame: Frame::default(),
            revision: 0,
            view: None,
            slots: Default::default(),
            orders: Vec::with_capacity(ENVIRONMENT_CAPACITY),
            projections: HashMap::new(),
            prune_dirty: false,
            retiring_projections: Vec::new(),
            retiring_withdrawal: None,
            pending: None,
            error: None,
            closed: false,
            pause_bound: false,
            seen_world: Weak::new(),
            seen_pause: false,
            seen_focus: None,
            withdrawal: None,
        }
    }
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<EnvironmentEffects>();
    crate::rendering::bevy_backend::environment_images::register(app);
}
/// A new level explicitly reopens its existing session owner, after the old
/// scene's worker, native slots and extracted image receipts have retired.
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    let previous = world.get_resource::<EnvironmentEffects>()
        .ok_or("environment consumer was not registered before scene setup")?;
    if previous.pending.is_some() || !previous.projections.is_empty() || !previous.retiring_projections.is_empty()
        || !previous.slots.is_empty() || !previous.frame.rows().is_empty() {
        return Err("previous environment scene is still owned".into());
    }
    if let Some(generation) = previous.withdrawal {
        if world.resource::<EnvironmentImages>().completed.load(Ordering::Acquire) != generation {
            return Err("previous environment image withdrawal is not acknowledged".into());
        }
    }
    world.insert_resource(EnvironmentEffects::default());
    let pool = world.get_resource::<FxSimulation>()
        .ok_or("environment initialization requires the published FX manager")?.environment_pool();
    if let Some(mut pause) = world.get_resource_mut::<super::pause_fx::PauseScreenFx>() {
        pause.initialize_environment_pool(pool.clone())?;
    }
    if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
        if let Some(focus) = state.player.as_mut().and_then(|p| p.combat_fx.as_mut()) {
            focus.initialize_environment_pool(pool)?;
        }
    }
    Ok(())
}

fn focus_identity(world: &World) -> Option<usize> {
    world.get_resource::<super::player::PlayerState>()
        .and_then(|s| s.player.as_ref()).and_then(|p| p.combat_fx.as_ref())
        .map(|focus| focus.environment_identity())
}
/// Cheap readonly run condition: an idle scene never enters the exclusive
/// preparation barrier or repeats metadata walks/image acquisitions.
pub(super) fn needs_prepare(world: &World) -> bool {
    let Some(owner) = world.get_resource::<EnvironmentEffects>() else { return false };
    if owner.pending.is_some() || !owner.retiring_projections.is_empty() { return true; }
    if owner.closed || owner.error.is_some() { return false; }
    let changed = world.get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>()
        .is_some_and(|sources| !owner.seen_world.ptr_eq(&Arc::downgrade(&sources.0)));
    owner.prune_dirty || changed || (!owner.seen_pause && world.contains_resource::<super::pause_fx::PauseScreenFx>())
        || owner.seen_focus != focus_identity(world)
}

fn publish_image_receipt(world: &mut World, owner: &EnvironmentEffects) {
    let mut receipt = world.resource_mut::<EnvironmentImages>();
    receipt.generation = receipt.generation.wrapping_add(1).max(1);
    receipt.projections = Arc::new(owner.projections.values().map(|p| ImageProjection {
        environment: p.environment.clone(),
        images: p.images.iter().flatten().map(Handle::id).collect(),
        lifetime: Arc::downgrade(&p.lifetime),
    }).collect());
}
impl EnvironmentEffects {
    pub fn image(
        &self,
        environment: &PreparedEnvironment,
        parameter: &str,
    ) -> Result<Handle<Image>, String> {
        let role = IMAGES
            .iter()
            .position(|name| *name == parameter)
            .ok_or("environment resource is not an88e220 special image")?;
        self.projections
            .get(&identity(environment))
            .and_then(|p| p.images[role].clone())
            .ok_or_else(|| {
                format!(
                    "environment {} image {parameter} is not prepared",
                    environment.name
                )
            })
    }
}
pub(super) fn startup_pending(world: &World) -> bool {
    let mut unprepared = false;
    if let Some(sources) =
        world.get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>()
    {
        unprepared = sources
            .0
            .environments()
            .any(|env| !env.prepared.load(Ordering::Acquire));
    }
    if let Some(pause) = world.get_resource::<super::pause_fx::PauseScreenFx>() {
        pause.visit_environment_sources(|env| unprepared |= !env.prepared.load(Ordering::Acquire));
    }
    if let Some(focus) = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|s| s.player.as_ref())
        .and_then(|p| p.combat_fx.as_ref())
    {
        focus.visit_environment_sources(|env| unprepared |= !env.prepared.load(Ordering::Acquire));
    }
    if unprepared {
        return true;
    }
    world
        .get_resource::<EnvironmentEffects>()
        .is_some_and(|owner| {
            owner.pending.is_some()
                || (!owner.closed
                    && owner.projections.values().any(|p| {
                        p.environment
                            .upgrade()
                            .is_some_and(|env| !env.prepared.load(Ordering::Acquire))
                    }))
        })
}
pub(super) fn startup_error(world: &World) -> Option<String> {
    world
        .get_resource::<EnvironmentEffects>()
        .and_then(|owner| owner.error.clone())
        .or_else(|| {
            world
                .get_resource::<super::pause_fx::PauseScreenFx>()
                .and_then(|fx| fx.environment_error().map(str::to_owned))
        })
}

/// Ungated metadata admission sees WorldEffectSources BEFORE gameplay_ready
/// can start the FX. Existing IO workers, ResidentImages and asset batches own
/// all reads/uploads; no per-frame archive/declaration/image work occurs.
pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<EnvironmentEffects>() else {
        return Ok(());
    };
    let result = (|| -> Result<(), String> {
        if let Some(generation) = owner.retiring_withdrawal {
            // Actual render-world observation is sufficient for removing this
            // projection alias. Fresh unrelated images need not be GPU-ready.
            if world.resource::<EnvironmentImages>().withdrawal_observed(generation) {
                owner.retiring_projections.clear();
                owner.retiring_withdrawal = None;
            }
        }
        if let Some(pending) = &mut owner.pending {
            if let Some(result) = block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "environment projection worker poisoned")?,
            )) {
                owner.pending = None;
                if owner.closed {
                    // Closed acquisitions are joined solely to release their
                    // canonical transaction; cancelled work is not a failure.
                    drop(result);
                    return Ok(());
                }
                let prepared = result?;
                if !owner.closed {
                    prepared
                        .images
                        .publish(&mut world.resource_mut::<Assets<Image>>());
                    owner.projections.extend(prepared.projections);
                    publish_image_receipt(world, &owner);
                }
            }
        }
        if owner.closed {
            return Ok(());
        }
        if let Some(error) = &owner.error {
            return Err(error.clone());
        }
        if owner.pending.is_some() {
            return Ok(());
        }
        let sources = world
            .get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>();
        let world_changed =
            sources.is_some_and(|sources| !owner.seen_world.ptr_eq(&Arc::downgrade(&sources.0)));
        let pause_changed =
            !owner.seen_pause && world.contains_resource::<super::pause_fx::PauseScreenFx>();
        let player_identity = focus_identity(world);
        let player_changed = owner.seen_focus != player_identity;
        let demand_changed = world_changed || pause_changed || player_changed;
        if !demand_changed && !owner.prune_dirty {
            return Ok(());
        }
        if let Some(sources) = sources {
            owner.seen_world = Arc::downgrade(&sources.0);
        }
        owner.seen_pause |= pause_changed;
        owner.seen_focus = player_identity;
        owner.prune_dirty = false;
        let released: Vec<_> = owner.projections.iter()
            .filter(|(_, projection)| projection.environment.strong_count() == 0)
            .map(|(&id, _)| id).collect();
        if !released.is_empty() {
            for id in released {
                let projection = owner.projections.remove(&id).expect("selected live projection");
                projection.withdraw();
                owner.retiring_projections.push(projection);
            }
            // Exclude withdrawn rows now; their handle payloads remain owned
            // until the render owner acknowledges this or a newer receipt.
            publish_image_receipt(world, &owner);
            owner.retiring_withdrawal = Some(world.resource::<EnvironmentImages>().generation);
        }
        if !demand_changed { return Ok(()); }
        let mut needed = HashMap::<usize, Arc<PreparedEnvironment>>::new();
        if let Some(sources) = world
            .get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>(
        ) {
            for env in sources.0.environments() {
                if !owner.projections.contains_key(&identity(env)) {
                    needed.insert(identity(env), env.clone());
                }
            }
        }
        if let Some(pause) = world.get_resource::<super::pause_fx::PauseScreenFx>() {
            pause.visit_environment_sources(|env| {
                if !owner.projections.contains_key(&identity(env)) {
                    needed.insert(identity(env), env.clone());
                }
            });
        }
        if let Some(focus) = world
            .get_resource::<super::player::PlayerState>()
            .and_then(|state| state.player.as_ref())
            .and_then(|p| p.combat_fx.as_ref())
        {
            focus.visit_environment_sources(|env| {
                if !owner.projections.contains_key(&identity(env)) {
                    needed.insert(identity(env), env.clone());
                }
            });
        }
        if needed.is_empty() {
            return Ok(());
        }
        let catalog = world
            .resource::<super::SceneData>()
            .catalog
            .clone()
            .ok_or("environment projection catalog absent")?;
        let mut images = super::resident_images::PreparedImages::new(
            world.resource::<Assets<Image>>(),
            world
                .resource::<super::resident_images::ResidentImages>()
                .clone(),
            world
                .resource::<super::texture_streaming::SharedImages>()
                .clone(),
        );
        images.begin_acquisition(Some(&catalog));
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let task = IoTaskPool::get().spawn(async move {
            let mut projections = Vec::with_capacity(needed.len());
            let resident = images.registry().consumer();
            for (id, env) in needed {
                if stop.load(Ordering::Acquire) {
                    return Err("environment projection preparation cancelled".into());
                }
                let mut handles = [None, None, None];
                for (role, parameter) in IMAGES.iter().enumerate() {
                    if let Some(PreparedValue::Image(source)) = env.values.get(*parameter) {
                        handles[role] = Some(if role == 0 {
                            resident.colour_cube(&catalog, source, images.assets())?
                        } else {
                            super::texture_streaming::load_resident_2d(
                                &catalog,
                                source,
                                images.assets(),
                                &resident,
                            )?
                        });
                    }
                }
                projections.push((
                    id,
                    Projection {
                        environment: Arc::downgrade(&env),
                        images: handles,
                        lifetime: Arc::new(AtomicBool::new(true)),
                    },
                ));
            }
            Ok(Prepared {
                images,
                projections,
            })
        });
        owner.pending = Some(Pending {
            cancelled,
            task: Mutex::new(task),
        });
        Ok(())
    })();
    if let Err(error) = &result {
        if !owner.closed {
            owner.error = Some(error.clone());
        }
    }
    world.insert_resource(owner);
    result
}

/// Readonly admission condition: unchanged frames never enter the binding
/// system's mutable Player/Pause/consumer access. No pool lock or Arc clone.
pub(super) fn needs_bind_legacy_pools(
    simulation: Option<Res<FxSimulation>>,
    pause: Option<Res<super::pause_fx::PauseScreenFx>>,
    state: Option<Res<super::player::PlayerState>>,
    effects: Res<EnvironmentEffects>,
) -> bool {
    if effects.closed || simulation.is_none() {
        return false;
    }
    (pause.is_some() && !effects.pause_bound)
        || state.as_ref().and_then(|state| state.player.as_ref()).and_then(|p| p.combat_fx.as_ref())
            .is_some_and(|focus| !focus.environment_pool_bound())
}

/// Bind the SAME native64-slot allocator before either legacy controller can
/// start. There is no independently constructed permit counter or pool.
pub(super) fn bind_legacy_pools(
    simulation: Option<Res<FxSimulation>>,
    mut pause: Option<ResMut<super::pause_fx::PauseScreenFx>>,
    mut state: ResMut<super::player::PlayerState>,
    mut effects: ResMut<EnvironmentEffects>,
) {
    if effects.closed {
        return;
    }
    let focus_needs_pool = state.player.as_ref().and_then(|p| p.combat_fx.as_ref())
        .is_some_and(|focus| !focus.environment_pool_bound());
    if effects.pause_bound && !focus_needs_pool {
        return;
    }
    let Some(simulation) = simulation else { return };
    let pool = simulation.environment_pool();
    let result = (|| -> Result<(), String> {
        if let Some(pause) = pause.as_mut().filter(|_| !effects.pause_bound) {
            pause.set_environment_pool(pool.clone())?;
            effects.pause_bound = true;
        }
        if let Some(player) = state.player.as_mut().filter(|_| focus_needs_pool) {
            if let Some(focus) = player.combat_fx.as_mut() {
                focus.set_environment_pool(pool)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        effects.error = Some(error);
    }
}
pub(super) fn publish(
    mut owner: ResMut<EnvironmentEffects>,
    simulation: Option<ResMut<FxSimulation>>,
    mut pause: Option<ResMut<super::pause_fx::PauseScreenFx>>,
    mut player: ResMut<super::player::PlayerState>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    if owner.withdrawal.is_some() {
        return;
    }
    let Some(mut simulation) = simulation else {
        return;
    };
    let result = (|| -> Result<(), String> {
        let view = cameras
            .single()
            .map_err(|_| "environment requires one actual gameplay camera")?;
        if owner.view.is_some_and(|old| old != view) {
            return Err("environment view destruction/rebinding producer is not admitted while effects are live".into());
        }
        owner.view = Some(view);
        let view = view.to_bits();
        let EnvironmentEffects {
            slots,
            orders,
            frame,
            revision,
            prune_dirty,
            ..
        } = &mut *owner;
        let registration = (|| -> Result<(), String> {
        simulation.flush_environment_retirements(view, slots)?;
        if let Some(pause) = pause.as_mut() {
            pause.flush_environment_retirements(view, slots);
        }
        let mut focus = player.player.as_mut().and_then(|p| p.combat_fx.as_mut());
        if let Some(focus) = focus.as_mut() {
            focus.flush_environment_retirements(view, slots);
        }
        orders.clear();
        simulation.visit_environment_orders(1, |order| orders.push((order, Producer::World)))?;
        if let Some(pause) = pause.as_ref() {
            pause.visit_environment_orders(1, |order| orders.push((order, Producer::Pause)));
        }
        if let Some(focus) = focus.as_ref() {
            focus.visit_environment_orders(1, |order| orders.push((order, Producer::Focus)));
        }
        if orders.len() > ENVIRONMENT_CAPACITY {
            return Err("environment producers exceed the actual shared64-slot family".into());
        }
        orders.sort_unstable_by_key(|(order, _)| *order);
        for &(order, producer) in orders.iter() {
            match producer {
                Producer::World => simulation.publish_environment(order, view, 1, slots)?,
                Producer::Pause => pause
                    .as_mut()
                    .ok_or("pause environment producer disappeared")?
                    .publish_environment(order, view, 1, slots)?,
                Producer::Focus => focus
                    .as_mut()
                    .ok_or("focused environment producer disappeared")?
                    .publish_environment(order, view, 1, slots)?,
            }
        }
        Ok(())
        })();
        // A real invariant failure after partial publication must still run
        // 8911a0/88e220's zero-weight retirement visit. Normal full16 is NULL
        // plus retry in the binding, and is not an error here.
        let mut count = 0;
        let mut changed = false;
        let mut failure = None;
        slots.visit_frame(|env, weight, influence| {
            // A replaced/truncated Frame row can release the last ENV Arc only
            // AFTER the earlier source-generation sweep. Wake one later prune.
            if frame.rows().get(count).is_some_and(|row| !Arc::ptr_eq(&row.environment, env)) {
                *prune_dirty = true;
            }
            match frame.update_slot(count, env, weight, influence) {
                Ok(updated) => changed |= updated,
                Err(error) => failure = Some(error),
            }
            count += 1;
        });
        if let Some(error) = failure {
            return Err(error);
        }
        let removed_rows = frame.finish_slots(count);
        *prune_dirty |= removed_rows;
        changed |= removed_rows;
        if changed {
            *revision = revision.wrapping_add(1);
        }
        registration
    })();
    if let Err(error) = result {
        owner.error = Some(error);
    }
}

/// Stop accepting before the canonical FX manager starts its full drain.
/// Legacy leases must be cleared first because its nativepool includes them.
pub(super) fn begin_retirement(world: &mut World) -> Result<(), String> {
    if let Some(mut owner) = world.get_resource_mut::<EnvironmentEffects>() {
        owner.closed = true;
        if let Some(pending) = &owner.pending {
            pending.cancelled.store(true, Ordering::Release);
        }
    }
    if let Some(mut pause) = world.get_resource_mut::<super::pause_fx::PauseScreenFx>() {
        pause.clear_environments()?;
    }
    if let Some(focus) = world
        .get_resource_mut::<super::player::PlayerState>()
        .and_then(|mut state| {
            state
                .player
                .as_mut()
                .and_then(|p| p.combat_fx.as_mut())
                .map(|fx| fx.clear_environments())
        })
    {
        focus?;
    }
    Ok(())
}
/// Called after native FX/override tails and the render publication have
/// retired, before the scene/image owners are purged. Join every accepted job.
pub(super) fn retire(world: &mut World) -> Result<bool, String> {
    begin_retirement(world)?;
    // A cancelled acquisition is joined and its AssetBatch dropped. It cannot
    // publish a late generation, resurrect residency, or recreate this owner.
    prepare(world)?;
    if world.resource::<EnvironmentEffects>().pending.is_some() {
        return Ok(false);
    }
    let mut owner = world.remove_resource::<EnvironmentEffects>()
        .ok_or("environment retirement owner disappeared")?;
    let result = (|| -> Result<bool, String> {
        if owner.withdrawal.is_none() {
            let flush = (|| -> Result<(), String> {
            if let Some(view) = owner.view.map(Entity::to_bits) {
                world.get_resource_mut::<FxSimulation>()
                    .ok_or("environment manager removed before view retirements")?
                    .flush_environment_retirements(view, &mut owner.slots)?;
                if let Some(mut pause) = world.get_resource_mut::<super::pause_fx::PauseScreenFx>() {
                    pause.flush_environment_retirements(view, &mut owner.slots);
                }
                if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
                    if let Some(focus) = state.player.as_mut().and_then(|p| p.combat_fx.as_mut()) {
                        focus.flush_environment_retirements(view, &mut owner.slots);
                    }
                }
            }
            Ok(())
            })();
            let mut count = 0;
            let mut failure = None;
            owner.slots.visit_frame(|env, weight, influence| {
                if let Err(error) = owner.frame.update_slot(count, env, weight, influence) {
                    failure = Some(error);
                }
                count += 1;
            });
            owner.frame.finish_slots(count);
            if let Some(error) = failure { return Err(error); }
            flush?;
            if !owner.slots.is_empty() {
                return Err("environment view still has live producers after FX shutdown".into());
            }
            owner.frame.clear();
            owner.revision = owner.revision.wrapping_add(1);
            // Withdraw the extracted receipt first. Projection handles stay
            // alive until the render owner has observed the empty generation.
            let mut images = world.resource_mut::<EnvironmentImages>();
            images.generation = images.generation.wrapping_add(1).max(1);
            images.projections = Arc::default();
            owner.withdrawal = Some(images.generation);
            return Ok(false);
        }
        let images = world.resource::<EnvironmentImages>();
        if images.completed.load(Ordering::Acquire) != owner.withdrawal.unwrap() {
            return Ok(false);
        }
        owner.projections.clear();
        owner.retiring_projections.clear();
        owner.retiring_withdrawal = None;
        owner.prune_dirty = false;
        Ok(true)
    })();
    world.insert_resource(owner);
    result
}
