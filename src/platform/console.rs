//! The command state the configs drive: key bindings and cvars, loaded the
//! way the game starts (`FUN_140040820`) and written back as
//! `dishonored2Config.cfg` (`FUN_140041770`).
use super::bindings::{versions, Bindings, ACTIONS, BINDING_SETS, CONFIG_VERSION, KEY_NAMES};
use super::cvars::{flag, CVars};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock, RwLockReadGuard};

pub const CONFIG_FILE: &str = "dishonored2Config.cfg";

/// The client's one command state (the game keeps one cvar system and one
/// binding manager for the process). Settings readers take their values from
/// it; the pause menu's options edit it.
pub struct Shared {
    console: RwLock<Console>,
    generation: AtomicU64,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// Loaded once from the installed `default.cfg` and the rewrite's config.
/// Without a game folder only the registered cvar defaults exist.
pub fn shared() -> &'static Shared {
    SHARED.get_or_init(|| {
        let user = super::game_settings::configuration_path().ok().and_then(|p| std::fs::read_to_string(p).ok());
        let console = crate::assets::installation::game_base()
            .ok()
            .and_then(|base| Console::load(&base, user.as_deref()).ok())
            .unwrap_or_else(|| Console { bindings: Bindings::default(), cvars: CVars::new(), default_versions: (None, None) });
        Shared { console: RwLock::new(console), generation: AtomicU64::new(0) }
    })
}

impl Shared {
    pub fn read(&self) -> RwLockReadGuard<'_, Console> {
        self.console.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Change the state; readers see a new [`Shared::generation`] when a cvar
    /// or a binding changed.
    pub fn write<R>(&self, f: impl FnOnce(&mut Console) -> R) -> R {
        let mut guard = self.console.write().unwrap_or_else(|e| e.into_inner());
        let (changes, bindings) = (guard.cvars.changes, guard.bindings.sets.clone());
        let r = f(&mut guard);
        if guard.cvars.changes != changes || guard.bindings.sets != bindings {
            self.generation.fetch_add(1, Ordering::Release);
        }
        r
    }

    /// Advances when settings changed.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Every registered cvar with its current value (defaults included), in
    /// the config file's `name "value"` lines the settings readers parse.
    pub fn cvar_text(&self) -> String {
        let mut out = String::new();
        for c in &self.read().cvars.list {
            out.push_str(&format!("{} \"{}\"\n", c.name, c.value));
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct Console {
    pub bindings: Bindings,
    pub cvars: CVars,
    /// `configVersion` / `bindingVersion` read from default.cfg (`+0x1e88`,
    /// `+0x1e8c`).
    pub default_versions: (Option<i64>, Option<i64>),
}

impl Console {
    /// Run command text: binding commands and `exec`, then cvar commands
    /// (`FUN_140100f20`); other engine commands are not state kept here.
    pub fn execute(&mut self, base: &Path, text: &str) -> Result<(), String> {
        let mut exec = |name: &str| std::fs::read_to_string(base.join(name)).ok();
        let cvars = &mut self.cvars;
        self.bindings.execute_with(text, &mut exec, &mut |args| {
            cvars.command(args);
        })
    }

    /// `base/default.cfg` (which execs `default_keybinds.cfg` and
    /// `default_sku.cfg`), then the user config when its `configVersion` is
    /// 10 and its `bindingVersion` is at least default.cfg's (chunk_000012
    /// ~2430); a config that fails the check is ignored entirely.
    pub fn load(base: &Path, user_config: Option<&str>) -> Result<Console, String> {
        let default = std::fs::read_to_string(base.join("default.cfg")).map_err(|_| "default.cfg not found")?;
        let mut console = Console { bindings: Bindings::default(), cvars: CVars::new(), default_versions: versions(&default) };
        console.execute(base, &default)?;
        if let Some(user) = user_config {
            // The default bindingVersion field keeps its initial value when
            // default.cfg has none; that value is not traced, 0 is assumed.
            let required = console.default_versions.1.unwrap_or(0);
            let (config, binding) = versions(user);
            if config == Some(CONFIG_VERSION) && binding.is_some_and(|v| v >= required) {
                console.execute(base, user)?;
            }
        }
        Ok(console)
    }

    /// `FUN_14003b120(manager, file, 0, 0x100)`: every set's bindings for
    /// keys below 0x100, after `arkUnbindAll`.
    fn write_bindings(&self, out: &mut String) {
        out.push_str("//========================== Binding commands ==========================\n");
        out.push_str("arkUnbindAll\n");
        for (set, (_, set_name)) in self.bindings.sets.iter().zip(BINDING_SETS) {
            out.push_str(&format!("\narkSwitchToBindingSet \"{set_name}\"\n"));
            for (key, entry) in set.iter().enumerate().take(0x100) {
                // The first key-name row for the code; unnamed keys are skipped.
                let Some((_, key_name, _)) = KEY_NAMES.iter().find(|k| k.0 as usize == key) else { continue };
                // Empty entries are not written after arkUnbindAll.
                if entry.action == 0 && entry.command.is_empty() {
                    continue;
                }
                let command = if entry.secondary { "arkBindSecondary" } else { "arkBind" };
                if *key_name == "\\" {
                    out.push_str(&format!("{command} \"\\\\\" "));
                } else {
                    out.push_str(&format!("{command} \"{key_name}\" "));
                }
                if entry.action != 0 {
                    // An id missing from the action table prints a null name.
                    let name = ACTIONS.iter().find(|a| a.0 == entry.action).map_or("(null)", |a| a.1);
                    out.push_str(&format!("\"{name}\"\n"));
                } else {
                    out.push_str(&format!("\"{}\"\n", entry.command));
                }
            }
        }
        out.push_str("\narkSwitchToDefaultBindingSet\n");
        out.push_str("//======================================================================\n");
    }

    /// `FUN_140041770(file, 2, 0x10000, false)`: the config text, refused
    /// when default.cfg's configVersion is not 10.
    pub fn config_text(&self) -> Result<String, String> {
        if self.default_versions.0 != Some(CONFIG_VERSION) {
            return Err(format!(
                "Unable to write '{CONFIG_FILE}' because default.cfg has version {:?} instead of current version {CONFIG_VERSION}",
                self.default_versions.0
            ));
        }
        let binding_version = self.default_versions.1.unwrap_or(0);
        let mut out = String::new();
        out.push_str(&format!("configVersion {CONFIG_VERSION}\n"));
        out.push_str(&format!("bindingVersion {binding_version}\n"));
        out.push_str(&format!("// Restoring last configured key bindings stored in file \"{CONFIG_FILE}\"\n"));
        out.push_str(&format!(
            "// All previous bindings will be unbound! Please increment the version number in default.cfg to apply any new bindings (current version is {binding_version})\n"
        ));
        out.push_str("//\n");
        self.write_bindings(&mut out);
        self.cvars.write(flag::ARCHIVE, &mut out);
        Ok(out)
    }

    /// `FUN_1400d2fc0` on a profile update after a config save: the archive
    /// flag is always marked modified, so the file is always rewritten.
    pub fn save_config(&mut self, path: &Path) -> Result<(), String> {
        self.cvars.modified_flags &= !flag::ARCHIVE;
        let text = self.config_text()?;
        std::fs::write(path, text).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires the installed game and its user config"]
    fn installed_config_round_trips() {
        let base = crate::assets::installation::game_base().unwrap();
        let path = crate::platform::game_settings::configuration_path().unwrap();
        let user = std::fs::read_to_string(&path).unwrap();
        let console = Console::load(&base, Some(&user)).unwrap();
        let written = console.config_text().unwrap();
        assert_eq!(written.replace("\r\n", "\n"), user.replace("\r\n", "\n"));
    }
}
