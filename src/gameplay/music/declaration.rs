//! arkDeclMusicAndAmbianceManager: constructor 140662be0 and reflected
//! fields 141804e20 / 1418052b0 / 1418054d0. Only compact resolved inputs
//! survive acquisition; sound bytes remain with the existing sound owner.
use std::{collections::HashMap, sync::Arc};
use crate::assets::entities::Value;
use crate::gameplay::options::decl::{atom, enum_key, field, scope};

// arkMusicTypes_t registration 141a0d8a0, not an inferred priority table.
pub(super) const STAGES: [&str; 8] = [
    "MUSIC_EXPLORATION", "MUSIC_SUSPENSE", "MUSIC_ESCAPE", "MUSIC_STRESS",
    "MUSIC_COMBAT_LEVEL1", "MUSIC_COMBAT_LEVEL2", "MUSIC_COMBAT_LEVEL3",
    "MUSIC_COMBAT_SPECIAL",
];

#[derive(Debug)]
pub struct Event {
    pub id: u32,
    /// Pointer identity in the game is declaration identity, not engine ID.
    pub declaration: Box<str>,
}

#[derive(Debug, Default)]
pub struct Stinger {
    pub event: Option<Arc<Event>>,
    pub cooldown: f32,
}

#[derive(Debug)]
pub struct Definition {
    pub start_music: Option<Arc<Event>>,
    pub stop_music: Option<Arc<Event>>,
    pub stages: [Option<Arc<Event>>; 8],
    pub elite_enemies: Box<[Box<str>]>,
    pub special_enemies: Box<[Box<str>]>,
    pub combat_level_2: [i32; 2],
    pub combat_level_3: [i32; 2],
    pub start_ambiance: Option<Arc<Event>>,
    pub stop_ambiance: Option<Arc<Event>>,
    pub stingers: [Stinger; 3],
}

impl Definition {
    /// The caller supplies its declaration-text owner, retaining no parallel
    /// music-specific source cache. The resolver already applies inheritance.
    pub fn load(name: &str, mut read: impl FnMut(&str) -> Result<Option<String>, String>)
        -> Result<Self, String>
    {
        let fields = crate::assets::entities::player::resolved_decl("musicmanager", name, &mut read)?;
        let music = scope(&fields, "m_musicParms").unwrap_or(&[]);
        let ambiance = scope(&fields, "m_ambianceParms").unwrap_or(&[]);
        // A temporary identity map avoids rereading a sound declaration used
        // by several slots. The returned references share only its identity/ID.
        let mut events: HashMap<String, Arc<Event>> = HashMap::new();
        let mut event = |reference: Option<&str>| -> Result<Option<Arc<Event>>, String> {
            let Some(reference) = reference.filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL")) else {
                return Ok(None);
            };
            let path = format!("generated/decls/soundevent/{reference}.decl").replace('\\', "/").to_ascii_lowercase();
            if let Some(event) = events.get(&path) { return Ok(Some(event.clone())); }
            let source = read(&path)?.ok_or_else(|| format!("missing music sound declaration {path}"))?;
            let id = crate::assets::wwise::declared_event_id(&source)
                .map_err(|error| format!("{path}: {error}"))?;
            let event = Arc::new(Event { id, declaration: path.clone().into_boxed_str() });
            events.insert(path, event.clone());
            Ok(Some(event))
        };
        let start_music = event(atom(music, "m_startMusic"))?;
        let stop_music = event(atom(music, "m_stopMusic"))?;
        let mut stages: [Option<Arc<Event>>; 8] = Default::default();
        if let Some(entries) = scope(music, "m_musicsStages") {
            for (key, value) in entries {
                let Some(key) = enum_key(key) else { continue };
                let index = STAGES.iter().position(|name| *name == key)
                    .ok_or_else(|| format!("unknown music stage {key}"))?;
                let Value::Atom(reference) = value else { return Err(format!("music stage {key} is not a declaration reference")); };
                stages[index] = event(Some(crate::gameplay::options::decl::unquote(reference)))?;
            }
        }
        let start_ambiance = event(atom(ambiance, "m_startAmbiance"))?;
        let stop_ambiance = event(atom(ambiance, "m_stopAmbiance"))?;
        let mut stingers = std::array::from_fn(|_| Stinger::default());
        if let Some(entries) = scope(&fields, "m_stingerParms").and_then(|fields| scope(fields, "m_stingers")) {
            for (key, value) in entries {
                let Some(key) = enum_key(key) else { continue };
                let index = ["STINGER_LEVEL_ZERO", "STINGER_LEVEL_ONE", "STINGER_LEVEL_TWO"]
                    .iter().position(|name| *name == key).ok_or_else(|| format!("unknown stinger level {key}"))?;
                let Value::Scope(fields) = value else { return Err(format!("stinger {key} is not a scope")); };
                let cooldown = scope(fields, "m_cooldown").unwrap_or(&[]);
                stingers[index] = Stinger { event: event(atom(fields, "m_stingerSound"))?,
                    cooldown: number(cooldown, "m_seconds", 0.0)? };
            }
        }
        Ok(Self { start_music, stop_music, stages,
            elite_enemies: references(music, "m_eliteEnemies")?,
            special_enemies: references(music, "m_specialEnemies")?,
            combat_level_2: [integer(music, "m_combatLevel2MinGuard", 2)?, integer(music, "m_combatLevel2MinElite", 1)?],
            combat_level_3: [integer(music, "m_combatLevel3MinGuard", 3)?, integer(music, "m_combatLevel3MinElite", 2)?],
            start_ambiance, stop_ambiance, stingers })
    }
}

fn integer(fields: &[(String, Value)], name: &str, default: i32) -> Result<i32, String> {
    atom(fields, name).map(|value| value.parse().map_err(|error| format!("music {name}: {error}")))
        .unwrap_or(Ok(default))
}
fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    atom(fields, name).map(|value| value.parse().map_err(|error| format!("music {name}: {error}")))
        .unwrap_or(Ok(default))
}
fn references(fields: &[(String, Value)], name: &str) -> Result<Box<[Box<str>]>, String> {
    let Some(fields) = scope(fields, name) else { return Ok(Box::default()) };
    let count = integer(fields, "num", 0)?;
    if count < 0 { return Err(format!("negative music {name} count")); }
    let mut values = Vec::new();
    for index in 0..count {
        let key = format!("item[{index}]");
        match field(fields, &key) {
            Some(Value::Atom(value)) => {
                let value = crate::gameplay::options::decl::unquote(value);
                if !value.is_empty() && !value.eq_ignore_ascii_case("NULL") {
                    values.push(Box::<str>::from(value));
                }
            },
            Some(_) => return Err(format!("music {name} {key} is not a declaration reference")),
            None => {},
        }
    }
    Ok(values.into_boxed_slice())
}
