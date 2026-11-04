//! File-source acquisition: the RIFF format and loop records precede audio.
//! 1411606d0 reads the Vorbis frame count from these records; 141149320
//! supplies the shared loop rule. Source preparation must not decode packets.
use super::{invalid, loop_points, slice_u16, slice_u32, wem_source_metadata};
use std::io::{self, Read};

pub struct StreamFormat {
    pub codec: u16,
    pub data: std::ops::Range<usize>,
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: usize,
    pub loop_points: Option<[u32; 2]>,
}

/// Read through the data chunk header, leaving its payload in the package.
/// `source_size` bounds this package member, independently of the next member.
pub fn read_stream_header(reader: &mut impl Read, source_size: usize) -> io::Result<Vec<u8>> {
    if source_size < 12 { return Err(invalid("truncated Wwise stream header")); }
    let mut bytes = vec![0; 12];
    reader.read_exact(&mut bytes)?;
    if &bytes[..4] != b"RIFF" || &bytes[8..] != b"WAVE" {
        return Err(invalid("Wwise stream is not RIFF/WAVE"));
    }
    loop {
        let at = bytes.len();
        if at.checked_add(8).is_none_or(|end| end > source_size) {
            return Err(invalid("Wwise stream has no data chunk"));
        }
        bytes.resize(at + 8, 0);
        reader.read_exact(&mut bytes[at..])?;
        let size = slice_u32(&bytes, at + 4)? as usize;
        let end = (at + 8).checked_add(size).filter(|end| *end <= source_size)
            .ok_or_else(|| invalid("Wwise stream chunk exceeds its package member"))?;
        if &bytes[at..at + 4] == b"data" { return Ok(bytes); }
        let padded = end.checked_add(size & 1).filter(|end| *end <= source_size)
            .ok_or_else(|| invalid("Wwise stream padding exceeds its package member"))?;
        bytes.resize(padded, 0);
        reader.read_exact(&mut bytes[at + 8..])?;
    }
}

/// The same authored channel/rate/frame/loop information used by resident
/// decoders, without constructing a packet index or decoding audio.
pub fn stream_format(bytes: &[u8], source_size: usize) -> io::Result<Option<StreamFormat>> {
    let (mut fmt, mut data) = (None, None);
    let mut cursor = 12usize;
    while cursor.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let size = slice_u32(bytes, cursor + 4)? as usize;
        let start = cursor + 8;
        let end = start.checked_add(size).filter(|end| *end <= source_size)
            .ok_or_else(|| invalid("Wwise stream chunk exceeds source size"))?;
        match &bytes[cursor..cursor + 4] {
            b"fmt " => fmt = Some(bytes.get(start..end).ok_or_else(|| invalid("truncated Wwise stream format"))?),
            b"data" => { data = Some(start..end); break; },
            _ => {},
        }
        cursor = end + (size & 1);
    }
    let fmt = fmt.ok_or_else(|| invalid("Wwise stream has no format"))?;
    let data = data.ok_or_else(|| invalid("Wwise stream has no data"))?;
    let codec = slice_u16(fmt, 0)?;
    if !matches!(codec, 2 | 0xffff) { return Ok(None); }
    let channels = slice_u16(fmt, 2)?;
    let sample_rate = slice_u32(fmt, 4)?;
    if channels == 0 || sample_rate == 0 { return Err(invalid("invalid Wwise stream sample format")); }
    let (frames, points) = wem_source_metadata(bytes)?;
    let frames = if codec == 2 {
        let block = 0x24 * usize::from(channels);
        if channels > 8 || slice_u16(fmt, 14)? != 4 || usize::from(slice_u16(fmt, 12)?) != block || data.len() % block != 0 {
            return Err(invalid("invalid Wwise streamed IMA block layout"));
        }
        data.len() / block * 64
    } else {
        let Some(frames) = frames else { return Ok(None); };
        frames as usize
    };
    Ok(Some(StreamFormat { codec, data, channels, sample_rate, frames, loop_points: loop_points(points, frames)? }))
}
