//! Save headers on disk and the pause menu's save/load lists
//! (`artifacts/ui-pause/notes-saves.md`).
use crate::assets::lang::StringTable;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `0x444d0003`: `gameVersion` of the full game; the trial writes `0x4445009f`.
pub const TRIAL_VERSION: i32 = 0x4445009f;

/// A `game.details` header (`FUN_1400e5150`, parsed by `FUN_1400d64e0`).
#[derive(Clone, Debug)]
pub struct Header {
    /// Directory name, e.g. `SLOT01` (+0x50).
    pub name: String,
    /// Last write time of `game.details` in Unix seconds (+0x48).
    pub time: i64,
    /// Key/value dictionary, cleared when corrupted.
    pub values: BTreeMap<String, String>,
    /// Flags bit0.
    pub corrupted: bool,
}

impl Header {
    /// `strtol(value, 10)` as a 32-bit signed value; missing keys read 0.
    pub fn int(&self, key: &str) -> i32 {
        self.values.get(key).map_or(0, |v| strtol(v))
    }

    /// Last two characters of the directory name (`FUN_140658950`).
    pub fn slot(&self) -> i32 {
        if self.name.len() <= 2 {
            return -1;
        }
        strtol(&self.name[self.name.len() - 2..])
    }

    /// `FUN_140657a10`: `slotType`, else from `slot % 30`.
    pub fn header_type(&self) -> i32 {
        match self.int("slotType") {
            0 => match self.slot() % 30 {
                0 => 5,
                1 | 2 => 3,
                3 => 1,
                _ => 2,
            },
            t => t,
        }
    }

    /// `FUN_140657580`: mission number of a mission-start slot.
    pub fn mission_start(&self) -> Option<i32> {
        let m = self.slot() % 30;
        (21..=29).contains(&m).then_some(m - 20)
    }
}

/// C `strtol` base 10, saturated to 32 bits.
fn strtol(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
    let v: i64 = digits.parse::<i64>().unwrap_or(if digits.is_empty() { 0 } else { i64::MAX });
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `FUN_140104670`: `key = rest of line` with bytes below `'!'` trimmed and
/// optional `[section]` prefixes. Returns `None` on a parse failure.
fn parse_details(text: &[u8]) -> Option<BTreeMap<String, String>> {
    let trim = |s: &[u8]| -> Vec<u8> {
        let blank = |b: &u8| (*b as i8) < b'!' as i8;
        let start = s.iter().position(|b| !blank(b)).unwrap_or(s.len());
        let end = s.iter().rposition(|b| !blank(b)).map_or(start, |e| e + 1);
        s[start..end].to_vec()
    };
    let mut prefix = String::new();
    let mut out = BTreeMap::new();
    for line in text.split(|&b| b == b'\n') {
        let line = trim(line);
        if line.is_empty() {
            continue;
        }
        if line[0] == b'[' {
            let end = line.iter().position(|&b| b == b']')?;
            prefix = format!("{}/", String::from_utf8_lossy(&line[1..end]));
            continue;
        }
        let eq = line.iter().position(|&b| b == b'=')?;
        let key = String::from_utf8_lossy(&trim(&line[..eq])).into_owned();
        let value = String::from_utf8_lossy(&trim(&line[eq + 1..])).into_owned();
        if out.insert(format!("{prefix}{key}"), value).is_some() {
            return None;
        }
    }
    Some(out)
}

/// `FUN_140103ee0`: CRC-32 over key-sorted `key + value` bytes without the
/// checksum keys.
fn checksum(values: &BTreeMap<String, String>) -> u32 {
    let mut crc = Crc32::default();
    for (k, v) in values.iter().filter(|(k, _)| *k != "fileChecksum" && *k != "checksum") {
        crc.update(k.as_bytes());
        crc.update(v.as_bytes());
    }
    crc.finish()
}

struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Crc32(0xffff_ffff)
    }
}

impl Crc32 {
    fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u32;
            for _ in 0..8 {
                self.0 = if self.0 & 1 != 0 { (self.0 >> 1) ^ 0xedb8_8320 } else { self.0 >> 1 };
            }
        }
    }
    fn finish(&self) -> u32 {
        !self.0
    }
}

/// `FUN_1400e5150`: every `game.details` below the save folder.
pub fn read_headers(savegame: &Path) -> Vec<Header> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(savegame) else { return out };
    for dir in dirs.flatten() {
        let path = dir.path().join("game.details");
        let Ok(text) = std::fs::read(&path) else { continue };
        let time = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let parsed = parse_details(&text);
        let corrupted = match &parsed {
            None => true,
            Some(v) => {
                let stored = v.get("fileChecksum").or_else(|| v.get("checksum")).map_or(0, |s| strtol(s));
                stored == 0 || checksum(v) != stored as u32
            }
        };
        let name = dir.file_name().to_string_lossy().into_owned();
        let values = if corrupted { BTreeMap::new() } else { parsed.unwrap_or_default() };
        out.push(Header { name, time, values, corrupted });
    }
    out
}

/// The profile dictionary (`savegame/profile.bin`): NUL-terminated key and
/// value strings after a u32 pair count at 0x1a. The loader is not traced;
/// the count is checked against the pairs read.
pub fn read_profile(savegame: &Path) -> Option<BTreeMap<String, String>> {
    let bytes = std::fs::read(savegame.join("profile.bin")).ok()?;
    let count = u32::from_le_bytes(bytes.get(0x1a..0x1e)?.try_into().ok()?) as usize;
    let mut parts = bytes.get(0x1e..)?.split(|&b| b == 0);
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let key = String::from_utf8_lossy(parts.next()?).into_owned();
        let value = String::from_utf8_lossy(parts.next()?).into_owned();
        out.insert(key, value);
    }
    Some(out)
}

/// The save system's list (`FUN_140658950`): headers of the profile's
/// campaign, newest first, at most 30.
pub fn system_list(headers: Vec<Header>, campaign: i32) -> Vec<Header> {
    let mut list: Vec<Header> = Vec::new();
    for h in headers {
        if list.len() == 30 {
            break;
        }
        let slot = h.slot();
        let c = if slot < 0 { -1 } else { slot / 30 };
        if c != campaign {
            continue;
        }
        let at = list.iter().position(|e| e.time < h.time).unwrap_or(list.len());
        list.insert(at, h);
    }
    list
}

/// `sys+0x18`: the slot a new manual save uses, by `campaignSaveType`.
pub fn next_free_slot(list: &[Header], campaign: i32, save_type: i32) -> i32 {
    let used = |s: i32| list.iter().any(|h| h.slot() == s);
    match save_type {
        0 => (campaign * 30 + 4..=campaign * 30 + 20).find(|&s| !used(s)).unwrap_or(-1),
        1 if !used(campaign * 30 + 3) => campaign * 30 + 3,
        _ => -1,
    }
}

/// One `savesData.*Data.data[i]` element (`FUN_1404657c0`), as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub mission_name: String,
    pub mission_index: i32,
    pub level_chaos: i32,
    pub character_name: String,
    pub save_date: String,
    pub kind: i32,
    pub slot: i32,
    pub difficulty: String,
    pub playtime: String,
    pub location: String,
    pub corrupted: bool,
    pub mission_image: Option<String>,
    /// `gameVersion == TRIAL_VERSION` (entry +0x169).
    pub trial_save: bool,
}

/// `m_chaptersInformation` of `ui/gamelogic/mainmenu/sections/campaign.mainmenusectioncampaign`.
const CHAPTERS: [(i32, &str, &str); 9] = [
    (1, "Mission_DW", "dw"),
    (2, "Mission_EotW", "eotw"),
    (3, "Mission_IS", "is"),
    (4, "Mission_MI", "mi"),
    (5, "Mission_CU", "cu"),
    (6, "Mission_DD", "dd"),
    (7, "Mission_BK", "bk"),
    (8, "Mission_PA", "pa"),
    (9, "Mission_DTTE", "dtte"),
];

/// `arkPlayer::m_characterNames` from `models/characters/player/base/body.def`.
const CHARACTER_NAMES: [Option<&str>; 3] = [None, Some("ui_charactername_corvo"), Some("ui_charactername_emily")];

fn text(strings: &StringTable, id: u32) -> String {
    strings.text(id).to_string()
}

/// `FUN_140154990(t, pad 1, " - ")`: local time; English `MM/DD/YYYY -
/// hh:mm:ssam`, every other language `DD/MM/YYYY - HH:MM:SS`.
pub fn format_date(time: i64, english: bool) -> String {
    let tm = crate::platform::local_time::local_time(time);
    let (a, b) = if english { (tm.month, tm.day) } else { (tm.day, tm.month) };
    let mut s = format!("{a:02}/{b:02}/{} - ", tm.year);
    if english {
        let hour = match tm.hour {
            0 => "12".to_string(),
            h if h > 12 => format!("{:02}", h - 12),
            h => format!("{h:02}"),
        };
        s += &format!("{hour}:{:02}:{:02}{}", tm.minute, tm.second, if tm.hour < 12 { "am" } else { "pm" });
    } else {
        s += &format!("{:02}:{:02}:{:02}", tm.hour, tm.minute, tm.second);
    }
    s
}

/// `FUN_14080cf50`: hours (not wrapped), minutes and seconds with the
/// localized postfixes.
pub fn format_playtime(strings: &StringTable, seconds: i32) -> String {
    let s = (seconds as f32) as i32 as u32;
    let post = |k: &str| strings.lookup(k).to_string();
    format!(
        "{:02}{} {:02}{} {:02}{}",
        s / 3600,
        post("ark_gamelogic_results_hourspostfix"),
        (s / 60) % 60,
        post("ark_gamelogic_results_minutespostfix"),
        s % 60,
        post("ark_gamelogic_results_secondspostfix")
    )
}

/// `FUN_140467010`: the helper list (types 4 and 5 left out), converted by
/// `FUN_1404657c0`.
pub fn helper_entries(list: &[Header], strings: &StringTable, english: bool) -> Vec<Entry> {
    let id = |k: &str| crate::assets::lang::key_id(k);
    list.iter()
        .filter(|h| !matches!(h.header_type(), 4 | 5))
        .map(|h| {
            let mission_start = h.mission_start();
            let kind = match h.header_type() {
                1 => 1,
                2 => {
                    if mission_start.is_some() {
                        4
                    } else {
                        0
                    }
                }
                3 | 5 => 2,
                _ => -1,
            };
            let chapter = h.int("chapter");
            let (mission_name, mission_image) = match mission_start {
                None => {
                    let image = if h.values.contains_key("screenshot") { format!("_savegame{}", h.name) } else { "dummysavegamescreenshot".into() };
                    (text(strings, h.int("mission") as u32), Some(image))
                }
                Some(_) => match CHAPTERS.iter().find(|c| c.0 == chapter) {
                    Some((_, key, image)) => (text(strings, id(key)), Some(format!("models/guis/diary_chapters/diaryimage_{image}_is.png"))),
                    None => (text(strings, 0), None),
                },
            };
            let character = h.int("character");
            let character_name = CHARACTER_NAMES
                .get(character as usize)
                .copied()
                .flatten()
                .map_or_else(|| text(strings, 0), |k| text(strings, id(k)));
            Entry {
                mission_name,
                mission_index: chapter,
                level_chaos: (h.int("chaosLevel") != 0) as i32,
                character_name,
                save_date: format_date(h.time, english),
                kind,
                slot: h.slot(),
                difficulty: text(strings, h.int("difficulty") as u32),
                playtime: format_playtime(strings, h.int("time")),
                location: text(strings, h.int("mapDesc") as u32),
                corrupted: h.corrupted,
                mission_image,
                trial_save: h.int("gameVersion") == TRIAL_VERSION,
            }
        })
        .collect()
}

/// The save folder and profile values the lists use.
pub struct Saves {
    pub folder: PathBuf,
    pub campaign: i32,
    pub save_type: i32,
}

impl Saves {
    /// The rewrite's `saves/` (the original's `savegame` folder layout).
    pub fn locate() -> Option<Saves> {
        let folder = crate::platform::home::home().saves_dir();
        let profile = read_profile(&folder).unwrap_or_default();
        let int = |k: &str| profile.get(k).map_or(0, |v| strtol(v));
        let campaign = int("campaignIndex");
        let save_type = int(&format!("campaignSaveType_{campaign:02}"));
        Some(Saves { folder, campaign, save_type })
    }

    pub fn profile_int(&self, key: &str) -> i32 {
        read_profile(&self.folder).and_then(|p| p.get(key).map(|v| strtol(v))).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_parse_and_checksum() {
        let mut v = parse_details(b"chapter=5\nmapName=campaign/x\n").unwrap();
        let sum = checksum(&v);
        v.insert("fileChecksum".into(), (sum as i32).to_string());
        assert_eq!(checksum(&v), sum);
        assert!(parse_details(b"a=1\na=2\n").is_none());
        let h = Header { name: "SLOT23".into(), time: 0, values: BTreeMap::new(), corrupted: false };
        assert_eq!((h.slot(), h.header_type(), h.mission_start()), (23, 2, Some(3)));
    }

    #[test]
    #[ignore = "requires the user's saves"]
    fn installed_saves_convert() {
        let saves = Saves::locate().unwrap();
        let list = system_list(read_headers(&saves.folder), saves.campaign);
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
        let strings = StringTable::load(&catalog, "english").unwrap();
        for e in helper_entries(&list, &strings, true) {
            println!("{e:?}");
            assert!(!e.corrupted, "{e:?}");
        }
    }
}
