//! Host-side session diagnostics, independent of the current working directory.
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    sync::{mpsc, Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static SESSION_LOG: OnceLock<Arc<File>> = OnceLock::new();

/// File output written by one log thread. Formatting stays on the calling
/// thread, but the write does not: a stderr, session-log or report write
/// blocked for 120-240 ms on file-cache flushes and stalled the frame
/// thread that wrote it (eotw walk benchmark). Each write happens as soon
/// as it arrives, without buffering, so a crash still keeps the tail.
enum Output { Session, File(Arc<File>) }

struct LogThread {
    sender: Mutex<mpsc::Sender<(Output, Vec<u8>)>>,
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

fn log_thread() -> &'static LogThread {
    static THREAD: OnceLock<LogThread> = OnceLock::new();
    THREAD.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<(Output, Vec<u8>)>();
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let written = pending.clone();
        let _ = std::thread::Builder::new().name("Log Writer".into()).spawn(move || {
            for (output, bytes) in receiver {
                match output {
                    Output::Session => if let Some(file) = SESSION_LOG.get() { let _ = file.as_ref().write_all(&bytes); },
                    Output::File(file) => { let _ = file.as_ref().write_all(&bytes); }
                }
                written.fetch_sub(1, std::sync::atomic::Ordering::Release);
            }
        });
        LogThread { sender: Mutex::new(sender), pending }
    })
}

/// Waits up to `limit` for queued log output to be written (panics).
pub fn drain_log(limit: Duration) {
    let started = std::time::Instant::now();
    while log_thread().pending.load(std::sync::atomic::Ordering::Acquire) != 0 && started.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// One formatted event, queued for the log thread when dropped.
pub struct QueuedWrite { output: Option<Output>, bytes: Vec<u8> }

impl Write for QueuedWrite {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.bytes.extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl Drop for QueuedWrite {
    fn drop(&mut self) {
        let (Some(output), false) = (self.output.take(), self.bytes.is_empty()) else { return };
        let thread = log_thread();
        thread.pending.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let sent = thread.sender.lock().map(|sender| sender.send((output, std::mem::take(&mut self.bytes))).is_ok());
        if !matches!(sent, Ok(true)) { thread.pending.fetch_sub(1, std::sync::atomic::Ordering::AcqRel); }
    }
}

/// A file whose writes the log thread performs.
#[derive(Clone)]
pub struct QueuedFile(Arc<File>);

impl QueuedFile {
    pub fn new(file: File) -> Self { Self(Arc::new(file)) }
    /// A writer that queues everything written to it when dropped.
    pub fn writer(&self) -> QueuedWrite { QueuedWrite { output: Some(Output::File(self.0.clone())), bytes: Vec::new() } }
}

#[derive(Clone, Copy)]
struct SessionOutput;

impl<'a> bevy::log::tracing_subscriber::fmt::MakeWriter<'a> for SessionOutput {
    type Writer = QueuedWrite;
    fn make_writer(&'a self) -> QueuedWrite { QueuedWrite { output: Some(Output::Session), bytes: Vec::new() } }
}

/// Standard output and error go through in-memory pipes that a writer
/// thread drains into the original handles. Rust's stdio resolves the
/// standard handle on every write, so every print, including Bevy's stderr
/// log formatter, only copies into the pipe; it blocks only when 1 MiB is
/// waiting. A redirected console or file then never stalls the frame.
#[cfg(windows)]
mod standard_streams {
    use std::sync::OnceLock;
    type Handle = isize;
    unsafe extern "system" {
        fn GetStdHandle(id: u32) -> Handle;
        fn SetStdHandle(id: u32, handle: Handle) -> i32;
        fn CreatePipe(read: *mut Handle, write: *mut Handle, attributes: *const core::ffi::c_void, size: u32) -> i32;
        fn ReadFile(file: Handle, buffer: *mut u8, length: u32, read: *mut u32, overlapped: *mut core::ffi::c_void) -> i32;
        fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut core::ffi::c_void) -> i32;
        fn PeekNamedPipe(pipe: Handle, buffer: *mut u8, size: u32, read: *mut u32, available: *mut u32, left: *mut u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    static PIPES: OnceLock<Vec<Handle>> = OnceLock::new();

    pub fn decouple() {
        let mut pipes = Vec::new();
        for id in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: plain Win32 calls on handles owned by this process.
            unsafe {
                let original = GetStdHandle(id);
                if original == 0 || original == -1 { continue; }
                let (mut read, mut write) = (0, 0);
                if CreatePipe(&mut read, &mut write, std::ptr::null(), 1 << 20) == 0 { continue; }
                if SetStdHandle(id, write) == 0 { continue; }
                pipes.push(read);
                let _ = std::thread::Builder::new().name("Console Writer".into()).spawn(move || {
                    let mut buffer = vec![0u8; 1 << 16];
                    loop {
                        let mut count = 0u32;
                        if ReadFile(read, buffer.as_mut_ptr(), buffer.len() as u32, &mut count, std::ptr::null_mut()) == 0 || count == 0 { break; }
                        let mut offset = 0;
                        while offset < count {
                            let mut written = 0u32;
                            if WriteFile(original, buffer.as_ptr().add(offset as usize), count - offset, &mut written, std::ptr::null_mut()) == 0 || written == 0 { break; }
                            offset += written;
                        }
                    }
                });
            }
        }
        let _ = PIPES.set(pipes);
    }

    /// Waits up to `limit` for the pipes to empty (panics).
    pub fn drain(limit: std::time::Duration) {
        let started = std::time::Instant::now();
        let pending = || PIPES.get().is_some_and(|pipes| pipes.iter().any(|&pipe| {
            let mut available = 0u32;
            // SAFETY: PIPES holds this process's pipe read handles.
            unsafe { PeekNamedPipe(pipe, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut available, std::ptr::null_mut()) != 0 && available != 0 }
        }));
        while pending() && started.elapsed() < limit { std::thread::sleep(std::time::Duration::from_millis(1)); }
    }
}

pub fn initialize() {
    #[cfg(windows)]
    standard_streams::decouple();
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
        drain_log(Duration::from_millis(500));
        #[cfg(windows)]
        standard_streams::drain(Duration::from_millis(500));
        record(&format!(
            "PANIC on thread {:?}: {info}\n{}",
            std::thread::current().name(),
            std::backtrace::Backtrace::force_capture()
        ));
        previous(info);
        #[cfg(windows)]
        standard_streams::drain(Duration::from_millis(500));
    }));
}

/// The rewrite folder's first-run work and the active setup: game folder,
/// enabled mods and the setup fingerprint.
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
        record(&format!("Mod: {} {} (rewrite {}, {})", m.name, m.version, m.rewrite, m.saves.label()));
    }
    record(&format!("Mod resources: {}", layers.files.len()));
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
    SESSION_LOG.get()?;
    Some(Box::new(
        bevy::log::tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(SessionOutput),
    ))
}
