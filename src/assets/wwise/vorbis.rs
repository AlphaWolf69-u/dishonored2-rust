//! Converted Wwise Vorbis media: a packet index for decoding from any frame.
//! Wwise positions its Vorbis sources with the media's seek table
//! (141160ba0, used by the virtual-voice return 1411609c0). The converted
//! Ogg stream's granules do not count frames across block-size changes, so
//! each packet's frame position comes from the decoder's own windowing.
use lewton::{audio::{get_decoded_sample_count, read_audio_packet_generic, PreviousWindowRight},
    header::{read_header_ident, read_header_setup, IdentHeader, SetupHeader}, samples::InterleavedSamples};
use std::{io, sync::Arc};

pub struct PacketIndex {
    ident: IdentHeader,
    setup: SetupHeader,
    /// Audio packets: a byte range of the Ogg stream, or of `joined` for a
    /// packet continued across pages.
    packets: Vec<(u32, u32, bool)>,
    joined: Vec<u8>,
    /// Frame at which each packet's decoded output begins; one more entry
    /// holds the stream's decoded length.
    starts: Vec<u64>,
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message.to_owned()) }

impl PacketIndex {
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
        let setup = read_header_setup(bytes(setup.packet), ident.audio_channels, (ident.blocksize_0, ident.blocksize_1))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}")))?;
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
        Ok(Self { ident, setup, packets, joined, starts })
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
        let mut samples = read_audio_packet_generic::<InterleavedSamples<f32>>(&index.ident, &index.setup, data, &mut self.window_right)
            .ok()?.samples;
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

