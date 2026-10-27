//! Event controls issued to an audio component (PostEvent / StopEvent).
pub mod mix;
pub mod containers;
pub mod states;
pub mod switches;
pub mod spatial;
pub mod panning;
pub mod filters;
pub mod properties;
pub mod resampler;
pub mod gain;
pub mod component;
pub mod admission;
pub mod propagation;
pub mod acoustic_world;
pub mod paths;
pub mod positions;
pub mod environment;
pub mod auxiliary;
pub mod room_reverb;
pub mod matrix_reverb;
pub mod bus_effects;
pub mod ducking;
pub mod activity;
pub mod delay;
pub mod tail;
pub mod listener;
pub mod looping;
pub mod silence;

/// The material pair and physical inputs submitted to the contact FX manager
/// (14060a480). Sound selection is owned by the installed contact database.
#[derive(Clone, Debug)]
pub struct Contact {
    pub source: String,
    pub target: String,
    pub position: bevy_math::Vec3,
    pub normal: bevy_math::Vec3,
    pub direction: bevy_math::Vec3,
    pub speed: f32,
    pub mass: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Emitter {
    Menu,
    PlayerBody,
    PlayerLeftArm,
    PlayerRightArm,
    Effect(u64),
    Component { handle: u64, generation: u32 },
}

impl Emitter {
    /// Casting bits used by arkPlayerState::AddPlayerCasting (1408542b0).
    pub fn player_part(self) -> u8 {
        match self {
            Self::Menu | Self::Effect(_) | Self::Component { .. } => 0,
            Self::PlayerBody => 1,
            Self::PlayerLeftArm => 2,
            Self::PlayerRightArm => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    Post { event: u32, emitter: Emitter },
    PostWithParameter { event: u32, emitter: Emitter, parameter: u32, value: f32 },
    PostAt { event: u32, emitter: Emitter, position: bevy_math::Vec3,
        front: Option<bevy_math::Vec3>, parameter: Option<(u32, f32)> },
    Stop { event: u32, emitter: Emitter },
}
