//! Update checks: the rewrite's newer release tags (or a newer default
//! branch), and each mod's repository tags against its `version`.
use super::source::{self, version_key};
use dishonored2_rust::platform::home::ModInfo;

#[derive(Clone, Debug, Default)]
pub struct Updates {
    /// A newer rewrite release tag, or "main" when the branch moved.
    pub rewrite: Option<String>,
    /// (mod name, newer version tag).
    pub mods: Vec<(String, String)>,
    pub problems: Vec<String>,
}

fn newest_tag(refs: &[(String, git2::Oid)]) -> Option<String> {
    refs.iter()
        .filter_map(|(name, _)| name.strip_prefix("refs/tags/"))
        .map(|t| t.trim_end_matches("^{}").to_string())
        .max_by(|a, b| version_key(a).cmp(&version_key(b)))
}

pub fn check(current_revision: &str, local_branch: Option<git2::Oid>, mods: &[ModInfo]) -> Updates {
    let mut u = Updates::default();
    match source::remote_refs(source::REPOSITORY) {
        Ok(refs) => {
            if let Some(tag) = newest_tag(&refs) {
                if version_key(&tag) > version_key(current_revision) && !version_key(current_revision).is_empty() {
                    u.rewrite = Some(tag);
                }
            }
            if version_key(current_revision).is_empty() {
                let remote = refs.iter().find(|(n, _)| *n == format!("refs/heads/{current_revision}")).map(|r| r.1);
                if remote.is_some() && remote != local_branch {
                    u.rewrite = Some(current_revision.to_string());
                }
            }
        }
        Err(e) => u.problems.push(format!("Update check for the rewrite failed: {e}")),
    }
    for m in mods.iter().filter(|m| !m.repository.is_empty()) {
        match source::remote_refs(&m.repository) {
            Ok(refs) => {
                if let Some(tag) = newest_tag(&refs) {
                    if version_key(&tag) > version_key(&m.version) {
                        u.mods.push((m.name.clone(), tag));
                    }
                }
            }
            Err(e) => u.problems.push(format!("Update check for {} failed: {e}", m.name)),
        }
    }
    u
}
