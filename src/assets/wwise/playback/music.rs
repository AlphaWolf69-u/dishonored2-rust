use super::*;
use crate::assets::wwise::{MusicActor, MusicGraph, MusicKind, MusicMedia, MusicAutomation, MusicRule,
    media_from_descriptor};
use crate::gameplay::sound::{music_clip::rounded_sample, music::timing::{Meter, Segment, Cue}};

impl PlaybackBanks<'_> {
    /// Immutable actor/source definitions are shared between Play events.
    /// The weak index cannot keep a graph, map bank or media stream resident.
    pub(super) fn resolve_music(&self, root: u32,
        media: &mut impl FnMut(usize, u32, u8) -> io::Result<Option<MediaBytes>>,
    ) -> io::Result<Arc<MusicGraph>> {
        if let Some(graph) = self.music_graphs.lock().unwrap().get(&root).and_then(Weak::upgrade) { return Ok(graph); }
        let mut graph = MusicGraph { root, actors: HashMap::new(), media: Vec::new(), meters: HashMap::new(), history: self.music_history.clone() };
        let mut pending = vec![root];
        let mut visited = HashSet::new();
        while let Some(id) = pending.pop() {
            if id == 0 || !visited.insert(id) { continue; }
            let object = *self.actors.get(&id).ok_or_else(|| invalid_owned(format!("missing music actor {id:#010x}")))?;
            let bytes = self.bytes(object);
            let bank = &self.banks[object.bank];
            let properties = self.media_properties(id)?;
            for parent in properties.nodes.iter().map(|node| node.id) {
                if graph.meters.contains_key(&parent) { continue; }
                let Some(object) = self.actors.get(&parent).copied() else { continue; };
                let data = self.bytes(object);
                let feedback = self.banks[object.bank].feedback;
                let local = match object.kind {
                    10 => Meter::read(bank::music_segment(data, feedback)?.node.meter, 48000),
                    12 => Meter::read(bank::music_switch(data, feedback)?.node.meter, 48000),
                    13 => Meter::read(bank::music_playlist(data, feedback)?.node.meter, 48000),
                    _ => continue,
                }.map_err(invalid_owned)?;
                if let Some(meter) = local { graph.meters.insert(parent, meter); }
            }
            let mut common = |node: &super::super::MusicNodePrelude<'_>| -> io::Result<_> {
                pending.extend(node.children());
                let stingers: Vec<_> = node.stingers().collect();
                pending.extend(stingers.iter().map(|entry| entry[1]));
                let meter = Meter::read(node.meter, 48000).map_err(invalid_owned)?;
                Ok((meter, stingers))
            };
            let (meter, stingers, kind) = match object.kind {
                10 => {
                    let segment = bank::music_segment(bytes, bank.feedback)?;
                    let (meter, stingers) = common(&segment.node)?;
                    let cues = segment.cues().map(|cue| cue.map(|cue| Cue { id: cue.id,
                        position: rounded_sample(48000.0 * cue.position_ms * 0.001) })).collect::<io::Result<Vec<_>>>()?;
                    if cues.len() < 2 { return Err(invalid("music segment has no entry/exit markers")); }
                    (meter, stingers, MusicKind::Segment { timing: Segment {
                        duration: rounded_sample(48000.0 * segment.duration_ms * 0.001), cues }, tracks: segment.node.children().collect() })
                },
                11 => {
                    let track = bank::music_track(bytes, bank.feedback)?;
                    // All 7,767 installed tracks use normal playback. Keep
                    // unsupported selectors explicit rather than treating a
                    // random/sequence/switch track as parallel subtracks.
                    if track.playback_type != 0 { return Err(invalid_owned(format!("music track selector {} is not implemented", track.playback_type))); }
                    let (index, data) = bank.media_sections()?;
                    for source in track.prelude.sources() {
                        let source = source?;
                        if source.plugin & 0xf != 1 { return Err(invalid_owned(format!("music source plugin {:#x} is not implemented", source.plugin))); }
                        let index = index.map(|bytes| BankMediaIndex { bytes, ordered: bank.media_index_ordered });
                        let (media, streamed) = media_from_descriptor(source.media, source.stream_type, index, data,
                            &mut |id, stream| media(object.bank, id, stream), |bytes| bank.bytes.media(bytes))?
                            .ok_or_else(|| invalid_owned(format!("missing music media {:#x}", source.media)))?;
                        graph.media.push(MusicMedia { track: id, source: source.media, variant: MediaVariant {
                            media, streamed: streamed.map(|source| (object.bank, source)), weight: 1, properties: properties.clone() } });
                    }
                    let clips = track.prelude.clips().filter_map(|clip| clip.prepare(48000).transpose())
                        .collect::<Result<Vec<_>, _>>().map_err(invalid_owned)?;
                    let automation = track.prelude.automations().map(|automation| automation.map(|automation| MusicAutomation {
                        clip: automation.clip, kind: automation.kind, points: automation.points().map(|mut point| {
                            // 141159600 stores engine sample positions as float.
                            point.input = rounded_sample(48000.0 * f64::from(point.input)) as f32;
                            point
                        }).collect() })).collect::<io::Result<_>>()?;
                    (None, Vec::new(), MusicKind::Track { clips, automation, lookahead: track.lookahead })
                },
                12 => {
                    let switch = bank::music_switch(bytes, bank.feedback)?;
                    // Every installed music controller uses a State group.
                    // Do not silently read object-scoped Switches as States.
                    if switch.groups().any(|(_, kind)| kind != 1) { return Err(invalid("music controller requires object-switch bindings")); }
                    let (meter, stingers) = common(&switch.node)?;
                    let rules = switch.rules.iter().map(|rule| rule.map(MusicRule::read)).collect::<io::Result<Vec<_>>>()?;
                    pending.extend(rules.iter().filter_map(|rule| rule.bridge.as_ref().map(|bridge| bridge.segment)));
                    (meter, stingers, MusicKind::Switch { groups: switch.groups().collect(), tree: switch.tree.to_vec(),
                        mode: switch.tree_mode, continue_playback: switch.continue_playback, rules })
                },
                13 => {
                    let playlist = bank::music_playlist(bytes, bank.feedback)?;
                    let (meter, stingers) = common(&playlist.node)?;
                    let items = playlist.items().collect::<Vec<_>>();
                    pending.extend(items.iter().map(|item| item.segment));
                    let rules = playlist.rules.iter().map(|rule| rule.map(MusicRule::read)).collect::<io::Result<Vec<_>>>()?;
                    pending.extend(rules.iter().filter_map(|rule| rule.bridge.as_ref().map(|bridge| bridge.segment)));
                    let playlist = Arc::new(crate::gameplay::sound::music::playlist::Playlist::new(items).map_err(invalid_owned)?);
                    (meter, stingers, MusicKind::Playlist { playlist, rules })
                },
                _ => return Err(invalid_owned(format!("non-music actor {id:#x} in music hierarchy"))),
            };
            graph.actors.insert(id, MusicActor { properties, meter, stingers, kind });
        }
        let mut cache = self.music_graphs.lock().unwrap();
        if let Some(graph) = cache.get(&root).and_then(Weak::upgrade) { return Ok(graph); }
        let graph = Arc::new(graph);
        cache.insert(root, Arc::downgrade(&graph));
        Ok(graph)
    }
}
