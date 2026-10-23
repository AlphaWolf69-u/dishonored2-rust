//! Host-side session diagnostics, independent of the current working directory.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

static SESSION_LOG: OnceLock<Arc<File>> = OnceLock::new();

pub fn initialize() {
    let result = (|| -> std::io::Result<_> {
        let root = crate::platform::home::home().logs_dir();
        fs::create_dir_all(&root)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let path = root.join(format!(
            "session-{}-{}.log",
            timestamp.as_millis(),
            std::process::id()
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        Ok((path, file))
    })();
    let (path, file) = match result {
        Ok(result) => result,
        Err(error) => {
            eprintln!("Unable to open session log: {error}");
            return;
        }
    };
    if SESSION_LOG.set(Arc::new(file)).is_err() {
        return;
    }
    eprintln!("Session log: {}", path.display());
    record(&format!(
        "Executable: {:?}\nArguments: {:?}\nWorking directory: {:?}",
        std::env::current_exe(),
        std::env::args_os().collect::<Vec<_>>(),
        std::env::current_dir()
    ));
    for name in [
        "WGPU_BACKEND",
        "WGPU_DX12_COMPILER",
        "DH2_BORDERLESS",
        "DH2_BENCHMARK_SIZE",
        "DH2_GAME_BASE",
        "DH2_REWRITE_HOME",
    ] {
        record(&format!("{name}={:?}", std::env::var_os(name)));
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record(&format!(
            "PANIC on thread {:?}: {info}\n{}",
            std::thread::current().name(),
            std::backtrace::Backtrace::force_capture()
        ));
        previous(info);
    }));
}

/// The rewrite folder's first-run work and the active setup: game folder,
/// enabled mods, overrides and the setup fingerprint.
pub fn record_setup(base: &std::path::Path) {
    use crate::platform::home;
    let h = home::home();
    let setup = home::setup();
    record(&format!("Rewrite folder: {}", h.root.display()));
    if setup.created {
        record("Rewrite folder created");
    }
    let layers = home::layers();
    for problem in setup.problems.iter().chain(&layers.problems) {
        record(&format!("Setup problem: {problem}"));
        eprintln!("Setup problem: {problem}");
    }
    record(&format!("Game folder: {}", base.display()));
    for m in &layers.mods {
        record(&format!("Mod: {} {} ({})", m.id, m.version, m.name));
    }
    record(&format!("Replaced resources: {}", layers.files.len()));
    record(&format!("Setup fingerprint: {}", home::fingerprint(base, &layers)));
}

pub fn record(message: &str) {
    if let Some(file) = SESSION_LOG.get() {
        // One append per record; no buffered tail can be lost during a crash.
        let mut writer = file.as_ref();
        let _ = writer.write_all(format!("{message}\n").as_bytes());
    }
}

pub fn log_layer(_: &mut bevy::app::App) -> Option<bevy::log::BoxedLayer> {
    let file = SESSION_LOG.get()?.clone();
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(file),
    ))
}
