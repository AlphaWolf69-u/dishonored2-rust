//! Key bindings: the binding manager (`PTR_DAT_142274340`), the config
//! commands that fill it, and the per-action data the UI reads
//! (`bindableActionsData`, `FUN_140788e30`).
use crate::assets::lang::StringTable;
use std::path::Path;

#[path = "binding_tables.rs"]
mod binding_tables;
pub use binding_tables::{ACTIONS, BINDING_SETS, KEY_ICONS, KEY_NAMES};

/// Entries per binding set, indexed by engine key code (`FUN_1400396e0`).
pub const KEYS: usize = 256;
/// `IN_GAME`, the set the UI reads regardless of the active set.
pub const IN_GAME: usize = 2;
/// `configVersion` the engine accepts in the user config (chunk_000012).
pub(crate) const CONFIG_VERSION: i64 = 10;

/// One `0x30`-byte set entry: `+0` secondary flag, `+4` action id, `+8`
/// command string.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub secondary: bool,
    pub action: i32,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct Bindings {
    pub sets: Vec<Vec<Binding>>,
    pub current: usize,
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings { sets: vec![vec![Binding::default(); KEYS]; BINDING_SETS.len()], current: 0 }
    }
}

/// `FUN_14003b4f0`: a key name (case-insensitive) to its engine code, 0 when
/// unknown.
pub fn key_code(name: &str) -> u32 {
    if name.is_empty() {
        return 0;
    }
    KEY_NAMES.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)).map_or(0, |k| k.0)
}

/// `FUN_140038b60`: the UI label of an action id.
pub fn action_label(id: i32) -> Option<&'static str> {
    ACTIONS.iter().find(|a| a.0 == id).map(|a| a.2)
}

/// `FUN_140039d20`: pad keys are 0xeb..0xff.
pub fn is_pad_key(key: u32) -> bool {
    key.wrapping_sub(0xeb) < 0x15
}

/// `FUN_140039e50`: the controller-button icon name of a key.
pub fn key_icon(key: u32) -> Option<&'static str> {
    KEY_ICONS.iter().find(|k| k.0 == key).map(|k| k.1)
}

impl Bindings {
    fn set(&mut self) -> &mut Vec<Binding> {
        &mut self.sets[self.current]
    }

    /// `FUN_1400396e0` (`arkUnbindAll`): clears every set; the loop leaves
    /// the last set current.
    pub fn unbind_all(&mut self) {
        for (index, set) in self.sets.iter_mut().enumerate() {
            self.current = index;
            set.iter_mut().for_each(|b| *b = Binding::default());
        }
    }

    /// `FUN_140039ad0`: the keys bound to `action` in `set`, ascending.
    pub fn keys_for_action(&self, set: usize, action: i32) -> Vec<u32> {
        (0..KEYS as u32).filter(|&k| self.sets[set][k as usize].action == action).collect()
    }

    /// `FUN_140039b90`: the keys whose command string equals `command`
    /// (case-sensitive, `FUN_140140130`).
    pub fn keys_for_command(&self, set: usize, command: &str) -> Vec<u32> {
        (0..KEYS as u32).filter(|&k| self.sets[set][k as usize].command == command).collect()
    }

    /// One tokenized command: the binding commands registered in
    /// chunk_000958 and `exec`; any other command goes to `other`.
    fn command(&mut self, args: &[String], exec: &mut dyn FnMut(&str) -> Option<String>, other: &mut dyn FnMut(&[String])) -> Result<(), String> {
        let Some(name) = args.first() else { return Ok(()) };
        let arg = |i: usize| args.get(i).map_or("", String::as_str);
        if name.eq_ignore_ascii_case("arkBind") || name.eq_ignore_ascii_case("arkBindSecondary") {
            // `FUN_14003b780(args, secondary)`.
            let secondary = name.eq_ignore_ascii_case("arkBindSecondary");
            if args.len() < 2 {
                return Ok(());
            }
            let key = key_code(arg(1));
            // Unknown keys are reported; two arguments print the binding.
            if key == 0 || args.len() == 2 {
                return Ok(());
            }
            let command = args[2..].join(" ");
            let action = ACTIONS
                .iter()
                .find(|a| !command.is_empty() && a.1.eq_ignore_ascii_case(&command))
                .map_or(0, |a| a.0);
            // `FUN_140039380` stores an action and clears the command;
            // otherwise the command string is stored with action 0.
            let entry = &mut self.set()[key as usize];
            *entry = if action != 0 {
                Binding { secondary, action, command: String::new() }
            } else {
                Binding { secondary, action: 0, command }
            };
        } else if name.eq_ignore_ascii_case("arkSwitchToBindingSet") {
            // `FUN_14003b9d0`/`FUN_14003b460`: an unknown name selects set 0.
            let wanted = arg(1);
            self.current = BINDING_SETS
                .iter()
                .find(|s| !wanted.is_empty() && s.1.eq_ignore_ascii_case(wanted))
                .map_or(0, |s| s.0 as usize);
        } else if name.eq_ignore_ascii_case("arkUnbindAll") {
            self.unbind_all();
        } else if name.eq_ignore_ascii_case("exec") {
            if let Some(text) = exec(arg(1)) {
                self.execute_with(&text, exec, other)?;
            }
        } else {
            other(args);
        }
        Ok(())
    }

    /// `FUN_1400ff8f0`: run a command buffer. A command ends at a newline,
    /// a carriage return, or a `;` outside quotes; `//` before any quote on
    /// the line ends it, and `/* */` comments may span lines.
    pub fn execute(&mut self, text: &str, exec: &mut dyn FnMut(&str) -> Option<String>) -> Result<(), String> {
        self.execute_with(text, exec, &mut |_| {})
    }

    /// [`Self::execute`] with the non-binding commands passed to `other`.
    pub fn execute_with(&mut self, text: &str, exec: &mut dyn FnMut(&str) -> Option<String>, other: &mut dyn FnMut(&[String])) -> Result<(), String> {
        let bytes = text.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            let mut quotes = 0u32;
            let mut i = start;
            let (end, next) = loop {
                if i >= bytes.len() {
                    break (bytes.len(), bytes.len());
                }
                let c = bytes[i];
                if c == b'"' {
                    quotes += 1;
                }
                if c == b'/' && quotes == 0 && i + 1 < bytes.len() {
                    if bytes[i + 1] == b'/' {
                        let stop = bytes[i..].iter().position(|&b| b == b'\n' || b == b'\r').map_or(bytes.len(), |p| i + p);
                        break (i, (stop + 1).min(bytes.len()));
                    }
                    if bytes[i + 1] == b'*' {
                        // The closing `/` becomes the newline that ends this command.
                        let close = (i..bytes.len().saturating_sub(1)).find(|&j| bytes[j] == b'*' && bytes[j + 1] == b'/');
                        break (i, close.map_or(bytes.len(), |j| j + 2));
                    }
                }
                if (c == b';' && quotes % 2 == 0) || c == b'\n' || c == b'\r' {
                    break (i, i + 1);
                }
                i += 1;
            };
            let line = &text[start..end];
            start = next;
            let args = tokenize(line);
            if let Some(first) = args.first() {
                let binding_command = ["arkBind", "arkBindSecondary", "arkSwitchToBindingSet", "arkUnbindAll", "exec"]
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(first.as_deref().unwrap_or("")));
                match args.into_iter().collect::<Option<Vec<_>>>() {
                    Some(args) => self.command(&args, exec, other)?,
                    None if binding_command => {
                        return Err(format!("unquoted punctuation in {line:?}: idCmdArgs tokenization not yet traced"))
                    }
                    None => {}
                }
            }
        }
        Ok(())
    }
}

/// Command arguments (`FUN_140132630`, lexer flags 0xa33): quoted strings
/// (no escapes), `//` comments, and unquoted names, which include `.` (the
/// game execs `default_keybinds.cfg` unquoted from default.cfg). A token with
/// other unquoted punctuation is `None`: those lexer rules are not traced.
fn tokenize(line: &str) -> Vec<Option<String>> {
    let mut args = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '/' && chars.clone().nth(1) == Some('/') {
            break;
        } else if c == '"' {
            chars.next();
            args.push(Some(chars.by_ref().take_while(|&c| c != '"').collect()));
        } else {
            let mut token = String::new();
            let mut plain = true;
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                plain &= c.is_ascii_alphanumeric() || c == '_' || c == '.';
                token.push(c);
                chars.next();
            }
            args.push(plain.then_some(token));
        }
    }
    args
}

/// The version header the engine reads from a config (chunk_000012
/// ~2398): `configVersion <int>` then `bindingVersion <int>`, each pair read
/// in that order from the first tokens; a missing pair is `None`.
pub(crate) fn versions(source: &str) -> (Option<i64>, Option<i64>) {
    let mut tokens = source.split_whitespace().peekable();
    let mut pair = |name: &str| -> Option<i64> {
        let token = tokens.next()?;
        if !token.eq_ignore_ascii_case(name) {
            return None;
        }
        tokens.next()?.parse().ok()
    };
    let config = pair("configVersion");
    (config, pair("bindingVersion"))
}

/// The bindings the game starts with ([`super::console::Console::load`]).
pub fn game_bindings(base: &Path, user_config: Option<&str>) -> Result<Bindings, String> {
    super::console::Console::load(base, user_config).map(|c| c.bindings)
}

/// One `bindableActionsData.data[i]` element.
#[derive(Clone, Debug, PartialEq)]
pub struct BindableAction {
    pub label: String,
    pub bindings: [String; 3],
    pub localized_bindings: [String; 3],
    pub key_codes: [i32; 3],
}

/// `FUN_140788e30`: actions 0..46 by id, then quick load and quick save by
/// command. Keys are scanned in ascending code order and the last one wins
/// each slot: pad keys slot 0, primary keys slot 1, secondary keys slot 2.
pub fn bindable_actions(bindings: &Bindings, strings: &StringTable, keyboard: &mut Keyboard) -> Vec<BindableAction> {
    let mut actions = Vec::with_capacity(49);
    let slots = |keys: Vec<u32>| {
        let mut slots = [0u32; 3];
        for key in keys {
            if is_pad_key(key) {
                slots[0] = key;
            } else if !bindings.sets[IN_GAME][key as usize].secondary {
                slots[1] = key;
            } else {
                slots[2] = key;
            }
        }
        slots
    };
    let mut push = |label: &str, keys: [u32; 3], keyboard: &mut Keyboard| {
        let mut action = BindableAction {
            label: label.to_string(),
            bindings: Default::default(),
            localized_bindings: Default::default(),
            key_codes: [-1; 3],
        };
        for (k, &key) in keys.iter().enumerate() {
            if key != 0 {
                action.bindings[k] = key_icon(key).unwrap_or("").to_string();
                action.localized_bindings[k] = keyboard.localized_name(key, strings);
                action.key_codes[k] = keyboard.virtual_key(key).map_or(key as i32, |vk| vk as i32);
            }
        }
        actions.push(action);
    };
    for id in 0..47 {
        push(action_label(id).unwrap_or(""), slots(bindings.keys_for_action(IN_GAME, id)), keyboard);
    }
    for (label, command) in [("<GBA_QUICK_LOAD>", "loadgame"), ("<GBA_QUICK_SAVE>", "savegame")] {
        push(label, slots(bindings.keys_for_command(IN_GAME, command)), keyboard);
    }
    actions
}

/// `arkDirectInputKeyboard` key translation against the current Windows
/// keyboard layout (`FUN_1400e1fe0`) and the VK mapping of `FUN_140032f20`.
#[derive(Default)]
pub struct Keyboard {
    /// `{scan, vk, character}` of the US layout, sorted by character; built
    /// while it is empty and the current layout is not US.
    us_characters: Vec<(u32, u32, u16)>,
}

impl Keyboard {
    /// `FUN_140039d60`: the localized name of a key: the string-table text
    /// of its US key's `#str_key_*` entry, else the key name when it has no
    /// character, else the character. Translation failure is `???`.
    pub fn localized_name(&mut self, key: u32, strings: &StringTable) -> String {
        if key == 0 {
            return String::new();
        }
        let Some((us_key, character)) = self.translate(key) else { return "???".into() };
        if us_key != 0 {
            for (_, name, string_key) in KEY_NAMES.iter().filter(|k| k.0 == us_key) {
                if let Some(text) = strings.get(string_key) {
                    return text.to_string();
                }
                if character.is_empty() {
                    return name.to_string();
                }
            }
        }
        character
    }

    /// `FUN_1400e1fe0`: the US-layout key and the UTF-8 character of `key`
    /// in the current layout; `None` when both are empty.
    pub fn translate(&mut self, key: u32) -> Option<(u32, String)> {
        let (us_key, character) = os::translate(self, key);
        (us_key != 0 || !character.is_empty()).then_some((us_key, character))
    }

    /// `FUN_140032f20`: the Windows virtual key of an engine key, `None`
    /// when the game keeps the engine code.
    pub fn virtual_key(&self, key: u32) -> Option<u32> {
        // `FUN_140033120`: numpad digits.
        let numpad = match key {
            0x47 => Some(0x67),
            0x48 => Some(0x68),
            0x49 => Some(0x69),
            0x4b => Some(0x64),
            0x4c => Some(0x65),
            0x4d => Some(0x66),
            0x4f => Some(0x61),
            0x50 => Some(0x62),
            0x51 => Some(0x63),
            0x52 => Some(0x60),
            _ => None,
        };
        if numpad.is_some() {
            return numpad;
        }
        match os::scan_to_virtual_key(key) {
            0 => match key {
                0x9c => Some(0x6c),
                0x9d => Some(0x11),
                0xb5 => Some(0x6f),
                0xb8 => Some(0x12),
                0xc7 => Some(0x24),
                0xc8 => Some(0x26),
                0xc9 => Some(0x21),
                0xcb => Some(0x25),
                0xcd => Some(0x27),
                0xcf => Some(0x23),
                0xd0 => Some(0x28),
                0xd1 => Some(0x22),
                0xd2 => Some(0x2d),
                0xd3 => Some(0x2e),
                _ => None,
            },
            vk => Some(vk),
        }
    }
}

/// `idSearch` insertion point (`FUN_1400e1e80`): before equal characters.
fn insert_position(table: &[(u32, u32, u16)], ch: u16) -> usize {
    if table.is_empty() {
        return 0;
    }
    let (mut offset, mut n) = (0usize, table.len());
    loop {
        let half = n >> 1;
        let after = table[offset + half].2 < ch;
        if after {
            offset += half;
        }
        n -= half;
        if half == 0 {
            return offset + after as usize;
        }
    }
}

/// `idSearch` find (`FUN_1400e1d90`): the last entry not above `ch`, if equal.
fn find(table: &[(u32, u32, u16)], ch: u16) -> Option<usize> {
    if table.is_empty() {
        return None;
    }
    let (mut offset, mut n) = (0usize, table.len());
    loop {
        let half = n >> 1;
        if table[offset + half].2 <= ch {
            offset += half;
        }
        n -= half;
        if half == 0 {
            break;
        }
    }
    (table[offset].2 == ch).then_some(offset)
}

#[cfg(target_os = "windows")]
mod os {
    use super::{find, insert_position, Keyboard};
    use std::ffi::c_void;
    type Hkl = *mut c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn LoadKeyboardLayoutA(id: *const u8, flags: u32) -> Hkl;
        fn GetKeyboardLayout(thread: u32) -> Hkl;
        fn MapVirtualKeyExA(code: u32, map: u32, layout: Hkl) -> u32;
        fn GetSystemMetrics(index: i32) -> i32;
    }
    const MAPVK_VK_TO_CHAR: u32 = 2;
    const MAPVK_VSC_TO_VK_EX: u32 = 3;
    const MAPVK_VSC_TO_VK: u32 = 1;
    const SM_SWAPBUTTON: i32 = 0x17;

    fn us_layout() -> Hkl {
        static US: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *US.get_or_init(|| unsafe { LoadKeyboardLayoutA(b"00000409\0".as_ptr(), 0) as usize }) as Hkl
    }

    /// `MAPVK_VK_TO_CHAR` keeps the low 16 bits (`(short)` in the game).
    fn character(vk: u32, layout: Hkl) -> u16 {
        unsafe { MapVirtualKeyExA(vk, MAPVK_VK_TO_CHAR, layout) as u16 }
    }

    fn utf8(ch: u16) -> String {
        char::decode_utf16([ch]).map(|c| c.unwrap_or('\u{fffd}')).collect()
    }

    pub(super) fn translate(keyboard: &mut Keyboard, key: u32) -> (u32, String) {
        if key >= 0xe0 {
            // Mouse buttons 1 and 2 swap with the system setting.
            if (key == 0xe0 || key == 0xe1) && unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
                return (if key == 0xe0 { 0xe1 } else { 0xe0 }, String::new());
            }
            return (key, String::new());
        }
        let us = us_layout();
        let layout = unsafe { GetKeyboardLayout(0) };
        let is_us = layout == us;
        let table = &mut keyboard.us_characters;
        if !is_us && table.is_empty() {
            for scan in 1..0xe0 {
                let vk = unsafe { MapVirtualKeyExA(scan, MAPVK_VSC_TO_VK_EX, us) };
                if vk != 0 {
                    let ch = character(vk, us);
                    if ch != 0 {
                        let at = insert_position(table, ch);
                        table.insert(at, (scan, vk, ch));
                    }
                }
            }
        }
        let mut us_key = if is_us { key } else { 0 };
        let digit = match key {
            2 | 0x4f => Some('1'),
            3 | 0x50 => Some('2'),
            4 | 0x51 => Some('3'),
            5 | 0x4b => Some('4'),
            6 | 0x4c => Some('5'),
            7 | 0x4d => Some('6'),
            8 | 0x47 => Some('7'),
            9 | 0x48 => Some('8'),
            10 | 0x49 => Some('9'),
            0xb | 0x52 => Some('0'),
            0x53 => Some('.'),
            _ => None,
        };
        if let Some(digit) = digit {
            return (us_key, digit.to_string());
        }
        let vk = unsafe { MapVirtualKeyExA(key, MAPVK_VSC_TO_VK_EX, layout) };
        if vk == 0 {
            return (key, String::new());
        }
        let ch = character(vk, layout);
        if ch == 0 {
            return (key, String::new());
        }
        if !is_us {
            if let Some(i) = find(table, ch) {
                us_key = table[i].0;
            }
        }
        (us_key, utf8(ch))
    }

    pub(super) fn scan_to_virtual_key(key: u32) -> u32 {
        unsafe { MapVirtualKeyExA(key, MAPVK_VSC_TO_VK, GetKeyboardLayout(0)) }
    }
}

#[cfg(not(target_os = "windows"))]
mod os {
    use super::Keyboard;
    // The game's translation uses Windows keyboard layouts only.
    pub(super) fn translate(_: &mut Keyboard, key: u32) -> (u32, String) {
        (key, String::new())
    }
    pub(super) fn scan_to_virtual_key(_: u32) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Bindings {
        let mut b = Bindings::default();
        b.execute(text, &mut |_| None).unwrap();
        b
    }

    #[test]
    fn commands_follow_the_engine() {
        let b = run("arkUnbindAll\n/* block\narkBind \"Q\" \"JUMP\"\n*/arkSwitchToBindingSet \"IN_GAME\"\narkBind \"SPACE\" \"jump\" // comment\narkBindSecondary UPARROW \"MOVE_FORWARD\"; arkBind F5 \"savegame\"\narkBind \"NOPE\" \"JUMP\"\n");
        assert_eq!(b.current, IN_GAME);
        let set = &b.sets[IN_GAME];
        assert_eq!(set[key_code("SPACE") as usize], Binding { secondary: false, action: 14, command: String::new() });
        assert_eq!(set[key_code("Q") as usize].action, 0);
        assert_eq!(set[key_code("UPARROW") as usize], Binding { secondary: true, action: 2, command: String::new() });
        assert_eq!(set[key_code("F5") as usize].command, "savegame");
        assert_eq!(b.keys_for_command(IN_GAME, "savegame"), vec![key_code("F5")]);
        let b = run("arkSwitchToBindingSet \"unknown\"\n");
        assert_eq!(b.current, 0);
    }

    #[test]
    fn character_search_matches_the_engine() {
        let mut table = Vec::new();
        for (scan, ch) in [(1, 'b'), (2, 'a'), (3, 'b'), (4, 'c')] {
            let at = insert_position(&table, ch as u16);
            table.insert(at, (scan, 0, ch as u16));
        }
        // Later equal characters insert before earlier ones; find returns the last.
        assert_eq!(table.iter().map(|e| e.0).collect::<Vec<_>>(), vec![2, 3, 1, 4]);
        assert_eq!(find(&table, 'b' as u16).map(|i| table[i].0), Some(1));
        assert_eq!(find(&table, 'z' as u16), None);
    }

    #[test]
    #[ignore = "requires the installed game"]
    fn installed_defaults_bind_pause_and_quick_slots() {
        let base = crate::assets::installation::game_base().unwrap();
        let b = game_bindings(&base, None).unwrap();
        assert_eq!(b.keys_for_action(IN_GAME, 43), vec![key_code("ESCAPE"), key_code("PAD_START_OPTIONS")]);
        assert_eq!(b.keys_for_command(IN_GAME, "loadgame"), vec![key_code("F9")]);
        let strings = StringTable::parse(b"").unwrap();
        let actions = bindable_actions(&b, &strings, &mut Keyboard::default());
        assert_eq!(actions.len(), 49);
        assert_eq!(actions[47].label, "<GBA_QUICK_LOAD>");
        assert_eq!(actions[47].bindings[1], "<keyboard_F9>");
        assert_eq!(actions[47].key_codes[1], 0x78);
        assert_eq!(actions[47].key_codes[2], -1);
    }
}
