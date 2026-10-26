//! Loaded actor, action and event registries (1410d64e0 / 1410f5a80).
//! References are global; media keeps the bank that owns its source object.
use super::{bank, bank_chunks, bank_feedback, bank_objects, invalid, invalid_owned,
    media_from_sound, read_u32_slice, MediaProperties, MediaVariant, PlaybackCommand,
    PlaybackContainer, PlaybackNode};
use std::{borrow::Cow, collections::{HashMap, HashSet}, io};

struct LoadedBank<'a> { bytes: Cow<'a, [u8]>, feedback: bool }

#[derive(Clone, Copy)]
struct Object { kind: u8, bank: usize, start: usize, length: usize }

#[derive(Debug, Default)]
pub struct References {
    pub music_actors: usize,
    pub parents_in_other_banks: usize,
    pub playlist_items_in_other_banks: usize,
    pub actions_in_other_banks: usize,
    pub play_targets_in_other_banks: usize,
    pub missing_parents: usize,
    pub missing_playlist_items: usize,
    pub missing_actions: usize,
    pub missing_play_targets: usize,
    pub unresolved_playlist: Vec<(u32, u32)>,
    pub unresolved_play_targets: Vec<(u32, u32)>,
}

#[derive(Default)]
pub struct PlaybackBanks<'a> {
    banks: Vec<LoadedBank<'a>>,
    actors: HashMap<u32, Object>,
    actions: HashMap<u32, Object>,
    events: HashMap<u32, Object>,
}

impl<'a> PlaybackBanks<'a> {
    /// Insert in bank-load order. Native loaders retain existing actor/event
    /// objects (1410f2370 / 1410f2270 / 1410f6450). Standard codec sources
    /// retain their resident media too (1410f1910 / 141122400, flags 4/12|128).
    pub fn insert(&mut self, bytes: Cow<'a, [u8]>) -> io::Result<usize> {
        let chunks = bank_chunks(&bytes)?;
        let feedback = bank_feedback(&chunks)?;
        let mut objects = chunks.get(b"HIRC").map(|hierarchy| bank_objects(hierarchy))
            .transpose()?.unwrap_or_default().into_iter().collect::<Vec<_>>();
        objects.sort_by_key(|(_, object)| object.as_ptr() as usize);
        let bank = self.banks.len();
        let records = objects.into_iter().map(|((kind, id), object)| (id, Object { kind, bank,
            start: object.as_ptr() as usize - bytes.as_ptr() as usize, length: object.len() })).collect::<Vec<_>>();
        self.banks.push(LoadedBank { bytes, feedback });
        for (id, object) in records {
            match object.kind {
                // 141150a90 loads music kinds 10..13 into the same actor
                // namespace (141150660 / 1411508b0), via the music engine.
                2 | 5 | 6 | 7 | 9 | 10..=13 => { self.actors.entry(id).or_insert(object); },
                3 => { self.actions.entry(id).or_insert(object); },
                4 => { self.events.entry(id).or_insert(object); },
                _ => {},
            }
        }
        Ok(bank)
    }

    pub fn event_count(&self) -> usize { self.events.len() }
    pub fn actor_count(&self) -> usize { self.actors.len() }
    pub fn event_ids(&self) -> Vec<u32> {
        let mut ids = self.events.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// Inspect reference ownership without decoding media or running playback.
    pub fn references(&self) -> io::Result<References> {
        let mut result = References::default();
        for (&actor_id, &actor) in &self.actors {
            if matches!(actor.kind, 10..=13) { result.music_actors += 1; continue; }
            let bytes = self.bytes(actor);
            let feedback = self.banks[actor.bank].feedback;
            let properties = bank::node_properties(bytes, actor.kind, feedback)?;
            if properties.parent != 0 {
                match self.actors.get(&properties.parent) {
                    Some(parent) => result.parents_in_other_banks += usize::from(parent.bank != actor.bank),
                    None => result.missing_parents += 1,
                }
            }
            if actor.kind == 5 {
                for (id, _) in bank::node_children(bytes, actor.kind, feedback)?.1.items {
                    match self.actors.get(&id) {
                        Some(child) => result.playlist_items_in_other_banks += usize::from(child.bank != actor.bank),
                        None => { result.missing_playlist_items += 1; result.unresolved_playlist.push((actor_id, id)); },
                    }
                }
            }
        }
        for &event in self.events.values() {
            let bytes = self.bytes(event);
            for index in 0..read_u32_slice(bytes, 4)? as usize {
                let id = read_u32_slice(bytes, 8 + index * 4)?;
                match self.actions.get(&id) {
                    Some(action) => result.actions_in_other_banks += usize::from(action.bank != event.bank),
                    None => result.missing_actions += 1,
                }
            }
        }
        for (&action_id, &action) in &self.actions {
            let bytes = self.bytes(action);
            if bytes.get(5) != Some(&4) { continue; }
            let id = read_u32_slice(bytes, 6)?;
            match self.actors.get(&id) {
                Some(target) => result.play_targets_in_other_banks += usize::from(target.bank != action.bank),
                None => { result.missing_play_targets += 1; result.unresolved_play_targets.push((action_id, id)); },
            }
        }
        result.unresolved_playlist.sort_unstable();
        result.unresolved_play_targets.sort_unstable();
        Ok(result)
    }

    fn bytes(&self, object: Object) -> &[u8] {
        &self.banks[object.bank].bytes[object.start..object.start + object.length]
    }

    fn media_properties(&self, mut id: u32) -> io::Result<MediaProperties> {
        let mut properties = MediaProperties::default();
        let mut visited = HashSet::new();
        while id != 0 {
            if !visited.insert(id) { return Err(invalid("cyclic Wwise parent hierarchy")); }
            let object = *self.actors.get(&id).ok_or_else(|| invalid_owned(format!("missing Wwise parent {id:#010x}")))?;
            if matches!(object.kind, 10..=13) {
                return Err(invalid_owned(format!("Wwise music parent {id:#010x} requires music properties")));
            }
            let node = bank::node_properties(self.bytes(object), object.kind, self.banks[object.bank].feedback)?;
            if properties.bus == 0 { properties.bus = node.bus; }
            id = node.parent;
            properties.nodes.push(node);
        }
        Ok(properties)
    }

    fn resolve_node(&self, id: u32, media: &mut impl FnMut(usize, u32) -> io::Result<Option<Vec<u8>>>,
        visited: &mut HashSet<u32>) -> io::Result<Option<PlaybackNode>> {
        if !visited.insert(id) { return Err(invalid("cycle in Wwise playback graph")); }
        let Some(&object) = self.actors.get(&id) else { visited.remove(&id); return Ok(None); };
        let bytes = self.bytes(object);
        let feedback = self.banks[object.bank].feedback;
        if matches!(object.kind, 10..=13) {
            return Err(invalid_owned(format!("Wwise music actor {id:#010x} (HIRC {}) requires music scheduling", object.kind)));
        }
        if object.kind == 2 {
            if read_u32_slice(bytes, 4)? & 0xf != 1 { return Err(invalid("Wwise source-plugin playback requires its source processor")); }
            let chunks = bank_chunks(&self.banks[object.bank].bytes)?;
            let media = media_from_sound(bytes, &chunks, &mut |id| media(object.bank, id))?;
            let properties = self.media_properties(id)?;
            visited.remove(&id);
            return Ok(media.map(|media| PlaybackNode::Sound { node: id, variant: MediaVariant { media, weight: 1, properties } }));
        }
        if !matches!(object.kind, 5 | 9) { visited.remove(&id); return Ok(None); }
        let (children, playlist) = bank::node_children(bytes, object.kind, feedback)?;
        let node = if object.kind == 5 {
            let mut children = Vec::new();
            for (child, weight) in playlist.items {
                if !self.actors.contains_key(&child) {
                    children.push((PlaybackNode::Missing { node: child }, weight));
                } else if let Some(node) = self.resolve_node(child, media, visited)? { children.push((node, weight)); }
            }
            PlaybackNode::Container(PlaybackContainer { node: id, properties: self.media_properties(id)?, global: playlist.global,
                avoid_repeat: playlist.avoid_repeat, mode: playlist.mode, reset_each_play: playlist.reset_each_play,
                playback: playlist.playback, children })
        } else {
            let mut layers = Vec::new();
            for child in children {
                if let Some(node) = self.resolve_node(child, media, visited)? { layers.push(node); }
            }
            PlaybackNode::Layer { node: id, properties: self.media_properties(id)?, children: layers }
        };
        visited.remove(&id);
        Ok(Some(node))
    }

    /// The media callback receives the loaded-bank index of each Sound,
    /// including when its event, action or actor parent came from another bank.
    pub fn actor_playback(&self, actor: u32,
        mut media: impl FnMut(usize, u32) -> io::Result<Option<Vec<u8>>>) -> io::Result<Option<PlaybackNode>> {
        self.resolve_node(actor, &mut media, &mut HashSet::new())
    }

    /// Resolve event actions through the same loaded actor registry.
    pub fn event_playback(&self, event: u32,
        mut media: impl FnMut(usize, u32) -> io::Result<Option<Vec<u8>>>) -> io::Result<Option<Vec<PlaybackCommand>>> {
        let Some(&event) = self.events.get(&event) else { return Ok(None); };
        let bytes = self.bytes(event);
        let count = read_u32_slice(bytes, 4)? as usize;
        let mut commands = Vec::new();
        for index in 0..count {
            let id = read_u32_slice(bytes, 8 + index * 4)?;
            let Some(&action) = self.actions.get(&id) else { continue; };
            let action = self.bytes(action);
            if action.len() < 10 { return Err(invalid("truncated Wwise action")); }
            let target = read_u32_slice(action, 6)?;
            let command = match action[5] {
                1 => bank::stop_action(action).map(|stop| Some(PlaybackCommand::Stop(stop))),
                4 => {
                    self.actor_playback(target, &mut media).and_then(|node| node.map(|node|
                        bank::play_settings(action).map(|settings| PlaybackCommand::Play { target, settings, node })).transpose())
                },
                _ => continue,
            };
            match command {
                Ok(Some(command)) => commands.push(command),
                Ok(None) => {},
                Err(error) => commands.push(PlaybackCommand::Failed { action: id, target, reason: error.to_string() }),
            }
        }
        Ok((!commands.is_empty()).then_some(commands))
    }
}
