//! Borrowed track metadata from 141159840; backing stays with the sound bank.
use super::{CurvePoint, NodeProperties, Reader};
use crate::gameplay::sound::music_clip::SourceClip;
use std::io;

pub struct MusicSource<'a> {
    pub plugin: u32,
    pub stream_type: u8,
    pub media: u32,
    pub memory_size: u32,
    pub flags: u8,
    pub parameters: &'a [u8],
}

pub struct TrackAutomation<'a> {
    pub clip: u32,
    pub kind: u32,
    points: &'a [u8],
}
impl TrackAutomation<'_> {
    pub fn points(&self) -> impl Iterator<Item = CurvePoint> + '_ {
        self.points.chunks_exact(12).map(|point| CurvePoint {
            input: f32::from_le_bytes(point[..4].try_into().unwrap()),
            output: f32::from_le_bytes(point[4..8].try_into().unwrap()),
            interpolation: u32::from_le_bytes(point[8..12].try_into().unwrap()),
        })
    }
}

pub struct MusicTrackPrelude<'a> {
    pub flags: u8,
    pub source_count: u32,
    pub subtrack_count: Option<u32>,
    pub automation_count: u32,
    sources: &'a [u8],
    clips: &'a [u8],
    automations: &'a [u8],
}

/// 14115a0a0's selector followed by 141159840's eight transition words.
/// Transition interpretation belongs to the scheduler, not this byte reader.
pub struct TrackSwitch<'a> {
    pub group_type: u8,
    pub group: u32,
    pub default_switch: u32,
    pub transition: [u32; 8],
    switches: &'a [u8],
}
impl TrackSwitch<'_> {
    pub fn switches(&self) -> impl Iterator<Item = u32> + '_ {
        self.switches
            .chunks_exact(4)
            .map(|id| u32::from_le_bytes(id.try_into().unwrap()))
    }
}

pub struct MusicTrack<'a> {
    pub prelude: MusicTrackPrelude<'a>,
    pub properties: NodeProperties,
    pub playback_type: u8,
    pub switch: Option<TrackSwitch<'a>>,
    /// Serialized value passed to 141159280. Its engine-rate conversion is
    /// separate from the clip conversion and also uses 143eb8c60.
    pub lookahead: i32,
}

/// 141154620's common music-node fields. Meter and stinger words remain
/// serialized here; the scheduler supplies their source-defined semantics.
pub struct MusicNodePrelude<'a> {
    pub flags: u8,
    pub properties: NodeProperties,
    pub meter: &'a [u8],
    children: &'a [u8],
    stingers: &'a [u8],
}
impl MusicNodePrelude<'_> {
    pub fn children(&self) -> impl Iterator<Item = u32> + '_ {
        self.children
            .chunks_exact(4)
            .map(|id| u32::from_le_bytes(id.try_into().unwrap()))
    }
    pub fn stingers(&self) -> impl Iterator<Item = [u32; 6]> + '_ {
        self.stingers.chunks_exact(24).map(|entry| {
            std::array::from_fn(|index| {
                u32::from_le_bytes(entry[index * 4..index * 4 + 4].try_into().unwrap())
            })
        })
    }
}

pub struct MusicCue<'a> {
    pub id: u32,
    pub position_ms: f64,
    pub label: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct MusicFade {
    pub milliseconds: i32,
    pub curve: u32,
    pub offset_ms: i32,
}
pub struct SourceTransition {
    pub fade: MusicFade,
    pub sync: u32,
    pub cue_filter: u32,
    pub play_post_exit: bool,
}
pub struct DestinationTransition {
    pub fade: MusicFade,
    pub cue_filter: u32,
    pub jump_to: u32,
    pub entry: u16,
    pub play_pre_entry: bool,
    pub match_source_cue_name: bool,
}
pub struct TransitionSegment {
    pub segment: u32,
    pub fade_in: MusicFade,
    pub fade_out: MusicFade,
    pub play_pre_entry: bool,
    pub play_post_exit: bool,
}
pub struct MusicTransitionRule<'a> {
    pub source: SourceTransition,
    pub destination: DestinationTransition,
    pub bridge: Option<TransitionSegment>,
    sources: &'a [u8],
    destinations: &'a [u8],
}
impl MusicTransitionRule<'_> {
    pub fn sources(&self) -> impl Iterator<Item = u32> + '_ {
        self.sources
            .chunks_exact(4)
            .map(|id| u32::from_le_bytes(id.try_into().unwrap()))
    }
    pub fn destinations(&self) -> impl Iterator<Item = u32> + '_ {
        self.destinations
            .chunks_exact(4)
            .map(|id| u32::from_le_bytes(id.try_into().unwrap()))
    }
}
pub struct MusicTransitionRules<'a> {
    pub count: u32,
    records: &'a [u8],
}
impl<'a> MusicTransitionRules<'a> {
    pub fn iter(&self) -> impl Iterator<Item = io::Result<MusicTransitionRule<'a>>> + '_ {
        let mut reader = Reader {
            bytes: self.records,
            offset: 0,
        };
        (0..self.count).map(move |_| reader.music_transition())
    }
}

pub struct MusicSwitchContainer<'a> {
    pub node: MusicNodePrelude<'a>,
    pub rules: MusicTransitionRules<'a>,
    pub continue_playback: bool,
    pub tree_mode: u8,
    pub tree: &'a [u8],
    groups: &'a [u8],
    group_types: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct MusicPlaylistItem {
    pub segment: u32,
    pub id: u32,
    pub child_count: u32,
    pub kind: u32,
    pub loops: i16,
    pub loop_min: i16,
    pub loop_max: i16,
    pub weight: u32,
    pub avoid_repeat: u16,
    pub use_weights: bool,
    pub shuffle: bool,
}

/// 1411576a0's serialized preorder playlist. Runtime selections/cursors are
/// per playing instance, not part of this immutable bank-backed definition.
pub struct MusicPlaylistContainer<'a> {
    pub node: MusicNodePrelude<'a>,
    pub rules: MusicTransitionRules<'a>,
    items: &'a [u8],
}
impl MusicPlaylistContainer<'_> {
    pub fn items(&self) -> impl ExactSizeIterator<Item = MusicPlaylistItem> + '_ {
        self.items.chunks_exact(30).map(|record| {
            let word = |offset| u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
            let short = |offset| i16::from_le_bytes(record[offset..offset + 2].try_into().unwrap());
            MusicPlaylistItem {
                segment: word(0),
                id: word(4),
                child_count: word(8),
                kind: word(12),
                loops: short(16),
                loop_min: short(18),
                loop_max: short(20),
                weight: word(22),
                avoid_repeat: short(26) as u16,
                use_weights: record[28] != 0,
                shuffle: record[29] != 0,
            }
        })
    }
}
impl MusicSwitchContainer<'_> {
    pub fn groups(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.groups
            .chunks_exact(4)
            .zip(self.group_types)
            .map(|(id, &kind)| (u32::from_le_bytes(id.try_into().unwrap()), kind))
    }
}

/// Segment duration/markers (141155700); labels borrow the bank instead of
/// duplicating the source's temporary, then retained marker-name allocation.
pub struct MusicSegment<'a> {
    pub node: MusicNodePrelude<'a>,
    pub duration_ms: f64,
    pub cue_count: u32,
    cues: &'a [u8],
}
impl<'a> MusicSegment<'a> {
    pub fn cues(&self) -> impl Iterator<Item = io::Result<MusicCue<'a>>> + '_ {
        let mut reader = Reader {
            bytes: self.cues,
            offset: 0,
        };
        (0..self.cue_count).map(move |_| reader.music_cue())
    }
}
impl<'a> MusicTrackPrelude<'a> {
    pub fn sources(&self) -> impl Iterator<Item = io::Result<MusicSource<'a>>> + '_ {
        let mut reader = Reader {
            bytes: self.sources,
            offset: 0,
        };
        (0..self.source_count).map(move |_| reader.source_descriptor())
    }
    pub fn clips(&self) -> impl Iterator<Item = SourceClip> + '_ {
        self.clips.chunks_exact(40).map(|record| {
            let double =
                |offset| f64::from_le_bytes(record[offset..offset + 8].try_into().unwrap());
            SourceClip {
                track: u32::from_le_bytes(record[..4].try_into().unwrap()),
                source: u32::from_le_bytes(record[4..8].try_into().unwrap()),
                position_ms: double(8),
                trim_begin_ms: double(16),
                trim_end_ms: double(24),
                source_duration_ms: double(32),
            }
        })
    }
    pub fn automations(&self) -> impl Iterator<Item = io::Result<TrackAutomation<'a>>> + '_ {
        let mut reader = Reader {
            bytes: self.automations,
            offset: 0,
        };
        (0..self.automation_count).map(move |_| reader.track_automation())
    }
}

impl<'a> Reader<'a> {
    fn music_fade(&mut self) -> io::Result<MusicFade> {
        Ok(MusicFade {
            milliseconds: self.u32()? as i32,
            curve: self.u32()?,
            offset_ms: self.u32()? as i32,
        })
    }
    fn music_ids(&mut self) -> io::Result<&'a [u8]> {
        let count = self.u32()? as usize;
        let start = self.offset;
        self.records(count, 4)?;
        Ok(&self.bytes[start..self.offset])
    }
    /// 141151320: source/destination selectors followed by both fade rules
    /// and an optional transition segment. Preserve only defined flag bits.
    fn music_transition(&mut self) -> io::Result<MusicTransitionRule<'a>> {
        let sources = self.music_ids()?;
        let destinations = self.music_ids()?;
        let source = SourceTransition {
            fade: self.music_fade()?,
            sync: self.u32()? & 31,
            cue_filter: self.u32()?,
            play_post_exit: self.u8()? & 1 != 0,
        };
        let destination = DestinationTransition {
            fade: self.music_fade()?,
            cue_filter: self.u32()?,
            jump_to: self.u32()?,
            entry: self.u16()? & 7,
            play_pre_entry: self.u8()? & 1 != 0,
            match_source_cue_name: self.u8()? & 1 != 0,
        };
        let bridge = if self.u8()? != 0 {
            Some(TransitionSegment {
                segment: self.u32()?,
                fade_in: self.music_fade()?,
                fade_out: self.music_fade()?,
                play_pre_entry: self.u8()? & 1 != 0,
                play_post_exit: self.u8()? & 1 != 0,
            })
        } else {
            None
        };
        Ok(MusicTransitionRule {
            source,
            destination,
            bridge,
            sources,
            destinations,
        })
    }
    fn music_rules(&mut self) -> io::Result<MusicTransitionRules<'a>> {
        let count = self.u32()?;
        let start = self.offset;
        for _ in 0..count {
            self.music_transition()?;
        }
        Ok(MusicTransitionRules {
            count,
            records: &self.bytes[start..self.offset],
        })
    }
    fn f64(&mut self) -> io::Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn music_cue(&mut self) -> io::Result<MusicCue<'a>> {
        let id = self.u32()?;
        let position_ms = self.f64()?;
        let size = self.u32()? as usize;
        Ok(MusicCue {
            id,
            position_ms,
            label: self.take(size)?,
        })
    }
    fn music_node(&mut self, id: u32, feedback: bool) -> io::Result<MusicNodePrelude<'a>> {
        let flags = self.u8()?;
        let mut properties = self.node_base(feedback)?;
        properties.id = id;
        let count = self.u32()? as usize;
        let start = self.offset;
        self.records(count, 4)?;
        let children = &self.bytes[start..self.offset];
        let meter = self.take(23)?;
        let count = self.u32()? as usize;
        let start = self.offset;
        self.records(count, 24)?;
        Ok(MusicNodePrelude {
            flags,
            properties,
            meter,
            children,
            stingers: &self.bytes[start..self.offset],
        })
    }
    pub(super) fn source_descriptor(&mut self) -> io::Result<MusicSource<'a>> {
        let plugin = self.u32()?;
        let header = self.take(10)?;
        let parameters = if matches!(plugin & 0xf, 2 | 5) {
            let size = self.u32()? as usize;
            self.take(size)?
        } else {
            &self.bytes[..0]
        };
        Ok(MusicSource {
            plugin,
            stream_type: header[0],
            media: u32::from_le_bytes(header[1..5].try_into().unwrap()),
            memory_size: u32::from_le_bytes(header[5..9].try_into().unwrap()),
            flags: header[9],
            parameters,
        })
    }
    fn track_automation(&mut self) -> io::Result<TrackAutomation<'a>> {
        let clip = self.u32()?;
        let kind = self.u32()?;
        let count = self.u32()? as usize;
        let start = self.offset;
        self.records(count, 12)?;
        Ok(TrackAutomation {
            clip,
            kind,
            points: &self.bytes[start..self.offset],
        })
    }
    pub(super) fn track_prelude(&mut self) -> io::Result<MusicTrackPrelude<'a>> {
        let flags = self.u8()?;
        let source_count = self.u32()?;
        let source_start = self.offset;
        for _ in 0..source_count {
            self.source_descriptor()?;
        }
        let sources = &self.bytes[source_start..self.offset];
        let clip_count = self.u32()? as usize;
        let clip_start = self.offset;
        self.records(clip_count, 40)?;
        let clips = &self.bytes[clip_start..self.offset];
        let subtrack_count = if clip_count == 0 {
            None
        } else {
            Some(self.u32()?)
        };
        let automation_count = self.u32()?;
        let automation_start = self.offset;
        for _ in 0..automation_count {
            self.track_automation()?;
        }
        Ok(MusicTrackPrelude {
            flags,
            source_count,
            subtrack_count,
            automation_count,
            sources,
            clips,
            automations: &self.bytes[automation_start..self.offset],
        })
    }
}

/// Common node properties follow this prefix; playback-type/switch settings
/// follow those properties. This does not mistake the prefix for a full track.
pub fn music_track_prelude(object: &[u8]) -> io::Result<MusicTrackPrelude<'_>> {
    let mut reader = Reader {
        bytes: object,
        offset: 0,
    };
    reader.u32()?; // HIRC object identity.
    reader.track_prelude()
}

/// Complete version-113 track layout, acquired on demand from a bank owner.
/// Media, plugin parameters, clip and selector arrays remain borrowed. Only
/// the ordinary node properties use their existing decoded representation.
pub fn music_track(object: &[u8], feedback: bool) -> io::Result<MusicTrack<'_>> {
    let mut reader = Reader {
        bytes: object,
        offset: 0,
    };
    let id = reader.u32()?;
    let prelude = reader.track_prelude()?;
    let mut properties = reader.node_base(feedback)?;
    properties.id = id;
    let playback_type = reader.u8()?;
    let switch = if playback_type == 3 {
        let group_type = reader.u8()?;
        let group = reader.u32()?;
        let default_switch = reader.u32()?;
        let count = reader.u32()? as usize;
        let start = reader.offset;
        reader.records(count, 4)?;
        let switches = &reader.bytes[start..reader.offset];
        let mut transition = [0; 8];
        for word in &mut transition {
            *word = reader.u32()?;
        }
        Some(TrackSwitch {
            group_type,
            group,
            default_switch,
            transition,
            switches,
        })
    } else {
        None
    };
    let lookahead = reader.u32()? as i32;
    Ok(MusicTrack {
        prelude,
        properties,
        playback_type,
        switch,
        lookahead,
    })
}

pub fn music_segment(object: &[u8], feedback: bool) -> io::Result<MusicSegment<'_>> {
    let mut reader = Reader {
        bytes: object,
        offset: 0,
    };
    let id = reader.u32()?;
    let node = reader.music_node(id, feedback)?;
    let duration_ms = reader.f64()?;
    let cue_count = reader.u32()?;
    let start = reader.offset;
    for _ in 0..cue_count {
        reader.music_cue()?;
    }
    Ok(MusicSegment {
        node,
        duration_ms,
        cue_count,
        cues: &reader.bytes[start..reader.offset],
    })
}

/// 1411583d0 / 1411582e0 / 1410ee910. Argument arrays and the decision tree
/// share bank storage. Tree selection/continuous switching use a separate
/// runtime; this reader never substitutes ordinary switch-container rules.
pub fn music_switch(object: &[u8], feedback: bool) -> io::Result<MusicSwitchContainer<'_>> {
    let mut reader = Reader {
        bytes: object,
        offset: 0,
    };
    let id = reader.u32()?;
    let node = reader.music_node(id, feedback)?;
    let rules = reader.music_rules()?;
    let continue_playback = reader.u8()? != 0;
    let count = reader.u32()? as usize;
    let start = reader.offset;
    reader.records(count, 4)?;
    let groups = &reader.bytes[start..reader.offset];
    let group_types = reader.take(count)?;
    let tree_size = reader.u32()? as usize;
    let tree_mode = reader.u8()?;
    let tree = reader.take(tree_size)?;
    Ok(MusicSwitchContainer {
        node,
        rules,
        continue_playback,
        tree_mode,
        tree,
        groups,
        group_types,
    })
}

pub fn music_playlist(object: &[u8], feedback: bool) -> io::Result<MusicPlaylistContainer<'_>> {
    let mut reader = Reader {
        bytes: object,
        offset: 0,
    };
    let id = reader.u32()?;
    let node = reader.music_node(id, feedback)?;
    let rules = reader.music_rules()?;
    let count = reader.u32()? as usize;
    let start = reader.offset;
    reader.records(count, 30)?;
    Ok(MusicPlaylistContainer {
        node,
        rules,
        items: &reader.bytes[start..reader.offset],
    })
}
