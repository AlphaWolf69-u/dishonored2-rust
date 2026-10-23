//! Putting the enabled mods' patches onto the rewrite: each mod's patches
//! are applied to the revision they were made against (`rewrite` in its
//! mod.toml), then that commit is merged three-way onto the result so far.
//! Cargo manifests both sides changed merge by key (`manifest`); a lock
//! file both changed keeps the merged side, and cargo adds what the other
//! side's dependencies need during the build.
use super::{manifest, source};
use dishonored2_rust::platform::home::ModInfo;
use git2::{build::CheckoutBuilder, Commit, Diff, Index, Oid, Repository, Signature, Tree};
use std::path::Path;

fn signature() -> Signature<'static> {
    Signature::now("Dishonored 2 rewrite launcher", "launcher@localhost").expect("signature")
}

/// The commit of one mod: its patches on its base revision.
fn mod_commit(repo: &Repository, m: &ModInfo) -> Result<Oid, String> {
    let base = repo.find_commit(source::resolve(repo, &m.rewrite).map_err(|e| format!("{}: {e}", m.name))?).map_err(|e| e.message().to_string())?;
    let mut tree = base.tree().map_err(|e| e.message().to_string())?;
    for patch in m.patches() {
        let bytes = std::fs::read(&patch).map_err(|e| format!("{}: {e}", patch.display()))?;
        let name = patch.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let diff = Diff::from_buffer(&bytes).map_err(|e| format!("{} {name}: not a patch: {}", m.name, e.message()))?;
        let mut index = repo
            .apply_to_tree(&tree, &diff, None)
            .map_err(|e| format!("{} {name} does not apply to rewrite {}: {}", m.name, m.rewrite, e.message()))?;
        let id = index.write_tree_to(repo).map_err(|e| e.message().to_string())?;
        tree = repo.find_tree(id).map_err(|e| e.message().to_string())?;
    }
    let sig = signature();
    repo.commit(None, &sig, &sig, &format!("{} {}", m.name, m.version), &tree, &[&base]).map_err(|e| e.message().to_string())
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
        let diff = repo.diff_tree_to_tree(Some(base), Some(tree), None).map_err(|e| e.message().to_string())?;
        Ok(diff
            .deltas()
            .filter_map(|d| d.new_file().path().or(d.old_file().path()).map(|p| p.to_string_lossy().replace('\\', "/")))
            .filter(|p| is_manifest(p) || is_lock(p))
            .collect())
    };
    let ours = changed(ours)?;
    Ok(changed(theirs)?.into_iter().filter(|p| ours.contains(p)).collect())
}

fn text(repo: &Repository, tree: &Tree, path: &str) -> Result<Option<String>, String> {
    let Ok(entry) = tree.get_path(Path::new(path)) else { return Ok(None) };
    let blob = repo.find_blob(entry.id()).map_err(|e| e.message().to_string())?;
    String::from_utf8(blob.content().to_vec()).map(Some).map_err(|_| format!("{path} is not UTF-8"))
}

/// Put `id` at `path` in the merge result, resolving any conflict there.
fn put(index: &mut Index, path: &str, id: Oid) -> Result<(), String> {
    let _ = index.conflict_remove(Path::new(path));
    let entry = git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index.add(&entry).map_err(|e| e.message().to_string())
}

/// Merge the Cargo files both sides changed into `index` (the textual merge
/// of `ours` and `theirs`). Errors name the conflicting manifest keys.
fn merge_cargo_files(repo: &Repository, index: &mut Index, ours: &Commit, theirs: &Commit) -> Result<(), String> {
    let base = repo.find_commit(repo.merge_base(ours.id(), theirs.id()).map_err(|e| e.message().to_string())?).map_err(|e| e.message().to_string())?;
    let trees = (base.tree(), ours.tree(), theirs.tree());
    let (Ok(b), Ok(o), Ok(t)) = trees else { return Err("reading the merged trees failed".into()) };
    for path in changed_on_both(repo, &b, &o, &t)? {
        let (bt, ot, tt) = (text(repo, &b, &path)?, text(repo, &o, &path)?, text(repo, &t, &path)?);
        if is_lock(&path) {
            if let Some(entry) = o.get_path(Path::new(&path)).ok() {
                put(index, &path, entry.id())?;
            }
            continue;
        }
        let (Some(ot), Some(tt)) = (ot, tt) else {
            return Err(format!("{path} is removed on one side and changed on the other"));
        };
        let merged = manifest::merge(bt.as_deref().unwrap_or(""), &ot, &tt).map_err(|keys| format!("{path}: {keys}"))?;
        put(index, &path, repo.blob(merged.as_bytes()).map_err(|e| e.message().to_string())?)?;
    }
    Ok(())
}

/// Merge the mods (load order) onto `target` and check the result out into
/// the source folder.
pub fn prepare(repo: &Repository, target: Oid, mods: &[ModInfo]) -> Result<Oid, String> {
    let current = merge_mods(repo, target, mods)?;
    let tree = repo.find_commit(current).and_then(|c| c.tree()).map_err(|e| e.message().to_string())?;
    repo.checkout_tree(tree.as_object(), Some(CheckoutBuilder::new().force().remove_untracked(true)))
        .map_err(|e| format!("Writing the source failed: {}", e.message()))?;
    repo.set_head_detached(current).map_err(|e| e.message().to_string())?;
    Ok(current)
}

/// The commit of `target` with the mods merged onto it in load order.
fn merge_mods(repo: &Repository, target: Oid, mods: &[ModInfo]) -> Result<Oid, String> {
    let sig = signature();
    let mut current = repo.find_commit(target).map_err(|e| e.message().to_string())?;
    let mut merged: Vec<&str> = Vec::new();
    for m in mods.iter().filter(|m| !m.patches().is_empty()) {
        let theirs = repo.find_commit(mod_commit(repo, m)?).map_err(|e| e.message().to_string())?;
        let mut index = repo.merge_commits(&current, &theirs, None).map_err(|e| e.message().to_string())?;
        let earlier = || if merged.is_empty() { "the rewrite".to_string() } else { merged.join(", ") };
        merge_cargo_files(repo, &mut index, &current, &theirs).map_err(|e| format!("{} conflicts with {} in {e}", m.name, earlier()))?;
        if index.has_conflicts() {
            let paths: Vec<String> = index
                .conflicts()
                .map_err(|e| e.message().to_string())?
                .flatten()
                .filter_map(|c| c.our.or(c.their).or(c.ancestor))
                .map(|e| String::from_utf8_lossy(&e.path).into_owned())
                .collect();
            return Err(format!("{} conflicts with {} in {}", m.name, earlier(), paths.join(", ")));
        }
        let tree = repo.find_tree(index.write_tree_to(repo).map_err(|e| e.message().to_string())?).map_err(|e| e.message().to_string())?;
        let id = repo
            .commit(None, &sig, &sig, &format!("Merge {}", m.name), &tree, &[&current, &theirs])
            .map_err(|e| e.message().to_string())?;
        current = repo.find_commit(id).map_err(|e| e.message().to_string())?;
        merged.push(&m.name);
    }
    Ok(current.id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dishonored2_rust::platform::home::SaveSafety;
    use std::path::PathBuf;

    const MANIFEST: &str = "[package]\nname = \"game\"\nversion = \"0.1.0\"\n\n[features]\ngame = []\n\n[dependencies]\nanyhow = \"1.0\"\n";
    const LOCK: &str = "version = 4\n\n[[package]]\nname = \"anyhow\"\nversion = \"1.0.0\"\n";

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
        text(repo, &tree, path).unwrap().unwrap()
    }

    #[test]
    fn mods_adding_dependencies_in_the_same_place_merge() {
        let dir = scratch("deps");
        let repo = Repository::init_bare(dir.join("repo")).unwrap();
        let base = commit(&repo, None, &[("Cargo.toml", MANIFEST), ("Cargo.lock", LOCK)]);
        let a = make_mod(&repo, &dir, base, "A", &[
            ("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.9\"\n").replace("game = []", "game = [\"dep:rand\"]")),
            ("Cargo.lock", &format!("{LOCK}\n[[package]]\nname = \"rand\"\nversion = \"0.9.0\"\n")),
        ]);
        let b = make_mod(&repo, &dir, base, "B", &[
            ("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nserde = \"1\"\n").replace("game = []", "game = [\"dep:serde\"]")),
            ("Cargo.lock", &format!("{LOCK}\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n")),
        ]);
        let merged = merge_mods(&repo, base, &[a, b]).unwrap();
        let manifest: toml_edit::DocumentMut = file(&repo, merged, "Cargo.toml").parse().unwrap();
        assert_eq!(manifest["dependencies"]["rand"].as_str(), Some("0.9"));
        assert_eq!(manifest["dependencies"]["serde"].as_str(), Some("1"));
        assert_eq!(manifest["features"]["game"].as_array().unwrap().len(), 2);
        // The lock file keeps the first mod's; cargo adds serde at build time.
        assert!(file(&repo, merged, "Cargo.lock").contains("rand"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incompatible_dependency_versions_name_the_mods_and_key() {
        let dir = scratch("conflict");
        let repo = Repository::init_bare(dir.join("repo")).unwrap();
        let base = commit(&repo, None, &[("Cargo.toml", MANIFEST)]);
        let a = make_mod(&repo, &dir, base, "A", &[("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.8\"\n"))]);
        let b = make_mod(&repo, &dir, base, "B", &[("Cargo.toml", &MANIFEST.replace("anyhow = \"1.0\"\n", "anyhow = \"1.0\"\nrand = \"0.9\"\n"))]);
        let error = merge_mods(&repo, base, &[a, b]).unwrap_err();
        assert_eq!(error, "B conflicts with A in Cargo.toml: dependencies.rand (version \"0.8\" and \"0.9\" are not compatible)");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
