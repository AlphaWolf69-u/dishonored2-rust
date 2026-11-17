//! Actual owned ranged-component dependencies, prepared before Fire.
//! Workers request the canonical runtime-entity prototype; no projectile
//! declaration, model or collision shape has a second cache here.
use super::*;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<PreparedOwned, String>>>,
}
struct PreparedOwned {
    index: usize,
    projectile: Option<Arc<Prepared>>,
    spread: [Option<Arc<crate::assets::explosions::Declaration>>; 3],
}
pub(super) struct Owned {
    pub(super) identity: Arc<str>,
    pub(super) entity: EntityId,
    settings: Arc<Ranged>,
    pub(super) descriptor: Arc<crate::assets::equipment::Descriptor>,
    pending: Option<Pending>,
    failure: Option<String>,
    pub(super) projectiles: Vec<Option<Arc<Prepared>>>,
    pub(super) spreads: Vec<[Option<Arc<crate::assets::explosions::Declaration>>; 3]>,
    reservation: Arc<str>,
    effect_paths: Vec<String>,
    pub(super) effects: Vec<Effect>,
    requested: bool,
    requested_projectiles: Vec<bool>,
    acquired_ammunition: Vec<bool>,
    variants: Vec<Vec<crate::assets::equipment::attachment::AmmoUpgrade>>,
    installed: bool,
    upgrades_installed: bool,
    pub(super) muzzle_overrides: Vec<Option<Arc<str>>>,
    pub(super) explosion_overrides: Vec<Option<Arc<str>>>,
    pub(super) ready: bool,
    retired: bool,
    stopping: bool,
}
pub(super) struct Effect {
    pub(super) key: Arc<str>,
    pub(super) ticket: super::super::usable_effects::EffectTicket,
    pub(super) attached: bool,
}
impl Owned {
    pub(super) fn is_retiring(&self) -> bool { self.retired }
}

pub(in crate::app) fn request_owned(
    world: &mut World,
    identity: Arc<str>,
    descriptor: Arc<crate::assets::equipment::Descriptor>,
) -> Result<(), String> {
    if !world.contains_resource::<Firing>() {
        world.insert_resource(Firing::default());
    }
    let settings = Arc::new(
        descriptor
            .ranged
            .as_ref()
            .ok_or("owned firing prototype is not ranged")?
            .clone(),
    );
    let entity = super::super::equipment_identity::id(world, &identity)
        .ok_or("owned ranged component has no managed entity")?;
    if world.resource::<Firing>().owned.contains_key(&entity) {
        return Ok(());
    }
    let mut owner = world.resource_mut::<Firing>();
    owner.generation = owner.generation.wrapping_add(1);
    let reservation = Arc::from(format!("weapon:{}:{}", owner.generation, identity));
    let mut effect_paths: Vec<String> = Vec::new();
    effect_paths.extend(settings.muzzle_attached_fx.iter().cloned());
    effect_paths.sort_unstable();
    effect_paths.dedup();
    owner.owned.insert(
        entity,
        Owned {
            identity,
            entity,
            projectiles: vec![None; settings.ammunition.len()],
            requested_projectiles: vec![false; settings.ammunition.len()],
            acquired_ammunition: vec![false; settings.ammunition.len()],
            spreads: vec![[None, None, None]; settings.ammunition.len()],
            variants: Vec::new(),
            settings,
            descriptor,
            pending: None,
            failure: None,
            reservation,
            effect_paths,
            effects: Vec::new(),
            requested: false,
            installed: false,
            upgrades_installed: false,
            muzzle_overrides: Vec::new(),
            explosion_overrides: Vec::new(),
            ready: false,
            retired: false,
            stopping: false,
        },
    );
    Ok(())
}

fn acquire(world: &World, entry: &mut Owned, index: usize) -> Result<(), String> {
    let catalog = world
        .get_resource::<super::super::SceneData>()
        .and_then(|scene| scene.catalog.clone())
        .ok_or("owned ranged component has no catalog lease")?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = cancelled.clone();
    let sources = entry.settings.clone();
    let overrides = entry.variants[index].clone();
    let task = IoTaskPool::get().spawn(async move {
        let cancelled = || cancellation.load(Ordering::Acquire);
        let ammo = &sources.ammunition[index];
        if cancelled() {
            return Err("ranged dependency acquisition cancelled".into());
        }
        let projectile = ammo
            .projectile
            .as_deref()
            .map(|path| {
                crate::assets::runtime_entity::Definition::acquire(&catalog, path, Some(&cancelled))
                    .and_then(|base| {
                        crate::assets::runtime_entity::weapon_variant(
                            &catalog,
                            base.clone(),
                            &overrides,
                            Some(&cancelled),
                        )
                        .and_then(|entity| Prepared::from_weapon_entity(&base, entity))
                    })
            })
            .transpose()?;
        let mut spread = [None, None, None];
        if sources.family == RangedFamily::Gun && ammo.spread_damage {
            //44e0c0 selects exactly one source declaration, with reduced
            //taking precedence over upgraded. Unused branches need no
            //concrete explosion query/effect admission.
            let flags = overrides
                .iter()
                .fold(0, |flags, record| flags | record.flags);
            let slot = if flags & 8 != 0 {
                1
            } else if flags & 0x10 != 0 {
                2
            } else {
                0
            };
            let path = [
                &sources.spread_area,
                &sources.spread_area_reduced,
                &sources.spread_area_upgraded,
            ][slot];
            if cancelled() {
                return Err("ranged spread acquisition cancelled".into());
            }
            spread[slot] = path
                .as_deref()
                .map(|path| crate::assets::explosions::Declaration::acquire(&catalog, path))
                .transpose()?;
        }
        Ok(PreparedOwned {
            index,
            projectile,
            spread,
        })
    });
    entry.pending = Some(Pending {
        cancelled,
        task: Mutex::new(task),
    });
    Ok(())
}

pub(in crate::app) fn remove_owned(world: &mut World, identity: &str) {
    if let Some(mut owner) = world.get_resource_mut::<Firing>() {
        for entry in owner
            .owned
            .values_mut()
            .filter(|entry| entry.identity.as_ref() == identity)
        {
            entry.retired = true;
            entry.ready = false;
            if let Some(pending) = &entry.pending {
                pending.cancelled.store(true, Ordering::Release);
            }
        }
    }
}

pub(super) fn advance(
    world: &mut World,
    owner: &mut Firing,
    player: &mut Player,
) -> Result<(), String> {
    for entry in owner.owned.values_mut() {
        if let Some(pending) = &mut entry.pending {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(result) = block_on(poll_once(task)) {
                entry.pending = None;
                if !entry.retired {
                    match result {
                        Ok(prepared) => {
                            entry.projectiles[prepared.index] = prepared.projectile;
                            entry.acquired_ammunition[prepared.index] = true;
                            entry.installed = false;
                            entry.spreads[prepared.index] = prepared.spread;
                        }
                        Err(error) => {
                            entry.failure = Some(error.clone());
                            return Err(error);
                        }
                    }
                }
            }
        }
        entry
            .effects
            .retain(|effect| !super::super::usable_effects::entity_retired(world, &effect.ticket));
        if entry.retired && !entry.stopping
            && !owner.deliveries.iter().any(|delivery| delivery.message.entity == Some(entry.entity)) {
            //44f2a0 stops the gun's retained barrel-smoke actors. Independent
            //world muzzle one-shots retain their ordinary finite tail.
            for effect in entry.effects.iter().filter(|effect| effect.attached) {
                world
                    .resource_mut::<super::super::usable_effects::EffectRequests>()
                    .stop_entity(effect.key.clone());
            }
            entry.stopping = true;
        }
        if entry.retired {
            continue;
        }
        if let Some(error) = &entry.failure {
            return Err(error.clone());
        }
        if !entry.upgrades_installed {
            let Some(upgrades) = super::super::equipment_attachment_admission::upgrade_definitions(
                world,
                entry.entity,
            ) else {
                continue;
            };
            let mut flags = Vec::with_capacity(entry.settings.ammunition.len());
            let mut variants = Vec::with_capacity(entry.settings.ammunition.len());
            for ammo in &entry.settings.ammunition {
                //638190 inserts component-declaration overrides at the head;
                //other matching records retain the authored upgrade order.
                let mut matching = Vec::new();
                let mut combined = 0;
                for upgrade in upgrades {
                    for record in upgrade
                        .ammunition
                        .iter()
                        .filter(|record| record.item == ammo.item)
                    {
                        combined |= record.flags;
                        if record.projectile.is_some() {
                            matching.insert(0, record);
                        } else {
                            matching.push(record);
                        }
                    }
                }
                let muzzle = matching
                    .iter()
                    .find_map(|record| record.muzzle_fx.as_deref())
                    .map(Arc::from);
                let explosion = matching
                    .iter()
                    .rev()
                    .find_map(|record| record.explosion_fx.as_deref())
                    .map(Arc::from);
                entry.muzzle_overrides.push(muzzle);
                entry.explosion_overrides.push(explosion);
                flags.push(combined);
                variants.push(matching.into_iter().cloned().collect());
            }
            entry.effect_paths.sort_unstable();
            entry.effect_paths.dedup();
            player.install_ranged_ammunition_flags(&entry.identity, flags)?;
            entry.variants = variants;
            entry.upgrades_installed = true;
        }
        let selected = player
            .ranged_ammunition_index(&entry.identity)
            .ok_or("owned ranged component lost its actual runtime")?;
        if !entry.acquired_ammunition[selected] && entry.pending.is_none() {
            //639f00 retains the authored ammo references but initializes its
            //current record only.63b5f0 constructs the selected actor on Fire;
            //unused Grenade/Mine families must not be instantiated at Start.
            //The selected branch always undergoes full canonical admission;
            //an unsupported actual component remains an explicit error.
            //Host demand preparation defers unused typed prototypes while
            //retaining all authored source references. Previously selected
            //leases survive with this real weapon to avoid switch-back IO;
            //they release together on actual component depletion.
            if let Some(path) = entry.settings.ammunition[selected].muzzle_world_fx.as_ref() {
                entry.effect_paths.push(path.clone());
            }
            if let Some(path) = &entry.muzzle_overrides[selected] {
                entry.effect_paths.push(path.to_string());
            }
            if let Some(path) = &entry.explosion_overrides[selected] {
                entry.effect_paths.push(path.to_string());
            }
            entry.effect_paths.sort_unstable();
            entry.effect_paths.dedup();
            acquire(world, entry, selected)?;
            entry.ready = false;
        }
        if entry.pending.is_some() {
            continue;
        }
        if !entry.requested_projectiles[selected] {
            //Each real owned weapon generation holds its own prototype scope.
            //A failed later request must still release every earlier scope.
            let index = selected;
            //request_for may acquire the parent before a dependent
            //chunk fails. Retain this scope even on that partial error.
            entry.requested_projectiles[index] = true;
            if let Some(projectile) = &entry.projectiles[index] {
                if let Err(error) = super::super::projectiles::prepare_for(
                    world,
                    Arc::from(format!("{}:ammo:{index}", entry.reservation)),
                    projectile,
                ) {
                    entry.failure = Some(error.clone());
                    return Err(error);
                }
            }
            for(slot,spread)in entry.spreads[selected].iter().enumerate().filter_map(|(slot,spread)|spread.as_ref().map(|spread|(slot,spread))){
                super::super::explosions::prepare_owned(world,Arc::from(format!("{}:spread:{selected}:{slot}",entry.reservation)),spread)?;
            }
            world
                .resource_mut::<super::super::usable_effects::EffectRequests>()
                .reserve(entry.reservation.clone(), entry.effect_paths.clone());
            entry.requested = true;
        }
        entry.ready = super::super::usable_effects::reservation_ready(world, &entry.reservation)?;
        for projectile in entry.projectiles[selected].iter() {
            entry.ready &= super::super::projectiles::prepared_ready(world, projectile)?;
        }
        for(slot,spread)in entry.spreads[selected].iter().enumerate().filter_map(|(slot,spread)|spread.as_ref().map(|spread|(slot,spread))){
            entry.ready &= super::super::explosions::ready_owned(world,&format!("{}:spread:{selected}:{slot}",entry.reservation),spread)?;
        }
        if entry.ready && !entry.installed {
            player.install_ranged_projectiles(&entry.identity, entry.projectiles.clone())?;
            entry.installed = true;
        }
    }
    let mut retirement_error = None;
    let deliveries = &owner.deliveries;
    owner.owned.retain(|_, entry| {
        if !entry.retired || entry.pending.is_some() || !entry.effects.is_empty() {
            return true;
        }
        //Only an actual removal scans accepted callbacks. Keep both the
        //prototype leases and source identity while a retained Fire can still
        //publish a projectile or an attached/world muzzle actor.
        if deliveries.iter().any(|delivery| delivery.message.entity == Some(entry.entity)) {
            return true;
        }
        if entry.requested {
            world
                .resource_mut::<super::super::usable_effects::EffectRequests>()
                .release_reservation(entry.reservation.clone());
        }
        if let Err(error) = release_projectiles(world, entry) {
            retirement_error = Some(error);
            return true;
        }
        false
    });
    retirement_error.map_or(Ok(()), Err)
}

fn release_projectiles(world: &mut World, entry: &mut Owned) -> Result<(), String> {
    for index in 0..entry.requested_projectiles.len() {
        if !entry.requested_projectiles[index] {
            continue;
        }
        if let Some(projectile) = &entry.projectiles[index] {
            super::super::projectiles::release_prepared_for(
                world,
                &format!("{}:ammo:{index}", entry.reservation),
                projectile,
            )?;
        }
        for slot in 0..entry.spreads[index].len(){
            super::super::explosions::release_owned(world,&format!("{}:spread:{index}:{slot}",entry.reservation))?;
        }
        entry.requested_projectiles[index] = false;
    }
    Ok(())
}

pub(in crate::app) fn startup_pending(world: &World) -> bool {
    world.get_resource::<Firing>().is_some_and(|owner| {
        owner
            .owned
            .values()
            .any(|entry| !entry.retired && !entry.ready)
    })
}
/// Closing cancels workers but retains them until their archive reads return.
/// Actual projectile instances and effect tails retire with their own owners.
pub(in crate::app) fn shutdown(world: &mut World) -> Result<bool, String> {
    //A retained C8/weapon callback owns its actual component until its
    //projectile/spread/muzzle forwarding acknowledges. The early producer
    //fence completes this before the shared FX/source admission closes.
    if !super::finish_accepted(world)? { return Ok(false); }
    let Some(mut owner) = world.remove_resource::<Firing>() else {
        return Ok(true);
    };
    let mut retirement_error = None;
    for entry in owner.owned.values_mut() {
        entry.retired = true;
        entry.ready = false;
        if let Err(error) = release_projectiles(world, entry) {
            retirement_error = Some(error);
            break;
        }
        if let Some(pending) = &mut entry.pending {
            pending.cancelled.store(true, Ordering::Release);
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            if block_on(poll_once(task)).is_some() {
                entry.pending = None;
            }
        }
        if entry.requested {
            world
                .resource_mut::<super::super::usable_effects::EffectRequests>()
                .release_reservation(entry.reservation.clone());
            entry.requested = false;
        }
        if !entry.stopping {
            for effect in &entry.effects {
                world
                    .resource_mut::<super::super::usable_effects::EffectRequests>()
                    .stop_entity(effect.key.clone());
            }
            entry.stopping = true;
        }
        entry
            .effects
            .retain(|effect| !super::super::usable_effects::entity_retired(world, &effect.ticket));
    }
    if let Some(error) = retirement_error {
        world.insert_resource(owner);
        return Err(error);
    }
    owner
        .owned
        .retain(|_, entry| entry.pending.is_some() || !entry.effects.is_empty());
    let done = owner.owned.is_empty();
    if !done {
        world.insert_resource(owner);
    }
    Ok(done)
}
