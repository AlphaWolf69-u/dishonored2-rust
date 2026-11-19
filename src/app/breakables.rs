//! Concrete Breakable journal bridge to the shared entity/physics/render/FX owners.
//!7ed000/7ee0e0/7ee610; callbacks acknowledge only after the real operation succeeds.
use crate::gameplay::{
    breakable::Effect,
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
};
use bevy::prelude::*;
#[path = "destruction_timing.rs"]
mod timing;
pub(super) use timing::{measure, Owner as TimingOwner};

#[derive(Resource, Default)]
pub(super) struct Breakables {
    tails: Vec<Tail>,
}
//Host A/B controls are read once, never from the per-actor polling path.
//Counters describe logical resource transfers, not compiler memcpy count or
//the heap memory owned by the resource. ETW remains the CPU-time measurement.
struct DebugBorrowing {
    legacy_scene_scope: bool,
    count_transfers: bool,
    scene_scopes: std::sync::atomic::AtomicU64,
    live_scopes: std::sync::atomic::AtomicU64,
}
fn debug_borrowing() -> &'static DebugBorrowing {
    static CONTROL: std::sync::OnceLock<DebugBorrowing> = std::sync::OnceLock::new();
    CONTROL.get_or_init(|| DebugBorrowing {
        legacy_scene_scope: std::env::var("DH2_DEBUG_BREAKABLE_LEGACY_SCOPE").as_deref() == Ok("1"),
        count_transfers: std::env::var("DH2_DEBUG_BREAKABLE_BORROW_COST").as_deref() == Ok("1"),
        scene_scopes: std::sync::atomic::AtomicU64::new(0),
        live_scopes: std::sync::atomic::AtomicU64::new(0),
    })
}
impl Drop for Breakables {
    fn drop(&mut self) {
        let control = debug_borrowing();
        if !control.count_transfers { return; }
        use std::sync::atomic::Ordering;
        let scene = control.scene_scopes.load(Ordering::Relaxed);
        let live = control.live_scopes.load(Ordering::Relaxed);
        let scene_size = std::mem::size_of::<super::physics_scene::PhysicsScene>() as u64;
        let live_size = std::mem::size_of::<LiveEntities>() as u64;
        eprintln!("BREAKABLE_BORROW_COST mode={} calls={} scene_scopes={} live_scopes={} scene_size={} live_size={} logical_scene_transfer_bytes={} logical_live_transfer_bytes={}",
            if control.legacy_scene_scope { "scene" } else { "live" }, scene + live,
            scene, live, scene_size, live_size, scene.saturating_mul(scene_size).saturating_mul(2),
            live.saturating_mul(live_size).saturating_mul(2));
    }
}
struct Tail {
    source: EntityId,
    component: crate::gameplay::pickable_component::CheckedComponent,
    step: usize,
    owner: EntityId,
    ticket: super::usable_effects::EffectTicket,
    remove_owner: bool,
}
/// Borrow only this damaged source generation's real destruction requests.
/// Retained child FX targets remain attributable to their originating component.
pub(super) fn visit_tails(world: &World, source: EntityId,
    mut visit: impl FnMut(crate::gameplay::pickable_component::CheckedComponent, usize, &super::usable_effects::EffectTicket)) {
    if let Some(owner) = world.get_resource::<Breakables>() {
        for tail in owner.tails.iter().filter(|tail| tail.source == source) {
            visit(tail.component, tail.step, &tail.ticket);
        }
    }
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Breakables>();
}

pub(super) fn write_diagnostics(world:&World,eye:[f32;3],report:&mut String) {
    use std::fmt::Write;
    if let Some(scene)=world.get_resource::<super::physics_scene::PhysicsScene>() {
        scene.write_breakable_diagnostics(eye,report);
    }
    if let Some(owner)=world.get_resource::<Breakables>() {
        let _=writeln!(report,"  Breakable destruction FX tails={} (actual tickets, at most32):",owner.tails.len());
        for tail in owner.tails.iter().take(32) {
            let _=writeln!(report,"    owner={:?} remove_owner={} {}",tail.owner,tail.remove_owner,super::usable_effects::debug_ticket(world,&tail.ticket));
        }
    }
}

///All source operations use actual owners. The source front remains queued
///on an error; successful operations are acknowledged before the next one.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<super::physics_scene::PhysicsScene>() {
        return Ok(());
    }
    if !world.contains_resource::<Breakables>() {
        return Err("Breakable effect consumer absent".into());
    }
    world.resource_scope(|world, mut owner: Mut<Breakables>| {
        let mut at = 0;
        while at < owner.tails.len() {
            if !super::usable_effects::effect_completed(&owner.tails[at].ticket) { at += 1; continue; }
            let tail = &owner.tails[at];
            if tail.remove_owner && world.resource::<LiveEntities>().get(tail.owner).is_some() {
                super::entity_functions::enqueue_remove_or_ghost(world, tail.owner, 0.)?;
            }
            owner.tails.swap_remove(at);
        }
        while let Some((handle, generation, effect)) = world.resource::<super::physics_scene::PhysicsScene>()
            .pending_breakable_effect().map(|(handle, generation, effect)| (handle, generation, effect.clone())) {
            let phase = match &effect {
                Effect::HideAndDetachChildren { .. } => "breakable_hide_detach",
                Effect::StopOwnerRigid { .. } => "breakable_stop_rigid",
                Effect::SpawnFx { .. } => "breakable_request_fx",
                Effect::ReleaseChunk { .. } => "breakable_release_chunk_dispatch",
                Effect::ChunkImpulse { .. } => "breakable_chunk_impulse",
                Effect::KeepBindInfo { .. } => "breakable_keep_binding",
                Effect::Damage { .. } => "breakable_delayed_damage",
                Effect::SoundOcclusion { .. } => "breakable_sound_occlusion",
                Effect::BrokenCallback { .. } => "breakable_graph_callback",
                Effect::BrokenMessage { .. } => "breakable_message",
                Effect::Remove { .. } | Effect::RemoveOrGhost { .. } => "breakable_remove_request",
            };
            let _timing = measure(phase, TimingOwner::Component { handle, generation });
            match effect {
                Effect::HideAndDetachChildren { owner: id, culprit } => {
                    //7ed000 snapshots actual bound children. Each unbind removes
                    //it from this same roster; a failed callback resumes there.
                    world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                        loop {
                            let Some(child) = live.children(id)?.first().copied() else { break; };
                            world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                                child_instigator(world, &mut scene, &live, child, culprit)?;
                                let mut events = world.resource_mut::<EntityEvents>();
                                scene.unbind_entity(&mut live, child, &mut events)
                            })?;
                        }
                        super::model_motion::set_entity_visible(world, &live, id, false)?;
                        world.resource_mut::<super::physics_scene::PhysicsScene>().breakable_hidden(&live, id, true)
                    })?;
                }
                Effect::StopOwnerRigid { owner: id } => {
                    world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                        world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                            scene.breakable_stop_rigid(&mut live, &mut world.resource_mut::<EntityEvents>(), id)
                        })
                    })?;
                }
                Effect::Remove { entity, delay } => super::entity_functions::enqueue_removal(world, entity, delay)?,
                Effect::RemoveOrGhost { entity, delay } => super::entity_functions::enqueue_remove_or_ghost(world, entity, delay)?,
                Effect::Damage { entity, delay, params } => super::entity_functions::enqueue_damage(world, entity, params, delay)?,
                Effect::ReleaseChunk { owner: id, entity, culprit } => {
                    world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                        world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                            world.resource_scope(|world, mut events: Mut<EntityEvents>| {
                                scene.breakable_release_chunk(world, &mut live, &mut events, id, entity)
                            })
                        })?;
                        super::time_sync::synchronize_entity_time(world, &live, id, entity)?;
                        //7ee0e0 installs player provenance after87caa0 has
                        //reenabled and restarted the actual prebuilt actor.
                        world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                            child_instigator(world, &mut scene, &live, entity, culprit)
                        })?;
                        super::model_motion::set_entity_visible(world, &live, entity, true)
                    })?;
                    super::projected_decals::copy_parent_projection_contexts(world, id, entity)?;
                }
                Effect::ChunkImpulse { entity, point, direction, scale, falloff } => {
                    //7ee0e0 queries878230 only for positive falloff and never
                    //posts FA for a chunk beyond that radius (zero impulses
                    //would still alter activation/publication state).
                    let center = if falloff > 0. {
                        match super::runtime_entities::bounds(world, entity)? {
                            Some(bounds) => (bevy_math::Vec3::from_array(bounds[0]) + bevy_math::Vec3::from_array(bounds[1])) * 0.5,
                            None => bevy_math::Vec3::from_array(world.resource::<super::physics_scene::PhysicsScene>()
                                .published_entity_frame(world.resource::<LiveEntities>(), entity)?.pose.position),
                        }
                    } else { bevy_math::Vec3::ZERO };
                    if falloff <= 0. || 1. - (point - center).length() / falloff > 0. {
                        let impulse = crate::gameplay::breakable::chunk_impulse(center, point, direction, scale, falloff);
                        let mut bytes = Vec::with_capacity(24);
                        for value in impulse.to_array().into_iter().chain(point.to_array()) { bytes.extend_from_slice(&value.to_le_bytes()); }
                        world.resource_mut::<EntityEvents>().post(Event { id: 0xfa, target: Some(entity), payload: Payload::Bytes(bytes) });
                    }
                }
                Effect::KeepBindInfo { owner: id, entity } => {
                    world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                        world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                            scene.breakable_keep_binding(&mut live, &mut world.resource_mut::<EntityEvents>(), id, entity)
                        })
                    })?;
                }
                Effect::SpawnFx { component, target, step, chunk, fx, params,
                    cannot_incapacitate_player_and_allies, hold_owner } => {
                    let path = fx.declaration.as_deref().ok_or("Breakable FX effect declaration absent")?;
                    let live = world.resource::<LiveEntities>();
                    let target_name = live.get(target).ok_or("Breakable FX target generation stale")?.0.to_owned();
                    let frame = world.resource::<super::physics_scene::PhysicsScene>().published_entity_frame(live, target)?;
                    let is_player = params.attacker.is_some_and(|id| world.get_resource::<super::player::PlayerState>()
                        .and_then(|state| state.player.as_ref()).is_some_and(|player| player.managed_entity() == Some(id)));
                    let context = crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                        owner: (component == target).then_some(target), culprit: (component == target).then_some(params.attacker).flatten(),
                        player_impact: component == target && hold_owner && is_player,
                        cannot_incapacitate_player_and_allies: component == target && cannot_incapacitate_player_and_allies,
                        velocities: frame.velocities,
                        contact_speed:0.,contact_mass:0.,contact_stimulus_enabled:None,
                    };
                    let key = std::sync::Arc::<str>::from(format!("breakable:{component:?}:{generation}:{step}:{chunk:?}"));
                    let ticket = world.resource_mut::<super::usable_effects::EffectRequests>().start_context_once(key, path,
                        super::usable_effects::EntityPlacement { target: crate::gameplay::event_graph::EntityReference::Named(target_name),
                            tag: String::new(), local: fx.local, attached: fx.attached },
                        crate::gameplay::reference_frame::compose(frame.pose, fx.local), context);
                    owner.tails.push(Tail { source: component,
                        component: crate::gameplay::pickable_component::CheckedComponent { handle, generation },
                        step, owner: target, ticket, remove_owner: hold_owner });
                }
                Effect::SoundOcclusion { owner: id, strength } => {
                    let acoustic=world.get_resource::<super::audio::level::LevelAudio>().map(|level|level.acoustic_world());
                    let mut acoustic=acoustic.as_ref().map(|owner|owner.lock().map_err(|_|"Breakable acoustic world poisoned")).transpose()?;
                    world.resource_mut::<super::physics_scene::PhysicsScene>().breakable_sound_occlusion(id, strength,
                        acoustic.as_deref_mut())?;
                }
                Effect::BrokenCallback { owner: id, step, will_be_removed } => {
                    let frame = world.resource::<super::physics_scene::PhysicsScene>().published_entity_frame(world.resource::<LiveEntities>(), id)?;
                    //The same manager owns source graph subscriptions and queues.
                    //Absence is permitted only when no graph manager was admitted.
                    if world.contains_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>() {
                        world.resource_scope(|world, mut effects: Mut<crate::rendering::bevy_backend::camera_scene::FxSimulation>| {
                            let errors = effects.dispatch_broken(id, step as i32, will_be_removed, frame.pose.position, world.resource::<LiveEntities>())?;
                            if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
                        })?;
                    }
                }
                Effect::BrokenMessage { owner: id } => world.resource_mut::<EntityEvents>().post(Event { id: 0xa1, target: None, payload: Payload::Entity(id) }),
            }
            world.resource_mut::<super::physics_scene::PhysicsScene>().acknowledge_breakable_effect(handle, generation)?;
        }
        Ok(())
    })
}

fn child_instigator(
    world: &mut World,
    scene: &mut super::physics_scene::PhysicsScene,
    live: &LiveEntities,
    child: EntityId,
    culprit: Option<EntityId>,
) -> Result<(), String> {
    let player = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| player.managed_entity());
    let Some(culprit) = culprit.filter(|id| Some(*id) == player) else {
        return Ok(());
    };
    if !scene.breakable_has_rigid(child)? {
        return Ok(());
    }
    let mut clock = world.resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>();
    //Placed prebuilt chunks use the same lazy874c50 clock admission as other
    //placed actors.80e9c0 synchronization intentionally does nothing outside
    //bent time, so it cannot be the prerequisite for7278e0's timestamp read.
    super::time_sync::register_entity_time(scene,live,&mut clock,child)?;
    scene.set_rigid_instigator(
        live,
        child,
        culprit,
        clock.entity_time(child)?,
        clock.entity_delta(child)?,
    )
}

///7ed9a0/7ecc90/7ecc00: ordinary remove receives retained chunks synchronously,
///then the array relinquishes them. GameLocal owns their asynchronous teardown.
pub(super) fn begin_entity_retirement(world: &mut World, id: EntityId) -> Result<(), String> {
    //Source destruction waits for this owner's journal. A global dispatcher
    //sweep for every unrelated actor repeats a whole-scene scan during exit.
    //Keep its existing ordering/acknowledgements whenever actual work exists.
    if world.resource::<super::physics_scene::PhysicsScene>()
        .entity_has_pending_breakable_effect(id) {
        update(world)?;
    }
    let children = world
        .resource_mut::<super::physics_scene::PhysicsScene>()
        .begin_breakable_retirement(id)?;
    for child in children {
        if world.resource::<LiveEntities>().get(child).is_some() {
            world
                .resource_mut::<LiveEntities>()
                .request_removal(child)?;
        }
        world
            .resource_mut::<super::physics_scene::PhysicsScene>()
            .acknowledge_breakable_chunk_removal(id, child)?;
    }
    Ok(())
}
pub(super) fn entity_retired(world: &mut World, id: EntityId) -> Result<bool, String> {
    let control = debug_borrowing();
    if control.legacy_scene_scope {
        if control.count_transfers {
            control.scene_scopes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        return world.resource_scope(
            |world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                scene.breakable_retirement_complete(world.resource::<LiveEntities>(), id)
            },
        );
    }
    if control.count_transfers {
        control.live_scopes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    //7ed9a0/7ecc90 borrows its manager; it does not copy the world. Scope the
    //small identity owner so Scene stays in its canonical ECS resource slot.
    world.resource_scope(|world, live: Mut<LiveEntities>| {
        world.resource_mut::<super::physics_scene::PhysicsScene>()
            .breakable_retirement_complete(&live, id)
    })
}

///The shared FX shutdown owns all actual particle/sound tails. This producer
///releases its weak completion callbacks and never schedules a stale owner
///removal into the next level's entity-event clock.
pub(super) fn begin_retirement(world: &mut World) {
    if let Some(mut owner) = world.get_resource_mut::<Breakables>() {
        owner.tails.clear();
    }
}

///7ed000 executes within accepted message delivery in the game. The host
///must finish its retained journal while all operation owners are still live,
///before closing timers or starting component destruction. FX tails already
///transferred to the canonical effect owner drain with that owner's shutdown.
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    if !world.contains_resource::<super::physics_scene::PhysicsScene>() {
        return Ok(true);
    }
    update(world)?;
    Ok(world
        .resource::<super::physics_scene::PhysicsScene>()
        .pending_breakable_effect()
        .is_none())
}
