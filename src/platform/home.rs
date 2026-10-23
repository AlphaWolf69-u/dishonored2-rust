//! The rewrite's own folder (`DH2Rewrite/`, next to the executable): its
//! config, saves, mods, resource overrides, cache and logs. The installed
//! game is only read; its Saved Games config and saves are imported once.
//!
//! Host integration (a deliberate deviation): the original keeps its config
//! and saves under `Saved Games/Arkane Studios/Dishonored2/base`.
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

pub const CONFIG_FILE: &str = "dishonored2Config.cfg";
const SETTINGS_FILE: &str = "rewrite.toml";
const MODLIST_FILE: &str = "modlist.toml";
const MOD_MANIFEST: &str = "mod.toml";
const MOD_RESOURCES: &str = "resources";

const MODLIST_TEMPLATE: &str = "\
# Enabled mods: folder names under mods/, in load order.
# A later mod's files win over an earlier mod's; files in overrides/ win over every mod.
enabled = []
";

const SETTINGS_TEMPLATE: &str = "\
# Dishonored 2 rewrite settings.
# game_path: the installed game's \"base\" folder (the one containing game1_patch.index).
# Leave it out to look for a Steam installation.
";

const OVERRIDES_README: &str = "\
Resource overrides: a file here replaces the game resource with the same path.

Use the paths the game's archives use, for example
  overrides/strings/english_m.lang
  overrides/generated/decls/<type>/<name>.decl
  overrides/ui/corvo_pause_menu.iggy

Overrides win over every mod in modlist.toml. Remove a file to restore the original.
";

const MODS_README: &str = "\
One folder per mod:
  mods/<id>/mod.toml        id = \"<id>\", name, version
  mods/<id>/resources/...   files at the game's resource paths (like overrides/)

Enable mods in ../modlist.toml.
";

/// The folder layout.
#[derive(Clone, Debug)]
pub struct Home {
    pub root: PathBuf,
}

/// What the first run did, for the session log.
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
    pub fn overrides_dir(&self) -> PathBuf {
        self.root.join("overrides")
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
        for dir in [self.config_dir(), self.saves_dir(), self.mods_dir(), self.overrides_dir(), self.cache_dir(), self.logs_dir()] {
            note(fs::create_dir_all(&dir), &dir.display().to_string());
        }
        note(write_new(&self.modlist_file(), MODLIST_TEMPLATE), MODLIST_FILE);
        note(write_new(&self.settings_file(), SETTINGS_TEMPLATE), SETTINGS_FILE);
        note(write_new(&self.overrides_dir().join("README.txt"), OVERRIDES_README), "overrides/README.txt");
        note(write_new(&self.mods_dir().join("README.txt"), MODS_README), "mods/README.txt");
        setup
    }
}

/// The rewrite's folder, created (and imported into) on first use.
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

/// `config/rewrite.toml`'s `game_path`.
pub fn saved_game_path() -> Option<PathBuf> {
    let text = fs::read_to_string(home().settings_file()).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get("game_path")?.as_str().map(PathBuf::from)
}

/// Remember the game folder in `config/rewrite.toml`, keeping its comments.
pub fn save_game_path(path: &Path) -> Result<(), String> {
    let file = home().settings_file();
    let text = fs::read_to_string(&file).unwrap_or_else(|_| SETTINGS_TEMPLATE.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    doc["game_path"] = toml_edit::value(path.display().to_string());
    fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
}

/// One enabled mod.
#[derive(Clone, Debug, PartialEq)]
pub struct Mod {
    pub id: String,
    pub name: String,
    pub version: String,
    pub folder: PathBuf,
}

/// A relative path made only of normal components.
fn contained(path: &str) -> bool {
    !path.is_empty() && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

/// A mod folder under `mods/`, valid or not.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Why the folder cannot be enabled, if it cannot.
    pub problem: Option<String>,
}

fn read_manifest(id: &str, folder: &Path) -> Result<toml_edit::DocumentMut, String> {
    if !folder.is_dir() {
        return Err("the mod folder is not installed".into());
    }
    fs::read_to_string(folder.join(MOD_MANIFEST))
        .map_err(|e| if e.kind() == io::ErrorKind::NotFound { format!("{MOD_MANIFEST} is missing") } else { format!("{MOD_MANIFEST}: {e}") })
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{MOD_MANIFEST}: {e}")))
        .and_then(|m| {
            let field = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            if field("id") != id || field("version").is_empty() {
                Err(format!("{MOD_MANIFEST}: id must be {id:?} and version is required"))
            } else {
                Ok(m)
            }
        })
}

/// Every folder under `mods/`, by name.
pub fn installed_mods() -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(home().mods_dir()) else { return out };
    for entry in entries.flatten().filter(|e| e.path().is_dir()) {
        let id = entry.file_name().to_string_lossy().into_owned();
        let (name, version, description, problem) = match read_manifest(&id, &entry.path()) {
            Ok(m) => {
                let field = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                (field("name"), field("version"), field("description"), None)
            }
            Err(e) => (String::new(), String::new(), String::new(), Some(e)),
        };
        out.push(Installed { id, name, version, description, problem });
    }
    out.sort_by(|a, b| a.id.to_lowercase().cmp(&b.id.to_lowercase()));
    out
}

/// The ids `modlist.toml` lists, in order (valid or not).
pub fn modlist() -> Vec<String> {
    fs::read_to_string(home().modlist_file())
        .ok()
        .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|d| d.get("enabled")?.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

/// Write `enabled` to `modlist.toml`, keeping its comments.
pub fn save_modlist(ids: &[String]) -> Result<(), String> {
    let file = home().modlist_file();
    let text = fs::read_to_string(&file).unwrap_or_else(|_| MODLIST_TEMPLATE.to_string());
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{}: {e}", file.display()))?;
    let mut array = toml_edit::Array::new();
    for id in ids {
        array.push(id.as_str());
    }
    doc["enabled"] = toml_edit::value(array);
    fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
}

/// The mods `modlist.toml` enables, in load order. A mod that is missing or
/// whose `mod.toml` is invalid is reported and skipped.
pub fn enabled_mods(problems: &mut Vec<String>) -> Vec<Mod> {
    let home = home();
    let Ok(text) = fs::read_to_string(home.modlist_file()) else { return Vec::new() };
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            problems.push(format!("{MODLIST_FILE}: {e}"));
            return Vec::new();
        }
    };
    let Some(list) = doc.get("enabled").and_then(|v| v.as_array()) else { return Vec::new() };
    let mut mods = Vec::new();
    for id in list.iter().filter_map(|v| v.as_str()) {
        if !contained(id) || id.contains(['/', '\\']) || mods.iter().any(|m: &Mod| m.id == id) {
            problems.push(format!("{MODLIST_FILE}: invalid or repeated mod {id:?}"));
            continue;
        }
        let folder = home.mods_dir().join(id);
        let manifest = match read_manifest(id, &folder) {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("Mod {id} skipped: {e}"));
                continue;
            }
        };
        let field = |k: &str| manifest.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        mods.push(Mod { id: id.to_string(), name: field("name"), version: field("version"), folder });
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
                let key = rel.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
                if key != "readme.txt" {
                    out.push((key, path));
                }
            }
        }
    }
    walk(root, root, out);
}

/// The resource layers above the game archives, lowest first: each enabled
/// mod's `resources/`, then `overrides/`. A later layer's file wins.
#[derive(Clone, Debug, Default)]
pub struct Layers {
    pub mods: Vec<Mod>,
    /// (resource path, file), the winning file per path.
    pub files: Vec<(String, PathBuf)>,
    pub problems: Vec<String>,
}

pub fn layers() -> Layers {
    let mut problems = Vec::new();
    let mods = enabled_mods(&mut problems);
    let mut files = std::collections::BTreeMap::new();
    let mut layer_roots: Vec<PathBuf> = mods.iter().map(|m| m.folder.join(MOD_RESOURCES)).collect();
    layer_roots.push(home().overrides_dir());
    for root in layer_roots {
        let mut layer = Vec::new();
        resource_files(&root, &mut layer);
        files.extend(layer);
    }
    Layers { mods, files: files.into_iter().collect(), problems }
}

/// BLAKE3 over length-prefixed parts.
fn hash(h: &mut blake3::Hasher, part: &[u8]) {
    h.update(&(part.len() as u64).to_le_bytes());
    h.update(part);
}

/// The setup fingerprint: the rewrite version, the installed game's active
/// indices, and every enabled mod and override (ids, versions, resource
/// paths and contents). Load order counts: it decides which file wins.
pub fn fingerprint(base: &Path, layers: &Layers) -> String {
    let mut h = blake3::Hasher::new();
    hash(&mut h, env!("CARGO_PKG_VERSION").as_bytes());
    for group in 1..=4 {
        let index = fs::read(base.join(format!("game{group}_patch.index"))).unwrap_or_default();
        hash(&mut h, &index);
    }
    for m in &layers.mods {
        hash(&mut h, m.id.as_bytes());
        hash(&mut h, m.version.as_bytes());
    }
    for (key, path) in &layers.files {
        hash(&mut h, key.as_bytes());
        hash(&mut h, &fs::read(path).unwrap_or_default());
    }
    h.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_paths_must_stay_inside() {
        assert!(contained("ui/x.iggy"));
        assert!(!contained("../x"));
        assert!(!contained("C:/x"));
        assert!(!contained(""));
    }
}
