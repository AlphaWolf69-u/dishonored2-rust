//! The console variable system (`idCVarSystemLocal`, `PTR_DAT_14228bae8`):
//! every static idCVar the executable registers, their value validation,
//! the set paths the options screens use, and the archive writer behind
//! `dishonored2Config.cfg`.

#[path = "cvar_table.rs"]
mod cvar_table;
pub use cvar_table::CVARS;

/// idCVar `+0x48` flag bits.
pub mod flag {
    pub const BOOL: u32 = 0x1;
    pub const INT: u32 = 0x2;
    pub const FLOAT: u32 = 0x4;
    /// Settable in retail builds (`FUN_140101640` sets it for archived and
    /// similar cvars).
    pub const RETAIL: u32 = 0x10;
    /// No value parsing (`FUN_140102cb0`).
    pub const RAW: u32 = 0x40;
    pub const COMMAND_LINE_ONLY: u32 = 0x4000;
    pub const READ_ONLY: u32 = 0x8000;
    pub const ARCHIVE: u32 = 0x10000;
    /// Set by every validation.
    pub const MODIFIED: u32 = 0x40000;
    /// Set when a set call changed the value.
    pub const CHANGED: u32 = 0x80000;
}

/// One registered idCVar: `+0x00` value string, `+0x28` int, `+0x2c` float,
/// `+0x30` name, `+0x38` reset string, `+0x48` flags, `+0x4c`/`+0x50` range,
/// `+0x58` value strings.
#[derive(Clone, Debug)]
pub struct CVar {
    pub name: &'static str,
    pub reset: &'static str,
    pub value: String,
    pub int: i32,
    pub float: f32,
    pub flags: u32,
    pub min: f32,
    pub max: f32,
    pub values: &'static [&'static str],
}

/// `FUN_1401435f0`/`FUN_140143650`: ASCII case-insensitive equality.
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// `FUN_140143c70`: an optional `-`, then digits with at most one `.`.
fn is_numeric(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let mut dot = false;
    for c in digits.bytes() {
        if !c.is_ascii_digit() {
            if c != b'.' || dot {
                return false;
            }
            dot = true;
        }
    }
    true
}

/// The CRT `atof` (`FUN_1411b578c`): the longest decimal prefix after
/// leading whitespace, 0 when there is none.
pub fn atof(s: &str) -> f64 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let b = s.as_bytes();
    let mut end = 0;
    if end < b.len() && (b[end] == b'+' || b[end] == b'-') {
        end += 1;
    }
    let digits_start = end;
    while end < b.len() && b[end].is_ascii_digit() {
        end += 1;
    }
    if end < b.len() && b[end] == b'.' {
        end += 1;
        while end < b.len() && b[end].is_ascii_digit() {
            end += 1;
        }
    }
    if end == digits_start || (end == digits_start + 1 && b[digits_start] == b'.') {
        return 0.0;
    }
    if end < b.len() && (b[end] == b'e' || b[end] == b'E') {
        let mut e = end + 1;
        if e < b.len() && (b[e] == b'+' || b[e] == b'-') {
            e += 1;
        }
        if e < b.len() && b[e].is_ascii_digit() {
            while e < b.len() && b[e].is_ascii_digit() {
                e += 1;
            }
            end = e;
        }
    }
    s[..end].parse().unwrap_or(0.0)
}

/// The CRT `atoi` (`FUN_1411ada38`): `strtol` base 10, saturating.
pub fn atoi(s: &str) -> i32 {
    let s = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut value: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        value = (value * 10 + (c - b'0') as i64).min(i32::MAX as i64 + 1);
    }
    let value = if negative { -value } else { value };
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// `FUN_14013f8a0`: `%f`, then trailing zeros and dots removed.
pub fn float_string(v: f32) -> String {
    let mut s = format!("{:.6}", v as f64);
    while s.len() > 1 && s.ends_with('0') {
        s.pop();
    }
    while s.len() > 1 && s.ends_with('.') {
        s.pop();
    }
    s
}

impl CVar {
    /// `FUN_140102cb0`: parse the value string into the int and float
    /// fields, clamping ranged numbers and canonicalising the string.
    fn validate(&mut self) {
        self.flags |= flag::MODIFIED;
        if self.flags & flag::RAW != 0 {
            return;
        }
        if self.flags & flag::BOOL != 0 {
            let on = atoi(&self.value) != 0;
            self.int = on as i32;
            self.float = on as i32 as f32;
            if !same(&self.value, "0") && !same(&self.value, "1") {
                self.value = (on as i32).to_string();
            }
            return;
        }
        if self.flags & flag::INT != 0 {
            self.int = atoi(&self.value);
            let mut rewrite = false;
            if self.min < self.max {
                if (self.int as f32) < self.min {
                    self.int = self.min as i32;
                    rewrite = true;
                } else if (self.int as f32) > self.max {
                    self.int = self.max as i32;
                    rewrite = true;
                }
            }
            if rewrite || !is_numeric(&self.value) {
                self.value = self.int.to_string();
            }
            self.float = self.int as f32;
            return;
        }
        if self.flags & flag::FLOAT != 0 {
            self.float = atof(&self.value) as f32;
            let mut rewrite = false;
            if self.min < self.max {
                if self.float < self.min {
                    self.float = self.min;
                    rewrite = true;
                } else if self.float > self.max {
                    self.float = self.max;
                    rewrite = true;
                }
            }
            if rewrite || !is_numeric(&self.value) {
                self.value = float_string(self.float);
            }
            self.int = self.float as i32;
            return;
        }
        if !self.values.is_empty() {
            // A value list: the index of the matching name (0 when none), and
            // the string takes the list's spelling.
            self.int = self.values.iter().position(|v| same(&self.value, v)).unwrap_or(0) as i32;
            self.value = self.values[self.int as usize].to_string();
            self.float = self.int as f32;
            return;
        }
        if self.value.len() < 0x20 {
            let v = atof(&self.value);
            self.int = v as i32;
            self.float = v as f32;
        } else {
            self.int = 0;
            self.float = 0.0;
        }
    }
}

/// The developer flag `DAT_1432cd1c8` (zero in retail, so cvars without the
/// retail flag refuse unforced sets). Diagnostic: `DH2_DEBUG_DEVELOPER=1`
/// sets it, so a config may set debug cvars.
pub fn developer() -> bool {
    static DEVELOPER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEVELOPER.get_or_init(|| std::env::var("DH2_DEBUG_DEVELOPER").as_deref() == Ok("1"))
}

#[derive(Clone, Debug)]
pub struct CVars {
    /// Registration order (the archive writer walks it backwards).
    pub list: Vec<CVar>,
    /// The system's modified-flags accumulator (vfunc +0x88 / +0x90).
    pub modified_flags: u32,
    /// Counts value changes (rewrite bookkeeping for live settings).
    pub changes: u64,
}

impl Default for CVars {
    fn default() -> Self {
        Self::new()
    }
}

impl CVars {
    /// Every static registration, each validated from its reset string
    /// (`FUN_140101640`).
    pub fn new() -> Self {
        let list = CVARS
            .iter()
            .map(|&(name, reset, flags, min, max, values)| {
                let mut c = CVar { name, reset, value: reset.to_string(), int: 0, float: 0.0, flags, min, max, values };
                c.validate();
                c
            })
            .collect();
        CVars { list, modified_flags: 0, changes: 0 }
    }

    /// `FUN_1401013e0`: case-insensitive lookup.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|c| same(c.name, name))
    }

    pub fn get(&self, name: &str) -> Option<&CVar> {
        self.find(name).map(|i| &self.list[i])
    }

    pub fn int(&self, name: &str) -> Option<i32> {
        self.get(name).map(|c| c.int)
    }

    pub fn float(&self, name: &str) -> Option<f32> {
        self.get(name).map(|c| c.float)
    }

    /// `FUN_140102400`: set the value string. Unforced sets respect the
    /// command-line-only, read-only and retail gates ([`developer`]).
    /// Returns whether the value changed.
    pub fn set(&mut self, index: usize, value: Option<&str>, force: bool) -> bool {
        let c = &mut self.list[index];
        if !force && (c.flags & (flag::COMMAND_LINE_ONLY | flag::READ_ONLY) != 0 || c.flags & flag::RETAIL == 0 && !developer()) {
            return false;
        }
        let value = value.unwrap_or(c.reset);
        if same(&c.value, value) {
            return false;
        }
        c.value = value.to_string();
        c.validate();
        self.modified_flags |= c.flags;
        c.flags |= flag::CHANGED;
        self.changes += 1;
        true
    }

    /// `FUN_140102520`.
    pub fn set_bool(&mut self, index: usize, v: bool, force: bool) -> bool {
        self.set(index, Some(&(v as i32).to_string()), force)
    }

    /// `FUN_1401028e0`.
    pub fn set_int(&mut self, index: usize, v: i32, force: bool) -> bool {
        self.set(index, Some(&v.to_string()), force)
    }

    /// `FUN_140102880`.
    pub fn set_float(&mut self, index: usize, v: f32, force: bool) -> bool {
        self.set(index, Some(&float_string(v)), force)
    }

    /// `FUN_1401022d0`: back to the reset string (no system flags, no
    /// changed bit).
    pub fn reset(&mut self, index: usize) {
        let c = &mut self.list[index];
        if !same(&c.value, c.reset) {
            self.changes += 1;
        }
        c.value = c.reset.to_string();
        c.validate();
    }

    /// `FUN_140100f20`: a console command naming a cvar. Returns false when
    /// no cvar has that name; one argument only prints the value.
    pub fn command(&mut self, args: &[String]) -> bool {
        let Some(index) = args.first().and_then(|n| self.find(n)) else { return false };
        if args.len() > 1 {
            // `FUN_140132470(args, 1, -1)`: the remaining arguments joined.
            let value = args[1..].join(" ");
            self.set(index, Some(&value), false);
        }
        true
    }

    /// `FUN_140102fc0(flags)`: `name "value"` for every cvar with one of
    /// `mask`'s flags whose value differs from its reset string, newest
    /// registration first.
    pub fn write(&self, mask: u32, out: &mut String) {
        for c in self.list.iter().rev() {
            if c.flags & mask != 0 && !same(&c.value, c.reset) {
                out.push_str(&format!("{} \"{}\"\n", c.name, c.value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_follows_the_engine() {
        let mut cvars = CVars::new();
        // A float default that is not plain numeric is canonicalised.
        let sens = cvars.find("g_mouseSensitivity").unwrap();
        assert_eq!((cvars.list[sens].value.as_str(), cvars.list[sens].float), ("1", 1.0));
        // Ranged floats clamp and rewrite; the int field truncates.
        let gamma = cvars.find("R_GAMMA").unwrap();
        assert!(cvars.set_float(gamma, 9.0, true));
        assert_eq!((cvars.list[gamma].value.as_str(), cvars.list[gamma].int), ("4", 4));
        assert!(cvars.list[gamma].flags & flag::CHANGED != 0);
        // Unranged floats keep a numeric string as given.
        let scale = cvars.find("rs_minimumResolutionScale").unwrap();
        cvars.set(scale, Some("0.750"), true);
        assert_eq!((cvars.list[scale].value.as_str(), cvars.list[scale].float), ("0.750", 0.75));
        // Bools take 0/1 spellings.
        let rumble = cvars.find("ark_rumble_enable").unwrap();
        cvars.set(rumble, Some("yes"), true);
        assert_eq!((cvars.list[rumble].value.as_str(), cvars.list[rumble].int), ("0", 0));
        // Value lists take the list's spelling.
        let filter = cvars.find("image_magFilter").unwrap();
        cvars.set(filter, Some("aniso"), true);
        assert_eq!((cvars.list[filter].value.as_str(), cvars.list[filter].int), ("ANISO", 2));
        assert_eq!(float_string(100.0), "100");
        assert_eq!(float_string(0.85), "0.85");
    }

    #[test]
    fn changes_count_only_real_changes() {
        let mut cvars = CVars::new();
        let i = cvars.find("com_timeScaler").unwrap();
        cvars.set(i, Some("2"), true);
        assert_eq!(cvars.changes, 0);
        cvars.set(i, Some("4"), true);
        cvars.reset(i);
        cvars.reset(i);
        assert_eq!(cvars.changes, 2);
    }

    #[test]
    fn archive_lists_changed_cvars_newest_first() {
        let mut cvars = CVars::new();
        let mut set = |name: &str, value: &str| {
            let i = cvars.find(name).unwrap();
            cvars.set(i, Some(value), false);
        };
        set("com_timeScaler", "4");
        set("s_MasterBus_Gain", "100");
        set("ark_fieldOfView", "110");
        // Without the retail flag an unforced set is refused.
        set("in_controlInactiveWindow", "1");
        let mut out = String::new();
        cvars.write(flag::ARCHIVE, &mut out);
        // Defaults that validation rewrites ("1.0f" -> "1") differ from
        // their reset string and are archived too, as in the game's config.
        let expected = [
            "ark_fieldOfView \"110\"",
            "g_lookStickSensitivityY \"1\"",
            "g_lookStickSensitivityX \"1\"",
            "g_mouseSensitivity \"1\"",
            "g_mouseClamp \"360\"",
            "ark_player_autoaim_strength_mouse \"1\"",
            "ark_player_autoaim_strength_pad \"1\"",
            "s_MasterBus_Gain \"100\"",
            "com_timeScaler \"4\"",
        ];
        assert_eq!(out.lines().collect::<Vec<_>>(), expected);
        assert_eq!(cvars.get("in_controlInactiveWindow").unwrap().value, "0");
    }
}
