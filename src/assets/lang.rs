//! Localized string tables (`strings/<language>_m.lang`, idLangDict).
//!
//! The game builds the path as `"%s/%s%s.%s"` from `strings`, the language
//! name, `_m` and `lang` (`FUN_1400a3ed0`) and reads it with
//! `FUN_140164a30`. Keys compare case-insensitively over ASCII
//! (`FUN_140165560` hashes with `FUN_140145960`, then compares with
//! `FUN_1401435f0`).
use super::archive::Catalog;
use std::collections::HashMap;
use std::path::Path;

/// `arkLangSystem` language descriptions (`FUN_141357160`), indexed by
/// `eLanguagesID_t` (`FUN_141a01920`: `eLID_English` 0 .. `eLID_Japanese` 10).
pub const LANGUAGES: [(&str, &str); 11] = [
    ("english", "eLID_English"),
    ("french", "eLID_French"),
    ("italian", "eLID_Italian"),
    ("german", "eLID_German"),
    ("spanish", "eLID_Spanish"),
    ("russian", "eLID_Russian"),
    ("polish", "eLID_Polish"),
    ("mexican", "eLID_MexicanSpanish"),
    ("brazilian", "eLID_BrazilianPortuguese"),
    ("chinese", "eLID_Chinese"),
    ("japanese", "eLID_Japanese"),
];
const NOT_SET: u32 = 0xffff;
const SPANISH: u32 = 4;
const MEXICAN: u32 = 7;

/// The languages an installation ships (`base/SKUConfig.txt`, read by
/// `FUN_1400a4640`): a `TEXT:` and an `AUDIO:` section of enum names. The
/// first entry of each is the SKU default (`FUN_1400a4500`).
#[derive(Clone, Debug, PartialEq)]
pub struct Sku {
    pub text: Vec<u32>,
    pub audio: Vec<u32>,
}

impl Sku {
    pub fn parse(source: &str) -> Result<Self, String> {
        let (mut text, mut audio) = (Vec::new(), Vec::new());
        let mut section = None;
        for line in source.lines() {
            let line = line.trim();
            if line.starts_with("TEXT:") {
                section = Some(0);
            } else if line.starts_with("AUDIO:") {
                section = Some(1);
            } else if !line.is_empty() {
                let id = LANGUAGES
                    .iter()
                    .position(|(_, e)| e.eq_ignore_ascii_case(line))
                    .ok_or_else(|| format!("Malformed SKU Config file: {line}"))? as u32;
                match section {
                    Some(0) => text.push(id),
                    Some(_) => audio.push(id),
                    None => return Err(format!("Malformed SKU Config file: {line} outside a section")),
                }
            }
        }
        if text.is_empty() || audio.is_empty() {
            return Err("Could not read SKU Config file: empty language list".into());
        }
        Ok(Sku { text, audio })
    }

    pub fn read(base: &Path) -> Result<Self, String> {
        let path = base.join("SKUConfig.txt");
        let source = std::fs::read_to_string(&path)
            .map_err(|e| format!("Could not read SKU Config file {}: {e}", path.display()))?;
        Self::parse(&source)
    }

    /// `+0x160`: a text list of exactly English and Chinese ignores Steam's
    /// language and starts from English.
    fn ignores_steam(&self) -> bool {
        matches!(self.text.as_slice(), [0, 9] | [9, 0])
    }
}

/// `FUN_1400e45d0`: Steam's language name to a language id; anything else
/// (and no Steam) is English. Steam `spanish` selects Mexican Spanish.
pub fn steam_language_id(steam: Option<&str>) -> u32 {
    let Some(steam) = steam else { return 0 };
    let is = |name: &str| steam.eq_ignore_ascii_case(name);
    if is("german") {
        3
    } else if is("french") {
        1
    } else if is("italian") {
        2
    } else if is("spanish") {
        MEXICAN
    } else if is("tchinese") || is("schinese") {
        9
    } else if is("russian") {
        5
    } else if is("japanese") {
        10
    } else if is("polish") {
        6
    } else if is("brazilian") || is("portuguese") {
        8
    } else {
        0
    }
}

/// The text language `arkLangSystem` starts with (`FUN_1400a3da0`): the
/// saved cvar, else Steam's language; with Steam Mexican Spanish the text is
/// Spain's when the audio language is Spanish and Mexican otherwise; a
/// language the SKU does not ship falls back to the SKU default
/// (`FUN_1400a38e0`).
pub fn text_language(sku: &Sku, steam: Option<&str>, saved_text: u32, saved_audio: u32) -> &'static str {
    let steam = steam_language_id(if sku.ignores_steam() { Some("english") } else { steam });
    let audio = if saved_audio != NOT_SET { saved_audio } else { steam };
    let mut text = if saved_text != NOT_SET { saved_text } else { steam };
    if steam == MEXICAN {
        text = if audio == SPANISH { SPANISH } else { MEXICAN };
    }
    let text = if sku.text.contains(&text) { text } else { sku.text[0] };
    LANGUAGES[text as usize].0
}

/// The installed game's text language, from the files the game reads.
pub fn read_text_language(base: &Path) -> Result<&'static str, String> {
    let sku = Sku::read(base)?;
    let steam = super::installation::steam_language(base).ok();
    let (text, audio) = crate::platform::game_settings::read_saved_languages()?;
    Ok(text_language(&sku, steam.as_deref(), text, audio))
}

#[derive(Clone, Debug, Default)]
pub struct StringTable {
    entries: HashMap<String, String>,
}

/// `FUN_140145e50`: one UTF-8 sequence, lead-byte length table at
/// 0x141af2010 and masks at 0x141af1ff0, continuation bytes unchecked; in an
/// ASCII file a byte above 127 is fatal.
fn read_char(bytes: &[u8], pos: &mut usize, utf8: bool, line: usize) -> Result<u32, String> {
    let lead = bytes[*pos];
    *pos += 1;
    if !utf8 {
        if lead >= 0x80 {
            return Err(format!("Language file is supposed to be plain ASCII, but has byte values > 127 (line {line})"));
        }
        return Ok(lead as u32);
    }
    if lead == 0 {
        return Ok(0);
    }
    const MASKS: [u32; 6] = [0x7f, 0x1f, 0x0f, 0x07, 0x03, 0x01];
    let extra = match lead {
        0..=0xbf => 0,
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        0xf8..=0xfb => 4,
        _ => 5,
    };
    let mut ch = lead as u32 & MASKS[extra];
    for _ in 0..extra {
        let next = bytes.get(*pos).copied().unwrap_or(0);
        *pos += 1;
        ch = (ch << 6) + (next as u32 & 0x3f);
    }
    Ok(ch)
}

/// `FUN_1401652c0`: FNV-1a over the lower-cased, sign-extended bytes; 0 is 1.
fn key_hash(lower: &str) -> u32 {
    let hash = lower.bytes().fold(0x811c9dc5u32, |h, b| (h ^ (b as i8 as i32 as u32)).wrapping_mul(0x0100_0193));
    if hash == 0 { 1 } else { hash }
}

impl StringTable {
    /// `FUN_140164a30`. Outside quotes, `/` skips to the end of the line,
    /// `}` ends the table and other bytes are ignored. A key runs to the next
    /// quote; its value starts at the following quote and runs to the end of
    /// the line, keeping the characters before its last closing quote, with
    /// `\n`, `\t`, `\"` and `\\` escapes. A UTF-8 BOM selects UTF-8 decoding;
    /// otherwise the file must be plain ASCII. Keys whose hashes collide are
    /// fatal ("Computed a duplicate ID").
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let utf8 = bytes.len() > 3 && bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let len = bytes.len();
        let mut pos = 0;
        let mut line = 1;
        let mut entries = HashMap::new();
        let mut hashes = HashMap::new();
        while pos < len {
            let c = bytes[pos];
            pos += 1;
            match c {
                b'/' => {
                    while pos < len {
                        pos += 1;
                        if bytes[pos - 1] == b'\n' {
                            line += 1;
                            break;
                        }
                    }
                }
                b'}' => break,
                b'\n' => line += 1,
                b'"' => {
                    let key_start = pos;
                    let key_end = bytes[pos..]
                        .iter()
                        .position(|&b| b == b'"')
                        .map(|i| pos + i)
                        .ok_or_else(|| format!("File ended while reading key at line {line}"))?;
                    pos = key_end + 1;
                    let key = String::from_utf8_lossy(&bytes[key_start..key_end]).into_owned();
                    let open = bytes[pos..]
                        .iter()
                        .position(|&b| b == b'"')
                        .ok_or_else(|| format!("File ended while reading value at line {line}"))?;
                    pos += open + 1;
                    let mut closed = false;
                    let mut value = String::new();
                    while pos < len {
                        let mut ch = read_char(bytes, &mut pos, utf8, line)?;
                        match ch {
                            0x22 => {
                                closed = true;
                                continue;
                            }
                            0x0a => {
                                line += 1;
                                break;
                            }
                            0x0d => continue,
                            0x5c => {
                                // An escape at the end of the file reads a NUL.
                                ch = if pos < len { read_char(bytes, &mut pos, utf8, line)? } else { 0 };
                                ch = match ch {
                                    0x6e => 0x0a,
                                    0x74 => 0x09,
                                    // `"` and `\` are literal; any other character is
                                    // reported ("Unknown escape sequence") and kept.
                                    other => other,
                                };
                            }
                            _ => {}
                        }
                        // Characters after the closing quote are reported and dropped.
                        if !closed {
                            // `FUN_140140d00` encodes up to 0x1fffff; Rust strings cannot
                            // hold surrogates or values above U+10FFFF.
                            value.push(if ch >= 0x20_0000 { '?' } else { char::from_u32(ch).unwrap_or('\u{fffd}') });
                        }
                    }
                    if !closed {
                        return Err(format!("File ended while reading value at line {line}"));
                    }
                    let lower = key.to_ascii_lowercase();
                    if let Some(previous) = hashes.insert(key_hash(&lower), key.clone()) {
                        return Err(format!("Computed a duplicate ID for strings {previous} and {key}"));
                    }
                    entries.insert(lower, value);
                }
                _ => {}
            }
        }
        Ok(StringTable { entries })
    }

    pub fn load(catalog: &Catalog, language: &str) -> Result<Self, String> {
        let path = format!("strings/{language}_m.lang");
        let bytes = catalog
            .read(&path)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Unable to load language file {path}!"))?;
        Self::parse(&bytes)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(&key.to_ascii_lowercase()).map(String::as_str)
    }

    /// `FUN_140165420`: the entry for `key`, or `key` itself.
    pub fn lookup<'a>(&'a self, key: &'a str) -> &'a str {
        self.get(key).unwrap_or(key)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The language of the localized UI libraries for a text language
/// (`FUN_14002fae0`): the languages listed at 0x142273e68 (French, Italian,
/// German, Spanish, Mexican Spanish, Brazilian Portuguese, Polish) use
/// English.
pub fn library_language(language: &str) -> &str {
    const ENGLISH_LIBRARIES: [u32; 7] = [1, 2, 3, 4, 7, 8, 6];
    match LANGUAGES.iter().position(|(name, _)| *name == language) {
        Some(id) if ENGLISH_LIBRARIES.contains(&(id as u32)) => LANGUAGES[0].0,
        _ => language,
    }
}

/// UI library resource for a library name: `lib_loc_flash_*` names use the
/// text language's localized file (`FUN_14002e7a0` replaces `flash` with
/// the `library_language` string).
pub fn library_resource(name: &str, language: &str) -> String {
    let stem = name.strip_suffix(".swf").unwrap_or(name);
    let stem = match stem.strip_prefix("lib_loc_flash") {
        Some(rest) => format!("lib_loc_{}{rest}", library_language(language)),
        None => stem.to_string(),
    };
    format!("ui/{}.iggy", stem.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_pairs_and_comments() {
        let t = StringTable::parse("\u{feff}// string table\n//\n\n{\n\t\"#number_0\"\t\"0\"\n\t\"UI_PauseGame\"\t\"Pause\"\n}\n".as_bytes()).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t.get("ui_pausegame"), Some("Pause"));
        assert_eq!(t.lookup("missing"), "missing");
    }

    #[test]
    fn localized_libraries_use_the_language() {
        assert_eq!(library_resource("lib_loc_flash_font.swf", "russian"), "ui/lib_loc_russian_font.iggy");
        assert_eq!(library_resource("lib_loc_flash_pause3D.swf", "russian"), "ui/lib_loc_russian_pause3d.iggy");
        assert_eq!(library_resource("lib_main.swf", "russian"), "ui/lib_main.iggy");
        assert_eq!(library_resource("lib_loc_flash_font.swf", "polish"), "ui/lib_loc_english_font.iggy");
        assert_eq!(library_resource("lib_loc_flash_font.swf", "japanese"), "ui/lib_loc_japanese_font.iggy");
    }

    #[test]
    fn text_language_follows_the_game_rules() {
        let all: String = LANGUAGES.iter().map(|(_, e)| format!("{e}\n")).collect();
        let sku = Sku::parse(&format!("TEXT:11\n{all}AUDIO:2\neLID_English\neLID_Spanish\n")).unwrap();
        assert_eq!(sku.text.len(), 11);
        assert_eq!(sku.audio, vec![0, 4]);
        assert_eq!(text_language(&sku, Some("russian"), NOT_SET, NOT_SET), "russian");
        assert_eq!(text_language(&sku, Some("schinese"), NOT_SET, NOT_SET), "chinese");
        assert_eq!(text_language(&sku, Some("portuguese"), NOT_SET, NOT_SET), "brazilian");
        assert_eq!(text_language(&sku, Some("koreana"), NOT_SET, NOT_SET), "english");
        assert_eq!(text_language(&sku, None, NOT_SET, NOT_SET), "english");
        assert_eq!(text_language(&sku, Some("spanish"), NOT_SET, NOT_SET), "mexican");
        assert_eq!(text_language(&sku, Some("spanish"), 5, SPANISH), "spanish");
        assert_eq!(text_language(&sku, Some("russian"), 1, NOT_SET), "french");
        let small = Sku::parse("TEXT:2\neLID_English\neLID_Chinese\nAUDIO:1\neLID_English\n").unwrap();
        assert_eq!(text_language(&small, Some("russian"), NOT_SET, NOT_SET), "english");
        assert_eq!(text_language(&small, Some("russian"), 5, NOT_SET), "english");
    }

    #[test]
    fn parser_follows_the_game_reader() {
        let t = StringTable::parse(
            "\u{feff}stray {\n\"a\" x \"one \\\"two\\\" \\n\"  trailing\r\n/ comment \"skipped\" \"x\"\n\"B\"\t\"\u{0436}\"\n}\n\"after\" \"end\"\n".as_bytes(),
        )
        .unwrap();
        assert_eq!(t.get("a"), Some("one \"two\" \n"));
        assert_eq!(t.get("b"), Some("\u{0436}"));
        assert_eq!(t.get("after"), None);
        assert_eq!(t.len(), 2);
        assert!(StringTable::parse(b"\"a\" \"1\"\n\"A\" \"2\"\n").is_err());
        assert!(StringTable::parse(b"\"a\" \"unterminated\n").is_err());
        assert!(StringTable::parse("\"a\" \"\u{0436}\"".as_bytes()).is_err());
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn every_installed_language_table_parses() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let paths = catalog.paths_with_suffix(".lang");
        assert!(!paths.is_empty());
        let failures: Vec<_> = paths
            .iter()
            .filter_map(|path| StringTable::parse(&catalog.read(path).unwrap().unwrap()).err().map(|e| format!("{path}: {e}")))
            .collect();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_russian_table_has_pause_strings() {
        let catalog = Catalog::open(&super::super::installation::game_base().unwrap()).unwrap();
        let t = StringTable::load(&catalog, "russian").unwrap();
        assert!(t.len() > 6000);
        assert!(t.get("ui_pause_restart").is_some());
    }
}
