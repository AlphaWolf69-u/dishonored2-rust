//! The rewrite's launcher and mod manager: the game folder, explicit save
//! and config imports, the mod list (`modlist.toml`), and Launch. It works
//! on the same rewrite folder as the game (`platform::home`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use dishonored2_rust::assets::installation;
use dishonored2_rust::platform::{game_folder, home};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
        spacing: 10px;
        Text { text: root.title; color: Theme.brass; font-size: 12px; font-weight: 700; letter-spacing: 2px; }
        @children
    }

    component Panel inherits Rectangle {
        background: Theme.panel;
        border-width: 1px;
        border-color: Theme.panel-edge;
        border-radius: 2px;
    }

    export struct ModRow { id: string, title: string, detail: string, problem: string, enabled: bool, can-enable: bool, first: bool, last: bool }

    export component Launcher inherits Window {
        title: "Dishonored 2 Rewrite";
        preferred-width: 880px; preferred-height: 820px;
        min-width: 720px; min-height: 640px;
        background: #110e0c;
        default-font-family: "Segoe UI";
        default-font-size: 14px;

        in property <string> version;
        in property <string> game-path;
        in property <bool> game-found;
        in property <string> home-path;
        in property <[ModRow]> mods;
        in property <string> overrides-text;
        in property <string> fingerprint;
        in property <string> status;
        in property <bool> status-error;

        callback browse-game();
        callback toggle-mod(string, bool);
        callback move-mod(string, int);
        callback import-saves();
        callback import-config();
        callback open(string);
        callback refresh();
        callback launch();

        VerticalLayout {
            padding: 28px;
            spacing: 22px;

            HorizontalLayout {
                spacing: 16px;
                VerticalLayout {
                    spacing: 2px;
                    Text { text: "DISHONORED 2"; color: Theme.ink; font-size: 30px; font-weight: 800; letter-spacing: 4px; }
                    Text { text: "REWRITE  ·  LAUNCHER AND MODS"; color: Theme.muted; font-size: 12px; font-weight: 600; letter-spacing: 3px; }
                }
                Rectangle { horizontal-stretch: 1; }
                Text { text: "v" + root.version; color: Theme.faint; font-size: 12px; vertical-alignment: top; }
            }

            Rectangle { height: 1px; background: Theme.panel-edge; }

            Section {
                title: "GAME";
                Panel {
                    HorizontalLayout {
                        padding: 14px; spacing: 14px;
                        VerticalLayout {
                            alignment: center;
                            spacing: 3px;
                            horizontal-stretch: 1;
                            Text {
                                text: root.game-found ? root.game-path : "Dishonored 2 was not found. Choose its installation folder.";
                                color: root.game-found ? Theme.ink : Theme.warn;
                                overflow: elide;
                            }
                            Text { text: root.game-found ? "Installed game files (read only)" : "Any store's installation with the standard game files works."; color: Theme.faint; font-size: 12px; }
                        }
                        Action { text: "Browse…"; clicked => { root.browse-game(); } }
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
                            height: 120px;
                        }
                        ScrollView {
                            visible: root.mods.length > 0;
                            VerticalLayout {
                                spacing: 2px;
                                for row in root.mods : Rectangle {
                                    height: 58px;
                                    background: row-hit.has-hover ? #231e1a : transparent;
                                    row-hit := TouchArea {}
                                    HorizontalLayout {
                                        padding-left: 10px; padding-right: 10px; spacing: 14px;
                                        alignment: stretch;
                                        VerticalLayout { alignment: center; Toggle {
                                            checked: row.enabled;
                                            enabled: row.can-enable || row.enabled;
                                            toggled(on) => { root.toggle-mod(row.id, on); }
                                        } }
                                        VerticalLayout {
                                            alignment: center;
                                            spacing: 2px;
                                            horizontal-stretch: 1;
                                            Text { text: row.title; color: row.problem == "" ? Theme.ink : Theme.muted; font-weight: 600; overflow: elide; }
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
                                                Action { text: "▲"; min-width: 34px; enabled: !row.first; clicked => { root.move-mod(row.id, -1); } }
                                                Action { text: "▼"; min-width: 34px; enabled: !row.last; clicked => { root.move-mod(row.id, 1); } }
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
                    Text { text: "Enabled mods load top to bottom; a lower mod's files win. " + root.overrides-text; color: Theme.faint; font-size: 12px; vertical-alignment: center; horizontal-stretch: 1; wrap: word-wrap; }
                    Action { text: "Mods folder"; clicked => { root.open("mods"); } }
                    Action { text: "Overrides folder"; clicked => { root.open("overrides"); } }
                    Action { text: "Refresh"; clicked => { root.refresh(); } }
                }
            }

            Section {
                title: "SAVES AND SETTINGS";
                HorizontalLayout {
                    spacing: 10px;
                    Action { text: "Import saves…"; clicked => { root.import-saves(); } }
                    Action { text: "Import config…"; clicked => { root.import-config(); } }
                    Rectangle { horizontal-stretch: 1; }
                    Action { text: "Rewrite folder"; clicked => { root.open("home"); } }
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
                    Text { text: "Setup " + root.fingerprint; color: Theme.faint; font-size: 11px; font-family: "Consolas"; overflow: elide; }
                }
                Action { text: "LAUNCH"; primary: true; min-width: 180px; enabled: root.game-found; clicked => { root.launch(); } }
            }
        }
    }
}

/// The rows: enabled mods in load order (including listed folders that are
/// missing), then the other installed mods by name.
fn rows(installed: &[home::Installed], enabled: &[String]) -> Vec<ModRow> {
    let mut out = Vec::new();
    for (i, id) in enabled.iter().enumerate() {
        let found = installed.iter().find(|m| &m.id == id);
        out.push(ModRow {
            id: id.into(),
            title: found.map_or(id.clone(), |m| if m.name.is_empty() { m.id.clone() } else { m.name.clone() }).into(),
            detail: found.map_or(String::new(), |m| detail(m)).into(),
            problem: match found {
                None => "Listed in modlist.toml but not installed".into(),
                Some(m) => m.problem.clone().unwrap_or_default().into(),
            },
            enabled: true,
            can_enable: true,
            first: i == 0,
            last: i + 1 == enabled.len(),
        });
    }
    for m in installed.iter().filter(|m| !enabled.contains(&m.id)) {
        out.push(ModRow {
            id: m.id.clone().into(),
            title: (if m.name.is_empty() { m.id.clone() } else { m.name.clone() }).into(),
            detail: detail(m).into(),
            problem: m.problem.clone().unwrap_or_default().into(),
            enabled: false,
            can_enable: m.problem.is_none(),
            first: false,
            last: false,
        });
    }
    out
}

fn detail(m: &home::Installed) -> String {
    let mut s = format!("{}  ·  {}", m.id, m.version);
    if !m.description.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&m.description);
    }
    s
}

fn refresh(ui: &Launcher, status: Option<(String, bool)>) {
    let h = home::home();
    ui.set_home_path(h.root.display().to_string().into());
    let base = installation::game_base().ok();
    ui.set_game_found(base.is_some());
    ui.set_game_path(base.as_ref().map(|b| b.display().to_string()).unwrap_or_default().into());
    let installed = home::installed_mods();
    let enabled = home::modlist();
    ui.set_mods(ModelRc::new(VecModel::from(rows(&installed, &enabled))));
    let layers = home::layers();
    let overrides: Vec<_> = layers.files.iter().filter(|(_, p)| p.starts_with(h.overrides_dir())).collect();
    ui.set_overrides_text(match overrides.len() {
        0 => "No resource overrides.".into(),
        1 => "1 resource replaced by overrides/.".into(),
        n => format!("{n} resources replaced by overrides/.").into(),
    });
    ui.set_fingerprint(base.as_deref().map_or("unavailable (no game folder)".into(), |b| home::fingerprint(b, &layers)).into());
    let (text, error) = status.unwrap_or_else(|| match layers.problems.first() {
        Some(p) => (p.clone(), true),
        None if base.is_none() => ("Choose the game folder to launch.".into(), true),
        None => ("Ready.".into(), false),
    });
    ui.set_status(text.into());
    ui.set_status_error(error);
}

fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    let _ = std::process::Command::new("explorer").arg(path).spawn();
}

/// The game executable next to the launcher.
fn game_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "dishonored2-rust.exe" } else { "dishonored2-rust" };
    let exe = std::env::current_exe().ok()?.parent()?.join(name);
    exe.is_file().then_some(exe)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = Launcher::new()?;
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    refresh(&ui, None);
    let weak = ui.as_weak();
    let with = move |f: Rc<dyn Fn(&Launcher) -> Option<(String, bool)>>| {
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                let status = f(&ui);
                refresh(&ui, status);
            }
        }
    };

    ui.on_browse_game(with(Rc::new(|_| {
        let picked = game_folder::pick_folder("Select your Dishonored 2 installation folder (the one containing \"base\").")?;
        Some(match installation::base_of(&picked) {
            Some(base) => match home::save_game_path(&base) {
                Ok(()) => (format!("Game folder set: {}", base.display()), false),
                Err(e) => (e, true),
            },
            None => (format!("{} has no Dishonored 2 game files (base\\game1_patch.index).", picked.display()), true),
        })
    })));

    let weak = ui.as_weak();
    ui.on_toggle_mod(move |id, on| {
        let mut list = home::modlist();
        list.retain(|m| m != id.as_str());
        if on {
            list.push(id.to_string());
        }
        let status = home::save_modlist(&list).err().map(|e| (e, true));
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, status);
        }
    });

    let weak = ui.as_weak();
    ui.on_move_mod(move |id, step| {
        let mut list = home::modlist();
        if let Some(i) = list.iter().position(|m| m == id.as_str()) {
            let j = i as i32 + step;
            if (0..list.len() as i32).contains(&j) {
                list.swap(i, j as usize);
            }
        }
        let status = home::save_modlist(&list).err().map(|e| (e, true));
        if let Some(ui) = weak.upgrade() {
            refresh(&ui, status);
        }
    });

    ui.on_import_saves(with(Rc::new(|_| {
        let folder = game_folder::pick_folder("Select the Dishonored 2 save folder to import (the folder with profile.bin and SLOT folders).")?;
        Some(match home::import_saves(&folder) {
            Ok(r) => (format!("Imported {} save files from {}; {} already present were kept.", r.copied, folder.display(), r.kept), false),
            Err(e) => (e, true),
        })
    })));

    ui.on_import_config(with(Rc::new(|_| {
        let folder = game_folder::pick_folder("Select the folder containing the dishonored2Config.cfg to import. It replaces the rewrite's config.")?;
        let file = folder.join(home::CONFIG_FILE);
        Some(match home::import_config(&file) {
            Ok(()) => (format!("Imported config from {}.", file.display()), false),
            Err(e) => (e, true),
        })
    })));

    ui.on_open(move |which| {
        let h = home::home();
        open_folder(&match which.as_str() {
            "mods" => h.mods_dir(),
            "overrides" => h.overrides_dir(),
            "logs" => h.logs_dir(),
            _ => h.root.clone(),
        });
    });

    ui.on_refresh(with(Rc::new(|_| None)));

    ui.on_launch(with(Rc::new(|_| {
        let Some(exe) = game_exe() else {
            return Some(("dishonored2-rust.exe was not found next to the launcher.".into(), true));
        };
        let mut command = std::process::Command::new(&exe);
        // The game finds the same rewrite folder.
        command.env("DH2_REWRITE_HOME", &home::home().root);
        if let Some(dir) = exe.parent() {
            command.current_dir(dir);
        }
        Some(match command.spawn() {
            Ok(_) => ("Game started.".into(), false),
            Err(e) => (format!("Could not start {}: {e}", exe.display()), true),
        })
    })));

    ui.run()
}
