//! Music-node meter and segment synchronization (141154250, 141154e10).
use super::super::music_clip::rounded_sample;

#[derive(Clone, Copy, Debug, Default)]
pub struct Meter {
    pub beat: u32,
    pub bar: u32,
    pub grid: u32,
    pub grid_offset: u32,
}
impl Meter {
    /// 141154620 admits a local meter only when serialized byte 22 is set;
    /// otherwise 141153f60 walks music parents to the nearest local meter.
    pub fn read(bytes: &[u8], rate: u32) -> Result<Option<Self>, String> {
        if bytes.len() != 23 { return Err("invalid music meter length".into()); }
        if bytes[22] == 0 { return Ok(None); }
        let tempo = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let beat = rounded_sample((4.0 / f64::from(bytes[21])) * (60.0 / f64::from(tempo)) * f64::from(rate)) as u32;
        Ok(Some(Self {
            beat,
            bar: u32::from(bytes[20]).wrapping_mul(beat),
            grid: rounded_sample(f64::from(rate) * f64::from_le_bytes(bytes[..8].try_into().unwrap()) * 0.001) as u32,
            grid_offset: rounded_sample(f64::from(rate) * f64::from_le_bytes(bytes[8..16].try_into().unwrap()) * 0.001) as u32,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct Cue { pub id: u32, pub position: i32 }

/// Positions are relative to the first (entry) marker, not the media start.
/// Markers remain in serialized order, as in 1411558e0.
#[derive(Clone, Debug)]
pub struct Segment {
    pub duration: i32,
    pub cues: Vec<Cue>,
}
impl Segment {
    pub fn entry(&self) -> i32 { self.cues.first().map_or(0, |cue| cue.position) }
    pub fn exit(&self) -> i32 { self.cues.last().map_or(0, |cue| cue.position.wrapping_sub(self.entry())) }
    /// Destination marker choice (141154d80 / 1411552b0). Entry modes two
    /// and three choose matching markers before the exit, including/excluding
    /// the entry respectively. The no-match fallback is the entry marker.
    pub fn destination(&self, kind: u16, minimum: u32, filter: u32, mut random15: impl FnMut() -> u16) -> (u32, u32) {
        if kind == 0 && minimum == 0 { return (self.cues.first().map_or(0, |cue| cue.id), 0); }
        if !matches!(kind, 2 | 3) { return (0, minimum); }
        let end = self.cues.len().saturating_sub(1);
        let begin = usize::from(kind == 3);
        let first = (begin..end).find(|&index| self.cues[index].position as u32 >= (self.entry() as u32).wrapping_add(minimum)).unwrap_or(end);
        let count = (first..end).filter(|&index| filter == 0 || index == 0 || self.cues[index].id == filter).count();
        if count == 0 { return (self.cues.first().map_or(0, |cue| cue.id), 0); }
        let selected = usize::from(random15()) % count;
        let index = (first..end).filter(|&index| filter == 0 || index == 0 || self.cues[index].id == filter).nth(selected).unwrap();
        (self.cues[index].id, self.cues[index].position.wrapping_sub(self.entry()) as u32)
    }
    /// 141154e10: 0 immediate, 1 grid, 2 bar, 3 beat, 4 marker,
    /// 5 user marker, 6 entry marker, 7 exit marker. None is source result 2.
    pub fn sync(&self, position: u32, kind: u32, cue_filter: u32, skip_entry: bool, meter: Meter) -> Option<(u32, u32)> {
        let exit = self.exit() as u32;
        if position > exit { return None; }
        let periodic = |period: u32, offset: u32| {
            let target = position.wrapping_add(u32::from(skip_entry));
            let value = if period == 0 { target.wrapping_add(1) }
                else if offset >= target { offset }
                else { offset.wrapping_add(target.wrapping_sub(offset).div_ceil(period).wrapping_mul(period)) };
            (value <= exit).then_some((0, value))
        };
        match kind {
            0 => Some((0, position)),
            1 => periodic(meter.grid, meter.grid_offset),
            2 => periodic(meter.bar, 0),
            3 => periodic(meter.beat, 0),
            4 => { // 141155050: no match falls back to the exit cue.
                let target = (self.entry() as u32).wrapping_add(position);
                self.cues.iter().skip(usize::from(skip_entry)).find(|cue|
                    cue.position as u32 >= target && (cue_filter == 0 || cue.id == cue_filter))
                    .or_else(|| self.cues.last()).map(|cue| (cue.id, cue.position.wrapping_sub(self.entry()) as u32))
            },
            5 => { // 1411550e0 excludes entry and exit and compares strictly.
                let target = (self.entry() as u32).wrapping_add(position);
                self.cues.get(1..self.cues.len().saturating_sub(1)).unwrap_or_default().iter()
                    .find(|cue| cue.position as u32 > target && (cue_filter == 0 || cue.id == cue_filter))
                    .map(|cue| (cue.id, cue.position.wrapping_sub(self.entry()) as u32))
            },
            6 if position == 0 && !skip_entry => self.cues.first().map(|cue| (cue.id, 0)),
            7 => self.cues.last().map(|cue| (cue.id, exit)),
            _ => None,
        }
    }
}
