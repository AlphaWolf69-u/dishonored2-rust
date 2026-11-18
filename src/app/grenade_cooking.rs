//! Held-grenade HUD45ba30/45af50/45bd60 over the canonical runtime entity.
//! The Grenade owns its fuse; this owner keeps only model/movie source leases.
use crate::{
    assets::{explosions::Declaration, grenade_cooking::Definition},
    gameplay::entity_lifetime::EntityId,
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Pending<T> {
    cancel: Arc<AtomicBool>,
    task: Mutex<Task<Result<T, String>>>,
}
impl<T: Send + 'static> Pending<T> {
    fn poll(&mut self) -> Result<Option<T>, String> {
        let result = block_on(poll_once(
            self.task
                .get_mut()
                .map_err(|_| "grenade HUD worker poisoned")?,
        ));
        match result {
            Some(Ok(value)) => Ok(Some(value)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }
}
struct Lead {
    event_seconds: f32,
    explosion: Option<Arc<Declaration>>,
    enabled: bool,
}
struct WeaponLead {
    key: (EntityId, usize),
    definition: Arc<crate::assets::grenades::Definition>,
    pending: Option<Pending<Lead>>,
    value: Option<Lead>,
}
#[derive(Resource, Default)]
struct Hud {
    scene: Option<usize>,
    scope: Option<Arc<str>>,
    initialized: bool,
    closed: bool,
    pending: Option<Pending<Option<Definition>>>,
    definition: Option<Definition>,
    requested: bool,
    entity: Option<EntityId>,
    active: Option<(EntityId, EntityId)>,
    lead: Option<WeaponLead>,
    error: Option<String>,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Hud>();
}
pub(super) fn startup_pending(world: &World) -> bool {
    world.get_resource::<Hud>().is_some_and(|hud| {
        !hud.closed && (!hud.initialized || hud.definition.is_some() && hud.entity.is_none())
    })
}
pub(super) fn startup_error(world: &World) -> Option<&str> {
    world
        .get_resource::<Hud>()
        .and_then(|hud| hud.error.as_deref())
}

///Read-only receipt for the opt-in production-input diagnostic. `captured`
///comes from the renderer's real world-movie acknowledgement.
pub(super) fn debug_gauge(
    world: &mut World,
) -> Result<Option<(EntityId, bool, bool, Option<EntityId>)>, String> {
    let Some(entity) = world.get_resource::<Hud>().and_then(|hud| hud.entity) else {
        return Ok(None);
    };
    let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(None);
    };
    let mut position = 0;
    while let Some(key) = scene.flash_update_key(position)? {
        position += 1;
        if key.entity == entity {
            let parent = world
                .resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .parent(entity)?;
            return Ok(super::flash_surface_movie::status(world, key)?
                .map(|status| (entity, status.configured, status.captured, parent)));
        }
    }
    Ok(None)
}

///Small read-only preparation receipt for the opt-in action diagnostic.
pub(super) fn debug_lead(world: &World) -> Option<(EntityId, usize, bool, bool)> {
    let lead = world.get_resource::<Hud>()?.lead.as_ref()?;
    Some((lead.key.0, lead.key.1, lead.pending.is_some(), lead.value.is_some()))
}

pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(());
    };
    let scene = scene.grenade_cooking_owner();
    let Some(mut hud) = world.remove_resource::<Hud>() else {
        return Ok(());
    };
    let result = (|| {
        if hud.scene != Some(scene) {
            if hud.entity.is_some() || hud.pending.is_some() || hud.requested {
                return Err("new grenade HUD Scene preceded old owner retirement".into());
            }
            hud = Hud {
                scene: Some(scene),
                scope: Some(Arc::from(format!("hud:grenade-cooking:{scene}"))),
                ..Default::default()
            };
        }
        if hud.closed {
            return Ok(());
        }
        if let Some(error) = &hud.error {
            return Err(error.clone());
        }
        if !hud.initialized && hud.pending.is_none() {
            let catalog = world
                .resource::<super::SceneData>()
                .catalog
                .clone()
                .ok_or("grenade HUD catalog absent")?;
            let cancel = Arc::new(AtomicBool::new(false));
            let cancellation = cancel.clone();
            let task = IoTaskPool::get().spawn(async move {
                Definition::acquire(&catalog, Some(&|| cancellation.load(Ordering::Acquire)))
            });
            hud.pending = Some(Pending {
                cancel,
                task: Mutex::new(task),
            });
        }
        if let Some(pending) = &mut hud.pending {
            if let Some(value) = pending.poll()? {
                hud.definition = value;
                hud.pending = None;
                hud.initialized = true;
            }
        }
        let Some(definition) = &hud.definition else {
            return Ok(());
        };
        if !hud.requested {
            super::runtime_entities::request_scaled_for(
                world,
                hud.scope
                    .clone()
                    .ok_or("grenade HUD resource scope absent")?,
                definition.entity.clone(),
                definition.scale,
            )?;
            hud.requested = true;
        }
        if hud.entity.is_none()
            && super::runtime_entities::ready_scaled(world, &definition.entity, definition.scale)?
        {
            hud.entity = Some(super::runtime_entities::spawn(
                world,
                definition.entity.clone(),
                super::runtime_entities::SpawnOptions {
                    frame: Default::default(),
                    scale: Some(definition.scale),
                    hidden_disabled: true,
                },
            )?);
        }
        prepare_lead(world, &mut hud)?;
        Ok::<_, String>(())
    })();
    if let Err(error) = &result {
        hud.error = Some(error.clone());
    }
    world.insert_resource(hud);
    result
}
fn prepare_lead(world: &World, hud: &mut Hud) -> Result<(), String> {
    let Some(player) = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return retire_lead(hud);
    };
    let Some(weapon) = player.throwable.as_ref() else {
        return retire_lead(hud);
    };
    let Some((identity, _, _)) = player.equipment_action_status(0) else {
        return Ok(());
    };
    let Some(metadata) = world
        .get_resource::<super::player_actions::PlayerActions>()
        .and_then(|actions| actions.equipment_metadata(identity))
    else {
        return Ok(());
    };
    let Some(scene_owner) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(());
    };
    let Some(live) = world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>() else {
        return Ok(());
    };
    let Some(grenade) = scene_owner.grenade_cooking_definition(live, weapon.entity)? else {
        return Ok(());
    };
    let key = (weapon.entity, weapon.ammunition_index);
    if hud
        .lead
        .as_ref()
        .is_some_and(|lead| lead.key == key && Arc::ptr_eq(&lead.definition, &grenade))
    {
        let lead = hud.lead.as_mut().unwrap();
        if let Some(pending) = &mut lead.pending {
            if let Some(value) = pending.poll()? {
                lead.value = Some(value);
                lead.pending = None;
            }
        }
        return Ok(());
    }
    let fire = metadata.equipment.scenes.get("m_fireMidnight");
    //A real scene which has not been prepared yet is pending, not an absent
    //native optional pointer (which takes45b550's literal-one branch).
    let scene = fire
        .and_then(|path| player.animation_scene_asset(path))
        .cloned();
    if fire.is_some() && scene.is_none() {
        return Ok(());
    }
    if let Some(old) = &mut hud.lead {
        if let Some(pending) = &mut old.pending {
            pending.cancel.store(true, Ordering::Release);
            if block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "grenade HUD lead worker poisoned")?,
            ))
            .is_none()
            {
                return Ok(());
            }
            old.pending = None;
        }
    }
    let catalog = world
        .resource::<super::SceneData>()
        .catalog
        .clone()
        .ok_or("grenade HUD lead catalog absent")?;
    let effect = grenade.explosion_fx.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancellation = cancel.clone();
    let task = IoTaskPool::get().spawn(async move {
        let Some((effect, scene)) = effect.zip(scene) else {
            return Ok(Lead {
                event_seconds: 0.,
                explosion: None,
                enabled: false,
            });
        };
        let cancelled = || cancellation.load(Ordering::Acquire);
        let effects = crate::assets::fx::declaration::acquire(
            &catalog,
            &effect,
            crate::assets::fx::definition::Consumer::Common,
            Some(&cancelled),
        )?;
        //196fc0/1954a0/195770 select the first scene's PlayerLeftEquipment
        //event track and first C8; an absent event returns signed -1ms.
        let event = scene
            .decl
            .scenes
            .first()
            .and_then(|scene_data| {
                scene_data
                    .groups
                    .iter()
                    .find(|group| scene.decl.group_name(group.group) == Some("PlayerLeftEquipment"))
            })
            .and_then(|group| {
                group.events.iter().find(|event| {
                    event.duration_ms.is_none() && event.event == "ARK_MDNT_MSG_WEAPON_FIRE"
                })
            })
            .map_or(-1, |event| event.time_ms);
        let mut explosion = None;
        //45b550 overwrites its radius for each group, stopping at the first
        //non-NULL explosion in that group; disabled flags are not tested here.
        for group in &effects.groups {
            if let Some(path) = group
                .asset
                .explosions
                .iter()
                .find_map(|entry| entry.explosion.as_deref())
            {
                if cancelled() {
                    return Err("grenade HUD lead preparation cancelled".into());
                }
                explosion = Some(Declaration::acquire(&catalog, path)?);
            }
        }
        Ok(Lead {
            event_seconds: event as f32 * 0.001,
            explosion,
            enabled: true,
        })
    });
    hud.lead = Some(WeaponLead {
        key,
        definition: grenade,
        pending: Some(Pending {
            cancel,
            task: Mutex::new(task),
        }),
        value: None,
    });
    Ok(())
}
fn retire_lead(hud: &mut Hud) -> Result<(), String> {
    if let Some(lead) = &mut hud.lead {
        if let Some(pending) = &mut lead.pending {
            pending.cancel.store(true, Ordering::Release);
            if block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "grenade HUD lead worker poisoned")?,
            ))
            .is_none()
            {
                return Ok(());
            }
        }
    }
    hud.lead = None;
    Ok(())
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    let Some(mut hud) = world.remove_resource::<Hud>() else {
        return Ok(());
    };
    let result = update_hud(world, &mut hud);
    if let Err(error) = &result {
        hud.error = Some(error.clone());
    }
    world.insert_resource(hud);
    result
}
fn update_hud(world: &mut World, hud: &mut Hud) -> Result<(), String> {
    if hud.closed {
        return Ok(());
    }
    let Some(entity) = hud.entity else {
        return Ok(());
    };
    if hud.active.is_none()
        && world
            .get_resource::<super::player::PlayerState>()
            .and_then(|state| state.player.as_ref())
            .and_then(|player| player.throwable.as_ref())
            .is_none_or(|weapon| weapon.cooking_duration <= 0.)
    {
        return Ok(());
    }
    let enabled = crate::platform::console::shared()
        .read()
        .cvars
        .int("ark_ui_show_grenade_cooking_gauge")
        .is_some_and(|value| value != 0);
    let input = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| {
            let weapon = player.throwable.as_ref()?;
            if !enabled
                || weapon.cooking_duration <= 0.
                || player
                    .machine
                    .is_active(crate::gameplay::player::states::StateId(0x4c))
            {
                return None;
            }
            let (identity, equipped, transitioning) = player.equipment_action_status(0)?;
            if !equipped || transitioning {
                return None;
            }
            let metadata = world
                .get_resource::<super::player_actions::PlayerActions>()?
                .equipment_metadata(identity)?;
            Some((
                weapon.entity,
                weapon.ammunition_index,
                metadata.equipment.archetype,
                weapon.definition.throw_speed,
                weapon.definition.safe_cooking_extra_time_for_ui,
            ))
        });
    let state = if let Some((weapon, ammo, kind, speed, extra)) = input {
        let state = super::runtime_entities::with_scene(world, |world, scene, live, _| {
            //491080/45bc10 use877530(held equipment): the immediate ARM
            //parent UID is the HUD record's binding target, not the grenade.
            let Some(parent) = live.parent(weapon)? else {
                return Ok(None);
            };
            let owned_arm = world
                .get_resource::<super::player::PlayerState>()
                .and_then(|state| state.player.as_ref())
                .is_some_and(|player| {
                    player
                        .body
                        .arms
                        .iter()
                        .any(|arm| arm.managed_entity == Some(parent))
                });
            if !owned_arm {
                return Ok(None);
            }
            Ok(scene
                .grenade_cooking_state(live, weapon)?
                .map(|state| (parent, state)))
        })?;
        match state {
            Some((parent, (total, progress, true, false))) => {
                Some((weapon, parent, ammo, kind, speed, extra, total, progress))
            }
            _ => None,
        }
    } else {
        None
    };
    let Some((weapon, parent, ammo, kind, speed, extra, total, progress)) = state else {
        if hud.active.is_some() {
            super::runtime_entities::with_scene(world, |world, scene, live, events| {
                scene.set_entity_enabled(world, live, events, entity, false)?;
                scene.unbind_entity(live, entity, events)
            })?;
            hud.active = None;
        }
        return Ok(());
    };
    let Some(lead) = hud
        .lead
        .as_ref()
        .filter(|lead| lead.key == (weapon, ammo))
        .and_then(|lead| lead.value.as_ref())
    else {
        return Ok(());
    };
    let initial = if hud.active == Some((weapon, parent)) {
        None
    } else {
        Some(if lead.enabled {
            let radius = lead
                .explosion
                .as_ref()
                .map(|declaration| {
                    super::explosions::player_owned_damage_radius(world, declaration)
                })
                .transpose()?
                .unwrap_or(0.);
            let value = (total - (radius / speed + lead.event_seconds + extra)) / total;
            if 0. <= value {
                if value > 1. { 1. } else { value }
            } else {
                0.
            }
        } else {
            1.
        })
    };
    let kind = if kind == 5 { 9 } else { 8 };
    super::runtime_entities::with_scene(world, |_, scene, live, _| {
        scene.set_grenade_cooking(live, entity, initial, kind, progress)
    })?;
    if hud.active != Some((weapon, parent)) {
        if hud.active.is_some() {
            super::runtime_entities::with_scene(world, |world, scene, live, events| {
                scene.set_entity_enabled(world, live, events, entity, false)?;
                scene.unbind_entity(live, entity, events)
            })?;
            hud.active = None;
            return Ok(());
        }
        //A previous movie's actual Stop/render fence must finish before this
        //same component generation starts again. No fabricated readiness.
        if !world
            .resource::<super::physics_scene::PhysicsScene>()
            .grenade_cooking_can_start(entity)?
        {
            return Ok(());
        }
        let tag = if kind == 9 {
            &hud.definition.as_ref().unwrap().sticky_tag
        } else {
            &hud.definition.as_ref().unwrap().tag
        };
        world.resource_scope(|world, player: Mut<super::player::PlayerState>| {
            super::runtime_entities::with_scene(world, |world, scene, live, events| {
                scene.bind_cooking_equipment(
                    live,
                    events,
                    entity,
                    parent,
                    player
                        .player
                        .as_ref()
                        .ok_or("GrenadeCooking Player absent")?,
                    tag,
                    hud.definition
                        .as_ref()
                        .and_then(|definition| definition.entity.model.as_ref())
                        .map(|model| model.model.as_str()),
                )?;
                scene.set_entity_enabled(world, live, events, entity, true)
            })
        })?;
        hud.active = Some((weapon, parent));
    }
    Ok(())
}

pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut hud) = world.remove_resource::<Hud>() else {
        return Ok(true);
    };
    let result = (|| {
        hud.closed = true;
        if let Some(pending) = &hud.pending {
            pending.cancel.store(true, Ordering::Release);
        }
        if let Some(pending) = &mut hud.pending {
            let done = block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "grenade HUD worker poisoned")?,
            ))
            .is_some();
            if !done {
                return Ok(false);
            }
            hud.pending = None;
        }
        if let Some(lead) = &mut hud.lead {
            if let Some(pending) = &mut lead.pending {
                pending.cancel.store(true, Ordering::Release);
                if block_on(poll_once(
                    pending
                        .task
                        .get_mut()
                        .map_err(|_| "grenade HUD lead worker poisoned")?,
                ))
                .is_none()
                {
                    return Ok(false);
                }
                lead.pending = None;
            }
        }
        if let Some(entity) = hud.entity {
            if world
                .resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .get(entity)
                .is_some()
                && world
                    .resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                    .parent(entity)?
                    .is_some()
            {
                //45bd60's ordinary Disable/unbind leaves the real movie Stop
                //and render ACK with its canonical component/batch owner.
                super::runtime_entities::with_scene(world, |world, scene, live, events| {
                    scene.set_entity_enabled(world, live, events, entity, false)?;
                    scene.unbind_entity(live, entity, events)
                })?;
                hud.active = None;
            }
            if !super::entity_removal::request_and_wait(world, entity)? {
                return Ok(false);
            }
            hud.entity = None;
        }
        if hud.requested {
            if let Some(definition) = &hud.definition {
                super::runtime_entities::release_for(
                    world,
                    hud.scope.as_deref().ok_or("grenade HUD scope absent")?,
                    &definition.entity,
                )?;
            }
            hud.requested = false;
        }
        hud.definition = None;
        hud.lead = None;
        hud.active = None;
        Ok(true)
    })();
    world.insert_resource(hud);
    result
}
