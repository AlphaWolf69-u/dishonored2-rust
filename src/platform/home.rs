//! The rewrite's own folder (`DH2Rewrite/`, next to the launcher and the
//! executable): config, saves, mods, the rewrite source, cache and logs.
//! The installed game is only read.
//!
//! Host integration (a deliberate deviation): the original keeps its config
//! and saves under `Saved Games/Arkane Studios/Dishonored2/base`.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const CONFIG_FILE: &str = "dishonored2Config.cfg";
const SETTINGS_FILE: &str = "rewrite.toml";
const MODLIST_FILE: &str = "modlist.toml";
pub const MOD_MANIFEST: &str = "mod.toml";
const MOD_RESOURCES: &str = "resources";
const MOD_PATCHES: &str = "patches";

const MODLIST_TEMPLATE: &str = "\
# Enabled mods by name (mod.toml `name`), in load order.
enabled = []
";

const SETTINGS_TEMPLATE: &str = "\
# Dishonored 2 rewrite settings.
# game_path: the installed game's \"base\" folder (the one containing game1_patch.index).
# Leave it out to look for a Steam installation.
";

const MODS_README: &str = "\
One folder per mod (any folder name):
  mod.toml       name, version, author, rewrite, repository, description, saves
  patches/       *.patch: changes to the rewrite source, made against `rewrite`
  resources/     files at the game's resource paths, replacing or adding resources

`rewrite` is the rewrite release tag (or commit) the patches were made against.
`saves` is safe (never touches saves), cleanup (removes its data from saves when
disabled) or permanent (saves started with it need it).
";

/// The folder layout.
#[derive(Clone, Debug)]
pub struct Home {
    pub root: PathBuf,
}

/// What the first use did, for the session log.
#[derive(Clone, Debug, Default)]
pub struct Setup {
    pub created: bool,
    pub problems: Vec<String>,
}

static HOME: OnceLock<(Home, Setup)> = OnceLock::new();

impl Home {
    /// `DH2_REWRITE_HOME`, else the executable's folder.
    fn locate() -> Home {
        let root = std::env::var_os("DH2_REWRITE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::current_exe().ok()?.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Home { root }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("saves")
    }
    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }
    /// The rewrite source the launcher downloads and builds.
    pub fn source_dir(&self) -> PathBuf {
        self.root.join("source")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn config_file(&self) -> PathBuf {
        self.config_dir().join(CONFIG_FILE)
    }
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir().join(SETTINGS_FILE)
    }
    pub fn modlist_file(&self) -> PathBuf {
        self.root.join(MODLIST_FILE)
    }

    /// Create the layout. Config and saves are imported only on request
    /// ([`import_config`], [`import_saves`]).
    fn ensure(&self) -> Setup {
        let mut setup = Setup::default();
        let mut note = |r: io::Result<()>, what: &str| {
            if let Err(e) = r {
                setup.problems.push(format!("{what}: {e}"));
            }
        };
        setup.created = !self.config_dir().exists();
        for dir in [self.config_dir(), self.saves_dir(), self.mods_dir(), self.cache_dir(), self.logs_dir()] {
            note(fs::create_dir_all(&dir), &dir.display().to_string());
        }
        note(write_new(&self.modlist_file(), MODLIST_TEMPLATE), MODLIST_FILE);
        note(write_new(&self.settings_file(), SETTINGS_TEMPLATE), SETTINGS_FILE);
        note(write_new(&self.mods_dir().join("README.txt"), MODS_README), "mods/README.txt");
        setup
    }
}

/// The rewrite's folder, created on first use.
pub fn home() -> &'static Home {
    &HOME.get_or_init(|| {
        let home = Home::locate();
        let setup = home.ensure();
        (home, setup)
    })
    .0
}

/// What the first use of [`home`] did.
pub fn setup() -> &'static Setup {
    home();
    &HOME.get().unwrap().1
}

fn write_new(path: &Path, text: &str) -> io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    fs::write(path, text)
}

/// Copy a game config (`dishonored2Config.cfg`) the user chose into
/// `config/`, replacing the rewrite's.
pub fn import_config(from: &Path) -> Result<(), String> {
    let text = fs::read_to_string(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if !text.trim_start().starts_with("configVersion") {
        return Err(format!("{} is not a Dishonored 2 config (no configVersion header)", from.display()));
    }
    fs::write(home().config_file(), text).map_err(|e| e.to_string())
}

/// The result of [`import_saves`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    pub copied: usize,
    /// Files already in `saves/`, left as they are.
    pub kept: usize,
}

/// Copy a game save folder (the original's `savegame` layout: `profile.bin`
/// and `SLOTxx` folders) into `saves/`, never overwriting a file there.
pub fn import_saves(from: &Path) -> Result<Imported, String> {
    if !from.join("profile.bin").is_file() && !fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten().any(|e| e.file_name().to_string_lossy().starts_with("SLOT")) {
        return Err(format!("{} is not a Dishonored 2 save folder (no profile.bin or SLOT folders)", from.display()));
    }
    let mut imported = Imported::default();
    copy_tree(from, &home().saves_dir(), &mut imported).map_err(|e| e.to_string())?;
    Ok(imported)
}

fn copy_tree(from: &Path, to: &Path, imported: &mut Imported) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target)?;
            copy_tree(&entry.path(), &target, imported)?;
        } else if target.exists() {
            imported.kept += 1;
        } else {
            fs::copy(entry.path(), &target)?;
            imported.copied += 1;
        }
    }
    Ok(())
}

/// A string setting in `config/rewrite.toml`.
pub fn setting(key: &str) -> Option<String> {
    let text = fs::read_to_string(home().settings_file()).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get(key)?.as_str().map(str::to_string)
}

/// Remove a setting from `config/rewrite.toml`, keeping its comments.
pub fn remove_setting(key: &str) -> Result<(), String> {
    let file = home().settings_file();
    let Ok(text) = fs::read_to_string(&file) else { return Ok(()) };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    doc.remove(key);
    fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
}

/// Set a string setting in `config/rewrite.toml`, keeping its comments.
pub fn set_setting(key: &str, value: &str) -> Result<(), String> {
    let file = home().settings_file();
    let text = fs::read_to_string(&file).unwrap_or_else(|_| SETTINGS_TEMPLATE.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    doc[key] = toml_edit::value(value);
    fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
}

/// `game_path`.
pub fn saved_game_path() -> Option<PathBuf> {
    setting("game_path").map(PathBuf::from)
}

/// Remember the game folder.
pub fn save_game_path(path: &Path) -> Result<(), String> {
    set_setting("game_path", &path.display().to_string())
}

/// `saves` in `mod.toml`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SaveSafety {
    /// Never touches saves; can be removed any time.
    Safe,
    /// Removes its own data from saves when it is disabled.
    Cleanup,
    /// Saves started with it need it.
    Permanent,
}

impl SaveSafety {
    pub fn label(self) -> &'static str {
        match self {
            SaveSafety::Safe => "save-safe",
            SaveSafety::Cleanup => "cleans up saves",
            SaveSafety::Permanent => "permanent in saves",
        }
    }
}

/// One mod folder under `mods/`, valid or not.
#[derive(Clone, Debug, PartialEq)]
pub struct ModInfo {
    /// The mod's identity (modlist, dependencies, saves).
    pub name: String,
    pub version: String,
    pub author: String,
    /// The rewrite revision its patches were made against.
    pub rewrite: String,
    pub repository: String,
    pub description: String,
    pub saves: SaveSafety,
    /// (mod name, version requirement): mods it needs enabled and loaded
    /// before it. The requirement is `*`, `1.2` (compatible with 1.2 and
    /// newer, cargo's caret rule), `>=1.2` or `=1.2.0`.
    pub requires: Vec<(String, String)>,
    /// Mods it cannot be enabled with.
    pub incompatible: Vec<String>,
    /// Mods it loads after (or before) when both are enabled.
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub folder: PathBuf,
    /// Why the mod cannot be enabled, if it cannot.
    pub problem: Option<String>,
}

impl ModInfo {
    /// `patches/*.patch`, by file name.
    pub fn patches(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = fs::read_dir(self.folder.join(MOD_PATCHES))
            .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("patch"))).collect())
            .unwrap_or_default();
        out.sort();
        out
    }
}

/// Read a mod folder's `mod.toml`.
pub fn read_mod(folder: &Path) -> ModInfo {
    let fallback = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut info = ModInfo {
        name: fallback,
        version: String::new(),
        author: String::new(),
        rewrite: String::new(),
        repository: String::new(),
        description: String::new(),
        saves: SaveSafety::Permanent,
        requires: Vec::new(),
        incompatible: Vec::new(),
        load_after: Vec::new(),
        load_before: Vec::new(),
        folder: folder.to_path_buf(),
        problem: None,
    };
    let manifest = match fs::read_to_string(folder.join(MOD_MANIFEST)) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            info.problem = Some(format!("{MOD_MANIFEST} is missing"));
            return info;
        }
        Err(e) => {
            info.problem = Some(format!("{MOD_MANIFEST}: {e}"));
            return info;
        }
    };
    let doc: toml_edit::DocumentMut = match manifest.parse() {
        Ok(d) => d,
        Err(e) => {
            info.problem = Some(format!("{MOD_MANIFEST}: {e}"));
            return info;
        }
    };
    let field = |k: &str| doc.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if !field("name").is_empty() {
        info.name = field("name");
    }
    info.version = field("version");
    info.author = field("author");
    info.rewrite = field("rewrite");
    info.repository = field("repository");
    info.description = field("description");
    let saves = field("saves");
    let names = |k: &str| -> Vec<String> {
        doc.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str()).map(|v| v.trim().to_string()).collect()).unwrap_or_default()
    };
    // `requires = ["A"]` or `requires = { A = "1.2" }`.
    info.requires = match doc.get("requires") {
        Some(v) if v.is_table_like() => {
            v.as_table_like().unwrap().iter().map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("*").trim().to_string())).collect()
        }
        _ => names("requires").into_iter().map(|n| (n, "*".to_string())).collect(),
    };
    info.incompatible = names("incompatible");
    info.load_after = names("load_after");
    info.load_before = names("load_before");
    let mut missing: Vec<&str> = ["name", "version"].into_iter().filter(|k| field(k).is_empty()).collect();
    if !info.patches().is_empty() && info.rewrite.is_empty() {
        missing.push("rewrite (its patches need the rewrite release they were made against)");
    }
    info.saves = match saves.as_str() {
        "safe" => SaveSafety::Safe,
        "cleanup" => SaveSafety::Cleanup,
        "permanent" => SaveSafety::Permanent,
        _ => {
            missing.push("saves (safe, cleanup or permanent)");
            SaveSafety::Permanent
        }
    };
    if !missing.is_empty() {
        info.problem = Some(format!("{MOD_MANIFEST} needs {}", missing.join(", ")));
    }
    info
}

/// Every mod folder under `mods/`, by name. Two folders with the same name
/// are both reported.
pub fn installed_mods() -> Vec<ModInfo> {
    let mut out: Vec<ModInfo> = fs::read_dir(home().mods_dir())
        .map(|d| d.flatten().filter(|e| e.path().is_dir()).map(|e| read_mod(&e.path())).collect())
        .unwrap_or_default();
    let names: Vec<String> = out.iter().map(|m| m.name.to_lowercase()).collect();
    for m in &mut out {
        if names.iter().filter(|n| **n == m.name.to_lowercase()).count() > 1 && m.problem.is_none() {
            m.problem = Some(format!("another folder also contains a mod named {:?}", m.name));
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// The names `modlist.toml` lists, in order (valid or not).
pub fn modlist() -> Vec<String> {
    fs::read_to_string(home().modlist_file())
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| d.get("enabled")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// Write `enabled` to `modlist.toml`, keeping its comments.
pub fn save_modlist(names: &[String]) -> Result<(), String> {
    let file = home().modlist_file();
    let text = fs::read_to_string(&file).unwrap_or_else(|_| MODLIST_TEMPLATE.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    let mut array = toml_edit::Array::new();
    for name in names {
        array.push(name.as_str());
    }
    doc["enabled"] = toml_edit::value(array);
    fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
}

/// The valid mods `modlist.toml` enables, in load order; the others are
/// reported and skipped. A mod whose requirements are missing or which is
/// incompatible with an enabled mod is skipped too; a load order its
/// relationships forbid is reported (`sort_modlist` fixes it).
pub fn enabled_mods(problems: &mut Vec<String>) -> Vec<ModInfo> {
    let installed = installed_mods();
    let mut mods: Vec<ModInfo> = Vec::new();
    for name in modlist() {
        if mods.iter().any(|m| m.name.eq_ignore_ascii_case(&name)) {
            continue;
        }
        match installed.iter().find(|m| m.name.eq_ignore_ascii_case(&name)) {
            None => problems.push(format!("Mod {name} skipped: it is not installed")),
            Some(m) => match &m.problem {
                Some(p) => problems.push(format!("Mod {name} skipped: {p}")),
                None => mods.push(m.clone()),
            },
        }
    }
    // Of two incompatible mods the later one in load order is skipped;
    // skipping a mod can leave another without its requirement: repeat.
    while let Some((i, p)) = mods.iter().enumerate().find_map(|(i, m)| relationship_problem(m, &mods[..i], &mods, &installed).map(|p| (i, p))) {
        problems.push(format!("Mod {} skipped: {p}", mods[i].name));
        mods.remove(i);
    }
    problems.extend(order_problems(&mods));
    mods
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// A missing requirement among the `enabled` mods, or an incompatible mod
/// among those loading `earlier`.
pub fn relationship_problem(m: &ModInfo, earlier: &[ModInfo], enabled: &[ModInfo], installed: &[ModInfo]) -> Option<String> {
    for (name, req) in &m.requires {
        match enabled.iter().find(|e| same(&e.name, name)) {
            Some(e) if !version_satisfies(&e.version, req) => return Some(format!("it needs {name} {req}, but {name} is version {}", e.version)),
            Some(_) => {}
            None if installed.iter().any(|i| same(&i.name, name)) => return Some(format!("it needs {name}; enable it")),
            None => return Some(format!("it needs {name}, which is not installed")),
        }
    }
    for e in earlier.iter().filter(|e| !same(&e.name, &m.name)) {
        if m.incompatible.iter().any(|n| same(n, &e.name)) || e.incompatible.iter().any(|n| same(n, &m.name)) {
            return Some(format!("it is incompatible with {}", e.name));
        }
    }
    None
}

/// Whether `a` must load before `b`: `b` requires or loads after `a`, or
/// `a` loads before `b`.
fn must_precede(a: &ModInfo, b: &ModInfo) -> bool {
    b.requires.iter().any(|(n, _)| same(n, &a.name)) || b.load_after.iter().any(|n| same(n, &a.name)) || a.load_before.iter().any(|n| same(n, &b.name))
}

/// Load-order relationships the order of `mods` breaks.
pub fn order_problems(mods: &[ModInfo]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in mods.iter().enumerate() {
        for b in &mods[..i] {
            if must_precede(a, b) {
                out.push(format!("Load order: {} must load after {}", b.name, a.name));
            }
        }
    }
    out
}

/// `names` reordered so every relationship holds, moving as little as
/// possible: at each step the earliest mod whose predecessors are placed.
/// Mods in a cycle keep their order.
pub fn sort_by_relationships(names: &[String], installed: &[ModInfo]) -> Vec<String> {
    let info = |n: &str| installed.iter().find(|m| same(&m.name, n));
    let mut left: Vec<String> = names.to_vec();
    let mut out = Vec::new();
    while !left.is_empty() {
        let next = (0..left.len())
            .find(|&i| {
                let Some(m) = info(&left[i]) else { return true };
                !left.iter().enumerate().any(|(j, o)| j != i && info(o).is_some_and(|o| must_precede(o, m)))
            })
            .unwrap_or(0);
        out.push(left.remove(next));
    }
    out
}

/// Reorder `modlist.toml` by the relationships.
pub fn sort_modlist() -> Result<(), String> {
    save_modlist(&sort_by_relationships(&modlist(), &installed_mods()))
}

/// `version` against a requirement: `*` (or empty), `=1.2.0`, `>=1.2`, or
/// `1.2` / `^1.2` (at least 1.2 within its compatibility range: the
/// left-most non-zero component matches, cargo's caret rule).
pub fn version_satisfies(version: &str, req: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> { v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect() };
    let pad = |v: Vec<u64>| [0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0));
    let req = req.trim();
    let have = pad(nums(version));
    if req.is_empty() || req == "*" {
        return true;
    }
    if let Some(r) = req.strip_prefix(">=") {
        return have >= pad(nums(r));
    }
    if let Some(r) = req.strip_prefix('=') {
        return have == pad(nums(r));
    }
    let want = pad(nums(req.trim_start_matches('^')));
    let first = want.iter().position(|&n| n != 0).unwrap_or(2);
    have >= want && have[..=first] == want[..=first]
}

/// Every file under `root` as (normalized resource path, file).
fn resource_files(root: &Path, out: &mut Vec<(String, PathBuf)>) {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_string_lossy().replace('\\', "/").to_ascii_lowercase(), path));
            }
        }
    }
    walk(root, root, out);
}

/// The enabled mods' resources above the game archives: each mod's
/// `resources/` in load order; a later mod's file wins.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    pub mods: Vec<ModInfo>,
    /// (resource path, file), the winning file per path.
    pub files: Vec<(String, PathBuf)>,
    pub problems: Vec<String>,
}

pub fn layers() -> Layers {
    let mut problems = Vec::new();
    let mods = enabled_mods(&mut problems);
    let mut files = std::collections::BTreeMap::new();
    for m in &mods {
        let mut layer = Vec::new();
        resource_files(&m.folder.join(MOD_RESOURCES), &mut layer);
        files.extend(layer);
    }
    Layers { mods, files: files.into_iter().collect(), problems }
}

/// BLAKE3 over length-prefixed parts.
pub fn hash_part(h: &mut blake3::Hasher, part: &[u8]) {
    h.update(&(part.len() as u64).to_le_bytes());
    h.update(part);
}

/// The setup fingerprint: the rewrite version, the installed game's active
/// indices, and every enabled mod (names, versions, patches, resource paths
/// and contents) in load order.
pub fn fingerprint(base: &Path, layers: &Layers) -> String {
    let mut h = blake3::Hasher::new();
    hash_part(&mut h, env!("CARGO_PKG_VERSION").as_bytes());
    for group in 1..=4 {
        let index = fs::read(base.join(format!("game{group}_patch.index"))).unwrap_or_default();
        hash_part(&mut h, &index);
    }
    for m in &layers.mods {
        hash_part(&mut h, m.name.as_bytes());
        hash_part(&mut h, m.version.as_bytes());
        for p in m.patches() {
            hash_part(&mut h, &fs::read(p).unwrap_or_default());
        }
    }
    for (key, path) in &layers.files {
        hash_part(&mut h, key.as_bytes());
        hash_part(&mut h, &fs::read(path).unwrap_or_default());
    }
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod relationship_tests {
    use super::*;

    fn m(name: &str, version: &str) -> ModInfo {
        ModInfo {
            name: name.into(),
            version: version.into(),
            author: String::new(),
            rewrite: String::new(),
            repository: String::new(),
            description: String::new(),
            saves: SaveSafety::Safe,
            requires: Vec::new(),
            incompatible: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            folder: PathBuf::new(),
            problem: None,
        }
    }

    #[test]
    fn versions_follow_the_requirement_forms() {
        assert!(version_satisfies("1.4.0", "1.2"));
        assert!(!version_satisfies("2.0.0", "1.2"));
        assert!(!version_satisfies("1.1.9", "1.2"));
        assert!(version_satisfies("0.3.5", "0.3.1"));
        assert!(!version_satisfies("0.4.0", "0.3"));
        assert!(version_satisfies("2.0.0", ">=1.2"));
        assert!(!version_satisfies("1.2.1", "=1.2.0"));
        assert!(version_satisfies("anything", "*"));
    }

    #[test]
    fn requirements_and_incompatibilities_are_reported() {
        let mut a = m("A", "1.0.0");
        a.requires = vec![("Core".into(), "2".into())];
        let core = m("Core", "1.5.0");
        assert_eq!(relationship_problem(&a, &[], &[core.clone()], &[core.clone()]).unwrap(), "it needs Core 2, but Core is version 1.5.0");
        assert_eq!(relationship_problem(&a, &[], &[], &[core.clone()]).unwrap(), "it needs Core; enable it");
        assert_eq!(relationship_problem(&a, &[], &[], &[]).unwrap(), "it needs Core, which is not installed");
        let mut b = m("B", "1.0.0");
        b.incompatible = vec!["core".into()];
        // The later of an incompatible pair is the one with the problem.
        assert_eq!(relationship_problem(&core, &[b.clone()], &[b.clone(), core.clone()], &[]).unwrap(), "it is incompatible with B");
        assert_eq!(relationship_problem(&b, &[], &[b.clone(), core.clone()], &[]), None);
    }

    #[test]
    fn sorting_moves_as_little_as_possible() {
        let mut a = m("A", "1");
        a.requires = vec![("Core".into(), "*".into())];
        let mut late = m("Late", "1");
        late.load_after = vec!["B".into()];
        let mut early = m("Early", "1");
        early.load_before = vec!["A".into()];
        let installed = [a.clone(), late, early, m("B", "1"), m("Core", "1")];
        let names: Vec<String> = ["Late", "A", "B", "Core", "Early"].map(String::from).to_vec();
        let sorted = sort_by_relationships(&names, &installed);
        assert_eq!(sorted, ["B", "Late", "Core", "Early", "A"]);
        let mods: Vec<ModInfo> = sorted.iter().map(|n| installed.iter().find(|m| &m.name == n).unwrap().clone()).collect();
        assert!(order_problems(&mods).is_empty());
        let unsorted: Vec<ModInfo> = names.iter().map(|n| installed.iter().find(|m| &m.name == n).unwrap().clone()).collect();
        assert!(order_problems(&unsorted).contains(&"Load order: A must load after Core".to_string()));
    }
}
