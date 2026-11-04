//! Converted Wwise Vorbis media: a packet index for decoding from any frame.
//! Wwise positions its Vorbis sources with the media's seek table
//! (141160ba0, used by the virtual-voice return 1411609c0). The converted
//! Ogg stream's granules do not count frames across block-size changes, so
//! each packet's frame position comes from the decoder's own windowing.
use lewton::{audio::{get_decoded_sample_count, read_audio_packet_generic, read_wwise_audio_packet_generic,
    wwise_decoded_sample_count, wwise_packet_mode, PreviousWindowRight},
    header::{read_header_ident, read_header_setup, IdentHeader, SetupHeader}, samples::InterleavedSamples};
use std::{io, sync::Arc};

pub struct PacketIndex {
    ident: Arc<IdentHeader>,
    /// Shared by every stream with the same setup packet and block sizes.
    setup: Arc<SetupHeader>,
    /// Audio packets: a byte range of the Ogg stream, or of `joined` for a
    /// packet continued across pages.
    packets: Vec<(u32, u32, bool)>,
    joined: Vec<u8>,
    /// Frame at which each packet's decoded output begins; one more entry
    /// holds the stream's decoded length.
    starts: Vec<u64>,
    /// Installed Wwise packets decoded in place: each packet's long-block
    /// flag, which gives its neighbours' window flags (None: Ogg packets).
    wwise_long: Option<Vec<bool>>,
}

/// Installed Wwise Vorbis layout, as ww2ogg's `parse_vorb_chunk` reads it,
/// for the variants with block sizes in the header (vorb 0x2A/0x32/0x34).
pub struct WwiseLayout {
    data: std::ops::Range<usize>,
    setup: usize,
    first_audio: usize,
    channels: u8,
    sample_rate: u32,
    blocksizes: (u8, u8),
    mod_packets: bool,
    granule_headers: bool,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> { Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?)) }
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> { Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?)) }

/// The layout of a Wwise Vorbis RIFF, or None for a variant decoded through
/// the Ogg conversion (old packet headers, or no block sizes in `vorb`).
pub fn wwise_layout(bytes: &[u8]) -> Option<WwiseLayout> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") { return None; }
    let (mut fmt, mut vorb, mut data) = (None, None, None);
    let mut cursor = 12usize;
    while cursor + 8 <= bytes.len() {
        let size = u32_at(bytes, cursor + 4)? as usize;
        let range = cursor + 8..(cursor + 8).checked_add(size)?;
        match &bytes[cursor..cursor + 4] {
            b"fmt " => fmt = Some(range.clone()),
            b"vorb" => vorb = Some(range.clone()),
            b"data" => data = Some(range.clone()),
            _ => {}
        }
        cursor = range.end + (size & 1);
    }
    let (fmt, data) = (fmt?, data?);
    if u16_at(bytes, fmt.start)? != 0xffff || data.end > bytes.len() { return None; }
    let channels = u8::try_from(u16_at(bytes, fmt.start + 2)?).ok()?;
    // Without a vorb chunk its fields follow the 0x18-byte fmt header.
    let vorb = vorb.or_else(|| (fmt.len() == 0x42).then(|| fmt.start + 0x18..fmt.end))?;
    let at = |offset: usize| vorb.start + offset;
    let (mod_packets, granule_headers, offsets, blocksizes) = match vorb.len() {
        0x2A => {
            let signal = u32_at(bytes, at(0x4))?;
            (!matches!(signal, 0x4A | 0x4B | 0x69 | 0x70), false, at(0x10), at(0x28))
        }
        0x32 | 0x34 => (false, true, at(0x18), at(0x30)),
        _ => return None,
    };
    Some(WwiseLayout {
        setup: u32_at(bytes, offsets)? as usize,
        first_audio: u32_at(bytes, offsets + 4)? as usize,
        data,
        channels,
        sample_rate: u32_at(bytes, fmt.start + 4)?,
        blocksizes: (*bytes.get(blocksizes)?, *bytes.get(blocksizes + 1)?),
        mod_packets,
        granule_headers,
    })
}

/// Decoder headers by Wwise setup packet: rebuilt once from the codebook
/// library by converting only the media's header (no audio packets).
type HeaderKey = (SetupKey, u32);
static WWISE_HEADERS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<HeaderKey, (Arc<IdentHeader>, Arc<SetupHeader>)>>> =
    std::sync::LazyLock::new(Default::default);

fn wwise_headers(bytes: &[u8], layout: &WwiseLayout) -> io::Result<(Arc<IdentHeader>, Arc<SetupHeader>)> {
    let data = &bytes[layout.data.clone()];
    let setup = data.get(layout.setup..layout.first_audio).ok_or_else(|| invalid("Wwise setup outside data"))?;
    // Codebooks can share across sample rates; the identification header
    // cannot. 1411606d0 reads each source's fmt sample rate independently.
    let key = ((setup.to_vec(), layout.channels, layout.blocksizes), layout.sample_rate);
    if let Some(headers) = WWISE_HEADERS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(headers.clone());
    }
    let mut header = bytes[..layout.data.start + layout.first_audio].to_vec();
    header[layout.data.start - 4..layout.data.start].copy_from_slice(&(layout.first_audio as u32).to_le_bytes());
    let riff = (header.len() - 8) as u32;
    header[4..8].copy_from_slice(&riff.to_le_bytes());
    let index = PacketIndex::new(&super::decode_wem(&header)?)?;
    let headers = (index.ident, index.setup);
    Ok(WWISE_HEADERS.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(headers).clone())
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.to_owned()) }

/// Decoded setups by (setup packet, channels, block sizes). Wwise encodes
/// from a shared codebook library, so many streams carry the same setup;
/// its decoded codebooks are large and identical, so each is kept once.
type SetupKey = (Vec<u8>, u8, (u8, u8));
static SETUPS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<SetupKey, Arc<SetupHeader>>>> =
    std::sync::LazyLock::new(Default::default);

/// Bank/source release (141122400/1410f1910) also ends ownership of decoder
/// definitions. Host sharing must not make unused codebooks process resident.
/// Run after outgoing voices and prepared media have released their references.
pub(crate) fn release_unused_headers() {
    let retain = std::env::var("DH2_DEBUG_RETAIN_SOUND_HEADERS").as_deref() == Ok("1");
    let mut headers = WWISE_HEADERS.lock().unwrap_or_else(|e| e.into_inner());
    let before_headers = headers.len();
    if !retain {
        // Ident is private to this header identity. A live PacketIndex holds
        // it too; setup alone also has the SETUPS cache reference below.
        headers.retain(|_, (ident, _)| Arc::strong_count(ident) > 1);
    }
    let mut setups = SETUPS.lock().unwrap_or_else(|e| e.into_inner());
    let before_setups = setups.len();
    if !retain { setups.retain(|_, setup| Arc::strong_count(setup) > 1); }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY sound header release: retain={retain} headers={before_headers}->{} setups={before_setups}->{}", headers.len(), setups.len());
    }
}

fn shared_setup(packet: &[u8], channels: u8, blocksizes: (u8, u8)) -> io::Result<Arc<SetupHeader>> {
    let key = (packet.to_vec(), channels, blocksizes);
    if let Some(setup) = SETUPS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(setup.clone());
    }
    let setup = Arc::new(read_header_setup(packet, channels, blocksizes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?);
    Ok(SETUPS.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_insert(setup).clone())
}

impl PacketIndex {
    /// DH2: index installed Wwise Vorbis packets in place; `bytes` is the
    /// whole media file the packet ranges point into. Frame positions follow
    /// the decoder's windowing as for converted media, and the last packet is
    /// trimmed to the header's sample count as ww2ogg's final granule does.
    pub fn wwise(bytes: &[u8], layout: &WwiseLayout, sample_count: u64) -> io::Result<Self> {
        let (ident, setup) = wwise_headers(bytes, layout)?;
        let header = if layout.granule_headers { 6 } else { 2 };
        let mut raw = Vec::new();
        let mut offset = layout.data.start + layout.first_audio;
        while offset < layout.data.end {
            let size = u16_at(bytes, offset).ok_or_else(|| invalid("Wwise packet header truncated"))? as usize;
            let start = offset + header;
            let end = start + size;
            if end > layout.data.end { return Err(invalid("Wwise packet exceeds its data")); }
            let packet = &bytes[start..end];
            let long = if size == 0 || !layout.mod_packets { false } else {
                wwise_packet_mode(&setup, packet).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?.1
            };
            raw.push(((start as u32, size as u32, false), long));
            offset = end;
        }
        let blocksize = |long: bool| 1u64 << if long { ident.blocksize_1 } else { ident.blocksize_0 };
        let mut starts = Vec::with_capacity(raw.len() + 1);
        let mut frame = 0u64;
        // ww2ogg's page granules: blocks of the current and previous packet.
        let mut granule = 0u64;
        let mut previous_granule = None;
        for (number, &((start, size, _), long)) in raw.iter().enumerate() {
            let previous_long = number.checked_sub(1).is_some_and(|i| raw[i].1);
            let next_long = raw.get(number + 1).is_some_and(|next| next.1);
            let mut decoded = if number == 0 { 0 } else if layout.mod_packets {
                wwise_decoded_sample_count(&ident, long, previous_long, next_long) as u64
            } else {
                get_decoded_sample_count(&ident, &setup, &bytes[start as usize..(start + size) as usize]).unwrap_or(0) as u64
            };
            if number > 0 { granule += (blocksize(previous_long) + blocksize(long)) / 4; }
            let last = number + 1 == raw.len();
            let page = if last && sample_count > 0 { sample_count } else { granule };
            if let (Some(position), true) = (previous_granule, last) {
                decoded = decoded.min(page.saturating_sub(position));
            }
            previous_granule = Some(page);
            starts.push(frame);
            frame += decoded;
        }
        starts.push(frame);
        let wwise_long = layout.mod_packets.then(|| raw.iter().map(|&(_, long)| long).collect());
        Ok(Self { ident, setup, packets: raw.into_iter().map(|(packet, _)| packet).collect(), joined: Vec::new(), starts, wwise_long })
    }

    pub fn new(ogg: &[u8]) -> io::Result<Self> {
        struct Raw { packet: (u32, u32, bool), granule: u64, last_in_page: bool, last_in_stream: bool }
        let mut raws = Vec::new();
        let mut joined = Vec::new();
        let mut partial: Option<Vec<u8>> = None;
        let mut offset = 0usize;
        while offset < ogg.len() {
            let header = ogg.get(offset..offset + 27).ok_or_else(|| invalid("truncated Ogg page header"))?;
            if &header[..4] != b"OggS" { return Err(invalid("missing Ogg page capture pattern")); }
            let flags = header[5];
            let granule = u64::from_le_bytes(header[6..14].try_into().unwrap());
            let segments = usize::from(header[26]);
            let lacing = ogg.get(offset + 27..offset + 27 + segments).ok_or_else(|| invalid("truncated Ogg lacing table"))?;
            let mut position = offset + 27 + segments;
            let mut packet_start = position;
            let first_in_page = raws.len();
            for &length in lacing {
                position += usize::from(length);
                if length == 255 { continue; }
                let bytes = ogg.get(packet_start..position).ok_or_else(|| invalid("truncated Ogg packet"))?;
                let packet = match partial.take() {
                    Some(mut previous) => {
                        previous.extend_from_slice(bytes);
                        let start = joined.len() as u32;
                        joined.extend_from_slice(&previous);
                        (start, previous.len() as u32, true)
                    }
                    None => (packet_start as u32, bytes.len() as u32, false),
                };
                raws.push(Raw { packet, granule, last_in_page: false, last_in_stream: false });
                packet_start = position;
            }
            if packet_start < position {
                let bytes = ogg.get(packet_start..position).ok_or_else(|| invalid("truncated Ogg packet"))?;
                partial.get_or_insert_with(Vec::new).extend_from_slice(bytes);
            }
            if let Some(last) = raws[first_in_page..].last_mut() {
                last.last_in_page = true;
                last.last_in_stream = flags & 4 != 0;
            }
            offset = position;
        }
        u32::try_from(ogg.len()).map_err(|_| invalid("Ogg stream exceeds the 32-bit packet index"))?;
        let bytes = |packet: (u32, u32, bool)| {
            let source = if packet.2 { &joined[..] } else { ogg };
            &source[packet.0 as usize..(packet.0 + packet.1) as usize]
        };
        let [ident, _, setup, ..] = &raws[..] else { return Err(invalid("Ogg Vorbis stream has no headers")) };
        let ident = read_header_ident(bytes(ident.packet)).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?;
        let setup = shared_setup(bytes(setup.packet), ident.audio_channels, (ident.blocksize_0, ident.blocksize_1))?;
        // The first packet decodes nothing; each later one decodes the span
        // between its window centres (a packet that cannot be read counts as
        // none, and decoding stops there). read_dec_packet_generic truncates the
        // stream's last packet to its page granule, counting from the last
        // complete page before it.
        let mut starts = Vec::with_capacity(raws.len() - 2);
        let mut packets = Vec::with_capacity(raws.len() - 3);
        let mut frame = 0u64;
        let mut granule: Option<u64> = None;
        for (number, raw) in raws[3..].iter().enumerate() {
            let data = bytes(raw.packet);
            let mut decoded = if number == 0 { 0 } else { get_decoded_sample_count(&ident, &setup, data).unwrap_or(0) as u64 };
            if let (Some(position), true) = (granule, raw.last_in_stream) {
                decoded = decoded.min(raw.granule.saturating_sub(position));
            }
            if raw.last_in_page { granule = Some(raw.granule); } else if let Some(position) = &mut granule { *position += decoded; }
            starts.push(frame);
            packets.push(raw.packet);
            frame += decoded;
        }
        starts.push(frame);
        Ok(Self { ident: Arc::new(ident), setup, packets, joined, starts, wwise_long: None })
    }

    pub fn channels(&self) -> u16 { u16::from(self.ident.audio_channels) }
    pub fn sample_rate(&self) -> u32 { self.ident.audio_sample_rate }
}

/// One voice's decoder over converted Vorbis media: packets decode forward
/// into a window that keeps the frame before the last request (the
/// resampler reads frames `n - 1` and `n`). A request before the window or
/// well past it decodes from the packet holding it, after the one before
/// it rebuilds the overlap.
pub struct VorbisCursor {
    media: Arc<super::EncodedVorbis>,
    window_right: PreviousWindowRight,
    next: usize,
    window: Vec<f32>,
    start: usize,
    /// The first HEAD_FRAMES decoded frames, kept from the first pass: a
    /// loop wrap's reads alternate between the stream's last frames and its
    /// first ones.
    head: Vec<f32>,
}

/// Frames kept from a stream's start: well past any resampler's reach back
/// across a loop seam, and a small fraction of a second of audio.
const HEAD_FRAMES: usize = 8192;

impl VorbisCursor {
    pub fn new(media: Arc<super::EncodedVorbis>) -> Self {
        Self { media, window_right: PreviousWindowRight::new(), next: 0, window: Vec::new(), start: 0, head: Vec::new() }
    }

    fn decode(&mut self, packet: usize) -> Option<Vec<f32>> {
        let index = &self.media.index;
        let (start, length, joined) = *index.packets.get(packet)?;
        let source = if joined { &index.joined[..] } else { &self.media.ogg[..] };
        let data = &source[start as usize..(start + length) as usize];
        let mut samples = match &index.wwise_long {
            Some(long) => read_wwise_audio_packet_generic::<InterleavedSamples<f32>>(&index.ident, &*index.setup, data,
                packet.checked_sub(1).is_some_and(|i| long[i]), long.get(packet + 1).copied().unwrap_or(false), &mut self.window_right),
            None => read_audio_packet_generic::<InterleavedSamples<f32>>(&index.ident, &*index.setup, data, &mut self.window_right),
        }.ok()?.samples;
        let frames = (index.starts[packet + 1] - index.starts[packet]) as usize;
        samples.truncate(frames * usize::from(self.media.channels));
        Some(samples)
    }

    fn seek(&mut self, frame: usize) -> Option<()> {
        let starts = &self.media.index.starts;
        let packet = starts.partition_point(|&start| start <= frame as u64).checked_sub(1)?;
        self.window_right = PreviousWindowRight::new();
        if packet > 0 { self.decode(packet - 1)?; }
        self.window.clear();
        self.start = usize::try_from(self.media.index.starts[packet]).ok()?;
        self.next = packet;
        Some(())
    }

    pub fn sample(&mut self, frame: usize, channel: usize) -> Option<f32> {
        let channels = usize::from(self.media.channels);
        if frame >= self.media.frames || channel >= channels { return None; }
        if frame < self.start && frame < self.head.len() / channels {
            return self.head.get(frame * channels + channel).copied();
        }
        let end = self.start + self.window.len() / channels;
        if frame < self.start || frame >= end + (1usize << self.media.index.ident.blocksize_1) {
            self.seek(frame)?;
        }
        while frame >= self.start + self.window.len() / channels {
            let keep = frame.saturating_sub(1).saturating_sub(self.start).min(self.window.len() / channels);
            self.window.drain(..keep * channels);
            self.start += keep;
            let packet = self.next;
            let samples = self.decode(packet)?;
            self.next += 1;
            let position = self.start + self.window.len() / channels;
            if position == self.head.len() / channels && position < HEAD_FRAMES {
                let take = samples.len().min((HEAD_FRAMES - position) * channels);
                self.head.extend_from_slice(&samples[..take]);
            }
            self.window.extend(samples);
        }
        self.window.get((frame - self.start) * channels + channel).copied()
    }
}

