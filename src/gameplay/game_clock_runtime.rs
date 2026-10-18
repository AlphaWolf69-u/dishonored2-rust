//! Loaded-game clock owner, using the original counter conversion, measured-FPS
//! producer and five-group update. This publishes real game-clock state, not a
//! Bevy elapsed-time substitute. The original identity-ordered gameplay view
//! clocks are published after gameplay and extracted for native rendering.
//! Camera construction, model lifecycle and save/time-stream transition
//! ownership remain separate integrations.
use crate::{
    game_clock::{
        self, ClockControls, ClockStepInputs, CounterMicroseconds, FrameTimingEstimator,
        NativeGameClock,
    },
    game_settings,
};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};

#[cfg(target_os = "windows")]
mod platform {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryPerformanceCounter(value: *mut i64) -> i32;
        fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    }
    pub fn counter() -> Result<i64, String> {
        let mut value = 0;
        if unsafe { QueryPerformanceCounter(&mut value) } == 0 {
            return Err(format!(
                "QueryPerformanceCounter failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(value)
    }
    pub fn frequency() -> Result<i64, String> {
        let mut value = 0;
        if unsafe { QueryPerformanceFrequency(&mut value) } == 0 || value <= 0 {
            return Err(format!(
                "QueryPerformanceFrequency failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(value)
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    pub fn counter() -> Result<i64, String> {
        Err("native QPC clock requires Windows".into())
    }
    pub fn frequency() -> Result<i64, String> {
        Err("native QPC clock requires Windows".into())
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
            .ok_or_else(|| "native counter scale unavailable".into())
    }
}

struct LoadedClock {
    clock: NativeGameClock,
    source: PerformanceCounter,
    timing: FrameTimingEstimator,
    controls: ClockControls,
    /// Native common constructor 03e4c0 initializes +1ed8 to zero. Loaded
    /// normal gameplay has no save/time-stream override. 048940/048a50's
    /// transition I/O/callback lifecycle is not silently emulated here.
    common_counter: i32,
    advanced: bool,
}

impl LoadedClock {
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
        Ok(())
    }
}

/// Actual main-world owner available to the native scene publisher. A platform
/// or configuration error stays unavailable: no zero-clock/Instant fallback.
#[derive(Resource)]
pub struct NativeClockRuntime(Result<LoadedClock, String>);

impl NativeClockRuntime {
    pub fn clock(&self) -> Result<&NativeGameClock, &str> {
        self.0
            .as_ref()
            .map(|state| &state.clock)
            .map_err(String::as_str)
    }
    pub fn advanced(&self) -> bool {
        self.0.as_ref().is_ok_and(|state| state.advanced)
    }
}

impl FromWorld for NativeClockRuntime {
    fn from_world(_: &mut World) -> Self {
        let state = (|| {
            let (path, controls) = game_settings::read_game_clock_controls()?;
            let source = PerformanceCounter::new()?;
            let mut clock = NativeGameClock::from_bytes([0; game_clock::STATE_SIZE]);
            clock.initialize(); // Opaque owned padding is not given field meanings.
            info!("Native game clock: QPC {} Hz, controls {:?} from {} (registered defaults where absent)",
                source.frequency, controls, path.display());
            Ok(LoadedClock {
                clock,
                source,
                timing: FrameTimingEstimator::default(),
                controls,
                common_counter: 0,
                advanced: false,
            })
        })();
        if let Err(error) = &state {
            error!("Native game clock unavailable: {error}");
        }
        Self(state)
    }
}

/// A coherent original gameplay-view clock snapshot. All models in one
/// publication use the same five native seconds/deltas, not per-draw QPC reads.
/// Missing owner state remains an error, never a zero or Bevy-clock fallback.
#[derive(Resource, Clone, ExtractResource)]
pub struct NativeGameplayViewClock(Result<game_clock::GameplayViewClocks, String>);

impl NativeGameplayViewClock {
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
                    "native model clock group {} is outside the five gameplay clocks",
                    crate::render_model::model_shader_time_group(model_flags_1c31)
                )
            })
    }
}

impl FromWorld for NativeGameplayViewClock {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource::<NativeClockRuntime>()
                .clock()
                .map(game_clock::GameplayViewClocks::from_clock)
                .map_err(str::to_owned),
        )
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeClockSystems {
    Advance,
    PublishView,
}

fn publish_gameplay_view_clock(
    owner: Res<NativeClockRuntime>,
    mut view: ResMut<NativeGameplayViewClock>,
) {
    view.0 = owner
        .clock()
        .map(game_clock::GameplayViewClocks::from_clock)
        .map_err(str::to_owned);
}

fn advance_native_clock(mut owner: ResMut<NativeClockRuntime>) {
    if let Ok(state) = &mut owner.0 {
        if let Err(error) = state.advance() {
            error!("Native game clock unavailable: {error}");
            owner.0 = Err(error);
        }
    }
}

pub struct NativeClockRuntimePlugin;
impl Plugin for NativeClockRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeClockRuntime>()
            .init_resource::<NativeGameplayViewClock>()
            .add_plugins(ExtractResourcePlugin::<NativeGameplayViewClock>::default())
            .add_systems(
                First,
                advance_native_clock.in_set(NativeClockSystems::Advance),
            )
            .add_systems(
                PostUpdate,
                publish_gameplay_view_clock.in_set(NativeClockSystems::PublishView),
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
            .add_plugins(NativeClockRuntimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        let owner = app.world().resource::<NativeClockRuntime>();
        let before = owner.clock().unwrap().time(3).unwrap();
        assert!(owner.advanced());
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        let owner = app.world().resource::<NativeClockRuntime>();
        let clock = owner.clock().unwrap();
        assert!(clock.time(3).unwrap() > before);
        assert!(clock.delta(3).unwrap() > 0.0);
        assert!(owner.advanced());
        let view = app.world().resource::<NativeGameplayViewClock>();
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
            .add_plugins(NativeClockRuntimePlugin);
        app.world_mut().insert_resource(NativeClockRuntime(Err(
            "test unavailable native clock".into()
        )));
        app.update();
        let view = app.world().resource::<NativeGameplayViewClock>();
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
    fn gameplay_view_clock_is_extracted_after_native_publication() {
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
        .add_plugins(NativeClockRuntimePlugin);
        app.finish();
        app.cleanup();
        app.update();
        let main = app
            .world()
            .resource::<NativeGameplayViewClock>()
            .clocks()
            .unwrap();
        let render = app
            .sub_app(RenderApp)
            .world()
            .resource::<NativeGameplayViewClock>()
            .clocks()
            .unwrap();
        let mut main_bytes = [0; 0x3c];
        let mut render_bytes = [0; 0x3c];
        main.write_game_result(&mut main_bytes).unwrap();
        render.write_game_result(&mut render_bytes).unwrap();
        assert_eq!(main_bytes, render_bytes);
        let native = app
            .world()
            .resource::<NativeClockRuntime>()
            .clock()
            .unwrap();
        for group in 0..game_clock::GROUP_COUNT {
            assert_eq!(
                &main_bytes[group * 8..group * 8 + 8],
                &native.time(group).unwrap().to_le_bytes()
            );
            assert_eq!(
                &main_bytes[0x28 + group * 4..0x2c + group * 4],
                &native.delta(group).unwrap().to_le_bytes()
            );
        }
    }
}
