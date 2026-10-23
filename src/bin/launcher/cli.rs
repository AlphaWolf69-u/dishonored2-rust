//! The launcher without its window: `dishonored2-launcher <command>`
//! prints one JSON object on stdout (`"ok"` and the result, or `"error"`)
//! and progress on stderr; the exit code is 0 on success. For scripts and
//! agents that drive the launcher.
use super::resolutions::{Choice, Conflict, Store};
use super::{build, ops, source, toolchain, updates};
use dishonored2_rust::assets::installation;
use dishonored2_rust::platform::home;
use serde_json::{json, Value};
use std::path::Path;

const HELP: &str = "\
dishonored2-launcher [command]    (no command: the window)

  status [--updates]            the rewrite folder, source, build tools, game, mods, build
  conflicts                     merge the enabled mods in memory; list conflicts and resolutions
  resolve <id> edit             write the file with conflict markers to edit; prints its path
  resolve <id> earlier|mod      keep the earlier side, or take the mod's version
  resolve <id> edited           use the edited file (no markers left)
  resolve <id> undo             forget the resolution
  enable <mod>                  also enables the mods it requires, then orders by relationships
  disable <mod> [--force]       --force is needed for mods permanent in saves
  move <mod> <position>         1-based load position
  sort                          order the enabled mods by their relationships
  download [--update]           download or fetch the source; --update moves to a newer release
  use-local <folder>            build a local checkout as it is
  use-download                  build the downloaded source again
  game-path <folder>            the installed game (or its base folder)
  import-saves <folder>         copy saves in (existing files are kept)
  import-config <file>          replace the rewrite's dishonored2Config.cfg
  build                         merge and build if anything changed
  launch                        build if needed, then start the game
";

#[cfg(windows)]
fn attach_console() {
    // The release launcher is a window program: without an inherited
    // stdout (a pipe), print to the console it was started from.
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if out == 0 || out == -1 {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    #[cfg(windows)]
    attach_console();
    if matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print!("{HELP}");
        return 0;
    }
    let result = command(args);
    let (ok, mut out) = match result {
        Ok(v) => (true, v),
        Err(e) => (false, json!({ "error": e })),
    };
    if let Value::Object(map) = &mut out {
        map.insert("ok".into(), Value::Bool(ok));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    if ok {
        0
    } else {
        1
    }
}

fn message(text: String) -> Value {
    json!({ "message": text })
}

fn progress(line: String) {
    eprintln!("{line}");
}

fn arg<'a>(args: &'a [String], i: usize, what: &str) -> Result<&'a str, String> {
    args.get(i).map(String::as_str).ok_or_else(|| format!("missing {what}; see `help`"))
}

fn cargo() -> Option<std::path::PathBuf> {
    let t = toolchain::detect();
    t.ready().then_some(t.cargo).flatten()
}

fn command(args: &[String]) -> Result<Value, String> {
    let rest = &args[1..];
    match args[0].as_str() {
        "status" => Ok(status(rest.iter().any(|a| a == "--updates"))),
        "conflicts" => conflicts(),
        "resolve" => resolve(arg(rest, 0, "conflict id")?, arg(rest, 1, "edit, earlier, mod, edited or undo")?),
        "enable" => ops::set_enabled(arg(rest, 0, "mod name")?, true).map(message),
        "disable" => {
            let name = arg(rest, 0, "mod name")?;
            let permanent = home::installed_mods().iter().any(|m| m.name.eq_ignore_ascii_case(name) && m.saves == home::SaveSafety::Permanent);
            if permanent && !rest.iter().any(|a| a == "--force") {
                return Err(format!("{name} is permanent in saves: saves started with it need it to load. Pass --force to disable it anyway."));
            }
            ops::set_enabled(name, false).map(message)
        }
        "move" => {
            let position: usize = arg(rest, 1, "position")?.parse().map_err(|_| "the position is a number from 1".to_string())?;
            ops::move_to(arg(rest, 0, "mod name")?, position.saturating_sub(1)).map(message)
        }
        "sort" => home::sort_modlist().map(|()| json!({ "enabled": home::modlist() })),
        "download" => {
            let newer = if rest.iter().any(|a| a == "--update") {
                let repo = source::open();
                let revision = source::revision(repo.as_ref());
                let branch = repo.as_ref().and_then(|r| r.find_reference(&format!("refs/remotes/origin/{revision}")).ok()?.target());
                updates::check(&revision, branch, &[]).rewrite
            } else {
                None
            };
            ops::download(newer, &mut progress).map(message)
        }
        "use-local" => ops::set_local_source(Some(Path::new(arg(rest, 0, "folder")?))).map(message),
        "use-download" => ops::set_local_source(None).map(message),
        "game-path" => ops::set_game_folder(Path::new(arg(rest, 0, "folder")?)).map(message),
        "import-saves" => {
            let r = home::import_saves(Path::new(arg(rest, 0, "folder")?))?;
            Ok(json!({ "copied": r.copied, "kept": r.kept }))
        }
        "import-config" => home::import_config(Path::new(arg(rest, 0, "file")?)).map(|()| message("Imported the config.".into())),
        "build" => ops::build(cargo(), &mut progress).map(|built| json!({ "built": built, "exe": build::game_exe() })),
        "launch" => ops::launch(cargo(), &mut progress).map(message),
        other => Err(format!("unknown command {other:?}; see `help`")),
    }
}

fn status(with_updates: bool) -> Value {
    let h = home::home();
    let local = source::local();
    let repo = source::open();
    let revision = source::revision(repo.as_ref());
    let commit = match (&repo, &local) {
        (Some(r), Some(_)) => r.head().ok().and_then(|h| h.peel_to_commit().ok()).map(|c| c.id()),
        (Some(r), None) => source::resolve(r, &revision).ok(),
        _ => None,
    };
    let t = toolchain::detect();
    let base = installation::game_base().ok();
    let enabled = home::modlist();
    let mut problems = Vec::new();
    let usable = home::enabled_mods(&mut problems);
    let mods: Vec<Value> = home::installed_mods()
        .iter()
        .map(|m| {
            let position = enabled.iter().position(|n| n.eq_ignore_ascii_case(&m.name));
            json!({
                "name": m.name,
                "version": m.version,
                "author": m.author,
                "rewrite": m.rewrite,
                "repository": m.repository,
                "description": m.description,
                "saves": m.saves.label(),
                "folder": m.folder,
                "enabled": position.is_some(),
                "position": position.map(|p| p + 1),
                "used": usable.iter().any(|u| u.name == m.name),
                "patches": m.patches().len(),
                "requires": m.requires.iter().map(|(n, r)| json!({ "name": n, "version": r })).collect::<Vec<_>>(),
                "incompatible": m.incompatible,
                "load_after": m.load_after,
                "load_before": m.load_before,
                "problem": m.problem,
            })
        })
        .collect();
    let layers = home::layers();
    let up_to_date = match (&local, commit) {
        (None, Some(id)) => Some(build::built().as_deref() == Some(build::fingerprint(&id.to_string(), &layers.mods).as_str())),
        _ => None,
    };
    let mut out = json!({
        "home": h.root,
        "source": {
            "kind": if local.is_some() { "local" } else { "download" },
            "path": local.clone().unwrap_or_else(|| h.source_dir()),
            "present": repo.is_some(),
            "revision": if local.is_some() { Value::Null } else { json!(revision) },
            "commit": commit.map(|c| c.to_string()),
            "describe": match (&repo, commit) {
                (Some(r), _) if local.is_some() => json!(source::describe_head(r)),
                (Some(r), Some(id)) => json!(source::describe(r, id)),
                _ => Value::Null,
            },
        },
        "toolchain": { "cargo": t.cargo, "cargo_version": t.cargo_version, "msvc": t.msvc, "winget": t.winget, "ready": t.ready() },
        "game": base,
        "config": h.config_file(),
        "mods": mods,
        "enabled": enabled,
        "problems": problems,
        "build": { "exe": build::game_exe(), "exists": build::game_exe().is_file(), "up_to_date": up_to_date },
        "fingerprint": base.as_deref().map(|b| home::fingerprint(b, &layers)),
    });
    if with_updates {
        let branch = repo.as_ref().and_then(|r| r.find_reference(&format!("refs/remotes/origin/{revision}")).ok()?.target());
        let mut p = Vec::new();
        let u = updates::check(&revision, branch, &home::enabled_mods(&mut p));
        out["updates"] = json!({ "rewrite": u.rewrite, "mods": u.mods, "problems": u.problems });
    }
    out
}

fn conflict_json(store: &Store, c: &Conflict) -> Value {
    let working = store.working_file(c);
    json!({
        "id": c.id,
        "path": c.path,
        "mod": c.module,
        "earlier": c.earlier,
        "detail": c.detail,
        "description": c.describe(),
        "choice": c.choice.map(Choice::name),
        "working_file": working.exists().then_some(working),
    })
}

fn conflicts() -> Result<Value, String> {
    let store = Store::home();
    let Some(merged) = ops::merge_check()? else {
        return Ok(json!({ "conflicts": [], "unresolved": 0, "note": "no enabled mod changes the source" }));
    };
    let list: Vec<Value> = merged.conflicts.iter().map(|c| conflict_json(&store, c)).collect();
    Ok(json!({ "conflicts": list, "unresolved": merged.unresolved().count() }))
}

fn resolve(id: &str, how: &str) -> Result<Value, String> {
    let store = Store::home();
    if how == "undo" {
        store.discard(id)?;
        return Ok(message(format!("Forgot the resolution {id}.")));
    }
    let merged = ops::merge_check()?.ok_or("No enabled mod changes the source.")?;
    let c = merged.conflicts.iter().find(|c| c.id == id).ok_or_else(|| format!("No current conflict has the id {id}; run `conflicts`."))?;
    match how {
        "edit" => {
            let repo = source::open().ok_or("Download the rewrite source first.")?;
            let file = store.open(&repo, c)?;
            Ok(json!({ "working_file": file, "message": "Edit the file, remove the conflict markers, then run `resolve <id> edited`." }))
        }
        other => {
            let choice = Choice::parse(other).ok_or_else(|| format!("unknown resolution {other:?}; use edit, earlier, mod, edited or undo"))?;
            store.choose(c, choice)?;
            Ok(message(format!("Resolved {}: {}.", c.path, choice.name())))
        }
    }
}
