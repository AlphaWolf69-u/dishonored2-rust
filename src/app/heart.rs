//! Actual equipped Heart612c70 and player RuneCharms sensor20 owners.
//! Assets remain with the shared declaration/audio/Midnight owners.
use crate::{
    assets::wwise::SoundEventDeclaration,
    gameplay::{
        entity_events::{EntityEvents, Event, Listener, Payload},
        entity_lifetime::{EntityId, LiveEntities},
        game_clock_runtime::{ClockRuntime, GamePaused},
        player::weapons::heart::{self, Effect, sensor},
        sound::{Emitter, Request},
    },
};
use bevy::{camera::CameraProjection, prelude::*};
use std::{collections::HashMap, sync::Arc};
mod barks;

struct Entry {
    runtime: heart::Runtime,
    equip: Option<Arc<SoundEventDeclaration>>,
    unequip: Option<Arc<SoundEventDeclaration>>,
    scope: Arc<str>,
    device: Option<barks::Device>,
}
#[derive(Resource, Default)]
pub(super) struct Hearts {
    entries: HashMap<EntityId, Entry>,
    sensor: sensor::Sensor,
    sensor_definition: Option<Arc<sensor::Definition>>,
    sensor_sound: Option<Arc<SoundEventDeclaration>>,
    sensor_scope: Option<Arc<str>>,
    subscription: Option<EntityId>,
    view: Option<bevy::ecs::query::QueryState<&'static Projection, With<Camera3d>>>,
}
impl Hearts {
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  Heart owners={} listener={:?}", self.entries.len(), self.subscription);
        for (id, entry) in self.entries.iter().take(4) {
            let runtime = &entry.runtime;
            let _ = writeln!(report, "    entity={:?} equipped={} target={:?} bark_target={:?} bark_playing={} bark_time={} last_tick={} device={}", id, runtime.equipped, runtime.target, runtime.bark_target, runtime.bark_playing, runtime.bark_time, runtime.last_tick, entry.device.is_some());
            if let Some(device) = &entry.device { device.write_diagnostics(report); }
        }
    }
}
fn initialize(world: &mut World) {
    if !world.contains_resource::<Hearts>() {
        world.insert_resource(Hearts::default());
    }
}
fn subscribe(world: &mut World) -> Result<(), String> {
    if world.resource::<Hearts>().subscription.is_some() {
        return Ok(());
    }
    let player = player(world)?;
    world
        .get_resource_mut::<EntityEvents>()
        .ok_or("UseHeart listener event owner absent")?
        .register(0x11d, Some(player), Listener::HeartState { player });
    world.resource_mut::<Hearts>().subscription = Some(player);
    Ok(())
}
fn sound(world: &World, path: Option<&str>) -> Result<Option<Arc<SoundEventDeclaration>>, String> {
    let Some(path) = path else { return Ok(None) };
    let catalog = world
        .resource::<super::SceneData>()
        .catalog
        .as_ref()
        .ok_or("Heart sound catalog absent")?;
    crate::assets::wwise::event_resources::acquire_with(catalog, path, |path| {
        crate::assets::declaration_texts::read_common_or_catalog(catalog, path)
    })
    .map(Some)
}
pub(super) fn prepare(
    world: &mut World,
    id: EntityId,
    definition: Arc<heart::Definition>,
) -> Result<(), String> {
    initialize(world);
    subscribe(world)?;
    if world.resource::<Hearts>().entries.contains_key(&id) {
        return Ok(());
    }
    let equip = sound(world, definition.equip_sound.as_deref())?;
    let unequip = sound(world, definition.unequip_sound.as_deref())?;
    let scope: Arc<str> = Arc::from(format!("heart:{id:?}"));
    if let Err(error) = super::audio::reserve_events(
        world,
        scope.clone(),
        equip.iter().chain(unequip.iter()).cloned(),
    ) {
        super::audio::release_events(world, &scope)?;
        return Err(error);
    }
    world.resource_mut::<Hearts>().entries.insert(
        id,
        Entry {
            runtime: heart::Runtime::new(definition),
            equip,
            unequip,
            scope,
            device: None,
        },
    );
    world.resource_mut::<EntityEvents>().register(
        0xde,
        Some(id),
        Listener::HeartComponent { entity: id },
    );
    Ok(())
}
pub(super) fn prepare_barks(
    world: &mut World,
    id: EntityId,
    definition: Arc<heart::barks::Definition>,
) -> Result<(), String> {
    let mut owner = world
        .get_resource_mut::<Hearts>()
        .ok_or("Heart Barks preparation has no actual Heart owner")?;
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("Heart Barks source generation absent")?;
    if entry.device.is_none() {
        entry.device = Some(barks::Device::new(definition));
    }
    Ok(())
}
impl Hearts {
    pub(super) fn speaking(&self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.runtime.equipped && entry.runtime.bark_playing)
    }
    ///611e10 is invoked by the same normal DE FIFO as other entity listeners.
    pub(super) fn speech_finished(
        &mut self,
        live: &LiveEntities,
        id: EntityId,
        event: &Event,
    ) -> Result<(), String> {
        if event.id != 0xde || event.target != Some(id) || live.get(id).is_none() {
            return Err("Heart Speech completion source generation differs".into());
        }
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or("Heart DE component owner absent")?;
        entry.runtime.bark_finished(live);
        if let Some(device) = entry.device.as_mut() {
            device.acknowledge_completion();
        }
        Ok(())
    }
}
pub(super) fn ready(world: &World, id: EntityId) -> Result<bool, String> {
    let entry = world
        .get_resource::<Hearts>()
        .and_then(|owner| owner.entries.get(&id))
        .ok_or("Heart source owner absent")?;
    if !super::audio::events_ready(world, &entry.scope)? {
        return Ok(false);
    }
    if let Some(scope) = world.resource::<Hearts>().sensor_scope.as_deref() {
        if !super::audio::events_ready(world, scope)? {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn configure_sensor(
    world: &mut World,
    definition: Arc<sensor::Definition>,
) -> Result<bool, String> {
    initialize(world);
    subscribe(world)?;
    if let Some(current) = world.resource::<Hearts>().sensor_definition.as_ref() {
        return if Arc::ptr_eq(current, &definition) {
            Ok(false)
        } else {
            Err("RuneCharms sensor already owns a different declaration".into())
        };
    }
    let event = sound(world, definition.sound.as_deref())?;
    let player = world
        .resource::<super::player::PlayerState>()
        .player
        .as_ref()
        .and_then(|player| player.managed_entity())
        .ok_or("RuneCharms sensor requires actual Player")?;
    let scope: Arc<str> = Arc::from(format!("rune-charms:{player:?}"));
    if let Err(error) = super::audio::reserve_events(world, scope.clone(), event.iter().cloned()) {
        super::audio::release_events(world, &scope)?;
        return Err(error);
    }
    let mut owner = world.resource_mut::<Hearts>();
    owner.sensor_definition = Some(definition);
    owner.sensor_sound = event;
    owner.sensor_scope = Some(scope);
    Ok(true)
}
pub(super) fn update_frame(world: &mut World) -> Result<(), String> {
    let mode = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|owner| owner.player.as_ref())
        .is_some_and(|player| player.heart_default_mode());
    update(world, mode)
}
pub(super) fn register_pickable(
    world: &mut World,
    id: EntityId,
    item_type: u32,
    item: i32,
    invisible_npc_loot: bool,
) {
    initialize(world);
    world.resource_mut::<Hearts>().sensor.register_pickable(
        id,
        item_type,
        item,
        invisible_npc_loot,
    );
}
pub(super) fn unregister_pickable(world: &mut World, id: EntityId) {
    if let Some(mut owner) = world.get_resource_mut::<Hearts>() {
        owner.sensor.unregister(id)
    }
}
fn post(world: &mut World, id: u16, target: EntityId, payload: Payload) -> Result<(), String> {
    world
        .get_resource_mut::<EntityEvents>()
        .ok_or("Heart entity event owner absent")?
        .post(Event {
            id,
            target: Some(target),
            payload,
        });
    Ok(())
}
fn player(world: &World) -> Result<EntityId, String> {
    super::player_lifetime::id(world).ok_or_else(|| "Heart actual Player absent".into())
}
pub(super) fn equip(world: &mut World, id: EntityId) -> Result<(), String> {
    if !ready(world, id)? {
        return Err("Heart audio preparation is incomplete".into());
    }
    let mut owner = world.resource_mut::<Hearts>();
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("equipped Heart component absent")?;
    entry.runtime.equip();
    let event = entry.equip.as_ref().map(|event| event.id);
    drop(owner);
    if let Some(event) = event {
        world.write_message(super::audio::SoundEvent(Request::Post {
            event,
            emitter: Emitter::Equipment {
                entity: id,
                part: 8,
            },
        }));
    }
    post(world, 0x123, id, Payload::None)
}
pub(super) fn unequip(world: &mut World, id: EntityId) -> Result<(), String> {
    let Some(mut owner) = world.get_resource_mut::<Hearts>() else {
        return Ok(());
    };
    let Some(entry) = owner.entries.get_mut(&id) else {
        return Ok(());
    };
    if !entry.runtime.equipped {
        return Ok(());
    }
    entry.runtime.unequip();
    let event = entry.unequip.as_ref().map(|event| event.id);
    drop(owner);
    if let Some(event) = event {
        world.write_message(super::audio::SoundEvent(Request::Post {
            event,
            emitter: Emitter::Equipment {
                entity: id,
                part: 8,
            },
        }));
    }
    post(world, 0x124, id, Payload::None)?;
    let target = player(world)?;
    post(world, 0x11b, target, Payload::Bytes(vec![0]))
}
fn effect(world: &mut World, target: EntityId, effect: Effect) -> Result<(), String> {
    let (id, bytes) = match effect {
        Effect::Playrate(value) => (0x11d, value.to_le_bytes().to_vec()),
        Effect::TargetLost => (0x11b, vec![0]),
        Effect::Rumble(value) => {
            let mut bytes = Vec::with_capacity(12);
            for value in [value.low, value.high, value.duration] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            (0x11a, bytes)
        }
    };
    post(world, id, target, Payload::Bytes(bytes))
}
/// Run after actual Player view publication, only in the playing schedule.
/// `default_mode` is the actual state-machine mode802de0==0, not possession.
pub(super) fn update(world: &mut World, default_mode: bool) -> Result<(), String> {
    if world
        .get_resource::<GamePaused>()
        .is_some_and(|paused| paused.0)
    {
        return Ok(());
    }
    if !world.contains_resource::<Hearts>() {
        return Ok(());
    }
    let Some(player) = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return Ok(());
    };
    let target = player
        .managed_entity()
        .ok_or("Heart Player identity absent")?;
    let (eye, axis) = player.published_camera_frame();
    let clock = world.resource::<ClockRuntime>();
    if !clock.advanced() {
        return Ok(());
    }
    let now = clock.player_time().map_err(str::to_owned)?;
    let view = Mat4::from_cols(
        (-axis.y_axis).extend(0.),
        axis.z_axis.extend(0.),
        (-axis.x_axis).extend(0.),
        eye.extend(1.),
    );
    let mut owner = world
        .remove_resource::<Hearts>()
        .ok_or("Heart owner disappeared")?;
    let result = (|| {
        //Voice requests retain the actual equipment and target generations
        //captured by State66. A stale outgoing identity cannot select new gear.
        loop {
            let request = world
                .resource::<super::player::PlayerState>()
                .player
                .as_ref()
                .and_then(|player| player.heart_use_request());
            let Some(request) = request else { break };
            let entry = owner
                .entries
                .get_mut(&request.equipment)
                .ok_or("Heart use actor generation absent")?;
            if world
                .resource::<LiveEntities>()
                .get(request.equipment)
                .is_none()
            {
                return Err("Heart use actor is stale".into());
            }
            barks::request(world, request.equipment, request.target, entry)?;
            world
                .resource_mut::<super::player::PlayerState>()
                .player
                .as_mut()
                .ok_or("Heart use Player absent")?
                .acknowledge_heart_use();
        }
        for (&id, entry) in &mut owner.entries {
            if let Some(device) = entry.device.as_mut() {
                let dt = world
                    .resource::<ClockRuntime>()
                    .entity_delta(id)
                    .map_err(str::to_owned)?;
                barks::advance(world, id, device, dt)?;
            }
        }
        let speaking = owner
            .entries
            .values()
            .any(|entry| entry.runtime.equipped && entry.runtime.bark_playing);
        world
            .resource_mut::<super::player::PlayerState>()
            .player
            .as_mut()
            .ok_or("Heart status Player absent")?
            .heart_bark_status(speaking);
        let camera = owner
            .view
            .get_or_insert_with(|| world.query_filtered::<&Projection, With<Camera3d>>());
        let projection = camera
            .single(world)
            .map_err(|error| format!("RuneCharms view: {error}"))?
            .get_clip_from_view();
        let clip = projection * view.inverse();
        owner.sensor.update(
            eye,
            axis.x_axis,
            |id| {
                if world.resource::<LiveEntities>().get(id).is_none() {
                    return Ok(None);
                }
                if let Some(bounds) = super::runtime_entities::bounds(world, id)? {
                    return Ok(Some(bounds));
                }
                //8780d0 returns zero-size origin bounds when the real render
                //owner is NULL; absence of a model does not remove the sensor row.
                let live = world.resource::<LiveEntities>();
                let frame = world
                    .resource::<super::physics_scene::PhysicsScene>()
                    .entity_frame(live, id)?
                    .ok_or("RuneCharms actual entity frame absent")?;
                Ok(Some([frame.position; 2]))
            },
            |center| {
                let point = clip * center.extend(1.);
                (point.w > 0.).then(|| [(point.x / point.w), (point.y / point.w)])
            },
        )?;
        let equipped = owner.entries.values().any(|entry| entry.runtime.equipped);
        if equipped {
            //612c70 re-requests persistent67 when another state interrupted
            //it; an already active state never accumulates duplicate requests.
            if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
                if let Some(player) = state.player.as_mut() {
                    player.equip_heart_state(true);
                }
            }
        }
        if let Some(definition) = &owner.sensor_definition {
            if owner.sensor.ambient_beat(definition, equipped, now) {
                if let Some(event) = &owner.sensor_sound {
                    world.write_message(super::audio::SoundEvent(Request::Post {
                        event: event.id,
                        emitter: Emitter::PlayerBody,
                    }));
                }
                effect(world, target, Effect::Rumble(definition.rumble))?;
            }
        }
        for (&id, entry) in &mut owner.entries {
            if !entry.runtime.equipped {
                continue;
            }
            let entity_now = world
                .resource::<ClockRuntime>()
                .entity_time(id)
                .map_err(str::to_owned)?;
            entry
                .runtime
                .select_target(owner.sensor.rows.iter().map(|row| row.target), entity_now);
            entry.runtime.update(entity_now, default_mode);
            while let Some(pending) = entry.runtime.pending() {
                effect(world, target, pending)?;
                entry.runtime.acknowledge();
            }
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result
}
pub(super) fn begin_retirement(world: &mut World, id: EntityId) -> Result<(), String> {
    unequip(world, id)?;
    let Some(mut owner) = world.remove_resource::<Hearts>() else {
        return Ok(());
    };
    let cancelled = owner
        .entries
        .get_mut(&id)
        .and_then(|entry| entry.device.as_mut())
        .is_some_and(|device| device.begin_retirement(world, id));
    world.insert_resource(owner);
    if cancelled {
        post(world, 0xde, id, Payload::None)?;
    }
    Ok(())
}
pub(super) fn retired(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<Hearts>()
        .and_then(|owner| owner.entries.get(&id))
        .is_none_or(|entry| {
            !entry.runtime.bark_playing
                && entry.device.as_ref().is_none_or(|device| device.retired())
        })
}
pub(super) fn retire(world: &mut World, id: EntityId) -> Result<bool, String> {
    begin_retirement(world, id)?;
    //The normal playing update may already be stopped during teardown. Poll
    //actual Source End receipts here without advancing another gameplay frame.
    if let Some(mut owner) = world.remove_resource::<Hearts>() {
        let result = if let Some(entry) = owner.entries.get_mut(&id) {
            if let Some(device) = entry.device.as_mut() {
                barks::advance(world, id, device, 0.)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };
        world.insert_resource(owner);
        result?;
    }
    if !retired(world, id) {
        return Ok(false);
    }
    let Some(scope) = world
        .get_resource::<Hearts>()
        .and_then(|owner| owner.entries.get(&id))
        .map(|entry| entry.scope.clone())
    else {
        return Ok(true);
    };
    super::audio::release_events(world, &scope)?;
    world.resource_mut::<EntityEvents>().unregister(
        0xde,
        Some(id),
        Listener::HeartComponent { entity: id },
    );
    world.resource_mut::<Hearts>().entries.remove(&id);
    Ok(true)
}
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let ids = world
        .get_resource::<Hearts>()
        .map(|owner| owner.entries.keys().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    for id in ids {
        if !retire(world, id)? {
            return Ok(false);
        }
    }
    let scope = world
        .get_resource::<Hearts>()
        .and_then(|owner| owner.sensor_scope.clone());
    if let Some(scope) = scope {
        super::audio::release_events(world, &scope)?;
    }
    if let Some(player) = world
        .get_resource::<Hearts>()
        .and_then(|owner| owner.subscription)
    {
        world
            .get_resource_mut::<EntityEvents>()
            .ok_or("UseHeart listener shutdown owner absent")?
            .unregister(0x11d, Some(player), Listener::HeartState { player });
    }
    world.remove_resource::<Hearts>();
    Ok(true)
}
pub(super) fn startup_pending(world: &World) -> bool {
    let Some(owner) = world.get_resource::<Hearts>() else {
        return false;
    };
    owner
        .entries
        .keys()
        .any(|id| ready(world, *id).is_ok_and(|ready| !ready))
        || owner
            .sensor_scope
            .as_deref()
            .is_some_and(|scope| super::audio::events_ready(world, scope).is_ok_and(|ready| !ready))
}
pub(super) fn startup_error(world: &World) -> Option<String> {
    let owner = world.get_resource::<Hearts>()?;
    owner
        .entries
        .keys()
        .find_map(|id| ready(world, *id).err())
        .or_else(|| {
            owner
                .sensor_scope
                .as_deref()
                .and_then(|scope| super::audio::events_ready(world, scope).err())
        })
}
