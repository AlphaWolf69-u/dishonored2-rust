//! The rewrite's launcher and mod manager. It keeps the rewrite source
//! (`source/`), checks the build tools, merges the enabled mods' patches,
//! builds the game when the source or the mod selection changed, and
//! launches it. Folder layout: `platform::home`. With a command it runs
//! without its window (`cli`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod build;
mod cli;
mod manifest;
mod merge;
mod ops;
mod resolutions;
mod source;
mod toolchain;
mod updates;

use dishonored2_rust::assets::installation;
use dishonored2_rust::platform::{game_folder, home};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::path::Path;

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

    // One status line: a mark, a text with detail, and up to two actions.
    component Line inherits HorizontalLayout {
        in property <bool> ok;
        in property <string> text;
        in property <string> detail;
        in property <string> action;
        in property <string> action2;
        in property <bool> action-enabled: true;
        callback act();
        callback act2();
        spacing: 12px;
        Text { text: root.ok ? "✓" : "×"; color: root.ok ? Theme.good : Theme.warn; font-size: 16px; font-weight: 800; vertical-alignment: center; width: 16px; }
        VerticalLayout {
            alignment: center;
            spacing: 2px;
            horizontal-stretch: 1;
            Text { text: root.text; color: Theme.ink; overflow: elide; }
            if root.detail != "" : Text { text: root.detail; color: Theme.faint; font-size: 12px; overflow: elide; }
        }
        if root.action2 != "" : Action { text: root.action2; enabled: root.action-enabled; clicked => { root.act2(); } }
        if root.action != "" : Action { text: root.action; enabled: root.action-enabled; clicked => { root.act(); } }
    }

    export struct ModRow { name: string, detail: string, badge: string, badge-warn: bool, update: string, problem: string, enabled: bool, can-enable: bool }
    export struct ConflictRow { id: string, path: string, mods: string, module: string, detail: string, resolved: bool, editing: bool }

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
        in property <int> enabled-count;
        in property <[ConflictRow]> conflicts;
        in property <int> unresolved;
        // The row being dragged and where it would go (-1: none).
        property <int> drag-from: -1;
        property <int> drag-to: -1;
        property <length> row-pitch: 50px;
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
        callback drop-mod(string, int);
        callback conflict-edit(string);
        callback conflict-choose(string, string);
        callback conflict-undo(string);
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
                            // For developers: build an own checkout as it is.
                            action2: root.source-is-local ? "" : "Use local source…";
                            act => { if root.source-is-local { root.source-local(); } else { root.source-act(); } }
                            act2 => { root.source-local(); }
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
                                for row[i] in root.mods : Rectangle {
                                    height: root.row-pitch - 2px;
                                    background: root.drag-from == i ? #2e2722 : row-hit.has-hover ? #231e1a : transparent;
                                    row-hit := TouchArea {}
                                    // Where the dragged row would be dropped.
                                    if root.drag-from >= 0 && root.drag-to == i && root.drag-to != root.drag-from : Rectangle {
                                        x: 0; width: parent.width; height: 2px;
                                        y: root.drag-to < root.drag-from ? 0 : parent.height - 2px;
                                        background: Theme.brass;
                                    }
                                    HorizontalLayout {
                                        padding-left: 4px; padding-right: 10px; spacing: 10px;
                                        // Drag handle: enabled mods have a load order.
                                        Rectangle {
                                            width: 20px;
                                            Text {
                                                text: row.enabled ? "≡" : "";
                                                color: handle.pressed || handle.has-hover ? Theme.ink : Theme.faint;
                                                font-size: 20px;
                                                horizontal-alignment: center;
                                                vertical-alignment: center;
                                            }
                                            handle := TouchArea {
                                                enabled: row.enabled && !root.busy;
                                                mouse-cursor: self.pressed ? MouseCursor.grabbing : MouseCursor.grab;
                                                moved => {
                                                    root.drag-from = i;
                                                    root.drag-to = clamp(i + round((self.mouse-y - self.pressed-y) / root.row-pitch), 0, root.enabled-count - 1);
                                                }
                                                pointer-event(e) => {
                                                    if e.kind == PointerEventKind.up {
                                                        if root.drag-from == i && root.drag-to >= 0 && root.drag-to != i {
                                                            root.drop-mod(row.name, root.drag-to);
                                                        }
                                                        root.drag-from = -1;
                                                        root.drag-to = -1;
                                                    }
                                                }
                                            }
                                        }
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
                                    }
                                }
                                if root.conflicts.length > 0 : Text {
                                    text: root.unresolved > 0 ? "CONFLICTS  ·  " + root.unresolved + " to resolve before building" : "CONFLICTS  ·  all resolved";
                                    color: root.unresolved > 0 ? Theme.warn : Theme.good;
                                    font-size: 11px; font-weight: 700; letter-spacing: 2px;
                                    height: 30px; vertical-alignment: bottom;
                                }
                                for c in root.conflicts : Rectangle {
                                    height: root.row-pitch - 2px;
                                    background: c-hit.has-hover ? #231e1a : transparent;
                                    c-hit := TouchArea {}
                                    HorizontalLayout {
                                        padding-left: 10px; padding-right: 10px; spacing: 8px;
                                        Text { text: c.resolved ? "✓" : "×"; color: c.resolved ? Theme.good : Theme.warn; font-size: 16px; font-weight: 800; vertical-alignment: center; width: 16px; }
                                        VerticalLayout {
                                            alignment: center; spacing: 2px; horizontal-stretch: 1;
                                            Text { text: c.path; color: Theme.ink; font-weight: 600; overflow: elide; }
                                            Text { text: c.detail != "" ? c.mods + "  ·  " + c.detail : c.mods; color: Theme.faint; font-size: 12px; overflow: elide; }
                                        }
                                        if !c.resolved : VerticalLayout { alignment: center; Action { text: c.editing ? "Open again" : "Edit…"; enabled: !root.busy; clicked => { root.conflict-edit(c.id); } } }
                                        if !c.resolved && c.editing : VerticalLayout { alignment: center; Action { text: "Use edited"; enabled: !root.busy; clicked => { root.conflict-choose(c.id, "edited"); } } }
                                        if !c.resolved && !c.editing : VerticalLayout { alignment: center; Action { text: "Keep earlier"; enabled: !root.busy; clicked => { root.conflict-choose(c.id, "earlier"); } } }
                                        if !c.resolved && !c.editing : VerticalLayout { alignment: center; Action { text: "Take " + c.module; enabled: !root.busy; clicked => { root.conflict-choose(c.id, "mod"); } } }
                                        if c.resolved : VerticalLayout { alignment: center; Action { text: "Undo"; enabled: !root.busy; clicked => { root.conflict-undo(c.id); } } }
                                    }
                                }
                            }
                        }
                    }
                }
                HorizontalLayout {
                    spacing: 10px;
                    Text { text: "Enabled mods are merged top to bottom into the rewrite before it is built; drag ≡ to reorder."; color: Theme.faint; font-size: 12px; vertical-alignment: center; horizontal-stretch: 1; wrap: word-wrap; }
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
    /// The enabled mods' merge conflicts (an in-memory merge), or why the
    /// merge could not be checked.
    conflicts: Vec<resolutions::Conflict>,
    merge_error: Option<String>,
}

thread_local! {
    // The window thread's state.
    static FOUND: RefCell<Found> = RefCell::new(Found::default());
}

fn rows(installed: &[home::ModInfo], enabled: &[String], problems: &[String], updates: &updates::Updates) -> Vec<ModRow> {
    let row = |name: &str, m: Option<&home::ModInfo>, on: bool| {
        let detail = m.map_or(String::new(), |m| {
            let mut parts = vec![format!("v{}", m.version)];
            if !m.author.is_empty() {
                parts.push(format!("by {}", m.author));
            }
            if !m.rewrite.is_empty() {
                // A commit id is shown short, as git does.
                let hex = m.rewrite.len() == 40 && m.rewrite.bytes().all(|b| b.is_ascii_hexdigit());
                parts.push(format!("for rewrite {}", if hex { &m.rewrite[..8] } else { &m.rewrite }));
            }
            if !m.requires.is_empty() {
                parts.push(format!("needs {}", m.requires.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", ")));
            }
            if !m.description.is_empty() {
                parts.push(m.description.clone());
            }
            parts.join("  ·  ")
        });
        // An enabled mod's relationship problem ("skipped: it needs ...").
        let skipped = format!("Mod {name} skipped: ");
        let problem = match m {
            None => "Enabled in modlist.toml but not installed".to_string(),
            Some(m) => m.problem.clone().or_else(|| problems.iter().find_map(|p| p.strip_prefix(&skipped)).filter(|_| on).map(|p| format!("Skipped: {p}"))).unwrap_or_default(),
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
        }
    };
    let mut out: Vec<ModRow> = enabled
        .iter()
        .map(|name| row(name, installed.iter().find(|m| m.name.eq_ignore_ascii_case(name)), true))
        .collect();
    for m in installed.iter().filter(|m| !enabled.iter().any(|e| e.eq_ignore_ascii_case(&m.name))) {
        out.push(row(&m.name, Some(m), false));
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
        let layers = home::layers();
        ui.set_mods(ModelRc::new(VecModel::from(rows(&installed, &enabled, &layers.problems, &found.updates))));
        ui.set_enabled_count(enabled.len() as i32);
        let store = resolutions::Store::home();
        let conflict_rows: Vec<ConflictRow> = found
            .conflicts
            .iter()
            .map(|c| ConflictRow {
                id: c.id.as_str().into(),
                path: c.path.as_str().into(),
                mods: format!("{} vs {}", c.module, c.earlier).into(),
                module: c.module.as_str().into(),
                detail: match c.choice {
                    Some(choice) => format!("resolved: {}", match choice {
                        resolutions::Choice::Earlier => "kept the earlier version",
                        resolutions::Choice::Mod => "took the mod's version",
                        resolutions::Choice::Edited => "edited",
                    }),
                    None => c.detail.clone(),
                }
                .into(),
                resolved: c.choice.is_some(),
                editing: c.choice.is_none() && store.working_file(c).exists(),
            })
            .collect();
        ui.set_unresolved(found.conflicts.iter().filter(|c| c.choice.is_none()).count() as i32);
        ui.set_conflicts(ModelRc::new(VecModel::from(conflict_rows)));
        ui.set_fingerprint(base.as_deref().map_or("unavailable (no game folder)".into(), |b| home::fingerprint(b, &layers)).into());
        let open_conflicts = found.conflicts.iter().filter(|c| c.choice.is_none()).count();
        ui.set_build_text(match target {
            None if local.is_some() => "Launch builds the local checkout as it is (cargo rebuilds only what changed)".into(),
            None => "Not built: the source is missing".into(),
            Some(id) if build::built().as_deref() == Some(build::fingerprint(&id.to_string(), &layers.mods).as_str()) => {
                "Built and up to date with the enabled mods".into()
            }
            Some(_) if open_conflicts > 0 => format!("Resolve {open_conflicts} conflict(s) before building").into(),
            Some(_) => "Launch will build the rewrite with the enabled mods first".into(),
        });
        let (text, error) = status.unwrap_or_else(|| {
            if let Some(p) = layers.problems.first().or(found.merge_error.as_ref()).or(found.updates.problems.first()) {
                (p.clone(), true)
            } else if let Some(c) = found.conflicts.iter().find(|c| c.choice.is_none()) {
                (format!("{}. Resolve the conflicts below to build.", c.describe()), true)
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
        let (conflicts, merge_error) = check_merge();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            FOUND.with(|f| *f.borrow_mut() = Found { toolchain, updates, conflicts, merge_error });
            if !ui.get_busy() {
                // Keep a result the user has not seen replaced (a build error,
                // "Game started"); only the checking notice gives way.
                let keep = (ui.get_status() != CHECKING).then(|| (ui.get_status().to_string(), ui.get_status_error()));
                refresh(&ui, keep);
            }
        });
    });
}

fn check_merge() -> (Vec<resolutions::Conflict>, Option<String>) {
    match ops::merge_check() {
        Ok(m) => (m.map(|m| m.conflicts).unwrap_or_default(), None),
        Err(e) => (Vec::new(), Some(e)),
    }
}

/// Merge the mods again (after a resolution or a mod change) and show the
/// conflicts; `status` is shown when done.
fn recheck(ui: &Launcher, status: Option<(String, bool)>) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let (conflicts, merge_error) = check_merge();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            FOUND.with(|f| {
                let mut f = f.borrow_mut();
                f.conflicts = conflicts;
                f.merge_error = merge_error;
            });
            refresh(&ui, status);
        });
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(cli::run(&args));
    }
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
        background(&ui, move |progress| ops::download(update, progress));
    });

    let s = status.clone();
    ui.on_install_rust(move || s(Some(FOUND.with(|f| toolchain::install_rust(&f.borrow().toolchain)).map_or_else(|e| (e, true), |m| (m, false)))));
    let s = status.clone();
    ui.on_install_build_tools(move || s(Some(FOUND.with(|f| toolchain::install_build_tools(&f.borrow().toolchain)).map_or_else(|e| (e, true), |m| (m, false)))));

    let s = status.clone();
    ui.on_browse_game(move || {
        let Some(picked) = game_folder::pick_folder("Select your Dishonored 2 installation folder (the one containing \"base\").") else { return };
        s(Some(ops::set_game_folder(&picked).map_or_else(|e| (e, true), |m| (m, false))));
    });

    let weak = ui.as_weak();
    ui.on_toggle_mod(move |name, on| {
        let Some(ui) = weak.upgrade() else { return };
        let permanent = home::installed_mods().iter().any(|m| m.name.eq_ignore_ascii_case(&name) && m.saves == home::SaveSafety::Permanent);
        if !on && permanent && !game_folder::confirm(&format!("{name} is permanent in saves: saves started with it need it to load.\n\nDisable it anyway?")) {
            return;
        }
        recheck(&ui, Some(ops::set_enabled(&name, on).map_or_else(|e| (e, true), |m| (m, false))));
    });

    let weak = ui.as_weak();
    ui.on_drop_mod(move |name, index| {
        let Some(ui) = weak.upgrade() else { return };
        recheck(&ui, Some(ops::move_to(&name, index.max(0) as usize).map_or_else(|e| (e, true), |m| (m, false))));
    });

    let weak = ui.as_weak();
    ui.on_conflict_edit(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        let opened = FOUND.with(|f| {
            let f = f.borrow();
            let c = f.conflicts.iter().find(|c| c.id == id.as_str()).ok_or("The conflict is gone; refresh.".to_string())?;
            let repo = source::open().ok_or("Download the rewrite source first.".to_string())?;
            resolutions::Store::home().open(&repo, c)
        });
        let status = match opened {
            Ok(file) => {
                let _ = std::process::Command::new("explorer").arg(&file).spawn();
                (format!("Editing {}: remove the conflict markers, save, then press Use edited.", file.display()), false)
            }
            Err(e) => (e, true),
        };
        refresh(&ui, Some(status));
    });

    let weak = ui.as_weak();
    ui.on_conflict_choose(move |id, how| {
        let Some(ui) = weak.upgrade() else { return };
        let result = FOUND.with(|f| {
            let f = f.borrow();
            let c = f.conflicts.iter().find(|c| c.id == id.as_str()).ok_or("The conflict is gone; refresh.".to_string())?;
            let choice = resolutions::Choice::parse(&how).ok_or("unknown resolution".to_string())?;
            resolutions::Store::home().choose(c, choice).map(|()| format!("Resolved {}.", c.path))
        });
        recheck(&ui, Some(result.map_or_else(|e| (e, true), |m| (m, false))));
    });

    let weak = ui.as_weak();
    ui.on_conflict_undo(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        recheck(&ui, Some(resolutions::Store::home().discard(&id).map_or_else(|e| (e, true), |()| ("The conflict is open again.".into(), false))));
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
            return s(Some(ops::set_local_source(None).map_or_else(|e| (e, true), |m| (m, false))));
        }
        let Some(folder) = game_folder::pick_folder("Select your rewrite checkout (the folder with Cargo.toml).") else { return };
        s(Some(ops::set_local_source(Some(&folder)).map_or_else(|e| (e, true), |m| (m, false))));
    });

    let weak = ui.as_weak();
    ui.on_launch(move || {
        let Some(ui) = weak.upgrade() else { return };
        let cargo = FOUND.with(|f| {
            let t = &f.borrow().toolchain;
            t.ready().then(|| t.cargo.clone()).flatten()
        });
        background(&ui, move |progress| ops::launch(cargo, progress));
    });

    ui.run()
}
