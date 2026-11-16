//! Actual owned ranged-component dependencies, prepared before selection.
//! Workers request the canonical runtime-entity prototype; no projectile
//! declaration, model or collision shape has a second cache here.
use super::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once};
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};

struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<PreparedOwned, String>>>,
}
struct PreparedOwned { projectiles: Vec<Option<Arc<Prepared>>>, spread: [Option<Arc<crate::assets::explosions::Declaration>>; 3] }
pub(super) struct Owned {
    pub(super) identity: Arc<str>,
    pub(super) entity: EntityId,
    settings: Arc<Ranged>,
    pub(super) descriptor: Arc<crate::assets::equipment::Descriptor>,
    pending: Option<Pending>,
    pub(super) projectiles: Vec<Option<Arc<Prepared>>>,
    pub(super) spread: [Option<Arc<crate::assets::explosions::Declaration>>; 3],
    reservation: Arc<str>,
    effect_paths: Vec<String>,
    pub(super) effects: Vec<Effect>,
    requested: bool,
    installed: bool,
    upgrades_installed: bool,
    pub(super) muzzle_overrides: Vec<Option<Arc<str>>>,
    pub(super) explosion_overrides: Vec<Option<Arc<str>>>,
    pub(super) ready: bool,
    retired: bool,
    stopping: bool,
}
pub(super) struct Effect { pub(super) key: Arc<str>, pub(super) ticket: super::super::usable_effects::EffectTicket, pub(super) attached: bool }

pub(in crate::app) fn request_owned(world: &mut World, identity: Arc<str>, descriptor: Arc<crate::assets::equipment::Descriptor>) -> Result<(), String> {
    if !world.contains_resource::<Firing>() { world.insert_resource(Firing::default()); }
    let settings = Arc::new(descriptor.ranged.as_ref().ok_or("owned firing prototype is not ranged")?.clone());
    let entity = super::super::equipment_identity::id(world, &identity).ok_or("owned ranged component has no managed entity")?;
    if world.resource::<Firing>().owned.contains_key(&entity) { return Ok(()); }
    let catalog = world.get_resource::<super::super::SceneData>().and_then(|scene| scene.catalog.clone())
        .ok_or("owned ranged component has no catalog lease")?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = cancelled.clone();
    let sources = settings.clone();
    let task = IoTaskPool::get().spawn(async move {
        let cancelled = || cancellation.load(Ordering::Acquire);
        let mut prepared = Vec::with_capacity(sources.ammunition.len());
        for ammo in &sources.ammunition {
            if cancelled() { return Err("ranged dependency acquisition cancelled".into()); }
            prepared.push(ammo.projectile.as_deref().map(|path|
                crate::assets::runtime_entity::Definition::acquire(&catalog, path, Some(&cancelled))
                    .and_then(Prepared::from_entity)).transpose()?);
        }
        let mut spread = [None, None, None];
        for (slot, path) in spread.iter_mut().zip([&sources.spread_area, &sources.spread_area_reduced, &sources.spread_area_upgraded]) {
            if cancelled() { return Err("ranged spread acquisition cancelled".into()); }
            *slot = path.as_deref().map(|path| crate::assets::explosions::Declaration::acquire(&catalog, path)).transpose()?;
        }
        Ok(PreparedOwned { projectiles: prepared, spread })
    });
    let mut owner = world.resource_mut::<Firing>();
    owner.generation = owner.generation.wrapping_add(1);
    let reservation = Arc::from(format!("weapon:{}:{}", owner.generation, identity));
    let mut effect_paths: Vec<String> = settings.ammunition.iter().filter_map(|ammo| ammo.muzzle_world_fx.clone()).collect();
    effect_paths.extend(settings.muzzle_attached_fx.iter().cloned());
    effect_paths.sort_unstable(); effect_paths.dedup();
    owner.owned.insert(entity, Owned { identity, entity, settings, descriptor,
        pending: Some(Pending { cancelled, task: Mutex::new(task) }), projectiles: Vec::new(), spread: [None, None, None], reservation,
        effect_paths, effects: Vec::new(), requested: false, installed: false, upgrades_installed: false,
        muzzle_overrides: Vec::new(), explosion_overrides: Vec::new(), ready: false, retired: false, stopping: false });
    Ok(())
}

pub(in crate::app) fn remove_owned(world: &mut World, identity: &str) {
    if let Some(mut owner) = world.get_resource_mut::<Firing>() {
        for entry in owner.owned.values_mut().filter(|entry| entry.identity.as_ref() == identity) {
            entry.retired = true; entry.ready = false;
            if let Some(pending) = &entry.pending { pending.cancelled.store(true, Ordering::Release); }
        }
    }
}

pub(super) fn advance(world: &mut World, owner: &mut Firing, player: &mut Player) -> Result<(), String> {
    for entry in owner.owned.values_mut() {
        if let Some(pending) = &mut entry.pending {
            let task = pending.task.get_mut().unwrap_or_else(|error| error.into_inner());
            if let Some(result) = block_on(poll_once(task)) {
                entry.pending = None;
                if !entry.retired { let prepared = result?; entry.projectiles = prepared.projectiles; entry.spread = prepared.spread; }
            }
        }
        entry.effects.retain(|effect| !super::super::usable_effects::entity_retired(world, &effect.ticket));
        if entry.retired && !entry.stopping {
            //44f2a0 stops the gun's retained barrel-smoke actors. Independent
            //world muzzle one-shots retain their ordinary finite tail.
            for effect in entry.effects.iter().filter(|effect| effect.attached) {
                world.resource_mut::<super::super::usable_effects::EffectRequests>().stop_entity(effect.key.clone());
            }
            entry.stopping = true;
        }
        if entry.retired || entry.pending.is_some() { continue; }
        if !entry.upgrades_installed {
            let Some(upgrades) = super::super::equipment_attachment_admission::upgrade_definitions(world, entry.entity) else { continue };
            let mut flags = Vec::with_capacity(entry.settings.ammunition.len());
            for ammo in &entry.settings.ammunition {
                //638190 inserts component-declaration overrides at the head;
                //other matching records retain the authored upgrade order.
                let mut matching = Vec::new();
                let mut combined = 0;
                for upgrade in upgrades {
                    for record in upgrade.ammunition.iter().filter(|record| record.item == ammo.item) {
                        combined |= record.flags;
                        if record.projectile.is_some() { matching.insert(0, record); } else { matching.push(record); }
                    }
                }
                let muzzle = matching.iter().find_map(|record| record.muzzle_fx.as_deref()).map(Arc::from);
                let explosion = matching.iter().rev().find_map(|record| record.explosion_fx.as_deref()).map(Arc::from);
                if let Some(muzzle) = &muzzle { entry.effect_paths.push(muzzle.to_string()); }
                if let Some(explosion) = &explosion { entry.effect_paths.push(explosion.to_string()); }
                entry.muzzle_overrides.push(muzzle);
                entry.explosion_overrides.push(explosion);
                flags.push(combined);
            }
            entry.effect_paths.sort_unstable(); entry.effect_paths.dedup();
            player.install_ranged_ammunition_flags(&entry.identity, flags)?;
            entry.upgrades_installed = true;
        }
        if !entry.requested {
            for projectile in entry.projectiles.iter().flatten() { super::super::projectiles::prepare(world, projectile)?; }
            for spread in entry.spread.iter().flatten() { super::super::explosions::prepare(world, spread)?; }
            world.resource_mut::<super::super::usable_effects::EffectRequests>()
                .reserve(entry.reservation.clone(), entry.effect_paths.clone());
            entry.requested = true;
        }
        entry.ready = super::super::usable_effects::reservation_ready(world, &entry.reservation)?;
        for projectile in entry.projectiles.iter().flatten() {
            entry.ready &= super::super::projectiles::prepared_ready(world, projectile)?;
        }
        for spread in entry.spread.iter().flatten() { entry.ready &= super::super::explosions::ready(world, spread)?; }
        if entry.ready && !entry.installed {
            player.install_ranged_projectiles(&entry.identity, entry.projectiles.clone())?;
            entry.installed = true;
        }
    }
    owner.owned.retain(|_, entry| {
        if !entry.retired || entry.pending.is_some() || !entry.effects.is_empty() { return true; }
        if entry.requested {
            world.resource_mut::<super::super::usable_effects::EffectRequests>().release_reservation(entry.reservation.clone());
        }
        false
    });
    Ok(())
}

pub(in crate::app) fn startup_pending(world: &World) -> bool {
    world.get_resource::<Firing>().is_some_and(|owner| owner.owned.values().any(|entry| !entry.retired && !entry.ready))
}
/// Closing cancels workers but retains them until their archive reads return.
/// Actual projectile instances and effect tails retire with their own owners.
pub(in crate::app) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.remove_resource::<Firing>() else { return Ok(true) };
    for entry in owner.owned.values_mut() {
        entry.retired = true; entry.ready = false;
        if let Some(pending) = &mut entry.pending {
            pending.cancelled.store(true, Ordering::Release);
            let task = pending.task.get_mut().unwrap_or_else(|error| error.into_inner());
            if block_on(poll_once(task)).is_some() { entry.pending = None; }
        }
        if entry.requested {
            world.resource_mut::<super::super::usable_effects::EffectRequests>().release_reservation(entry.reservation.clone());
            entry.requested = false;
        }
        if !entry.stopping {
            for effect in &entry.effects { world.resource_mut::<super::super::usable_effects::EffectRequests>().stop_entity(effect.key.clone()); }
            entry.stopping = true;
        }
        entry.effects.retain(|effect| !super::super::usable_effects::entity_retired(world, &effect.ticket));
    }
    owner.owned.retain(|_, entry| entry.pending.is_some() || !entry.effects.is_empty());
    let done = owner.owned.is_empty();
    if !done { world.insert_resource(owner); }
    Ok(done)
}
