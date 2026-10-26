//! Event controls issued to an audio component (PostEvent / StopEvent).
pub mod mix;
pub mod containers;
pub mod spatial;
pub mod panning;
pub mod filters;
pub mod properties;
pub mod resampler;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Emitter {
    Menu,
    PlayerBody,
    PlayerLeftArm,
    PlayerRightArm,
}

impl Emitter {
    /// Casting bits used by arkPlayerState::AddPlayerCasting (1408542b0).
    pub fn player_part(self) -> u8 {
        match self {
            Self::Menu => 0,
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
    Stop { event: u32, emitter: Emitter },
}
