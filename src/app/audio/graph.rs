//! Sample-clock playback of nested Wwise containers. Media is decoded before
//! submission; the audio worker selects nodes and mixes PCM or authored generators.
use super::pcm::PcmAudio;
use crate::{assets::wwise::{ContainerPlayback, MediaProperties, PlaybackNode, SelectionMode, TransitionMode},
    gameplay::sound::{containers::{transition_ms, Loops, Selection}, filters::{Kind as FilterKind, VoiceFilter},
        mix::{Mixer, power10}, properties::RandomProperties, resampler::{pitch_step, Resampler}, gain::GainRamp, Emitter}};
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use hashbrown::HashMap;
use std::{io, sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}}, time::Duration};

mod sequence;
mod flow;
use sequence::Sequence;

pub(super) type Selections = Arc<Mutex<HashMap<(u32, Option<Emitter>), Selection>>>;

/// Vorbis and IMA ADPCM stay encoded and are decoded per voice while
/// playing, as Wwise does; PCM media are decoded when the event is prepared.
#[derive(Clone)]
enum Samples { Pcm(PcmAudio), Vorbis(Arc<crate::assets::wwise::EncodedVorbis>), Adpcm(Arc<crate::assets::wwise::EncodedAdpcm>),
    File(Arc<crate::assets::wwise::file_source::FileMedia>, Option<Arc<Samples>>),
    Silence(crate::gameplay::sound::silence::Parameters), Streamed(Arc<StreamedSource>) }

/// Package location of a streamed source's media.
#[derive(Clone)]
pub(super) struct StreamLocator {
    pub(super) path: std::path::PathBuf,
    pub(super) offset: u64,
    pub(super) size: usize,
    pub(super) pool: Arc<crate::assets::wwise::stream_io::ReadPool>,
}

/// A streamed or prefetched Wwise source (stream type != 0). The original
/// reads its media through the stream manager while it plays instead of
/// keeping it resident. Here its format is read at load; its media is read
/// when an event plays it, held by that playback, and released when no
/// playback holds it any more.
pub(super) struct StreamedSource {
    sound: u32,
    prefix: Option<crate::assets::wwise::MediaBytes>,
    prefetched: Option<Arc<Samples>>,
    locator: StreamLocator,
    resident: Mutex<std::sync::Weak<Samples>>,
    loading: Mutex<std::sync::Weak<StreamRead>>,
}

struct StreamRead {
    state: Mutex<StreamReadState>,
}

struct StreamReadState {
    task: Option<bevy::tasks::Task<io::Result<Samples>>>,
    samples: Option<Arc<Samples>>,
}

/// A pending event owns its IO request; an admitted playback owns its media.
/// The cached source owns neither, so cancelling the last request also drops
/// a completed-but-unclaimed result (141160cc0 releases file-source state).
#[derive(Clone)]
pub(super) struct StreamHold {
    resident: Option<Arc<Samples>>,
    _read: Option<Arc<StreamRead>>,
}

impl StreamHold {
    pub(super) fn is_ready(&self) -> bool { self.resident.is_some() }
}

impl Drop for StreamRead {
    fn drop(&mut self) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let unclaimed = self.state.get_mut().unwrap_or_else(|e| e.into_inner()).task.is_some();
            eprintln!("MEMORY sound read release: unclaimed={unclaimed}");
        }
    }
}

impl StreamedSource {
    /// Game thread: retain this hold even while its IO is not ready. A retry
    /// acquires the replacement hold before dropping its previous one.
    pub(super) fn acquire(&self) -> io::Result<StreamHold> {
        if let Some(samples) = &self.prefetched { return Ok(StreamHold { resident: Some(samples.clone()), _read: None }); }
        if let Some(samples) = self.resident.lock().unwrap_or_else(|e| e.into_inner()).upgrade() {
            return Ok(StreamHold { resident: Some(samples), _read: None });
        }
        let mut loading = self.loading.lock().unwrap_or_else(|e| e.into_inner());
        let read = if let Some(read) = loading.upgrade() { read } else {
            let locator = self.locator.clone();
            let prefix = self.prefix.clone();
            let read = Arc::new(StreamRead { state: Mutex::new(StreamReadState {
                task: Some(bevy::tasks::IoTaskPool::get().spawn(async move { read_streamed(&locator, prefix) })),
                samples: None,
            }) });
            *loading = Arc::downgrade(&read);
            read
        };
        let mut state = read.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(samples) = &state.samples {
            return Ok(StreamHold { resident: Some(samples.clone()), _read: None });
        }
        let pending = state.task.as_mut().ok_or_else(|| invalid("streamed read completed without resident media"))?;
        if let Some(result) = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(pending)) {
            state.task = None;
            let samples = Arc::new(result?);
            state.samples = Some(samples.clone());
            *self.resident.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&samples);
            return Ok(StreamHold { resident: Some(samples), _read: None });
        }
        drop(state);
        Ok(StreamHold { resident: None, _read: Some(read) })
    }
}

fn read_streamed(locator: &StreamLocator, prefix: Option<crate::assets::wwise::MediaBytes>) -> io::Result<Samples> {
    use std::io::{Read, Seek, SeekFrom};
    let started = (std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1")).then(std::time::Instant::now);
    let bounded = if std::env::var("DH2_DEBUG_BOUNDED_SOUND_STREAMS").as_deref() == Ok("1") {
        crate::assets::wwise::file_source::FileMedia::open(locator.pool.clone(), Arc::new(locator.path.clone()), locator.offset, locator.size, prefix)?
    } else { None };
    if let Some(media) = bounded.as_ref() {
        if media.format.channels == 0 { return Err(invalid("invalid streamed source format")); }
        if std::env::var("DH2_DEBUG_COMPARE_STREAM_PCM").as_deref() != Ok("1") {
            return Ok(Samples::File(Arc::new(bounded.unwrap()), None));
        }
    }
    let mut file = std::fs::File::open(&locator.path)?;
    file.seek(SeekFrom::Start(locator.offset))?;
    let mut bytes = vec![0; locator.size];
    file.read_exact(&mut bytes)?;
    let io_ms = started.map(|started| started.elapsed().as_secs_f64() * 1000.0);
    let prepared = match crate::assets::wwise::prepare_shared_wem(&bytes.into())? {
        crate::assets::wwise::SourceMedia::Vorbis(media) => Ok(Samples::Vorbis(Arc::new(media))),
        crate::assets::wwise::SourceMedia::Adpcm(media) => Ok(Samples::Adpcm(Arc::new(media))),
        crate::assets::wwise::SourceMedia::Decoded(_) => Err(invalid("streamed Wwise source no longer stays encoded")),
    };
    if let Some(started) = started {
        eprintln!("MEMORY streamed source acquisition: bytes={} io_ms={:.3} total_ms={:.3}",
            locator.size, io_ms.unwrap(), started.elapsed().as_secs_f64() * 1000.0);
    }
    match bounded {
        Some(media) => Ok(Samples::File(Arc::new(media), Some(Arc::new(prepared?)))),
        None => prepared,
    }
}

/// A voice's decoder over its encoded media.
enum EncodedCursor {
    Vorbis(crate::assets::wwise::VorbisCursor), Adpcm(crate::assets::wwise::AdpcmCursor),
    File { cursor: crate::assets::wwise::file_source::FileCursor, reference: Option<Box<EncodedCursor>>,
        failed: bool, compared: u64, differences: u64, maximum_error: f32 },
}
impl EncodedCursor {
    fn new(samples: &Samples) -> Option<Self> {
        match samples {
            Samples::Vorbis(media) => Some(Self::Vorbis(crate::assets::wwise::VorbisCursor::new(media.clone()))),
            Samples::Adpcm(media) => Some(Self::Adpcm(crate::assets::wwise::AdpcmCursor::new(media.clone()))),
            Samples::File(media, reference) => Some(Self::File { cursor: crate::assets::wwise::file_source::FileCursor::new(media.clone()),
                reference: reference.as_ref().and_then(|reference| Self::new(reference)).map(Box::new),
                failed: false, compared: 0, differences: 0, maximum_error: 0.0 }),
            _ => None,
        }
    }
    fn sample(&mut self, frame: usize, channel: usize) -> Option<f32> {
        match self {
            Self::Vorbis(cursor) => cursor.sample(frame, channel),
            Self::Adpcm(cursor) => cursor.sample(frame, channel),
            Self::File { cursor, reference, failed, compared, differences, maximum_error } => {
                if *failed { return None; }
                match cursor.sample(frame, channel) {
                    Ok(value) => {
                        if let (Some(value), Some(reference)) = (value, reference.as_mut()) {
                            let expected = reference.sample(frame, channel);
                            *compared += 1;
                            if expected != Some(value) {
                                *differences += 1;
                                *maximum_error = maximum_error.max(expected.map_or(f32::INFINITY, |expected| (expected - value).abs()));
                                if *differences == 1 { eprintln!("STREAM_PCM_DIFFERENCE frame={frame} channel={channel} expected={expected:?} actual={value}"); }
                            }
                        }
                        value
                    }
                    Err(error) => { warn!("Streamed sound decoder failed: {error}"); *failed = true; None }
                }
            }
        }
    }
    fn waiting(&self) -> bool { matches!(self, Self::File { cursor, failed: false, .. } if cursor.waiting()) }
}

impl Drop for EncodedCursor {
    fn drop(&mut self) {
        if let Self::File { reference: Some(_), compared, differences, maximum_error, .. } = self {
            eprintln!("STREAM_PCM_COMPARISON samples={compared} differences={differences} maximum_error={maximum_error}");
        }
    }
}

enum Stream {
    Resident(EncodedCursor),
    Streamed { source: Arc<StreamedSource>, hold: Option<StreamHold>, decoder: Option<EncodedCursor>, retry_at: u64, failed: bool },
}
impl Stream {
    fn new(samples: &Samples, now: u64) -> Option<Self> {
        match samples {
            Samples::Streamed(source) => {
                let mut stream = Self::Streamed { source: source.clone(), hold: None, decoder: None, retry_at: 0, failed: false };
                stream.ready(now); // Selected-source preparation, before its scheduled frame.
                Some(stream)
            }
            _ => EncodedCursor::new(samples).map(Self::Resident),
        }
    }
    fn awaiting_read(&self, now: u64) -> bool {
        matches!(self, Self::Streamed { decoder, retry_at, failed: false, .. }
            if now < *retry_at && decoder.as_ref().is_none_or(EncodedCursor::waiting))
    }
    fn ready(&mut self, now: u64) -> bool {
        let Self::Streamed { source, hold, decoder, retry_at, failed } = self else { return true };
        if *failed { return true; }
        if let Some(decoder) = decoder {
            if decoder.waiting() {
                if now < *retry_at { return false; }
                *retry_at = (now / 1024).saturating_add(1).saturating_mul(1024);
            }
            return true;
        }
        if now < *retry_at { return false; }
        *retry_at = (now / 1024).saturating_add(1).saturating_mul(1024);
        match source.acquire() {
            Ok(next) => {
                if let Some(samples) = &next.resident {
                    *decoder = EncodedCursor::new(samples);
                    *failed = decoder.is_none();
                    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                        eprintln!("MEMORY sound selected source ready: source={:08x} source_bytes={} prefetched={} failed={failed}",
                            source.sound, source.locator.size, source.prefetched.is_some());
                    }
                }
                // Acquire the replacement before dropping an earlier pending
                // lease, so a completed read cannot disappear between retries.
                *hold = Some(next);
                decoder.is_some() || *failed
            }
            Err(error) => {
                warn!("Selected streamed sound unavailable: {error}");
                *hold = None;
                *failed = true;
                true // Sampling fails as EOF/error, never as an endless wait.
            }
        }
    }
    fn sample(&mut self, frame: usize, channel: usize) -> Option<f32> {
        match self {
            Self::Resident(cursor) => cursor.sample(frame, channel),
            Self::Streamed { decoder, .. } => decoder.as_mut()?.sample(frame, channel),
        }
    }
    fn waiting(&self) -> bool {
        match self { Self::Resident(cursor) => cursor.waiting(),
            Self::Streamed { decoder, failed, .. } => !*failed && decoder.as_ref().is_none_or(EncodedCursor::waiting) }
    }
}

/// Temporary same-build control of the former all-child admission path.
pub(super) fn preload_event_streams() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_PRELOAD_EVENT_STREAMS").as_deref() == Ok("1"))
}

/// Prepared media by (Wwise sound id, media size). A sound's media is
/// resident once however many events play it (1410f1910 / 141122400 share
/// a loaded Sound's source), across events as within one.
type PreparedMedia = (Samples, ChannelCount, SampleRate, usize, Option<[u32; 2]>);
static MEDIA: std::sync::LazyLock<Mutex<HashMap<(u32, usize), PreparedMedia>>> = std::sync::LazyLock::new(Default::default);
/// Streamed sources by (Wwise sound id, media size): one per sound, so all
/// events playing it share its resident media.
static STREAMED: std::sync::LazyLock<Mutex<HashMap<(u32, usize), PreparedMedia>>> = std::sync::LazyLock::new(Default::default);

/// Bank unload releases only its sound sources; common/init sources keep
/// their existing encoded/PCM identity (1410f1910 / 141122400).
pub(super) fn retain_media(sounds: &std::collections::HashSet<u32>) {
    MEDIA.lock().unwrap_or_else(|e| e.into_inner()).retain(|(sound, _), _| sounds.contains(sound));
    STREAMED.lock().unwrap_or_else(|e| e.into_inner()).retain(|(sound, _), _| sounds.contains(sound));
    crate::assets::wwise::release_unused_headers();
}

/// A streamed source's format, read once at load; its media is not kept.
/// Media that decode to PCM stay resident (`prepare_media`).
fn prepare_streamed(sound: u32, media: &crate::assets::wwise::MediaBytes, locator: StreamLocator) -> io::Result<PreparedMedia> {
    if let Some(prepared) = STREAMED.lock().unwrap_or_else(|e| e.into_inner()).get(&(sound, media.source_size())) {
        return Ok(prepared.clone());
    }
    let format = if media.is_prefetched() || std::env::var("DH2_DEBUG_EAGER_STREAM_ACQUISITION").as_deref() != Ok("1") {
        crate::assets::wwise::stream_format(media, media.source_size())?
    } else { None };
    let (channels, rate, frames, points) = if let Some(format) = format {
        (format.channels, format.sample_rate, format.frames, format.loop_points)
    } else {
        let format = if std::env::var("DH2_DEBUG_SOUND_COPIED_MEDIA").as_deref() == Ok("1") {
            crate::assets::wwise::prepare_wem(media)?
        } else { crate::assets::wwise::prepare_shared_wem(media)? };
        let (channels, rate, frames, points) = match format {
            crate::assets::wwise::SourceMedia::Vorbis(media) => (media.channels, media.sample_rate, media.frames, media.loop_points),
            crate::assets::wwise::SourceMedia::Adpcm(media) => (media.channels, media.sample_rate, media.frames, media.loop_points),
            crate::assets::wwise::SourceMedia::Decoded(_) => return prepare_media(sound, media),
        };
        (channels, rate, frames, points)
    };
    let channels = ChannelCount::new(channels).ok_or_else(|| invalid("invalid Wwise streamed channel count"))?;
    let rate = SampleRate::new(rate).ok_or_else(|| invalid("invalid Wwise streamed sample rate"))?;
    let prefix = media.is_prefetched().then(|| media.clone());
    let prefetched = if std::env::var("DH2_DEBUG_COMPARE_STREAM_PCM").as_deref() != Ok("1") {
        prefix.as_ref().map(|prefix| crate::assets::wwise::file_source::FileMedia::from_prefetch(
            locator.pool.clone(), Arc::new(locator.path.clone()), locator.offset, locator.size, prefix.clone()))
            .transpose()?.flatten().map(|media| Arc::new(Samples::File(Arc::new(media), None)))
    } else { None };
    let source = StreamedSource { sound, locator, prefix, prefetched, resident: Mutex::new(std::sync::Weak::new()), loading: Mutex::new(std::sync::Weak::new()) };
    let prepared = (Samples::Streamed(Arc::new(source)), channels, rate, frames, points);
    // Parallel event preparation can reach the same source concurrently.
    // Publish one identity; losing preparations must not escape into graphs.
    Ok(STREAMED.lock().unwrap_or_else(|e| e.into_inner())
        .entry((sound, media.source_size())).or_insert(prepared).clone())
}

fn prepare_media(sound: u32, media: &crate::assets::wwise::MediaBytes) -> io::Result<PreparedMedia> {
    if let Some(prepared) = MEDIA.lock().unwrap_or_else(|e| e.into_inner()).get(&(sound, media.len())) {
        return Ok(prepared.clone());
    }
    let _tag = crate::platform::allocation_tags::tag("sound media");
    let source = if std::env::var("DH2_DEBUG_SOUND_COPIED_MEDIA").as_deref() == Ok("1") {
        crate::assets::wwise::prepare_wem(media)?
    } else { crate::assets::wwise::prepare_shared_wem(media)? };
    let prepared = match source {
        crate::assets::wwise::SourceMedia::Vorbis(media) => {
            let channels = ChannelCount::new(media.channels).ok_or_else(|| invalid("invalid Wwise Vorbis channel count"))?;
            let rate = SampleRate::new(media.sample_rate).ok_or_else(|| invalid("invalid Wwise Vorbis sample rate"))?;
            let (frames, points) = (media.frames, media.loop_points);
            (Samples::Vorbis(Arc::new(media)), channels, rate, frames, points)
        }
        crate::assets::wwise::SourceMedia::Adpcm(media) => {
            let channels = ChannelCount::new(media.channels).ok_or_else(|| invalid("invalid Wwise IMA channel count"))?;
            let rate = SampleRate::new(media.sample_rate).ok_or_else(|| invalid("invalid Wwise IMA sample rate"))?;
            let (frames, points) = (media.frames, media.loop_points);
            (Samples::Adpcm(Arc::new(media)), channels, rate, frames, points)
        }
        crate::assets::wwise::SourceMedia::Decoded(audio) => {
            let pcm = PcmAudio::from_decoded(audio)?;
            let frames = pcm.samples.len() / usize::from(pcm.channels.get());
            let (channels, rate, points) = (pcm.channels, pcm.sample_rate, pcm.loop_points);
            (Samples::Pcm(pcm), channels, rate, frames, points)
        }
    };
    {
        let mut cache = MEDIA.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prepared) = cache.get(&(sound, media.len())) { return Ok(prepared.clone()); }
        cache.insert((sound, media.len()), prepared.clone());
    }
    Ok(prepared)
}

/// Current source views, not cumulative acquisitions from previously visited
/// maps. Logical view lengths can overlap a bank allocation; physical backing
/// is accounted for separately by resident_allocations and the bank owner.
pub(super) fn resident_media_stats() -> std::collections::BTreeMap<&'static str, (usize, usize, usize)> {
    let mut result = std::collections::BTreeMap::new();
    for ((_, source_bytes), prepared) in MEDIA.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        let (representation, logical_bytes) = match &prepared.0 {
            Samples::Vorbis(media) => ("Vorbis", media.ogg.len()),
            Samples::Adpcm(media) => ("IMA ADPCM", media.data.len()),
            Samples::Pcm(media) => ("decoded PCM", media.samples.len() * std::mem::size_of::<f32>()),
            Samples::Silence(_) | Samples::Streamed(_) | Samples::File(_, _) => continue,
        };
        let entry = result.entry(representation).or_insert((0, 0, 0));
        entry.0 += 1; entry.1 += source_bytes; entry.2 += logical_bytes;
    }
    result
}

/// Physical encoded allocations, not the sum of overlapping source views.
pub(super) fn resident_allocations() -> Vec<(usize, usize)> {
    MEDIA.lock().unwrap_or_else(|e| e.into_inner()).values().filter_map(|prepared| match &prepared.0 {
        Samples::Vorbis(media) => Some(media.ogg.allocation()),
        Samples::Adpcm(media) => Some(media.data.allocation()),
        _ => None,
    }).collect()
}

/// Numeric capture of the actual prepared source bytes and playback metadata.
/// Optional: reading every encoded byte is inappropriate for ordinary loads.
pub(super) fn capture_media_signatures() {
    use std::hash::{Hash, Hasher};
    if std::env::var("DH2_DEBUG_STREAM_FORMATS").as_deref() == Ok("1") {
        for (&(sound, size), (_, channels, rate, frames, points)) in STREAMED.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            println!("STREAM source={sound:08x} size={size} channels={} rate={} frames={frames} loops={points:?}", channels.get(), rate.get());
        }
    }
    if std::env::var("DH2_DEBUG_SOUND_HASHES").as_deref() != Ok("1") { return; }
    let cache = MEDIA.lock().unwrap_or_else(|e| e.into_inner());
    for (&(sound, size), (samples, channels, rate, frames, points)) in cache.iter() {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        match samples {
            Samples::Vorbis(media) => media.ogg.as_ref().hash(&mut hash),
            Samples::Adpcm(media) => media.data.as_ref().hash(&mut hash),
            Samples::Pcm(pcm) => for value in pcm.samples.iter() { value.to_bits().hash(&mut hash); },
            _ => continue,
        }
        println!("MEDIA source={sound:08x} size={size} channels={} rate={} frames={frames} loops={points:?} hash={:016x}",
            channels.get(), rate.get(), hash.finish());
    }
}
struct Sound {
    samples: Samples,
    channels: ChannelCount,
    sample_rate: SampleRate,
    properties: Arc<MediaProperties>,
    spatial: Option<super::spatial::Voice>,
    looping: Option<crate::gameplay::sound::looping::SourceLoop>,
    loop_count: crate::gameplay::sound::looping::LoopCount,
    threshold: (crate::assets::wwise::BelowThreshold, crate::assets::wwise::VirtualReturn),
}
impl Sound {
    fn sample(&self, resampler: &Resampler, channel: usize,
        looping: Option<crate::gameplay::sound::looping::SourceLoop>,
        silence: Option<&crate::gameplay::sound::silence::Voice>,
        stream: Option<&mut Stream>) -> Option<f32> {
        match &self.samples {
            Samples::Pcm(pcm) => resampler.sample_looped(&pcm.samples, usize::from(self.channels.get()), channel, looping),
            Samples::Vorbis(_) | Samples::Adpcm(_) | Samples::File(_, _) | Samples::Streamed(_) => {
                let stream = stream?;
                resampler.sample_with(|frame| {
                    let frame = match looping { Some(looping) => looping.frame(frame as u64)?, None => frame };
                    stream.sample(frame, channel)
                })
            }
            Samples::Silence(_) => resampler.sample_with(|frame| silence?.sample(frame as u64)),
        }
    }
}
struct Container {
    properties: crate::assets::wwise::NodeProperties,
    children: Vec<usize>,
    weights: Vec<u32>,
    mode: SelectionMode,
    global: bool,
    reset: bool,
    avoid: u16,
    playback: ContainerPlayback,
}
struct Switch { playback: Arc<crate::assets::wwise::SwitchPlayback>, children: HashMap<u32, Option<usize>> }
enum Kind { Missing, NoSource, Sound(usize), Layer(Vec<usize>), Container(Container), Switch(Switch) }
struct Node { id: u32, ancestors: Vec<u32>, kind: Kind }

pub(super) struct Definition {
    nodes: Vec<Node>,
    sounds: Vec<Sound>,
    root: usize,
    properties: Arc<MediaProperties>,
    channels: ChannelCount,
    rate: SampleRate,
}

fn invalid(message: impl Into<String>) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.into()) }
fn random15() -> u16 { rand::random_range(0..=32767) }

impl Definition {
    /// The streamed sources this playback reads while it plays.
    pub(super) fn streamed(&self) -> impl Iterator<Item = &Arc<StreamedSource>> {
        self.sounds.iter().filter_map(|sound| match &sound.samples { Samples::Streamed(source) => Some(source), _ => None })
    }
}

impl Definition {
    fn matches_actor(&self, node: usize, target: u32) -> bool {
        self.nodes[node].id == target || self.nodes[node].ancestors.contains(&target)
    }
    fn sample_accurate(&self) -> bool {
        self.nodes.iter().any(|node| matches!(&node.kind, Kind::Container(container)
            if container.playback.continuous && container.playback.transition == TransitionMode::SampleAccurate))
    }
    fn has_switch(&self) -> bool { self.nodes.iter().any(|node| matches!(node.kind, Kind::Switch(_))) }
    pub fn can_route(&self) -> bool { self.channels.get() == 2 && self.sounds.iter().all(|sound| sound.channels.get() <= 2) }
    /// `locate` gives the package location of a streamed source's media id.
    pub fn decode(root: PlaybackNode, attenuations: &HashMap<u32, Arc<crate::assets::wwise::Attenuation>>,
        locate: &dyn Fn(u32) -> Option<StreamLocator>) -> io::Result<Self> {
        // The Play target owns its properties even when no source is selected
        // (14111b7b0 / 1410ddd30). A descendant's output-bus override does not
        // belong to the target, and empty graphs have no descendant to borrow.
        let properties = Arc::new(match &root {
            PlaybackNode::Unsupported { node, reason } => return Err(invalid(format!("Wwise actor {node:#010x}: {reason}"))),
            PlaybackNode::Missing { .. } => return Err(invalid("missing Wwise Play target")),
            PlaybackNode::Generator { properties, .. } => properties.clone(),
            PlaybackNode::NoSource { properties, .. } => properties.clone(),
            PlaybackNode::Sound { variant, .. } => variant.properties.clone(),
            PlaybackNode::Layer { properties, .. } => properties.clone(),
            PlaybackNode::Container(container) => container.properties.clone(),
            PlaybackNode::Switch(switch) => switch.properties.clone(),
        });
        fn node(source: PlaybackNode, nodes: &mut Vec<Node>, sounds: &mut Vec<Sound>,
            sources: &mut HashMap<u32, usize>,
            attenuations: &HashMap<u32, Arc<crate::assets::wwise::Attenuation>>,
            locate: &dyn Fn(u32) -> Option<StreamLocator>) -> io::Result<usize> {
            let properties = match &source {
                PlaybackNode::Sound { variant, .. } => Some(&variant.properties),
                PlaybackNode::Generator { properties, .. } | PlaybackNode::NoSource { properties, .. } |
                PlaybackNode::Layer { properties, .. } => Some(properties),
                PlaybackNode::Container(container) => Some(&container.properties),
                PlaybackNode::Switch(switch) => Some(&switch.properties),
                _ => None,
            };
            let ancestors = properties.map(|properties| properties.nodes.iter().map(|node| node.id).collect()).unwrap_or_default();
            let source_id = match &source {
                PlaybackNode::Sound { node, .. } | PlaybackNode::Generator { node, .. } => Some(*node),
                _ => None,
            };
            if let Some((id, sound)) = source_id.and_then(|id| sources.get(&id).map(|&sound| (id, sound))) {
                let index = nodes.len();
                nodes.push(Node { id, ancestors, kind: Kind::Sound(sound) });
                return Ok(index);
            }
            let (id, kind) = match source {
                PlaybackNode::Unsupported { node, reason } => return Err(invalid(format!("Wwise actor {node:#010x}: {reason}"))),
                PlaybackNode::Switch(switch) => {
                    if switch.playback.group_kind != 1 {
                        return Err(invalid("per-emitter switch-group updates require their game-parameter bindings"));
                    }
                    if switch.playback.parameters.iter().any(|controls| controls.on_switch != 1) {
                        return Err(invalid("switch Play To End requires decoder-loop release"));
                    }
                    let mut children = HashMap::new();
                    for (id, child) in switch.children {
                        children.insert(id, child.map(|child| node(child, nodes, sounds, sources, attenuations, locate)).transpose()?);
                    }
                    (switch.node, Kind::Switch(Switch { playback: Arc::new(switch.playback), children }))
                },
                PlaybackNode::Missing { node } => (node, Kind::Missing),
                PlaybackNode::NoSource { node, .. } => (node, Kind::NoSource),
                PlaybackNode::Sound { node, variant } => {
                    let (samples, channels, sample_rate, frames, loop_points) = match variant.streamed.and_then(locate) {
                        Some(locator) => prepare_streamed(node, &variant.media, locator)?,
                        None => prepare_media(node, &variant.media)?,
                    };
                    if frames == 0 { return Err(invalid("scheduled Wwise sound contains no samples")); }
                    let sound = sounds.len();
                    let spatial = super::spatial::Voice::read(&variant.properties, attenuations).map_err(invalid)?;
                    let loop_count = crate::gameplay::sound::looping::LoopCount::read(&variant.properties).map_err(invalid)?;
                    let looping = crate::gameplay::sound::looping::SourceLoop::new(loop_points, frames, 1).map_err(invalid)?;
                    let threshold = variant.properties.below_threshold();
                    sounds.push(Sound { channels, sample_rate, samples,
                        properties: Arc::new(variant.properties), spatial, looping: Some(looping), loop_count, threshold });
                    (node, Kind::Sound(sound))
                },
                PlaybackNode::Generator { node, properties, definition } => {
                    if definition.plugin != 0x00650002 {
                        return Err(invalid(format!("source plugin {:#010x} requires its source processor", definition.plugin)));
                    }
                    if !definition.curves.is_empty() || !definition.values.is_empty() {
                        return Err(invalid("silence source parameter automation requires FX subscriptions"));
                    }
                    let parameters = crate::gameplay::sound::silence::Parameters::read(&definition.parameters).map_err(invalid)?;
                    let sound = sounds.len();
                    let spatial = super::spatial::Voice::read(&properties, attenuations).map_err(invalid)?;
                    let loop_count = crate::gameplay::sound::looping::LoopCount::read(&properties).map_err(invalid)?;
                    let threshold = properties.below_threshold();
                    sounds.push(Sound { samples: Samples::Silence(parameters), channels: ChannelCount::new(1).unwrap(),
                        sample_rate: SampleRate::new(48000).unwrap(), properties: Arc::new(properties), spatial,
                        looping: None, loop_count, threshold });
                    (node, Kind::Sound(sound))
                },
                PlaybackNode::Layer { node: id, children, .. } => {
                    let children = children.into_iter().map(|child| node(child, nodes, sounds, sources, attenuations, locate)).collect::<io::Result<_>>()?;
                    (id, Kind::Layer(children))
                },
                PlaybackNode::Container(container) => {
                    let mut children = Vec::new();
                    let mut weights = Vec::new();
                    for (child, weight) in container.children {
                        children.push(node(child, nodes, sounds, sources, attenuations, locate)?);
                        weights.push(weight);
                    }
                    let properties = container.properties.nodes.into_iter().next()
                        .filter(|properties| properties.id == container.node)
                        .ok_or_else(|| invalid("Wwise container has no own properties"))?;
                    (container.node, Kind::Container(Container { properties, children, weights, mode: container.mode,
                        global: container.global, reset: container.reset_each_play, avoid: container.avoid_repeat,
                        playback: container.playback }))
                },
            };
            // Sources share immutable decoded media/configuration. Each
            // Runtime still owns its cursor, randomization, filters and PBI
            // activity, including overlapping plays of the same actor.
            if let Kind::Sound(sound) = &kind { sources.insert(id, *sound); }
            let index = nodes.len();
            nodes.push(Node { id, ancestors, kind });
            Ok(index)
        }
        let mut nodes = Vec::new();
        let mut sounds = Vec::new();
        let root = node(root, &mut nodes, &mut sounds, &mut HashMap::new(), attenuations, locate)?;
        let sample_accurate = nodes.iter().any(|node| matches!(&node.kind, Kind::Container(container)
            if container.playback.continuous && container.playback.transition == TransitionMode::SampleAccurate));
        if sample_accurate && nodes.iter().any(|node| matches!(node.kind, Kind::Switch(_))) {
            return Err(invalid("switch playback with sample-accurate handoff requires its source-buffer transfer"));
        }
        if sample_accurate && nodes.iter().any(|node| matches!(&node.kind, Kind::Container(container)
            if container.playback.continuous && matches!(container.playback.transition,
                TransitionMode::CrossfadeAmplitude | TransitionMode::CrossfadePower))) {
            return Err(invalid("sample-accurate decoder handoff with crossfades requires its source-buffer transfer"));
        }
        let stereo = sounds.iter().all(|sound| sound.channels.get() <= 2);
        let channels = if stereo { ChannelCount::new(2).ok_or_else(|| invalid("invalid stereo channel count"))? }
            else { sounds[0].channels };
        if channels.get() > 8 { return Err(invalid("scheduled Wwise graph exceeds eight output channels")); }
        // The original engine's DSP/output clock is 48 kHz (142427c24).
        // Each source's fixed-point resampler converts its own media rate.
        let rate = SampleRate::new(48000).ok_or_else(|| invalid("invalid output sample rate"))?;
        if !stereo && sounds.iter().any(|sound| sound.spatial.is_some() || sound.channels != channels) {
            return Err(invalid("scheduled Wwise graph requires channel format conversion"));
        }
        Ok(Self { nodes, sounds, root, properties, channels, rate })
    }

    pub fn root_properties(&self) -> Arc<MediaProperties> {
        self.properties.clone()
    }

    pub fn prepare(self: &Arc<Self>, selections: Selections, emitter: Emitter, mixer: &Mixer,
        parameter: Option<(u32, f32)>, scene: &super::spatial::Scene, position: Option<super::spatial::Position>,
        settings: crate::assets::wwise::PlaySettings) -> Result<(ScheduledAudio, Playing), String>
    {
        if self.has_switch() && !self.can_route() { return Err("switch sources require shared-output channel conversion".into()); }
        let control = Arc::new(Control { volumes_db: (0..self.sounds.len()).map(|_| AtomicU32::new(0)).collect(),
            transitions: (0..self.nodes.len()).map(|_| AtomicU32::new(0)).collect(),
            spatial: (0..self.sounds.len()).map(|_| std::array::from_fn(|index| AtomicU32::new(if index < 4 { 1.0f32.to_bits() } else { 0 }))).collect(),
            rays: (0..self.sounds.len()).map(|_| std::array::from_fn(|_| AtomicU32::new(1.0f32.to_bits()))).collect(),
            pitches: (0..self.sounds.len()).map(|_| AtomicU32::new(0)).collect(),
            filters: (0..self.sounds.len()).map(|_| [AtomicU32::new(0), AtomicU32::new(0)]).collect(),
            source_ready: AtomicU32::new(0), threshold_db: AtomicU32::new(mixer.volume_threshold_db.to_bits()), stops: Mutex::new(Vec::new()),
            routes: Mutex::new(Arc::new(Vec::new())), switch_count: AtomicU32::new(0), activity: mixer.activity.clone(),
            registrations: self.sounds.iter().map(|sound| mixer.activity.as_ref()
                .map(|activity| activity.registration(sound.properties.bus)).transpose()).collect::<Result<Vec<_>, String>>()? });
        let playing = Playing { definition: self.clone(), control: control.clone(), parameter, position, emitter };
        playing.update(mixer, scene)?;
        Ok((ScheduledAudio { definition: self.clone(), control, selections, emitter,
            delay_frames: settings.delay.sample_frames(random15), settings, switches: None, event: None }, playing))
    }
}

struct Control { source_ready: AtomicU32, spatial: Vec<[AtomicU32; 16]>, rays: Vec<[AtomicU32; 3]>, volumes_db: Vec<AtomicU32>, pitches: Vec<AtomicU32>, filters: Vec<[AtomicU32; 2]>,
    transitions: Vec<AtomicU32>,
    threshold_db: AtomicU32, stops: Mutex<Vec<(Option<u32>, i32, u8)>>, routes: Mutex<Arc<Vec<VoiceRoute>>>,
    switch_count: AtomicU32, activity: Option<Arc<crate::gameplay::sound::activity::Activity>>,
    registrations: Vec<Option<crate::gameplay::sound::activity::Registration>> }

struct VoiceRoute {
    routing: crate::gameplay::sound::mix::VoiceRouting,
    sends: Vec<crate::gameplay::sound::auxiliary::ControlSend>,
    auxiliary_enabled: bool,
}

struct AuxiliaryConnection {
    bus: u32,
    kind: crate::gameplay::sound::auxiliary::SendKind,
    target: f32,
    gain: GainRamp,
}

#[derive(Component, Clone)]
pub(super) struct Playing {
    definition: Arc<Definition>,
    control: Arc<Control>,
    parameter: Option<(u32, f32)>,
    position: Option<super::spatial::Position>,
    emitter: Emitter,
}
impl Playing {
    /// Diagnostic levels: each sound's mixed volume (dB) and its loudest
    /// spatial channel gain, as last published by `update`.
    pub fn debug_levels(&self) -> Vec<(u32, f32, f32)> {
        self.definition.sounds.iter().zip(self.control.volumes_db.iter().zip(&self.control.spatial)).map(|(sound, (volume, spatial))| {
            let peak = spatial[..4].iter().map(|gain| f32::from_bits(gain.load(Ordering::Relaxed))).fold(0.0, f32::max);
            (sound.properties.nodes.first().map_or(0, |node| node.id), f32::from_bits(volume.load(Ordering::Relaxed)), peak)
        }).collect()
    }
    /// Diagnostic: the first sound's ancestor IDs, sound first, and its bus.
    pub fn debug_chain(&self) -> Vec<u32> {
        self.definition.sounds.first().map_or_else(Vec::new, |sound| {
            sound.properties.nodes.iter().map(|node| node.id).chain([sound.properties.bus]).collect()
        })
    }
    pub fn stop(&self, target: u32, milliseconds: i32, curve: u8) {
        // An action can target a parent outside this Play action's subtree.
        let target = if self.definition.root_properties().nodes.iter().any(|node| node.id == target) {
            self.definition.nodes[self.definition.root].id
        } else { target };
        self.control.stops.lock().unwrap_or_else(|error| error.into_inner()).push((Some(target), milliseconds, curve));
    }
    pub fn stop_all(&self, milliseconds: i32, curve: u8) {
        self.control.stops.lock().unwrap_or_else(|error| error.into_inner()).push((None, milliseconds, curve));
    }
    pub fn update(&self, mixer: &Mixer, scene: &super::spatial::Scene) -> Result<(), String> {
        let parameters = scene.parameters(self.emitter, self.parameter);
        for (node, target) in self.definition.nodes.iter().zip(&self.control.transitions) {
            if let Kind::Container(container) = &node.kind {
                target.store(mixer.transition_seconds(&container.properties, &parameters)?.to_bits(), Ordering::Relaxed);
            }
        }
        let routes = if self.definition.can_route() {
            let game_sends = scene.propagated(self.emitter).map_or(&[][..], |propagated| propagated.auxiliary.as_slice());
            Some(Arc::new(self.definition.sounds.iter().map(|sound| Ok(VoiceRoute {
                routing: mixer.voice_routing_base(&sound.properties, &parameters)?,
                sends: mixer.auxiliary_controls(&sound.properties, game_sends, &parameters)?,
                auxiliary_enabled: crate::gameplay::sound::auxiliary::enabled(&sound.properties),
            })).collect::<Result<Vec<_>, String>>()?))
        } else { None };
        for (index, ((sound, targets), rays)) in self.definition.sounds.iter().zip(&self.control.spatial).zip(&self.control.rays).enumerate() {
            let values = if let Some(spatial) = &sound.spatial {
                if let Some(propagated) = scene.propagated(self.emitter) {
                    spatial.propagated_targets(scene, propagated, sound.channels.get(), &mixer.environment)?
                } else {
                    spatial.targets(scene, self.position.or_else(|| scene.emitter(self.emitter)), sound.channels.get())?
                }
            } else if routes.is_some() {
                let gains = mixer.panner_2d(&sound.properties, &parameters)?.stereo_matrix(sound.channels.get())?;
                super::spatial::Targets { gains, auxiliary: [gains; 2], voice_filters: [0.0; 2], output_filters: [None; 2], rays: [1.0; 3] }
            } else { continue };
            for (target, value) in rays.iter().zip(values.rays) { target.store(value.to_bits(), Ordering::Relaxed); }
            let gains = values.gains;
            let filters = values.voice_filters;
            for (target, value) in targets[..6].iter().zip([gains[0], gains[1], gains[2], gains[3], filters[0], filters[1]]) {
                target.store(value.to_bits(), Ordering::Relaxed);
            }
            for (slot, (target, value)) in targets[6..8].iter().zip(values.output_filters).enumerate() {
                let base = routes.as_ref().map_or(0.0, |routes| routes[index].routing.output_filters[slot]);
                let value = value.map_or(base, |value| if value <= base { base } else { value });
                target.store(value.to_bits(), Ordering::Relaxed);
            }
            for (target, value) in targets[8..].iter().zip(values.auxiliary.into_iter().flatten()) {
                target.store(value.to_bits(), Ordering::Relaxed);
            }
        }
        for (sound, volume) in self.definition.sounds.iter().zip(&self.control.volumes_db) {
            volume.store(mixer.volume_db_with_parameters(&sound.properties, &parameters)?.to_bits(), Ordering::Relaxed);
        }
        for (sound, filters) in self.definition.sounds.iter().zip(&self.control.filters) {
            for (target, property) in filters.iter().zip([3, 4]) {
                target.store(mixer.source_property_with_parameters(&sound.properties, property, &parameters)?.to_bits(), Ordering::Relaxed);
            }
        }
        for (sound, pitch) in self.definition.sounds.iter().zip(&self.control.pitches) {
            let cents = mixer.source_property_with_parameters(&sound.properties, 2, &parameters)?;
            // Validate the complete authored random range before the worker
            // starts a voice; malformed pitch must remain a host-side error.
            let mut bounds = [cents; 2];
            for node in &sound.properties.nodes {
                if let Some(range) = node.ranges.iter().find(|range| range.property == 2) {
                    bounds[0] += range.min;
                    bounds[1] += range.max;
                }
            }
            for bound in bounds { pitch_step(bound, sound.sample_rate.get(), self.definition.rate.get())?; }
            pitch.store(cents.to_bits(), Ordering::Relaxed);
        }
        self.control.threshold_db.store(mixer.volume_threshold_db.to_bits(), Ordering::Relaxed);
        if let Some(routes) = routes { *self.control.routes.lock().unwrap_or_else(|error| error.into_inner()) = routes; }
        Ok(())
    }
}

#[derive(Asset, TypePath, Clone)]
pub(super) struct ScheduledAudio {
    definition: Arc<Definition>,
    control: Arc<Control>,
    selections: Selections,
    emitter: Emitter,
    delay_frames: u32,
    settings: crate::assets::wwise::PlaySettings,
    event: Option<crate::gameplay::sound::playing::Ticket>,
    switches: Option<SwitchBinding>,
}

#[derive(Clone)]
pub(super) struct SwitchContext { owner: Arc<Control>, node: usize }
#[derive(Default)]
pub(super) struct SwitchControls {
    pub(super) controllers: crate::gameplay::sound::switches::Switches<SwitchContext>,
    pub(super) definitions: HashMap<u32, Arc<crate::assets::wwise::SwitchPlayback>>,
}
pub(super) type SwitchRegistry = Arc<Mutex<SwitchControls>>;
#[derive(Clone)]
struct SwitchBinding { registry: SwitchRegistry, states: super::bevy_backend::StateGroups, playing_id: u32 }
impl ScheduledAudio {
    pub fn retain_event(&mut self, event: crate::gameplay::sound::playing::Ticket) { self.event = Some(event); }
    pub fn can_route(&self) -> bool { self.definition.can_route() }
    pub fn bind_switches(&mut self, registry: SwitchRegistry, states: super::bevy_backend::StateGroups, playing_id: u32) {
        self.switches = Some(SwitchBinding { registry, states, playing_id });
    }
}

enum State {
    Sound { frame: u64, random: RandomProperties, looping: Option<crate::gameplay::sound::looping::SourceLoop>,
        silence: Option<crate::gameplay::sound::silence::Voice>,
        stream: Option<Stream>,
        resampler: Option<Resampler>, gain: GainRamp, gain_tick: Option<u64>, filters: Vec<[VoiceFilter; 2]>, output_filters: Vec<[VoiceFilter; 2]>,
        routes: Option<Arc<Vec<VoiceRoute>>>, auxiliary: Vec<AuxiliaryConnection>, auxiliary_initialized: bool,
        /// Below the volume threshold with the virtual behaviour: no decoding.
        virtualized: bool,
        _activity: Option<crate::gameplay::sound::activity::Token> },
    Layer(Vec<Runtime>),
    Container { active: Vec<Runtime>, sequence: Sequence, next: Option<u64>, pending: Option<usize>, exhausted: bool },
    Done,
}
struct Runtime { node: usize, state: State, fade: Option<super::fade::Transition>,
    fade_cache: Option<(u64, super::fade::Envelope)>, failed_start: bool }

impl Runtime {
    fn duration_ms(&self, audio: &ScheduledAudio) -> Option<f32> {
        let State::Sound { random, looping, silence, .. } = &self.state else { return None; };
        let Kind::Sound(index) = audio.definition.nodes[self.node].kind else { return None; };
        let sound = &audio.definition.sounds[index];
        let duration = match silence {
            Some(silence) => silence.duration_ms(),
            None => looping.as_ref()?.duration_ms(sound.sample_rate.get()),
        };
        let cents = f32::from_bits(audio.control.pitches[index].load(Ordering::Relaxed)) + random.pitch_cents;
        Some(duration / crate::gameplay::sound::resampler::pitch_ratio(cents))
    }

    fn new(node: usize, now: u64, definition: &Definition,
        control: &Control) -> Self {
        let state = match &definition.nodes[node].kind {
            Kind::Missing | Kind::NoSource | Kind::Switch(_) => State::Done,
            // Random modifiers belong to each newly started source voice,
            // including when the same leaf overlaps itself in a sequence.
            Kind::Sound(index) => {
                control.source_ready.fetch_add(1, Ordering::Relaxed);
                let sound = &definition.sounds[*index];
                let random = RandomProperties::sample(&sound.properties, random15);
                let count = sound.loop_count.sample(random15);
                let silence = match sound.samples {
                    Samples::Silence(parameters) => Some(crate::gameplay::sound::silence::Voice::new(parameters, sound.sample_rate.get(), count, random15)),
                    Samples::Pcm(_) | Samples::Vorbis(_) | Samples::Adpcm(_) | Samples::File(_, _) | Samples::Streamed(_) => None,
                };
                let stream = Stream::new(&sound.samples, now);
                State::Sound { frame: 0, random, silence, stream,
                looping: sound.looping.map(|looping| looping.with_count(count)),
                resampler: None,
                gain: GainRamp::default(), gain_tick: None,
                routes: None, auxiliary: Vec::new(), auxiliary_initialized: false, virtualized: false,
                _activity: control.registrations[*index].as_ref().map(|registration| registration.start()),
                filters: (0..definition.sounds[*index].channels.get()).map(|_| [
                    VoiceFilter::new(FilterKind::LowPass, definition.rate.get(), 1024, false),
                    VoiceFilter::new(FilterKind::HighPass, definition.rate.get(), 1024, false),
                ]).collect(),
                output_filters: (0..definition.channels.get()).map(|_| [
                    VoiceFilter::new(FilterKind::LowPass, definition.rate.get(), 1024, false),
                    VoiceFilter::new(FilterKind::HighPass, definition.rate.get(), 1024, false),
                ]).collect() } },
            Kind::Layer(children) => State::Layer(children.iter().map(|child| Self::new(*child, now, definition, control)).collect()),
            Kind::Container(container) => State::Container { active: Vec::new(), sequence: Sequence::new(node, &container.playback),
                next: Some(now), pending: None, exhausted: false },
        };
        Self { node, state, fade: None, fade_cache: None,
            failed_start: matches!(definition.nodes[node].kind, Kind::Missing | Kind::NoSource) }
    }

    fn halt_new(&mut self) {
        match &mut self.state {
            State::Container { active, next, pending, exhausted, .. } => {
                *next = None;
                *pending = None;
                *exhausted = true;
                for child in active { child.halt_new(); }
            },
            State::Layer(children) => for child in children { child.halt_new(); },
            _ => {},
        }
    }

    /// Resolve the initial child graph when the Play action executes, before
    /// the PBI's sample offset elapses (14113b5c0 / 1410e6a70 / 14111b7b0).
    fn prime(&mut self, now: u64, audio: &ScheduledAudio) {
        self.start(now, audio);
        if let State::Layer(children) = &mut self.state {
            for child in children.iter_mut() { child.prime(now, audio); }
            children.retain(|child| !child.finished());
            self.failed_start = children.is_empty();
        }
    }

    fn finished(&self) -> bool {
        match &self.state {
            State::Done => true,
            State::Layer(children) => children.is_empty(),
            State::Container { active, next, exhausted, .. } => *exhausted && next.is_none() && active.is_empty(),
            State::Sound { .. } => false,
        }
    }

    fn continue_failed(&mut self, now: u64, audio: &ScheduledAudio) {
        let node = &audio.definition.nodes[self.node];
        let (State::Container { active, sequence, next, pending, exhausted }, Kind::Container(container)) =
            (&mut self.state, &node.kind) else { return; };
        if !active.is_empty() || next.is_some() || *exhausted { return; }
        // 1410ddd30 pops failed lookup records. Select the enclosing actor
        // now, retaining that exact choice while its delayed action is queued.
        *pending = sequence.next(audio);
        *pending = pending.filter(|&index| !matches!(audio.definition.nodes[index].kind, Kind::Missing));
        if pending.is_none() {
            *exhausted = true;
            self.failed_start = true;
            return;
        }
        let seconds = f32::from_bits(audio.control.transitions[self.node].load(Ordering::Relaxed));
        let milliseconds = transition_ms(&container.playback, seconds, random15);
        *next = Some(crate::gameplay::sound::containers::timing::after_failed_start(
            now, container.playback.transition, milliseconds, audio.definition.rate.get()));
        self.failed_start = false;
    }

    fn start(&mut self, now: u64, audio: &ScheduledAudio) {
        let definition = &audio.definition;
        let node = &definition.nodes[self.node];
        let (State::Container { active, sequence, next, pending, exhausted }, Kind::Container(container)) =
            (&mut self.state, &node.kind) else { return; };
        if !next.is_some_and(|deadline| now >= deadline) || *exhausted { return; }
        let first = !sequence.initialized;
        let selected = pending.take().or_else(|| sequence.next(audio));
        *next = None;
        if let Some(index) = selected {
            if matches!(definition.nodes[index].kind, Kind::Missing) {
                *exhausted = true;
                self.failed_start = true;
                return;
            }
            let mut child = Self::new(index, now, definition, &audio.control);
            child.prime(now, audio);
            if !container.playback.continuous { *exhausted = true; }
            if child.finished() && child.failed_start {
                self.failed_start = true;
                self.continue_failed(now, audio);
                return;
            }
            self.failed_start = false;
            active.push(child);
            if container.playback.continuous && container.playback.transition == TransitionMode::TriggerRate {
                let seconds = f32::from_bits(audio.control.transitions[self.node].load(Ordering::Relaxed));
                let milliseconds = transition_ms(&container.playback, seconds, random15);
                *next = Some(now + frames(milliseconds, definition.rate).max(1024));
            }
        } else { *exhausted = true; self.failed_start = first || self.failed_start; }
    }

    fn stop(&mut self, target: u32, milliseconds: i32, curve: u8, now: u64, definition: &Definition) {
        if definition.nodes[self.node].id == target {
            self.failed_start = false;
            self.fade_cache = None;
            if milliseconds == 0 {
                if let Some(fade) = self.fade.as_ref().filter(|fade| fade.active(now)) { fade.stop(now, 0, curve); }
                self.state = State::Done;
            }
            else {
                let duration = super::fade::frames(milliseconds, definition.rate.get());
                if let Some(fade) = self.fade.as_ref().filter(|fade| fade.active(now)) { fade.stop(now, duration, curve); }
                else {
                    let gain = self.fade.as_ref().map_or(1.0, |fade| fade.value(now).unwrap_or(0.0));
                    self.fade = Some(super::fade::Transition::new(super::fade::Envelope::new(now, duration, gain, curve)));
                }
                self.halt_new();
            }
            return;
        }
        match &mut self.state {
            State::Layer(children) | State::Container { active: children, .. } => {
                for child in children { child.stop(target, milliseconds, curve, now, definition); }
            },
            _ => {},
        }
    }

    fn fade_envelope(&mut self, now: u64) -> Option<super::fade::Envelope> {
        let fade = self.fade.as_ref()?;
        let tick = now / 1024;
        if self.fade_cache.is_none_or(|(cached, _)| cached != tick) { self.fade_cache = Some((tick, fade.snapshot())); }
        self.fade_cache.map(|(_, envelope)| envelope)
    }

    fn fade_value(&mut self, now: u64) -> Option<f32> {
        let Some(envelope) = self.fade_envelope(now) else { return Some(1.0); };
        if !matches!(self.state, State::Sound { .. }) { return envelope.value(now); }
        // 1410e6800 advances transitions to tick+1 before rendering. The
        // completed stop still renders its final zero-target block; 1410ebae0
        // retires the PBI afterward. The mixer ramps the gain product.
        envelope.block_value(now)
    }

    fn stop_buffer(&mut self, now: u64) {
        // 1411068e0 forces an initialized source to zero (141105410), or
        // finishes its retained gain transition with the zero-ms curve 4.
        if let Some(fade) = self.fade.as_ref().filter(|fade| fade.active(now)) { fade.stop(now, 0, 4); }
        else { self.fade = Some(super::fade::Transition::new(super::fade::Envelope::new(now, 0, 0.0, 4))); }
        self.fade_cache = None;
    }

    fn fade_stopping(&mut self, now: u64) -> bool {
        self.fade_envelope(now).is_some_and(|envelope| envelope.value(now / 1024 * 1024 + 1024).is_none())
    }

    fn frame(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32], mut routing: Option<&mut RouteContext<'_>>) -> bool {
        let Some(gain) = self.fade_value(now) else { return false; };
        if matches!(self.state, State::Sound { .. }) {
            return self.frame_inner(now, audio, output, routing, gain);
        }
        let mut local = [0.0; 8];
        let alive = if let Some(routing) = routing.as_mut() {
            let previous = routing.gain;
            routing.gain *= gain;
            let alive = self.frame_inner(now, audio, &mut local[..output.len()], Some(&mut **routing), 1.0);
            routing.gain = previous;
            alive
        } else { self.frame_inner(now, audio, &mut local[..output.len()], None, 1.0) };
        for (out, sample) in output.iter_mut().zip(local) { *out += sample * gain; }
        alive
    }

    fn frame_inner(&mut self, now: u64, audio: &ScheduledAudio, output: &mut [f32], mut routing: Option<&mut RouteContext<'_>>, source_gain: f32) -> bool {
        self.start(now, audio);
        let definition = &audio.definition;
        let node = &definition.nodes[self.node];
        match (&mut self.state, &node.kind) {
            (State::Sound { frame, random, looping, silence, stream, resampler, gain: ramp, gain_tick, filters, output_filters,
                routes, auxiliary, auxiliary_initialized, virtualized, _activity: _ }, Kind::Sound(index)) => {
                let sound = &definition.sounds[*index];
                if *gain_tick == Some(now / 1024) && stream.as_ref().is_some_and(|stream| stream.awaiting_read(now)) {
                    return true;
                }
                if *frame % 1024 == 0 {
                    let cents = f32::from_bits(audio.control.pitches[*index].load(Ordering::Relaxed)) + random.pitch_cents;
                    let Ok(step) = pitch_step(cents, sound.sample_rate.get(), definition.rate.get()) else { return false };
                    match resampler {
                        Some(resampler) => resampler.set_step(step, true),
                        None => *resampler = Some(Resampler::new(step)),
                    }
                }
                let Some(resampler) = resampler else { return false };
                let channels = usize::from(sound.channels.get());
                let mut audible = None;
                let update_gain = *gain_tick != Some(now / 1024);
                if routing.is_some() && update_gain {
                    *routes = Some(audio.control.routes.lock().unwrap_or_else(|error| error.into_inner()).clone());
                }
                let route = if routing.is_some() { routes.as_ref().and_then(|routes| routes.get(*index)) } else { None };
                if update_gain {
                    // The voice gain sets this block's ramp targets only, so it
                    // is evaluated once per block rather than per frame.
                    let volume = f32::from_bits(audio.control.volumes_db[*index].load(Ordering::Relaxed))
                        + random.volume_db + random.makeup_db;
                    let threshold = f32::from_bits(audio.control.threshold_db.load(Ordering::Relaxed));
                    let gain = if volume <= threshold { 0.0 } else {
                        power10(route.map_or(volume, |route| route.routing.volume_db + random.volume_db
                            + audio.control.activity.as_ref().map_or(0.0, |activity| activity.voice_ducking(sound.properties.bus))) * 0.05)
                    };
                    *gain_tick = Some(now / 1024);
                    // 1410fc5c0 includes the transition amplitude in voice
                    // gain before 14113ef50 interpolates its speaker product.
                    let gain = gain * source_gain;
                    let dry = gain * route.map_or(1.0, |route| power10((route.routing.dry_db
                        + audio.control.activity.as_ref().map_or(0.0, |activity| activity.output_ducking(sound.properties.bus, true))) * 0.05));
                    let mut target = [dry; 8];
                    if sound.spatial.is_some() || route.is_some() {
                        for (channel, target) in target[..channels * output.len()].iter_mut().enumerate() {
                            *target *= f32::from_bits(audio.control.spatial[*index][channel].load(Ordering::Relaxed));
                        }
                    }
                    ramp.begin(target, 1024);
                    if let Some(route) = route {
                        use crate::gameplay::sound::auxiliary::{bus_sends, SendKind};
                        let sends = bus_sends(&route.sends);
                        // 1410ec0e0 retains a connection's kind if the bus is
                        // reintroduced before 1410fc0f0 removes it, including
                        // after its previous target reached zero.
                        auxiliary.retain(|connection| connection.target > 0.0 || sends.iter().any(|send| send.bus == connection.bus));
                        for connection in auxiliary.iter_mut() { connection.target = 0.0; }
                        for send in sends {
                            if let Some(connection) = auxiliary.iter_mut().find(|connection| connection.bus == send.bus) {
                                connection.target = send.gain;
                            } else {
                                let mut ramp = GainRamp::default();
                                if *auxiliary_initialized { ramp.begin([0.0; 8], 1024); }
                                auxiliary.push(AuxiliaryConnection { bus: send.bus, kind: send.kind, target: send.gain, gain: ramp });
                            }
                        }
                        for connection in auxiliary.iter_mut() {
                            let offset = if connection.kind == SendKind::Game { 8 } else { 12 };
                            let mut targets = [0.0; 8];
                            for (coefficient, target) in targets[..channels * 2].iter_mut().enumerate() {
                                *target = gain * connection.target * f32::from_bits(audio.control.spatial[*index][offset + coefficient].load(Ordering::Relaxed));
                            }
                            connection.gain.begin(targets, 1024);
                        }
                        if route.auxiliary_enabled { *auxiliary_initialized = true; }
                    }
                    // DH2_DEBUG_VIRTUAL_VOICES=0 keeps every voice decoding (measurement).
                    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    let enabled = *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_VIRTUAL_VOICES").as_deref() != Ok("0"));
                    if let (true, Some(context), Some(route)) = (enabled, routing.as_deref(), route) {
                        // 1410fc0f0 gives each connection the voice volume, its
                        // loudest ray of that connection's kind and its mixing
                        // bus volume; 1410fcf50 finds the voice inaudible when
                        // every connection is at or below 1410ca700's linear
                        // threshold. No installed bus enables HDR and no sound
                        // has modulators or feedback (wwise-audit
                        // --virtual-voices), so their factors are 1.
                        use crate::gameplay::sound::auxiliary::SendKind;
                        let threshold = power10(threshold * 0.05).max(10f32.powf(threshold * 0.05));
                        let rays: [f32; 3] = std::array::from_fn(|kind| f32::from_bits(audio.control.rays[*index][kind].load(Ordering::Relaxed)));
                        let voice = context.gain;
                        audible = Some(dry * voice * rays[0] * context.buses.output_gain(route.routing.bus) > threshold
                            || auxiliary.iter().any(|connection| gain * connection.target * voice
                                * rays[if connection.kind == SendKind::Game { 1 } else { 2 }]
                                * context.buses.output_gain(connection.bus) > threshold));
                    }
                    // Source offsets occupy the beginning of the output
                    // block. Start its matrix ramp at that block position.
                    for _ in 0..now % 1024 {
                        ramp.next_frame();
                        for connection in auxiliary.iter_mut() { connection.gain.next_frame(); }
                    }
                }
                // 1410fdda0's below-threshold behaviour (1410dd720 resolves it).
                use crate::assets::wwise::{BelowThreshold, VirtualReturn};
                let (behaviour, queue) = sound.threshold;
                static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                if audible.is_some_and(|audible| audible == *virtualized || behaviour == BelowThreshold::Kill && !audible)
                    && *TRACE.get_or_init(|| std::env::var("DH2_TRACE_VIRTUAL_VOICES").as_deref() == Ok("1"))
                {
                    eprintln!("virtual voice {:#010x}: audible={audible:?} behaviour={behaviour:?} return={queue:?} frame={frame}", node.id);
                }
                match audible {
                    Some(true) if *virtualized => {
                        *virtualized = false;
                        // 1411609c0 restarts a from-beginning source; a voice
                        // from elapsed time continues where its skips led.
                        if queue == VirtualReturn::FromBeginning { resampler.restart(); }
                    }
                    Some(false) if behaviour == BelowThreshold::Kill => return false,
                    Some(false) if behaviour == BelowThreshold::Virtual && !*virtualized => {
                        *virtualized = true;
                        // 141161c80 frees the decoder unless the voice resumes.
                        if queue != VirtualReturn::Resume { *stream = Stream::new(&sound.samples, now); }
                    }
                    _ => {}
                }
                if *virtualized {
                    if update_gain && queue == VirtualReturn::FromElapsedTime {
                        resampler.skip(resampler.buffer_frames(1024));
                        // The time skip stops a source that runs out of data.
                        let last = resampler.position().saturating_sub(1);
                        let ended = match looping {
                            Some(looping) => looping.frame(last).is_none(),
                            None => silence.as_ref().is_some_and(|silence| silence.sample(last).is_none()),
                        };
                        if ended { return false; }
                    }
                    *frame += 1;
                    return true;
                }
                // File-source readiness is not source exhaustion. Original
                // 141161770/141161240 defer without advancing the source clock.
                if stream.as_mut().is_some_and(|stream| !stream.ready(now)) { return true; }
                let mut samples = [0.0; 8];
                for (channel, sample) in samples[..channels].iter_mut().enumerate() {
                    let Some(value) = sound.sample(resampler, channel, *looping, silence.as_ref(), stream.as_mut()) else {
                        return stream.as_ref().is_some_and(Stream::waiting);
                    };
                    *sample = value;
                }
                let gains = ramp.next_frame();
                let low_pass = f32::from_bits(audio.control.filters[*index][0].load(Ordering::Relaxed)) + random.low_pass;
                let high_pass = f32::from_bits(audio.control.filters[*index][1].load(Ordering::Relaxed)) + random.high_pass;
                let distance_low = f32::from_bits(audio.control.spatial[*index][4].load(Ordering::Relaxed));
                let distance_high = f32::from_bits(audio.control.spatial[*index][5].load(Ordering::Relaxed));
                if let Some(route) = route {
                    // 1410fcd00 supplies source makeup/normalization gain
                    // independently from the connection's voice-volume ramp.
                    let makeup = power10((route.routing.makeup_db + random.makeup_db) * 0.05);
                    for sample in &mut samples[..channels] { *sample *= makeup; }
                }
                for (sample, filters) in samples[..channels].iter_mut().zip(filters) {
                    if *frame % 1024 == 0 {
                        filters[0].set_target(low_pass.max(distance_low));
                        filters[1].set_target(high_pass.max(distance_high));
                    }
                    *sample = filters[0].process(*sample);
                    *sample = filters[1].process(*sample);
                }
                if let (Some(context), Some(route)) = (routing.as_deref_mut(), route) {
                    let mut dry = stereo_frame(&samples[..channels], gains);
                    for (channel, sample) in dry.iter_mut().enumerate() {
                        let filters = &mut output_filters[channel];
                        if *frame % 1024 == 0 {
                            filters[0].set_target(f32::from_bits(audio.control.spatial[*index][6].load(Ordering::Relaxed)));
                            filters[1].set_target(f32::from_bits(audio.control.spatial[*index][7].load(Ordering::Relaxed)));
                        }
                        let low = filters[0].process(*sample);
                        *sample = filters[1].process(low) * context.gain;
                    }
                    context.add(route.routing.bus, dry);
                    for connection in auxiliary.iter_mut() {
                        let wet = stereo_frame(&samples[..channels], connection.gain.next_frame()).map(|sample| sample * context.gain);
                        context.add(connection.bus, wet);
                    }
                    resampler.advance();
                    *frame += 1;
                    return (sound.sample(resampler, 0, *looping, silence.as_ref(), stream.as_mut()).is_some()
                    || stream.as_ref().is_some_and(Stream::waiting));
                }
                if sound.spatial.is_some() {
                    let output_channels = output.len();
                    for (channel, out) in output.iter_mut().enumerate() {
                        let mut mixed = 0.0;
                        for (input, sample) in samples[..channels].iter().enumerate() {
                            mixed += sample * gains[input * output_channels + channel];
                        }
                        let filters = &mut output_filters[channel];
                        if *frame % 1024 == 0 {
                            filters[0].set_target(f32::from_bits(audio.control.spatial[*index][6].load(Ordering::Relaxed)));
                            filters[1].set_target(f32::from_bits(audio.control.spatial[*index][7].load(Ordering::Relaxed)));
                        }
                        let mixed = filters[0].process(mixed);
                        *out += filters[1].process(mixed);
                    }
                } else {
                    for ((out, sample), gain) in output.iter_mut().zip(samples).zip(gains) { *out += sample * gain; }
                }
                resampler.advance();
                *frame += 1;
                (sound.sample(resampler, 0, *looping, silence.as_ref(), stream.as_mut()).is_some()
                    || stream.as_ref().is_some_and(Stream::waiting))
            },
            (State::Layer(children), _) => {
                let mut failed = false;
                let mut ended = false;
                children.retain_mut(|child| {
                    let alive = child.frame(now, audio, output, routing.as_deref_mut());
                    if !alive { if child.failed_start { failed = true; } else { ended = true; } }
                    alive
                });
                self.failed_start = children.is_empty() && failed && !ended;
                !children.is_empty()
            },
            (State::Container { active, next, exhausted, .. }, Kind::Container(container)) => {
                let mut failed = false;
                let mut ended = false;
                active.retain_mut(|child| {
                    let alive = child.frame(now, audio, output, routing.as_deref_mut());
                    if !alive { if child.failed_start { failed = true; } else { ended = true; } }
                    alive
                });
                if active.is_empty() && failed && !ended {
                    self.failed_start = true;
                    if !*exhausted && next.is_none() {
                        self.continue_failed(now, audio);
                        return !self.finished();
                    }
                }
                if active.is_empty() && next.is_none() && !*exhausted {
                    let delay = if container.playback.transition == TransitionMode::Delay {
                        let seconds = f32::from_bits(audio.control.transitions[self.node].load(Ordering::Relaxed));
                        frames(transition_ms(&container.playback, seconds, random15), definition.rate)
                    } else { 0 };
                    *next = Some(crate::gameplay::sound::containers::timing::after_source_end(
                        now, container.playback.transition, delay as u32));
                }
                !*exhausted || !active.is_empty()
            },
            _ => false,
        }
    }
}

fn stereo_frame(input: &[f32], matrix: [f32; 8]) -> [f32; 2] {
    let mut output = [0.0; 2];
    for (input_channel, sample) in input.iter().enumerate() {
        for (output_channel, out) in output.iter_mut().enumerate() { *out += sample * matrix[input_channel * 2 + output_channel]; }
    }
    output
}

struct RouteContext<'a> {
    buses: &'a mut super::bevy_backend::buses::Buses,
    offset: usize,
    gain: f32,
    error: Option<String>,
}
impl RouteContext<'_> {
    fn add(&mut self, bus: u32, samples: [f32; 2]) {
        if self.error.is_none() {
            if let Err(error) = self.buses.add(bus, self.offset, samples) { self.error = Some(error); }
        }
    }
}

/// Pending actions and their sample offset on the output clock.
struct Playback {
    audio: ScheduledAudio,
    root: Option<Runtime>,
    flow: Option<flow::Flow>,
    prepared: bool,
    external_delay: bool,
    frame: u64,
    stopped: bool,
}
impl Playback {
    fn new(audio: ScheduledAudio) -> Self {
        // Sample-accurate VPL handoff retains its existing path while its
        // decoder-buffer transfer is integrated into source continuations.
        let flow = (!audio.definition.sample_accurate()).then(flow::Flow::default);
        let mut playback = Self { audio, root: None, flow, prepared: false, external_delay: false, frame: 0, stopped: false };
        playback.prepare();
        playback
    }

    fn prepare(&mut self) {
        if self.stopped || self.prepared || self.external_delay { return; }
        let deadline = u64::from(self.audio.delay_frames);
        // 1410e5600 executes the action in its quotient quantum and passes
        // the remainder to the PBI as a sample offset. Create the voice in
        // that quantum, preserving its bus activity before samples start.
        if deadline.saturating_sub(self.frame) >= 1024 { return; }
        if !self.audio.settings.admits(random15) { self.stopped = true; return; }
        let milliseconds = self.audio.settings.transition.sample_ms(random15);
        self.prepared = true;
        if let Some(flow) = &mut self.flow {
            let fade = (milliseconds != 0).then_some(flow::Fade { start: self.frame, milliseconds,
                curve: self.audio.settings.transition.curve });
            flow.play(self.audio.definition.root, deadline, self.frame, &self.audio, None, fade, None, false);
            return;
        }
        let mut root = Runtime::new(self.audio.definition.root, deadline, &self.audio.definition, &self.audio.control);
        root.prime(deadline, &self.audio);
        if milliseconds != 0 {
            root.fade = Some(super::fade::Transition::new(super::fade::Envelope::fade_in(self.frame,
                super::fade::frames(milliseconds, self.audio.definition.rate.get()), self.audio.settings.transition.curve)));
        }
        self.root = Some(root);
    }

    fn flush_stops(&mut self) {
        for (target, milliseconds, curve) in self.audio.control.stops.lock().unwrap_or_else(|error| error.into_inner()).drain(..) {
            if let Some(flow) = &mut self.flow {
                if self.prepared {
                    if let Some(target) = target { flow.stop(target, milliseconds, curve, self.frame, &self.audio, false); }
                    else { flow.stop_all(milliseconds, curve, self.frame, &self.audio, false); }
                }
                else if target.is_none_or(|target| self.audio.definition.matches_actor(self.audio.definition.root, target)) { self.stopped = true; }
            }
            else if let Some(root) = &mut self.root {
                root.stop(target.unwrap_or(self.audio.definition.nodes[self.audio.definition.root].id), milliseconds, curve, self.frame, &self.audio.definition);
            }
            else if target.is_none_or(|target| self.audio.definition.matches_actor(self.audio.definition.root, target)) { self.stopped = true; }
        }
    }

    fn begin(&mut self, paused: bool) {
        self.flush_stops();
        if paused { return; }
        self.prepare();
        if let Some(flow) = &mut self.flow { flow.begin(self.frame, &self.audio); }
        self.publish_source_ready();
    }

    fn publish_source_ready(&self) {
        // 1410fdd60 ->1411287f0: first actual source-processing pass,
        // separately from command acceptance or predecoding its media.
        let count = self.audio.control.source_ready.swap(0, Ordering::Relaxed);
        if let Some(event) = &self.audio.event {
            for _ in 0..count { event.source_ready(); }
        }
    }

    fn frame(&mut self, output: &mut [f32], routing: Option<&mut RouteContext<'_>>) -> bool {
        if self.stopped { return false; }
        let now = self.frame;
        self.frame += 1;
        if now < u64::from(self.audio.delay_frames) { return true; }
        let alive = if let Some(flow) = &mut self.flow { flow.frame(now, &self.audio, output, routing) }
        else { self.root.as_mut().is_some_and(|root| root.frame(now, &self.audio, output, routing)) };
        self.publish_source_ready();
        alive
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        if let Some(binding) = &self.audio.switches {
            let removed = binding.registry.lock().unwrap_or_else(|error| error.into_inner()).controllers
                .remove_where(|controller| Arc::ptr_eq(&controller.context.owner, &self.audio.control));
            self.audio.control.switch_count.fetch_sub(removed as u32, Ordering::Relaxed);
        }
    }
}

/// Scheduled mono/stereo sources on the shared output clock. Multichannel
/// graphs still require their separate conversion and speaker-layout paths.
pub(super) struct Routed {
    playback: Playback,
}
impl Routed {
    pub fn new(audio: ScheduledAudio) -> Result<Self, String> {
        if !audio.can_route() { return Err("shared audio graph requires its multichannel format conversion path".into()); }
        if audio.definition.has_switch() && audio.switches.is_none() { return Err("switch playback has no shared controller registry".into()); }
        if audio.control.routes.lock().unwrap_or_else(|error| error.into_inner()).len() != audio.definition.sounds.len() {
            return Err("shared audio graph has no validated voice routes".into());
        }
        let mut playback = Playback::new(audio);
        playback.external_delay = playback.audio.delay_frames >= 1024;
        Ok(Self { playback })
    }

    pub fn begin(&mut self, paused: bool) {
        self.playback.begin(paused);
    }

    pub fn delay_quanta(&self) -> u64 { u64::from(self.playback.audio.delay_frames) / 1024 }
    pub fn release_delayed(&mut self) {
        self.playback.external_delay = false;
        self.playback.prepare();
    }
    pub fn cancelled(&self) -> bool { self.playback.stopped }

    pub fn emitter(&self) -> Emitter { self.playback.audio.emitter }
    pub fn owns(&self, context: &SwitchContext) -> bool { Arc::ptr_eq(&self.playback.audio.control, &context.owner) }
    pub fn flush_stops(&mut self) { self.playback.flush_stops(); }
    pub fn pause_controllers(&mut self, paused: bool) {
        let audio = &self.playback.audio;
        if let Some(binding) = &audio.switches {
            binding.registry.lock().unwrap_or_else(|error| error.into_inner()).controllers
                .pause_where(|controller| Arc::ptr_eq(&controller.context.owner, &audio.control), paused);
        }
    }
    pub fn cancel_pending(&mut self, target: u32) {
        let playback = &mut self.playback;
        if !playback.prepared {
            if playback.audio.definition.matches_actor(playback.audio.definition.root, target) { playback.stopped = true; }
        } else if let Some(flow) = &mut playback.flow { flow.cancel_pending(target, &playback.audio); }
    }
    pub fn stop_actor(&mut self, target: u32, milliseconds: i32, curve: u8, paused: bool) {
        let playback = &mut self.playback;
        if !playback.prepared {
            if playback.audio.definition.matches_actor(playback.audio.definition.root, target) { playback.stopped = true; }
        } else if let Some(flow) = &mut playback.flow {
            flow.stop(target, milliseconds, curve, playback.frame, &playback.audio, paused);
            if flow.finished(&playback.audio) { playback.stopped = true; }
        } else if let Some(root) = &mut playback.root {
            root.stop(target, milliseconds, curve, playback.frame, &playback.audio.definition);
        }
    }
    pub fn stop_all(&mut self, milliseconds: i32, curve: u8, paused: bool) {
        let playback = &mut self.playback;
        if !playback.prepared { playback.stopped = true; }
        else if let Some(flow) = &mut playback.flow {
            flow.stop_all(milliseconds, curve, playback.frame, &playback.audio, paused);
            if flow.finished(&playback.audio) { playback.stopped = true; }
        } else if let Some(root) = &mut playback.root {
            let id = playback.audio.definition.nodes[root.node].id;
            root.stop(id, milliseconds, curve, playback.frame, &playback.audio.definition);
        }
    }
    pub fn play_switch(&mut self, context: &SwitchContext, child: u32, milliseconds: i32, curve: u8) {
        let playback = &mut self.playback;
        let Kind::Switch(switch) = &playback.audio.definition.nodes[context.node].kind else { return; };
        let Some(node) = switch.children.get(&child).copied().flatten() else { return; };
        if let Some(flow) = &mut playback.flow {
            let fade = (milliseconds != 0).then_some(flow::Fade { start: playback.frame, milliseconds, curve });
            flow.play(node, playback.frame, playback.frame, &playback.audio, None, fade, None, false);
        }
    }

    pub fn frame(&mut self, buses: &mut super::bevy_backend::buses::Buses, offset: usize) -> Result<bool, String> {
        let mut context = RouteContext { buses, offset, gain: 1.0, error: None };
        let alive = self.playback.frame(&mut [0.0; 2], Some(&mut context));
        if let Some(error) = context.error { return Err(error); }
        Ok(alive)
    }
}

fn frames(milliseconds: f32, rate: SampleRate) -> u64 {
    u64::from(crate::gameplay::sound::containers::timing::samples_from_ms(f64::from(milliseconds), rate.get()))
}

pub(super) struct Decoder {
    playback: Playback,
    output: [f32; 8],
    channel: usize,
    alive: bool,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let channels = usize::from(self.playback.audio.definition.channels.get());
        if self.channel == channels {
            if !self.alive { return None; }
            self.output.fill(0.0);
            if self.playback.frame % 1024 == 0 { self.playback.begin(false); }
            self.alive = self.playback.frame(&mut self.output[..channels], None);
            self.channel = 0;
        }
        let sample = self.output[self.channel];
        self.channel += 1;
        Some(sample)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> ChannelCount { self.playback.audio.definition.channels }
    fn sample_rate(&self) -> SampleRate { self.playback.audio.definition.rate }
    fn total_duration(&self) -> Option<Duration> { None }
}
impl Decodable for ScheduledAudio {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder { playback: Playback::new(self.clone()),
            output: [0.0; 8], channel: usize::from(self.definition.channels.get()), alive: true }
    }
}
