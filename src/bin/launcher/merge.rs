//! Putting the enabled mods' patches onto the rewrite: each mod's patches
//! are applied to the revision they were made against (`rewrite` in its
//! mod.toml), then that commit is merged three-way onto the result so far.
use super::source;
use dishonored2_rust::platform::home::ModInfo;
use git2::{build::CheckoutBuilder, Diff, Oid, Repository, Signature};

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

/// Merge the mods (load order) onto `target` and check the result out into
/// the source folder. A conflict only in `Cargo.lock` keeps the merged
/// side; cargo updates the lock file when the build needs it.
pub fn prepare(repo: &Repository, target: Oid, mods: &[ModInfo]) -> Result<Oid, String> {
    let sig = signature();
    let mut current = repo.find_commit(target).map_err(|e| e.message().to_string())?;
    let mut merged: Vec<&str> = Vec::new();
    for m in mods.iter().filter(|m| !m.patches().is_empty()) {
        let theirs = repo.find_commit(mod_commit(repo, m)?).map_err(|e| e.message().to_string())?;
        let mut index = repo.merge_commits(&current, &theirs, None).map_err(|e| e.message().to_string())?;
        if index.has_conflicts() {
            let paths: Vec<String> = index
                .conflicts()
                .map_err(|e| e.message().to_string())?
                .flatten()
                .filter_map(|c| c.our.or(c.their).or(c.ancestor))
                .map(|e| String::from_utf8_lossy(&e.path).into_owned())
                .collect();
            if paths.iter().all(|p| p == "Cargo.lock") {
                index.remove_path(std::path::Path::new("Cargo.lock")).map_err(|e| e.message().to_string())?;
                if let Some(ours) = current.tree().ok().and_then(|t| t.get_path(std::path::Path::new("Cargo.lock")).ok()) {
                    let entry = git2::IndexEntry {
                        ctime: git2::IndexTime::new(0, 0),
                        mtime: git2::IndexTime::new(0, 0),
                        dev: 0,
                        ino: 0,
                        mode: ours.filemode() as u32,
                        uid: 0,
                        gid: 0,
                        file_size: 0,
                        id: ours.id(),
                        flags: 0,
                        flags_extended: 0,
                        path: b"Cargo.lock".to_vec(),
                    };
                    index.add(&entry).map_err(|e| e.message().to_string())?;
                }
            } else {
                let earlier = if merged.is_empty() { "the rewrite".to_string() } else { merged.join(", ") };
                return Err(format!("{} conflicts with {earlier} in {}", m.name, paths.join(", ")));
            }
        }
        let tree = repo.find_tree(index.write_tree_to(repo).map_err(|e| e.message().to_string())?).map_err(|e| e.message().to_string())?;
        let id = repo
            .commit(None, &sig, &sig, &format!("Merge {}", m.name), &tree, &[&current, &theirs])
            .map_err(|e| e.message().to_string())?;
        current = repo.find_commit(id).map_err(|e| e.message().to_string())?;
        merged.push(&m.name);
    }
    let tree = current.tree().map_err(|e| e.message().to_string())?;
    repo.checkout_tree(tree.as_object(), Some(CheckoutBuilder::new().force().remove_untracked(true)))
        .map_err(|e| format!("Writing the source failed: {}", e.message()))?;
    repo.set_head_detached(current.id()).map_err(|e| e.message().to_string())?;
    Ok(current.id())
}
