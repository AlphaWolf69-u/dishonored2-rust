//! Read Wwise AKPK file packages used by the installed game.
//!
//! Dishonored 2 stores its Wwise banks and media in `base/pck/*.pck`. Package
//! entries carry numeric IDs and absolute file offsets; their audio data stays
//! in the installation and is read on demand.
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
mod bank;
pub use bank::{CurvePoint, NodeProperties, PropertyRange, RtpcCurve};

/// The authoritative Wwise ID stored in an installed arkDeclSoundEvent.
pub fn declared_event_id(source: &str) -> io::Result<u32> {
    declared_engine_id(source, "m_EngineID")
}

pub fn declared_rtpc_id(source: &str) -> io::Result<u32> {
    declared_engine_id(source, "m_engineID")
}

fn declared_engine_id(source: &str, member: &str) -> io::Result<u32> {
    use crate::entities::scene_motion::{atom, child};
    let fields = crate::entities::parse_declaration(source).map_err(invalid_owned)?;
    let id = atom(child(child(&fields, "edit"), member), "m_ID")
        .ok_or_else(|| invalid("sound declaration has no engine ID"))?;
    id.parse().map_err(|_| invalid("invalid sound event engine ID"))
}

/// Decode the Wwise media formats used by the installed game into audio
/// playable by Bevy. The RIFF codec ID selects Wwise Vorbis (`0xffff`) or
/// Wwise IMA ADPCM (`0x0002`).
pub fn decode_wem(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(invalid("Wwise media is not a little-endian RIFF/WAVE file"));
    }
    let mut cursor = 12usize;
    let mut format = None;
    let mut data = None;
    while cursor.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let tag = &bytes[cursor..cursor + 4];
        let size = slice_u32(bytes, cursor + 4)? as usize;
        let start = cursor + 8;
        let end = start.checked_add(size).ok_or_else(|| invalid("Wwise media chunk size overflow"))?;
        let chunk = bytes.get(start..end).ok_or_else(|| invalid("Wwise media chunk exceeds file size"))?;
        match tag {
            b"fmt " => format = Some(chunk),
            b"data" => data = Some(chunk),
            _ => {}
        }
        cursor = end + (size & 1);
    }
    let format = format.ok_or_else(|| invalid("Wwise media has no fmt chunk"))?;
    let audio = data.ok_or_else(|| invalid("Wwise media has no data chunk"))?;
    let codec = slice_u16(format, 0)?;
    let channels = slice_u16(format, 2)? as usize;
    let sample_rate = slice_u32(format, 4)?;
    let block_align = slice_u16(format, 12)? as usize;
    let bits_per_sample = slice_u16(format, 14)?;
    match codec {
        0xffff => {
            let codebooks = ww2ogg::CodebookLibrary::default_codebooks()
                .map_err(|error| invalid_owned(format!("Wwise Vorbis codebooks unavailable: {error}")))?;
            let mut converter = ww2ogg::WwiseRiffVorbis::new(Cursor::new(bytes), codebooks)
                .map_err(|error| invalid_owned(format!("invalid Wwise Vorbis media: {error}")))?;
            let mut ogg = Vec::new();
            converter.generate_ogg(&mut ogg)
                .map_err(|error| invalid_owned(format!("Wwise Vorbis conversion failed: {error}")))?;
            Ok(ogg)
        }
        1 | 0xfffe if channels > 0 && channels <= 8 && sample_rate > 0 => {
            let samples: Vec<i16> = match bits_per_sample {
                8 if block_align == channels => audio.iter().map(|sample| ((*sample as i16) - 128) << 8).collect(),
                16 if block_align == channels * 2 && audio.len() % 2 == 0 => audio.chunks_exact(2)
                    .map(|sample| i16::from_le_bytes([sample[0], sample[1]])).collect(),
                _ => return Err(invalid("unsupported Wwise PCM format parameters")),
            };
            Ok(pcm_wav(&samples, channels as u16, sample_rate))
        }
        2 => {
            if channels == 0 || channels > 8 || sample_rate == 0 || bits_per_sample != 4
                || block_align != 0x24 * channels
            {
                return Err(invalid("unsupported Wwise IMA format parameters"));
            }
            let channel_block = block_align / channels;
            let mut pcm = Vec::<i16>::new();
            let mut offset = 0usize;
            while offset + block_align <= audio.len() {
                let mut decoded_channels = Vec::with_capacity(channels);
                for channel in 0..channels {
                    let start = offset + channel * channel_block;
                    decoded_channels.push(decode_ima_block(&audio[start..start + channel_block])?);
                }
                for sample in 0..64 {
                    for channel in &decoded_channels { pcm.push(channel[sample]); }
                }
                offset += block_align;
            }
            if offset != audio.len() { return Err(invalid("Wwise IMA data ends with a partial block")); }
            Ok(pcm_wav(&pcm, channels as u16, sample_rate))
        }
        _ => Err(invalid_owned(format!("unsupported Wwise media codec {codec:#06x}"))),
    }
}

fn decode_ima_block(block: &[u8]) -> io::Result<Vec<i16>> {
    if block.len() != 0x24 { return Err(invalid("invalid Wwise IMA block size")); }
    let mut predictor = i16::from_le_bytes([block[0], block[1]]) as i32;
    let mut index = block[2] as i32;
    if index > 88 { return Err(invalid("Wwise IMA step index is out of range")); }
    let mut samples = Vec::with_capacity(64);
    // Wwise's 0x24-byte blocks use Xbox IMA framing: emit the predictor
    // sample from the header, then decode 63 nibbles and skip the final one.
    samples.push(predictor as i16);
    for &byte in &block[4..] {
        for nibble in [byte & 0x0f, byte >> 4] {
            if samples.len() == 64 { return Ok(samples); }
            let step = IMA_STEP_TABLE[index as usize] as i32;
            let mut delta = step >> 3;
            if nibble & 1 != 0 { delta += step >> 2; }
            if nibble & 2 != 0 { delta += step >> 1; }
            if nibble & 4 != 0 { delta += step; }
            predictor = (predictor + if nibble & 8 != 0 { -delta } else { delta }).clamp(i16::MIN as i32, i16::MAX as i32);
            index = (index + IMA_INDEX_TABLE[nibble as usize] as i32).clamp(0, 88);
            samples.push(predictor as i16);
        }
    }
    Ok(samples)
}

const IMA_STEP_TABLE: [i16; 89] = [
    7,8,9,10,11,12,13,14,16,17,19,21,23,25,28,31,34,37,41,45,50,55,60,66,73,80,88,97,107,118,130,143,
    157,173,190,209,230,253,279,307,337,371,408,449,494,544,598,658,724,796,876,963,1060,1166,1282,1411,
    1552,1707,1878,2066,2272,2499,2749,3024,3327,3660,4026,4428,4871,5358,5894,6484,7132,7845,8630,9493,
    10442,11487,12635,13899,15289,16818,18500,20350,22385,24623,27086,29794,32767,
];
const IMA_INDEX_TABLE: [i8; 16] = [-1,-1,-1,-1,2,4,6,8,-1,-1,-1,-1,2,4,6,8];

fn pcm_wav(samples: &[i16], channels: u16, sample_rate: u32) -> Vec<u8> {
    let data_size = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(data_size as usize + 44);
    wav.extend_from_slice(b"RIFF"); wav.extend_from_slice(&(data_size + 36).to_le_bytes()); wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); wav.extend_from_slice(&1u16.to_le_bytes()); wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes()); wav.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    wav.extend_from_slice(&(channels * 2).to_le_bytes()); wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data"); wav.extend_from_slice(&data_size.to_le_bytes());
    for sample in samples { wav.extend_from_slice(&sample.to_le_bytes()); }
    wav
}

fn slice_u16(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(bytes.get(offset..offset + 2).ok_or_else(|| invalid("truncated Wwise media field"))?.try_into().unwrap()))
}
fn slice_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(bytes.get(offset..offset + 4).ok_or_else(|| invalid("truncated Wwise media field"))?.try_into().unwrap()))
}
fn invalid_owned(message: String) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    pub id: u32,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub language_id: u32,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub languages: Vec<Language>,
    pub banks: Vec<Entry>,
    pub media: Vec<Entry>,
    pub external_media: Vec<Entry>,
    pub data_offset: u64,
    file_size: u64,
}

impl Package {
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Open and index an installed Wwise package without loading its media.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        Self::read(&mut file)
    }

    /// Read the package tables from a seekable source.
    pub fn read<R: Read + Seek>(reader: &mut R) -> io::Result<Self> {
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(0))?;

        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        if &magic != b"AKPK" {
            return Err(invalid("Wwise package signature"));
        }

        let header_size = read_u32(reader)? as u64;
        let _flags = read_u32(reader)?;
        let section_sizes = [read_u32(reader)? as u64, read_u32(reader)? as u64, read_u32(reader)? as u64];
        let first_three = section_sizes.iter().try_fold(0u64, |sum, size| sum.checked_add(*size))
            .ok_or_else(|| invalid("Wwise section size overflow"))?;

        // File Packager's header_size excludes its fixed 0x10-byte prefix and
        // padding. Later packages add a fourth (external media) section.
        if first_three.checked_add(0x10).is_some_and(|size| size < header_size) {
            let external_size = read_u32(reader)? as u64;
            // Keep the three standard sections in order; the external table
            // follows them and is parsed separately below.
            let language_bytes = read_section(reader, section_sizes[0], file_size)?;
            let bank_bytes = read_section(reader, section_sizes[1], file_size)?;
            let media_bytes = read_section(reader, section_sizes[2], file_size)?;
            let external_bytes = read_section(reader, external_size, file_size)?;
            let data_offset = reader.stream_position()?;
            return Self::from_sections(language_bytes, bank_bytes, media_bytes, external_bytes, data_offset, file_size);
        }

        let language_bytes = read_section(reader, section_sizes[0], file_size)?;
        let bank_bytes = read_section(reader, section_sizes[1], file_size)?;
        let media_bytes = read_section(reader, section_sizes[2], file_size)?;
        let data_offset = reader.stream_position()?;
        Self::from_sections(language_bytes, bank_bytes, media_bytes, Vec::new(), data_offset, file_size)
    }

    fn from_sections(
        language_bytes: Vec<u8>,
        bank_bytes: Vec<u8>,
        media_bytes: Vec<u8>,
        external_bytes: Vec<u8>,
        data_offset: u64,
        file_size: u64,
    ) -> io::Result<Self> {
        let languages = parse_languages(&language_bytes)?;
        let banks = parse_entries(&bank_bytes, EntryTable::Standard)?;
        let media = parse_entries(&media_bytes, EntryTable::Standard)?;
        let external_media = if external_bytes.is_empty() {
            Vec::new()
        } else {
            parse_entries(&external_bytes, EntryTable::External)?
        };

        let package = Self { languages, banks, media, external_media, data_offset, file_size };
        for entry in package.banks.iter().chain(&package.media).chain(&package.external_media) {
            let end = entry.offset.checked_add(entry.size).ok_or_else(|| invalid("Wwise entry range overflow"))?;
            if entry.offset < data_offset || end > file_size {
                return Err(invalid("Wwise entry outside package data"));
            }
        }
        Ok(package)
    }

    pub fn bank(&self, id: u64, language_id: u32) -> Option<Entry> {
        self.banks.iter().copied().find(|entry| entry.id == id && entry.language_id == language_id)
    }

    pub fn media_file(&self, id: u64, language_id: u32) -> Option<Entry> {
        self.media.iter().chain(&self.external_media).copied()
            .find(|entry| entry.id == id && entry.language_id == language_id)
    }

    /// Read one indexed bank or media item from the package.
    pub fn read_entry<R: Read + Seek>(&self, reader: &mut R, entry: Entry) -> io::Result<Vec<u8>> {
        let end = entry.offset.checked_add(entry.size).ok_or_else(|| invalid("Wwise entry range overflow"))?;
        if entry.offset < self.data_offset || end > self.file_size {
            return Err(invalid("Wwise entry outside package data"));
        }
        let size = usize::try_from(entry.size).map_err(|_| invalid("Wwise entry too large for host"))?;
        reader.seek(SeekFrom::Start(entry.offset))?;
        let mut bytes = vec![0; size];
        reader.read_exact(&mut bytes)?;
        Ok(bytes)
    }
}

/// Wwise's `FUN_1400c1210`: ASCII-lowercase FNV-1 used by sound event IDs.
pub fn event_id(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5, |hash, byte| {
        let byte = if byte.is_ascii_uppercase() { byte + 0x20 } else { byte };
        hash.wrapping_mul(0x0100_0193) ^ byte as u32
    })
}

/// Resolve one-shot media groups for an event. Each group is played once;
/// random containers contribute the weighted alternatives for that group.
/// This follows the installed menu bank's Wwise 2015.1 (version 113) layout.
pub fn event_media(
    bank: &[u8],
    event: u32,
    package_media: &std::collections::HashMap<u32, Vec<u8>>,
) -> io::Result<Option<Vec<MediaGroup>>> {
    event_media_with(bank, event, |id| Ok(package_media.get(&id).cloned()))
}

/// Resolve an event while loading streamed media only when a resolved sound
/// references it. The callback receives the source media ID from a Sound HIRC
/// object, including media not embedded in the bank's DIDX/DATA chunks.
pub fn event_media_with(
    bank: &[u8],
    event: u32,
    package_media: impl FnMut(u32) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Option<Vec<MediaGroup>>> {
    Ok(event_commands_with(bank, event, package_media)?.and_then(|commands| commands.into_iter().find_map(|command| {
        match command { EventCommand::Play { groups, .. } => Some(groups), EventCommand::Stop { .. } => None }
    })))
}

/// Resolve Wwise Play and Stop actions in authored event order. Stop targets
/// use the same HIRC object ID as the corresponding Play action.
pub fn event_commands_with(
    bank: &[u8],
    event: u32,
    mut package_media: impl FnMut(u32) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Option<Vec<EventCommand>>> {
    let chunks = bank_chunks(bank)?;
    let feedback = bank_feedback(&chunks)?;
    let Some(hierarchy) = chunks.get(b"HIRC") else { return Ok(None) };
    let objects = bank_objects(hierarchy)?;

    let Some(event_object) = objects.get(&(4, event)).copied() else { return Ok(None) };
    if event_object.len() < 8 { return Err(invalid("truncated Wwise event")); }
    let action_count = read_u32_slice(event_object, 4)? as usize;
    fn resolve_node(
        id: u32,
        objects: &BankObjects<'_>,
        chunks: &std::collections::HashMap<[u8; 4], &[u8]>,
        package_media: &mut impl FnMut(u32) -> io::Result<Option<Vec<u8>>>,
        visited: &mut std::collections::HashSet<u32>,
        feedback: bool,
    ) -> io::Result<Vec<MediaGroup>> {
        if !visited.insert(id) { return Ok(Vec::new()); }
        let Some((kind, object)) = audio_node(objects, id) else { return Ok(Vec::new()) };
        if kind == 2 {
            let media = media_from_sound(object, chunks, package_media)?;
            let properties = media_properties(id, objects, feedback)?;
            visited.remove(&id);
            return Ok(media.map_or_else(Vec::new, |media| vec![MediaGroup {
                node: id, global: true, avoid_repeat: 0,
                mode: SelectionMode::Single,
                reset_each_play: true,
                variants: vec![MediaVariant { media, weight: 1, properties }],
            }]));
        }
        if kind != 5 && kind != 9 { visited.remove(&id); return Ok(Vec::new()); }
        let (children, playlist) = bank::node_children(object, kind, feedback)?;
        let mut groups = Vec::new();
        if kind == 5 {
            let mut variants = Vec::new();
            for (child, weight) in playlist.items {
                let resolved = resolve_node(child, objects, chunks, package_media, visited, feedback)?;
                if resolved.len() > 1 || resolved.iter().any(|group| group.mode != SelectionMode::Single) {
                    return Err(invalid("nested Wwise selection containers are not supported yet"));
                }
                for mut group in resolved {
                    for variant in &mut group.variants { variant.weight = weight; }
                    variants.append(&mut group.variants);
                }
            }
            if !variants.is_empty() {
                groups.push(MediaGroup {
                    node: id, global: playlist.global, avoid_repeat: playlist.avoid_repeat,
                    mode: playlist.mode, reset_each_play: playlist.reset_each_play, variants,
                });
            }
        } else {
            for child in children {
                groups.extend(resolve_node(child, objects, chunks, package_media, visited, feedback)?);
            }
        }
        visited.remove(&id);
        Ok(groups)
    }
    let mut commands = Vec::new();
    for index in 0..action_count {
        let action_id = read_u32_slice(event_object, 8 + index * 4)?;
        let Some(action) = objects.get(&(3, action_id)).copied() else { continue };
        if action.len() < 10 { continue; }
        let target = read_u32_slice(action, 6)?;
        if action[5] == 0x01 { // Wwise action type 0x01 is Stop.
            commands.push(EventCommand::Stop { target });
            continue;
        }
        if action[5] != 0x04 { continue; } // Wwise action type 0x04 is Play.
        let mut visited = std::collections::HashSet::new();
        let groups = resolve_node(target, &objects, &chunks, &mut package_media, &mut visited, feedback)?;
        if !groups.is_empty() { commands.push(EventCommand::Play { target, groups }); }
    }
    Ok((!commands.is_empty()).then_some(commands))
}

#[derive(Clone, Debug)]
pub enum EventCommand {
    Stop { target: u32 },
    Play { target: u32, groups: Vec<MediaGroup> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode { Single, Random, Shuffle, Sequence }

#[derive(Clone, Debug)]
pub struct MediaGroup {
    pub node: u32,
    pub global: bool,
    pub avoid_repeat: u16,
    pub mode: SelectionMode,
    pub reset_each_play: bool,
    pub variants: Vec<MediaVariant>,
}

#[derive(Clone, Debug)]
pub struct MediaVariant {
    pub media: Vec<u8>,
    pub weight: u32,
    pub properties: MediaProperties,
}

#[derive(Clone, Debug, Default)]
pub struct MediaProperties {
    /// Sound and actor/container ancestors, from the sound to the root.
    pub nodes: Vec<NodeProperties>,
    pub bus: u32,
}

fn media_properties(mut id: u32, objects: &BankObjects<'_>, feedback: bool) -> io::Result<MediaProperties> {
    let mut properties = MediaProperties::default();
    let mut visited = std::collections::HashSet::new();
    while id != 0 {
        if !visited.insert(id) { return Err(invalid("cyclic Wwise parent hierarchy")); }
        let Some((kind, object)) = audio_node(objects, id) else { return Err(invalid_owned(format!("missing Wwise parent {id:#010x}"))); };
        if !matches!(kind, 2 | 5 | 6 | 7 | 9) { return Err(invalid("unsupported Wwise parent type")); }
        let node = bank::node_properties(object, kind, feedback)?;
        if properties.bus == 0 { properties.bus = node.bus; }
        id = node.parent;
        properties.nodes.push(node);
    }
    Ok(properties)
}

#[derive(Clone, Debug)]
pub struct BankMix {
    pub buses: Vec<NodeProperties>,
    pub parameters: std::collections::HashMap<u32, f32>,
    pub volume_threshold_db: f32,
}

/// The initialization bank's authored buses and global game-parameter defaults.
pub fn bank_mix(bytes: &[u8]) -> io::Result<Option<BankMix>> {
    let chunks = bank_chunks(bytes)?;
    let Some(settings) = chunks.get(b"STMG") else { return Ok(None) };
    let feedback = bank_feedback(&chunks)?;
    let (volume_threshold_db, parameters) = bank::global_parameters(settings)?;
    let objects = bank_objects(chunks.get(b"HIRC").ok_or_else(|| invalid("Wwise initialization bank has no hierarchy"))?)?;
    let mut buses = Vec::new();
    for ((kind, _), object) in objects {
        if kind == 8 { buses.push(bank::bus_properties(object, feedback)?); }
    }
    Ok(Some(BankMix { buses, parameters, volume_threshold_db }))
}

fn media_from_sound(
    sound: &[u8],
    chunks: &std::collections::HashMap<[u8; 4], &[u8]>,
    package_media: &mut impl FnMut(u32) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<Option<Vec<u8>>> {
    if sound.len() < 13 { return Ok(None); }
    let media_id = read_u32_slice(sound, 9)?;
    // Streamed and prefetched sources use the full package file. DIDX may
    // contain only a prefetch prefix, which must not replace the full media.
    if sound[8] != 0 {
        if let Some(media) = package_media(media_id)? { return Ok(Some(media)); }
    }
    if let (Some(didx), Some(data)) = (chunks.get(b"DIDX"), chunks.get(b"DATA")) {
        if didx.len() % 12 != 0 { return Err(invalid("invalid Wwise DIDX size")); }
        for record in didx.chunks_exact(12) {
            if read_u32_slice(record, 0)? != media_id { continue; }
            let start = read_u32_slice(record, 4)? as usize;
            let end = start.checked_add(read_u32_slice(record, 8)? as usize).ok_or_else(|| invalid("Wwise media range overflow"))?;
            return Ok(Some(data.get(start..end).ok_or_else(|| invalid("Wwise media outside DATA chunk"))?.to_vec()));
        }
    }
    package_media(media_id)
}

/// List event object IDs in the HIRC chunk for building an installed-bank index.
pub fn bank_event_ids(bank: &[u8]) -> io::Result<Vec<u32>> {
    let chunks = bank_chunks(bank)?;
    let Some(hierarchy) = chunks.get(b"HIRC") else { return Ok(Vec::new()) };
    let objects = bank_objects(hierarchy)?;
    let mut ids = objects.into_iter().filter_map(|((kind, id), _)| (kind == 4).then_some(id)).collect::<Vec<_>>();
    ids.sort_unstable();
    Ok(ids)
}

type BankChunks<'a> = std::collections::HashMap<[u8; 4], &'a [u8]>;
type BankObjects<'a> = std::collections::HashMap<(u8, u32), &'a [u8]>;

fn audio_node<'a>(objects: &BankObjects<'a>, id: u32) -> Option<(u8, &'a [u8])> {
    [2, 5, 6, 7, 9].into_iter().find_map(|kind| objects.get(&(kind, id)).map(|object| (kind, *object)))
}

fn bank_feedback(chunks: &BankChunks<'_>) -> io::Result<bool> {
    let header = chunks.get(b"BKHD").ok_or_else(|| invalid("Wwise bank has no BKHD"))?;
    let version = read_u32_slice(header, 0)?;
    if version != 113 { return Err(invalid_owned(format!("unsupported Wwise hierarchy version {version}"))); }
    Ok(read_u32_slice(header, 12)? & 1 != 0)
}

fn bank_chunks(bank: &[u8]) -> io::Result<BankChunks<'_>> {
    let mut chunks = BankChunks::new();
    let mut offset = 0usize;
    while offset.checked_add(8).is_some_and(|end| end <= bank.len()) {
        let tag: [u8; 4] = bank[offset..offset + 4].try_into().unwrap();
        let size = read_u32_slice(bank, offset + 4)? as usize;
        let start = offset + 8;
        let end = start.checked_add(size).ok_or_else(|| invalid("Wwise bank chunk size overflow"))?;
        let data = bank.get(start..end).ok_or_else(|| invalid("Wwise bank chunk outside entry"))?;
        if chunks.insert(tag, data).is_some() { return Err(invalid("duplicate Wwise bank chunk")); }
        offset = end;
    }
    if offset != bank.len() { return Err(invalid("trailing bytes after Wwise bank chunks")); }
    Ok(chunks)
}

fn bank_objects(hierarchy: &[u8]) -> io::Result<BankObjects<'_>> {
    let mut objects = BankObjects::new();
    let count = read_u32_slice(hierarchy, 0)? as usize;
    let mut cursor = 4usize;
    for _ in 0..count {
        let kind = *hierarchy.get(cursor).ok_or_else(|| invalid("truncated HIRC object"))?;
        let size = read_u32_slice(hierarchy, cursor + 1)? as usize;
        let start = cursor.checked_add(5).ok_or_else(|| invalid("HIRC object offset overflow"))?;
        let end = start.checked_add(size).ok_or_else(|| invalid("HIRC object size overflow"))?;
        let object = hierarchy.get(start..end).ok_or_else(|| invalid("HIRC object outside chunk"))?;
        if objects.insert((kind, read_u32_slice(object, 0)?), object).is_some() { return Err(invalid("duplicate Wwise hierarchy ID within one object type")); }
        cursor = end;
    }
    if cursor != hierarchy.len() { return Err(invalid("HIRC object count did not consume chunk")); }
    Ok(objects)
}

fn read_u32_slice(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let end = offset.checked_add(4).ok_or_else(|| invalid("Wwise field offset overflow"))?;
    let field = bytes.get(offset..end).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("truncated Wwise bank field at {offset:#x} (object size {:#x})", bytes.len())))?;
    Ok(u32::from_le_bytes(field.try_into().unwrap()))
}

#[derive(Clone, Copy)]
enum EntryTable {
    Standard,
    External,
}

fn parse_languages(bytes: &[u8]) -> io::Result<Vec<Language>> {
    let mut cursor = Cursor::new(bytes);
    let count = read_u32(&mut cursor)? as usize;
    let records_end = 4usize.checked_add(count.checked_mul(8).ok_or_else(|| invalid("Wwise language count overflow"))?)
        .ok_or_else(|| invalid("Wwise language table overflow"))?;
    if records_end > bytes.len() {
        return Err(invalid("truncated Wwise language table"));
    }

    let mut languages = Vec::with_capacity(count);
    for _ in 0..count {
        let name_offset = read_u32(&mut cursor)? as usize;
        let id = read_u32(&mut cursor)?;
        if name_offset >= bytes.len() {
            return Err(invalid("Wwise language name outside table"));
        }
        languages.push(Language { id, name: read_string(bytes, name_offset)? });
    }
    Ok(languages)
}

fn read_string(bytes: &[u8], offset: usize) -> io::Result<String> {
    let data = &bytes[offset..];
    if data.len() >= 2 && data[1] == 0 {
        let end = data.chunks_exact(2).position(|pair| pair == [0, 0])
            .ok_or_else(|| invalid("unterminated Wwise UTF-16 language"))? * 2;
        let units = data[..end].chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| invalid("invalid Wwise UTF-16 language"))
    } else {
        let end = data.iter().position(|byte| *byte == 0).unwrap_or(data.len());
        std::str::from_utf8(&data[..end]).map(str::to_owned).map_err(|_| invalid("invalid Wwise language"))
    }
}

fn parse_entries(bytes: &[u8], table: EntryTable) -> io::Result<Vec<Entry>> {
    let mut cursor = Cursor::new(bytes);
    let count = read_u32(&mut cursor)? as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let remaining = bytes.len().checked_sub(4).ok_or_else(|| invalid("truncated Wwise entry table"))?;
    if remaining % count != 0 {
        return Err(invalid("invalid Wwise entry table size"));
    }
    let entry_size = remaining / count;
    if entry_size != 20 && entry_size != 24 {
        return Err(invalid("unsupported Wwise entry size"));
    }

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let id = if matches!(table, EntryTable::External) && entry_size == 24 {
            read_u64(&mut cursor)?
        } else {
            read_u32(&mut cursor)? as u64
        };
        let block_size = read_u32(&mut cursor)?;
        let size = if entry_size == 24 && matches!(table, EntryTable::Standard) {
            read_u64(&mut cursor)?
        } else {
            read_u32(&mut cursor)? as u64
        };
        let offset = read_u32(&mut cursor)? as u64;
        let language_id = read_u32(&mut cursor)?;
        entries.push(Entry { id, language_id, offset: scaled_offset(offset, block_size)?, size });
    }
    Ok(entries)
}

fn scaled_offset(offset: u64, block_size: u32) -> io::Result<u64> {
    if block_size == 0 { Ok(offset) } else { offset.checked_mul(block_size as u64).ok_or_else(|| invalid("Wwise entry offset overflow")) }
}

fn read_section<R: Read + Seek>(reader: &mut R, size: u64, file_size: u64) -> io::Result<Vec<u8>> {
    let end = reader.stream_position()?.checked_add(size).ok_or_else(|| invalid("Wwise section size overflow"))?;
    if end > file_size {
        return Err(invalid("Wwise section outside package"));
    }
    let size = usize::try_from(size).map_err(|_| invalid("Wwise section too large for host"))?;
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_u32<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
