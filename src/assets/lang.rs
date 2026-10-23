//! Localized string tables (`strings/<language>_m.lang`, idLangDict).
//!
//! The game builds the path as `"%s/%s%s.%s"` from `strings`, the language
//! name, `_m` and `lang` (`FUN_1400a3ed0`). Files are a brace-enclosed list
//! of quoted key/value pairs with `//` comments, UTF-8 with a BOM. Keys
//! compare case-insensitively over ASCII (`FUN_140165560` hashes with
//! `FUN_140145960`, then compares with `FUN_1401435f0`).
use super::archive::Catalog;
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct StringTable {
    entries: HashMap<String, String>,
}

impl StringTable {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(bytes).map_err(|e| format!("language file is not UTF-8: {e}"))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut tokens = Vec::new();
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '/' if text[i..].starts_with("//") => {
                    for (_, c) in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                '"' => {
                    let start = i + 1;
                    let mut end = None;
                    for (j, c) in chars.by_ref() {
                        if c == '"' {
                            end = Some(j);
                            break;
                        }
                    }
                    let end = end.ok_or("unterminated string in language file")?;
                    tokens.push(&text[start..end]);
                }
                '{' | '}' => {}
                c if c.is_whitespace() => {}
                c => return Err(format!("unexpected {c:?} in language file")),
            }
        }
        if tokens.len() % 2 != 0 {
            return Err("language file has a key without a value".into());
        }
        let entries = tokens.chunks_exact(2).map(|p| (p[0].to_ascii_lowercase(), p[1].to_string())).collect();
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

/// UI library resource for a library name: `lib_loc_flash_*` names use the
/// language's localized file (`FUN_14002e7a0` replaces `flash` with the
/// language string).
pub fn library_resource(name: &str, language: &str) -> String {
    let stem = name.strip_suffix(".swf").unwrap_or(name);
    let stem = match stem.strip_prefix("lib_loc_flash") {
        Some(rest) => format!("lib_loc_{language}{rest}"),
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
