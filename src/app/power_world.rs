//! Bend Time's loaded-world adapter. The real five-channel clock owns easing;
//! the timer decrements with its spawner's entity clock (779010, mode2).
//! Immutable sounds retain leases from the existing sound declaration owner.
use std::sync::Arc;
use bevy::prelude::*;
use crate::{assets::{archive::Catalog, power_journal::Power, wwise::SoundEventDeclaration}, gameplay::{
    entity_events::{EntityEvents, Event, Payload}, entity_lifetime::EntityId,
    game_clock_runtime::{ClockRuntime, GamePaused},
    powers::world_effects::{BendTime, BendTimeAttributes, BendTimeSettings, BendTimeWorld, TimeLease, TimeRequest, TimeRequests},
    sound::{Emitter, Request},
}};

#[derive(Resource)]
pub(super) struct PowerWorld {
    bend_time: Option<BendTime>,
    state: State,
}
struct State {
    player: Option<EntityId>,
    requests: TimeRequests,
    timer: Option<f32>,
    /// Actual NPC roots registered by their live component owner; map
    /// placements are never admitted as invented targets.
    npcs: Vec<EntityId>,
    sounds: Vec<(String, Arc<SoundEventDeclaration>)>,
    pending_sounds: Vec<Request>,
    active: bool,
}
impl PowerWorld {
    pub fn prepare(catalog: &Catalog, powers: &[Power]) -> Result<Self, String> {
        let settings = powers.iter().find(|power| crate::assets::power_journal::power_id(&power.power_id) == Some(1))
            .map(|power| BendTimeSettings::read(&power.component)).transpose()?;
        let mut sounds = Vec::new();
        if let Some(settings) = &settings {
            for name in [&settings.start_sound, &settings.stop_sound].into_iter().flatten() {
                if sounds.iter().any(|(path, _)| path == name) { continue; }
                let source = crate::assets::fx::SoundEventAsset { delay: [0.0; 2], event: Some(name.clone()) }
                    .resolve(catalog)?.ok_or_else(|| format!("Bend Time sound {name} has no declaration"))?;
                sounds.push((name.clone(), source));
            }
        }
        Ok(Self { bend_time: settings.map(BendTime::new), state: State { player: None,
            requests: TimeRequests::default(), timer: None, npcs: Vec::new(), sounds,
            pending_sounds: Vec::new(), active: false } })
    }
    /// Bind only the registered player identity from the real entity owner.
    pub fn bind_player(&mut self, entity: EntityId) { self.state.player = Some(entity); }
    pub fn register_npc(&mut self, entity: EntityId, clock: &mut ClockRuntime) -> Result<(), String> {
        if self.state.npcs.contains(&entity) { return Ok(()); }
        clock.register_entity_time(entity, 0)?;
        self.state.npcs.push(entity); Ok(())
    }
    pub fn unregister_npc(&mut self, entity: EntityId, clock: &mut ClockRuntime) {
        self.state.npcs.retain(|id| *id != entity); clock.unregister_entity_time(entity);
    }
    pub fn bend_time_available(&self, clock: &ClockRuntime) -> bool {
        self.bend_time.is_some() && self.state.player.is_some() && clock.clock().is_ok()
    }
    pub fn bend_time_active(&self) -> bool { self.state.active }
    pub fn bend_time_cost(&self, normal: f32) -> f32 { self.bend_time.as_ref().map_or(normal, |power| power.mana_cost(normal)) }
    pub fn begin_bend_time_cast(&mut self) { if let Some(power) = &mut self.bend_time { power.begin_cast(); } }
    pub fn cast_bend_time(&mut self, attributes: BendTimeAttributes, held: bool, pay: impl FnMut(),
        clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3) -> bool {
        let Some(power) = self.bend_time.as_mut() else { return false };
        let mut host = Host { state: &mut self.state, clock, events, position };
        power.cast(&mut host, attributes, held, pay)
    }
    pub fn finish_bend_time_cast(&mut self, attributes: BendTimeAttributes,
        clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3) {
        if let Some(power) = &mut self.bend_time {
            power.finish_cast(&mut Host { state: &mut self.state, clock, events, position }, attributes);
        }
    }
    pub fn update_bend_time_hold(&mut self, attributes: BendTimeAttributes, held: bool, pressure: f32,
        deadzone: f32, cast_active: bool, cast_committed: bool, pay: impl FnMut(),
        clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3) {
        if let Some(power) = &mut self.bend_time {
            power.update_hold(&mut Host { state: &mut self.state, clock, events, position }, attributes,
                held, pressure, deadzone, cast_active, cast_committed, pay);
        }
    }
    pub fn attributes_changed(&mut self, attributes: BendTimeAttributes,
        clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3) {
        if let Some(power) = &mut self.bend_time {
            power.attributes_changed(&mut Host { state: &mut self.state, clock, events, position }, attributes);
        }
    }
    pub fn tick(&mut self, clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3,
        cast_active: bool, paused: bool) {
        if paused || !clock.advanced() { return; }
        // Native timer mode2 uses the spawner entity's own timespace. Do
        // not use wall time, Bevy Time, or the stopped world's group3 delta.
        if let (Some(timer), Ok(dt)) = (self.state.timer.as_mut(), clock.player_delta()) {
            *timer -= dt;
            if *timer <= 0.0 {
                self.state.timer = None;
                if let Some(power) = &mut self.bend_time {
                    power.timer_expired(&mut Host { state: &mut self.state, clock, events, position }, cast_active, false);
                }
            }
        }
    }
    pub fn clear(&mut self, clock: &mut ClockRuntime, events: &mut EntityEvents, position: Vec3) {
        if let Some(power) = &mut self.bend_time {
            power.stop(&mut Host { state: &mut self.state, clock, events, position });
        }
        for entity in self.state.npcs.drain(..) { clock.unregister_entity_time(entity); }
        self.state.timer = None; self.state.player = None;
    }
}
struct Host<'a> { state: &'a mut State, clock: &'a mut ClockRuntime, events: &'a mut EntityEvents, position: Vec3 }
impl BendTimeWorld for Host<'_> {
    fn available(&self) -> bool { self.state.player.is_some() && self.clock.clock().is_ok() }
    fn acquire_time(&mut self, request: TimeRequest) -> TimeLease {
        let (lease, target) = self.state.requests.acquire(request);
        // Both channel indices are native, and validated statically here.
        // Errors indicate loss of the actual clock owner; admission checked
        // that owner in this synchronous operation, never a fallback clock.
        self.clock.set_rate_target(3, target.world, target.ease).expect("admitted clock owner");
        self.clock.set_rate_target(4, target.unbent, target.ease).expect("admitted clock owner");
        lease
    }
    fn release_time(&mut self, lease: TimeLease, ease: f32) {
        if let Some(target) = self.state.requests.release(lease, ease) {
            self.clock.set_rate_target(3, target.world, target.ease).expect("admitted clock owner");
            self.clock.set_rate_target(4, target.unbent, target.ease).expect("admitted clock owner");
        }
    }
    fn start_timer(&mut self, duration: f32) { self.state.timer = Some(duration); }
    fn cancel_timer(&mut self) { self.state.timer = None; }
    fn set_player_time_override(&mut self, duration: f32) {
        self.clock.set_player_time_override(duration).expect("admitted clock owner");
    }
    fn ease_out_stopped_npcs(&mut self, duration: f32) {
        for entity in &self.state.npcs {
            self.clock.set_entity_time_override(*entity, duration).expect("registered NPC clock root");
        }
    }
    fn message(&mut self, id: u32) {
        self.events.post(Event { id: id as u16, target: self.state.player, payload: Payload::None });
    }
    fn usage_event(&mut self, kind: u32) {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&kind.to_le_bytes());
        for value in self.position.to_array() { bytes.extend_from_slice(&value.to_le_bytes()); }
        self.events.post(Event { id: 0x73, target: self.state.player, payload: Payload::Bytes(bytes) });
    }
    fn sound(&mut self, name: &str) {
        if let Some((_, sound)) = self.state.sounds.iter().find(|(path, _)| path == name) {
            self.state.pending_sounds.push(Request::Post { event: sound.id, emitter: Emitter::PlayerBody });
        }
    }
    fn power_state(&mut self, active: bool) { self.state.active = active; }
}
/// Shared audio playback retains the prepared event leases in PowerWorld;
/// pending posts do not acquire archive data in the gameplay thread.
pub(super) fn publish_sounds(mut owner: Option<ResMut<PowerWorld>>, mut sounds: MessageWriter<super::audio::SoundEvent>, paused: Res<GamePaused>) {
    if paused.0 { return; }
    let Some(owner) = owner.as_deref_mut() else { return };
    for request in owner.state.pending_sounds.drain(..) { sounds.write(super::audio::SoundEvent(request)); }
}

/// Schedule after ClockSystems::Advance and before the player frame. The
/// timer uses the same coherent clock publication that frame will consume.
pub(super) fn update(mut owner: Option<ResMut<PowerWorld>>, mut clock: ResMut<ClockRuntime>,
    mut events: Option<ResMut<EntityEvents>>, player: Res<super::player::PlayerState>,
    powers: Option<Res<super::powers::Powers>>, paused: Res<GamePaused>) {
    let (Some(owner), Some(events), Some(player)) = (owner.as_deref_mut(), events.as_deref_mut(), player.player.as_ref()) else { return };
    let cast_active = powers.as_deref().and_then(|powers| powers.selected())
        .is_some_and(|cast| cast.settings.id == 1 && cast.phase() != crate::gameplay::powers::Phase::Idle);
    owner.tick(&mut clock, events, player.feet(), cast_active, paused.0);
}
