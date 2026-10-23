//! Building the rewrite with the enabled mods, only when the source or the
//! mod selection changed since the last build.
use dishonored2_rust::platform::home::{self, ModInfo};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const STAMP: &str = "build.txt";

/// What a build depends on: the rewrite commit and each enabled mod's
/// name, version and patches, in load order. Mod resources are read at run
/// time and need no build.
pub fn fingerprint(target: &str, mods: &[ModInfo]) -> String {
    let mut h = blake3::Hasher::new();
    home::hash_part(&mut h, target.as_bytes());
    for m in mods {
        home::hash_part(&mut h, m.name.as_bytes());
        home::hash_part(&mut h, m.version.as_bytes());
        for p in m.patches() {
            home::hash_part(&mut h, &std::fs::read(p).unwrap_or_default());
        }
    }
    h.finalize().to_hex().to_string()
}

/// The game executable the builds produce, next to the launcher.
pub fn game_exe() -> PathBuf {
    home::home().root.join(if cfg!(windows) { "dishonored2-rust.exe" } else { "dishonored2-rust" })
}

/// The fingerprint of the current game executable, if it is up to date.
pub fn built() -> Option<String> {
    game_exe().is_file().then(|| std::fs::read_to_string(home::home().cache_dir().join(STAMP)).ok()).flatten().map(|s| s.trim().to_string())
}

/// `cargo build --release` of the game in `source/`, output in
/// `cache/target`, log in `logs/build-*.log`; the executable is copied
/// next to the launcher.
pub fn build(cargo: &Path, fingerprint: &str, progress: &mut dyn FnMut(String)) -> Result<(), String> {
    let h = home::home();
    let log_path = h.logs_dir().join(format!("build-{}.log", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()));
    let mut log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let target = h.cache_dir().join("target");
    let mut child = Command::new(cargo)
        .args(["build", "--release", "--features", "game", "--bin", "dishonored2-rust"])
        .current_dir(h.source_dir())
        .env("CARGO_TARGET_DIR", &target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start cargo: {e}"))?;
    let stderr = child.stderr.take().expect("piped stderr");
    let mut compiled = 0;
    let mut last_error = None;
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let _ = writeln!(log, "{line}");
        let trimmed = line.trim_start();
        if trimmed.starts_with("Compiling") {
            compiled += 1;
            progress(format!("Building the rewrite ({compiled} crates): {trimmed}"));
        } else if trimmed.starts_with("error") && last_error.is_none() {
            last_error = Some(trimmed.to_string());
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("The build failed: {} (full log: {})", last_error.unwrap_or_else(|| status.to_string()), log_path.display()));
    }
    let built = target.join("release").join(game_exe().file_name().unwrap());
    std::fs::copy(&built, game_exe()).map_err(|e| format!("Could not replace {} (is the game running?): {e}", game_exe().display()))?;
    std::fs::write(h.cache_dir().join(STAMP), fingerprint).map_err(|e| e.to_string())
}
