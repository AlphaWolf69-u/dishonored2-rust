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
        let root = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Dishonored2Rewrite")
            .join("logs");
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
