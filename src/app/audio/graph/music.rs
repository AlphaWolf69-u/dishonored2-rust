//! Host integration for the music timeline. All source cursors, packet reads,
//! voice routing, bus effects and output remain in the shared sound engine.
use super::*;
use crate::assets::wwise::MusicGraph;
use crate::assets::wwise::{MusicKind, MusicFade};
use crate::gameplay::sound::music::{branch::{self, Branch}, timing::Segment as SegmentTiming};

pub(super) fn rebuild_controls() -> bool {
    static REBUILD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REBUILD.get_or_init(|| std::env::var("DH2_DEBUG_REBUILD_MUSIC_CONTROLS").as_deref() == Ok("1"))
}
fn trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_MUSIC").as_deref() == Ok("1"))
}
/// Music contexts enter the same transition owner as ordinary PBIs
/// (14114d880/14114daf0 -> 1410efda0 -> 141130c50). Scheduling offsets
/// remain sample positions; gain durations round up to 21-ms ticks.
fn context_gain(now: i64, start: i64, fade: MusicFade, outgoing: bool) -> Option<f32> {
    if now < start { return Some(if outgoing { 1.0 } else { 0.0 }); }
    let duration = super::super::fade::frames(fade.milliseconds.max(0), 48000);
    // A negative pre-entry offset may precede clock zero. Shift by whole
    // quanta to preserve the shared envelope's absolute tick alignment and
    // its final zero-target ramp block before source retirement.
    let shift = if start < 0 { (-start + 1023) / 1024 * 1024 } else { 0 };
    let start = (start + shift) as u64;
    let envelope = if outgoing { super::super::fade::Envelope::new(start, duration, 1.0, fade.curve as u8) }
        else { super::super::fade::Envelope::fade_in(start, duration, fade.curve as u8) };
    envelope.block_value((now + shift) as u64)
}
pub(super) struct UpdateCosts { pub(super) root: u32, sources: usize, calls: std::sync::atomic::AtomicU64, rebuilds: std::sync::atomic::AtomicU64, cycles: std::sync::atomic::AtomicU64 }
pub(super) struct CostSample<'a> { cost: &'a UpdateCosts, start: Option<u64>, pub(super) rebuild: bool }
impl UpdateCosts {
    pub(super) fn new(root: u32, sources: usize) -> Option<Self> {
        trace().then(|| Self { root, sources, calls: 0.into(), rebuilds: 0.into(), cycles: 0.into() })
    }
    pub(super) fn sample(&self) -> CostSample<'_> { CostSample { cost: self, start: crate::loading::thread_cycles(), rebuild: false } }
}
impl Drop for CostSample<'_> {
    fn drop(&mut self) {
        if let Some((start, end)) = self.start.zip(crate::loading::thread_cycles()) {
            self.cost.cycles.fetch_add(end.saturating_sub(start), Ordering::Relaxed);
        }
        self.cost.calls.fetch_add(1, Ordering::Relaxed);
        self.cost.rebuilds.fetch_add(u64::from(self.rebuild), Ordering::Relaxed);
    }
}
impl Drop for UpdateCosts {
    fn drop(&mut self) {
        eprintln!("MUSIC_CONTROL_COST root={:08x} sources={} force_rebuild={} calls={} rebuilds={} thread_cycles={}", self.root, self.sources,
            rebuild_controls(), self.calls.load(Ordering::Relaxed), self.rebuilds.load(Ordering::Relaxed), self.cycles.load(Ordering::Relaxed));
    }
}

pub(super) struct Prepared {
    graph: Arc<MusicGraph>,
    sources: HashMap<(u32, u32), usize>,
    pub(super) controls_static: bool,
}

struct ClipVoice {
    track: u32,
    clip: usize,
    node: usize,
    start: i64,
    end: i64,
    admit: i64,
    source_offset: i32,
    source_duration: i32,
    runtime: Option<Runtime>,
    automation_tick: Option<u64>,
    automation: [f32; 5],
    random_filters: [f32; 2],
    ended: bool,
}
struct SegmentVoice {
    serial: u64,
    actor: u32,
    origin: i64,
    entry: i64,
    exit: i64,
    stop: Option<(i64, MusicFade)>,
    fade_in: Option<(i64, MusicFade)>,
    clips: Vec<ClipVoice>,
}
impl SegmentVoice {
    fn new(serial: u64, actor: u32, entry: i64, seek: u32, pre_entry: bool, fade: Option<MusicFade>, prepared: &Prepared) -> Option<Self> {
        let definition = prepared.graph.actors.get(&actor)?;
        let (entry_offset, exit_offset, tracks) = match &definition.kind {
            MusicKind::Segment { timing, tracks } => (timing.entry(), timing.exit(), tracks.as_slice()),
            MusicKind::Track { clips, .. } => (0, clips.iter().map(|clip| clip.start.wrapping_add(clip.duration)).max().unwrap_or(0), std::slice::from_ref(&actor)),
            _ => return None,
        };
        let origin = entry - i64::from(entry_offset) - i64::from(seek);
        // 141152ee0 / 14115caa0: a negative destination fade offset
        // starts the new source before its synchronization point, including
        // when SameTime selects a nonzero position inside the segment.
        let first_position = if let Some(fade) = fade.filter(|fade| fade.milliseconds > 0) {
            let position = i64::from(seek) + i64::from(fade.offset_ms) * 48;
            if pre_entry { position } else { position.max(0) }
        } else if pre_entry && seek == 0 { -i64::from(entry_offset) } else { i64::from(seek) };
        let first = origin + i64::from(entry_offset) + first_position;
        let mut voices = Vec::new();
        for &track in tracks {
            let MusicKind::Track { clips, lookahead, .. } = &prepared.graph.actors.get(&track)?.kind else { return None; };
            for (index, clip) in clips.iter().enumerate() {
                let Some(&node) = prepared.sources.get(&(track, clip.source)) else { continue; };
                let start = origin + i64::from(clip.start);
                let end = start + i64::from(clip.duration);
                if end <= first { continue; }
                let first = start.max(first);
                voices.push(ClipVoice { track, clip: index, node, start: first, end,
                    // 141159280 / 141152640: create source reads before its
                    // scheduled clip; source offset accounts for trimmed entry.
                    admit: first - i64::from(*lookahead).max(0) * 48,
                    source_offset: ((i64::from(clip.source_offset) + first - start) % i64::from(clip.source_duration)) as i32,
                    source_duration: clip.source_duration, runtime: None, automation_tick: None,
                    automation: [1.0, 0.0, 0.0, 1.0, 1.0], random_filters: [0.0; 2], ended: false });
            }
        }
        voices.sort_by_key(|voice| voice.admit);
        Some(Self { serial, actor, origin, entry: entry - i64::from(seek), exit: entry + i64::from(exit_offset) - i64::from(seek),
            stop: None, fade_in: fade.map(|fade| (entry + i64::from(fade.offset_ms) * 48, fade)), clips: voices })
    }
    fn position(&self, now: i64) -> u32 { now.saturating_sub(self.entry).max(0) as u32 }
    fn frame(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32], mut routing: Option<&mut RouteContext<'_>>, parent_gain: f32) -> bool {
        let Some(prepared) = audio.definition.music.as_ref() else { return false; };
        let time = now as i64;
        let stop_gain = self.stop.map_or(Some(1.0), |(end, fade)| {
            context_gain(time, end - i64::from(fade.milliseconds).max(0) * 48, fade, true)
        });
        let Some(mut segment_gain) = stop_gain else { self.clips.clear(); return false; };
        segment_gain *= parent_gain;
        if let Some((start, fade)) = self.fade_in {
            segment_gain *= context_gain(time, start, fade, false).unwrap_or(0.0);
        }
        let mut alive = time < self.exit;
        for voice in &mut self.clips {
            if voice.ended { continue; }
            if time >= voice.end { voice.runtime = None; voice.ended = true; continue; }
            alive = true;
            if time < voice.admit { continue; }
            if voice.runtime.is_none() {
                let mut runtime = Runtime::new(voice.node, now, &audio.definition, &audio.control);
                let Kind::Sound(index) = audio.definition.nodes[voice.node].kind else { continue; };
                let sound = &audio.definition.sounds[index];
                if let State::Sound { resampler, random, .. } = &mut runtime.state {
                    voice.random_filters = [random.low_pass, random.high_pass];
                    let Ok(step) = pitch_step(0.0, sound.sample_rate.get(), audio.definition.rate.get()) else {
                        voice.ended = true; continue;
                    };
                    let mut cursor = Resampler::new(step);
                    let offset = i64::from(voice.source_offset) + (time - voice.start).max(0);
                    cursor.skip((offset as u64 * u64::from(sound.sample_rate.get())) / 48000);
                    *resampler = Some(cursor);
                    // 141152250 applies the clip's engine-frame modulo to
                    // its initial seek. Source wrapping still uses the WEM's
                    // exact frame count, already prepared by the sound owner.
                }
                voice.runtime = Some(runtime);
                if trace() {
                    eprintln!("MUSIC_CLIP segment={:08x} track={:08x} source={:08x} admitted={now} start={} end={} source_offset={} source_duration={} source_rate={}",
                        self.actor, voice.track, match &prepared.graph.actors[&voice.track].kind {
                            MusicKind::Track { clips, .. } => clips[voice.clip].source, _ => 0 },
                        voice.start, voice.end, voice.source_offset, voice.source_duration, sound.sample_rate.get());
                }
            }
            if time < voice.start { continue; }
            let runtime = voice.runtime.as_mut().unwrap();
            if voice.automation_tick != Some(now / 1024) {
                voice.automation_tick = Some(now / 1024);
                if let MusicKind::Track { clips, automation, .. } = &prepared.graph.actors[&voice.track].kind {
                    voice.automation = [1.0, 0.0, 0.0, 1.0, 1.0];
                    let input = (time - self.origin - i64::from(clips[voice.clip].start)) as f32;
                    for curve in automation.iter().filter(|curve| curve.clip as usize == voice.clip) {
                        let Some(value) = crate::gameplay::sound::mix::conversion_value(&curve.points, 0, input) else { continue; };
                        if let Some(target) = voice.automation.get_mut(curve.kind as usize) {
                            *target = if curve.kind == 0 { value + 1.0 } else { value };
                        }
                    }
                }
                if let State::Sound { random, .. } = &mut runtime.state {
                    random.low_pass = voice.random_filters[0] + voice.automation[1];
                    random.high_pass = voice.random_filters[1] + voice.automation[2];
                }
            }
            let mut local = [0.0; 8];
            let Some(fade_gain) = runtime.fade_value(now) else {
                voice.runtime = None;
                voice.ended = true;
                continue;
            };
            // 14114d650/14114e0e0 -> PBI gain ->14113ef50: all context
            // and clip amplitudes enter the shared speaker-product ramp.
            let source_gain = segment_gain * voice.automation[0] * voice.automation[3] * voice.automation[4] * fade_gain;
            let voice_alive = if let Some(context) = routing.as_deref_mut() {
                runtime.frame_inner(now, audio, &mut local[..output.len()], Some(context), source_gain)
            } else {
                let result = runtime.frame_inner(now, audio, &mut local[..output.len()], None, source_gain);
                for (out, sample) in output.iter_mut().zip(local) { *out += sample; }
                result
            };
            if !voice_alive { voice.runtime = None; voice.ended = true; }
        }
        alive
    }
}

/// One music hierarchy per Play. It owns only selection cursors and active
/// segment/clip voices; the Definition retains shared bank/media metadata.
pub(super) struct Playback {
    branch: Branch,
    segments: Vec<SegmentVoice>,
    current: Option<u64>,
    next_serial: u64,
    pending: std::collections::VecDeque<(i64, Option<u64>)>,
    stopped: bool,
    initialized: bool,
    action_fade: Option<super::super::fade::Envelope>,
}
impl Playback {
    pub(super) fn new(audio: &ScheduledAudio) -> Option<Self> {
        let prepared = audio.definition.music.as_ref()?;
        let binding = audio.switches.as_ref()?;
        let states = binding.states.lock().unwrap_or_else(|error| error.into_inner());
        let branch = Branch::new(prepared.graph.root, &prepared.graph, &mut |group, _kind| states.current(group), &mut random15)?;
        Some(Self { branch, segments: Vec::new(), current: None, next_serial: 0, pending: Default::default(),
            stopped: false, initialized: false, action_fade: None })
    }
    pub(super) fn fade_in(&mut self, now: u64, milliseconds: i32, curve: u8) {
        if milliseconds != 0 { self.action_fade = Some(super::super::fade::Envelope::fade_in(now,
            super::super::fade::frames(milliseconds, 48000), curve)); }
    }
    fn add(&mut self, actor: u32, entry: i64, seek: u32, pre_entry: bool, fade: Option<MusicFade>, audio: &ScheduledAudio) -> Option<u64> {
        self.next_serial = self.next_serial.wrapping_add(1);
        let voice = SegmentVoice::new(self.next_serial, actor, entry, seek, pre_entry, fade, audio.definition.music.as_ref()?)?;
        let serial = voice.serial;
        self.segments.push(voice);
        Some(serial)
    }
    fn timing<'a>(prepared: &'a Prepared, actor: u32) -> Option<&'a SegmentTiming> {
        match &prepared.graph.actors.get(&actor)?.kind { MusicKind::Segment { timing, .. } => Some(timing), _ => None }
    }
    fn select(&mut self, audio: &ScheduledAudio) -> Option<u32> {
        let prepared = audio.definition.music.as_ref()?;
        let states = audio.switches.as_ref()?.states.lock().unwrap_or_else(|error| error.into_inner());
        self.branch.next(&prepared.graph, &mut |group, _kind| states.current(group), &mut random15)
    }
    fn schedule(&mut self, mut destination: Option<u32>, owner: u32, now: u64, changed: bool, audio: &ScheduledAudio) {
        let prepared = audio.definition.music.as_ref().unwrap();
        let source_actor = self.current.and_then(|serial| self.segments.iter().find(|voice| voice.serial == serial)).map_or(0, |source| source.actor);
        let selected_rule = branch::rule(&prepared.graph, owner, source_actor, destination.unwrap_or(0));
        if let Some(id) = selected_rule.map(|rule| rule.destination.jump_to).filter(|&id| id != 0) {
            if self.branch.jump(&prepared.graph, id, &mut random15) { destination = self.select(audio); }
        }
        let source = self.current.and_then(|serial| self.segments.iter().find(|voice| voice.serial == serial));
        let mut cut = source.map_or(now as i64, |source| source.exit);
        let mut cue = 0;
        if changed {
            let source_lead = selected_rule.map_or(0, |rule|
                (i64::from(rule.source.fade.milliseconds) - i64::from(rule.source.fade.offset_ms)).max(0) * 48);
            let seek = if selected_rule.is_some_and(|rule| rule.destination.entry == 1) {
                let position = source.map_or(0, |source| source.position(now as i64 + source_lead));
                destination.and_then(|id| Self::timing(prepared, id)).map_or(position, |timing|
                    if timing.exit() > 0 { position % timing.exit() as u32 } else { 0 })
            } else { 0 };
            let destination_lead = destination.map_or(0, |id| prepared.lead(id, seek,
                selected_rule.is_none_or(|rule| rule.destination.play_pre_entry), selected_rule.map(|rule| rule.destination.fade)));
            let minimum = now as i64 + source_lead.max(destination_lead);
            cut = minimum;
            if let (Some(source), Some(rule), Some(timing)) = (source, selected_rule, Self::timing(prepared, source_actor)) {
                let position = source.position(minimum);
                let skip_entry = !(rule.destination.entry == 1 || (rule.destination.match_source_cue_name && matches!(rule.destination.entry, 2 | 3)));
                if let Some((marker, position)) = timing.sync(position, rule.source.sync, rule.source.cue_filter,
                    skip_entry, branch::meter(&prepared.graph, source_actor)) {
                    cut = source.entry + i64::from(position);
                    cue = marker;
                } else { cut = source.exit.max(now as i64); }
            }
        }
        let source_serial = self.current;
        let source_position = source.map_or(0, |source| source.position(cut));
        if let Some(serial) = source_serial {
            if let Some(source) = self.segments.iter_mut().find(|voice| voice.serial == serial) {
                if let Some(rule) = selected_rule {
                    let fade = rule.source.fade;
                    if !(fade.milliseconds == 0 && fade.offset_ms == 0 && rule.source.play_post_exit) {
                        source.stop = Some((cut + i64::from(fade.offset_ms) * 48, fade));
                    }
                } else if changed { source.stop = Some((cut, MusicFade { milliseconds: 0, curve: 4, offset_ms: 0 })); }
            }
        }
        let mut destination_entry = cut;
        if let Some(bridge) = selected_rule.and_then(|rule| rule.bridge.as_ref()) {
            if let Some(serial) = self.add(bridge.segment, cut, 0, bridge.play_pre_entry, Some(bridge.fade_in), audio) {
                let segment = self.segments.iter_mut().find(|voice| voice.serial == serial).unwrap();
                segment.fade_in = Some((cut + i64::from(bridge.fade_in.offset_ms) * 48, bridge.fade_in));
                destination_entry = segment.exit;
                if !(bridge.fade_out.milliseconds == 0 && bridge.fade_out.offset_ms == 0 && bridge.play_post_exit) {
                    segment.stop = Some((destination_entry + i64::from(bridge.fade_out.offset_ms) * 48, bridge.fade_out));
                }
                self.pending.push_back((cut, Some(serial)));
            }
        }
        let mut next = None;
        if let Some(destination) = destination {
            let mut seek = 0;
            let mut pre_entry = true;
            let mut fade = None;
            if let Some(rule) = selected_rule {
                let settings = &rule.destination;
                let minimum = if settings.entry == 1 {
                    Self::timing(prepared, destination).map_or(source_position, |timing|
                        if timing.exit() > 0 { source_position % timing.exit() as u32 } else { 0 })
                } else { 0 };
                let filter = if settings.match_source_cue_name { cue } else { settings.cue_filter };
                seek = Self::timing(prepared, destination).map_or(minimum, |timing|
                    timing.destination(settings.entry, minimum, filter, random15).1);
                pre_entry = settings.play_pre_entry;
                fade = Some(settings.fade);
            }
            next = self.add(destination, destination_entry, seek, pre_entry, fade, audio);
            if let Some(fade) = fade {
                if let Some(segment) = next.and_then(|serial| self.segments.iter_mut().find(|voice| voice.serial == serial)) {
                    segment.fade_in = Some((destination_entry + i64::from(fade.offset_ms) * 48, fade));
                }
            }
        }
        self.pending.push_back((destination_entry, next));
        if trace() {
            eprintln!("MUSIC_TRANSITION owner={owner:08x} source={source_actor:08x} destination={:08x} requested={now} sync={cut} current_seek={source_position} changed={changed}", destination.unwrap_or(0));
        }
    }
    pub(super) fn begin(&mut self, now: u64, deadline: u64, audio: &ScheduledAudio) {
        if self.stopped { return; }
        if !self.initialized {
            self.initialized = true;
            if let Some(actor) = self.select(audio) {
                let owner = audio.definition.music.as_ref().unwrap().graph.root;
                self.schedule(Some(actor), owner, deadline, true, audio);
            }
        }
        let prepared = audio.definition.music.as_ref().unwrap();
        let changed = {
            let Some(binding) = &audio.switches else { return; };
            let states = binding.states.lock().unwrap_or_else(|error| error.into_inner());
            self.branch.refresh(&prepared.graph, &mut |group, _kind| states.current(group), &mut random15)
        };
        if let Some(owner) = changed {
            for (_, serial) in self.pending.drain(..) {
                if let Some(serial) = serial.filter(|serial| Some(*serial) != self.current) { self.segments.retain(|voice| voice.serial != serial); }
            }
            let next = self.select(audio);
            self.schedule(next, owner, now, true, audio);
        }
        while let Some(&(cut, next)) = self.pending.front() {
            if (now as i64) < cut { break; }
            self.current = next;
            self.pending.pop_front();
        }
        if self.pending.is_empty() {
            if let Some(source) = self.current.and_then(|serial| self.segments.iter().find(|voice| voice.serial == serial)) {
                let owner = source.actor;
                // Playlist rules belong to its parent; music ancestors are
                // already in the bank owner's shared property chain.
                let owner = prepared.graph.actors[&owner].properties.nodes.iter().find_map(|node|
                    prepared.graph.actors.get(&node.id).and_then(|actor| matches!(actor.kind, MusicKind::Playlist { .. }).then_some(node.id))).unwrap_or(owner);
                let next = self.select(audio);
                self.schedule(next, owner, now, false, audio);
            }
        }
    }
    pub(super) fn finished(&self) -> bool { self.stopped && self.segments.is_empty() }
    pub(super) fn stop(&mut self, target: Option<u32>, milliseconds: i32, curve: u8, now: u64, audio: &ScheduledAudio, paused: bool) {
        let prepared = audio.definition.music.as_ref().unwrap();
        let all = target.is_none_or(|target| audio.definition.matches_actor(audio.definition.root, target));
        if all { self.stopped = true; self.pending.clear(); }
        // 1411068e0: a paused PBI retires immediately. Its frozen sample
        // clock cannot execute a zero-target ramp or a scheduled stop later.
        if paused {
            self.segments.retain_mut(|segment| {
                if all || target.is_some_and(|target| prepared.graph.actors[&segment.actor].properties.nodes.iter().any(|node| node.id == target)) { return false; }
                segment.clips.retain(|voice| !target.is_some_and(|target| audio.definition.matches_actor(voice.node, target)));
                true
            });
            self.pending.retain(|(_, serial)| serial.is_none_or(|serial| self.segments.iter().any(|voice| voice.serial == serial)));
            return;
        }
        for segment in &mut self.segments {
            if all || target.is_some_and(|target| prepared.graph.actors[&segment.actor].properties.nodes.iter().any(|node| node.id == target)) {
                segment.stop = Some((now as i64 + i64::from(milliseconds).max(0) * 48,
                    MusicFade { milliseconds, curve: u32::from(curve), offset_ms: 0 }));
            } else if let Some(target) = target {
                for voice in &mut segment.clips {
                    if audio.definition.matches_actor(voice.node, target) {
                        voice.end = voice.end.min(now as i64 + i64::from(milliseconds).max(0) * 48);
                        if let Some(runtime) = &mut voice.runtime { runtime.stop(target, milliseconds, curve, now, &audio.definition); }
                    }
                }
            }
        }
    }
    pub(super) fn frame(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32], mut routing: Option<&mut RouteContext<'_>>) -> bool {
        let gain = self.action_fade.map_or(1.0, |fade| fade.block_value(now).unwrap_or(0.0));
        if let Some(context) = routing.as_deref_mut() {
            self.segments.retain_mut(|segment| segment.frame(now, audio, output, Some(&mut *context), gain));
        } else {
            self.segments.retain_mut(|segment| segment.frame(now, audio, output, None, gain));
        }
        !self.segments.is_empty() || (!self.stopped && (!self.pending.is_empty() || self.branch.waiting()))
    }
}
impl Prepared {
    pub(super) fn inventory(&self) -> (u32, usize, usize) { (self.graph.root, self.graph.actors.len(), self.sources.len()) }
    /// Source preparation horizon (14114f950 / 141152250), in frames before
    /// the selected segment's synchronization point. Only the selected clips
    /// acquire stream readers; other branches retain format metadata.
    fn lead(&self, actor: u32, seek: u32, pre_entry: bool, fade: Option<MusicFade>) -> i64 {
        let Some(definition) = self.graph.actors.get(&actor) else { return 0; };
        let (entry, tracks) = match &definition.kind {
            MusicKind::Segment { timing, tracks } => (timing.entry(), tracks.as_slice()),
            MusicKind::Track { .. } => (0, std::slice::from_ref(&actor)),
            _ => return 0,
        };
        let first = if let Some(fade) = fade.filter(|fade| fade.milliseconds > 0) {
            let position = i64::from(seek) + i64::from(fade.offset_ms) * 48;
            if pre_entry { position } else { position.max(0) }
        } else if pre_entry && seek == 0 { -i64::from(entry) } else { i64::from(seek) };
        let mut minimum = 0;
        for track in tracks {
            let Some(definition) = self.graph.actors.get(track) else { continue; };
            let MusicKind::Track { clips, lookahead, .. } = &definition.kind else { continue; };
            for clip in clips {
                let start = i64::from(clip.start) - i64::from(entry);
                if start + i64::from(clip.duration) <= first { continue; }
                let admit = start.max(first) - i64::from(seek) - i64::from(*lookahead).max(0) * 48;
                minimum = minimum.min(admit);
            }
        }
        -minimum
    }
    pub(super) fn decode(graph: Arc<MusicGraph>,
        attenuations: &HashMap<u32, Arc<crate::assets::wwise::Attenuation>>,
        locate: &dyn Fn((usize, u32)) -> Option<StreamLocator>, generation: u64,
    ) -> io::Result<Definition> {
        let properties = Arc::new(graph.actors.get(&graph.root).ok_or_else(|| invalid("missing music root"))?.properties.clone());
        let mut nodes = Vec::with_capacity(graph.media.len() + 1);
        let mut sounds = Vec::with_capacity(graph.media.len());
        let mut sources = HashMap::with_capacity(graph.media.len());
        for media in &graph.media {
            let variant = &media.variant;
            let identity = (media.track, media.source);
            let (samples, channels, sample_rate, frames, _) = match variant.streamed.and_then(locate) {
                Some(locator) => prepare_streamed(identity, &variant.media, locator, generation)?,
                None => prepare_media(identity, &variant.media, generation)?,
            };
            if frames == 0 { return Err(invalid("music source contains no samples")); }
            let index = sounds.len();
            sources.insert(identity, nodes.len());
            nodes.push(Node { id: media.track, ancestors: variant.properties.nodes.iter().map(|node| node.id).collect(), kind: Kind::Sound(index) });
            sounds.push(Sound { samples, channels, sample_rate,
                properties: Arc::new(variant.properties.clone()),
                spatial: super::super::spatial::Voice::read(&variant.properties, attenuations).map_err(invalid)?,
                // Music clips own duration and source offset (141152640).
                // Do not apply Sound's AkPropID_Loop or RIFF loop region.
                looping: Some(crate::gameplay::sound::looping::SourceLoop::new(None, frames, 0).map_err(invalid)?),
                loop_count: crate::gameplay::sound::looping::LoopCount { count: 0, modifiers: None },
                threshold: variant.properties.below_threshold() });
        }
        let root = nodes.len();
        nodes.push(Node { id: graph.root, ancestors: properties.nodes.iter().map(|node| node.id).collect(), kind: Kind::NoSource });
        if sounds.iter().any(|sound| sound.channels.get() > 2) {
            return Err(invalid("music source requires host multichannel output conversion"));
        }
        let controls_static = sounds.iter().all(|sound| sound.spatial.is_none());
        if trace() { eprintln!("MUSIC_DEFINITION root={:08x} actors={} sources={} static_controls={controls_static}", graph.root, graph.actors.len(), sounds.len()); }
        Ok(Definition { nodes, sounds, root, properties, channels: ChannelCount::new(2).unwrap(),
            rate: SampleRate::new(48000).unwrap(), music: Some(Self { graph, sources, controls_static }) })
    }
}
