//! What the launcher does, for the window and the command line alike.
use super::merge::{self, Merged};
use super::resolutions::Store;
use super::{build, source};
use dishonored2_rust::assets::installation;
use dishonored2_rust::platform::home::{self, ModInfo};
use std::path::PathBuf;

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Enable or disable a mod. Enabling also enables the installed mods it
/// requires; the list is then put in an order its relationships allow.
/// Returns what changed.
pub fn set_enabled(name: &str, on: bool) -> Result<String, String> {
    let installed = home::installed_mods();
    let m = installed.iter().find(|m| same(&m.name, name)).ok_or_else(|| format!("No mod named {name} is installed."))?;
    let mut list = home::modlist();
    list.retain(|n| !same(n, &m.name));
    let mut notes = Vec::new();
    if on {
        list.push(m.name.clone());
        let mut queue = vec![m.clone()];
        while let Some(next) = queue.pop() {
            for (req, _) in &next.requires {
                if list.iter().any(|n| same(n, req)) {
                    continue;
                }
                if let Some(r) = installed.iter().find(|i| same(&i.name, req)) {
                    list.push(r.name.clone());
                    notes.push(format!("also enabled {} (required by {})", r.name, next.name));
                    queue.push(r.clone());
                }
            }
        }
        list = home::sort_by_relationships(&list, &installed);
    } else {
        let needed_by: Vec<&str> = installed
            .iter()
            .filter(|i| list.iter().any(|n| same(n, &i.name)) && i.requires.iter().any(|(r, _)| same(r, &m.name)))
            .map(|i| i.name.as_str())
            .collect();
        if !needed_by.is_empty() {
            notes.push(format!("{} need it and will be skipped", needed_by.join(", ")));
        }
    }
    home::save_modlist(&list)?;
    if on {
        let mut problems = Vec::new();
        home::enabled_mods(&mut problems);
        notes.extend(problems.iter().filter(|p| p.starts_with("Mod ") && p.contains(" skipped: ")).cloned());
    }
    let done = format!("{} {}", if on { "Enabled" } else { "Disabled" }, m.name);
    Ok(if notes.is_empty() { format!("{done}.") } else { format!("{done}; {}.", notes.join("; ")) })
}

/// Move an enabled mod to `index` in the load order, then restore the order
/// its relationships require (the rest of the order stays as placed).
pub fn move_to(name: &str, index: usize) -> Result<String, String> {
    let mut list = home::modlist();
    let from = list.iter().position(|n| same(n, name)).ok_or_else(|| format!("{name} is not enabled."))?;
    let moved = list.remove(from);
    list.insert(index.min(list.len()), moved.clone());
    let sorted = home::sort_by_relationships(&list, &home::installed_mods());
    let note = if sorted != list { " Its relationships keep some mods in their required order." } else { "" };
    home::save_modlist(&sorted)?;
    let at = sorted.iter().position(|n| *n == moved).unwrap_or(0) + 1;
    Ok(format!("{moved} now loads {at} of {}.{note}", sorted.len()))
}

/// The enabled mods merged in memory (nothing is checked out): the
/// conflicts and their resolutions. `None` when no enabled mod has patches,
/// or the source is a local checkout (which mods with patches do not use).
pub fn merge_check() -> Result<Option<Merged>, String> {
    if source::local().is_some() {
        return Ok(None);
    }
    let mut problems = Vec::new();
    let mods = home::enabled_mods(&mut problems);
    if mods.iter().all(|m| m.patches().is_empty()) {
        return Ok(None);
    }
    let repo = source::open().ok_or("Download the rewrite source first.")?;
    let target = source::resolve(&repo, &source::revision(Some(&repo)))?;
    merge::merge_mods(&repo, target, &mods, &Store::home()).map(Some)
}

/// The enabled mods, or the first problem with them.
fn usable_mods() -> Result<Vec<ModInfo>, String> {
    let mut problems = Vec::new();
    let mods = home::enabled_mods(&mut problems);
    match problems.first() {
        Some(p) => Err(format!("{p}. Fix or disable it first.")),
        None => Ok(mods),
    }
}

/// Merge and build if anything changed since the last build. Returns
/// whether a build ran.
pub fn build(cargo: Option<PathBuf>, progress: &mut dyn FnMut(String)) -> Result<bool, String> {
    let repo = source::open().ok_or("Download the rewrite source first.")?;
    installation::game_base().map_err(|_| "Choose the game folder first.".to_string())?;
    let mods = usable_mods()?;
    let cargo = || cargo.clone().ok_or("Install Rust and the C++ Build Tools to build the rewrite.");
    if let Some(checkout) = source::local() {
        // The checkout is the developer's work: merging mods into it would
        // replace that, so only resource mods apply.
        if let Some(m) = mods.iter().find(|m| !m.patches().is_empty()) {
            return Err(format!("{} changes the source; mods with patches are not merged into a local source. Disable it, or use the download.", m.name));
        }
        progress("Building the local source…".into());
        build::build_local(&cargo()?, &checkout, progress)?;
        return Ok(true);
    }
    let target = source::resolve(&repo, &source::revision(Some(&repo)))?;
    let fingerprint = build::fingerprint(&target.to_string(), &mods);
    if build::built().as_deref() == Some(fingerprint.as_str()) {
        return Ok(false);
    }
    let cargo = cargo()?;
    progress("Merging the enabled mods into the rewrite source…".into());
    merge::prepare(&repo, target, &mods)?;
    progress("Building the rewrite (the first build takes a while)…".into());
    build::build(&cargo, &fingerprint, progress)?;
    Ok(true)
}

/// Build if needed and start the game.
pub fn launch(cargo: Option<PathBuf>, progress: &mut dyn FnMut(String)) -> Result<String, String> {
    build(cargo, progress)?;
    start()
}

/// Start the built game in the rewrite folder.
pub fn start() -> Result<String, String> {
    let exe = build::game_exe();
    std::process::Command::new(&exe)
        .env("DH2_REWRITE_HOME", &home::home().root)
        .current_dir(&home::home().root)
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
    Ok("Game started.".into())
}

/// Download the source, or fetch it; with `update`, also move to the newer
/// release tag `newer` when there is one.
pub fn download(newer: Option<String>, progress: &mut dyn FnMut(String)) -> Result<String, String> {
    source::download(progress)?;
    if let Some(newer) = newer.filter(|v| !source::version_key(v).is_empty()) {
        source::set_revision(&newer)?;
        return Ok(format!("Updated the rewrite to {newer}."));
    }
    Ok("The rewrite source is downloaded and current.".into())
}

/// Use `picked` (the game folder or its `base`) as the installed game.
pub fn set_game_folder(picked: &std::path::Path) -> Result<String, String> {
    let base = installation::base_of(picked).ok_or_else(|| format!("{} has no Dishonored 2 game files (base\\game1_patch.index).", picked.display()))?;
    home::save_game_path(&base)?;
    Ok(format!("Game folder set: {}", base.display()))
}

/// Build `folder` as it is from now on (`None`: the downloaded source).
pub fn set_local_source(folder: Option<&std::path::Path>) -> Result<String, String> {
    match folder {
        None => {
            source::set_local(None)?;
            Ok("Launch builds the downloaded source again.".into())
        }
        Some(f) if !f.join("Cargo.toml").is_file() => Err(format!("{} has no Cargo.toml.", f.display())),
        Some(f) => {
            source::set_local(Some(f))?;
            Ok(format!("Launch now builds {} as it is.", f.display()))
        }
    }
}
