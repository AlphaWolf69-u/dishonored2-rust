//! Sound-event declaration fields (1414fe270) and constructor defaults
//! (1400b3750). Metadata drives component startup and audible-range admission.
use std::io;
use super::{invalid, invalid_owned};

#[derive(Clone, Debug)]
pub struct SoundEventDeclaration {
    pub id: u32,
    pub game_properties: Option<String>,
    pub game_defined: bool,
    pub spatialized: bool,
    pub has_voices: bool,
    pub internally_stopped: bool,
    pub looping: bool,
    pub max_distance: f32,
    pub duration: [f32; 2],
}
impl SoundEventDeclaration {
    pub fn parse(source: &str) -> io::Result<Self> {
        use crate::entities::scene_motion::child;
        let fields = crate::entities::parse_declaration(source).map_err(invalid_owned)?;
        Self::from_fields(child(&fields, "edit"))
    }

    pub fn from_fields(edit: &[(String, crate::entities::Value)]) -> io::Result<Self> {
        use crate::entities::scene_motion::{atom, child};
        let boolean = |name: &str| match atom(edit, name) {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            _ => Err(invalid_owned(format!("invalid sound-event {name}"))),
        };
        let number = |name: &str, default: f32| {
            let value = atom(edit, name).map(|value| value.parse::<f32>()).transpose()
                .map_err(|error| invalid_owned(format!("sound-event {name}: {error}")))?.unwrap_or(default);
            if value.is_finite() { Ok(value) } else { Err(invalid("non-finite sound-event property")) }
        };
        Ok(Self {
            id: atom(child(edit, "m_EngineID"), "m_ID").ok_or_else(|| invalid("sound declaration has no engine ID"))?
                .parse().map_err(|_| invalid("invalid sound event engine ID"))?,
            game_properties: atom(edit, "m_gameProperties")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL")).map(str::to_owned),
            game_defined: boolean("m_isGameDefined")?, spatialized: boolean("m_Spatialized")?,
            has_voices: boolean("m_HasVoices")?, internally_stopped: boolean("m_IsInternallyStopped")?,
            looping: boolean("m_isLooping")?, max_distance: number("m_MaxDistance", 0.0)?,
            duration: [number("m_minDuration", -1.0)?, number("m_maxDuration", -1.0)?],
        })
    }

    /// 1400b3850 excludes internally stopped events from persistent loops.
    pub fn persistent_loop(&self) -> bool { self.looping && !self.internally_stopped }
}
