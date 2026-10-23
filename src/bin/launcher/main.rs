//! The rewrite's launcher and mod manager. It keeps the rewrite source
//! (`source/`), checks the build tools, merges the enabled mods' patches,
//! builds the game when the source or the mod selection changed, and
//! launches it. Folder layout: `platform::home`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod build;
mod manifest;
mod merge;
mod source;
mod toolchain;
mod updates;

use dishonored2_rust::assets::installation;
use dishonored2_rust::platform::{game_folder, home};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

slint::slint! {
    import { ScrollView } from "std-widgets.slint";

    global Theme {
        out property <color> ink: #e6dac3;
        out property <color> muted: #9a8d79;
        out property <color> faint: #5e554a;
        out property <color> panel: #1c1815;
        out property <color> panel-edge: #3a3129;
        out property <color> blood: #a8282c;
        out property <color> blood-hover: #c13236;
        out property <color> brass: #b59a68;
        out property <color> warn: #d9734f;
        out property <color> good: #8fae6e;
    }

    component Action inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        in property <bool> primary: false;
        callback clicked();
        min-width: max(96px, label.preferred-width + 30px);
        horizontal-stretch: 0;
        vertical-stretch: 0;
        height: primary ? 48px : 34px;
        border-radius: 2px;
        border-width: primary ? 0px : 1px;
        border-color: hit.has-hover && enabled ? Theme.brass : Theme.panel-edge;
        background: !enabled ? #2a2420 : primary ? (hit.pressed ? #8c1f22 : hit.has-hover ? Theme.blood-hover : Theme.blood)
            : (hit.pressed ? #2e2722 : hit.has-hover ? #27211d : Theme.panel);
        label := Text {
            text: root.text;
            color: !root.enabled ? Theme.faint : Theme.ink;
            font-size: root.primary ? 17px : 13px;
            font-weight: root.primary ? 700 : 600;
            letter-spacing: root.primary ? 2px : 0.5px;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
        hit := TouchArea { enabled: root.enabled; clicked => { root.clicked(); } }
    }

    component Toggle inherits Rectangle {
        in property <bool> checked;
        in property <bool> enabled: true;
        callback toggled(bool);
        width: 22px; height: 22px;
        border-radius: 2px;
        border-width: 1px;
        border-color: !enabled ? Theme.faint : checked ? Theme.brass : Theme.muted;
        background: checked ? Theme.blood : transparent;
        Text { text: "✓"; visible: root.checked; color: Theme.ink; font-size: 15px; font-weight: 800; horizontal-alignment: center; vertical-alignment: center; }
        TouchArea { enabled: root.enabled; clicked => { root.toggled(!root.checked); } }
    }

    component Section inherits VerticalLayout {
        in property <string> title;
        spacing: 8px;
        Text { text: root.title; color: Theme.brass; font-size: 12px; font-weight: 700; letter-spacing: 2px; }
        @children
    }

    component Panel inherits Rectangle {
        background: Theme.panel;
        border-width: 1px;
        border-color: Theme.panel-edge;
        border-radius: 2px;
    }

    // One status line: a mark, a text with detail, and an optional action.
    component Line inherits HorizontalLayout {
        in property <bool> ok;
        in property <string> text;
        in property <string> detail;
        in property <string> action;
        in property <bool> action-enabled: true;
        callback act();
        spacing: 12px;
        Text { text: root.ok ? "✓" : "×"; color: root.ok ? Theme.good : Theme.warn; font-size: 16px; font-weight: 800; vertical-alignment: center; width: 16px; }
        VerticalLayout {
            alignment: center;
            spacing: 2px;
            horizontal-stretch: 1;
            Text { text: root.text; color: Theme.ink; overflow: elide; }
            if root.detail != "" : Text { text: root.detail; color: Theme.faint; font-size: 12px; overflow: elide; }
        }
        if root.action != "" : Action { text: root.action; enabled: root.action-enabled; clicked => { root.act(); } }
    }

    export struct ModRow { name: string, detail: string, badge: string, badge-warn: bool, update: string, problem: string, enabled: bool, can-enable: bool, first: bool, last: bool }

    export component Launcher inherits Window {
        title: "Dishonored 2 Rewrite";
        preferred-width: 900px; preferred-height: 780px;
        min-width: 760px; min-height: 620px;
        background: #110e0c;
        default-font-family: "Segoe UI";
        default-font-size: 14px;

        in property <string> version;
        in property <bool> busy;
        in property <bool> source-ok;
        in property <string> source-text;
        in property <string> source-detail;
        in property <string> source-action;
        in property <bool> source-is-local;
        in property <bool> rust-ok;
        in property <string> rust-text;
        in property <bool> msvc-ok;
        in property <bool> game-found;
        in property <string> game-path;
        in property <[ModRow]> mods;
        in property <string> build-text;
        in property <string> fingerprint;
        in property <string> status;
        in property <bool> status-error;

        callback source-act();
        callback source-local();
        callback install-rust();
        callback install-build-tools();
        callback browse-game();
        callback toggle-mod(string, bool);
        callback move-mod(string, int);
        callback import-saves();
        callback import-config();
        callback open(string);
        callback refresh();
        callback launch();

        VerticalLayout {
            padding: 22px;
            spacing: 14px;

            HorizontalLayout {
                spacing: 16px;
                VerticalLayout {
                    spacing: 2px;
                    Text { text: "DISHONORED 2"; color: Theme.ink; font-size: 30px; font-weight: 800; letter-spacing: 4px; }
                    Text { text: "REWRITE  ·  LAUNCHER AND MODS"; color: Theme.muted; font-size: 12px; font-weight: 600; letter-spacing: 3px; }
                }
                Rectangle { horizontal-stretch: 1; }
                Text { text: "launcher v" + root.version; color: Theme.faint; font-size: 12px; vertical-alignment: top; }
            }

            Rectangle { height: 1px; background: Theme.panel-edge; }

            Section {
                title: "REWRITE AND BUILD TOOLS";
                Panel {
                    VerticalLayout {
                        padding: 10px; spacing: 6px;
                        Line {
                            ok: root.source-ok; text: root.source-text; detail: root.source-detail; action: root.source-action; action-enabled: !root.busy;
                            act => { if root.source-is-local { root.source-local(); } else { root.source-act(); } }
                        }
                        if !root.source-is-local : Line {
                            ok: true;
                            text: "Developing the rewrite? Launch can build your own checkout as it is.";
                            action: "Use local source…";
                            action-enabled: !root.busy;
                            act => { root.source-local(); }
                        }
                        Line { ok: root.rust-ok; text: root.rust-text; action: root.rust-ok ? "" : "Install Rust"; action-enabled: !root.busy; act => { root.install-rust(); } }
                        Line {
                            ok: root.msvc-ok;
                            text: root.msvc-ok ? "C++ Build Tools (MSVC linker)" : "C++ Build Tools are missing (Rust needs their linker)";
                            action: root.msvc-ok ? "" : "Install Build Tools";
                            action-enabled: !root.busy;
                            act => { root.install-build-tools(); }
                        }
                    }
                }
            }

            Section {
                title: "GAME";
                Panel {
                    VerticalLayout {
                        padding: 12px;
                        Line {
                            ok: root.game-found;
                            text: root.game-found ? root.game-path : "Dishonored 2 was not found. Choose its installation folder.";
                            detail: root.game-found ? "Installed game files (read only)" : "Any store's installation with the standard game files works.";
                            action: "Browse…";
                            action-enabled: !root.busy;
                            act => { root.browse-game(); }
                        }
                    }
                }
            }

            Section {
                vertical-stretch: 1;
                title: "MODS";
                Panel {
                    vertical-stretch: 1;
                    VerticalLayout {
                        padding: 6px;
                        if root.mods.length == 0 : Text {
                            text: "No mods installed. Put each mod in its own folder under mods/.";
                            color: Theme.muted;
                            horizontal-alignment: center;
                            vertical-alignment: center;
                            height: 100px;
                        }
                        ScrollView {
                            visible: root.mods.length > 0;
                            VerticalLayout {
                                spacing: 2px;
                                for row in root.mods : Rectangle {
                                    height: 56px;
                                    background: row-hit.has-hover ? #231e1a : transparent;
                                    row-hit := TouchArea {}
                                    HorizontalLayout {
                                        padding-left: 10px; padding-right: 10px; spacing: 14px;
                                        VerticalLayout { alignment: center; Toggle {
                                            checked: row.enabled;
                                            enabled: !root.busy && (row.can-enable || row.enabled);
                                            toggled(on) => { root.toggle-mod(row.name, on); }
                                        } }
                                        VerticalLayout {
                                            alignment: center;
                                            spacing: 2px;
                                            horizontal-stretch: 1;
                                            HorizontalLayout {
                                                spacing: 10px;
                                                Text { text: row.name; color: row.problem == "" ? Theme.ink : Theme.muted; font-weight: 600; overflow: elide; }
                                                if row.badge != "" : Text { text: row.badge; color: row.badge-warn ? Theme.warn : Theme.good; font-size: 11px; vertical-alignment: center; }
                                                if row.update != "" : Text { text: row.update; color: Theme.brass; font-size: 11px; vertical-alignment: center; }
                                                Rectangle { horizontal-stretch: 1; }
                                            }
                                            Text {
                                                text: row.problem != "" ? row.problem : row.detail;
                                                color: row.problem != "" ? Theme.warn : Theme.faint;
                                                font-size: 12px;
                                                overflow: elide;
                                            }
                                        }
                                        if row.enabled : VerticalLayout {
                                            alignment: center;
                                            HorizontalLayout {
                                                spacing: 6px;
                                                Action { text: "▲"; min-width: 34px; enabled: !root.busy && !row.first; clicked => { root.move-mod(row.name, -1); } }
                                                Action { text: "▼"; min-width: 34px; enabled: !root.busy && !row.last; clicked => { root.move-mod(row.name, 1); } }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                HorizontalLayout {
                    spacing: 10px;
                    Text { text: "Enabled mods are merged top to bottom into the rewrite before it is built."; color: Theme.faint; font-size: 12px; vertical-alignment: center; horizontal-stretch: 1; wrap: word-wrap; }
                    Action { text: "Mods folder"; clicked => { root.open("mods"); } }
                    Action { text: "Refresh"; enabled: !root.busy; clicked => { root.refresh(); } }
                }
            }

            Section {
                title: "SAVES AND SETTINGS";
                HorizontalLayout {
                    spacing: 10px;
                    Action { text: "Import saves…"; enabled: !root.busy; clicked => { root.import-saves(); } }
                    Action { text: "Import config…"; enabled: !root.busy; clicked => { root.import-config(); } }
                    Rectangle { horizontal-stretch: 1; }
                    Action { text: "Logs"; clicked => { root.open("logs"); } }
                }
            }

            Rectangle { height: 1px; background: Theme.panel-edge; }

            HorizontalLayout {
                spacing: 18px;
                VerticalLayout {
                    alignment: center;
                    spacing: 4px;
                    horizontal-stretch: 1;
                    Text { text: root.status; color: root.status-error ? Theme.warn : Theme.muted; wrap: word-wrap; }
                    Text { text: root.build-text; color: Theme.faint; font-size: 12px; }
                    Text { text: "Setup " + root.fingerprint; color: Theme.faint; font-size: 11px; font-family: "Consolas"; overflow: elide; }
                }
                Action { text: root.busy ? "WORKING…" : "LAUNCH"; primary: true; min-width: 180px; enabled: !root.busy; clicked => { root.launch(); } }
            }
        }
    }
}

/// What takes time to find: the build tools and the update checks.
#[derive(Default)]
struct Found {
    toolchain: toolchain::Toolchain,
    updates: updates::Updates,
}

thread_local! {
    // The window thread's state.
    static FOUND: RefCell<Found> = RefCell::new(Found::default());
}

fn rows(installed: &[home::ModInfo], enabled: &[String], updates: &updates::Updates) -> Vec<ModRow> {
    let row = |name: &str, m: Option<&home::ModInfo>, on: bool, i: usize| {
        let detail = m.map_or(String::new(), |m| {
            let mut parts = vec![format!("v{}", m.version)];
            if !m.author.is_empty() {
                parts.push(format!("by {}", m.author));
            }
            if !m.rewrite.is_empty() {
                parts.push(format!("for rewrite {}", m.rewrite));
            }
            if !m.description.is_empty() {
                parts.push(m.description.clone());
            }
            parts.join("  ·  ")
        });
        let problem = match m {
            None => "Enabled in modlist.toml but not installed".to_string(),
            Some(m) => m.problem.clone().unwrap_or_default(),
        };
        ModRow {
            name: name.into(),
            detail: detail.into(),
            badge: m.filter(|m| m.problem.is_none()).map_or("", |m| m.saves.label()).into(),
            badge_warn: m.is_some_and(|m| m.saves == home::SaveSafety::Permanent),
            update: updates.mods.iter().find(|u| u.0 == name).map_or(String::new(), |u| format!("update {} available", u.1)).into(),
            problem: problem.into(),
            enabled: on,
            can_enable: m.is_some_and(|m| m.problem.is_none()),
            first: on && i == 0,
            last: on && i + 1 == enabled.len(),
        }
    };
    let mut out: Vec<ModRow> = enabled
        .iter()
        .enumerate()
        .map(|(i, name)| row(name, installed.iter().find(|m| m.name.eq_ignore_ascii_case(name)), true, i))
        .collect();
    for m in installed.iter().filter(|m| !enabled.iter().any(|e| e.eq_ignore_ascii_case(&m.name))) {
        out.push(row(&m.name, Some(m), false, 0));
    }
    out
}

fn refresh(ui: &Launcher, status: Option<(String, bool)>) {
    FOUND.with(|found| {
        let found = found.borrow();
        let local = source::local();
        let repo = source::open();
        let revision = source::revision(repo.as_ref());
        let target = repo.as_ref().filter(|_| local.is_none()).and_then(|r| source::resolve(r, &revision).ok());
        ui.set_source_is_local(local.is_some());
        ui.set_source_ok(target.is_some() || (local.is_some() && repo.is_some()));
        match (&repo, target) {
            _ if local.is_some() => {
                let path = local.as_ref().unwrap().display();
                ui.set_source_text(if repo.is_some() { format!("Local source {path}") } else { format!("Local source {path} is not a git checkout") }.into());
                ui.set_source_detail(repo.as_ref().map(source::describe_head).unwrap_or_default().into());
            }
            (Some(r), Some(id)) => {
                ui.set_source_text(format!("Rewrite {revision}").into());
                ui.set_source_detail(source::describe(r, id).into());
            }
            (Some(_), None) => {
                ui.set_source_text(format!("Rewrite {revision} is not in the downloaded source").into());
                ui.set_source_detail("".into());
            }
            _ => {
                ui.set_source_text("The rewrite source is not downloaded yet".into());
                ui.set_source_detail(source::REPOSITORY.into());
            }
        }
        ui.set_source_action(match (&repo, &found.updates.rewrite) {
            _ if local.is_some() => "Use download".into(),
            (None, _) => "Download".into(),
            (Some(_), Some(newer)) => format!("Update to {newer}").into(),
            (Some(_), None) => "Check for updates".into(),
        });
        let t = &found.toolchain;
        ui.set_rust_ok(t.cargo.is_some());
        ui.set_rust_text(if t.cargo.is_some() { t.cargo_version.clone() } else { "Rust is not installed (needed to build the rewrite)".into() }.into());
        ui.set_msvc_ok(t.msvc);
        let base = installation::game_base().ok();
        ui.set_game_found(base.is_some());
        ui.set_game_path(base.as_ref().map(|b| b.display().to_string()).unwrap_or_default().into());
        let installed = home::installed_mods();
        let enabled = home::modlist();
        ui.set_mods(ModelRc::new(VecModel::from(rows(&installed, &enabled, &found.updates))));
        let layers = home::layers();
        ui.set_fingerprint(base.as_deref().map_or("unavailable (no game folder)".into(), |b| home::fingerprint(b, &layers)).into());
        ui.set_build_text(match target {
            None if local.is_some() => "Launch builds the local checkout as it is (cargo rebuilds only what changed)".into(),
            None => "Not built: the source is missing".into(),
            Some(id) if build::built().as_deref() == Some(build::fingerprint(&id.to_string(), &layers.mods).as_str()) => {
                "Built and up to date with the enabled mods".into()
            }
            Some(_) => "Launch will build the rewrite with the enabled mods first".into(),
        });
        let (text, error) = status.unwrap_or_else(|| {
            if let Some(p) = layers.problems.first().or(found.updates.problems.first()) {
                (p.clone(), true)
            } else if repo.is_none() {
                ("Download the rewrite source to start.".into(), true)
            } else if !t.ready() {
                ("Install the build tools to build the rewrite.".into(), true)
            } else if base.is_none() {
                ("Choose the game folder.".into(), true)
            } else {
                ("Ready.".into(), false)
            }
        });
        ui.set_status(text.into());
        ui.set_status_error(error);
    });
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// Run `work` off the window thread; its progress lines show as the status
/// and its result ends it. The window is busy meanwhile.
fn background(ui: &Launcher, work: impl FnOnce(&mut dyn FnMut(String)) -> Result<String, String> + Send + 'static) {
    ui.set_busy(true);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let progress_weak = weak.clone();
        let mut progress = move |line: String| {
            let _ = progress_weak.upgrade_in_event_loop(move |ui| {
                ui.set_status(line.into());
                ui.set_status_error(false);
            });
        };
        let result = work(&mut progress);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_busy(false);
            refresh(&ui, Some(result.map_or_else(|e| (e, true), |s| (s, false))));
            discover(&ui);
        });
    });
}

const CHECKING: &str = "Checking the build tools and updates…";

/// Detect the build tools and check for updates off the window thread.
fn discover(ui: &Launcher) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let toolchain = toolchain::detect();
        let repo = source::open();
        let revision = source::revision(repo.as_ref());
        let branch = repo.as_ref().and_then(|r| r.find_reference(&format!("refs/remotes/origin/{revision}")).ok()?.target());
        let mut problems = Vec::new();
        let updates = updates::check(&revision, branch, &home::enabled_mods(&mut problems));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            FOUND.with(|f| *f.borrow_mut() = Found { toolchain, updates });
            if !ui.get_busy() {
                // Keep a result the user has not seen replaced (a build error,
                // "Game started"); only the checking notice gives way.
                let keep = (ui.get_status() != CHECKING).then(|| (ui.get_status().to_string(), ui.get_status_error()));
                refresh(&ui, keep);
            }
        });
    });
}

/// Merge, build if needed, start the game.
fn launch(cargo: Option<PathBuf>, progress: &mut dyn FnMut(String)) -> Result<String, String> {
    let repo = source::open().ok_or("Download the rewrite source first.")?;
    installation::game_base().map_err(|_| "Choose the game folder first.".to_string())?;
    let mut problems = Vec::new();
    let mods = home::enabled_mods(&mut problems);
    if let Some(p) = problems.first() {
        return Err(format!("{p}. Fix or disable it first."));
    }
    if let Some(checkout) = source::local() {
        // The checkout is the developer's work: merging mods into it would
        // replace that, so only resource mods apply.
        if let Some(m) = mods.iter().find(|m| !m.patches().is_empty()) {
            return Err(format!("{} changes the source; mods with patches are not merged into a local source. Disable it, or use the download.", m.name));
        }
        let cargo = cargo.ok_or("Install Rust and the C++ Build Tools to build the rewrite.")?;
        progress("Building the local source…".into());
        build::build_local(&cargo, &checkout, progress)?;
        return start();
    }
    let revision = source::revision(Some(&repo));
    let target = source::resolve(&repo, &revision)?;
    let fingerprint = build::fingerprint(&target.to_string(), &mods);
    if build::built().as_deref() != Some(fingerprint.as_str()) {
        let cargo = cargo.ok_or("Install Rust and the C++ Build Tools to build the rewrite.")?;
        progress("Merging the enabled mods into the rewrite source…".into());
        merge::prepare(&repo, target, &mods)?;
        progress("Building the rewrite (the first build takes a while)…".into());
        build::build(&cargo, &fingerprint, progress)?;
    }
    start()
}

/// Start the built game in the rewrite folder.
fn start() -> Result<String, String> {
    let exe = build::game_exe();
    std::process::Command::new(&exe)
        .env("DH2_REWRITE_HOME", &home::home().root)
        .current_dir(&home::home().root)
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
    Ok("Game started.".into())
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = Launcher::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    refresh(&ui, Some((CHECKING.into(), false)));
    discover(&ui);

    let weak = ui.as_weak();
    let status = move |s: Option<(String, bool)>| {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, s);
        }
    };

    let weak = ui.as_weak();
    ui.on_source_act(move || {
        let Some(ui) = weak.upgrade() else { return };
        let update = FOUND.with(|f| f.borrow().updates.rewrite.clone());
        background(&ui, move |progress| {
            source::download(progress)?;
            if let Some(newer) = update.filter(|v| !source::version_key(v).is_empty()) {
                source::set_revision(&newer)?;
                return Ok(format!("Updated the rewrite to {newer}."));
            }
            Ok("The rewrite source is downloaded and current.".into())
        });
    });

    let s = status.clone();
    ui.on_install_rust(move || s(Some(FOUND.with(|f| toolchain::install_rust(&f.borrow().toolchain)).map_or_else(|e| (e, true), |m| (m, false)))));
    let s = status.clone();
    ui.on_install_build_tools(move || s(Some(FOUND.with(|f| toolchain::install_build_tools(&f.borrow().toolchain)).map_or_else(|e| (e, true), |m| (m, false)))));

    let s = status.clone();
    ui.on_browse_game(move || {
        let Some(picked) = game_folder::pick_folder("Select your Dishonored 2 installation folder (the one containing \"base\").") else { return };
        s(Some(match installation::base_of(&picked) {
            Some(base) => match home::save_game_path(&base) {
                Ok(()) => (format!("Game folder set: {}", base.display()), false),
                Err(e) => (e, true),
            },
            None => (format!("{} has no Dishonored 2 game files (base\\game1_patch.index).", picked.display()), true),
        }));
    });

    let s = status.clone();
    ui.on_toggle_mod(move |name, on| {
        let permanent = home::installed_mods().iter().any(|m| m.name.eq_ignore_ascii_case(&name) && m.saves == home::SaveSafety::Permanent);
        if !on && permanent && !game_folder::confirm(&format!("{name} is permanent in saves: saves started with it need it to load.\n\nDisable it anyway?")) {
            return;
        }
        let mut list = home::modlist();
        list.retain(|m| !m.eq_ignore_ascii_case(&name));
        if on {
            list.push(name.to_string());
        }
        s(home::save_modlist(&list).err().map(|e| (e, true)));
    });

    let s = status.clone();
    ui.on_move_mod(move |name, step| {
        let mut list = home::modlist();
        if let Some(i) = list.iter().position(|m| m.eq_ignore_ascii_case(&name)) {
            let j = i as i32 + step;
            if (0..list.len() as i32).contains(&j) {
                list.swap(i, j as usize);
            }
        }
        s(home::save_modlist(&list).err().map(|e| (e, true)));
    });

    let s = status.clone();
    ui.on_import_saves(move || {
        let Some(folder) = game_folder::pick_folder("Select the Dishonored 2 save folder to import (the folder with profile.bin and SLOT folders).") else { return };
        s(Some(match home::import_saves(&folder) {
            Ok(r) => (format!("Imported {} save files from {}; {} already present were kept.", r.copied, folder.display(), r.kept), false),
            Err(e) => (e, true),
        }));
    });

    let s = status.clone();
    ui.on_import_config(move || {
        let Some(folder) = game_folder::pick_folder("Select the folder containing the dishonored2Config.cfg to import. It replaces the rewrite's config.") else { return };
        let file = folder.join(home::CONFIG_FILE);
        s(Some(match home::import_config(&file) {
            Ok(()) => (format!("Imported config from {}.", file.display()), false),
            Err(e) => (e, true),
        }));
    });

    ui.on_open(move |which| {
        let h = home::home();
        open_folder(&if which == "logs" { h.logs_dir() } else { h.mods_dir() });
    });

    let weak = ui.as_weak();
    ui.on_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, Some((CHECKING.into(), false)));
            discover(&ui);
        }
    });

    let s = status.clone();
    ui.on_source_local(move || {
        if source::local().is_some() {
            return s(Some(source::set_local(None).map_or_else(|e| (e, true), |()| ("Launch builds the downloaded source again.".into(), false))));
        }
        let Some(folder) = game_folder::pick_folder("Select your rewrite checkout (the folder with Cargo.toml).") else { return };
        s(Some(if !folder.join("Cargo.toml").is_file() {
            (format!("{} has no Cargo.toml.", folder.display()), true)
        } else {
            match source::set_local(Some(&folder)) {
                Ok(()) => (format!("Launch now builds {} as it is.", folder.display()), false),
                Err(e) => (e, true),
            }
        }));
    });

    let weak = ui.as_weak();
    ui.on_launch(move || {
        let Some(ui) = weak.upgrade() else { return };
        let cargo = FOUND.with(|f| {
            let t = &f.borrow().toolchain;
            t.ready().then(|| t.cargo.clone()).flatten()
        });
        background(&ui, move |progress| launch(cargo, progress));
    });

    ui.run()
}
