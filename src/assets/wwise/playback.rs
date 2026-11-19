//! Loaded actor, action and event registries (1410d64e0 / 1410f5a80).
//! References are global; media keeps the bank that owns its source object.
use super::{bank, bank_chunks, bank_feedback, bank_objects, invalid, invalid_owned, BankMediaIndex,
    media_from_sound, read_u32_slice, MediaProperties, MediaVariant, PlaybackCommand,
    PlaybackContainer, PlaybackNode, PlaybackSwitch, MediaBytes};
use std::{borrow::Cow, collections::{HashMap, HashSet}, io, ops::Range,
    sync::{Arc, Mutex, OnceLock, Weak, atomic::{AtomicU64, Ordering}}};
mod music;

#[derive(Clone)]
struct LoadedBank<'a> {
    bytes: BankBytes<'a>,
    feedback: bool,
    media_index: Option<Range<usize>>,
    media_data: Option<Range<usize>>,
    media_index_ordered: bool,
}

static MEDIA_SECTION_MODE: OnceLock<(bool, bool)> = OnceLock::new();
static MEDIA_SECTION_LOOKUPS: AtomicU64 = AtomicU64::new(0);
static MEDIA_SECTION_CYCLES: AtomicU64 = AtomicU64::new(0);

impl LoadedBank<'_> {
    fn media_sections(&self) -> io::Result<(Option<&[u8]>, Option<&[u8]>)> {
        let &(walk, trace) = MEDIA_SECTION_MODE.get_or_init(|| (
            std::env::var_os("DH2_DEBUG_WALK_WWISE_BANK_CHUNKS").is_some_and(|v| v == "1"),
            std::env::var_os("DH2_DEBUG_WWISE_MEDIA_SECTIONS").is_some_and(|v| v == "1"),
        ));
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let sections = if walk {
            let chunks = bank_chunks(&self.bytes)?;
            (chunks.get(b"DIDX").copied(), chunks.get(b"DATA").copied())
        } else {
            (self.media_index.as_ref().map(|range| &self.bytes[range.clone()]),
                self.media_data.as_ref().map(|range| &self.bytes[range.clone()]))
        };
        if trace {
            MEDIA_SECTION_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            if let Some(cycles) = started.zip(crate::loading::thread_cycles())
                .map(|(start, end)| end.saturating_sub(start)) {
                MEDIA_SECTION_CYCLES.fetch_add(cycles, Ordering::Relaxed);
            }
        }
        Ok(sections)
    }
}

#[derive(Clone)]
enum BankBytes<'a> {
    Borrowed(&'a [u8]),
    Shared(MediaBytes),
}

impl std::ops::Deref for BankBytes<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self { Self::Borrowed(bytes) => bytes, Self::Shared(bytes) => bytes }
    }
}

impl BankBytes<'_> {
    fn media(&self, bytes: &[u8]) -> MediaBytes {
        if std::env::var("DH2_DEBUG_SOUND_COPIED_MEDIA").as_deref() == Ok("1") {
            return bytes.into();
        }
        match self {
            Self::Borrowed(_) => bytes.into(),
            Self::Shared(owner) => {
                let start = bytes.as_ptr() as usize - owner.as_ptr() as usize;
                owner.slice(start..start + bytes.len())
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Object { kind: u8, bank: usize, start: usize, length: usize }
#[derive(Clone)]
struct StateObject { bank: usize, definition: Arc<super::NodeStateDefinition> }

#[derive(Default)]
pub struct Controls {
    pub containers: Vec<(u32, super::ContainerPlayback)>,
    pub switches: Vec<u32>,
    pub music: Vec<u32>,
    pub missing: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct References {
    pub music_actors: usize,
    pub switch_actors: usize,
    pub continuous_switch_actors: usize,
    pub state_switch_actors: usize,
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
    custom_effects: HashMap<u32, Object>,
    states: HashMap<u32, StateObject>,
    ///One demanded binding owner per actual loaded actor, shared by every
    ///child's property hierarchy and released with that actor's bank suffix.
    node_states: Mutex<HashMap<u32, Arc<super::NodeStates>>>,
    property_chains: Mutex<HashMap<u32, Weak<[super::NodeProperties]>>>,
    music_graphs: Mutex<HashMap<u32, Weak<super::MusicGraph>>>,
    music_history: crate::gameplay::sound::music::playlist::History,
}

impl Clone for PlaybackBanks<'_> {
    fn clone(&self) -> Self {
        Self {
            banks: self.banks.clone(), actors: self.actors.clone(), actions: self.actions.clone(),
            events: self.events.clone(), custom_effects: self.custom_effects.clone(),
            states: self.states.clone(),
            node_states: Mutex::new(self.node_states.lock().unwrap_or_else(|error| error.into_inner()).clone()),
            property_chains: Mutex::new(self.property_chains.lock().unwrap_or_else(|error| error.into_inner()).clone()),
            music_graphs: Mutex::new(self.music_graphs.lock().unwrap_or_else(|error| error.into_inner()).clone()),
            music_history: self.music_history.clone(),
        }
    }
}

impl<'a> PlaybackBanks<'a> {
    /// Release the map-acquired suffix after its instances have stopped.
    /// 1400c97e0 -> 1400c0360 unloads the banks recorded by map acquisition;
    /// 1400c93c0's startup banks have a separate lifetime. Earlier HIRC
    /// definitions remain the first owners, as during insertion.
    pub fn truncate(&mut self, count: usize) {
        self.banks.truncate(count);
        self.actors.retain(|_, object| object.bank < count);
        self.actions.retain(|_, object| object.bank < count);
        self.events.retain(|_, object| object.bank < count);
        self.custom_effects.retain(|_, object| object.bank < count);
        self.states.retain(|_, object| object.bank < count);
        self.node_states.get_mut().unwrap().retain(|id, _| self.actors.contains_key(id));
        // Existing instances retain their own source/bank leases until their
        // output voices drain; no outgoing graph may be reused by a new map.
        self.music_graphs.get_mut().unwrap().clear();
        self.music_history.lock().unwrap().retain(|(actor, _), _| self.actors.contains_key(actor));
        self.property_chains.get_mut().unwrap().retain(|id, weak| {
            self.actors.contains_key(id) && weak.upgrade().is_some_and(|chain|
                chain.iter().all(|node| self.actors.contains_key(&node.id)))
        });
    }

    /// Insert in bank-load order. Native loaders retain existing actor/event
    /// objects (1410f2370 / 1410f2270 / 1410f6450). Standard codec sources
    /// retain their resident media too (1410f1910 / 141122400, flags 4/12|128).
    pub fn insert(&mut self, bytes: Cow<'a, [u8]>) -> io::Result<usize> {
        let bytes = match bytes {
            Cow::Borrowed(bytes) => BankBytes::Borrowed(bytes),
            Cow::Owned(bytes) => BankBytes::Shared(bytes.into()),
        };
        self.insert_bank(bytes)
    }

    pub(crate) fn insert_resident(&mut self, bytes: MediaBytes) -> io::Result<usize> {
        self.insert_bank(BankBytes::Shared(bytes))
    }

    ///Initialization-bank bus parsing already acquired these exact HIRC1
    ///definitions. Reuse their source Arcs in the same playback namespace.
    pub(crate) fn insert_resident_with_states(&mut self, bytes: MediaBytes,
        states: &[(u32, Arc<super::NodeStateDefinition>)]) -> io::Result<usize> {
        self.insert_bank_with_states(BankBytes::Shared(bytes), states)
    }

    fn insert_bank(&mut self, bytes: BankBytes<'a>) -> io::Result<usize> {
        self.insert_bank_with_states(bytes, &[])
    }
    fn insert_bank_with_states(&mut self, bytes: BankBytes<'a>,
        prepared: &[(u32, Arc<super::NodeStateDefinition>)]) -> io::Result<usize> {
        let chunks = bank_chunks(&bytes)?;
        let feedback = bank_feedback(&chunks)?;
        let mut objects = chunks.get(b"HIRC").map(|hierarchy| bank_objects(hierarchy))
            .transpose()?.unwrap_or_default().into_iter().collect::<Vec<_>>();
        objects.sort_by_key(|(_, object)| object.as_ptr() as usize);
        let bank = self.banks.len();
        let records = objects.into_iter().map(|((kind, id), object)| (id, Object { kind, bank,
            start: object.as_ptr() as usize - bytes.as_ptr() as usize, length: object.len() })).collect::<Vec<_>>();
        //1410f6560 loads HIRC1 into its separate State namespace. Parse once
        //before registry mutation; active node definitions retain shared Arcs
        //after a map's bank-index suffix is withdrawn.
        let mut states = Vec::new();
        for &(id, object) in &records {
            if object.kind == 1 && !self.states.contains_key(&id) {
                let definition = match prepared.iter().find(|(instance, _)| *instance == id) {
                    Some((_, definition)) => definition.clone(),
                    None => Arc::new(bank::state_definition(&bytes[object.start..object.start + object.length])?),
                };
                states.push((id, StateObject { bank, definition }));
            }
        }
        let range = |tag: &[u8; 4]| chunks.get(tag).map(|chunk| {
            let start = chunk.as_ptr() as usize - bytes.as_ptr() as usize;
            start..start + chunk.len()
        });
        let media_index = range(b"DIDX");
        let media_data = range(b"DATA");
        let media_index_ordered = media_index.as_ref().is_some_and(|range| BankMediaIndex::ordered(&bytes[range.clone()]));
        self.banks.push(LoadedBank { bytes, feedback, media_index, media_data, media_index_ordered });
        for (id, state) in states { self.states.entry(id).or_insert(state); }
        for (id, object) in records {
            match object.kind {
                // 141150a90 loads music kinds 10..13 into the same actor
                // namespace (141150660 / 1411508b0), via the music engine.
                2 | 5 | 6 | 7 | 9 | 10..=13 => { self.actors.entry(id).or_insert(object); },
                3 => { self.actions.entry(id).or_insert(object); },
                4 => { self.events.entry(id).or_insert(object); },
                // 1410f1f70 retains the first definition in the custom FX
                // namespace, which source generators also reference.
                19 => { self.custom_effects.entry(id).or_insert(object); },
                _ => {},
            }
        }
        Ok(bank)
    }

    pub fn event_count(&self) -> usize { self.events.len() }
    pub fn actor_count(&self) -> usize { self.actors.len() }
    pub fn actor_ids(&self) -> impl Iterator<Item = u32> + '_ { self.actors.keys().copied() }
    #[cfg(feature = "game")]
    pub(crate) fn report_media_sections(&self) {
        if let Some(&(walk, true)) = MEDIA_SECTION_MODE.get() {
            let lookups = MEDIA_SECTION_LOOKUPS.swap(0, Ordering::Relaxed);
            let cycles = MEDIA_SECTION_CYCLES.swap(0, Ordering::Relaxed);
            let directory_walks = if walk { lookups } else { 0 };
            let banks = self.banks.len();
            let range_bytes = banks * 2 * std::mem::size_of::<Option<Range<usize>>>();
            println!("WWISE_MEDIA_SECTIONS banks={banks} range_bytes={range_bytes} lookups={lookups} directory_walks={directory_walks} cycles={cycles}");
        }
    }
    pub fn music_actor_ids(&self) -> impl Iterator<Item = (u32, u8)> + '_ {
        self.actors.iter().filter_map(|(&id, object)|
            matches!(object.kind, 10..=13).then_some((id, object.kind)))
    }
    /// Acquire track metadata from this same loaded bank namespace. The
    /// returned view borrows encoded descriptors/clip/automation arrays;
    /// it cannot survive unloading the bank or create a second media owner.
    pub fn music_track(&self, id: u32) -> io::Result<Option<super::MusicTrack<'_>>> {
        let Some(&object) = self.actors.get(&id) else { return Ok(None) };
        if object.kind != 11 {
            return Err(invalid_owned(format!("Wwise actor {id:#010x} is not a music track")));
        }
        bank::music_track(self.bytes(object), self.banks[object.bank].feedback).map(Some)
    }
    pub fn music_segment(&self, id: u32) -> io::Result<Option<super::MusicSegment<'_>>> {
        let Some(&object) = self.actors.get(&id) else { return Ok(None) };
        if object.kind != 10 {
            return Err(invalid_owned(format!("Wwise actor {id:#010x} is not a music segment")));
        }
        bank::music_segment(self.bytes(object), self.banks[object.bank].feedback).map(Some)
    }
    pub fn music_switch(&self, id: u32) -> io::Result<Option<super::MusicSwitchContainer<'_>>> {
        let Some(&object) = self.actors.get(&id) else { return Ok(None) };
        if object.kind != 12 {
            return Err(invalid_owned(format!("Wwise actor {id:#010x} is not a music switch container")));
        }
        bank::music_switch(self.bytes(object), self.banks[object.bank].feedback).map(Some)
    }
    pub fn music_playlist(&self, id: u32) -> io::Result<Option<super::MusicPlaylistContainer<'_>>> {
        let Some(&object) = self.actors.get(&id) else { return Ok(None) };
        if object.kind != 13 {
            return Err(invalid_owned(format!("Wwise actor {id:#010x} is not a music playlist container")));
        }
        bank::music_playlist(self.bytes(object), self.banks[object.bank].feedback).map(Some)
    }
    /// Sources still owned by the banks after a map-bank unload.
    pub fn sound_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.actors.iter().filter_map(|(&id, object)| matches!(object.kind, 2 | 11).then_some(id))
    }
    /// Allocation identities shared with the prepared encoded sources.
    pub fn resident_allocations(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.banks.iter().filter_map(|bank| match &bank.bytes {
            BankBytes::Shared(bytes) => Some(bytes.allocation()), BankBytes::Borrowed(_) => None,
        })
    }
    /// The actual parent chain used by pending-action cancellation
    /// (141115df0 / 1410e9950), independently of playlist membership.
    pub fn actor_hierarchy(&self, id: u32) -> io::Result<Option<Vec<u32>>> {
        if !self.actors.contains_key(&id) { return Ok(None); }
        Ok(Some(self.node_hierarchy(id)?.iter().map(|node| node.id).collect()))
    }
    /// Follow the actual Play references without reading or decoding media.
    /// Playlist/case membership is independent of serialized hierarchy lists.
    pub fn controls(&self, root: u32) -> io::Result<Controls> {
        let mut result = Controls::default();
        let mut pending = vec![root];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if !visited.insert(id) { continue; }
            let Some(&object) = self.actors.get(&id) else { result.missing.push(id); continue; };
            let bytes = self.bytes(object);
            let feedback = self.banks[object.bank].feedback;
            match object.kind {
                5 => {
                    let (_, playlist) = bank::node_children(bytes, object.kind, feedback)?;
                    pending.extend(playlist.items.iter().map(|&(id, _)| id));
                    result.containers.push((id, playlist.playback));
                },
                6 => {
                    result.switches.push(id);
                    pending.extend(bank::switch_playback(bytes, feedback)?.cases.into_iter().flat_map(|case| case.children));
                },
                9 => pending.extend(bank::node_children(bytes, object.kind, feedback)?.0),
                10..=13 => result.music.push(id),
                _ => {},
            }
        }
        result.containers.sort_by_key(|&(id, _)| id);
        result.switches.sort_unstable();
        result.music.sort_unstable();
        result.missing.sort_unstable();
        Ok(result)
    }
    pub fn generator_ids(&self) -> io::Result<Vec<u32>> {
        let mut ids = Vec::new();
        for (&id, &object) in &self.actors {
            if object.kind == 2 && matches!(read_u32_slice(self.bytes(object), 4)? & 0xf, 2 | 5) { ids.push(id); }
        }
        ids.sort_unstable();
        Ok(ids)
    }
    /// For each Play action of an event: its target's node chain (target
    /// first) with each node's 1410de430 playback-limit settings.
    pub fn play_limits(&self, event: u32) -> io::Result<Vec<Vec<(u32, super::AdvancedSettings)>>> {
        let Some(&event) = self.events.get(&event) else { return Ok(Vec::new()) };
        let bytes = self.bytes(event);
        let mut chains = Vec::new();
        for index in 0..read_u32_slice(bytes, 4)? as usize {
            let Some(&action) = self.actions.get(&read_u32_slice(bytes, 8 + index * 4)?) else { continue };
            let action = self.bytes(action);
            if action.get(5) != Some(&4) { continue; }
            let chain = self.node_hierarchy(read_u32_slice(action, 6)?)?;
            chains.push(chain.iter().map(|node| (node.id, node.advanced)).collect());
        }
        Ok(chains)
    }
    /// Each sound's resolved below-threshold settings and whether its chain
    /// has a curve driven by something other than a game parameter (such as
    /// a modulator), plus whether any loaded bank carries feedback data.
    pub fn below_threshold_settings(&self) -> io::Result<(Vec<(u32, super::BelowThreshold, super::VirtualReturn, bool)>, bool)> {
        let mut sounds = Vec::new();
        for (&id, object) in &self.actors {
            if object.kind != 2 { continue; }
            let properties = MediaProperties { nodes: self.node_hierarchy(id)?, bus: 0 };
            let (behaviour, queue) = properties.below_threshold();
            let other_curves = properties.nodes.iter().any(|node| node.curves.iter().any(|curve| curve.kind != 0));
            sounds.push((id, behaviour, queue, other_curves));
        }
        sounds.sort_unstable_by_key(|&(id, ..)| id);
        Ok((sounds, self.banks.iter().any(|bank| bank.feedback)))
    }
    pub fn event_ids(&self) -> Vec<u32> {
        let mut ids = self.events.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// Inspect reference ownership without decoding media or running playback.
    pub fn references(&self) -> io::Result<References> {
        let mut result = References::default();
        for (&actor_id, &actor) in &self.actors {
            if matches!(actor.kind, 10..=13) { result.music_actors += 1; }
            let bytes = self.bytes(actor);
            let feedback = self.banks[actor.bank].feedback;
            if actor.kind == 6 {
                let settings = bank::switch_playback(bytes, feedback)?;
                result.switch_actors += 1;
                result.continuous_switch_actors += usize::from(settings.continuous);
                result.state_switch_actors += usize::from(settings.group_kind == 1);
            }
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

    fn node_hierarchy(&self, mut id: u32) -> io::Result<super::property_chains::Chain> {
        let root = id;
        if !super::property_chains::separate() {
            let existing = self.property_chains.lock().unwrap().get(&root).and_then(Weak::upgrade);
            if let Some(chain) = existing {
                super::property_chains::requested(root, &chain);
                return Ok(chain);
            }
        }
        let mut nodes = Vec::new();
        let mut visited = HashSet::new();
        while id != 0 {
            if !visited.insert(id) { return Err(invalid("cyclic Wwise parent hierarchy")); }
            let object = *self.actors.get(&id).ok_or_else(|| invalid_owned(format!("missing Wwise parent {id:#010x}")))?;
            let mut node = bank::node_properties(self.bytes(object), object.kind, self.banks[object.bank].feedback)?;
            {
                let mut states = self.node_states.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(existing) = states.get(&id) {
                    node.use_resolved_states(existing.clone());
                } else {
                    node.resolve_states(|instance| self.states.get(&instance).map(|state| state.definition.clone())
                        .ok_or_else(|| invalid_owned(format!("missing loaded Wwise State instance {instance:#010x}"))))?;
                    if !node.states.groups.is_empty() { states.insert(id, node.states.clone()); }
                }
            }
            id = node.parent;
            nodes.push(node);
        }
        let chain = super::property_chains::make(nodes);
        let chain = if super::property_chains::separate() { chain } else {
            let mut cached = self.property_chains.lock().unwrap();
            if let Some(existing) = cached.get(&root).and_then(Weak::upgrade) { existing }
            else { cached.insert(root, Arc::downgrade(&chain)); chain }
        };
        super::property_chains::requested(root, &chain);
        Ok(chain)
    }

    fn media_properties(&self, id: u32) -> io::Result<MediaProperties> {
        let nodes = self.node_hierarchy(id)?;
        let bus = nodes.iter().find(|node| node.bus != 0).map_or(0, |node| node.bus);
        Ok(MediaProperties { nodes, bus })
    }

    fn resolve_node(&self, id: u32, media: &mut impl FnMut(usize, u32, u8) -> io::Result<Option<MediaBytes>>,
        visited: &mut HashSet<u32>, sources: &mut HashMap<u32, Option<(MediaBytes, Option<u32>)>>) -> io::Result<Option<PlaybackNode>> {
        if !visited.insert(id) { return Err(invalid("cycle in Wwise playback graph")); }
        let Some(&object) = self.actors.get(&id) else { visited.remove(&id); return Ok(None); };
        let bytes = self.bytes(object);
        let feedback = self.banks[object.bank].feedback;
        if matches!(object.kind, 10..=13) {
            visited.remove(&id);
            return self.resolve_music(id, media).map(|graph| Some(PlaybackNode::Music(graph)));
        }
        if object.kind == 6 {
            let playback = bank::switch_playback(bytes, feedback)?;
            let mut children = Vec::new();
            let mut seen = HashSet::new();
            for &child in playback.cases.iter().flat_map(|case| &case.children) {
                if seen.insert(child) { children.push((child, self.resolve_node(child, media, visited, sources)?)); }
            }
            let properties = self.media_properties(id)?;
            visited.remove(&id);
            return Ok(Some(PlaybackNode::Switch(PlaybackSwitch { node: id, properties, playback, children })));
        }
        if object.kind == 2 {
            let plugin = read_u32_slice(bytes, 4)?;
            if plugin & 0xf != 1 {
                let resource = read_u32_slice(bytes, 9)?;
                if let Some(&effect) = self.custom_effects.get(&resource) {
                    let definition = bank::effect_definition(self.bytes(effect), false)?;
                    let properties = self.media_properties(id)?;
                    visited.remove(&id);
                    return Ok(Some(PlaybackNode::Generator { node: id, properties, definition }));
                }
                visited.remove(&id);
                return Ok(Some(PlaybackNode::Unsupported { node: id,
                    reason: format!("source plugin {plugin:#010x} has no loaded custom FX {resource:#010x}") }));
            }
            // All references to a loaded Sound share its resident source
            // (1410f1910 / 141122400). Cases must not duplicate WEM storage.
            let source = match sources.entry(id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let bank = &self.banks[object.bank];
                    let (index, data) = bank.media_sections()?;
                    let index = index.map(|bytes| BankMediaIndex { bytes, ordered: bank.media_index_ordered });
                    let source = media_from_sound(bytes, index, data,
                        &mut |id, stream| media(object.bank, id, stream), |bytes| bank.bytes.media(bytes))?;
                    entry.insert(source).clone()
                },
            };
            let properties = self.media_properties(id)?;
            visited.remove(&id);
            return Ok(source.map(|(media, streamed)| PlaybackNode::Sound { node: id, variant: MediaVariant {
                media, streamed: streamed.map(|media_id| (object.bank, media_id)), weight: 1, properties,
            } }));
        }
        if object.kind == 7 {
            let properties = self.media_properties(id)?;
            visited.remove(&id);
            return Ok(Some(PlaybackNode::NoSource { node: id, properties }));
        }
        if !matches!(object.kind, 5 | 9) { visited.remove(&id); return Ok(None); }
        let (children, playlist) = bank::node_children(bytes, object.kind, feedback)?;
        let node = if object.kind == 5 {
            let mut children = Vec::new();
            for (child, weight) in playlist.items {
                if !self.actors.contains_key(&child) {
                    children.push((PlaybackNode::Missing { node: child }, weight));
                } else if let Some(node) = self.resolve_node(child, media, visited, sources)? { children.push((node, weight)); }
            }
            PlaybackNode::Container(PlaybackContainer { node: id, properties: self.media_properties(id)?, global: playlist.global,
                avoid_repeat: playlist.avoid_repeat, mode: playlist.mode, reset_each_play: playlist.reset_each_play,
                playback: playlist.playback, children })
        } else {
            let mut layers = Vec::new();
            for child in children {
                if let Some(node) = self.resolve_node(child, media, visited, sources)? { layers.push(node); }
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
        self.resolve_node(actor, &mut |owner, id, _| media(owner, id).map(|bytes| bytes.map(Into::into)), &mut HashSet::new(), &mut HashMap::new())
    }

    /// Resolve event actions through the same loaded actor registry.
    pub fn event_playback(&self, event: u32,
        mut media: impl FnMut(usize, u32) -> io::Result<Option<Vec<u8>>>) -> io::Result<Option<Vec<PlaybackCommand>>> {
        self.event_playback_shared(event, |owner, id, _| media(owner, id).map(|bytes| bytes.map(Into::into)))
    }

    /// Registered sources can supply shared resident ranges or format-only
    /// headers for package streams, without copying an entire encoded file.
    pub fn event_playback_shared(&self, event: u32,
        mut media: impl FnMut(usize, u32, u8) -> io::Result<Option<MediaBytes>>) -> io::Result<Option<Vec<PlaybackCommand>>> {
        let Some(&event) = self.events.get(&event) else { return Ok(None); };
        let bytes = self.bytes(event);
        let count = read_u32_slice(bytes, 4)? as usize;
        let mut commands = Vec::new();
        let mut sources = HashMap::new();
        for index in 0..count {
            let id = read_u32_slice(bytes, 8 + index * 4)?;
            let Some(&action) = self.actions.get(&id) else { continue; };
            let action = self.bytes(action);
            if action.len() < 10 { return Err(invalid("truncated Wwise action")); }
            let target = read_u32_slice(action, 6)?;
            let command = match action[5] {
                1 => bank::stop_action(action).map(|stop| Some(PlaybackCommand::Stop(stop))),
                0x12 => bank::state_action(action).map(|state| Some(PlaybackCommand::SetState(state))),
                4 => {
                    self.resolve_node(target, &mut media, &mut HashSet::new(), &mut sources).and_then(|node| node.map(|node|
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
