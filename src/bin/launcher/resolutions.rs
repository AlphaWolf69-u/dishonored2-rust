//! Merge conflicts between mods and how the user resolved them, kept in the
//! launcher's `resolutions/` folder. A resolution belongs to the exact
//! three versions of the file it resolved (the common version, the earlier
//! mods' and the mod's): it applies again on every build while they are
//! unchanged, and stops applying when a mod update changes that file.
//!
//! `resolutions/<id>/resolution.toml` records the file, the mods and the
//! choice (`earlier`, `mod` or `edited`); `resolutions/<id>/<file name>`
//! is the file with conflict markers to edit, for `edited`.
use git2::{MergeFileInput, MergeFileOptions, Oid, Repository};
use std::path::{Path, PathBuf};

/// One file two sides of a mod merge changed incompatibly.
#[derive(Clone, Debug)]
pub struct Conflict {
    /// The resolution key: a hash of the path and the three versions.
    pub id: String,
    pub path: String,
    /// The mod being merged.
    pub module: String,
    /// What it was merged onto: "the rewrite" or the earlier mods.
    pub earlier: String,
    /// More about a manifest conflict (the keys), or empty.
    pub detail: String,
    pub base: Option<Oid>,
    pub ours: Option<Oid>,
    pub theirs: Option<Oid>,
    pub choice: Option<Choice>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    /// Keep the earlier mods' version (or the rewrite's).
    Earlier,
    /// Take the mod's version.
    Mod,
    /// The edited file in the resolution folder.
    Edited,
}

impl Choice {
    pub fn name(self) -> &'static str {
        match self {
            Choice::Earlier => "earlier",
            Choice::Mod => "mod",
            Choice::Edited => "edited",
        }
    }

    pub fn parse(name: &str) -> Option<Choice> {
        [Choice::Earlier, Choice::Mod, Choice::Edited].into_iter().find(|c| c.name() == name)
    }
}

impl Conflict {
    pub fn describe(&self) -> String {
        let detail = if self.detail.is_empty() { String::new() } else { format!(" ({})", self.detail) };
        format!("{} conflicts with {} in {}{detail}", self.module, self.earlier, self.path)
    }
}

pub struct Store(pub PathBuf);

const RECORD: &str = "resolution.toml";

fn hex(id: Option<Oid>) -> String {
    id.map_or("-".into(), |i| i.to_string())
}

pub fn key(path: &str, base: Option<Oid>, ours: Option<Oid>, theirs: Option<Oid>) -> String {
    let mut h = blake3::Hasher::new();
    for part in [path.to_string(), hex(base), hex(ours), hex(theirs)] {
        dishonored2_rust::platform::home::hash_part(&mut h, part.as_bytes());
    }
    h.finalize().to_hex()[..16].to_string()
}

fn has_markers(text: &[u8]) -> bool {
    String::from_utf8_lossy(text).lines().any(|l| l.starts_with("<<<<<<<") || l.starts_with(">>>>>>>") || l == "=======")
}

impl Store {
    pub fn home() -> Store {
        Store(dishonored2_rust::platform::home::home().root.join("resolutions"))
    }

    fn folder(&self, id: &str) -> PathBuf {
        self.0.join(id)
    }

    /// The file to edit for `conflict`.
    pub fn working_file(&self, conflict: &Conflict) -> PathBuf {
        let name = Path::new(&conflict.path).file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "file".into());
        self.folder(&conflict.id).join(name)
    }

    pub fn choice(&self, id: &str) -> Option<Choice> {
        let text = std::fs::read_to_string(self.folder(id).join(RECORD)).ok()?;
        let doc: toml_edit::DocumentMut = text.parse().ok()?;
        Choice::parse(doc.get("choice")?.as_str()?)
    }

    /// What the resolution puts at the path: `Some(None)` removes the file,
    /// `None` when unresolved.
    pub fn resolved(&self, repo: &Repository, conflict: &Conflict) -> Result<Option<Option<Oid>>, String> {
        Ok(match conflict.choice {
            None => None,
            Some(Choice::Earlier) => Some(conflict.ours),
            Some(Choice::Mod) => Some(conflict.theirs),
            Some(Choice::Edited) => {
                let file = self.working_file(conflict);
                let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                Some(Some(repo.blob(&bytes).map_err(|e| e.message().to_string())?))
            }
        })
    }

    fn record(&self, conflict: &Conflict, choice: Option<Choice>) -> Result<(), String> {
        let folder = self.folder(&conflict.id);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let mut doc = toml_edit::DocumentMut::new();
        doc["path"] = toml_edit::value(conflict.path.as_str());
        doc["mod"] = toml_edit::value(conflict.module.as_str());
        doc["earlier"] = toml_edit::value(conflict.earlier.as_str());
        doc["choice"] = toml_edit::value(choice.map_or("", Choice::name));
        let file = folder.join(RECORD);
        std::fs::write(&file, doc.to_string()).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// Write the file with conflict markers to edit (kept if it exists, so
    /// an edit in progress survives) and return its path.
    pub fn open(&self, repo: &Repository, conflict: &Conflict) -> Result<PathBuf, String> {
        let file = self.working_file(conflict);
        if !file.exists() {
            let blob = |id: Option<Oid>| -> Result<Vec<u8>, String> {
                id.map_or(Ok(Vec::new()), |id| repo.find_blob(id).map(|b| b.content().to_vec()).map_err(|e| e.message().to_string()))
            };
            let (b, o, t) = (blob(conflict.base)?, blob(conflict.ours)?, blob(conflict.theirs)?);
            fn input<'a>(bytes: &'a [u8], path: &str) -> MergeFileInput<'a> {
                let mut i = MergeFileInput::new();
                i.content(bytes).path(path);
                i
            }
            let input = |bytes| input(bytes, &conflict.path);
            let mut options = MergeFileOptions::new();
            options.ancestor_label("common version").our_label(&conflict.earlier).their_label(&conflict.module);
            let merged = git2::merge_file(&input(&b), &input(&o), &input(&t), Some(&mut options)).map_err(|e| e.message().to_string())?;
            std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&file, merged.content()).map_err(|e| format!("{}: {e}", file.display()))?;
        }
        if self.choice(&conflict.id).is_none() {
            self.record(conflict, None)?;
        }
        Ok(file)
    }

    /// Resolve `conflict`. `Edited` needs the edited file without conflict
    /// markers (and, for a Cargo manifest, valid TOML).
    pub fn choose(&self, conflict: &Conflict, choice: Choice) -> Result<(), String> {
        if choice == Choice::Edited {
            let file = self.working_file(conflict);
            let bytes = std::fs::read(&file).map_err(|_| format!("{} has not been opened for editing", conflict.path))?;
            if has_markers(&bytes) {
                return Err(format!("{} still has conflict markers (<<<<<<<, =======, >>>>>>>)", file.display()));
            }
            if conflict.path.ends_with("Cargo.toml") {
                String::from_utf8_lossy(&bytes).parse::<toml_edit::DocumentMut>().map_err(|e| format!("{} is not valid TOML: {e}", file.display()))?;
            }
        }
        self.record(conflict, Some(choice))
    }

    /// Forget the resolution and the edited file.
    pub fn discard(&self, id: &str) -> Result<(), String> {
        let folder = self.folder(id);
        if folder.exists() {
            std::fs::remove_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        Ok(())
    }
}
