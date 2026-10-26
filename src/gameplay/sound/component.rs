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

/// Common ownership and default-slot state of arkCpntAudio. The host audio
/// backend owns voice storage; this component retains the sound object after
/// its voices stop, until the component's shutdown releases it (140708620).
pub struct AudioComponent {
    pub owner: crate::gameplay::component_state::ComponentOwner,
    pub startup: Startup,
    pub allocated: bool,
    pub started: bool,
}
impl Default for AudioComponent {
    fn default() -> Self {
        Self { owner: Default::default(), startup: Startup::default(), allocated: false, started: false }
    }
}
impl crate::gameplay::component_state::ComponentInstance for AudioComponent {
    fn owner(&self) -> &crate::gameplay::component_state::ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut crate::gameplay::component_state::ComponentOwner { &mut self.owner }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub dilation_factor: f32,
    pub occlusion_and_obstruction: bool,
    pub auxiliary_sends: bool,
    pub occlusion_at_emitter: bool,
}
impl Default for Settings {
    fn default() -> Self { Self { dilation_factor: 1.0, occlusion_and_obstruction: true,
        auxiliary_sends: true, occlusion_at_emitter: true } } // 14000ce50 sets all four low flag bits.
}
impl Settings {
    /// Reflected at offset 0x6c by 1415536e0.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut settings = Self::default();
        let boolean = |name| match crate::entities::scene_motion::atom(fields, name) {
            None | Some("true") => Ok(true),
            Some("false") => Ok(false),
            _ => Err(format!("invalid audio {name}")),
        };
        settings.occlusion_and_obstruction = boolean("m_enableOcclusionAndObstruction")?;
        settings.auxiliary_sends = boolean("m_enableAuxiliarySends")?;
        settings.occlusion_at_emitter = boolean("m_enableOcclusionAtEmitterPos")?;
        if let Some(value) = crate::entities::scene_motion::atom(fields, "m_dilationFactor") {
            settings.dilation_factor = value.parse::<f32>().map_err(|error| format!("audio dilation factor: {error}"))?;
            if !settings.dilation_factor.is_finite() { return Err("non-finite audio dilation factor".into()); }
        }
        Ok(settings)
    }

    pub fn rate(self, clock_rate: f32) -> f32 { clock_rate * self.dilation_factor }

    /// 140708340 applies declaration flags through 1400c3b40/1400c3b70.
    /// The two scales are independent: one affects propagation path scoring,
    /// the other scales the levels eventually submitted to Wwise.
    pub fn source_occlusion_scale(self) -> f32 { if self.occlusion_at_emitter { 1.0 } else { 0.0 } }
    pub fn level_scale(self) -> f32 { if self.occlusion_and_obstruction { 1.0 } else { 0.0 } }
}
