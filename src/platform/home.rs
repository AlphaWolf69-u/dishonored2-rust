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
/// reported and skipped.
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
    mods
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
