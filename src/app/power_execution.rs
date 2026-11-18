//! Compose real family owners with the player's shared cast state. Admission
//! precedes state activation; only the family's accepted execution pays mana.
//! Source ordering: 61bce0/61bea0, Bend Time 61c670, Blink 61ec20.
use bevy::prelude::*;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::LiveEntities,
    game_clock_runtime::{ClockRuntime, GamePaused},
    powers::{self, Phase, world_effects::BendTimeAttributes},
    player::power_states::PowerLifecycle,
};

use crate::gameplay::{entity_lifetime::EntityId, reference_frame::ReferenceFrame, event_graph::EntityReference};
use std::{collections::HashMap, sync::Arc};

#[derive(Resource, Default)]
struct WorldEffectSequence(u64);
/// Reuse the same canonical geometry/query adapter as player movement.
/// One map-scoped scratch owner serves both traversal powers, retaining only
/// numeric results and capacity; it holds no asset/body leases between calls.
#[derive(Resource, Default)]
struct PowerQueries(std::sync::Mutex<std::cell::RefCell<super::player::collision::Scratch>>);
fn post_world_effect(world: &mut World, path: &str, frame: ReferenceFrame) -> Result<(), String> {
    world.init_resource::<WorldEffectSequence>();
    let key = {
        let mut serial = world.resource_mut::<WorldEffectSequence>();
        serial.0 = serial.0.wrapping_add(1);
        Arc::<str>::from(format!("power-world-{}", serial.0))
    };
    let mut effects = world.get_resource_mut::<super::usable_effects::EffectRequests>()
        .ok_or("power world FX request owner is absent")?;
    // Native87cb90 explicit world matrix has no parent reference. The
    // canonical FX owner releases this producer on actual instance completion.
    effects.start_world_once(key, path, frame);
    Ok(())
}
///441220/87cb90 explicit pose clears attachment but keeps the actual entity,
///its clock and inherited velocities. The shared once-owner drains every tail.
pub(super) fn post_grapple_impact(world: &mut World, path: &str, target: EntityId,
    frame: ReferenceFrame) -> Result<(), String> {
    world.init_resource::<WorldEffectSequence>();
    let key = {
        let mut serial = world.resource_mut::<WorldEffectSequence>();
        serial.0 = serial.0.wrapping_add(1);
        Arc::<str>::from(format!("power-impact-{}", serial.0))
    };
    let name = world.resource::<LiveEntities>().get(target)
        .ok_or("Far Reach impact target retired")?.0.to_owned();
    let published = world.resource::<super::physics_scene::PhysicsScene>()
        .published_entity_frame(world.resource::<LiveEntities>(), target)?;
    world.resource_mut::<super::usable_effects::EffectRequests>().start_context_once(key, path,
        super::usable_effects::EntityPlacement {target: EntityReference::Named(name), tag: String::new(),
            local: Default::default(), attached: false}, frame,
        crate::rendering::bevy_backend::world_effects::EntityEffectContext {
            owner: Some(target), culprit: None, cannot_incapacitate_player_and_allies: false,
            player_impact: false, velocities: published.velocities, contact_speed: 0., contact_mass: 0.,
            contact_stimulus_enabled: None,
        });
    Ok(())
}

#[derive(Resource, Default)]
struct GrappleBeamEffect { owner: Option<Arc<str>> }
fn publish_grapple_beam(world: &mut World, player: &mut crate::gameplay::player::Player) -> Result<(), String> {
    use crate::gameplay::player::body::grapple::BeamRequest;
    let mut pending = player.drain_grapple_beam_requests();
    let Some(first) = pending.next() else { return Ok(()) };
    let mut producer = world.remove_resource::<GrappleBeamEffect>().unwrap_or_default();
    let Some(mut effects) = world.remove_resource::<super::usable_effects::EffectRequests>() else {
        world.insert_resource(producer); return Err("grapple beam FX request owner is absent".into());
    };
    world.init_resource::<WorldEffectSequence>();
    let mut result = Ok(());
    for request in std::iter::once(first).chain(pending) {
        match request {
            BeamRequest::Start { effect, origin, endpoint } => {
                if let Some(old) = producer.owner.take() { effects.stop_entity(old); }
                let owner = {
                    let mut serial = world.resource_mut::<WorldEffectSequence>();
                    serial.0 = serial.0.wrapping_add(1);
                    Arc::<str>::from(format!("player-grapple-beam-{}", serial.0))
                };
                effects.start_beam(owner.clone(), &effect, origin, endpoint);
                producer.owner = Some(owner);
            }
            BeamRequest::Update { origin, endpoint } => {
                if let Some(owner) = &producer.owner { effects.update_beam(owner.clone(), origin, endpoint); }
                else { result = Err("grapple beam endpoint update has no started FX producer".into()); }
            }
            BeamRequest::Stop => {
                if let Some(owner) = producer.owner.take() { effects.stop_entity(owner); }
            }
        }
    }
    world.insert_resource(effects); world.insert_resource(producer); result
}

#[derive(Default)]
struct WindEffectEntry {
    next: u32,
    handles: Vec<(Arc<str>, super::usable_effects::EffectTicket)>,
    retired: bool,
}
/// Mutable producer handles only. Definitions, acquisition and tails remain
/// in the shared FX owner; a blast retains its real identity until its own
/// actual tickets acknowledge retirement.
#[derive(Resource, Default)]
struct WindEffects(HashMap<EntityId, WindEffectEntry>);
impl WindEffects {
    fn refresh(&mut self, world: &World) {
        for entry in self.0.values_mut() {
            entry.retired = entry.handles.iter().all(|(_, ticket)| super::usable_effects::entity_retired(world, ticket));
        }
        if let Some(live) = world.get_resource::<LiveEntities>() {
            self.0.retain(|id, entry| live.get(*id).is_some() || !entry.retired);
        }
    }
}
struct WindEffectHost<'a> {
    handles: &'a mut WindEffects,
    requests: &'a mut super::usable_effects::EffectRequests,
    player: Option<(EntityId, ReferenceFrame)>,
}
impl WindEffectHost<'_> {
    fn start_world_at(&mut self, entity: EntityId, path: &str, frame: ReferenceFrame) {
        let entry = self.handles.0.entry(entity).or_default();
        entry.next = entry.next.wrapping_add(1);
        let owner: Arc<str> = format!("power-windblast-{entity:?}-{}", entry.next).into();
        let ticket = self.requests.start_world(owner.clone(), path, frame);
        entry.handles.push((owner, ticket)); entry.retired = false;
    }
    fn start_at(&mut self, entity: EntityId, path: &str, placement: super::usable_effects::EntityPlacement) {
        let entry = self.handles.0.entry(entity).or_default();
        entry.next = entry.next.wrapping_add(1);
        let owner: Arc<str> = format!("power-windblast-{entity:?}-{}", entry.next).into();
        let ticket = self.requests.start_entity(owner.clone(), path, placement);
        entry.handles.push((owner, ticket)); entry.retired = false;
    }
}
fn named(live: &LiveEntities, entity: EntityId) -> Result<EntityReference, String> {
    live.get(entity).map(|(name, _)| EntityReference::Named(name.to_owned()))
        .ok_or_else(|| "power FX target generation is stale".to_owned())
}
impl super::windblast::Effects for WindEffectHost<'_> {
    fn start(&mut self, live: &LiveEntities, entity: EntityId, path: &str,
        frame: ReferenceFrame, spawner: Option<EntityId>) -> Result<(), String> {
        if let Some(spawner) = spawner {
            let (_, frame) = self.player.filter(|(id, _)| *id == spawner)
                .ok_or("Windblast spawner frame is absent")?;
            self.start_world_at(entity, path, frame);
            return Ok(());
        }
        let _ = frame;
        let placement = super::usable_effects::EntityPlacement { target: named(live, entity)?, tag: String::new(),
            local: ReferenceFrame::default(), attached: true };
        self.start_at(entity, path, placement); Ok(())
    }
    fn impact(&mut self, live: &LiveEntities, entity: EntityId, target: EntityId,
        path: &str, tag: Option<&str>, parent: ReferenceFrame, frame: ReferenceFrame) -> Result<(), String> {
        let attached = tag.is_some_and(|tag| !tag.is_empty());
        if !attached { self.start_world_at(entity, path, frame); return Ok(()); }
        let _ = parent;
        self.start_at(entity, path, super::usable_effects::EntityPlacement {
            target: named(live, target)?, tag: tag.unwrap_or("").to_owned(), attached: true,
            local: ReferenceFrame::default(),
        }); Ok(())
    }
    fn stop(&mut self, entity: EntityId) -> Result<(), String> {
        if let Some(entry) = self.handles.0.get(&entity) {
            for (owner, _) in &entry.handles { self.requests.stop_entity(owner.clone()); }
        }
        Ok(())
    }
    fn retired(&self, entity: EntityId) -> bool {
        self.handles.0.get(&entity).is_none_or(|entry| entry.retired)
    }
}

/// Stop producers before physics/player shutdown. Poll only after the shared
/// FX simulation and audio tails acknowledge shutdown, before LiveEntities.
pub(super) fn begin_wind_retirement(world: &mut World) -> Result<(), String> {
    wind_retirement(world, false).map(|_| ())
}
pub(super) fn poll_wind_retirement(world: &mut World) -> Result<bool, String> {
    wind_retirement(world, true)
}
fn wind_retirement(world: &mut World, poll: bool) -> Result<bool, String> {
    let Some(mut blasts) = world.remove_resource::<super::windblast::Windblasts>() else {
        if poll {
            // Called only after canonical FX/audio shutdown acknowledgement.
            world.remove_resource::<WindEffects>();
            world.remove_resource::<WorldEffectSequence>();
            world.remove_resource::<GrappleBeamEffect>();
        }
        return Ok(true);
    };
    let mut handles = world.remove_resource::<WindEffects>().unwrap_or_default(); handles.refresh(world);
    let result = if poll {
        match (world.remove_resource::<LiveEntities>(), world.remove_resource::<ClockRuntime>()) {
            (Some(mut live), Some(mut clock)) => {
                let host_requests = world.remove_resource::<super::usable_effects::EffectRequests>().unwrap_or_default();
                let mut requests = host_requests;
                let host = WindEffectHost { handles: &mut handles, requests: &mut requests, player: None };
                let result = blasts.poll_retirement(&mut live, &mut clock, &host);
                world.insert_resource(requests); world.insert_resource(live); world.insert_resource(clock); result
            }
            (live, clock) => { if let Some(live) = live { world.insert_resource(live); } if let Some(clock) = clock { world.insert_resource(clock); }
                Err("Windblast retirement lost live identity or clock owner".into()) }
        }
    } else if let Some(mut requests) = world.remove_resource::<super::usable_effects::EffectRequests>() {
        let mut host = WindEffectHost { handles: &mut handles, requests: &mut requests, player: None };
        let result = blasts.begin_retirement(&mut host).map(|_| false); world.insert_resource(requests); result
    } else { Err("Windblast retirement lost FX request owner".into()) };
    if poll && matches!(&result, Ok(true)) {
        // These lazy producer resources were created after the level's
        // baseline capture. Actual FX tickets have acknowledged retirement;
        // release their generation keys and source-path references now.
        world.remove_resource::<WorldEffectSequence>();
        world.remove_resource::<GrappleBeamEffect>();
    } else { world.insert_resource(handles); }
    world.insert_resource(blasts); result
}

fn service_wind(world: &mut World, mut player: Option<(&mut crate::gameplay::player::Player, EntityId)>,
    clock: &mut ClockRuntime, events: &mut EntityEvents, spawn: bool) -> Result<bool, String> {
    let Some(mut blasts) = world.remove_resource::<super::windblast::Windblasts>() else { return Ok(false) };
    if !spawn && !blasts.has_active() { world.insert_resource(blasts); return Ok(false); }
    let mut handles = world.remove_resource::<WindEffects>().unwrap_or_default(); handles.refresh(world);
    let mut live = world.remove_resource::<LiveEntities>();
    let mut scene = world.remove_resource::<super::physics_scene::PhysicsScene>();
    let mut requests = world.remove_resource::<super::usable_effects::EffectRequests>();
    let result = (|| {
        let live = live.as_mut().ok_or("Windblast live entity owner is absent")?;
        let scene = scene.as_mut().ok_or("Windblast physics owner is absent")?;
        let requests = requests.as_mut().ok_or("Windblast FX request owner is absent")?;
        let mut host = WindEffectHost { handles: &mut handles, requests,
            player: player.as_ref().map(|(player, id)| (*id, ReferenceFrame { position: player.feet().to_array(), basis: player.physics.axis().to_cols_array_2d() })) };
        let accepted = if spawn {
            let (player, id) = player.as_mut().ok_or("Windblast player identity is absent")?;
            let view = player.view();
            let cast = player.selected_power().ok_or("Windblast selected cast is absent")?;
            let settings = cast.settings.windblast.as_ref().ok_or("Windblast typed settings are absent")?;
            let changed = |tag| crate::gameplay::player::power_states::attribute_changed(&player.attributes, tag);
            let greater = changed("ARK_ATTRIBUTE_WINDBLAST_GREATER");
            let shockwave = changed("ARK_ATTRIBUTE_WINDBLAST_SHOCKWAVE");
            let path = settings.entity(greater, shockwave).ok_or("Windblast selected variant is absent")?;
            let range = player.attributes.0.get("ARK_ATTRIBUTE_WINDBLAST_RANGE_FACTOR").copied().unwrap_or(1.0);
            let falloff = player.attributes.0.get("ARK_ATTRIBUTE_WINDBLAST_FALOFF_FACTOR").copied().unwrap_or(1.0);
            blasts.spawn(path, *id, ReferenceFrame { position: view.origin.to_array(), basis: view.axis.to_cols_array_2d() },
                player.camera.pitch, range, falloff, live, clock, &mut host)?
        } else { false };
        if !spawn {
            let (thickness, minimum) = {
                let console = crate::platform::console::shared().read();
                (console.cvars.float("ark_windblast_thicknessCoef").unwrap_or(powers::blast::DEFAULT_THICKNESS),
                 console.cvars.float("ark_windblast_minImpulseForPhysicEntities").unwrap_or(powers::blast::DEFAULT_MINIMUM_IMPULSE))
            };
            blasts.update(scene, live, clock, thickness, minimum, &mut host,
                |scene, live, attacker, inflictor, target, declaration, amount, direction| {
                    let mut parameters = crate::gameplay::damage::Parameters::from_declaration(declaration);
                    parameters.set_amount(declaration, amount, true);
                    parameters.attacker = Some(attacker); parameters.inflictor = Some(inflictor);
                    parameters.hit_direction = Vec3::from_array(direction);
                    scene.damage_entity(live, events, target, &parameters).map(|result| result.is_some())
                })?;
        }
        Ok(accepted)
    })();
    if let Some(live) = live { world.insert_resource(live); }
    if let Some(scene) = scene { world.insert_resource(scene); }
    if let Some(requests) = requests { world.insert_resource(requests); }
    world.insert_resource(handles); world.insert_resource(blasts); result
}

/// Service only the component which accepted this state12 request: another
/// newly selected Blink component must not acquire its time lease or consume
/// its completion notification. Both service phases use the same real owner.
fn service_blinks(world: &mut World, player: &mut crate::gameplay::player::Player,
    clock: &mut ClockRuntime, aim: bool) -> Result<(), String> {
    world.init_resource::<PowerQueries>();
    let Some(mut actions) = world.remove_resource::<super::player_actions::PlayerActions>() else { return Ok(()) };
    if !actions.has_family_work() { world.insert_resource(actions); return Ok(()); }
    let Some(mut effects) = world.remove_resource::<super::usable_effects::EffectRequests>() else {
        world.insert_resource(actions); return Err("Blink FX request owner is absent".into());
    };
    let result = (|| {
        if actions.needs_preparation() {
            world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                actions.prepare_family_runtime(player, clock, &mut effects, &mut scene, world.resource::<LiveEntities>())
            })?;
        }
        if actions.blink.is_none() && actions.active_blink.is_none() { return Ok(()); }
        let collision = world.get_resource::<super::NavigationWorld>().ok_or("Blink collision world is absent")?;
        let live = world.get_resource::<LiveEntities>().ok_or("Blink entity owner is absent")?;
        let scene = world.get_resource::<super::physics_scene::PhysicsScene>();
        if let Some(blink) = actions.active_blink.as_mut() {
            blink.stop_helpers(&mut effects);
            let retained = if blink.has_activity() { blink.service(player, clock, &mut effects, &collision.0, scene, live)? } else { false };
            if !retained { blink.cancel(player); actions.finish_blink_activity(); }
        }
        if let Some(blink) = actions.blink.as_mut() {
            if blink.has_activity() { blink.service(player, clock, &mut effects, &collision.0, scene, live)?; }
            if aim {
                let aiming = player.power_cast_active(2) && player.selected_power().is_some_and(|cast| cast.settings.id == 2 && cast.phase() == Phase::Aiming);
                if aiming && !blink.has_activity() {
                    let scene = scene.ok_or("Blink canonical physics owner is absent")?;
                    let query_owner = scene.point_queries()?;
                    let query_world = query_owner.read().map_err(|_| "Blink canonical query world is poisoned")?;
                    let scratch = world.resource::<PowerQueries>().0.lock().map_err(|_| "power query scratch is poisoned")?;
                    let queries = super::player::collision::Collision::new(&collision.0, scene, &query_world, &scratch);
                    let view = player.view();
                    blink.aim(player, &view, &queries, &collision.0, live,
                        world.get_resource::<super::interactions::Interactions>(), Some(scene))?;
                    if let Some(error) = queries.take_error() { return Err(error); }
                    blink.publish_helpers(&mut effects, player);
                } else {
                    blink.stop_helpers(&mut effects);
                    // The release callback below still consumes the aimed
                    // target after UsePower changes Aiming to Cast.
                    if !blink.has_activity() && !player.power_cast_active(2) { blink.cancel(player); }
                }
            }
        }
        Ok(())
    })();
    let result = result.and_then(|()| {
        if aim || (!actions.blink.as_ref().is_some_and(|blink| blink.has_motion())
            && !actions.active_blink.as_ref().is_some_and(|blink| blink.has_motion())) { return Ok(()); }
        if !world.contains_resource::<super::physics_scene::PhysicsScene>()
            || !world.contains_resource::<LiveEntities>() || !world.contains_resource::<EntityEvents>() {
            return Err("Blink damage has no canonical scene/entity/event owner".into());
        }
        let dt = clock.player_delta().map_err(str::to_owned)?;
        world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
            world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                let mut events = world.resource_mut::<EntityEvents>();
                if let Some(blink) = actions.active_blink.as_ref() {
                    blink.damage_breakables(player, dt, &mut scene, &mut live, &mut events)?;
                }
                if let Some(blink) = actions.blink.as_ref() {
                    blink.damage_breakables(player, dt, &mut scene, &mut live, &mut events)?;
                }
                Ok(())
            })
        })
    });
    world.insert_resource(effects); world.insert_resource(actions);
    result
}

/// The accepted Grapple motion owns its sensor/scene projection through
/// equipment changes. Service its real state completion independently of
/// whichever power is currently selected.
fn service_far_reach(world: &mut World, player: &mut crate::gameplay::player::Player,
    clock: &mut ClockRuntime, events: &mut EntityEvents, aim: bool) {
    world.init_resource::<PowerQueries>();
    let Some(mut actions) = world.remove_resource::<super::player_actions::PlayerActions>() else { return };
    let mut impacts = [None, None];
    let result = (|| {
        if let Some(active) = actions.active_far_reach.as_mut() {
            active.service_objects(world, player, clock, events)?;
            let retained = active.service(player)?;
            impacts[0] = active.take_world_impact();
            if !retained { actions.finish_far_reach_activity(); }
        }
        if let Some(far) = actions.far_reach.as_mut() {
            far.service_objects(world, player, clock, events)?;
            if far.has_activity() { far.service(player)?; }
            impacts[1] = far.take_world_impact();
            if aim {
                let aiming = player.power_cast_active(6) && player.selected_power().is_some_and(|cast| cast.phase() == Phase::Aiming);
                if aiming && !far.has_cast_activity() {
                    let view = player.view();
                    world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                        let live = world.resource::<LiveEntities>();
                        far.acquire(&mut scene, live, player, &view, crate::platform::console::shared().read().cvars
                            .int("ark_shadow_grapple_snapping_enabled").unwrap_or(1) != 0)
                    })?;
                    if let Some(collision) = world.get_resource::<super::NavigationWorld>() {
                        let scene = world.get_resource::<super::physics_scene::PhysicsScene>().ok_or("Far Reach canonical physics owner is absent")?;
                        let query_owner = scene.point_queries()?;
                        let query_world = query_owner.read().map_err(|_| "Far Reach canonical query world is poisoned")?;
                        let scratch = world.resource::<PowerQueries>().0.lock().map_err(|_| "power query scratch is poisoned")?;
                        let queries = super::player::collision::Collision::new(&collision.0, scene, &query_world, &scratch);
                        let view = player.view();
                        let snapping = crate::platform::console::shared().read().cvars
                            .int("ark_shadow_grapple_snapping_enabled").unwrap_or(1) != 0;
                        far.aim(player, &view, &queries, snapping);
                        if let Some(error) = queries.take_error() { return Err(error); }
                    }
                    world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                        far.submit_acquisition(&mut scene, world.resource::<LiveEntities>(), player)
                    })?;
                } else {
                    world.resource_scope(|world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                        far.cancel_acquisition(&mut scene, world.resource::<LiveEntities>(), player)
                    })?;
                    if !far.has_cast_activity() && !player.power_cast_active(6) { far.cancel(player); }
                }
            }
        }
        Ok::<(), String>(())
    })();
    world.insert_resource(actions);
    if let Err(error) = result { power_error(world, &error); }
    for (path, position, axis) in impacts.into_iter().flatten() {
        if let Err(error) = post_world_effect(world, &path, ReferenceFrame {position: position.to_array(),basis: axis.to_cols_array_2d()}) {
            power_error(world, &error);
        }
    }
}

/// Map retirement releases both selected and detached consumers before the
/// canonical clock, request queue and player are destroyed.
pub(super) fn retire_blinks(world: &mut World) -> Result<(), String> {
    super::player_actions::retire_effects(world)?;
    world.remove_resource::<PowerQueries>();
    Ok(())
}

fn power_error(world: &mut World, error: &str) {
    error!("Power owner publication failed: {error}");
    crate::platform::client_diagnostics::record(&format!("Power owner publication failed: {error}"));
    world.write_message(AppExit::error());
}

fn cast_factor(player: &crate::gameplay::player::Player, id: i32, active_bend: bool) -> f32 {
    if id == 1 && active_bend { return 0.0; }
    // Family cost overrides (including Windblast's exact floorf result)
    // belong to Cast, before the shared random cost rolls.
    powers::cost_factor(&player.attributes)
}

/// Runs after selection publication, before the shared player state machine.
pub(super) fn before_frame(world: &mut World) {
    if world.resource::<GamePaused>().0 { return; }
    let Some(now) = world.get_resource::<ClockRuntime>()
        .filter(|clock| clock.advanced()).and_then(|clock| clock.player_time().ok()) else { return };
    let player_id = super::player_lifetime::id(world);
    let Some(mut state) = world.remove_resource::<super::player::PlayerState>() else { return };
    if let Some(player) = state.player.as_mut() {
        player.set_power_time(now);
        if let Some(mut clock) = world.remove_resource::<ClockRuntime>() {
            if let Some(mut events) = world.remove_resource::<EntityEvents>() {
                service_far_reach(world, player, &mut clock, &mut events, false);
                world.insert_resource(events);
            }
            let result = service_blinks(world, player, &mut clock, false);
            world.insert_resource(clock);
            if let Err(error) = result { power_error(world, &error); }
        }
        if let Some(id) = player.selected_power().map(|cast| cast.settings.id) {
            let owned = world.get_resource::<super::powers::Powers>()
                .zip(world.get_resource::<super::interaction_focus::DiscoveredDocuments>())
                .is_some_and(|(powers, inventory)| powers.definitions.0.iter().any(|power|
                    crate::assets::power_journal::power_id(&power.power_id) == Some(id)
                    && inventory.quantity(&power.item) > 0));
            let bend = world.get_resource::<super::power_world::PowerWorld>();
            let active_bend = id == 1 && bend.is_some_and(|owner| owner.bend_time_active());
            if id == 1 && player.power_cast_active(1) && bend.is_some_and(|owner| owner.bend_time_must_cancel()) { player.cancel_power_use(); }
            let available = match id {
                1 => bend.zip(world.get_resource::<ClockRuntime>())
                    .is_some_and(|(owner, clock)| owner.bend_time_available(clock)),
                2 => world.get_resource::<super::player_actions::PlayerActions>().is_some_and(|actions| actions.blink.is_some())
                    && player_id.is_some()
                    && world.contains_resource::<super::NavigationWorld>()
                    && world.contains_resource::<LiveEntities>()
                    && world.contains_resource::<super::usable_effects::EffectRequests>(),
                6 => world.get_resource::<super::player_actions::PlayerActions>()
                    .is_some_and(|actions| actions.far_reach.is_some()) && world.contains_resource::<super::NavigationWorld>(),
                9 => world.get_resource::<super::dark_vision::DarkVisionOwner>().is_some_and(|owner| owner.available()),
                10 => world.contains_resource::<super::windblast::Windblasts>() && player_id.is_some()
                    && world.contains_resource::<super::physics_scene::PhysicsScene>()
                    && world.contains_resource::<super::usable_effects::EffectRequests>(),
                _ => false,
            };
            // 399620 returns literal false in the retail executable.
            let factor = cast_factor(player, id, active_bend);
            // 61c0e0 reads idCVar+28; registration1413869c0 names
            // gp_powerCooldownEnable, default1 (canonical CVARS table).
            let cooldown_enabled = crate::platform::console::shared().read().cvars
                .int("gp_powerCooldownEnable").unwrap_or(1) != 0;
            player.set_power_cooldown_bypass(active_bend || !cooldown_enabled);
            player.set_power_admission(now, owned, available, false, factor);
        }
    }
    world.insert_resource(state);
}

/// 854fd0/855750 dispatch the actual animation callback before state updates.
/// In particular, the last scene frame can both cast and finish UsePower.
pub(super) fn execute_animation_actions(world: &mut World) -> Result<(), String> {
    if world.get_resource::<GamePaused>().is_none_or(|paused| paused.0) { return Ok(()); }
    let Some(now) = world.get_resource::<ClockRuntime>().filter(|clock| clock.advanced())
        .and_then(|clock| clock.player_time().ok()) else { return Ok(()); };
    if !world.get_resource::<super::player::PlayerState>().and_then(|state| state.player.as_ref())
        .is_some_and(|player| player.power_execution_pending()) { return Ok(()); }
    let player_id = super::player_lifetime::id(world);
    world.resource_scope(|world, mut state: Mut<super::player::PlayerState>| {
        world.resource_scope(|world, mut clock: Mut<ClockRuntime>| {
            world.resource_scope(|world, mut events: Mut<EntityEvents>| {
                let player = state.player.as_mut().ok_or("power animation callback lost its player")?;
                execute_pending(world, player, player_id, now, &mut clock, &mut events)
            })
        })
    })
}

fn execute_pending(world: &mut World, player: &mut crate::gameplay::player::Player,
    player_id: Option<EntityId>, now: f64, clock: &mut ClockRuntime, events: &mut EntityEvents) -> Result<(), String> {
    if !player.take_power_execution_request() { return Ok(()); }
    let id = player.selected_power().map(|cast| cast.settings.id);
    let factor = id.map_or(1.0, |id| cast_factor(player, id, false));
    let accepted = match id {
        Some(1) => {
            let input = player.power_inputs();
            let attributes = BendTimeAttributes::read(&player.attributes);
            let position = player.feet();
            world.get_resource_mut::<super::power_world::PowerWorld>().is_some_and(|mut owner| {
                let factor = if owner.bend_time_active() { 0.0 } else { factor };
                owner.cast_bend_time(attributes, input.held,
                    || { player.commit_power_cast(now, factor); }, clock, events, position)
            })
        }
        Some(2) => {
            let view = player.view();
            let accepted = world.get_resource_mut::<super::player_actions::PlayerActions>()
                .and_then(|mut actions| actions.blink.as_mut().map(|blink| blink.execute(player, &view)))
                .unwrap_or(false);
            if accepted { player.commit_power_cast(now, factor); }
            accepted
        }
        Some(6) => {
            let view = player.view();
            let available = world.get_resource::<super::player_actions::PlayerActions>()
                .and_then(|actions| actions.far_reach.as_ref().map(|far| far.object_admission(world, player)))
                .unwrap_or(Ok(false))?;
            if !available {player.cancel_power_use(); return Ok(());}
            let accepted = world.get_resource_mut::<super::player_actions::PlayerActions>()
                .and_then(|mut actions| actions.far_reach.as_mut().map(|far| far.execute(player, &view))).unwrap_or(Ok(false))?;
            if accepted { player.commit_power_cast(now, factor); }
            accepted
        }
        Some(9) => {
            // 14043d490: deactivating or a blocked start costs nothing.
            let factor = super::dark_vision::mana_factor(world, factor);
            super::dark_vision::cast(world, || { player.commit_power_cast(now, factor); })
        }
        Some(10) => {
            let accepted = service_wind(world, player_id.map(|id| (&mut *player, id)), clock, events, true)?;
            if accepted { player.commit_power_cast(now, factor); }
            accepted
        }
        _ => false,
    };
    if accepted { player.finish_power_execution(); } else { player.cancel_power_use(); }
    Ok(())
}

/// The action state, rather than raw mouse release, decides when to execute.
pub(super) fn after_frame(world: &mut World) {
    if world.resource::<GamePaused>().0 { return; }
    let Some(now) = world.get_resource::<ClockRuntime>()
        .filter(|clock| clock.advanced()).and_then(|clock| clock.player_time().ok()) else { return };
    let player_id = super::player_lifetime::id(world);
    let Some(mut state) = world.remove_resource::<super::player::PlayerState>() else { return };
    let Some(mut clock) = world.remove_resource::<ClockRuntime>() else {
        world.insert_resource(state); return;
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(clock); world.insert_resource(state); return;
    };
    if let Some(player) = state.player.as_mut() {
        let attributes = BendTimeAttributes::read(&player.attributes);
        let position = player.feet();
        for event in player.take_power_sound_events() {
            world.write_message(super::audio::SoundEvent(crate::gameplay::sound::Request::Post {
                event, emitter: crate::gameplay::sound::Emitter::PlayerBody,
            }));
        }
        for effect in player.take_grapple_effects() {
            if let Err(error) = post_world_effect(world, &effect.path, effect.frame) { power_error(world, &error); }
        }
        let lifecycle = player.take_power_lifecycle_events();
        let attribute_messages = player.take_power_attribute_change_count();
        if let Some(mut owner) = world.get_resource_mut::<super::power_world::PowerWorld>() {
            for event in lifecycle.iter().copied() {
                match event {
                    PowerLifecycle::Began(1) => owner.begin_bend_time_cast(),
                    PowerLifecycle::Finished { id: 1, .. } =>
                        owner.finish_bend_time_cast(attributes, &mut clock, &mut events, position),
                    _ => {},
                }
            }
            // This is the actual attribute owner's d6 fanout, not a value
            // comparison: native Bend Time posts its loop cue per message.
            for _ in 0..attribute_messages { owner.attributes_changed(attributes, &mut clock, &mut events, position); }
            let input = player.power_inputs();
            let committed = player.selected_power().is_some_and(|cast| cast.settings.id == 1 && cast.phase() == Phase::Cast);
            let factor = if owner.bend_time_active() { 0.0 } else { powers::cost_factor(&player.attributes) };
            owner.update_bend_time_hold(attributes, input.held, input.pressure,
                player.controller.trigger_threshold, player.power_cast_active(1), committed,
                || { player.commit_power_cast(now, factor); }, &mut clock, &mut events, position);
        }
        let far_delta = match clock.player_delta() {
            Ok(delta) => Some(delta),
            Err(error) => { power_error(world, error); None },
        };
        if let Some(mut actions) = world.get_resource_mut::<super::player_actions::PlayerActions>() {
            // These are the same power component's real Begin/Finish
            // callbacks; its hold state is never inferred from frame changes.
            let actions = &mut *actions;
            let far = actions.far_reach.as_mut().or(actions.active_far_reach.as_mut());
            if let Some(far) = far {
                for event in lifecycle {
                    match event {
                        PowerLifecycle::Began(6) => far.begin_cast(),
                        PowerLifecycle::Finished { id: 6, .. } => far.finish_cast(),
                        _ => {},
                    }
                }
                let mode = crate::platform::console::shared().read().cvars
                    .int("ark_shadow_grapple_cast_mode").unwrap_or(0);
                if let Some(delta) = far_delta { far.update_cast(player, delta, mode); }
            }
        }
        if let Ok(delta) = clock.player_delta() {
            super::dark_vision::update(world, Some(&*player), delta);
        }
        service_far_reach(world, player, &mut clock, &mut events, true);
        if let Err(error) = service_blinks(world, player, &mut clock, true) { power_error(world, &error); }
        if let Err(error) = execute_pending(world, player, player_id, now, &mut clock, &mut events) { power_error(world, &error); }
        // Body owner has already advanced its actual hand-tag endpoint;
        // family Start/Stop callbacks above are in this same ordered queue.
        if let Err(error) = publish_grapple_beam(world, player) { power_error(world, &error); }
    }
    if let Err(error) = service_wind(world, state.player.as_mut().zip(player_id), &mut clock, &mut events, false) {
        power_error(world, &error);
    }
    // 7194a0 sends the native health-potion message through the same registry.
    let mut publication_error = None;
    if let Some(mut actions) = world.remove_resource::<super::player_actions::PlayerActions>() {
        if let Some(id) = player_id {
            if let Some(mut statistics) = world.get_resource_mut::<super::player_statistics::PlayerStatistics>() {
                for result in actions.potion_results.drain(..) {
                    if result.health_used_message { events.post(Event { id: 0x15c, target: Some(id), payload: Payload::None }); }
                    if let Some(statistic) = result.statistic {
                        if let Err(error) = statistics.increment(statistic) { publication_error = Some(error); break; }
                    }
                }
                statistics.publish_updates(id, &mut events);
            } else if !actions.potion_results.is_empty() {
                publication_error = Some("potion statistics owner is absent".into());
            }
        } else if !actions.potion_results.is_empty() {
            publication_error = Some("potion result has no registered player identity".into());
        }
        world.insert_resource(actions);
    }
    world.insert_resource(events); world.insert_resource(clock);
    world.insert_resource(state);
    if let Some(error) = publication_error {
        error!("Player potion publication failed: {error}");
        crate::platform::client_diagnostics::record(&format!("Player potion publication failed: {error}"));
        world.write_message(AppExit::error());
    }
    if let Err(error) = super::player_damage::flush(world) {
        error!("Player damage publication failed: {error}");
        crate::platform::client_diagnostics::record(&format!("Player damage publication failed: {error}"));
        world.write_message(AppExit::error());
    }
}
