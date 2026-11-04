//! File-source metadata and bounded decoders. Vorbis 1411614e0 retains the
//! seek table/setup; 141161240 assembles one packet across IO buffers.
//! Host: lewton's output windows need a priming offset at long/short changes.
use super::{stream_io::{ReadPool, Window}, vorbis::{wwise_headers, wwise_layout_sized}, StreamFormat};
use lewton::{audio::{read_audio_packet_generic, read_wwise_audio_packet_generic, wwise_packet_mode, PreviousWindowRight},
    header::{IdentHeader, SetupHeader}, samples::InterleavedSamples};
use std::{fs::File, io::{self, Read, Seek, SeekFrom}, path::PathBuf, sync::Arc};

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

enum Codec {
    Vorbis { ident: Arc<IdentHeader>, setup: Arc<SetupHeader>, seek: Arc<[u8]>, first: usize,
        modified: bool, packet_header: usize, leading: usize },
    Adpcm,
}

pub(crate) struct FileMedia {
    pub(crate) format: StreamFormat,
    pool: Arc<ReadPool>,
    path: Arc<PathBuf>,
    offset: u64,
    size: usize,
    prefix: Option<super::MediaBytes>,
    codec: Codec,
}

impl FileMedia {
    /// Background acquisition reads metadata/setup and two mode prefixes;
    /// neither audio payload nor a per-packet index is retained.
    pub(crate) fn open(pool: Arc<ReadPool>, path: Arc<PathBuf>, offset: u64, size: usize,
        prefetch: Option<super::MediaBytes>) -> io::Result<Option<Self>> {
        if let Some(prefix) = &prefetch {
            if let Some(media) = Self::from_prefetch(pool.clone(), path.clone(), offset, size, prefix.clone())? { return Ok(Some(media)); }
        }
        let mut file = File::open(path.as_ref())?;
        file.seek(SeekFrom::Start(offset))?;
        let mut prefix = super::read_stream_header(&mut file, size)?;
        let Some(format) = super::stream_format(&prefix, size)? else { return Ok(None) };
        if format.codec == 0xffff {
            let Some(layout) = wwise_layout_sized(&prefix, size) else { return Ok(None) };
            let end = format.data.start.checked_add(layout.first_audio).filter(|end| *end <= format.data.end)
                .ok_or_else(|| invalid("Vorbis setup exceeds source"))?;
            let acquired = prefix.len();
            prefix.resize(end, 0);
            file.read_exact(&mut prefix[acquired..])?;
        }
        let Some(codec) = Self::codec(&prefix, &format, &mut file, offset)? else { return Ok(None) };
        Self::prepared(pool, path, offset, size, format, codec, prefetch, prefix.len()).map(Some)
    }

    /// Native type-1 startup: parse the bank's borrowed prefix without opening
    /// the package. An incomplete prefix defers to background continuation.
    pub(crate) fn from_prefetch(pool: Arc<ReadPool>, path: Arc<PathBuf>, offset: u64, size: usize,
        prefix: super::MediaBytes) -> io::Result<Option<Self>> {
        if prefix.len() > size { return Err(invalid("sound prefetch exceeds package source")); }
        let mut reader = std::io::Cursor::new(prefix.as_ref());
        let header = match super::read_stream_header(&mut reader, size) {
            Ok(header) => header,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(format) = super::stream_format(&header, size)? else { return Ok(None) };
        let codec = match Self::codec(&prefix, &format, &mut reader, 0) {
            Ok(Some(codec)) => codec,
            Ok(None) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        };
        let acquired = prefix.len();
        Self::prepared(pool, path, offset, size, format, codec, Some(prefix), acquired).map(Some)
    }

    fn codec(prefix: &[u8], format: &StreamFormat, reader: &mut (impl Read + Seek), source: u64) -> io::Result<Option<Codec>> {
        Ok(Some(match format.codec {
            2 => Codec::Adpcm,
            0xffff => {
                let Some(layout) = wwise_layout_sized(prefix, format.data.end) else { return Ok(None) };
                let end = format.data.start.checked_add(layout.first_audio).filter(|end| *end <= format.data.end)
                    .ok_or_else(|| invalid("Vorbis setup exceeds source"))?;
                if end > prefix.len() { return Ok(None); }
                let (ident, setup) = wwise_headers(prefix, &layout)?;
                let seek: Arc<[u8]> = prefix.get(format.data.start..format.data.start + layout.setup)
                    .ok_or_else(|| invalid("Vorbis seek table exceeds setup"))?.into();
                let first = format.data.start + layout.first_audio;
                let packet_header = if layout.granule_headers { 6 } else { 2 };
                let (long, next) = initial_modes(reader, source, first, format.data.end, packet_header, &setup, layout.mod_packets)?;
                let leading = tail_bias(&ident, long, next);
                Codec::Vorbis { ident, setup, seek, first, modified: layout.mod_packets, packet_header, leading }
            }
            _ => return Ok(None),
        }))
    }

    fn prepared(pool: Arc<ReadPool>, path: Arc<PathBuf>, offset: u64, size: usize,
        format: StreamFormat, codec: Codec, prefix: Option<super::MediaBytes>, acquired: usize) -> io::Result<Self> {
        pool.start()?;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            let (kind, retained) = match &codec { Codec::Vorbis { seek, .. } => ("Vorbis", seek.len()), Codec::Adpcm => ("IMA ADPCM", 0) };
            eprintln!("MEMORY file sound metadata: representation={kind} source_bytes={size} prefix_bytes={acquired} retained_seek_bytes={retained} bank_prefix_bytes={}", prefix.as_ref().map_or(0, |prefix| prefix.len()));
        }
        Ok(Self { format, pool, path, offset, size, prefix, codec })
    }

    fn window(&self) -> Window { Window::new(self.pool.clone(), self.path.clone(), self.offset, self.size, self.prefix.clone()) }

    /// 141160ba0: keep the authored packed table; expand only the selected
    /// pair while seeking, rather than retaining an expanded packet index.
    fn seek(&self, target: usize) -> io::Result<(usize, usize)> {
        let Codec::Vorbis { seek, first, leading, .. } = &self.codec else { return Ok((0, self.format.data.start)) };
        let (mut frame, mut bytes, mut selected) = (0usize, 0usize, (0usize, *first));
        for entry in seek.chunks_exact(4) {
            frame = frame.checked_add(usize::from(u16::from_le_bytes([entry[0], entry[1]])))
                .ok_or_else(|| invalid("Vorbis seek frame overflow"))?;
            if frame > target.saturating_add(*leading) { break; }
            bytes = bytes.checked_add(usize::from(u16::from_le_bytes([entry[2], entry[3]])))
                .ok_or_else(|| invalid("Vorbis seek offset overflow"))?;
            let offset = self.format.data.start.checked_add(seek.len()).and_then(|start| start.checked_add(bytes))
                .filter(|&offset| offset >= *first && offset < self.format.data.end)
                .ok_or_else(|| invalid("Vorbis seek point outside audio"))?;
            selected = (frame, offset);
        }
        Ok(selected)
    }
}

impl Drop for FileMedia { fn drop(&mut self) { self.pool.report(); } }

fn packet_long(setup: &SetupHeader, byte: u8, modified: bool) -> io::Result<bool> {
    // Vorbis has at most 64 modes: either packet layout's mode fits here.
    let byte = if modified { byte } else { byte >> 1 };
    wwise_packet_mode(setup, &[byte]).map(|(_, long)| long).map_err(|_| invalid("invalid Vorbis mode"))
}

fn tail_bias(ident: &IdentHeader, long: bool, next: bool) -> usize {
    if long && !next { ((1usize << ident.blocksize_1) - (1usize << ident.blocksize_0)) / 4 } else { 0 }
}

fn initial_modes(file: &mut (impl Read + Seek), source: u64, first: usize, end: usize, header: usize,
    setup: &SetupHeader, modified: bool) -> io::Result<(bool, bool)> {
    let mut read = |at: usize| -> io::Result<Option<(bool, usize)>> {
        if at == end { return Ok(None); }
        if at.checked_add(header + 1).is_none_or(|last| last > end) { return Err(invalid("short Vorbis packet prefix")); }
        file.seek(SeekFrom::Start(source.checked_add(at as u64).ok_or_else(|| invalid("sound package offset overflow"))?))?;
        let mut prefix = [0; 7];
        file.read_exact(&mut prefix[..header + 1])?;
        let size = usize::from(u16::from_le_bytes([prefix[0], prefix[1]]));
        let next = at.checked_add(header + size).filter(|&next| size != 0 && next <= end)
            .ok_or_else(|| invalid("Vorbis packet exceeds audio"))?;
        Ok(Some((packet_long(setup, prefix[header], modified)?, next)))
    };
    let (first, next) = read(first)?.ok_or_else(|| invalid("Vorbis source has no packets"))?;
    Ok((first, read(next)?.is_some_and(|(long, _)| long)))
}

pub(crate) enum FileCursor { Vorbis(VorbisCursor), Adpcm(AdpcmCursor) }
impl FileCursor {
    pub(crate) fn new(media: Arc<FileMedia>) -> Self {
        match &media.codec {
            Codec::Vorbis { first, .. } => Self::Vorbis(VorbisCursor { window: media.window(), next: *first,
                media, packet: Packet::default(), right: PreviousWindowRight::new(), samples: Vec::new(),
                start: 0, head: Vec::new(), previous_long: false, nominal: 0, priming: true, target: 0, waiting: false }),
            Codec::Adpcm => Self::Adpcm(AdpcmCursor { window: media.window(), media, block: usize::MAX,
                filled: 0, encoded: Vec::new(), samples: Vec::new(), waiting: false }),
        }
    }
    pub(crate) fn sample(&mut self, frame: usize, channel: usize) -> io::Result<Option<f32>> {
        match self { Self::Vorbis(cursor) => cursor.sample(frame, channel), Self::Adpcm(cursor) => cursor.sample(frame, channel) }
    }
    pub(crate) fn waiting(&self) -> bool {
        match self { Self::Vorbis(cursor) => cursor.waiting, Self::Adpcm(cursor) => cursor.waiting }
    }
}

pub(crate) struct AdpcmCursor {
    media: Arc<FileMedia>, window: Window, block: usize, filled: usize,
    encoded: Vec<u8>, samples: Vec<i16>, waiting: bool,
}
impl AdpcmCursor {
    fn sample(&mut self, frame: usize, channel: usize) -> io::Result<Option<f32>> {
        self.waiting = false;
        let channels = usize::from(self.media.format.channels);
        if frame >= self.media.format.frames || channel >= channels { return Ok(None); }
        let block = frame / 64;
        let length = 0x24 * channels;
        if self.block != block {
            self.block = block; self.filled = 0; self.encoded.resize(length, 0); self.samples.clear();
        }
        while self.filled < length {
            let copied = self.window.copy(self.media.format.data.start + block * length + self.filled, &mut self.encoded[self.filled..])?;
            if copied == 0 { self.waiting = true; return Ok(None); }
            self.filled += copied;
        }
        if self.samples.is_empty() {
            self.samples.resize(64 * channels, 0);
            for c in 0..channels {
                let decoded = super::decode_ima_block(&self.encoded[c * 0x24..(c + 1) * 0x24])?;
                for (i, sample) in decoded.into_iter().enumerate() { self.samples[i * channels + c] = sample; }
            }
        }
        Ok(Some(f32::from(self.samples[frame % 64 * channels + channel]) / 32768.0))
    }
}

#[derive(Default)]
struct Packet { offset: Option<usize>, header: [u8; 6], header_read: usize, data: Vec<u8>, read: usize,
    following: [u8; 7], following_read: usize, next: usize, inline: bool, payload: usize, length: usize }
impl Packet {
    fn prepare(&mut self, window: &mut Window, offset: usize, end: usize, header: usize) -> io::Result<bool> {
        if self.offset != Some(offset) {
            self.offset = Some(offset); self.header_read = 0; self.read = 0; self.following_read = 0;
        }
        while self.header_read < header {
            let copied = window.copy(offset + self.header_read, &mut self.header[self.header_read..header])?;
            if copied == 0 { return Ok(false); }
            self.header_read += copied;
        }
        let size = usize::from(u16::from_le_bytes([self.header[0], self.header[1]]));
        self.next = offset.checked_add(header + size).filter(|&next| size != 0 && next <= end)
            .ok_or_else(|| invalid("Vorbis packet exceeds source"))?;
        self.payload = offset + header;
        self.length = size;
        // Lewton also needs the next mode before consuming this packet.
        // Spill across that boundary so a pending lookahead can release the
        // old IO block without losing this packet's bytes.
        let lookahead = if self.next < end { header + 1 } else { 0 };
        self.inline = window.contiguous(self.payload, size + lookahead);
        if self.inline {
            if window.slice(self.payload, size)?.is_none() { return Ok(false); }
        } else {
            self.data.resize(size, 0);
            while self.read < size {
                let copied = window.copy(self.payload + self.read, &mut self.data[self.read..])?;
                if copied == 0 { return Ok(false); }
                self.read += copied;
            }
        }
        if self.next < end {
            if self.next.checked_add(header + 1).is_none_or(|next| next > end) { return Err(invalid("short next Vorbis packet")); }
            while self.following_read < header + 1 {
                let copied = window.copy(self.next + self.following_read, &mut self.following[self.following_read..header + 1])?;
                if copied == 0 { return Ok(false); }
                self.following_read += copied;
            }
        }
        Ok(true)
    }
}

pub(crate) struct VorbisCursor {
    media: Arc<FileMedia>, window: Window, packet: Packet, right: PreviousWindowRight,
    next: usize, samples: Vec<f32>, start: usize, head: Vec<f32>, previous_long: bool,
    nominal: usize, priming: bool, target: usize, waiting: bool,
}
impl VorbisCursor {
    fn seek(&mut self, frame: usize) -> io::Result<()> {
        let (nominal, offset) = self.media.seek(frame)?;
        self.nominal = nominal; self.next = offset; self.target = frame; self.priming = true;
        self.start = frame; self.samples.clear(); self.right = PreviousWindowRight::new(); self.previous_long = false;
        Ok(())
    }

    fn sample(&mut self, frame: usize, channel: usize) -> io::Result<Option<f32>> {
        self.waiting = false;
        let channels = usize::from(self.media.format.channels);
        if frame >= self.media.format.frames || channel >= channels { return Ok(None); }
        if frame < self.start && frame < self.head.len() / channels { return Ok(self.head.get(frame * channels + channel).copied()); }
        let end = self.start + self.samples.len() / channels;
        // The ordinary sample path touches no header reference counts.
        if !self.priming && frame >= self.start && frame < end {
            return Ok(self.samples.get((frame - self.start) * channels + channel).copied());
        }
        let Codec::Vorbis { ident, first, modified, packet_header, leading, .. } = &self.media.codec else { unreachable!() };
        let (first, modified, header, leading, block_frames) = (*first, *modified, *packet_header, *leading, 1usize << ident.blocksize_1);
        if (!self.priming && (frame < self.start || frame >= end + block_frames))
            || (self.priming && self.target != frame && frame >= end + block_frames) {
            self.seek(frame)?;
        }
        while self.priming || frame >= self.start + self.samples.len() / channels {
            let Codec::Vorbis { ident, setup, .. } = &self.media.codec else { unreachable!() };
            if self.next >= self.media.format.data.end { return Err(invalid("Vorbis stream ended before its authored frame count")); }
            if !self.packet.prepare(&mut self.window, self.next, self.media.format.data.end, header)? {
                self.waiting = true; return Ok(None);
            }
            let payload = if self.packet.inline {
                let Some(payload) = self.window.slice(self.packet.payload, self.packet.length)? else {
                    self.waiting = true; return Ok(None);
                };
                payload
            } else { &self.packet.data };
            let long = packet_long(setup, payload[0], modified)?;
            let next_long = if self.packet.next == self.media.format.data.end { false }
                else { packet_long(setup, self.packet.following[header], modified)? };
            if self.priming {
                let origin = if self.next == first { 0 } else {
                    self.nominal.checked_add(tail_bias(ident, long, next_long)).and_then(|frame| frame.checked_sub(leading))
                        .ok_or_else(|| invalid("Vorbis priming frame outside source"))?
                };
                if origin > self.target {
                    // Lewton emits a long packet's unwindowed tail early.
                    // Prime from an earlier authored point if that tail is
                    // needed; never substitute silence or shift the PCM.
                    self.seek(self.nominal.saturating_sub(leading).saturating_sub(1))?;
                    self.target = frame;
                    continue;
                }
                self.start = origin;
            }
            let mut decoded = if modified {
                read_wwise_audio_packet_generic::<InterleavedSamples<f32>>(ident, setup, payload,
                    self.previous_long, next_long, &mut self.right)
            } else { read_audio_packet_generic::<InterleavedSamples<f32>>(ident, setup, payload, &mut self.right) }
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("Vorbis file packet: {error:?}")))?.samples;
            if !self.priming {
                let previous = 1usize << if self.previous_long { ident.blocksize_1 } else { ident.blocksize_0 };
                let current = 1usize << if long { ident.blocksize_1 } else { ident.blocksize_0 };
                if self.packet.next == self.media.format.data.end {
                    decoded.truncate(self.media.format.frames.saturating_sub(self.nominal) * channels);
                }
                self.nominal += (previous + current) / 4;
            }
            self.priming = false; self.previous_long = long; self.next = self.packet.next;
            let keep = frame.saturating_sub(1).saturating_sub(self.start).min(self.samples.len() / channels);
            self.samples.drain(..keep * channels); self.start += keep;
            let position = self.start + self.samples.len() / channels;
            const HEAD_FRAMES: usize = 8192;
            if position == self.head.len() / channels && position < HEAD_FRAMES {
                self.head.extend_from_slice(&decoded[..decoded.len().min((HEAD_FRAMES - position) * channels)]);
            }
            self.samples.extend(decoded);
        }
        Ok(self.samples.get((frame - self.start) * channels + channel).copied())
    }
}
