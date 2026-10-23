//! Putting the enabled mods' patches onto the rewrite: each mod's patches
//! are applied to the revision they were made against (`rewrite` in its
//! mod.toml), then that commit is merged three-way onto the result so far.
//! Cargo manifests both sides changed merge by key (`manifest`); a lock
//! file both changed keeps the merged side, and cargo adds what the other
//! side's dependencies need during the build.
//!
//! Every conflict is recorded (`resolutions::Conflict`); a stored
//! resolution of it applies, otherwise the merge continues with the earlier
//! side so the later mods' conflicts are found too, and `prepare` refuses to
//! build until all are resolved.
use super::resolutions::{self, Conflict, Store};
use super::{manifest, source};
use dishonored2_rust::platform::home::ModInfo;
use git2::{build::CheckoutBuilder, Commit, Diff, Index, Oid, Repository, Signature, Tree};
use std::path::Path;

fn signature() -> Signature<'static> {
    Signature::now("Dishonored 2 rewrite launcher", "launcher@localhost").expect("signature")
}

fn git(e: git2::Error) -> String {
    e.message().to_string()
}

/// The commit of one mod: its patches on its base revision.
fn mod_commit(repo: &Repository, m: &ModInfo) -> Result<Oid, String> {
    let base = repo.find_commit(source::resolve(repo, &m.rewrite).map_err(|e| format!("{}: {e}", m.name))?).map_err(git)?;
    let mut tree = base.tree().map_err(git)?;
    for patch in m.patches() {
        let bytes = std::fs::read(&patch).map_err(|e| format!("{}: {e}", patch.display()))?;
        let name = patch.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let diff = Diff::from_buffer(&bytes).map_err(|e| format!("{} {name}: not a patch: {}", m.name, e.message()))?;
        let mut index = repo
            .apply_to_tree(&tree, &diff, None)
            .map_err(|e| format!("{} {name} does not apply to rewrite {}: {}", m.name, m.rewrite, e.message()))?;
        let id = index.write_tree_to(repo).map_err(git)?;
        tree = repo.find_tree(id).map_err(git)?;
    }
    let sig = signature();
    repo.commit(None, &sig, &sig, &format!("{} {}", m.name, m.version), &tree, &[&base]).map_err(git)
}

fn is_manifest(path: &str) -> bool {
    path == "Cargo.toml" || path.ends_with("/Cargo.toml")
}

fn is_lock(path: &str) -> bool {
    path == "Cargo.lock" || path.ends_with("/Cargo.lock")
}

/// The Cargo files changed on both sides since `base`.
fn changed_on_both(repo: &Repository, base: &Tree, ours: &Tree, theirs: &Tree) -> Result<Vec<String>, String> {
    let changed = |tree: &Tree| -> Result<Vec<String>, String> {
        let diff = repo.diff_tree_to_tree(Some(base), Some(tree), None).map_err(git)?;
        Ok(diff
            .deltas()
            .filter_map(|d| d.new_file().path().or(d.old_file().path()).map(|p| p.to_string_lossy().replace('\\', "/")))
            .filter(|p| is_manifest(p) || is_lock(p))
            .collect())
    };
    let ours = changed(ours)?;
    Ok(changed(theirs)?.into_iter().filter(|p| ours.contains(p)).collect())
}

fn blob_at(tree: &Tree, path: &str) -> Option<(Oid, u32)> {
    tree.get_path(Path::new(path)).ok().map(|e| (e.id(), e.filemode() as u32))
}

fn text(repo: &Repository, id: Option<Oid>, path: &str) -> Result<Option<String>, String> {
    let Some(id) = id else { return Ok(None) };
    let blob = repo.find_blob(id).map_err(git)?;
    String::from_utf8(blob.content().to_vec()).map(Some).map_err(|_| format!("{path} is not UTF-8"))
}

/// Put `id` at `path` in the merge result (`None` removes it), resolving
/// any conflict there.
fn put(index: &mut Index, path: &str, id: Option<Oid>, mode: u32) -> Result<(), String> {
    let _ = index.conflict_remove(Path::new(path));
    let Some(id) = id else {
        let _ = index.remove_path(Path::new(path));
        return Ok(());
    };
    let entry = git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: 0,
        id,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add(&entry).map_err(git)
}

/// The enabled mods merged onto the rewrite, and every conflict met.
pub struct Merged {
    pub commit: Oid,
    pub conflicts: Vec<Conflict>,
}

impl Merged {
    pub fn unresolved(&self) -> impl Iterator<Item = &Conflict> {
        self.conflicts.iter().filter(|c| c.choice.is_none())
    }
}

struct Step<'a> {
    repo: &'a Repository,
    store: &'a Store,
    module: &'a str,
    earlier: String,
    conflicts: Vec<Conflict>,
}

impl Step<'_> {
    /// Record a conflict; apply its resolution, or keep the earlier side.
    fn conflict(&mut self, index: &mut Index, path: &str, ids: [Option<Oid>; 3], mode: u32, detail: String) -> Result<(), String> {
        let [base, ours, theirs] = ids;
        let id = resolutions::key(path, base, ours, theirs);
        let mut c = Conflict { choice: self.store.choice(&id), id, path: path.into(), module: self.module.into(), earlier: self.earlier.clone(), detail, base, ours, theirs };
        let resolved = self.store.resolved(self.repo, &c);
        let put_id = match resolved {
            Ok(Some(id)) => id,
            Ok(None) => ours,
            Err(e) => {
                // The edited file is gone: unresolved again.
                c.choice = None;
                c.detail = if c.detail.is_empty() { e } else { format!("{}; {e}", c.detail) };
                ours
            }
        };
        put(index, path, put_id, mode)?;
        self.conflicts.push(c);
        Ok(())
    }

    /// Merge the Cargo files both sides changed into `index` (the textual
    /// merge of `ours` and `theirs`).
    fn cargo_files(&mut self, index: &mut Index, ours: &Commit, theirs: &Commit) -> Result<(), String> {
        let repo = self.repo;
        let base = repo.find_commit(repo.merge_base(ours.id(), theirs.id()).map_err(git)?).map_err(git)?;
        let (b, o, t) = (base.tree().map_err(git)?, ours.tree().map_err(git)?, theirs.tree().map_err(git)?);
        for path in changed_on_both(repo, &b, &o, &t)? {
            let (bb, ob, tb) = (blob_at(&b, &path), blob_at(&o, &path), blob_at(&t, &path));
            let mode = ob.or(tb).map_or(0o100644, |e| e.1);
            if is_lock(&path) {
                if ob.is_some() {
                    put(index, &path, ob.map(|e| e.0), mode)?;
                }
                continue;
            }
            let ids = [bb.map(|e| e.0), ob.map(|e| e.0), tb.map(|e| e.0)];
            let (bt, ot, tt) = (text(repo, ids[0], &path)?, text(repo, ids[1], &path)?, text(repo, ids[2], &path)?);
            let (Some(ot), Some(tt)) = (ot, tt) else {
                self.conflict(index, &path, ids, mode, "removed on one side, changed on the other".into())?;
                continue;
            };
            match manifest::merge(bt.as_deref().unwrap_or(""), &ot, &tt) {
                Ok(merged) => put(index, &path, Some(repo.blob(merged.as_bytes()).map_err(git)?), mode)?,
                Err(keys) => self.conflict(index, &path, ids, mode, keys)?,
            }
        }
        Ok(())
    }
}

/// `target` with the mods merged onto it in load order, using the stored
/// resolutions.
pub fn merge_mods(repo: &Repository, target: Oid, mods: &[ModInfo], store: &Store) -> Result<Merged, String> {
    let sig = signature();
    let mut current = repo.find_commit(target).map_err(git)?;
    let mut merged: Vec<&str> = Vec::new();
    let mut conflicts = Vec::new();
    for m in mods.iter().filter(|m| !m.patches().is_empty()) {
        let theirs = repo.find_commit(mod_commit(repo, m)?).map_err(git)?;
        let mut index = repo.merge_commits(&current, &theirs, None).map_err(git)?;
        let earlier = if merged.is_empty() { "the rewrite".to_string() } else { format!("the rewrite with {}", merged.join(", ")) };
        let mut step = Step { repo, store, module: &m.name, earlier, conflicts: Vec::new() };
        step.cargo_files(&mut index, &current, &theirs)?;
        let textual: Vec<git2::IndexConflict> = index.conflicts().map_err(git)?.flatten().collect();
        for c in textual {
            let Some(entry) = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref()) else { continue };
            let path = String::from_utf8_lossy(&entry.path).into_owned();
            let ids = [c.ancestor.as_ref().map(|e| e.id), c.our.as_ref().map(|e| e.id), c.their.as_ref().map(|e| e.id)];
            step.conflict(&mut index, &path, ids, entry.mode, String::new())?;
        }
        conflicts.append(&mut step.conflicts);
        let tree = repo.find_tree(index.write_tree_to(repo).map_err(git)?).map_err(git)?;
        let id = repo.commit(None, &sig, &sig, &format!("Merge {}", m.name), &tree, &[&current, &theirs]).map_err(git)?;
        current = repo.find_commit(id).map_err(git)?;
        merged.push(&m.name);
    }
    Ok(Merged { commit: current.id(), conflicts })
}

/// Merge the mods (load order) onto `target` and check the result out into
/// the source folder; every conflict must be resolved.
pub fn prepare(repo: &Repository, target: Oid, mods: &[ModInfo]) -> Result<Oid, String> {
    let merged = merge_mods(repo, target, mods, &Store::home())?;
    let open: Vec<String> = merged.unresolved().map(Conflict::describe).collect();
    if !open.is_empty() {
        return Err(format!("{} unresolved conflict(s): {}. Resolve them first.", open.len(), open.join("; ")));
    }
    let tree = repo.find_commit(merged.commit).and_then(|c| c.tree()).map_err(git)?;
    repo.checkout_tree(tree.as_object(), Some(CheckoutBuilder::new().force().remove_untracked(true)))
        .map_err(|e| format!("Writing the source failed: {}", e.message()))?;
    repo.set_head_detached(merged.commit).map_err(git)?;
    Ok(merged.commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dishonored2_rust::platform::home::SaveSafety;
    use resolutions::Choice;
    use std::path::PathBuf;

    const MANIFEST: &str = "[package]\nname = \"game\"\nversion = \"0.1.0\"\n\n[features]\ngame = []\n\n[dependencies]\nanyhow = \"1.0\"\n";
    const LOCK: &str = "version = 4\n\n[[package]]\nname = \"anyhow\"\nversion = \"1.0.0\"\n";
    const CODE: &str = "fn speed() -> f32 {\n    1.0\n}\n";

    fn commit(repo: &Repository, parent: Option<&Commit>, files: &[(&str, &str)]) -> Oid {
        let mut builder = repo.treebuilder(parent.map(|p| p.tree().unwrap()).as_ref()).unwrap();
        for (path, text) in files {
            builder.insert(path, repo.blob(text.as_bytes()).unwrap(), 0o100644).unwrap();
        }
        let tree = repo.find_tree(builder.write().unwrap()).unwrap();
        let parents: Vec<&Commit> = parent.into_iter().collect();
        repo.commit(None, &signature(), &signature(), "test", &tree, &parents).unwrap()
    }

    /// A mod folder whose patch changes the base's files to `files`.
    fn make_mod(repo: &Repository, dir: &Path, base: Oid, name: &str, files: &[(&str, &str)]) -> ModInfo {
        let base_commit = repo.find_commit(base).unwrap();
        let changed = repo.find_commit(commit(repo, Some(&base_commit), files)).unwrap();
        let diff = repo.diff_tree_to_tree(Some(&base_commit.tree().unwrap()), Some(&changed.tree().unwrap()), None).unwrap();
        let folder = dir.join(name);
        std::fs::create_dir_all(folder.join("patches")).unwrap();
        let mut patch = Vec::new();
        diff.print(git2::DiffFormat::Patch, |_, _, line| {
            if matches!(line.origin(), '+' | '-' | ' ') {
                patch.push(line.origin() as u8);
            }
            patch.extend_from_slice(line.content());
            true
        })
        .unwrap();
        std::fs::write(folder.join("patches/1.patch"), patch).unwrap();
        ModInfo {
            name: name.into(),
            version: "1.0.0".into(),
            author: String::new(),
            rewrite: base.to_string(),
            repository: String::new(),
            description: String::new(),
            saves: SaveSafety::Safe,
            requires: Vec::new(),
            incompatible: Vec::new(),
            load_after: Vec::new(),
            load_before: Vec::new(),
            folder,
            problem: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dh2-launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(repo: &Repository, id: Oid, path: &str) -> String {
        let tree = repo.find_commit(id).unwrap().tree().unwrap();
        text(repo, blob_at(&tree, path).map(|e| e.0), path).unwrap().unwrap()
    }

    #[test]
    fn mods_adding_dependencies_in_the_same_place_merge() {
        let dir = scratch("deps");
        let repo = Repository::init_bare(dir.join("repo")).unwrap();
        let store = Store(dir.join("resolutions"));
        let base = commit(&repo, None, &[("Cargo.toml", MANIFEST), ("Cargo.lock", LOCK)]);
        let a = make_mod(&repo, &dir, base, "A", &[
            ("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.9\"\n").replace("game = []", "game = [\"dep:rand\"]")),
            ("Cargo.lock", &format!("{LOCK}\n[[package]]\nname = \"rand\"\nversion = \"0.9.0\"\n")),
        ]);
        let b = make_mod(&repo, &dir, base, "B", &[
            ("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nserde = \"1\"\n").replace("game = []", "game = [\"dep:serde\"]")),
            ("Cargo.lock", &format!("{LOCK}\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n")),
        ]);
        let merged = merge_mods(&repo, base, &[a, b], &store).unwrap();
        assert!(merged.conflicts.is_empty());
        let manifest: toml_edit::DocumentMut = file(&repo, merged.commit, "Cargo.toml").parse().unwrap();
        assert_eq!(manifest["dependencies"]["rand"].as_str(), Some("0.9"));
        assert_eq!(manifest["dependencies"]["serde"].as_str(), Some("1"));
        assert_eq!(manifest["features"]["game"].as_array().unwrap().len(), 2);
        // The lock file keeps the first mod's; cargo adds serde at build time.
        assert!(file(&repo, merged.commit, "Cargo.lock").contains("rand"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incompatible_dependency_versions_name_the_mods_and_key() {
        let dir = scratch("conflict");
        let repo = Repository::init_bare(dir.join("repo")).unwrap();
        let store = Store(dir.join("resolutions"));
        let base = commit(&repo, None, &[("Cargo.toml", MANIFEST)]);
        let a = make_mod(&repo, &dir, base, "A", &[("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.8\"\n"))]);
        let b = make_mod(&repo, &dir, base, "B", &[("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.9\"\n"))]);
        let merged = merge_mods(&repo, base, &[a, b], &store).unwrap();
        let open: Vec<String> = merged.unresolved().map(Conflict::describe).collect();
        assert_eq!(open, ["B conflicts with the rewrite with A in Cargo.toml (dependencies.rand (version \"0.8\" and \"0.9\" are not compatible))"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conflicts_are_all_found_and_resolutions_apply_again() {
        let dir = scratch("resolve");
        let repo = Repository::init_bare(dir.join("repo")).unwrap();
        let store = Store(dir.join("resolutions"));
        let base = commit(&repo, None, &[("speed.rs", CODE), ("jump.rs", CODE)]);
        let a = make_mod(&repo, &dir, base, "A", &[("speed.rs", &CODE.replace("1.0", "1.5")), ("jump.rs", &CODE.replace("1.0", "2.0"))]);
        let b = make_mod(&repo, &dir, base, "B", &[("speed.rs", &CODE.replace("1.0", "3.0")), ("jump.rs", &CODE.replace("1.0", "4.0"))]);
        let mods = [a, b];
        let merged = merge_mods(&repo, base, &mods, &store).unwrap();
        assert_eq!(merged.unresolved().count(), 2);
        let speed = merged.conflicts.iter().find(|c| c.path == "speed.rs").unwrap().clone();
        let jump = merged.conflicts.iter().find(|c| c.path == "jump.rs").unwrap().clone();

        // Editing: the working file has labelled markers and must lose them.
        let working = store.open(&repo, &speed).unwrap();
        let marked = std::fs::read_to_string(&working).unwrap();
        assert!(marked.contains("<<<<<<< the rewrite with A") && marked.contains(">>>>>>> B"), "{marked}");
        assert!(store.choose(&speed, Choice::Edited).unwrap_err().contains("still has conflict markers"));
        std::fs::write(&working, CODE.replace("1.0", "2.25")).unwrap();
        store.choose(&speed, Choice::Edited).unwrap();
        store.choose(&jump, Choice::Mod).unwrap();

        let merged = merge_mods(&repo, base, &mods, &store).unwrap();
        assert_eq!(merged.unresolved().count(), 0);
        assert!(file(&repo, merged.commit, "speed.rs").contains("2.25"));
        assert!(file(&repo, merged.commit, "jump.rs").contains("4.0"));

        // A mod changing the file again makes its resolution stale.
        let b2 = make_mod(&repo, &dir, base, "B", &[("speed.rs", &CODE.replace("1.0", "3.5")), ("jump.rs", &CODE.replace("1.0", "4.0"))]);
        let merged = merge_mods(&repo, base, &[mods[0].clone(), b2], &store).unwrap();
        let open: Vec<&str> = merged.unresolved().map(|c| c.path.as_str()).collect();
        assert_eq!(open, ["speed.rs"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
