//! Prepared music definitions share the loaded actor namespace and encoded
//! source leases (141150a90, 141159840). Mutable timelines live in the player.
use super::{MediaProperties, MediaVariant, SourceTransition, DestinationTransition, TransitionSegment,
    CurvePoint};
use crate::gameplay::sound::{music_clip::Clip, music::timing::{Meter, Segment}};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct MusicRule {
    pub sources: Vec<u32>,
    pub destinations: Vec<u32>,
    pub source: SourceTransition,
    pub destination: DestinationTransition,
    pub bridge: Option<TransitionSegment>,
}
impl MusicRule {
    pub(super) fn read(rule: super::MusicTransitionRule<'_>) -> Self {
        Self { sources: rule.sources().collect(), destinations: rule.destinations().collect(),
            source: rule.source, destination: rule.destination, bridge: rule.bridge }
    }
}
#[derive(Clone, Debug)]
pub struct MusicAutomation { pub clip: u32, pub kind: u32, pub points: Vec<CurvePoint> }
#[derive(Clone, Debug)]
pub struct MusicMedia { pub track: u32, pub source: u32, pub variant: MediaVariant }
#[derive(Clone, Debug)]
pub enum MusicKind {
    Track { clips: Vec<Clip>, automation: Vec<MusicAutomation>, lookahead: i32 },
    Segment { timing: Segment, tracks: Vec<u32> },
    Playlist { playlist: std::sync::Arc<crate::gameplay::sound::music::playlist::Playlist>, rules: Vec<MusicRule> },
    Switch { groups: Vec<(u32, u8)>, tree: Vec<u8>, mode: u8, continue_playback: bool, rules: Vec<MusicRule> },
}
#[derive(Clone, Debug)]
pub struct MusicActor {
    pub properties: MediaProperties,
    pub meter: Option<Meter>,
    pub stingers: Vec<[u32; 6]>,
    pub kind: MusicKind,
}
#[derive(Debug)]
pub struct MusicGraph {
    pub root: u32,
    pub actors: HashMap<u32, MusicActor>,
    pub media: Vec<MusicMedia>,
    pub meters: HashMap<u32, Meter>,
    pub history: crate::gameplay::sound::music::playlist::History,
}
