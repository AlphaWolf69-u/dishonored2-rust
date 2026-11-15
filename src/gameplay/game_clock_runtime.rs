//! Loaded-game clock owner, using the original counter conversion, measured-FPS
//! producer and five-group update. This publishes real game-clock state, not a
//! Bevy elapsed-time substitute. The original identity-ordered gameplay view
//! clocks are published after gameplay and extracted for native rendering.
//! Camera construction, model lifecycle and save/time-stream transition
//! ownership remain separate integrations.
use crate::{
    game_clock::{
        self, ClockControls, ClockStepInputs, CounterMicroseconds, FrameTimingEstimator, GameClock,
    },
    game_settings,
};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};

use crate::platform::performance_counter as platform;
use crate::gameplay::entity_lifetime::EntityId;
use std::collections::HashMap;
use crate::gameplay::powers::world_effects::{TimeLease, TimeRequest, TimeRequests};

struct EntityClock { timespace: u32, offset: f64, remaining: Option<f32>, managed_override: bool }
impl EntityClock {
    fn advance(&mut self, clock: &GameClock, bent: bool, step: f32) {
        if !self.managed_override { return; }
        let unbent = if let Some(remaining) = self.remaining.as_mut() {
            let active = bent && step < *remaining;
            if *remaining < f32::from_bits(0x7149_f2ca) { *remaining -= step; }
            if !active { self.remaining = None; }
            active
        } else { false };
        if self.timespace > 1 { return; }
        let timespace = u32::from(unbent);
        if self.remaining.is_none() { self.managed_override = false; }
        if timespace == self.timespace { return; }
        let old = if self.timespace == 0 { 3 } else { 4 };
        let new = if timespace == 0 { 3 } else { 4 };
        let elapsed = (clock.previous_time(old).unwrap_or(0.0) - self.offset) as f32;
        self.offset = clock.previous_time(new).unwrap_or(0.0) - f64::from(elapsed);
        self.timespace = timespace;
    }
}

struct PerformanceCounter {
    origin: i64,
    frequency: i64,
    microseconds: CounterMicroseconds,
}

impl PerformanceCounter {
    fn new() -> Result<Self, String> {
        let frequency = platform::frequency()?;
        let origin = platform::counter()?;
        let mut microseconds = CounterMicroseconds::default();
        microseconds
            .read(0, frequency)
            .ok_or("QPC frequency has no native microsecond divisor")?;
        Ok(Self {
            origin,
            frequency,
            microseconds,
        })
    }
    fn elapsed_counter(&self) -> Result<i64, String> {
        Ok(platform::counter()?.wrapping_sub(self.origin))
    }
    fn microseconds(&mut self) -> Result<u64, String> {
        let counter = self.elapsed_counter()?;
        self.microseconds
            .read(counter, self.frequency)
            .ok_or_else(|| "counter scale unavailable".into())
    }
}

struct LoadedClock {
    clock: GameClock,
    source: PerformanceCounter,
    timing: FrameTimingEstimator,
    controls: ClockControls,
    /// Native common constructor 03e4c0 initializes +1ed8 to zero. Loaded
    /// normal gameplay has no save/time-stream override. 048940/048a50's
    /// transition I/O/callback lifecycle is not silently emulated here.
    common_counter: i32,
    advanced: bool,
    /// 80d8c0/80f040: the player root's timed unbent request. This is a
    /// duration in game-local unquantized time, not an extra time factor.
    player_override: Option<f32>,
    player_timespace: u32,
    player_clock_offset: f64,
    entity_clocks: HashMap<EntityId, EntityClock>,
    entity_overrides: Vec<EntityId>,
    power_requests: TimeRequests,
}

impl LoadedClock {
    /// A paused frame (`FUN_1408ad580`): 0450d0 still updates the frame
    /// estimator, but 8af320 (frame counter and 059990) is not called.
    fn paused(&mut self) -> Result<(), String> {
        self.timing.update(|| self.source.microseconds())?;
        self.advanced = false;
        Ok(())
    }

    fn advance(&mut self) -> Result<(), String> {
        // 0450d0 updates the frame estimator before running the game. 8af320
        // increments the clock's frame counter before invoking 059990.
        self.timing.update(|| self.source.microseconds())?;
        self.clock.increment_frame_counter();
        self.advanced = self.clock.step(ClockStepInputs {
            counter_since_origin: self.source.elapsed_counter()?,
            counter_frequency: self.source.frequency,
            variable_step: game_clock::allows_variable_step(self.common_counter),
            force_fixed_step: self.controls.force_fixed_step,
            fixed_step: game_clock::DEFAULT_STEP,
            jitter_window: self.controls.physics_step_tolerance,
            rate_cap: self.timing.frame_rate(),
            grow_step_limit: self.controls.limit_step_variation,
        });
        if self.advanced {
            let bent = self.clock.rates(3).is_some_and(|v| v[0] < 1.0)
                || self.clock.rates(4).is_some_and(|v| v[0] < 1.0);
            // 8af320 passes clock group4 to 80f040, including the
            // current unbent scaling; it is not the wall-clock delta.
            let step = self.clock.delta(4).unwrap_or(0.0);
            let unbent = if let Some(remaining) = self.player_override.as_mut() {
                let active = bent && step < *remaining;
                if *remaining < f32::from_bits(0x7149_f2ca) { *remaining -= step; }
                if !active { self.player_override = None; }
                active
            } else { false };
            let timespace = u32::from(unbent);
            if timespace != self.player_timespace {
                // 87d050 (and its 87ce70 counterpart): retain the entity's
                // local elapsed time while switching native clock groups.
                let previous_group = if self.player_timespace == 0 { 3 } else { 4 };
                let next_group = if timespace == 0 { 3 } else { 4 };
                let previous = self.clock.previous_time(previous_group).unwrap_or(0.0);
                let next = self.clock.previous_time(next_group).unwrap_or(0.0);
                let elapsed = (previous - self.player_clock_offset) as f32;
                self.player_clock_offset = next - f64::from(elapsed);
                self.player_timespace = timespace;
            }
            let clocks = &mut self.entity_clocks;
            self.entity_overrides.retain(|id| {
                let Some(entity) = clocks.get_mut(id) else { return false };
                entity.advance(&self.clock, bent, step);
                entity.managed_override
            });
        }
        Ok(())
    }
}

/// Actual main-world owner available to the native scene publisher. A platform
/// or configuration error stays unavailable: no zero-clock/Instant fallback.
#[derive(Resource)]
pub struct ClockRuntime(Result<LoadedClock, String>);

impl ClockRuntime {
    pub fn clock(&self) -> Result<&GameClock, &str> {
        self.0
            .as_ref()
            .map(|state| &state.clock)
            .map_err(String::as_str)
    }
    pub fn advanced(&self) -> bool {
        self.0.as_ref().is_ok_and(|state| state.advanced)
    }
    /// 140059890 publication, using the existing byte-exact easing owner.
    pub fn set_rate_target(&mut self, group: usize, value: f32, ease: f32) -> Result<(), String> {
        self.0.as_mut().map_err(|error| error.clone())?.clock.set_rate_target(group, value, ease)
    }
    pub fn acquire_time_request(&mut self, request: TimeRequest) -> Result<TimeLease, String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        let (lease, target) = state.power_requests.acquire(request);
        state.clock.set_rate_target(3, target.world, target.ease)?;
        state.clock.set_rate_target(4, target.unbent, target.ease)?;
        Ok(lease)
    }
    pub fn release_time_request(&mut self, lease: TimeLease, ease: f32) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        if let Some(target) = state.power_requests.release(lease, ease) {
            state.clock.set_rate_target(3, target.world, target.ease)?;
            state.clock.set_rate_target(4, target.unbent, target.ease)?;
        }
        Ok(())
    }
    /// World retirement invalidates every source-owned request and entity
    /// identity before another map can use this persistent clock session.
    pub fn clear_power_time(&mut self) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        state.power_requests = TimeRequests::default();
        state.entity_overrides.clear(); state.entity_clocks.clear();
        state.player_override = None; state.player_timespace = 0; state.player_clock_offset = 0.0;
        state.clock.set_rate_target(3, 1.0, 0.0)?;
        state.clock.set_rate_target(4, 1.0, 0.0)
    }
    /// 14080d8c0: an indefinite request cannot be replaced by a shorter
    /// request until the last world-time lease has restored normal time.
    pub fn set_player_time_override(&mut self, duration: f32) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        if state.player_override != Some(f32::from_bits(0x7149_f2ca)) { state.player_override = Some(duration); }
        Ok(())
    }
    pub fn player_timespace(&self) -> Result<u32, &str> {
        self.0.as_ref().map(|state| state.player_timespace).map_err(String::as_str)
    }
    pub fn player_delta(&self) -> Result<f32, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        state.clock.entity_delta(state.player_timespace).ok_or("player clock timespace is invalid")
    }
    pub fn player_time(&self) -> Result<f64, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let group = if state.player_timespace == 0 { 3 } else { 4 };
        Ok(state.clock.time(group).ok_or("player clock group is absent")? - state.player_clock_offset)
    }
    /// Actual entity roots register once, then their component/model owners
    /// query this same timespace selection. No entity is fabricated here.
    pub fn register_entity_time(&mut self, entity: EntityId, timespace: u32) -> Result<(), String> {
        if timespace > 3 { return Err("entity timespace is outside 0..4".into()); }
        self.0.as_mut().map_err(|e| e.clone())?.entity_clocks.entry(entity)
            .or_insert(EntityClock { timespace, offset: 0.0, remaining: None, managed_override: false });
        Ok(())
    }
    pub fn unregister_entity_time(&mut self, entity: EntityId) {
        if let Ok(state) = &mut self.0 {
            state.entity_clocks.remove(&entity); state.entity_overrides.retain(|id| *id != entity);
        }
    }
    pub fn set_entity_time_override(&mut self, entity: EntityId, duration: f32) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        let record = state.entity_clocks.get_mut(&entity).ok_or("entity clock root is not registered")?;
        if record.remaining != Some(f32::from_bits(0x7149_f2ca)) { record.remaining = Some(duration); }
        record.managed_override = true;
        if !state.entity_overrides.contains(&entity) { state.entity_overrides.push(entity); }
        Ok(())
    }
    pub fn entity_delta(&self, entity: EntityId) -> Result<f32, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let record = state.entity_clocks.get(&entity).ok_or("entity clock root is not registered")?;
        state.clock.entity_delta(record.timespace).ok_or("entity timespace is invalid")
    }
    pub fn entity_time(&self, entity: EntityId) -> Result<f64, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let record = state.entity_clocks.get(&entity).ok_or("entity clock root is not registered")?;
        let group = [3, 4, 2, 0][record.timespace as usize];
        Ok(state.clock.time(group).ok_or("entity clock group is absent")? - record.offset)
    }
}

impl FromWorld for ClockRuntime {
    fn from_world(_: &mut World) -> Self {
        let state = (|| {
            let (path, controls) = game_settings::read_game_clock_controls()?;
            let source = PerformanceCounter::new()?;
            let mut clock = GameClock::from_bytes([0; game_clock::STATE_SIZE]);
            clock.initialize(); // Opaque owned padding is not given field meanings.
            info!(
                "Game clock: QPC {} Hz, controls {:?} from {} (registered defaults where absent)",
                source.frequency,
                controls,
                path.display()
            );
            Ok(LoadedClock {
                clock,
                source,
                timing: FrameTimingEstimator::default(),
                controls,
                common_counter: 0,
                advanced: false,
                player_override: None,
                player_timespace: 0,
                player_clock_offset: 0.0,
                entity_clocks: HashMap::new(),
                entity_overrides: Vec::new(),
                power_requests: TimeRequests::default(),
            })
        })();
        if let Err(error) = &state {
            error!("Game clock unavailable: {error}");
        }
        Self(state)
    }
}

/// A coherent original gameplay-view clock snapshot. All models in one
/// publication use the same five native seconds/deltas, not per-draw QPC reads.
/// Missing owner state remains an error, never a zero or Bevy-clock fallback.
#[derive(Resource, Clone, ExtractResource)]
pub struct GameplayViewClock(Result<game_clock::GameplayViewClocks, String>);

impl GameplayViewClock {
    pub fn clocks(&self) -> Result<&game_clock::GameplayViewClocks, &str> {
        self.0.as_ref().map_err(String::as_str)
    }
    /// Actual world/view bindings, distinct from a render model's offset
    /// and selected clock. Keep the source definition byte4, not Time2->2.
    pub fn view_shader_times(
        &self,
        source_flag_04: u8,
    ) -> Result<game_clock::ViewShaderTimes, String> {
        Ok(self
            .clocks()
            .map_err(str::to_owned)?
            .view_shader_times(source_flag_04))
    }

    pub fn model_local_time(&self, offset: f32, model_flags_1c31: u8) -> Result<[f32; 2], String> {
        self.clocks()
            .map_err(str::to_owned)?
            .model_local_time(offset, model_flags_1c31)
            .ok_or_else(|| {
                format!(
                    "model clock group {} is outside the five gameplay clocks",
                    crate::render_model::model_shader_time_group(model_flags_1c31)
                )
            })
    }
}

impl FromWorld for GameplayViewClock {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource::<ClockRuntime>()
                .clock()
                .map(game_clock::GameplayViewClocks::from_clock)
                .map_err(str::to_owned),
        )
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClockSystems {
    Advance,
    PublishView,
}

fn publish_gameplay_view_clock(owner: Res<ClockRuntime>, mut view: ResMut<GameplayViewClock>) {
    view.0 = owner
        .clock()
        .map(game_clock::GameplayViewClocks::from_clock)
        .map_err(str::to_owned);
}

/// `idGameLocal::IsPaused` (vtable slot 29, `FUN_1408a89b0`): the top game
/// logic state's decl `m_pause` (or `gameLocal+0xc`). While set, the game
/// frame renders the world without simulating it.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct GamePaused(pub bool);

fn advance_clock(mut owner: ResMut<ClockRuntime>, paused: Res<GamePaused>) {
    if let Ok(state) = &mut owner.0 {
        let result = if paused.0 { state.paused() } else { state.advance() };
        if let Err(error) = result {
            error!("Game clock unavailable: {error}");
            owner.0 = Err(error);
        }
    }
}

pub struct ClockRuntimePlugin;
impl Plugin for ClockRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClockRuntime>()
            .init_resource::<GamePaused>()
            .init_resource::<GameplayViewClock>()
            .add_plugins(ExtractResourcePlugin::<GameplayViewClock>::default())
            .add_systems(First, advance_clock.in_set(ClockSystems::Advance))
            .add_systems(
                PostUpdate,
                publish_gameplay_view_clock.in_set(ClockSystems::PublishView),
            );
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn loaded_clock_owner_uses_actual_qpc_without_window_or_bevy_elapsed() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ClockRuntimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        let owner = app.world().resource::<ClockRuntime>();
        let before = owner.clock().unwrap().time(3).unwrap();
        assert!(owner.advanced());
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        let owner = app.world().resource::<ClockRuntime>();
        let clock = owner.clock().unwrap();
        assert!(clock.time(3).unwrap() > before);
        assert!(clock.delta(3).unwrap() > 0.0);
        assert!(owner.advanced());
        let view = app.world().resource::<GameplayViewClock>();
        for group in 0..game_clock::GROUP_COUNT {
            let actual = view.model_local_time(0.5, (group as u8) << 4).unwrap();
            let expected = crate::render_model::model_shader_local_time(
                0.5,
                clock.time(group).unwrap(),
                clock.delta(group).unwrap(),
            );
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        }
        assert!(view.model_local_time(0.0, 0x50).is_err());
        assert_eq!(
            app.world().resource::<Time>().elapsed(),
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn gameplay_view_clock_preserves_unavailable_owner() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ClockRuntimePlugin);
        app.world_mut()
            .insert_resource(ClockRuntime(Err("test unavailable native clock".into())));
        app.update();
        let view = app.world().resource::<GameplayViewClock>();
        assert_eq!(view.clocks().unwrap_err(), "test unavailable native clock");
        assert_eq!(
            view.view_shader_times(game_clock::RESET_VIEW_LOCAL_TIME_FLAG)
                .unwrap_err(),
            "test unavailable native clock"
        );
        assert_eq!(
            view.model_local_time(0.0, 0x30).unwrap_err(),
            "test unavailable native clock"
        );
    }

    #[test]
    #[ignore = "requires a graphics adapter; verifies native main-world publication/render-world extraction"]
    fn gameplay_view_clock_is_extracted_after_publication() {
        use bevy::render::RenderApp;
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        )
        .add_plugins(ClockRuntimePlugin);
        app.finish();
        app.cleanup();
        app.update();
        let main = app
            .world()
            .resource::<GameplayViewClock>()
            .clocks()
            .unwrap();
        let render = app
            .sub_app(RenderApp)
            .world()
            .resource::<GameplayViewClock>()
            .clocks()
            .unwrap();
        let mut main_bytes = [0; 0x3c];
        let mut render_bytes = [0; 0x3c];
        main.write_game_result(&mut main_bytes).unwrap();
        render.write_game_result(&mut render_bytes).unwrap();
        assert_eq!(main_bytes, render_bytes);
        let game = app.world().resource::<ClockRuntime>().clock().unwrap();
        for group in 0..game_clock::GROUP_COUNT {
            assert_eq!(
                &main_bytes[group * 8..group * 8 + 8],
                &game.time(group).unwrap().to_le_bytes()
            );
            assert_eq!(
                &main_bytes[0x28 + group * 4..0x2c + group * 4],
                &game.delta(group).unwrap().to_le_bytes()
            );
        }
    }
}
