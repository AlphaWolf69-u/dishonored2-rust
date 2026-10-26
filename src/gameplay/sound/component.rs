//! arkCpntAudio time scaling (140708960 / 140708250 / 140708b00).
use crate::assets::entities::Value;
use crate::assets::wwise::SoundEventDeclaration;

/// 140708540: null list slots are ignored. Maximum range is accumulated even
/// when one event prevents deferred startup of the whole list.
#[derive(Clone, Copy, Debug, Default)]
pub struct Startup {
    pub deferred: bool,
    pub pending: bool,
    pub max_distance: f32,
}
impl Startup {
    pub fn from_events(events: &[Option<SoundEventDeclaration>]) -> Self {
        let mut state = Self { deferred: !events.is_empty(), ..Self::default() };
        for event in events.iter().flatten() {
            if state.max_distance < event.max_distance { state.max_distance = event.max_distance; }
            state.deferred &= event.persistent_loop() && event.spatialized && event.game_properties.is_none();
        }
        state.pending = state.deferred;
        state
    }

    /// 1407088b0 returns the required operation. The owner performs the
    /// original listener/propagation query before requesting a pending start.
    pub fn update(&mut self, allocated: bool, object_inaudible: bool, origin_inaudible: bool) -> StartupAction {
        if !self.deferred { return StartupAction::None; }
        if !self.pending {
            if !allocated { self.pending = true; }
            else if object_inaudible { self.pending = true; return StartupAction::Stop; }
        } else if !origin_inaudible { return StartupAction::Post; }
        StartupAction::None
    }

    pub fn posted(&mut self, allocated: bool) { self.pending = !allocated; }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupAction { None, Stop, Post }

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub dilation_factor: f32,
}
impl Default for Settings {
    fn default() -> Self { Self { dilation_factor: 1.0 } } // 14000ce50
}
impl Settings {
    /// Reflected at offset 0x6c by 1415536e0.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut settings = Self::default();
        if let Some(value) = crate::entities::scene_motion::atom(fields, "m_dilationFactor") {
            settings.dilation_factor = value.parse::<f32>().map_err(|error| format!("audio dilation factor: {error}"))?;
            if !settings.dilation_factor.is_finite() { return Err("non-finite audio dilation factor".into()); }
        }
        Ok(settings)
    }

    pub fn rate(self, clock_rate: f32) -> f32 { clock_rate * self.dilation_factor }
}
