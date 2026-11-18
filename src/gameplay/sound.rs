//! Event controls issued to an audio component (PostEvent / StopEvent).
pub mod mix;
pub mod music_clip;
pub mod music;
pub mod bark_history;
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
pub mod volume_control;
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
pub mod playing;
pub mod objects;
pub mod silence;

/// The material pair and physical inputs submitted to the contact FX manager
/// (14060a480). Sound selection is owned by the installed contact database.
#[derive(Clone, Debug)]
pub struct Contact {
    pub source: String,
    pub target: String,
    ///Actual mapped material ID from the accepted ground witness. Static
    ///navigation snapshots have no runtime ID and retain their actual name.
    pub target_material_id: Option<u16>,
    pub position: bevy_math::Vec3,
    pub normal: bevy_math::Vec3,
    pub direction: bevy_math::Vec3,
    pub speed: f32,
    pub mass: f32,
    ///770990 passes !controller-possession(active), not a guessed movement flag.
    pub stimulus_enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Emitter {
    Menu,
    PlayerBody,
    PlayerLeftArm,
    PlayerRightArm,
    PlayerLeftEquipment,
    PlayerRightEquipment,
    /// Host audio object for an actual managed equipment generation. The
    /// authored casting bit is retained for scene cancellation/retirement.
    Equipment { entity: crate::gameplay::entity_lifetime::EntityId, part: u8 },
    /// 708340 owns one sound object per actual animation joint. Equipment
    /// identity includes the full managed generation, never just its slot.
    PlayerJoint { entity: Option<crate::gameplay::entity_lifetime::EntityId>, part: u8, joint: u16 },
    Effect(u64),
    Component { handle: u64, generation: u32 },
    ///708340: actual component-generation slot joint+1, retained until708620.
    ComponentJoint { handle: u64, generation: u32, joint: i16 },
}

impl Emitter {
    /// Casting bits used by arkPlayerState::AddPlayerCasting (1408542b0).
    pub fn player_part(self) -> u8 {
        match self {
            Self::Menu | Self::Effect(_) | Self::Component { .. } | Self::ComponentJoint { .. } => 0,
            Self::PlayerBody => 1,
            Self::PlayerLeftArm => 2,
            Self::PlayerRightArm => 4,
            Self::PlayerLeftEquipment => 8,
            Self::PlayerRightEquipment => 16,
            Self::Equipment { part, .. } => part,
            Self::PlayerJoint { part, .. } => part,
        }
    }

    pub fn player_owner(self) -> Self {
        match self {
            Self::ComponentJoint { handle, generation, .. } => Self::Component { handle, generation },
            Self::PlayerJoint { entity: Some(entity), part, .. } => Self::Equipment { entity, part },
            Self::PlayerJoint { part: 1, .. } => Self::PlayerBody,
            Self::PlayerJoint { part: 2, .. } => Self::PlayerLeftArm,
            Self::PlayerJoint { part: 4, .. } => Self::PlayerRightArm,
            Self::PlayerJoint { part: 8, .. } => Self::PlayerLeftEquipment,
            Self::PlayerJoint { part: 16, .. } => Self::PlayerRightEquipment,
            _ => self,
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
    /// Host FIFO boundary for1400c7a20 deferred object return.
    Retire { emitter: Emitter },
}
