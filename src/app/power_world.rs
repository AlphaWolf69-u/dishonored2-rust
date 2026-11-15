//! Bend Time's loaded-world adapter. The real five-channel clock owns easing;
//! the timer decrements with its spawner's entity clock (779010, mode2).
//! Immutable sounds retain leases from the existing sound declaration owner.
use std::sync::Arc;
use bevy::prelude::*;
use crate::{assets::{archive::Catalog, power_journal::Power, wwise::SoundEventDeclaration}, gameplay::{
    entity_events::{EntityEvents, Event, Payload}, entity_lifetime::EntityId,
    game_clock_runtime::{ClockRuntime, GamePaused},
    powers::world_effects::{BendTime, BendTimeAttributes, BendTimeSettings, BendTimeWorld, TimeLease, TimeRequest},
    sound::{Emitter, Request},
}};

#[derive(Resource)]
pub(super) struct PowerWorld {
    catalog: u64,
    bend_time: Option<BendTime>,
    state: State,
    retired: bool,
}
/// Actual owned Bend Time acquisition; no live clock request, timer or player
/// binding exists on the source worker. Sounds retain their canonical leases.
pub(super) struct PreparedSources {
    catalog: u64,
    settings: BendTimeSettings,
    sounds: Vec<(String, Arc<SoundEventDeclaration>)>,
}
impl PreparedSources {
    pub fn acquire_power(catalog:&Catalog,power:&Power)->Result<Self,String> {
        if crate::assets::power_journal::power_id(&power.power_id)!=Some(1) {
            return Err("Bend Time acquisition requires actual power id1".into());
        }
        let settings=BendTimeSettings::read(&power.component)?;
        let mut sounds=Vec::new();
        for name in [&settings.start_sound,&settings.stop_sound].into_iter().flatten() {
            if sounds.iter().any(|(path,_)|path==name) { continue; }
            let source=crate::assets::fx::SoundEventAsset { delay:[0.;2],event:Some(name.clone()) }
                .resolve(catalog)?.ok_or_else(||format!("Bend Time sound {name} has no declaration"))?;
            sounds.push((name.clone(),source));
        }
        Ok(Self { catalog:catalog.identity(),settings,sounds })
    }
    /// The inventory predicate runs before any family sound acquisition.
    pub fn acquire_owned<'a>(catalog:&Catalog,powers:impl IntoIterator<Item=&'a Power>,
        mut owns_item:impl FnMut(&str)->bool)->Result<Option<Self>,String> {
        let Some(power)=powers.into_iter().find(|power|
            crate::assets::power_journal::power_id(&power.power_id)==Some(1)&&owns_item(&power.item)) else { return Ok(None) };
        Self::acquire_power(catalog,power).map(Some)
    }
    pub fn effect_paths(&self)->impl Iterator<Item=&str> { std::iter::empty() }
}
struct State {
    player: Option<EntityId>,
    timer: Option<f32>,
    /// Actual NPC roots registered by their live component owner; map
    /// placements are never admitted as invented targets.
    npcs: Vec<EntityId>,
    sounds: Vec<(String, Arc<SoundEventDeclaration>)>,
    /// A depleted component's accepted stop request may still be queued or
    /// playing. Retain the canonical declarations until audio acknowledges it.
    retired_sounds: Vec<Arc<SoundEventDeclaration>>,
    pending_sounds: Vec<Request>,
    active: bool,
}
impl PowerWorld {
    pub fn empty(catalog:&Catalog)->Self {
        Self { catalog:catalog.identity(),bend_time:None,state:State { player:None,timer:None,
            npcs:Vec::new(),sounds:Vec::new(),retired_sounds:Vec::new(),pending_sounds:Vec::new(),active:false },retired:false }
    }
    /// Existing live state always wins: reacquisition never swaps out its
    /// active clock lease, original timer slot, cancellation flag or sounds.
    pub fn absorb(&mut self,sources:PreparedSources)->Result<(),String> {
        if self.retired||self.catalog!=sources.catalog { return Err("Bend Time sources belong to a retired world".into()); }
        if self.bend_time.is_none() {
            self.bend_time=Some(BendTime::new(sources.settings));
            self.state.sounds=sources.sounds;
        }
        Ok(())
    }
    /// Quantity loss does not invent an effect cancellation. The caller may
    /// release these idle sources only after the canonical audio consumer has
    /// acknowledged the actual requests/tails (never after a frame timeout).
    pub fn release_unowned(&mut self,actual_quantity:u32,audio_retired:bool)->bool {
        if actual_quantity!=0||!audio_retired||self.state.active||self.state.timer.is_some()
            ||!self.state.pending_sounds.is_empty()||self.bend_time.as_ref().is_some_and(BendTime::active) { return false; }
        self.bend_time=None;
        self.state.sounds.clear();
        self.state.retired_sounds.clear();
        true
    }
    /// Actual inventory-driven component destruction: virtual110 at
    ///141b9b748 dispatches61c4a0, which deactivates61cd60 before61bcc0's
    ///cast cancellation. This is not an ordinary equip-selection callback.
    /// Preserve this reusable world owner, real player/NPC bindings and every
    ///queued stop sound; acquisition may install a fresh component later.
    pub fn release_owned_sources(&mut self,clock:&mut ClockRuntime,events:&mut EntityEvents,
        position:Vec3)->Result<(),String> {
        if self.bend_time.as_ref().is_some_and(BendTime::active) {
            clock.clock().map_err(str::to_owned)?;
            if self.state.player.is_none() { return Err("Bend Time destruction has no actual player owner".into()); }
        }
        if let Some(power)=&mut self.bend_time {
            power.stop(&mut Host { state:&mut self.state,clock,events,position });
        }
        self.bend_time=None;
        self.state.timer=None;
        self.state.active=false;
        for (_,source) in self.state.sounds.drain(..) {
            if !self.state.retired_sounds.iter().any(|old|Arc::ptr_eq(old,&source)) { self.state.retired_sounds.push(source); }
        }
        Ok(())
    }
    /// Only a real canonical audio completion may release these tails. Map
    /// teardown already retains the whole resource until that same drain.
    pub fn retire_released_sounds(&mut self,audio_retired:bool) {
        if audio_retired&&self.state.pending_sounds.is_empty() { self.state.retired_sounds.clear(); }
    }
    /// Compatibility for an already ownership-filtered subset. Ordinary map
    /// setup uses empty(); the owned source worker supplies absorb() later.
    pub fn prepare(catalog: &Catalog, powers: &[Power]) -> Result<Self, String> {
        let mut owner=Self::empty(catalog);
        if let Some(power)=powers.iter().find(|power|crate::assets::power_journal::power_id(&power.power_id)==Some(1)) {
            owner.absorb(PreparedSources::acquire_power(catalog,power)?)?;
        }
        Ok(owner)
    }
    /// Bind only the registered player identity from the real entity owner.
    pub fn bind_player(&mut self, entity: EntityId) {
        if !self.retired { self.state.player = Some(entity); }
    }
    pub fn register_npc(&mut self, entity: EntityId, clock: &mut ClockRuntime) -> Result<(), String> {
        if self.state.npcs.contains(&entity) { return Ok(()); }
        clock.register_entity_time(entity, 0)?;
        self.state.npcs.push(entity); Ok(())
    }
    pub fn unregister_npc(&mut self, entity: EntityId, clock: &mut ClockRuntime) {
        self.state.npcs.retain(|id| *id != entity); clock.unregister_entity_time(entity);
    }
    pub fn bend_time_available(&self, clock: &ClockRuntime) -> bool {
        !self.retired && self.bend_time.is_some() && self.state.player.is_some() && clock.clock().is_ok()
    }
    pub fn bend_time_active(&self) -> bool { self.state.active }
    pub fn bend_time_must_cancel(&self) -> bool { self.bend_time.as_ref().is_some_and(BendTime::must_cancel) }
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
        self.state.timer = None; self.state.player = None; self.state.active = false;
    }
}
struct Host<'a> { state: &'a mut State, clock: &'a mut ClockRuntime, events: &'a mut EntityEvents, position: Vec3 }
impl BendTimeWorld for Host<'_> {
    fn available(&self) -> bool { self.state.player.is_some() && self.clock.clock().is_ok() }
    fn acquire_time(&mut self, request: TimeRequest) -> TimeLease {
        // Both channel indices are native, and validated statically here.
        // Errors indicate loss of the actual clock owner; admission checked
        // that owner in this synchronous operation, never a fallback clock.
        self.clock.acquire_time_request(request).expect("admitted clock owner")
    }
    fn release_time(&mut self, lease: TimeLease, ease: f32) {
        self.clock.release_time_request(lease, ease).expect("admitted clock owner");
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
    live: Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>, paused: Res<GamePaused>) {
    let (Some(owner), Some(events), Some(player)) = (owner.as_deref_mut(), events.as_deref_mut(), player.player.as_ref()) else { return };
    if owner.retired || !owner.state.player.is_some_and(|id| live.as_deref().is_some_and(|live| live.get(id).is_some())) { return; }
    let cast_active = player.power_cast_active(1);
    owner.tick(&mut clock, events, player.feet(), cast_active, paused.0);
}

/// Call before player identity, physics and audio retirement. It keeps the
/// same sound declaration leases resident until LevelLifetime's canonical
/// audio acknowledgement removes all level resources. Errors restore every
/// owner and cannot silently lose a running clock/timer request.
pub(super) fn retire(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<PowerWorld>() else {
        if let Some(mut clock) = world.get_resource_mut::<ClockRuntime>() { clock.clear_power_time()?; }
        return Ok(());
    };
    if owner.retired { world.insert_resource(owner); return Ok(()); }
    let Some(mut clock) = world.remove_resource::<ClockRuntime>() else {
        world.insert_resource(owner); return Err("power retirement has no clock owner".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(clock); world.insert_resource(owner);
        return Err("power retirement has no entity event owner".into());
    };
    let result = (|| {
        clock.clock().map_err(str::to_owned)?;
        if let Some(id) = owner.state.player {
            let live = world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .ok_or("power retirement has no managed entity owner")?;
            if live.get(id).is_none() { return Err("power retirement player generation is stale".into()); }
        }
        if !owner.state.pending_sounds.is_empty() || owner.state.active {
            if !world.contains_resource::<Messages<super::audio::SoundEvent>>() {
                return Err("power retirement has no canonical audio message owner".into());
            }
        }
        let position = world.get_resource::<super::player::PlayerState>()
            .and_then(|state| state.player.as_ref()).map_or(Vec3::ZERO, |player| player.feet());
        owner.clear(&mut clock, &mut events, position);
        clock.clear_power_time()?;
        for request in owner.state.pending_sounds.drain(..) { world.write_message(super::audio::SoundEvent(request)); }
        owner.retired = true;
        Ok(())
    })();
    world.insert_resource(events); world.insert_resource(clock); world.insert_resource(owner);
    result
}
