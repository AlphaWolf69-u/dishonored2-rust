//! The launcher's copy of the rewrite source: a git clone of the rewrite
//! repository in `source/`, fetched over HTTPS by the bundled libgit2.
use dishonored2_rust::platform::home;
use git2::{build::RepoBuilder, FetchOptions, Oid, RemoteCallbacks, Repository};

pub const REPOSITORY: &str = "https://github.com/AlphaWolf69-u/dishonored2-rust.git";
/// The rewrite revision setting in `config/rewrite.toml`.
const REVISION_SETTING: &str = "rewrite_revision";
const DEFAULT_BRANCH: &str = "main";

pub fn open() -> Option<Repository> {
    Repository::open(home::home().source_dir()).ok()
}

fn fetch_options<'a>(progress: &'a mut dyn FnMut(String)) -> FetchOptions<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.transfer_progress(move |p| {
        if p.total_objects() > 0 {
            progress(format!(
                "Downloading the rewrite source: {}% ({:.1} MB)",
                p.received_objects() * 100 / p.total_objects(),
                p.received_bytes() as f64 / 1_048_576.0
            ));
        }
        true
    });
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks).download_tags(git2::AutotagOption::All);
    options
}

/// Clone the repository into `source/`, or fetch its branches and tags.
pub fn download(progress: &mut dyn FnMut(String)) -> Result<(), String> {
    let dir = home::home().source_dir();
    match open() {
        Some(repo) => {
            let mut remote = repo.find_remote("origin").map_err(|e| e.message().to_string())?;
            remote
                .fetch(&["+refs/heads/*:refs/remotes/origin/*", "+refs/tags/*:refs/tags/*"], Some(&mut fetch_options(progress)), None)
                .map_err(|e| format!("Fetching the rewrite source failed: {}", e.message()))
        }
        None => {
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            RepoBuilder::new()
                .fetch_options(fetch_options(progress))
                .clone(REPOSITORY, &dir)
                .map(|_| ())
                .map_err(|e| format!("Downloading the rewrite source failed: {}", e.message()))
        }
    }
}

/// A revision (tag, branch or commit) as a commit id: tags, then the
/// fetched branches, then anything git understands.
pub fn resolve(repo: &Repository, revision: &str) -> Result<Oid, String> {
    for name in [format!("refs/tags/{revision}"), format!("refs/remotes/origin/{revision}")] {
        if let Ok(r) = repo.find_reference(&name) {
            if let Ok(c) = r.peel_to_commit() {
                return Ok(c.id());
            }
        }
    }
    repo.revparse_single(revision)
        .and_then(|o| o.peel_to_commit())
        .map(|c| c.id())
        .map_err(|_| format!("the rewrite revision {revision:?} is not in the downloaded source; update the source"))
}

/// Release tags, newest first (by version numbers in the tag).
pub fn release_tags(repo: &Repository) -> Vec<String> {
    let mut tags: Vec<String> = repo.tag_names(None).map(|t| t.iter().flatten().flatten().map(str::to_string).collect()).unwrap_or_default();
    tags.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    tags
}

/// The numbers of a version string (`v1.10.2` -> [1, 10, 2]).
pub fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect()
}

/// The revision the launcher builds: the setting, else the newest release
/// tag, else the default branch.
pub fn revision(repo: Option<&Repository>) -> String {
    home::setting(REVISION_SETTING)
        .or_else(|| repo.and_then(|r| release_tags(r).into_iter().next()))
        .unwrap_or_else(|| DEFAULT_BRANCH.to_string())
}

/// Move to a newer revision (a release tag): remembered in the settings.
pub fn set_revision(revision: &str) -> Result<(), String> {
    home::set_setting(REVISION_SETTING, revision)
}

/// The commit's short id and summary, for display.
pub fn describe(repo: &Repository, id: Oid) -> String {
    match repo.find_commit(id) {
        Ok(c) => format!("{} · {}", &id.to_string()[..8], c.summary().ok().flatten().unwrap_or("")),
        Err(_) => id.to_string()[..8].to_string(),
    }
}

/// A remote repository's references (`git ls-remote`), for update checks.
pub fn remote_refs(url: &str) -> Result<Vec<(String, Oid)>, String> {
    let mut remote = git2::Remote::create_detached(url).map_err(|e| e.message().to_string())?;
    remote.connect(git2::Direction::Fetch).map_err(|e| e.message().to_string())?;
    let refs = remote.list().map_err(|e| e.message().to_string())?.iter().map(|h| (h.name().to_string(), h.oid())).collect();
    Ok(refs)
}
