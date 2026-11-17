//! Main-thread consumers of the source FX manager gameplay-event FIFO.
//! Canonical entity/explosion owners retain preparation and spawned lifetimes.
use crate::{
    assets::{explosions::Declaration, runtime_entity::Definition},
    gameplay::effects::gameplay::Request,
    rendering::{
        bevy_backend::camera_scene::FxSimulation,
        particles::world_sources::{WorldEntityDef, WorldExplosion, WorldFxSources},
    },
};
use bevy::prelude::*;
use std::{
    collections::HashMap,
    sync::{Arc, Weak, atomic::Ordering},
};
enum Source {
    Entity(Weak<WorldEntityDef>),
    Explosion(Weak<WorldExplosion>),
}
impl Source {
    fn ready(&self, ready: bool) {
        match self {
            Self::Entity(source) => {
                if let Some(source) = source.upgrade() {
                    source.prepared.store(ready, Ordering::Release);
                }
            }
            Self::Explosion(source) => {
                if let Some(source) = source.upgrade() {
                    source.prepared.store(ready, Ordering::Release);
                }
            }
        }
    }
}
struct Entry {
    ready: bool,
    source: Source,
    lifetime: Weak<()>,
    scope: Arc<str>,
    entities: Vec<Arc<Definition>>,
    explosion: Option<Arc<Declaration>>,
    requested: usize,
    attempted: usize,
    explosion_requested: bool,
    explosion_attempted: bool,
}
#[derive(Resource, Default)]
pub(super) struct FxGameplay {
    entries: HashMap<(u8, usize), Entry>,
    snapshot: Weak<WorldFxSources>,
    generation: u64,
    closed: bool,
}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<FxGameplay>() {
        return Err("FX gameplay owner already exists before level publication".into());
    }
    world.insert_resource(FxGameplay::default());
    Ok(())
}
fn release(world: &mut World, entry: &mut Entry) -> Result<(), String> {
    while entry.attempted != 0 {
        let index = entry.attempted - 1;
        super::runtime_entities::release_for(world, &entry.scope, &entry.entities[index])?;
        entry.attempted -= 1;
    }
    entry.requested = 0;
    if entry.explosion_attempted {
        super::explosions::release_owned(world, &entry.scope)?;
        entry.explosion_requested = false;
        entry.explosion_attempted = false;
    }
    Ok(())
}
/// Only a changed immutable FX source snapshot is walked. Weak source records
/// keep no FX alive; actual instance/sub-FX tails retain the defining groups.
pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    if let Some(error)=super::fx_lights::error(world) { return Err(format!("FX light publication: {error}")); }
    let Some(mut owner) = world.remove_resource::<FxGameplay>() else {
        return Ok(());
    };
    let result = (|| {
        let mut failure = None;
        owner.entries.retain(|_, entry| {
            if entry.lifetime.strong_count() != 0 {
                true
            } else {
                match release(world, entry) {
                    Ok(()) => false,
                    Err(error) => {
                        failure = Some(error);
                        true
                    }
                }
            }
        });
        if let Some(error) = failure {
            return Err(error);
        }
        if owner.closed {
            return Ok(());
        }
        let sources = world
            .resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>()
            .0
            .clone();
        if !owner
            .snapshot
            .upgrade()
            .is_some_and(|previous| Arc::ptr_eq(&previous, &sources))
        {
            //The combined renderer snapshot deduplicates effect names. The
            //actual source bundles can contain separate equal-named gameplay
            //records, each of which must acquire and acknowledge its own
            //canonical source lease. Entry keys still deduplicate shared Arcs.
            let additions=super::usable_effects::gameplay_sources(world);
            for group in std::iter::once(&sources).chain(additions.iter())
                .flat_map(|sources| sources
                .effects
                .values()
                .filter_map(|effect| effect.as_ref().ok())
                .flat_map(|effect| &effect.groups))
            {
                for source in &group.entity_defs {
                    let key = (0, Arc::as_ptr(source) as usize);
                    if owner.entries.contains_key(&key) {
                        continue;
                    }
                    owner.generation = owner.generation.wrapping_add(1);
                    owner.entries.insert(
                        key,
                        Entry {
                            ready: false,
                            source: Source::Entity(Arc::downgrade(source)),
                            lifetime: Arc::downgrade(&source.lifetime),
                            scope: Arc::from(format!("fx-entity-source:{}", owner.generation)),
                            entities: source.declarations.iter().flatten().cloned().collect(),
                            explosion: None,
                            requested: 0,
                            attempted: 0,
                            explosion_requested: false,
                            explosion_attempted: false,
                        },
                    );
                }
                for source in &group.explosions {
                    let key = (1, Arc::as_ptr(source) as usize);
                    if owner.entries.contains_key(&key) {
                        continue;
                    }
                    owner.generation = owner.generation.wrapping_add(1);
                    owner.entries.insert(
                        key,
                        Entry {
                            ready: false,
                            source: Source::Explosion(Arc::downgrade(source)),
                            lifetime: Arc::downgrade(&source.lifetime),
                            scope: Arc::from(format!("fx-explosion-source:{}", owner.generation)),
                            entities: Vec::new(),
                            explosion: Some(source.declaration.clone()),
                            requested: 0,
                            attempted: 0,
                            explosion_requested: false,
                            explosion_attempted: false,
                        },
                    );
                }
            }
            owner.snapshot = Arc::downgrade(&sources);
        }
        for entry in owner.entries.values_mut() {
            //Demand owns all canonical leases after the initial publication;
            //unchanged ready entries need no per-frame resource-manager polls.
            if entry.ready {
                continue;
            }
            while entry.requested < entry.entities.len() {
                let index = entry.requested;
                entry.attempted = index + 1;
                super::runtime_entities::request_for(
                    world,
                    entry.scope.clone(),
                    entry.entities[index].clone(),
                )?;
                entry.requested += 1;
            }
            if !entry.explosion_requested {
                if let Some(declaration) = &entry.explosion {
                    entry.explosion_attempted = true;
                    super::explosions::prepare_owned(world, entry.scope.clone(), declaration)?;
                    entry.explosion_requested = true;
                }
            }
            let mut ready = true;
            for definition in &entry.entities {
                ready &= super::runtime_entities::ready(world, definition)?;
            }
            if let Some(declaration) = &entry.explosion {
                ready &= super::explosions::ready_owned(world, &entry.scope, declaration)?;
            }
            if ready {
                entry.source.ready(true);
                entry.ready = true;
            }
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result
}
pub(super) fn startup_pending(world: &World) -> bool {
    world
        .get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectSources>()
        .is_some_and(|sources| !sources.0.gameplay_ready())
}
///Print actual pending graph nodes once, with scope names as edges rather
///than recursively expanding them. Cyclic source graphs remain diagnosable.
pub(super) fn debug_pending(world:&World)->String {
    use std::fmt::Write;
    let Some(owner)=world.get_resource::<FxGameplay>()else{return "FX gameplay owner=absent".into();};
    let mut report=format!("FX gameplay nodes={} closed={} generation={}",owner.entries.len(),owner.closed,owner.generation);
    for (key,entry) in owner.entries.iter().filter(|(_,entry)|!entry.ready) {
        let _=write!(report,"\n    source={key:?} scope={} alive={} entities={}/{} attempted={} explosion_requested={} explosion_attempted={}",
            entry.scope,entry.lifetime.strong_count()!=0,entry.requested,entry.entities.len(),entry.attempted,
            entry.explosion_requested,entry.explosion_attempted);
        for definition in &entry.entities {
            let _=write!(report,"\n      entity={} ready={:?}",definition.entity,super::runtime_entities::ready(world,definition));
        }
        if let Some(declaration)=&entry.explosion {
            let _=write!(report,"\n      explosion_ready={:?}\n      {}",super::explosions::ready_owned(world,&entry.scope,declaration),
                super::explosions::debug_owned(world,&entry.scope));
        }
    }
    report
}
/// The actual FX queue is retained until canonical acceptance succeeds.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if world
        .get_resource::<FxGameplay>()
        .is_none_or(|owner| owner.closed)
    {
        return Ok(());
    }
    while let Some(request) = world
        .get_resource::<FxSimulation>()
        .and_then(|simulation| simulation.pending_gameplay())
        .cloned()
    {
        match request {
            Request::Entity {
                definition, frame, ..
            } => {
                if !super::runtime_entities::ready(world, &definition)? {
                    return Ok(());
                }
                super::runtime_entities::spawn(
                    world,
                    definition,
                    super::runtime_entities::SpawnOptions {
                        frame,
                        scale: None,
                        hidden_disabled: false,
                    },
                )?;
            }
            Request::Explosion {
                request,
                source_lease,
            } => super::explosions::enqueue_with_lease(world, request, source_lease)?,
        }
        world
            .resource_mut::<FxSimulation>()
            .acknowledge_gameplay()?;
    }
    Ok(())
}
pub(super) fn begin_retirement(world: &mut World) {
    if let Some(mut owner) = world.get_resource_mut::<FxGameplay>() {
        owner.closed = true;
    }
    if let Some(mut simulation) = world.get_resource_mut::<FxSimulation>() {
        simulation.clear_gameplay();
    }
}
/// Invoke after the FX simulation has acknowledged all tails, before closing
/// canonical runtime-entity acquisition. Source declarations may still reside.
pub(super) fn retire(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<FxGameplay>() else {
        return Ok(());
    };
    owner.closed = true;
    let result = (|| {
        for entry in owner.entries.values_mut() {
            release(world, entry)?;
        }
        Ok(())
    })();
    if result.is_err() {
        world.insert_resource(owner);
    }
    result
}

/// Consume the queues written by 219370/21a730 after this frame's FX update.
/// The earlier post-physics explosion phase remains in place for weapon and
/// contact requests. Both visits use its sole acceptance cursor; no accepted
/// damage is replayed, and ordinary entity removal still has one frame batch.
pub(super) fn consume_after_simulation(world: &mut World) -> Result<(), String> {
    update(world)?;
    super::explosions::update(world)
}
